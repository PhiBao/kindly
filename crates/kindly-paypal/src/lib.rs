//! PayPal integration for Kindly.
//!
//! Two implementations of [`PayPalApi`] ship in this crate:
//!
//! * [`live::LivePayPal`] — the real thing: OAuth client credentials, the
//!   Invoicing v2 API, Checkout Orders v2, and signature-verified webhooks.
//! * [`sim::SimPayPal`] — a faithful in-process simulator used by tests, by
//!   `kindly serve --sim`, and by anyone who wants to run Kindly without a
//!   PayPal developer account. It implements the same trait and emits the same
//!   webhook events through the same verification path.
//!
//! Nothing in the product logic knows which one it is talking to. That is on
//! purpose: the demo can be deterministic without the integration being fake.

pub mod error;
pub mod live;
pub mod model;
pub mod sim;
pub mod webhook;

pub use error::{PayPalError, PayPalResult};
pub use live::{LiveConfig, LivePayPal};
pub use model::*;
pub use sim::SimPayPal;

use async_trait::async_trait;

/// The PayPal surface Kindly depends on. Kept deliberately narrow: if it is not
/// here, Kindly does not do it.
#[async_trait]
pub trait PayPalApi: Send + Sync {
    /// Create a draft invoice mirroring one of Kindly's invoices.
    async fn create_invoice(&self, request: NewInvoice) -> PayPalResult<PayPalInvoice>;

    /// Send a draft invoice (PayPal emails the payer a hosted pay page link).
    async fn send_invoice(
        &self,
        paypal_invoice_id: &str,
        subject: &str,
        note: &str,
    ) -> PayPalResult<()>;

    /// Send PayPal's own reminder for an invoice (max two per day, per PayPal).
    async fn remind_invoice(
        &self,
        paypal_invoice_id: &str,
        subject: &str,
        note: &str,
    ) -> PayPalResult<()>;

    /// Record a payment against an invoice (full or partial).
    async fn record_payment(
        &self,
        paypal_invoice_id: &str,
        amount_minor: i64,
        currency: &str,
        method: &str,
        note: &str,
    ) -> PayPalResult<()>;

    async fn show_invoice(&self, paypal_invoice_id: &str) -> PayPalResult<PayPalInvoice>;

    async fn cancel_invoice(&self, paypal_invoice_id: &str) -> PayPalResult<()>;

    /// PNG bytes of the invoice QR code (the "scan to pay" surface).
    async fn generate_qr(&self, paypal_invoice_id: &str) -> PayPalResult<Vec<u8>>;

    /// List invoices (used for reconciliation when webhooks are missed).
    async fn list_invoices(&self, page_size: u8) -> PayPalResult<Vec<PayPalInvoice>>;

    /// Create a capture order for the client portal's "Pay now" button.
    async fn create_order(&self, request: NewOrder) -> PayPalResult<PayPalOrder>;

    /// Capture an approved order.
    async fn capture_order(&self, order_id: &str) -> PayPalResult<PayPalCapture>;

    /// Verify that a webhook delivery is authentic and return the parsed event.
    async fn verify_webhook(
        &self,
        headers: &WebhookHeaders,
        raw_body: &[u8],
    ) -> PayPalResult<VerifiedEvent>;
}
