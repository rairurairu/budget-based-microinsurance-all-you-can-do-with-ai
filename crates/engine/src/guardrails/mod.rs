//! M10 Guardrails: the decision log, random audit and risk review (R14), the anomaly score behind
//! risk review, review requests (R15), the fairness audit (R19), human care and emergencies (R46)
//! and the people's work queue (ADR-010).
pub mod anomaly;
pub mod appeals;
pub mod audit;
pub mod fairness;
pub mod human_care;
pub mod queue;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Who made a decision.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub enum Actor {
    /// A recommendation of the AI layer, applied by the engine's rules.
    Ai,
    /// The engine's rules alone, such as a trigger match.
    Rules,
    Person,
    /// The customer's own choice, such as an allocation they confirmed (R03).
    Customer,
}

/// One append-only decision-log row.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct DecisionRecord {
    pub decision_id: String,
    pub actor: Actor,
    /// The person who decided, when `actor` is a person.
    #[serde(default)]
    pub reviewer: Option<String>,
    /// What was decided about, such as `claim` or `prevention_task`.
    pub subject_type: String,
    pub subject_id: String,
    pub customer_id: String,
    pub decision: String,
    /// SHA-256 of the inputs, so the decision can be replayed.
    pub inputs_hash: String,
    pub reason: String,
    pub at: DateTime<Utc>,
}
