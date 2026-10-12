//! The collection mandate: the user-authored contract that bounds what Kindly
//! may do on its own.
//!
//! This is the trust core of the product. The language model can propose any
//! action it likes; the policy engine in [`crate::policy`] evaluates every
//! proposal against this mandate and the model has no way to override the
//! result. Mirrors the spirit of AP2-style *open mandates* ("here is what you
//! may do without asking again") but for receivables instead of purchases.

use crate::ids::MandateId;
use crate::money::{Currency, Money};
use crate::{Day, Timestamp};
use serde::{Deserialize, Serialize};

/// How much rope the agent has.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AutonomyLevel {
    /// Watch and analyze only. No outbound action, ever.
    Observe,
    /// Propose actions; the owner approves each one before it goes out.
    SuggestOnly,
    /// Act inside the mandate limits; ask only when a limit is exceeded.
    ActWithinLimits,
}

impl AutonomyLevel {
    pub fn label(self) -> &'static str {
        match self {
            AutonomyLevel::Observe => "Observe only",
            AutonomyLevel::SuggestOnly => "Suggest, I approve",
            AutonomyLevel::ActWithinLimits => "Act within limits",
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            AutonomyLevel::Observe => "Kindly watches your receivables and reports. It never contacts a client.",
            AutonomyLevel::SuggestOnly => "Kindly drafts every message and action; nothing leaves without your approval.",
            AutonomyLevel::ActWithinLimits => {
                "Kindly acts on its own inside your limits and asks whenever a limit would be crossed."
            }
        }
    }

    pub fn allows_outbound(self) -> bool {
        !matches!(self, AutonomyLevel::Observe)
    }
}

/// Tone rungs, in escalation order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Tone {
    Warm,
    Neutral,
    Firm,
    Formal,
}

impl Tone {
    pub const LADDER: [Tone; 4] = [Tone::Warm, Tone::Neutral, Tone::Firm, Tone::Formal];

    pub fn label(self) -> &'static str {
        match self {
            Tone::Warm => "Warm",
            Tone::Neutral => "Neutral",
            Tone::Firm => "Firm",
            Tone::Formal => "Formal",
        }
    }

    pub fn index(self) -> u8 {
        match self {
            Tone::Warm => 0,
            Tone::Neutral => 1,
            Tone::Firm => 2,
            Tone::Formal => 3,
        }
    }

    /// Tone for the Nth contact, clamped to the end of the ladder.
    pub fn for_contact(contact_number: u8, ladder: &[Tone]) -> Tone {
        if ladder.is_empty() {
            return Tone::Neutral;
        }
        let idx = (contact_number.saturating_sub(1) as usize).min(ladder.len() - 1);
        ladder[idx]
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Channel {
    /// The client's Kindly portal page (always available, carries a pay link).
    Portal,
    /// PayPal's own invoice email to the client.
    PayPalInvoiceEmail,
    /// Agent-to-agent channel with the client's AP agent.
    AgentToAgent,
}

impl Channel {
    pub fn label(self) -> &'static str {
        match self {
            Channel::Portal => "Kindly link",
            Channel::PayPalInvoiceEmail => "PayPal email",
            Channel::AgentToAgent => "Agent-to-agent",
        }
    }
}

/// When Kindly may contact clients (in the owner's local time).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContactWindow {
    /// Local hour when contact may start (0–23).
    pub start_hour: u8,
    /// Local hour when contact must stop (1–24; 24 means midnight).
    pub end_hour: u8,
    /// Business days only (Mon–Fri).
    pub weekdays_only: bool,
    /// Owner's UTC offset in minutes, e.g. -480 for US Pacific.
    pub utc_offset_minutes: i32,
}

impl Default for ContactWindow {
    fn default() -> Self {
        Self {
            start_hour: 8,
            end_hour: 18,
            weekdays_only: true,
            utc_offset_minutes: 0,
        }
    }
}

impl ContactWindow {
    /// Is `now` inside the window?
    pub fn allows(&self, now: Timestamp) -> bool {
        let offset = chrono::Duration::minutes(self.utc_offset_minutes as i64);
        let local = now + offset;
        if self.weekdays_only {
            use chrono::Datelike;
            let weekday = local.weekday().num_days_from_monday();
            if weekday > 4 {
                return false;
            }
        }
        use chrono::Timelike;
        let hour = local.hour() as u8;
        hour >= self.start_hour && hour < self.end_hour
    }

    pub fn describe(&self) -> String {
        let suffix = if self.weekdays_only { " on weekdays" } else { "" };
        format!(
            "{:02}:00–{:02}:00 local{}",
            self.start_hour, self.end_hour, suffix
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EscalationKind {
    /// Ask the owner to step in personally.
    HumanReview,
    /// Suggest a call or a meeting.
    SuggestCall,
    /// Flag the invoice as at risk of default.
    FlagAtRisk,
}

impl EscalationKind {
    pub fn label(self) -> &'static str {
        match self {
            EscalationKind::HumanReview => "Human review",
            EscalationKind::SuggestCall => "Suggest a call",
            EscalationKind::FlagAtRisk => "Flag as at risk",
        }
    }
}

/// The full mandate. Every field has a safe default.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CollectionMandate {
    pub id: MandateId,
    pub version: u32,
    pub autonomy: AutonomyLevel,

    // --- contact limits -------------------------------------------------
    pub max_contacts_per_week: u8,
    pub min_days_between_contacts: u8,
    pub contact_window: ContactWindow,
    pub tone_ladder: Vec<Tone>,

    // --- money limits ---------------------------------------------------
    /// Never settle for less than this share of the outstanding balance up front.
    pub min_upfront_settlement_bps: i32,
    /// Absolute ceiling on any discount, ever.
    pub max_discount_bps: i32,
    /// Discounts above this need explicit owner approval.
    pub max_discount_without_approval_bps: i32,
    /// Ceiling on a promise to pay in installments.
    pub max_installments: u8,
    /// A payment plan must complete within this many days.
    pub max_plan_days: u16,
    /// Don't propose installments smaller than this.
    pub min_installment_amount: Money,

    // --- escalation -----------------------------------------------------
    /// After this many contacts on one invoice, suggest human review.
    pub escalate_after_contacts: u8,
    /// After this much silence, stop collecting and propose a write-off review.
    pub stop_after_days_silent: u16,

    // --- agent-to-agent -------------------------------------------------
    pub allow_agent_to_agent: bool,

    pub updated_at: Timestamp,
}

impl CollectionMandate {
    pub fn default_for(currency: Currency, at: Timestamp) -> Self {
        Self {
            id: MandateId::new(),
            version: 1,
            autonomy: AutonomyLevel::ActWithinLimits,
            max_contacts_per_week: 2,
            min_days_between_contacts: 3,
            contact_window: ContactWindow::default(),
            tone_ladder: vec![Tone::Warm, Tone::Warm, Tone::Neutral, Tone::Firm, Tone::Formal],
            min_upfront_settlement_bps: 7_000,
            max_discount_bps: 1_000,
            max_discount_without_approval_bps: 0,
            max_installments: 3,
            max_plan_days: 45,
            min_installment_amount: Money::new(5_000, currency),
            escalate_after_contacts: 4,
            stop_after_days_silent: 90,
            allow_agent_to_agent: true,
            updated_at: at,
        }
    }

    pub fn allow_agent_to_agent(&self) -> bool {
        self.allow_agent_to_agent && self.autonomy.allows_outbound()
    }

    /// Plain-language rendering of the active limits, used in the UI and in
    /// the demo ("your mandate, in one breath").
    pub fn digest(&self) -> Vec<String> {
        let mut out = vec![
            format!(
                "{} — up to {} contact{} a week",
                self.autonomy.label(),
                self.max_contacts_per_week,
                if self.max_contacts_per_week == 1 { "" } else { "s" }
            ),
            format!(
                "Never less than {}% up front",
                (self.min_upfront_settlement_bps as f64 / 100.0).round()
            ),
            format!(
                "Discounts capped at {}% ({}% without asking)",
                (self.max_discount_bps as f64 / 100.0).round(),
                (self.max_discount_without_approval_bps as f64 / 100.0).round()
            ),
            format!("Up to {} installments within {} days", self.max_installments, self.max_plan_days),
            format!("Contact only {}", self.contact_window.describe()),
        ];
        if self.allow_agent_to_agent {
            out.push("May negotiate agent-to-agent".to_string());
        }
        out
    }
}

impl Default for CollectionMandate {
    fn default() -> Self {
        Self::default_for(Currency::Usd, chrono::Utc::now())
    }
}

/// A promise to pay that the client made and Kindly recorded.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PromiseToPay {
    pub amount: Money,
    pub on: Day,
    pub note: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{TimeZone, Utc};

    #[test]
    fn contact_window_respects_hours_and_weekdays() {
        let window = ContactWindow {
            start_hour: 8,
            end_hour: 18,
            weekdays_only: true,
            utc_offset_minutes: 0,
        };
        // Monday 2026-10-12
        assert!(window.allows(Utc.with_ymd_and_hms(2026, 10, 12, 9, 0, 0).unwrap()));
        assert!(!window.allows(Utc.with_ymd_and_hms(2026, 10, 12, 7, 59, 0).unwrap()));
        assert!(!window.allows(Utc.with_ymd_and_hms(2026, 10, 12, 18, 0, 0).unwrap()));
        // Saturday
        assert!(!window.allows(Utc.with_ymd_and_hms(2026, 10, 10, 12, 0, 0).unwrap()));
    }

    #[test]
    fn contact_window_applies_utc_offset() {
        let window = ContactWindow {
            start_hour: 9,
            end_hour: 17,
            weekdays_only: false,
            utc_offset_minutes: -480, // Pacific
        };
        // 17:00 UTC == 09:00 Pacific on a Tuesday
        assert!(window.allows(Utc.with_ymd_and_hms(2026, 10, 13, 17, 0, 0).unwrap()));
        // 16:59 UTC == 08:59 Pacific
        assert!(!window.allows(Utc.with_ymd_and_hms(2026, 10, 13, 16, 59, 0).unwrap()));
    }

    #[test]
    fn tone_ladder_clamps_at_the_end() {
        let ladder = vec![Tone::Warm, Tone::Neutral, Tone::Firm];
        assert_eq!(Tone::for_contact(1, &ladder), Tone::Warm);
        assert_eq!(Tone::for_contact(2, &ladder), Tone::Neutral);
        assert_eq!(Tone::for_contact(9, &ladder), Tone::Firm);
        assert_eq!(Tone::for_contact(1, &[]), Tone::Neutral);
    }

    #[test]
    fn default_mandate_is_conservative() {
        let m = CollectionMandate::default();
        assert_eq!(m.autonomy, AutonomyLevel::ActWithinLimits);
        assert_eq!(m.max_discount_without_approval_bps, 0, "no discounts without asking");
        assert!(m.min_upfront_settlement_bps >= 5_000);
        assert!(!m.digest().is_empty());
    }
}
