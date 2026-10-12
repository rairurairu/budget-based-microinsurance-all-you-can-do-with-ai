//! The production engine, callable from Python, so economic models and backtests run the same
//! code as the live system. Money is in cents, ratios in basis points, rates in parts per
//! million; structured inputs and outputs are JSON in the shapes of `contracts/`.
use chrono::{Datelike, NaiveDate};
use microinsurance_engine::config::{self, ConfigSources, EngineConfig};
use microinsurance_engine::money::{Bps, Money, Rate};
use microinsurance_engine::optimiser::rating::{self, RatingBasis, RatingInputs};
use microinsurance_engine::optimiser::{self, PeriodContext};
use microinsurance_engine::plans;
use microinsurance_engine::prevention::{self, PreventionTaskDraft};
use microinsurance_engine::triggers::{self, SourceReport, TriggerEvent};
use microinsurance_engine::types::{
    Allocation, Policy, PolicyPeriod, Profile, Schedule, WorkWindow, add_policy_periods,
};
use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use std::collections::{BTreeMap, BTreeSet};

fn py_err(e: impl std::fmt::Display) -> PyErr {
    PyValueError::new_err(e.to_string())
}

fn draft(
    p_before_bps: u32,
    p_after_bps: u32,
    avg_claim_cost_cents: i64,
    reward_cents: i64,
) -> PreventionTaskDraft {
    PreventionTaskDraft {
        task_type: String::new(),
        cover_id: String::new(),
        event_reference: None,
        action: String::new(),
        evidence: String::new(),
        claim_prob_before: Bps(p_before_bps),
        claim_prob_after: Bps(p_after_bps),
        avg_claim_cost: Money(avg_claim_cost_cents),
        reward_cost: Money(reward_cents),
    }
}

/// R18 with the engine's integer arithmetic. Probabilities in basis points, money in cents.
#[pyfunction]
fn prevention_test_passes(
    reward_cost_cents: i64,
    p_before_bps: u32,
    p_after_bps: u32,
    avg_claim_cost_cents: i64,
) -> bool {
    prevention::admit(&draft(
        p_before_bps,
        p_after_bps,
        avg_claim_cost_cents,
        reward_cost_cents,
    ))
    .is_ok()
}

/// Expected claim cost avoided per customer per month, in cents (R18).
#[pyfunction]
fn claims_avoided_cents(p_before_bps: u32, p_after_bps: u32, avg_claim_cost_cents: i64) -> i64 {
    prevention::claims_avoided(&draft(p_before_bps, p_after_bps, avg_claim_cost_cents, 0)).0
}

/// `cents × bps / 10,000`, rounded half-up away from zero (ADR-004).
#[pyfunction]
fn mul_bps(cents: i64, bps: u32) -> i64 {
    Money(cents).mul_bps(Bps(bps)).0
}

/// R32: (this period's own cost, average over the horizon) from consecutive monthly costs in ppm.
#[pyfunction]
fn horizon_cost_ppm(costs_ppm: Vec<u32>) -> PyResult<(u32, u32)> {
    let start = NaiveDate::from_ymd_opt(2000, 1, 1).ok_or_else(|| py_err("date"))?;
    let horizon = u32::try_from(costs_ppm.len()).map_err(py_err)?;
    let curve: BTreeMap<NaiveDate, Rate> = costs_ppm
        .iter()
        .zip(0..)
        .map(|(c, i)| (add_policy_periods(start, i), Rate(*c)))
        .collect();
    let (own, avg) = rating::horizon_cost("curve", &curve, start, horizon).map_err(py_err)?;
    Ok((own.0, avg.0))
}

/// The validated configuration, loaded from the texts of the `config/` files.
#[pyclass(frozen)]
struct Engine {
    cfg: EngineConfig,
}

#[pymethods]
impl Engine {
    #[new]
    #[allow(clippy::too_many_arguments)]
    fn new(
        insurer: &str,
        lines_of_business: &str,
        rating_factors: &str,
        event_types: &str,
        trigger_sources: Vec<String>,
        exposure_types: &str,
        data_types: &str,
        catalogue: Vec<String>,
        prevention_measures: &str,
        suppliers: &str,
    ) -> PyResult<Engine> {
        let catalogue: Vec<&str> = catalogue.iter().map(String::as_str).collect();
        let trigger_sources: Vec<&str> = trigger_sources.iter().map(String::as_str).collect();
        let cfg = config::from_yaml(&ConfigSources {
            insurer,
            lines_of_business,
            rating_factors,
            event_types: &[event_types],
            trigger_sources: &trigger_sources,
            exposure_types,
            data_types,
            catalogue: &catalogue,
            prevention_measures,
            suppliers,
            pilot: None,
        })
        .map_err(py_err)?;
        Ok(Engine { cfg })
    }

    /// R32: the expected cost this customer is rated on.
    fn base_cost_ppm(
        &self,
        own_ppm: u32,
        horizon_average_ppm: u32,
        continuous_periods: u32,
    ) -> u32 {
        rating::base_cost(
            Rate(own_ppm),
            Rate(horizon_average_ppm),
            continuous_periods,
            &self.cfg,
        )
        .0
    }

    /// R31: premium per unit of sum insured, in ppm.
    fn rate_ppm(&self, base_cost_ppm: u32, relativity_bps: u32) -> u32 {
        rating::rate(
            Rate(base_cost_ppm),
            Bps(relativity_bps),
            self.cfg.pricing.target_loss_ratio,
        )
        .0
    }

    /// R39: the risk class id for a relativity.
    fn risk_class(&self, relativity_bps: u32) -> String {
        rating::risk_class(Bps(relativity_bps), &self.cfg).id
    }

    /// R40: one customer's reward for a measure, in cents, with the claims it avoids:
    /// `(reward_cents, claims_avoided_cents)`.
    fn prevention_reward_cents(
        &self,
        p_without_bps: u32,
        p_with_bps: u32,
        avg_claim_cost_cents: i64,
        relativity_bps: u32,
        sum_insured_cents: i64,
    ) -> (i64, i64) {
        let terms = prevention::customer_terms(
            &prevention::MeasureClaims {
                claim_prob_without: Bps(p_without_bps),
                claim_prob_with: Bps(p_with_bps),
                avg_claim_cost: Money(avg_claim_cost_cents),
            },
            Bps(relativity_bps),
            Money(sum_insured_cents),
            self.cfg.rewards.reward_share_of_claims_avoided,
        );
        (terms.reward.0, terms.claims_avoided.0)
    }

    /// R02: the optimiser's suggestion for one customer, as `Suggestion` JSON. Rates every
    /// cover of the plan first (R31–R33). All classes open, no event in force, no zone full.
    fn suggest(
        &self,
        plan_id: &str,
        period_start: &str,
        profile_json: &str,
        rating_inputs_json: &str,
        rating_basis_json: &str,
    ) -> PyResult<String> {
        let plan = self
            .cfg
            .plan(plan_id)
            .ok_or_else(|| py_err(format!("unknown plan {plan_id}")))?;
        let starts: NaiveDate = period_start.parse().map_err(py_err)?;
        let profile: Profile = serde_json::from_str(profile_json).map_err(py_err)?;
        let inputs: RatingInputs = serde_json::from_str(rating_inputs_json).map_err(py_err)?;
        let basis: RatingBasis = serde_json::from_str(rating_basis_json).map_err(py_err)?;
        let ratings = rating::rate_covers(
            &plans::covers_for(plan, &self.cfg),
            starts,
            &inputs,
            &profile.consents,
            &basis,
            &self.cfg,
        )
        .map_err(py_err)?;
        let (statuses, in_force, full_zones) = (BTreeMap::new(), BTreeSet::new(), BTreeSet::new());
        let ctx = PeriodContext {
            cfg: &self.cfg,
            period: PolicyPeriod {
                index: 0,
                starts,
                ends: add_policy_periods(starts, 1),
            },
            ratings: &ratings,
            statuses: &statuses,
            in_force: &in_force,
            full_zones: &full_zones,
        };
        let suggestion = optimiser::suggest(plan, &profile, None, &ctx).map_err(py_err)?;
        serde_json::to_string(&suggestion).map_err(py_err)
    }

    /// R31, R55: trigger pricing on a source's historical record. Replays the record (reports
    /// with their ends, as `SourceReport` JSON) through the production matcher for one cover and
    /// one declared work pattern (`WorkWindow` JSON), loss occurrence by loss occurrence (R51), and
    /// returns what it would have paid, by month of each occurrence's start (`YYYY-MM`), in ppm
    /// of the benefit amount.
    fn payout_by_month_ppm(
        &self,
        reports_json: &str,
        cover_id: &str,
        work_windows_json: &str,
    ) -> PyResult<BTreeMap<String, i64>> {
        let reports: Vec<SourceReport> = serde_json::from_str(reports_json).map_err(py_err)?;
        let work_windows: Vec<WorkWindow> =
            serde_json::from_str(work_windows_json).map_err(py_err)?;
        let cover = self.cfg.cover(cover_id).map_err(py_err)?;
        let plan = self.cfg.plans.first().ok_or_else(|| py_err("no plans"))?;
        // A benefit amount of 1,000,000 cents makes an entitlement in cents a ppm.
        let unit = Money(i64::from(Rate::PER_UNIT));
        let long_ago = NaiveDate::from_ymd_opt(1900, 1, 1).ok_or_else(|| py_err("date"))?;
        let offset = self.cfg.local_offset();
        let mut out: BTreeMap<String, i64> = BTreeMap::new();
        let events: BTreeMap<String, TriggerEvent> = triggers::from_history(&reports, &self.cfg)
            .into_iter()
            .map(|e| (e.event_id.clone(), e))
            .collect();
        let history: Vec<TriggerEvent> = events.values().cloned().collect();
        for occurrence in triggers::group_history(&history, &self.cfg) {
            let local = occurrence.starts.with_timezone(&offset).date_naive();
            let month = local.with_day(1).ok_or_else(|| py_err("date"))?;
            let policy = Policy {
                policy_id: "pricing".into(),
                customer_id: "pricing".into(),
                plan: plan.clone(),
                schedule: Schedule {
                    plan_id: plan.id.clone(),
                    allocations: vec![Allocation {
                        cover_id: cover.id.clone(),
                        premium: Money::ZERO,
                        sum_insured: unit,
                    }],
                },
                period: PolicyPeriod {
                    index: 0,
                    starts: month,
                    ends: add_policy_periods(month, 1),
                },
                cover_starts: [(cover.id.clone(), long_ago)].into(),
                work_windows: work_windows.clone(),
                commute_routes: Vec::new(),
                work_areas: Vec::new(),
                upcoming_events: Vec::new(),
                subscribed_at: occurrence.starts,
            };
            let policies = [policy];
            let index = triggers::build_index(&policies, &self.cfg.catalogue);
            let as_of = occurrence.quiet_since.unwrap_or(occurrence.starts);
            let paid: i64 = triggers::match_occurrence(
                &occurrence,
                &events,
                &policies,
                &index,
                as_of,
                &self.cfg,
            )
            .iter()
            .map(|m| m.entitlement.0)
            .sum();
            *out.entry(month.format("%Y-%m").to_string()).or_default() += paid;
        }
        Ok(out)
    }
}

#[pymodule]
fn microinsurance_py(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(prevention_test_passes, m)?)?;
    m.add_function(wrap_pyfunction!(claims_avoided_cents, m)?)?;
    m.add_function(wrap_pyfunction!(mul_bps, m)?)?;
    m.add_function(wrap_pyfunction!(horizon_cost_ppm, m)?)?;
    m.add_class::<Engine>()?;
    Ok(())
}
