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
    let now = chrono::Utc::now();
    for (invoice, negotiation) in invoices {
        if !invoice.is_open() {
            continue;
        }
        // Instalments are obligations under their parent invoice; the parent's
        // negotiation drives them, so ticking them would double the work.
        if invoice.parent_invoice_id.is_some() {
            continue;
        }
        // The runtime schedules its own next look (after a contact, an approval
        // request, a block, or a quiet spell). Respect that instead of
        // re-evaluating every invoice every 90 seconds.
        let due = negotiation
            .next_check_at
            .map(|at| at <= now)
            .unwrap_or(true);
        if due && invoice.days_overdue(today) > 0 {
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
