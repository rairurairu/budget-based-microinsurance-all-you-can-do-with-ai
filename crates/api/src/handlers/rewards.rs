//! The reward balance (ADR-009): rewards granted within the monthly cap (R17), spent on cash for
//! all of it, always first (R16), or on a partner reward the plan opens (R45), ranked by the AI;
//! a partner reward becomes an in-kind order to its supplier, paid only for what it delivers and
//! returned to the balance if unused (R56).
use super::{customer, require_policy};
use crate::auth::CustomerCaller;
use crate::bodies::{AiSource, RedeemRequest, RewardsView};
use crate::error::{ApiError, ApiResult};
use crate::state::{AppState, Inner};
use axum::Json;
use axum::extract::State;
use microinsurance_engine::money::Money;
use microinsurance_engine::prevention::Completion;
use microinsurance_engine::rewards::{self, Grant, Redemption, RewardOffer};
use microinsurance_engine::suppliers::{self, InKindOrder, OrderSource, OrderState};
use microinsurance_engine::types::Plan;
use microinsurance_ledger::{LedgerStore, postings};
use microinsurance_store::{By, Keys, Record, StoreError, collections as c};

/// Credits a verified prevention activity: its points (R10, R11) and its reward, granted within
/// the monthly cap (R17) and posted as a loss-prevention expense. Once per task.
pub async fn credit(state: &Inner, done: Completion) -> Result<Grant, ApiError> {
    let customer_id = done.reward.customer_id.clone();
    let keys = Keys::customer(&customer_id);
    match state
        .store
        .insert(
            c::ACTIVITIES,
            &done.reward.reward_id.clone(),
            keys.clone(),
            done.activity,
        )
        .await
    {
        Ok(_) | Err(StoreError::Conflict { .. }) => {}
        Err(e) => return Err(e.into()),
    }
    let key = format!("reward/{}", done.reward.reward_id);
    if let Some(g) = state.store.get::<Grant>(c::GRANTS, &key).await? {
        return Ok(g.body);
    }
    let previous: Vec<Grant> = grants(state, &customer_id).await?;
    let grant = rewards::grant(done.reward, &previous, &state.cfg);
    let (ledger_key, entries) = postings::reward_granted(&grant);
    state.ledger.post(&ledger_key, &entries).await?;
    let stored = state
        .store
        .insert(c::GRANTS, &grant.idempotency_key.clone(), keys, grant)
        .await?;
    Ok(stored.body)
}

async fn grants(state: &Inner, customer_id: &str) -> Result<Vec<Grant>, ApiError> {
    Ok(state
        .store
        .list::<Grant>(c::GRANTS, By::Customer(customer_id))
        .await?
        .into_iter()
        .map(|r| r.body)
        .collect())
}

/// R16, R56: granted, less spent, plus partner rewards returned unused.
async fn balance(state: &Inner, customer_id: &str) -> Result<(Money, Vec<Grant>), ApiError> {
    let grants = grants(state, customer_id).await?;
    let spent: Vec<Redemption> = state
        .store
        .list::<Redemption>(c::REDEMPTIONS, By::Customer(customer_id))
        .await?
        .into_iter()
        .map(|r| r.body)
        .collect();
    let returned: Money = state
        .store
        .list::<InKindOrder>(c::IN_KIND_ORDERS, By::Customer(customer_id))
        .await?
        .into_iter()
        .map(|r| r.body)
        .filter(|o| {
            o.source == OrderSource::Reward && matches!(o.state, OrderState::Expired { .. })
        })
        .map(|o| o.reserved)
        .sum();
    Ok((rewards::balance(&grants, &spent, returned, &[]), grants))
}

async fn offers(state: &Inner) -> Result<Vec<Record<RewardOffer>>, ApiError> {
    Ok(state
        .store
        .list::<RewardOffer>(c::REWARD_OFFERS, By::All)
        .await?)
}

async fn menu(
    state: &Inner,
    customer_id: &str,
    plan: &Plan,
) -> Result<(Money, Vec<Grant>, Vec<RewardOffer>), ApiError> {
    let (balance, grants) = balance(state, customer_id).await?;
    let offered: Vec<RewardOffer> = offers(state).await?.into_iter().map(|r| r.body).collect();
    Ok((
        balance,
        grants,
        rewards::menu(balance, &offered, plan, &state.cfg),
    ))
}

/// The reward balance and its menu: cash for all of it first, then partner rewards in the
/// order the AI ranks them (R16).
pub async fn view(State(state): State<AppState>, caller: CustomerCaller) -> ApiResult<RewardsView> {
    let policy = require_policy(&state, &caller.customer_id).await?;
    let c = customer(&state, &caller.customer_id, caller.partner_id.as_deref()).await?;
    let (balance, grants, menu) =
        menu(&state, &caller.customer_id, &policy.body.policy.plan).await?;
    let ranked = microinsurance_ai::rewards::rank(&state.ai, &c.body.profile, &menu).await;
    let position = |o: &RewardOffer| {
        ranked
            .value
            .iter()
            .position(|id| *id == o.offer_id)
            .unwrap_or(usize::MAX)
    };
    let mut shown = menu;
    shown.sort_by_key(|o| (!o.is_cash, position(o)));
    Ok(Json(RewardsView {
        balance,
        menu: shown,
        ranked_by: AiSource::from(ranked.source),
        grants,
    }))
}

/// The customer's choice from the menu (R16): cash, or a partner reward ordered from its
/// supplier (R56).
pub async fn redeem(
    State(state): State<AppState>,
    caller: CustomerCaller,
    Json(req): Json<RedeemRequest>,
) -> ApiResult<Redemption> {
    let policy = require_policy(&state, &caller.customer_id).await?;
    let (_, _, menu) = menu(&state, &caller.customer_id, &policy.body.policy.plan).await?;
    let redemption_id = state.new_id("redeem");
    let r = rewards::redeem(&caller.customer_id, &redemption_id, &req.offer_id, &menu)?;
    let (key, entries) = postings::reward_redeemed(&r);
    state.ledger.post(&key, &entries).await?;
    let keys = Keys::customer(&caller.customer_id);
    state
        .store
        .insert(c::REDEMPTIONS, &redemption_id, keys.clone(), r.clone())
        .await?;
    if !r.offer_id.eq(rewards::CASH) {
        let order = suppliers::order_for_reward(&r, state.today(), &state.cfg)?;
        let supplier = order.supplier_id.clone();
        state
            .store
            .insert(
                c::IN_KIND_ORDERS,
                &order.order_id.clone(),
                keys.with_partner(Some(&supplier)),
                order,
            )
            .await?;
        // One less in stock.
        if let Some(mut offer) = offers(&state)
            .await?
            .into_iter()
            .find(|o| o.body.offer_id == r.offer_id)
        {
            offer.body.inventory = offer.body.inventory.map(|n| n.saturating_sub(1));
            state.store.update(c::REWARD_OFFERS, offer).await?;
        }
    }
    Ok(Json(r))
}
