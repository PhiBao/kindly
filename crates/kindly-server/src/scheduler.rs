//! The autonomous pass.
//!
//! Runs every 90 seconds in the background: advance every open conversation that
//! is due for attention, then reconcile with PayPal so a lost webhook is never a
//! lost payment.

use crate::{engine, state::AppState, sync};
use anyhow::Result;
use std::sync::Arc;
use tracing::warn;

#[derive(Debug, Default)]
pub struct Summary {
    pub ticks: usize,
    pub reconciled: usize,
}

pub async fn pass(state: &Arc<AppState>) -> Result<Summary> {
    let mut summary = Summary::default();
    let today = chrono::Utc::now().date_naive();

    let invoices = state.store.invoices_with_negotiation().await?;
    for (invoice, negotiation) in invoices {
        if !invoice.is_open() {
            continue;
        }
        let due_for_attention = match negotiation.last_contact_at {
            Some(last) => (chrono::Utc::now() - last).num_days() >= 2,
            None => true,
        };
        if due_for_attention && invoice.days_overdue(today) > 0 {
            engine::tick_invoice(state, invoice.id).await?;
            summary.ticks += 1;
        }
    }

    summary.reconciled = match sync::reconcile(state).await {
        Ok(count) => count,
        Err(error) => {
            warn!(%error, "reconcile pass failed");
            0
        }
    };
    Ok(summary)
}
