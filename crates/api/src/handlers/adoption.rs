//! What the insurer's people need to run and adopt the platform (ADR-012): the job queue's
//! state, the experience study for the actuary, and the pilot's report for its gates. Each is an
//! aggregate over the book; none carries a customer id.
use crate::auth::StaffCaller;
use crate::bodies::{
    ClaimRecord, CustomerRecord, JobsView, PilotGate, PilotReport, PolicyRecord, Quote, TaskRecord,
};
use crate::error::{ApiError, ApiResult};
use crate::state::{AppState, Inner};
use axum::Json;
use axum::extract::{Query, State};
use chrono::NaiveDate;
use microinsurance_engine::claims::{ClaimState, DecidedBy, DeclineReason};
use microinsurance_engine::guardrails::audit::AuditFinding;
use microinsurance_engine::pilot::{self, Cohort};
use microinsurance_engine::prevention::TaskState;
use microinsurance_engine::rewards::{Grant, Redemption};
use microinsurance_engine::types::CoverType;
use microinsurance_learning::experience::ExperienceStudy;
use microinsurance_learning::pilot::{self as metrics, PilotBook, PilotCustomer};
use microinsurance_ledger::{Account, LedgerStore};
use microinsurance_store::{By, collections as c};
use microinsurance_workers::book::PayoutNotice;
use microinsurance_workers::jobs::{Batch, PolicyPeriodRecord};
use microinsurance_workers::ops::{AuditStore, snapshot};
use microinsurance_workers::queue::JobStatus;
use serde::Deserialize;
use std::collections::{BTreeMap, BTreeSet};

/// Failed jobs listed at most.
const FAILED_SHOWN: u32 = 100;

/// The job queue: how many jobs in each state, and the failed ones for a person (ADR-012).
pub async fn jobs(State(state): State<AppState>, _caller: StaffCaller) -> ApiResult<JobsView> {
    let counts = state.queue.counts().await.map_err(ApiError::internal)?;
    let failed = state
        .queue
        .rows(JobStatus::Failed, FAILED_SHOWN)
        .await
        .map_err(ApiError::internal)?;
    Ok(Json(JobsView { counts, failed }))
}

#[derive(Debug, Deserialize)]
pub struct MonthQuery {
    /// `YYYY-MM`; this month when unset.
    pub month: Option<String>,
}

/// The experience study to a month: the one the monthly job kept, or one worked out now.
pub async fn experience(
    State(state): State<AppState>,
    _caller: StaffCaller,
    Query(q): Query<MonthQuery>,
) -> ApiResult<ExperienceStudy> {
    let month = q
        .month
        .unwrap_or_else(|| state.today().format("%Y-%m").to_string());
    if NaiveDate::parse_from_str(&format!("{month}-01"), "%Y-%m-%d").is_err() {
        return Err(ApiError::bad_request(format!(
            "{month}: a month is YYYY-MM"
        )));
    }
    if let Some(kept) = state
        .store
        .get::<ExperienceStudy>(c::EXPERIENCE_STUDIES, &month)
        .await?
    {
        return Ok(Json(kept.body));
    }
    let settings = state
        .config
        .experience_settings()
        .map_err(ApiError::internal)?;
    let in_force = BTreeSet::new();
    let batch = Batch {
        store: &state.store,
        ledger: &state.ledger,
        cfg: &state.cfg,
        basis: &state.rating,
        in_force: &in_force,
        full_credibility_claims: settings.full_credibility_claims,
    };
    batch
        .study(&month)
        .await
        .map(Json)
        .map_err(|e| ApiError::internal(e.0))
}

#[derive(Debug, Deserialize)]
pub struct WindowQuery {
    /// Count customers bound from this day; the pilot's start when unset.
    pub from: Option<NaiveDate>,
    /// Up to this day; the pilot's end when unset.
    pub to: Option<NaiveDate>,
}

#[derive(Deserialize)]
struct GatesFile {
    pilot: Gates,
}

#[derive(Deserialize)]
struct Gates {
    gates: Vec<PilotGate>,
}

/// The pilot's report: its metrics from the book, with the gates fixed before it started.
pub async fn pilot_report(
    State(state): State<AppState>,
    _caller: StaffCaller,
    Query(q): Query<WindowQuery>,
) -> ApiResult<PilotReport> {
    let pilot = state
        .cfg
        .pilot
        .as_ref()
        .ok_or_else(|| ApiError::not_found("this market has no pilot (pilot.yaml)"))?;
    let text = state
        .config
        .pilot_text()
        .map_err(ApiError::internal)?
        .ok_or_else(|| ApiError::internal("pilot.yaml"))?;
    let gates: GatesFile = serde_saphyr::from_str(&text)
        .map_err(|e| ApiError::internal(format!("pilot.yaml gates: {e}")))?;
    let (from, to) = (q.from.unwrap_or(pilot.starts), q.to.unwrap_or(pilot.ends));
    let book = pilot_book(&state, from, to).await?;
    Ok(Json(PilotReport {
        pilot_id: pilot.id.clone(),
        starts: pilot.starts,
        ends: pilot.ends,
        gates_fixed_on: pilot.gates_fixed_on,
        from,
        to,
        as_of: state.today(),
        metrics: metrics::metrics(&book),
        gates: gates.pilot.gates,
    }))
}

/// The book as the pilot counts it: customers bound in `[from, to]`, and the money and decisions
/// of the whole book.
async fn pilot_book(state: &Inner, from: NaiveDate, to: NaiveDate) -> Result<PilotBook, ApiError> {
    let cfg = &state.cfg;
    let today = state.today();
    let offset = cfg.local_offset();
    let policies: BTreeMap<String, PolicyRecord> = state
        .store
        .list::<PolicyRecord>(c::POLICIES, By::All)
        .await?
        .into_iter()
        .map(|r| (r.body.policy.customer_id.clone(), r.body))
        .collect();
    let quoted: BTreeSet<String> = state
        .store
        .list::<Quote>(c::QUOTES, By::All)
        .await?
        .into_iter()
        .map(|r| r.body.offer.customer_id)
        .collect();
    let claims: Vec<ClaimRecord> = state
        .store
        .list::<ClaimRecord>(c::CLAIMS, By::All)
        .await?
        .into_iter()
        .map(|r| r.body)
        .collect();
    let periods: Vec<PolicyPeriodRecord> = state
        .store
        .list::<PolicyPeriodRecord>(c::POLICY_PERIODS, By::All)
        .await?
        .into_iter()
        .map(|r| r.body)
        .collect();
    let indemnity = |cover: &str| {
        cfg.cover(cover)
            .is_ok_and(|c| c.cover_type == CoverType::Indemnity)
    };
    let mut customers = Vec::new();
    for r in state
        .store
        .list::<CustomerRecord>(c::CUSTOMERS, By::All)
        .await?
    {
        let id = r.id;
        let policy = policies.get(&id);
        let bound_on = policy.map(|p| p.policy.subscribed_at.with_timezone(&offset).date_naive());
        // Customers are counted by when they joined: quoted or bound in the window.
        let joined =
            bound_on.unwrap_or_else(|| r.body.created_at.with_timezone(&offset).date_naive());
        if joined < from || joined > to {
            continue;
        }
        let tasks = state
            .store
            .list::<TaskRecord>(c::PREVENTION_TASKS, By::Customer(&id))
            .await?;
        let grants = state
            .store
            .list::<Grant>(c::GRANTS, By::Customer(&id))
            .await?;
        let redeemed = state
            .store
            .list::<Redemption>(c::REDEMPTIONS, By::Customer(&id))
            .await?;
        let (loss_periods, loss_claims) = match policy {
            None => (0, 0),
            Some(p) => {
                let recorded: Vec<&PolicyPeriodRecord> = periods
                    .iter()
                    .filter(|pp| pp.policy_id == p.policy.policy_id)
                    .collect();
                let held = if recorded.is_empty() {
                    u32::from(
                        p.policy
                            .schedule
                            .allocations
                            .iter()
                            .any(|a| indemnity(&a.cover_id)),
                    )
                } else {
                    u32::try_from(
                        recorded
                            .iter()
                            .filter(|pp| {
                                pp.schedule
                                    .allocations
                                    .iter()
                                    .any(|a| indemnity(&a.cover_id))
                            })
                            .count(),
                    )
                    .unwrap_or(u32::MAX)
                };
                let n = claims
                    .iter()
                    .filter(|cl| cl.claim.customer_id == id && indemnity(&cl.claim.cover_id))
                    .count();
                (held, u32::try_from(n).unwrap_or(u32::MAX))
            }
        };
        customers.push(PilotCustomer {
            control: bound_on.is_some_and(|d| {
                pilot::cohort(cfg.pilot.as_ref(), &id, d, 1, state.audit_seed) == Cohort::WaitList
            }),
            quoted: quoted.contains(&id),
            bound: policy.is_some(),
            due_to_renew: policy
                .is_some_and(|p| p.policy.period.index > 1 || p.policy.period.ends <= today),
            renewed: policy.is_some_and(|p| p.policy.period.index > 1),
            cancelled: r.body.cancelled.is_some(),
            tasks_offered: u32::try_from(tasks.len()).unwrap_or(u32::MAX),
            tasks_verified: u32::try_from(
                tasks
                    .iter()
                    .filter(|t| matches!(t.body.task.state, TaskState::Verified { .. }))
                    .count(),
            )
            .unwrap_or(u32::MAX),
            loss_periods,
            loss_claims,
            rewards_granted: u32::try_from(grants.len()).unwrap_or(u32::MAX),
            rewards_redeemed: u32::try_from(redeemed.len()).unwrap_or(u32::MAX),
        });
    }
    let payouts: i64 = state
        .store
        .list::<PayoutNotice>(c::PAYOUTS, By::All)
        .await?
        .iter()
        .map(|r| r.body.decision.amount.0)
        .sum();
    let mut claims_decided = 0_u32;
    let mut straight_through = 0_u32;
    let mut claim_paid = 0_i64;
    let mut hours = Vec::new();
    let mut fraud = 0_u32;
    for cl in &claims {
        match &cl.claim.state {
            ClaimState::Approved { by, on, amount } => {
                claims_decided += 1;
                straight_through += u32::from(*by == DecidedBy::Stp);
                claim_paid = claim_paid.saturating_add(amount.0);
                hours.push((*on - cl.claim.notified_at).num_hours());
            }
            ClaimState::Declined { reason, .. } => {
                claims_decided += 1;
                fraud += u32::from(*reason == DeclineReason::Fraud);
            }
            _ => {}
        }
    }
    let mut findings = state.audit_store.audit_findings.clone();
    findings.extend(
        state
            .store
            .list::<AuditFinding>(c::AUDIT_FINDINGS, By::All)
            .await?
            .into_iter()
            .map(|r| r.body),
    );
    let sampled: Vec<&AuditFinding> = findings
        .iter()
        .filter(|f| f.reason == microinsurance_engine::guardrails::audit::AuditReason::RandomSample)
        .collect();
    let decisions = state
        .store
        .list::<serde_json::Value>(c::DECISIONS, By::All)
        .await?
        .len();
    let fairness = snapshot(
        state.clock.now(),
        Vec::new(),
        &AuditStore {
            fairness: state.audit_store.fairness.clone(),
            audit_findings: Vec::new(),
        },
        cfg,
    )
    .map_err(ApiError::internal)?;
    let premium = state
        .ledger
        .balance(Account::CustomerPremiums)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;
    Ok(PilotBook {
        customers,
        premium_minor: premium.0,
        paid_minor: payouts.saturating_add(claim_paid),
        claims_decided,
        claims_straight_through: straight_through,
        claim_payment_hours: hours,
        audits_reviewed: u32::try_from(sampled.len()).unwrap_or(u32::MAX),
        audits_overturned: u32::try_from(sampled.iter().filter(|f| !f.agreed).count())
            .unwrap_or(u32::MAX),
        fraud_confirmed: fraud,
        decisions: u32::try_from(decisions).unwrap_or(u32::MAX),
        fairness_min_ratio_bps: fairness
            .fairness
            .iter()
            .filter_map(|r| r.min_ratio)
            .map(|b| i64::from(b.0))
            .min(),
    })
}
