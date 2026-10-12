//! Random human audit of automated decisions (R14). Sampling is a pure function of the decision
//! id and a seed, so any audit selection can be replayed. The seed is the insurer's secret,
//! passed in and recorded with each sample: a seed the customer knew would tell them which
//! decisions are checked (ADR-009).
use crate::config::GuardrailConfig;
use crate::money::Bps;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub fn should_audit(decision_id: &str, rate: Bps, seed: u64) -> bool {
    // FNV-1a, then a 64-bit finaliser for an even spread over the low digits.
    let mut h: u64 = 0xcbf2_9ce4_8422_2325 ^ seed;
    for b in decision_id.as_bytes() {
        h ^= u64::from(*b);
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    h ^= h >> 33;
    h = h.wrapping_mul(0xff51_afd7_ed55_8ccd);
    h ^= h >> 33;
    h = h.wrapping_mul(0xc4ce_b9fe_1a85_ec53);
    h ^= h >> 33;
    h % u64::from(Bps::ONE.0) < u64::from(rate.0)
}

pub fn needs_risk_review(anomaly: Bps, threshold: Bps) -> bool {
    anomaly >= threshold
}

/// Why a person looks at an automated decision.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub enum AuditReason {
    /// Its anomaly score reached `risk_review_threshold`.
    RiskReview,
    /// Drawn in the random sample, `audit_sample_rate` of decisions (R14).
    RandomSample,
}

/// Whether a person checks this decision, and why: always at or above the risk review
/// threshold, otherwise in the random sample.
pub fn select(
    decision_id: &str,
    anomaly: Option<Bps>,
    seed: u64,
    cfg: &GuardrailConfig,
) -> Option<AuditReason> {
    if anomaly.is_some_and(|a| needs_risk_review(a, cfg.risk_review_threshold)) {
        Some(AuditReason::RiskReview)
    } else if should_audit(decision_id, cfg.audit_rate, seed) {
        Some(AuditReason::RandomSample)
    } else {
        None
    }
}

/// The decisions a person audits after the fact, such as payouts already paid (R14), in id
/// order. `decisions` is each decision's id and anomaly score, if any.
pub fn sample<'a>(
    decisions: impl IntoIterator<Item = (&'a str, Option<Bps>)>,
    seed: u64,
    cfg: &GuardrailConfig,
) -> Vec<(String, AuditReason)> {
    let mut out: Vec<(String, AuditReason)> = decisions
        .into_iter()
        .filter_map(|(id, anomaly)| select(id, anomaly, seed, cfg).map(|r| (id.to_string(), r)))
        .collect();
    out.sort();
    out.dedup();
    out
}

/// A person's finding on an audited decision: agreed, or overturned with a note (R14).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct AuditFinding {
    pub decision_id: String,
    /// The decision kind, such as `claim` or `payout`.
    pub kind: String,
    pub reason: AuditReason,
    pub reviewer: String,
    pub agreed: bool,
    pub note: String,
    pub on: DateTime<Utc>,
}

/// How often people overturn the automated path, per decision kind. Only the random sample
/// counts: risk-reviewed decisions were chosen for looking odd, so they would overstate it. The
/// rate estimates the automated path's error and shows drift between periods.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct AuditStats {
    pub kind: String,
    pub sampled: u32,
    pub overturned: u32,
    pub overturn_rate: Bps,
}

pub fn overturn_rates(findings: &[AuditFinding]) -> Vec<AuditStats> {
    let mut by_kind: BTreeMap<&str, (u32, u32)> = BTreeMap::new();
    for f in findings
        .iter()
        .filter(|f| f.reason == AuditReason::RandomSample)
    {
        let c = by_kind.entry(&f.kind).or_default();
        c.0 += 1;
        c.1 += u32::from(!f.agreed);
    }
    by_kind
        .into_iter()
        .map(|(kind, (sampled, overturned))| AuditStats {
            kind: kind.to_string(),
            sampled,
            overturned,
            overturn_rate: Bps::ratio(u64::from(overturned), u64::from(sampled)),
        })
        .collect()
}
