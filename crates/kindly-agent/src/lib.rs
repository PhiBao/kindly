//! Kindly's agent layer.
//!
//! Three responsibilities, kept in separate modules so each can be reasoned
//! about and tested on its own:
//!
//! * [`llm`] — provider adapters (OpenAI-compatible HTTP + a deterministic
//!   template fallback so Kindly always works, even without an API key);
//! * [`understand`] — turning free-form client replies into closed intents,
//!   including refusing instruction-injection attempts;
//! * [`compose`] — writing in the owner's voice;
//! * [`planner`] — deciding the next move for an invoice;
//! * [`buyer`] and [`katp`] — the client-side accounts-payable agent and the
//!   agent-to-agent terms protocol it speaks with Kindly.
//!
//! The rule everywhere: the model may *propose*; [`kindly_core::policy`]
//! disposes.

pub mod buyer;
pub mod compose;
pub mod katp;
pub mod llm;
pub mod planner;
pub mod prompts;
pub mod understand;

pub use buyer::{BuyerAgent, BuyerDecision, BuyerMandate};
pub use katp::{
    public_key_b64, AgentRef, KatpEnvelope, KatpMessage, MandateAttestation, TermsProposal,
};
pub use llm::{LlmProvider, LlmRequest, LlmResponse};
pub use planner::{DraftMessage, InvoiceBrief, PlanOutcome, Planner};
pub use understand::{InjectionScan, ReplyReading};
