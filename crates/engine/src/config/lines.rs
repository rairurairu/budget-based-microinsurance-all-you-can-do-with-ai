//! Lines and classes of business: a closed tree, validated at load (R37); points pool depth (R36);
//! experience links between classes (R39).
use super::decimal::DecBps;
use crate::money::Bps;
use crate::types::{ClassId, CoverId, PoolId};
use crate::{EngineError, Result};
use serde::Deserialize;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClassNode {
    pub id: ClassId,
    pub name: String,
    /// One plain sentence; the prevention task writer reads it to stay in scope.
    pub description: Option<String>,
    pub retired: bool,
}

/// The only way a claim in one class of business moves another (R39). Backed by data.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExperienceLink {
    pub from: ClassId,
    pub to: ClassId,
    pub weight: Bps,
    pub evidence: String,
}

#[derive(Debug, Clone)]
pub struct LinesOfBusiness {
    nodes: BTreeMap<ClassId, ClassNode>,
    parents: BTreeSet<ClassId>,
    pub points_pool_depth: usize,
    pub experience_links: Vec<ExperienceLink>,
}

#[derive(Deserialize)]
struct RawLines {
    points_pool_depth: usize,
    #[serde(default)]
    experience_links: Vec<RawLink>,
    nodes: Vec<RawNode>,
}

#[derive(Deserialize)]
struct RawNode {
    id: String,
    name: String,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    retired: bool,
}

#[derive(Deserialize)]
struct RawLink {
    from: String,
    to: String,
    weight: DecBps,
    evidence: String,
}

fn r37(detail: String) -> EngineError {
    EngineError::Rule {
        rule: "R37",
        detail,
    }
}

impl LinesOfBusiness {
    pub(crate) fn parse(text: &str) -> Result<LinesOfBusiness> {
        let raw: RawLines = serde_saphyr::from_str(text)
            .map_err(|e| EngineError::Config(format!("lines_of_business.yaml: {e}")))?;
        if raw.points_pool_depth < 1 {
            return Err(EngineError::Config(
                "lines_of_business.yaml: points_pool_depth must be at least 1 (R36)".into(),
            ));
        }
        let mut nodes = BTreeMap::new();
        for n in raw.nodes {
            let valid_segments = n.id.split('.').all(|s| {
                !s.is_empty()
                    && s.bytes()
                        .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
            });
            if !valid_segments {
                return Err(r37(format!(
                    "class id {:?} must be dotted snake_case segments",
                    n.id
                )));
            }
            let node = ClassNode {
                id: n.id.clone(),
                name: n.name,
                description: n.description,
                retired: n.retired,
            };
            if nodes.insert(n.id.clone(), node).is_some() {
                return Err(r37(format!("duplicate class id {}", n.id)));
            }
        }
        let mut parents = BTreeSet::new();
        for id in nodes.keys() {
            if let Some((parent, _)) = id.rsplit_once('.') {
                if !nodes.contains_key(parent) {
                    return Err(r37(format!("class {id} has no parent {parent}")));
                }
                parents.insert(parent.to_string());
            }
        }
        let mut lines = LinesOfBusiness {
            nodes,
            parents,
            points_pool_depth: raw.points_pool_depth,
            experience_links: Vec::new(),
        };
        for l in raw.experience_links {
            for end in [&l.from, &l.to] {
                if !lines.is_leaf(end) {
                    return Err(EngineError::Rule {
                        rule: "R39",
                        detail: format!("experience link names {end}, which is not a leaf class"),
                    });
                }
            }
            if l.from == l.to || l.weight.0 > Bps::ONE || l.evidence.trim().is_empty() {
                return Err(EngineError::Rule {
                    rule: "R39",
                    detail: format!(
                        "experience link {} → {} needs two classes, a weight of at most 1 and evidence",
                        l.from, l.to
                    ),
                });
            }
            lines.experience_links.push(ExperienceLink {
                from: l.from,
                to: l.to,
                weight: l.weight.0,
                evidence: l.evidence,
            });
        }
        Ok(lines)
    }

    pub fn node(&self, id: &str) -> Option<&ClassNode> {
        self.nodes.get(id)
    }

    /// Known, active and without children: the only kind of class a cover may name (R37).
    pub fn is_leaf(&self, id: &str) -> bool {
        self.nodes.get(id).is_some_and(|n| !n.retired) && !self.parents.contains(id)
    }

    /// The points pool a class belongs to (R36).
    pub fn pool_of(&self, id: &str) -> PoolId {
        id.split('.')
            .take(self.points_pool_depth)
            .collect::<Vec<_>>()
            .join(".")
    }

    /// True when `ancestor` is `id` or one of its parents.
    pub fn contains(ancestor: &str, id: &str) -> bool {
        id.strip_prefix(ancestor)
            .is_some_and(|rest| rest.is_empty() || rest.starts_with('.'))
    }

    pub(crate) fn check_cover_class(&self, cover_id: &CoverId, class: &str) -> Result<()> {
        match self.nodes.get(class) {
            None => Err(r37(format!(
                "cover {cover_id} names unknown class of business {class}"
            ))),
            Some(n) if n.retired => Err(r37(format!(
                "cover {cover_id} names retired class of business {class}"
            ))),
            Some(_) if self.parents.contains(class) => Err(r37(format!(
                "cover {cover_id} names {class}, which has subclasses; name exactly one leaf class"
            ))),
            Some(_) => Ok(()),
        }
    }
}
