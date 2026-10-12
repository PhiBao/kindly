//! Agent actions and their lifecycle.
//!
//! Every outward-facing thing Kindly does is an [`AgentAction`]: proposed,
//! judged by the policy engine, then either executed, queued for approval, or
//! blocked. Actions are append-only history — the audit trail is the product.

use crate::ids::{ActionId, InvoiceId};
use crate::policy::{PolicyDecision, ProposedAction};
use crate::Timestamp;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActionKind {
    /// A reminder message to the client (the "ask").
    Nudge,
    /// Terms put on the table (plan and/or discount).
    OfferTerms,
    /// Executing agreed terms by issuing plan invoices in PayPal.
    IssuePlan,
    /// Recording a payment that arrived outside the invoice flow.
    RecordPartialPayment,
    /// Granting a discount (always paired with terms).
    GrantDiscount,
    /// Escalating to the owner.
    Escalate,
    /// Deliberately waiting.
    Pause,
    /// Opening an agent-to-agent negotiation with the client's AP agent.
    StartAgentNegotiation,
    /// Closing the receivable as uncollectible (owner-approved).
    CloseUncollectible,
}

impl ActionKind {
    pub fn label(self) -> &'static str {
        match self {
            ActionKind::Nudge => "Reminder",
            ActionKind::OfferTerms => "Offer terms",
            ActionKind::IssuePlan => "Issue payment plan",
            ActionKind::RecordPartialPayment => "Record payment",
            ActionKind::GrantDiscount => "Grant discount",
            ActionKind::Escalate => "Escalate",
            ActionKind::Pause => "Wait",
            ActionKind::StartAgentNegotiation => "Agent-to-agent",
            ActionKind::CloseUncollectible => "Close as uncollectible",
        }
    }

    /// Does this action reach the client (and therefore count against contact caps)?
    pub fn is_client_contact(self) -> bool {
        matches!(self, ActionKind::Nudge | ActionKind::OfferTerms)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActionStatus {
    Proposed,
    AwaitingApproval,
    Approved,
    Executing,
    Executed,
    Failed,
    Blocked,
    Rejected,
}

impl ActionStatus {
    pub fn label(self) -> &'static str {
        match self {
            ActionStatus::Proposed => "Proposed",
            ActionStatus::AwaitingApproval => "Waiting for you",
            ActionStatus::Approved => "Approved",
            ActionStatus::Executing => "Happening now",
            ActionStatus::Executed => "Done",
            ActionStatus::Failed => "Failed",
            ActionStatus::Blocked => "Blocked by your limits",
            ActionStatus::Rejected => "Rejected",
        }
    }

    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            ActionStatus::Executed
                | ActionStatus::Failed
                | ActionStatus::Blocked
                | ActionStatus::Rejected
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentAction {
    pub id: ActionId,
    pub invoice_id: InvoiceId,
    pub kind: ActionKind,
    pub status: ActionStatus,
    pub proposal: ProposedAction,
    /// The policy verdict, verbatim. Shown in the UI and kept forever.
    pub decision: PolicyDecision,
    /// One-line explanation in the agent's own voice, shown to the owner.
    pub rationale: String,
    /// Idempotency key for the underlying PayPal call (when there is one).
    pub idempotency_key: String,
    pub created_at: Timestamp,
    pub executed_at: Option<Timestamp>,
    pub error: Option<String>,
}

impl AgentAction {
    pub fn is_open(&self) -> bool {
        !self.status.is_terminal()
    }
}
