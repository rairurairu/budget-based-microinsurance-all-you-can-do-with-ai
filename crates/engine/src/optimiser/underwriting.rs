//! Re-underwriting each policy period, per class of business (R39), and postponement (R41,
//! ADR-003) with the re-entry waiting period after it (R47). A rule-based underwriting decision,
//! never an AI one; the reason is explained (R23) and can be reviewed by a person (R15).
//!
//! The policy is a monthly subscription: it renews every period and only cancellation ends it.
//! Postponement never stops the policy; it excludes one class of business from the schedule, with
//! advance notice. Each period's terms come from that period's re-underwriting alone, so a notice
//! announces an exclusion but never fixes it: if risk has fallen by then, nothing is excluded.
use crate::config::{EngineConfig, RiskClass};
use crate::types::add_policy_periods;
use chrono::{Days, NaiveDate};
use serde::{Deserialize, Serialize};

/// Where a customer stands in one class of business.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub enum ClassStatus {
    /// Premium may be allocated. After a postponement, losses before `reentry_waiting_until`
    /// are not covered (R47).
    Open {
        reentry_waiting_until: Option<NaiveDate>,
    },
    /// The customer has been told the class will be excluded from `excluded_from`. Cover in it
    /// continues until then, without increase.
    ExclusionNotice { excluded_from: NaiveDate },
    /// Excluded: no premium can be allocated before `until`; then the risk class is reassessed.
    Postponed { until: NaiveDate },
}

impl Default for ClassStatus {
    fn default() -> ClassStatus {
        ClassStatus::Open {
            reentry_waiting_until: None,
        }
    }
}

/// How much premium a class may take this policy period (R41).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AllocationRight {
    Any,
    /// Existing cover runs on until the exclusion; no increase.
    NoIncrease,
    None,
}

impl ClassStatus {
    pub fn allocation_right(&self) -> AllocationRight {
        match self {
            ClassStatus::Open { .. } => AllocationRight::Any,
            ClassStatus::ExclusionNotice { .. } => AllocationRight::NoIncrease,
            ClassStatus::Postponed { .. } => AllocationRight::None,
        }
    }
}

/// The status for the policy period starting on `period_start`, given the status before it and
/// this period's risk class. Changes happen only at a period start, so cover already in force
/// runs to the end of its period (R04, R41).
pub fn review(
    previous: &ClassStatus,
    risk: &RiskClass,
    period_start: NaiveDate,
    cfg: &EngineConfig,
) -> ClassStatus {
    let uw = &cfg.underwriting;
    let postponed_from = |starts: NaiveDate| ClassStatus::Postponed {
        until: add_policy_periods(starts, uw.postponement_periods),
    };
    match *previous {
        ClassStatus::Open { .. } if risk.is_postponed() => {
            if uw.exclusion_notice_periods == 0 {
                postponed_from(period_start)
            } else {
                ClassStatus::ExclusionNotice {
                    excluded_from: add_policy_periods(period_start, uw.exclusion_notice_periods),
                }
            }
        }
        ClassStatus::Open {
            reentry_waiting_until,
        } => ClassStatus::Open {
            reentry_waiting_until: reentry_waiting_until.filter(|d| *d > period_start),
        },
        // This period's re-underwriting no longer supports the exclusion: nothing to apply.
        ClassStatus::ExclusionNotice { .. } if !risk.is_postponed() => ClassStatus::default(),
        ClassStatus::ExclusionNotice { excluded_from } if period_start >= excluded_from => {
            postponed_from(period_start)
        }
        notice @ ClassStatus::ExclusionNotice { .. } => notice,
        postponed @ ClassStatus::Postponed { until } if period_start < until => postponed,
        // Postponement over: reassessed now.
        ClassStatus::Postponed { .. } if risk.is_postponed() => postponed_from(period_start),
        ClassStatus::Postponed { .. } => ClassStatus::Open {
            reentry_waiting_until: Some(
                period_start
                    .checked_add_days(Days::new(u64::from(uw.reentry_waiting_period_days)))
                    .unwrap_or(NaiveDate::MAX),
            ),
        },
    }
}
