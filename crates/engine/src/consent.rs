//! Consent model: offers, taps, policy periods and data opt-ins.
//! R03 no change without a tap · R04 next policy period, never mid-event · R05 minimum term ·
//! R20 per-data-type opt-in, purpose-bound · R35 renewal notice · R52 licensed partner ·
//! R53 grace, lapse, reinstatement · R54 data on cancellation.
use crate::config::{EngineConfig, FactorData, KindTree, LicenceType};
use crate::money::Money;
use crate::optimiser::{AllocationOutcome, Alternative};
use crate::types::{
    CoverId, CustomerId, DataTypeId, ExposureKind, PartnerId, PlanId, Policy, PolicyPeriod,
    Profile, Schedule, add_policy_periods,
};
use crate::{EngineError, Result, catalogue};
use chrono::{DateTime, Days, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub enum OfferKind {
    /// New business: a first schedule, bound through `partner_id` or, with none, the insurer's
    /// own journey (R52, R58).
    NewPolicy {
        schedule: Schedule,
        partner_id: Option<PartnerId>,
    },
    Reallocation {
        schedule: Schedule,
    },
    /// Only ever the customer's own choice of plan (R45).
    PlanChange {
        plan: PlanId,
    },
    AddCover {
        cover_id: CoverId,
    },
    RemoveCover {
        cover_id: CoverId,
    },
}

/// Terms put to the customer.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct Offer {
    pub offer_id: String,
    pub customer_id: CustomerId,
    pub change: OfferKind,
    /// One plain sentence from the AI layer (R23).
    pub explanation: String,
    pub created_at: DateTime<Utc>,
}

/// The customer's tap. The only way an offer becomes a change (R03).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct CustomerTap {
    pub offer_id: String,
    pub customer_id: CustomerId,
    pub at: DateTime<Utc>,
}

/// An accepted change, effective from the next policy period (R04).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct Endorsement {
    pub policy_id: String,
    pub effective_period: u32,
    pub change: OfferKind,
}

fn rule(rule: &'static str, detail: String) -> EngineError {
    EngineError::Rule { rule, detail }
}

/// R03: the tap must answer this offer, from this customer, after it was made. R04: the change
/// takes effect from the next policy period. R05, R24 and R34 are checked for the change itself.
pub fn accept(
    offer: &Offer,
    tap: &CustomerTap,
    policy: &Policy,
    cfg: &EngineConfig,
) -> Result<Endorsement> {
    if tap.offer_id != offer.offer_id
        || tap.customer_id != offer.customer_id
        || offer.customer_id != policy.customer_id
        || tap.at < offer.created_at
    {
        return Err(rule(
            "R03",
            format!(
                "offer {} has no matching tap from its customer",
                offer.offer_id
            ),
        ));
    }
    match &offer.change {
        OfferKind::NewPolicy { .. } => {
            return Err(rule(
                "R58",
                format!(
                    "offer {} is new business: it binds a policy (`bind`)",
                    offer.offer_id
                ),
            ));
        }
        OfferKind::AddCover { cover_id } => catalogue::check_bundled(cover_id, cfg)?,
        OfferKind::RemoveCover { cover_id } if !can_remove(cover_id, policy, cfg) => {
            return Err(rule(
                "R05",
                format!(
                    "{cover_id} can be removed once {} policy periods are served",
                    cfg.policy_period.min_policy_periods
                ),
            ));
        }
        OfferKind::Reallocation { schedule } => check_schedule(schedule, cfg)?,
        OfferKind::PlanChange { .. } | OfferKind::RemoveCover { .. } => {}
    }
    Ok(Endorsement {
        policy_id: policy.policy_id.clone(),
        effective_period: policy.period.index + 1,
        change: offer.change.clone(),
    })
}

/// R24, R34: only bundled covers, and the allocation totals the plan's price.
fn check_schedule(schedule: &Schedule, cfg: &EngineConfig) -> Result<()> {
    for a in &schedule.allocations {
        catalogue::check_bundled(&a.cover_id, cfg)?;
    }
    let price = cfg.plan(&schedule.plan_id).map(|p| p.price);
    if price != Some(schedule.total_premium()) {
        return Err(rule(
            "R34",
            format!(
                "the allocation must total the price of plan {}",
                schedule.plan_id
            ),
        ));
    }
    Ok(())
}

/// R58: new business. The customer's tap on a new-policy offer binds the policy, on the market
/// local day the offer was made (the rates quoted are that day's), through a partner licensed on
/// that day or the insurer's own journey (R52), for a schedule of bundled covers totalling the
/// plan price (R24, R34). Cover starts at 00:00 local the next day, never during the day of the
/// tap (R04), and every waiting period runs from then (R07). The policy's facts are the
/// customer's declarations in `profile`.
pub fn bind(
    offer: &Offer,
    tap: &CustomerTap,
    policy_id: &str,
    profile: &Profile,
    cfg: &EngineConfig,
) -> Result<Policy> {
    let OfferKind::NewPolicy {
        schedule,
        partner_id,
    } = &offer.change
    else {
        return Err(rule(
            "R58",
            format!("offer {} is not new business", offer.offer_id),
        ));
    };
    if tap.offer_id != offer.offer_id
        || tap.customer_id != offer.customer_id
        || profile.customer_id != offer.customer_id
        || tap.at < offer.created_at
    {
        return Err(rule(
            "R03",
            format!(
                "offer {} has no matching tap from its customer",
                offer.offer_id
            ),
        ));
    }
    let offset = cfg.local_offset();
    let tapped_on = tap.at.with_timezone(&offset).date_naive();
    if tapped_on != offer.created_at.with_timezone(&offset).date_naive() {
        return Err(rule(
            "R58",
            format!(
                "offer {} was valid on the day it was made; ask for a new quote",
                offer.offer_id
            ),
        ));
    }
    check_binding_partner(partner_id.as_deref(), tapped_on, cfg)?;
    check_schedule(schedule, cfg)?;
    let plan = cfg.plan(&schedule.plan_id).ok_or_else(|| {
        rule(
            "R01",
            format!("plan {} is not configured", schedule.plan_id),
        )
    })?;
    let starts = tapped_on.succ_opt().unwrap_or(NaiveDate::MAX);
    Ok(Policy {
        policy_id: policy_id.to_string(),
        customer_id: offer.customer_id.clone(),
        plan: plan.clone(),
        schedule: schedule.clone(),
        period: PolicyPeriod {
            index: 1,
            starts,
            ends: add_policy_periods(starts, 1),
        },
        cover_starts: schedule
            .allocations
            .iter()
            .map(|a| (a.cover_id.clone(), starts))
            .collect(),
        work_windows: profile.work_windows.clone(),
        commute_routes: profile.commute_routes.clone(),
        work_areas: profile.work_areas.clone(),
        upcoming_events: profile.upcoming_events.clone(),
        subscribed_at: tap.at,
    })
}

/// True when cover held from `start` has served the minimum policy periods by `effective` (R05).
pub(crate) fn served_min_periods(
    start: NaiveDate,
    effective: NaiveDate,
    cfg: &EngineConfig,
) -> bool {
    add_policy_periods(start, cfg.policy_period.min_policy_periods) <= effective
}

/// R05: a cover may be removed from the next policy period once it has served the minimum term.
pub fn can_remove(cover_id: &str, policy: &Policy, cfg: &EngineConfig) -> bool {
    policy
        .cover_starts
        .get(cover_id)
        .is_some_and(|start| served_min_periods(*start, policy.period.ends, cfg))
}

/// R20: optional data is used only with its opt-in, and only for a cover it supports: a
/// registered rating factor of that cover's class reads it, or the cover's claims ask for
/// evidence of that type. Prevention and rating work without location.
pub fn may_use(
    data: &str,
    cover_id: &str,
    consents: &BTreeSet<DataTypeId>,
    cfg: &EngineConfig,
) -> bool {
    use crate::config::FactorData;
    let Ok(cover) = cfg.cover(cover_id) else {
        return false;
    };
    let as_evidence = cover.claim_evidence.iter().any(|e| {
        cfg.data_types
            .evidence_kinds
            .get(e)
            .is_some_and(|d| d.as_deref() == Some(data))
    });
    let as_factor = cfg.rating_factors.values().any(|f| {
        matches!(&f.data, Some(FactorData::Consented(d)) if d == data)
            && f.rates(&cover.class_of_business)
    });
    consents.contains(data) && (as_evidence || as_factor)
}

/// One cover's sum insured before and after re-rating.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct SumInsuredChange {
    pub cover_id: CoverId,
    pub premium: Money,
    pub before: Money,
    pub after: Money,
    /// The declared value of the insured item, where there is one.
    pub declared_value: Option<Money>,
    /// R48: losses are paid on a first-loss basis up to the sum insured; the customer is told
    /// when it falls below the declared value.
    pub below_declared_value: bool,
}

/// R35: the new sums insured, shown `renewal_notice_days` before the period, so the customer can
/// reallocate, change plan or cancel. `ai::explain` adds one sentence per line (R23).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct RenewalNotice {
    pub period_starts: NaiveDate,
    /// The notice must reach the customer by this day.
    pub issue_by: NaiveDate,
    pub changes: Vec<SumInsuredChange>,
    pub unallocated: Money,
    /// Premium the default places beyond what the customer can use now.
    pub placed_beyond_need: Money,
    /// R42: no plan is named; the customer chooses.
    pub may_change_plan_or_cancel_without_penalty: bool,
    /// Set when the customer is asked to update the allocation in the app or web.
    pub update_request: Option<UpdateRequest>,
}

/// The customer is asked to update the allocation; without an update by `respond_by`, the
/// optimiser's default in this notice applies (R03, R34, R42).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct UpdateRequest {
    /// The last day an update can still apply to this period (R04).
    pub respond_by: NaiveDate,
    pub alternatives: Vec<Alternative>,
}

pub fn renewal_notice(
    current: &Schedule,
    renewed: &AllocationOutcome,
    profile: &Profile,
    period: &PolicyPeriod,
    cfg: &EngineConfig,
) -> RenewalNotice {
    let ids: BTreeSet<&CoverId> = current
        .allocations
        .iter()
        .chain(&renewed.schedule.allocations)
        .map(|a| &a.cover_id)
        .collect();
    let changes = ids
        .into_iter()
        .map(|id| {
            let after = renewed.schedule.allocation(id);
            let declared_value = cfg
                .cover(id)
                .ok()
                .filter(|c| c.exposure.kind == ExposureKind::DeclaredItem)
                .and_then(|c| {
                    profile
                        .declared_items
                        .iter()
                        .filter(|i| KindTree::under(&c.exposure.of, &i.kind))
                        .map(|i| i.declared_value)
                        .sum::<Option<Money>>()
                });
            let after_si = after.map_or(Money::ZERO, |a| a.sum_insured);
            SumInsuredChange {
                cover_id: id.clone(),
                premium: after.map_or(Money::ZERO, |a| a.premium),
                before: current
                    .allocation(id)
                    .map_or(Money::ZERO, |a| a.sum_insured),
                after: after_si,
                declared_value,
                below_declared_value: declared_value
                    .is_some_and(|v| after.is_some() && after_si < v),
            }
        })
        .collect();
    RenewalNotice {
        period_starts: period.starts,
        issue_by: notice_due(period.starts, cfg),
        changes,
        unallocated: renewed.unallocated,
        placed_beyond_need: renewed.placed_beyond_need,
        may_change_plan_or_cancel_without_penalty: renewed
            .may_change_plan_or_cancel_without_penalty(),
        update_request: renewed.needs_customer_update().then(|| UpdateRequest {
            respond_by: period.starts.pred_opt().unwrap_or(period.starts),
            alternatives: renewed.alternatives.clone(),
        }),
    }
}

/// The schedule that applies to a policy period: the customer's accepted reallocation for it,
/// or, without one, the optimiser's default from the renewal (R03, R34).
pub fn schedule_for_period(
    default: &Schedule,
    update: Option<&Endorsement>,
    period_index: u32,
) -> Schedule {
    match update {
        Some(Endorsement {
            effective_period,
            change: OfferKind::Reallocation { schedule },
            ..
        }) if *effective_period == period_index => schedule.clone(),
        _ => default.clone(),
    }
}

fn notice_due(period_starts: NaiveDate, cfg: &EngineConfig) -> NaiveDate {
    period_starts
        .checked_sub_days(Days::new(u64::from(cfg.policy_period.renewal_notice_days)))
        .unwrap_or(NaiveDate::MIN)
}

/// R35: re-rated sums insured take effect only if the notice was issued in time.
pub fn check_notice_served(
    issued_on: NaiveDate,
    period_starts: NaiveDate,
    cfg: &EngineConfig,
) -> Result<()> {
    let due = notice_due(period_starts, cfg);
    if issued_on <= due {
        Ok(())
    } else {
        Err(rule(
            "R35",
            format!("renewal notice issued {issued_on}; it was due by {due}"),
        ))
    }
}

/// R52: a policy is bound only through a partner holding a valid licence on that day, or on the
/// insurer's own journey (`None`). A referral-only partner refers; it never binds.
pub fn check_binding_partner(
    partner_id: Option<&str>,
    on: NaiveDate,
    cfg: &EngineConfig,
) -> Result<()> {
    let Some(id) = partner_id else {
        return Ok(());
    };
    let partner = cfg.distribution_partners.get(id).ok_or_else(|| {
        rule(
            "R52",
            format!("{id} is not a recorded distribution partner"),
        )
    })?;
    if partner.licence_type == LicenceType::ReferralOnly {
        return Err(rule(
            "R52",
            format!("{id} may refer the customer to the insurer's own journey, not bind"),
        ));
    }
    match partner.licence_expiry {
        Some(expiry) if on <= expiry => Ok(()),
        _ => Err(rule("R52", format!("{id}'s licence is not valid on {on}"))),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub enum PremiumStatus {
    Paid,
    /// Cover continues; a claim is paid net of the premium due (R53).
    InGrace {
        last_day: NaiveDate,
    },
    Lapsed {
        since: NaiveDate,
        reinstate_by: NaiveDate,
    },
}

fn lapse_day(due: NaiveDate, cfg: &EngineConfig) -> NaiveDate {
    due.checked_add_days(Days::new(
        u64::from(cfg.policy_period.grace_period_days) + 1,
    ))
    .unwrap_or(NaiveDate::MAX)
}

/// R53: a premium due on `due` and unpaid by `today`.
pub fn premium_status(
    due: NaiveDate,
    paid_on: Option<NaiveDate>,
    today: NaiveDate,
    cfg: &EngineConfig,
) -> PremiumStatus {
    let lapses = lapse_day(due, cfg);
    match paid_on {
        Some(p) if p <= today && p < lapses => PremiumStatus::Paid,
        _ if today < lapses => PremiumStatus::InGrace {
            last_day: lapses.pred_opt().unwrap_or(lapses),
        },
        _ => PremiumStatus::Lapsed {
            since: lapses,
            reinstate_by: lapses
                .checked_add_days(Days::new(u64::from(cfg.policy_period.reinstatement_days)))
                .unwrap_or(NaiveDate::MAX),
        },
    }
}

/// What a payment after the grace period does (R53). No late fee either way.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub enum LatePayment {
    /// Within the grace period: nothing lapsed.
    InTime,
    /// Within `reinstatement_days` of the lapse: no new waiting periods.
    Reinstated,
    /// Later: a new policy, waiting periods apply again (R07, R47).
    NewPolicy,
}

pub fn late_payment(due: NaiveDate, paid_on: NaiveDate, cfg: &EngineConfig) -> LatePayment {
    match premium_status(due, None, paid_on, cfg) {
        PremiumStatus::Paid | PremiumStatus::InGrace { .. } => LatePayment::InTime,
        PremiumStatus::Lapsed { reinstate_by, .. } if paid_on <= reinstate_by => {
            LatePayment::Reinstated
        }
        PremiumStatus::Lapsed { .. } => LatePayment::NewPolicy,
    }
}

/// R53: a claim arising in the grace period is paid net of the premium due.
pub fn net_of_premium_due(amount: Money, premium_due: Money) -> Money {
    (amount - premium_due).max(Money::ZERO)
}

/// What cancellation does to data (R20, R54).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct CancellationData {
    /// Optional data the customer opted into; deleted now.
    pub delete: BTreeSet<DataTypeId>,
    /// Policy, premium, claims, ledger and offer records are kept, access-restricted, until then.
    pub retain_records_until: NaiveDate,
}

pub fn on_cancellation(
    profile: &Profile,
    cancelled_on: NaiveDate,
    cfg: &EngineConfig,
) -> CancellationData {
    CancellationData {
        delete: profile.consents.clone(),
        retain_records_until: add_policy_periods(
            cancelled_on,
            cfg.privacy.record_retention_years.saturating_mul(12),
        ),
    }
}

/// R20, R54: of the factors a cancelled customer answered, those whose answers are erased: each
/// factor whose value came from an optional data type the cancellation deletes. Declared answers
/// and the insurer's own records are kept with the policy's records.
pub fn answers_to_erase<'a>(
    answered: impl IntoIterator<Item = &'a str>,
    cancellation: &CancellationData,
    cfg: &EngineConfig,
) -> BTreeSet<String> {
    answered
        .into_iter()
        .filter(|f| {
            cfg.rating_factors.get(*f).is_some_and(|factor| {
                matches!(&factor.data, Some(FactorData::Consented(t)) if cancellation.delete.contains(t))
            })
        })
        .map(str::to_string)
        .collect()
}
