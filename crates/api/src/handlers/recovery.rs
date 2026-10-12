//! Recovery: automatic payouts, shown and streamed as they are decided, with the customer's
//! choice where an alternative is offered (R16, R28); indemnity claims from the notification of
//! loss to payment or a person (R13, R46, R57, ADR-010); and review requests on any decision
//! (R15).
use super::{customer, require_policy, suitable};
use crate::auth::CustomerCaller;
use crate::bodies::{
    AiSource, ClaimEvidence, ClaimRecord, ClaimRequest, GuaranteeRecord, PayoutNotice,
    PolicyRecord, ReviewOpened, ReviewRequest, SettlementChoice,
};
use crate::error::{ApiError, ApiResult};
use crate::state::{AppState, Inner};
use axum::Json;
use axum::extract::{Path, State};
use axum::response::sse::{Event, KeepAlive, Sse};
use microinsurance_engine::claims::{
    self, Claim, ClaimContext, ClaimNotice, ClaimPayment, ClaimState, StpOutcome,
};
use microinsurance_engine::consent::PremiumStatus;
use microinsurance_engine::guardrails::DecisionRecord;
use microinsurance_engine::guardrails::appeals;
use microinsurance_engine::guardrails::human_care;
use microinsurance_engine::money::Money;
use microinsurance_engine::optimiser::underwriting::ClassStatus;
use microinsurance_engine::payouts::{self, Settled, Settlement};
use microinsurance_engine::suppliers;
use microinsurance_engine::types::{Policy, Profile, sub_policy_periods};
use microinsurance_ledger::{LedgerStore, postings};
use microinsurance_store::{By, Keys, Record, collections as c};
use microinsurance_workers::claims::ClaimsDesk;
use std::collections::BTreeSet;
use std::convert::Infallible;
use tokio_stream::StreamExt as _;
use tokio_stream::wrappers::BroadcastStream;

/// The customer's payouts, newest first.
pub async fn payouts(
    State(state): State<AppState>,
    caller: CustomerCaller,
) -> ApiResult<Vec<PayoutNotice>> {
    let mut out: Vec<PayoutNotice> = state
        .store
        .list::<PayoutNotice>(c::PAYOUTS, By::Customer(&caller.customer_id))
        .await?
        .into_iter()
        .map(|r| r.body)
        .collect();
    out.sort_by_key(|a| std::cmp::Reverse(a.decided_at));
    Ok(Json(out))
}

/// Each payout the moment it is decided or settled, as server-sent events named `payout`,
/// from this process or, with PostgreSQL, any worker.
pub async fn payout_stream(
    State(state): State<AppState>,
    caller: CustomerCaller,
) -> Sse<impl tokio_stream::Stream<Item = Result<Event, Infallible>>> {
    let me = caller.customer_id;
    let store = state.store.clone();
    let changes = BroadcastStream::new(state.store.subscribe())
        .filter_map(move |change| {
            change
                .ok()
                .filter(|c| c.collection == c::PAYOUTS && c.customer_id.as_deref() == Some(&me))
        })
        .then(move |change| {
            let store = store.clone();
            async move {
                store
                    .get::<PayoutNotice>(c::PAYOUTS, &change.id)
                    .await
                    .ok()
                    .flatten()
            }
        })
        .filter_map(|notice| {
            notice.and_then(|n| {
                Event::default()
                    .event("payout")
                    .json_data(&n.body)
                    .ok()
                    .map(Ok)
            })
        });
    Sse::new(changes).keep_alive(KeepAlive::default())
}

/// Premium due and unpaid in the grace period, set off against cash (R53).
fn premium_due(record: &PolicyRecord, state: &Inner) -> Money {
    match super::premium_status(record, state.today(), state) {
        PremiumStatus::InGrace { .. } => record.policy.schedule.total_premium(),
        _ => Money::ZERO,
    }
}

/// R16, R28: the customer settles a payout in cash or an alternative it offered.
pub async fn payout_choice(
    State(state): State<AppState>,
    caller: CustomerCaller,
    Path(key): Path<String>,
    Json(choice): Json<SettlementChoice>,
) -> ApiResult<PayoutNotice> {
    let mut notice = state
        .store
        .get::<PayoutNotice>(c::PAYOUTS, &key)
        .await?
        .filter(|r| r.body.customer_id == caller.customer_id)
        .ok_or_else(|| ApiError::not_found(format!("no payout {key}")))?;
    if notice.body.settled.is_some() {
        return Err(ApiError::conflict(format!("{key} is already settled")));
    }
    let policy = require_policy(&state, &caller.customer_id).await?;
    let settled = payouts::settle(
        &notice.body.decision,
        Some(&choice.settlement),
        premium_due(&policy.body, &state),
    )?;
    post_settled(&state, &caller.customer_id, &settled).await?;
    notice.body.settled = Some(settled);
    let notice = state.store.update(c::PAYOUTS, notice).await?;
    Ok(Json(notice.body))
}

/// Posts a settlement and, for one in kind, orders it from the supplier (R56).
async fn post_settled(state: &Inner, customer_id: &str, settled: &Settled) -> Result<(), ApiError> {
    let (key, entries) = postings::payout_settled(settled);
    state.ledger.post(&key, &entries).await?;
    if let Settlement::InKind(_) = settled.method {
        let order = suppliers::order_for_payout(settled, state.today(), &state.cfg)?;
        let supplier = order.supplier_id.clone();
        let keys = Keys::customer(customer_id).with_partner(Some(&supplier));
        match state
            .store
            .insert(c::IN_KIND_ORDERS, &order.order_id.clone(), keys, order)
            .await
        {
            Ok(_) | Err(microinsurance_store::StoreError::Conflict { .. }) => {}
            Err(e) => return Err(e.into()),
        }
    }
    Ok(())
}

/// What the rules know about the customer when a claim is notified or decided.
pub struct ClaimFacts {
    pub policy: Policy,
    pub class_status: ClassStatus,
    pub premium: PremiumStatus,
    pub premium_due: Money,
    pub claims_in_class: u32,
    pub monthly_earnings: Option<Money>,
    pub suitable: BTreeSet<String>,
}

impl ClaimFacts {
    pub async fn load(
        state: &Inner,
        record: &PolicyRecord,
        profile: &Profile,
        class: &str,
        current: Option<&str>,
    ) -> Result<ClaimFacts, ApiError> {
        // Claims in the class within `claims_experience_periods`, other than this one (R14).
        let since = sub_policy_periods(
            state.today(),
            state.cfg.underwriting.claims_experience_periods,
        );
        let claims_in_class = state
            .store
            .list::<ClaimRecord>(c::CLAIMS, By::Customer(&record.policy.customer_id))
            .await?
            .iter()
            .filter(|r| Some(r.id.as_str()) != current)
            .filter(|r| r.body.claim.class_of_business == class && r.body.claim.loss_date >= since)
            .count();
        Ok(ClaimFacts {
            policy: record.policy.clone(),
            class_status: ClassStatus::default(),
            premium: super::premium_status(record, state.today(), state),
            premium_due: premium_due(record, state),
            claims_in_class: u32::try_from(claims_in_class).unwrap_or(u32::MAX),
            monthly_earnings: human_care::monthly_earnings(profile),
            suitable: suitable(state),
        })
    }

    pub fn ctx(&self, evidence_reused: bool) -> ClaimContext<'_> {
        ClaimContext {
            policy: &self.policy,
            class_status: &self.class_status,
            premium: &self.premium,
            claims_in_class: self.claims_in_class.saturating_add(1),
            evidence_reused,
            monthly_earnings: self.monthly_earnings,
            suitable: &self.suitable,
        }
    }
}

pub fn desk(state: &Inner) -> ClaimsDesk<'_, microinsurance_ai::client::AnyModel> {
    ClaimsDesk {
        ai: &state.ai,
        cfg: &state.cfg,
        seed: state.audit_seed,
    }
}

/// Pays an approved claim (R27): the payment is posted once; with cash its only option it is
/// settled at once, net of premium due in the grace period (R53); otherwise the customer
/// chooses (R16, R28).
pub async fn pay(
    state: &Inner,
    record: &mut ClaimRecord,
    payment: ClaimPayment,
    premium_due: Money,
) -> Result<(), ApiError> {
    let (key, entries) = postings::claim_approved(&payment);
    state.ledger.post(&key, &entries).await?;
    if payment.options == [Settlement::Cash] {
        let settled = claims::settle(&payment, None, premium_due)?;
        post_settled(state, &record.claim.customer_id, &settled).await?;
        record.settled = Some(settled);
    }
    record.payment = Some(payment);
    Ok(())
}

async fn claim(
    state: &Inner,
    customer_id: &str,
    claim_id: &str,
) -> Result<Record<ClaimRecord>, ApiError> {
    state
        .store
        .get::<ClaimRecord>(c::CLAIMS, claim_id)
        .await?
        .filter(|r| r.body.claim.customer_id == customer_id)
        .ok_or_else(|| ApiError::not_found(format!("no claim {claim_id}")))
}

pub async fn claims(
    State(state): State<AppState>,
    caller: CustomerCaller,
) -> ApiResult<Vec<ClaimRecord>> {
    let mut out: Vec<ClaimRecord> = state
        .store
        .list::<ClaimRecord>(c::CLAIMS, By::Customer(&caller.customer_id))
        .await?
        .into_iter()
        .map(|r| r.body)
        .collect();
    out.sort_by_key(|a| std::cmp::Reverse(a.claim.notified_at));
    Ok(Json(out))
}

pub async fn get_claim(
    State(state): State<AppState>,
    caller: CustomerCaller,
    Path(claim_id): Path<String>,
) -> ApiResult<ClaimRecord> {
    Ok(Json(
        claim(&state, &caller.customer_id, &claim_id).await?.body,
    ))
}

/// `NotifyClaim`: every notification is registered and triaged at once; an urgent case gets its
/// first response now, before anyone is asked, and a guarantee above the automatic cap goes to
/// the assistance provider to confirm (R13, R46).
pub async fn notify(
    State(state): State<AppState>,
    caller: CustomerCaller,
    Json(req): Json<ClaimRequest>,
) -> ApiResult<ClaimRecord> {
    let policy = require_policy(&state, &caller.customer_id).await?;
    let c = customer(&state, &caller.customer_id, caller.partner_id.as_deref()).await?;
    let cover = state.cfg.cover(&req.cover_id)?;
    let facts = ClaimFacts::load(
        &state,
        &policy.body,
        &c.body.profile,
        &cover.class_of_business,
        None,
    )
    .await?;
    let now = state.clock.now();
    let notice = ClaimNotice {
        claim_id: state.new_id("claim"),
        cover_id: req.cover_id,
        loss_date: req.loss_date,
        notified_at: now,
        amount_claimed: req.amount_claimed,
        circumstance: req.circumstance,
        asked_for_person: req.asked_for_person,
        guarantee_requested: req.guarantee_requested.unwrap_or(Money::ZERO),
    };
    let (claim, triage) = desk(&state)
        .notify(notice, &facts.ctx(false))
        .map_err(|e| ApiError::rule("R13", e))?;
    let mut guarantee_reference = None;
    if let Some(g) = triage.first_response.as_ref().and_then(|f| f.guarantee)
        && g.to_confirm > Money::ZERO
    {
        let reference = state
            .assistance
            .request_guarantee(&claim, g.to_confirm)
            .map_err(ApiError::internal)?;
        let record = GuaranteeRecord {
            claim_id: claim.claim_id.clone(),
            customer_id: claim.customer_id.clone(),
            automatic: g.automatic,
            to_confirm: g.to_confirm,
            reference: reference.clone(),
            requested_at: now,
        };
        state
            .store
            .insert(
                c::GUARANTEES,
                &claim.claim_id,
                Keys::customer(&claim.customer_id),
                record,
            )
            .await?;
        guarantee_reference = Some(reference);
    }
    let record = ClaimRecord {
        claim: claim.clone(),
        description: req.description,
        triage,
        guarantee_reference,
        payment: None,
        settled: None,
        evidence: Vec::new(),
        assessed_by: None,
    };
    let keys = Keys::customer(&caller.customer_id).with_partner(policy.keys.partner_id.as_deref());
    let stored = state
        .store
        .insert(c::CLAIMS, &claim.claim_id, keys, record)
        .await?;
    Ok(Json(stored.body))
}

/// Evidence for a claim. Once nothing the cover asks for is missing, `AssessClaim` runs: the
/// assessor recommends, the engine approves straight through within the STP authority or
/// refers to a person (R13, R14, R57), and an approval is paid (R27, R48).
pub async fn claim_evidence(
    State(state): State<AppState>,
    caller: CustomerCaller,
    Path(claim_id): Path<String>,
    Json(upload): Json<ClaimEvidence>,
) -> ApiResult<ClaimRecord> {
    let mut record = claim(&state, &caller.customer_id, &claim_id).await?;
    let files = super::evidence(&upload.files)?;
    let seen = super::seen(&state, &files).await?;
    let policy = require_policy(&state, &caller.customer_id).await?;
    let c = customer(&state, &caller.customer_id, caller.partner_id.as_deref()).await?;
    let facts = ClaimFacts::load(
        &state,
        &policy.body,
        &c.body.profile,
        &record.body.claim.class_of_business,
        Some(&claim_id),
    )
    .await?;
    let updated = claims::add_evidence(record.body.claim.clone(), &upload.kinds, &state.cfg)?;
    let prints = super::remember(&state, &caller.customer_id, &claim_id, &files).await?;
    record.body.evidence.extend(prints);
    record.body.claim = updated;
    if record.body.claim.state == ClaimState::Assessing {
        let now = state.clock.now();
        let assessed = desk(&state)
            .assess(
                record.body.claim.clone(),
                &record.body.description,
                &files,
                &seen,
                c.body.profile.language.as_deref(),
                facts.ctx(!seen.is_empty()),
                now,
            )
            .await
            .map_err(ApiError::conflict)?;
        record.body.assessed_by = Some(AiSource::from(assessed.assessment.source));
        record.body.claim = assessed.decided.claim.clone();
        if let Some(t) = assessed.decided.newly_urgent {
            record.body.triage = t;
        }
        super::log(&state, assessed.record).await?;
        if let StpOutcome::Approved { payment } = assessed.decided.outcome {
            pay(&state, &mut record.body, payment, facts.premium_due).await?;
        }
    }
    let record = state.store.update(c::CLAIMS, record).await?;
    Ok(Json(record.body))
}

/// R16, R28: the customer settles an approved claim in cash or an alternative it offered.
pub async fn claim_choice(
    State(state): State<AppState>,
    caller: CustomerCaller,
    Path(claim_id): Path<String>,
    Json(choice): Json<SettlementChoice>,
) -> ApiResult<ClaimRecord> {
    let mut record = claim(&state, &caller.customer_id, &claim_id).await?;
    let Some(payment) = record.body.payment.clone() else {
        return Err(ApiError::conflict(format!(
            "{claim_id} has no payment to settle"
        )));
    };
    if record.body.settled.is_some() {
        return Err(ApiError::conflict(format!("{claim_id} is already settled")));
    }
    let policy = require_policy(&state, &caller.customer_id).await?;
    let settled = claims::settle(
        &payment,
        Some(&choice.settlement),
        premium_due(&policy.body, &state),
    )?;
    post_settled(&state, &caller.customer_id, &settled).await?;
    record.body.settled = Some(settled);
    let record = state.store.update(c::CLAIMS, record).await?;
    Ok(Json(record.body))
}

/// R15: any decision about the customer goes to a person on their request. A declined claim
/// returns to a person's queue with the name of the person who declined it, who cannot decide
/// it again; any other decision opens a review request.
pub async fn review_request(
    State(state): State<AppState>,
    caller: CustomerCaller,
    Json(req): Json<ReviewRequest>,
) -> ApiResult<ReviewOpened> {
    let decision = state
        .store
        .get::<DecisionRecord>(c::DECISIONS, &req.decision_id)
        .await?
        .filter(|r| r.body.customer_id == caller.customer_id)
        .ok_or_else(|| ApiError::not_found(format!("no decision {}", req.decision_id)))?
        .body;
    let now = state.clock.now();
    if decision.subject_type == "claim" {
        let mut record = claim(&state, &caller.customer_id, &decision.subject_id).await?;
        if matches!(record.body.claim.state, ClaimState::Declined { .. }) {
            let reviewed: Claim = claims::appeal(record.body.claim.clone(), now, &state.cfg)?;
            record.body.claim = reviewed.clone();
            state.store.update(c::CLAIMS, record).await?;
            return Ok(Json(ReviewOpened {
                appeal: None,
                claim: Some(reviewed),
            }));
        }
    }
    let appeal = appeals::open(
        &decision,
        &caller.customer_id,
        &req.reason,
        now,
        &state.cfg.guardrails,
    )?;
    let keys = Keys::customer(&caller.customer_id);
    let stored = state
        .store
        .insert(c::APPEALS, &appeal.appeal_id.clone(), keys, appeal)
        .await
        .map_err(|e| match e {
            microinsurance_store::StoreError::Conflict { .. } => {
                ApiError::conflict("a review of this decision is already open")
            }
            other => other.into(),
        })?;
    Ok(Json(ReviewOpened {
        appeal: Some(stored.body),
        claim: None,
    }))
}
