//! Shared application state.

use crate::config::Config;
use crate::events::EventBus;
use crate::store::Store;
use chrono::{DateTime, Utc};
use ed25519_dalek::SigningKey;
use kindly_agent::buyer::BuyerAgent;
use kindly_agent::llm::LlmProvider;
use kindly_agent::planner::Planner;
use kindly_paypal::{PayPalApi, SimPayPal};
use rand_core::OsRng;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;

pub struct AppState {
    pub config: Config,
    pub store: Store,
    pub paypal: Arc<dyn PayPalApi>,
    /// Present only in simulation mode: lets the demo drive payments.
    pub sim: Option<Arc<SimPayPal>>,
    pub llm: Arc<dyn LlmProvider>,
    pub planner: Arc<Planner>,
    pub events: EventBus,
    /// Live agent-to-agent sessions. Buyer agents hold ephemeral signing keys;
    /// the transcripts themselves are persisted in the database.
    pub buyer_sessions: Arc<Mutex<HashMap<String, Arc<BuyerAgent>>>>,
    /// Kindly's own agent key, used to sign its side of agent-to-agent talks.
    pub seller_key: SigningKey,
    pub seller_agent_id: String,
    pub started_at: DateTime<Utc>,
}

impl AppState {
    pub fn new(
        config: Config,
        store: Store,
        paypal: Arc<dyn PayPalApi>,
        llm: Arc<dyn LlmProvider>,
        sim: Option<Arc<SimPayPal>>,
    ) -> Self {
        let planner = Arc::new(Planner::new(llm.clone(), config.owner_name.clone()));
        Self {
            config,
            store,
            paypal,
            sim,
            llm,
            planner,
            events: EventBus::new(),
            buyer_sessions: Arc::new(Mutex::new(HashMap::new())),
            seller_key: SigningKey::generate(&mut OsRng),
            seller_agent_id: "kindly-collections".to_string(),
            started_at: Utc::now(),
        }
    }

    pub fn is_sim(&self) -> bool {
        self.sim.is_some()
    }
}

pub type SharedState = Arc<AppState>;
