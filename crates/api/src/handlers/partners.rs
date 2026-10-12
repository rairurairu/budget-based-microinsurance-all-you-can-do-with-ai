//! A distribution partner's server: the token exchange for its customers (ADR-011), its results
//! as aggregates over enough customers only (R21), its spare capacity as reward offers (R16,
//! R56), and premiums its wallet collected (R53).
use crate::auth::{PartnerCaller, Principal, customer_id_for};
use crate::bodies::{
    CustomerRecord, IssuedToken, PayoutNotice, PolicyRecord, PremiumPayment, RewardOffers,
    TaskRecord, TokenRequest,
};
use crate::error::{ApiError, ApiResult};
use crate::state::AppState;
use axum::Json;
use axum::extract::State;
use chrono::Duration;
use microinsurance_engine::prevention::TaskState;
use microinsurance_engine::rewards::{Redemption, RewardOffer};
use microinsurance_learning::insights::{self, CustomerFacts, PartnerResults};
use microinsurance_ledger::{LedgerStore, postings};
use microinsurance_store::{By, Keys, Record, collections as c};
use std::collections::BTreeMap;

/// The token exchange: the partner names its own customer and gets a short-lived customer token
/// for the embedded journey. The customer's id is the insurer's.
pub async fn customer_token(
    State(state): State<AppState>,
    caller: PartnerCaller,
    Json(req): Json<TokenRequest>,
) -> ApiResult<IssuedToken> {
    if !state
        .cfg
        .distribution_partners
        .contains_key(&caller.partner_id)
    {
        return Err(ApiError::rule(
            "R52",
            format!(
                "{} is not a recorded distribution partner",
                caller.partner_id
            ),
        ));
    }
    let reference = req.customer_ref.trim();
    if reference.is_empty() || reference.len() > 128 {
        return Err(ApiError::bad_request("customer_ref is 1 to 128 characters"));
    }
    let customer_id = customer_id_for(&caller.partner_id, reference);
    let expires_at =
        state.clock.now() + Duration::minutes(i64::from(state.channels.customer_token_minutes));
    let token = state.keys.issue(
        &Principal::Customer {
            customer_id: customer_id.clone(),
            partner_id: Some(caller.partner_id.clone()),
        },
        expires_at,
    );
    Ok(Json(IssuedToken {
        token,
        customer_id,
        expires_at,
    }))
}

/// R21: the partner's results over the customers who came through it and everyone who took one
/// of its rewards; a figure from fewer than `partner_insights_min_cohort` customers is withheld.
pub async fn results(
    State(state): State<AppState>,
    caller: PartnerCaller,
) -> ApiResult<PartnerResults> {
    let me = caller.partner_id.as_str();
    let today = state.today();
    let mut facts: BTreeMap<String, CustomerFacts> = BTreeMap::new();
    for r in state
        .store
        .list::<CustomerRecord>(c::CUSTOMERS, By::Partner(me))
        .await?
    {
        facts.entry(r.id).or_default().through_partner = true;
    }
    for r in state
        .store
        .list::<PolicyRecord>(c::POLICIES, By::Partner(me))
        .await?
    {
        let p = &r.body.policy;
        let f = facts.entry(p.customer_id.clone()).or_default();
        f.through_partner = true;
        f.bound = true;
        f.in_force = p.period.starts <= today && today < p.period.ends;
        f.due_to_renew = p.period.index > 1 || p.period.ends <= today;
        f.renewed = p.period.index > 1;
    }
    // One query per collection for all of the partner's customers, never one per customer.
    let mine: Vec<String> = facts.keys().cloned().collect();
    let by = By::Customers(&mine);
    let owner = |keys: &microinsurance_store::Keys| keys.customer_id.clone().unwrap_or_default();
    for t in state
        .store
        .list::<TaskRecord>(c::PREVENTION_TASKS, by)
        .await?
    {
        let f = facts.entry(owner(&t.keys)).or_default();
        f.tasks_offered += 1;
        f.tasks_verified += u32::from(matches!(t.body.task.state, TaskState::Verified { .. }));
    }
    for p in state.store.list::<PayoutNotice>(c::PAYOUTS, by).await? {
        let f = facts.entry(owner(&p.keys)).or_default();
        f.payouts += 1;
        f.payout_minor = f.payout_minor.saturating_add(p.body.decision.amount.0);
    }
    for q in state.store.list::<serde_json::Value>(c::QUOTES, by).await? {
        facts.entry(owner(&q.keys)).or_default().quoted = true;
    }
    for g in state.store.list::<serde_json::Value>(c::GRANTS, by).await? {
        facts.entry(owner(&g.keys)).or_default().rewards_granted += 1;
    }
    for r in state
        .store
        .list::<serde_json::Value>(c::REDEMPTIONS, by)
        .await?
    {
        facts.entry(owner(&r.keys)).or_default().rewards_redeemed += 1;
    }
    let offers: Vec<String> = state
        .store
        .list::<RewardOffer>(c::REWARD_OFFERS, By::Partner(me))
        .await?
        .into_iter()
        .map(|r| r.body.offer_id)
        .collect();
    if !offers.is_empty() {
        for r in state
            .store
            .list::<Redemption>(c::REDEMPTIONS, By::All)
            .await?
        {
            if offers.contains(&r.body.offer_id) {
                facts
                    .entry(r.body.customer_id.clone())
                    .or_default()
                    .redemptions_with_partner += 1;
            }
        }
    }
    let rows: Vec<CustomerFacts> = facts.into_values().collect();
    Ok(Json(insights::partner_results(
        me,
        &rows,
        state.channels.partner_insights_min_cohort,
    )))
}

/// The partner's reward offers, as last published.
pub async fn reward_offers(
    State(state): State<AppState>,
    caller: PartnerCaller,
) -> ApiResult<RewardOffers> {
    Ok(Json(RewardOffers {
        offers: state
            .store
            .list::<RewardOffer>(c::REWARD_OFFERS, By::Partner(&caller.partner_id))
            .await?
            .into_iter()
            .map(|r| r.body)
            .collect(),
    }))
}

/// Spare capacity published as partner rewards, with price and stock (R16, R56). Only a
/// supplier registered for rewards may publish, only its own offers, never cash.
pub async fn publish_reward_offers(
    State(state): State<AppState>,
    caller: PartnerCaller,
    Json(body): Json<RewardOffers>,
) -> ApiResult<RewardOffers> {
    let me = caller.partner_id.as_str();
    if state.cfg.suppliers.reward_supplier(me).is_none() {
        return Err(ApiError::rule(
            "R56",
            format!("{me} is not a supplier registered for partner rewards"),
        ));
    }
    for o in &body.offers {
        if o.is_cash || o.partner_id.as_deref() != Some(me) {
            return Err(ApiError::rule(
                "R16",
                format!(
                    "{}: a partner publishes only its own partner rewards",
                    o.offer_id
                ),
            ));
        }
        if !state.cfg.rewards.families.contains(&o.family) {
            return Err(ApiError::rule(
                "R45",
                format!("{} is not a configured reward family", o.family),
            ));
        }
    }
    let mut out = Vec::new();
    for o in body.offers {
        let id = o.offer_id.clone();
        let stored = match state
            .store
            .get::<RewardOffer>(c::REWARD_OFFERS, &id)
            .await?
        {
            Some(r) if r.keys.partner_id.as_deref() != Some(me) => {
                return Err(ApiError::conflict(format!(
                    "{id} belongs to another partner"
                )));
            }
            Some(r) => {
                state
                    .store
                    .update(c::REWARD_OFFERS, Record { body: o, ..r })
                    .await?
            }
            None => {
                let keys = Keys {
                    customer_id: None,
                    partner_id: Some(me.to_string()),
                };
                state.store.insert(c::REWARD_OFFERS, &id, keys, o).await?
            }
        };
        out.push(stored.body);
    }
    Ok(Json(RewardOffers { offers: out }))
}

/// R53: the partner's wallet collected the premium for a policy bound through it.
pub async fn premium_payment(
    State(state): State<AppState>,
    caller: PartnerCaller,
    Json(p): Json<PremiumPayment>,
) -> ApiResult<PolicyRecord> {
    let mut r = state
        .store
        .get::<PolicyRecord>(c::POLICIES, &p.policy_id)
        .await?
        .filter(|r| r.body.partner_id.as_deref() == Some(caller.partner_id.as_str()))
        .ok_or_else(|| ApiError::not_found(format!("no policy {} through you", p.policy_id)))?;
    if p.period_index != r.body.policy.period.index {
        return Err(ApiError::conflict(
            "that is not the policy's current period",
        ));
    }
    let policy = &r.body.policy;
    let (key, entries) = postings::premium_received(
        &policy.policy_id,
        policy.period.index,
        policy.schedule.total_premium(),
    );
    state.ledger.post(&key, &entries).await?;
    r.body.premium_paid_on = Some(state.today());
    let r = state.store.update(c::POLICIES, r).await?;
    Ok(Json(r.body))
}
