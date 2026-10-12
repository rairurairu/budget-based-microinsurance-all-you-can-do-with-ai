//! Parametric payout decisions. R51 two stages within the peril's event limit · R27 one payout per
//! (policy, loss occurrence, stage) · R16 cash always offered · R28 settlement in kind only where
//! suitable, permitted by the product terms, worth at least the cash it replaces and accepted ·
//! R53 a cash payout in the grace period is paid net of the premium due. Amounts are final cents;
//! the ledger never rounds (ADR-004).
use crate::config::{EngineConfig, Tariff};
use crate::money::Money;
use crate::triggers::{LossOccurrence, Match};
use crate::types::{CoverId, PolicyId};
use crate::{EngineError, Result};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "method", content = "option", rename_all = "snake_case")]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub enum Settlement {
    Cash,
    /// A registered alternative the cover's terms permit, such as `lounge_access`.
    InKind(String),
}

/// R51: the guaranteed share is paid at once; the balance when the loss occurrence closes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub enum PayoutStage {
    Guaranteed,
    Balance,
}

impl PayoutStage {
    fn as_str(self) -> &'static str {
        match self {
            PayoutStage::Guaranteed => "guaranteed",
            PayoutStage::Balance => "balance",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct PayoutLine {
    pub cover_id: CoverId,
    pub amount: Money,
}

/// One payout to one policy on one loss occurrence, at one stage (R27).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct PayoutDecision {
    pub idempotency_key: String,
    pub policy_id: PolicyId,
    pub occurrence_id: String,
    pub stage: PayoutStage,
    /// By cover id.
    pub lines: Vec<PayoutLine>,
    pub amount: Money,
    /// Cash first, always (R16); then alternatives every cover in the payout permits, that are
    /// suitable now and worth at least the cash they replace (R28).
    pub options: Vec<Settlement>,
}

/// R27: the ledger's idempotency key. A retried payout posts under the same key and is never
/// paid twice.
pub fn idempotency_key(policy_id: &str, occurrence_id: &str, stage: PayoutStage) -> String {
    format!("payout/{policy_id}/{occurrence_id}/{}", stage.as_str())
}

/// Payouts on one loss occurrence so far, carried by the caller from its opening to its close
/// (R51).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OccurrencePayouts {
    pub occurrence_id: String,
    /// The peril's `event_limit`: all payouts on the occurrence together.
    pub event_limit: Money,
    /// Benefit in force for the occurrence when it opened (`triggers::benefit_in_force`).
    pub benefit_in_force: Money,
    /// Guaranteed shares paid, by policy and cover.
    pub guaranteed: BTreeMap<PolicyId, BTreeMap<CoverId, Money>>,
}

impl OccurrencePayouts {
    /// Opens the payouts on an occurrence with its peril's event limit and the benefit in force,
    /// the sum of `Match::benefit` over every cover `match_occurrence` found eligible when it
    /// opened.
    pub fn open(
        occurrence: &LossOccurrence,
        benefit_in_force: Money,
        cfg: &EngineConfig,
    ) -> Result<OccurrencePayouts> {
        let event_limit = cfg
            .accumulation
            .perils
            .get(&occurrence.peril)
            .map(|p| p.event_limit)
            .ok_or_else(|| EngineError::Rule {
                rule: "R51",
                detail: format!("peril {} has no event limit", occurrence.peril),
            })?;
        Ok(OccurrencePayouts {
            occurrence_id: occurrence.occurrence_id.clone(),
            event_limit,
            benefit_in_force,
            guaranteed: BTreeMap::new(),
        })
    }

    pub fn guaranteed_total(&self) -> Money {
        self.guaranteed
            .values()
            .flat_map(|c| c.values())
            .copied()
            .sum()
    }

    /// The guaranteed share as a fraction `event_limit / benefit_in_force`, at most 1.
    fn share(&self, amount: Money) -> Money {
        if self.benefit_in_force <= self.event_limit {
            return amount;
        }
        let v = i128::from(amount.0) * i128::from(self.event_limit.0)
            / i128::from(self.benefit_in_force.0);
        Money(i64::try_from(v).unwrap_or(i64::MAX))
    }
}

/// R28: an in-kind option at a fixed tariff replaces a cash amount only if its retail value is
/// at least that amount and the tariff not above it; an invoiced option is charged at most the
/// amount, and the rest is paid in cash (R56).
pub fn fair_value(option: &str, amount: Money, cfg: &EngineConfig) -> bool {
    cfg.suppliers
        .in_kind
        .get(option)
        .is_some_and(|t| match t.tariff {
            Tariff::Fixed(tariff) => {
                tariff <= amount && t.retail_value.is_some_and(|v| v >= amount)
            }
            Tariff::Invoiced => true,
        })
}

/// The settlement options for an amount owed under these covers: cash, then every alternative
/// all their terms permit, that is suitable now and of fair value for the amount (R16, R28).
pub fn settlement_options(
    covers: &[&str],
    amount: Money,
    suitable: &BTreeSet<String>,
    cfg: &EngineConfig,
) -> Vec<Settlement> {
    let permitted: Option<BTreeSet<&String>> = covers
        .iter()
        .map(|c| {
            cfg.catalogue
                .get(*c)
                .map(|c| c.settlement_in_kind.iter().collect::<BTreeSet<_>>())
                .unwrap_or_default()
        })
        .reduce(|a, b| a.intersection(&b).copied().collect());
    let mut out = vec![Settlement::Cash];
    out.extend(
        permitted
            .unwrap_or_default()
            .into_iter()
            .filter(|o| suitable.contains(*o) && fair_value(o, amount, cfg))
            .map(|o| Settlement::InKind(o.clone())),
    );
    out
}

fn decision(
    policy_id: &str,
    occurrence_id: &str,
    stage: PayoutStage,
    lines: Vec<PayoutLine>,
    suitable: &BTreeSet<String>,
    cfg: &EngineConfig,
) -> PayoutDecision {
    PayoutDecision {
        idempotency_key: idempotency_key(policy_id, occurrence_id, stage),
        policy_id: policy_id.to_string(),
        occurrence_id: occurrence_id.to_string(),
        stage,
        amount: lines.iter().map(|l| l.amount).sum(),
        options: settlement_options(
            &lines
                .iter()
                .map(|l| l.cover_id.as_str())
                .collect::<Vec<_>>(),
            lines.iter().map(|l| l.amount).sum(),
            suitable,
            cfg,
        ),
        lines,
    }
}

/// R51 stage one, paid at once: for each policy that qualifies for the first time, the
/// guaranteed share (event limit ÷ benefit in force, at most 100%) of what it is owed, up to one
/// day's or one event's benefit, down to the cent. Each policy is paid this once per occurrence
/// (R27); everything else it is owed is paid in the balance. The guaranteed total never passes
/// the event limit.
pub fn guaranteed(
    book: &mut OccurrencePayouts,
    matches: &[Match],
    suitable: &BTreeSet<String>,
    cfg: &EngineConfig,
) -> Vec<PayoutDecision> {
    let mut by_policy: BTreeMap<&PolicyId, Vec<&Match>> = BTreeMap::new();
    for m in matches.iter().filter(|m| m.entitlement > Money::ZERO) {
        if m.occurrence_id == book.occurrence_id && !book.guaranteed.contains_key(&m.policy_id) {
            by_policy.entry(&m.policy_id).or_default().push(m);
        }
    }
    let mut room = book.event_limit - book.guaranteed_total();
    let mut out = Vec::new();
    for (policy_id, ms) in by_policy {
        let mut lines: Vec<PayoutLine> = ms
            .iter()
            .map(|m| {
                let amount = book
                    .share(m.entitlement.min(m.benefit))
                    .min(room)
                    .max(Money::ZERO);
                room = room - amount;
                PayoutLine {
                    cover_id: m.cover_id.clone(),
                    amount,
                }
            })
            .collect();
        lines.sort_by(|a, b| a.cover_id.cmp(&b.cover_id));
        book.guaranteed.insert(
            policy_id.clone(),
            lines
                .iter()
                .map(|l| (l.cover_id.clone(), l.amount))
                .collect(),
        );
        if lines.iter().any(|l| l.amount > Money::ZERO) {
            out.push(decision(
                policy_id,
                &book.occurrence_id,
                PayoutStage::Guaranteed,
                lines,
                suitable,
                cfg,
            ));
        }
    }
    out
}

/// R51 stage two, when the loss occurrence has closed: what each policy is owed in full less its
/// guaranteed share, reduced pro rata if the total would pass the event limit. Each reduced
/// amount is rounded down to the cent and the cents left go one each to the largest remainders,
/// ties by policy and cover id, so the total paid equals the limit exactly (ADR-004).
/// `matches` is `match_occurrence` as of the close.
pub fn balance(
    book: &OccurrencePayouts,
    occurrence: &LossOccurrence,
    matches: &[Match],
    suitable: &BTreeSet<String>,
    cfg: &EngineConfig,
) -> Result<Vec<PayoutDecision>> {
    if !occurrence.closed || occurrence.occurrence_id != book.occurrence_id {
        return Err(EngineError::Rule {
            rule: "R51",
            detail: format!("the balance is paid when {} closes", book.occurrence_id),
        });
    }
    let owed: Vec<(&PolicyId, &CoverId, i64)> = matches
        .iter()
        .filter(|m| m.occurrence_id == book.occurrence_id)
        .map(|m| {
            let paid = book
                .guaranteed
                .get(&m.policy_id)
                .and_then(|c| c.get(&m.cover_id))
                .copied()
                .unwrap_or(Money::ZERO);
            (&m.policy_id, &m.cover_id, (m.entitlement - paid).0.max(0))
        })
        .filter(|(_, _, r)| *r > 0)
        .collect();
    let total: i128 = owed.iter().map(|(_, _, r)| i128::from(*r)).sum();
    let room = i128::from((book.event_limit - book.guaranteed_total()).0.max(0));
    let amounts: Vec<i64> = if total <= room {
        owed.iter().map(|(_, _, r)| *r).collect()
    } else {
        largest_remainder(&owed, total, room)
    };
    let mut by_policy: BTreeMap<&PolicyId, Vec<PayoutLine>> = BTreeMap::new();
    for ((policy_id, cover_id, _), amount) in owed.iter().zip(amounts) {
        if amount > 0 {
            by_policy.entry(policy_id).or_default().push(PayoutLine {
                cover_id: (*cover_id).clone(),
                amount: Money(amount),
            });
        }
    }
    Ok(by_policy
        .into_iter()
        .map(|(policy_id, mut lines)| {
            lines.sort_by(|a, b| a.cover_id.cmp(&b.cover_id));
            decision(
                policy_id,
                &book.occurrence_id,
                PayoutStage::Balance,
                lines,
                suitable,
                cfg,
            )
        })
        .collect())
}

/// Each claim × room ÷ total, rounded down; the cents left one each to the largest remainders,
/// ties by policy and cover id.
fn largest_remainder(owed: &[(&PolicyId, &CoverId, i64)], total: i128, room: i128) -> Vec<i64> {
    let mut amounts: Vec<i64> = Vec::with_capacity(owed.len());
    let mut remainders: Vec<(i128, &PolicyId, &CoverId, usize)> = Vec::with_capacity(owed.len());
    for (i, (p, c, r)) in owed.iter().enumerate() {
        let scaled = i128::from(*r) * room;
        amounts.push(i64::try_from(scaled / total).unwrap_or(i64::MAX));
        remainders.push((scaled % total, p, c, i));
    }
    let given: i128 = amounts.iter().map(|a| i128::from(*a)).sum();
    let left = usize::try_from(room - given).unwrap_or(0);
    remainders.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(b.1)).then(a.2.cmp(b.2)));
    for (_, _, _, i) in remainders.into_iter().take(left) {
        if let Some(a) = amounts.get_mut(i) {
            *a += 1;
        }
    }
    amounts
}

/// How a payout was settled.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct Settled {
    pub idempotency_key: String,
    pub method: Settlement,
    pub amount: Money,
    /// Premium due set off against a cash payout in the grace period (R53).
    pub premium_set_off: Money,
    /// Paid to the customer; zero when settled in kind.
    pub cash: Money,
}

/// Settles a payout in cash unless the customer accepted one of its offered alternatives (R16,
/// R28). A cash payout in the grace period is paid net of the premium due (R53); settlement in
/// kind leaves the premium due unchanged.
pub fn settle(
    decision: &PayoutDecision,
    accepted: Option<&Settlement>,
    premium_due: Money,
) -> Result<Settled> {
    settle_amount(
        &decision.idempotency_key,
        decision.amount,
        &decision.options,
        accepted,
        premium_due,
    )
}

/// Settles an amount owed under the key `key` with the options it was offered: payouts here,
/// claim payments in `claims::settle`.
pub fn settle_amount(
    key: &str,
    amount: Money,
    options: &[Settlement],
    accepted: Option<&Settlement>,
    premium_due: Money,
) -> Result<Settled> {
    let method = match accepted {
        None | Some(Settlement::Cash) => Settlement::Cash,
        Some(choice) if options.contains(choice) => choice.clone(),
        Some(choice) => {
            return Err(EngineError::Rule {
                rule: "R28",
                detail: format!("{choice:?} was not offered on {key}"),
            });
        }
    };
    let set_off = match method {
        Settlement::Cash => premium_due.max(Money::ZERO).min(amount),
        Settlement::InKind(_) => Money::ZERO,
    };
    let cash = match method {
        Settlement::Cash => amount - set_off,
        Settlement::InKind(_) => Money::ZERO,
    };
    Ok(Settled {
        idempotency_key: format!("{key}/settled"),
        method,
        amount,
        premium_set_off: set_off,
        cash,
    })
}
