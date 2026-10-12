-- Worker state kept between runs, one row per job (for example the trigger desk: open trigger
-- events, their payout books and the ledger outbox). Applied by PgState::migrate.
CREATE TABLE IF NOT EXISTS worker_state (
    name       TEXT PRIMARY KEY,
    state      TEXT NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
