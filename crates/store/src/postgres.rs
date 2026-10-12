//! The PostgreSQL store. Each write is one statement: it checks the version it read (`UPDATE …
//! WHERE version = $n`, or an insert that must not exist) and, for a streamed collection, sends
//! `pg_notify('records', …)` in the same transaction, which every listening process forwards to
//! its own subscribers. Only the collections customers are streamed (`STREAMED`) send a notice:
//! NOTIFY serialises the commits that send one, so a write nobody listens for sends none.
use crate::{By, Change, Keys, Raw, StoreError};
use serde_json::Value;
use sqlx::Row;
use sqlx::postgres::{PgListener, PgPool, PgPoolOptions, PgRow};
use std::time::Duration;
use tokio::sync::broadcast;

const MIGRATION: &str = include_str!("../migrations/0001_records.sql");
const CHANNEL: &str = "records";

/// The collections whose writes are streamed to customers: the payout stream (ADR-011).
const STREAMED: &[&str] = &[crate::collections::PAYOUTS];

fn store(e: &sqlx::Error) -> StoreError {
    StoreError::Store(e.to_string())
}

#[derive(Debug, Clone)]
pub struct PgStore {
    pool: PgPool,
    changes: broadcast::Sender<Change>,
}

fn raw(row: &PgRow) -> Result<Raw, StoreError> {
    let text: String = row.try_get("body").map_err(|e| store(&e))?;
    let id: String = row.try_get("id").map_err(|e| store(&e))?;
    let body = serde_json::from_str(&text).map_err(|e| StoreError::Store(format!("{id}: {e}")))?;
    Ok(Raw {
        keys: Keys {
            customer_id: row.try_get("customer_id").map_err(|e| store(&e))?,
            partner_id: row.try_get("partner_id").map_err(|e| store(&e))?,
        },
        version: row.try_get("version").map_err(|e| store(&e))?,
        id,
        body,
    })
}

/// Forwards every notice on the channel to this process's subscribers, reconnecting after a
/// lost connection.
async fn listen(pool: PgPool, changes: broadcast::Sender<Change>) {
    loop {
        let Ok(mut listener) = PgListener::connect_with(&pool).await else {
            tokio::time::sleep(Duration::from_secs(1)).await;
            continue;
        };
        if listener.listen(CHANNEL).await.is_err() {
            tokio::time::sleep(Duration::from_secs(1)).await;
            continue;
        }
        while let Ok(n) = listener.recv().await {
            if let Ok(change) = serde_json::from_str::<Change>(n.payload()) {
                let _ = changes.send(change);
            }
        }
    }
}

impl PgStore {
    /// Connects with a pool of 8 and starts listening for writes (needs a Tokio runtime).
    pub async fn connect(database_url: &str) -> Result<PgStore, StoreError> {
        PgStore::connect_with(database_url, 8).await
    }

    /// Connects with a pool of `connections`: one per request served at once.
    pub async fn connect_with(database_url: &str, connections: u32) -> Result<PgStore, StoreError> {
        let pool = PgPoolOptions::new()
            .max_connections(connections.max(1))
            .connect(database_url)
            .await
            .map_err(|e| store(&e))?;
        let changes = broadcast::channel(1024).0;
        tokio::spawn(listen(pool.clone(), changes.clone()));
        Ok(PgStore { pool, changes })
    }

    /// Creates the table if it does not exist.
    pub async fn migrate(&self) -> Result<(), StoreError> {
        sqlx::raw_sql(MIGRATION)
            .execute(&self.pool)
            .await
            .map_err(|e| store(&e))?;
        Ok(())
    }

    pub(crate) async fn get(&self, collection: &str, id: &str) -> Result<Option<Raw>, StoreError> {
        sqlx::query(
            "SELECT id, customer_id, partner_id, version, body::text AS body FROM records \
             WHERE collection = $1 AND id = $2",
        )
        .bind(collection)
        .bind(id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| store(&e))?
        .as_ref()
        .map(raw)
        .transpose()
    }

    pub(crate) async fn write(
        &self,
        collection: &str,
        id: &str,
        keys: &Keys,
        body: Value,
        expected: Option<i64>,
    ) -> Result<i64, StoreError> {
        let text = body.to_string();
        let write = match expected {
            None => {
                "INSERT INTO records (collection, id, customer_id, partner_id, version, body) \
                 VALUES ($1, $2, $3, $4, 1, $5::jsonb) \
                 ON CONFLICT (collection, id) DO NOTHING RETURNING version"
            }
            Some(_) => {
                "UPDATE records SET customer_id = $3, partner_id = $4, body = $5::jsonb, \
                 version = version + 1, updated_at = now() \
                 WHERE collection = $1 AND id = $2 AND version = $6 RETURNING version"
            }
        };
        // One statement, so one round trip and one commit. A streamed collection's notice goes
        // out only if the row was written, in the same transaction; other writes send none, as
        // NOTIFY serialises the commits that send one.
        let streamed = STREAMED.contains(&collection);
        let next = if expected.is_some() { 7 } else { 6 };
        let sql = if streamed {
            format!(
                "WITH w AS ({write}) SELECT w.version, pg_notify(${next}, ${})::text FROM w",
                next + 1
            )
        } else {
            format!("WITH w AS ({write}) SELECT w.version, ''::text FROM w")
        };
        let mut query = sqlx::query_as::<_, (i64, String)>(&sql)
            .bind(collection)
            .bind(id)
            .bind(&keys.customer_id)
            .bind(&keys.partner_id)
            .bind(&text);
        if let Some(v) = expected {
            query = query.bind(v);
        }
        if streamed {
            let notice = serde_json::to_string(&Change {
                collection: collection.to_string(),
                id: id.to_string(),
                customer_id: keys.customer_id.clone(),
            })
            .map_err(|e| StoreError::Store(e.to_string()))?;
            query = query.bind(CHANNEL).bind(notice);
        }
        let written = query
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| store(&e))?;
        written.map(|(v, _)| v).ok_or_else(|| StoreError::Conflict {
            collection: collection.to_string(),
            id: id.to_string(),
        })
    }

    pub(crate) async fn list(&self, collection: &str, by: By<'_>) -> Result<Vec<Raw>, StoreError> {
        let select = "SELECT id, customer_id, partner_id, version, body::text AS body FROM records \
                      WHERE collection = $1";
        let rows = match by {
            By::All => {
                sqlx::query(&format!("{select} ORDER BY id COLLATE \"C\""))
                    .bind(collection)
                    .fetch_all(&self.pool)
                    .await
            }
            By::Customer(c) => {
                sqlx::query(&format!(
                    "{select} AND customer_id = $2 ORDER BY id COLLATE \"C\""
                ))
                .bind(collection)
                .bind(c)
                .fetch_all(&self.pool)
                .await
            }
            By::Customers(cs) => {
                sqlx::query(&format!(
                    "{select} AND customer_id = ANY($2) ORDER BY id COLLATE \"C\""
                ))
                .bind(collection)
                .bind(cs)
                .fetch_all(&self.pool)
                .await
            }
            By::Partner(p) => {
                sqlx::query(&format!(
                    "{select} AND partner_id = $2 ORDER BY id COLLATE \"C\""
                ))
                .bind(collection)
                .bind(p)
                .fetch_all(&self.pool)
                .await
            }
        }
        .map_err(|e| store(&e))?;
        rows.iter().map(raw).collect()
    }

    pub(crate) fn subscribe(&self) -> broadcast::Receiver<Change> {
        self.changes.subscribe()
    }
}
