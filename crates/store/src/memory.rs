//! The reference store: every record in memory, behind one lock that is never held across an
//! await.
use crate::{By, Change, Keys, Raw, StoreError};
use serde_json::Value;
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, PoisonError};
use tokio::sync::broadcast;

type Records = BTreeMap<(String, String), Raw>;

#[derive(Debug, Clone)]
pub struct MemoryStore {
    records: Arc<Mutex<Records>>,
    changes: broadcast::Sender<Change>,
}

impl Default for MemoryStore {
    fn default() -> MemoryStore {
        MemoryStore {
            records: Arc::default(),
            changes: broadcast::channel(1024).0,
        }
    }
}

impl MemoryStore {
    fn records(&self) -> std::sync::MutexGuard<'_, Records> {
        self.records.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub(crate) fn get(&self, collection: &str, id: &str) -> Option<Raw> {
        self.records()
            .get(&(collection.to_string(), id.to_string()))
            .cloned()
    }

    pub(crate) fn write(
        &self,
        collection: &str,
        id: &str,
        keys: &Keys,
        body: Value,
        expected: Option<i64>,
    ) -> Result<i64, StoreError> {
        let key = (collection.to_string(), id.to_string());
        let version = {
            let mut records = self.records();
            let current = records.get(&key).map(|r| r.version);
            if current != expected {
                return Err(StoreError::Conflict {
                    collection: collection.to_string(),
                    id: id.to_string(),
                });
            }
            let version = expected.map_or(1, |v| v + 1);
            records.insert(
                key,
                Raw {
                    id: id.to_string(),
                    keys: keys.clone(),
                    version,
                    body,
                },
            );
            version
        };
        // No subscriber is not an error.
        let _ = self.changes.send(Change {
            collection: collection.to_string(),
            id: id.to_string(),
            customer_id: keys.customer_id.clone(),
        });
        Ok(version)
    }

    pub(crate) fn list(&self, collection: &str, by: By<'_>) -> Vec<Raw> {
        self.records()
            .range((collection.to_string(), String::new())..)
            .take_while(|((c, _), _)| c == collection)
            .map(|(_, r)| r)
            .filter(|r| match by {
                By::All => true,
                By::Customer(c) => r.keys.customer_id.as_deref() == Some(c),
                By::Customers(cs) => r.keys.customer_id.as_ref().is_some_and(|c| cs.contains(c)),
                By::Partner(p) => r.keys.partner_id.as_deref() == Some(p),
            })
            .cloned()
            .collect()
    }

    pub(crate) fn subscribe(&self) -> broadcast::Receiver<Change> {
        self.changes.subscribe()
    }
}
