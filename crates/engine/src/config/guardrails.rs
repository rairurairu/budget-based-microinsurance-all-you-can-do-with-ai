//! The `guardrails` and `claims` sections of the insurer profile: audit sampling and risk review
//! (R14), review requests (R15), the fairness audit (R19), human care and emergencies (R46), and
//! the straight-through processing authority for indemnity claims (R13, R57; ADR-010).
use super::decimal::{DecBps, DecMoney};
use crate::guardrails::anomaly::Indicator;
use crate::guardrails::human_care::FirstAction;
use crate::money::{Bps, Money};
use crate::{EngineError, Result};
use serde::Deserialize;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone)]
pub struct GuardrailConfig {
    pub audit_rate: Bps,
    pub risk_review_threshold: Bps,
    /// A person decides a review request within this (R15).
    pub appeal_sla_hours: u32,
    pub human_care: HumanCareConfig,
    pub fairness: FairnessConfig,
}

/// R46: urgent cases get an automatic first response; people for those who need one.
#[derive(Debug, Clone)]
pub struct HumanCareConfig {
    /// Circumstances a customer can report that make a case urgent, such as `injury`.
    pub urgent: BTreeSet<String>,
    /// A loss at least this share of the customer's monthly earnings is urgent.
    pub large_loss_share_of_monthly_income: Bps,
    pub urgent_response_seconds: u32,
    /// What the first response does, in order.
    pub first_response: Vec<FirstAction>,
    /// The most guarantee of payment given automatically per case; above it the assistance
    /// provider confirms.
    pub auto_guarantee_cap: Money,
}

/// R19: approval rates compared across audit groups, as an adverse impact ratio.
#[derive(Debug, Clone)]
pub struct FairnessConfig {
    /// Attributes the audit store groups customers by, such as `age_band`. Never AI inputs.
    pub groups: BTreeSet<String>,
    /// A group's approval rate below this share of the best credible group's is flagged
    /// (the four-fifths rule at 0.8).
    pub min_parity_ratio: Bps,
    /// Decisions a group needs before its rate is compared.
    pub min_group_decisions: u32,
}

/// Straight-through processing (STP) authority for indemnity claims (R13, R57).
#[derive(Debug, Clone)]
pub struct ClaimsConfig {
    /// The largest claim payment approved without a person.
    pub stp_limit: Money,
    /// The assessor's confidence below which a person decides.
    pub stp_min_confidence: Bps,
    /// How far the amount claimed may exceed the assessor's estimate.
    pub estimate_tolerance: Bps,
    /// A person decides a referred claim within this.
    pub referral_sla_hours: u32,
    pub anomaly: AnomalyConfig,
}

/// Fraud indicators (red flags) behind a claim's anomaly score (R14).
#[derive(Debug, Clone)]
pub struct AnomalyConfig {
    pub early_claim_days: u32,
    pub late_notification_days: u32,
    /// Claims in the class within `claims_experience_periods`, this one included.
    pub frequent_claims: u32,
    pub near_sum_insured: Bps,
    /// Weight of each indicator; a missing indicator weighs nothing.
    pub weights: BTreeMap<Indicator, Bps>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct RawGuardrails {
    audit_sample_rate: DecBps,
    risk_review_threshold: DecBps,
    appeal_sla_hours: u32,
    human_care: RawHumanCare,
    fairness: RawFairness,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawHumanCare {
    urgent: BTreeSet<String>,
    large_loss_share_of_monthly_income: DecBps,
    urgent_response_seconds: u32,
    first_response: Vec<FirstAction>,
    auto_guarantee_cap: DecMoney,
    person_on_request: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawFairness {
    groups: BTreeSet<String>,
    min_parity_ratio: DecBps,
    min_group_decisions: u32,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct RawClaims {
    stp_limit: DecMoney,
    stp_min_confidence: DecBps,
    estimate_tolerance: DecBps,
    referral_sla_hours: u32,
    anomaly: RawAnomaly,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawAnomaly {
    early_claim_days: u32,
    late_notification_days: u32,
    frequent_claims: u32,
    near_sum_insured: DecBps,
    weights: BTreeMap<Indicator, DecBps>,
}

fn rule(rule: &'static str, detail: &str) -> EngineError {
    EngineError::Rule {
        rule,
        detail: detail.into(),
    }
}

pub(super) fn parse_guardrails(raw: RawGuardrails) -> Result<GuardrailConfig> {
    let audit_rate = raw.audit_sample_rate.0;
    if audit_rate.0 == 0 || audit_rate > Bps::ONE {
        return Err(rule(
            "R14",
            "guardrails.audit_sample_rate must be above 0 and at most 1",
        ));
    }
    if raw.risk_review_threshold.0 > Bps::ONE {
        return Err(rule("R14", "guardrails.risk_review_threshold is at most 1"));
    }
    if raw.appeal_sla_hours == 0 {
        return Err(rule("R15", "guardrails.appeal_sla_hours must be positive"));
    }
    let hc = raw.human_care;
    if !hc.person_on_request {
        return Err(rule(
            "R46",
            "guardrails.human_care.person_on_request must be true",
        ));
    }
    if hc.urgent.is_empty() || hc.urgent.iter().any(|u| u.trim().is_empty()) {
        return Err(rule(
            "R46",
            "guardrails.human_care.urgent lists at least one circumstance",
        ));
    }
    if hc.urgent_response_seconds == 0 || hc.first_response.is_empty() {
        return Err(rule(
            "R46",
            "guardrails.human_care needs a first response and a positive urgent_response_seconds",
        ));
    }
    let actions: BTreeSet<&FirstAction> = hc.first_response.iter().collect();
    if actions.len() != hc.first_response.len() {
        return Err(rule(
            "R46",
            "guardrails.human_care.first_response lists an action twice",
        ));
    }
    if hc.auto_guarantee_cap.0 < Money::ZERO || hc.large_loss_share_of_monthly_income.0.0 == 0 {
        return Err(rule(
            "R46",
            "guardrails.human_care: auto_guarantee_cap is not negative and \
             large_loss_share_of_monthly_income is positive",
        ));
    }
    let fr = raw.fairness;
    let bad_group = fr
        .groups
        .iter()
        .any(|g| g.is_empty() || g.contains(char::is_whitespace));
    if fr.groups.is_empty()
        || bad_group
        || fr.min_parity_ratio.0 > Bps::ONE
        || fr.min_group_decisions == 0
    {
        return Err(rule(
            "R19",
            "guardrails.fairness needs attribute ids as groups, min_parity_ratio at most 1 and \
             a positive min_group_decisions",
        ));
    }
    Ok(GuardrailConfig {
        audit_rate,
        risk_review_threshold: raw.risk_review_threshold.0,
        appeal_sla_hours: raw.appeal_sla_hours,
        human_care: HumanCareConfig {
            urgent: hc.urgent,
            large_loss_share_of_monthly_income: hc.large_loss_share_of_monthly_income.0,
            urgent_response_seconds: hc.urgent_response_seconds,
            first_response: hc.first_response,
            auto_guarantee_cap: hc.auto_guarantee_cap.0,
        },
        fairness: FairnessConfig {
            groups: fr.groups,
            min_parity_ratio: fr.min_parity_ratio.0,
            min_group_decisions: fr.min_group_decisions,
        },
    })
}

pub(super) fn parse_claims(raw: RawClaims) -> Result<ClaimsConfig> {
    if raw.stp_limit.0 < Money::ZERO {
        return Err(rule("R57", "claims.stp_limit is not negative"));
    }
    if raw.stp_min_confidence.0 > Bps::ONE {
        return Err(rule("R57", "claims.stp_min_confidence is at most 1"));
    }
    if raw.referral_sla_hours == 0 {
        return Err(rule("R57", "claims.referral_sla_hours must be positive"));
    }
    let a = raw.anomaly;
    if a.near_sum_insured.0 > Bps::ONE || a.weights.values().any(|w| w.0 > Bps::ONE) {
        return Err(rule(
            "R14",
            "claims.anomaly: near_sum_insured and every weight are at most 1",
        ));
    }
    Ok(ClaimsConfig {
        stp_limit: raw.stp_limit.0,
        stp_min_confidence: raw.stp_min_confidence.0,
        estimate_tolerance: raw.estimate_tolerance.0,
        referral_sla_hours: raw.referral_sla_hours,
        anomaly: AnomalyConfig {
            early_claim_days: a.early_claim_days,
            late_notification_days: a.late_notification_days,
            frequent_claims: a.frequent_claims,
            near_sum_insured: a.near_sum_insured.0,
            weights: a.weights.into_iter().map(|(k, v)| (k, v.0)).collect(),
        },
    })
}
