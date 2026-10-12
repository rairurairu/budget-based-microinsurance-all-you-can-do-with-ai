//! What people at the insurer work on (ADR-010): the one queue, read live from the store, and a
//! person's decision on an item. The engine says which actions each item offers
//! (`guardrails::queue`) and the API refuses any other, so a decline can be made only on a
//! claim referral (R13). The person deciding is the caller, never a field of the request: a
//! decline is reviewed by someone else (R15).
use super::recovery::{ClaimFacts, desk, pay};
use crate::auth::StaffCaller;
use crate::bodies::{
    AuditRecord, ClaimRecord, CustomerRecord, PersonDecision, PolicyRecord, TaskRecord,
};
use crate::error::{ApiError, ApiResult};
use crate::state::{AppState, Inner};
use axum::Json;
use axum::extract::State;
use microinsurance_engine::claims::{ClaimState, PersonOutcome};
use microinsurance_engine::guardrails::appeals::{self, Appeal, AppealState};
use microinsurance_engine::guardrails::audit::AuditFinding;
use microinsurance_engine::guardrails::queue::{
    self, OpsSnapshot, PersonAction, WorkItem, WorkKind,
};
use microinsurance_engine::guardrails::{Actor, DecisionRecord};
use microinsurance_engine::prevention::{self, TaskState};
use microinsurance_store::{By, Keys, Record, collections as c};
use microinsurance_workers::claims::inputs_hash;
use microinsurance_workers::ops::{AuditStore, snapshot};

async fn items(state: &Inner) -> Result<Vec<WorkItem>, ApiError> {
    let now = state.clock.now();
    let mut items = Vec::new();
    for r in state.store.list::<ClaimRecord>(c::CLAIMS, By::All).await? {
        items.extend(queue::claim_item(&r.body.claim));
    }
    for r in state
        .store
        .list::<TaskRecord>(c::PREVENTION_TASKS, By::All)
        .await?
    {
        let since = r.body.with_person_since.unwrap_or(now);
        items.extend(queue::task_item(&r.body.task, since, &state.cfg));
    }
    for r in state.store.list::<AuditRecord>(c::AUDITS, By::All).await? {
        if r.body.open {
            items.push(r.body.item);
        }
    }
    for r in state.store.list::<Appeal>(c::APPEALS, By::All).await? {
        items.extend(queue::appeal_item(&r.body));
    }
    Ok(items)
}

/// The queue in the engine's order (urgent first, then due time), the fairness audit (R19) and
/// the random sample's overturn rate per decision kind (R14).
pub async fn queue_view(
    State(state): State<AppState>,
    _caller: StaffCaller,
) -> ApiResult<OpsSnapshot> {
    let mut findings = state.audit_store.audit_findings.clone();
    findings.extend(
        state
            .store
            .list::<AuditFinding>(c::AUDIT_FINDINGS, By::All)
            .await?
            .into_iter()
            .map(|r| r.body),
    );
    let store = AuditStore {
        fairness: state.audit_store.fairness.clone(),
        audit_findings: findings,
    };
    let items = items(&state).await?;
    snapshot(state.clock.now(), items, &store, &state.cfg)
        .map(Json)
        .map_err(ApiError::internal)
}

fn offered(item: &WorkItem, action: PersonAction) -> Result<(), ApiError> {
    if item.actions.contains(&action) {
        Ok(())
    } else {
        Err(ApiError::rule(
            "R13",
            format!("{action:?} is not an action {} offers", item.item_id),
        ))
    }
}

#[allow(clippy::too_many_arguments)]
fn person_record(
    staff: &str,
    subject_type: &str,
    subject_id: &str,
    customer_id: &str,
    decision: String,
    hash: String,
    note: &str,
    state: &Inner,
) -> DecisionRecord {
    DecisionRecord {
        decision_id: format!("decision/{subject_id}/person"),
        actor: Actor::Person,
        reviewer: Some(staff.to_string()),
        subject_type: subject_type.into(),
        subject_id: subject_id.into(),
        customer_id: customer_id.into(),
        decision,
        inputs_hash: hash,
        reason: note.to_string(),
        at: state.clock.now(),
    }
}

/// A person's decision on a work item, in their own name.
pub async fn decide(
    State(state): State<AppState>,
    caller: StaffCaller,
    Json(d): Json<PersonDecision>,
) -> ApiResult<DecisionRecord> {
    let staff = caller.staff_id.as_str();
    if let Some(r) = state
        .store
        .get::<ClaimRecord>(c::CLAIMS, &d.item_id)
        .await?
        && matches!(r.body.claim.state, ClaimState::WithPerson { .. })
    {
        return claim(&state, staff, r, d).await.map(Json);
    }
    if let Some(r) = state
        .store
        .get::<TaskRecord>(c::PREVENTION_TASKS, &d.item_id)
        .await?
        && matches!(r.body.task.state, TaskState::WithPerson { .. })
    {
        return task(&state, staff, r, d).await.map(Json);
    }
    if let Some(r) = state
        .store
        .get::<AuditRecord>(c::AUDITS, &d.item_id)
        .await?
        && r.body.open
    {
        return audit(&state, staff, r, d).await.map(Json);
    }
    if let Some(r) = state.store.get::<Appeal>(c::APPEALS, &d.item_id).await?
        && r.body.state == AppealState::Open
    {
        return appeal(&state, staff, r, d).await.map(Json);
    }
    Err(ApiError::not_found(format!(
        "{} is not waiting for a person",
        d.item_id
    )))
}

async fn claim(
    state: &Inner,
    staff: &str,
    mut r: Record<ClaimRecord>,
    d: PersonDecision,
) -> Result<DecisionRecord, ApiError> {
    let item =
        queue::claim_item(&r.body.claim).ok_or_else(|| ApiError::conflict("not with a person"))?;
    offered(&item, d.action)?;
    let outcome = match d.action {
        PersonAction::Approve => PersonOutcome::Approve {
            amount: d
                .amount
                .ok_or_else(|| ApiError::bad_request("an approval names the loss assessed"))?,
        },
        PersonAction::Decline => PersonOutcome::Decline {
            reason: d
                .decline_reason
                .ok_or_else(|| ApiError::rule("R13", "a decline names its reason"))?,
        },
        PersonAction::RequestEvidence => PersonOutcome::RequestEvidence {
            kinds: d.evidence_kinds.unwrap_or_default(),
        },
        _ => return Err(ApiError::rule("R13", "not a claim action")),
    };
    let policy = state
        .store
        .get::<PolicyRecord>(c::POLICIES, &r.body.claim.policy_id)
        .await?
        .ok_or_else(|| ApiError::internal("the claim's policy is missing"))?;
    let customer = state
        .store
        .get::<CustomerRecord>(c::CUSTOMERS, &r.body.claim.customer_id)
        .await?
        .ok_or_else(|| ApiError::internal("the claim's customer is missing"))?;
    let facts = ClaimFacts::load(
        state,
        &policy.body,
        &customer.body.profile,
        &r.body.claim.class_of_business,
        Some(&r.id),
    )
    .await?;
    let (claim, payment, mut record) = desk(state)
        .person_decides(
            r.body.claim.clone(),
            staff,
            outcome,
            &facts.ctx(false),
            state.clock.now(),
        )
        .map_err(|e| {
            let rule = if e.contains("R15") { "R15" } else { "R13" };
            ApiError::rule(rule, e)
        })?;
    record.reason = d.note.clone();
    r.body.claim = claim;
    if let Some(p) = payment {
        pay(state, &mut r.body, p, facts.premium_due).await?;
    }
    state.store.update(c::CLAIMS, r).await?;
    super::log(state, record.clone()).await?;
    Ok(record)
}

async fn task(
    state: &Inner,
    staff: &str,
    mut r: Record<TaskRecord>,
    d: PersonDecision,
) -> Result<DecisionRecord, ApiError> {
    let item = queue::task_item(&r.body.task, state.clock.now(), &state.cfg)
        .ok_or_else(|| ApiError::conflict("not with a person"))?;
    offered(&item, d.action)?;
    let verified = d.action == PersonAction::Verify;
    let hash = inputs_hash(&r.body.task);
    let (task, completion) = prevention::person_decides(
        r.body.task.clone(),
        verified,
        &d.note,
        state.today(),
        &state.cfg,
    )?;
    let record = person_record(
        staff,
        "prevention_task",
        &task.task_id,
        &task.customer_id,
        if verified { "verified" } else { "not verified" }.into(),
        hash,
        &d.note,
        state,
    );
    r.body.task = task;
    r.body.with_person_since = None;
    state.store.update(c::PREVENTION_TASKS, r).await?;
    if let Some(done) = completion {
        super::rewards::credit(state, done).await?;
    }
    super::log(state, record.clone()).await?;
    Ok(record)
}

async fn audit(
    state: &Inner,
    staff: &str,
    mut r: Record<AuditRecord>,
    d: PersonDecision,
) -> Result<DecisionRecord, ApiError> {
    offered(&r.body.item, d.action)?;
    let WorkKind::Audit {
        decision_id,
        decision_kind,
        reason,
    } = r.body.item.work.clone()
    else {
        return Err(ApiError::internal("an audit record holds an audit"));
    };
    let agreed = d.action == PersonAction::Agree;
    let finding = AuditFinding {
        decision_id: decision_id.clone(),
        kind: decision_kind.clone(),
        reason,
        reviewer: staff.to_string(),
        agreed,
        note: d.note.clone(),
        on: state.clock.now(),
    };
    let customer = r.body.item.customer_id.clone();
    state
        .store
        .insert(
            c::AUDIT_FINDINGS,
            &r.id.clone(),
            Keys::customer(&customer),
            finding,
        )
        .await?;
    r.body.open = false;
    state.store.update(c::AUDITS, r).await?;
    // An overturned payout is a finding for people: recovery is their decision outside the
    // system, and a payout paid in good faith is not recovered (ADR-010).
    Ok(person_record(
        staff,
        &decision_kind,
        &decision_id,
        &customer,
        if agreed {
            "audit agreed"
        } else {
            "audit overturned"
        }
        .into(),
        String::new(),
        &d.note,
        state,
    ))
}

async fn appeal(
    state: &Inner,
    staff: &str,
    mut r: Record<Appeal>,
    d: PersonDecision,
) -> Result<DecisionRecord, ApiError> {
    let item = queue::appeal_item(&r.body).ok_or_else(|| ApiError::conflict("already decided"))?;
    offered(&item, d.action)?;
    let overturn = d.action == PersonAction::Overturn;
    let decided = appeals::decide(r.body.clone(), staff, overturn, &d.note, state.clock.now())?;
    let record = DecisionRecord {
        decision_id: format!("decision/{}/review", r.body.decision_id),
        ..person_record(
            staff,
            &decided.subject_type,
            &decided.subject_id,
            &decided.customer_id,
            if overturn {
                "review: overturned"
            } else {
                "review: upheld"
            }
            .into(),
            inputs_hash(&r.body),
            &d.note,
            state,
        )
    };
    r.body = decided;
    state.store.update(c::APPEALS, r).await?;
    super::log(state, record.clone()).await?;
    Ok(record)
}
