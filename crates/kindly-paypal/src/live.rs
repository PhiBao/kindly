//! The live PayPal client: OAuth, Invoicing v2, Orders v2, webhook verification.

use crate::error::{PayPalError, PayPalResult};
use crate::model::*;
use crate::PayPalApi;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use reqwest::{Client, Method, StatusCode};
use serde_json::{json, Value};
use std::time::Duration;
use tokio::sync::Mutex;
use tracing::{debug, warn};

#[derive(Debug, Clone)]
pub struct LiveConfig {
    /// e.g. `https://api-m.sandbox.paypal.com`
    pub base_url: String,
    pub client_id: String,
    pub client_secret: String,
    /// Webhook id from the PayPal developer dashboard; required to verify
    /// webhook signatures.
    pub webhook_id: Option<String>,
    /// The PayPal account email invoices are issued from.
    pub invoicer_email: String,
    /// Optional `PayPal-Partner-Attribution-Id` (used by partners).
    pub partner_attribution_id: Option<String>,
}

impl LiveConfig {
    pub fn sandbox(
        client_id: impl Into<String>,
        client_secret: impl Into<String>,
        invoicer_email: impl Into<String>,
    ) -> Self {
        Self {
            base_url: "https://api-m.sandbox.paypal.com".to_string(),
            client_id: client_id.into(),
            client_secret: client_secret.into(),
            webhook_id: None,
            invoicer_email: invoicer_email.into(),
            partner_attribution_id: None,
        }
    }
}

#[derive(Debug, Clone)]
struct CachedToken {
    access_token: String,
    expires_at: DateTime<Utc>,
}

pub struct LivePayPal {
    http: Client,
    config: LiveConfig,
    token: Mutex<Option<CachedToken>>,
}

impl LivePayPal {
    pub fn new(config: LiveConfig) -> PayPalResult<Self> {
        let http = Client::builder()
            .timeout(Duration::from_secs(30))
            .connect_timeout(Duration::from_secs(10))
            .user_agent("kindly/0.1 (PayPal AI Hackathon)")
            .build()
            .map_err(|e| PayPalError::Config(format!("http client: {e}")))?;
        Ok(Self {
            http,
            config,
            token: Mutex::new(None),
        })
    }

    fn url(&self, path: &str) -> String {
        format!("{}{}", self.config.base_url.trim_end_matches('/'), path)
    }

    async fn access_token(&self) -> PayPalResult<String> {
        {
            let guard = self.token.lock().await;
            if let Some(cached) = guard.as_ref() {
                if cached.expires_at > Utc::now() + chrono::Duration::seconds(60) {
                    return Ok(cached.access_token.clone());
                }
            }
        }

        let response = self
            .http
            .post(self.url("/v1/oauth2/token"))
            .basic_auth(&self.config.client_id, Some(&self.config.client_secret))
            .header("Accept", "application/json")
            .form(&[("grant_type", "client_credentials")])
            .send()
            .await
            .map_err(|e| PayPalError::Transport(e.to_string()))?;

        let status = response.status();
        let body: Value = response
            .json()
            .await
            .map_err(|e| PayPalError::Shape(format!("oauth response: {e}")))?;

        if !status.is_success() {
            let message = body
                .get("error_description")
                .and_then(|v| v.as_str())
                .unwrap_or("oauth failed")
                .to_string();
            return Err(PayPalError::Auth(message));
        }

        let access_token = body
            .get("access_token")
            .and_then(|v| v.as_str())
            .ok_or_else(|| PayPalError::Shape("oauth response missing access_token".into()))?
            .to_string();
        let expires_in = body
            .get("expires_in")
            .and_then(|v| v.as_i64())
            .unwrap_or(3_600);

        let cached = CachedToken {
            access_token: access_token.clone(),
            expires_at: Utc::now() + chrono::Duration::seconds(expires_in.max(60)),
        };
        *self.token.lock().await = Some(cached);
        Ok(access_token)
    }

    /// One API call with auth, idempotency key, and bounded retries.
    async fn call(
        &self,
        method: Method,
        path: &str,
        request_id: Option<&str>,
        body: Option<&Value>,
    ) -> PayPalResult<(StatusCode, Value, Vec<u8>)> {
        let mut attempt = 0u32;
        let mut backoff = Duration::from_millis(250);
        loop {
            attempt += 1;
            let token = self.access_token().await?;
            let mut request = self
                .http
                .request(method.clone(), self.url(path))
                .bearer_auth(&token)
                .header("Accept", "application/json");
            if let Some(id) = request_id {
                request = request.header("PayPal-Request-Id", id);
            }
            if let Some(attr) = &self.config.partner_attribution_id {
                request = request.header("PayPal-Partner-Attribution-Id", attr);
            }
            if let Some(body) = body {
                request = request.json(body);
            }

            let response = match request.send().await {
                Ok(response) => response,
                Err(error) => {
                    if attempt < 3 {
                        warn!(attempt, error = %error, "paypal transport failure; retrying");
                        tokio::time::sleep(backoff).await;
                        backoff *= 4;
                        continue;
                    }
                    return Err(PayPalError::Transport(error.to_string()));
                }
            };

            let status = response.status();
            let retry_after = response
                .headers()
                .get("retry-after")
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.parse::<u64>().ok());
            let bytes = response
                .bytes()
                .await
                .map_err(|e| PayPalError::Transport(e.to_string()))?
                .to_vec();
            let value: Value = if bytes.is_empty() {
                Value::Null
            } else {
                serde_json::from_slice(&bytes).unwrap_or(Value::Null)
            };

            if status.is_success() {
                debug!(path, status = status.as_u16(), "paypal call ok");
                return Ok((status, value, bytes));
            }

            let issue = value
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown")
                .to_string();
            let message = value
                .get("message")
                .and_then(|v| v.as_str())
                .or_else(|| {
                    value
                        .get("details")
                        .and_then(|d| d.as_array())
                        .and_then(|a| a.first())
                        .and_then(|d| d.get("description"))
                        .and_then(|v| v.as_str())
                })
                .unwrap_or("paypal api error")
                .to_string();

            let retryable = matches!(status.as_u16(), 429 | 500 | 502 | 503 | 504);
            if retryable && attempt < 3 {
                let wait = retry_after
                    .map(Duration::from_secs)
                    .unwrap_or(backoff)
                    .min(Duration::from_secs(5));
                warn!(
                    attempt,
                    status = status.as_u16(),
                    "paypal transient failure; retrying"
                );
                tokio::time::sleep(wait).await;
                backoff *= 4;
                continue;
            }

            if status == StatusCode::NOT_FOUND {
                return Err(PayPalError::NotFound(path.to_string()));
            }
            return Err(PayPalError::Api {
                status: status.as_u16(),
                issue,
                message,
            });
        }
    }
}

#[async_trait]
impl PayPalApi for LivePayPal {
    async fn create_invoice(&self, request: NewInvoice) -> PayPalResult<PayPalInvoice> {
        let body = PayPalInvoice::build_create_body(&request);
        let request_id = format!("kindly-inv-{}", uuid::Uuid::now_v7());
        let (_, value, _) = self
            .call(
                Method::POST,
                "/v2/invoicing/invoices",
                Some(&request_id),
                Some(&body),
            )
            .await?;
        PayPalInvoice::from_resource(&value)
    }

    async fn send_invoice(
        &self,
        paypal_invoice_id: &str,
        subject: &str,
        note: &str,
    ) -> PayPalResult<()> {
        let body = json!({
            "send_to_invoicer": false,
            "send_to_recipient": true,
            "subject": subject,
            "note": note,
        });
        self.call(
            Method::POST,
            &format!("/v2/invoicing/invoices/{paypal_invoice_id}/send"),
            None,
            Some(&body),
        )
        .await
        .map(|_| ())
    }

    async fn remind_invoice(
        &self,
        paypal_invoice_id: &str,
        subject: &str,
        note: &str,
    ) -> PayPalResult<()> {
        let body = json!({
            "subject": subject,
            "note": note,
            "send_to_invoicer": false,
            "additional_recipients": [],
        });
        self.call(
            Method::POST,
            &format!("/v2/invoicing/invoices/{paypal_invoice_id}/remind"),
            None,
            Some(&body),
        )
        .await
        .map(|_| ())
    }

    async fn record_payment(
        &self,
        paypal_invoice_id: &str,
        amount_minor: i64,
        currency: &str,
        method: &str,
        note: &str,
    ) -> PayPalResult<()> {
        let body = json!({
            "method": method,
            "payment_date": Utc::now().format("%Y-%m-%d").to_string(),
            "amount": {
                "currency_code": currency,
                "value": format_minor(amount_minor, currency),
            },
            "note": note,
        });
        self.call(
            Method::POST,
            &format!("/v2/invoicing/invoices/{paypal_invoice_id}/payments"),
            None,
            Some(&body),
        )
        .await
        .map(|_| ())
    }

    async fn show_invoice(&self, paypal_invoice_id: &str) -> PayPalResult<PayPalInvoice> {
        let (_, value, _) = self
            .call(
                Method::GET,
                &format!("/v2/invoicing/invoices/{paypal_invoice_id}"),
                None,
                None,
            )
            .await?;
        PayPalInvoice::from_resource(&value)
    }

    async fn cancel_invoice(&self, paypal_invoice_id: &str) -> PayPalResult<()> {
        let body = json!({ "subject": "Invoice cancelled", "note": "Cancelled by the sender." });
        self.call(
            Method::POST,
            &format!("/v2/invoicing/invoices/{paypal_invoice_id}/cancel"),
            None,
            Some(&body),
        )
        .await
        .map(|_| ())
    }

    async fn generate_qr(&self, paypal_invoice_id: &str) -> PayPalResult<Vec<u8>> {
        let (_, _, bytes) = self
            .call(
                Method::POST,
                &format!("/v2/invoicing/invoices/{paypal_invoice_id}/generate-qr-code"),
                None,
                Some(&json!({ "width": 300, "height": 300 })),
            )
            .await?;
        Ok(bytes)
    }

    async fn list_invoices(&self, page_size: u8) -> PayPalResult<Vec<PayPalInvoice>> {
        let (_, value, _) = self
            .call(
                Method::GET,
                &format!(
                    "/v2/invoicing/invoices?page=1&page_size={}&total_required=false",
                    page_size.clamp(1, 100)
                ),
                None,
                None,
            )
            .await?;
        let items = value
            .get("items")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        items
            .iter()
            .map(PayPalInvoice::from_resource)
            .collect::<PayPalResult<Vec<_>>>()
    }

    async fn create_order(&self, request: NewOrder) -> PayPalResult<PayPalOrder> {
        let body = json!({
            "intent": "CAPTURE",
            "purchase_units": [{
                "custom_id": request.custom_id,
                "description": request.description,
                "amount": {
                    "currency_code": request.currency_code,
                    "value": format_minor(request.amount_minor, &request.currency_code),
                }
            }],
        });
        let request_id = format!("kindly-order-{}", uuid::Uuid::now_v7());
        let (_, value, _) = self
            .call(
                Method::POST,
                "/v2/checkout/orders",
                Some(&request_id),
                Some(&body),
            )
            .await?;
        let id = value
            .get("id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| PayPalError::Shape("order missing id".into()))?
            .to_string();
        let status = value
            .get("status")
            .and_then(|v| v.as_str())
            .unwrap_or("CREATED")
            .to_string();
        Ok(PayPalOrder {
            id,
            status,
            raw: value,
        })
    }

    async fn capture_order(&self, order_id: &str) -> PayPalResult<PayPalCapture> {
        let (_, value, _) = self
            .call(
                Method::POST,
                &format!("/v2/checkout/orders/{order_id}/capture"),
                Some(&format!("kindly-capture-{order_id}")),
                Some(&json!({})),
            )
            .await?;
        let capture = value
            .get("purchase_units")
            .and_then(|v| v.as_array())
            .and_then(|units| units.first())
            .and_then(|unit| unit.get("payments"))
            .and_then(|p| p.get("captures"))
            .and_then(|c| c.as_array())
            .and_then(|captures| captures.first())
            .ok_or_else(|| PayPalError::Shape("capture response missing captures".into()))?;

        let capture_id = capture
            .get("id")
            .and_then(|v| v.as_str())
            .unwrap_or(order_id)
            .to_string();
        let status = capture
            .get("status")
            .and_then(|v| v.as_str())
            .unwrap_or("UNKNOWN")
            .to_string();
        let currency_code = capture
            .get("amount")
            .and_then(|a| a.get("currency_code"))
            .and_then(|v| v.as_str())
            .unwrap_or("USD")
            .to_string();
        let amount_minor = capture
            .get("amount")
            .and_then(|a| a.get("value"))
            .and_then(|v| v.as_str())
            .and_then(|v| parse_minor(v, &currency_code).ok())
            .unwrap_or(0);

        Ok(PayPalCapture {
            order_id: order_id.to_string(),
            capture_id,
            status,
            amount_minor,
            currency_code,
            raw: value,
        })
    }

    async fn verify_webhook(
        &self,
        headers: &WebhookHeaders,
        raw_body: &[u8],
    ) -> PayPalResult<VerifiedEvent> {
        let webhook_id = self.config.webhook_id.as_ref().ok_or_else(|| {
            PayPalError::Config("PAYPAL_WEBHOOK_ID is required to verify webhooks".into())
        })?;

        let event: Value = serde_json::from_slice(raw_body)
            .map_err(|e| PayPalError::Shape(format!("webhook body is not json: {e}")))?;

        let transmission_id = headers
            .transmission_id
            .clone()
            .ok_or_else(|| PayPalError::SignatureInvalid("missing transmission id".into()))?;
        let transmission_time = headers
            .transmission_time
            .clone()
            .ok_or_else(|| PayPalError::SignatureInvalid("missing transmission time".into()))?;
        let cert_url = headers
            .cert_url
            .clone()
            .ok_or_else(|| PayPalError::SignatureInvalid("missing cert url".into()))?;
        let auth_algo = headers
            .auth_algo
            .clone()
            .ok_or_else(|| PayPalError::SignatureInvalid("missing auth algo".into()))?;
        let transmission_sig = headers.transmission_sig.clone().ok_or_else(|| {
            PayPalError::SignatureInvalid("missing transmission signature".into())
        })?;

        let body = json!({
            "transmission_id": transmission_id,
            "transmission_time": transmission_time,
            "cert_url": cert_url,
            "auth_algo": auth_algo,
            "transmission_sig": transmission_sig,
            "webhook_id": webhook_id,
            "webhook_event": event,
        });

        let (_, response, _) = self
            .call(
                Method::POST,
                "/v1/notifications/verify-webhook-signature",
                None,
                Some(&body),
            )
            .await?;

        let status = response
            .get("verification_status")
            .and_then(|v| v.as_str())
            .unwrap_or("FAILURE");
        if status != "SUCCESS" {
            return Err(PayPalError::SignatureInvalid(format!(
                "paypal reported verification_status={status}"
            )));
        }

        parse_event(&event, true)
    }
}

/// Shared parsing of a PayPal webhook envelope.
pub fn parse_event(event: &Value, verified: bool) -> PayPalResult<VerifiedEvent> {
    let event_id = event
        .get("id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| PayPalError::Shape("webhook missing id".into()))?
        .to_string();
    let event_type = event
        .get("event_type")
        .and_then(|v| v.as_str())
        .ok_or_else(|| PayPalError::Shape("webhook missing event_type".into()))?
        .to_string();
    let resource_type = event
        .get("resource_type")
        .and_then(|v| v.as_str())
        .map(str::to_string);
    let resource = event.get("resource").cloned().unwrap_or(Value::Null);
    let received_at = event
        .get("create_time")
        .and_then(|v| v.as_str())
        .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
        .map(|dt| dt.with_timezone(&Utc));

    Ok(VerifiedEvent {
        event_id,
        event_type,
        resource_type,
        resource,
        verified,
        raw: event.clone(),
        received_at,
    })
}
