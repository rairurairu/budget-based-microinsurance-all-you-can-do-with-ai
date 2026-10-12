//! What rating one customer for one policy period needs from the policy store: their answers to
//! the rating factors, how long they have held each class (R32), and the trigger zones over the
//! accumulation limit (R51). The API (quotes, the renewal view) and the renewal jobs both rate
//! through here, so a notice and a quote rate alike.
use crate::book::{PolicyRecord, RatingAnswers};
use chrono::{DateTime, Duration, Utc};
use microinsurance_engine::EngineError;
use microinsurance_engine::config::EngineConfig;
use microinsurance_engine::optimiser::PeriodContext;
use microinsurance_engine::optimiser::rating::{
    self, ClassHistory, CoverRating, RatingBasis, RatingInputs,
};
use microinsurance_engine::optimiser::underwriting::ClassStatus;
use microinsurance_engine::plans;
use microinsurance_engine::triggers::{self, Occurrence, TriggerZone};
use microinsurance_engine::types::{ClassId, CoverId, Plan, Policy, PolicyPeriod, Profile};
use microinsurance_store::{By, Keys, Record, Store, StoreError, collections as c};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

/// The customer's rating inputs: their answers, and how long they have been covered in each
/// class they hold (R32).
pub async fn rating_inputs<E: From<StoreError>>(
    store: &Store,
    cfg: &EngineConfig,
    customer_id: &str,
    held: Option<&Policy>,
) -> Result<RatingInputs, E> {
    let answers = store
        .get::<RatingAnswers>(c::RATING_ANSWERS, customer_id)
        .await?
        .map(|r| r.body.answers)
        .unwrap_or_default();
    let mut classes: BTreeMap<ClassId, ClassHistory> = BTreeMap::new();
    if let Some(p) = held {
        for a in &p.schedule.allocations {
            if let Ok(cover) = cfg.cover(&a.cover_id) {
                classes.insert(
                    cover.class_of_business.clone(),
                    ClassHistory {
                        continuous_periods: p.period.index,
                        ..ClassHistory::default()
                    },
                );
            }
        }
    }
    Ok(RatingInputs {
        factor_values: answers,
        classes,
    })
}

/// Benefit in force per trigger zone as measured at `as_of`: the zones over the accumulation
/// limit (R51). The accumulation job writes it every `accumulation.refresh_minutes`, so a quote
/// reads one record instead of the whole book.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct AccumulationSnapshot {
    pub as_of: DateTime<Utc>,
    pub full_zones: BTreeSet<TriggerZone>,
}

/// The id of the latest snapshot.
pub const CURRENT: &str = "current";

/// Measures every policy in the store now (R51).
pub async fn measure_zones<E: From<StoreError>>(
    store: &Store,
    cfg: &EngineConfig,
) -> Result<BTreeSet<TriggerZone>, E> {
    let policies: Vec<Policy> = store
        .list::<PolicyRecord>(c::POLICIES, By::All)
        .await?
        .into_iter()
        .map(|r| r.body.policy)
        .collect();
    let by_zone = triggers::benefit_in_force_by_zone(&policies, cfg);
    Ok(triggers::full_zones(&by_zone, cfg))
}

/// Trigger zones over the accumulation limit (R51): the latest snapshot when it is no older than
/// twice `refresh_minutes`, otherwise measured now (a new book, or the job not running).
pub async fn full_zones<E: From<StoreError>>(
    store: &Store,
    cfg: &EngineConfig,
    now: DateTime<Utc>,
) -> Result<BTreeSet<TriggerZone>, E> {
    let fresh = Duration::minutes(i64::from(cfg.accumulation.refresh_minutes) * 2);
    if let Some(s) = store
        .get::<AccumulationSnapshot>(c::ACCUMULATION, CURRENT)
        .await?
        .filter(|s| now - s.body.as_of <= fresh)
    {
        return Ok(s.body.full_zones);
    }
    measure_zones(store, cfg).await
}

/// Measures the book and keeps the snapshot as the latest.
pub async fn snapshot<E: From<StoreError>>(
    store: &Store,
    cfg: &EngineConfig,
    now: DateTime<Utc>,
) -> Result<usize, E> {
    let full_zones = measure_zones::<E>(store, cfg).await?;
    let n = full_zones.len();
    let body = AccumulationSnapshot {
        as_of: now,
        full_zones,
    };
    match store
        .get::<AccumulationSnapshot>(c::ACCUMULATION, CURRENT)
        .await?
    {
        Some(r) if r.body.as_of >= now => {}
        Some(r) => {
            // Another worker wrote one meanwhile: theirs stands.
            match store.update(c::ACCUMULATION, Record { body, ..r }).await {
                Ok(_) | Err(StoreError::Conflict { .. }) => {}
                Err(e) => return Err(e.into()),
            }
        }
        None => match store
            .insert(c::ACCUMULATION, CURRENT, Keys::default(), body)
            .await
        {
            Ok(_) | Err(StoreError::Conflict { .. }) => {}
            Err(e) => return Err(e.into()),
        },
    }
    Ok(n)
}

/// Everything a `PeriodContext` borrows, for one customer and policy period.
pub struct Rated {
    pub period: PolicyPeriod,
    pub ratings: BTreeMap<CoverId, CoverRating>,
    pub statuses: BTreeMap<ClassId, ClassStatus>,
    pub in_force: BTreeSet<Occurrence>,
    pub full_zones: BTreeSet<TriggerZone>,
}

/// What rating needs besides the customer: the store, the config and the rating basis.
pub struct Rater<'a> {
    pub store: &'a Store,
    pub cfg: &'a EngineConfig,
    pub basis: &'a RatingBasis,
    /// Now, for the accumulation snapshot's age.
    pub now: DateTime<Utc>,
}

impl Rater<'_> {
    /// Rates `profile`'s covers on `plan` for `period`, with the event types and stages in
    /// force now (R06).
    pub async fn rate<E: From<StoreError> + From<EngineError>>(
        &self,
        plan: &Plan,
        profile: &Profile,
        held: Option<&Policy>,
        period: PolicyPeriod,
        in_force: BTreeSet<Occurrence>,
    ) -> Result<Rated, E> {
        let inputs = rating_inputs::<E>(self.store, self.cfg, &profile.customer_id, held).await?;
        let ratings = rating::rate_covers(
            &plans::covers_for(plan, self.cfg),
            period.starts,
            &inputs,
            &profile.consents,
            self.basis,
            self.cfg,
        )?;
        Ok(Rated {
            period,
            ratings,
            statuses: BTreeMap::new(),
            in_force,
            full_zones: full_zones::<E>(self.store, self.cfg, self.now).await?,
        })
    }
}

impl Rated {
    pub fn ctx<'a>(&'a self, cfg: &'a EngineConfig) -> PeriodContext<'a> {
        PeriodContext {
            cfg,
            period: self.period,
            ratings: &self.ratings,
            statuses: &self.statuses,
            in_force: &self.in_force,
            full_zones: &self.full_zones,
        }
    }
}
