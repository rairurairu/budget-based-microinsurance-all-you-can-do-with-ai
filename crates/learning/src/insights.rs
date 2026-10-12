//! Partners receive consented, aggregated insights only (R21). No individual-level export path
//! exists: what a partner sees is a count or a rate over a cohort of customers, and a figure
//! from fewer than `min_cohort` customers (`channels.partner_insights_min_cohort` in the insurer
//! profile) is withheld, not rounded or noised.
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SegmentRow {
    pub segment: String,
    pub metric: String,
    pub value_bps: u32,
    pub cohort_size: u32,
}

pub fn suppress_small_cohorts(rows: Vec<SegmentRow>, min_cohort: u32) -> Vec<SegmentRow> {
    rows.into_iter()
        .filter(|r| r.cohort_size >= min_cohort)
        .collect()
}

/// What one customer adds to a partner's results. It carries no identity: the caller builds one
/// per customer and the partner only ever sees the sums.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CustomerFacts {
    /// Came through this partner's channel (R52).
    pub through_partner: bool,
    /// Was shown a quote (R49).
    pub quoted: bool,
    /// Holds a policy.
    pub bound: bool,
    /// The policy is in force today.
    pub in_force: bool,
    /// Their first policy period has ended, so they could have renewed.
    pub due_to_renew: bool,
    /// They hold a second policy period or later.
    pub renewed: bool,
    pub tasks_offered: u32,
    pub tasks_verified: u32,
    pub payouts: u32,
    /// Payouts decided, in minor units, as the engine decided them.
    pub payout_minor: i64,
    /// Rewards this customer took from the partner's own offers.
    pub redemptions_with_partner: u32,
    /// Rewards granted to them and taken from their reward balance, any supplier's.
    pub rewards_granted: u32,
    pub rewards_redeemed: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub enum Unit {
    Customers,
    Count,
    /// Basis points: 10,000 is 100%.
    Bps,
    /// Minor units of the market's currency.
    Minor,
    Hours,
}

/// One aggregate. `value` is withheld when the cohort is too small (R21).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct Figure {
    pub metric: String,
    pub unit: Unit,
    pub value: Option<i64>,
    /// Customers the figure is computed over.
    pub cohort_size: u32,
}

/// A distribution partner's results: aggregates over the customers who came through it, and
/// over everyone who took one of its rewards.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct PartnerResults {
    pub partner_id: String,
    pub min_cohort: u32,
    pub figures: Vec<Figure>,
}

fn count<T: Into<u64>>(values: impl Iterator<Item = T>) -> i64 {
    i64::try_from(values.map(Into::into).sum::<u64>()).unwrap_or(i64::MAX)
}

fn cohort(n: usize) -> u32 {
    u32::try_from(n).unwrap_or(u32::MAX)
}

/// The partner's results, each figure withheld below `min_cohort` customers (R21).
pub fn partner_results(
    partner_id: &str,
    facts: &[CustomerFacts],
    min_cohort: u32,
) -> PartnerResults {
    let figure = |metric: &str, unit: Unit, value: i64, size: usize| {
        let cohort_size = cohort(size);
        Figure {
            metric: metric.to_string(),
            unit,
            value: (cohort_size >= min_cohort.max(1)).then_some(value),
            cohort_size,
        }
    };
    let mine: Vec<&CustomerFacts> = facts.iter().filter(|f| f.through_partner).collect();
    let with_tasks: Vec<&&CustomerFacts> = mine.iter().filter(|f| f.tasks_offered > 0).collect();
    let offered = count(with_tasks.iter().map(|f| f.tasks_offered));
    let verified = count(with_tasks.iter().map(|f| f.tasks_verified));
    let completion = if offered > 0 {
        verified.saturating_mul(10_000) / offered
    } else {
        0
    };
    let paid: Vec<&&CustomerFacts> = mine.iter().filter(|f| f.payouts > 0).collect();
    let redeemers: Vec<&CustomerFacts> = facts
        .iter()
        .filter(|f| f.redemptions_with_partner > 0)
        .collect();
    let quoted: Vec<&&CustomerFacts> = mine.iter().filter(|f| f.quoted).collect();
    let due: Vec<&&CustomerFacts> = mine.iter().filter(|f| f.due_to_renew).collect();
    let granted: Vec<&&CustomerFacts> = mine.iter().filter(|f| f.rewards_granted > 0).collect();
    let rate = |n: i64, d: i64| {
        if d > 0 {
            n.saturating_mul(10_000) / d
        } else {
            0
        }
    };
    PartnerResults {
        partner_id: partner_id.to_string(),
        min_cohort,
        figures: vec![
            figure(
                "subscribers",
                Unit::Customers,
                i64::try_from(mine.len()).unwrap_or(i64::MAX),
                mine.len(),
            ),
            figure(
                "in_force",
                Unit::Customers,
                count(mine.iter().map(|f| u32::from(f.in_force))),
                mine.len(),
            ),
            figure(
                "prevention_task_completion",
                Unit::Bps,
                completion,
                with_tasks.len(),
            ),
            figure(
                "payouts",
                Unit::Count,
                count(paid.iter().map(|f| f.payouts)),
                paid.len(),
            ),
            figure(
                "payouts_amount",
                Unit::Minor,
                paid.iter().map(|f| f.payout_minor).sum(),
                paid.len(),
            ),
            figure(
                "redemptions",
                Unit::Count,
                count(redeemers.iter().map(|f| f.redemptions_with_partner)),
                redeemers.len(),
            ),
            // The funnel and retention (ADR-012): quoted, bound from a quote, renewed past the
            // first period, rewards taken from the balance.
            figure(
                "quoted",
                Unit::Customers,
                i64::try_from(quoted.len()).unwrap_or(i64::MAX),
                mine.len(),
            ),
            figure(
                "conversion",
                Unit::Bps,
                rate(
                    count(quoted.iter().map(|f| u32::from(f.bound))),
                    i64::try_from(quoted.len()).unwrap_or(i64::MAX),
                ),
                quoted.len(),
            ),
            figure(
                "renewal",
                Unit::Bps,
                rate(
                    count(due.iter().map(|f| u32::from(f.renewed))),
                    i64::try_from(due.len()).unwrap_or(i64::MAX),
                ),
                due.len(),
            ),
            figure(
                "reward_redemption",
                Unit::Bps,
                rate(
                    count(granted.iter().map(|f| f.rewards_redeemed)),
                    count(granted.iter().map(|f| f.rewards_granted)),
                ),
                granted.len(),
            ),
        ],
    }
}
