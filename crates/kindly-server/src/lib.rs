//! Kindly's server library.
//!
//! The binary in `main.rs` is a thin CLI over these modules; exposing them as a
//! library lets the integration tests drive the real runtime (policy gate,
//! agent loop, PayPal sync, ledger) instead of testing around it.

pub mod a2a;
pub mod config;
pub mod db;
pub mod engine;
pub mod errors;
pub mod events;
pub mod portal;
pub mod routes;
pub mod scheduler;
pub mod seed;
pub mod state;
pub mod store;
pub mod sync;
