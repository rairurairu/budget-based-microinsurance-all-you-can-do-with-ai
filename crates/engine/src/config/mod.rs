//! Everything the engine reads from `config/`. The caller reads the files and passes the text in;
//! the engine parses and validates it once and never touches the disk (R30).
pub(crate) mod decimal;
mod factors;
mod guardrails;
mod lines;
mod measures;
mod registry;
mod suppliers;

pub use factors::{FactorBasis, FactorData, FactorLevel, FactorStatus, RatingFactor};
pub use guardrails::{
    AnomalyConfig, ClaimsConfig, FairnessConfig, GuardrailConfig, HumanCareConfig,
};
pub use lines::{ClassNode, ExperienceLink, LinesOfBusiness};
pub use measures::{PreventionMeasure, PreventionMeasures};
pub use registry::{
    DataSource, DataTypes, EventType, ExposureTypes, KindTree, PricingBasis, SourceCode,
    SourceStatus, Triggers, ZoneUnit,
};
pub use suppliers::{InKindTerms, Supplier, SupplierId, Suppliers, Tariff};

use crate::money::{Bps, Money};
use crate::pilot::PilotConfig;
use crate::rewards::Funder;
use crate::types::{
    CoverId, CoverSpec, CoverType, EventTypeId, Exposure, ExposureKind, FactorId, LaunchPhase,
    MatchKey, Measure, PartnerId, PayoutBasis, PayoutSpec, Plan, PlanId, Privileges, Profile,
    TriggerSpec,
};
use crate::{EngineError, Result};
use chrono::{FixedOffset, NaiveDate, Offset, Utc};
use decimal::{DecBps, DecMoney};
use registry::{RawCondition, r55};
use serde::de::{self, Deserializer, Visitor};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

/// Validated configuration. Built only by `from_yaml`.
#[derive(Debug, Clone)]
pub struct EngineConfig {
    pub market: String,
    pub currency: String,
    /// Minutes east of UTC of the market's local time; declared hours and daily payouts count in
    /// it (R08). A fixed offset: a market with daylight saving needs a time zone table first.
    pub utc_offset_minutes: i32,
    /// Covers with this launch phase or lower are offered (R24 also applies).
    pub launch_phase: u8,
    pub policy_period: PolicyPeriodConfig,
    pub plans: Vec<Plan>,
    pub promotions: Vec<Promotion>,
    pub catalogue: BTreeMap<CoverId, CoverSpec>,
    /// R24: only these covers may be bundled.
    pub approved_products: BTreeSet<CoverId>,
    pub lines: LinesOfBusiness,
    pub rating_factors: BTreeMap<FactorId, RatingFactor>,
    pub pricing: PricingConfig,
    pub underwriting: UnderwritingConfig,
    pub score: ScoreConfig,
    pub rewards: RewardConfig,
    pub guardrails: GuardrailConfig,
    /// Straight-through processing authority for indemnity claims (R13, R57).
    pub claims: ClaimsConfig,
    pub privacy: PrivacyConfig,
    pub distribution_partners: BTreeMap<PartnerId, DistributionPartner>,
    /// Event limits and accumulation control, from the reinsurance programme (R51).
    pub accumulation: AccumulationConfig,
    /// Languages the insurer offers customers; each customer's setting picks one.
    pub languages: Languages,
    /// Trigger event types and data sources (R55).
    pub triggers: Triggers,
    /// What a customer can declare (R55).
    pub exposure_types: ExposureTypes,
    /// Optional data types and evidence kinds (R20, R55).
    pub data_types: DataTypes,
    /// Recognised loss prevention measures (R18, R38, ADR-009).
    pub prevention: PreventionMeasures,
    /// Suppliers of settlement in kind and partner rewards (R28, R56).
    pub suppliers: Suppliers,
    /// The pilot, when one is configured (`pilot.yaml`, R59, ADR-012).
    pub pilot: Option<PilotConfig>,
}

impl EngineConfig {
    /// R55: a profile names only registered upcoming event kinds, item kinds and data types.
    pub fn check_profile(&self, profile: &Profile) -> Result<()> {
        let kinds = &self.exposure_types;
        if let Some(e) = profile
            .upcoming_events
            .iter()
            .find(|e| !kinds.upcoming_event_kinds.contains(&e.kind))
        {
            return Err(r55(format!("unregistered upcoming event kind {}", e.kind)));
        }
        if let Some(i) = profile
            .declared_items
            .iter()
            .find(|i| !kinds.declared_item_kinds.contains(&i.kind))
        {
            return Err(r55(format!("unregistered declared item kind {}", i.kind)));
        }
        if let Some(d) = profile
            .consents
            .iter()
            .find(|d| !self.data_types.data_types.contains(*d))
        {
            return Err(r55(format!("unregistered data type {d}")));
        }
        if let Some(l) = profile
            .language
            .as_ref()
            .filter(|l| !self.languages.offered.contains(*l))
        {
            return Err(EngineError::Config(format!("language {l} is not offered")));
        }
        Ok(())
    }

    /// Covers that can be offered: approved, and in the launch phase or a plan's privileges.
    pub fn offered_covers(&self) -> BTreeSet<&CoverId> {
        self.catalogue
            .values()
            .filter(|c| self.approved_products.contains(&c.id))
            .filter(|c| {
                matches!(c.launch_phase, LaunchPhase::Phase(p) if p <= self.launch_phase)
                    || self
                        .plans
                        .iter()
                        .any(|p| p.privileges.extra_covers.contains(&c.id))
            })
            .map(|c| &c.id)
            .collect()
    }

    /// The market's local time (R08).
    pub fn local_offset(&self) -> FixedOffset {
        FixedOffset::east_opt(self.utc_offset_minutes.saturating_mul(60))
            .unwrap_or_else(|| Utc.fix())
    }

    pub fn plan(&self, id: &str) -> Option<&Plan> {
        self.plans.iter().find(|p| p.id == id)
    }

    pub fn cover(&self, id: &str) -> Result<&CoverSpec> {
        self.catalogue
            .get(id)
            .ok_or_else(|| EngineError::Config(format!("unknown cover {id}")))
    }
}

#[derive(Debug, Clone, Copy)]
pub struct PolicyPeriodConfig {
    pub allocation_step: Money,
    /// R05: minimum policy periods per cover.
    pub min_policy_periods: u32,
    /// R35
    pub renewal_notice_days: u32,
    /// R53
    pub grace_period_days: u32,
    /// R53
    pub reinstatement_days: u32,
}

/// A time-limited campaign, the only way a plan price goes down (R44).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Promotion {
    pub id: String,
    pub starts: NaiveDate,
    /// Last day of the campaign, inclusive.
    pub ends: NaiveDate,
    pub plans: BTreeSet<PlanId>,
    pub discount: Bps,
    /// Eligibility by channel and new-customer status only; never by risk (R44).
    pub channel: Option<String>,
    pub new_customers_only: bool,
    pub funded_by: Funder,
}

#[derive(Debug, Clone, Copy)]
pub struct PricingConfig {
    pub target_loss_ratio: Bps,
    /// R32
    pub rating_horizon_periods: u32,
    /// R32
    pub continuous_cover_periods: u32,
    /// R33: exposure (policy periods) at which an undeclared group's own average gets full weight.
    pub undeclared_credibility_full_exposure: u32,
}

/// One of the insurer's standard risk classes (R39). The last, without a threshold, is postponed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct RiskClass {
    pub id: String,
    pub max_relativity: Option<Bps>,
}

impl RiskClass {
    pub fn is_postponed(&self) -> bool {
        self.max_relativity.is_none()
    }
}

#[derive(Debug, Clone)]
pub struct UnderwritingConfig {
    /// Ascending thresholds; the last is postponed.
    pub risk_classes: Vec<RiskClass>,
    pub claims_experience_periods: u32,
    pub claims_experience_threshold: Bps,
    pub credibility_full_periods: u32,
    pub exclusion_notice_periods: u32,
    pub postponement_periods: u32,
    pub reentry_waiting_period_days: u32,
}

#[derive(Debug, Clone, Copy)]
pub struct ScoreConfig {
    /// R11: per points pool.
    pub max_effect: Bps,
    /// R12
    pub point_half_life_days: u32,
    pub points_per_verified_activity: u32,
    /// Points at which a pool reaches `max_effect`.
    pub points_for_max_effect: u32,
}

#[derive(Debug, Clone)]
pub struct RewardConfig {
    /// Reward families offers and plan privileges may name. A family named in a plan's
    /// privileges is open only on those plans (R45); the others are open on every plan.
    pub families: BTreeSet<String>,
    /// R17: rewards granted to one customer in one calendar month of market local time.
    pub monthly_cap: Money,
    /// R40; below 1 (R18).
    pub reward_share_of_claims_avoided: Bps,
    /// R26: participation rewards; none when not configured.
    pub loyalty: Option<LoyaltyConfig>,
}

/// R26: a reward for sustained participation, never for not claiming.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LoyaltyConfig {
    /// Consecutive policy periods with a verified prevention activity in one points pool.
    pub streak_periods: u32,
    pub reward: Money,
}

/// A peril, as the reinsurance programme defines a loss occurrence: event types that one
/// physical event produces together (a typhoon's signals and its rain). The peril's trigger
/// events in one zone, each starting within `aggregation_hours` of the last one ending, are one
/// loss occurrence: one event limit, and one daily benefit per day (R08, R51).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Peril {
    pub id: String,
    pub event_types: BTreeSet<EventTypeId>,
    /// All payouts on one loss occurrence together.
    pub event_limit: Money,
    pub aggregation_hours: u32,
    /// One loss occurrence across every zone of the territory, as the reinsurance event
    /// definition counts one typhoon however many counties suspend work (ADR-012); otherwise one
    /// per zone (a rail line, a flight, a ticketed event).
    pub across_zones: bool,
}

/// R51: limits set from the insurer's reinsurance programme and stated in the key features.
#[derive(Debug, Clone)]
pub struct AccumulationConfig {
    pub perils: BTreeMap<String, Peril>,
    /// Benefit in force per trigger zone above which new premium allocation into it stops.
    pub zone_accumulation_limit: Money,
    /// Minutes between measurements of benefit in force per zone: new allocation is checked
    /// against the latest (ADR-012).
    pub refresh_minutes: u32,
}

impl AccumulationConfig {
    pub fn peril_of(&self, event_type: &str) -> Option<&Peril> {
        self.perils
            .values()
            .find(|p| p.event_types.contains(event_type))
    }
}

/// Hours of quiet that make a new trigger event a new loss occurrence for a daily benefit: below
/// a day, two disruptions on one day could each pay that day (R08).
const DAILY_AGGREGATION_HOURS: u32 = 24;

fn r51(detail: String) -> EngineError {
    EngineError::Rule {
        rule: "R51",
        detail,
    }
}

fn parse_accumulation(raw: RawAccumulation, triggers: &Triggers) -> Result<AccumulationConfig> {
    let mut perils = BTreeMap::new();
    let mut seen: BTreeSet<EventTypeId> = BTreeSet::new();
    for p in raw.perils {
        let mut zones = BTreeSet::new();
        for t in &p.event_types {
            let ty = triggers
                .event_types
                .get(t)
                .ok_or_else(|| r55(format!("peril {} names unregistered event type {t}", p.id)))?;
            zones.insert(format!("{:?}", ty.zone));
            if !seen.insert(t.clone()) {
                return Err(r51(format!("event type {t} is in more than one peril")));
            }
        }
        if p.event_types.is_empty() || (zones.len() > 1 && !p.across_zones) {
            return Err(r51(format!(
                "peril {} needs event types reported in the same zone unit, or across_zones",
                p.id
            )));
        }
        if p.event_limit.0 <= Money::ZERO {
            return Err(r51(format!("peril {}: event_limit must be positive", p.id)));
        }
        let peril = Peril {
            id: p.id.clone(),
            event_types: p.event_types,
            event_limit: p.event_limit.0,
            aggregation_hours: p.aggregation_hours,
            across_zones: p.across_zones,
        };
        if perils.insert(p.id.clone(), peril).is_some() {
            return Err(r51(format!("duplicate peril {}", p.id)));
        }
    }
    Ok(AccumulationConfig {
        perils,
        zone_accumulation_limit: raw.zone_accumulation_limit.0,
        refresh_minutes: raw.refresh_minutes.max(1),
    })
}

/// The languages customers can choose (BCP 47 tags such as `en`, `zh-Hant`), and the one used
/// when a customer has not chosen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Languages {
    pub default: String,
    pub offered: BTreeSet<String>,
}

impl Languages {
    /// The customer's setting when offered, otherwise the market default.
    pub fn resolve<'a>(&'a self, setting: Option<&'a str>) -> &'a str {
        setting
            .filter(|l| self.offered.contains(*l))
            .unwrap_or(&self.default)
    }
}

#[derive(Debug, Clone, Copy)]
pub struct PrivacyConfig {
    /// R54
    pub record_retention_years: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LicenceType {
    LicensedInsuranceAgency,
    LicensedInsuranceBroker,
    /// May refer a customer to the insurer's own journey, never bind (R52).
    ReferralOnly,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DistributionPartner {
    pub id: PartnerId,
    pub licence_type: LicenceType,
    pub licence_number: Option<String>,
    /// Last valid day, inclusive.
    pub licence_expiry: Option<NaiveDate>,
}

/// The text of each config file.
#[derive(Debug, Clone, Copy)]
pub struct ConfigSources<'a> {
    pub insurer: &'a str,
    pub lines_of_business: &'a str,
    pub rating_factors: &'a str,
    /// `event_types.yaml`, then a market overlay's: a later file's entry replaces the earlier
    /// one with the same id (ADR-012).
    pub event_types: &'a [&'a str],
    /// One text per `config/triggers/*.yaml`.
    pub trigger_sources: &'a [&'a str],
    pub exposure_types: &'a str,
    pub data_types: &'a str,
    /// One text per `config/catalogue/*.yaml`.
    pub catalogue: &'a [&'a str],
    pub prevention_measures: &'a str,
    pub suppliers: &'a str,
    /// `pilot.yaml`, when the market runs a pilot (R59).
    pub pilot: Option<&'a str>,
}

/// Parse and validate config. Converts decimals to `Bps`/`Money` exactly (R29, ADR-004) and
/// refuses an invalid class tree (R37), factor registry (R19, R33, R43), event, exposure or data
/// type registry (R55), or plan privilege naming an unapproved cover (R24, R45). Covers not in
/// `approved_products` load but are never offered.
pub fn from_yaml(src: &ConfigSources) -> Result<EngineConfig> {
    let raw: RawInsurer = serde_saphyr::from_str(src.insurer)
        .map_err(|e| EngineError::Config(format!("insurer: {e}")))?;
    let lines = LinesOfBusiness::parse(src.lines_of_business)?;
    let data_types = DataTypes::parse(src.data_types)?;
    let rating_factors = factors::parse(src.rating_factors, &lines, &data_types)?;
    let triggers = Triggers::parse(src.event_types, src.trigger_sources)?;
    let exposure_types = ExposureTypes::parse(src.exposure_types)?;
    let registries = Registries {
        lines: &lines,
        triggers: &triggers,
        exposure_types: &exposure_types,
        data_types: &data_types,
    };

    let mut catalogue = BTreeMap::new();
    for text in src.catalogue {
        let cover = parse_cover(text, &registries)?;
        if let Some(dup) = catalogue.insert(cover.id.clone(), cover) {
            return Err(EngineError::Config(format!("duplicate cover {}", dup.id)));
        }
    }

    let approved_products = raw.carrier.approved_products;
    if let Some(unknown) = approved_products
        .iter()
        .find(|c| !catalogue.contains_key(*c))
    {
        return Err(EngineError::Config(format!(
            "approved product {unknown} is not in the catalogue"
        )));
    }

    let pp = raw.policy_period;
    if pp.length != "month" {
        return Err(EngineError::Config(format!(
            "policy_period.length {:?}: only month is supported",
            pp.length
        )));
    }
    let step = pp.allocation_step.0;
    if step <= Money::ZERO {
        return Err(EngineError::Config(
            "allocation_step must be positive".into(),
        ));
    }
    let policy_period = PolicyPeriodConfig {
        allocation_step: step,
        min_policy_periods: pp.min_policy_periods,
        renewal_notice_days: pp.renewal_notice_days,
        grace_period_days: pp.grace_period_days,
        reinstatement_days: pp.reinstatement_days,
    };

    let mut plans: Vec<Plan> = Vec::new();
    for p in raw.plans {
        if plans.iter().any(|q| q.id == p.id) {
            return Err(EngineError::Config(format!("duplicate plan {}", p.id)));
        }
        if p.price.0 <= Money::ZERO || !p.price.0.is_multiple_of(step) {
            return Err(EngineError::Config(format!(
                "plan {}: price must be a positive multiple of the allocation step",
                p.id
            )));
        }
        if let Some(c) = p
            .privileges
            .extra_covers
            .iter()
            .find(|c| !approved_products.contains(*c))
        {
            return Err(EngineError::Rule {
                rule: "R24",
                detail: format!("plan {} privilege names unapproved cover {c}", p.id),
            });
        }
        if let Some(f) = p
            .privileges
            .extra_reward_families
            .iter()
            .find(|f| !raw.rewards.families.contains(*f))
        {
            return Err(EngineError::Rule {
                rule: "R45",
                detail: format!("plan {} privilege names unknown reward family {f}", p.id),
            });
        }
        plans.push(Plan {
            id: p.id,
            price: p.price.0,
            privileges: Privileges {
                extra_covers: p.privileges.extra_covers,
                extra_reward_families: p.privileges.extra_reward_families,
            },
        });
    }
    if plans.is_empty() {
        return Err(EngineError::Config("no plans configured".into()));
    }

    let mut promotions = Vec::new();
    for p in raw.promotions {
        if p.starts > p.ends || p.discount.0 > Bps::ONE {
            return Err(EngineError::Config(format!(
                "promotion {}: needs starts <= ends and a discount of at most 1",
                p.id
            )));
        }
        if let Some(unknown) = p
            .plans
            .iter()
            .find(|id| !plans.iter().any(|q| &q.id == *id))
        {
            return Err(EngineError::Config(format!(
                "promotion {} names unknown plan {unknown}",
                p.id
            )));
        }
        promotions.push(Promotion {
            id: p.id,
            starts: p.starts,
            ends: p.ends,
            plans: p.plans,
            discount: p.discount.0,
            channel: p.eligibility.channel,
            new_customers_only: p.eligibility.new_customers_only,
            funded_by: p.funded_by,
        });
    }

    let pr = raw.pricing;
    if pr.target_loss_ratio.0 == Bps::ZERO || pr.target_loss_ratio.0 > Bps::ONE {
        return Err(EngineError::Config(
            "target_loss_ratio must be above 0 and at most 1".into(),
        ));
    }
    if pr.rating_horizon_periods == 0 {
        return Err(EngineError::Config(
            "rating_horizon_periods must be at least 1 (R32)".into(),
        ));
    }
    let pricing = PricingConfig {
        target_loss_ratio: pr.target_loss_ratio.0,
        rating_horizon_periods: pr.rating_horizon_periods,
        continuous_cover_periods: pr.continuous_cover_periods,
        undeclared_credibility_full_exposure: pr.undeclared_credibility_full_exposure.max(1),
    };

    let uw = raw.underwriting;
    let risk_classes: Vec<RiskClass> = uw
        .risk_classes
        .into_iter()
        .map(|r| RiskClass {
            id: r.id,
            max_relativity: r.max_relativity.map(|b| b.0),
        })
        .collect();
    let thresholds: Vec<Option<Bps>> = risk_classes.iter().map(|r| r.max_relativity).collect();
    let well_formed = matches!(thresholds.last(), Some(None))
        && thresholds[..thresholds.len() - 1]
            .windows(2)
            .all(|w| matches!(w, [Some(a), Some(b)] if a < b))
        && thresholds[..thresholds.len() - 1]
            .iter()
            .all(Option::is_some);
    if !well_formed {
        return Err(EngineError::Rule {
            rule: "R39",
            detail:
                "risk_classes need ascending max_relativity and a last, postponed class without one"
                    .into(),
        });
    }
    let underwriting = UnderwritingConfig {
        risk_classes,
        claims_experience_periods: uw.claims_experience_periods,
        claims_experience_threshold: uw.claims_experience_threshold.0,
        credibility_full_periods: uw.credibility_full_periods.max(1),
        exclusion_notice_periods: uw.exclusion_notice_periods,
        postponement_periods: uw.postponement_periods,
        reentry_waiting_period_days: uw.reentry_waiting_period_days,
    };

    let sc = raw.score;
    if sc.max_effect.0 > Bps::ONE || sc.points_for_max_effect == 0 {
        return Err(EngineError::Config(
            "score.max_effect must be at most 1 and points_for_max_effect positive (R11)".into(),
        ));
    }
    let score = ScoreConfig {
        max_effect: sc.max_effect.0,
        point_half_life_days: sc.point_half_life_days.max(1),
        points_per_verified_activity: sc.points_per_verified_activity,
        points_for_max_effect: sc.points_for_max_effect,
    };

    let rw = raw.rewards;
    if rw.reward_share_of_claims_avoided.0 >= Bps::ONE {
        return Err(EngineError::Rule {
            rule: "R18",
            detail: "reward_share_of_claims_avoided must stay below 1".into(),
        });
    }
    if !rw.always_offer_cash {
        return Err(EngineError::Rule {
            rule: "R16",
            detail: "rewards.always_offer_cash must be true".into(),
        });
    }
    let loyalty = match rw.loyalty {
        None => None,
        Some(l) if l.streak_periods > 0 && l.reward.0 > Money::ZERO => Some(LoyaltyConfig {
            streak_periods: l.streak_periods,
            reward: l.reward.0,
        }),
        Some(_) => {
            return Err(EngineError::Rule {
                rule: "R26",
                detail: "rewards.loyalty needs positive streak_periods and reward".into(),
            });
        }
    };
    let rewards = RewardConfig {
        families: rw.families,
        monthly_cap: rw.monthly_cap_per_customer.0,
        reward_share_of_claims_avoided: rw.reward_share_of_claims_avoided.0,
        loyalty,
    };

    if !raw.privacy.delete_on_cancellation {
        return Err(EngineError::Rule {
            rule: "R54",
            detail: "privacy.delete_on_cancellation must be true".into(),
        });
    }

    let mut distribution_partners = BTreeMap::new();
    for p in raw.distribution_partners {
        let licensed = p.licence_type != LicenceType::ReferralOnly;
        if licensed && (p.licence_number.is_none() || p.licence_expiry.is_none()) {
            return Err(EngineError::Rule {
                rule: "R52",
                detail: format!(
                    "distribution partner {} needs a licence number and expiry",
                    p.id
                ),
            });
        }
        let partner = DistributionPartner {
            id: p.id.clone(),
            licence_type: p.licence_type,
            licence_number: p.licence_number,
            licence_expiry: p.licence_expiry,
        };
        if distribution_partners
            .insert(p.id.clone(), partner)
            .is_some()
        {
            return Err(EngineError::Config(format!(
                "duplicate distribution partner {}",
                p.id
            )));
        }
    }

    if raw.utc_offset_minutes.abs() > 18 * 60 {
        return Err(EngineError::Config(
            "utc_offset_minutes must be within 18 hours of UTC".into(),
        ));
    }
    let accumulation = parse_accumulation(raw.accumulation, &triggers)?;
    let guardrails = guardrails::parse_guardrails(raw.guardrails)?;
    let claims = guardrails::parse_claims(raw.claims)?;
    let languages = parse_languages(raw.languages)?;
    let prevention =
        PreventionMeasures::parse(src.prevention_measures, &lines, &data_types, &catalogue)?;
    let suppliers = Suppliers::parse(src.suppliers, &catalogue)?;
    let pilot = src.pilot.map(PilotConfig::parse).transpose()?;

    Ok(EngineConfig {
        market: raw.market,
        currency: raw.currency,
        utc_offset_minutes: raw.utc_offset_minutes,
        launch_phase: raw.launch_phase,
        policy_period,
        plans,
        promotions,
        catalogue,
        approved_products,
        lines,
        rating_factors,
        pricing,
        underwriting,
        score,
        rewards,
        guardrails,
        claims,
        privacy: PrivacyConfig {
            record_retention_years: raw.privacy.record_retention_years,
        },
        distribution_partners,
        accumulation,
        languages,
        triggers,
        exposure_types,
        data_types,
        prevention,
        suppliers,
        pilot,
    })
    .and_then(check_offered_triggers)
}

/// R55: every event type an offered cover pays on has an available source and a pricing basis.
/// R51: and belongs to a peril, all of the cover's event types to the same one, so one loss
/// occurrence never pays a cover twice; a daily benefit's peril aggregates over at least a day.
fn check_offered_triggers(cfg: EngineConfig) -> Result<EngineConfig> {
    for id in cfg.offered_covers() {
        let Some(cover) = cfg.catalogue.get(id) else {
            continue;
        };
        let Some(trigger) = cover.trigger.as_ref() else {
            continue;
        };
        let perils: BTreeSet<Option<&str>> = trigger
            .events
            .iter()
            .map(|e| {
                cfg.accumulation
                    .peril_of(&e.event_type)
                    .map(|p| p.id.as_str())
            })
            .collect();
        let peril = match perils.into_iter().collect::<Vec<_>>().as_slice() {
            [Some(p)] => cfg
                .accumulation
                .perils
                .get(*p)
                .ok_or_else(|| r51(String::new()))?,
            _ => {
                return Err(r51(format!(
                    "cover {id} is offered: its event types must all belong to one peril in \
                     accumulation.perils"
                )));
            }
        };
        let daily = cover
            .payout
            .as_ref()
            .is_some_and(|p| p.basis == PayoutBasis::FixedDaily);
        if daily && peril.aggregation_hours < DAILY_AGGREGATION_HOURS {
            return Err(r51(format!(
                "cover {id} pays a daily benefit: peril {} needs aggregation_hours of at least \
                 {DAILY_AGGREGATION_HOURS}",
                peril.id
            )));
        }
        for e in &trigger.events {
            if !cfg.triggers.has_available_source(&e.event_type) {
                return Err(r55(format!(
                    "cover {id} is offered but no available data source reports {}",
                    e.event_type
                )));
            }
            if !cfg.triggers.priceable(&e.event_type) {
                return Err(r55(format!(
                    "cover {id} is offered but {} has no source history to price on; \
                     use the insurer's filed rate or add a source with history",
                    e.event_type
                )));
            }
        }
    }
    Ok(cfg)
}

struct Registries<'a> {
    lines: &'a LinesOfBusiness,
    triggers: &'a Triggers,
    exposure_types: &'a ExposureTypes,
    data_types: &'a DataTypes,
}

fn check_exposure(cover_id: &str, e: &Exposure, reg: &Registries) -> Result<()> {
    let tree = match e.kind {
        ExposureKind::UpcomingEvent => Some(&reg.exposure_types.upcoming_event_kinds),
        ExposureKind::DeclaredItem => Some(&reg.exposure_types.declared_item_kinds),
        _ => None,
    };
    match tree {
        Some(_) if e.of.is_empty() => Err(r55(format!(
            "cover {cover_id}: exposure {:?} needs `of` (registered kinds)",
            e.kind
        ))),
        Some(t) => match e.of.iter().find(|k| !t.contains(k)) {
            Some(k) => Err(r55(format!(
                "cover {cover_id}: unregistered exposure kind {k}"
            ))),
            None => Ok(()),
        },
        None if !e.of.is_empty() => Err(r55(format!(
            "cover {cover_id}: exposure {:?} takes no `of`",
            e.kind
        ))),
        None => Ok(()),
    }
}

fn parse_cover(text: &str, reg: &Registries) -> Result<CoverSpec> {
    let raw: RawCover =
        serde_saphyr::from_str(text).map_err(|e| EngineError::Config(format!("catalogue: {e}")))?;
    reg.lines
        .check_cover_class(&raw.id, &raw.class_of_business)?;
    let offered = raw.launch_phase != LaunchPhase::Concept;
    let exposure = match (raw.exposure, offered) {
        (Some(e), _) => e,
        (None, false) => Exposure {
            kind: ExposureKind::Everyone,
            of: BTreeSet::new(),
        },
        (None, true) => {
            return Err(EngineError::Config(format!(
                "cover {}: an offered cover needs an exposure",
                raw.id
            )));
        }
    };
    check_exposure(&raw.id, &exposure, reg)?;
    if offered && raw.product_limit.is_none() {
        return Err(EngineError::Config(format!(
            "cover {}: an offered cover needs a product_limit (R34)",
            raw.id
        )));
    }
    let trigger = match raw.trigger {
        Some(t) => Some(TriggerSpec {
            events: t
                .events
                .iter()
                .map(|c| reg.triggers.condition(c, &raw.id))
                .collect::<Result<_>>()?,
            requires_declared_hours: t.requires_declared_hours,
            resumption_minutes: t.resumption_minutes,
            min_disrupted_minutes: t.min_disrupted_minutes,
            match_on: t.match_on,
        }),
        None => None,
    };
    if let Some(t) = &trigger {
        check_match_key(&raw.id, t, reg.triggers)?;
        if !t.requires_declared_hours && (t.resumption_minutes > 0 || t.min_disrupted_minutes > 0) {
            return Err(EngineError::Rule {
                rule: "R08",
                detail: format!(
                    "cover {}: resumption_minutes and min_disrupted_minutes apply to declared hours",
                    raw.id
                ),
            });
        }
    }
    let parametric = raw.cover_type == CoverType::Parametric;
    if parametric && trigger.as_ref().is_none_or(|t| t.events.is_empty()) {
        return Err(r55(format!(
            "cover {}: a parametric cover pays only on registered event types",
            raw.id
        )));
    }
    let payout = match raw.payout {
        Some(p) => Some(parse_payout(&raw.id, p, trigger.as_ref(), reg.triggers)?),
        None => None,
    };
    let evidence = raw.claim.map(|c| c.evidence).unwrap_or_default();
    if let Some(e) = evidence
        .iter()
        .find(|e| !reg.data_types.evidence_kinds.contains_key(*e))
    {
        return Err(r55(format!(
            "cover {}: unregistered evidence kind {e}",
            raw.id
        )));
    }
    let block_new_while = raw
        .eligibility
        .map(|e| e.block_new_while)
        .unwrap_or_default()
        .iter()
        .map(|c| reg.triggers.condition(c, &raw.id))
        .collect::<Result<_>>()?;
    Ok(CoverSpec {
        id: raw.id,
        name: raw.name,
        cover_type: raw.cover_type,
        class_of_business: raw.class_of_business,
        key_features: raw.key_features,
        launch_phase: raw.launch_phase,
        exposure,
        product_limit: raw.product_limit.map(|m| m.0),
        waiting_period_days: raw.waiting_period_days,
        trigger,
        payout,
        claim_evidence: evidence.into_iter().collect(),
        block_new_while,
        settlement_in_kind: raw.settlement_in_kind,
    })
}

/// R55: an event reported per zone pays only the customers whose declaration names that zone, so
/// the cover says which declaration it matches on; a territory-wide event matches everyone.
fn check_match_key(cover_id: &str, trigger: &TriggerSpec, triggers: &Triggers) -> Result<()> {
    for e in &trigger.events {
        let zone = triggers
            .event_types
            .get(&e.event_type)
            .map(|t| t.zone)
            .ok_or_else(|| r55(format!("cover {cover_id}: unregistered {}", e.event_type)))?;
        let fits = matches!(
            (zone, trigger.match_on),
            (ZoneUnit::Territory, None)
                | (ZoneUnit::Line, Some(MatchKey::DeclaredRoute))
                | (ZoneUnit::District, Some(MatchKey::DeclaredArea))
                | (ZoneUnit::Event, Some(MatchKey::TicketReference))
                | (ZoneUnit::Flight, Some(MatchKey::DeclaredBooking))
        );
        if !fits {
            return Err(r55(format!(
                "cover {cover_id}: {} is reported per {zone:?}; its `match` ({:?}) cannot \
                 name the customer's zone",
                e.event_type, trigger.match_on
            )));
        }
    }
    Ok(())
}

fn parse_payout(
    cover_id: &str,
    p: RawPayout,
    trigger: Option<&TriggerSpec>,
    triggers: &Triggers,
) -> Result<PayoutSpec> {
    let cap = p.cap_ratio_of_daily_earnings.map(|b| b.0);
    if cap.is_some_and(|c| c >= Bps::ONE) {
        return Err(EngineError::Rule {
            rule: "R09",
            detail: format!("cover {cover_id}: cap_ratio_of_daily_earnings must be below 1"),
        });
    }
    let bad = |detail: &str| r55(format!("cover {cover_id}: {detail}"));
    let mut tiers = BTreeMap::new();
    match p.basis {
        PayoutBasis::TieredByStage => {
            let [condition] = trigger.map(|t| t.events.as_slice()).unwrap_or_default() else {
                return Err(bad("tiered_by_stage needs exactly one event type"));
            };
            let ty = triggers
                .event_types
                .get(&condition.event_type)
                .ok_or_else(|| bad("unregistered event type"))?;
            for (stage, share) in &p.tiers {
                let rank = ty
                    .stage_rank(stage)
                    .filter(|r| *r >= condition.from_stage)
                    .ok_or_else(|| bad(&format!("tier {stage} is not a stage it pays on")))?;
                if share.0 == Bps::ZERO || share.0 > Bps::ONE {
                    return Err(bad("tier shares must be above 0 and at most 1"));
                }
                tiers.insert(rank, share.0);
            }
            let shares: Vec<Bps> = tiers.values().copied().collect();
            if tiers.first_key_value().map(|(r, _)| *r) != Some(condition.from_stage)
                || !shares.windows(2).all(|w| w[0] <= w[1])
            {
                return Err(bad(
                    "tiers must start at from_stage and never fall as stages rise",
                ));
            }
        }
        PayoutBasis::PerHour => {
            let in_force = trigger.is_some_and(|t| {
                t.events.iter().all(|e| {
                    triggers
                        .event_types
                        .get(&e.event_type)
                        .is_some_and(|ty| ty.measure == Measure::InForce)
                })
            });
            if p.max_hours.is_none() || !in_force {
                return Err(bad("per_hour needs max_hours and events measured in force"));
            }
        }
        _ if !p.tiers.is_empty() => return Err(bad("tiers apply only to tiered_by_stage")),
        _ => {}
    }
    Ok(PayoutSpec {
        basis: p.basis,
        cap_ratio_of_daily_earnings: cap,
        tiers,
        max_hours: p.max_hours,
    })
}

// ---- raw shapes of the YAML files

#[derive(Deserialize)]
struct RawInsurer {
    market: String,
    currency: String,
    utc_offset_minutes: i32,
    launch_phase: u8,
    carrier: RawCarrier,
    policy_period: RawPolicyPeriod,
    plans: Vec<RawPlan>,
    #[serde(default)]
    promotions: Vec<RawPromotion>,
    pricing: RawPricing,
    score: RawScore,
    underwriting: RawUnderwriting,
    rewards: RawRewards,
    guardrails: guardrails::RawGuardrails,
    claims: guardrails::RawClaims,
    privacy: RawPrivacy,
    #[serde(default)]
    distribution_partners: Vec<RawPartner>,
    accumulation: RawAccumulation,
    languages: RawLanguages,
}

#[derive(Deserialize)]
struct RawLanguages {
    default: String,
    offered: BTreeSet<String>,
}

fn parse_languages(raw: RawLanguages) -> Result<Languages> {
    let tag = |t: &str| {
        !t.is_empty()
            && t.split('-')
                .all(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_alphanumeric()))
    };
    if let Some(bad) = raw.offered.iter().find(|t| !tag(t)) {
        return Err(EngineError::Config(format!(
            "languages: {bad:?} is not a language tag"
        )));
    }
    if !raw.offered.contains(&raw.default) {
        return Err(EngineError::Config(format!(
            "languages: the default {} is not offered",
            raw.default
        )));
    }
    Ok(Languages {
        default: raw.default,
        offered: raw.offered,
    })
}

#[derive(Deserialize)]
struct RawAccumulation {
    perils: Vec<RawPeril>,
    zone_accumulation_limit: DecMoney,
    #[serde(default = "five")]
    refresh_minutes: u32,
}

fn five() -> u32 {
    5
}

#[derive(Deserialize)]
struct RawPeril {
    id: String,
    event_types: BTreeSet<EventTypeId>,
    event_limit: DecMoney,
    #[serde(default)]
    aggregation_hours: u32,
    #[serde(default)]
    across_zones: bool,
}

#[derive(Deserialize)]
struct RawCarrier {
    #[serde(default)]
    approved_products: BTreeSet<CoverId>,
}

#[derive(Deserialize)]
struct RawPolicyPeriod {
    length: String,
    allocation_step: DecMoney,
    min_policy_periods: u32,
    renewal_notice_days: u32,
    grace_period_days: u32,
    reinstatement_days: u32,
}

#[derive(Deserialize)]
struct RawPlan {
    id: PlanId,
    price: DecMoney,
    #[serde(default)]
    privileges: RawPrivileges,
}

#[derive(Deserialize, Default)]
struct RawPrivileges {
    #[serde(default)]
    extra_covers: BTreeSet<CoverId>,
    #[serde(default)]
    extra_reward_families: BTreeSet<String>,
}

#[derive(Deserialize)]
struct RawPromotion {
    id: String,
    starts: NaiveDate,
    ends: NaiveDate,
    plans: BTreeSet<PlanId>,
    discount: DecBps,
    #[serde(default)]
    eligibility: RawPromotionEligibility,
    funded_by: Funder,
}

#[derive(Deserialize, Default)]
struct RawPromotionEligibility {
    #[serde(default)]
    channel: Option<String>,
    #[serde(default)]
    new_customers_only: bool,
}

#[derive(Deserialize)]
struct RawPricing {
    target_loss_ratio: DecBps,
    rating_horizon_periods: u32,
    continuous_cover_periods: u32,
    undeclared_credibility_full_exposure: u32,
}

#[derive(Deserialize)]
struct RawScore {
    max_effect: DecBps,
    point_half_life_days: u32,
    points_per_verified_activity: u32,
    points_for_max_effect: u32,
}

#[derive(Deserialize)]
struct RawUnderwriting {
    risk_classes: Vec<RawRiskClass>,
    claims_experience_periods: u32,
    claims_experience_threshold: DecBps,
    credibility_full_periods: u32,
    exclusion_notice_periods: u32,
    postponement_periods: u32,
    reentry_waiting_period_days: u32,
}

#[derive(Deserialize)]
struct RawRiskClass {
    id: String,
    #[serde(default)]
    max_relativity: Option<DecBps>,
}

#[derive(Deserialize)]
struct RawRewards {
    families: BTreeSet<String>,
    monthly_cap_per_customer: DecMoney,
    reward_share_of_claims_avoided: DecBps,
    #[serde(default = "yes")]
    always_offer_cash: bool,
    #[serde(default)]
    loyalty: Option<RawLoyalty>,
}

#[derive(Deserialize)]
struct RawLoyalty {
    streak_periods: u32,
    reward: DecMoney,
}

fn yes() -> bool {
    true
}

#[derive(Deserialize)]
struct RawPrivacy {
    #[serde(default)]
    delete_on_cancellation: bool,
    record_retention_years: u32,
}

#[derive(Deserialize)]
struct RawPartner {
    id: PartnerId,
    licence_type: LicenceType,
    #[serde(default)]
    licence_number: Option<String>,
    #[serde(default)]
    licence_expiry: Option<NaiveDate>,
}

#[derive(Deserialize)]
struct RawCover {
    id: CoverId,
    name: String,
    #[serde(rename = "type")]
    cover_type: CoverType,
    class_of_business: String,
    key_features: String,
    launch_phase: LaunchPhase,
    #[serde(default)]
    exposure: Option<Exposure>,
    #[serde(default)]
    product_limit: Option<DecMoney>,
    #[serde(default)]
    waiting_period_days: u16,
    #[serde(default)]
    trigger: Option<RawTrigger>,
    #[serde(default)]
    payout: Option<RawPayout>,
    #[serde(default)]
    claim: Option<RawClaim>,
    #[serde(default)]
    eligibility: Option<RawEligibility>,
    #[serde(default)]
    settlement_in_kind: Vec<String>,
}

#[derive(Deserialize)]
struct RawTrigger {
    events: Vec<RawCondition>,
    #[serde(default)]
    requires_declared_hours: bool,
    #[serde(default)]
    resumption_minutes: u32,
    #[serde(default)]
    min_disrupted_minutes: u32,
    #[serde(default, rename = "match")]
    match_on: Option<MatchKey>,
}

#[derive(Deserialize)]
struct RawPayout {
    basis: PayoutBasis,
    #[serde(default)]
    cap_ratio_of_daily_earnings: Option<DecBps>,
    #[serde(default)]
    tiers: BTreeMap<String, DecBps>,
    #[serde(default)]
    max_hours: Option<u32>,
}

#[derive(Deserialize)]
struct RawClaim {
    #[serde(default)]
    evidence: Vec<String>,
}

#[derive(Deserialize)]
struct RawEligibility {
    #[serde(default)]
    block_new_while: Vec<RawCondition>,
}

/// `launch_phase: 1` or `launch_phase: concept`.
impl<'de> Deserialize<'de> for LaunchPhase {
    fn deserialize<D: Deserializer<'de>>(d: D) -> std::result::Result<Self, D::Error> {
        struct PhaseVisitor;
        impl Visitor<'_> for PhaseVisitor {
            type Value = LaunchPhase;
            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("a launch phase number or `concept`")
            }
            fn visit_u64<E: de::Error>(self, v: u64) -> std::result::Result<LaunchPhase, E> {
                u8::try_from(v)
                    .map(LaunchPhase::Phase)
                    .map_err(|_| E::custom("launch phase out of range"))
            }
            fn visit_i64<E: de::Error>(self, v: i64) -> std::result::Result<LaunchPhase, E> {
                u8::try_from(v)
                    .map(LaunchPhase::Phase)
                    .map_err(|_| E::custom("launch phase out of range"))
            }
            fn visit_str<E: de::Error>(self, v: &str) -> std::result::Result<LaunchPhase, E> {
                match v {
                    "concept" => Ok(LaunchPhase::Concept),
                    _ => Err(E::custom(format!("unknown launch phase {v:?}"))),
                }
            }
        }
        d.deserialize_any(PhaseVisitor)
    }
}
