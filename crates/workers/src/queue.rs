//! The job queue for batch work over the book (ADR-012): one PostgreSQL table, no broker.
//!
//! - **Enqueue once.** Every job has a unique key (`open_policy_period/<policy>/<period>`): a key
//!   already queued, running, done or failed is not added again, so a schedule that runs twice,
//!   or two workers scheduling at once, add one job.
//! - **Claim with a lease.** A worker claims due jobs with `FOR UPDATE SKIP LOCKED`, so workers
//!   never wait on each other or take the same job; the claim holds a lease, and a job whose lease
//!   runs out (its worker stopped) is claimed again, up to `max_attempts`.
//! - **Retry, then a person.** A failed attempt is retried after an exponential backoff; after
//!   `max_attempts` the job stays `failed`, listed for a person (`GET /ops/jobs`).
//!
//! Delivery is at least once, so every job is idempotent in what it writes: versioned store
//! writes and the ledger's idempotency keys (R27). Time is passed in, never read here, so a test
//! can run a lease out. `MemoryQueue` passes the same tests (`tests/queue.rs`).
use chrono::{DateTime, Duration, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::Row;
use sqlx::postgres::{PgPool, PgPoolOptions, PgRow};
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, PoisonError};

const MIGRATION: &str = include_str!("../migrations/0002_jobs.sql");

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

/// A job to add.
#[derive(Debug, Clone, PartialEq)]
pub struct NewJob {
    pub kind: String,
    /// Unique: a key is enqueued once.
    pub key: String,
    pub payload: Value,
    pub run_at: DateTime<Utc>,
}

/// A claimed job.
#[derive(Debug, Clone, PartialEq)]
pub struct Job {
    pub id: i64,
    pub kind: String,
    pub key: String,
    pub payload: Value,
    /// This attempt's number, from 1.
    pub attempts: u32,
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum JobStatus {
    Queued,
    Running,
    Done,
    /// Out of attempts: a person looks.
    Failed,
}

impl JobStatus {
    fn as_str(self) -> &'static str {
        match self {
            JobStatus::Queued => "queued",
            JobStatus::Running => "running",
            JobStatus::Done => "done",
            JobStatus::Failed => "failed",
        }
    }

    fn parse(s: &str) -> JobStatus {
        match s {
            "running" => JobStatus::Running,
            "done" => JobStatus::Done,
            "failed" => JobStatus::Failed,
            _ => JobStatus::Queued,
        }
    }
}

/// A job as the ops console lists it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct JobRow {
    pub id: i64,
    pub kind: String,
    pub key: String,
    pub status: JobStatus,
    pub attempts: u32,
    pub run_at: DateTime<Utc>,
    pub last_error: Option<String>,
    pub finished_at: Option<DateTime<Utc>>,
}

/// How long a claim lasts and how a failed job is retried.
#[derive(Debug, Clone, Copy)]
pub struct QueueSettings {
    pub lease: Duration,
    pub max_attempts: u32,
    /// The first retry's delay; each later one doubles, up to `max_backoff`.
    pub backoff: Duration,
    pub max_backoff: Duration,
}

impl Default for QueueSettings {
    fn default() -> QueueSettings {
        QueueSettings {
            lease: Duration::minutes(5),
            max_attempts: 5,
            backoff: Duration::seconds(30),
            max_backoff: Duration::hours(1),
        }
    }
}

impl QueueSettings {
    /// When attempt `attempts` (from 1) that failed is tried again.
    fn retry_at(&self, attempts: u32, now: DateTime<Utc>) -> DateTime<Utc> {
        let doublings = attempts.saturating_sub(1).min(20);
        let delay = self
            .backoff
            .checked_mul(1 << doublings)
            .unwrap_or(self.max_backoff)
            .min(self.max_backoff);
        now + delay
    }
}

/// The queue a process uses.
#[derive(Debug, Clone)]
pub enum Queue {
    Memory(MemoryQueue),
    Pg(PgQueue),
}

impl Queue {
    pub fn memory() -> Queue {
        Queue::Memory(MemoryQueue::default())
    }

    fn settings(&self) -> QueueSettings {
        match self {
            Queue::Memory(q) => q.settings,
            Queue::Pg(q) => q.settings,
        }
    }

    /// Adds the job unless its key was ever added; true when added.
    pub async fn enqueue(&self, job: NewJob) -> Result<bool, String> {
        match self {
            Queue::Memory(q) => Ok(q.enqueue(job)),
            Queue::Pg(q) => q.enqueue(job).await,
        }
    }

    /// Claims up to `limit` jobs due at `now` for `worker`, oldest first. A job whose lease ran
    /// out on its last attempt is marked failed instead.
    pub async fn claim(
        &self,
        worker: &str,
        limit: u32,
        now: DateTime<Utc>,
    ) -> Result<Vec<Job>, String> {
        match self {
            Queue::Memory(q) => Ok(q.claim(worker, limit, now)),
            Queue::Pg(q) => q.claim(worker, limit, now).await,
        }
    }

    /// The job is done; only the worker holding it can say so.
    pub async fn complete(
        &self,
        job: &Job,
        worker: &str,
        now: DateTime<Utc>,
    ) -> Result<(), String> {
        match self {
            Queue::Memory(q) => {
                q.finish(job.id, worker, None, now);
                Ok(())
            }
            Queue::Pg(q) => q.complete(job, worker, now).await,
        }
    }

    /// The attempt failed: retried after the backoff, or failed for good after the last.
    pub async fn fail(
        &self,
        job: &Job,
        worker: &str,
        error: &str,
        now: DateTime<Utc>,
    ) -> Result<(), String> {
        let settings = self.settings();
        let retry =
            (job.attempts < settings.max_attempts).then(|| settings.retry_at(job.attempts, now));
        match self {
            Queue::Memory(q) => {
                q.finish(job.id, worker, Some((error, retry)), now);
                Ok(())
            }
            Queue::Pg(q) => q.fail(job, worker, error, retry, now).await,
        }
    }

    /// Jobs by status, newest first, at most `limit`.
    pub async fn rows(&self, status: JobStatus, limit: u32) -> Result<Vec<JobRow>, String> {
        match self {
            Queue::Memory(q) => Ok(q.rows(status, limit)),
            Queue::Pg(q) => q.rows(status, limit).await,
        }
    }

    /// How many jobs are in each status.
    pub async fn counts(&self) -> Result<BTreeMap<JobStatus, i64>, String> {
        match self {
            Queue::Memory(q) => Ok(q.counts()),
            Queue::Pg(q) => q.counts().await,
        }
    }
}

#[derive(Debug, Clone)]
struct Stored {
    row: JobRow,
    payload: Value,
    max_attempts: u32,
    locked_by: Option<String>,
    locked_until: Option<DateTime<Utc>>,
}

#[derive(Debug, Default)]
struct Jobs {
    next: i64,
    by_id: BTreeMap<i64, Stored>,
    by_key: BTreeMap<String, i64>,
}

/// The reference queue, in memory: tests and demo mode.
#[derive(Debug, Clone)]
pub struct MemoryQueue {
    jobs: Arc<Mutex<Jobs>>,
    pub settings: QueueSettings,
}

impl Default for MemoryQueue {
    fn default() -> MemoryQueue {
        MemoryQueue::with(QueueSettings::default())
    }
}

impl MemoryQueue {
    pub fn with(settings: QueueSettings) -> MemoryQueue {
        MemoryQueue {
            jobs: Arc::default(),
            settings,
        }
    }

    fn jobs(&self) -> std::sync::MutexGuard<'_, Jobs> {
        self.jobs.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn enqueue(&self, job: NewJob) -> bool {
        let mut jobs = self.jobs();
        if jobs.by_key.contains_key(&job.key) {
            return false;
        }
        jobs.next += 1;
        let id = jobs.next;
        jobs.by_key.insert(job.key.clone(), id);
        jobs.by_id.insert(
            id,
            Stored {
                row: JobRow {
                    id,
                    kind: job.kind,
                    key: job.key,
                    status: JobStatus::Queued,
                    attempts: 0,
                    run_at: job.run_at,
                    last_error: None,
                    finished_at: None,
                },
                payload: job.payload,
                max_attempts: self.settings.max_attempts,
                locked_by: None,
                locked_until: None,
            },
        );
        true
    }

    fn claim(&self, worker: &str, limit: u32, now: DateTime<Utc>) -> Vec<Job> {
        let mut jobs = self.jobs();
        let mut due: Vec<(DateTime<Utc>, i64)> = Vec::new();
        for s in jobs.by_id.values_mut() {
            let lapsed = s.row.status == JobStatus::Running
                && s.locked_until.is_some_and(|until| until < now);
            if lapsed && s.row.attempts >= s.max_attempts {
                s.row.status = JobStatus::Failed;
                s.row.last_error = Some("its lease ran out on the last attempt".into());
                s.row.finished_at = Some(now);
                s.locked_by = None;
                continue;
            }
            if (s.row.status == JobStatus::Queued && s.row.run_at <= now) || lapsed {
                due.push((s.row.run_at, s.row.id));
            }
        }
        due.sort();
        due.truncate(usize::try_from(limit).unwrap_or(usize::MAX));
        let mut out = Vec::new();
        for (_, id) in due {
            if let Some(s) = jobs.by_id.get_mut(&id) {
                s.row.status = JobStatus::Running;
                s.row.attempts += 1;
                s.locked_by = Some(worker.to_string());
                s.locked_until = Some(now + self.settings.lease);
                out.push(Job {
                    id,
                    kind: s.row.kind.clone(),
                    key: s.row.key.clone(),
                    payload: s.payload.clone(),
                    attempts: s.row.attempts,
                });
            }
        }
        out
    }

    /// Done (`failure` none), or failed with a retry time (none: for good).
    fn finish(
        &self,
        id: i64,
        worker: &str,
        failure: Option<(&str, Option<DateTime<Utc>>)>,
        now: DateTime<Utc>,
    ) {
        let mut jobs = self.jobs();
        let Some(s) = jobs.by_id.get_mut(&id) else {
            return;
        };
        if s.locked_by.as_deref() != Some(worker) || s.row.status != JobStatus::Running {
            return;
        }
        s.locked_by = None;
        s.locked_until = None;
        match failure {
            None => {
                s.row.status = JobStatus::Done;
                s.row.finished_at = Some(now);
            }
            Some((error, retry)) => {
                s.row.last_error = Some(error.to_string());
                match retry {
                    Some(at) => {
                        s.row.status = JobStatus::Queued;
                        s.row.run_at = at;
                    }
                    None => {
                        s.row.status = JobStatus::Failed;
                        s.row.finished_at = Some(now);
                    }
                }
            }
        }
    }

    fn rows(&self, status: JobStatus, limit: u32) -> Vec<JobRow> {
        self.jobs()
            .by_id
            .values()
            .rev()
            .filter(|s| s.row.status == status)
            .take(usize::try_from(limit).unwrap_or(usize::MAX))
            .map(|s| s.row.clone())
            .collect()
    }

    fn counts(&self) -> BTreeMap<JobStatus, i64> {
        let mut out = BTreeMap::new();
        for s in self.jobs().by_id.values() {
            *out.entry(s.row.status).or_insert(0) += 1;
        }
        out
    }
}

/// The queue in PostgreSQL (`migrations/0002_jobs.sql`).
#[derive(Debug, Clone)]
pub struct PgQueue {
    pool: PgPool,
    pub settings: QueueSettings,
}

fn job_row(row: &PgRow) -> Result<JobRow, String> {
    let status: String = row.try_get("status").map_err(err)?;
    let attempts: i32 = row.try_get("attempts").map_err(err)?;
    Ok(JobRow {
        id: row.try_get("id").map_err(err)?,
        kind: row.try_get("kind").map_err(err)?,
        key: row.try_get("key").map_err(err)?,
        status: JobStatus::parse(&status),
        attempts: u32::try_from(attempts).unwrap_or(0),
        run_at: row.try_get("run_at").map_err(err)?,
        last_error: row.try_get("last_error").map_err(err)?,
        finished_at: row.try_get("finished_at").map_err(err)?,
    })
}

fn attempts(n: u32) -> i32 {
    i32::try_from(n).unwrap_or(i32::MAX)
}

impl PgQueue {
    pub async fn connect(database_url: &str) -> Result<PgQueue, String> {
        let pool = PgPoolOptions::new()
            .max_connections(4)
            .connect(database_url)
            .await
            .map_err(err)?;
        Ok(PgQueue {
            pool,
            settings: QueueSettings::default(),
        })
    }

    pub async fn migrate(&self) -> Result<(), String> {
        sqlx::raw_sql(MIGRATION)
            .execute(&self.pool)
            .await
            .map_err(err)?;
        Ok(())
    }

    async fn enqueue(&self, job: NewJob) -> Result<bool, String> {
        let added = sqlx::query(
            "INSERT INTO jobs (kind, key, payload, run_at, max_attempts) \
             VALUES ($1, $2, $3::jsonb, $4, $5) ON CONFLICT (key) DO NOTHING",
        )
        .bind(&job.kind)
        .bind(&job.key)
        .bind(job.payload.to_string())
        .bind(job.run_at)
        .bind(attempts(self.settings.max_attempts))
        .execute(&self.pool)
        .await
        .map_err(err)?
        .rows_affected();
        Ok(added == 1)
    }

    async fn claim(
        &self,
        worker: &str,
        limit: u32,
        now: DateTime<Utc>,
    ) -> Result<Vec<Job>, String> {
        sqlx::query(
            "UPDATE jobs SET status = 'failed', last_error = 'its lease ran out on the last attempt', \
             finished_at = $1, locked_by = NULL, locked_until = NULL \
             WHERE status = 'running' AND locked_until < $1 AND attempts >= max_attempts",
        )
        .bind(now)
        .execute(&self.pool)
        .await
        .map_err(err)?;
        let rows = sqlx::query(
            "UPDATE jobs SET status = 'running', attempts = attempts + 1, locked_by = $1, \
             locked_until = $2 \
             WHERE id IN (SELECT id FROM jobs \
                 WHERE (status = 'queued' AND run_at <= $3) \
                    OR (status = 'running' AND locked_until < $3) \
                 ORDER BY run_at, id FOR UPDATE SKIP LOCKED LIMIT $4) \
             RETURNING id, kind, key, payload::text AS payload, attempts, run_at",
        )
        .bind(worker)
        .bind(now + self.settings.lease)
        .bind(now)
        .bind(i64::from(limit))
        .fetch_all(&self.pool)
        .await
        .map_err(err)?;
        let mut out: Vec<(DateTime<Utc>, Job)> = Vec::new();
        for row in &rows {
            let payload: String = row.try_get("payload").map_err(err)?;
            let n: i32 = row.try_get("attempts").map_err(err)?;
            out.push((
                row.try_get("run_at").map_err(err)?,
                Job {
                    id: row.try_get("id").map_err(err)?,
                    kind: row.try_get("kind").map_err(err)?,
                    key: row.try_get("key").map_err(err)?,
                    payload: serde_json::from_str(&payload).map_err(err)?,
                    attempts: u32::try_from(n).unwrap_or(0),
                },
            ));
        }
        out.sort_by_key(|(at, job)| (*at, job.id));
        Ok(out.into_iter().map(|(_, j)| j).collect())
    }

    async fn complete(&self, job: &Job, worker: &str, now: DateTime<Utc>) -> Result<(), String> {
        sqlx::query(
            "UPDATE jobs SET status = 'done', finished_at = $3, locked_by = NULL, \
             locked_until = NULL WHERE id = $1 AND locked_by = $2 AND status = 'running'",
        )
        .bind(job.id)
        .bind(worker)
        .bind(now)
        .execute(&self.pool)
        .await
        .map_err(err)?;
        Ok(())
    }

    async fn fail(
        &self,
        job: &Job,
        worker: &str,
        error: &str,
        retry: Option<DateTime<Utc>>,
        now: DateTime<Utc>,
    ) -> Result<(), String> {
        let (status, run_at, finished) = match retry {
            Some(at) => (JobStatus::Queued, Some(at), None),
            None => (JobStatus::Failed, None, Some(now)),
        };
        sqlx::query(
            "UPDATE jobs SET status = $3, last_error = $4, run_at = COALESCE($5, run_at), \
             finished_at = $6, locked_by = NULL, locked_until = NULL \
             WHERE id = $1 AND locked_by = $2 AND status = 'running'",
        )
        .bind(job.id)
        .bind(worker)
        .bind(status.as_str())
        .bind(error)
        .bind(run_at)
        .bind(finished)
        .execute(&self.pool)
        .await
        .map_err(err)?;
        Ok(())
    }

    async fn rows(&self, status: JobStatus, limit: u32) -> Result<Vec<JobRow>, String> {
        sqlx::query(
            "SELECT id, kind, key, status, attempts, run_at, last_error, finished_at FROM jobs \
             WHERE status = $1 ORDER BY id DESC LIMIT $2",
        )
        .bind(status.as_str())
        .bind(i64::from(limit))
        .fetch_all(&self.pool)
        .await
        .map_err(err)?
        .iter()
        .map(job_row)
        .collect()
    }

    async fn counts(&self) -> Result<BTreeMap<JobStatus, i64>, String> {
        let rows = sqlx::query("SELECT status, count(*) AS n FROM jobs GROUP BY status")
            .fetch_all(&self.pool)
            .await
            .map_err(err)?;
        let mut out = BTreeMap::new();
        for row in &rows {
            let status: String = row.try_get("status").map_err(err)?;
            out.insert(JobStatus::parse(&status), row.try_get("n").map_err(err)?);
        }
        Ok(out)
    }
}
