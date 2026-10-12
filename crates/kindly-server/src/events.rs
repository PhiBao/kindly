//! The in-process event bus that feeds the live UI (SSE).
//!
//! Every meaningful thing that happens is published here *and* written to the
//! ledger. The ledger is the durable record; the bus is the pulse.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::broadcast;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Event {
    /// Machine name, e.g. `agent_action`, `policy_blocked`, `invoice_paid`.
    pub kind: String,
    pub at: DateTime<Utc>,
    pub payload: Value,
}

#[derive(Clone)]
pub struct EventBus {
    sender: broadcast::Sender<Event>,
}

impl Default for EventBus {
    fn default() -> Self {
        Self::new()
    }
}

impl EventBus {
    pub fn new() -> Self {
        let (sender, _) = broadcast::channel(512);
        Self { sender }
    }

    pub fn publish(&self, kind: impl Into<String>, payload: Value) {
        let event = Event {
            kind: kind.into(),
            at: Utc::now(),
            payload,
        };
        // No subscribers is fine: the ledger still holds the record.
        let _ = self.sender.send(event);
    }

    pub fn subscribe(&self) -> broadcast::Receiver<Event> {
        self.sender.subscribe()
    }
}
