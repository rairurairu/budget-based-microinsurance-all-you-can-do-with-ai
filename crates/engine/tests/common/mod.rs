//! Synthetic config and personas for the rule tests. Values are illustrative, not the adopting
//! insurer's; tests must not depend on `config/`.
#![allow(dead_code)]

use chrono::{DateTime, NaiveDate, Utc};
use microinsurance_engine::config::{self, ConfigSources, EngineConfig};
use microinsurance_engine::money::{Bps, Money, Rate};
use microinsurance_engine::optimiser::PeriodContext;
use microinsurance_engine::optimiser::rating::{
    self, CoverRating, FactorTable, RatingBasis, RatingInputs, UndeclaredGroup,
};
use microinsurance_engine::optimiser::underwriting::ClassStatus;
use microinsurance_engine::plans;
use microinsurance_engine::triggers::{Occurrence, TriggerZone};
use microinsurance_engine::types::{
    ClassId, CoverId, DeclaredItem, Plan, Policy, PolicyPeriod, Profile, Schedule, UpcomingEvent,
    WorkWindow, add_policy_periods,
};
use std::collections::{BTreeMap, BTreeSet};

pub const INSURER: &str = r#"
market: HK
currency: HKD
languages: { default: en, offered: [en, zh-Hant] }
utc_offset_minutes: 480
launch_phase: 1
carrier:
  approved_products: [weather_income, commute_disruption, event_ticket, device_damage, personal_accident, flight_delay]
policy_period:
  length: month
  allocation_step: 5
  min_policy_periods: 1
  renewal_notice_days: 7
  grace_period_days: 7
  reinstatement_days: 30
plans:
  - { id: starter, price: 40 }
  - { id: essential, price: 80 }
  - { id: max, price: 150, privileges: { extra_covers: [flight_delay], extra_reward_families: [loyalty] } }
promotions:
  - id: launch
    starts: 2027-01-01
    ends: 2027-03-31
    plans: [essential]
    discount: 0.20
    eligibility: { channel: partner_app, new_customers_only: true }
    funded_by: insurer
pricing:
  target_loss_ratio: 0.5
  rating_horizon_periods: 3
  continuous_cover_periods: 3
  undeclared_credibility_full_exposure: 1000
score:
  max_effect: 0.20
  point_half_life_days: 45
  points_per_verified_activity: 100
  points_for_max_effect: 1000
underwriting:
  risk_classes:
    - { id: preferred, max_relativity: 0.80 }
    - { id: standard, max_relativity: 1.25 }
    - { id: substandard, max_relativity: 2.00 }
    - { id: postponed }
  claims_experience_periods: 12
  claims_experience_threshold: 1.5
  credibility_full_periods: 12
  exclusion_notice_periods: 1
  postponement_periods: 3
  reentry_waiting_period_days: 14
rewards:
  families: [insurance, partner, loyalty]
  monthly_cap_per_customer: 30
  reward_share_of_claims_avoided: 0.25
  loyalty: { streak_periods: 3, reward: 10 }
guardrails:
  audit_sample_rate: 0.05
  risk_review_threshold: 0.8
  appeal_sla_hours: 72
  human_care:
    urgent: [injury, hospitalisation, emergency_abroad]
    large_loss_share_of_monthly_income: 0.5
    urgent_response_seconds: 5
    first_response: [emergency_numbers, assistance_provider_connect, guarantee_of_payment]
    auto_guarantee_cap: 5000
    person_on_request: true
  fairness:
    groups: [age_band, gender, district]
    min_parity_ratio: 0.8
    min_group_decisions: 30
claims:
  stp_limit: 2000
  stp_min_confidence: 0.85
  estimate_tolerance: 0.15
  referral_sla_hours: 48
  anomaly:
    early_claim_days: 30
    late_notification_days: 30
    frequent_claims: 3
    near_sum_insured: 0.9
    weights: { early_claim: 0.3, late_notification: 0.1, frequent_claims: 0.3, near_sum_insured: 0.2, evidence_reused: 0.8 }
privacy:
  delete_on_cancellation: true
  record_retention_years: 7
distribution_partners:
  - { id: delivery_app, licence_type: licensed_insurance_agency, licence_number: "TEST-0001", licence_expiry: 2027-12-31 }
  - { id: blog, licence_type: referral_only }
accumulation:
  perils:
    - { id: windstorm_and_rain, event_types: [weather.tropical_cyclone, weather.rainstorm], event_limit: 1000000, aggregation_hours: 24 }
    - { id: heat, event_types: [workplace.heat_stress], event_limit: 100000, aggregation_hours: 24 }
    - { id: rail, event_types: [transit.rail_disruption], event_limit: 100000, aggregation_hours: 1 }
    - { id: flight, event_types: [travel.flight_delay], event_limit: 100000 }
    - { id: ticketed_events, event_types: [event.disruption], event_limit: 100000 }
  zone_accumulation_limit: 5000000
"#;

pub const LINES: &str = r#"
points_pool_depth: 1
experience_links:
  - { from: travel.trip, to: travel.flight_delay, weight: 0.5, evidence: "synthetic test link" }
nodes:
  - { id: property, name: Property }
  - { id: property.device, name: Personal devices }
  - { id: property.home_contents, name: Home contents }
  - { id: accident_health, name: Accident and health }
  - { id: accident_health.personal_accident, name: Personal accident }
  - { id: travel, name: Travel }
  - { id: travel.trip, name: Trip }
  - { id: travel.flight_delay, name: Flight delay }
  - { id: pecuniary, name: Pecuniary loss }
  - { id: pecuniary.income_weather, name: Weather income }
  - { id: pecuniary.commute, name: Commute }
  - { id: pecuniary.event_ticket, name: Event ticket }
"#;

pub const EVENT_TYPES: &str = r#"
event_types:
  - { id: weather.tropical_cyclone, name: Tropical cyclone, stages: [signal_1, signal_3, signal_8, signal_9, signal_10], measure: in_force, zone: territory, pricing_basis: historical_record }
  - { id: weather.rainstorm, name: Rainstorm, stages: [amber, red, black], measure: in_force, zone: territory, pricing_basis: historical_record }
  - { id: workplace.heat_stress, name: Heat stress, stages: [amber, red, black], measure: in_force, zone: territory, pricing_basis: insurer_filed_rate }
  - { id: transit.rail_disruption, name: Rail disruption, stages: [delay, suspension], measure: in_force, zone: line, pricing_basis: insurer_filed_rate }
  - { id: travel.flight_delay, name: Flight delay, stages: [delayed], measure: delay, zone: flight, pricing_basis: insurer_filed_rate }
  - { id: event.disruption, name: Event disruption, stages: [postponed, cancelled], measure: in_force, zone: event, pricing_basis: insurer_filed_rate }
"#;

pub const HKO: &str = r#"
id: hko_warnings
status: available
history: { available: true, since: 1946 }
codes:
  TC1: { event_type: weather.tropical_cyclone, stage: signal_1 }
  TC3: { event_type: weather.tropical_cyclone, stage: signal_3 }
  TC8NE: { event_type: weather.tropical_cyclone, stage: signal_8 }
  TC9: { event_type: weather.tropical_cyclone, stage: signal_9 }
  TC10: { event_type: weather.tropical_cyclone, stage: signal_10 }
  WRAINA: { event_type: weather.rainstorm, stage: amber }
  WRAINB: { event_type: weather.rainstorm, stage: black }
"#;

pub const TRANSIT: &str = r#"
id: transit_disruptions
status: available
codes:
  RAIL_DELAY: { event_type: transit.rail_disruption, stage: delay }
  RAIL_SUSPENDED: { event_type: transit.rail_disruption, stage: suspension }
"#;

pub const FLIGHTS: &str = r#"
id: flight_status
status: available
codes:
  DELAYED: { event_type: travel.flight_delay, stage: delayed }
"#;

pub const ORGANISERS: &str = r#"
id: organiser_notices
status: available
codes:
  CANCELLED: { event_type: event.disruption, stage: cancelled }
"#;

pub const HEAT: &str = r#"
id: labour_heat_stress
status: available
history: { available: true, since: 2024 }
codes:
  HSWR: { event_type: workplace.heat_stress, stage: red }
  HSWB: { event_type: workplace.heat_stress, stage: black }
"#;

pub const SOURCES: [&str; 5] = [HKO, TRANSIT, FLIGHTS, ORGANISERS, HEAT];

pub const EXPOSURES: &str = r#"
upcoming_event_kinds:
  - { id: event, name: Ticketed event }
  - { id: event.concert, name: Concert }
  - { id: event.sports, name: Sports match }
  - { id: trip, name: Trip }
  - { id: trip.air, name: Trip by air }
declared_item_kinds:
  - { id: device, name: Personal device }
  - { id: device.phone, name: Phone }
  - { id: device.laptop, name: Laptop }
  - { id: vehicle, name: Vehicle }
  - { id: vehicle.bicycle, name: Bicycle }
  - { id: home_contents, name: Belongings at home }
"#;

pub const DATA_TYPES: &str = r#"
data_types:
  - { id: trip_data, name: Trip data }
  - { id: location, name: Location }
  - { id: device_diagnostics, name: Device diagnostics }
  - { id: calendar_plans, name: Calendar plans }
  - { id: photos, name: Photos }
evidence_kinds:
  - { id: photo, data_type: photos }
  - { id: hospital_receipt }
"#;

pub const FACTORS: &str = r#"
factors:
  - { id: hazard_frequency, level: market, basis: industry, status: approved, classes: [pecuniary], formula: "P(trigger)" }
  - { id: age_band, level: individual, basis: industry, status: approved, data: declared, classes: [accident_health.personal_accident], formula: "x age relativity" }
  - { id: trip_plan, level: individual, basis: industry, status: approved, data: calendar_plans, classes: [travel], formula: "x trip relativity" }
  - { id: claims_experience, level: individual, basis: industry, status: approved, data: own_claims, classes: [property, accident_health, travel, pecuniary], formula: "peer-relative" }
  - { id: declared_work_hours, level: individual, basis: novel, status: approved, data: declared, classes: [pecuniary.income_weather], formula: "x share of hazard hours" }
  - { id: device_condition, level: individual, basis: novel, status: candidate, data: device_diagnostics, classes: [property.device], formula: "x condition" }
"#;

pub const WEATHER: &str = r#"
id: weather_income
name: Weather income cover
type: parametric
class_of_business: pecuniary.income_weather
launch_phase: 1
key_features: "Paid when a typhoon stops work"
exposure: { kind: declared_hours }
product_limit: 800
trigger:
  events:
    - { event_type: weather.tropical_cyclone, from_stage: signal_8 }
    - { event_type: weather.rainstorm, from_stage: black }
  requires_declared_hours: true
  resumption_minutes: 120
  min_disrupted_minutes: 60
payout: { basis: fixed_daily, cap_ratio_of_daily_earnings: 0.8 }
eligibility:
  block_new_while: [{ event_type: weather.tropical_cyclone, from_stage: signal_1 }]
waiting_period_days: 0
"#;

pub const COMMUTE: &str = r#"
id: commute_disruption
name: Commute disruption cover
type: parametric
class_of_business: pecuniary.commute
launch_phase: 1
key_features: "Paid when your route is postponed"
exposure: { kind: declared_route }
product_limit: 300
trigger:
  events: [{ event_type: transit.rail_disruption, from_stage: delay, min_minutes: 30 }]
  match: declared_route
payout: { basis: fixed_per_event }
"#;

pub const EVENT: &str = r#"
id: event_ticket
name: Event ticket cover
type: parametric
class_of_business: pecuniary.event_ticket
launch_phase: 1
key_features: "Ticket money back if the show is cancelled"
exposure: { kind: upcoming_event, of: [event] }
product_limit: 2000
trigger:
  events: [{ event_type: event.disruption, from_stage: cancelled }]
  match: ticket_reference
payout: { basis: declared_value }
"#;

pub const DEVICE: &str = r#"
id: device_damage
name: Phone and device damage cover
type: indemnity
class_of_business: property.device
launch_phase: 1
key_features: "Repair or replace your phone fast"
exposure: { kind: declared_item, of: [device] }
product_limit: 8000
claim: { evidence: [photo], assessed_by: ai_estimate_then_rules }
waiting_period_days: 14
settlement_in_kind: [partner_repair]
"#;

pub const ACCIDENT: &str = r#"
id: personal_accident
name: Personal accident cover
type: indemnity
class_of_business: accident_health.personal_accident
launch_phase: 1
key_features: "Hospital and injury costs after an accident"
exposure: { kind: everyone }
product_limit: 50000
claim: { evidence: [hospital_receipt] }
"#;

pub const FLIGHT: &str = r#"
id: flight_delay
name: Flight delay cover
type: parametric
class_of_business: travel.flight_delay
launch_phase: 3
key_features: "Paid when your flight is late"
exposure: { kind: upcoming_event, of: [trip.air] }
product_limit: 1000
trigger:
  events: [{ event_type: travel.flight_delay, from_stage: delayed, min_minutes: 120 }]
  match: declared_booking
payout: { basis: fixed_per_event }
settlement_in_kind: [lounge_access]
"#;

pub const CONTENTS: &str = r#"
id: contents
name: Renter's contents cover
type: indemnity
class_of_business: property.home_contents
launch_phase: concept
key_features: "Your belongings at home"
"#;

pub const CATALOGUE: [&str; 7] = [WEATHER, COMMUTE, EVENT, DEVICE, ACCIDENT, FLIGHT, CONTENTS];

pub const PREVENTION: &str = r#"
measures:
  - { id: case_and_screen_protector, class_of_business: property.device, description: "A case and screen protector on the phone.", evidence: [photo], repeat_after_days: 90 }
  - { id: helmet_worn, class_of_business: accident_health.personal_accident, description: "A helmet worn when riding.", evidence: [photo], repeat_after_days: 30 }
"#;

pub const SUPPLIERS: &str = r#"
suppliers:
  - id: lounges
    name: Lounge network
    in_kind:
      - { option: lounge_access, tariff: 200, retail_value: 650, valid_days: 30 }
  - id: airline
    name: Airline rebooking
    in_kind:
      - { option: alternative_seat, invoiced: true, valid_days: 7 }
  - id: repairer
    name: Repair network
    in_kind:
      - { option: partner_repair, invoiced: true, valid_days: 14 }
  - id: partner_shop
    name: Partner shop
    reward_valid_days: 30
"#;

pub fn load(
    insurer: &str,
    lines: &str,
    factors: &str,
    catalogue: &[&str],
) -> microinsurance_engine::Result<EngineConfig> {
    load_registries(
        insurer,
        lines,
        factors,
        EVENT_TYPES,
        &SOURCES,
        EXPOSURES,
        catalogue,
    )
}

pub fn load_registries(
    insurer: &str,
    lines: &str,
    factors: &str,
    event_types: &str,
    sources: &[&str],
    exposures: &str,
    catalogue: &[&str],
) -> microinsurance_engine::Result<EngineConfig> {
    config::from_yaml(&ConfigSources {
        insurer,
        lines_of_business: lines,
        rating_factors: factors,
        event_types: &[event_types],
        trigger_sources: sources,
        exposure_types: exposures,
        data_types: DATA_TYPES,
        catalogue,
        prevention_measures: PREVENTION,
        suppliers: SUPPLIERS,
        pilot: None,
    })
}

/// The synthetic config texts; tests override one field to check a validation.
pub fn sources() -> ConfigSources<'static> {
    ConfigSources {
        insurer: INSURER,
        lines_of_business: LINES,
        rating_factors: FACTORS,
        event_types: &[EVENT_TYPES],
        trigger_sources: &SOURCES,
        exposure_types: EXPOSURES,
        data_types: DATA_TYPES,
        catalogue: &CATALOGUE,
        prevention_measures: PREVENTION,
        suppliers: SUPPLIERS,
        pilot: None,
    }
}

pub fn cfg() -> EngineConfig {
    load(INSURER, LINES, FACTORS, &CATALOGUE).expect("synthetic config loads")
}

pub fn day(y: i32, m: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, d).expect("valid date")
}

pub fn at(y: i32, m: u32, d: u32) -> DateTime<Utc> {
    day(y, m, d)
        .and_hms_opt(12, 0, 0)
        .expect("valid time")
        .and_utc()
}

pub fn period(index: u32, y: i32, m: u32) -> PolicyPeriod {
    let starts = day(y, m, 1);
    PolicyPeriod {
        index,
        starts,
        ends: add_policy_periods(starts, 1),
    }
}

pub fn plan(cfg: &EngineConfig, id: &str) -> Plan {
    cfg.plan(id).expect("configured plan").clone()
}

fn table(rels: &[(&str, u32)], undeclared: u32, exposure: u32) -> FactorTable {
    FactorTable {
        relativities: rels
            .iter()
            .map(|(k, v)| ((*k).to_string(), Bps(*v)))
            .collect(),
        undeclared: UndeclaredGroup {
            relativity: Bps(undeclared),
            exposure_periods: exposure,
        },
    }
}

/// Flat monthly expected claim cost (ppm of sum insured) from 2026-07 to 2028-06.
pub fn flat_curve(ppm: u32) -> BTreeMap<NaiveDate, Rate> {
    (0..24)
        .map(|i| (add_policy_periods(day(2026, 7, 1), i), Rate(ppm)))
        .collect()
}

pub fn basis() -> RatingBasis {
    let mut b = RatingBasis::default();
    for (cover, ppm) in [
        ("weather_income", 20_000),
        ("commute_disruption", 30_000),
        ("event_ticket", 5_000),
        ("device_damage", 4_000),
        ("personal_accident", 200),
        ("flight_delay", 50_000),
    ] {
        b.expected_cost.insert(cover.into(), flat_curve(ppm));
    }
    let mut tables = |factor: &str, class: &str, t: FactorTable| {
        b.factor_tables
            .entry(factor.into())
            .or_default()
            .insert(class.into(), t);
    };
    tables(
        "age_band",
        "accident_health",
        table(
            &[("18-30", 10_000), ("31-50", 9_000), ("51-65", 13_000)],
            12_000,
            500,
        ),
    );
    tables(
        "declared_work_hours",
        "pecuniary.income_weather",
        table(&[("day", 10_000), ("night", 6_000)], 10_000, 0),
    );
    tables(
        "device_condition",
        "property.device",
        table(&[("good", 8_000), ("poor", 15_000)], 10_000, 0),
    );
    tables(
        "trip_plan",
        "travel",
        table(&[("short", 10_000), ("long", 14_000)], 11_000, 1_000),
    );
    b
}

/// A delivery rider: daytime hours, a route, a phone and a concert this month.
pub fn rider() -> Profile {
    Profile {
        customer_id: "cust-rider".into(),
        occupation: Some("delivery rider".into()),
        work_windows: vec![WorkWindow {
            weekday: 1,
            start_minute: 9 * 60,
            end_minute: 18 * 60,
        }],
        declared_items: vec![DeclaredItem {
            kind: "device.phone".into(),
            description: "phone".into(),
            declared_value: Some(Money::from_major(4_000)),
        }],
        commute_routes: vec!["Tsuen Wan line".into()],
        work_areas: Vec::new(),
        upcoming_events: vec![UpcomingEvent {
            kind: "event.concert".into(),
            starts: day(2026, 10, 20),
            ends: None,
            reference: Some("TICKET-1".into()),
            value: Some(Money::from_major(800)),
        }],
        typical_daily_earnings: Some(Money::from_major(600)),
        consents: BTreeSet::new(),
        language: None,
    }
}

pub fn inputs() -> RatingInputs {
    RatingInputs {
        factor_values: [("age_band", "31-50"), ("declared_work_hours", "day")]
            .into_iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect(),
        classes: BTreeMap::new(),
    }
}

pub fn ratings(
    cfg: &EngineConfig,
    plan: &Plan,
    profile: &Profile,
    inputs: &RatingInputs,
    basis: &RatingBasis,
    period: &PolicyPeriod,
) -> BTreeMap<CoverId, CoverRating> {
    rating::rate_covers(
        &plans::covers_for(plan, cfg),
        period.starts,
        inputs,
        &profile.consents,
        basis,
        cfg,
    )
    .expect("covers rate")
}

/// Owns what a `PeriodContext` borrows.
pub struct World {
    pub cfg: EngineConfig,
    pub period: PolicyPeriod,
    pub ratings: BTreeMap<CoverId, CoverRating>,
    pub statuses: BTreeMap<ClassId, ClassStatus>,
    pub in_force: BTreeSet<Occurrence>,
    pub full_zones: BTreeSet<TriggerZone>,
}

impl World {
    /// Rates every approved cover for the rider's inputs.
    pub fn new(profile: &Profile, basis: &RatingBasis, period: PolicyPeriod) -> World {
        World::with_inputs(profile, &inputs(), basis, period)
    }

    pub fn with_inputs(
        profile: &Profile,
        inputs: &RatingInputs,
        basis: &RatingBasis,
        period: PolicyPeriod,
    ) -> World {
        let cfg = cfg();
        let covers: Vec<String> = cfg.approved_products.iter().cloned().collect();
        let ratings = rating::rate_covers(
            &covers,
            period.starts,
            inputs,
            &profile.consents,
            basis,
            &cfg,
        )
        .expect("covers rate");
        World {
            cfg,
            period,
            ratings,
            statuses: BTreeMap::new(),
            in_force: BTreeSet::new(),
            full_zones: BTreeSet::new(),
        }
    }

    pub fn ctx(&self) -> PeriodContext<'_> {
        PeriodContext {
            cfg: &self.cfg,
            period: self.period,
            ratings: &self.ratings,
            statuses: &self.statuses,
            in_force: &self.in_force,
            full_zones: &self.full_zones,
        }
    }

    pub fn plan(&self, id: &str) -> Plan {
        plan(&self.cfg, id)
    }
}

pub fn policy(cfg: &EngineConfig, schedule: Schedule, period: PolicyPeriod) -> Policy {
    let plan = plan(cfg, &schedule.plan_id);
    Policy {
        policy_id: "pol-1".into(),
        customer_id: "cust-rider".into(),
        cover_starts: schedule
            .allocations
            .iter()
            .map(|a| (a.cover_id.clone(), period.starts))
            .collect(),
        plan,
        schedule,
        period,
        work_windows: rider().work_windows,
        commute_routes: rider().commute_routes,
        work_areas: Vec::new(),
        upcoming_events: rider().upcoming_events,
        subscribed_at: at(2026, 9, 1),
    }
}

pub fn consent_to(profile: &mut Profile, data: &str) {
    profile.consents.insert(data.to_string());
}

/// A market whose income trigger is announced per area (ADR-012): the local government's work
/// suspension, one loss occurrence across every area, paid where the customer declared they work.
pub const AREA_EVENT_TYPES: &str = r#"
event_types:
  - { id: work.suspension, name: Work suspended by the local government, stages: [suspended], measure: in_force, zone: district, pricing_basis: insurer_filed_rate }
"#;

pub const SUSPENSIONS: &str = r#"
id: work_suspensions
status: available
codes:
  WORK_SUSPENDED: { event_type: work.suspension, stage: suspended }
"#;

pub const AREA_WEATHER: &str = r#"
id: weather_income
name: Weather income cover
type: parametric
class_of_business: pecuniary.income_weather
launch_phase: 1
key_features: "Paid for a working day the local government suspends work where you work"
exposure: { kind: declared_hours }
product_limit: 800
trigger:
  events:
    - { event_type: work.suspension, from_stage: suspended }
  match: declared_area
  requires_declared_hours: true
  resumption_minutes: 0
  min_disrupted_minutes: 60
payout: { basis: fixed_daily }
waiting_period_days: 0
"#;

/// The synthetic insurer with the suspension peril counted across areas.
pub fn area_insurer() -> String {
    INSURER.replace(
        "  zone_accumulation_limit: 5000000",
        "    - { id: suspension, event_types: [work.suspension], event_limit: 1000000, aggregation_hours: 24, across_zones: true }\n  zone_accumulation_limit: 5000000",
    )
}

/// The area market's config: the base registries with the overlay's event types, its source and
/// its weather income cover.
pub fn area_cfg() -> EngineConfig {
    let insurer = area_insurer();
    let feeds = [HKO, TRANSIT, FLIGHTS, ORGANISERS, HEAT, SUSPENSIONS];
    config::from_yaml(&ConfigSources {
        insurer: &insurer,
        event_types: &[EVENT_TYPES, AREA_EVENT_TYPES],
        trigger_sources: &feeds,
        catalogue: &[
            AREA_WEATHER,
            COMMUTE,
            EVENT,
            DEVICE,
            ACCIDENT,
            FLIGHT,
            CONTENTS,
        ],
        ..sources()
    })
    .expect("the area market's config loads")
}
