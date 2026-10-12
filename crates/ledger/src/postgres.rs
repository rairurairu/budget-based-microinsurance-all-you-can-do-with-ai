//! The PostgreSQL store. Entries are inserted with `ON CONFLICT DO NOTHING` on their idempotency
//! key; when a key exists, the stored fingerprint tells a retry (`AlreadyPosted`) from a reused
//! key (`KeyReused`). A batch is set-based (three statements, one transaction) so a trigger burst
//! posts at the rate in docs/performance.md; an entry and its postings commit together.
use crate::{Account, LedgerError, LedgerStore, PostOutcome, Posting, check_balanced, fingerprint};
use microinsurance_engine::money::Money;
use sqlx::postgres::{PgPool, PgPoolOptions};
use std::collections::{BTreeMap, BTreeSet};

const MIGRATION: &str = include_str!("../migrations/0001_ledger.sql");

fn store(e: &sqlx::Error) -> LedgerError {
    LedgerError::Store(e.to_string())
}

#[derive(Debug, Clone)]
pub struct PgLedger {
    pool: PgPool,
}

impl PgLedger {
    pub async fn connect(database_url: &str) -> Result<PgLedger, LedgerError> {
        PgLedger::connect_with(database_url, 8).await
    }

    /// Connects with a pool of `connections`.
    pub async fn connect_with(
        database_url: &str,
        connections: u32,
    ) -> Result<PgLedger, LedgerError> {
        let pool = PgPoolOptions::new()
            .max_connections(connections.max(1))
            .connect(database_url)
            .await
            .map_err(|e| store(&e))?;
        Ok(PgLedger { pool })
    }

    pub fn from_pool(pool: PgPool) -> PgLedger {
        PgLedger { pool }
    }

    /// Creates the ledger tables if they do not exist.
    pub async fn migrate(&self) -> Result<(), LedgerError> {
        sqlx::raw_sql(MIGRATION)
            .execute(&self.pool)
            .await
            .map_err(|e| store(&e))?;
        Ok(())
    }

    /// Writes the batch's new keys in one transaction and returns the stored fingerprint of
    /// every key that already existed.
    async fn write(
        &self,
        first: &BTreeMap<&str, (String, &[Posting])>,
    ) -> Result<BTreeMap<String, String>, sqlx::Error> {
        let keys: Vec<&str> = first.keys().copied().collect();
        let prints: Vec<&str> = first.values().map(|(f, _)| f.as_str()).collect();
        let mut tx = self.pool.begin().await?;
        let inserted: BTreeSet<String> = sqlx::query_scalar(
            "INSERT INTO ledger_entries (idempotency_key, fingerprint) \
             SELECT * FROM UNNEST($1::text[], $2::text[]) \
             ON CONFLICT (idempotency_key) DO NOTHING RETURNING idempotency_key",
        )
        .bind(&keys)
        .bind(&prints)
        .fetch_all(&mut *tx)
        .await?
        .into_iter()
        .collect();
        let (mut p_keys, mut lines, mut accounts, mut amounts) =
            (Vec::new(), Vec::new(), Vec::new(), Vec::new());
        for (key, (_, entries)) in first.iter().filter(|(k, _)| inserted.contains(**k)) {
            for (i, p) in entries.iter().enumerate() {
                p_keys.push(*key);
                lines.push(i16::try_from(i).unwrap_or(i16::MAX));
                accounts.push(p.account.as_str());
                amounts.push(p.amount.0);
            }
        }
        if !p_keys.is_empty() {
            sqlx::query(
                "INSERT INTO ledger_postings (idempotency_key, line, account, amount_cents) \
                 SELECT * FROM UNNEST($1::text[], $2::smallint[], $3::text[], $4::bigint[])",
            )
            .bind(&p_keys)
            .bind(&lines)
            .bind(&accounts)
            .bind(&amounts)
            .execute(&mut *tx)
            .await?;
        }
        let existing: Vec<&str> = keys
            .iter()
            .copied()
            .filter(|k| !inserted.contains(*k))
            .collect();
        let stored: Vec<(String, String)> = if existing.is_empty() {
            Vec::new()
        } else {
            sqlx::query_as(
                "SELECT idempotency_key, fingerprint FROM ledger_entries \
                 WHERE idempotency_key = ANY($1::text[])",
            )
            .bind(&existing)
            .fetch_all(&mut *tx)
            .await?
        };
        tx.commit().await?;
        Ok(stored.into_iter().collect())
    }
}

impl LedgerStore for PgLedger {
    async fn post(
        &self,
        idempotency_key: &str,
        entries: &[Posting],
    ) -> Result<PostOutcome, LedgerError> {
        let batch = [(idempotency_key.to_string(), entries.to_vec())];
        self.post_many(&batch)
            .await
            .pop()
            .unwrap_or_else(|| Err(LedgerError::Store("no outcome".into())))
    }

    /// One transaction for the batch. An unbalanced entry or a reused key fails alone; a key
    /// repeated within the batch is posted once.
    async fn post_many(
        &self,
        batch: &[(String, Vec<Posting>)],
    ) -> Vec<Result<PostOutcome, LedgerError>> {
        let mut first: BTreeMap<&str, (String, &[Posting])> = BTreeMap::new();
        for (key, entries) in batch {
            if check_balanced(entries).is_ok() {
                first
                    .entry(key.as_str())
                    .or_insert_with(|| (fingerprint(entries), entries.as_slice()));
            }
        }
        let stored = if first.is_empty() {
            Ok(BTreeMap::new())
        } else {
            self.write(&first).await
        };
        let stored = match stored {
            Ok(s) => s,
            Err(e) => return batch.iter().map(|_| Err(store(&e))).collect(),
        };
        let mut seen: BTreeSet<&str> = BTreeSet::new();
        batch
            .iter()
            .map(|(key, entries)| {
                check_balanced(entries)?;
                let print = fingerprint(entries);
                let earlier = match stored.get(key) {
                    Some(f) => Some(f.as_str()),
                    None if !seen.insert(key.as_str()) => {
                        first.get(key.as_str()).map(|(f, _)| f.as_str())
                    }
                    None => None,
                };
                match earlier {
                    None => Ok(PostOutcome::Posted),
                    Some(f) if f == print => Ok(PostOutcome::AlreadyPosted),
                    Some(_) => Err(LedgerError::KeyReused(key.clone())),
                }
            })
            .collect()
    }

    async fn balance(&self, account: Account) -> Result<Money, LedgerError> {
        let sum: i64 = sqlx::query_scalar(
            "SELECT COALESCE(SUM(amount_cents), 0)::bigint FROM ledger_postings WHERE account = $1",
        )
        .bind(account.as_str())
        .fetch_one(&self.pool)
        .await
        .map_err(|e| store(&e))?;
        Ok(Money(sum))
    }
}
