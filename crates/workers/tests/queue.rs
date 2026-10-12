//! The job queue (ADR-012): a key is enqueued once, a claim holds a lease, a job whose worker
//! stopped is claimed again, a failed attempt is retried after a backoff and then left failed for
//! a person, and two workers never take the same job. The same scenario runs on the in-memory
//! queue and, with `DATABASE_URL`, on PostgreSQL (in a schema of its own).
use chrono::{DateTime, Duration, TimeZone, Utc};
use microinsurance_workers::queue::{JobStatus, MemoryQueue, NewJob, PgQueue, Queue};
use serde_json::json;
use std::collections::BTreeSet;

fn t0() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 11, 1, 0, 0, 0)
        .single()
        .expect("valid time")
}

fn job(key: &str, run_at: DateTime<Utc>) -> NewJob {
    NewJob {
        kind: "open_policy_period".into(),
        key: key.into(),
        payload: json!({ "policy_id": key }),
        run_at,
    }
}

async fn scenario(q: &Queue) {
    let t = t0();
    let settings = match q {
        Queue::Memory(m) => m.settings,
        Queue::Pg(p) => p.settings,
    };
    // Enqueued once, however often it is scheduled.
    assert!(q.enqueue(job("a", t)).await.expect("enqueue"));
    assert!(!q.enqueue(job("a", t)).await.expect("enqueue again"));
    // Not due before its time.
    assert!(
        q.claim("w1", 10, t - Duration::seconds(1))
            .await
            .expect("claim")
            .is_empty()
    );
    let claimed = q.claim("w1", 10, t).await.expect("claim");
    assert_eq!(claimed.len(), 1);
    let a = &claimed[0];
    assert_eq!((a.key.as_str(), a.attempts), ("a", 1));
    assert_eq!(a.payload["policy_id"], "a");
    // Leased: another worker does not take it.
    assert!(q.claim("w2", 10, t).await.expect("claim").is_empty());
    // A failed attempt is retried after the backoff.
    q.fail(a, "w1", "store busy", t).await.expect("fail");
    assert!(q.claim("w2", 10, t).await.expect("claim").is_empty());
    let retry = t + settings.backoff;
    let again = q.claim("w2", 10, retry).await.expect("claim");
    assert_eq!(again.len(), 1);
    assert_eq!(again[0].attempts, 2);
    // Its worker stops: once the lease runs out, another claims it.
    let lapsed = retry + settings.lease + Duration::seconds(1);
    let third = q.claim("w3", 10, lapsed).await.expect("claim");
    assert_eq!(third.len(), 1);
    assert_eq!(third[0].attempts, 3);
    // Only the worker holding it can finish it.
    q.complete(&again[0], "w2", lapsed)
        .await
        .expect("stale complete");
    assert!(q.rows(JobStatus::Done, 10).await.expect("rows").is_empty());
    q.complete(&third[0], "w3", lapsed).await.expect("complete");
    let done = q.rows(JobStatus::Done, 10).await.expect("rows");
    assert_eq!(done.len(), 1);
    assert_eq!(done[0].key, "a");

    // Out of attempts: failed for good, listed for a person.
    assert!(q.enqueue(job("b", t)).await.expect("enqueue"));
    let mut now = t;
    for n in 1..=settings.max_attempts {
        let claimed = q.claim("w1", 10, now).await.expect("claim");
        assert_eq!(claimed.len(), 1, "attempt {n}");
        q.fail(&claimed[0], "w1", "the policy record is invalid", now)
            .await
            .expect("fail");
        now += settings.max_backoff;
    }
    assert!(q.claim("w1", 10, now).await.expect("claim").is_empty());
    let failed = q.rows(JobStatus::Failed, 10).await.expect("rows");
    assert_eq!(failed.len(), 1);
    assert_eq!(
        failed[0].last_error.as_deref(),
        Some("the policy record is invalid")
    );
    let counts = q.counts().await.expect("counts");
    assert_eq!(counts.get(&JobStatus::Done), Some(&1));
    assert_eq!(counts.get(&JobStatus::Failed), Some(&1));

    // Two workers claiming at once share the work without overlap.
    for i in 0..20 {
        q.enqueue(job(&format!("c{i:02}"), t))
            .await
            .expect("enqueue");
    }
    let (x, y) = tokio::join!(q.claim("w1", 12, now), q.claim("w2", 12, now));
    let (x, y) = (x.expect("claim"), y.expect("claim"));
    let keys: BTreeSet<&str> = x.iter().chain(&y).map(|j| j.key.as_str()).collect();
    assert_eq!(x.len() + y.len(), 20);
    assert_eq!(keys.len(), 20, "no job claimed twice");
}

#[tokio::test]
async fn memory_queue() {
    scenario(&Queue::Memory(MemoryQueue::default())).await;
}

#[tokio::test]
async fn postgres_queue() {
    let Ok(url) = std::env::var("DATABASE_URL") else {
        eprintln!("DATABASE_URL not set: PostgreSQL job queue test skipped");
        return;
    };
    let schema = format!(
        "queue_test_{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    );
    let admin = sqlx::PgPool::connect(&url).await.expect("connects");
    sqlx::raw_sql(&format!("CREATE SCHEMA {schema}"))
        .execute(&admin)
        .await
        .expect("schema");
    let sep = if url.contains('?') { '&' } else { '?' };
    let scoped = format!("{url}{sep}options=-c%20search_path%3D{schema}");
    let q = PgQueue::connect(&scoped).await.expect("connects");
    q.migrate().await.expect("migrates");
    scenario(&Queue::Pg(q)).await;
    sqlx::raw_sql(&format!("DROP SCHEMA {schema} CASCADE"))
        .execute(&admin)
        .await
        .expect("drop schema");
}
