//! The languages customers can choose, from `config/languages/<tag>.yaml`: each pack's sentence
//! shape, the words the fixed checks look for, and the saved answers (R22). A customer's text
//! comes in their own language setting (`Profile::language`), or the insurer's default; nothing
//! about a language is in code.
//!
//! Packs are checked when loaded: every offered language has one, every cover has a saved
//! sentence and a name, and every saved answer passes the same checks as a model's reply. A
//! process with an invalid pack does not start, so a fallback can never fail.
use crate::text::{self, SentenceRules, Terms};
use crate::understand::ProposalQuestion;
use microinsurance_engine::config::EngineConfig;
use serde::Deserialize;
use std::collections::{BTreeMap, BTreeSet};

/// The most questions a proposal form has.
pub const MAX_QUESTIONS: usize = 5;
const MAX_OPTIONS: usize = 8;
const MAX_OPTION_CHARS: usize = 60;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawPack {
    language: String,
    sentence: RawSentence,
    checks: RawChecks,
    saved_answers: SavedAnswers,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawSentence {
    terminators: Vec<String>,
    #[serde(default)]
    max_words: Option<usize>,
    #[serde(default)]
    max_chars: Option<usize>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawChecks {
    currency: Vec<String>,
    protected: Vec<String>,
}

/// A renewal change in fixed words; `{cover}` is the cover's name.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChangeSentences {
    pub more: String,
    pub less: String,
    pub same: String,
    pub below_declared_value: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VerifySentences {
    pub no_evidence: String,
    pub reused: String,
    pub unreadable: String,
    pub refer: String,
}

/// What a claimant reads when the assessor refers their claim to a person.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AssessSentences {
    pub no_evidence: String,
    pub reused: String,
    pub unreadable: String,
    pub refer: String,
}

/// What a customer sees when the AI is slow, fails or answers outside the rules (R22).
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SavedAnswers {
    pub proposal_form: Vec<ProposalQuestion>,
    /// One sentence per catalogue cover.
    pub covers: BTreeMap<String, String>,
    /// Each cover's name inside a sentence.
    pub cover_names: BTreeMap<String, String>,
    pub change: ChangeSentences,
    pub verify: VerifySentences,
    pub assess: AssessSentences,
}

#[derive(Debug, Clone)]
pub struct LanguagePack {
    pub tag: String,
    pub sentence: SentenceRules,
    pub saved: SavedAnswers,
}

impl LanguagePack {
    /// A change sentence with the cover's name filled in.
    pub fn change(&self, template: &str, cover_id: &str) -> String {
        let name = self
            .saved
            .cover_names
            .get(cover_id)
            .map_or(cover_id, String::as_str);
        template.replace("{cover}", name)
    }
}

#[derive(Debug, Clone)]
pub struct Languages {
    default: LanguagePack,
    packs: BTreeMap<String, LanguagePack>,
    /// Every pack's currency terms: a reply in any language is checked against all (R02).
    currency: Terms,
    /// Every pack's protected terms (R19).
    protected: Terms,
}

const PROTECTED_FIELDS: &[&str] = crate::harness::PROTECTED_FIELDS;

impl Languages {
    /// Builds the packs from `(file stem, text)` pairs and checks them against the config.
    pub fn load(files: &[(String, String)], cfg: &EngineConfig) -> Result<Languages, String> {
        let mut raws = Vec::new();
        for (stem, text) in files {
            let raw: RawPack =
                serde_saphyr::from_str(text).map_err(|e| format!("languages/{stem}.yaml: {e}"))?;
            if raw.language != *stem {
                return Err(format!(
                    "languages/{stem}.yaml declares language {}",
                    raw.language
                ));
            }
            raws.push(raw);
        }
        let currency = Terms::new(
            raws.iter()
                .flat_map(|r| r.checks.currency.iter().map(String::as_str)),
        );
        let protected = Terms::new(
            raws.iter()
                .flat_map(|r| r.checks.protected.iter().map(String::as_str)),
        );
        let mut packs = BTreeMap::new();
        for raw in raws {
            let mut terminators = Vec::new();
            for t in &raw.sentence.terminators {
                let mut chars = t.chars();
                match (chars.next(), chars.next()) {
                    (Some(c), None) => terminators.push(c),
                    _ => {
                        return Err(format!(
                            "{}: terminator {t:?} is not one character",
                            raw.language
                        ));
                    }
                }
            }
            if raw.sentence.max_words.is_none() && raw.sentence.max_chars.is_none() {
                return Err(format!("{}: set max_words or max_chars", raw.language));
            }
            let pack = LanguagePack {
                tag: raw.language.clone(),
                sentence: SentenceRules {
                    terminators,
                    max_words: raw.sentence.max_words,
                    max_chars: raw.sentence.max_chars,
                },
                saved: raw.saved_answers,
            };
            packs.insert(raw.language, pack);
        }
        let default = packs
            .get(&cfg.languages.default)
            .cloned()
            .ok_or_else(|| format!("the default language {} has no pack", cfg.languages.default))?;
        let out = Languages {
            default,
            packs,
            currency,
            protected,
        };
        out.check_saved(cfg)?;
        Ok(out)
    }

    fn check_saved(&self, cfg: &EngineConfig) -> Result<(), String> {
        let covers: BTreeSet<&String> = cfg.catalogue.keys().collect();
        for tag in &cfg.languages.offered {
            let pack = self
                .packs
                .get(tag)
                .ok_or_else(|| format!("language {tag} is offered but has no pack"))?;
            let at = |e: String| format!("languages/{tag}.yaml: {e}");
            let saved = &pack.saved;
            for (what, map) in [
                ("covers", &saved.covers),
                ("cover_names", &saved.cover_names),
            ] {
                let given: BTreeSet<&String> = map.keys().collect();
                if given != covers {
                    return Err(at(format!("{what} must name every catalogue cover")));
                }
            }
            for sentence in saved.covers.values() {
                self.check_sentence(pack, sentence, &[]).map_err(at)?;
            }
            for id in &covers {
                let c = &saved.change;
                for template in [&c.more, &c.less, &c.same, &c.below_declared_value] {
                    if !template.contains("{cover}") {
                        return Err(at(format!("{template:?} has no {{cover}}")));
                    }
                    self.check_sentence(pack, &pack.change(template, id), &[])
                        .map_err(at)?;
                }
            }
            let v = &saved.verify;
            for s in [&v.no_evidence, &v.reused, &v.unreadable, &v.refer] {
                self.check_sentence(pack, s, &[]).map_err(at)?;
            }
            let a = &saved.assess;
            for s in [&a.no_evidence, &a.reused, &a.unreadable, &a.refer] {
                self.check_sentence(pack, s, &[]).map_err(at)?;
            }
            self.check_questions(pack, &saved.proposal_form)
                .map_err(at)?;
        }
        Ok(())
    }

    /// The pack for a customer's setting: their language when offered, otherwise the default.
    pub fn pack(&self, setting: Option<&str>) -> &LanguagePack {
        setting
            .and_then(|l| self.packs.get(l))
            .unwrap_or(&self.default)
    }

    /// R02: a currency term in any offered language, or one of `amounts`.
    pub fn states_money(&self, text: &str, amounts: &[i64]) -> bool {
        text::states_money(text, &self.currency, amounts)
    }

    /// R19: a protected attribute named in any offered language.
    pub fn mentions_protected(&self, text: &str) -> bool {
        self.protected.found_in(text)
    }

    /// The fixed checks for one customer-facing sentence in `pack`'s language.
    pub fn check_sentence(
        &self,
        pack: &LanguagePack,
        text: &str,
        amounts: &[i64],
    ) -> Result<(), String> {
        if !text::is_one_sentence(text, &pack.sentence) {
            return Err(format!(
                "R23: not one plain {} sentence: {text:?}",
                pack.tag
            ));
        }
        if self.states_money(text, amounts) {
            return Err(format!("R02: states money: {text:?}"));
        }
        if self.mentions_protected(text) {
            return Err(format!("R19: mentions a protected attribute: {text:?}"));
        }
        Ok(())
    }

    /// The checks a proposal form's questions must pass.
    pub fn check_questions(
        &self,
        pack: &LanguagePack,
        questions: &[ProposalQuestion],
    ) -> Result<(), String> {
        if questions.is_empty() || questions.len() > MAX_QUESTIONS {
            return Err(format!(
                "{} questions; 1 to {MAX_QUESTIONS} allowed",
                questions.len()
            ));
        }
        let mut ids = BTreeSet::new();
        for q in questions {
            if !text::is_snake_id(&q.id) || !ids.insert(q.id.as_str()) {
                return Err(format!("question id {:?} is invalid or repeated", q.id));
            }
            if q.id.split('_').any(|part| PROTECTED_FIELDS.contains(&part)) {
                return Err(format!(
                    "R19: question {:?} asks for a protected attribute",
                    q.id
                ));
            }
            self.check_sentence(pack, &q.text, &[])?;
            if q.options.len() > MAX_OPTIONS {
                return Err(format!("question {:?} has too many options", q.id));
            }
            for o in &q.options {
                let o = o.trim();
                if o.is_empty()
                    || o.chars().count() > MAX_OPTION_CHARS
                    || self.states_money(o, &[])
                    || self.mentions_protected(o)
                {
                    return Err(format!("option {o:?} is not allowed"));
                }
            }
        }
        Ok(())
    }
}
