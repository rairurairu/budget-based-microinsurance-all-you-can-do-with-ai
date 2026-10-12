//! Event-specific prevention: identify a controllable cause of loss for a known event and propose
//! one action the customer can do and prove, for one of the insurer's registered loss prevention
//! measures for the cover's class (ADR-009). Only loss-based covers (R38): behaviour cannot move
//! a parametric trigger. The proposal names no amount; the engine sizes and admits it with the
//! measure's claims data (R18, R40). Evidence is limited to the measure's kinds the customer has
//! consented to (R20), so prevention works without location. The action is written in the
//! customer's language.
use crate::client::Model;
use crate::harness::{Ask, Harness, Harnessed};
use crate::language::{LanguagePack, Languages};
use crate::settings::Tier;
use crate::text;
use crate::vocab::Vocabulary;
use microinsurance_engine::config::EngineConfig;
use microinsurance_engine::prevention::PreventionTaskProposal;
use microinsurance_engine::types::{CoverType, Profile, UpcomingEvent};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::BTreeSet;

pub const COMPONENT: &str = "prevention_tasks";

#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PreventionReply {
    /// None when no action the customer controls would make a loss less likely.
    pub task: Option<PreventionTaskProposal>,
}

/// Evidence kinds this customer can send: those that are not optional data, and optional data
/// they opted into (R20).
pub fn allowed_evidence(profile: &Profile, vocab: &Vocabulary) -> Vec<String> {
    vocab
        .evidence_kinds
        .iter()
        .filter(|(_, data)| data.as_ref().is_none_or(|d| profile.consents.contains(d)))
        .map(|(k, _)| k.clone())
        .collect()
}

/// A registered measure the model may choose, with the evidence this customer can send for it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MeasureChoice {
    pub task_type: String,
    pub description: String,
    pub evidence: BTreeSet<String>,
}

/// The registered measures for `class` that this customer can prove with consented evidence.
pub fn measures(class: &str, allowed: &[String], cfg: &EngineConfig) -> Vec<MeasureChoice> {
    cfg.prevention
        .for_class(class)
        .map(|m| MeasureChoice {
            task_type: m.id.clone(),
            description: m.description.clone(),
            evidence: m
                .evidence
                .iter()
                .filter(|e| allowed.contains(e))
                .cloned()
                .collect(),
        })
        .filter(|m| !m.evidence.is_empty())
        .collect()
}

/// The checks a proposal must pass.
pub fn check(
    reply: PreventionReply,
    choices: &[MeasureChoice],
    languages: &Languages,
    pack: &LanguagePack,
) -> Result<Option<PreventionTaskProposal>, String> {
    let Some(mut task) = reply.task else {
        return Ok(None);
    };
    if !text::is_snake_id(&task.task_type) {
        return Err(format!(
            "task type {:?} is not a snake_case id",
            task.task_type
        ));
    }
    let Some(choice) = choices.iter().find(|m| m.task_type == task.task_type) else {
        return Err(format!(
            "R18: {:?} is not a registered measure for this cover",
            task.task_type
        ));
    };
    if !choice.evidence.contains(&task.evidence) {
        return Err(format!(
            "R20: evidence {:?} does not prove {} or is not consented",
            task.evidence, task.task_type
        ));
    }
    task.action = task.action.trim().to_string();
    languages.check_sentence(pack, &task.action, &[])?;
    Ok(Some(task))
}

/// One verifiable action for `event` that protects `cover_id`, or none. Without a valid
/// reply, no task is offered.
pub async fn propose<M: Model>(
    h: &Harness<M>,
    event: &UpcomingEvent,
    cover_id: &str,
    profile: &Profile,
    cfg: &EngineConfig,
    vocab: &Vocabulary,
) -> Harnessed<Option<PreventionTaskProposal>> {
    let cover = match cfg.cover(cover_id) {
        Ok(c) => c,
        Err(e) => return Harnessed::rule(None, e.to_string()),
    };
    if cover.cover_type == CoverType::Parametric {
        return Harnessed::rule(
            None,
            format!("R38: {cover_id} is parametric; prevention tasks target loss-based covers"),
        );
    }
    let allowed = allowed_evidence(profile, vocab);
    let choices = measures(&cover.class_of_business, &allowed, cfg);
    if choices.is_empty() {
        return Harnessed::rule(
            None,
            format!("no registered measure for {cover_id} the customer can prove"),
        );
    }
    let pack = h.languages.pack(profile.language.as_deref());
    let input = json!({
        "language": pack.tag,
        "event": { "kind": event.kind, "starts": event.starts, "ends": event.ends },
        "cover": {
            "cover_id": cover.id,
            "name": cover.name,
            "key_features": cover.key_features,
            "class_of_business": cover.class_of_business,
        },
        "measures": choices,
    });
    let ask = Ask {
        component: COMPONENT,
        tier: Tier::Strong,
        input,
        images: &[],
    };
    h.ask(
        ask,
        |r: PreventionReply| check(r, &choices, &h.languages, pack),
        || None,
    )
    .await
}
