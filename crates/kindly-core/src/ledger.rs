//! The ledger: an append-only, human-readable record of everything that
//! happened. It is both the audit trail and the product's narrative surface.

use crate::ids::Actor;
use crate::ids::{ClientId, InvoiceId, LedgerEventId};
use crate::Timestamp;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LedgerKind {
    // invoice lifecycle
    InvoiceCreated,
    InvoiceSent,
    InvoicePaid,
    InvoicePartiallyPaid,
    PaymentRecorded,
    // agent behaviour
    AgentProposed,
    PolicyAllowed,
    PolicyBlocked,
    PolicyApprovalRequested,
    PolicyApprovalGranted,
    PolicyApprovalRejected,
    ActionExecuted,
    ActionFailed,
    MessageSent,
    MessageReceived,
    // negotiation
    NegotiationEvent,
    PlanIssued,
    AgentToAgentStep,
    // trust + safety
    SecurityEvent,
    MandateUpdated,
    // integration
    PayPalCall,
    PayPalWebhook,
    // platform
    ForecastUpdated,
    System,
}

impl LedgerKind {
    /// Should this kind be emphasised in the "trust" view?
    pub fn is_trust_relevant(self) -> bool {
        matches!(
            self,
            LedgerKind::PolicyBlocked
                | LedgerKind::PolicyApprovalRequested
                | LedgerKind::PolicyApprovalGranted
                | LedgerKind::SecurityEvent
                | LedgerKind::MandateUpdated
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LedgerEvent {
    pub id: LedgerEventId,
    pub at: Timestamp,
    pub actor: Actor,
    pub kind: LedgerKind,
    /// One plain sentence, written for the owner, not for a log file.
    pub summary: String,
    /// Optional structured detail (policy verdict, PayPal ids, amounts).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub invoice_id: Option<InvoiceId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_id: Option<ClientId>,
}

impl LedgerEvent {
    pub fn new(at: Timestamp, actor: Actor, kind: LedgerKind, summary: impl Into<String>) -> Self {
        Self {
            id: LedgerEventId::new(),
            at,
            actor,
            kind,
            summary: summary.into(),
            detail: None,
            invoice_id: None,
            client_id: None,
        }
    }

    pub fn with_detail(mut self, detail: serde_json::Value) -> Self {
        self.detail = Some(detail);
        self
    }

    pub fn with_invoice(mut self, invoice_id: InvoiceId) -> Self {
        self.invoice_id = Some(invoice_id);
        self
    }

    pub fn with_client(mut self, client_id: ClientId) -> Self {
        self.client_id = Some(client_id);
        self
    }
}
