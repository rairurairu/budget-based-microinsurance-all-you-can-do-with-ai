//! M7 Reward engine (ADR-009). Rewards are earned only for verified prevention (R10, R40) and for
//! sustained participation (R26), never for not claiming; granted within a monthly cap (R17);
//! kept as a reward balance the customer spends on cash, always offered, or a partner reward
//! (R16). The AI only ranks the menu.
use crate::config::{EngineConfig, SupplierId};
use crate::money::Money;
use crate::score::VerifiedActivity;
use crate::types::{CustomerId, Plan, PolicyPeriod, PoolId};
use crate::{EngineError, Result};
use chrono::{Datelike, NaiveDate};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub enum Funder {
    Insurer,
    Partner,
    CoFunded,
}

/// What a reward can be earned for. A closed type with no claim-free variant: no reward is ever
/// conditional on not claiming (R26).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "for", rename_all = "snake_case")]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub enum EarnedFor {
    /// A verified prevention activity (R10, R40).
    VerifiedActivity { task_id: String },
    /// Verified activity in this many consecutive policy periods in one points pool (R26).
    ParticipationStreak { periods: u32 },
}

/// A reward earned, before the monthly cap.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct RewardEarned {
    pub reward_id: String,
    pub customer_id: CustomerId,
    /// The points pool of the behaviour it rewards (R36).
    pub pool: PoolId,
    pub earned_for: EarnedFor,
    pub amount: Money,
    /// Market local date.
    pub on: NaiveDate,
}

/// A reward granted into the customer's reward balance: the amount earned, reduced to the room
/// left under the monthly cap (R17).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct Grant {
    pub idempotency_key: String,
    pub reward: RewardEarned,
    pub granted: Money,
    /// Earned but over the monthly cap; the customer is told.
    pub over_cap: Money,
}

fn same_month(a: NaiveDate, b: NaiveDate) -> bool {
    a.year() == b.year() && a.month() == b.month()
}

/// Rewards granted to a customer in the calendar month of `on` (R17).
pub fn month_to_date(customer_id: &str, on: NaiveDate, previous: &[Grant]) -> Money {
    previous
        .iter()
        .filter(|g| g.reward.customer_id == customer_id && same_month(g.reward.on, on))
        .map(|g| g.granted)
        .sum()
}

/// R17: grants a reward within the room left under `monthly_cap` in its calendar month.
/// `previous` are the customer's earlier grants.
pub fn grant(earned: RewardEarned, previous: &[Grant], cfg: &EngineConfig) -> Grant {
    let used = month_to_date(&earned.customer_id, earned.on, previous);
    let room = (cfg.rewards.monthly_cap - used).max(Money::ZERO);
    let granted = earned.amount.max(Money::ZERO).min(room);
    Grant {
        idempotency_key: format!("reward/{}", earned.reward_id),
        over_cap: earned.amount - granted,
        granted,
        reward: earned,
    }
}

/// R26: a loyalty reward when the policy period just closed (the last of `periods`, the
/// customer's periods in order) completes a multiple of `loyalty.streak_periods` consecutive
/// periods with a verified prevention activity in `pool`. Claims are not an input.
pub fn loyalty(
    customer_id: &str,
    pool: &str,
    periods: &[PolicyPeriod],
    history: &[VerifiedActivity],
    cfg: &EngineConfig,
) -> Option<RewardEarned> {
    let terms = cfg.rewards.loyalty?;
    let last = periods.last()?;
    let active = |p: &PolicyPeriod| {
        history.iter().any(|a| {
            p.starts <= a.on()
                && a.on() < p.ends
                && cfg.lines.pool_of(a.class_of_business()) == pool
        })
    };
    let mut streak: u32 = 0;
    let mut next: Option<&PolicyPeriod> = None;
    for p in periods.iter().rev() {
        let contiguous = next.is_none_or(|n| p.ends == n.starts);
        if !contiguous || !active(p) {
            break;
        }
        streak += 1;
        next = Some(p);
    }
    if streak == 0 || !streak.is_multiple_of(terms.streak_periods) {
        return None;
    }
    Some(RewardEarned {
        reward_id: format!("loyalty/{customer_id}/{pool}/{}", last.index),
        customer_id: customer_id.to_string(),
        pool: pool.to_string(),
        earned_for: EarnedFor::ParticipationStreak { periods: streak },
        amount: terms.reward,
        on: last.ends,
    })
}

/// A way to spend the reward balance: cash, or a partner reward from a registered supplier.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct RewardOffer {
    pub offer_id: String,
    /// A configured reward family (`rewards.families`); `cash` for the cash offer.
    pub family: String,
    /// The supplier that delivers it (`config/suppliers.yaml`); none for cash.
    pub partner_id: Option<SupplierId>,
    /// The partner's own words; the app shows cash in the customer's language.
    pub description: String,
    /// What the customer sees.
    pub perceived_value: Money,
    /// Taken from the reward balance: what the insurer pays the supplier when it is delivered.
    /// The supplier's spare capacity funds the value above it.
    pub price: Money,
    /// Who funds the value above the price.
    pub funded_by: Funder,
    pub inventory: Option<u32>,
    pub is_cash: bool,
}

pub const CASH: &str = "cash";

/// Cash for the whole balance (R16).
pub fn cash(balance: Money) -> RewardOffer {
    RewardOffer {
        offer_id: CASH.into(),
        family: CASH.into(),
        partner_id: None,
        description: String::new(),
        perceived_value: balance,
        price: balance,
        funded_by: Funder::Insurer,
        inventory: None,
        is_cash: true,
    }
}

/// R45: families open to a plan. A family some plan names as a privilege is open only on the
/// plans that name it; the rest are open on every plan.
pub fn families_for(plan: &Plan, cfg: &EngineConfig) -> BTreeSet<String> {
    let reserved: BTreeSet<&String> = cfg
        .plans
        .iter()
        .flat_map(|p| &p.privileges.extra_reward_families)
        .collect();
    cfg.rewards
        .families
        .iter()
        .filter(|f| !reserved.contains(f) || plan.privileges.extra_reward_families.contains(*f))
        .cloned()
        .collect()
}

/// What the customer can spend `balance` on: cash for all of it first, always (R16); then
/// partner offers in a family the plan opens (R45), from a supplier registered for rewards,
/// with stock left and a price within the balance, by offer id. Empty when there is nothing to
/// spend.
pub fn menu(
    balance: Money,
    offers: &[RewardOffer],
    plan: &Plan,
    cfg: &EngineConfig,
) -> Vec<RewardOffer> {
    if balance <= Money::ZERO {
        return Vec::new();
    }
    let open = families_for(plan, cfg);
    let mut partner: Vec<RewardOffer> = offers
        .iter()
        .filter(|o| !o.is_cash && o.offer_id != CASH)
        .filter(|o| open.contains(&o.family))
        .filter(|o| {
            o.partner_id
                .as_deref()
                .is_some_and(|s| cfg.suppliers.reward_supplier(s).is_some())
        })
        .filter(|o| o.inventory != Some(0))
        .filter(|o| Money::ZERO < o.price && o.price <= balance)
        .cloned()
        .collect();
    partner.sort_by(|a, b| a.offer_id.cmp(&b.offer_id));
    partner.dedup_by(|a, b| a.offer_id == b.offer_id);
    let mut out = vec![cash(balance)];
    out.extend(partner);
    out
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "method", rename_all = "snake_case")]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub enum Redeemed {
    Cash,
    /// Delivered by the supplier, which is paid the price on delivery (R56).
    InKind {
        supplier_id: SupplierId,
    },
}

/// The customer spent part or all of their reward balance.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct Redemption {
    /// The ledger's idempotency key.
    pub redemption_id: String,
    pub customer_id: CustomerId,
    pub offer_id: String,
    pub amount: Money,
    pub method: Redeemed,
}

/// The customer's choice from the menu (R16). Only what the menu offered can be chosen.
pub fn redeem(
    customer_id: &str,
    redemption_id: &str,
    offer_id: &str,
    menu: &[RewardOffer],
) -> Result<Redemption> {
    let offer = menu
        .iter()
        .find(|o| o.offer_id == offer_id)
        .ok_or_else(|| EngineError::Rule {
            rule: "R16",
            detail: format!("{offer_id} was not on the customer's menu"),
        })?;
    let method = match (&offer.partner_id, offer.is_cash) {
        (_, true) => Redeemed::Cash,
        (Some(s), false) => Redeemed::InKind {
            supplier_id: s.clone(),
        },
        (None, false) => {
            return Err(EngineError::Rule {
                rule: "R56",
                detail: format!("{offer_id} has no supplier"),
            });
        }
    };
    Ok(Redemption {
        redemption_id: redemption_id.to_string(),
        customer_id: customer_id.to_string(),
        offer_id: offer.offer_id.clone(),
        amount: offer.price,
        method,
    })
}

/// The reward balance: granted, less spent and voided, plus partner rewards returned unused
/// (R56).
pub fn balance(
    grants: &[Grant],
    redemptions: &[Redemption],
    returned: Money,
    voided: &[Voided],
) -> Money {
    let granted: Money = grants.iter().map(|g| g.granted).sum();
    let spent: Money = redemptions.iter().map(|r| r.amount).sum();
    let taken_back: Money = voided.iter().map(|v| v.amount).sum();
    granted - spent - taken_back + returned
}

/// A person's finding that a rewarded activity was fraudulent. Only a person finds fraud (R10,
/// R13).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct FraudFinding {
    pub reviewer: String,
    pub on: NaiveDate,
    pub note: String,
}

/// A reward voided on a person's finding of fraud.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct Voided {
    pub idempotency_key: String,
    pub reward_id: String,
    pub customer_id: CustomerId,
    /// Taken back from the reward balance.
    pub amount: Money,
    /// Granted but already spent: recorded for the insurer to recover outside the reward
    /// balance, if at all.
    pub unrecovered: Money,
    pub finding: FraudFinding,
}

/// Voids a granted reward on a person's finding of fraud (ADR-010): what is left of it in the
/// reward balance is taken back, the rest is recorded as unrecovered. Points for the activity
/// go with it: the caller drops the activity from the customer's history.
pub fn void(grant: &Grant, finding: FraudFinding, balance: Money) -> Result<Voided> {
    if finding.reviewer.trim().is_empty() {
        return Err(EngineError::Rule {
            rule: "R13",
            detail: format!("only a person voids {} for fraud", grant.reward.reward_id),
        });
    }
    let amount = grant.granted.min(balance.max(Money::ZERO));
    Ok(Voided {
        idempotency_key: format!("{}/void", grant.idempotency_key),
        reward_id: grant.reward.reward_id.clone(),
        customer_id: grant.reward.customer_id.clone(),
        unrecovered: grant.granted - amount,
        amount,
        finding,
    })
}
