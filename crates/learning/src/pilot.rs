//! Pilot metrics (docs/07-pilot-and-metrics.md, ADR-012): what the book shows about the pilot,
//! for the gates set before it started. Every figure is an aggregate; the caller builds the facts
//! from the policy store, and `microinsurance_economics.pilot_evaluation` evaluates the report
//! against the gates. A metric with nothing to measure is reported without a value, and a gate on
//! it never passes.
//!
//! Claims avoided are measured against the wait-list control group (R59): claim frequency on
//! loss-based covers among customers offered prevention tasks against those not yet offered any.
use crate::insights::Unit;
use serde::{Deserialize, Serialize};

/// One customer, as the pilot counts them. No identity.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PilotCustomer {
    /// In the wait-list control group (R59).
    pub control: bool,
    pub quoted: bool,
    pub bound: bool,
    /// Their first policy period has ended, so they could have renewed.
    pub due_to_renew: bool,
    /// They hold a second policy period or later.
    pub renewed: bool,
    pub cancelled: bool,
    pub tasks_offered: u32,
    pub tasks_verified: u32,
    /// Policy periods holding a loss-based (indemnity) cover, and the claims on them.
    pub loss_periods: u32,
    pub loss_claims: u32,
    pub rewards_granted: u32,
    pub rewards_redeemed: u32,
}

/// The book's totals beyond each customer.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PilotBook {
    pub customers: Vec<PilotCustomer>,
    pub premium_minor: i64,
    /// Parametric payouts and claim payments, as decided.
    pub paid_minor: i64,
    /// Indemnity claims decided, and those approved straight through (R57).
    pub claims_decided: u32,
    pub claims_straight_through: u32,
    /// Hours from notification of loss to approval, per approved claim.
    pub claim_payment_hours: Vec<i64>,
    /// The random audit sample's findings, and those where the person disagreed (R14).
    pub audits_reviewed: u32,
    pub audits_overturned: u32,
    /// Fraud a person confirmed, against all decisions logged (R10, ADR-010).
    pub fraud_confirmed: u32,
    pub decisions: u32,
    /// The lowest adverse impact ratio among credible groups, in basis points (R19).
    pub fairness_min_ratio_bps: Option<i64>,
}

/// One pilot metric.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct PilotMetric {
    pub metric: String,
    pub unit: Unit,
    /// None when there is nothing to measure yet.
    pub value: Option<i64>,
    /// What the figure rests on: customers, claims, findings or decisions.
    pub sample: u32,
}

fn rate(numerator: u64, denominator: u64) -> Option<i64> {
    (denominator > 0).then(|| {
        i64::try_from(u128::from(numerator) * 10_000 / u128::from(denominator)).unwrap_or(i64::MAX)
    })
}

fn n(count: usize) -> u32 {
    u32::try_from(count).unwrap_or(u32::MAX)
}

/// The value at the `pct` percentile (nearest rank) of `values`.
pub fn percentile(values: &[i64], pct: u32) -> Option<i64> {
    if values.is_empty() {
        return None;
    }
    let mut sorted = values.to_vec();
    sorted.sort_unstable();
    let rank = (u64::from(pct.min(100)) * u64::try_from(sorted.len()).unwrap_or(u64::MAX))
        .div_ceil(100)
        .max(1);
    sorted.get(usize::try_from(rank - 1).unwrap_or(0)).copied()
}

fn sum<T: Into<u64>>(values: impl Iterator<Item = T>) -> u64 {
    values.map(Into::into).sum()
}

/// The pilot's metrics, in the order of docs/07-pilot-and-metrics.md.
pub fn metrics(book: &PilotBook) -> Vec<PilotMetric> {
    let m = |metric: &str, unit: Unit, value: Option<i64>, sample: u32| PilotMetric {
        metric: metric.to_string(),
        unit,
        value,
        sample,
    };
    let c = &book.customers;
    let quoted: Vec<&PilotCustomer> = c.iter().filter(|x| x.quoted).collect();
    let bound: Vec<&PilotCustomer> = c.iter().filter(|x| x.bound).collect();
    let due: Vec<&&PilotCustomer> = bound.iter().filter(|x| x.due_to_renew).collect();
    let offered: Vec<&&PilotCustomer> = bound.iter().filter(|x| x.tasks_offered > 0).collect();
    let group = |control: bool| -> (u64, u64, u32) {
        let g: Vec<&&PilotCustomer> = bound.iter().filter(|x| x.control == control).collect();
        (
            sum(g.iter().map(|x| x.loss_claims)),
            sum(g.iter().map(|x| x.loss_periods)),
            n(g.len()),
        )
    };
    let (treated_claims, treated_periods, treated_n) = group(false);
    let (control_claims, control_periods, control_n) = group(true);
    let treated = rate(treated_claims, treated_periods);
    let control = rate(control_claims, control_periods);
    let avoided = match (treated, control) {
        (Some(t), Some(c)) if c > 0 => Some((c - t) * 10_000 / c),
        _ => None,
    };
    let granted = sum(c.iter().map(|x| x.rewards_granted));
    vec![
        // Customer
        m(
            "conversion_rate",
            Unit::Bps,
            rate(n(bound.len()).into(), n(quoted.len()).into()),
            n(quoted.len()),
        ),
        m(
            "renewal_rate",
            Unit::Bps,
            rate(
                n(due.iter().filter(|x| x.renewed).count()).into(),
                n(due.len()).into(),
            ),
            n(due.len()),
        ),
        m(
            "retention_rate",
            Unit::Bps,
            rate(
                n(bound.iter().filter(|x| !x.cancelled).count()).into(),
                n(bound.len()).into(),
            ),
            n(bound.len()),
        ),
        // Prevention
        m(
            "prevention_completion_rate",
            Unit::Bps,
            rate(
                sum(offered.iter().map(|x| x.tasks_verified)),
                sum(offered.iter().map(|x| x.tasks_offered)),
            ),
            n(offered.len()),
        ),
        m("claim_frequency_treatment", Unit::Bps, treated, treated_n),
        m("claim_frequency_control", Unit::Bps, control, control_n),
        m(
            "claims_avoided_vs_control",
            Unit::Bps,
            avoided,
            treated_n.saturating_add(control_n),
        ),
        // Claims
        m(
            "straight_through_rate",
            Unit::Bps,
            rate(
                book.claims_straight_through.into(),
                book.claims_decided.into(),
            ),
            book.claims_decided,
        ),
        m(
            "claim_payment_hours_p95",
            Unit::Hours,
            percentile(&book.claim_payment_hours, 95),
            n(book.claim_payment_hours.len()),
        ),
        m(
            "audit_disagreement_rate",
            Unit::Bps,
            rate(book.audits_overturned.into(), book.audits_reviewed.into()),
            book.audits_reviewed,
        ),
        // Fraud
        m(
            "fraud_rate",
            Unit::Bps,
            rate(book.fraud_confirmed.into(), book.decisions.into()),
            book.decisions,
        ),
        // Rewards
        m(
            "reward_redemption_rate",
            Unit::Bps,
            rate(sum(c.iter().map(|x| x.rewards_redeemed)), granted),
            u32::try_from(granted).unwrap_or(u32::MAX),
        ),
        // Insurer
        m(
            "loss_ratio",
            Unit::Bps,
            (book.premium_minor > 0).then(|| {
                i64::try_from(i128::from(book.paid_minor) * 10_000 / i128::from(book.premium_minor))
                    .unwrap_or(i64::MAX)
            }),
            n(bound.len()),
        ),
        // Fairness
        m(
            "fairness_min_parity",
            Unit::Bps,
            book.fairness_min_ratio_bps,
            book.claims_decided,
        ),
    ]
}
