//! Webhook helpers: classify PayPal events into the small set of facts Kindly
//! cares about.

use crate::model::VerifiedEvent;
use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WebhookFact {
    InvoiceCreated,
    InvoiceSent,
    InvoiceUpdated,
    InvoiceCancelled,
    /// Money landed on an invoice (partial or full).
    InvoicePaid,
    PaymentCaptured,
    Other(String),
}

impl WebhookFact {
    pub fn from_event_type(event_type: &str) -> Self {
        match event_type {
            "INVOICING.INVOICE.CREATED" => Self::InvoiceCreated,
            "INVOICING.INVOICE.SENT" => Self::InvoiceSent,
            "INVOICING.INVOICE.UPDATED" => Self::InvoiceUpdated,
            "INVOICING.INVOICE.CANCELLED" => Self::InvoiceCancelled,
            "INVOICING.INVOICE.PAID" => Self::InvoicePaid,
            "PAYMENT.CAPTURE.COMPLETED" => Self::PaymentCaptured,
            other => Self::Other(other.to_string()),
        }
    }

    pub fn is_money(&self) -> bool {
        matches!(self, Self::InvoicePaid | Self::PaymentCaptured)
    }
}

/// Amount of the payment described by a webhook, in minor units.
pub fn event_amount_minor(event: &VerifiedEvent) -> Option<(i64, String)> {
    let resource = &event.resource;
    // Capture resources carry their own amount.
    if let Some(amount) = resource.get("amount") {
        let currency = amount
            .get("currency_code")
            .and_then(Value::as_str)
            .unwrap_or("USD")
            .to_string();
        let value = amount.get("value").and_then(Value::as_str)?;
        return crate::model::parse_minor(value, &currency).ok().map(|m| (m, currency));
    }
    // Invoice resources: prefer the delta between total paid and what was paid
    // before, which the caller resolves; here we return the paid total.
    let breakdown = resource
        .get("amount")
        .and_then(|a| a.get("breakdown"))?;
    let currency = resource
        .get("detail")
        .and_then(|d| d.get("currency_code"))
        .and_then(Value::as_str)
        .unwrap_or("USD")
        .to_string();
    let paid = breakdown
        .get("total_paid_amount")
        .and_then(|a| a.get("value"))
        .and_then(Value::as_str)?;
    crate::model::parse_minor(paid, &currency)
        .ok()
        .map(|m| (m, currency))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_event_types() {
        assert_eq!(
            WebhookFact::from_event_type("INVOICING.INVOICE.PAID"),
            WebhookFact::InvoicePaid
        );
        assert!(WebhookFact::from_event_type("PAYMENT.CAPTURE.COMPLETED").is_money());
        assert!(!WebhookFact::from_event_type("INVOICING.INVOICE.SENT").is_money());
        assert_eq!(
            WebhookFact::from_event_type("SOMETHING.ELSE"),
            WebhookFact::Other("SOMETHING.ELSE".to_string())
        );
    }

    #[test]
    fn extracts_amounts_from_capture_resources() {
        let event: VerifiedEvent =
            serde_json::from_value(serde_json::json!({
                "event_id": "WH-1",
                "event_type": "PAYMENT.CAPTURE.COMPLETED",
                "resource_type": "capture",
                "resource": {
                    "id": "CAP-1",
                    "amount": {"currency_code": "USD", "value": "1200.00"}
                },
                "verified": true,
                "raw": {},
                "received_at": null
            }))
            .unwrap();
        assert_eq!(event_amount_minor(&event), Some((120_000, "USD".to_string())));
    }
}
