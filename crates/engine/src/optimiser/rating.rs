//! Policy-period rating: how much cover each premium HK$ buys (R31), averaged over the rating
//! horizon (R32), from registered factors only (R33), with peer-relative claims experience (R39)
//! and candidate factors in shadow (R43). The plan price never moves here (R01).
use crate::config::{
    EngineConfig, FactorData, FactorLevel, FactorStatus, LinesOfBusiness, RiskClass,
};
use crate::money::{Bps, Rate};
use crate::types::{
    ClassId, CoverId, CoverSpec, DataTypeId, FactorId, add_policy_periods, sub_policy_periods,
};
use crate::{EngineError, Result};
use chrono::NaiveDate;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

/// Market data for rating, from the economic models (trigger pricing) and the insurer's filed
/// rates. Never set by AI (R02).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RatingBasis {
    /// Portfolio expected claim cost per unit of sum insured, by cover and policy period start.
    pub expected_cost: BTreeMap<CoverId, BTreeMap<NaiveDate, Rate>>,
    /// Relativity tables of individual factors, by factor and the class of business each table
    /// is filed for (a parent class covers its subtree).
    pub factor_tables: BTreeMap<FactorId, BTreeMap<ClassId, FactorTable>>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct FactorTable {
    pub relativities: BTreeMap<String, Bps>,
    /// Customers who withheld this factor (R33).
    pub undeclared: UndeclaredGroup,
}

/// The observed average of customers who withheld a factor, and how much data it rests on.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
pub struct UndeclaredGroup {
    pub relativity: Bps,
    pub exposure_periods: u32,
}

/// What rating knows about one customer. Kept apart from `Profile`, which reaches the AI layer:
/// protected attributes registered as factors (R19) live only here.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RatingInputs {
    /// Values of individual factors: declared answers, or values derived upstream from
    /// consented data. A consented factor counts only while its opt-in holds (R20).
    pub factor_values: BTreeMap<FactorId, String>,
    pub classes: BTreeMap<ClassId, ClassHistory>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ClassHistory {
    /// Consecutive policy periods covered in this class up to the one being rated (R32).
    pub continuous_periods: u32,
    pub experience: ClaimsExperience,
}

/// The customer's claims and payouts in one class of business, with the peer comparison (R39).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ClaimsExperience {
    pub claims: Vec<ExperienceClaim>,
    /// Policy periods this customer was covered in the class within the experience window.
    pub exposure_periods: u32,
    /// Claims per policy period, in millionths, among peers with the same rated exposure over
    /// the same periods. A shock that hits everyone raises this too, so it cancels out.
    pub peer_frequency_ppm: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExperienceClaim {
    /// Start of the policy period the claim or payout arose in.
    pub period_starts: NaiveDate,
    pub status: ClaimStatus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClaimStatus {
    Paid,
    /// Counts only once decided (R39).
    UnderReview,
    NotPaid,
}

/// The rate of one cover for one customer and policy period, with what it rests on (R49).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct CoverRating {
    pub cover_id: CoverId,
    pub class_of_business: ClassId,
    /// Premium per unit of sum insured.
    pub rate: Rate,
    /// Expected claim cost per unit of sum insured used: the horizon average, or for a new or
    /// re-entering customer the higher of that and this period's own (R32).
    pub base_cost: Rate,
    /// Customer expected claims / portfolio, from approved factors and experience (R39).
    pub relativity: Bps,
    /// As `relativity`, also including candidate factors. Never applied (R43).
    pub shadow_relativity: Bps,
    /// The claims experience multiplier included in `relativity` (1.0 = none).
    pub experience: Bps,
    pub risk_class: RiskClass,
    pub factors: Vec<AppliedFactor>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct AppliedFactor {
    pub factor_id: FactorId,
    pub relativity: Bps,
    /// Not disclosed or not consented: rated at the undeclared group average (R33).
    pub withheld: bool,
    /// A candidate factor, rated in shadow only (R43).
    pub shadow: bool,
}

fn r33(detail: String) -> EngineError {
    EngineError::Rule {
        rule: "R33",
        detail,
    }
}

/// The expected cost in force for a period starting `starts`: the curve's entry effective on
/// that day, as a filed rate applies to policies incepting or renewing from its effective date
/// (ADR-011). An entry is effective for one policy period from its date, so a curve that has run
/// out is an error, never a stale rate.
fn effective_cost(curve: &BTreeMap<NaiveDate, Rate>, starts: NaiveDate) -> Option<Rate> {
    curve
        .range(..=starts)
        .next_back()
        .filter(|(from, _)| add_policy_periods(**from, 1) > starts)
        .map(|(_, cost)| *cost)
}

/// This period's own expected cost and the average over the rating horizon (R32).
pub fn horizon_cost(
    cover_id: &str,
    curve: &BTreeMap<NaiveDate, Rate>,
    period_start: NaiveDate,
    horizon: u32,
) -> Result<(Rate, Rate)> {
    let mut sum: u64 = 0;
    let mut own = Rate(0);
    for i in 0..horizon {
        let starts = add_policy_periods(period_start, i);
        let cost = effective_cost(curve, starts).ok_or_else(|| {
            EngineError::Config(format!(
                "rating basis has no expected cost for {cover_id} in the period from {starts}"
            ))
        })?;
        if i == 0 {
            own = cost;
        }
        sum += u64::from(cost.0);
    }
    let n = u64::from(horizon.max(1));
    let avg = u32::try_from((sum + n / 2) / n).unwrap_or(u32::MAX);
    Ok((own, Rate(avg)))
}

/// R32: continuously covered customers pay the horizon average; new or re-entering customers
/// the higher of this period's own cost and that average, so joining for a peak never pays.
pub fn base_cost(
    own: Rate,
    horizon_average: Rate,
    continuous_periods: u32,
    cfg: &EngineConfig,
) -> Rate {
    if continuous_periods < cfg.pricing.continuous_cover_periods {
        own.max(horizon_average)
    } else {
        horizon_average
    }
}

/// R31: premium per unit of sum insured. Rising risk raises it, so the same premium buys less.
pub fn rate(base_cost: Rate, relativity: Bps, target_loss_ratio: Bps) -> Rate {
    base_cost.mul_bps(relativity).div_bps(target_loss_ratio)
}

/// R39: the insurer's standard class for a relativity; above the last threshold, postponed.
pub fn risk_class(relativity: Bps, cfg: &EngineConfig) -> RiskClass {
    let classes = &cfg.underwriting.risk_classes;
    classes
        .iter()
        .find(|c| c.max_relativity.is_some_and(|max| relativity <= max))
        .or(classes.last())
        .cloned()
        .unwrap_or(RiskClass {
            id: "postponed".into(),
            max_relativity: None,
        })
}

/// R39: claims experience as evidence of exposure. Own paid frequency over the window against
/// peers in the same periods; counts only above the threshold, credibility-weighted. `weight`
/// scales it for an experience link from another class.
pub fn experience_factor(
    exp: &ClaimsExperience,
    period_start: NaiveDate,
    weight: Bps,
    cfg: &EngineConfig,
) -> Bps {
    let uw = &cfg.underwriting;
    let window_start = sub_policy_periods(period_start, uw.claims_experience_periods);
    let paid = exp
        .claims
        .iter()
        .filter(|c| c.status == ClaimStatus::Paid)
        .filter(|c| window_start <= c.period_starts && c.period_starts < period_start)
        .count();
    if exp.exposure_periods == 0 || exp.peer_frequency_ppm == 0 {
        return Bps::ONE;
    }
    let paid = u64::try_from(paid).unwrap_or(u64::MAX);
    let own_ppm = paid * 1_000_000 / u64::from(exp.exposure_periods);
    let experience = Bps::ratio(own_ppm, u64::from(exp.peer_frequency_ppm));
    if experience <= uw.claims_experience_threshold {
        return Bps::ONE;
    }
    let full = uw.credibility_full_periods;
    let z = Bps::ratio(u64::from(exp.exposure_periods.min(full)), u64::from(full));
    let excess = Bps(experience.0 - Bps::ONE.0);
    Bps(Bps::ONE.0.saturating_add((weight * (z * excess)).0))
}

fn table_for<'a>(factor_id: &str, class: &str, basis: &'a RatingBasis) -> Result<&'a FactorTable> {
    basis
        .factor_tables
        .get(factor_id)
        .and_then(|tables| {
            tables
                .iter()
                .filter(|(filed, _)| LinesOfBusiness::contains(filed, class))
                .max_by_key(|(filed, _)| filed.len())
                .map(|(_, t)| t)
        })
        .ok_or_else(|| {
            r33(format!(
                "no relativity table for factor {factor_id} in {class}"
            ))
        })
}

/// Rates one cover for one customer and policy period.
pub fn rate_cover(
    cover: &CoverSpec,
    period_start: NaiveDate,
    inputs: &RatingInputs,
    consents: &BTreeSet<DataTypeId>,
    basis: &RatingBasis,
    cfg: &EngineConfig,
) -> Result<CoverRating> {
    for id in inputs.factor_values.keys() {
        match cfg.rating_factors.get(id) {
            Some(f) if f.level == FactorLevel::Individual => {}
            _ => {
                return Err(r33(format!(
                    "{id} is not a registered individual rating factor"
                )));
            }
        }
    }
    let class = &cover.class_of_business;
    let mut relativity = Bps::ONE;
    let mut shadow_relativity = Bps::ONE;
    let mut factors = Vec::new();
    let mut experience_rated = false;
    for f in cfg.rating_factors.values() {
        if f.level != FactorLevel::Individual || !f.rates(class) {
            continue;
        }
        let shadow = f.status == FactorStatus::Candidate;
        if f.data == Some(FactorData::OwnClaims) {
            experience_rated |= !shadow;
            continue;
        }
        let available = match &f.data {
            Some(FactorData::Consented(d)) => consents.contains(d),
            _ => true,
        };
        let value = inputs.factor_values.get(&f.id).filter(|_| available);
        let table = match table_for(&f.id, class, basis) {
            Ok(t) => t,
            // A candidate without a table yet has nothing to rate in shadow (R43).
            Err(_) if shadow => continue,
            Err(e) => return Err(e),
        };
        let rel = match value {
            Some(v) => *table.relativities.get(v).ok_or_else(|| {
                r33(format!(
                    "{v:?} has no relativity for factor {} in {class}",
                    f.id
                ))
            })?,
            None => {
                let g = table.undeclared;
                let full = cfg.pricing.undeclared_credibility_full_exposure;
                let z = Bps::ratio(u64::from(g.exposure_periods.min(full)), u64::from(full));
                // Z × group + (1 − Z) × portfolio, with the portfolio at 1.0.
                let blended = i64::from(Bps::ONE.0)
                    + i64::from(z.0) * (i64::from(g.relativity.0) - i64::from(Bps::ONE.0))
                        / i64::from(Bps::ONE.0);
                Bps::saturating(blended)
            }
        };
        if !shadow {
            relativity = relativity * rel;
        }
        shadow_relativity = shadow_relativity * rel;
        factors.push(AppliedFactor {
            factor_id: f.id.clone(),
            relativity: rel,
            withheld: value.is_none(),
            shadow,
        });
    }

    let mut experience = Bps::ONE;
    if experience_rated {
        if let Some(h) = inputs.classes.get(class) {
            experience = experience * experience_factor(&h.experience, period_start, Bps::ONE, cfg);
        }
        for link in cfg.lines.experience_links.iter().filter(|l| &l.to == class) {
            if let Some(h) = inputs.classes.get(&link.from) {
                experience =
                    experience * experience_factor(&h.experience, period_start, link.weight, cfg);
            }
        }
    }
    relativity = relativity * experience;
    shadow_relativity = shadow_relativity * experience;

    let curve = basis.expected_cost.get(&cover.id).ok_or_else(|| {
        EngineError::Config(format!(
            "rating basis has no expected cost for {}",
            cover.id
        ))
    })?;
    let (own, avg) = horizon_cost(
        &cover.id,
        curve,
        period_start,
        cfg.pricing.rating_horizon_periods,
    )?;
    let continuous = inputs
        .classes
        .get(class)
        .map_or(0, |h| h.continuous_periods);
    let base = base_cost(own, avg, continuous, cfg);
    Ok(CoverRating {
        cover_id: cover.id.clone(),
        class_of_business: class.clone(),
        rate: rate(base, relativity, cfg.pricing.target_loss_ratio),
        base_cost: base,
        relativity,
        shadow_relativity,
        experience,
        risk_class: risk_class(relativity, cfg),
        factors,
    })
}

/// Rates every listed cover.
pub fn rate_covers(
    cover_ids: &[String],
    period_start: NaiveDate,
    inputs: &RatingInputs,
    consents: &BTreeSet<DataTypeId>,
    basis: &RatingBasis,
    cfg: &EngineConfig,
) -> Result<BTreeMap<CoverId, CoverRating>> {
    cover_ids
        .iter()
        .map(|id| {
            let cover = cfg.cover(id)?;
            rate_cover(cover, period_start, inputs, consents, basis, cfg).map(|r| (id.clone(), r))
        })
        .collect()
}
