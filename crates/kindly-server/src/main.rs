//! Kindly's server binary.
//!
//! ```text
//! kindly serve                      # run the API (plus the web app if built)
//! kindly serve --paypal live        # talk to the real PayPal sandbox
//! kindly seed [--reset]            # populate the demo dataset
//! kindly tick                       # run one agent pass over open invoices
//! kindly paypal-check               # verify PayPal credentials
//! ```
//!
//! In simulation mode (`--paypal sim`, the default) the whole product works
//! with no external accounts: a faithful in-process PayPal implements the same
//! surface, signs the same webhook deliveries, and the ledger records that
//! simulated money moved.

use anyhow::{Context, Result};
use std::sync::Arc;

mod a2a;
mod config;
mod db;
mod engine;
mod errors;
mod events;
mod portal;
mod routes;
mod seed;
mod state;
mod store;
mod sync;

use config::{Config, PayPalMode};
use kindly_agent::llm::{NullLlm, OpenAiCompat};
use kindly_paypal::{LivePayPal, PayPalApi, SimPayPal};
use state::AppState;
use store::Store;
use tracing::{info, warn};
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new("info,kindly_server=info,sqlx=warn")),
        )
        .with_target(false)
        .init();

    let mut args = std::env::args().skip(1);
    let command = args.next().unwrap_or_else(|| "serve".to_string());
    let flags: Vec<String> = args.collect();

    match command.as_str() {
        "serve" => serve(flags).await,
        "seed" => {
            let state = build_state().await?;
            let summary = seed::seed_demo(&state, flags.iter().any(|f| f == "--reset")).await?;
            println!("{}", serde_json::to_string_pretty(&summary)?);
            Ok(())
        }
        "tick" => {
            let state = build_state().await?;
            let invoices = state.store.open_invoices().await?;
            let mut executed = 0;
            for invoice in invoices {
                let report = engine::tick_invoice(&state, invoice.id).await?;
                executed += report.executed;
                println!(
                    "{} ({}): {} proposed, {} executed, {} awaiting approval, {} blocked{}",
                    invoice.number,
                    invoice.status.label(),
                    report.proposals,
                    report.executed,
                    report.awaiting_approval,
                    report.blocked,
                    report
                        .idle_reason
                        .map(|r| format!(" — {r}"))
                        .unwrap_or_default()
                );
            }
            println!("{executed} action(s) executed");
            Ok(())
        }
        "paypal-check" => paypal_check().await,
        "help" | "--help" | "-h" => {
            println!(
                "kindly <command>\n\n  serve [--paypal live|sim]   run the API and the web app\n  seed [--reset]              populate the demo dataset\n  tick                        run one agent pass\n  paypal-check                verify PayPal credentials\n"
            );
            Ok(())
        }
        other => {
            eprintln!("unknown command: {other}");
            std::process::exit(2);
        }
    }
}

fn config_with_flags(flags: &[String]) -> Config {
    if flags.iter().any(|flag| flag == "--paypal") {
        let mode = flags
            .iter()
            .position(|flag| flag == "--paypal")
            .and_then(|index| flags.get(index + 1))
            .cloned()
            .unwrap_or_else(|| "sim".to_string());
        // SAFETY: single-threaded at this point; the value is read once below.
        std::env::set_var("KINDLY_PAYPAL_MODE", mode);
    }
    if flags.iter().any(|flag| flag == "--reset") {
        std::env::set_var("KINDLY_SEED_RESET", "1");
    }
    Config::from_env()
}

async fn build_state_from(config: Config) -> Result<Arc<AppState>> {
    let pool = db::connect(&config.database_url)
        .await
        .map_err(|error| anyhow::anyhow!(error.to_string()))
        .context("connecting to the database")?;
    let store = Store::new(pool);

    let mut simulator: Option<Arc<SimPayPal>> = None;
    let paypal: Arc<dyn PayPalApi> = match config.paypal_mode {
        PayPalMode::Live => {
            if !config.paypal_configured() {
                anyhow::bail!(
                    "PAYPAL_CLIENT_ID and PAYPAL_CLIENT_SECRET are required for live mode.\n\
                     Create a sandbox app at https://developer.paypal.com/dashboard and copy its credentials,\n\
                     or run with --paypal sim to use the built-in simulator."
                );
            }
            let mut live = kindly_paypal::LiveConfig::sandbox(
                config.paypal_client_id.clone(),
                config.paypal_client_secret.clone(),
                config.invoicer_email.clone(),
            );
            live.base_url = config.paypal_base_url.clone();
            live.webhook_id = config.paypal_webhook_id.clone();
            Arc::new(LivePayPal::new(live).map_err(|e| anyhow::anyhow!(e.to_string()))?)
        }
        PayPalMode::Sim => {
            let sim = Arc::new(SimPayPal::new(config.sim_webhook_secret.clone()));
            simulator = Some(sim.clone());
            sim
        }
    };

    let llm: Arc<dyn kindly_agent::llm::LlmProvider> = match OpenAiCompat::from_env() {
        Some(provider) => Arc::new(provider),
        None => {
            warn!("no KINDLY_LLM_API_KEY set — writing from templates instead of a model");
            Arc::new(NullLlm)
        }
    };

    Ok(Arc::new(AppState::new(config, store, paypal, llm, simulator)))
}

async fn build_state() -> Result<Arc<AppState>> {
    let config = Config::from_env();
    build_state_from(config).await
}

async fn serve(flags: Vec<String>) -> Result<()> {
    let config = config_with_flags(&flags);
    let reset = flags.iter().any(|flag| flag == "--reset");

    if config.paypal_mode == PayPalMode::Sim {
        warn!("PayPal simulation mode: no money leaves this machine. Run with --paypal live for the sandbox.");
    } else {
        info!(base = %config.paypal_base_url, "PayPal live mode");
    }
    if config.admin_token.is_empty() {
        warn!("KINDLY_ADMIN_TOKEN is not set — the dashboard API is unauthenticated (fine for local demos only)");
    }

    let state = build_state_from(config).await?;

    if state.config.seed_on_boot {
        let summary = seed::seed_demo(&state, reset).await?;
        if summary.already_present {
            info!(clients = summary.clients, invoices = summary.invoices, "demo data present");
        } else {
            info!(
                clients = summary.clients,
                invoices = summary.invoices,
                "seeded the demo dataset"
            );
        }
    }

    // In simulation mode the PayPal objects live in memory, so after a restart
    // we rebuild them from the database. The database stays authoritative.
    if let Some(sim) = &state.sim {
        let invoices = state.store.invoices().await?;
        let mut hydrated = 0;
        for invoice in invoices {
            let (Some(paypal_id), Some(client)) = (
                invoice.paypal_invoice_id.clone(),
                state.store.client(invoice.client_id).await.ok(),
            ) else {
                continue;
            };
            let status = match invoice.paypal_status.as_deref() {
                Some("PAID") => kindly_paypal::model::PayPalInvoiceStatus::Paid,
                Some("PARTIALLY_PAID") => kindly_paypal::model::PayPalInvoiceStatus::PartiallyPaid,
                Some("CANCELLED") => kindly_paypal::model::PayPalInvoiceStatus::Cancelled,
                Some("DRAFT") => kindly_paypal::model::PayPalInvoiceStatus::Draft,
                _ => kindly_paypal::model::PayPalInvoiceStatus::Sent,
            };
            let first = invoice.lines.first();
            sim.import_invoice(
                &paypal_id,
                &invoice.number,
                status,
                invoice.currency.code(),
                invoice.total.minor,
                invoice.amount_paid.minor,
                invoice.due_date,
                &client.email,
                client.company.as_deref(),
                invoice.memo.as_deref(),
                first.map(|line| line.quantity.display()).unwrap_or_else(|| "1".into()).as_str(),
                first.map(|line| line.unit_price.minor).unwrap_or(invoice.total.minor),
            )
            .ok();
            hydrated += 1;
        }
        if hydrated > 0 {
            info!(hydrated, "hydrated the PayPal simulator from the database");
        }
    }

    let scheduler_state = state.clone();
    if state.config.scheduler_enabled {
        tokio::spawn(async move {
            // First pass after a short delay so a freshly started demo lets the
            // human see the "before" state.
            let mut ticker = tokio::time::interval_at(
                tokio::time::Instant::now() + std::time::Duration::from_secs(90),
                std::time::Duration::from_secs(90),
            );
            loop {
                ticker.tick().await;
                match scheduler::pass(&scheduler_state).await {
                    Ok(summary) => {
                        if summary.ticks > 0 || summary.reconciled > 0 {
                            info!(
                                ticks = summary.ticks,
                                reconciled = summary.reconciled,
                                "scheduler pass"
                            );
                        }
                    }
                    Err(error) => warn!(%error, "scheduler pass failed"),
                }
            }
        });
    }

    let app = routes::router(state.clone());
    let addr = format!("0.0.0.0:{}", state.config.port);
    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .with_context(|| format!("binding {addr}"))?;
    info!(%addr, "Kindly is listening");
    println!(
        "\n  Kindly — http://localhost:{}\n  PayPal mode: {}\n  Language model: {}\n",
        state.config.port,
        state.config.paypal_mode.label(),
        state.llm.description()
    );

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await
        .context("server error")?;
    Ok(())
}

async fn paypal_check() -> Result<()> {
    let config = Config::from_env();
    let live = LivePayPal::new(kindly_paypal::LiveConfig {
        base_url: config.paypal_base_url.clone(),
        client_id: config.paypal_client_id.clone(),
        client_secret: config.paypal_client_secret.clone(),
        webhook_id: config.paypal_webhook_id.clone(),
        invoicer_email: config.invoicer_email.clone(),
        partner_attribution_id: None,
    })
    .map_err(|e| anyhow::anyhow!(e.to_string()))?;

    match live.list_invoices(5).await {
        Ok(invoices) => {
            println!(
                "PayPal OK — {} invoice(s) visible to this app at {}",
                invoices.len(),
                config.paypal_base_url
            );
            for invoice in invoices {
                println!(
                    "  {} {:?} due {:?}",
                    invoice.id, invoice.status, invoice.due_date
                );
            }
            Ok(())
        }
        Err(error) => {
            eprintln!("PayPal check failed: {error}");
            std::process::exit(1);
        }
    }
}

async fn shutdown_signal() {
    let _ = tokio::signal::ctrl_c().await;
    info!("shutting down");
}

/// The autonomous pass: advance conversations and reconcile with PayPal.
mod scheduler {
    use super::*;

    #[derive(Debug, Default)]
    pub struct Summary {
        pub ticks: usize,
        pub reconciled: usize,
    }

    pub async fn pass(state: &Arc<AppState>) -> Result<Summary> {
        let mut summary = Summary::default();
        let today = chrono::Utc::now().date_naive();

        let invoices = state.store.invoices_with_negotiation().await?;
        for (invoice, negotiation) in invoices {
            if !invoice.is_open() {
                continue;
            }
            let due_for_attention = match negotiation.last_contact_at {
                Some(last) => (chrono::Utc::now() - last).num_days() >= 2,
                None => true,
            };
            if due_for_attention && invoice.days_overdue(today) > 0 {
                engine::tick_invoice(state, invoice.id).await?;
                summary.ticks += 1;
            }
        }

        summary.reconciled = sync::reconcile(state).await.unwrap_or(0);
        Ok(summary)
    }
}
