//! Check a photo, screenshot or diagnostics against the prevention task; flag reuse or edits.
//! Evidence already used is flagged by a fixed check before any model call. A verdict the model
//! cannot give, or gives in an invalid form, goes to a person; it never becomes a fail (R15).
//! A pass is a recommendation: points follow only through `engine::prevention::complete`, and a
//! person audits a sample (R10, R14). Every message is in the customer's language.
use crate::client::{Image, Model};
use crate::harness::{Ask, Harness, Harnessed};
use crate::language::{LanguagePack, Languages};
use crate::settings::Tier;
use microinsurance_engine::prevention::{PreventionTaskDraft, Verdict};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;

pub const COMPONENT: &str = "verify";

/// Evidence as the customer sent it.
#[derive(Debug, Clone)]
pub struct Evidence {
    /// `image/*` is looked at; `application/json` and `text/*` (diagnostics) are read.
    pub media_type: String,
    pub bytes: Vec<u8>,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct VerifyReply {
    /// The evidence shows the action done, as the task describes.
    pub passed: bool,
    /// The evidence looks like it was taken earlier or for something else.
    pub reuse_suspected: bool,
    /// The evidence looks edited or generated.
    pub edited_suspected: bool,
    /// One plain sentence for the customer.
    pub reason: String,
}

/// What happens to the evidence.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum VerifyOutcome {
    Verdict(Verdict),
    /// A person checks it (no verdict could be given).
    ReferToPerson {
        reason: String,
    },
}

/// The fingerprint kept for each piece of evidence, so the same file cannot earn twice.
pub fn fingerprint(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

const IMAGE_TYPES: &[&str] = &["image/jpeg", "image/png", "image/gif", "image/webp"];

/// Images to look at and documents to read, or the media type that can be neither.
pub(crate) fn read(evidence: &[Evidence]) -> Result<(Vec<Image>, Vec<Value>), String> {
    let mut images = Vec::new();
    let mut documents: Vec<Value> = Vec::new();
    for e in evidence {
        let kind = e.media_type.to_ascii_lowercase();
        if IMAGE_TYPES.contains(&kind.as_str()) {
            images.push(Image {
                media_type: kind,
                bytes: e.bytes.clone(),
            });
        } else if kind == "application/json" || kind.starts_with("text/") {
            let text = String::from_utf8_lossy(&e.bytes).into_owned();
            documents.push(serde_json::from_str(&text).unwrap_or(Value::String(text)));
        } else {
            return Err(kind);
        }
    }
    Ok((images, documents))
}

pub fn check(
    reply: VerifyReply,
    languages: &Languages,
    pack: &LanguagePack,
) -> Result<VerifyOutcome, String> {
    let reason = reply.reason.trim().to_string();
    languages.check_sentence(pack, &reason, &[])?;
    let suspected = reply.reuse_suspected || reply.edited_suspected;
    Ok(VerifyOutcome::Verdict(Verdict {
        passed: reply.passed && !suspected,
        reason,
        reuse_suspected: suspected,
    }))
}

fn refer(reason: &str) -> VerifyOutcome {
    VerifyOutcome::ReferToPerson {
        reason: reason.into(),
    }
}

/// The verdict on `evidence` for `task`. `seen` holds the fingerprints of evidence already
/// submitted by anyone; `language` is the customer's setting.
pub async fn verdict<M: Model>(
    h: &Harness<M>,
    task: &PreventionTaskDraft,
    evidence: &[Evidence],
    seen: &BTreeSet<String>,
    language: Option<&str>,
) -> Harnessed<VerifyOutcome> {
    let pack = h.languages.pack(language);
    let said = &pack.saved.verify;
    if evidence.is_empty() {
        return Harnessed::rule(refer(&said.no_evidence), "no evidence");
    }
    if evidence
        .iter()
        .any(|e| seen.contains(&fingerprint(&e.bytes)))
    {
        return Harnessed::rule(
            VerifyOutcome::Verdict(Verdict {
                passed: false,
                reason: said.reused.clone(),
                reuse_suspected: true,
            }),
            "evidence fingerprint already seen",
        );
    }
    let (images, documents) = match read(evidence) {
        Ok(read) => read,
        Err(kind) => {
            return Harnessed::rule(
                refer(&said.unreadable),
                format!("evidence type {kind} cannot be read"),
            );
        }
    };
    let input = json!({
        "language": pack.tag,
        "task": {
            "action": task.action,
            "evidence_kind": task.evidence,
            "event_reference": task.event_reference,
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
        |r: VerifyReply| check(r, &h.languages, pack),
        || refer(&said.refer),
    )
    .await
}
