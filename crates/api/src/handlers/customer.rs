//! The customer's own journey: declarations and opt-ins, the proposal form, quotes, the tap that
//! binds or changes the policy, the policy itself, its renewal notice, identity checks and
//! cancellation. Nothing changes without the customer's tap (R03); every quote is kept with what
//! it showed (R49); the plan is never recommended (R45).
use super::{customer, policy, premium_status, require_policy};
use crate::auth::CustomerCaller;
use crate::bodies::{
    Accepted, ChangeSentence, ConsentsUpdate, CustomerRecord, IdentityCheck, PolicyRecord,
    PolicyView, ProfileFactsView, ProfileUpdate, ProposalAnswers, ProposalFormRequest,
    ProposalFormView, Quote, QuoteRequest, RatingAnswers, RenewalRecord, RenewalView,
};
use crate::error::{ApiError, ApiResult};
use crate::state::{AppState, Inner};
use axum::Json;
use axum::extract::{Path, State};
use microinsurance_ai::{explain, understand};
use microinsurance_engine::consent::{
    self, CancellationData, CustomerTap, Endorsement, Offer, OfferKind,
};
use microinsurance_engine::guardrails::{Actor, DecisionRecord};
use microinsurance_engine::optimiser;
use microinsurance_engine::plans::{self, PurchaseContext};
use microinsurance_engine::types::{PolicyPeriod, Profile, add_policy_periods};
use microinsurance_ledger::{LedgerStore, postings};
use microinsurance_store::{By, Keys, Record, collections as c};
use microinsurance_workers::renewal;

fn check_profile(state: &Inner, p: &Profile) -> Result<(), ApiError> {
    if let Some(l) = &p.language
        && !state.cfg.languages.offered.contains(l)
    {
        return Err(ApiError::bad_request(format!(
            "{l} is not an offered language"
        )));
    }
    for w in &p.work_windows {
        if !(1..=7).contains(&w.weekday) || w.start_minute >= 1440 || w.end_minute > 1440 {
            return Err(ApiError::bad_request(
                "a working window needs weekday 1-7 and minutes in a day",
            ));
        }
    }
    let v = &state.vocab;
    if let Some(i) = p
        .declared_items
        .iter()
        .find(|i| !v.declared_item_kinds.contains(&i.kind))
    {
        return Err(ApiError::rule(
            "R55",
            format!("{} is not a registered item kind", i.kind),
        ));
    }
    if let Some(e) = p
        .upcoming_events
        .iter()
        .find(|e| !v.upcoming_event_kinds.contains(&e.kind))
    {
        return Err(ApiError::rule(
            "R55",
            format!("{} is not a registered event kind", e.kind),
        ));
    }
    if let Some(r) = p.commute_routes.iter().find(|r| !v.routes.contains(*r)) {
        return Err(ApiError::rule(
            "R55",
            format!("{r} is not a registered route"),
        ));
    }
    if let Some(a) = p.work_areas.iter().find(|a| !state.areas.contains(*a)) {
        return Err(ApiError::rule(
            "R55",
            format!("{a} is not a registered work area"),
        ));
    }
    Ok(())
}

pub async fn get_profile(
    State(state): State<AppState>,
    caller: CustomerCaller,
) -> ApiResult<CustomerRecord> {
    let c = customer(&state, &caller.customer_id, caller.partner_id.as_deref()).await?;
    Ok(Json(c.body))
}

/// What the customer declares (R08, R42, R55). It changes the policy's facts only from the next
/// policy period (R04).
pub async fn update_profile(
    State(state): State<AppState>,
    caller: CustomerCaller,
    Json(u): Json<ProfileUpdate>,
) -> ApiResult<CustomerRecord> {
    let mut c = customer(&state, &caller.customer_id, caller.partner_id.as_deref()).await?;
    let p = &mut c.body.profile;
    if let Some(v) = u.occupation {
        p.occupation = Some(v).filter(|s| !s.trim().is_empty());
    }
    if let Some(v) = u.work_windows {
        p.work_windows = v;
    }
    if let Some(v) = u.declared_items {
        p.declared_items = v;
    }
    if let Some(v) = u.commute_routes {
        p.commute_routes = v;
    }
    if let Some(v) = u.work_areas {
        p.work_areas = v;
    }
    if let Some(v) = u.upcoming_events {
        p.upcoming_events = v;
    }
    if let Some(v) = u.typical_daily_earnings {
        p.typical_daily_earnings = Some(v);
    }
    if let Some(v) = u.language {
        p.language = Some(v);
    }
    check_profile(&state, p)?;
    let c = state.store.update(c::CUSTOMERS, c).await?;
    Ok(Json(c.body))
}

/// R20: opt-ins per registered data type, each used only for the covers it supports.
pub async fn update_consents(
    State(state): State<AppState>,
    caller: CustomerCaller,
    Json(u): Json<ConsentsUpdate>,
) -> ApiResult<CustomerRecord> {
    if let Some(d) = u
        .consents
        .iter()
        .find(|d| !state.cfg.data_types.data_types.contains(*d))
    {
        return Err(ApiError::rule(
            "R20",
            format!("{d} is not a registered data type"),
        ));
    }
    let mut c = customer(&state, &caller.customer_id, caller.partner_id.as_deref()).await?;
    c.body.profile.consents = u.consents;
    let c = state.store.update(c::CUSTOMERS, c).await?;
    Ok(Json(c.body))
}

/// Answers to registered rating factors, kept apart from the profile (R19, R33).
pub async fn update_rating_answers(
    State(state): State<AppState>,
    caller: CustomerCaller,
    Json(a): Json<RatingAnswers>,
) -> ApiResult<RatingAnswers> {
    for (factor, answer) in &a.answers {
        let known = state.cfg.rating_factors.get(factor).is_some_and(|f| {
            f.level == microinsurance_engine::config::FactorLevel::Individual
                && f.data == Some(microinsurance_engine::config::FactorData::Declared)
        });
        let rated = state
            .rating
            .factor_tables
            .get(factor)
            .is_some_and(|t| t.values().any(|t| t.relativities.contains_key(answer)));
        if !known || !rated {
            return Err(ApiError::rule(
                "R33",
                format!("{factor}: {answer:?} is not a registered answer"),
            ));
        }
    }
    customer(&state, &caller.customer_id, caller.partner_id.as_deref()).await?;
    let keys = Keys::customer(&caller.customer_id);
    let stored = match state
        .store
        .get::<RatingAnswers>(c::RATING_ANSWERS, &caller.customer_id)
        .await?
    {
        Some(r) => {
            state
                .store
                .update(c::RATING_ANSWERS, Record { body: a, ..r })
                .await?
        }
        None => {
            state
                .store
                .insert(c::RATING_ANSWERS, &caller.customer_id, keys, a)
                .await?
        }
    };
    Ok(Json(stored.body))
}

/// A short proposal form for the channel, in the customer's language (ai::understand).
pub async fn proposal_form(
    State(state): State<AppState>,
    caller: CustomerCaller,
    Json(r): Json<ProposalFormRequest>,
) -> ApiResult<ProposalFormView> {
    let c = customer(&state, &caller.customer_id, caller.partner_id.as_deref()).await?;
    let form = understand::proposal_form(
        &state.ai,
        &r.channel,
        c.body.profile.language.as_deref(),
        &state.vocab,
    )
    .await;
    Ok(Json(ProposalFormView {
        source: form.source.into(),
        form: form.value,
    }))
}

/// The answers as profile facts in registered ids, for the customer to confirm: nothing is
/// saved until they update their profile.
pub async fn proposal_answers(
    State(state): State<AppState>,
    _caller: CustomerCaller,
    Json(a): Json<ProposalAnswers>,
) -> ApiResult<ProfileFactsView> {
    let answers =
        serde_json::to_value(&a.answers).map_err(|e| ApiError::bad_request(e.to_string()))?;
    let facts = understand::map_answers(&state.ai, &a.form, &answers, &state.vocab).await;
    Ok(Json(ProfileFactsView {
        occupation: facts.value.occupation,
        devices: facts.value.devices,
        commute_routes: facts.value.commute_routes,
        source: facts.source.into(),
    }))
}

/// A quote for a plan the customer picked: the optimiser's suggestion or the customer's own
/// split, priced (R02, R34), one sentence per cover (R23), and the price with any promotional
/// discount (R44). For a policyholder it is a change from the next policy period (R04). Kept as
/// shown (R49).
pub async fn quote(
    State(state): State<AppState>,
    caller: CustomerCaller,
    Json(req): Json<QuoteRequest>,
) -> ApiResult<Quote> {
    let c = customer(&state, &caller.customer_id, caller.partner_id.as_deref()).await?;
    if c.body.cancelled.is_some() {
        return Err(ApiError::conflict("this customer has cancelled"));
    }
    let profile = c.body.profile;
    let plan = state
        .cfg
        .plan(&req.plan_id)
        .ok_or_else(|| ApiError::rule("R01", format!("{} is not a plan", req.plan_id)))?
        .clone();
    let held = policy(&state, &caller.customer_id).await?;
    let now = state.clock.now();
    let today = state.today();
    let held_policy = held.as_ref().map(|r| &r.body.policy);
    let period = match held_policy {
        None => {
            let starts = today.succ_opt().unwrap_or(today);
            PolicyPeriod {
                index: 1,
                starts,
                ends: add_policy_periods(starts, 1),
            }
        }
        Some(p) => renewal::next_period(&p.period),
    };
    let rated = super::rated(&state, &plan, &profile, held_policy, period).await?;
    let ctx = rated.ctx(&state.cfg);
    let current = held_policy
        .filter(|p| p.plan.id == plan.id)
        .map(|p| &p.schedule);
    let (schedule, suggestion) = match &req.premiums {
        Some(p) => (
            optimiser::price_allocation(&plan, p, current, &profile, &ctx)?,
            None,
        ),
        None => {
            let s = optimiser::suggest(&plan, &profile, current, &ctx)?;
            (s.outcome.schedule.clone(), Some(s))
        }
    };
    let explained = explain::explain_schedule(&state.ai, &schedule, &profile, &state.cfg).await;
    let price = plans::price(
        &plan,
        &PurchaseContext {
            channel: caller.partner_id.clone(),
            new_customer: held.is_none(),
        },
        today,
        &state.cfg,
    );
    let change = match held_policy {
        None => OfferKind::NewPolicy {
            schedule: schedule.clone(),
            partner_id: caller.partner_id.clone(),
        },
        Some(p) if p.plan.id != plan.id => OfferKind::PlanChange {
            plan: plan.id.clone(),
        },
        Some(_) => OfferKind::Reallocation {
            schedule: schedule.clone(),
        },
    };
    let top = schedule.allocations.iter().max_by_key(|a| a.premium);
    let explanation = top
        .and_then(|t| explained.value.iter().find(|e| e.cover_id == t.cover_id))
        .map(|e| e.sentence.clone())
        .unwrap_or_default();
    let offer = Offer {
        offer_id: state.new_id("quote"),
        customer_id: caller.customer_id.clone(),
        change,
        explanation,
        created_at: now,
    };
    let quote = Quote {
        schedule,
        price,
        suggestion,
        explained_by: explained.source.into(),
        explanations: explained.value,
        period_starts: period.starts,
        shown_at: now,
        accepted_at: None,
        offer,
    };
    let keys = Keys::customer(&caller.customer_id).with_partner(caller.partner_id.as_deref());
    let stored = state
        .store
        .insert(c::QUOTES, &quote.offer.offer_id.clone(), keys, quote)
        .await?;
    Ok(Json(stored.body))
}

pub async fn get_quote(
    State(state): State<AppState>,
    caller: CustomerCaller,
    Path(quote_id): Path<String>,
) -> ApiResult<Quote> {
    state
        .store
        .get::<Quote>(c::QUOTES, &quote_id)
        .await?
        .filter(|r| r.body.offer.customer_id == caller.customer_id)
        .map(|r| Json(r.body))
        .ok_or_else(|| ApiError::not_found(format!("no quote {quote_id}")))
}

fn endorsement_id(e: &Endorsement) -> String {
    let kind = match &e.change {
        OfferKind::PlanChange { .. } => "plan",
        _ => "schedule",
    };
    format!("{}/{}/{kind}", e.policy_id, e.effective_period)
}

/// The customer's tap on a quote (R03): new business binds a policy (R52, R58) and bills its
/// first premium; a change becomes an endorsement for the next policy period (R04, R05, R34).
/// A quote is accepted once, however many times it is tapped.
pub async fn accept(
    State(state): State<AppState>,
    caller: CustomerCaller,
    Path(quote_id): Path<String>,
) -> ApiResult<Accepted> {
    let mut quote = state
        .store
        .get::<Quote>(c::QUOTES, &quote_id)
        .await?
        .filter(|r| r.body.offer.customer_id == caller.customer_id)
        .ok_or_else(|| ApiError::not_found(format!("no quote {quote_id}")))?;
    if quote.body.accepted_at.is_some() {
        return Err(ApiError::conflict(format!(
            "{quote_id} is already accepted"
        )));
    }
    let now = state.clock.now();
    let tap = CustomerTap {
        offer_id: quote.body.offer.offer_id.clone(),
        customer_id: caller.customer_id.clone(),
        at: now,
    };
    let offer = quote.body.offer.clone();
    let held = policy(&state, &caller.customer_id).await?;
    let accepted = match (&offer.change, held) {
        (OfferKind::NewPolicy { partner_id, .. }, None) => {
            let c = customer(&state, &caller.customer_id, caller.partner_id.as_deref()).await?;
            // One policy per customer: its id is theirs, so two taps, on one quote or two, bind once.
            let policy_id = format!("pol-{}", caller.customer_id.trim_start_matches("cust-"));
            let bound = consent::bind(&offer, &tap, &policy_id, &c.body.profile, &state.cfg)?;
            let record = PolicyRecord {
                premium_due: bound.period.starts,
                // Demo mode stands in for the payment the partner's wallet or the insurer's
                // payment provider confirms (`POST /partners/me/premium-payments`).
                premium_paid_on: state.demo.then(|| state.today()),
                partner_id: partner_id.clone(),
                quote_id: quote_id.clone(),
                policy: bound.clone(),
            };
            let keys = Keys::customer(&caller.customer_id).with_partner(partner_id.as_deref());
            state
                .store
                .insert(c::POLICIES, &policy_id, keys, record)
                .await
                .map_err(|e| match e {
                    microinsurance_store::StoreError::Conflict { .. } => {
                        ApiError::conflict("the customer already holds a policy")
                    }
                    other => other.into(),
                })?;
            quote.body.accepted_at = Some(now);
            state.store.update(c::QUOTES, quote).await?;
            let (key, entries) = postings::premium_billed(
                &policy_id,
                bound.period.index,
                bound.schedule.total_premium(),
            );
            state.ledger.post(&key, &entries).await?;
            if state.demo {
                let (key, entries) = postings::premium_received(
                    &policy_id,
                    bound.period.index,
                    bound.schedule.total_premium(),
                );
                state.ledger.post(&key, &entries).await?;
            }
            super::log(
                &state,
                DecisionRecord {
                    decision_id: format!("decision/{policy_id}/bound"),
                    actor: Actor::Customer,
                    reviewer: None,
                    subject_type: "policy".into(),
                    subject_id: policy_id.clone(),
                    customer_id: caller.customer_id.clone(),
                    decision: "bound".into(),
                    inputs_hash: microinsurance_workers::claims::inputs_hash(&offer),
                    reason: quote_id.clone(),
                    at: now,
                },
            )
            .await?;
            Accepted {
                policy: Some(bound),
                endorsement: None,
            }
        }
        (OfferKind::NewPolicy { .. }, Some(_)) => {
            return Err(ApiError::conflict("the customer already holds a policy"));
        }
        (_, None) => return Err(ApiError::not_found("no policy to change")),
        (change, Some(held)) => {
            let endorsement = match change {
                OfferKind::PlanChange { .. } => {
                    plans::change_plan(&held.body.policy, &offer, &tap, &state.cfg)?
                }
                _ => consent::accept(&offer, &tap, &held.body.policy, &state.cfg)?,
            };
            quote.body.accepted_at = Some(now);
            state.store.update(c::QUOTES, quote).await?;
            let id = endorsement_id(&endorsement);
            let keys =
                Keys::customer(&caller.customer_id).with_partner(held.keys.partner_id.as_deref());
            match state.store.get::<Endorsement>(c::ENDORSEMENTS, &id).await? {
                // The customer's latest choice for that period replaces an earlier one (R34).
                Some(r) => {
                    state
                        .store
                        .update(
                            c::ENDORSEMENTS,
                            Record {
                                body: endorsement.clone(),
                                ..r
                            },
                        )
                        .await?;
                }
                None => {
                    state
                        .store
                        .insert(c::ENDORSEMENTS, &id, keys, endorsement.clone())
                        .await?;
                }
            }
            Accepted {
                policy: None,
                endorsement: Some(endorsement),
            }
        }
    };
    Ok(Json(accepted))
}

/// The customer's policy today, with its premium status (R53), the changes accepted for the
/// next period (R04) and one sentence per cover (R23).
pub async fn get_policy(
    State(state): State<AppState>,
    caller: CustomerCaller,
) -> ApiResult<PolicyView> {
    let record = require_policy(&state, &caller.customer_id).await?;
    let c = customer(&state, &caller.customer_id, caller.partner_id.as_deref()).await?;
    let endorsements: Vec<Endorsement> = state
        .store
        .list::<Endorsement>(c::ENDORSEMENTS, By::Customer(&caller.customer_id))
        .await?
        .into_iter()
        .map(|r| r.body)
        .filter(|e| {
            e.policy_id == record.body.policy.policy_id
                && e.effective_period > record.body.policy.period.index
        })
        .collect();
    let explained = explain::explain_schedule(
        &state.ai,
        &record.body.policy.schedule,
        &c.body.profile,
        &state.cfg,
    )
    .await;
    Ok(Json(PolicyView {
        premium: premium_status(&record.body, state.today(), &state),
        partner_id: record.body.partner_id.clone(),
        policy: record.body.policy,
        endorsements,
        explanations: explained.value,
    }))
}

/// R35: the next period's sums insured, re-rated, with one sentence per change.
pub async fn renewal(
    State(state): State<AppState>,
    caller: CustomerCaller,
) -> ApiResult<RenewalView> {
    let record = require_policy(&state, &caller.customer_id).await?;
    let c = customer(&state, &caller.customer_id, caller.partner_id.as_deref()).await?;
    let profile = c.body.profile;
    let held = &record.body.policy;
    let next = renewal::next_period(&held.period);
    let rated = super::rated(&state, &held.plan, &profile, Some(held), next).await?;
    let ctx = rated.ctx(&state.cfg);
    let proposal = renewal::propose(held, &profile, &ctx)?;
    // The notice as issued, once the renewal job has issued it (R35); until then a preview.
    let issued = state
        .store
        .get::<RenewalRecord>(c::RENEWALS, &RenewalRecord::id(&held.policy_id, next.index))
        .await?
        .map(|r| r.body);
    let issued_on = issued.as_ref().map(|r| r.issued_on);
    let notice = issued.map_or(proposal.notice, |r| r.notice);
    let basis = optimiser::suggest(&held.plan, &profile, Some(&held.schedule), &ctx)?.basis;
    let mut sentences = Vec::new();
    for change in &notice.changes {
        let said = explain::explain_change(
            &state.ai,
            change,
            basis.iter().find(|b| b.cover_id == change.cover_id),
            profile.language.as_deref(),
            &state.cfg,
        )
        .await;
        sentences.push(ChangeSentence {
            cover_id: change.cover_id.clone(),
            source: said.source.into(),
            sentence: said.value,
        });
    }
    Ok(Json(RenewalView {
        notice,
        issued_on,
        sentences,
    }))
}

/// R20, R54: the customer cancels. Cover runs to the end of the period paid for; optional data
/// is deleted now; policy, premium, claims, ledger and quote records are kept until the date
/// returned, access-restricted.
pub async fn cancel(
    State(state): State<AppState>,
    caller: CustomerCaller,
) -> ApiResult<CancellationData> {
    let mut c = customer(&state, &caller.customer_id, caller.partner_id.as_deref()).await?;
    if let Some(done) = &c.body.cancelled {
        return Ok(Json(done.clone()));
    }
    let data = consent::on_cancellation(&c.body.profile, state.today(), &state.cfg);
    c.body.profile.consents.clear();
    c.body.cancelled = Some(data.clone());
    state.store.update(c::CUSTOMERS, c).await?;
    super::log(
        &state,
        DecisionRecord {
            decision_id: format!("decision/{}/cancelled", caller.customer_id),
            actor: Actor::Customer,
            reviewer: None,
            subject_type: "customer".into(),
            subject_id: caller.customer_id.clone(),
            customer_id: caller.customer_id.clone(),
            decision: "cancelled".into(),
            inputs_hash: String::new(),
            reason: String::new(),
            at: state.clock.now(),
        },
    )
    .await?;
    Ok(Json(data))
}

/// R25: starts an identity check with the licensed eKYC provider.
pub async fn start_identity_check(
    State(state): State<AppState>,
    caller: CustomerCaller,
) -> ApiResult<IdentityCheck> {
    let session_id = state
        .kyc
        .start(&caller.customer_id)
        .map_err(|e| ApiError::rule("R25", e))?;
    let status = state
        .kyc
        .status(&session_id)
        .map_err(|e| ApiError::rule("R25", e))?;
    let check = IdentityCheck { session_id, status };
    let keys = Keys::customer(&caller.customer_id);
    match state
        .store
        .insert(
            c::IDENTITY_CHECKS,
            &check.session_id.clone(),
            keys,
            check.clone(),
        )
        .await
    {
        Ok(_) | Err(microinsurance_store::StoreError::Conflict { .. }) => Ok(Json(check)),
        Err(e) => Err(e.into()),
    }
}

pub async fn identity_check(
    State(state): State<AppState>,
    caller: CustomerCaller,
    Path(session_id): Path<String>,
) -> ApiResult<IdentityCheck> {
    let record = state
        .store
        .get::<IdentityCheck>(c::IDENTITY_CHECKS, &session_id)
        .await?
        .filter(|r| r.keys.customer_id.as_deref() == Some(caller.customer_id.as_str()))
        .ok_or_else(|| ApiError::not_found(format!("no identity check {session_id}")))?;
    let status = state
        .kyc
        .status(&session_id)
        .map_err(|e| ApiError::rule("R25", e))?;
    let updated = state
        .store
        .update(
            c::IDENTITY_CHECKS,
            Record {
                body: IdentityCheck {
                    session_id: session_id.clone(),
                    status,
                },
                ..record
            },
        )
        .await?;
    Ok(Json(updated.body))
}

/// Decisions about the customer, any of which they may ask a person to review (R15).
pub async fn decisions(
    State(state): State<AppState>,
    caller: CustomerCaller,
) -> ApiResult<Vec<DecisionRecord>> {
    Ok(Json(
        state
            .store
            .list::<DecisionRecord>(c::DECISIONS, By::Customer(&caller.customer_id))
            .await?
            .into_iter()
            .map(|r| r.body)
            .collect(),
    ))
}

/// The customer's answers to rating factors, as last saved.
pub async fn rating_answers(
    State(state): State<AppState>,
    caller: CustomerCaller,
) -> ApiResult<RatingAnswers> {
    Ok(Json(
        state
            .store
            .get::<RatingAnswers>(c::RATING_ANSWERS, &caller.customer_id)
            .await?
            .map(|r| r.body)
            .unwrap_or_default(),
    ))
}
