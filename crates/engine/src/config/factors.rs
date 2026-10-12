//! Rating factors: the only data the rating formula may use (R33), industry or novel, approved or
//! candidate (R43). Protected attributes only as R19 allows.
use super::lines::LinesOfBusiness;
use super::registry::DataTypes;
use crate::types::{DataTypeId, FactorId};
use crate::{EngineError, Result};
use serde::Deserialize;
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FactorLevel {
    /// The same for every customer; carried in the rating basis' expected claim cost.
    Market,
    /// This customer's exposure; rated through a relativity table.
    Individual,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FactorBasis {
    Industry,
    Novel,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FactorStatus {
    Approved,
    /// Rated in shadow only; never affects terms (R43).
    Candidate,
}

/// Where an individual factor's value comes from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FactorData {
    /// The customer's own answers.
    Declared,
    /// The insurer's records of this policy.
    OwnClaims,
    /// A registered optional data type, usable only with the customer's opt-in (R20).
    Consented(DataTypeId),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RatingFactor {
    pub id: FactorId,
    pub level: FactorLevel,
    pub basis: FactorBasis,
    pub status: FactorStatus,
    pub data: Option<FactorData>,
    /// Classes of business it rates; a parent covers its subtree.
    pub classes: Vec<String>,
    pub formula: String,
}

impl RatingFactor {
    pub fn rates(&self, class: &str) -> bool {
        self.classes
            .iter()
            .any(|c| LinesOfBusiness::contains(c, class))
    }
}

/// Never a rating factor, under any name (R19).
const NEVER_FACTORS: [&str; 4] = ["ethnicity", "race", "religion", "nationality"];

#[derive(Deserialize)]
struct RawFactors {
    factors: Vec<RawFactor>,
}

#[derive(Deserialize)]
struct RawFactor {
    id: String,
    level: FactorLevel,
    basis: FactorBasis,
    status: FactorStatus,
    #[serde(default)]
    data: Option<String>,
    classes: Vec<String>,
    formula: String,
}

fn data_source(text: &str, data_types: &DataTypes) -> Option<FactorData> {
    match text {
        "declared" => Some(FactorData::Declared),
        "own_claims" => Some(FactorData::OwnClaims),
        d if data_types.data_types.contains(d) => Some(FactorData::Consented(d.to_string())),
        _ => None,
    }
}

pub(crate) fn parse(
    text: &str,
    lines: &LinesOfBusiness,
    data_types: &DataTypes,
) -> Result<BTreeMap<FactorId, RatingFactor>> {
    let raw: RawFactors = serde_saphyr::from_str(text)
        .map_err(|e| EngineError::Config(format!("rating_factors.yaml: {e}")))?;
    let mut out = BTreeMap::new();
    for f in raw.factors {
        let id_words: Vec<&str> = f.id.split('_').collect();
        if let Some(banned) = NEVER_FACTORS.iter().find(|w| id_words.contains(w)) {
            return Err(EngineError::Rule {
                rule: "R19",
                detail: format!(
                    "rating factor {} uses {banned}, which is never a factor",
                    f.id
                ),
            });
        }
        if f.basis == FactorBasis::Industry && f.status == FactorStatus::Candidate {
            return Err(EngineError::Rule {
                rule: "R43",
                detail: format!(
                    "rating factor {}: only novel factors start as candidates",
                    f.id
                ),
            });
        }
        let data = match (f.level, f.data.as_deref()) {
            (FactorLevel::Market, None) => None,
            (FactorLevel::Individual, Some(d)) => {
                Some(data_source(d, data_types).ok_or_else(|| {
                    EngineError::Config(format!("rating factor {}: unknown data {d:?}", f.id))
                })?)
            }
            (FactorLevel::Market, Some(_)) => {
                return Err(EngineError::Config(format!(
                    "rating factor {}: a market factor has no individual data source",
                    f.id
                )));
            }
            (FactorLevel::Individual, None) => {
                return Err(EngineError::Config(format!(
                    "rating factor {}: an individual factor needs a data source",
                    f.id
                )));
            }
        };
        if f.classes.is_empty() {
            return Err(EngineError::Config(format!(
                "rating factor {} rates no class of business",
                f.id
            )));
        }
        if let Some(unknown) = f.classes.iter().find(|c| lines.node(c).is_none()) {
            return Err(EngineError::Rule {
                rule: "R33",
                detail: format!(
                    "rating factor {} names unknown class of business {unknown}",
                    f.id
                ),
            });
        }
        let factor = RatingFactor {
            id: f.id.clone(),
            level: f.level,
            basis: f.basis,
            status: f.status,
            data,
            classes: f.classes,
            formula: f.formula,
        };
        if out.insert(f.id.clone(), factor).is_some() {
            return Err(EngineError::Config(format!(
                "duplicate rating factor {}",
                f.id
            )));
        }
    }
    Ok(out)
}
