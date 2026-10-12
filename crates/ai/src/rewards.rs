//! Rank eligible reward offers by what this customer is most likely to value. The engine has
//! already filtered for eligibility, the monthly cap and cash (R16, R17); the ranking is a
//! permutation of those offers, so it can reorder but never add, drop or price one. The model
//! sees what the customer sees, not what an offer costs its funder.
use crate::client::Model;
use crate::harness::{Ask, Harness, Harnessed};
use crate::settings::Tier;
use microinsurance_engine::rewards::RewardOffer;
use microinsurance_engine::types::Profile;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::BTreeSet;

pub const COMPONENT: &str = "rewards";

#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RankReply {
    /// Every offer id, best first.
    pub offer_ids: Vec<String>,
}

/// The saved order: cash first, then by perceived value, then by id.
pub fn fallback(offers: &[RewardOffer]) -> Vec<String> {
    let mut sorted: Vec<&RewardOffer> = offers.iter().collect();
    sorted.sort_by(|a, b| {
        b.is_cash
            .cmp(&a.is_cash)
            .then(b.perceived_value.cmp(&a.perceived_value))
            .then(a.offer_id.cmp(&b.offer_id))
    });
    sorted.into_iter().map(|o| o.offer_id.clone()).collect()
}

/// The ranking must be exactly the offers given, each once.
pub fn check(reply: RankReply, offers: &[RewardOffer]) -> Result<Vec<String>, String> {
    let given: BTreeSet<&str> = offers.iter().map(|o| o.offer_id.as_str()).collect();
    let ranked: BTreeSet<&str> = reply.offer_ids.iter().map(String::as_str).collect();
    if ranked != given || reply.offer_ids.len() != offers.len() {
        return Err(format!(
            "ranking {:?} is not a permutation of the offers",
            reply.offer_ids
        ));
    }
    Ok(reply.offer_ids)
}

/// Offer ids, best first.
pub async fn rank<M: Model>(
    h: &Harness<M>,
    profile: &Profile,
    offers: &[RewardOffer],
) -> Harnessed<Vec<String>> {
    if offers.len() < 2 {
        return Harnessed::rule(fallback(offers), "nothing to rank");
    }
    let shown: Vec<Value> = offers
        .iter()
        .map(|o| {
            json!({
                "offer_id": o.offer_id,
                "family": o.family,
                "description": o.description,
                "is_cash": o.is_cash,
                "value_to_customer": o.perceived_value.0 / 100,
            })
        })
        .collect();
    let items: BTreeSet<&str> = profile
        .declared_items
        .iter()
        .map(|i| i.kind.as_str())
        .collect();
    let events: BTreeSet<&str> = profile
        .upcoming_events
        .iter()
        .map(|e| e.kind.as_str())
        .collect();
    let input = json!({
        "customer": {
            "occupation": profile.occupation,
            "declared_items": items,
            "upcoming_events": events,
        },
        "offers": shown,
    });
    let ask = Ask {
        component: COMPONENT,
        tier: Tier::Fast,
        input,
        images: &[],
    };
    h.ask(ask, |r: RankReply| check(r, offers), || fallback(offers))
        .await
}
