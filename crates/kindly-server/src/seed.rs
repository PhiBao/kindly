//! Demo dataset.
//!
//! Seeds a small, believable studio: four clients, four invoices at different
//! stages of trouble, and the history that makes their profiles meaningful.
//! Everything created here is created through the same code paths as the live
//! product (including the PayPal calls), so the demo exercises the real
//! pipeline rather than a parallel one.

use crate::engine::{self, Ledger};
use crate::errors::ApiResult;
use crate::state::AppState;
use chrono::{Duration, Utc};
use kindly_core::action::{ActionKind, ActionStatus, AgentAction};
use kindly_core::client::{Client, PaymentBehavior};
use kindly_core::ids::{Actor, ActionId, InvoiceId};
use kindly_core::invoice::{Invoice, InvoiceLine, InvoiceStatus, Quantity};
use kindly_core::ledger::{LedgerEvent, LedgerKind};
use kindly_core::mandate::{AutonomyLevel, CollectionMandate, ContactWindow, Tone};
use kindly_core::money::{Currency, Money};
use kindly_core::negotiation::{self as nego, NegoEvent, ReplyIntent, Stage};
use kindly_core::policy::{PolicyDecision, ProposedAction};
use serde_json::json;

#[derive(Debug, serde::Serialize)]
pub struct SeedSummary {
    pub clients: usize,
    pub invoices: usize,
    pub already_present: bool,
}

pub async fn seed_demo(state: &AppState, reset: bool) -> ApiResult<SeedSummary> {
    if reset {
        state.store.clear_all().await?;
    }
    if state.store.count_rows("clients").await? > 0 {
        return Ok(SeedSummary {
            clients: state.store.count_rows("clients").await? as usize,
            invoices: state.store.count_rows("invoices").await? as usize,
            already_present: true,
        });
    }

    let now = Utc::now();
    let today = now.date_naive();

    // ── mandate ──────────────────────────────────────────────────────
    // Demo-friendly defaults: the contact window is wide so the demo works at
    // any hour, and the "strict hours" preset in the UI shows quiet hours
    // working when a judge wants to see it.
    let mut mandate = CollectionMandate::default_for(Currency::Usd, now);
    mandate.autonomy = AutonomyLevel::ActWithinLimits;
    mandate.contact_window = ContactWindow {
        start_hour: 0,
        end_hour: 24,
        weekdays_only: false,
        utc_offset_minutes: 0,
    };
    mandate.max_contacts_per_week = 2;
    mandate.min_days_between_contacts = 2;
    mandate.min_upfront_settlement_bps = 7_000;
    mandate.max_discount_bps = 1_000;
    mandate.max_discount_without_approval_bps = 0;
    mandate.max_installments = 3;
    mandate.max_plan_days = 45;
    mandate.min_installment_amount = Money::new(5_000, Currency::Usd);
    mandate.escalate_after_contacts = 4;
    mandate.stop_after_days_silent = 90;
    mandate.allow_agent_to_agent = true;
    mandate.tone_ladder = vec![Tone::Warm, Tone::Warm, Tone::Neutral, Tone::Firm, Tone::Formal];
    state.store.save_mandate(&mandate).await?;

    let ledger = Ledger::new(&state.store, Some(&state.events));
    ledger
        .note(
            Actor::Owner,
            LedgerKind::MandateUpdated,
            format!(
                "Your mandate is set: {} — up to {} contacts a week, never less than {}% up front.",
                mandate.autonomy.label(),
                mandate.max_contacts_per_week,
                mandate.min_upfront_settlement_bps / 100
            ),
            None,
        )
        .await?;

    // ── clients ──────────────────────────────────────────────────────
    let acme = Client {
        behavior: PaymentBehavior {
            avg_days_late: 14,
            typical_pay_weekday: Some(4),
            invoices_paid: 4,
            invoices_late: 3,
            last_paid_at: Some(now - Duration::days(48)),
        },
        ..Client::new("Sam Chen", "ap@acmestudio.test", now)
    };
    let mut acme = acme;
    acme.company = Some("Acme Studio".into());
    acme.notes = Some("Prefers one email thread. Pays in Friday runs.".into());

    let mut northwind = Client::new("Priya Patel", "finance@northwindcoffee.test", now);
    northwind.company = Some("Northwind Coffee".into());
    northwind.behavior = PaymentBehavior {
        avg_days_late: 6,
        typical_pay_weekday: Some(2),
        invoices_paid: 6,
        invoices_late: 2,
        last_paid_at: Some(now - Duration::days(70)),
    };

    let mut brightline = Client::new("Tom Alvarez", "accounts@brightlinemedia.test", now);
    brightline.company = Some("Brightline Media".into());
    brightline.behavior = PaymentBehavior {
        avg_days_late: 32,
        typical_pay_weekday: None,
        invoices_paid: 2,
        invoices_late: 2,
        last_paid_at: Some(now - Duration::days(210)),
    };
    brightline.notes = Some("Agency; their client pays them first — cash cycles are slow.".into());

    let mut kestrel = Client::new("Dana Okafor", "dana@kestrellabs.test", now);
    kestrel.company = Some("Kestrel Labs".into());
    kestrel.behavior = PaymentBehavior {
        avg_days_late: 0,
        typical_pay_weekday: Some(0),
        invoices_paid: 5,
        invoices_late: 0,
        last_paid_at: Some(now - Duration::days(30)),
    };

    for (client, has_ap_agent) in [
        (&acme, true),
        (&northwind, false),
        (&brightline, false),
        (&kestrel, false),
    ] {
        state.store.upsert_client(client, has_ap_agent).await?;
    }

    // ── invoices ─────────────────────────────────────────────────────
    // 1. Acme — the agent-to-agent story.
    let (acme_invoice, acme_invoice_work) = make_invoice(
        &acme,
        "1042",
        240_000,
        today - Duration::days(11),
        vec![(
            "Brand identity sprint — logo suite, type system, 40-page brand book",
            1_000,
            240_000,
        )],
        Some("Brand identity sprint"),
        now,
    )?;
    // 2. Northwind — the human conversation story.
    let (northwind_invoice, northwind_invoice_work) = make_invoice(
        &northwind,
        "1039",
        115_000,
        today - Duration::days(34),
        vec![("Website copy refresh — 12 pages, two rounds of edits", 1_000, 115_000)],
        Some("Website copy refresh"),
        now,
    )?;
    // 3. Brightline — the escalation story.
    let (brightline_invoice, brightline_invoice_work) = make_invoice(
        &brightline,
        "1027",
        480_000,
        today - Duration::days(96),
        vec![("Campaign shoot — 2 days on location, 60 retouched frames", 1_000, 480_000)],
        Some("Campaign shoot"),
        now,
    )?;
    // 4. Kestrel — an invoice that is not late yet.
    let (kestrel_invoice, kestrel_invoice_work) = make_invoice(
        &kestrel,
        "1051",
        90_000,
        today + Duration::days(3),
        vec![("October design retainer", 1_000, 90_000)],
        Some("October retainer"),
        now,
    )?;

    for (invoice, work, client) in [
        (&acme_invoice, &acme_invoice_work, &acme),
        (&northwind_invoice, &northwind_invoice_work, &northwind),
        (&brightline_invoice, &brightline_invoice_work, &brightline),
        (&kestrel_invoice, &kestrel_invoice_work, &kestrel),
    ] {
        let mut invoice = invoice.clone();
        engine::push_invoice_to_paypal(state, &mut invoice, client, work.as_deref()).await?;
        let subject = format!("Invoice {} from {}", invoice.number, state.config.owner_business);
        engine::send_invoice_via_paypal(
            state,
            &mut invoice,
            &subject,
            "Thanks — the work is delivered and this is ready when you are.",
        )
        .await?;
        ledger
            .note(
                Actor::Owner,
                LedgerKind::InvoiceSent,
                format!(
                    "Sent invoice {} to {} — {} due {}",
                    invoice.number,
                    client.display_name(),
                    invoice.total.display_with_symbol(),
                    invoice.due_date.format("%-d %B")
                ),
                Some(invoice.id),
            )
            .await?;
    }

    // ── history that makes the profiles real ──────────────────────────
    seed_contact_history(
        state,
        northwind_invoice.id,
        &northwind,
        2,
        18,
        Some(ReplyIntent::RequestsPlan {
            pay_now: Some(Money::new(57_500, Currency::Usd)),
            installments: Some(2),
        }),
    )
    .await?;
    seed_contact_history(state, brightline_invoice.id, &brightline, 5, 88, None).await?;
    seed_contact_history(state, acme_invoice.id, &acme, 0, 0, None).await?;

    // Brightline's negotiation is back to "ready" (they went quiet), so the
    // planner proposes escalation on the next pass.
    let mut negotiation = state.store.negotiation(brightline_invoice.id).await?;
    negotiation.stage = Stage::Idle;
    state.store.save_negotiation(&negotiation).await?;

    Ok(SeedSummary {
        clients: 4,
        invoices: 4,
        already_present: false,
    })
}

fn make_invoice(
    client: &Client,
    number: &str,
    total_minor: i64,
    due: chrono::NaiveDate,
    lines: Vec<(&str, i64, i64)>,
    work_summary: Option<&str>,
    now: chrono::DateTime<Utc>,
) -> ApiResult<(Invoice, Option<String>)> {
    let invoice_lines = lines
        .into_iter()
        .map(|(description, quantity_thousandths, unit_minor)| {
            Ok(InvoiceLine::new(
                description,
                Quantity::from_thousandths(quantity_thousandths).map_err(crate::errors::ApiError::Domain)?,
                Money::new(unit_minor, Currency::Usd),
            ))
        })
        .collect::<ApiResult<Vec<_>>>()?;
    let mut invoice = Invoice::new(client.id, number, Currency::Usd, due, invoice_lines, now)?;
    invoice.total = Money::new(total_minor, Currency::Usd);
    invoice.memo = work_summary.map(str::to_string);
    Ok((invoice, work_summary.map(str::to_string)))
}

/// Create believable past contact: executed nudges, delivered messages, and
/// (optionally) a client reply that is waiting for a plan.
async fn seed_contact_history(
    state: &AppState,
    invoice_id: InvoiceId,
    client: &Client,
    contacts: u8,
    days_ago: i64,
    reply: Option<ReplyIntent>,
) -> ApiResult<()> {
    if contacts == 0 && reply.is_none() {
        return Ok(());
    }
    let invoice = state.store.invoice(invoice_id).await?;
    let thread_id = state
        .store
        .ensure_thread(invoice_id, "portal", client.display_name())
        .await?;
    let mandate = state.store.active_mandate().await?;
    let ledger = Ledger::new(&state.store, Some(&state.events));

    for index in 0..contacts {
        let at = Utc::now() - Duration::days(days_ago - (index as i64 * 7));
        let tone = Tone::for_contact(index + 1, &mandate.tone_ladder);
        let first_name = client.name.split_whitespace().next().unwrap_or(&client.name);
        let (subject, body) = match index {
            0 => (
                format!("Invoice {} — quick check-in", invoice.number),
                format!(
                    "Hi {first_name},\n\nJust a quiet note that invoice {} for {} was due on {}. If it's already queued in your payment run, thank you — ignore me. The payment link is in this thread.\n\n— {}",
                    invoice.number,
                    invoice.outstanding().display_with_symbol(),
                    invoice.due_date.format("%-d %B"),
                    state.config.owner_name
                ),
            ),
            1 => (
                format!("Following up: invoice {}", invoice.number),
                format!(
                    "Hi {first_name},\n\nFollowing up on invoice {} ({}) — still open on my side. You can settle it from the link here. If something is holding it up, tell me and we'll sort it out.\n\n— {}",
                    invoice.number,
                    invoice.outstanding().display_with_symbol(),
                    state.config.owner_name
                ),
            ),
            _ => (
                format!(
                    "Invoice {} — {} days past due",
                    invoice.number,
                    invoice.days_overdue(Utc::now().date_naive())
                ),
                format!(
                    "Hi {first_name},\n\nInvoice {} ({}) is now past 30 days. I'd rather sort this between us than escalate it — could you confirm a payment date, or tell me what's blocking it?\n\n— {}",
                    invoice.number,
                    invoice.outstanding().display_with_symbol(),
                    state.config.owner_name
                ),
            ),
        };
        state
            .store
            .add_message_at(
                thread_id,
                Actor::KindlyAgent,
                "Kindly",
                Some(&subject),
                &body,
                None,
                Some("sent"),
                at,
            )
            .await?;

        let action = AgentAction {
            id: ActionId::new(),
            invoice_id,
            kind: ActionKind::Nudge,
            status: ActionStatus::Executed,
            proposal: ProposedAction::SendNudge {
                channel: kindly_core::mandate::Channel::Portal,
                tone,
            },
            decision: PolicyDecision::Allow,
            rationale: "Inside your limits: one polite reminder with a payment link.".into(),
            idempotency_key: format!("seed-{invoice_id}-nudge-{index}"),
            created_at: at,
            executed_at: Some(at),
            error: None,
        };
        state.store.save_action(&action).await?;

        let mut negotiation = state.store.negotiation(invoice_id).await?;
        let effects = nego::apply(
            &mut negotiation,
            NegoEvent::NudgeSent {
                at,
                tone,
                channel: kindly_core::mandate::Channel::Portal,
            },
            &mandate,
        )
        .map_err(crate::errors::ApiError::Domain)?;
        state.store.save_negotiation(&negotiation).await?;
        for effect in effects {
            engine::apply_effect(state, &invoice_id, effect).await?;
        }

        ledger
            .write(
                LedgerEvent::new(
                    at,
                    Actor::KindlyAgent,
                    LedgerKind::ActionExecuted,
                    format!(
                        "Sent {} reminder for {}",
                        tone.label().to_lowercase(),
                        invoice.number
                    ),
                )
                .with_invoice(invoice_id)
                .with_detail(json!({ "seeded": true })),
            )
            .await?;
    }

    if let Some(intent) = reply {
        let at = Utc::now() - Duration::days(2);
        let text = match &intent {
            ReplyIntent::RequestsPlan { .. } => {
                "Thanks for the nudge. Cash is tight this month — could we split it, say half now and the rest in a couple of weeks?"
            }
            _ => "Thanks, we'll sort it out shortly.",
        };
        state
            .store
            .add_message_at(
                thread_id,
                Actor::Client,
                client.display_name(),
                None,
                text,
                Some(&serde_json::to_string(&intent)?),
                Some("sent"),
                at,
            )
            .await?;

        let mut negotiation = state.store.negotiation(invoice_id).await?;
        // Ensure the machine is in a state where a reply is expected.
        if negotiation.stage == Stage::Idle {
            negotiation.stage = Stage::Contacted;
        }
        let effects = nego::apply(
            &mut negotiation,
            NegoEvent::ReplyReceived {
                at,
                intent: intent.clone(),
            },
            &mandate,
        )
        .map_err(crate::errors::ApiError::Domain)?;
        state.store.save_negotiation(&negotiation).await?;
        for effect in effects {
            engine::apply_effect(state, &invoice_id, effect).await?;
        }

        ledger
            .note(
                Actor::Client,
                LedgerKind::MessageReceived,
                format!(
                    "{} replied: “{}”",
                    client.display_name(),
                    text
                ),
                Some(invoice_id),
            )
            .await?;
        // The reply is older than the last contact, so make the follow-up
        // window sane for the planner.
        let mut negotiation = state.store.negotiation(invoice_id).await?;
        negotiation.last_event_at = at;
        negotiation.last_contact_at = Some(at - Duration::days(7));
        state.store.save_negotiation(&negotiation).await?;
    }

    // Make sure the invoice reads as overdue when it is.
    let mut invoice = state.store.invoice(invoice_id).await?;
    let today = Utc::now().date_naive();
    if invoice.due_date < today {
        invoice.status = InvoiceStatus::Overdue;
        state.store.upsert_invoice(&invoice, None).await?;
    }
    Ok(())
}
