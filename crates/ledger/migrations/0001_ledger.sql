-- Double-entry ledger (R27). One row per idempotency key; its postings balance to zero.
-- Applied by PgLedger::migrate; every statement is idempotent.
CREATE TABLE IF NOT EXISTS ledger_entries (
    idempotency_key TEXT PRIMARY KEY,
    -- The postings as "account:amount;..." in order, to tell a retry from a reused key.
    fingerprint     TEXT NOT NULL,
    posted_at       TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE IF NOT EXISTS ledger_postings (
    idempotency_key TEXT NOT NULL REFERENCES ledger_entries (idempotency_key),
    line            SMALLINT NOT NULL,
    account         TEXT NOT NULL,
    amount_cents    BIGINT NOT NULL,
    PRIMARY KEY (idempotency_key, line)
);

CREATE INDEX IF NOT EXISTS ledger_postings_account ON ledger_postings (account);
