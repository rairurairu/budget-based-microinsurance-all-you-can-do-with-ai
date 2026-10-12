# microinsurance-store

The policy store (ADR-011): the system of record for what customers, partners and the insurer's
people do through the API, and for what the workers decide that a customer sees. Each record is
a JSON document in a named collection (`src/collections.rs`), filed by id and, where it has them,
by customer and distribution partner.

- **Versioned writes.** `insert` writes a new id once; `update` names the version it read and
  fails with `Conflict` if someone wrote since, so two taps on one quote bind one policy.
- **What is streamed is heard.** `subscribe` yields writes; with PostgreSQL each write is one
  statement, and a write to a collection streamed to customers (payouts) sends its `pg_notify` in
  it, which every process's listener forwards, so a payout a worker decides reaches the
  customer's live stream in the API. Other writes send no notice: NOTIFY serialises the commits
  that send one (ADR-012).
- **Several customers at once.** `By::Customers` lists the records of a set of customers in one
  query (a partner's results).
- **Two stores, one behaviour.** `MemoryStore` for tests and demo mode, `PgStore` (one `records`
  table, `migrations/0001_records.sql`) in production; both pass `tests/store.rs` (the PostgreSQL
  run needs `DATABASE_URL`).

The store keeps the engine's decisions as they were made; it never decides or computes an amount.
Records are kept for `record_retention_years` (R54).
