-- The job queue for batch work over the book (ADR-012). A job is claimed with
-- FOR UPDATE SKIP LOCKED, so any number of workers share the queue without a broker; a claim
-- holds a lease, and a job whose lease ran out (its worker stopped) is claimed again. `key` is
-- unique: a job is enqueued once however many times it is scheduled. Applied by PgQueue::migrate.
CREATE TABLE IF NOT EXISTS jobs (
    id           BIGSERIAL PRIMARY KEY,
    kind         TEXT NOT NULL,
    key          TEXT NOT NULL UNIQUE,
    payload      JSONB NOT NULL,
    status       TEXT NOT NULL DEFAULT 'queued',   -- queued | running | done | failed
    run_at       TIMESTAMPTZ NOT NULL,
    attempts     INT NOT NULL DEFAULT 0,
    max_attempts INT NOT NULL,
    locked_by    TEXT,
    locked_until TIMESTAMPTZ,
    last_error   TEXT,
    created_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    finished_at  TIMESTAMPTZ
);
CREATE INDEX IF NOT EXISTS jobs_due ON jobs (run_at, id) WHERE status IN ('queued', 'running');
CREATE INDEX IF NOT EXISTS jobs_by_status ON jobs (status);
