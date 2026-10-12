//! Clients: the people and companies who owe the user money.

use crate::ids::ClientId;
use crate::Timestamp;
use serde::{Deserialize, Serialize};

/// What Kindly has learned about how a client pays. Everything here is derived
/// from observed payment history — never guessed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct PaymentBehavior {
    /// Mean days late across settled invoices (negative = pays early).
    pub avg_days_late: i32,
    /// Weekday the client typically runs payments (0 = Monday .. 6 = Sunday).
    pub typical_pay_weekday: Option<u8>,
    pub invoices_paid: u32,
    pub invoices_late: u32,
    pub last_paid_at: Option<Timestamp>,
}

impl PaymentBehavior {
    pub fn risk_band(&self) -> RiskBand {
        if self.invoices_paid == 0 {
            return RiskBand::Unknown;
        }
        let lateness = self.invoices_late as f64 / self.invoices_paid as f64;
        match (lateness, self.avg_days_late) {
            (l, _) if l <= 0.2 => RiskBand::Reliable,
            (_, d) if d >= 21 => RiskBand::Slow,
            (l, _) if l <= 0.6 => RiskBand::Watch,
            _ => RiskBand::Slow,
        }
    }

    pub fn record_late_payment(&mut self, days_late: i32, at: Timestamp) {
        let prior_total = self.avg_days_late as i64 * self.invoices_paid as i64;
        self.invoices_paid = self.invoices_paid.saturating_add(1);
        self.invoices_late = self.invoices_late.saturating_add(1);
        let new_mean = (prior_total + days_late as i64) / self.invoices_paid.max(1) as i64;
        self.avg_days_late = new_mean.clamp(-3650, 3650) as i32;
        self.last_paid_at = Some(at);
    }

    pub fn record_on_time_payment(&mut self, at: Timestamp) {
        let prior_total = self.avg_days_late as i64 * self.invoices_paid as i64;
        self.invoices_paid = self.invoices_paid.saturating_add(1);
        let new_mean = prior_total / self.invoices_paid.max(1) as i64;
        self.avg_days_late = new_mean as i32;
        self.last_paid_at = Some(at);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RiskBand {
    Unknown,
    Reliable,
    Watch,
    Slow,
}

impl RiskBand {
    pub fn label(self) -> &'static str {
        match self {
            RiskBand::Unknown => "New client",
            RiskBand::Reliable => "Pays reliably",
            RiskBand::Watch => "Worth watching",
            RiskBand::Slow => "Pays slowly",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Client {
    pub id: ClientId,
    pub name: String,
    pub company: Option<String>,
    pub email: String,
    /// PayPal payer id when known (from invoice payments).
    pub paypal_payer_id: Option<String>,
    pub notes: Option<String>,
    pub behavior: PaymentBehavior,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

impl Client {
    pub fn new(name: impl Into<String>, email: impl Into<String>, at: Timestamp) -> Self {
        Self {
            id: ClientId::new(),
            name: name.into(),
            company: None,
            email: email.into(),
            paypal_payer_id: None,
            notes: None,
            behavior: PaymentBehavior::default(),
            created_at: at,
            updated_at: at,
        }
    }

    pub fn display_name(&self) -> &str {
        match &self.company {
            Some(company) if !company.trim().is_empty() => company,
            _ => &self.name,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    #[test]
    fn behavior_tracks_running_mean() {
        let mut behavior = PaymentBehavior::default();
        assert_eq!(behavior.risk_band(), RiskBand::Unknown);
        behavior.record_late_payment(10, Utc::now());
        behavior.record_late_payment(20, Utc::now());
        assert_eq!(behavior.avg_days_late, 15);
        assert_eq!(behavior.invoices_paid, 2);
        assert_eq!(behavior.invoices_late, 2);
        assert_eq!(behavior.risk_band(), RiskBand::Slow);
    }

    #[test]
    fn reliable_clients_stay_reliable() {
        let mut behavior = PaymentBehavior::default();
        for _ in 0..5 {
            behavior.record_on_time_payment(Utc::now());
        }
        behavior.record_late_payment(2, Utc::now());
        assert_eq!(behavior.risk_band(), RiskBand::Reliable);
    }
}
