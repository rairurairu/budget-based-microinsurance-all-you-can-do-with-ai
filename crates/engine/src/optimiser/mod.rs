//! M2 Cover optimiser: a deterministic split of the plan price into a personal schedule (R02).
//!
//! Objective: the most usable sum insured per HK$: cover where the customer has exposure, up to
//! what they can use this policy period (their need). Premium beyond that is placed where limits
//! allow as a default, and the customer is asked to update it (R42). Subject to the plan's covers
//! (R24, R45), product limits (R34), the parametric earnings cap (R09), signals in force (R06),
//! zone accumulation (R51) and postponement (R41). Integer arithmetic only; ties broken by cover
//! id. The optimiser never sets a rate (rating) and never changes or suggests a plan (R45).
mod allocate;
pub mod rating;
pub mod underwriting;

use crate::config::EngineConfig;
use crate::config::KindTree;
use crate::money::{Money, Rate};
use crate::triggers::{Occurrence, TriggerZone};
use crate::types::{
    Allocation, ClassId, CoverId, CoverSpec, Exposure, ExposureKind, Plan, PolicyPeriod, Profile,
    Schedule,
};
use crate::{EngineError, Result, catalogue, plans};
use allocate::Candidate;
use chrono::Days;
use rating::{AppliedFactor, CoverRating};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use underwriting::{AllocationRight, ClassStatus};

/// One customer's policy period as the optimiser sees it. Everything is passed in (R30).
#[derive(Debug, Clone, Copy)]
pub struct PeriodContext<'a> {
    pub cfg: &'a EngineConfig,
    /// The policy period being allocated.
    pub period: PolicyPeriod,
    /// From `rating::rate_covers`, for every cover of the plan.
    pub ratings: &'a BTreeMap<CoverId, CoverRating>,
    /// From `underwriting::review`, by class of business; a missing class is open.
    pub statuses: &'a BTreeMap<ClassId, ClassStatus>,
    /// Event types and stages in force now (R06), from `triggers::classify`.
    pub in_force: &'a BTreeSet<Occurrence>,
    /// Trigger zones over the accumulation limit (R51), from `triggers::full_zones`.
    pub full_zones: &'a BTreeSet<TriggerZone>,
}

/// An allocation of the whole plan price. When part of it buys nothing the customer can use
/// yet, the customer is asked to update it in the app or web, with `alternatives` to choose from;
/// without an update, this schedule (the optimiser's default) applies (R03, R34, R42).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct AllocationOutcome {
    pub schedule: Schedule,
    /// Plan price no split can place: the remaining covers' limits are full, for example after a
    /// postponement (R41). Above zero, the customer may change plan or cancel without penalty
    /// before the period starts; no plan is suggested (R42, R45).
    pub unallocated: Money,
    /// The same shortfall measured on the plan's capacity; equals `unallocated`.
    pub beyond_capacity: Money,
    /// Premium the default places beyond what the customer can use now.
    pub placed_beyond_need: Money,
    /// Covers with room for that premium, and what makes each useful.
    pub alternatives: Vec<Alternative>,
}

impl AllocationOutcome {
    /// R42: only when the plan's covers cannot take the plan price in any split.
    pub fn may_change_plan_or_cancel_without_penalty(&self) -> bool {
        self.beyond_capacity > Money::ZERO
    }

    /// The customer is asked to update the allocation in the app or web before the period.
    pub fn needs_customer_update(&self) -> bool {
        self.placed_beyond_need > Money::ZERO || self.may_change_plan_or_cancel_without_penalty()
    }
}

/// The optimiser's suggested allocation: product information under the insurer's licence, with
/// the factors behind it (R49). The customer can change any part of it (R34).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct Suggestion {
    pub outcome: AllocationOutcome,
    pub basis: Vec<SuggestionBasis>,
}

/// Room for more premium in one cover of the plan.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct Alternative {
    pub cover_id: CoverId,
    /// The most extra premium the cover can take this period.
    pub can_take: Money,
    /// What makes the extra cover usable: declaring it (a trip, a ticket, a device).
    pub exposure: Exposure,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct SuggestionBasis {
    pub cover_id: CoverId,
    /// The sum insured this customer can use this period; zero means no exposure.
    pub need: Money,
    pub rate: Rate,
    pub risk_class: String,
    pub factors: Vec<AppliedFactor>,
}

/// The sum insured this customer can use in the period: zero without exposure, otherwise the
/// declared values involved, within the product limit and the earnings cap (R09).
pub fn need(cover: &CoverSpec, profile: &Profile, period: &PolicyPeriod) -> Money {
    let limit = cover.product_limit.unwrap_or(Money(i64::MAX));
    let covered_from = period
        .starts
        .checked_add_days(Days::new(u64::from(cover.waiting_period_days)))
        .unwrap_or(period.ends);
    let sum = |values: &mut dyn Iterator<Item = Option<Money>>| -> Money {
        values.fold(Money::ZERO, |total, v| {
            Money(total.0.saturating_add(v.unwrap_or(limit).0)).min(limit)
        })
    };
    let of = &cover.exposure.of;
    let raw = match cover.exposure.kind {
        ExposureKind::DeclaredHours if profile.work_windows.is_empty() => Money::ZERO,
        ExposureKind::DeclaredRoute if profile.commute_routes.is_empty() => Money::ZERO,
        // A cover matched on a work area protects no one who has not declared one (R42).
        _ if area_matched(cover) && profile.work_areas.is_empty() => Money::ZERO,
        ExposureKind::DeclaredHours | ExposureKind::DeclaredRoute | ExposureKind::Everyone => limit,
        ExposureKind::UpcomingEvent => sum(&mut profile
            .upcoming_events
            .iter()
            .filter(|e| KindTree::under(of, &e.kind))
            .filter(|e| covered_from <= e.starts && e.starts < period.ends)
            .map(|e| e.value)),
        ExposureKind::DeclaredItem => sum(&mut profile
            .declared_items
            .iter()
            .filter(|i| KindTree::under(of, &i.kind))
            .map(|i| i.declared_value)),
    };
    let cap = catalogue::max_parametric_payout(cover, profile).unwrap_or(limit);
    raw.min(limit).min(cap)
}

/// The most premium a cover may take this period, and the rule that sets it.
fn ceiling(
    cover: &CoverSpec,
    rating: &CoverRating,
    current: Money,
    profile: &Profile,
    ctx: &PeriodContext,
) -> (Money, &'static str) {
    let step = ctx.cfg.policy_period.allocation_step;
    let status = ctx
        .statuses
        .get(&cover.class_of_business)
        .copied()
        .unwrap_or_default();
    let mut best = match catalogue::max_parametric_payout(cover, profile) {
        Some(cap) => (rating.rate.max_premium(cap, step), "R09"),
        None => (Money(i64::MAX), "R34"),
    };
    let mut lower = |value: Money, rule: &'static str| {
        if value < best.0 {
            best = (value, rule);
        }
    };
    if let Some(limit) = cover.product_limit {
        lower(rating.rate.max_premium(limit, step), "R34");
    }
    match status.allocation_right() {
        AllocationRight::Any => {}
        AllocationRight::NoIncrease => lower(current, "R41"),
        AllocationRight::None => lower(Money::ZERO, "R41"),
    }
    if catalogue::check_new_cover(cover, ctx.in_force).is_err() {
        lower(current, "R06");
    }
    if catalogue::check_accumulation(cover, profile, ctx.full_zones, ctx.cfg).is_err() {
        lower(current, "R51");
    }
    best
}

fn candidates(
    plan: &Plan,
    profile: &Profile,
    current: Option<&Schedule>,
    keep_current: bool,
    ctx: &PeriodContext,
) -> Result<Vec<Candidate>> {
    plans::covers_for(plan, ctx.cfg)
        .iter()
        .map(|id| {
            let cover = ctx.cfg.cover(id)?;
            let rating = ctx.ratings.get(id).ok_or_else(|| EngineError::Rule {
                rule: "R31",
                detail: format!("cover {id} has not been rated for this policy period"),
            })?;
            let existing = current.map_or(Money::ZERO, |s| s.premium(id));
            let (cap, _) = ceiling(cover, rating, existing, profile, ctx);
            Ok(Candidate {
                cover_id: id.clone(),
                rate: rating.rate,
                need: need(cover, profile, &ctx.period),
                floor: if keep_current {
                    existing.min(cap)
                } else {
                    Money::ZERO
                },
                ceiling: cap,
            })
        })
        .collect()
}

/// Plan price the covers' ceilings cannot take in any split (R42).
fn beyond_capacity(cands: &[Candidate], price: Money) -> Money {
    let capacity = cands
        .iter()
        .fold(0_i64, |sum, c| sum.saturating_add(c.ceiling.0));
    (price - Money(capacity)).max(Money::ZERO)
}

fn to_schedule(plan: &Plan, premiums: &BTreeMap<CoverId, Money>, ctx: &PeriodContext) -> Schedule {
    Schedule {
        plan_id: plan.id.clone(),
        allocations: premiums
            .iter()
            .filter(|(_, p)| **p > Money::ZERO)
            .map(|(id, p)| Allocation {
                cover_id: id.clone(),
                premium: *p,
                sum_insured: ctx
                    .ratings
                    .get(id)
                    .map_or(Money::ZERO, |r| r.rate.sum_insured(*p)),
            })
            .collect(),
    }
}

/// Places `budget` on top of the candidates' floors: usable cover first; then, for what is
/// left, the alternatives the customer could use it for, and the default placement wherever the
/// limits allow, so premium always buys cover.
fn place(
    cands: &[Candidate],
    budget: Money,
    plan: &Plan,
    ctx: &PeriodContext,
) -> AllocationOutcome {
    let step = ctx.cfg.policy_period.allocation_step;
    let usable = allocate::fill(cands, budget, step, false);
    let placed = |id: &str| usable.premiums.get(id).copied().unwrap_or(Money::ZERO);
    let alternatives = if usable.left >= step {
        cands
            .iter()
            .filter_map(|c| {
                let can_take = c.ceiling.min(plan.price) - placed(&c.cover_id);
                let exposure = ctx.cfg.cover(&c.cover_id).ok()?.exposure.clone();
                (can_take >= step).then(|| Alternative {
                    cover_id: c.cover_id.clone(),
                    can_take,
                    exposure,
                })
            })
            .collect()
    } else {
        Vec::new()
    };
    let rest: Vec<Candidate> = cands
        .iter()
        .map(|c| Candidate {
            floor: placed(&c.cover_id),
            ..c.clone()
        })
        .collect();
    let filled = allocate::fill(&rest, usable.left, step, true);
    AllocationOutcome {
        schedule: to_schedule(plan, &filled.premiums, ctx),
        unallocated: filled.left,
        beyond_capacity: beyond_capacity(cands, plan.price),
        placed_beyond_need: filled.beyond_need,
        alternatives,
    }
}

/// R02: the optimiser's allocation of the whole plan price from scratch. A suggestion only; it
/// takes effect only with the customer's tap (R03).
pub fn suggest(
    plan: &Plan,
    profile: &Profile,
    current: Option<&Schedule>,
    ctx: &PeriodContext,
) -> Result<Suggestion> {
    let cands = candidates(plan, profile, current, false, ctx)?;
    let basis = cands
        .iter()
        .filter_map(|c| {
            let r = ctx.ratings.get(&c.cover_id)?;
            Some(SuggestionBasis {
                cover_id: c.cover_id.clone(),
                need: c.need,
                rate: r.rate,
                risk_class: r.risk_class.id.clone(),
                factors: r.factors.clone(),
            })
        })
        .collect();
    Ok(Suggestion {
        outcome: place(&cands, plan.price, plan, ctx),
        basis,
    })
}

/// R34: automatic renewal. The unchanged premium allocation renews and its sums insured are
/// re-rated at this period's rates. Premium a cover can no longer take (postponed class R41,
/// product limit R34, cover withdrawn) is re-placed by the optimiser: usable cover first, then
/// wherever the limits allow; the renewal notice asks the customer to update it (R35). Only what
/// no cover can take is left over (R42). A cheaper plan starts from scratch.
pub fn renew(
    current: &Schedule,
    plan: &Plan,
    profile: &Profile,
    ctx: &PeriodContext,
) -> Result<AllocationOutcome> {
    let mut cands = candidates(plan, profile, Some(current), true, ctx)?;
    let mut kept: Money = cands.iter().map(|c| c.floor).sum();
    if kept > plan.price {
        cands.iter_mut().for_each(|c| c.floor = Money::ZERO);
        kept = Money::ZERO;
    }
    Ok(place(&cands, plan.price - kept, plan, ctx))
}

/// Prices the customer's own allocation (R34: any split, including all of it on one cover up to
/// its product limit) and refuses one the rules do not allow.
pub fn price_allocation(
    plan: &Plan,
    premiums: &BTreeMap<CoverId, Money>,
    current: Option<&Schedule>,
    profile: &Profile,
    ctx: &PeriodContext,
) -> Result<Schedule> {
    let step = ctx.cfg.policy_period.allocation_step;
    let total: Money = premiums.values().copied().sum();
    if total != plan.price {
        return Err(EngineError::Rule {
            rule: "R34",
            detail: format!(
                "allocations total {} cents; the plan price is {} cents",
                total.0, plan.price.0
            ),
        });
    }
    let offered = plans::covers_for(plan, ctx.cfg);
    for (id, premium) in premiums {
        if *premium < Money::ZERO || !premium.is_multiple_of(step) {
            return Err(EngineError::Rule {
                rule: "R02",
                detail: format!("{id}: premium must be a multiple of the allocation step"),
            });
        }
        if *premium == Money::ZERO {
            continue;
        }
        catalogue::check_bundled(id, ctx.cfg)?;
        if !offered.contains(id) {
            return Err(EngineError::Rule {
                rule: "R45",
                detail: format!("cover {id} is not offered on plan {}", plan.id),
            });
        }
        let cover = ctx.cfg.cover(id)?;
        let rating = ctx.ratings.get(id).ok_or_else(|| EngineError::Rule {
            rule: "R31",
            detail: format!("cover {id} has not been rated for this policy period"),
        })?;
        let existing = current.map_or(Money::ZERO, |s| s.premium(id));
        let (cap, rule) = ceiling(cover, rating, existing, profile, ctx);
        if *premium > cap {
            return Err(EngineError::Rule {
                rule,
                detail: format!("{id} can take at most {} cents this period", cap.0),
            });
        }
    }
    Ok(to_schedule(plan, premiums, ctx))
}

/// True for a cover whose trigger pays only in the customer's declared work areas (ADR-012).
fn area_matched(cover: &CoverSpec) -> bool {
    cover
        .trigger
        .as_ref()
        .is_some_and(|t| t.match_on == Some(crate::types::MatchKey::DeclaredArea))
}
