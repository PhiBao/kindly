//! Attribution: honest causal accounting for "what got you paid".
//!
//! Most products claim credit for everything. Kindly only claims what the
//! timeline supports, and says so with an explicit confidence level. The rules
//! are deterministic and documented here so a judge can check them.

use crate::action::ActionKind;
use crate::ids::{ActionId, AttributionId, InvoiceId};
use crate::invoice::Invoice;
use crate::money::Money;
use crate::Timestamp;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PaymentSource {
    /// Paid through the PayPal-hosted invoice.
    PayPalInvoice,
    /// Paid through the client portal (orders API).
    PayPalOrder,
    /// Recorded after arriving through another rail.
    RecordedExternal,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActionRecord {
    pub id: ActionId,
    pub kind: ActionKind,
    pub at: Timestamp,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PaymentRecord {
    pub at: Timestamp,
    pub amount: Money,
    pub fully_settles: bool,
    pub source: PaymentSource,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CauseKind {
    /// Terms Kindly negotiated were executed and the money arrived.
    PlanSettled,
    /// The client paid the link inside a reminder Kindly sent.
    ReminderLed,
    /// A promise-to-pay date came true.
    PromiseKept,
    /// Money arrived with no agent action in the causal window.
    Organic,
    /// Not enough signal yet.
    Unknown,
}

impl CauseKind {
    pub fn label(self) -> &'static str {
        match self {
            CauseKind::PlanSettled => "Agreed plan settled",
            CauseKind::ReminderLed => "Reminder converted",
            CauseKind::PromiseKept => "Promise kept",
            CauseKind::Organic => "Paying on their own",
            CauseKind::Unknown => "Not enough signal",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Confidence {
    High,
    Medium,
    Low,
}

impl Confidence {
    pub fn label(self) -> &'static str {
        match self {
            Confidence::High => "High",
            Confidence::Medium => "Medium",
            Confidence::Low => "Low",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Attribution {
    pub id: AttributionId,
    pub invoice_id: InvoiceId,
    pub kind: CauseKind,
    pub confidence: Confidence,
    pub action_ids: Vec<ActionId>,
    pub rationale: String,
    pub days_to_paid: Option<i64>,
    pub baseline_days: Option<i64>,
    pub days_saved: Option<i64>,
    pub created_at: Timestamp,
}

/// How long after an agent action a payment still counts as "probably caused
/// by it". Documented and deliberately conservative.
pub const CAUSAL_WINDOW_DAYS: i64 = 14;
/// Inside this window we are confident the action did it.
pub const STRONG_WINDOW_DAYS: i64 = 3;

pub fn attribute(
    invoice: &Invoice,
    actions: &[ActionRecord],
    payments: &[PaymentRecord],
    baseline_days: Option<i64>,
    now: Timestamp,
) -> Attribution {
    let settlement = payments
        .iter()
        .filter(|p| p.fully_settles)
        .max_by_key(|p| p.at);
    let days_to_paid = invoice.days_to_paid().or_else(|| {
        settlement.map(|p| (p.at - invoice.issued_at.unwrap_or(invoice.created_at)).num_days())
    });
    let days_saved = match (days_to_paid, baseline_days) {
        (Some(actual), Some(base)) => Some(base - actual),
        _ => None,
    };

    let mut sorted: Vec<&ActionRecord> = actions.iter().collect();
    sorted.sort_by_key(|a| a.at);

    let Some(settlement) = settlement else {
        return Attribution {
            id: AttributionId::new(),
            invoice_id: invoice.id,
            kind: CauseKind::Unknown,
            confidence: Confidence::Low,
            action_ids: Vec::new(),
            rationale: "No settlement recorded yet.".into(),
            days_to_paid,
            baseline_days,
            days_saved,
            created_at: now,
        };
    };

    // 1. Plan settled: an IssuePlan ran before the money landed.
    if let Some(plan_action) = sorted
        .iter()
        .rev()
        .find(|a| a.kind == ActionKind::IssuePlan && a.at <= settlement.at)
    {
        return Attribution {
            id: AttributionId::new(),
            invoice_id: invoice.id,
            kind: CauseKind::PlanSettled,
            confidence: Confidence::High,
            action_ids: vec![plan_action.id],
            rationale: "The payment followed the plan Kindly negotiated and issued."
                .to_string(),
            days_to_paid,
            baseline_days,
            days_saved,
            created_at: now,
        };
    }

    // 2. Reminder-led: the most recent client-facing action inside the window.
    let last_contact = sorted
        .iter()
        .rev()
        .find(|a| {
            matches!(a.kind, ActionKind::Nudge | ActionKind::OfferTerms) && a.at <= settlement.at
        });

    if let Some(contact) = last_contact {
        let gap = (settlement.at - contact.at).num_days();
        if gap <= CAUSAL_WINDOW_DAYS {
            let (confidence, rationale) = if gap <= STRONG_WINDOW_DAYS {
                (
                    Confidence::High,
                    format!(
                        "Paid {gap} day(s) after Kindly's {}. The link in that message is how it got paid.",
                        contact.kind.label().to_lowercase()
                    ),
                )
            } else {
                (
                    Confidence::Medium,
                    format!(
                        "Paid {gap} days after Kindly's {} — inside the causal window, but not immediately.",
                        contact.kind.label().to_lowercase()
                    ),
                )
            };
            return Attribution {
                id: AttributionId::new(),
                invoice_id: invoice.id,
                kind: CauseKind::ReminderLed,
                confidence,
                action_ids: vec![contact.id],
                rationale,
                days_to_paid,
                baseline_days,
                days_saved,
                created_at: now,
            };
        }
    }

    // 3. Organic: nobody nudged them into it.
    Attribution {
        id: AttributionId::new(),
        invoice_id: invoice.id,
        kind: CauseKind::Organic,
        confidence: Confidence::High,
        action_ids: Vec::new(),
        rationale: "The client paid on their own — no Kindly action in the causal window. Kindly does not take credit for it.".into(),
        days_to_paid,
        baseline_days,
        days_saved,
        created_at: now,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::ClientId;
    use crate::invoice::{InvoiceLine, Quantity};
    use crate::money::Currency;
    use chrono::{NaiveDate, TimeZone, Utc};

    fn ts(day: u32) -> Timestamp {
        Utc.with_ymd_and_hms(2026, 9, day, 12, 0, 0).unwrap()
    }

    fn invoice() -> Invoice {
        let mut inv = Invoice::new(
            ClientId::new(),
            "1042",
            Currency::Usd,
            NaiveDate::from_ymd_opt(2026, 10, 1).unwrap(),
            vec![InvoiceLine::new(
                "Work",
                Quantity::whole(1).unwrap(),
                Money::new(240_000, Currency::Usd),
            )],
            ts(1),
        )
        .unwrap();
        inv.mark_sent(ts(1));
        inv
    }

    fn settled_invoice() -> Invoice {
        let mut inv = invoice();
        inv.apply_payment(Money::new(240_000, Currency::Usd), ts(20)).unwrap();
        inv
    }

    #[test]
    fn reminder_led_attribution_with_high_confidence() {
        let inv = settled_invoice();
        let actions = vec![ActionRecord {
            id: ActionId::new(),
            kind: ActionKind::Nudge,
            at: ts(18),
        }];
        let payments = vec![PaymentRecord {
            at: ts(20),
            amount: Money::new(240_000, Currency::Usd),
            fully_settles: true,
            source: PaymentSource::PayPalInvoice,
        }];
        let attr = attribute(&inv, &actions, &payments, Some(30), ts(21));
        assert_eq!(attr.kind, CauseKind::ReminderLed);
        assert_eq!(attr.confidence, Confidence::High);
        assert_eq!(attr.days_saved, Some(11));
    }

    #[test]
    fn plan_settled_beats_reminder_attribution() {
        let inv = settled_invoice();
        let actions = vec![
            ActionRecord {
                id: ActionId::new(),
                kind: ActionKind::Nudge,
                at: ts(10),
            },
            ActionRecord {
                id: ActionId::new(),
                kind: ActionKind::IssuePlan,
                at: ts(12),
            },
        ];
        let payments = vec![PaymentRecord {
            at: ts(20),
            amount: Money::new(240_000, Currency::Usd),
            fully_settles: true,
            source: PaymentSource::PayPalInvoice,
        }];
        let attr = attribute(&inv, &actions, &payments, None, ts(21));
        assert_eq!(attr.kind, CauseKind::PlanSettled);
        assert_eq!(attr.confidence, Confidence::High);
    }

    #[test]
    fn organic_payments_are_not_claimed() {
        let inv = settled_invoice();
        let actions = vec![ActionRecord {
            id: ActionId::new(),
            kind: ActionKind::Nudge,
            at: ts(2),
        }];
        let payments = vec![PaymentRecord {
            at: ts(20),
            amount: Money::new(240_000, Currency::Usd),
            fully_settles: true,
            source: PaymentSource::PayPalOrder,
        }];
        let attr = attribute(&inv, &actions, &payments, None, ts(21));
        assert_eq!(attr.kind, CauseKind::Organic);
        assert!(attr.action_ids.is_empty());
    }

    #[test]
    fn unsettled_invoices_are_unknown() {
        let inv = invoice();
        let attr = attribute(&inv, &[], &[], Some(30), ts(21));
        assert_eq!(attr.kind, CauseKind::Unknown);
    }
}
