//! The people's work queue, as the ops console shows it (ADR-010): claims referred to a person,
//! prevention tasks with a person, decisions drawn for audit after payment and review requests.
//! Each item carries the actions a person may take on it, decided here: only a claim referral
//! offers a decline (R13), only an audit offers agree or overturn (R14), only a review request
//! offers uphold (R15). Urgent cases first (R46), then by due time.
use super::appeals::{Appeal, AppealState};
use super::audit::{AuditReason, AuditStats};
use super::fairness::ParityReport;
use crate::claims::{Claim, ClaimReferral, ClaimState};
use crate::config::EngineConfig;
use crate::money::Money;
use crate::prevention::{self, PreventionTask, TaskState};
use crate::types::CoverId;
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};

/// What a person can do with an item.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub enum PersonAction {
    /// Approve a claim at the loss the person assessed.
    Approve,
    /// Decline a claim, with a reason (R13).
    Decline,
    RequestEvidence,
    /// Credit a prevention task (R10).
    Verify,
    NotVerify,
    /// Agree with an audited decision (R14).
    Agree,
    /// Overturn an audited decision, or a decision under review (R14, R15).
    Overturn,
    /// Let a decision under review stand (R15).
    Uphold,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub enum WorkKind {
    /// A claim with a person: the only place a decline is made (R13).
    ClaimReferral {
        claim_id: String,
        cover_id: CoverId,
        amount_claimed: Money,
        referral: ClaimReferral,
    },
    /// A prevention task with a person (R10, R14, R15).
    TaskReferral {
        task_id: String,
        referral: prevention::Referral,
    },
    /// A decision drawn for audit after the fact, such as a payout already paid (R14).
    Audit {
        decision_id: String,
        decision_kind: String,
        reason: AuditReason,
    },
    /// A review request on any other decision (R15).
    Appeal {
        appeal_id: String,
        subject_type: String,
        subject_id: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct WorkItem {
    pub item_id: String,
    pub customer_id: String,
    pub opened_at: DateTime<Utc>,
    pub due_at: DateTime<Utc>,
    pub urgent: bool,
    pub work: WorkKind,
    pub actions: Vec<PersonAction>,
}

/// The actions a person may take on this kind of item.
pub fn actions(work: &WorkKind) -> Vec<PersonAction> {
    use PersonAction::*;
    match work {
        WorkKind::ClaimReferral { .. } => vec![Approve, Decline, RequestEvidence],
        WorkKind::TaskReferral { .. } => vec![Verify, NotVerify],
        WorkKind::Audit { .. } => vec![Agree, Overturn],
        WorkKind::Appeal { .. } => vec![Uphold, Overturn],
    }
}

fn item(
    item_id: String,
    customer_id: &str,
    opened_at: DateTime<Utc>,
    due_at: DateTime<Utc>,
    urgent: bool,
    work: WorkKind,
) -> WorkItem {
    WorkItem {
        item_id,
        customer_id: customer_id.to_string(),
        opened_at,
        due_at,
        urgent,
        actions: actions(&work),
        work,
    }
}

/// A claim with a person, if it is.
pub fn claim_item(claim: &Claim) -> Option<WorkItem> {
    let ClaimState::WithPerson {
        since,
        due,
        referral,
    } = &claim.state
    else {
        return None;
    };
    Some(item(
        claim.claim_id.clone(),
        &claim.customer_id,
        *since,
        *due,
        claim.urgent,
        WorkKind::ClaimReferral {
            claim_id: claim.claim_id.clone(),
            cover_id: claim.cover_id.clone(),
            amount_claimed: claim.amount_claimed,
            referral: referral.clone(),
        },
    ))
}

/// A prevention task with a person, if it is, due `referral_sla_hours` after `opened_at`.
pub fn task_item(
    task: &PreventionTask,
    opened_at: DateTime<Utc>,
    cfg: &EngineConfig,
) -> Option<WorkItem> {
    let TaskState::WithPerson { referral, .. } = &task.state else {
        return None;
    };
    Some(item(
        task.task_id.clone(),
        &task.customer_id,
        opened_at,
        opened_at + Duration::hours(i64::from(cfg.claims.referral_sla_hours)),
        false,
        WorkKind::TaskReferral {
            task_id: task.task_id.clone(),
            referral: *referral,
        },
    ))
}

/// A decision drawn for audit after the fact (R14), due `referral_sla_hours` after `opened_at`.
pub fn audit_item(
    decision_id: &str,
    decision_kind: &str,
    customer_id: &str,
    reason: AuditReason,
    opened_at: DateTime<Utc>,
    cfg: &EngineConfig,
) -> WorkItem {
    item(
        format!("audit/{decision_id}"),
        customer_id,
        opened_at,
        opened_at + Duration::hours(i64::from(cfg.claims.referral_sla_hours)),
        false,
        WorkKind::Audit {
            decision_id: decision_id.to_string(),
            decision_kind: decision_kind.to_string(),
            reason,
        },
    )
}

/// An open review request (R15). A declined claim under review is a claim referral instead
/// (`claims::appeal`).
pub fn appeal_item(appeal: &Appeal) -> Option<WorkItem> {
    if appeal.state != AppealState::Open {
        return None;
    }
    Some(item(
        appeal.appeal_id.clone(),
        &appeal.customer_id,
        appeal.opened_at,
        appeal.due_at,
        false,
        WorkKind::Appeal {
            appeal_id: appeal.appeal_id.clone(),
            subject_type: appeal.subject_type.clone(),
            subject_id: appeal.subject_id.clone(),
        },
    ))
}

/// Urgent first (R46), then by due time, then by id.
pub fn order(items: &mut [WorkItem]) {
    items.sort_by(|a, b| {
        b.urgent
            .cmp(&a.urgent)
            .then(a.due_at.cmp(&b.due_at))
            .then(a.item_id.cmp(&b.item_id))
    });
}

/// What the ops console shows: the queue in order, the fairness audit and the audit overturn
/// rates.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct OpsSnapshot {
    pub as_of: DateTime<Utc>,
    pub items: Vec<WorkItem>,
    pub fairness: Vec<ParityReport>,
    pub audit: Vec<AuditStats>,
}

impl OpsSnapshot {
    pub fn new(
        as_of: DateTime<Utc>,
        mut items: Vec<WorkItem>,
        fairness: Vec<ParityReport>,
        audit: Vec<AuditStats>,
    ) -> OpsSnapshot {
        order(&mut items);
        OpsSnapshot {
            as_of,
            items,
            fairness,
            audit,
        }
    }
}
