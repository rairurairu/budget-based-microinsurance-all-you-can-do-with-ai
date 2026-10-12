//! The claim assessor: reads a small indemnity claim's photos and documents and recommends
//! approve or refer, never decline (R13): the reply type has no decline, so a reply that tries
//! one fails the schema and the claim goes to a person (R22). Its estimate of the loss is a check
//! on the amount claimed, never the amount paid; the engine decides the claim (`claims::decide`,
//! R57). The model never sees the amount claimed, the sum insured or the premium, so it cannot
//! anchor on them (ADR-007, ADR-010). It may escalate a case as urgent or needing human care,
//! never the reverse (R46). Evidence already used is caught by fingerprint before any model call.
use crate::client::Model;
use crate::harness::{Ask, Harness, Harnessed};
use crate::language::{LanguagePack, Languages};
use crate::settings::Tier;
use crate::verify::{self, Evidence, fingerprint};
use microinsurance_engine::claims::{Assessment, Claim, Recommendation, Review};
use microinsurance_engine::config::EngineConfig;
use microinsurance_engine::guardrails::human_care::Escalation;
use microinsurance_engine::money::{Bps, Money};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::BTreeSet;

pub const COMPONENT: &str = "assess";

#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AssessReply {
    /// `approve` when the evidence clearly shows an insured loss of the kind described, otherwise
    /// `refer`. There is no decline: people decide declines.
    pub recommend: Recommendation,
    /// The loss the evidence shows, in whole units of the market currency.
    pub estimate: u32,
    /// How sure the assessment is, from 0 to 100.
    pub confidence: u8,
    /// The evidence fits the description and the loss date.
    pub consistent: bool,
    /// An injury, a hospital stay, an emergency abroad or other urgent need.
    pub urgent: bool,
    /// Signs of distress or vulnerability: a person should take the case.
    pub human_care: bool,
    /// One plain sentence for the customer saying what was seen.
    pub reason: String,
}

/// What happens to the claim.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum AssessOutcome {
    Assessed {
        assessment: Assessment,
        reason: String,
    },
    /// A person assesses it: no assessment could be made.
    ReferToPerson {
        reason: String,
        /// The fingerprint check found evidence sent before (R14 anomaly indicator).
        evidence_reused: bool,
    },
}

impl AssessOutcome {
    /// What the engine decides on.
    pub fn review(&self) -> Review {
        match self {
            AssessOutcome::Assessed { assessment, .. } => Review::Assessed(*assessment),
            AssessOutcome::ReferToPerson { .. } => Review::NoAssessment,
        }
    }

    pub fn evidence_reused(&self) -> bool {
        matches!(
            self,
            AssessOutcome::ReferToPerson {
                evidence_reused: true,
                ..
            }
        )
    }

    /// The sentence the customer reads.
    pub fn reason(&self) -> &str {
        match self {
            AssessOutcome::Assessed { reason, .. }
            | AssessOutcome::ReferToPerson { reason, .. } => reason,
        }
    }
}

/// The fixed checks: a confidence in range, an approval with a loss, one plain sentence in the
/// customer's language with no amount (R02, R23). A reply saying the evidence does not fit is
/// never an approval.
pub fn check(
    reply: AssessReply,
    languages: &Languages,
    pack: &LanguagePack,
) -> Result<AssessOutcome, String> {
    if reply.confidence > 100 {
        return Err(format!("confidence {} is above 100", reply.confidence));
    }
    let recommend = if reply.consistent {
        reply.recommend
    } else {
        Recommendation::Refer
    };
    if recommend == Recommendation::Approve && reply.estimate == 0 {
        return Err("an approval with no loss estimated".into());
    }
    let reason = reply.reason.trim().to_string();
    languages.check_sentence(pack, &reason, &[])?;
    Ok(AssessOutcome::Assessed {
        assessment: Assessment {
            estimate: Money::from_major(i64::from(reply.estimate)),
            confidence: Bps(u32::from(reply.confidence) * 100),
            recommend,
            escalation: Escalation {
                urgent: reply.urgent,
                human_care: reply.human_care,
            },
        },
        reason,
    })
}

fn refer(reason: &str, evidence_reused: bool) -> AssessOutcome {
    AssessOutcome::ReferToPerson {
        reason: reason.into(),
        evidence_reused,
    }
}

/// The assessment of `claim` from its `evidence` and the customer's own `description`. `seen`
/// holds the fingerprints of evidence already sent by anyone; `language` is the customer's
/// setting.
pub async fn assess<M: Model>(
    h: &Harness<M>,
    claim: &Claim,
    description: &str,
    evidence: &[Evidence],
    seen: &BTreeSet<String>,
    language: Option<&str>,
    cfg: &EngineConfig,
) -> Harnessed<AssessOutcome> {
    let pack = h.languages.pack(language);
    let said = &pack.saved.assess;
    if evidence.is_empty() {
        return Harnessed::rule(refer(&said.no_evidence, false), "no evidence");
    }
    if evidence
        .iter()
        .any(|e| seen.contains(&fingerprint(&e.bytes)))
    {
        return Harnessed::rule(
            refer(&said.reused, true),
            "evidence fingerprint already seen",
        );
    }
    let (images, documents) = match verify::read(evidence) {
        Ok(read) => read,
        Err(kind) => {
            return Harnessed::rule(
                refer(&said.unreadable, false),
                format!("evidence type {kind} cannot be read"),
            );
        }
    };
    let cover = cfg.cover(&claim.cover_id).ok();
    let input = json!({
        "language": pack.tag,
        "currency": cfg.currency,
        "cover": {
            "name": cover.map(|c| c.name.as_str()),
            "class_of_business": claim.class_of_business,
            "evidence_kinds": cover.map(|c| &c.claim_evidence),
        },
        "claim": {
            "loss_date": claim.loss_date,
            "circumstance": claim.circumstance,
            "description": description,
            "evidence_sent": claim.evidence,
        },
        "images": images.len(),
        "documents": documents,
    });
    let ask = Ask {
        component: COMPONENT,
        tier: Tier::Strong,
        input,
        images: &images,
    };
    h.ask(
        ask,
        |r: AssessReply| check(r, &h.languages, pack),
        || refer(&said.refer, false),
    )
    .await
}
