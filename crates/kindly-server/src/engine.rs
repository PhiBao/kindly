//! The Kindly runtime.
//!
//! One place where the pieces meet: plan → policy → PayPal → ledger → UI.
//! Every function here is written so that the *policy engine* is the only
//! thing that decides whether an action may happen, and the *ledger* is
//! written for every decision, including the ones that were blocked.

use crate::errors::{ApiError, ApiResult};
use crate::events::EventBus;
use crate::state::AppState;
use crate::store::Store;
use chrono::Utc;
use kindly_agent::compose::{compose_message, ComposeFacts};
use kindly_agent::planner::{DraftMessage, InvoiceBrief};
use kindly_agent::understand::read_reply;
use kindly_core::action::{ActionKind, ActionStatus, AgentAction};
use kindly_core::attribution;
use kindly_core::client::Client;
use kindly_core::ids::{ActionId, Actor, InvoiceId, MessageId, PlanId};
use kindly_core::invoice::{Invoice, InvoiceStatus};
use kindly_core::ledger::{LedgerEvent, LedgerKind};
use kindly_core::mandate::{Channel, CollectionMandate};
use kindly_core::money::Money;
use kindly_core::negotiation::{
    self as nego, CloseOutcome, EffectPurpose, NegoEffect, NegoEvent, Negotiation, ReplyIntent,
    Stage, Terms,
};
use kindly_core::policy::{self, PolicyContext, PolicyDecision, ProposedAction};
use kindly_paypal::model::{NewInvoice, NewInvoiceLine};
use serde_json::json;
use tracing::{info, warn};

/// Idempotency key for an agent action: stable for the lifetime of the action
/// (so a retried PayPal call is recognised as the same operation) and unique
/// across passes.
fn action_id_key(invoice_id: InvoiceId, kind: ActionKind, index: usize) -> String {
    format!("{invoice_id}:{}:{index}:{}", kind.label(), ActionId::new())
}

/// The outcome of one agent pass over one invoice.
#[derive(Debug, Default, serde::Serialize)]
pub struct TickReport {
    pub invoice_id: String,
    pub proposals: usize,
    pub executed: usize,
    pub awaiting_approval: usize,
    pub blocked: usize,
    pub idle_reason: Option<String>,
    pub narrative: Vec<String>,
}

// ─────────────────────────────── ledger helper ───────────────────────────────

pub struct Ledger<'a> {
    store: &'a Store,
    bus: Option<&'a EventBus>,
}

impl<'a> Ledger<'a> {
    pub fn new(store: &'a Store, bus: Option<&'a EventBus>) -> Self {
        Self { store, bus }
    }

    pub async fn write(&self, event: LedgerEvent) -> ApiResult<()> {
        if let Some(bus) = self.bus {
            bus.publish(
                "ledger",
                json!({
                    "at": event.at,
                    "actor": event.actor.as_str(),
                    "kind": event.kind,
                    "summary": event.summary,
                    "invoice_id": event.invoice_id,
                    "trust": event.kind.is_trust_relevant(),
                }),
            );
        }
        self.store.append_ledger(&event).await
    }

    pub async fn note(
        &self,
        actor: Actor,
        kind: LedgerKind,
        summary: impl Into<String>,
        invoice: Option<InvoiceId>,
    ) -> ApiResult<()> {
        let mut event = LedgerEvent::new(Utc::now(), actor, kind, summary);
        if let Some(id) = invoice {
            event = event.with_invoice(id);
        }
        self.write(event).await
    }
}

// ─────────────────────────────── invoice plumbing ───────────────────────────────

/// Push a Kindly invoice into PayPal (creating the mirror object).
pub async fn push_invoice_to_paypal(
    state: &AppState,
    invoice: &mut Invoice,
    client: &Client,
    work_summary: Option<&str>,
) -> ApiResult<()> {
    let lines = invoice
        .lines
        .iter()
        .map(|line| NewInvoiceLine {
            name: line.description.clone(),
            description: None,
            quantity: line.quantity.display(),
            unit_amount_minor: line.unit_price.minor,
        })
        .collect::<Vec<_>>();

    let request = NewInvoice {
        invoice_number: invoice.number.clone(),
        currency_code: invoice.currency.code().to_string(),
        client_email: client.email.clone(),
        client_name: client.company.clone().or_else(|| Some(client.name.clone())),
        invoicer_email: state.config.invoicer_email.clone(),
        due_date: invoice.due_date,
        memo: invoice.memo.clone(),
        note: work_summary.map(str::to_string),
        lines,
        allow_partial_payment: true,
    };

    let created = state.paypal.create_invoice(request).await?;
    invoice.paypal_invoice_id = Some(created.id.clone());
    invoice.paypal_status = Some(created.status.as_str().to_string());
    state.store.upsert_invoice(invoice, work_summary).await?;
    Ledger::new(&state.store, Some(&state.events))
        .write(
            LedgerEvent::new(
                Utc::now(),
                Actor::PayPal,
                LedgerKind::PayPalCall,
                format!(
                    "Created PayPal invoice {} for {} {}",
                    created.id,
                    invoice.total.display_with_symbol(),
                    invoice.number
                ),
            )
            .with_invoice(invoice.id)
            .with_detail(
                json!({ "paypal_invoice_id": created.id, "status": created.status.as_str() }),
            ),
        )
        .await?;
    Ok(())
}

/// Send an invoice to the client through PayPal.
pub async fn send_invoice_via_paypal(
    state: &AppState,
    invoice: &mut Invoice,
    subject: &str,
    note: &str,
) -> ApiResult<()> {
    let Some(paypal_id) = invoice.paypal_invoice_id.clone() else {
        return Err(ApiError::Conflict(
            "invoice has no PayPal mirror yet".to_string(),
        ));
    };
    state.paypal.send_invoice(&paypal_id, subject, note).await?;
    invoice.mark_sent(Utc::now());
    invoice.paypal_status = Some("SENT".to_string());
    state.store.upsert_invoice(invoice, None).await?;
    Ok(())
}

/// Ask PayPal to email its own reminder for an invoice. Best effort: PayPal
/// caps reminders at two per invoice per day, and that cap is respected.
pub async fn paypal_reminder(state: &AppState, invoice: &Invoice) -> ApiResult<bool> {
    let Some(paypal_id) = invoice.paypal_invoice_id.clone() else {
        return Ok(false);
    };
    let subject = format!("Reminder: invoice {}", invoice.number);
    let note = format!(
        "Invoice {} for {} was due on {}. You can pay it from the link in this email.",
        invoice.number,
        invoice.outstanding().display_with_symbol(),
        invoice.due_date.format("%-d %B")
    );
    match state
        .paypal
        .remind_invoice(&paypal_id, &subject, &note)
        .await
    {
        Ok(()) => Ok(true),
        Err(error) => {
            warn!(invoice = %invoice.number, %error, "paypal reminder not sent");
            Ok(false)
        }
    }
}

// ─────────────────────────────── brief building ───────────────────────────────

pub struct BriefInputs {
    pub invoice: Invoice,
    pub client: Client,
    pub negotiation: Negotiation,
    pub mandate: CollectionMandate,
    pub work_summary: Option<String>,
    pub has_ap_agent: bool,
    pub pending_reply: Option<ReplyIntent>,
    pub history: Vec<(String, String)>,
    pub pay_link: String,
}

pub async fn load_brief(state: &AppState, invoice_id: InvoiceId) -> ApiResult<BriefInputs> {
    let store = &state.store;
    let mut invoice = store.invoice(invoice_id).await?;
    let client = store.client(invoice.client_id).await?;
    let negotiation = store.negotiation(invoice_id).await?;
    let mandate = store.active_mandate().await?;
    let work_summary = store.invoice_work_summary(invoice_id).await?;
    let has_ap_agent = store.client_has_ap_agent(invoice.client_id).await?;

    let today = Utc::now().date_naive();
    if invoice.is_open() && invoice.due_date < today && invoice.status != InvoiceStatus::Overdue {
        invoice.mark_overdue();
        store.upsert_invoice(&invoice, None).await?;
    }

    // The most recent client message decides what we owe them.
    let mut pending_reply = None;
    let mut history = Vec::new();
    if let Some(thread_id) = store.thread_for_invoice(invoice_id).await? {
        let messages = store.messages(thread_id).await?;
        for message in &messages {
            if message.source.as_deref() != Some("sent") {
                continue;
            }
            history.push((message.author_label.clone(), message.body.clone()));
            if message.author == Actor::Client {
                if let Some(intent_json) = &message.intent_json {
                    if let Ok(intent) = serde_json::from_str::<ReplyIntent>(intent_json) {
                        pending_reply = Some(intent);
                    }
                }
            }
        }
        if !matches!(
            negotiation.stage,
            Stage::Negotiating | Stage::AwaitingReply | Stage::Contacted
        ) {
            pending_reply = pending_reply.filter(|intent| {
                matches!(
                    intent,
                    ReplyIntent::RequestsPlan { .. } | ReplyIntent::RequestsDiscount { .. }
                )
            });
        }
    }

    let pay_link = format!(
        "{}/c/{}",
        state.config.public_url.trim_end_matches('/'),
        crate::portal::sign_portal_token(
            &state.config.portal_secret,
            invoice.id,
            crate::portal::PORTAL_TOKEN_TTL_DAYS
        )
    );

    Ok(BriefInputs {
        invoice,
        client,
        negotiation,
        mandate,
        work_summary,
        has_ap_agent,
        pending_reply,
        history,
        pay_link,
    })
}

pub fn brief_for(inputs: &BriefInputs) -> InvoiceBrief {
    let today = Utc::now().date_naive();
    InvoiceBrief {
        invoice_number: inputs.invoice.number.clone(),
        currency: inputs.invoice.currency.code().to_string(),
        total_minor: inputs.invoice.total.minor,
        outstanding_minor: inputs.invoice.outstanding().minor,
        due_date: inputs.invoice.due_date,
        today,
        days_overdue: inputs.invoice.days_overdue(today),
        client_company: inputs.client.display_name().to_string(),
        client_contact: inputs.client.name.clone(),
        client_avg_days_late: inputs.client.behavior.avg_days_late,
        client_risk: inputs.client.behavior.risk_band(),
        work_summary: inputs.work_summary.clone(),
        stage: inputs.negotiation.stage,
        contact_count: inputs.negotiation.contact_count,
        last_contact_at: inputs.negotiation.last_contact_at,
        promised_on: inputs.negotiation.promised_on,
        pending_reply: inputs.pending_reply.clone(),
        rounds: inputs.negotiation.rounds,
        client_has_ap_agent: inputs.has_ap_agent,
        pay_link: inputs.pay_link.clone(),
        history: inputs.history.clone(),
    }
}

// ─────────────────────────────── the tick ───────────────────────────────

/// Plan one pass for an invoice and carry out whatever the mandate allows.
pub async fn tick_invoice(state: &AppState, invoice_id: InvoiceId) -> ApiResult<TickReport> {
    let inputs = load_brief(state, invoice_id).await?;
    let mandate = inputs.mandate.clone();
    let brief = brief_for(&inputs);

    let mut report = TickReport {
        invoice_id: invoice_id.to_string(),
        ..Default::default()
    };

    if !brief.stage.is_terminal() && inputs.invoice.outstanding().is_positive() {
        let outcome = state.planner.plan(&brief, &mandate).await;
        report.idle_reason = outcome.idle_reason.clone();
        report.narrative = outcome.narrative.clone();
        report.proposals = outcome.actions.len();

        let ledger = Ledger::new(&state.store, Some(&state.events));
        for narrative in &outcome.narrative {
            ledger
                .note(
                    Actor::KindlyAgent,
                    LedgerKind::AgentProposed,
                    narrative.clone(),
                    Some(invoice_id),
                )
                .await?;
        }

        // Contact history and open-escalation state are read once, then kept
        // up to date as we create actions in this pass.
        let mut recent = state.store.contact_history(invoice_id).await?;
        let mut open_escalation = state
            .store
            .actions_for_invoice(invoice_id)
            .await?
            .iter()
            .any(|a| {
                matches!(a.kind, ActionKind::Escalate)
                    && matches!(
                        a.status,
                        ActionStatus::AwaitingApproval | ActionStatus::Approved
                    )
            });

        let now = Utc::now();
        for (index, proposal) in outcome.actions.iter().enumerate() {
            let draft = outcome
                .drafts
                .iter()
                .find(|d| d.action_kind == proposal.kind());
            let ctx = PolicyContext {
                now,
                mandate: &mandate,
                invoice: &inputs.invoice,
                recent_actions: &recent,
                open_escalation,
            };
            let decision = policy::evaluate(proposal, &ctx);

            // Draft message (if any) is persisted immediately so the approval
            // inbox can show the exact words that would go out.
            let draft_message_id = match draft {
                Some(draft) => Some(persist_draft(state, &inputs, draft, &decision).await?),
                None => None,
            };

            let status = match &decision {
                PolicyDecision::Allow => ActionStatus::Approved,
                PolicyDecision::RequireApproval { .. } => ActionStatus::AwaitingApproval,
                PolicyDecision::Block { .. } => ActionStatus::Blocked,
            };

            let action = AgentAction {
                id: ActionId::new(),
                invoice_id,
                kind: proposal.kind(),
                status,
                proposal: proposal.clone(),
                decision: decision.clone(),
                rationale: outcome
                    .narrative
                    .first()
                    .cloned()
                    .unwrap_or_else(|| decision.explain()),
                idempotency_key: action_id_key(invoice_id, proposal.kind(), index),
                created_at: now,
                executed_at: None,
                error: None,
            };
            // stash the draft message id for the executor
            let action_id = action.id;
            state.store.save_action(&action).await?;
            state
                .store
                .set_action_draft_message(action_id, draft_message_id)
                .await?;

            match &decision {
                PolicyDecision::Allow => {
                    ledger
                        .note(
                            Actor::KindlyAgent,
                            LedgerKind::PolicyAllowed,
                            format!(
                                "{} allowed — {}",
                                proposal.kind().label(),
                                decision.explain()
                            ),
                            Some(invoice_id),
                        )
                        .await?;
                    let executed = execute_action(state, action_id).await;
                    match executed {
                        Ok(()) => {
                            report.executed += 1;
                            if proposal.contacts_client() {
                                recent.push(kindly_core::policy::ActionSummary::new(
                                    proposal.kind(),
                                    now,
                                ));
                            }
                        }
                        Err(error) => {
                            ledger
                                .write(
                                    LedgerEvent::new(
                                        Utc::now(),
                                        Actor::System,
                                        LedgerKind::ActionFailed,
                                        format!("{} failed: {error}", proposal.kind().label()),
                                    )
                                    .with_invoice(invoice_id),
                                )
                                .await?;
                            report.narrative.push(format!("Action failed: {error}"));
                        }
                    }
                }
                PolicyDecision::RequireApproval { .. } => {
                    report.awaiting_approval += 1;
                    ledger
                        .note(
                            Actor::KindlyAgent,
                            LedgerKind::PolicyApprovalRequested,
                            format!(
                                "{} needs your approval — {}",
                                proposal.kind().label(),
                                decision.explain()
                            ),
                            Some(invoice_id),
                        )
                        .await?;
                }
                PolicyDecision::Block { .. } => {
                    report.blocked += 1;
                    ledger
                        .note(
                            Actor::KindlyAgent,
                            LedgerKind::PolicyBlocked,
                            format!(
                                "{} blocked — {}",
                                proposal.kind().label(),
                                decision.explain()
                            ),
                            Some(invoice_id),
                        )
                        .await?;
                }
            }

            if matches!(proposal.kind(), ActionKind::Escalate) && !decision.is_blocked() {
                open_escalation = true;
            }
        }
    }

    Ok(report)
}

async fn persist_draft(
    state: &AppState,
    inputs: &BriefInputs,
    draft: &DraftMessage,
    decision: &PolicyDecision,
) -> ApiResult<MessageId> {
    let thread_id = state
        .store
        .ensure_thread(
            inputs.invoice.id,
            channel_key(draft.channel),
            inputs.client.display_name(),
        )
        .await?;
    let source = match decision {
        PolicyDecision::Allow => "ready",
        _ => "draft",
    };
    state
        .store
        .add_message(
            thread_id,
            Actor::KindlyAgent,
            "Kindly",
            Some(&draft.subject),
            &draft.body,
            None,
            Some(source),
        )
        .await
}

fn channel_key(channel: Channel) -> &'static str {
    match channel {
        Channel::Portal => "portal",
        Channel::PayPalInvoiceEmail => "paypal_email",
        Channel::AgentToAgent => "agent_to_agent",
    }
}

// ─────────────────────────────── execution ───────────────────────────────

pub async fn execute_action(state: &AppState, action_id: ActionId) -> ApiResult<()> {
    let mut action = state.store.action(action_id).await?;
    let ledger = Ledger::new(&state.store, Some(&state.events));

    action.status = ActionStatus::Executing;
    state.store.save_action(&action).await?;

    let result = match action.proposal.clone() {
        ProposedAction::SendNudge { channel, .. } => send_nudge(state, &action, channel).await,
        ProposedAction::OfferTerms { terms, channel } => {
            offer_terms(state, &action, &terms, channel).await
        }
        ProposedAction::IssuePlan { terms } => issue_plan(state, &action, &terms).await,
        ProposedAction::RecordPartialPayment { amount } => {
            record_payment_with_paypal(state, &action, amount).await
        }
        ProposedAction::GrantDiscount { amount, reason: _ } => {
            // A discount is only ever granted as part of agreed terms; on its
            // own it is recorded and applied to the negotiation as a proposed
            // concession.
            ledger
                .note(
                    Actor::KindlyAgent,
                    LedgerKind::NegotiationEvent,
                    format!(
                        "Discount of {} prepared for the next terms offer.",
                        amount.display_with_symbol()
                    ),
                    Some(action.invoice_id),
                )
                .await?;
            Ok(())
        }
        ProposedAction::Escalate { reason } => escalate(state, &action, &reason).await,
        ProposedAction::Pause { days, reason } => {
            let mut negotiation = state.store.negotiation(action.invoice_id).await?;
            let effects = nego::apply(
                &mut negotiation,
                NegoEvent::Paused {
                    at: Utc::now(),
                    days,
                    reason: reason.clone(),
                },
                &state.store.active_mandate().await?,
            )?;
            state.store.save_negotiation(&negotiation).await?;
            for effect in effects {
                apply_effect(state, &action.invoice_id, effect).await?;
            }
            Ok(())
        }
        ProposedAction::StartAgentNegotiation { terms } => {
            crate::a2a::start_session(state, action.invoice_id, terms.clone(), action.id).await
        }
        ProposedAction::CloseUncollectible { reason } => {
            let mut negotiation = state.store.negotiation(action.invoice_id).await?;
            let effects = nego::apply(
                &mut negotiation,
                NegoEvent::Closed {
                    at: Utc::now(),
                    outcome: CloseOutcome::Uncollectible,
                },
                &state.store.active_mandate().await?,
            )?;
            state.store.save_negotiation(&negotiation).await?;
            for effect in effects {
                apply_effect(state, &action.invoice_id, effect).await?;
            }
            ledger
                .note(
                    Actor::Owner,
                    LedgerKind::NegotiationEvent,
                    format!("Closed as uncollectible: {reason}"),
                    Some(action.invoice_id),
                )
                .await?;
            Ok(())
        }
    };

    match result {
        Ok(()) => {
            action.status = ActionStatus::Executed;
            action.executed_at = Some(Utc::now());
            state.store.save_action(&action).await?;
            state.events.publish(
                "agent_action",
                json!({
                    "action_id": action.id,
                    "invoice_id": action.invoice_id,
                    "kind": action.kind,
                    "status": action.status,
                    "label": action.kind.label(),
                    "rationale": action.rationale,
                }),
            );
            ledger
                .note(
                    Actor::KindlyAgent,
                    LedgerKind::ActionExecuted,
                    format!("{} — done", action.kind.label()),
                    Some(action.invoice_id),
                )
                .await?;
            Ok(())
        }
        Err(error) => {
            action.status = ActionStatus::Failed;
            action.error = Some(error.to_string());
            state.store.save_action(&action).await?;
            Err(error)
        }
    }
}

async fn send_nudge(state: &AppState, action: &AgentAction, channel: Channel) -> ApiResult<()> {
    let invoice = state.store.invoice(action.invoice_id).await?;
    let client = state.store.client(invoice.client_id).await?;
    let thread_id = state
        .store
        .ensure_thread(
            action.invoice_id,
            channel_key(channel),
            client.display_name(),
        )
        .await?;

    // The draft stored at proposal time is the message the owner saw.
    let mut subject = None;
    let mut body = None;
    if let Some(message_id) = state.store.action_draft_message(action.id).await? {
        let messages = state.store.messages(thread_id).await?;
        if let Some(message) = messages.iter().find(|m| m.id == message_id) {
            subject = message.subject.clone();
            body = Some(message.body.clone());
        }
        state.store.set_message_source(message_id, "sent").await?;
    }

    if body.is_none() {
        // Re-compose if the draft is missing (e.g. after an approval).
        let facts = compose_facts(state, &invoice, &client, None).await?;
        let tone = kindly_core::mandate::Tone::for_contact(
            state
                .store
                .negotiation(action.invoice_id)
                .await?
                .contact_count
                + 1,
            &state.store.active_mandate().await?.tone_ladder,
        );
        let composed =
            compose_message(state.llm.as_ref(), EffectPurpose::FollowUp, tone, &facts).await;
        subject = Some(composed.subject.clone());
        body = Some(composed.body.clone());
        state
            .store
            .add_message(
                thread_id,
                Actor::KindlyAgent,
                "Kindly",
                Some(&composed.subject),
                &composed.body,
                None,
                Some("sent"),
            )
            .await?;
    }

    let body = body.unwrap_or_default();
    let subject = subject.unwrap_or_else(|| format!("Invoice {}", invoice.number));
    let had_draft = !body.is_empty();

    // Real client-facing delivery through PayPal's own reminder email, when
    // the invoice has a PayPal mirror. Best effort by design (2/day cap).
    let paypal_sent = if matches!(channel, Channel::PayPalInvoiceEmail | Channel::Portal) {
        paypal_reminder(state, &invoice).await?
    } else {
        false
    };

    // When the message was already drafted (the normal path), it has been
    // marked sent and must not be duplicated here.
    if !had_draft {
        state
            .store
            .add_message(
                thread_id,
                Actor::KindlyAgent,
                "Kindly",
                Some(&subject),
                &format!(
                    "{body}\n\n— {}{}",
                    state.config.owner_name,
                    if paypal_sent {
                        "\n\n(Sent with a PayPal payment link.)"
                    } else {
                        ""
                    }
                ),
                None,
                Some("sent"),
            )
            .await?;
    } else if paypal_sent {
        // Note the PayPal delivery on the existing letter's thread.
        state
            .store
            .add_message(
                thread_id,
                Actor::System,
                "Kindly system",
                None,
                "Delivered with a PayPal payment link.",
                None,
                Some("note"),
            )
            .await?;
    }

    // Advance the negotiation state machine.
    let mut negotiation = state.store.negotiation(action.invoice_id).await?;
    let mandate = state.store.active_mandate().await?;
    let tone = kindly_core::mandate::Tone::for_contact(
        negotiation.contact_count + 1,
        &mandate.tone_ladder,
    );
    let effects = nego::apply(
        &mut negotiation,
        NegoEvent::NudgeSent {
            at: Utc::now(),
            tone,
            channel,
        },
        &mandate,
    )?;
    state.store.save_negotiation(&negotiation).await?;
    for effect in effects {
        apply_effect(state, &action.invoice_id, effect).await?;
    }

    state.events.publish(
        "client_message",
        json!({
            "invoice_id": action.invoice_id,
            "subject": subject,
            "delivered_via_paypal": paypal_sent,
        }),
    );
    Ok(())
}

async fn offer_terms(
    state: &AppState,
    action: &AgentAction,
    terms: &Terms,
    channel: Channel,
) -> ApiResult<()> {
    let invoice = state.store.invoice(action.invoice_id).await?;
    let client = state.store.client(invoice.client_id).await?;
    let thread_id = state
        .store
        .ensure_thread(
            action.invoice_id,
            channel_key(channel),
            client.display_name(),
        )
        .await?;

    let summary = kindly_agent::planner::summarise_terms(terms, invoice.currency.code());

    let body = if let Some(message_id) = state.store.action_draft_message(action.id).await? {
        let messages = state.store.messages(thread_id).await?;
        let text = messages
            .iter()
            .find(|m| m.id == message_id)
            .map(|m| m.body.clone());
        state.store.set_message_source(message_id, "sent").await?;
        text
    } else {
        None
    };

    let had_draft = body.is_some();
    let body = body.unwrap_or_else(|| {
        format!("Here are the terms we discussed: {summary}. You can pay the first part here.")
    });
    let subject = format!("Invoice {} — plan: {}", invoice.number, summary);

    if !had_draft {
        state
            .store
            .add_message(
                thread_id,
                Actor::KindlyAgent,
                "Kindly",
                Some(&subject),
                &format!("{body}\n\n— {}", state.config.owner_name),
                None,
                Some("sent"),
            )
            .await?;
    }

    let mut negotiation = state.store.negotiation(action.invoice_id).await?;
    let mandate = state.store.active_mandate().await?;
    let effects = nego::apply(
        &mut negotiation,
        NegoEvent::TermsProposed {
            at: Utc::now(),
            terms: terms.clone(),
        },
        &mandate,
    )?;
    state.store.save_negotiation(&negotiation).await?;
    for effect in effects {
        apply_effect(state, &action.invoice_id, effect).await?;
    }
    Ok(())
}

/// Issue the agreed plan: one PayPal invoice per instalment (after the
/// up-front payment, if any).
async fn issue_plan(state: &AppState, action: &AgentAction, terms: &Terms) -> ApiResult<()> {
    issue_plan_invoices(state, action.invoice_id, terms).await
}

pub async fn issue_plan_invoices(
    state: &AppState,
    invoice_id: InvoiceId,
    terms: &Terms,
) -> ApiResult<()> {
    let mut invoice = state.store.invoice(invoice_id).await?;
    let client = state.store.client(invoice.client_id).await?;
    let plan_id = PlanId::new();

    let mut issued = Vec::new();
    for (index, installment) in terms.installments.iter().enumerate() {
        let number = format!("{}-P{}", invoice.number, index + 1);
        let mut plan_invoice = Invoice::new(
            invoice.client_id,
            number.clone(),
            invoice.currency,
            installment.due,
            vec![kindly_core::invoice::InvoiceLine::new(
                format!(
                    "Instalment {} of payment plan for invoice {}",
                    index + 1,
                    invoice.number
                ),
                kindly_core::invoice::Quantity::whole(1).map_err(ApiError::Domain)?,
                installment.amount,
            )],
            Utc::now(),
        )?;
        plan_invoice.memo = Some(format!("Instalment plan for invoice {}", invoice.number));
        plan_invoice.parent_invoice_id = Some(invoice.id);
        push_invoice_to_paypal(state, &mut plan_invoice, &client, invoice.memo.as_deref()).await?;
        let _ = send_invoice_via_paypal(
            state,
            &mut plan_invoice,
            &format!("Instalment {} for invoice {}", index + 1, invoice.number),
            "Part of the payment plan we agreed.",
        )
        .await;
        issued.push(plan_invoice);
    }

    state
        .store
        .save_plan(plan_id, invoice.id, "issued", terms)
        .await?;

    // The plan is agreed from the negotiation's point of view.
    let mut negotiation = state.store.negotiation(invoice_id).await?;
    let mandate = state.store.active_mandate().await?;
    let effects = nego::apply(
        &mut negotiation,
        NegoEvent::PlanIssued {
            at: Utc::now(),
            plan_id,
        },
        &mandate,
    )?;
    state.store.save_negotiation(&negotiation).await?;
    for effect in effects {
        apply_effect(state, &invoice_id, effect).await?;
    }

    if let Some(first) = issued.first() {
        invoice.updated_at = Utc::now();
        state.store.upsert_invoice(&invoice, None).await?;
        Ledger::new(&state.store, Some(&state.events))
            .note(
                Actor::KindlyAgent,
                LedgerKind::PlanIssued,
                format!(
                    "Issued {} PayPal invoice(s) for the agreed plan; first instalment {} due {}",
                    issued.len(),
                    first.total.display_with_symbol(),
                    first.due_date.format("%-d %B")
                ),
                Some(invoice.id),
            )
            .await?;
    }

    state.events.publish(
        "plan_issued",
        json!({
            "invoice_id": invoice_id,
            "plan_id": plan_id,
            "installments": issued.len(),
        }),
    );
    Ok(())
}

async fn record_payment_with_paypal(
    state: &AppState,
    action: &AgentAction,
    amount: Money,
) -> ApiResult<()> {
    let invoice = state.store.invoice(action.invoice_id).await?;
    if let Some(paypal_id) = &invoice.paypal_invoice_id {
        state
            .paypal
            .record_payment(
                paypal_id,
                amount.minor,
                invoice.currency.code(),
                "OTHER",
                "Recorded by Kindly after the client paid outside PayPal.",
            )
            .await?;
    }
    apply_payment(
        state,
        invoice.id,
        amount,
        "recorded",
        None,
        None,
        ActionKind::RecordPartialPayment,
    )
    .await?;
    Ok(())
}

async fn escalate(state: &AppState, action: &AgentAction, reason: &str) -> ApiResult<()> {
    let mut negotiation = state.store.negotiation(action.invoice_id).await?;
    let mandate = state.store.active_mandate().await?;
    let effects = nego::apply(
        &mut negotiation,
        NegoEvent::HumanRequested {
            at: Utc::now(),
            reason: reason.to_string(),
        },
        &mandate,
    )?;
    state.store.save_negotiation(&negotiation).await?;
    for effect in effects {
        apply_effect(state, &action.invoice_id, effect).await?;
    }
    Ok(())
}

pub async fn apply_effect(
    state: &AppState,
    invoice_id: &InvoiceId,
    effect: NegoEffect,
) -> ApiResult<()> {
    let ledger = Ledger::new(&state.store, Some(&state.events));
    match effect {
        NegoEffect::NotifyOwner { summary, security } => {
            ledger
                .note(
                    if security {
                        Actor::System
                    } else {
                        Actor::KindlyAgent
                    },
                    if security {
                        LedgerKind::SecurityEvent
                    } else {
                        LedgerKind::NegotiationEvent
                    },
                    summary,
                    Some(*invoice_id),
                )
                .await?;
            state.events.publish(
                "owner_notice",
                json!({ "invoice_id": invoice_id, "security": security }),
            );
        }
        NegoEffect::RequestApproval { reason } => {
            ledger
                .note(
                    Actor::KindlyAgent,
                    LedgerKind::PolicyApprovalRequested,
                    reason,
                    Some(*invoice_id),
                )
                .await?;
        }
        NegoEffect::ScheduleFollowUp { .. } => {
            // The scheduler does this work; nothing to write.
        }
        NegoEffect::CreatePlanInvoices { terms } => {
            ledger
                .note(
                    Actor::KindlyAgent,
                    LedgerKind::PlanIssued,
                    format!(
                        "Plan accepted: {} now, {} instalment(s).",
                        terms.pay_now.display_with_symbol(),
                        terms.installment_count()
                    ),
                    Some(*invoice_id),
                )
                .await?;

            // Tell the client what was agreed, where they will look for it.
            let invoice = state.store.invoice(*invoice_id).await?;
            let client = state.store.client(invoice.client_id).await?;
            let thread_id = state
                .store
                .ensure_thread(*invoice_id, "portal", client.display_name())
                .await?;
            let summary = kindly_agent::planner::summarise_terms(&terms, invoice.currency.code());
            state
                .store
                .add_message(
                    thread_id,
                    Actor::KindlyAgent,
                    "Kindly",
                    Some(&format!("Invoice {} — plan agreed", invoice.number)),
                    &format!(
                        "As agreed: {summary}. You can pay each part from its own link, and the first one from this page.\n\n— {}",
                        state.config.owner_name
                    ),
                    None,
                    Some("sent"),
                )
                .await?;
        }
        NegoEffect::Stop { reason } => {
            ledger
                .note(
                    Actor::System,
                    LedgerKind::NegotiationEvent,
                    format!("Stopped: {reason}"),
                    Some(*invoice_id),
                )
                .await?;
        }
        NegoEffect::SendMessage {
            channel,
            tone,
            purpose,
        } => {
            let invoice = state.store.invoice(*invoice_id).await?;
            let client = state.store.client(invoice.client_id).await?;
            let thread_id = state
                .store
                .ensure_thread(*invoice_id, channel_key(channel), client.display_name())
                .await?;
            let facts = compose_facts(state, &invoice, &client, None).await?;
            let composed = compose_message(state.llm.as_ref(), purpose, tone, &facts).await;
            state
                .store
                .add_message(
                    thread_id,
                    Actor::KindlyAgent,
                    "Kindly",
                    Some(&composed.subject),
                    &format!("{}\n\n— {}", composed.body, state.config.owner_name),
                    None,
                    Some("sent"),
                )
                .await?;
            state.events.publish(
                "client_message",
                json!({ "invoice_id": invoice_id, "subject": composed.subject }),
            );
        }
    }
    Ok(())
}

// ─────────────────────────────── approvals ───────────────────────────────

pub async fn approve_action(
    state: &AppState,
    action_id: ActionId,
    approved: bool,
) -> ApiResult<()> {
    let action = state.store.action(action_id).await?;
    if action.status != ActionStatus::AwaitingApproval {
        return Err(ApiError::Conflict(format!(
            "action is {} and cannot be decided",
            action.status.label()
        )));
    }
    let ledger = Ledger::new(&state.store, Some(&state.events));
    if approved {
        ledger
            .note(
                Actor::Owner,
                LedgerKind::PolicyApprovalGranted,
                format!("You approved: {}", action.kind.label()),
                Some(action.invoice_id),
            )
            .await?;
        // Mark the draft as ready, then execute.
        if let Some(message_id) = state.store.action_draft_message(action.id).await? {
            state.store.set_message_source(message_id, "ready").await?;
        }
        execute_action(state, action_id).await?;
    } else {
        let mut action = action;
        action.status = ActionStatus::Rejected;
        action.executed_at = Some(Utc::now());
        state.store.save_action(&action).await?;
        ledger
            .note(
                Actor::Owner,
                LedgerKind::PolicyApprovalRejected,
                format!("You rejected: {}", action.kind.label()),
                Some(action.invoice_id),
            )
            .await?;
    }
    Ok(())
}

// ─────────────────────────────── payments ───────────────────────────────

/// Record money that arrived, advance the negotiation, and attribute cause.
pub async fn apply_payment(
    state: &AppState,
    invoice_id: InvoiceId,
    amount: Money,
    kind: &str,
    provider_ref: Option<&str>,
    raw: Option<&serde_json::Value>,
    _trigger: ActionKind,
) -> ApiResult<()> {
    let mut invoice = state.store.invoice(invoice_id).await?;
    let applied = invoice.apply_payment(amount, Utc::now())?;
    let fully_settles = invoice.outstanding().is_zero();
    state.store.upsert_invoice(&invoice, None).await?;
    state
        .store
        .add_payment(invoice_id, provider_ref, applied, kind, raw)
        .await?;

    let ledger = Ledger::new(&state.store, Some(&state.events));
    ledger
        .write(
            LedgerEvent::new(
                Utc::now(),
                Actor::PayPal,
                if fully_settles {
                    LedgerKind::InvoicePaid
                } else {
                    LedgerKind::InvoicePartiallyPaid
                },
                if fully_settles {
                    format!(
                        "{} settled — {} received",
                        invoice.number,
                        applied.display_with_symbol()
                    )
                } else {
                    format!(
                        "{} — {} received, {} still outstanding",
                        invoice.number,
                        applied.display_with_symbol(),
                        invoice.outstanding().display_with_symbol()
                    )
                },
            )
            .with_invoice(invoice_id)
            .with_client(invoice.client_id)
            .with_detail(json!({
                "amount": applied.paypal_value(),
                "currency": applied.currency.code(),
                "kind": kind,
                "provider_ref": provider_ref,
                "fully_settles": fully_settles,
            })),
        )
        .await?;

    // Advance the negotiation.
    let mut negotiation = state.store.negotiation(invoice_id).await?;
    let mandate = state.store.active_mandate().await?;
    let effects = nego::apply(
        &mut negotiation,
        NegoEvent::PaymentReceived {
            at: Utc::now(),
            amount: applied,
            fully_settles,
        },
        &mandate,
    )?;
    state.store.save_negotiation(&negotiation).await?;
    for effect in effects {
        apply_effect(state, &invoice_id, effect).await?;
    }

    // Client behaviour history.
    let mut client = state.store.client(invoice.client_id).await?;
    let today = Utc::now().date_naive();
    let days_late = (today - invoice.due_date).num_days().max(0) as i32;
    if days_late > 0 {
        client.behavior.record_late_payment(days_late, Utc::now());
    } else {
        client.behavior.record_on_time_payment(Utc::now());
    }
    client.behavior.typical_pay_weekday = Some(today.weekday_key());
    state
        .store
        .upsert_client(&client, state.store.client_has_ap_agent(client.id).await?)
        .await?;

    // Attribution for settled invoices.
    if fully_settles {
        let actions = state.store.actions_for_invoice(invoice_id).await?;
        let action_records = actions
            .iter()
            .filter(|a| a.status == ActionStatus::Executed)
            .filter_map(|a| {
                a.executed_at.map(|at| attribution::ActionRecord {
                    id: a.id,
                    kind: a.kind,
                    at,
                })
            })
            .collect::<Vec<_>>();
        let payments = state.store.payments(invoice_id).await?;
        let baseline = Some(client.behavior.avg_days_late as i64 + 30);
        let attribution =
            attribution::attribute(&invoice, &action_records, &payments, baseline, Utc::now());
        state.store.save_attribution(&attribution).await?;
        ledger
            .note(
                Actor::System,
                LedgerKind::System,
                format!("Attribution: {}", attribution.rationale),
                Some(invoice_id),
            )
            .await?;
        state.events.publish(
            "invoice_paid",
            json!({
                "invoice_id": invoice_id,
                "invoice_number": invoice.number,
                "amount": applied.paypal_value(),
                "currency": applied.currency.code(),
                "attribution": attribution.kind,
                "days_saved": attribution.days_saved,
            }),
        );
    } else {
        state.events.publish(
            "invoice_partially_paid",
            json!({
                "invoice_id": invoice_id,
                "amount": applied.paypal_value(),
                "outstanding": invoice.outstanding().paypal_value(),
            }),
        );
    }

    info!(invoice = %invoice.number, amount = %applied, "payment applied");
    Ok(())
}

trait WeekdayKey {
    fn weekday_key(&self) -> u8;
}

impl WeekdayKey for chrono::NaiveDate {
    fn weekday_key(&self) -> u8 {
        use chrono::Datelike;
        self.weekday().num_days_from_monday() as u8
    }
}

// ─────────────────────────────── client messages ───────────────────────────────

/// A client wrote something. Understand it (untrusted input!), record it,
/// advance the negotiation, and give the agent its turn.
pub async fn handle_client_message(
    state: &AppState,
    invoice_id: InvoiceId,
    body: &str,
) -> ApiResult<(ReplyIntent, Vec<TickReport>)> {
    let invoice = state.store.invoice(invoice_id).await?;
    let client = state.store.client(invoice.client_id).await?;
    let thread_id = state
        .store
        .ensure_thread(invoice_id, "portal", client.display_name())
        .await?;

    let reading = read_reply(
        state.llm.as_ref(),
        body,
        invoice.currency,
        Utc::now().date_naive(),
    )
    .await;

    state
        .store
        .add_message(
            thread_id,
            Actor::Client,
            client.display_name(),
            None,
            body,
            Some(&serde_json::to_string(&reading.intent)?),
            Some("sent"),
        )
        .await?;

    let ledger = Ledger::new(&state.store, Some(&state.events));
    ledger
        .note(
            Actor::Client,
            LedgerKind::MessageReceived,
            format!(
                "{} wrote: “{}” (read as {})",
                client.display_name(),
                truncate(body, 120),
                intent_label(&reading.intent)
            ),
            Some(invoice_id),
        )
        .await?;

    if reading.injection.suspicious {
        ledger
            .write(
                LedgerEvent::new(
                    Utc::now(),
                    Actor::System,
                    LedgerKind::SecurityEvent,
                    format!(
                        "Refused an instruction aimed at the system (matched: {}). The invoice was not changed.",
                        reading.injection.matched.join(", ")
                    ),
                )
                .with_invoice(invoice_id)
                .with_detail(json!({
                    "patterns": reading.injection.matched,
                    "reading_source": format!("{:?}", reading.source),
                })),
            )
            .await?;
        state.events.publish(
            "security_event",
            json!({
                "invoice_id": invoice_id,
                "patterns": reading.injection.matched,
                "excerpt": truncate(body, 160),
            }),
        );
    }

    // Advance the negotiation state machine.
    let mut negotiation = state.store.negotiation(invoice_id).await?;
    let mandate = state.store.active_mandate().await?;
    let effects = nego::apply(
        &mut negotiation,
        NegoEvent::ReplyReceived {
            at: Utc::now(),
            intent: reading.intent.clone(),
        },
        &mandate,
    )?;
    state.store.save_negotiation(&negotiation).await?;
    for effect in effects {
        apply_effect(state, &invoice_id, effect).await?;
    }

    state.events.publish(
        "client_reply",
        json!({
            "invoice_id": invoice_id,
            "intent": intent_label(&reading.intent),
            "source": format!("{:?}", reading.source),
        }),
    );

    // The agent answers while the client is still looking at the page.
    let report = tick_invoice(state, invoice_id).await?;
    Ok((reading.intent, vec![report]))
}

pub fn intent_label(intent: &ReplyIntent) -> &'static str {
    match intent {
        ReplyIntent::WillPaySoon { .. } => "promises a payment date",
        ReplyIntent::RequestsPlan { .. } => "asks for a payment plan",
        ReplyIntent::RequestsDiscount { .. } => "asks for a discount",
        ReplyIntent::DisputesInvoice { .. } => "disputes the invoice",
        ReplyIntent::RequestsDocuments { .. } => "asks for documents",
        ReplyIntent::ClaimsPaid { .. } => "says it is already paid",
        ReplyIntent::Unrelated { .. } => "writes about something else",
        ReplyIntent::InstructionAttempt { .. } => "tries to instruct the system",
    }
}

fn truncate(input: &str, max: usize) -> String {
    let trimmed = input.trim();
    if trimmed.chars().count() <= max {
        return trimmed.to_string();
    }
    let mut out: String = trimmed.chars().take(max).collect();
    out.push('…');
    out
}

// ─────────────────────────────── compose facts ───────────────────────────────

pub async fn compose_facts(
    state: &AppState,
    invoice: &Invoice,
    client: &Client,
    terms: Option<&Terms>,
) -> ApiResult<ComposeFacts> {
    let today = Utc::now().date_naive();
    let thread_id = state.store.thread_for_invoice(invoice.id).await?;
    let history = match thread_id {
        Some(thread_id) => state
            .store
            .messages(thread_id)
            .await?
            .into_iter()
            .filter(|m| m.source.as_deref() == Some("sent"))
            .map(|m| (m.author_label, m.body))
            .collect(),
        None => Vec::new(),
    };

    Ok(ComposeFacts {
        owner_name: state.config.owner_name.clone(),
        client_contact: client.name.clone(),
        client_company: client.display_name().to_string(),
        invoice_number: invoice.number.clone(),
        currency: invoice.currency.code().to_string(),
        amount_outstanding: Some(invoice.outstanding()),
        total: Some(invoice.total),
        due_date: Some(invoice.due_date),
        days_overdue: invoice.days_overdue(today),
        work_summary: state.store.invoice_work_summary(invoice.id).await?,
        pay_link: format!(
            "{}/c/{}",
            state.config.public_url.trim_end_matches('/'),
            crate::portal::sign_portal_token(
                &state.config.portal_secret,
                invoice.id,
                crate::portal::PORTAL_TOKEN_TTL_DAYS
            )
        ),
        plan_summary: terms
            .map(|t| kindly_agent::planner::summarise_terms(t, invoice.currency.code())),
        promised_on: None,
        history,
        language_hint: None,
    })
}
