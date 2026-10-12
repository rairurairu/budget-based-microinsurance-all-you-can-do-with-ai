//! R15: any decision the AI layer or the rules made can be put to a person on the customer's
//! request (a review request), and so can a person's adverse decision, such as a claim decline:
//! then a different person reviews it (four eyes, as in internal dispute resolution). A person
//! decides every review request within `appeal_sla_hours`. The customer's own choices are not
//! reviewed: they change them themselves.
use super::{Actor, DecisionRecord};
use crate::config::GuardrailConfig;
use crate::{EngineError, Result};
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub enum AppealState {
    Open,
    /// The decision stands.
    Upheld {
        reviewer: String,
        on: DateTime<Utc>,
        note: String,
    },
    /// The decision is reversed; the subject goes back to a person to decide afresh.
    Overturned {
        reviewer: String,
        on: DateTime<Utc>,
        note: String,
    },
}

/// One review request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct Appeal {
    pub appeal_id: String,
    /// The decision reviewed, from the decision log.
    pub decision_id: String,
    pub subject_type: String,
    pub subject_id: String,
    pub customer_id: String,
    /// Who made the decision.
    pub decided_by: Actor,
    /// The person whose decision is reviewed: they may not review it themselves.
    pub original_reviewer: Option<String>,
    /// In the customer's words.
    pub reason: String,
    pub opened_at: DateTime<Utc>,
    pub due_at: DateTime<Utc>,
    pub state: AppealState,
}

/// Opens a review request on a logged decision about the customer (R15), due within
/// `appeal_sla_hours`. One per decision: the id is the decision's.
pub fn open(
    decision: &DecisionRecord,
    customer_id: &str,
    reason: &str,
    now: DateTime<Utc>,
    cfg: &GuardrailConfig,
) -> Result<Appeal> {
    if decision.actor == Actor::Customer {
        return Err(EngineError::Rule {
            rule: "R15",
            detail: format!(
                "{} is the customer's own choice; they change it themselves",
                decision.decision_id
            ),
        });
    }
    if decision.customer_id != customer_id {
        return Err(EngineError::State(format!(
            "{} is not a decision about {customer_id}",
            decision.decision_id
        )));
    }
    Ok(Appeal {
        appeal_id: format!("appeal/{}", decision.decision_id),
        decision_id: decision.decision_id.clone(),
        subject_type: decision.subject_type.clone(),
        subject_id: decision.subject_id.clone(),
        customer_id: customer_id.to_string(),
        decided_by: decision.actor,
        original_reviewer: decision.reviewer.clone(),
        reason: reason.to_string(),
        opened_at: now,
        due_at: now + Duration::hours(i64::from(cfg.appeal_sla_hours)),
        state: AppealState::Open,
    })
}

/// A person's decision on an open review request: upheld, or overturned. Never the person whose
/// decision is reviewed.
pub fn decide(
    mut appeal: Appeal,
    reviewer: &str,
    overturn: bool,
    note: &str,
    now: DateTime<Utc>,
) -> Result<Appeal> {
    if appeal.state != AppealState::Open {
        return Err(EngineError::State(format!(
            "{} is already decided",
            appeal.appeal_id
        )));
    }
    if reviewer.trim().is_empty() || appeal.original_reviewer.as_deref() == Some(reviewer) {
        return Err(EngineError::Rule {
            rule: "R15",
            detail: format!(
                "{} needs a person other than the one who decided",
                appeal.appeal_id
            ),
        });
    }
    let (reviewer, note) = (reviewer.to_string(), note.to_string());
    appeal.state = if overturn {
        AppealState::Overturned {
            reviewer,
            on: now,
            note,
        }
    } else {
        AppealState::Upheld {
            reviewer,
            on: now,
            note,
        }
    };
    Ok(appeal)
}

/// Open past its due time.
pub fn overdue(appeal: &Appeal, now: DateTime<Utc>) -> bool {
    appeal.state == AppealState::Open && now > appeal.due_at
}
