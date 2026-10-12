//! The route handlers, one module per area (README.md). Each is glue: it loads records, asks the
//! AI layer for a recommendation where one is needed, asks the engine for every decision, posts
//! the engine's amounts to the ledger unchanged and keeps the records. Nothing here decides an
//! amount.
pub mod adoption;
pub mod catalogue;
pub mod customer;
pub mod feeds;
pub mod ops;
pub mod partners;
pub mod prevention;
pub mod recovery;
pub mod rewards;

use crate::bodies::{CustomerRecord, PolicyRecord, TaskRecord};
use crate::error::ApiError;
use crate::state::{AppState, Inner};
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use chrono::NaiveDate;
use microinsurance_ai::verify::{Evidence, fingerprint};
use microinsurance_engine::consent::{self, PremiumStatus};
use microinsurance_engine::guardrails::DecisionRecord;
use microinsurance_engine::score::VerifiedActivity;
use microinsurance_engine::triggers::{self, Occurrence};
use microinsurance_engine::types::{Plan, Policy, PolicyPeriod, Profile};
use microinsurance_store::{By, Keys, Record, collections as c};
pub use microinsurance_workers::rating::{Rated, Rater};
use std::collections::BTreeSet;

/// The customer's record, created on first use.
pub async fn customer(
    state: &Inner,
    customer_id: &str,
    partner_id: Option<&str>,
) -> Result<Record<CustomerRecord>, ApiError> {
    if let Some(r) = state.store.get(c::CUSTOMERS, customer_id).await? {
        return Ok(r);
    }
    let record = CustomerRecord {
        profile: Profile {
            customer_id: customer_id.to_string(),
            ..Profile::default()
        },
        partner_id: partner_id.map(str::to_string),
        created_at: state.clock.now(),
        cancelled: None,
    };
    let keys = Keys::customer(customer_id).with_partner(partner_id);
    match state
        .store
        .insert(c::CUSTOMERS, customer_id, keys, record)
        .await
    {
        Ok(r) => Ok(r),
        // Created by a request in parallel: read theirs.
        Err(microinsurance_store::StoreError::Conflict { .. }) => state
            .store
            .get(c::CUSTOMERS, customer_id)
            .await?
            .ok_or_else(|| ApiError::internal("customer record vanished")),
        Err(e) => Err(e.into()),
    }
}

/// The customer's policy, if they hold one.
pub async fn policy(
    state: &Inner,
    customer_id: &str,
) -> Result<Option<Record<PolicyRecord>>, ApiError> {
    Ok(state
        .store
        .list::<PolicyRecord>(c::POLICIES, By::Customer(customer_id))
        .await?
        .into_iter()
        .max_by_key(|r| r.body.policy.subscribed_at))
}

pub async fn require_policy(
    state: &Inner,
    customer_id: &str,
) -> Result<Record<PolicyRecord>, ApiError> {
    policy(state, customer_id)
        .await?
        .ok_or_else(|| ApiError::not_found("no policy: quote and accept one first"))
}

/// R53: the premium status of the current period today.
pub fn premium_status(record: &PolicyRecord, today: NaiveDate, state: &Inner) -> PremiumStatus {
    consent::premium_status(
        record.premium_due,
        record.premium_paid_on,
        today,
        &state.cfg,
    )
}

/// Event types and stages in force now, from the trigger desks this process holds (R06).
pub async fn in_force(state: &Inner) -> BTreeSet<Occurrence> {
    state
        .desks
        .lock()
        .await
        .values()
        .flat_map(|d| d.events.values())
        .filter(|e| !e.is_closed())
        .map(triggers::TriggerEvent::occurrence)
        .collect()
}

/// Rates `profile`'s covers on `plan` for `period` (`workers::rating`), with the event types in
/// force on the feeds this process holds (R06).
pub async fn rated(
    state: &Inner,
    plan: &Plan,
    profile: &Profile,
    held: Option<&Policy>,
    period: PolicyPeriod,
) -> Result<Rated, ApiError> {
    Rater {
        store: &state.store,
        cfg: &state.cfg,
        basis: &state.rating,
        now: state.clock.now(),
    }
    .rate(plan, profile, held, period, in_force(state).await)
    .await
}

/// Settlement in kind a registered supplier can deliver now (R28).
pub fn suitable(state: &Inner) -> BTreeSet<String> {
    state.cfg.suppliers.in_kind.keys().cloned().collect()
}

const EVIDENCE_TYPES: &[&str] = &[
    "image/jpeg",
    "image/png",
    "image/gif",
    "image/webp",
    "text/plain",
    "application/json",
];
const MAX_FILES: usize = 6;

/// The uploaded files, decoded.
pub fn evidence(files: &[crate::bodies::EvidenceFile]) -> Result<Vec<Evidence>, ApiError> {
    if files.is_empty() || files.len() > MAX_FILES {
        return Err(ApiError::bad_request(format!(
            "send between 1 and {MAX_FILES} files"
        )));
    }
    files
        .iter()
        .map(|f| {
            if !EVIDENCE_TYPES.contains(&f.media_type.as_str()) {
                return Err(ApiError::bad_request(format!(
                    "{} is not an accepted evidence type",
                    f.media_type
                )));
            }
            let bytes = STANDARD
                .decode(f.data_base64.trim())
                .map_err(|_| ApiError::bad_request("evidence is not valid base64"))?;
            Ok(Evidence {
                media_type: f.media_type.clone(),
                bytes,
            })
        })
        .collect()
}

/// The fingerprints of `files` already used, by anyone (R10, R14).
pub async fn seen(state: &Inner, files: &[Evidence]) -> Result<BTreeSet<String>, ApiError> {
    let mut out = BTreeSet::new();
    for f in files {
        let print = fingerprint(&f.bytes);
        if state
            .store
            .get::<serde_json::Value>(c::EVIDENCE, &print)
            .await?
            .is_some()
        {
            out.insert(print);
        }
    }
    Ok(out)
}

/// Records that `files` were used, against `subject`.
pub async fn remember(
    state: &Inner,
    customer_id: &str,
    subject: &str,
    files: &[Evidence],
) -> Result<Vec<String>, ApiError> {
    let mut prints = Vec::new();
    for f in files {
        let print = fingerprint(&f.bytes);
        let used = serde_json::json!({ "subject": subject, "at": state.clock.now() });
        match state
            .store
            .insert(c::EVIDENCE, &print, Keys::customer(customer_id), used)
            .await
        {
            Ok(_) | Err(microinsurance_store::StoreError::Conflict { .. }) => {}
            Err(e) => return Err(e.into()),
        }
        prints.push(print);
    }
    Ok(prints)
}

/// Appends to the decision log; the latest decision on a subject replaces an earlier one with
/// the same id, which is what a review request reviews.
pub async fn log(state: &Inner, record: DecisionRecord) -> Result<(), ApiError> {
    let keys = Keys::customer(&record.customer_id);
    match state
        .store
        .get::<DecisionRecord>(c::DECISIONS, &record.decision_id)
        .await?
    {
        Some(existing) => {
            state
                .store
                .update(
                    c::DECISIONS,
                    Record {
                        body: record,
                        ..existing
                    },
                )
                .await?;
        }
        None => {
            let id = record.decision_id.clone();
            state.store.insert(c::DECISIONS, &id, keys, record).await?;
        }
    }
    Ok(())
}

/// The customer's verified prevention activities (R10, R11).
pub async fn activities(
    state: &Inner,
    customer_id: &str,
) -> Result<Vec<VerifiedActivity>, ApiError> {
    Ok(state
        .store
        .list::<VerifiedActivity>(c::ACTIVITIES, By::Customer(customer_id))
        .await?
        .into_iter()
        .map(|r| r.body)
        .collect())
}

/// A task of the customer's.
pub async fn task(
    state: &AppState,
    customer_id: &str,
    task_id: &str,
) -> Result<Record<TaskRecord>, ApiError> {
    state
        .store
        .get::<TaskRecord>(c::PREVENTION_TASKS, task_id)
        .await?
        .filter(|r| r.body.task.customer_id == customer_id)
        .ok_or_else(|| ApiError::not_found(format!("no task {task_id}")))
}
