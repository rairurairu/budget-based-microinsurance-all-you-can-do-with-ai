//! Harnessed AI layer (M4). The only crate that calls a model provider. Returns typed
//! recommendations, each checked by fixed rules and replaced by a saved answer when the model is
//! slow, fails or answers outside them (R22); never decides money (ADR-002).

pub mod assess;
pub mod client;
pub mod explain;
pub mod foresee;
pub mod harness;
pub mod language;
pub mod prevention;
pub mod prompts;
pub mod replay;
pub mod rewards;
pub mod schema;
pub mod settings;
pub mod text;
pub mod understand;
pub mod verify;
pub mod vocab;
