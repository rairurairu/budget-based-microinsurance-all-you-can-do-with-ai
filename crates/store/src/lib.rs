//! The policy store (ADR-011): the system of record for what customers, partners and the
//! insurer's people do through the API, and for what the workers decide that a customer sees.
//! Each record is a JSON document in a named collection, keyed by id and, where it has them, by
//! customer and distribution partner. Every write names the version it read, so two taps on one
//! offer bind one policy (optimistic concurrency). Money is not here: amounts are the engine's
//! decisions as recorded, and the ledger holds the money (R27).
//!
//! `MemoryStore` is the reference store for tests and demo mode; `PgStore` keeps the same
//! records in PostgreSQL and tells every API process about each write (LISTEN / NOTIFY), so a
//! payout a worker decides reaches the customer's live stream. Both pass the same tests
//! (`tests/store.rs`). Records are kept for `record_retention_years` (R54); optional data is
//! erased by rewriting the customer's record, never by deleting the policy's.
pub mod collections;
mod memory;
mod postgres;

pub use memory::MemoryStore;
pub use postgres::PgStore;

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::broadcast;

/// What a record is filed under besides its id.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Keys {
    pub customer_id: Option<String>,
    /// The distribution partner the record came through, for the partner's aggregates (R21).
    pub partner_id: Option<String>,
}

impl Keys {
    pub fn customer(id: &str) -> Keys {
        Keys {
            customer_id: Some(id.to_string()),
            partner_id: None,
        }
    }

    pub fn with_partner(mut self, partner: Option<&str>) -> Keys {
        self.partner_id = partner.map(str::to_string);
        self
    }
}

/// A stored record as JSON.
#[derive(Debug, Clone, PartialEq)]
pub struct Raw {
    pub id: String,
    pub keys: Keys,
    pub version: i64,
    pub body: Value,
}

/// A stored record, decoded.
#[derive(Debug, Clone, PartialEq)]
pub struct Record<T> {
    pub id: String,
    pub keys: Keys,
    /// Rises by one on every write; an update names the version it read.
    pub version: i64,
    pub body: T,
}

/// Which records of a collection to list.
#[derive(Debug, Clone, Copy)]
pub enum By<'a> {
    All,
    Customer(&'a str),
    /// Any of these customers, in one query.
    Customers(&'a [String]),
    Partner(&'a str),
}

/// A write, as every subscriber hears of it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Change {
    pub collection: String,
    pub id: String,
    pub customer_id: Option<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    /// Someone else wrote the record since it was read, or it already exists.
    #[error("{collection}/{id} was changed by someone else; read it again")]
    Conflict { collection: String, id: String },
    #[error("{collection}/{id}: {detail}")]
    Decode {
        collection: String,
        id: String,
        detail: String,
    },
    #[error("store error: {0}")]
    Store(String),
}

/// The store a process uses.
#[derive(Debug, Clone)]
pub enum Store {
    Memory(MemoryStore),
    Pg(PgStore),
}

fn decode<T: DeserializeOwned>(collection: &str, raw: Raw) -> Result<Record<T>, StoreError> {
    let body = serde_json::from_value(raw.body).map_err(|e| StoreError::Decode {
        collection: collection.to_string(),
        id: raw.id.clone(),
        detail: e.to_string(),
    })?;
    Ok(Record {
        id: raw.id,
        keys: raw.keys,
        version: raw.version,
        body,
    })
}

fn encode<T: Serialize>(collection: &str, id: &str, body: &T) -> Result<Value, StoreError> {
    serde_json::to_value(body).map_err(|e| StoreError::Decode {
        collection: collection.to_string(),
        id: id.to_string(),
        detail: e.to_string(),
    })
}

impl Store {
    pub fn memory() -> Store {
        Store::Memory(MemoryStore::default())
    }

    async fn get_raw(&self, collection: &str, id: &str) -> Result<Option<Raw>, StoreError> {
        match self {
            Store::Memory(s) => Ok(s.get(collection, id)),
            Store::Pg(s) => s.get(collection, id).await,
        }
    }

    /// Writes `body` as the next version after `expected`; `None` writes a new record.
    async fn write_raw(
        &self,
        collection: &str,
        id: &str,
        keys: &Keys,
        body: Value,
        expected: Option<i64>,
    ) -> Result<i64, StoreError> {
        match self {
            Store::Memory(s) => s.write(collection, id, keys, body, expected),
            Store::Pg(s) => s.write(collection, id, keys, body, expected).await,
        }
    }

    async fn list_raw(&self, collection: &str, by: By<'_>) -> Result<Vec<Raw>, StoreError> {
        match self {
            Store::Memory(s) => Ok(s.list(collection, by)),
            Store::Pg(s) => s.list(collection, by).await,
        }
    }

    /// Every write from now on, in this process and, with PostgreSQL, in every other.
    pub fn subscribe(&self) -> broadcast::Receiver<Change> {
        match self {
            Store::Memory(s) => s.subscribe(),
            Store::Pg(s) => s.subscribe(),
        }
    }

    pub async fn get<T: DeserializeOwned>(
        &self,
        collection: &str,
        id: &str,
    ) -> Result<Option<Record<T>>, StoreError> {
        self.get_raw(collection, id)
            .await?
            .map(|raw| decode(collection, raw))
            .transpose()
    }

    /// A new record; `Conflict` if one with this id exists.
    pub async fn insert<T: Serialize>(
        &self,
        collection: &str,
        id: &str,
        keys: Keys,
        body: T,
    ) -> Result<Record<T>, StoreError> {
        let value = encode(collection, id, &body)?;
        let version = self.write_raw(collection, id, &keys, value, None).await?;
        Ok(Record {
            id: id.to_string(),
            keys,
            version,
            body,
        })
    }

    /// The record's next version; `Conflict` if it changed since `record` was read.
    pub async fn update<T: Serialize>(
        &self,
        collection: &str,
        record: Record<T>,
    ) -> Result<Record<T>, StoreError> {
        let value = encode(collection, &record.id, &record.body)?;
        let version = self
            .write_raw(
                collection,
                &record.id,
                &record.keys,
                value,
                Some(record.version),
            )
            .await?;
        Ok(Record { version, ..record })
    }

    /// Records in id order.
    pub async fn list<T: DeserializeOwned>(
        &self,
        collection: &str,
        by: By<'_>,
    ) -> Result<Vec<Record<T>>, StoreError> {
        self.list_raw(collection, by)
            .await?
            .into_iter()
            .map(|raw| decode(collection, raw))
            .collect()
    }
}
