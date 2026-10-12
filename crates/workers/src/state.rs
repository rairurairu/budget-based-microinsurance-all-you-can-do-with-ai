//! Worker state in PostgreSQL, so a restart in the middle of a typhoon neither repays a
//! guaranteed share nor forgets a balance (R27, R51). The trigger desk is saved after each poll,
//! before its outbox is posted: whatever was decided is either posted or still in the saved
//! outbox. One worker leads each job at a time (a session advisory lock); another waits.
use serde::Serialize;
use serde::de::DeserializeOwned;
use sqlx::postgres::{PgConnection, PgPool, PgPoolOptions};

const MIGRATION: &str = include_str!("../migrations/0001_worker_state.sql");

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

#[derive(Debug, Clone)]
pub struct PgState {
    pool: PgPool,
}

/// Held while this process leads a job. Its connection is detached from the pool, so dropping
/// it closes the session and releases the lead.
pub struct Lead {
    _conn: PgConnection,
}

/// A stable 64-bit lock id for a job name (FNV-1a).
fn lock_id(job: &str) -> i64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in job.bytes() {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    i64::from_ne_bytes(h.to_ne_bytes())
}

impl PgState {
    pub async fn connect(database_url: &str) -> Result<PgState, String> {
        let pool = PgPoolOptions::new()
            .max_connections(4)
            .connect(database_url)
            .await
            .map_err(err)?;
        Ok(PgState { pool })
    }

    pub async fn migrate(&self) -> Result<(), String> {
        sqlx::raw_sql(MIGRATION)
            .execute(&self.pool)
            .await
            .map_err(err)?;
        Ok(())
    }

    pub async fn load<T: DeserializeOwned>(&self, name: &str) -> Result<Option<T>, String> {
        let text: Option<String> =
            sqlx::query_scalar("SELECT state FROM worker_state WHERE name = $1")
                .bind(name)
                .fetch_optional(&self.pool)
                .await
                .map_err(err)?;
        text.map(|t| serde_json::from_str(&t).map_err(err))
            .transpose()
    }

    pub async fn save<T: Serialize>(&self, name: &str, state: &T) -> Result<(), String> {
        let text = serde_json::to_string(state).map_err(err)?;
        sqlx::query(
            "INSERT INTO worker_state (name, state) VALUES ($1, $2) \
             ON CONFLICT (name) DO UPDATE SET state = EXCLUDED.state, updated_at = now()",
        )
        .bind(name)
        .bind(text)
        .execute(&self.pool)
        .await
        .map_err(err)?;
        Ok(())
    }

    /// Leads `job` if no other worker does; `None` when another holds it.
    pub async fn try_lead(&self, job: &str) -> Result<Option<Lead>, String> {
        let mut conn = self.pool.acquire().await.map_err(err)?.detach();
        let got: bool = sqlx::query_scalar("SELECT pg_try_advisory_lock($1)")
            .bind(lock_id(job))
            .fetch_one(&mut conn)
            .await
            .map_err(err)?;
        Ok(got.then_some(Lead { _conn: conn }))
    }
}

#[cfg(test)]
mod tests {
    use super::lock_id;

    #[test]
    fn lock_ids_are_stable_and_distinct() {
        assert_eq!(
            lock_id("poll_triggers/hko_warnings"),
            lock_id("poll_triggers/hko_warnings")
        );
        assert_ne!(
            lock_id("poll_triggers/hko_warnings"),
            lock_id("open_policy_period")
        );
    }
}
