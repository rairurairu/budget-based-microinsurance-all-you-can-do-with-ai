use crate::money::{Bps, Money};
use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

pub type CustomerId = String;
pub type PolicyId = String;
pub type CoverId = String;
pub type PlanId = String;
pub type PartnerId = String;
/// A node of `config/lines_of_business.yaml`, as a dotted path (R37).
pub type ClassId = String;
/// The class of business at `points_pool_depth`: points and rewards never cross it (R36).
pub type PoolId = String;
/// A rating factor id from `config/rating_factors.yaml` (R33).
pub type FactorId = String;
/// A trigger event type from `config/event_types.yaml` (R55).
pub type EventTypeId = String;
/// An optional data type from `config/data_types.yaml` (R20).
pub type DataTypeId = String;

/// A fixed plan, the same for every customer (R01). Chosen by the customer, never recommended
/// (R45).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct Plan {
    pub id: PlanId,
    pub price: Money,
    #[serde(default)]
    pub privileges: Privileges,
}

/// What a plan adds beyond the launch-phase catalogue, set in config only (R45).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct Privileges {
    #[serde(default)]
    pub extra_covers: BTreeSet<CoverId>,
    #[serde(default)]
    pub extra_reward_families: BTreeSet<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub enum CoverType {
    Parametric,
    Indemnity,
}

/// When a cover is offered: from a numbered launch phase, or not yet (a concept).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LaunchPhase {
    Phase(u8),
    Concept,
}

/// How a cover finds whom it protects. The engine computes the customer's need differently for
/// each form; the kinds of items and events are registered in `config/exposure_types.yaml` (R55,
/// ADR-005).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub enum ExposureKind {
    /// Declared working hours (R08).
    DeclaredHours,
    /// A declared regular route.
    DeclaredRoute,
    /// A declared item of a registered kind, with its value.
    DeclaredItem,
    /// An upcoming event of a registered kind in the policy period, after the waiting period.
    UpcomingEvent,
    /// Anyone covered has the exposure.
    Everyone,
}

/// What a customer must have for a cover to protect them this policy period. The optimiser
/// places usable premium only where there is exposure (R42).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct Exposure {
    pub kind: ExposureKind,
    /// For declared items and upcoming events: registered kinds; a parent covers its subtree.
    #[serde(default)]
    pub of: BTreeSet<String>,
}

/// How a trigger event is measured (R55).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Measure {
    /// In force from a start to an end; a cover may require minutes in force.
    InForce,
    /// The source reports minutes late; a cover requires a minimum.
    Delay,
}

/// The event types and stages a cover pays on, or that stop new cover (R06, R55).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EventCondition {
    pub event_type: EventTypeId,
    /// Rank of the least severe stage that counts, in the event type's stage order.
    pub from_stage: u8,
    /// Minutes in force (or late, for a delay) before it counts.
    pub min_minutes: Option<u32>,
}

/// The personal reference a trigger must match, beyond the event itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MatchKey {
    DeclaredRoute,
    /// A declared work area: an event reported per district, county or city (ADR-012).
    DeclaredArea,
    TicketReference,
    DeclaredBooking,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TriggerSpec {
    /// Any one of these pays.
    pub events: Vec<EventCondition>,
    /// R08: pays only for events overlapping declared working hours.
    pub requires_declared_hours: bool,
    /// R08: minutes after the trigger ends, or falls below the cover's stage, that still count
    /// as disrupted: the time to resume work.
    pub resumption_minutes: u32,
    /// R08: the least disrupted time inside a day's declared hours for that day to count.
    pub min_disrupted_minutes: u32,
    pub match_on: Option<MatchKey>,
}

/// How a parametric payout is sized; each basis is computed differently.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PayoutBasis {
    /// The benefit amount once per day an event touches.
    FixedDaily,
    /// The benefit amount once per event.
    FixedPerEvent,
    /// The declared value of what the event spoils (a ticket price, a booked cost), up to the
    /// benefit amount.
    DeclaredValue,
    /// A share of the benefit amount by the event's stage (`tiers`).
    TieredByStage,
    /// The benefit amount per hour of the event within declared hours, up to `max_hours`.
    PerHour,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PayoutSpec {
    pub basis: PayoutBasis,
    /// R09: the benefit amount per event stays below this share of typical daily earnings.
    pub cap_ratio_of_daily_earnings: Option<Bps>,
    /// For `TieredByStage`: share of the benefit amount by stage rank, non-decreasing.
    pub tiers: BTreeMap<u8, Bps>,
    /// For `PerHour`.
    pub max_hours: Option<u32>,
}

/// One entry of `config/catalogue`, validated at load.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoverSpec {
    pub id: CoverId,
    pub name: String,
    pub cover_type: CoverType,
    /// Exactly one leaf class of business (R37).
    pub class_of_business: ClassId,
    pub key_features: String,
    pub launch_phase: LaunchPhase,
    pub exposure: Exposure,
    /// The most sum insured (or benefit amount per trigger event) one customer can hold (R34).
    pub product_limit: Option<Money>,
    pub waiting_period_days: u16,
    pub trigger: Option<TriggerSpec>,
    pub payout: Option<PayoutSpec>,
    /// Registered evidence kinds a claim asks for (`config/data_types.yaml`).
    pub claim_evidence: BTreeSet<String>,
    /// Event types and stages that stop new cover while in force (R06).
    pub block_new_while: Vec<EventCondition>,
    pub settlement_in_kind: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct Allocation {
    pub cover_id: CoverId,
    /// Share of the plan price.
    pub premium: Money,
    /// Sum insured (indemnity) or benefit amount per trigger event (parametric) this premium buys.
    pub sum_insured: Money,
}

/// The covers and sums insured for one policy period. Allocations are ordered by cover id.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct Schedule {
    pub plan_id: PlanId,
    pub allocations: Vec<Allocation>,
}

impl Schedule {
    pub fn premium(&self, cover_id: &str) -> Money {
        self.allocation(cover_id).map_or(Money::ZERO, |a| a.premium)
    }

    pub fn allocation(&self, cover_id: &str) -> Option<&Allocation> {
        self.allocations.iter().find(|a| a.cover_id == cover_id)
    }

    pub fn total_premium(&self) -> Money {
        self.allocations.iter().map(|a| a.premium).sum()
    }
}

/// A declared working window in market local time (R08). `weekday` is ISO: 1 = Monday to
/// 7 = Sunday. Minutes count from local midnight; an `end_minute` at or before `start_minute`
/// runs past midnight into the next day (a night shift), and the shift belongs to the day it
/// starts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct WorkWindow {
    pub weekday: u8,
    pub start_minute: u16,
    pub end_minute: u16,
}

/// A known future event, for example a concert in three months.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct UpcomingEvent {
    /// A registered upcoming event kind (`config/exposure_types.yaml`), such as `event.concert`.
    pub kind: String,
    pub starts: NaiveDate,
    pub ends: Option<NaiveDate>,
    pub reference: Option<String>,
    /// Ticket price or booked cost, when declared.
    #[serde(default)]
    pub value: Option<Money>,
}

/// Something the customer declared, with its declared value when given.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct DeclaredItem {
    /// A registered declared item kind (`config/exposure_types.yaml`), such as `device.phone`.
    pub kind: String,
    pub description: String,
    pub declared_value: Option<Money>,
}

/// What the engine may know about a customer. No protected attributes here (R19); industry
/// rating attributes (age, gender, district) arrive only through `optimiser::RatingInputs`,
/// which never reaches the AI layer.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct Profile {
    pub customer_id: CustomerId,
    pub occupation: Option<String>,
    pub work_windows: Vec<WorkWindow>,
    pub declared_items: Vec<DeclaredItem>,
    pub commute_routes: Vec<String>,
    /// Registered areas (a district, county or city) where the declared hours are worked, for a
    /// trigger reported per area (R55, ADR-012). The insured location: never an AI input (R19).
    #[serde(default)]
    pub work_areas: Vec<String>,
    pub upcoming_events: Vec<UpcomingEvent>,
    pub typical_daily_earnings: Option<Money>,
    /// Registered optional data types the customer opted into (`config/data_types.yaml`, R20).
    pub consents: BTreeSet<DataTypeId>,
    /// The customer's language setting, one of the insurer's `languages.offered`; none means
    /// the market default.
    #[serde(default)]
    pub language: Option<String>,
}

/// One policy period. `ends` is the first day after it, so consecutive periods share a date.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct PolicyPeriod {
    pub index: u32,
    pub starts: NaiveDate,
    pub ends: NaiveDate,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct Policy {
    pub policy_id: PolicyId,
    pub customer_id: CustomerId,
    pub plan: Plan,
    pub schedule: Schedule,
    pub period: PolicyPeriod,
    pub cover_starts: BTreeMap<CoverId, NaiveDate>,
    pub work_windows: Vec<WorkWindow>,
    /// Declared routes a route-matched trigger pays on (R55).
    #[serde(default)]
    pub commute_routes: Vec<String>,
    /// Declared work areas an area-matched trigger pays on (R55, ADR-012).
    #[serde(default)]
    pub work_areas: Vec<String>,
    /// Declared tickets and bookings a ticket- or booking-matched trigger pays on, with their
    /// declared value (R55).
    #[serde(default)]
    pub upcoming_events: Vec<UpcomingEvent>,
    pub subscribed_at: DateTime<Utc>,
}

/// The start of the policy period `n` periods after `start` (policy periods are months).
pub fn add_policy_periods(start: NaiveDate, n: u32) -> NaiveDate {
    start
        .checked_add_months(chrono::Months::new(n))
        .unwrap_or(NaiveDate::MAX)
}

/// The start of the policy period `n` periods before `start`.
pub fn sub_policy_periods(start: NaiveDate, n: u32) -> NaiveDate {
    start
        .checked_sub_months(chrono::Months::new(n))
        .unwrap_or(NaiveDate::MIN)
}
