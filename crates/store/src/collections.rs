//! The collections of the policy store and what each holds. The record types are the engine's
//! and the API's own (`crates/api`); the store only keeps them.

/// The customer's declarations and opt-ins (`Profile`, R20) and how they came (partner, R52).
pub const CUSTOMERS: &str = "customers";
/// Answers to registered rating factors, kept apart from the profile: they never reach the AI
/// layer (R19, R33).
pub const RATING_ANSWERS: &str = "rating_answers";
/// Every quote shown, with what it showed: the suggestion, its factors, the sentences and the
/// price (R49). Append-only.
pub const QUOTES: &str = "quotes";
pub const POLICIES: &str = "policies";
/// Accepted changes, effective from the next policy period (R04).
pub const ENDORSEMENTS: &str = "endorsements";
/// Parametric payouts decided, with how they were settled (R27).
pub const PAYOUTS: &str = "payouts";
pub const PREVENTION_TASKS: &str = "prevention_tasks";
/// Verified prevention activities, for points and the repeat window (R10, R11).
pub const ACTIVITIES: &str = "activities";
/// Rewards granted into the reward balance (R17).
pub const GRANTS: &str = "grants";
pub const REDEMPTIONS: &str = "redemptions";
pub const CLAIMS: &str = "claims";
/// Review requests (R15).
pub const APPEALS: &str = "appeals";
/// The decision log: who decided what, on which inputs (ADR-010).
pub const DECISIONS: &str = "decisions";
/// Decisions drawn for audit after the fact, waiting for a person (R14).
pub const AUDITS: &str = "audits";
pub const AUDIT_FINDINGS: &str = "audit_findings";
/// Evidence fingerprints already used, so reuse is caught (R10, R14).
pub const EVIDENCE: &str = "evidence";
/// Partner rewards each supplier offers from spare capacity, with stock (R16, R56).
pub const REWARD_OFFERS: &str = "reward_offers";
/// Identity checks with the eKYC provider (R25).
pub const IDENTITY_CHECKS: &str = "identity_checks";
/// Guarantees of payment sent to the assistance provider to confirm (R46).
pub const GUARANTEES: &str = "guarantees";
/// In-kind orders issued to suppliers, for settlement and expiry (R56).
pub const IN_KIND_ORDERS: &str = "in_kind_orders";
/// Renewal notices as issued, with the next period's default schedule (R35), by
/// `<policy>/<period index>` (ADR-012).
pub const RENEWALS: &str = "renewals";
/// Each policy period as it opened: the schedule in force, the exposure an experience study
/// counts (ADR-012). Append-only, by `<policy>/<period index>`.
pub const POLICY_PERIODS: &str = "policy_periods";
/// Records of completed erasures of a cancelled customer's optional data (R20, R54).
pub const ERASURES: &str = "erasures";
/// Experience studies for the actuary, by month (ADR-012).
pub const EXPERIENCE_STUDIES: &str = "experience_studies";
/// Benefit in force per trigger zone as last measured (`current`), for the accumulation control
/// (R51, ADR-012).
pub const ACCUMULATION: &str = "accumulation";
