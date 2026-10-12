//! The registered ids a model may answer with (R55): what it returns is checked against these,
//! and they are listed in its input so it can choose among them.
use microinsurance_engine::config::EngineConfig;
use microinsurance_engine::types::DataTypeId;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, Default)]
pub struct Vocabulary {
    /// `config/exposure_types.yaml` upcoming event kinds.
    pub upcoming_event_kinds: BTreeSet<String>,
    /// `config/exposure_types.yaml` declared item kinds.
    pub declared_item_kinds: BTreeSet<String>,
    /// Evidence kind → the optional data type it is (`config/data_types.yaml`).
    pub evidence_kinds: BTreeMap<String, Option<DataTypeId>>,
    /// Route ids a customer can declare (the transit source's rail zones).
    pub routes: BTreeSet<String>,
}

impl Vocabulary {
    pub fn from_config(cfg: &EngineConfig, routes: impl IntoIterator<Item = String>) -> Vocabulary {
        Vocabulary {
            upcoming_event_kinds: cfg
                .exposure_types
                .upcoming_event_kinds
                .ids()
                .map(str::to_string)
                .collect(),
            declared_item_kinds: cfg
                .exposure_types
                .declared_item_kinds
                .ids()
                .map(str::to_string)
                .collect(),
            evidence_kinds: cfg.data_types.evidence_kinds.clone(),
            routes: routes.into_iter().collect(),
        }
    }
}
