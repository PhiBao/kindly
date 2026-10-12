//! Vendor-shaped models for the PayPal surfaces Kindly uses.

use crate::error::{PayPalError, PayPalResult};
use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

fn default_currency() -> String {
    "USD".to_string()
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NewInvoiceLine {
    pub name: String,
    pub description: Option<String>,
    /// Decimal string as PayPal expects, e.g. `"2.5"`.
    pub quantity: String,
    pub unit_amount_minor: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NewInvoice {
    pub invoice_number: String,
    pub currency_code: String,
    pub client_email: String,
    pub client_name: Option<String>,
    pub invoicer_email: String,
    pub due_date: NaiveDate,
    pub memo: Option<String>,
    pub note: Option<String>,
    pub lines: Vec<NewInvoiceLine>,
    /// Allow the payer to pay part of the invoice.
    pub allow_partial_payment: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum PayPalInvoiceStatus {
    Draft,
    Sent,
    Scheduled,
    Paid,
    MarkedAsPaid,
    PartiallyPaid,
    PartiallyRefunded,
    Refunded,
    Cancelled,
    Unknown,
}

impl PayPalInvoiceStatus {
    pub fn parse(value: &str) -> Self {
        match value {
            "DRAFT" => Self::Draft,
            "SENT" => Self::Sent,
            "SCHEDULED" => Self::Scheduled,
            "PAID" => Self::Paid,
            "MARKED_AS_PAID" => Self::MarkedAsPaid,
            "PARTIALLY_PAID" => Self::PartiallyPaid,
            "PARTIALLY_REFUNDED" => Self::PartiallyRefunded,
            "REFUNDED" => Self::Refunded,
            "CANCELLED" => Self::Cancelled,
            _ => Self::Unknown,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Draft => "DRAFT",
            Self::Sent => "SENT",
            Self::Scheduled => "SCHEDULED",
            Self::Paid => "PAID",
            Self::MarkedAsPaid => "MARKED_AS_PAID",
            Self::PartiallyPaid => "PARTIALLY_PAID",
            Self::PartiallyRefunded => "PARTIALLY_REFUNDED",
            Self::Refunded => "REFUNDED",
            Self::Cancelled => "CANCELLED",
            Self::Unknown => "UNKNOWN",
        }
    }

    pub fn is_settled(self) -> bool {
        matches!(self, Self::Paid | Self::MarkedAsPaid)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PayPalInvoice {
    pub id: String,
    pub number: Option<String>,
    pub status: PayPalInvoiceStatus,
    #[serde(default = "default_currency")]
    pub currency_code: String,
    pub amount_due_minor: Option<i64>,
    pub amount_paid_minor: Option<i64>,
    pub total_minor: Option<i64>,
    pub due_date: Option<NaiveDate>,
    /// Hosted payment page for the payer.
    pub payer_view_url: Option<String>,
    pub created_at: Option<DateTime<Utc>>,
    pub raw: Value,
}

fn money_to_minor(value: &Value) -> Option<i64> {
    let code = value
        .get("currency_code")
        .and_then(|c| c.as_str())
        .unwrap_or("USD");
    let amount = value.get("value").and_then(|v| v.as_str())?;
    parse_minor(amount, code).ok()
}

/// Parse a PayPal decimal string into minor units.
pub fn parse_minor(amount: &str, currency: &str) -> PayPalResult<i64> {
    let cleaned: String = amount.chars().filter(|c| *c != ',').collect();
    let (whole, frac) = match cleaned.split_once('.') {
        Some((w, f)) => (w.to_string(), f.to_string()),
        None => (cleaned, String::new()),
    };
    let exponent: usize = match currency.to_ascii_uppercase().as_str() {
        "JPY" | "KRW" => 0,
        "BHD" | "KWD" | "OMR" => 3,
        _ => 2,
    };
    let negative = whole.starts_with('-');
    let whole = whole.trim_start_matches('-');
    let whole_value: i64 = whole
        .parse()
        .map_err(|_| PayPalError::Shape(format!("invalid amount {amount}")))?;
    let mut frac_digits = frac;
    while frac_digits.len() < exponent {
        frac_digits.push('0');
    }
    if frac_digits.len() > exponent {
        frac_digits.truncate(exponent);
    }
    let frac_value: i64 = if frac_digits.is_empty() {
        0
    } else {
        frac_digits
            .parse()
            .map_err(|_| PayPalError::Shape(format!("invalid amount {amount}")))?
    };
    let scale = 10i64.pow(exponent as u32);
    let minor = whole_value * scale + frac_value;
    Ok(if negative { -minor } else { minor })
}

/// Format minor units as a PayPal decimal string.
pub fn format_minor(minor: i64, currency: &str) -> String {
    let exponent: u32 = match currency.to_ascii_uppercase().as_str() {
        "JPY" | "KRW" => 0,
        "BHD" | "KWD" | "OMR" => 3,
        _ => 2,
    };
    let scale = 10i64.pow(exponent);
    let sign = if minor < 0 { "-" } else { "" };
    let abs = minor.abs();
    if exponent == 0 {
        format!("{sign}{abs}")
    } else {
        format!(
            "{sign}{}.{:0width$}",
            abs / scale,
            abs % scale,
            width = exponent as usize
        )
    }
}

impl PayPalInvoice {
    pub fn from_resource(resource: &Value) -> PayPalResult<Self> {
        let id = resource
            .get("id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| PayPalError::Shape("invoice resource missing id".into()))?
            .to_string();
        let status = resource
            .get("status")
            .and_then(|v| v.as_str())
            .map(PayPalInvoiceStatus::parse)
            .unwrap_or(PayPalInvoiceStatus::Unknown);
        let currency_code = resource
            .get("detail")
            .and_then(|d| d.get("currency_code"))
            .and_then(|v| v.as_str())
            .unwrap_or("USD")
            .to_string();
        let number = resource
            .get("detail")
            .and_then(|d| d.get("invoice_number"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        let due_date = resource
            .get("detail")
            .and_then(|d| d.get("payment_term"))
            .and_then(|t| t.get("due_date"))
            .and_then(|v| v.as_str())
            .and_then(|s| NaiveDate::parse_from_str(s, "%Y-%m-%d").ok());
        let total_minor = resource
            .get("amount")
            .and_then(|a| a.get("breakdown"))
            .and_then(|b| b.get("item_total"))
            .and_then(money_to_minor)
            .or_else(|| {
                resource
                    .get("amount")
                    .and_then(|a| a.get("value"))
                    .and_then(money_to_minor)
            });
        let amount_due_minor = resource
            .get("amount")
            .and_then(|a| a.get("breakdown"))
            .and_then(|b| b.get("outstanding_balance"))
            .and_then(money_to_minor)
            .or_else(|| {
                resource
                    .get("amount")
                    .and_then(|a| a.get("value"))
                    .and_then(money_to_minor)
            });
        let amount_paid_minor = resource
            .get("amount")
            .and_then(|a| a.get("breakdown"))
            .and_then(|b| b.get("total_paid_amount"))
            .and_then(money_to_minor);
        let payer_view_url = resource
            .get("detail")
            .and_then(|d| d.get("metadata"))
            .and_then(|m| m.get("payer_view_url"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        let created_at = resource
            .get("detail")
            .and_then(|d| d.get("metadata"))
            .and_then(|m| m.get("create_time"))
            .and_then(|v| v.as_str())
            .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
            .map(|dt| dt.with_timezone(&Utc));

        Ok(Self {
            id,
            number,
            status,
            currency_code,
            amount_due_minor,
            amount_paid_minor,
            total_minor,
            due_date,
            payer_view_url,
            created_at,
            raw: resource.clone(),
        })
    }

    /// Serialize back into a PayPal create-invoice request body.
    pub fn build_create_body(request: &NewInvoice) -> Value {
        let items: Vec<Value> = request
            .lines
            .iter()
            .map(|line| {
                let mut item = serde_json::json!({
                    "name": line.name,
                    "quantity": line.quantity,
                    "unit_amount": {
                        "currency_code": request.currency_code,
                        "value": format_minor(line.unit_amount_minor, &request.currency_code),
                    },
                });
                if let Some(description) = &line.description {
                    item["description"] = Value::String(description.clone());
                }
                item
            })
            .collect();

        let mut recipient = serde_json::json!({
            "billing_info": {
                "email_address": request.client_email,
            }
        });
        if let Some(name) = &request.client_name {
            recipient["billing_info"]["name"] = serde_json::json!({ "full_name": name });
        }

        let mut body = serde_json::json!({
            "detail": {
                "invoice_number": request.invoice_number,
                "currency_code": request.currency_code,
                "payment_term": {
                    "term_type": "DUE_ON_DATE_SPECIFIED",
                    "due_date": request.due_date.format("%Y-%m-%d").to_string(),
                },
            },
            "invoicer": {
                "email_address": request.invoicer_email,
            },
            "primary_recipients": [recipient],
            "items": items,
            "configuration": {
                "allow_tip": false,
                "partial_payment": {
                    "allow_partial_payment": request.allow_partial_payment,
                },
                "tax_calculated_after_discount": false,
                "tax_inclusive": false,
            },
        });
        if let Some(memo) = &request.memo {
            body["detail"]["memo"] = Value::String(memo.clone());
        }
        if let Some(note) = &request.note {
            body["detail"]["note"] = Value::String(note.clone());
        }
        body
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NewOrder {
    pub currency_code: String,
    pub amount_minor: i64,
    pub description: String,
    /// Opaque Kindly reference (portal token id + invoice id) stored in the
    /// order's custom_id so captures can be reconciled even if a webhook is lost.
    pub custom_id: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PayPalOrder {
    pub id: String,
    pub status: String,
    pub raw: Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PayPalCapture {
    pub order_id: String,
    pub capture_id: String,
    pub status: String,
    pub amount_minor: i64,
    pub currency_code: String,
    pub raw: Value,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct WebhookHeaders {
    pub transmission_id: Option<String>,
    pub transmission_time: Option<String>,
    pub cert_url: Option<String>,
    pub auth_algo: Option<String>,
    pub transmission_sig: Option<String>,
    /// Simulator-only header (dev); never accepted in live mode.
    pub sim_signature: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VerifiedEvent {
    pub event_id: String,
    pub event_type: String,
    pub resource_type: Option<String>,
    pub resource: Value,
    pub verified: bool,
    pub raw: Value,
    pub received_at: Option<DateTime<Utc>>,
}

impl VerifiedEvent {
    /// PayPal invoice id related to this event, when present.
    pub fn paypal_invoice_id(&self) -> Option<String> {
        if !self.event_type.starts_with("INVOICING.INVOICE") {
            return None;
        }
        self.resource
            .get("invoice_id")
            .and_then(|v| v.as_str())
            .or_else(|| self.resource.get("id").and_then(|v| v.as_str()))
            .map(str::to_string)
    }

    pub fn order_id(&self) -> Option<String> {
        self.resource
            .get("supplementary_data")
            .and_then(|s| s.get("related_ids"))
            .and_then(|r| r.get("order_id"))
            .and_then(|v| v.as_str())
            .map(str::to_string)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn minor_parse_and_format_round_trip() {
        assert_eq!(parse_minor("2400.00", "USD").unwrap(), 240_000);
        assert_eq!(parse_minor("0.05", "USD").unwrap(), 5);
        assert_eq!(parse_minor("1234", "USD").unwrap(), 123_400);
        assert_eq!(parse_minor("500", "JPY").unwrap(), 500);
        assert_eq!(format_minor(240_000, "USD"), "2400.00");
        assert_eq!(format_minor(-5, "USD"), "-0.05");
        assert_eq!(format_minor(500, "JPY"), "500");
    }

    #[test]
    fn builds_create_body_with_partial_payment_enabled() {
        let request = NewInvoice {
            invoice_number: "1042".into(),
            currency_code: "USD".into(),
            client_email: "ap@acme.test".into(),
            client_name: Some("Acme Co".into()),
            invoicer_email: "maya@studio.test".into(),
            due_date: NaiveDate::from_ymd_opt(2026, 10, 1).unwrap(),
            memo: Some("Thanks for the project!".into()),
            note: None,
            lines: vec![NewInvoiceLine {
                name: "Design sprint".into(),
                description: Some("Two weeks".into()),
                quantity: "1".into(),
                unit_amount_minor: 240_000,
            }],
            allow_partial_payment: true,
        };
        let body = PayPalInvoice::build_create_body(&request);
        assert_eq!(body["detail"]["invoice_number"], "1042");
        assert_eq!(body["items"][0]["unit_amount"]["value"], "2400.00");
        assert_eq!(
            body["configuration"]["partial_payment"]["allow_partial_payment"],
            true
        );
    }

    #[test]
    fn parses_invoice_resource() {
        let resource = serde_json::json!({
            "id": "INV2-ABC",
            "status": "PARTIALLY_PAID",
            "detail": {
                "currency_code": "USD",
                "invoice_number": "1042",
                "payment_term": {"due_date": "2026-10-01"},
                "metadata": {
                    "payer_view_url": "https://www.sandbox.paypal.com/invoice/p/#ABC",
                    "create_time": "2026-09-15T10:00:00Z"
                }
            },
            "amount": {
                "breakdown": {
                    "item_total": {"currency_code": "USD", "value": "2400.00"},
                    "outstanding_balance": {"currency_code": "USD", "value": "1200.00"},
                    "total_paid_amount": {"currency_code": "USD", "value": "1200.00"}
                }
            }
        });
        let invoice = PayPalInvoice::from_resource(&resource).unwrap();
        assert_eq!(invoice.id, "INV2-ABC");
        assert_eq!(invoice.status, PayPalInvoiceStatus::PartiallyPaid);
        assert_eq!(invoice.total_minor, Some(240_000));
        assert_eq!(invoice.amount_due_minor, Some(120_000));
        assert_eq!(invoice.amount_paid_minor, Some(120_000));
        assert!(invoice.payer_view_url.is_some());
    }
}
