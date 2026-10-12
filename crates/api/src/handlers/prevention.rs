//! Prevention tasks (ADR-009): the AI proposes one of the insurer's registered measures for the
//! customer's upcoming event and the engine sizes the reward (R18, R40); the customer accepts and
//! sends evidence; the verifier's verdict, a person, or the 1-in-20 sample decides before
//! anything is credited (R10, R14). Points fade (R12) and stay in their points pool (R36).
use super::{activities, customer, require_policy, task};
use crate::auth::CustomerCaller;
use crate::bodies::{PoolScore, ScoreView, TaskOutcome, TaskRecord};
use crate::error::{ApiError, ApiResult};
use crate::state::AppState;
use axum::Json;
use axum::extract::{Path, State};
use microinsurance_engine::guardrails::{Actor, DecisionRecord};
use microinsurance_engine::money::Bps;
use microinsurance_engine::pilot;
use microinsurance_engine::prevention::{self, HeldCover, PreventionTask, TaskState};
use microinsurance_engine::score;
use microinsurance_engine::types::CoverType;
use microinsurance_store::{By, Keys, Record, StoreError, collections as c};
use microinsurance_workers::prevention::{PreventionDesk, TaskRequest};

fn desk(state: &AppState) -> PreventionDesk<'_, microinsurance_ai::client::AnyModel> {
    PreventionDesk {
        ai: &state.ai,
        vocab: &state.vocab,
        cfg: &state.cfg,
        basis: &state.prevention,
        seed: state.audit_seed,
    }
}

/// The customer's tasks, newest offer first; tasks past their due date without evidence expire.
pub async fn list(
    State(state): State<AppState>,
    caller: CustomerCaller,
) -> ApiResult<Vec<PreventionTask>> {
    let today = state.today();
    let mut out = Vec::new();
    for r in state
        .store
        .list::<TaskRecord>(c::PREVENTION_TASKS, By::Customer(&caller.customer_id))
        .await?
    {
        let expired = prevention::expire(r.body.task.clone(), today);
        let r = if expired.state == r.body.task.state {
            r
        } else {
            let mut r = r;
            r.body.task = expired;
            state.store.update(c::PREVENTION_TASKS, r).await?
        };
        out.push(r.body.task);
    }
    out.sort_by(|a, b| {
        b.offered_on
            .cmp(&a.offered_on)
            .then(a.task_id.cmp(&b.task_id))
    });
    Ok(Json(out))
}

/// `RefreshPreventionTasks` for one customer: one task per upcoming event and loss-based cover
/// they hold, where the AI proposes a registered measure and the engine admits it (R18, R38).
pub async fn refresh(
    State(state): State<AppState>,
    caller: CustomerCaller,
) -> ApiResult<Vec<PreventionTask>> {
    let record = require_policy(&state, &caller.customer_id).await?;
    let c = customer(&state, &caller.customer_id, caller.partner_id.as_deref()).await?;
    let profile = c.body.profile;
    let held = &record.body.policy;
    let today = state.today();
    let history = activities(&state, &caller.customer_id).await?;
    let rated = super::rated(&state, &held.plan, &profile, Some(held), held.period).await?;
    // R59: a customer on a pilot's wait list is offered no task yet.
    let bound_on = held
        .subscribed_at
        .with_timezone(&state.cfg.local_offset())
        .date_naive();
    let cohort = pilot::cohort(
        state.cfg.pilot.as_ref(),
        &caller.customer_id,
        bound_on,
        held.period.index,
        state.audit_seed,
    );
    let desk = desk(&state);
    let mut offered = Vec::new();
    for event in profile.upcoming_events.iter().filter(|e| e.starts >= today) {
        for a in &held.schedule.allocations {
            let indemnity = state
                .cfg
                .cover(&a.cover_id)
                .is_ok_and(|cv| cv.cover_type == CoverType::Indemnity);
            if !indemnity {
                continue;
            }
            let relativity = rated
                .ratings
                .get(&a.cover_id)
                .map_or(Bps::ONE, |r| r.relativity);
            let request = TaskRequest {
                profile: &profile,
                policy_id: &held.policy_id,
                event,
                held: HeldCover {
                    cover_id: &a.cover_id,
                    sum_insured: a.sum_insured,
                    relativity,
                },
                offered_on: today,
                history: &history,
                cohort,
            };
            let Ok(task) = desk.propose(&request).await else {
                continue;
            };
            let keys = Keys::customer(&caller.customer_id);
            let record = TaskRecord {
                task: task.clone(),
                with_person_since: None,
            };
            match state
                .store
                .insert(c::PREVENTION_TASKS, &task.task_id.clone(), keys, record)
                .await
            {
                Ok(_) => offered.push(task),
                // Offered before for this event: one task per customer, measure and event.
                Err(StoreError::Conflict { .. }) => {}
                Err(e) => return Err(e.into()),
            }
        }
    }
    Ok(Json(offered))
}

/// The customer's tap to take a task on (R03).
pub async fn accept(
    State(state): State<AppState>,
    caller: CustomerCaller,
    Path(task_id): Path<String>,
) -> ApiResult<PreventionTask> {
    let mut r = task(&state, &caller.customer_id, &task_id).await?;
    r.body.task = prevention::accept(r.body.task, state.today())?;
    let r = state.store.update(c::PREVENTION_TASKS, r).await?;
    Ok(Json(r.body.task))
}

/// `CheckEvidence`: fingerprints, then the verifier; a pass is credited unless drawn for a
/// person first (R14); reuse goes to a person (R10).
pub async fn evidence(
    State(state): State<AppState>,
    caller: CustomerCaller,
    Path(task_id): Path<String>,
    Json(upload): Json<crate::bodies::EvidenceUpload>,
) -> ApiResult<TaskOutcome> {
    let r = task(&state, &caller.customer_id, &task_id).await?;
    let files = super::evidence(&upload.files)?;
    let seen = super::seen(&state, &files).await?;
    let c = customer(&state, &caller.customer_id, caller.partner_id.as_deref()).await?;
    let now = state.clock.now();
    let (checked, completion) = desk(&state)
        .check(
            r.body.task.clone(),
            &files,
            &seen,
            c.body.profile.language.as_deref(),
            state.today(),
        )
        .await
        .map_err(ApiError::conflict)?;
    super::remember(&state, &caller.customer_id, &task_id, &files).await?;
    let with_person = matches!(checked.state, TaskState::WithPerson { .. });
    let r = state
        .store
        .update(
            c::PREVENTION_TASKS,
            Record {
                body: TaskRecord {
                    task: checked.clone(),
                    with_person_since: with_person.then_some(now),
                },
                ..r
            },
        )
        .await?;
    if let TaskState::NotVerified { reason, .. } = &checked.state {
        super::log(
            &state,
            DecisionRecord {
                decision_id: format!("decision/{task_id}/verify"),
                actor: Actor::Ai,
                reviewer: None,
                subject_type: "prevention_task".into(),
                subject_id: task_id.clone(),
                customer_id: caller.customer_id.clone(),
                decision: "not verified".into(),
                inputs_hash: microinsurance_workers::claims::inputs_hash(&checked),
                reason: reason.clone(),
                at: now,
            },
        )
        .await?;
    }
    let granted = match completion {
        Some(done) => Some(super::rewards::credit(&state, done).await?),
        None => None,
    };
    Ok(Json(TaskOutcome {
        task: r.body.task,
        granted,
    }))
}

/// R15: a task not verified goes to a person on the customer's request.
pub async fn review_request(
    State(state): State<AppState>,
    caller: CustomerCaller,
    Path(task_id): Path<String>,
) -> ApiResult<PreventionTask> {
    let mut r = task(&state, &caller.customer_id, &task_id).await?;
    r.body.task = prevention::appeal(r.body.task, state.today())?;
    r.body.with_person_since = Some(state.clock.now());
    let r = state.store.update(c::PREVENTION_TASKS, r).await?;
    Ok(Json(r.body.task))
}

/// Points by points pool after fading (R12, R36) and the effect each earns (R11).
pub async fn score(State(state): State<AppState>, caller: CustomerCaller) -> ApiResult<ScoreView> {
    let history = activities(&state, &caller.customer_id).await?;
    let pools = score::points_by_pool(&history, state.today(), &[], &state.cfg)
        .into_iter()
        .map(|(pool, points)| PoolScore {
            effect: score::effect(points, &state.cfg.score),
            pool,
            points,
        })
        .collect();
    Ok(Json(ScoreView { pools }))
}
