//! Fairness audit (R19): for each kind of decision and each audit attribute, the approval rate of
//! every group, compared as an adverse impact ratio: a credible group whose rate is below
//! `min_parity_ratio` of the best credible group's is flagged for a person to investigate (the
//! four-fifths rule at 0.8). Group attributes come from a separate audit store; they never reach
//! the optimiser's rating outside the registered factors, the AI or any model. A flag is a finding
//! for people, never an automatic change to any decision.
use crate::config::FairnessConfig;
use crate::money::Bps;
use crate::{EngineError, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Decisions and approvals by group.
type Counts<'a> = BTreeMap<&'a str, (u32, u32)>;

/// One final decision about one customer, labelled with one audit group of theirs. A customer
/// appears once per attribute.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Observation {
    /// The decision kind, such as `claim` or `prevention_verification`.
    pub kind: String,
    /// One of `guardrails.fairness.groups`, such as `age_band`.
    pub attribute: String,
    pub group: String,
    /// Approved or verified, by the AI layer or a person, in the end.
    pub approved: bool,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct GroupRate {
    pub decisions: u32,
    pub approved: u32,
    pub rate: Bps,
    /// Enough decisions (`min_group_decisions`) to be compared.
    pub credible: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct ParityReport {
    pub kind: String,
    pub attribute: String,
    pub groups: BTreeMap<String, GroupRate>,
    /// The lowest credible rate over the highest; none with fewer than two credible groups.
    pub min_ratio: Option<Bps>,
    /// Credible groups below `min_parity_ratio` of the best.
    pub flagged: Vec<String>,
}

/// One report per decision kind and audit attribute, in that order. An attribute the insurer
/// has not configured is refused: the audit store holds only what the insurer declared.
pub fn parity_reports<'a>(
    observations: impl IntoIterator<Item = &'a Observation>,
    cfg: &FairnessConfig,
) -> Result<Vec<ParityReport>> {
    let mut counts: BTreeMap<(&str, &str), Counts> = BTreeMap::new();
    for o in observations {
        if !cfg.groups.contains(&o.attribute) {
            return Err(EngineError::Rule {
                rule: "R19",
                detail: format!("{} is not a configured fairness group", o.attribute),
            });
        }
        let c = counts
            .entry((&o.kind, &o.attribute))
            .or_default()
            .entry(&o.group)
            .or_default();
        c.0 += 1;
        c.1 += u32::from(o.approved);
    }
    Ok(counts
        .into_iter()
        .map(|((kind, attribute), groups)| report(kind, attribute, &groups, cfg))
        .collect())
}

fn report(kind: &str, attribute: &str, counts: &Counts, cfg: &FairnessConfig) -> ParityReport {
    let groups: BTreeMap<String, GroupRate> = counts
        .iter()
        .map(|(g, (decisions, approved))| {
            (
                (*g).to_string(),
                GroupRate {
                    decisions: *decisions,
                    approved: *approved,
                    rate: Bps::ratio(u64::from(*approved), u64::from(*decisions)),
                    credible: *decisions >= cfg.min_group_decisions,
                },
            )
        })
        .collect();
    let credible: Vec<(&String, &GroupRate)> = groups.iter().filter(|(_, r)| r.credible).collect();
    let best = credible.iter().map(|(_, r)| r.rate).max();
    let worst = credible.iter().map(|(_, r)| r.rate).min();
    let min_ratio = match (best, worst) {
        // Every credible group at zero is parity too.
        (Some(b), _) if credible.len() >= 2 && b == Bps::ZERO => Some(Bps::ONE),
        (Some(b), Some(w)) if credible.len() >= 2 => {
            Some(Bps::ratio(u64::from(w.0), u64::from(b.0)))
        }
        _ => None,
    };
    let flagged = match best {
        Some(b) if credible.len() >= 2 => credible
            .iter()
            // rate < min_parity_ratio × best, in integers
            .filter(|(_, r)| {
                u64::from(r.rate.0) * u64::from(Bps::ONE.0)
                    < u64::from(b.0) * u64::from(cfg.min_parity_ratio.0)
            })
            .map(|(g, _)| (*g).clone())
            .collect(),
        _ => Vec::new(),
    };
    ParityReport {
        kind: kind.to_string(),
        attribute: attribute.to_string(),
        groups,
        min_ratio,
        flagged,
    }
}
