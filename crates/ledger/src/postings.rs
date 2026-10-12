//! The entries each engine decision posts. Amounts come from the engine unchanged; each set
//! balances to zero and carries the decision's idempotency key (R27).
use crate::{Account, Posting};
use microinsurance_engine::claims::ClaimPayment;
use microinsurance_engine::config::Tariff;
use microinsurance_engine::money::Money;
use microinsurance_engine::payouts::{PayoutDecision, Settled, Settlement};
use microinsurance_engine::rewards::{Grant, Redeemed, Redemption, Voided};
use microinsurance_engine::suppliers::{InKindOrder, OrderSource, OrderState, SupplierStatement};

fn post(account: Account, amount: Money) -> Posting {
    Posting { account, amount }
}

/// A payout decided: the insurer's risk account pays it into payouts payable.
pub fn payout_decided(d: &PayoutDecision) -> (String, Vec<Posting>) {
    (
        d.idempotency_key.clone(),
        vec![
            post(Account::InsurerRisk, d.amount),
            post(Account::PayoutsPayable, Money(-d.amount.0)),
        ],
    )
}

/// A claim payment approved, straight through or by a person: like a payout, the insurer's risk
/// account pays it into payouts payable; it is settled by `payout_settled` (R27, R48).
pub fn claim_approved(p: &ClaimPayment) -> (String, Vec<Posting>) {
    (
        p.idempotency_key.clone(),
        vec![
            post(Account::InsurerRisk, p.amount),
            post(Account::PayoutsPayable, Money(-p.amount.0)),
        ],
    )
}

/// A payout settled: in cash to the customer, net of any premium set off (R53), or in kind to
/// the partner providing it (R28).
pub fn payout_settled(s: &Settled) -> (String, Vec<Posting>) {
    let mut entries = vec![post(Account::PayoutsPayable, s.amount)];
    match s.method {
        Settlement::Cash => {
            if s.cash > Money::ZERO {
                entries.push(post(Account::Cash, Money(-s.cash.0)));
            }
            if s.premium_set_off > Money::ZERO {
                entries.push(post(Account::CustomerPremiums, Money(-s.premium_set_off.0)));
            }
        }
        Settlement::InKind(_) => entries.push(post(Account::InKindPayable, Money(-s.amount.0))),
    }
    (s.idempotency_key.clone(), entries)
}

/// A policy period's premium billed to the customer and earned by the insurer's risk account.
pub fn premium_billed(
    policy_id: &str,
    period_index: u32,
    premium: Money,
) -> (String, Vec<Posting>) {
    (
        format!("premium/{policy_id}/{period_index}"),
        vec![
            post(Account::CustomerPremiums, premium),
            post(Account::InsurerRisk, Money(-premium.0)),
        ],
    )
}

/// A policy period's premium received (from the customer, or the partner's wallet that collected
/// it): cash in, the customer's premium receivable cleared (R53).
pub fn premium_received(
    policy_id: &str,
    period_index: u32,
    premium: Money,
) -> (String, Vec<Posting>) {
    (
        format!("premium/{policy_id}/{period_index}/received"),
        vec![
            post(Account::Cash, premium),
            post(Account::CustomerPremiums, Money(-premium.0)),
        ],
    )
}

/// A reward granted into the customer's reward balance: a loss-prevention expense (R17, R40).
pub fn reward_granted(g: &Grant) -> (String, Vec<Posting>) {
    (
        g.idempotency_key.clone(),
        vec![
            post(Account::RewardExpense, g.granted),
            post(Account::RewardsPayable, Money(-g.granted.0)),
        ],
    )
}

/// The reward balance spent: paid out in cash, or reserved for the supplier until it delivers
/// (R16, R56).
pub fn reward_redeemed(r: &Redemption) -> (String, Vec<Posting>) {
    let to = match r.method {
        Redeemed::Cash => Account::Cash,
        Redeemed::InKind { .. } => Account::InKindPayable,
    };
    (
        r.redemption_id.clone(),
        vec![
            post(Account::RewardsPayable, r.amount),
            post(to, Money(-r.amount.0)),
        ],
    )
}

/// An in-kind order delivered (R56). Below the amount reserved, a fixed tariff leaves the rest to
/// the insurer's risk account; an invoiced payout's rest is paid to the customer in cash. The
/// charge stays in `in_kind_payable` until the supplier's statement is paid. None when nothing
/// moves.
pub fn in_kind_delivered(o: &InKindOrder) -> Option<(String, Vec<Posting>)> {
    let OrderState::Delivered { charge, .. } = o.state else {
        return None;
    };
    let rest = o.reserved - charge;
    if rest <= Money::ZERO {
        return None;
    }
    let to = match (o.source, o.tariff) {
        (OrderSource::Payout, Tariff::Invoiced) => Account::Cash,
        (OrderSource::Payout, _) => Account::InsurerRisk,
        (OrderSource::Reward, _) => Account::RewardsPayable,
    };
    Some((
        format!("{}/delivered", o.order_id),
        vec![post(Account::InKindPayable, rest), post(to, Money(-rest.0))],
    ))
}

/// An order not delivered in time: the amount reserved goes back to the customer, a payout as
/// cash and a reward to the reward balance (R56).
pub fn in_kind_returned(o: &InKindOrder) -> Option<(String, Vec<Posting>)> {
    if !matches!(o.state, OrderState::Expired { .. }) {
        return None;
    }
    let to = match o.source {
        OrderSource::Payout => Account::Cash,
        OrderSource::Reward => Account::RewardsPayable,
    };
    Some((
        format!("{}/returned", o.order_id),
        vec![
            post(Account::InKindPayable, o.reserved),
            post(to, Money(-o.reserved.0)),
        ],
    ))
}

/// A supplier's statement paid: what it delivered in the period, at its charges (R56).
pub fn supplier_paid(s: &SupplierStatement) -> (String, Vec<Posting>) {
    (
        s.statement_id.clone(),
        vec![
            post(Account::InKindPayable, s.total),
            post(Account::Cash, Money(-s.total.0)),
        ],
    )
}

/// A reward voided on a person's finding of fraud: what was left of it comes out of the reward
/// balance and the loss-prevention expense is reversed by that much (ADR-010). The unrecovered
/// rest stays an expense.
pub fn reward_voided(v: &Voided) -> Option<(String, Vec<Posting>)> {
    if v.amount <= Money::ZERO {
        return None;
    }
    Some((
        v.idempotency_key.clone(),
        vec![
            post(Account::RewardsPayable, v.amount),
            post(Account::RewardExpense, Money(-v.amount.0)),
        ],
    ))
}
