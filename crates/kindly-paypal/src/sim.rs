//! A faithful in-process PayPal simulator.
//!
//! Why this exists:
//! * judges and reviewers can run Kindly end-to-end with zero credentials;
//! * automated tests exercise the *whole* pipeline (invoice lifecycle →
//!   webhook signature verification → ledger) deterministically;
//! * the demo can be replayed without burning sandbox state.
//!
//! It mirrors the live behaviour Kindly depends on, including PayPal's real
//! constraint that an invoice can only be reminded twice per day. Webhook
//! deliveries are HMAC-signed and verified through the same code path as live
//! deliveries. `kindly serve --paypal sim` prints a visible banner so nobody
//! can mistake simulated money for real money.

use crate::error::{PayPalError, PayPalResult};
use crate::live::parse_event;
use crate::model::*;
use crate::PayPalApi;
use async_trait::async_trait;
use base64::Engine;
use chrono::{DateTime, NaiveDate, Utc};
use hmac::{Hmac, Mac};
use serde_json::{json, Value};
use sha2::Sha256;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use tokio::sync::Mutex as AsyncMutex;

type HmacSha256 = Hmac<Sha256>;

#[derive(Debug, Clone)]
struct SimInvoice {
    id: String,
    number: String,
    status: PayPalInvoiceStatus,
    currency_code: String,
    lines: Vec<NewInvoiceLine>,
    total_minor: i64,
    paid_minor: i64,
    due_date: NaiveDate,
    client_email: String,
    client_name: Option<String>,
    memo: Option<String>,
    note: Option<String>,
    created_at: DateTime<Utc>,
    reminders: Vec<DateTime<Utc>>,
    payer_view_url: Option<String>,
}

impl SimInvoice {
    fn outstanding(&self) -> i64 {
        (self.total_minor - self.paid_minor).max(0)
    }

    fn to_resource(&self) -> Value {
        let items: Vec<Value> = self
            .lines
            .iter()
            .map(|line| {
                json!({
                    "name": line.name,
                    "description": line.description,
                    "quantity": line.quantity,
                    "unit_amount": {
                        "currency_code": self.currency_code,
                        "value": format_minor(line.unit_amount_minor, &self.currency_code),
                    }
                })
            })
            .collect();
        json!({
            "id": self.id,
            "status": self.status.as_str(),
            "detail": {
                "currency_code": self.currency_code,
                "invoice_number": self.number,
                "memo": self.memo,
                "note": self.note,
                "payment_term": {
                    "term_type": "DUE_ON_DATE_SPECIFIED",
                    "due_date": self.due_date.format("%Y-%m-%d").to_string(),
                },
                "metadata": {
                    "payer_view_url": self.payer_view_url,
                    "create_time": self.created_at.to_rfc3339(),
                }
            },
            "primary_recipients": [{
                "billing_info": {
                    "email_address": self.client_email,
                    "name": self.client_name.clone().map(|n| json!({"full_name": n})),
                }
            }],
            "items": items,
            "amount": {
                "currency_code": self.currency_code,
                "value": format_minor(self.total_minor, &self.currency_code),
                "breakdown": {
                    "item_total": {"currency_code": self.currency_code, "value": format_minor(self.total_minor, &self.currency_code)},
                    "outstanding_balance": {"currency_code": self.currency_code, "value": format_minor(self.outstanding(), &self.currency_code)},
                    "total_paid_amount": {"currency_code": self.currency_code, "value": format_minor(self.paid_minor, &self.currency_code)},
                }
            }
        })
    }
}

#[derive(Debug, Clone)]
struct SimOrder {
    id: String,
    amount_minor: i64,
    currency_code: String,
    description: String,
    custom_id: String,
    status: String,
    captured: Option<(String, DateTime<Utc>)>,
}

#[derive(Default)]
struct SimState {
    invoices: HashMap<String, SimInvoice>,
    orders: HashMap<String, SimOrder>,
}

pub struct SimPayPal {
    state: Mutex<SimState>,
    counter: AtomicU64,
    /// Secret used to sign simulated webhook deliveries.
    webhook_secret: Vec<u8>,
    /// Internal lock so `pay_invoice` can be async-safe.
    _guard: AsyncMutex<()>,
}

impl Default for SimPayPal {
    fn default() -> Self {
        Self::new("kindly-sim-webhook-secret")
    }
}

impl SimPayPal {
    pub fn new(webhook_secret: impl Into<String>) -> Self {
        Self {
            state: Mutex::new(SimState::default()),
            counter: AtomicU64::new(1),
            webhook_secret: webhook_secret.into().into_bytes(),
            _guard: AsyncMutex::new(()),
        }
    }

    fn next_id(&self, prefix: &str) -> String {
        let n = self.counter.fetch_add(1, Ordering::SeqCst);
        format!("{prefix}{n:06}")
    }

    /// Compute the simulator webhook signature for a raw body.
    pub fn sign_body(&self, body: &[u8]) -> String {
        let mut mac = HmacSha256::new_from_slice(&self.webhook_secret).expect("hmac key");
        mac.update(body);
        let digest = mac.finalize().into_bytes();
        base64::engine::general_purpose::STANDARD.encode(digest)
    }

    fn invoice_or_err(state: &SimState, id: &str) -> PayPalResult<SimInvoice> {
        state
            .invoices
            .get(id)
            .cloned()
            .ok_or_else(|| PayPalError::NotFound(format!("sim invoice {id}")))
    }

    fn envelope(event_type: &str, resource_type: &str, resource: Value) -> Value {
        json!({
            "id": format!("WH-{}", uuid::Uuid::now_v7()),
            "create_time": Utc::now().to_rfc3339(),
            "event_type": event_type,
            "resource_type": resource_type,
            "resource": resource,
            "summary": event_type,
        })
    }

    /// A client pays an invoice. Returns the webhook deliveries PayPal would
    /// send, each already shaped like a real delivery.
    pub fn pay_invoice(&self, invoice_id: &str, amount_minor: i64) -> PayPalResult<Vec<Value>> {
        let mut state = self.state.lock().expect("sim state");
        let mut invoice = Self::invoice_or_err(&state, invoice_id)?;
        if amount_minor <= 0 {
            return Err(PayPalError::Api {
                status: 422,
                issue: "INVALID_REQUEST".into(),
                message: "payment amount must be positive".into(),
            });
        }
        if amount_minor > invoice.outstanding() {
            return Err(PayPalError::Api {
                status: 422,
                issue: "AMOUNT_EXCEEDS_OUTSTANDING".into(),
                message: "payment exceeds the outstanding balance".into(),
            });
        }

        invoice.paid_minor += amount_minor;
        invoice.status = if invoice.outstanding() == 0 {
            PayPalInvoiceStatus::Paid
        } else {
            PayPalInvoiceStatus::PartiallyPaid
        };
        let resource = invoice.to_resource();
        state.invoices.insert(invoice_id.to_string(), invoice);

        let event_type = if resource["status"] == "PAID" {
            "INVOICING.INVOICE.PAID"
        } else {
            "INVOICING.INVOICE.PAID"
        };
        Ok(vec![Self::envelope(event_type, "invoice", resource)])
    }

    /// A client pays a portal order; returns the capture resource and the
    /// webhook deliveries PayPal would send.
    pub fn capture_order_sim(&self, order_id: &str) -> PayPalResult<(Value, Vec<Value>)> {
        let mut state = self.state.lock().expect("sim state");
        let mut order = state
            .orders
            .get(order_id)
            .cloned()
            .ok_or_else(|| PayPalError::NotFound(format!("sim order {order_id}")))?;
        if order.captured.is_none() {
            order.status = "COMPLETED".into();
            order.captured = Some((self.next_id("SIMCAP-"), Utc::now()));
        }
        let (capture_id, at) = order.captured.clone().expect("captured");
        state.orders.insert(order_id.to_string(), order.clone());

        let capture = json!({
            "id": capture_id,
            "status": "COMPLETED",
            "amount": {
                "currency_code": order.currency_code,
                "value": format_minor(order.amount_minor, &order.currency_code),
            },
            "create_time": at.to_rfc3339(),
            "custom_id": order.custom_id,
            "description": order.description,
        });
        let event = Self::envelope(
            "PAYMENT.CAPTURE.COMPLETED",
            "capture",
            json!({
                "id": capture_id,
                "status": "COMPLETED",
                "amount": capture["amount"],
                "custom_id": order.custom_id,
                "supplementary_data": {
                    "related_ids": { "order_id": order.id }
                }
            }),
        );
        Ok((capture, vec![event]))
    }

    /// Recreate a sim-side invoice object with a known id. Used at boot so the
    /// simulator mirrors the database after a restart.
    #[allow(clippy::too_many_arguments)]
    pub fn import_invoice(
        &self,
        id: &str,
        number: &str,
        status: PayPalInvoiceStatus,
        currency: &str,
        total_minor: i64,
        paid_minor: i64,
        due_date: NaiveDate,
        client_email: &str,
        client_name: Option<&str>,
        memo: Option<&str>,
        quantity: &str,
        unit_amount_minor: i64,
    ) -> PayPalResult<()> {
        let invoice = SimInvoice {
            id: id.to_string(),
            number: number.to_string(),
            status,
            currency_code: currency.to_string(),
            lines: vec![NewInvoiceLine {
                name: memo.unwrap_or(number).to_string(),
                description: None,
                quantity: quantity.to_string(),
                unit_amount_minor,
            }],
            total_minor,
            paid_minor,
            due_date,
            client_email: client_email.to_string(),
            client_name: client_name.map(str::to_string),
            memo: memo.map(str::to_string),
            note: None,
            created_at: Utc::now(),
            reminders: Vec::new(),
            payer_view_url: None,
        };
        self.state
            .lock()
            .expect("sim state")
            .invoices
            .insert(id.to_string(), invoice);
        Ok(())
    }

    /// Force an invoice into a status for scenario testing (e.g. marking it
    /// overdue). Simulator-only.
    pub fn force_status(&self, invoice_id: &str, status: PayPalInvoiceStatus) -> PayPalResult<()> {
        let mut state = self.state.lock().expect("sim state");
        let mut invoice = Self::invoice_or_err(&state, invoice_id)?;
        invoice.status = status;
        state.invoices.insert(invoice_id.to_string(), invoice);
        Ok(())
    }

    pub fn invoice_count(&self) -> usize {
        self.state.lock().expect("sim state").invoices.len()
    }
}

#[async_trait]
impl PayPalApi for SimPayPal {
    async fn create_invoice(&self, request: NewInvoice) -> PayPalResult<PayPalInvoice> {
        if request.lines.is_empty() {
            return Err(PayPalError::Api {
                status: 422,
                issue: "INVALID_REQUEST".into(),
                message: "an invoice needs at least one line".into(),
            });
        }
        let total: i64 = request
            .lines
            .iter()
            .map(|line| {
                let quantity_milli = parse_quantity_milli(&line.quantity).unwrap_or(1000);
                ((quantity_milli as i128 * line.unit_amount_minor as i128) / 1000) as i64
            })
            .sum();

        let id = self.next_id("SIM-INV2-");
        let invoice = SimInvoice {
            id: id.clone(),
            number: request.invoice_number.clone(),
            status: PayPalInvoiceStatus::Draft,
            currency_code: request.currency_code.clone(),
            lines: request.lines.clone(),
            total_minor: total,
            paid_minor: 0,
            due_date: request.due_date,
            client_email: request.client_email.clone(),
            client_name: request.client_name.clone(),
            memo: request.memo.clone(),
            note: request.note.clone(),
            created_at: Utc::now(),
            reminders: Vec::new(),
            payer_view_url: None,
        };
        let resource = invoice.to_resource();
        self.state.lock().expect("sim state").invoices.insert(id, invoice);
        PayPalInvoice::from_resource(&resource)
    }

    async fn send_invoice(&self, paypal_invoice_id: &str, _subject: &str, _note: &str) -> PayPalResult<()> {
        let mut state = self.state.lock().expect("sim state");
        let mut invoice = Self::invoice_or_err(&state, paypal_invoice_id)?;
        if invoice.status != PayPalInvoiceStatus::Draft && invoice.status != PayPalInvoiceStatus::Scheduled {
            return Err(PayPalError::Api {
                status: 422,
                issue: "INVOICE_ALREADY_SENT".into(),
                message: format!("invoice is {}", invoice.status.as_str()),
            });
        }
        invoice.status = PayPalInvoiceStatus::Sent;
        state.invoices.insert(paypal_invoice_id.to_string(), invoice);
        Ok(())
    }

    async fn remind_invoice(&self, paypal_invoice_id: &str, _subject: &str, _note: &str) -> PayPalResult<()> {
        let mut state = self.state.lock().expect("sim state");
        let mut invoice = Self::invoice_or_err(&state, paypal_invoice_id)?;
        let today = Utc::now().date_naive();
        let reminders_today = invoice
            .reminders
            .iter()
            .filter(|r| r.date_naive() == today)
            .count();
        if reminders_today >= 2 {
            // Mirrors the real API: only two reminders per invoice per day.
            return Err(PayPalError::Api {
                status: 422,
                issue: "REMINDER_LIMIT_EXCEEDED".into(),
                message: "an invoice can only be reminded twice in a day".into(),
            });
        }
        invoice.reminders.push(Utc::now());
        state.invoices.insert(paypal_invoice_id.to_string(), invoice);
        Ok(())
    }

    async fn record_payment(
        &self,
        paypal_invoice_id: &str,
        amount_minor: i64,
        currency: &str,
        _method: &str,
        _note: &str,
    ) -> PayPalResult<()> {
        let mut state = self.state.lock().expect("sim state");
        let mut invoice = Self::invoice_or_err(&state, paypal_invoice_id)?;
        if !invoice.currency_code.eq_ignore_ascii_case(currency) {
            return Err(PayPalError::Api {
                status: 422,
                issue: "CURRENCY_MISMATCH".into(),
                message: format!("invoice is {}", invoice.currency_code),
            });
        }
        if amount_minor > invoice.outstanding() {
            return Err(PayPalError::Api {
                status: 422,
                issue: "AMOUNT_EXCEEDS_OUTSTANDING".into(),
                message: "payment exceeds the outstanding balance".into(),
            });
        }
        invoice.paid_minor += amount_minor;
        invoice.status = if invoice.outstanding() == 0 {
            PayPalInvoiceStatus::MarkedAsPaid
        } else {
            PayPalInvoiceStatus::PartiallyPaid
        };
        state.invoices.insert(paypal_invoice_id.to_string(), invoice);
        Ok(())
    }

    async fn show_invoice(&self, paypal_invoice_id: &str) -> PayPalResult<PayPalInvoice> {
        let state = self.state.lock().expect("sim state");
        let invoice = Self::invoice_or_err(&state, paypal_invoice_id)?;
        PayPalInvoice::from_resource(&invoice.to_resource())
    }

    async fn cancel_invoice(&self, paypal_invoice_id: &str) -> PayPalResult<()> {
        let mut state = self.state.lock().expect("sim state");
        let mut invoice = Self::invoice_or_err(&state, paypal_invoice_id)?;
        invoice.status = PayPalInvoiceStatus::Cancelled;
        state.invoices.insert(paypal_invoice_id.to_string(), invoice);
        Ok(())
    }

    async fn generate_qr(&self, _paypal_invoice_id: &str) -> PayPalResult<Vec<u8>> {
        // A valid 1x1 PNG so the API surface stays honest without shipping
        // binary fixtures around.
        const TINY_PNG: &[u8] = &[
            0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44,
            0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00, 0x00, 0x1F,
            0x15, 0xC4, 0x89, 0x00, 0x00, 0x00, 0x0A, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9C, 0x63, 0x00,
            0x01, 0x00, 0x00, 0x05, 0x00, 0x01, 0x0D, 0x0A, 0x2D, 0xB4, 0x00, 0x00, 0x00, 0x00, 0x49,
            0x45, 0x4E, 0x44, 0xAE, 0x42, 0x60, 0x82,
        ];
        Ok(TINY_PNG.to_vec())
    }

    async fn list_invoices(&self, page_size: u8) -> PayPalResult<Vec<PayPalInvoice>> {
        let state = self.state.lock().expect("sim state");
        let mut invoices: Vec<&SimInvoice> = state.invoices.values().collect();
        invoices.sort_by_key(|i| i.created_at);
        invoices
            .into_iter()
            .take(page_size as usize)
            .map(|i| PayPalInvoice::from_resource(&i.to_resource()))
            .collect()
    }

    async fn create_order(&self, request: NewOrder) -> PayPalResult<PayPalOrder> {
        let id = self.next_id("SIM-ORDER-");
        let order = SimOrder {
            id: id.clone(),
            amount_minor: request.amount_minor,
            currency_code: request.currency_code.clone(),
            description: request.description.clone(),
            custom_id: request.custom_id.clone(),
            status: "CREATED".into(),
            captured: None,
        };
        self.state.lock().expect("sim state").orders.insert(id.clone(), order);
        Ok(PayPalOrder {
            id: id.clone(),
            status: "CREATED".into(),
            raw: json!({ "id": id, "status": "CREATED" }),
        })
    }

    async fn capture_order(&self, order_id: &str) -> PayPalResult<PayPalCapture> {
        let (capture, _events) = self.capture_order_sim(order_id)?;
        let amount_minor = capture["amount"]["value"]
            .as_str()
            .and_then(|v| parse_minor(v, "USD").ok())
            .unwrap_or(0);
        Ok(PayPalCapture {
            order_id: order_id.to_string(),
            capture_id: capture["id"].as_str().unwrap_or_default().to_string(),
            status: capture["status"].as_str().unwrap_or("COMPLETED").to_string(),
            amount_minor,
            currency_code: capture["amount"]["currency_code"]
                .as_str()
                .unwrap_or("USD")
                .to_string(),
            raw: capture,
        })
    }

    async fn verify_webhook(
        &self,
        headers: &WebhookHeaders,
        raw_body: &[u8],
    ) -> PayPalResult<VerifiedEvent> {
        let signature = headers
            .sim_signature
            .clone()
            .ok_or_else(|| PayPalError::SignatureInvalid("sim deliveries require x-kindly-sim-signature".into()))?;
        let expected = self.sign_body(raw_body);
        if !constant_time_eq(signature.as_bytes(), expected.as_bytes()) {
            return Err(PayPalError::SignatureInvalid("sim signature mismatch".into()));
        }
        let event: Value = serde_json::from_slice(raw_body)
            .map_err(|e| PayPalError::Shape(format!("sim webhook body is not json: {e}")))?;
        parse_event(&event, true)
    }
}

fn parse_quantity_milli(value: &str) -> Option<i64> {
    let (whole, frac) = match value.split_once('.') {
        Some((w, f)) => (w.to_string(), f.to_string()),
        None => (value.to_string(), String::new()),
    };
    let whole: i64 = whole.parse().ok()?;
    let mut frac = frac;
    while frac.len() < 3 {
        frac.push('0');
    }
    frac.truncate(3);
    let frac: i64 = frac.parse().ok()?;
    Some(whole * 1000 + frac)
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_request() -> NewInvoice {
        NewInvoice {
            invoice_number: "1042".into(),
            currency_code: "USD".into(),
            client_email: "ap@acme.test".into(),
            client_name: Some("Acme Co".into()),
            invoicer_email: "maya@studio.test".into(),
            due_date: NaiveDate::from_ymd_opt(2026, 10, 1).unwrap(),
            memo: None,
            note: None,
            lines: vec![NewInvoiceLine {
                name: "Design sprint".into(),
                description: None,
                quantity: "1".into(),
                unit_amount_minor: 240_000,
            }],
            allow_partial_payment: true,
        }
    }

    #[tokio::test]
    async fn full_invoice_lifecycle() {
        let paypal = SimPayPal::default();
        let invoice = paypal.create_invoice(sample_request()).await.unwrap();
        assert_eq!(invoice.status, PayPalInvoiceStatus::Draft);
        assert_eq!(invoice.total_minor, Some(240_000));

        paypal.send_invoice(&invoice.id, "Invoice 1042", "Thanks!").await.unwrap();
        let sent = paypal.show_invoice(&invoice.id).await.unwrap();
        assert_eq!(sent.status, PayPalInvoiceStatus::Sent);

        // partial payment
        let events = paypal.pay_invoice(&invoice.id, 120_000).unwrap();
        assert_eq!(events.len(), 1);
        let after_partial = paypal.show_invoice(&invoice.id).await.unwrap();
        assert_eq!(after_partial.status, PayPalInvoiceStatus::PartiallyPaid);
        assert_eq!(after_partial.amount_due_minor, Some(120_000));

        // overpayment is refused
        assert!(paypal.pay_invoice(&invoice.id, 999_999).is_err());

        let events = paypal.pay_invoice(&invoice.id, 120_000).unwrap();
        assert_eq!(events[0]["event_type"], "INVOICING.INVOICE.PAID");
        let paid = paypal.show_invoice(&invoice.id).await.unwrap();
        assert!(paid.status.is_settled());
        assert_eq!(paid.amount_due_minor, Some(0));
    }

    #[tokio::test]
    async fn reminders_are_capped_like_paypal() {
        let paypal = SimPayPal::default();
        let invoice = paypal.create_invoice(sample_request()).await.unwrap();
        paypal.send_invoice(&invoice.id, "s", "n").await.unwrap();
        paypal.remind_invoice(&invoice.id, "s", "n").await.unwrap();
        paypal.remind_invoice(&invoice.id, "s", "n").await.unwrap();
        let third = paypal.remind_invoice(&invoice.id, "s", "n").await;
        assert!(third.is_err(), "third reminder in a day must fail");
    }

    #[tokio::test]
    async fn webhook_signature_is_verified() {
        let paypal = SimPayPal::default();
        let invoice = paypal.create_invoice(sample_request()).await.unwrap();
        let events = paypal.pay_invoice(&invoice.id, 240_000).unwrap();
        let body = serde_json::to_vec(&events[0]).unwrap();

        let good = WebhookHeaders {
            sim_signature: Some(paypal.sign_body(&body)),
            ..Default::default()
        };
        let verified = paypal.verify_webhook(&good, &body).await.unwrap();
        assert_eq!(verified.event_type, "INVOICING.INVOICE.PAID");
        assert!(verified.verified);
        assert_eq!(verified.paypal_invoice_id().as_deref(), Some(invoice.id.as_str()));

        let bad = WebhookHeaders {
            sim_signature: Some("not-a-signature".into()),
            ..Default::default()
        };
        assert!(paypal.verify_webhook(&bad, &body).await.is_err());
    }

    #[tokio::test]
    async fn orders_capture_and_emit_events() {
        let paypal = SimPayPal::default();
        let order = paypal
            .create_order(NewOrder {
                currency_code: "USD".into(),
                amount_minor: 120_000,
                description: "Invoice 1042 — partial payment".into(),
                custom_id: "inv:1042".into(),
            })
            .await
            .unwrap();
        let (capture, events) = paypal.capture_order_sim(&order.id).unwrap();
        assert_eq!(capture["status"], "COMPLETED");
        assert_eq!(events[0]["event_type"], "PAYMENT.CAPTURE.COMPLETED");
        // capturing twice returns the same capture (idempotent)
        let (again, _) = paypal.capture_order_sim(&order.id).unwrap();
        assert_eq!(capture["id"], again["id"]);
    }
}
