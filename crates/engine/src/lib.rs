//! Deterministic engine: every money decision. No I/O, no AI, no clock reads, no randomness;
//! time, seeds and data are passed in. Same input, same output (R02, R30).

pub mod catalogue;
pub mod claims;
pub mod config;
pub mod consent;
pub mod error;
pub mod guardrails;
pub mod money;
pub mod optimiser;
pub mod payouts;
pub mod pilot;
pub mod plans;
pub mod prevention;
pub mod rewards;
pub mod score;
pub mod suppliers;
pub mod triggers;
pub mod types;

pub use error::EngineError;

pub type Result<T> = std::result::Result<T, EngineError>;
