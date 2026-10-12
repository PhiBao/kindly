//! Agent-to-agent negotiation sessions (KATP over HTTP).
//!
//! Flow, all of it real code and real signatures:
//!
//! 1. Kindly opens with a signed `Hello` (carrying its mandate attestation)
//!    and an `Offer`.
//! 2. The client's AP agent — a separate agent with its own key, its own
//!    mandate and its own cash policy — answers with `Accept`, `Counter` or
//!    `Reject`.
//! 3. Every counter is judged by Kindly's policy engine. If it breaks the
//!    owner's mandate, Kindly refuses automatically and counters inside the
//!    limits; if it merely needs judgement, the *owner* is asked. Machines
//!    never decide to go outside the mandate on their own.
//! 4. If the client's own release limit is exceeded, their human is asked —
//!    the AP2 "human present" approval step, on the client's side.
//! 5. On agreement, Kindly issues the plan as real PayPal invoices.
//!
//! Each transcript entry records whether the envelope verified.

use crate::engine::{self, Ledger};
use crate::errors::{ApiError, ApiResult};
use crate::state::AppState;
use crate::store::{A2aBuyerRow, A2aSessionRow, A2aTranscriptEntry};
use chrono::{Duration, Utc};
use kindly_agent::buyer::{BuyerAgent, BuyerDecision, BuyerMandate};
use kindly_agent::katp::{
    public_key_b64, AgentRef, KatpEnvelope, KatpMessage, MandateAttestation, ProposedInstallment,
    TermsProposal,
};
use kindly_core::ids::{ActionId, Actor, InvoiceId, SessionId};
use kindly_core::ledger::{LedgerEvent, LedgerKind};
use kindly_core::money::{Currency, Money};
use kindly_core::negotiation::{build_terms, Installment, Terms, TermsParams};
use kindly_core::policy::{self, PolicyDecision};
use serde_json::json;
use std::sync::Arc;
use tracing::info;

pub const MAX_ROUNDS: i64 = 5;

/// Session lifecycle states.
pub mod status {
    pub const NEGOTIATING: &str = "negotiating";
    pub const AWAITING_OWNER: &str = "awaiting_owner_approval";
    pub const AWAITING_BUYER: &str = "awaiting_buyer_approval";
    pub const AGREED: &str = "agreed";
    pub const PLAN_ISSUED: &str = "plan_issued";
    pub const SETTLED: &str = "settled";
    pub const REJECTED: &str = "rejected";
    pub const STALLED: &str = "stalled";
}

// ─────────────────────────── conversions ───────────────────────────

fn to_proposal(terms: &Terms, currency: &str, valid_days: i64) -> TermsProposal {
    TermsProposal {
        pay_now_minor: terms.pay_now.minor,
        discount_minor: terms.discount.minor,
        currency: currency.to_string(),
        installments: terms
            .installments
            .iter()
            .map(|installment| ProposedInstallment {
                amount_minor: installment.amount.minor,
                due: installment.due,
            })
            .collect(),
        valid_until: Utc::now().date_naive() + Duration::days(valid_days),
    }
}

fn from_proposal(proposal: &TermsProposal, currency: Currency) -> ApiResult<Terms> {
    let to_money = |minor: i64| Money::new(minor, currency);
    Ok(Terms {
        pay_now: to_money(proposal.pay_now_minor),
        discount: to_money(proposal.discount_minor),
        installments: proposal
            .installments
            .iter()
            .map(|installment| Installment {
                amount: to_money(installment.amount_minor),
                due: installment.due,
            })
            .collect(),
        deadline: Some(proposal.valid_until),
    })
}

// ─────────────────────────── seller identity ───────────────────────────

fn seller_ref(state: &AppState) -> AgentRef {
    AgentRef {
        agent_id: state.seller_agent_id.clone(),
        company: state.config.owner_business.clone(),
        public_key: public_key_b64(&state.seller_key),
    }
}

fn seller_attestation(
    state: &AppState,
    mandate: &kindly_core::mandate::CollectionMandate,
) -> MandateAttestation {
    let canonical = serde_json::to_string(mandate).unwrap_or_default();
    MandateAttestation::sign(&state.seller_key, &state.seller_agent_id, &canonical)
}

async fn seller_envelope(
    state: &AppState,
    message: KatpMessage,
    with_mandate: bool,
) -> ApiResult<KatpEnvelope> {
    let attestation = if with_mandate {
        let mandate = state.store.active_mandate().await?;
        Some(seller_attestation(state, &mandate))
    } else {
        None
    };
    Ok(KatpEnvelope::build(
        &state.seller_key,
        seller_ref(state),
        attestation,
        message,
    ))
}

// ─────────────────────────── session helpers ───────────────────────────

fn entry(speaker: &str, label: &str, envelope: &KatpEnvelope, note: &str) -> A2aTranscriptEntry {
    A2aTranscriptEntry {
        at: Utc::now(),
        speaker: speaker.to_string(),
        label: label.to_string(),
        message: serde_json::to_value(&envelope.message).unwrap_or(serde_json::Value::Null),
        note: note.to_string(),
        verified: envelope.verify(),
    }
}

fn last_terms_in(transcript: &[A2aTranscriptEntry], from_seller: bool) -> Option<TermsProposal> {
    for entry in transcript.iter().rev() {
        let Ok(message) = serde_json::from_value::<KatpMessage>(entry.message.clone()) else {
            continue;
        };
        let seller_spoke = entry.speaker == "kindly";
        if seller_spoke != from_seller {
            continue;
        }
        match message {
            KatpMessage::Offer { terms, .. } | KatpMessage::Counter { terms, .. } => {
                return Some(terms)
            }
            _ => {}
        }
    }
    None
}

async fn load_session(state: &AppState, session_id: &str) -> ApiResult<A2aSessionRow> {
    state.store.a2a_session(session_id).await
}

async fn save_session(state: &AppState, session: &mut A2aSessionRow) -> ApiResult<()> {
    session.updated_at = Utc::now();
    state.store.save_a2a_session(session).await?;
    state.events.publish(
        "a2a_update",
        json!({
            "session_id": session.id,
            "invoice_id": session.invoice_id,
            "status": session.status,
            "rounds": session.rounds,
        }),
    );
    Ok(())
}

fn buyer_row(agent: &BuyerAgent) -> A2aBuyerRow {
    A2aBuyerRow {
        company: agent.mandate.company.clone(),
        agent_id: agent.mandate.agent_id.clone(),
        contact_name: agent.mandate.contact_name.clone(),
        public_key: public_key_b64(agent.signing_key()),
        policy: agent.mandate.policy.clone(),
        max_unapproved_cash_minor: agent.mandate.max_unapproved_cash_minor,
        max_total_cash_minor: agent.mandate.max_total_cash_minor,
        max_installments: agent.mandate.max_installments,
        payment_run_weekdays: agent.mandate.payment_run_weekdays.clone(),
        discount_ask_bps: agent.mandate.discount_ask_bps,
    }
}

async fn buyer_for_session(state: &AppState, session_id: &str) -> ApiResult<Arc<BuyerAgent>> {
    state
        .buyer_sessions
        .lock()
        .await
        .get(session_id)
        .cloned()
        .ok_or_else(|| {
            ApiError::Conflict(
                "the counterparty agent for this session is no longer loaded (agent keys are ephemeral and reset with the server) — start a new session"
                    .into(),
            )
        })
}

fn invoice_id_of(session: &A2aSessionRow) -> ApiResult<InvoiceId> {
    session
        .invoice_id
        .parse()
        .map_err(|_| ApiError::Internal("bad invoice id in session".into()))
}

// ─────────────────────────── start ───────────────────────────

pub async fn start_session(
    state: &AppState,
    invoice_id: InvoiceId,
    initial_terms: Terms,
    origin_action: ActionId,
) -> ApiResult<()> {
    let invoice = state.store.invoice(invoice_id).await?;
    let client = state.store.client(invoice.client_id).await?;

    // The counterparty agent, built from the client's real profile.
    let mut mandate = BuyerMandate::example(client.display_name());
    if client.behavior.avg_days_late >= 25 {
        mandate.max_unapproved_cash_minor = 60_000;
        mandate
            .policy
            .push("Slow-payer profile: tighter release limits for this supplier.".into());
    }
    let buyer = Arc::new(BuyerAgent::new(mandate, state.llm.clone()));

    let session_id = SessionId::new().to_string();
    state
        .buyer_sessions
        .lock()
        .await
        .insert(session_id.clone(), buyer.clone());

    let ledger = Ledger::new(&state.store, Some(&state.events));
    ledger
        .note(
            Actor::KindlyAgent,
            LedgerKind::AgentToAgentStep,
            format!(
                "Opening agent-to-agent talks with {}. Their policy: {}",
                client.display_name(),
                buyer.mandate.policy.first().cloned().unwrap_or_default()
            ),
            Some(invoice_id),
        )
        .await?;

    let mut session = A2aSessionRow {
        id: session_id,
        invoice_id: invoice_id.to_string(),
        status: status::NEGOTIATING.to_string(),
        rounds: 1,
        transcript: Vec::new(),
        buyer: buyer_row(&buyer),
        created_at: Utc::now(),
        updated_at: Utc::now(),
    };

    // 1. Hello from Kindly (mandate attested).
    let hello = seller_envelope(
        state,
        KatpMessage::Hello {
            agent: state.seller_agent_id.clone(),
            company: state.config.owner_business.clone(),
            mandate_digest: state.store.active_mandate().await?.digest(),
        },
        true,
    )
    .await?;
    session.transcript.push(entry(
        "kindly",
        "Kindly (collections agent)",
        &hello,
        "Introduces itself and signs the limits it is acting under.",
    ));

    // 2. The client's AP agent answers with its own attested policy.
    let hello_reply = buyer.hello();
    session.transcript.push(entry(
        "client_agent",
        &buyer.mandate.contact_name,
        &hello_reply,
        "The client's AP agent introduces itself and attests to its cash policy.",
    ));

    // 3. Kindly's offer.
    let proposal = to_proposal(&initial_terms, invoice.currency.code(), 7);
    let offer = seller_envelope(
        state,
        KatpMessage::Offer {
            terms: proposal.clone(),
            note: format!(
                "Proposal: {} now and the balance shortly after.",
                initial_terms.pay_now.display_with_symbol()
            ),
        },
        false,
    )
    .await?;
    session.transcript.push(entry(
        "kindly",
        "Kindly (collections agent)",
        &offer,
        &format!(
            "Offers {} up front, {} instalment(s).",
            initial_terms.pay_now.display_with_symbol(),
            initial_terms.installment_count()
        ),
    ));

    // 4. The counterpary responds, then the seller judges that response.
    let (response, decision) = buyer
        .respond(
            &offer.message,
            invoice.outstanding().minor,
            invoice.currency.code(),
        )
        .await;
    session.transcript.push(entry(
        "client_agent",
        &buyer.mandate.contact_name,
        &response,
        &format!(
            "{} — {}",
            capitalise(decision.label()),
            decision.reason()
        ),
    ));

    match &response.message {
        KatpMessage::Accept { terms, .. } => {
            let agreed = from_proposal(terms, invoice.currency)?;
            accept_and_issue(state, &mut session, &agreed).await?;
        }
        KatpMessage::Counter { terms, .. } => {
            let countered = from_proposal(terms, invoice.currency)?;
            seller_answers_counter(state, &mut session, &countered, &buyer).await?;
        }
        KatpMessage::Reject { .. } => {
            session.status = status::REJECTED.to_string();
            ledger
                .note(
                    Actor::ClientAgent,
                    LedgerKind::AgentToAgentStep,
                    format!(
                        "{}'s agent declined the terms outright.",
                        client.display_name()
                    ),
                    Some(invoice_id),
                )
                .await?;
        }
        _ => {}
    }

    ledger
        .write(
            LedgerEvent::new(
                Utc::now(),
                Actor::ClientAgent,
                LedgerKind::AgentToAgentStep,
                format!(
                    "{}'s agent: {} ({})",
                    client.display_name(),
                    decision.label(),
                    decision.reason()
                ),
            )
            .with_invoice(invoice_id)
            .with_detail(json!({
                "verified": response.verify(),
                "status": session.status,
                "origin_action": origin_action.to_string(),
            })),
        )
        .await?;

    save_session(state, &mut session).await?;
    info!(session = %session.id, status = %session.status, "a2a session started");
    Ok(())
}

// ─────────────────────────── the seller's judgement ───────────────────────────

/// Judge the buyer's counter against the owner's mandate and act accordingly.
async fn seller_answers_counter(
    state: &AppState,
    session: &mut A2aSessionRow,
    countered: &Terms,
    buyer: &Arc<BuyerAgent>,
) -> ApiResult<()> {
    let invoice_id = invoice_id_of(session)?;
    let invoice = state.store.invoice(invoice_id).await?;
    let mandate = state.store.active_mandate().await?;
    let ledger = Ledger::new(&state.store, Some(&state.events));
    let decision = policy::evaluate_terms(countered, &invoice, &mandate, Utc::now().date_naive());

    match decision {
        PolicyDecision::Allow => {
            let accept = seller_envelope(
                state,
                KatpMessage::Accept {
                    terms: to_proposal(countered, invoice.currency.code(), 7),
                    note: "Agreed — inside what I can sign for.".to_string(),
                },
                false,
            )
            .await?;
            session.transcript.push(entry(
                "kindly",
                "Kindly (collections agent)",
                &accept,
                "Their counter is inside your limits; accepted.",
            ));
            ledger
                .note(
                    Actor::KindlyAgent,
                    LedgerKind::AgentToAgentStep,
                    format!(
                        "Accepted {}'s counter — inside your mandate.",
                        buyer.mandate.company
                    ),
                    Some(invoice_id),
                )
                .await?;
            accept_and_issue(state, session, countered).await
        }
        PolicyDecision::RequireApproval { .. } => {
            session.status = status::AWAITING_OWNER.to_string();
            ledger
                .write(
                    LedgerEvent::new(
                        Utc::now(),
                        Actor::KindlyAgent,
                        LedgerKind::PolicyApprovalRequested,
                        format!(
                            "{}'s agent offered {} up front — that needs your approval ({})",
                            buyer.mandate.company,
                            countered.pay_now.display_with_symbol(),
                            decision.explain()
                        ),
                    )
                    .with_invoice(invoice_id),
                )
                .await?;
            Ok(())
        }
        PolicyDecision::Block { .. } => {
            // Hold the line: counter with terms the mandate accepts.
            let params = TermsParams {
                upfront_bps: mandate.min_upfront_settlement_bps,
                installments: mandate.max_installments.min(2).max(1),
                discount_bps: 0,
                days_between_installments: 7,
                note: None,
            };
            let counter = build_terms(invoice.outstanding(), &params, &mandate, Utc::now().date_naive())
                .map_err(ApiError::Domain)?;
            let envelope = seller_envelope(
                state,
                KatpMessage::Counter {
                    terms: to_proposal(&counter, invoice.currency.code(), 7),
                    note: format!(
                        "I can't go below {} up front — that's a hard limit. This is the best I can do on that basis.",
                        counter.pay_now.display_with_symbol()
                    ),
                },
                false,
            )
            .await?;
            session.transcript.push(entry(
                "kindly",
                "Kindly (collections agent)",
                &envelope,
                &format!(
                    "Refused their counter automatically ({}). Kindly held your floor: {} up front.",
                    decision.explain(),
                    counter.pay_now.display_with_symbol()
                ),
            ));
            session.rounds += 1;
            ledger
                .write(
                    LedgerEvent::new(
                        Utc::now(),
                        Actor::KindlyAgent,
                        LedgerKind::PolicyBlocked,
                        format!(
                            "{}'s counter broke your mandate ({}). Kindly refused and countered inside your limits.",
                            buyer.mandate.company,
                            decision.explain()
                        ),
                    )
                    .with_invoice(invoice_id),
                )
                .await?;

            if session.rounds > MAX_ROUNDS {
                session.status = status::STALLED.to_string();
                ledger
                    .note(
                        Actor::KindlyAgent,
                        LedgerKind::AgentToAgentStep,
                        "Negotiation reached the round limit without agreement — your move.".to_string(),
                        Some(invoice_id),
                    )
                    .await?;
                return Ok(());
            }

            let (response, buyer_decision) = buyer
                .respond(
                    &envelope.message,
                    invoice.outstanding().minor,
                    invoice.currency.code(),
                )
                .await;
            session.transcript.push(entry(
                "client_agent",
                &buyer.mandate.contact_name,
                &response,
                &format!(
                    "{} — {}",
                    capitalise(buyer_decision.label()),
                    buyer_decision.reason()
                ),
            ));
            match &buyer_decision {
                BuyerDecision::Accept { .. } => {
                    if let KatpMessage::Accept { terms, .. } = &response.message {
                        let agreed = from_proposal(terms, invoice.currency)?;
                        accept_and_issue(state, session, &agreed).await?;
                    }
                }
                BuyerDecision::NeedsApproval { .. } => {
                    // Their agent cannot release this much cash alone, so the
                    // decision moves to the client's finance lead — and the
                    // offer standing in front of them is ours.
                    session.status = status::AWAITING_BUYER.to_string();
                    ledger
                        .note(
                            Actor::ClientAgent,
                            LedgerKind::AgentToAgentStep,
                            format!(
                                "{}'s agent escalated to its finance lead: releasing {} needs a human.",
                                buyer.mandate.company,
                                counter.pay_now.display_with_symbol()
                            ),
                            Some(invoice_id),
                        )
                        .await?;
                }
                BuyerDecision::Counter { .. } => {
                    session.status = status::NEGOTIATING.to_string();
                }
                BuyerDecision::Reject { .. } => {
                    session.status = status::REJECTED.to_string();
                }
            }
            Ok(())
        }
    }
}

/// Advance a negotiation that is waiting on Kindly's move.
pub async fn step_session(state: &AppState, session_id: &str) -> ApiResult<A2aSessionRow> {
    let mut session = load_session(state, session_id).await?;
    if session.status != status::NEGOTIATING {
        return Ok(session);
    }
    let invoice_id = invoice_id_of(&session)?;
    let invoice = state.store.invoice(invoice_id).await?;
    let buyer = buyer_for_session(state, session_id).await?;

    let Some(counter_proposal) = last_terms_in(&session.transcript, false) else {
        return Ok(session);
    };
    let countered = from_proposal(&counter_proposal, invoice.currency)?;
    seller_answers_counter(state, &mut session, &countered, &buyer).await?;
    save_session(state, &mut session).await?;
    Ok(session)
}

// ─────────────────────────── agreement ───────────────────────────

async fn accept_and_issue(
    state: &AppState,
    session: &mut A2aSessionRow,
    agreed: &Terms,
) -> ApiResult<()> {
    let invoice_id = invoice_id_of(session)?;
    let invoice = state.store.invoice(invoice_id).await?;
    let ledger = Ledger::new(&state.store, Some(&state.events));
    let deferred = Money::sum(
        agreed.installments.iter().map(|i| i.amount),
        invoice.currency,
    )
    .unwrap_or(Money::zero(invoice.currency));

    session.status = status::AGREED.to_string();
    ledger
        .note(
            Actor::KindlyAgent,
            LedgerKind::AgentToAgentStep,
            format!(
                "Agreement reached with {}: {} now, {} in {} instalment(s).",
                session.buyer.company,
                agreed.pay_now.display_with_symbol(),
                deferred.display_with_symbol(),
                agreed.installment_count()
            ),
            Some(invoice_id),
        )
        .await?;

    // Record the agreed terms against the negotiation…
    let mut negotiation = state.store.negotiation(invoice_id).await?;
    let mandate = state.store.active_mandate().await?;
    let effects = kindly_core::negotiation::apply(
        &mut negotiation,
        kindly_core::negotiation::NegoEvent::TermsAccepted {
            at: Utc::now(),
            terms: agreed.clone(),
        },
        &mandate,
    )
    .map_err(ApiError::Domain)?;
    state.store.save_negotiation(&negotiation).await?;
    for effect in effects {
        engine::apply_effect(state, &invoice_id, effect).await?;
    }

    // …and issue it for real.
    engine::issue_plan_invoices(state, invoice_id, agreed).await?;
    session.status = status::PLAN_ISSUED.to_string();
    Ok(())
}

// ─────────────────────────── human decisions ───────────────────────────

/// The owner decides about a counter that broke their limits.
pub async fn owner_decision(
    state: &AppState,
    session_id: &str,
    approve: bool,
) -> ApiResult<A2aSessionRow> {
    let mut session = load_session(state, session_id).await?;
    if session.status != status::AWAITING_OWNER {
        return Err(ApiError::Conflict(format!(
            "this negotiation is {} — nothing is waiting on you",
            session.status
        )));
    }
    let invoice_id = invoice_id_of(&session)?;
    let invoice = state.store.invoice(invoice_id).await?;
    let ledger = Ledger::new(&state.store, Some(&state.events));
    let buyer = buyer_for_session(state, session_id).await?;

    let Some(counter_proposal) = last_terms_in(&session.transcript, false) else {
        return Err(ApiError::Conflict("no counter to decide on".into()));
    };
    let countered = from_proposal(&counter_proposal, invoice.currency)?;

    if approve {
        let accept = seller_envelope(
            state,
            KatpMessage::Accept {
                terms: counter_proposal.clone(),
                note: "Our principal approved this exception. Agreed.".to_string(),
            },
            false,
        )
        .await?;
        session.transcript.push(entry(
            "kindly",
            "Kindly (collections agent)",
            &accept,
            "You approved the exception; accepted on your behalf.",
        ));
        ledger
            .note(
                Actor::Owner,
                LedgerKind::PolicyApprovalGranted,
                format!(
                    "You approved {}'s counter of {} up front.",
                    buyer.mandate.company,
                    countered.pay_now.display_with_symbol()
                ),
                Some(invoice_id),
            )
            .await?;
        accept_and_issue(state, &mut session, &countered).await?;
    } else {
        let mandate = state.store.active_mandate().await?;
        let params = TermsParams {
            upfront_bps: mandate.min_upfront_settlement_bps,
            installments: mandate.max_installments.min(2).max(1),
            discount_bps: 0,
            days_between_installments: 7,
            note: None,
        };
        let counter = build_terms(invoice.outstanding(), &params, &mandate, Utc::now().date_naive())
            .map_err(ApiError::Domain)?;
        let envelope = seller_envelope(
            state,
            KatpMessage::Counter {
                terms: to_proposal(&counter, invoice.currency.code(), 7),
                note: format!(
                    "My principal won't go below {} up front. This is where we stand.",
                    counter.pay_now.display_with_symbol()
                ),
            },
            false,
        )
        .await?;
        session.transcript.push(entry(
            "kindly",
            "Kindly (collections agent)",
            &envelope,
            "You held the line; Kindly countered inside your limits.",
        ));
        ledger
            .note(
                Actor::Owner,
                LedgerKind::PolicyApprovalRejected,
                format!(
                    "You declined {}'s counter; Kindly held your floor.",
                    buyer.mandate.company
                ),
                Some(invoice_id),
            )
            .await?;

        let (response, decision) = buyer
            .respond(
                &envelope.message,
                invoice.outstanding().minor,
                invoice.currency.code(),
            )
            .await;
        session.transcript.push(entry(
            "client_agent",
            &buyer.mandate.contact_name,
            &response,
            &format!(
                "{} — {}",
                capitalise(decision.label()),
                decision.reason()
            ),
        ));
        match &decision {
            BuyerDecision::Accept { .. } => {
                if let KatpMessage::Accept { terms, .. } = &response.message {
                    let agreed = from_proposal(terms, invoice.currency)?;
                    accept_and_issue(state, &mut session, &agreed).await?;
                }
            }
            BuyerDecision::NeedsApproval { .. } => {
                session.status = status::AWAITING_BUYER.to_string();
            }
            BuyerDecision::Counter { .. } => {
                session.status = status::NEGOTIATING.to_string();
            }
            BuyerDecision::Reject { .. } => {
                session.status = status::REJECTED.to_string();
            }
        }
    }

    save_session(state, &mut session).await?;
    Ok(session)
}

/// The client's human releases the cash their agent flagged.
pub async fn buyer_approval(state: &AppState, session_id: &str) -> ApiResult<A2aSessionRow> {
    let mut session = load_session(state, session_id).await?;
    if session.status != status::AWAITING_BUYER {
        return Err(ApiError::Conflict(format!(
            "the client's approver is not needed — this negotiation is {}",
            session.status
        )));
    }
    let invoice_id = invoice_id_of(&session)?;
    let invoice = state.store.invoice(invoice_id).await?;
    let Some(offer_proposal) = last_terms_in(&session.transcript, true) else {
        return Err(ApiError::Conflict("no offer to approve".into()));
    };
    let terms = from_proposal(&offer_proposal, invoice.currency)?;

    let buyer = buyer_for_session(state, session_id).await?;
    let accept = buyer.envelope(
        KatpMessage::Accept {
            terms: offer_proposal.clone(),
            note: format!(
                "Finance lead approved the release. {}'s first payment goes out now.",
                session.buyer.company
            ),
        },
        true,
    );
    session.transcript.push(entry(
        "client_agent",
        &session.buyer.contact_name,
        &accept,
        "The client's finance lead approved the release; their agent accepts.",
    ));

    let ledger = Ledger::new(&state.store, Some(&state.events));
    ledger
        .note(
            Actor::Owner,
            LedgerKind::AgentToAgentStep,
            format!(
                "{} approved the release on their side — acceptance signed by their agent.",
                session.buyer.company
            ),
            Some(invoice_id),
        )
        .await?;

    accept_and_issue(state, &mut session, &terms).await?;
    save_session(state, &mut session).await?;
    Ok(session)
}

/// Record that money moved against an agreed plan.
pub async fn record_settlement(
    state: &AppState,
    session_id: &str,
    amount: Money,
    reference: &str,
) -> ApiResult<()> {
    let mut session = load_session(state, session_id).await?;
    let settle = seller_envelope(
        state,
        KatpMessage::Settle {
            reference: reference.to_string(),
            amount_minor: amount.minor,
            currency: amount.currency.code().to_string(),
            note: "Payment received against the agreed plan — thank you.".to_string(),
        },
        false,
    )
    .await?;
    session.transcript.push(entry(
        "kindly",
        "Kindly (collections agent)",
        &settle,
        &format!("Settlement recorded (PayPal reference {reference})."),
    ));
    if matches!(
        session.status.as_str(),
        status::PLAN_ISSUED | status::AGREED | status::SETTLED
    ) {
        session.status = status::SETTLED.to_string();
    }
    save_session(state, &mut session).await?;
    Ok(())
}

fn capitalise(value: &str) -> String {
    let mut chars = value.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}
