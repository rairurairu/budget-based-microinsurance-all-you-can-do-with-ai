//! Greedy allocation in `allocation_step` increments. Each step goes to the cover where it adds
//! the most usable sum insured: cover up to the customer's need, since a first-loss cover pays no
//! more than the loss (R48). The value is concave per cover, so the best step each time is optimal
//! for this separable objective. With `fallback`, premium no usable cover can take still goes
//! where the plan's limits allow (covers with exposure first, then the most cover per HK$), so
//! premium always buys cover. Ties go to the lowest cover id; integer arithmetic only (R02, R29).
use crate::money::{Money, Rate};
use crate::types::CoverId;
use std::collections::BTreeMap;

#[derive(Debug, Clone)]
pub(crate) struct Candidate {
    pub cover_id: CoverId,
    pub rate: Rate,
    /// The sum insured this customer can use this period; zero means no exposure.
    pub need: Money,
    /// Premium already placed that this fill keeps.
    pub floor: Money,
    /// The most premium the cover may take (R06, R09, R34, R41).
    pub ceiling: Money,
}

pub(crate) struct Filled {
    pub premiums: BTreeMap<CoverId, Money>,
    /// Budget not placed.
    pub left: Money,
    /// Premium placed by the fallback, beyond what the customer can use now.
    pub beyond_need: Money,
}

/// Places `budget` in `step` increments on top of each candidate's floor (candidates in cover id
/// order).
pub(crate) fn fill(candidates: &[Candidate], budget: Money, step: Money, fallback: bool) -> Filled {
    let mut premiums: BTreeMap<CoverId, Money> = candidates
        .iter()
        .map(|c| (c.cover_id.clone(), c.floor))
        .collect();
    let mut left = budget;
    let mut beyond_need = Money::ZERO;
    while left >= step {
        // Key: (usable, has exposure, sum insured gained); usable steps always come first.
        let mut best: Option<(&Candidate, (bool, bool, Money))> = None;
        for c in candidates {
            let p = premiums.get(&c.cover_id).copied().unwrap_or(Money::ZERO);
            if p + step > c.ceiling {
                continue;
            }
            let before = c.rate.sum_insured(p);
            let after = c.rate.sum_insured(p + step);
            let usable = after.min(c.need) - before.min(c.need);
            let key = if usable > Money::ZERO {
                (true, true, usable)
            } else if fallback {
                (false, c.need > Money::ZERO, after - before)
            } else {
                continue;
            };
            // Strictly better only, so the lowest cover id (iteration order) wins ties.
            if best.as_ref().is_none_or(|(_, b)| key > *b) {
                best = Some((c, key));
            }
        }
        let Some((chosen, (usable, _, _))) = best else {
            break;
        };
        if let Some(p) = premiums.get_mut(&chosen.cover_id) {
            *p += step;
        }
        if !usable {
            beyond_need += step;
        }
        left = left - step;
    }
    Filled {
        premiums,
        left,
        beyond_need,
    }
}
