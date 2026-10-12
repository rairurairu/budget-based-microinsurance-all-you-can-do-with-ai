//! Write a short proposal form for the channel the customer came from, and map typed answers to
//! profile facts in registered ids. The form never asks for a protected attribute (R19); the
//! facts are only what the customer said, checked against the registries (R55). The form is in
//! the customer's language setting; its saved answer is the language pack's form.
use crate::client::Model;
use crate::harness::{Ask, Harness, Harnessed};
use crate::language::{LanguagePack, Languages, MAX_QUESTIONS};
use crate::settings::Tier;
use crate::vocab::Vocabulary;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;

pub const FORM: &str = "understand_form";
pub const FACTS: &str = "understand_facts";

const MAX_OCCUPATION_CHARS: usize = 60;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ProposalQuestion {
    /// snake_case, unique in the form.
    pub id: String,
    /// One plain question.
    pub text: String,
    /// Answers to tap; empty for a typed answer.
    pub options: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ProposalForm {
    pub channel: String,
    /// The language tag the form is written in.
    pub language: String,
    pub questions: Vec<ProposalQuestion>,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FormReply {
    pub questions: Vec<ProposalQuestion>,
}

/// Profile facts in registered ids, for the customer to confirm.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ProfileFacts {
    /// In the customer's words, short.
    pub occupation: Option<String>,
    /// Declared item kinds (`config/exposure_types.yaml`), such as `device.phone`.
    pub devices: Vec<String>,
    /// Declared route ids, such as `mtr.tsuen_wan`.
    pub commute_routes: Vec<String>,
}

/// The saved form: the language pack's short form, the same on every channel (R22).
pub fn fallback_form(channel: &str, pack: &LanguagePack) -> ProposalForm {
    ProposalForm {
        channel: channel.into(),
        language: pack.tag.clone(),
        questions: pack.saved.proposal_form.clone(),
    }
}

/// The checks a model's form must pass.
pub fn check_form(
    languages: &Languages,
    pack: &LanguagePack,
    channel: &str,
    reply: FormReply,
) -> Result<ProposalForm, String> {
    languages.check_questions(pack, &reply.questions)?;
    Ok(ProposalForm {
        channel: channel.into(),
        language: pack.tag.clone(),
        questions: reply.questions,
    })
}

/// A short proposal form for `channel` (for example `delivery_app`, `ticketing`, `direct`), in
/// the language the customer chose (`language`), or the market default.
pub async fn proposal_form<M: Model>(
    h: &Harness<M>,
    channel: &str,
    language: Option<&str>,
    vocab: &Vocabulary,
) -> Harnessed<ProposalForm> {
    let pack = h.languages.pack(language);
    let input = json!({
        "channel": channel,
        "language": pack.tag,
        "max_questions": MAX_QUESTIONS,
        "declared_item_kinds": vocab.declared_item_kinds,
        "upcoming_event_kinds": vocab.upcoming_event_kinds,
    });
    let ask = Ask {
        component: FORM,
        tier: Tier::Fast,
        input,
        images: &[],
    };
    h.ask(
        ask,
        |r: FormReply| check_form(&h.languages, pack, channel, r),
        || fallback_form(channel, pack),
    )
    .await
}

/// The checks the mapped facts must pass: registered ids only, nothing protected.
pub fn check_facts(
    reply: ProfileFacts,
    vocab: &Vocabulary,
    languages: &Languages,
) -> Result<ProfileFacts, String> {
    let mut devices = Vec::new();
    for d in reply.devices {
        if !vocab.declared_item_kinds.contains(&d) {
            return Err(format!("R55: {d:?} is not a registered declared item kind"));
        }
        if !devices.contains(&d) {
            devices.push(d);
        }
    }
    let mut routes = Vec::new();
    for r in reply.commute_routes {
        if !vocab.routes.contains(&r) {
            return Err(format!("R55: {r:?} is not a registered route"));
        }
        if !routes.contains(&r) {
            routes.push(r);
        }
    }
    let occupation = match reply.occupation.map(|o| o.trim().to_string()) {
        Some(o) if o.is_empty() => None,
        Some(o)
            if o.chars().count() > MAX_OCCUPATION_CHARS
                || languages.mentions_protected(&o)
                || languages.states_money(&o, &[]) =>
        {
            return Err(format!("occupation {o:?} is not allowed"));
        }
        other => other,
    };
    Ok(ProfileFacts {
        occupation,
        devices,
        commute_routes: routes,
    })
}

/// The customer's answers to `form` as profile facts, for the customer to confirm. Without a
/// valid reply, nothing is mapped and the customer fills the profile in themselves.
pub async fn map_answers<M: Model>(
    h: &Harness<M>,
    form: &ProposalForm,
    answers: &serde_json::Value,
    vocab: &Vocabulary,
) -> Harnessed<ProfileFacts> {
    let input = json!({
        "questions": form.questions,
        "answers": answers,
        "declared_item_kinds": vocab.declared_item_kinds,
        "routes": vocab.routes,
    });
    let ask = Ask {
        component: FACTS,
        tier: Tier::Fast,
        input,
        images: &[],
    };
    h.ask(
        ask,
        |r: ProfileFacts| check_facts(r, vocab, &h.languages),
        ProfileFacts::default,
    )
    .await
}
