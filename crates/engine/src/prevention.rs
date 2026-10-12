//! M6 Prevention task lifecycle (deterministic side). The AI proposes the action for a
//! registered loss prevention measure; the engine sizes the reward from the class's claims data
//! (R18, R36, R40), decides whether it may be offered, and carries the task from offer to
//! verified activity (R10), with a person on every adverse or audited step (R14, R15; ADR-009).
//!
//! Reference pattern for agents: `admit` is a rule implemented as a pure function that returns
//! `EngineError::Rule` with the rule id, with tests in tests/rules.rs.
use crate::config::EngineConfig;
use crate::guardrails::audit::{self, AuditReason};
use crate::money::{Bps, Money};
use crate::pilot::Cohort;
use crate::rewards::{EarnedFor, RewardEarned};
use crate::score::VerifiedActivity;
use crate::types::{ClassId, CoverId, CoverType, CustomerId, PolicyId, Profile};
use crate::{EngineError, Result};
use chrono::NaiveDate;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct PreventionTaskDraft {
    /// A registered loss prevention measure (`config/prevention_measures.yaml`).
    pub task_type: String,
    pub cover_id: CoverId,
    pub event_reference: Option<String>,
    /// What the customer does, in one sentence.
    pub action: String,
    /// A registered evidence kind (`config/data_types.yaml`).
    pub evidence: String,
    /// This customer's claim probability per policy period without and with the action (R40).
    pub claim_prob_before: Bps,
    pub claim_prob_after: Bps,
    /// Average claim cost in the class, at most the customer's sum insured (R48).
    pub avg_claim_cost: Money,
    pub reward_cost: Money,
}

/// What the AI prevention task writer proposes for a known event: the action and its evidence
/// for a registered measure, never an amount (R02). `size` turns it into a `PreventionTaskDraft`
/// with the claims data of the cover's class of business (R18, R36, R40).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct PreventionTaskProposal {
    /// A registered loss prevention measure for the cover's class, such as
    /// `screen_protector_fitted`.
    pub task_type: String,
    /// What the customer does, in one sentence (R23).
    pub action: String,
    /// One of the measure's evidence kinds the customer has consented to (R20).
    pub evidence: String,
}

/// The AI verifier's verdict, already parsed into this type.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct Verdict {
    pub passed: bool,
    pub reason: String,
    pub reuse_suspected: bool,
}

/// Claims data of one measure's class of business for a policy period, from the insurer's claims
/// experience (`python/economics`). Market data, like the rating basis; never set by AI (R02,
/// R18, R36).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PreventionBasis {
    /// By measure id.
    pub measures: BTreeMap<String, MeasureClaims>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct MeasureClaims {
    /// Portfolio claim probability per policy period without the measure, and with it.
    pub claim_prob_without: Bps,
    pub claim_prob_with: Bps,
    pub avg_claim_cost: Money,
}

/// Expected claim cost avoided per customer per policy period.
pub fn claims_avoided(p: &PreventionTaskDraft) -> Money {
    let delta = i64::from(p.claim_prob_before.0) - i64::from(p.claim_prob_after.0);
    Money(delta * p.avg_claim_cost.0 / i64::from(Bps::ONE.0))
}

/// R18: a prevention task may be offered only if its reward costs less than the claims it avoids.
pub fn admit(p: &PreventionTaskDraft) -> Result<()> {
    let avoided = claims_avoided(p);
    if p.reward_cost < avoided {
        Ok(())
    } else {
        Err(EngineError::Rule {
            rule: "R18",
            detail: format!(
                "reward {} cents is not below claims avoided {} cents",
                p.reward_cost.0, avoided.0
            ),
        })
    }
}

fn rule(rule: &'static str, detail: String) -> EngineError {
    EngineError::Rule { rule, detail }
}

/// The cover a task protects, as the customer holds it this policy period.
#[derive(Debug, Clone, Copy)]
pub struct HeldCover<'a> {
    pub cover_id: &'a str,
    pub sum_insured: Money,
    /// The customer's rated relativity in the cover's class (`CoverRating::relativity`, R39).
    pub relativity: Bps,
}

/// One measure's numbers for one customer (R40).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CustomerTerms {
    pub claim_prob_before: Bps,
    pub claim_prob_after: Bps,
    pub avg_claim_cost: Money,
    pub claims_avoided: Money,
    pub reward: Money,
}

/// R40: the measure's claim probabilities times the customer's relativity, at most 100%; the
/// average claim cost, at most the sum insured (R48); the reward `share` of the claims avoided,
/// rounded down to the cent.
pub fn customer_terms(
    claims: &MeasureClaims,
    relativity: Bps,
    sum_insured: Money,
    share: Bps,
) -> CustomerTerms {
    let before = (claims.claim_prob_without * relativity).min(Bps::ONE);
    let after = (claims.claim_prob_with * relativity).min(before);
    let avg_claim_cost = claims.avg_claim_cost.min(sum_insured).max(Money::ZERO);
    let delta = i128::from(before.0) - i128::from(after.0);
    let avoided = delta * i128::from(avg_claim_cost.0) / i128::from(Bps::ONE.0);
    let reward = avoided * i128::from(share.0) / i128::from(Bps::ONE.0);
    CustomerTerms {
        claim_prob_before: before,
        claim_prob_after: after,
        avg_claim_cost,
        claims_avoided: Money(i64::try_from(avoided).unwrap_or(0)),
        reward: Money(i64::try_from(reward).unwrap_or(0)),
    }
}

/// Sizes a proposal for this customer (R40): the measure's claim probabilities times the
/// customer's rated relativity in the class (R39), at most 100%; the class's average claim cost,
/// at most the customer's sum insured (first loss, R48); the reward is
/// `reward_share_of_claims_avoided` of the claims avoided, rounded down to the cent, and must be
/// below them (R18). Only loss-based covers (R38), only a registered measure of the cover's own
/// class (R18, R36), only its evidence and only evidence the customer consented to (R10, R20).
pub fn size(
    proposal: &PreventionTaskProposal,
    held: HeldCover<'_>,
    event_reference: Option<&str>,
    profile: &Profile,
    basis: &PreventionBasis,
    cfg: &EngineConfig,
) -> Result<PreventionTaskDraft> {
    let HeldCover {
        cover_id,
        sum_insured,
        relativity,
    } = held;
    let cover = cfg.cover(cover_id)?;
    if cover.cover_type == CoverType::Parametric {
        return Err(rule(
            "R38",
            format!("{cover_id} is parametric; prevention tasks target loss-based covers"),
        ));
    }
    let measure = cfg.prevention.active(&proposal.task_type).ok_or_else(|| {
        rule(
            "R18",
            format!(
                "{} is not a registered prevention measure",
                proposal.task_type
            ),
        )
    })?;
    if measure.class_of_business != cover.class_of_business {
        return Err(rule(
            "R36",
            format!(
                "measure {} is for {}, not {}",
                measure.id, measure.class_of_business, cover.class_of_business
            ),
        ));
    }
    if !measure.evidence.contains(&proposal.evidence) {
        return Err(rule(
            "R10",
            format!("{} is not evidence of {}", proposal.evidence, measure.id),
        ));
    }
    if let Some(Some(data)) = cfg.data_types.evidence_kinds.get(&proposal.evidence)
        && !profile.consents.contains(data)
    {
        return Err(rule(
            "R20",
            format!("evidence {} needs consent to {data}", proposal.evidence),
        ));
    }
    if sum_insured <= Money::ZERO {
        return Err(rule(
            "R40",
            format!("no cover in force in {cover_id}: no claims to avoid"),
        ));
    }
    let claims = basis.measures.get(&measure.id).ok_or_else(|| {
        EngineError::Config(format!(
            "prevention basis has no claims data for {}",
            measure.id
        ))
    })?;
    let terms = customer_terms(
        claims,
        relativity,
        sum_insured,
        cfg.rewards.reward_share_of_claims_avoided,
    );
    let draft = PreventionTaskDraft {
        task_type: measure.id.clone(),
        cover_id: cover.id.clone(),
        event_reference: event_reference.map(str::to_string),
        action: proposal.action.clone(),
        evidence: proposal.evidence.clone(),
        claim_prob_before: terms.claim_prob_before,
        claim_prob_after: terms.claim_prob_after,
        avg_claim_cost: terms.avg_claim_cost,
        reward_cost: terms.reward,
    };
    admit(&draft)?;
    Ok(draft)
}

/// Points only on a verified pass with no suspected reuse (R10), in the class of business of the
/// cover the task protects (R36). Self-reported completion never reaches this function, and this
/// is the only way a `VerifiedActivity` is made.
pub fn complete(
    task: &PreventionTaskDraft,
    verdict: &Verdict,
    on: NaiveDate,
    cfg: &EngineConfig,
) -> Option<VerifiedActivity> {
    if !verdict.passed || verdict.reuse_suspected {
        return None;
    }
    let cover = cfg.cover(&task.cover_id).ok()?;
    Some(VerifiedActivity::new(
        on,
        cfg.score.points_per_verified_activity,
        cover.class_of_business.clone(),
        task.task_type.clone(),
    ))
}

// ---- lifecycle

/// Why a task is with a person.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub enum Referral {
    /// The verifier gave no verdict (R22).
    NoVerdict,
    /// The evidence looks reused or edited: only a person decides fraud (R13).
    SuspectedReuse,
    /// A pass drawn for audit before it is credited (R14).
    Audit(AuditReason),
    /// The customer asked a person to look again (R15).
    Appeal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub enum VerifiedBy {
    Verifier,
    Person,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub enum TaskState {
    Offered,
    /// The customer tapped to take it on.
    Accepted {
        on: NaiveDate,
    },
    /// Evidence sent, by fingerprint.
    Submitted {
        on: NaiveDate,
        evidence: Vec<String>,
    },
    WithPerson {
        on: NaiveDate,
        referral: Referral,
    },
    /// Points and the reward are credited (R10).
    Verified {
        on: NaiveDate,
        by: VerifiedBy,
    },
    /// Can be sent again before the due date, or appealed (R15).
    NotVerified {
        on: NaiveDate,
        reason: String,
    },
    /// Not sent by the due date.
    Expired {
        on: NaiveDate,
    },
}

/// One prevention task offered to one customer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct PreventionTask {
    pub task_id: String,
    pub customer_id: CustomerId,
    pub policy_id: PolicyId,
    pub class_of_business: ClassId,
    pub draft: PreventionTaskDraft,
    pub offered_on: NaiveDate,
    /// Last day evidence can be sent: the event's start, or the policy period's last day.
    pub due: NaiveDate,
    pub state: TaskState,
}

/// What a verified task credits: points (R10, R11) and its reward (R40), both in the class's
/// points pool (R36).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Completion {
    pub activity: VerifiedActivity,
    pub reward: RewardEarned,
}

/// What the verifier returned for submitted evidence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Review {
    Verdict(Verdict),
    /// No verdict could be given; a person looks (R22).
    NoVerdict,
}

fn state_error(task: &PreventionTask, step: &str) -> EngineError {
    EngineError::State(format!(
        "{} cannot {step} from {:?}",
        task.task_id, task.state
    ))
}

/// The task id: one per customer, measure and event (or offer day, for a task without one).
pub fn task_id(customer_id: &str, draft: &PreventionTaskDraft, offered_on: NaiveDate) -> String {
    match &draft.event_reference {
        Some(r) => format!("task/{customer_id}/{}/{r}", draft.task_type),
        None => format!("task/{customer_id}/{}/{offered_on}", draft.task_type),
    }
}

/// Offers a sized task (R18) unless the customer verified the same measure within its
/// `repeat_after_days` (R10), or is on a pilot's wait list (`pilot::cohort`, R59). `history` is
/// the customer's verified activities.
#[allow(clippy::too_many_arguments)]
pub fn offer(
    draft: PreventionTaskDraft,
    customer_id: &str,
    policy_id: &str,
    offered_on: NaiveDate,
    due: NaiveDate,
    history: &[VerifiedActivity],
    cohort: Cohort,
    cfg: &EngineConfig,
) -> Result<PreventionTask> {
    if cohort == Cohort::WaitList {
        return Err(rule(
            "R59",
            format!(
                "{customer_id} is in the pilot's wait-list control group: no prevention tasks yet"
            ),
        ));
    }
    admit(&draft)?;
    let measure = cfg.prevention.active(&draft.task_type).ok_or_else(|| {
        rule(
            "R18",
            format!("{} is not a registered measure", draft.task_type),
        )
    })?;
    let window = chrono::Days::new(u64::from(measure.repeat_after_days));
    if let Some(last) = history
        .iter()
        .filter(|a| a.task_type() == measure.id && a.on() <= offered_on)
        .map(VerifiedActivity::on)
        .max()
        && last
            .checked_add_days(window)
            .is_some_and(|until| offered_on < until)
    {
        return Err(rule(
            "R10",
            format!(
                "{} was verified on {last}; it earns again after {} days",
                measure.id, measure.repeat_after_days
            ),
        ));
    }
    if due < offered_on {
        return Err(EngineError::State(format!(
            "due date {due} is before the offer on {offered_on}"
        )));
    }
    Ok(PreventionTask {
        task_id: task_id(customer_id, &draft, offered_on),
        customer_id: customer_id.to_string(),
        policy_id: policy_id.to_string(),
        class_of_business: measure.class_of_business.clone(),
        draft,
        offered_on,
        due,
        state: TaskState::Offered,
    })
}

/// The customer's tap to take the task on (R03), by the due date.
pub fn accept(mut task: PreventionTask, on: NaiveDate) -> Result<PreventionTask> {
    if task.state != TaskState::Offered || on > task.due {
        return Err(state_error(&task, "be accepted"));
    }
    task.state = TaskState::Accepted { on };
    Ok(task)
}

/// Evidence sent, by fingerprint, by the due date. A task not verified can be sent again.
pub fn submit(
    mut task: PreventionTask,
    on: NaiveDate,
    evidence: Vec<String>,
) -> Result<PreventionTask> {
    let open = matches!(
        task.state,
        TaskState::Accepted { .. } | TaskState::NotVerified { .. }
    );
    if !open {
        return Err(state_error(&task, "take evidence"));
    }
    if on > task.due {
        return Err(rule(
            "R10",
            format!("{} was due by {}", task.task_id, task.due),
        ));
    }
    if evidence.is_empty() {
        return Err(rule("R10", format!("{}: no evidence sent", task.task_id)));
    }
    task.state = TaskState::Submitted { on, evidence };
    Ok(task)
}

fn credit(task: &PreventionTask, on: NaiveDate, cfg: &EngineConfig) -> Result<Completion> {
    let pass = Verdict {
        passed: true,
        reason: String::new(),
        reuse_suspected: false,
    };
    let activity = complete(&task.draft, &pass, on, cfg)
        .ok_or_else(|| EngineError::Config(format!("unknown cover {}", task.draft.cover_id)))?;
    Ok(Completion {
        reward: RewardEarned {
            reward_id: task.task_id.clone(),
            customer_id: task.customer_id.clone(),
            pool: cfg.lines.pool_of(&task.class_of_business),
            earned_for: EarnedFor::VerifiedActivity {
                task_id: task.task_id.clone(),
            },
            amount: task.draft.reward_cost,
            on,
        },
        activity,
    })
}

/// Records the verifier's review of submitted evidence. A pass is credited at once unless it is
/// drawn for audit (R14: random sample, or `anomaly` at the risk review threshold), when a
/// person confirms it first; suspected reuse and no verdict go to a person; a fail can be sent
/// again or appealed. `seed` is the audit seed (`guardrails::audit`).
pub fn review(
    mut task: PreventionTask,
    outcome: Review,
    anomaly: Option<Bps>,
    on: NaiveDate,
    seed: u64,
    cfg: &EngineConfig,
) -> Result<(PreventionTask, Option<Completion>)> {
    if !matches!(task.state, TaskState::Submitted { .. }) {
        return Err(state_error(&task, "be reviewed"));
    }
    let verdict = match outcome {
        Review::NoVerdict => {
            task.state = TaskState::WithPerson {
                on,
                referral: Referral::NoVerdict,
            };
            return Ok((task, None));
        }
        Review::Verdict(v) => v,
    };
    if verdict.reuse_suspected {
        task.state = TaskState::WithPerson {
            on,
            referral: Referral::SuspectedReuse,
        };
        return Ok((task, None));
    }
    if !verdict.passed {
        task.state = TaskState::NotVerified {
            on,
            reason: verdict.reason,
        };
        return Ok((task, None));
    }
    if let Some(reason) = audit::select(&task.task_id, anomaly, seed, &cfg.guardrails) {
        task.state = TaskState::WithPerson {
            on,
            referral: Referral::Audit(reason),
        };
        return Ok((task, None));
    }
    let done = credit(&task, on, cfg)?;
    task.state = TaskState::Verified {
        on,
        by: VerifiedBy::Verifier,
    };
    Ok((task, Some(done)))
}

/// A person's decision on a task with them: verified, or not verified with the reason.
pub fn person_decides(
    mut task: PreventionTask,
    verified: bool,
    reason: &str,
    on: NaiveDate,
    cfg: &EngineConfig,
) -> Result<(PreventionTask, Option<Completion>)> {
    if !matches!(task.state, TaskState::WithPerson { .. }) {
        return Err(state_error(&task, "take a person's decision"));
    }
    if !verified {
        task.state = TaskState::NotVerified {
            on,
            reason: reason.to_string(),
        };
        return Ok((task, None));
    }
    let done = credit(&task, on, cfg)?;
    task.state = TaskState::Verified {
        on,
        by: VerifiedBy::Person,
    };
    Ok((task, Some(done)))
}

/// R15: a task not verified goes to a person on the customer's request.
pub fn appeal(mut task: PreventionTask, on: NaiveDate) -> Result<PreventionTask> {
    if !matches!(task.state, TaskState::NotVerified { .. }) {
        return Err(state_error(&task, "be appealed"));
    }
    task.state = TaskState::WithPerson {
        on,
        referral: Referral::Appeal,
    };
    Ok(task)
}

/// A task the customer has not sent evidence for expires after its due date. Evidence already
/// sent is always decided.
pub fn expire(mut task: PreventionTask, today: NaiveDate) -> PreventionTask {
    let waiting = matches!(
        task.state,
        TaskState::Offered | TaskState::Accepted { .. } | TaskState::NotVerified { .. }
    );
    if waiting && today > task.due {
        task.state = TaskState::Expired { on: today };
    }
    task
}
