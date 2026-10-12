//! Keeping Kindly and PayPal in agreement.
//!
//! Two paths lead to the same processing function:
//! * webhooks (real deliveries, signature verified before they get here);
//! * reconciliation (`show_invoice` / `list_invoices`), for when a delivery is
//!   lost — which happens.
//!
//! Both are idempotent: the same fact applied twice changes nothing.

use crate::engine::{apply_payment, Ledger};
use crate::errors::ApiResult;
use crate::state::AppState;
use chrono::Utc;
use kindly_core::ids::{Actor, InvoiceId};
use kindly_core::ledger::{LedgerEvent, LedgerKind};
use kindly_core::money::Currency;
use kindly_paypal::model::{PayPalInvoice, VerifiedEvent};
use kindly_paypal::webhook::{event_amount_minor, WebhookFact};
use serde_json::json;
use tracing::{info, warn};

/// Handle one verified PayPal event.
pub async fn process_event(state: &AppState, event: &VerifiedEvent) -> ApiResult<()> {
    let fact = WebhookFact::from_event_type(&event.event_type);
    let ledger = Ledger::new(&state.store, Some(&state.events));

    match fact {
        WebhookFact::InvoicePaid => {
            let Some(paypal_invoice_id) = event.paypal_invoice_id() else {
                return Ok(());
            };
            let Some(invoice) = state.store.invoice_by_paypal_id(&paypal_invoice_id).await? else {
                warn!(paypal_invoice_id, "invoice paid but not mirrored in Kindly");
                return Ok(());
            };

            // Work out how much arrived *since we last recorded something*.
            let (paid_total_minor, currency) = event_amount_minor(event).unwrap_or((
                invoice.amount_paid.minor,
                invoice.currency.code().to_string(),
            ));
            let already = state.store.payment_total_minor(invoice.id).await?;
            let delta = (paid_total_minor - already).max(0);
            let fully = paid_total_minor >= invoice.total.minor;
            let currency = Currency::from_code(&currency).unwrap_or(invoice.currency);

            ledger
                .write(
                    LedgerEvent::new(
                        Utc::now(),
                        Actor::PayPal,
                        LedgerKind::PayPalWebhook,
                        format!(
                            "PayPal webhook {} for invoice {} (paid total {})",
                            event.event_type,
                            invoice.number,
                            kindly_paypal::model::format_minor(paid_total_minor, currency.code())
                        ),
                    )
                    .with_invoice(invoice.id)
                    .with_detail(json!({
                        "event_id": event.event_id,
                        "event_type": event.event_type,
                        "verified": event.verified,
                    })),
                )
                .await?;

            if delta > 0 {
                let amount = kindly_core::money::Money::new(delta, currency);
                match invoice.parent_invoice_id {
                    Some(parent_id) => {
                        // An instalment was paid: close the instalment invoice
                        // and reduce the parent's balance.
                        apply_payment(
                            state,
                            invoice.id,
                            amount,
                            "settlement",
                            Some(event.event_id.as_str()),
                            Some(&event.raw),
                            kindly_core::action::ActionKind::RecordPartialPayment,
                        )
                        .await?;
                        apply_payment(
                            state,
                            parent_id,
                            amount,
                            if fully { "settlement" } else { "partial" },
                            Some(event.event_id.as_str()),
                            Some(&event.raw),
                            kindly_core::action::ActionKind::RecordPartialPayment,
                        )
                        .await?;
                    }
                    None => {
                        apply_payment(
                            state,
                            invoice.id,
                            amount,
                            if fully { "settlement" } else { "partial" },
                            Some(event.event_id.as_str()),
                            Some(&event.raw),
                            kindly_core::action::ActionKind::RecordPartialPayment,
                        )
                        .await?;
                    }
                }
            } else {
                // Nothing new (duplicate delivery, or a status change with the
                // same amount). Make sure our mirror says PAID.
                let mut mirror = invoice.clone();
                mirror.paypal_status = Some("PAID".to_string());
                if mirror.outstanding().is_zero() {
                    mirror.status = kindly_core::invoice::InvoiceStatus::Paid;
                }
                state.store.upsert_invoice(&mirror, None).await?;
            }

            // If this invoice was part of an agent-to-agent settlement, record it.
            settle_a2a_sessions(state, invoice.id).await?;
        }
        WebhookFact::PaymentCaptured => {
            // A capture carries the invoice it belongs to in `custom_id`
            // (`kindly:<invoice id>`), so a portal payment settles even if the
            // capture response never reached us.
            let custom_id = event
                .resource
                .get("custom_id")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string();
            let capture_id = event
                .resource
                .get("id")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string();

            ledger
                .note(
                    Actor::PayPal,
                    LedgerKind::PayPalWebhook,
                    format!(
                        "Capture {} completed{}",
                        if capture_id.is_empty() { "(no id)" } else { &capture_id },
                        event
                            .order_id()
                            .map(|order| format!(" for order {order}"))
                            .unwrap_or_default()
                    ),
                    None,
                )
                .await?;

            let Some(raw_invoice_id) = custom_id.strip_prefix("kindly:") else {
                return Ok(());
            };
            let Ok(invoice_id) = raw_invoice_id.parse::<uuid::Uuid>() else {
                return Ok(());
            };
            let invoice_id = InvoiceId::from_uuid(invoice_id);

            // Idempotency: never count the same capture twice.
            if !capture_id.is_empty() && state.store.has_payment_ref(&capture_id).await? {
                return Ok(());
            }
            let (amount_minor, currency_code) = event_amount_minor(event).unwrap_or((0, "USD".into()));
            if amount_minor <= 0 {
                return Ok(());
            }
            let invoice = state.store.invoice(invoice_id).await?;
            let currency = Currency::from_code(&currency_code).unwrap_or(invoice.currency);
            let amount = kindly_core::money::Money::new(amount_minor, currency);
            let fully = invoice.outstanding().minor <= amount_minor;

            apply_payment(
                state,
                invoice_id,
                amount,
                if fully { "settlement" } else { "partial" },
                Some(capture_id.as_str()),
                Some(&event.raw),
                kindly_core::action::ActionKind::RecordPartialPayment,
            )
            .await?;

            // Captures against an instalment invoice also reduce the parent.
            if let Some(parent_id) = invoice.parent_invoice_id {
                apply_payment(
                    state,
                    parent_id,
                    amount,
                    if fully { "settlement" } else { "partial" },
                    None,
                    Some(&event.raw),
                    kindly_core::action::ActionKind::RecordPartialPayment,
                )
                .await?;
            }

            settle_a2a_sessions(state, invoice_id).await?;
        }
        WebhookFact::InvoiceSent | WebhookFact::InvoiceCreated | WebhookFact::InvoiceUpdated => {
            if let Some(paypal_invoice_id) = event.paypal_invoice_id() {
                if let Some(mut invoice) = state.store.invoice_by_paypal_id(&paypal_invoice_id).await? {
                    if let Some(status) = event
                        .resource
                        .get("status")
                        .and_then(|v| v.as_str())
                    {
                        invoice.paypal_status = Some(status.to_string());
                        if status == "SENT" && invoice.issued_at.is_none() {
                            invoice.mark_sent(Utc::now());
                        }
                        state.store.upsert_invoice(&invoice, None).await?;
                    }
                }
            }
        }
        WebhookFact::InvoiceCancelled => {
            if let Some(paypal_invoice_id) = event.paypal_invoice_id() {
                if let Some(mut invoice) = state.store.invoice_by_paypal_id(&paypal_invoice_id).await? {
                    invoice.mark_cancelled(Utc::now());
                    invoice.paypal_status = Some("CANCELLED".to_string());
                    state.store.upsert_invoice(&invoice, None).await?;
                    ledger
                        .note(
                            Actor::PayPal,
                            LedgerKind::NegotiationEvent,
                            format!("Invoice {} was cancelled in PayPal", invoice.number),
                            Some(invoice.id),
                        )
                        .await?;
                }
            }
        }
        WebhookFact::Other(event_type) => {
            ledger
                .note(
                    Actor::PayPal,
                    LedgerKind::PayPalWebhook,
                    format!("PayPal event {event_type} (no action needed)"),
                    None,
                )
                .await?;
        }
    }
    Ok(())
}

async fn settle_a2a_sessions(state: &AppState, invoice_id: InvoiceId) -> ApiResult<()> {
    let sessions = state.store.a2a_sessions_for_invoice(invoice_id).await?;
    for session in sessions {
        if matches!(
            session.status.as_str(),
            crate::a2a::status::AGREED | crate::a2a::status::PLAN_ISSUED
        ) {
            // Record a settlement line with the most recent payment reference.
            let payments = state.store.payments(invoice_id).await?;
            if let Some(last) = payments.last() {
                let reference = format!("kindly-{}", invoice_id);
                crate::a2a::record_settlement(state, &session.id, last.amount, &reference).await?;
            }
        }
    }
    Ok(())
}

/// Mirror a PayPal invoice object into Kindly (used by reconciliation).
pub async fn mirror_paypal_invoice(state: &AppState, paypal: &PayPalInvoice) -> ApiResult<()> {
    if let Some(mut invoice) = state.store.invoice_by_paypal_id(&paypal.id).await? {
        invoice.paypal_status = Some(paypal.status.as_str().to_string());
        if let Some(paid) = paypal.amount_paid_minor {
            if paid > invoice.amount_paid.minor {
                let delta = paid - invoice.amount_paid.minor;
                let currency = Currency::from_code(&paypal.currency_code).unwrap_or(invoice.currency);
                let invoice_id = invoice.id;
                let parent = invoice.parent_invoice_id;
                let amount = kindly_core::money::Money::new(delta, currency);
                apply_payment(
                    state,
                    invoice_id,
                    amount,
                    if invoice.outstanding().is_zero() {
                        "settlement"
                    } else {
                        "partial"
                    },
                    None,
                    None,
                    kindly_core::action::ActionKind::RecordPartialPayment,
                )
                .await?;
                if let Some(parent_id) = parent {
                    apply_payment(
                        state,
                        parent_id,
                        amount,
                        "partial",
                        None,
                        None,
                        kindly_core::action::ActionKind::RecordPartialPayment,
                    )
                    .await?;
                }
                return Ok(());
            }
        }
        state.store.upsert_invoice(&invoice, None).await?;
    }
    Ok(())
}

/// Reconcile every open invoice against PayPal. Safe to run any time; the
/// scheduler runs it periodically so a lost webhook is never a lost payment.
pub async fn reconcile(state: &AppState) -> ApiResult<usize> {
    let invoices = state.store.open_invoices().await?;
    let mut checked = 0;
    for invoice in invoices {
        if let Some(paypal_id) = invoice.paypal_invoice_id.clone() {
            match state.paypal.show_invoice(&paypal_id).await {
                Ok(paypal_invoice) => {
                    mirror_paypal_invoice(state, &paypal_invoice).await?;
                    checked += 1;
                }
                Err(error) => {
                    warn!(paypal_invoice_id = %paypal_id, %error, "reconcile: could not read invoice");
                }
            }
        }
        // Also reconcile any plan instalments.
        if let Some(sessions) = state.store.a2a_sessions_for_invoice(invoice.id).await.ok() {
            for session in sessions {
                if session.status == crate::a2a::status::PLAN_ISSUED {
                    settle_a2a_sessions(state, invoice.id).await?;
                }
            }
        }
    }
    if checked > 0 {
        info!(checked, "reconciled invoices with PayPal");
    }
    Ok(checked)
}
