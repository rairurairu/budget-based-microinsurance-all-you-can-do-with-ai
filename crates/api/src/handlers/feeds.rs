//! Data sources that push their reports (`POST /webhooks/triggers/{source}`), such as an event
//! organiser's notices (R55). The pushed payload goes through the source's own adapter, then the
//! same trigger desk the workers run: trigger events, loss occurrences, payouts within the event
//! limit (R51), posted once each (R27). Each payout is recorded for the customer and streamed to
//! them, and payouts are sampled for audit after payment (R14), as the workers record theirs
//! (`workers::book`).
use super::suitable;
use crate::auth::FeedCaller;
use crate::bodies::FeedOutcome;
use crate::error::{ApiError, ApiResult};
use crate::state::AppState;
use axum::Json;
use axum::extract::{Path, State};
use microinsurance_engine::triggers;
use microinsurance_triggers::{Feed, TriggerSource};
use microinsurance_workers::book;
use microinsurance_workers::desk::{Book, TriggerDesk};
use microinsurance_workers::sources;

pub async fn push(
    State(state): State<AppState>,
    caller: FeedCaller,
    Path(source): Path<String>,
    body: String,
) -> ApiResult<FeedOutcome> {
    if caller.source != source {
        return Err(ApiError::forbidden(format!(
            "{} may push only its own reports",
            caller.source
        )));
    }
    push_payload(&state, &source, body).await.map(Json)
}

/// One pushed payload of `source`, through its adapter and the trigger desk.
pub async fn push_payload(
    state: &AppState,
    source: &str,
    body: String,
) -> Result<FeedOutcome, ApiError> {
    let adapter = sources::adapter(source, Feed::Inline(body), &state.config, &state.cfg)
        .map_err(|e| ApiError::rule("R55", e))?;
    let reports = adapter
        .poll()
        .await
        .map_err(|e| ApiError::bad_request(e.to_string()))?;
    let loaded = book::load(&state.store, state.today(), &state.cfg)
        .await
        .map_err(ApiError::internal)?;
    let index = triggers::build_index(&loaded.policies, &state.cfg.catalogue);
    let suitable = suitable(state);
    let book = Book {
        cfg: &state.cfg,
        policies: &loaded.policies,
        index: &index,
        premium_due: &loaded.premium_due,
        suitable: &suitable,
    };

    let now = state.clock.now();
    let key = sources::desk_key(source);
    let mut desks = state.desks.lock().await;
    if !desks.contains_key(source) {
        let saved = match &state.worker_state {
            Some(s) => s
                .load::<TriggerDesk>(&key)
                .await
                .map_err(ApiError::internal)?,
            None => None,
        };
        desks.insert(source.to_string(), saved.unwrap_or_default());
    }
    let desk = desks
        .get_mut(source)
        .ok_or_else(|| ApiError::internal("desk"))?;
    let outcome = desk
        .poll(source, &reports, now, &book)
        .map_err(ApiError::internal)?;
    // Saved before posting: whatever was decided is either posted or still in the outbox.
    if let Some(s) = &state.worker_state {
        s.save(&key, &*desk).await.map_err(ApiError::internal)?;
    }
    let failed = desk.flush(&state.ledger).await;
    if let Some(s) = &state.worker_state {
        s.save(&key, &*desk).await.map_err(ApiError::internal)?;
    }
    drop(desks);
    if !failed.is_empty() {
        return Err(ApiError::internal(failed.join("; ")));
    }
    book::record(
        &state.store,
        &outcome,
        &loaded.policies,
        now,
        state.audit_seed,
        &state.cfg,
    )
    .await
    .map_err(ApiError::internal)?;
    Ok(FeedOutcome {
        source: source.to_string(),
        opened: outcome.tracked.opened,
        closed: outcome.tracked.closed,
        payouts: outcome.decided,
    })
}
