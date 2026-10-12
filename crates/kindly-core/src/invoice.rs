//! Invoices and their lifecycle.

use crate::error::{CoreError, CoreResult};
use crate::ids::{ClientId, InvoiceId, InvoiceLineId};
use crate::money::{Currency, Money};
use crate::{Day, Timestamp};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InvoiceStatus {
    Draft,
    Sent,
    PartiallyPaid,
    Paid,
    Cancelled,
    /// Derived status: sent and past due (kept explicit so the UI and the
    /// ledger never have to re-derive it).
    Overdue,
}

impl InvoiceStatus {
    pub fn label(self) -> &'static str {
        match self {
            InvoiceStatus::Draft => "Draft",
            InvoiceStatus::Sent => "Sent",
            InvoiceStatus::PartiallyPaid => "Partly paid",
            InvoiceStatus::Paid => "Paid",
            InvoiceStatus::Cancelled => "Cancelled",
            InvoiceStatus::Overdue => "Overdue",
        }
    }

    pub fn is_open(self) -> bool {
        matches!(
            self,
            InvoiceStatus::Sent | InvoiceStatus::PartiallyPaid | InvoiceStatus::Overdue
        )
    }
}

/// Line quantity in thousandths (2.5 hours = 2500) so quantity math is exact.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Quantity(pub i64);

impl Quantity {
    pub fn whole(units: i64) -> CoreResult<Self> {
        units
            .checked_mul(1000)
            .map(Quantity)
            .ok_or(CoreError::Overflow {
                context: "quantity",
            })
    }

    pub fn from_thousandths(value: i64) -> CoreResult<Self> {
        if value < 0 {
            return Err(CoreError::InvalidQuantity(format!(
                "negative quantity {value}"
            )));
        }
        Ok(Quantity(value))
    }

    pub fn thousandths(self) -> i64 {
        self.0
    }

    pub fn display(self) -> String {
        let whole = self.0 / 1000;
        let frac = self.0 % 1000;
        if frac == 0 {
            format!("{whole}")
        } else {
            let mut s = format!("{}.{:03}", whole, frac);
            while s.ends_with('0') {
                s.pop();
            }
            s
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InvoiceLine {
    pub id: InvoiceLineId,
    pub description: String,
    pub quantity: Quantity,
    pub unit_price: Money,
}

impl InvoiceLine {
    pub fn new(description: impl Into<String>, quantity: Quantity, unit_price: Money) -> Self {
        Self {
            id: InvoiceLineId::new(),
            description: description.into(),
            quantity,
            unit_price,
        }
    }

    /// `quantity * unit_price`, rounded half away from zero at the line level.
    pub fn amount(&self) -> CoreResult<Money> {
        let scaled = (self.quantity.thousandths() as i128) * (self.unit_price.minor as i128);
        let rounded = divide_round_half_away_from_zero(scaled, 1000);
        let minor = i64::try_from(rounded).map_err(|_| CoreError::Overflow {
            context: "line amount",
        })?;
        Ok(Money::new(minor, self.unit_price.currency))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Invoice {
    pub id: InvoiceId,
    pub client_id: ClientId,
    pub number: String,
    pub status: InvoiceStatus,
    pub currency: Currency,
    pub lines: Vec<InvoiceLine>,
    pub total: Money,
    pub amount_paid: Money,
    pub due_date: Day,
    pub issued_at: Option<Timestamp>,
    pub paid_at: Option<Timestamp>,
    /// PayPal Invoicing API invoice id, once created there.
    pub paypal_invoice_id: Option<String>,
    pub paypal_status: Option<String>,
    pub memo: Option<String>,
    /// Set when this invoice is an instalment issued under a payment plan:
    /// payments against it belong to the parent invoice's balance.
    #[serde(default)]
    pub parent_invoice_id: Option<InvoiceId>,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

impl Invoice {
    pub fn new(
        client_id: ClientId,
        number: impl Into<String>,
        currency: Currency,
        due_date: Day,
        lines: Vec<InvoiceLine>,
        at: Timestamp,
    ) -> CoreResult<Self> {
        if lines.is_empty() {
            return Err(CoreError::Validation(
                "invoice needs at least one line".into(),
            ));
        }
        let total = Money::sum(
            lines
                .iter()
                .map(|l| l.amount())
                .collect::<CoreResult<Vec<_>>>()?,
            currency,
        )?;
        Ok(Self {
            id: InvoiceId::new(),
            client_id,
            number: number.into(),
            status: InvoiceStatus::Draft,
            currency,
            lines,
            total,
            amount_paid: Money::zero(currency),
            due_date,
            issued_at: None,
            paid_at: None,
            paypal_invoice_id: None,
            paypal_status: None,
            memo: None,
            parent_invoice_id: None,
            created_at: at,
            updated_at: at,
        })
    }

    pub fn outstanding(&self) -> Money {
        // amount_paid never exceeds total by construction, but guard anyway.
        let overdue = self.total.minor - self.amount_paid.minor;
        Money::new(overdue.max(0), self.currency)
    }

    pub fn is_open(&self) -> bool {
        self.status.is_open() && self.outstanding().is_positive()
    }

    pub fn days_overdue(&self, today: Day) -> i64 {
        if !self.is_open() {
            return 0;
        }
        (today - self.due_date).num_days().max(0)
    }

    pub fn days_to_paid(&self) -> Option<i64> {
        let issued = self.issued_at?;
        let paid = self.paid_at?;
        Some((paid - issued).num_days())
    }

    pub fn mark_sent(&mut self, at: Timestamp) {
        if self.status == InvoiceStatus::Draft {
            self.status = InvoiceStatus::Sent;
            self.issued_at = Some(at);
            self.updated_at = at;
        }
    }

    /// Apply a payment. Returns the amount actually applied (never more than
    /// the outstanding balance) so callers can reconcile overpayments safely.
    pub fn apply_payment(&mut self, amount: Money, at: Timestamp) -> CoreResult<Money> {
        if amount.currency != self.currency {
            return Err(CoreError::CurrencyMismatch {
                left: amount.currency.code().to_string(),
                right: self.currency.code().to_string(),
            });
        }
        if amount.is_negative() {
            return Err(CoreError::Validation(
                "payment amount cannot be negative".into(),
            ));
        }
        let outstanding = self.outstanding();
        let applied = if amount.minor > outstanding.minor {
            outstanding
        } else {
            amount
        };
        self.amount_paid =
            self.amount_paid
                .checked_add(applied)
                .map_err(|_| CoreError::Overflow {
                    context: "apply_payment",
                })?;
        if self.amount_paid.minor >= self.total.minor {
            self.status = InvoiceStatus::Paid;
            self.paid_at = Some(at);
        } else if self.amount_paid.is_positive() {
            self.status = InvoiceStatus::PartiallyPaid;
        }
        self.updated_at = at;
        Ok(applied)
    }

    pub fn mark_cancelled(&mut self, at: Timestamp) {
        self.status = InvoiceStatus::Cancelled;
        self.updated_at = at;
    }

    pub fn mark_overdue(&mut self) {
        if self.is_open() && self.status != InvoiceStatus::Overdue {
            self.status = InvoiceStatus::Overdue;
        }
    }
}

fn divide_round_half_away_from_zero(numerator: i128, denominator: i128) -> i128 {
    let sign = if numerator < 0 { -1 } else { 1 };
    let magnitude = numerator.abs();
    let quotient = magnitude / denominator;
    let remainder = magnitude % denominator;
    let rounded = if remainder * 2 >= denominator {
        quotient + 1
    } else {
        quotient
    };
    sign * rounded
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{NaiveDate, TimeZone, Utc};

    fn day(y: i32, m: u32, d: u32) -> Day {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    fn at(y: i32, m: u32, d: u32) -> Timestamp {
        Utc.with_ymd_and_hms(y, m, d, 12, 0, 0).unwrap()
    }

    fn sample_invoice() -> Invoice {
        let lines = vec![
            InvoiceLine::new(
                "Brand identity sprint",
                Quantity::whole(1).unwrap(),
                Money::new(180_000, Currency::Usd),
            ),
            InvoiceLine::new(
                "Revisions (hours)",
                Quantity::from_thousandths(2500).unwrap(),
                Money::new(12_000, Currency::Usd),
            ),
        ];
        Invoice::new(
            ClientId::new(),
            "1042",
            Currency::Usd,
            day(2026, 10, 1),
            lines,
            at(2026, 9, 15),
        )
        .unwrap()
    }

    #[test]
    fn totals_are_exact() {
        let inv = sample_invoice();
        // 1800.00 + 2.5 * 120.00 = 1800.00 + 300.00 = 2100.00
        assert_eq!(inv.total.minor, 210_000);
        assert_eq!(inv.outstanding().minor, 210_000);
        assert_eq!(inv.status, InvoiceStatus::Draft);
    }

    #[test]
    fn payments_move_status_and_cap_at_total() {
        let mut inv = sample_invoice();
        inv.mark_sent(at(2026, 9, 15));
        inv.apply_payment(Money::new(60_000, Currency::Usd), at(2026, 9, 20))
            .unwrap();
        assert_eq!(inv.status, InvoiceStatus::PartiallyPaid);
        assert_eq!(inv.outstanding().minor, 150_000);
        let applied = inv
            .apply_payment(Money::new(900_000, Currency::Usd), at(2026, 9, 25))
            .unwrap();
        assert_eq!(applied.minor, 150_000, "overpayment is capped");
        assert_eq!(inv.status, InvoiceStatus::Paid);
        assert_eq!(inv.amount_paid.minor, 210_000);
    }

    #[test]
    fn overdue_math() {
        let mut inv = sample_invoice();
        inv.mark_sent(at(2026, 9, 15));
        assert_eq!(inv.days_overdue(day(2026, 9, 30)), 0);
        assert_eq!(inv.days_overdue(day(2026, 10, 11)), 10);
        inv.mark_overdue();
        assert_eq!(inv.status, InvoiceStatus::Overdue);
    }

    #[test]
    fn days_to_paid_uses_issue_date() {
        let mut inv = sample_invoice();
        inv.mark_sent(at(2026, 9, 15));
        inv.apply_payment(Money::new(210_000, Currency::Usd), at(2026, 9, 25))
            .unwrap();
        assert_eq!(inv.days_to_paid(), Some(10));
    }
}
