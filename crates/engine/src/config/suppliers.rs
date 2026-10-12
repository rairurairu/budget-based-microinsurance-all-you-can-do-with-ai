//! The supplier panel (`config/suppliers.yaml`): partners who deliver settlement in kind and
//! partner rewards under an agreed tariff, and are paid only for what they delivered (R28, R56,
//! ADR-009).
use super::decimal::DecMoney;
use super::measures::is_snake_id;
use crate::money::Money;
use crate::types::{CoverId, CoverSpec};
use crate::{EngineError, Result};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

pub type SupplierId = String;

/// What the insurer pays a supplier for one delivery.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "amount", rename_all = "snake_case")]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub enum Tariff {
    /// An agreed price per delivery.
    Fixed(Money),
    /// The supplier's invoice, up to the amount reserved; for a payout, the rest is paid to the
    /// customer in cash.
    Invoiced,
}

/// The terms of one in-kind option a cover's terms may name (`settlement_in_kind`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InKindTerms {
    pub option: String,
    pub supplier_id: SupplierId,
    pub tariff: Tariff,
    /// What the customer would pay for it; with a fixed tariff, the option replaces a cash
    /// payout only up to this value (R28).
    pub retail_value: Option<Money>,
    /// Days the customer has to use it, from the day it is issued (R56).
    pub valid_days: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Supplier {
    pub id: SupplierId,
    pub name: String,
    /// Options this supplier delivers in kind.
    pub in_kind: BTreeSet<String>,
    /// Set when the supplier also supplies partner rewards: days a customer has to use one.
    pub reward_valid_days: Option<u32>,
}

#[derive(Debug, Clone, Default)]
pub struct Suppliers {
    pub suppliers: BTreeMap<SupplierId, Supplier>,
    /// By option id; each option has exactly one supplier.
    pub in_kind: BTreeMap<String, InKindTerms>,
}

fn r28(detail: String) -> EngineError {
    EngineError::Rule {
        rule: "R28",
        detail,
    }
}

impl Suppliers {
    pub(crate) fn parse(text: &str, catalogue: &BTreeMap<CoverId, CoverSpec>) -> Result<Suppliers> {
        let raw: RawSuppliers = serde_saphyr::from_str(text)
            .map_err(|e| EngineError::Config(format!("suppliers.yaml: {e}")))?;
        let mut out = Suppliers::default();
        for s in raw.suppliers {
            if !is_snake_id(&s.id) {
                return Err(EngineError::Config(format!(
                    "supplier {:?} is not a snake_case id",
                    s.id
                )));
            }
            if s.reward_valid_days == Some(0) {
                return Err(EngineError::Config(format!(
                    "supplier {}: reward_valid_days must be positive",
                    s.id
                )));
            }
            let mut options = BTreeSet::new();
            for o in s.in_kind {
                let tariff = match (o.tariff, o.invoiced) {
                    (Some(t), false) if t.0 > Money::ZERO => Tariff::Fixed(t.0),
                    (None, true) => Tariff::Invoiced,
                    _ => {
                        return Err(r28(format!(
                            "{}: in-kind option {} needs either a positive tariff or invoiced: true",
                            s.id, o.option
                        )));
                    }
                };
                let retail_value = o.retail_value.map(|v| v.0);
                if let Tariff::Fixed(t) = tariff
                    && retail_value.is_none_or(|v| v < t)
                {
                    return Err(r28(format!(
                        "{}: in-kind option {} at a fixed tariff needs a retail value of at least the tariff",
                        s.id, o.option
                    )));
                }
                if !is_snake_id(&o.option) || o.valid_days == 0 {
                    return Err(r28(format!(
                        "{}: in-kind option {:?} needs a snake_case id and positive valid_days",
                        s.id, o.option
                    )));
                }
                let terms = InKindTerms {
                    option: o.option.clone(),
                    supplier_id: s.id.clone(),
                    tariff,
                    retail_value,
                    valid_days: o.valid_days,
                };
                if let Some(other) = out.in_kind.insert(o.option.clone(), terms) {
                    return Err(r28(format!(
                        "in-kind option {} has two suppliers ({} and {})",
                        o.option, other.supplier_id, s.id
                    )));
                }
                options.insert(o.option);
            }
            let supplier = Supplier {
                id: s.id.clone(),
                name: s.name,
                in_kind: options,
                reward_valid_days: s.reward_valid_days,
            };
            if out.suppliers.insert(s.id.clone(), supplier).is_some() {
                return Err(EngineError::Config(format!("duplicate supplier {}", s.id)));
            }
        }
        for c in catalogue.values() {
            if let Some(o) = c
                .settlement_in_kind
                .iter()
                .find(|o| !out.in_kind.contains_key(*o))
            {
                return Err(r28(format!(
                    "cover {} names in-kind option {o}, which no supplier delivers",
                    c.id
                )));
            }
        }
        Ok(out)
    }

    /// The supplier of a partner reward, when registered for rewards.
    pub fn reward_supplier(&self, id: &str) -> Option<&Supplier> {
        self.suppliers
            .get(id)
            .filter(|s| s.reward_valid_days.is_some())
    }
}

#[derive(Deserialize)]
struct RawSuppliers {
    suppliers: Vec<RawSupplier>,
}

#[derive(Deserialize)]
struct RawSupplier {
    id: SupplierId,
    name: String,
    #[serde(default)]
    in_kind: Vec<RawInKind>,
    #[serde(default)]
    reward_valid_days: Option<u32>,
}

#[derive(Deserialize)]
struct RawInKind {
    option: String,
    #[serde(default)]
    tariff: Option<DecMoney>,
    #[serde(default)]
    invoiced: bool,
    #[serde(default)]
    retail_value: Option<DecMoney>,
    valid_days: u32,
}
