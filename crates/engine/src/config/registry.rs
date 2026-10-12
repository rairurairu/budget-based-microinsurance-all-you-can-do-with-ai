//! Registries (R55, ADR-005): trigger event types and the data sources that report them, the
//! kinds of things a customer can declare, optional data types and evidence kinds. Each is data
//! in `config/`, validated at load; the engine keeps closed types only where it computes
//! differently per variant (`Measure`, `ZoneUnit`, `ExposureKind`, `PayoutBasis`).
use crate::types::{DataTypeId, EventCondition, EventTypeId, Measure};
use crate::{EngineError, Result};
use serde::Deserialize;
use std::collections::{BTreeMap, BTreeSet};

pub(crate) fn r55(detail: String) -> EngineError {
    EngineError::Rule {
        rule: "R55",
        detail,
    }
}

fn valid_dotted_id(id: &str) -> bool {
    id.split('.').all(|s| {
        !s.is_empty()
            && s.bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
    })
}

/// The unit an event is reported for; matching differs per unit (Phase 2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ZoneUnit {
    Territory,
    District,
    Line,
    Flight,
    Event,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PricingBasis {
    /// A registered source's historical record (trigger pricing, R31).
    HistoricalRecord,
    /// The insurer's filed rate, in the rating basis.
    InsurerFiledRate,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EventType {
    pub id: EventTypeId,
    pub name: String,
    /// Least to most severe.
    pub stages: Vec<String>,
    pub measure: Measure,
    pub zone: ZoneUnit,
    pub pricing_basis: PricingBasis,
    pub retired: bool,
}

impl EventType {
    pub fn stage_rank(&self, stage: &str) -> Option<u8> {
        self.stages
            .iter()
            .position(|s| s == stage)
            .and_then(|i| u8::try_from(i).ok())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceStatus {
    /// Open data, or a feed under a signed agreement.
    Available,
    PendingAgreement,
    Planned,
}

/// A source code, resolved to an event type and stage rank.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceCode {
    pub event_type: EventTypeId,
    pub stage: u8,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DataSource {
    pub id: String,
    pub status: SourceStatus,
    pub history_available: bool,
    pub codes: BTreeMap<String, SourceCode>,
}

/// Event types and the sources that report them.
#[derive(Debug, Clone, Default)]
pub struct Triggers {
    pub event_types: BTreeMap<EventTypeId, EventType>,
    pub sources: BTreeMap<String, DataSource>,
}

#[derive(Deserialize)]
struct RawEventTypes {
    event_types: Vec<RawEventType>,
}

#[derive(Deserialize)]
struct RawEventType {
    id: String,
    name: String,
    stages: Vec<String>,
    measure: Measure,
    zone: ZoneUnit,
    pricing_basis: PricingBasis,
    #[serde(default)]
    retired: bool,
}

#[derive(Deserialize)]
struct RawSource {
    id: String,
    status: SourceStatus,
    #[serde(default)]
    history: RawHistory,
    codes: BTreeMap<String, RawSourceCode>,
}

#[derive(Deserialize, Default)]
struct RawHistory {
    #[serde(default)]
    available: bool,
}

#[derive(Deserialize)]
struct RawSourceCode {
    event_type: String,
    stage: String,
}

/// `{ event_type, from_stage, min_minutes }` in a catalogue file.
#[derive(Deserialize)]
pub(crate) struct RawCondition {
    event_type: String,
    from_stage: String,
    #[serde(default)]
    min_minutes: Option<u32>,
}

impl Triggers {
    /// The registry from `event_types` (the base file, then a market overlay's: an entry
    /// replaces the earlier one with its id, so a market states its own official stages,
    /// ADR-012) and the data sources.
    pub(crate) fn parse(event_types: &[&str], sources: &[&str]) -> Result<Triggers> {
        let mut out = Triggers::default();
        for text in event_types {
            let raw: RawEventTypes = serde_saphyr::from_str(text)
                .map_err(|e| EngineError::Config(format!("event_types.yaml: {e}")))?;
            let mut in_file = BTreeSet::new();
            for t in raw.event_types {
                let unique: BTreeSet<&String> = t.stages.iter().collect();
                if !valid_dotted_id(&t.id)
                    || t.stages.is_empty()
                    || unique.len() != t.stages.len()
                    || t.stages.len() > usize::from(u8::MAX)
                {
                    return Err(r55(format!(
                        "event type {:?} needs a dotted snake_case id and distinct stages",
                        t.id
                    )));
                }
                if !in_file.insert(t.id.clone()) {
                    return Err(r55(format!("duplicate event type {}", t.id)));
                }
                let ty = EventType {
                    id: t.id.clone(),
                    name: t.name,
                    stages: t.stages,
                    measure: t.measure,
                    zone: t.zone,
                    pricing_basis: t.pricing_basis,
                    retired: t.retired,
                };
                out.event_types.insert(t.id, ty);
            }
        }
        for text in sources {
            let src: RawSource = serde_saphyr::from_str(text)
                .map_err(|e| EngineError::Config(format!("triggers: {e}")))?;
            let mut codes = BTreeMap::new();
            for (code, c) in src.codes {
                let ty = out.event_types.get(&c.event_type).ok_or_else(|| {
                    r55(format!(
                        "source {} maps {code} to unregistered event type {}",
                        src.id, c.event_type
                    ))
                })?;
                let stage = ty.stage_rank(&c.stage).ok_or_else(|| {
                    r55(format!(
                        "source {} maps {code} to unknown stage {} of {}",
                        src.id, c.stage, c.event_type
                    ))
                })?;
                codes.insert(
                    code,
                    SourceCode {
                        event_type: c.event_type,
                        stage,
                    },
                );
            }
            let source = DataSource {
                id: src.id.clone(),
                status: src.status,
                history_available: src.history.available,
                codes,
            };
            if out.sources.insert(src.id.clone(), source).is_some() {
                return Err(r55(format!("duplicate data source {}", src.id)));
            }
        }
        Ok(out)
    }

    fn sources_of<'a>(&'a self, event_type: &'a str) -> impl Iterator<Item = &'a DataSource> {
        self.sources
            .values()
            .filter(move |s| s.codes.values().any(|c| c.event_type == event_type))
    }

    /// R55: an available source reports this event type.
    pub fn has_available_source(&self, event_type: &str) -> bool {
        self.sources_of(event_type)
            .any(|s| s.status == SourceStatus::Available)
    }

    /// R55: the stated pricing basis can be met.
    pub fn priceable(&self, event_type: &str) -> bool {
        match self.event_types.get(event_type).map(|t| t.pricing_basis) {
            Some(PricingBasis::InsurerFiledRate) => true,
            Some(PricingBasis::HistoricalRecord) => {
                self.sources_of(event_type).any(|s| s.history_available)
            }
            None => false,
        }
    }

    pub(crate) fn condition(&self, raw: &RawCondition, cover_id: &str) -> Result<EventCondition> {
        let ty = self
            .event_types
            .get(&raw.event_type)
            .filter(|t| !t.retired)
            .ok_or_else(|| {
                r55(format!(
                    "cover {cover_id} names unregistered event type {}",
                    raw.event_type
                ))
            })?;
        let from_stage = ty.stage_rank(&raw.from_stage).ok_or_else(|| {
            r55(format!(
                "cover {cover_id}: {} has no stage {}",
                raw.event_type, raw.from_stage
            ))
        })?;
        if ty.measure == Measure::Delay && raw.min_minutes.is_none() {
            return Err(r55(format!(
                "cover {cover_id}: {} is measured in minutes late; set min_minutes",
                raw.event_type
            )));
        }
        Ok(EventCondition {
            event_type: raw.event_type.clone(),
            from_stage,
            min_minutes: raw.min_minutes,
        })
    }
}

/// A dotted tree of registered kinds; a parent covers its subtree.
#[derive(Debug, Clone, Default)]
pub struct KindTree {
    ids: BTreeSet<String>,
}

#[derive(Deserialize)]
struct RawKind {
    id: String,
}

impl KindTree {
    fn build(what: &str, raw: Vec<RawKind>) -> Result<KindTree> {
        let mut ids = BTreeSet::new();
        for k in &raw {
            if !valid_dotted_id(&k.id) || !ids.insert(k.id.clone()) {
                return Err(r55(format!("{what}: {:?} is invalid or duplicated", k.id)));
            }
        }
        for id in &ids {
            if let Some((parent, _)) = id.rsplit_once('.')
                && !ids.contains(parent)
            {
                return Err(r55(format!("{what}: {id} has no parent {parent}")));
            }
        }
        Ok(KindTree { ids })
    }

    pub fn contains(&self, id: &str) -> bool {
        self.ids.contains(id)
    }

    /// Every registered id, in order.
    pub fn ids(&self) -> impl Iterator<Item = &str> {
        self.ids.iter().map(String::as_str)
    }

    /// True when `id` is one of `of` or under one of them.
    pub fn under(of: &BTreeSet<String>, id: &str) -> bool {
        of.iter().any(|o| {
            id.strip_prefix(o.as_str())
                .is_some_and(|rest| rest.is_empty() || rest.starts_with('.'))
        })
    }
}

/// What a customer can declare (`config/exposure_types.yaml`).
#[derive(Debug, Clone, Default)]
pub struct ExposureTypes {
    pub upcoming_event_kinds: KindTree,
    pub declared_item_kinds: KindTree,
}

#[derive(Deserialize)]
struct RawExposureTypes {
    upcoming_event_kinds: Vec<RawKind>,
    declared_item_kinds: Vec<RawKind>,
}

impl ExposureTypes {
    pub(crate) fn parse(text: &str) -> Result<ExposureTypes> {
        let raw: RawExposureTypes = serde_saphyr::from_str(text)
            .map_err(|e| EngineError::Config(format!("exposure_types.yaml: {e}")))?;
        Ok(ExposureTypes {
            upcoming_event_kinds: KindTree::build(
                "upcoming_event_kinds",
                raw.upcoming_event_kinds,
            )?,
            declared_item_kinds: KindTree::build("declared_item_kinds", raw.declared_item_kinds)?,
        })
    }
}

/// Optional data types (R20) and evidence kinds (`config/data_types.yaml`).
#[derive(Debug, Clone, Default)]
pub struct DataTypes {
    pub data_types: BTreeSet<DataTypeId>,
    /// Evidence kind → the optional data type it is, if any.
    pub evidence_kinds: BTreeMap<String, Option<DataTypeId>>,
}

#[derive(Deserialize)]
struct RawDataTypes {
    data_types: Vec<RawKind>,
    evidence_kinds: Vec<RawEvidence>,
}

#[derive(Deserialize)]
struct RawEvidence {
    id: String,
    #[serde(default)]
    data_type: Option<String>,
}

impl DataTypes {
    pub(crate) fn parse(text: &str) -> Result<DataTypes> {
        let raw: RawDataTypes = serde_saphyr::from_str(text)
            .map_err(|e| EngineError::Config(format!("data_types.yaml: {e}")))?;
        let mut out = DataTypes::default();
        for d in raw.data_types {
            if !valid_dotted_id(&d.id) || !out.data_types.insert(d.id.clone()) {
                return Err(r55(format!(
                    "data type {:?} is invalid or duplicated",
                    d.id
                )));
            }
        }
        for e in raw.evidence_kinds {
            if let Some(d) = &e.data_type
                && !out.data_types.contains(d)
            {
                return Err(r55(format!(
                    "evidence kind {} names unregistered data type {d}",
                    e.id
                )));
            }
            if out
                .evidence_kinds
                .insert(e.id.clone(), e.data_type)
                .is_some()
            {
                return Err(r55(format!("duplicate evidence kind {}", e.id)));
            }
        }
        Ok(out)
    }
}
