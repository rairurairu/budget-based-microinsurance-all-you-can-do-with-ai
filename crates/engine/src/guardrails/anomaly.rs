//! The anomaly score behind risk review (R14): rules-based fraud indicators (red flags), each
//! adding its configured weight, at most 1. Every indicator present is named, so a person sees
//! why a claim was flagged. No model and no protected attribute is an input (R19).
use crate::config::AnomalyConfig;
use crate::money::{Bps, Money};
use chrono::NaiveDate;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

/// A fraud indicator. A closed set: each is computed here from the claim's facts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub enum Indicator {
    /// The loss came within `early_claim_days` of the day cover for it began.
    EarlyClaim,
    /// The claim was notified more than `late_notification_days` after the loss.
    LateNotification,
    /// At least `frequent_claims` claims in the class within the claims experience window.
    FrequentClaims,
    /// The amount claimed is at least `near_sum_insured` of the sum insured.
    NearSumInsured,
    /// Evidence the AI layer's fingerprint check has seen before.
    EvidenceReused,
}

/// What the indicators are computed from.
#[derive(Debug, Clone, Copy)]
pub struct ClaimFacts {
    pub loss_date: NaiveDate,
    pub notified_on: NaiveDate,
    /// The first day a loss is covered (after any waiting period, `catalogue::covered_from`).
    pub covered_from: NaiveDate,
    pub amount_claimed: Money,
    pub sum_insured: Money,
    /// Claims in the class within `claims_experience_periods`, this one included.
    pub claims_in_class: u32,
    pub evidence_reused: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct AnomalyScore {
    pub score: Bps,
    pub indicators: BTreeSet<Indicator>,
}

/// The indicators present and their summed weight, at most 1.
pub fn score(facts: &ClaimFacts, cfg: &AnomalyConfig) -> AnomalyScore {
    let days_after = |a: NaiveDate, b: NaiveDate| (a - b).num_days();
    let mut indicators = BTreeSet::new();
    let early = days_after(facts.loss_date, facts.covered_from);
    if (0..i64::from(cfg.early_claim_days)).contains(&early) {
        indicators.insert(Indicator::EarlyClaim);
    }
    if days_after(facts.notified_on, facts.loss_date) > i64::from(cfg.late_notification_days) {
        indicators.insert(Indicator::LateNotification);
    }
    if cfg.frequent_claims > 0 && facts.claims_in_class >= cfg.frequent_claims {
        indicators.insert(Indicator::FrequentClaims);
    }
    if facts.sum_insured > Money::ZERO
        && i128::from(facts.amount_claimed.0) * i128::from(Bps::ONE.0)
            >= i128::from(facts.sum_insured.0) * i128::from(cfg.near_sum_insured.0)
    {
        indicators.insert(Indicator::NearSumInsured);
    }
    if facts.evidence_reused {
        indicators.insert(Indicator::EvidenceReused);
    }
    let total: u32 = indicators
        .iter()
        .map(|i| cfg.weights.get(i).map_or(0, |w| w.0))
        .sum();
    AnomalyScore {
        score: Bps(total.min(Bps::ONE.0)),
        indicators,
    }
}
