//! Runtime configuration, read from the environment.

use std::path::PathBuf;

/// Whether Kindly may create PayPal Invoice objects, or should settle through
/// Checkout payment links instead.
///
/// `Auto` asks for an invoice first and falls back to a Checkout order when the
/// app is not permitted to use Invoicing (PayPal returns 403 NOT_AUTHORIZED).
/// That keeps a judge-facing deployment working with ordinary merchant
/// credentials while still preferring real invoices when they are available.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InvoicingMode {
    Auto,
    On,
    Off,
}

impl InvoicingMode {
    pub fn label(self) -> &'static str {
        match self {
            InvoicingMode::Auto => "auto",
            InvoicingMode::On => "required",
            InvoicingMode::Off => "off",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PayPalMode {
    /// The real PayPal APIs (sandbox or live, depending on the base URL).
    Live,
    /// The in-process simulator: everything works, nothing leaves the machine.
    Sim,
}

impl PayPalMode {
    pub fn label(self) -> &'static str {
        match self {
            PayPalMode::Live => "live",
            PayPalMode::Sim => "simulation",
        }
    }
}

#[derive(Debug, Clone)]
pub struct Config {
    pub database_url: String,
    pub port: u16,
    pub paypal_mode: PayPalMode,
    /// Create real PayPal invoice objects, or settle with Checkout links.
    pub invoicing: InvoicingMode,
    pub paypal_base_url: String,
    pub paypal_client_id: String,
    pub paypal_client_secret: String,
    pub paypal_webhook_id: Option<String>,
    pub invoicer_email: String,
    pub owner_name: String,
    pub owner_business: String,
    /// Shared secret used to sign client-portal capability URLs.
    pub portal_secret: String,
    /// Shared secret used by the simulator to sign its webhook deliveries.
    pub sim_webhook_secret: String,
    /// Token the owner uses to reach the dashboard API. Empty = open (dev).
    pub admin_token: String,
    /// Public base URL, used when composing portal links.
    pub public_url: String,
    /// Seed the demo dataset on boot when the database is empty.
    pub seed_on_boot: bool,
    /// Run the autonomous scheduler loop.
    pub scheduler_enabled: bool,
    /// Public demo mode: reseed on every boot and keep the story fresh.
    pub demo_mode: bool,
}

impl Config {
    pub fn from_env() -> Self {
        let paypal_mode = match std::env::var("KINDLY_PAYPAL_MODE").as_deref() {
            Ok("live") => PayPalMode::Live,
            _ => PayPalMode::Sim,
        };
        let data_dir: PathBuf = std::env::var("KINDLY_DATA_DIR")
            .unwrap_or_else(|_| "./data".to_string())
            .into();
        let database_url = std::env::var("DATABASE_URL")
            .unwrap_or_else(|_| format!("sqlite:{}/kindly.db?mode=rwc", data_dir.display()));
        Self {
            database_url,
            // KINDLY_PORT wins; $PORT is what hosting platforms (Render, Heroku,
            // Fly) set for a web service.
            port: std::env::var("KINDLY_PORT")
                .ok()
                .or_else(|| std::env::var("PORT").ok())
                .and_then(|p| p.parse().ok())
                .unwrap_or(8787),
            paypal_mode,
            invoicing: match std::env::var("KINDLY_INVOICING").as_deref() {
                Ok("on") | Ok("require") | Ok("required") => InvoicingMode::On,
                Ok("off") | Ok("false") | Ok("0") => InvoicingMode::Off,
                _ => InvoicingMode::Auto,
            },
            paypal_base_url: std::env::var("PAYPAL_API_BASE")
                .unwrap_or_else(|_| "https://api-m.sandbox.paypal.com".to_string()),
            paypal_client_id: std::env::var("PAYPAL_CLIENT_ID").unwrap_or_default(),
            paypal_client_secret: std::env::var("PAYPAL_CLIENT_SECRET").unwrap_or_default(),
            paypal_webhook_id: std::env::var("PAYPAL_WEBHOOK_ID")
                .ok()
                .filter(|s| !s.is_empty()),
            // Empty means "the account that owns the API credentials", which
            // is what PayPal uses by default. A made-up address here would make
            // PayPal refuse to send, so there is no fake default.
            invoicer_email: std::env::var("KINDLY_INVOICER_EMAIL").unwrap_or_default(),
            owner_name: std::env::var("KINDLY_OWNER_NAME")
                .unwrap_or_else(|_| "Maya Rodriguez".to_string()),
            owner_business: std::env::var("KINDLY_OWNER_BUSINESS")
                .unwrap_or_else(|_| "Maya Rodriguez Studio".to_string()),
            portal_secret: std::env::var("KINDLY_PORTAL_SECRET")
                .unwrap_or_else(|_| "kindly-dev-portal-secret-change-me".to_string()),
            sim_webhook_secret: std::env::var("KINDLY_SIM_WEBHOOK_SECRET")
                .unwrap_or_else(|_| "kindly-sim-webhook-secret".to_string()),
            admin_token: std::env::var("KINDLY_ADMIN_TOKEN").unwrap_or_default(),
            public_url: std::env::var("KINDLY_PUBLIC_URL")
                .unwrap_or_else(|_| "http://localhost:8787".to_string()),
            seed_on_boot: std::env::var("KINDLY_SEED")
                .map(|v| v != "0" && v.to_lowercase() != "false")
                .unwrap_or(true),
            scheduler_enabled: std::env::var("KINDLY_SCHEDULER")
                .map(|v| v != "0" && v.to_lowercase() != "false")
                .unwrap_or(true),
            demo_mode: std::env::var("KINDLY_DEMO_MODE")
                .map(|v| v == "1" || v.to_lowercase() == "true")
                .unwrap_or(false),
        }
    }

    /// A configuration for tests: an in-memory database, simulation mode, no
    /// network, no credentials. Not part of the product surface.
    #[doc(hidden)]
    pub fn for_tests() -> Self {
        Self {
            database_url: "sqlite::memory:".to_string(),
            port: 0,
            paypal_mode: PayPalMode::Sim,
            invoicing: InvoicingMode::Auto,
            paypal_base_url: "http://localhost".to_string(),
            paypal_client_id: String::new(),
            paypal_client_secret: String::new(),
            paypal_webhook_id: None,
            invoicer_email: String::new(),
            owner_name: "Maya Rodriguez".to_string(),
            owner_business: "Maya Rodriguez Studio".to_string(),
            portal_secret: "test-portal-secret".to_string(),
            sim_webhook_secret: "test-sim-secret".to_string(),
            admin_token: String::new(),
            public_url: "http://localhost:8787".to_string(),
            seed_on_boot: false,
            scheduler_enabled: false,
            demo_mode: false,
        }
    }

    pub fn paypal_configured(&self) -> bool {
        !self.paypal_client_id.is_empty() && !self.paypal_client_secret.is_empty()
    }
}
