//! Experience studies (ADR-012): the data loop that turns what happened in the book into an
//! indication for the actuary. Every row is an aggregate by class of business and policy period
//! (or by prevention measure); no customer id enters or leaves. A study never changes a rate: the
//! actuary turns its indication into the next rating basis and prevention basis and files them
//! (`python/economics` `experience`, R18, R31, R43).
//!
//! - **Actual against expected (A/E).** Per class and policy period: the claims and payouts
//!   incurred against the expected claim cost of the rating basis in force for the period.
//! - **Credibility.** Limited-fluctuation credibility on the number of claims:
//!   `Z = min(1, sqrt(n / n_full))`, with `n_full` the claims for full credibility (1,082 is the
//!   classical standard: 90% probability of being within 5% of the expected).
//! - **Prevention.** Per class and measure: claim frequency in policy periods with a verified
//!   activity for the measure against those without, the evidence behind each measure's claim
//!   probabilities with and without it (R18, R40).
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// One cover in one customer's policy period, as recorded. The caller builds one per period and
/// cover; the study only sums them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExposureFacts {
    pub class_of_business: String,
    /// The policy period's month, `YYYY-MM`.
    pub period: String,
    pub premium_minor: i64,
    pub sum_insured_minor: i64,
    /// Expected claim cost for the period from the rating basis in force, in minor units.
    pub expected_minor: i64,
    /// Claims and parametric payouts arising in the period.
    pub claims: u32,
    /// What they paid, as decided.
    pub incurred_minor: i64,
}

/// One class of business in one policy period.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct StudyRow {
    pub class_of_business: String,
    pub period: String,
    /// Policy periods in force (exposure).
    pub policy_periods: u32,
    pub premium_minor: i64,
    pub sum_insured_minor: i64,
    pub expected_minor: i64,
    pub claims: u32,
    pub incurred_minor: i64,
    /// Incurred ÷ expected in basis points; none without an expected cost.
    pub actual_to_expected_bps: Option<i64>,
    /// Incurred ÷ premium in basis points; none without premium.
    pub loss_ratio_bps: Option<i64>,
    /// The weight the actuary gives this experience against the basis, in basis points.
    pub credibility_bps: i64,
}

/// One customer's policy period in a class with a prevention measure: whether they had a verified
/// activity for it, and the claims that period.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PreventionFacts {
    pub class_of_business: String,
    pub measure: String,
    pub verified: bool,
    pub claims: u32,
}

/// One prevention measure: claim frequency with and without a verified activity.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct MeasureRow {
    pub class_of_business: String,
    pub measure: String,
    pub periods_with: u32,
    pub claims_with: u32,
    pub periods_without: u32,
    pub claims_without: u32,
    /// Claims per policy period, in basis points.
    pub frequency_with_bps: Option<i64>,
    pub frequency_without_bps: Option<i64>,
    /// On the claims of both groups together.
    pub credibility_bps: i64,
}

/// A study over a book, for the actuary.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct ExperienceStudy {
    /// The month the study runs to, `YYYY-MM`.
    pub as_of: String,
    pub full_credibility_claims: u32,
    pub classes: Vec<StudyRow>,
    pub prevention: Vec<MeasureRow>,
}

/// The classical limited-fluctuation standard: 90% probability within 5% of the expected.
pub const FULL_CREDIBILITY_CLAIMS: u32 = 1_082;

/// `min(1, sqrt(claims / full))` in basis points, in integers.
pub fn credibility_bps(claims: u64, full: u32) -> i64 {
    let full = u64::from(full.max(1));
    if claims >= full {
        return 10_000;
    }
    i64::try_from((claims.saturating_mul(100_000_000) / full).isqrt()).unwrap_or(10_000)
}

fn ratio_bps(numerator: i64, denominator: i64) -> Option<i64> {
    (denominator > 0).then(|| {
        i64::try_from(i128::from(numerator) * 10_000 / i128::from(denominator)).unwrap_or(i64::MAX)
    })
}

/// The study's rows by class and period, in that order.
pub fn study(facts: &[ExposureFacts], full_credibility_claims: u32) -> Vec<StudyRow> {
    let mut by: BTreeMap<(&str, &str), StudyRow> = BTreeMap::new();
    for f in facts {
        let row = by
            .entry((f.class_of_business.as_str(), f.period.as_str()))
            .or_insert_with(|| StudyRow {
                class_of_business: f.class_of_business.clone(),
                period: f.period.clone(),
                policy_periods: 0,
                premium_minor: 0,
                sum_insured_minor: 0,
                expected_minor: 0,
                claims: 0,
                incurred_minor: 0,
                actual_to_expected_bps: None,
                loss_ratio_bps: None,
                credibility_bps: 0,
            });
        row.policy_periods += 1;
        row.premium_minor = row.premium_minor.saturating_add(f.premium_minor);
        row.sum_insured_minor = row.sum_insured_minor.saturating_add(f.sum_insured_minor);
        row.expected_minor = row.expected_minor.saturating_add(f.expected_minor);
        row.claims = row.claims.saturating_add(f.claims);
        row.incurred_minor = row.incurred_minor.saturating_add(f.incurred_minor);
    }
    by.into_values()
        .map(|mut r| {
            r.actual_to_expected_bps = ratio_bps(r.incurred_minor, r.expected_minor);
            r.loss_ratio_bps = ratio_bps(r.incurred_minor, r.premium_minor);
            r.credibility_bps = credibility_bps(u64::from(r.claims), full_credibility_claims);
            r
        })
        .collect()
}

/// The prevention rows by class and measure.
pub fn prevention_study(
    facts: &[PreventionFacts],
    full_credibility_claims: u32,
) -> Vec<MeasureRow> {
    let mut by: BTreeMap<(&str, &str), MeasureRow> = BTreeMap::new();
    for f in facts {
        let row = by
            .entry((f.class_of_business.as_str(), f.measure.as_str()))
            .or_insert_with(|| MeasureRow {
                class_of_business: f.class_of_business.clone(),
                measure: f.measure.clone(),
                periods_with: 0,
                claims_with: 0,
                periods_without: 0,
                claims_without: 0,
                frequency_with_bps: None,
                frequency_without_bps: None,
                credibility_bps: 0,
            });
        if f.verified {
            row.periods_with += 1;
            row.claims_with = row.claims_with.saturating_add(f.claims);
        } else {
            row.periods_without += 1;
            row.claims_without = row.claims_without.saturating_add(f.claims);
        }
    }
    by.into_values()
        .map(|mut r| {
            r.frequency_with_bps = ratio_bps(i64::from(r.claims_with), i64::from(r.periods_with));
            r.frequency_without_bps =
                ratio_bps(i64::from(r.claims_without), i64::from(r.periods_without));
            r.credibility_bps = credibility_bps(
                u64::from(r.claims_with) + u64::from(r.claims_without),
                full_credibility_claims,
            );
            r
        })
        .collect()
}
