//! The API's own boundary types: request bodies, responses and the records it keeps in the
//! policy store. Engine and AI types cross the boundary as they are; these only add what a
//! channel needs around them. Published in `contracts/` by `just gen`.
use crate::identity::KycStatus;
use chrono::{DateTime, NaiveDate, Utc};
use microinsurance_ai::explain::CoverExplanation;
use microinsurance_ai::understand::ProposalForm;
use microinsurance_engine::claims::{Claim, ClaimPayment, DeclineReason};
use microinsurance_engine::consent::{Endorsement, Offer, PremiumStatus};
use microinsurance_engine::guardrails::appeals::Appeal;
use microinsurance_engine::guardrails::human_care::Triage;
use microinsurance_engine::guardrails::queue::PersonAction;
use microinsurance_engine::money::{Bps, Money};
use microinsurance_engine::optimiser::Suggestion;
use microinsurance_engine::payouts::{PayoutDecision, Settled, Settlement};
use microinsurance_engine::plans::PlanPrice;
use microinsurance_engine::prevention::PreventionTask;
use microinsurance_engine::rewards::{Grant, RewardOffer};
use microinsurance_engine::types::{
    CoverId, CoverType, DeclaredItem, Exposure, PartnerId, Policy, Schedule, UpcomingEvent,
    WorkWindow,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

use microinsurance_learning::pilot::PilotMetric;
use microinsurance_workers::queue::{JobRow, JobStatus};

// Records the workers write too (`workers::book`).
pub use microinsurance_workers::book::{
    AuditRecord, CustomerRecord, PayoutNotice, PolicyRecord, RatingAnswers, RenewalRecord,
};

/// Where a recommendation came from: the model, its saved answer, or a fixed rule (R22).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AiSource {
    Model,
    Fallback,
    Rule,
}

impl From<microinsurance_ai::harness::Source> for AiSource {
    fn from(s: microinsurance_ai::harness::Source) -> AiSource {
        use microinsurance_ai::harness::Source;
        match s {
            Source::Model => AiSource::Model,
            Source::Fallback => AiSource::Fallback,
            Source::Rule => AiSource::Rule,
        }
    }
}

// ---- catalogue and channel

/// One cover of the catalogue as a customer reads it.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct CoverInfo {
    pub cover_id: CoverId,
    pub name: String,
    pub cover_type: CoverType,
    pub class_of_business: String,
    /// The key features sentence (R23).
    pub key_features: String,
    /// What makes the cover protect the customer (R42).
    pub exposure: Exposure,
    pub waiting_period_days: u16,
    pub product_limit: Option<Money>,
    /// A parametric cover paying its benefit once per disrupted day (R08); otherwise per event or
    /// per loss.
    pub daily_benefit: bool,
}

/// Whether a partner's channel can bind cover today (R52).
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ChannelInfo {
    pub partner_id: PartnerId,
    /// True only with a licence valid today.
    pub binds: bool,
    /// Where to send the customer when the partner cannot bind: the insurer's own journey.
    pub refer_to: Option<String>,
}

/// A registered rating factor the customer may answer, with the answers its table rates (R33).
/// Not answering is allowed: the factor is then rated at the undeclared group's average.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct RatingQuestion {
    pub factor_id: String,
    /// What the factor means, from `config/rating_factors.yaml`.
    pub formula: String,
    pub answers: Vec<String>,
}

// ---- the customer

/// What the customer declares. Any field left out keeps its value.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct ProfileUpdate {
    pub occupation: Option<String>,
    pub work_windows: Option<Vec<WorkWindow>>,
    pub declared_items: Option<Vec<DeclaredItem>>,
    pub commute_routes: Option<Vec<String>>,
    /// Registered areas where the declared hours are worked (ADR-012).
    pub work_areas: Option<Vec<String>>,
    pub upcoming_events: Option<Vec<UpcomingEvent>>,
    pub typical_daily_earnings: Option<Money>,
    /// One of the insurer's offered languages.
    pub language: Option<String>,
}

/// The customer's opt-ins, by registered data type (R20).
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ConsentsUpdate {
    pub consents: BTreeSet<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ProposalFormRequest {
    /// The channel the customer came from, such as `delivery_app` or `direct`.
    pub channel: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ProposalFormView {
    pub form: ProposalForm,
    pub source: AiSource,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ProposalAnswers {
    pub form: ProposalForm,
    /// Answers by question id.
    pub answers: BTreeMap<String, String>,
}

/// Facts read from the answers, in registered ids, for the customer to confirm in their profile.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ProfileFactsView {
    pub occupation: Option<String>,
    pub devices: Vec<String>,
    pub commute_routes: Vec<String>,
    pub source: AiSource,
}

// ---- quotes, binding and changes

/// What the customer asks a quote for: a plan, and optionally their own split of its price
/// (R34). Without a split, the optimiser suggests one (R02).
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct QuoteRequest {
    pub plan_id: String,
    #[serde(default)]
    pub premiums: Option<BTreeMap<CoverId, Money>>,
}

/// A quote: the terms put to the customer and everything they were shown with them, kept as
/// shown (R49). Accepting it is the customer's tap (R03).
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Quote {
    pub offer: Offer,
    /// The schedule offered: the suggestion's, or the customer's own split priced.
    pub schedule: Schedule,
    /// What the customer pays for the plan today, with any promotional discount (R44).
    pub price: PlanPrice,
    /// The optimiser's suggestion and the factors behind it, when one was asked for (R49).
    pub suggestion: Option<Suggestion>,
    /// One plain sentence per cover (R23).
    pub explanations: Vec<CoverExplanation>,
    pub explained_by: AiSource,
    /// The policy period the schedule is for.
    pub period_starts: NaiveDate,
    pub shown_at: DateTime<Utc>,
    pub accepted_at: Option<DateTime<Utc>>,
}

/// What the customer's tap did.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Accepted {
    /// New business: the policy bound (R58).
    pub policy: Option<Policy>,
    /// A change: effective from the next policy period (R04).
    pub endorsement: Option<Endorsement>,
}

/// The customer's policy today.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct PolicyView {
    pub policy: Policy,
    pub partner_id: Option<PartnerId>,
    pub premium: PremiumStatus,
    /// Accepted changes for the next policy period (R04).
    pub endorsements: Vec<Endorsement>,
    pub explanations: Vec<CoverExplanation>,
}

/// The renewal notice with one sentence per change (R35, R23).
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct RenewalView {
    pub notice: microinsurance_engine::consent::RenewalNotice,
    /// The day the notice was issued (`ProposeRenewal`, R35); none while it is a preview at
    /// today's rates.
    pub issued_on: Option<NaiveDate>,
    pub sentences: Vec<ChangeSentence>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ChangeSentence {
    pub cover_id: CoverId,
    pub sentence: String,
    pub source: AiSource,
}

// ---- prevention, score and rewards

/// Evidence as uploaded: base64 of the file, with its media type.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct EvidenceFile {
    /// `image/jpeg`, `image/png`, `image/webp`, `text/plain` or `application/json`.
    pub media_type: String,
    pub data_base64: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct EvidenceUpload {
    pub files: Vec<EvidenceFile>,
}

/// A prevention task as the store keeps it.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct TaskRecord {
    pub task: PreventionTask,
    /// When it went to a person, for the work queue's service level.
    pub with_person_since: Option<DateTime<Utc>>,
}

/// What evidence for a task did: the task, and the reward granted if it was credited.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct TaskOutcome {
    pub task: PreventionTask,
    pub granted: Option<Grant>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct PoolScore {
    pub pool: String,
    /// Points after fading (R12).
    pub points: u32,
    /// The effect they earn in this pool, at most `max_effect` (R11).
    pub effect: Bps,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ScoreView {
    pub pools: Vec<PoolScore>,
}

/// The reward balance and how it can be spent: cash for all of it first, always (R16).
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct RewardsView {
    pub balance: Money,
    /// In the order shown: cash first, then partner rewards as ranked.
    pub menu: Vec<RewardOffer>,
    pub ranked_by: AiSource,
    pub grants: Vec<Grant>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct RedeemRequest {
    pub offer_id: String,
}

/// A partner's spare capacity, published as reward offers (R16, R56).
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct RewardOffers {
    pub offers: Vec<RewardOffer>,
}

// ---- recovery

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SettlementChoice {
    pub settlement: Settlement,
}

/// The customer's notification of loss (R13, R46).
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ClaimRequest {
    pub cover_id: CoverId,
    pub loss_date: NaiveDate,
    pub amount_claimed: Money,
    /// In the customer's words, for the assessor.
    pub description: String,
    /// One of `guardrails.human_care.urgent`, when there is one (R46).
    #[serde(default)]
    pub circumstance: Option<String>,
    #[serde(default)]
    pub asked_for_person: bool,
    #[serde(default)]
    pub guarantee_requested: Option<Money>,
}

/// A claim as the store keeps it and the customer sees it.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ClaimRecord {
    pub claim: Claim,
    pub description: String,
    /// The triage when notified, with any first response (R46).
    pub triage: Triage,
    /// The guarantee above the automatic cap, sent to the assistance provider to confirm.
    pub guarantee_reference: Option<String>,
    pub payment: Option<ClaimPayment>,
    pub settled: Option<Settled>,
    /// Evidence fingerprints received.
    pub evidence: Vec<String>,
    pub assessed_by: Option<AiSource>,
}

/// Evidence for a claim: what each file is (registered kinds) and the files.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ClaimEvidence {
    pub kinds: BTreeSet<String>,
    pub files: Vec<EvidenceFile>,
}

/// The customer asks a person to look at a decision (R15).
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ReviewRequest {
    pub decision_id: String,
    /// In the customer's words.
    pub reason: String,
}

/// What a review request opened: a review of the decision, or, for a declined claim, the claim
/// back with a different person (R15).
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ReviewOpened {
    pub appeal: Option<Appeal>,
    pub claim: Option<Claim>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct IdentityCheck {
    pub session_id: String,
    pub status: KycStatus,
}

// ---- partners

/// A partner's own reference for its customer, to exchange for a customer token.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct TokenRequest {
    pub customer_ref: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct IssuedToken {
    pub token: String,
    /// The insurer's id for the customer.
    pub customer_id: String,
    pub expires_at: DateTime<Utc>,
}

// ---- people at the insurer

/// A person's decision on a work item. The action must be one the item offers; the person is
/// the caller (ADR-010).
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct PersonDecision {
    pub item_id: String,
    pub action: PersonAction,
    /// For a decline: the reason, from the closed list (R13).
    #[serde(default)]
    pub decline_reason: Option<DeclineReason>,
    /// For an approval: the loss the person assessed (R48).
    #[serde(default)]
    pub amount: Option<Money>,
    /// For a request for evidence: the registered kinds asked for.
    #[serde(default)]
    pub evidence_kinds: Option<BTreeSet<String>>,
    #[serde(default)]
    pub note: String,
}

/// A guarantee of payment sent to the assistance provider to confirm (R46).
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct GuaranteeRecord {
    pub claim_id: String,
    pub customer_id: String,
    pub automatic: Money,
    pub to_confirm: Money,
    pub reference: String,
    pub requested_at: DateTime<Utc>,
}

// ---- feeds

/// What one pushed report did.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct FeedOutcome {
    pub source: String,
    pub opened: Vec<String>,
    pub closed: Vec<String>,
    pub payouts: Vec<PayoutDecision>,
}

/// A premium the partner's wallet collected for a policy bound through it (R53).
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct PremiumPayment {
    pub policy_id: String,
    /// The policy period it pays for: the current one.
    pub period_index: u32,
}

/// What a customer can declare and opt into, and how the market writes money and time: the
/// registries the apps build their forms from (R20, R55).
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Registry {
    /// ISO 4217, such as `HKD`.
    pub currency: String,
    /// Minutes east of UTC of market local time.
    pub utc_offset_minutes: i32,
    pub default_language: String,
    pub languages: Vec<String>,
    pub data_types: Vec<String>,
    pub evidence_kinds: Vec<String>,
    pub upcoming_event_kinds: Vec<String>,
    pub declared_item_kinds: Vec<String>,
    pub routes: Vec<String>,
    /// The work areas a customer can declare: a city, county or district (ADR-012).
    pub areas: Vec<String>,
    /// The step premium is allocated in, in minor units (R34).
    pub allocation_step: Money,
    /// Circumstances that make a claim urgent (R46).
    pub urgent_circumstances: Vec<String>,
}

// ---- adoption: the job queue, the data loop and the pilot (ADR-012)

/// The job queue for people at the insurer: how many jobs are in each state, and the failed ones
/// waiting for a person.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct JobsView {
    pub counts: BTreeMap<JobStatus, i64>,
    pub failed: Vec<JobRow>,
}

/// One pilot gate, as fixed before the pilot started (`pilot.yaml`): a bound in the metric's own
/// terms, a rate as a fraction and hours as hours.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct PilotGate {
    pub metric: String,
    pub kind: GateKind,
    #[serde(default)]
    pub at_least: Option<f64>,
    #[serde(default)]
    pub at_most: Option<f64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum GateKind {
    /// Every one must pass, measured, to scale up.
    Scale,
    /// Any one breached stops the pilot.
    Stop,
}

/// The pilot's report: what the book shows, with the gates to evaluate it against
/// (`microinsurance_economics.pilot_evaluation`).
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct PilotReport {
    pub pilot_id: String,
    pub starts: NaiveDate,
    pub ends: NaiveDate,
    pub gates_fixed_on: NaiveDate,
    /// The binding dates counted: the pilot's, unless the request named others.
    pub from: NaiveDate,
    pub to: NaiveDate,
    pub as_of: NaiveDate,
    pub metrics: Vec<PilotMetric>,
    pub gates: Vec<PilotGate>,
}
