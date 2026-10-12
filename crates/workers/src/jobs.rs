//! Batch jobs over the book, run from the job queue (`queue`, ADR-012). Each is glue: it reads the
//! policy store, asks the engine for every decision, posts the engine's amounts to the ledger
//! unchanged and writes records once (versioned writes, idempotency keys), so running a job twice
//! changes nothing the second time.
//!
//! Every day, in market local time, `ScheduleDay` enqueues one job per policy or customer due:
//!
//! | Job | When | Does |
//! |---|---|---|
//! | `ProposeRenewal` | from `renewal_notice_days` before a period | the re-rated default and the notice, issued and kept as issued (R34, R35) |
//! | `OpenPolicyPeriod` | the period's first day | the customer's accepted change or the issued default becomes the schedule; the premium is billed (R03, R04, R53); a cancelled customer's policy is not renewed (R41) |
//! | `EraseCancelled` | after cancellation | optional data erased, with a record of completion (R20, R54) |
//! | `ExperienceStudy` | the first of each month | the experience study for the actuary (`learning::experience`) |
//! | `AccumulationSnapshot` | every `accumulation.refresh_minutes` | benefit in force per zone, for the check on new allocation (R51) |
//!
//! Points fade by the date they are read on (R12), so no job rewrites them.
use crate::book::{
    CustomerRecord, ErasureRecord, PayoutNotice, PolicyRecord, RatingAnswers, RenewalRecord,
};
use crate::queue::{Job, NewJob, Queue};
use crate::rating::Rater;
use crate::renewal;
use chrono::{DateTime, Datelike, Days, NaiveDate, Utc};
use microinsurance_engine::EngineError;
use microinsurance_engine::claims::{Claim, ClaimState};
use microinsurance_engine::config::EngineConfig;
use microinsurance_engine::consent::{self, Endorsement, PremiumStatus};
use microinsurance_engine::guardrails::{Actor, DecisionRecord};
use microinsurance_engine::optimiser::rating::{self, RatingBasis};
use microinsurance_engine::score::VerifiedActivity;
use microinsurance_engine::triggers::Occurrence;
use microinsurance_engine::types::{CoverType, PolicyId, PolicyPeriod, Schedule};
use microinsurance_learning::experience::{self, ExperienceStudy, ExposureFacts, PreventionFacts};
use microinsurance_ledger::LedgerStore;
use microinsurance_store::{By, Keys, Record, Store, StoreError, collections as c};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

/// Why a job failed; the queue retries it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JobError(pub String);

impl std::fmt::Display for JobError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl From<StoreError> for JobError {
    fn from(e: StoreError) -> JobError {
        JobError(e.to_string())
    }
}

impl From<EngineError> for JobError {
    fn from(e: EngineError) -> JobError {
        JobError(e.to_string())
    }
}

impl From<String> for JobError {
    fn from(e: String) -> JobError {
        JobError(e)
    }
}

/// One job's work, as its payload.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Work {
    ScheduleDay {
        date: NaiveDate,
    },
    ProposeRenewal {
        policy_id: PolicyId,
        period_index: u32,
    },
    OpenPolicyPeriod {
        policy_id: PolicyId,
        period_index: u32,
    },
    EraseCancelled {
        customer_id: String,
    },
    /// `month`: `YYYY-MM`, the last month the study counts.
    ExperienceStudy {
        month: String,
    },
    /// Benefit in force per zone, measured for the accumulation control (R51); `at` names the
    /// `refresh_minutes` interval, so each is measured once.
    AccumulationSnapshot {
        at: String,
    },
}

impl Work {
    pub fn kind(&self) -> &'static str {
        match self {
            Work::ScheduleDay { .. } => "schedule_day",
            Work::ProposeRenewal { .. } => "propose_renewal",
            Work::OpenPolicyPeriod { .. } => "open_policy_period",
            Work::EraseCancelled { .. } => "erase_cancelled",
            Work::ExperienceStudy { .. } => "experience_study",
            Work::AccumulationSnapshot { .. } => "accumulation_snapshot",
        }
    }

    /// Unique per piece of work: the queue adds a key once.
    pub fn key(&self) -> String {
        match self {
            Work::ScheduleDay { date } => format!("schedule_day/{date}"),
            Work::ProposeRenewal {
                policy_id,
                period_index,
            } => format!("propose_renewal/{policy_id}/{period_index}"),
            Work::OpenPolicyPeriod {
                policy_id,
                period_index,
            } => format!("open_policy_period/{policy_id}/{period_index}"),
            Work::EraseCancelled { customer_id } => format!("erase_cancelled/{customer_id}"),
            Work::ExperienceStudy { month } => format!("experience_study/{month}"),
            Work::AccumulationSnapshot { at } => format!("accumulation_snapshot/{at}"),
        }
    }

    pub fn job(&self, run_at: DateTime<Utc>) -> Result<NewJob, JobError> {
        Ok(NewJob {
            kind: self.kind().to_string(),
            key: self.key(),
            payload: serde_json::to_value(self).map_err(|e| JobError(e.to_string()))?,
            run_at,
        })
    }
}

/// This interval's accumulation snapshot (R51): every worker enqueues it, it is added once.
pub fn accumulation(now: DateTime<Utc>, cfg: &EngineConfig) -> Work {
    let every = i64::from(cfg.accumulation.refresh_minutes).max(1) * 60;
    let start = now.timestamp() - now.timestamp().rem_euclid(every);
    Work::AccumulationSnapshot {
        at: DateTime::from_timestamp(start, 0)
            .unwrap_or(now)
            .format("%Y%m%dT%H%MZ")
            .to_string(),
    }
}

/// Today's `ScheduleDay`, in market local time; enqueued by every worker, added once.
pub fn day(now: DateTime<Utc>, cfg: &EngineConfig) -> Work {
    Work::ScheduleDay {
        date: now.with_timezone(&cfg.local_offset()).date_naive(),
    }
}

/// A policy period as it opened: the exposure an experience study counts.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct PolicyPeriodRecord {
    pub policy_id: PolicyId,
    pub customer_id: String,
    pub period: PolicyPeriod,
    pub schedule: Schedule,
}

impl PolicyPeriodRecord {
    pub fn id(policy_id: &str, index: u32) -> String {
        format!("{policy_id}/{index}")
    }
}

/// What the jobs need besides the queue.
pub struct Batch<'a, L: LedgerStore> {
    pub store: &'a Store,
    pub ledger: &'a L,
    pub cfg: &'a EngineConfig,
    /// The rating basis in force (`python/economics`, the insurer's filed rates).
    pub basis: &'a RatingBasis,
    /// Event types and stages in force now, from the trigger desks (R06).
    pub in_force: &'a BTreeSet<Occurrence>,
    /// Claims for full credibility in an experience study.
    pub full_credibility_claims: u32,
}

fn added<T>(r: Result<T, StoreError>) -> Result<(), JobError> {
    match r {
        Ok(_) | Err(StoreError::Conflict { .. }) => Ok(()),
        Err(e) => Err(e.into()),
    }
}

impl<L: LedgerStore> Batch<'_, L> {
    fn today(&self, now: DateTime<Utc>) -> NaiveDate {
        now.with_timezone(&self.cfg.local_offset()).date_naive()
    }

    /// Runs one job; what it did, in a line.
    pub async fn run(
        &self,
        work: &Work,
        queue: &Queue,
        now: DateTime<Utc>,
    ) -> Result<String, JobError> {
        match work {
            Work::ScheduleDay { date } => self.schedule_day(*date, queue, now).await,
            Work::ProposeRenewal {
                policy_id,
                period_index,
            } => self.propose_renewal(policy_id, *period_index, now).await,
            Work::OpenPolicyPeriod {
                policy_id,
                period_index,
            } => self.open_policy_period(policy_id, *period_index, now).await,
            Work::EraseCancelled { customer_id } => self.erase_cancelled(customer_id, now).await,
            Work::ExperienceStudy { month } => self.experience_study(month).await,
            Work::AccumulationSnapshot { at } => {
                let full = crate::rating::snapshot::<JobError>(self.store, self.cfg, now).await?;
                Ok(format!("accumulation at {at}: {full} zones over the limit"))
            }
        }
    }

    async fn customers(&self) -> Result<BTreeMap<String, CustomerRecord>, JobError> {
        Ok(self
            .store
            .list::<CustomerRecord>(c::CUSTOMERS, By::All)
            .await?
            .into_iter()
            .map(|r| (r.id, r.body))
            .collect())
    }

    /// Enqueues the day's jobs over the book.
    async fn schedule_day(
        &self,
        date: NaiveDate,
        queue: &Queue,
        now: DateTime<Utc>,
    ) -> Result<String, JobError> {
        let customers = self.customers().await?;
        let notice_days = Days::new(u64::from(self.cfg.policy_period.renewal_notice_days));
        let mut enqueued = Vec::new();
        for r in self
            .store
            .list::<PolicyRecord>(c::POLICIES, By::All)
            .await?
        {
            let p = &r.body.policy;
            // The period in force is on record for experience studies.
            added(
                self.store
                    .insert(
                        c::POLICY_PERIODS,
                        &PolicyPeriodRecord::id(&p.policy_id, p.period.index),
                        Keys::customer(&p.customer_id),
                        PolicyPeriodRecord {
                            policy_id: p.policy_id.clone(),
                            customer_id: p.customer_id.clone(),
                            period: p.period,
                            schedule: p.schedule.clone(),
                        },
                    )
                    .await,
            )?;
            if customers
                .get(&p.customer_id)
                .is_some_and(|c| c.cancelled.is_some())
            {
                continue;
            }
            let next = renewal::next_period(&p.period);
            let notice_from = next
                .starts
                .checked_sub_days(notice_days)
                .unwrap_or(next.starts);
            let mut due = Vec::new();
            if notice_from <= date && date < next.starts {
                due.push(Work::ProposeRenewal {
                    policy_id: p.policy_id.clone(),
                    period_index: next.index,
                });
            }
            let lapsed = matches!(r.body.premium(date, self.cfg), PremiumStatus::Lapsed { .. });
            if p.period.ends <= date && !lapsed {
                due.push(Work::OpenPolicyPeriod {
                    policy_id: p.policy_id.clone(),
                    period_index: next.index,
                });
            }
            for w in due {
                if queue.enqueue(w.job(now)?).await? {
                    enqueued.push(w.key());
                }
            }
        }
        for (id, customer) in &customers {
            if customer.cancelled.is_some()
                && self
                    .store
                    .get::<ErasureRecord>(c::ERASURES, id)
                    .await?
                    .is_none()
            {
                let w = Work::EraseCancelled {
                    customer_id: id.clone(),
                };
                if queue.enqueue(w.job(now)?).await? {
                    enqueued.push(w.key());
                }
            }
        }
        if date.day() == 1 {
            let last = date.pred_opt().unwrap_or(date);
            let w = Work::ExperienceStudy {
                month: last.format("%Y-%m").to_string(),
            };
            if queue.enqueue(w.job(now)?).await? {
                enqueued.push(w.key());
            }
        }
        Ok(format!("{date}: {} jobs enqueued", enqueued.len()))
    }

    async fn policy(&self, policy_id: &str) -> Result<Record<PolicyRecord>, JobError> {
        self.store
            .get::<PolicyRecord>(c::POLICIES, policy_id)
            .await?
            .ok_or_else(|| JobError(format!("no policy {policy_id}")))
    }

    /// `ProposeRenewal`: the next period's default, re-rated, and its notice, kept as issued.
    async fn propose_renewal(
        &self,
        policy_id: &str,
        period_index: u32,
        now: DateTime<Utc>,
    ) -> Result<String, JobError> {
        let id = RenewalRecord::id(policy_id, period_index);
        if self
            .store
            .get::<RenewalRecord>(c::RENEWALS, &id)
            .await?
            .is_some()
        {
            return Ok(format!("{id}: already issued"));
        }
        let record = self.policy(policy_id).await?;
        let held = &record.body.policy;
        let next = renewal::next_period(&held.period);
        if next.index != period_index {
            return Ok(format!(
                "{id}: the policy is in period {}",
                held.period.index
            ));
        }
        let customer = self
            .store
            .get::<CustomerRecord>(c::CUSTOMERS, &held.customer_id)
            .await?
            .ok_or_else(|| JobError(format!("no customer {}", held.customer_id)))?;
        if customer.body.cancelled.is_some() {
            return Ok(format!("{id}: cancelled; not renewed (R41)"));
        }
        let profile = customer.body.profile;
        let rated = Rater {
            store: self.store,
            cfg: self.cfg,
            basis: self.basis,
            now,
        }
        .rate::<JobError>(
            &held.plan,
            &profile,
            Some(held),
            next,
            self.in_force.clone(),
        )
        .await?;
        let proposal = renewal::propose(held, &profile, &rated.ctx(self.cfg))?;
        let issued = RenewalRecord {
            policy_id: policy_id.to_string(),
            period: next,
            default: proposal.default.schedule,
            notice: proposal.notice,
            issued_on: self.today(now),
        };
        added(
            self.store
                .insert(c::RENEWALS, &id, Keys::customer(&held.customer_id), issued)
                .await,
        )?;
        Ok(format!("{id}: renewal notice issued"))
    }

    /// `OpenPolicyPeriod`: the next period opens on the customer's change or the issued default.
    async fn open_policy_period(
        &self,
        policy_id: &str,
        period_index: u32,
        now: DateTime<Utc>,
    ) -> Result<String, JobError> {
        let record = self.policy(policy_id).await?;
        let held = record.body.policy.clone();
        let tag = format!("{policy_id}/{period_index}");
        if held.period.index >= period_index {
            return Ok(format!("{tag}: already open"));
        }
        if held.period.index + 1 != period_index {
            return Err(JobError(format!(
                "{tag}: period {} opens first",
                held.period.index + 1
            )));
        }
        let today = self.today(now);
        if today < held.period.ends {
            return Err(JobError(format!(
                "{tag}: not due until {}",
                held.period.ends
            )));
        }
        let cancelled = self
            .store
            .get::<CustomerRecord>(c::CUSTOMERS, &held.customer_id)
            .await?
            .is_some_and(|c| c.body.cancelled.is_some());
        if cancelled {
            return Ok(format!("{tag}: cancelled; not renewed (R41)"));
        }
        if let PremiumStatus::Lapsed { .. } = record.body.premium(today, self.cfg) {
            return Ok(format!("{tag}: lapsed; not renewed (R53)"));
        }
        let next = renewal::next_period(&held.period);
        let issued = self
            .store
            .get::<RenewalRecord>(c::RENEWALS, &RenewalRecord::id(policy_id, period_index))
            .await?
            .map(|r| r.body)
            .filter(|r| consent::check_notice_served(r.issued_on, next.starts, self.cfg).is_ok());
        let (default, basis) = match &issued {
            Some(r) => (r.default.clone(), "renewed as notified (R35)"),
            None => (
                held.schedule.clone(),
                "renewed unchanged: no renewal notice in time (R35)",
            ),
        };
        let update = self
            .store
            .list::<Endorsement>(c::ENDORSEMENTS, By::Customer(&held.customer_id))
            .await?
            .into_iter()
            .map(|r| r.body)
            .find(|e| e.policy_id == policy_id && e.effective_period == period_index);
        let (renewed, (key, entries)) = renewal::open_period(&held, &default, update.as_ref());
        // Billed before the record moves on: a retry posts under the same key (R27).
        self.ledger
            .post(&key, &entries)
            .await
            .map_err(|e| JobError(e.to_string()))?;
        let schedule = renewed.schedule.clone();
        let mut next_record = record;
        next_record.body.policy = renewed;
        next_record.body.premium_due = next.starts;
        next_record.body.premium_paid_on = None;
        self.store.update(c::POLICIES, next_record).await?;
        let keys = Keys::customer(&held.customer_id);
        added(
            self.store
                .insert(
                    c::POLICY_PERIODS,
                    &PolicyPeriodRecord::id(policy_id, period_index),
                    keys.clone(),
                    PolicyPeriodRecord {
                        policy_id: policy_id.to_string(),
                        customer_id: held.customer_id.clone(),
                        period: next,
                        schedule,
                    },
                )
                .await,
        )?;
        let decision = DecisionRecord {
            decision_id: format!("decision/{policy_id}/renewed/{period_index}"),
            actor: if update.is_some() {
                Actor::Customer
            } else {
                Actor::Rules
            },
            reviewer: None,
            subject_type: "policy".into(),
            subject_id: policy_id.to_string(),
            customer_id: held.customer_id.clone(),
            decision: "renewed".into(),
            inputs_hash: String::new(),
            reason: if update.is_some() {
                "renewed on the customer's accepted change (R03, R04)".into()
            } else {
                basis.into()
            },
            at: now,
        };
        added(
            self.store
                .insert(c::DECISIONS, &decision.decision_id.clone(), keys, decision)
                .await,
        )?;
        Ok(format!("{tag}: opened from {}", next.starts))
    }

    /// `EraseCancelled`: the optional data of a cancelled customer, erased once, with a record.
    async fn erase_cancelled(
        &self,
        customer_id: &str,
        now: DateTime<Utc>,
    ) -> Result<String, JobError> {
        let Some(customer) = self
            .store
            .get::<CustomerRecord>(c::CUSTOMERS, customer_id)
            .await?
        else {
            return Ok(format!("{customer_id}: no such customer"));
        };
        let Some(cancellation) = customer.body.cancelled else {
            return Ok(format!("{customer_id}: not cancelled"));
        };
        if self
            .store
            .get::<ErasureRecord>(c::ERASURES, customer_id)
            .await?
            .is_some()
        {
            return Ok(format!("{customer_id}: already erased"));
        }
        let mut erased = BTreeSet::new();
        if let Some(mut answers) = self
            .store
            .get::<RatingAnswers>(c::RATING_ANSWERS, customer_id)
            .await?
        {
            erased = consent::answers_to_erase(
                answers.body.answers.keys().map(String::as_str),
                &cancellation,
                self.cfg,
            );
            if !erased.is_empty() {
                answers.body.answers.retain(|f, _| !erased.contains(f));
                self.store.update(c::RATING_ANSWERS, answers).await?;
            }
        }
        let record = ErasureRecord {
            customer_id: customer_id.to_string(),
            data_types: cancellation.delete.clone(),
            rating_factors: erased.clone(),
            erased_at: now,
        };
        added(
            self.store
                .insert(
                    c::ERASURES,
                    customer_id,
                    Keys::customer(customer_id),
                    record,
                )
                .await,
        )?;
        Ok(format!(
            "{customer_id}: erased {} data types and {} rating answers",
            cancellation.delete.len(),
            erased.len()
        ))
    }

    /// `ExperienceStudy`: every recorded policy period up to `month`, by class, and the
    /// prevention measures' claim frequency with and without a verified activity.
    pub async fn experience_study(&self, month: &str) -> Result<String, JobError> {
        let study = self.study(month).await?;
        let rows = study.classes.len();
        added(
            self.store
                .insert(c::EXPERIENCE_STUDIES, month, Keys::default(), study)
                .await,
        )?;
        Ok(format!("{month}: experience study of {rows} class periods"))
    }

    /// The study, without keeping it.
    pub async fn study(&self, month: &str) -> Result<ExperienceStudy, JobError> {
        // The recorded periods, and each policy's current one until the day's job records it.
        let mut recorded: BTreeMap<String, PolicyPeriodRecord> = self
            .store
            .list::<PolicyPeriodRecord>(c::POLICY_PERIODS, By::All)
            .await?
            .into_iter()
            .map(|r| (r.id, r.body))
            .collect();
        for r in self
            .store
            .list::<PolicyRecord>(c::POLICIES, By::All)
            .await?
        {
            let p = r.body.policy;
            recorded
                .entry(PolicyPeriodRecord::id(&p.policy_id, p.period.index))
                .or_insert(PolicyPeriodRecord {
                    policy_id: p.policy_id,
                    customer_id: p.customer_id,
                    period: p.period,
                    schedule: p.schedule,
                });
        }
        let periods: Vec<PolicyPeriodRecord> = recorded
            .into_values()
            .filter(|p| p.period.starts.format("%Y-%m").to_string().as_str() <= month)
            .collect();
        let payouts: Vec<PayoutNotice> = self
            .store
            .list::<PayoutNotice>(c::PAYOUTS, By::All)
            .await?
            .into_iter()
            .map(|r| r.body)
            .collect();
        let claims: Vec<Claim> = self
            .store
            .list::<StoredClaim>(c::CLAIMS, By::All)
            .await?
            .into_iter()
            .map(|r| r.body.claim)
            .collect();
        let activities: BTreeMap<String, Vec<VerifiedActivity>> = {
            let mut by: BTreeMap<String, Vec<VerifiedActivity>> = BTreeMap::new();
            for r in self
                .store
                .list::<VerifiedActivity>(c::ACTIVITIES, By::All)
                .await?
            {
                by.entry(r.keys.customer_id.unwrap_or_default())
                    .or_default()
                    .push(r.body);
            }
            by
        };
        let offset = self.cfg.local_offset();
        let in_period = |p: &PolicyPeriod, at: DateTime<Utc>| {
            let day = at.with_timezone(&offset).date_naive();
            p.starts <= day && day < p.ends
        };
        let mut exposure = Vec::new();
        let mut prevention = Vec::new();
        for pp in &periods {
            let month_of = pp.period.starts.format("%Y-%m").to_string();
            for a in pp
                .schedule
                .allocations
                .iter()
                .filter(|a| a.sum_insured.0 > 0)
            {
                let Ok(cover) = self.cfg.cover(&a.cover_id) else {
                    continue;
                };
                let expected = self
                    .basis
                    .expected_cost
                    .get(&a.cover_id)
                    .and_then(|curve| {
                        rating::horizon_cost(&a.cover_id, curve, pp.period.starts, 1).ok()
                    })
                    .map_or(0, |(own, _)| {
                        i64::try_from(i128::from(a.sum_insured.0) * i128::from(own.0) / 1_000_000)
                            .unwrap_or(i64::MAX)
                    });
                let paid: Vec<&PayoutNotice> = payouts
                    .iter()
                    .filter(|n| {
                        n.decision.policy_id == pp.policy_id && in_period(&pp.period, n.decided_at)
                    })
                    .filter(|n| n.decision.lines.iter().any(|l| l.cover_id == a.cover_id))
                    .collect();
                let occurrences: BTreeSet<&str> = paid
                    .iter()
                    .map(|n| n.decision.occurrence_id.as_str())
                    .collect();
                let payout_amount: i64 = paid
                    .iter()
                    .flat_map(|n| n.decision.lines.iter())
                    .filter(|l| l.cover_id == a.cover_id)
                    .map(|l| l.amount.0)
                    .sum();
                let cover_claims: Vec<&Claim> = claims
                    .iter()
                    .filter(|cl| {
                        cl.policy_id == pp.policy_id
                            && cl.cover_id == a.cover_id
                            && pp.period.starts <= cl.loss_date
                            && cl.loss_date < pp.period.ends
                    })
                    .collect();
                let claim_amount: i64 = cover_claims
                    .iter()
                    .filter_map(|cl| match &cl.state {
                        ClaimState::Approved { amount, .. } => Some(amount.0),
                        _ => None,
                    })
                    .sum();
                let count =
                    u32::try_from(occurrences.len() + cover_claims.len()).unwrap_or(u32::MAX);
                exposure.push(ExposureFacts {
                    class_of_business: cover.class_of_business.clone(),
                    period: month_of.clone(),
                    premium_minor: a.premium.0,
                    sum_insured_minor: a.sum_insured.0,
                    expected_minor: expected,
                    claims: count,
                    incurred_minor: payout_amount.saturating_add(claim_amount),
                });
                if cover.cover_type == CoverType::Indemnity {
                    let acts = activities.get(&pp.customer_id);
                    for measure in self.cfg.prevention.for_class(&cover.class_of_business) {
                        let verified = acts.is_some_and(|list| {
                            list.iter().any(|v| {
                                v.task_type() == measure.id
                                    && pp.period.starts <= v.on()
                                    && v.on() < pp.period.ends
                            })
                        });
                        prevention.push(PreventionFacts {
                            class_of_business: cover.class_of_business.clone(),
                            measure: measure.id.clone(),
                            verified,
                            claims: u32::try_from(cover_claims.len()).unwrap_or(u32::MAX),
                        });
                    }
                }
            }
        }
        Ok(ExperienceStudy {
            as_of: month.to_string(),
            full_credibility_claims: self.full_credibility_claims,
            classes: experience::study(&exposure, self.full_credibility_claims),
            prevention: experience::prevention_study(&prevention, self.full_credibility_claims),
        })
    }
}

/// A claim as the store keeps it: only the claim is read here.
#[derive(Deserialize)]
struct StoredClaim {
    claim: Claim,
}

/// One job run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ran {
    pub key: String,
    pub outcome: Result<String, String>,
}

/// Claims up to `limit` due jobs for `worker` and runs each, completing it or failing it for a
/// retry.
pub async fn run_due<L: LedgerStore>(
    queue: &Queue,
    worker: &str,
    batch: &Batch<'_, L>,
    now: DateTime<Utc>,
    limit: u32,
) -> Result<Vec<Ran>, String> {
    let mut out = Vec::new();
    for job in queue.claim(worker, limit, now).await? {
        let outcome = match parse(&job) {
            Ok(work) => batch.run(&work, queue, now).await,
            Err(e) => Err(e),
        };
        match &outcome {
            Ok(_) => queue.complete(&job, worker, now).await?,
            Err(e) => queue.fail(&job, worker, &e.0, now).await?,
        }
        out.push(Ran {
            key: job.key,
            outcome: outcome.map_err(|e| e.0),
        });
    }
    Ok(out)
}

fn parse(job: &Job) -> Result<Work, JobError> {
    serde_json::from_value(job.payload.clone())
        .map_err(|e| JobError(format!("{}: payload: {e}", job.key)))
}
