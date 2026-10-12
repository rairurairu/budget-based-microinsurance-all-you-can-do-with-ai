//! The book in the policy store (ADR-011): the records the API keeps and the workers read and
//! write. `PollTriggers` reads every policy from the store as its book, and records what each
//! poll decided where the customer sees it: the payout (streamed to them by the API), the
//! decision in the log (so they can ask a person to review it, R15) and the payouts drawn for
//! audit after payment (R14). The API's webhook for pushed reports records its polls the same way.
use crate::desk::PollOutcome;
use chrono::{DateTime, NaiveDate, Utc};
use microinsurance_engine::config::EngineConfig;
use microinsurance_engine::consent::{self, CancellationData, PremiumStatus, RenewalNotice};
use microinsurance_engine::guardrails::queue::{self, WorkItem};
use microinsurance_engine::guardrails::{Actor, DecisionRecord, audit};
use microinsurance_engine::money::Money;
use microinsurance_engine::payouts::{PayoutDecision, Settled};
use microinsurance_engine::types::{PartnerId, Policy, PolicyId, PolicyPeriod, Profile, Schedule};
use microinsurance_store::{By, Keys, Store, StoreError, collections as c};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

/// The policy as the store keeps it.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct PolicyRecord {
    pub policy: Policy,
    pub partner_id: Option<PartnerId>,
    /// The premium for the current period is due on its first day (R53).
    pub premium_due: NaiveDate,
    pub premium_paid_on: Option<NaiveDate>,
    /// The quote that bound it (R49).
    pub quote_id: String,
}

impl PolicyRecord {
    /// R53: the premium status of the current period on `today`.
    pub fn premium(&self, today: NaiveDate, cfg: &EngineConfig) -> PremiumStatus {
        consent::premium_status(self.premium_due, self.premium_paid_on, today, cfg)
    }
}

/// The customer as the store keeps them: their declarations and opt-ins, and how they came.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct CustomerRecord {
    pub profile: Profile,
    /// The partner whose channel they first came through (R52).
    pub partner_id: Option<PartnerId>,
    pub created_at: DateTime<Utc>,
    /// Set on cancellation: optional data deleted, records kept until a date (R54).
    pub cancelled: Option<CancellationData>,
}

/// Answers to rating factors, kept apart from the profile: never an AI input (R19, R33).
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct RatingAnswers {
    pub answers: BTreeMap<String, String>,
}

/// A renewal notice as issued (R35): the next period's default schedule, re-rated, and the
/// notice the customer was sent. Kept as issued, so the period opens on what was shown (R03,
/// R34); filed under `<policy id>/<period index>`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct RenewalRecord {
    pub policy_id: PolicyId,
    pub period: PolicyPeriod,
    /// Applies unless the customer accepts a change for the period (R03, R04).
    pub default: Schedule,
    pub notice: RenewalNotice,
    pub issued_on: NaiveDate,
}

impl RenewalRecord {
    pub fn id(policy_id: &str, period_index: u32) -> String {
        format!("{policy_id}/{period_index}")
    }
}

/// What erasing a cancelled customer's optional data removed, and when (R20, R54): the record
/// of completion.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ErasureRecord {
    pub customer_id: String,
    /// The optional data types erased.
    pub data_types: BTreeSet<String>,
    /// The rating answers erased with them, by factor.
    pub rating_factors: BTreeSet<String>,
    pub erased_at: DateTime<Utc>,
}

/// A parametric payout as the customer sees it.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct PayoutNotice {
    pub decision: PayoutDecision,
    pub customer_id: String,
    pub decided_at: DateTime<Utc>,
    /// None while the customer is choosing between cash and an alternative (R16, R28).
    pub settled: Option<Settled>,
}

/// A decision drawn for audit after the fact, waiting for a person (R14).
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct AuditRecord {
    pub item: WorkItem,
    pub open: bool,
}

/// Every policy in the store, with the premium due in the grace period by policy (R53).
pub struct Loaded {
    pub policies: Vec<Policy>,
    pub premium_due: BTreeMap<PolicyId, Money>,
}

pub async fn load(store: &Store, today: NaiveDate, cfg: &EngineConfig) -> Result<Loaded, String> {
    let records = store
        .list::<PolicyRecord>(c::POLICIES, By::All)
        .await
        .map_err(|e| e.to_string())?;
    let mut premium_due = BTreeMap::new();
    for r in &records {
        if let PremiumStatus::InGrace { .. } = r.body.premium(today, cfg) {
            premium_due.insert(r.id.clone(), r.body.policy.schedule.total_premium());
        }
    }
    Ok(Loaded {
        policies: records.into_iter().map(|r| r.body.policy).collect(),
        premium_due,
    })
}

fn once<T>(r: Result<T, StoreError>) -> Result<(), String> {
    match r {
        Ok(_) | Err(StoreError::Conflict { .. }) => Ok(()),
        Err(e) => Err(e.to_string()),
    }
}

/// Records what a poll decided, once each: the customer's payout notice, the decision log row,
/// and an audit item for each payout drawn for a person after payment (R14, `seed` the audit
/// seed).
pub async fn record(
    store: &Store,
    outcome: &PollOutcome,
    policies: &[Policy],
    now: DateTime<Utc>,
    seed: u64,
    cfg: &EngineConfig,
) -> Result<(), String> {
    let customers: BTreeMap<&str, &str> = policies
        .iter()
        .map(|p| (p.policy_id.as_str(), p.customer_id.as_str()))
        .collect();
    let customer = |d: &PayoutDecision| {
        customers
            .get(d.policy_id.as_str())
            .copied()
            .unwrap_or_default()
            .to_string()
    };
    for d in &outcome.decided {
        let customer_id = customer(d);
        let settled = outcome
            .settled
            .iter()
            .find(|s| s.idempotency_key == format!("{}/settled", d.idempotency_key))
            .cloned();
        let notice = PayoutNotice {
            decision: d.clone(),
            customer_id: customer_id.clone(),
            decided_at: now,
            settled,
        };
        let keys = Keys::customer(&customer_id);
        once(
            store
                .insert(c::PAYOUTS, &d.idempotency_key, keys.clone(), notice)
                .await,
        )?;
        let decision = DecisionRecord {
            decision_id: format!("decision/{}", d.idempotency_key),
            actor: Actor::Rules,
            reviewer: None,
            subject_type: "payout".into(),
            subject_id: d.idempotency_key.clone(),
            customer_id,
            decision: "paid".into(),
            inputs_hash: crate::claims::inputs_hash(d),
            reason: d.occurrence_id.clone(),
            at: now,
        };
        once(
            store
                .insert(c::DECISIONS, &decision.decision_id.clone(), keys, decision)
                .await,
        )?;
    }
    let drawn = audit::sample(
        outcome
            .decided
            .iter()
            .map(|d| (d.idempotency_key.as_str(), None)),
        seed,
        &cfg.guardrails,
    );
    for (key, reason) in drawn {
        let customer_id = outcome
            .decided
            .iter()
            .find(|d| d.idempotency_key == key)
            .map(customer)
            .unwrap_or_default();
        let item = queue::audit_item(&key, "payout", &customer_id, reason, now, cfg);
        let id = item.item_id.clone();
        let record = AuditRecord { item, open: true };
        once(
            store
                .insert(c::AUDITS, &id, Keys::customer(&customer_id), record)
                .await,
        )?;
    }
    Ok(())
}
