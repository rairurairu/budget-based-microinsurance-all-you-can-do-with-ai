//! Supplier settlement (R28, R56, ADR-009). When a customer accepts settlement in kind or spends
//! their reward balance on a partner reward, an in-kind order goes to the supplier. The supplier
//! is paid only for what it delivered, on a monthly statement: the agreed tariff, or its invoice
//! up to the amount reserved. Anything not delivered by its expiry goes back to the customer, a
//! payout as cash and a reward to the reward balance.
use crate::config::{EngineConfig, SupplierId, Tariff};
use crate::money::Money;
use crate::payouts::{Settled, Settlement};
use crate::rewards::{Redeemed, Redemption};
use crate::{EngineError, Result};
use chrono::NaiveDate;
use serde::{Deserialize, Serialize};

/// What the order settles.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub enum OrderSource {
    /// A parametric payout the customer took in kind (R28).
    Payout,
    /// A partner reward bought with the reward balance (R16).
    Reward,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub enum OrderState {
    Issued,
    /// The supplier delivered. `charge` is what it is paid; `to_customer` the rest of an
    /// invoiced payout, paid in cash.
    Delivered {
        on: NaiveDate,
        charge: Money,
        to_customer: Money,
    },
    /// Not delivered in time: the amount reserved goes back to the customer.
    Expired {
        on: NaiveDate,
    },
}

/// An instruction to a supplier to deliver one in-kind option, with the amount reserved for it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct InKindOrder {
    /// The settled payout's or the redemption's idempotency key.
    pub order_id: String,
    pub supplier_id: SupplierId,
    /// The in-kind option or the partner reward's offer id.
    pub option: String,
    pub source: OrderSource,
    pub tariff: Tariff,
    pub reserved: Money,
    pub issued_on: NaiveDate,
    /// Last day it can be delivered, inclusive.
    pub expires_on: NaiveDate,
    pub state: OrderState,
}

fn r56(detail: String) -> EngineError {
    EngineError::Rule {
        rule: "R56",
        detail,
    }
}

fn expiry(issued_on: NaiveDate, valid_days: u32) -> NaiveDate {
    issued_on
        .checked_add_days(chrono::Days::new(u64::from(valid_days.saturating_sub(1))))
        .unwrap_or(NaiveDate::MAX)
}

/// The order for a payout settled in kind, to the option's supplier on its terms.
pub fn order_for_payout(
    settled: &Settled,
    issued_on: NaiveDate,
    cfg: &EngineConfig,
) -> Result<InKindOrder> {
    let Settlement::InKind(option) = &settled.method else {
        return Err(r56(format!(
            "{} was settled in cash",
            settled.idempotency_key
        )));
    };
    let terms = cfg
        .suppliers
        .in_kind
        .get(option)
        .ok_or_else(|| r56(format!("no supplier delivers {option}")))?;
    Ok(InKindOrder {
        order_id: settled.idempotency_key.clone(),
        supplier_id: terms.supplier_id.clone(),
        option: option.clone(),
        source: OrderSource::Payout,
        tariff: terms.tariff,
        reserved: settled.amount,
        issued_on,
        expires_on: expiry(issued_on, terms.valid_days),
        state: OrderState::Issued,
    })
}

/// The order for a partner reward bought with the reward balance, at its price.
pub fn order_for_reward(
    redemption: &Redemption,
    issued_on: NaiveDate,
    cfg: &EngineConfig,
) -> Result<InKindOrder> {
    let Redeemed::InKind { supplier_id } = &redemption.method else {
        return Err(r56(format!(
            "{} was taken in cash",
            redemption.redemption_id
        )));
    };
    let valid_days = cfg
        .suppliers
        .reward_supplier(supplier_id)
        .and_then(|s| s.reward_valid_days)
        .ok_or_else(|| r56(format!("{supplier_id} is not registered for rewards")))?;
    Ok(InKindOrder {
        order_id: redemption.redemption_id.clone(),
        supplier_id: supplier_id.clone(),
        option: redemption.offer_id.clone(),
        source: OrderSource::Reward,
        tariff: Tariff::Fixed(redemption.amount),
        reserved: redemption.amount,
        issued_on,
        expires_on: expiry(issued_on, valid_days),
        state: OrderState::Issued,
    })
}

/// The supplier confirms delivery by the expiry. A fixed tariff is charged as agreed; an
/// invoiced option is charged its invoice, which must not pass the amount reserved, and for a
/// payout the rest is paid to the customer in cash.
pub fn deliver(
    mut order: InKindOrder,
    on: NaiveDate,
    invoice: Option<Money>,
) -> Result<InKindOrder> {
    if order.state != OrderState::Issued {
        return Err(EngineError::State(format!(
            "{} was already {:?}",
            order.order_id, order.state
        )));
    }
    if on < order.issued_on || on > order.expires_on {
        return Err(r56(format!(
            "{} could be delivered from {} to {}",
            order.order_id, order.issued_on, order.expires_on
        )));
    }
    let charge = match (order.tariff, invoice) {
        (Tariff::Fixed(t), None) => t.min(order.reserved),
        (Tariff::Invoiced, Some(i)) if Money::ZERO <= i && i <= order.reserved => i,
        (Tariff::Invoiced, Some(i)) => {
            return Err(r56(format!(
                "{}: invoice {} cents is outside 0 to the {} cents reserved",
                order.order_id, i.0, order.reserved.0
            )));
        }
        (Tariff::Fixed(_), Some(_)) => {
            return Err(r56(format!(
                "{} is at a fixed tariff, not invoiced",
                order.order_id
            )));
        }
        (Tariff::Invoiced, None) => {
            return Err(r56(format!(
                "{} needs the supplier's invoice",
                order.order_id
            )));
        }
    };
    let to_customer = match (order.source, order.tariff) {
        (OrderSource::Payout, Tariff::Invoiced) => order.reserved - charge,
        _ => Money::ZERO,
    };
    order.state = OrderState::Delivered {
        on,
        charge,
        to_customer,
    };
    Ok(order)
}

/// An order not delivered by its expiry expires: nothing is paid to the supplier and the amount
/// reserved goes back to the customer.
pub fn expire(mut order: InKindOrder, today: NaiveDate) -> InKindOrder {
    if order.state == OrderState::Issued && today > order.expires_on {
        order.state = OrderState::Expired { on: today };
    }
    order
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct StatementLine {
    pub order_id: String,
    pub option: String,
    pub delivered_on: NaiveDate,
    pub charge: Money,
}

/// What the insurer pays one supplier for what it delivered in a period.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct SupplierStatement {
    /// The ledger's idempotency key: one statement per supplier and period.
    pub statement_id: String,
    pub supplier_id: SupplierId,
    pub from: NaiveDate,
    /// First day after the period.
    pub until: NaiveDate,
    /// By delivery date, then order id.
    pub lines: Vec<StatementLine>,
    pub total: Money,
}

/// R56: the supplier's statement for deliveries in `from..until`.
pub fn statement(
    supplier_id: &str,
    from: NaiveDate,
    until: NaiveDate,
    orders: &[InKindOrder],
) -> SupplierStatement {
    let mut lines: Vec<StatementLine> = orders
        .iter()
        .filter(|o| o.supplier_id == supplier_id)
        .filter_map(|o| match o.state {
            OrderState::Delivered { on, charge, .. } if from <= on && on < until => {
                Some(StatementLine {
                    order_id: o.order_id.clone(),
                    option: o.option.clone(),
                    delivered_on: on,
                    charge,
                })
            }
            _ => None,
        })
        .collect();
    lines.sort_by(|a, b| {
        a.delivered_on
            .cmp(&b.delivered_on)
            .then(a.order_id.cmp(&b.order_id))
    });
    lines.dedup_by(|a, b| a.order_id == b.order_id);
    SupplierStatement {
        statement_id: format!("supplier/{supplier_id}/{from}"),
        supplier_id: supplier_id.to_string(),
        from,
        until,
        total: lines.iter().map(|l| l.charge).sum(),
        lines,
    }
}
