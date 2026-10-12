//! The pilot (docs/07-pilot-and-metrics.md, ADR-012): its dates, the date its gates were fixed,
//! and its wait-list control group (R59). The gates themselves are evaluated outside the engine
//! (`microinsurance_economics.pilot_evaluation`); the engine checks they were fixed before the
//! pilot started and decides who is in the control group.
//!
//! R59: a share of the customers bound during the pilot, drawn from the customer id and the
//! insurer's recorded seed, is offered no prevention tasks for their first `control.periods`
//! policy periods, then joins; everything else is the same. Their claims are the comparison for
//! the claims the prevention tasks avoid.
use crate::config::decimal::DecBps;
use crate::guardrails::audit::should_audit;
use crate::money::Bps;
use crate::{EngineError, Result};
use chrono::NaiveDate;
use serde::{Deserialize, Serialize};

/// The wait-list control group.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ControlGroup {
    /// The share of customers bound during the pilot drawn into it; below 100%.
    pub share: Bps,
    /// Policy periods from binding without prevention tasks.
    pub periods: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PilotConfig {
    pub id: String,
    pub starts: NaiveDate,
    pub ends: NaiveDate,
    /// The gates were fixed this day, before the pilot started.
    pub gates_fixed_on: NaiveDate,
    pub control: Option<ControlGroup>,
}

/// Which group a customer is in for prevention tasks (R59).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub enum Cohort {
    /// Offered prevention tasks: everyone outside a pilot's control group.
    Offered,
    /// In the wait-list control group: no prevention tasks yet.
    WaitList,
}

#[derive(Deserialize)]
struct RawFile {
    pilot: RawPilot,
}

#[derive(Deserialize)]
struct RawPilot {
    id: String,
    starts: NaiveDate,
    ends: NaiveDate,
    gates_fixed_on: NaiveDate,
    #[serde(default)]
    control: Option<RawControl>,
}

#[derive(Deserialize)]
struct RawControl {
    share: DecBps,
    periods: u32,
}

fn r59(detail: String) -> EngineError {
    EngineError::Rule {
        rule: "R59",
        detail,
    }
}

impl PilotConfig {
    /// `pilot.yaml`; its `gates` are read by whoever evaluates them.
    pub(crate) fn parse(text: &str) -> Result<PilotConfig> {
        let raw: RawFile = serde_saphyr::from_str(text)
            .map_err(|e| EngineError::Config(format!("pilot.yaml: {e}")))?;
        let p = raw.pilot;
        if p.ends < p.starts {
            return Err(EngineError::Config(format!(
                "pilot {}: it ends before it starts",
                p.id
            )));
        }
        if p.gates_fixed_on >= p.starts {
            return Err(r59(format!(
                "pilot {}: gates fixed on {} must be fixed before it starts on {}",
                p.id, p.gates_fixed_on, p.starts
            )));
        }
        let control = match p.control {
            None => None,
            Some(c) if c.share.0 >= Bps::ONE => {
                return Err(r59(format!(
                    "pilot {}: the control group must leave customers to compare it with",
                    p.id
                )));
            }
            Some(c) if c.share.0 > Bps::ZERO && c.periods == 0 => {
                return Err(r59(format!(
                    "pilot {}: a control group waits at least one policy period",
                    p.id
                )));
            }
            Some(c) => Some(ControlGroup {
                share: c.share.0,
                periods: c.periods,
            }),
        };
        Ok(PilotConfig {
            id: p.id,
            starts: p.starts,
            ends: p.ends,
            gates_fixed_on: p.gates_fixed_on,
            control,
        })
    }
}

/// R59: the customer's group in `period_index` (from 1) of a policy bound on `bound_on`, drawn
/// from their id and the insurer's seed, so it can be replayed and nobody can choose it.
pub fn cohort(
    pilot: Option<&PilotConfig>,
    customer_id: &str,
    bound_on: NaiveDate,
    period_index: u32,
    seed: u64,
) -> Cohort {
    let Some(p) = pilot else {
        return Cohort::Offered;
    };
    let Some(control) = p.control else {
        return Cohort::Offered;
    };
    let during = p.starts <= bound_on && bound_on <= p.ends;
    if during
        && period_index <= control.periods
        && should_audit(
            &format!("pilot-control/{}/{customer_id}", p.id),
            control.share,
            seed,
        )
    {
        Cohort::WaitList
    } else {
        Cohort::Offered
    }
}
