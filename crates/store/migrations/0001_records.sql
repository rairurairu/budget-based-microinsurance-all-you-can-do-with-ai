-- The policy store (ADR-011): one row per record, its body as JSON. `version` rises by one on
-- every write; a write names the version it read, so two writers never both win (optimistic
-- concurrency). Applied by PgStore::migrate.
CREATE TABLE IF NOT EXISTS records (
    collection  TEXT NOT NULL,
    id          TEXT NOT NULL,
    customer_id TEXT,
    partner_id  TEXT,
    version     BIGINT NOT NULL,
    body        JSONB NOT NULL,
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (collection, id)
);
CREATE INDEX IF NOT EXISTS records_by_customer ON records (collection, customer_id);
CREATE INDEX IF NOT EXISTS records_by_partner ON records (collection, partner_id);
