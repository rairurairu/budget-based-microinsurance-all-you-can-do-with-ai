//! One plain sentence per cover, in outcome language, for this customer's schedule (R23), and
//! one sentence for each renewal change (R35), in the customer's language setting. The model sees
//! what each cover does and which market conditions moved its rate; never a premium, a sum
//! insured or a personal rating factor, so it cannot state or reason from them (R02, R19). Every
//! sentence passes the fixed checks before it is shown; otherwise the language pack's saved
//! sentence is shown.
use crate::client::Model;
use crate::harness::{Ask, Harness, Harnessed};
use crate::language::{LanguagePack, Languages};
use crate::settings::Tier;
use microinsurance_engine::config::EngineConfig;
use microinsurance_engine::config::FactorLevel;
use microinsurance_engine::consent::SumInsuredChange;
use microinsurance_engine::money::{Bps, Money};
use microinsurance_engine::optimiser::SuggestionBasis;
use microinsurance_engine::types::{CoverSpec, Profile, Schedule};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::cmp::Ordering;
use std::collections::BTreeSet;

pub const SCHEDULE: &str = "explain";
pub const CHANGE: &str = "explain_change";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CoverExplanation {
    pub cover_id: String,
    pub sentence: String,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ExplainReply {
    pub explanations: Vec<CoverExplanation>,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SentenceReply {
    pub sentence: String,
}

/// What the model may know about a cover: what it does, never what it costs or pays.
fn cover_facts(cover: &CoverSpec, cfg: &EngineConfig) -> Value {
    let pays_on: Vec<&str> = cover
        .trigger
        .iter()
        .flat_map(|t| t.events.iter())
        .filter_map(|c| cfg.triggers.event_types.get(&c.event_type))
        .map(|t| t.name.as_str())
        .collect();
    json!({
        "cover_id": cover.id,
        "name": cover.name,
        "key_features": cover.key_features,
        "cover_type": cover.cover_type,
        "pays_on": pays_on,
        "waiting_period_days": cover.waiting_period_days,
    })
}

/// The money amounts a sentence must not repeat, in major units.
fn amounts(values: impl IntoIterator<Item = Money>) -> Vec<i64> {
    values
        .into_iter()
        .map(|m| m.0 / 100)
        .filter(|a| *a != 0)
        .collect()
}

/// The checks the schedule's sentences must pass: one per cover, each a plain sentence in the
/// customer's language with no amount and nothing protected.
pub fn check_schedule(
    reply: ExplainReply,
    schedule: &Schedule,
    languages: &Languages,
    pack: &LanguagePack,
) -> Result<Vec<CoverExplanation>, String> {
    let wanted: BTreeSet<&str> = schedule
        .allocations
        .iter()
        .map(|a| a.cover_id.as_str())
        .collect();
    let given: BTreeSet<&str> = reply
        .explanations
        .iter()
        .map(|e| e.cover_id.as_str())
        .collect();
    if wanted != given || reply.explanations.len() != wanted.len() {
        return Err(format!("covers {given:?}, expected {wanted:?}"));
    }
    let money = amounts(
        schedule
            .allocations
            .iter()
            .flat_map(|a| [a.premium, a.sum_insured]),
    );
    let mut out = reply.explanations;
    for e in &mut out {
        e.sentence = e.sentence.trim().to_string();
        languages.check_sentence(pack, &e.sentence, &money)?;
    }
    out.sort_by(|a, b| a.cover_id.cmp(&b.cover_id));
    Ok(out)
}

/// The saved answer for a schedule: each cover's saved sentence in the customer's language.
pub fn fallback_schedule(schedule: &Schedule, pack: &LanguagePack) -> Vec<CoverExplanation> {
    schedule
        .allocations
        .iter()
        .filter_map(|a| {
            Some(CoverExplanation {
                cover_id: a.cover_id.clone(),
                sentence: pack.saved.covers.get(&a.cover_id)?.clone(),
            })
        })
        .collect()
}

/// One sentence per cover in `schedule`, in outcome language for this customer.
pub async fn explain_schedule<M: Model>(
    h: &Harness<M>,
    schedule: &Schedule,
    profile: &Profile,
    cfg: &EngineConfig,
) -> Harnessed<Vec<CoverExplanation>> {
    let pack = h.languages.pack(profile.language.as_deref());
    let covers: Vec<Value> = schedule
        .allocations
        .iter()
        .filter_map(|a| cfg.cover(&a.cover_id).ok())
        .map(|c| cover_facts(c, cfg))
        .collect();
    let item_kinds: BTreeSet<&str> = profile
        .declared_items
        .iter()
        .map(|i| i.kind.as_str())
        .collect();
    let event_kinds: BTreeSet<&str> = profile
        .upcoming_events
        .iter()
        .map(|e| e.kind.as_str())
        .collect();
    let input = json!({
        "language": pack.tag,
        "covers": covers,
        "customer": {
            "occupation": profile.occupation,
            "declared_working_hours": !profile.work_windows.is_empty(),
            "declared_routes": profile.commute_routes.len(),
            "declared_items": item_kinds,
            "upcoming_events": event_kinds,
        },
    });
    let ask = Ask {
        component: SCHEDULE,
        tier: Tier::Fast,
        input,
        images: &[],
    };
    h.ask(
        ask,
        |r: ExplainReply| check_schedule(r, schedule, &h.languages, pack),
        || fallback_schedule(schedule, pack),
    )
    .await
}

/// How a renewal changes a cover, without amounts.
fn direction(change: &SumInsuredChange) -> &'static str {
    match change.after.cmp(&change.before) {
        Ordering::Greater => "more_cover_same_premium",
        Ordering::Less => "less_cover_same_premium",
        Ordering::Equal => "same_cover",
    }
}

/// The saved sentence for a change: the language pack's sentence for it.
pub fn fallback_change(change: &SumInsuredChange, pack: &LanguagePack) -> String {
    let c = &pack.saved.change;
    let template = if change.below_declared_value {
        &c.below_declared_value
    } else {
        match change.after.cmp(&change.before) {
            Ordering::Greater => &c.more,
            Ordering::Less => &c.less,
            Ordering::Equal => &c.same,
        }
    };
    pack.change(template, &change.cover_id)
}

/// One sentence for a renewal change (R35), in the customer's language. Only market factors
/// (the same for every customer) are named to the model; personal factors, protected or not,
/// never are (R19).
pub async fn explain_change<M: Model>(
    h: &Harness<M>,
    change: &SumInsuredChange,
    basis: Option<&SuggestionBasis>,
    language: Option<&str>,
    cfg: &EngineConfig,
) -> Harnessed<String> {
    let pack = h.languages.pack(language);
    let market: Vec<Value> = basis
        .into_iter()
        .flat_map(|b| b.factors.iter())
        .filter(|f| !f.shadow && f.relativity != Bps::ONE)
        .filter_map(|f| {
            let factor = cfg.rating_factors.get(&f.factor_id)?;
            (factor.level == FactorLevel::Market).then(|| {
                json!({
                    "factor": f.factor_id,
                    "means": factor.formula,
                    "risk": if f.relativity > Bps::ONE { "higher" } else { "lower" },
                })
            })
        })
        .collect();
    let input = json!({
        "language": pack.tag,
        "cover": cfg.cover(&change.cover_id).ok().map(|c| cover_facts(c, cfg)),
        "change": direction(change),
        "below_declared_value": change.below_declared_value,
        "market_factors": market,
    });
    let ask = Ask {
        component: CHANGE,
        tier: Tier::Fast,
        input,
        images: &[],
    };
    let money = amounts(
        [change.premium, change.before, change.after]
            .into_iter()
            .chain(change.declared_value),
    );
    h.ask(
        ask,
        |r: SentenceReply| {
            let s = r.sentence.trim().to_string();
            h.languages.check_sentence(pack, &s, &money).map(|()| s)
        },
        || fallback_change(change, pack),
    )
    .await
}
