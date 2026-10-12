//! Kindly domain core.
//!
//! This crate is deliberately free of I/O. It contains the rules that must be
//! correct for Kindly to be trustworthy:
//!
//! * [`money`] — exact integer money arithmetic (no floats, ever)
//! * [`mandate`] — the user-authored autonomy contract
//! * [`policy`] — the deterministic engine that decides whether the agent may act
//! * [`negotiation`] — the receivable negotiation state machine
//! * [`attribution`] — deterministic causal accounting ("what got you paid")
//!
//! The language model never lives here. Models propose; this crate disposes.

pub mod action;
pub mod attribution;
pub mod client;
pub mod error;
pub mod ids;
pub mod invoice;
pub mod ledger;
pub mod mandate;
pub mod money;
pub mod negotiation;
pub mod policy;

pub use error::{CoreError, CoreResult};

use chrono::{DateTime, NaiveDate, Utc};

/// Convenience alias for the wall clock used across the domain.
pub type Timestamp = DateTime<Utc>;

/// A calendar day in the user's billing context.
pub type Day = NaiveDate;
