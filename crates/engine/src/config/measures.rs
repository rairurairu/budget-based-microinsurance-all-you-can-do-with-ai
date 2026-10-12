//! Loss prevention measures (`config/prevention_measures.yaml`): the kinds of prevention task the
//! insurer recognises, each for one class of business with a loss-based cover (R18, R36, R38,
//! ADR-009). The AI writes the event-specific action for one of them; the engine sizes its
//! reward from the measure's claims data in the prevention basis.
use super::lines::LinesOfBusiness;
use super::registry::DataTypes;
use crate::types::{ClassId, CoverId, CoverSpec, CoverType};
use crate::{EngineError, Result};
use serde::Deserialize;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreventionMeasure {
    /// The task type the AI names (`PreventionTaskProposal::task_type`). Permanent once used.
    pub id: String,
    /// A leaf class of business with at least one indemnity cover (R36, R38).
    pub class_of_business: ClassId,
    /// One plain sentence; the prevention task writer reads it.
    pub description: String,
    /// Registered evidence kinds that prove the measure (`config/data_types.yaml`).
    pub evidence: BTreeSet<String>,
    /// One verified activity per customer in this many days (R10).
    pub repeat_after_days: u32,
    pub retired: bool,
}

#[derive(Debug, Clone, Default)]
pub struct PreventionMeasures {
    pub measures: BTreeMap<String, PreventionMeasure>,
}

impl PreventionMeasures {
    /// Active measures for a class of business, in id order.
    pub fn for_class<'a>(&'a self, class: &'a str) -> impl Iterator<Item = &'a PreventionMeasure> {
        self.measures
            .values()
            .filter(move |m| !m.retired && m.class_of_business == class)
    }

    /// An active measure.
    pub fn active(&self, id: &str) -> Option<&PreventionMeasure> {
        self.measures.get(id).filter(|m| !m.retired)
    }

    pub(crate) fn parse(
        text: &str,
        lines: &LinesOfBusiness,
        data_types: &DataTypes,
        catalogue: &BTreeMap<CoverId, CoverSpec>,
    ) -> Result<PreventionMeasures> {
        let raw: RawMeasures = serde_saphyr::from_str(text)
            .map_err(|e| EngineError::Config(format!("prevention_measures.yaml: {e}")))?;
        let mut out = PreventionMeasures::default();
        for m in raw.measures {
            if !is_snake_id(&m.id) {
                return Err(EngineError::Config(format!(
                    "prevention measure {:?} is not a snake_case id",
                    m.id
                )));
            }
            if !lines.is_leaf(&m.class_of_business) {
                return Err(EngineError::Rule {
                    rule: "R36",
                    detail: format!(
                        "prevention measure {} names {}, which is not an active leaf class of business",
                        m.id, m.class_of_business
                    ),
                });
            }
            let loss_based = catalogue.values().any(|c| {
                c.class_of_business == m.class_of_business && c.cover_type == CoverType::Indemnity
            });
            if !loss_based {
                return Err(EngineError::Rule {
                    rule: "R38",
                    detail: format!(
                        "prevention measure {}: class {} has no loss-based cover",
                        m.id, m.class_of_business
                    ),
                });
            }
            if m.evidence.is_empty() {
                return Err(EngineError::Rule {
                    rule: "R10",
                    detail: format!("prevention measure {} names no evidence", m.id),
                });
            }
            if let Some(e) = m
                .evidence
                .iter()
                .find(|e| !data_types.evidence_kinds.contains_key(*e))
            {
                return Err(EngineError::Rule {
                    rule: "R55",
                    detail: format!(
                        "prevention measure {} names unregistered evidence kind {e}",
                        m.id
                    ),
                });
            }
            if m.repeat_after_days == 0 {
                return Err(EngineError::Rule {
                    rule: "R10",
                    detail: format!(
                        "prevention measure {}: repeat_after_days must be positive",
                        m.id
                    ),
                });
            }
            let measure = PreventionMeasure {
                id: m.id.clone(),
                class_of_business: m.class_of_business,
                description: m.description,
                evidence: m.evidence,
                repeat_after_days: m.repeat_after_days,
                retired: m.retired,
            };
            if out.measures.insert(m.id.clone(), measure).is_some() {
                return Err(EngineError::Config(format!(
                    "duplicate prevention measure {}",
                    m.id
                )));
            }
        }
        Ok(out)
    }
}

/// Lowercase letters, digits and underscores, starting with a letter.
pub(crate) fn is_snake_id(id: &str) -> bool {
    id.bytes().next().is_some_and(|b| b.is_ascii_lowercase())
        && id
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
}

#[derive(Deserialize)]
struct RawMeasures {
    measures: Vec<RawMeasure>,
}

#[derive(Deserialize)]
struct RawMeasure {
    id: String,
    class_of_business: ClassId,
    description: String,
    evidence: BTreeSet<String>,
    repeat_after_days: u32,
    #[serde(default)]
    retired: bool,
}
