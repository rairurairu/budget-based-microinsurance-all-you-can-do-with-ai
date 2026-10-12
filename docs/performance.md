# Performance

Targets are design goals, verified by `just bench` (criterion) and load tests before a pilot.

| Path | Why it is hot | Target | Measured by |
|---|---|---|---|
| Trigger fan-out | Signal 8 matches every active weather policy at once | 1,000,000 policies matched and payout decisions made in < 5 s on 4 vCPU (measured 2026-10-09: 1.6 s on one core, by loss occurrence) | `benches/engine.rs: trigger_fanout` |
| Trigger → customer notified | "Payout notification within seconds" | p99 < 10 s from feed update, at 1M policies | load test |
| Urgent first response | Help must not wait for a person (R46) | first response delivered within `urgent_response_seconds` (default 5 s), p99 | load test |
| Optimiser | Runs for every customer at each policy period and on new upcoming events | < 50 µs per customer (1M customers < 1 min on one core; measured 2026-10-10: 32.8 µs) | `benches/engine.rs: optimise_one_customer` |
| API (non-AI routes) | Embedded in partner apps | p99 < 50 ms. Against PostgreSQL (below): met for every customer and public read and for quotes; not yet for binding and the back-office reads | `cargo run --release -p microinsurance-api --example load_test` |
| Ledger postings | Burst after a trigger | ≥ 5,000 idempotent postings/s (measured 2026-10-09, local PostgreSQL 16 in Docker: 8,400/s first post, 13,800/s retries) | `cargo run --release -p microinsurance-ledger --example throughput` |
| AI calls | I/O bound, not CPU | timeout 4 s, then saved fallback | harness metrics |

Design choices that serve these targets: Rust engine with integer money; policies indexed by
event type; batch ledger inserts with `ON CONFLICT DO NOTHING` on the idempotency key; a
PostgreSQL job queue (`FOR UPDATE SKIP LOCKED`) so no extra broker is needed.

Decide: the exact targets with the adopting insurer's expected volumes.

## Load test against PostgreSQL (2026-10-10, ADR-012)

`cargo run --release -p microinsurance-api --example load_test` against the release API with
`DATABASE_URL` (PostgreSQL 16 in Docker on the same laptop, `DATABASE_POOL_SIZE` 32) and one
jobs-only worker (`WORKER_POLLS=0`) keeping the accumulation snapshot. The book held about 1,100
synthetic customers, all through one partner. 200 customers bound through the partner's embedded
journey after an untimed warm-up, then 16 clients read for 30 s (297 calls/s).

| Route | p50 ms | p95 ms | p99 ms | Target |
|---|---|---|---|---|
| `GET /plans`, `GET /registry` | 0.8–0.9 | 2.4–2.6 | 4.9–8.2 | met |
| `GET /me`, `/me/payouts`, `/me/claims`, `/me/decisions` | 7.5–7.8 | 21–23 | 32–38 | met |
| `POST /partners/me/customer-tokens` | 0.8 | 1.5 | 1.8 | met |
| `POST /me/quotes` | 26 | 34 | 40 | met |
| `PATCH /me/profile` | 22 | 33 | 199 | missed (a few slow commits) |
| `POST /me/quotes/{id}/accept` (binding) | 57 | 67 | 75 | missed |
| `GET /ops/queue`, `GET /ops/jobs` (people at the insurer) | 16–42 | 37–76 | 80–96 | missed |
| `GET /partners/me/results` (the partner's console) | 410 | 612 | 683 | missed |

Fixed on the way, each found by the test (ADR-012): the partner's results asked five queries per
customer (850 ms at 50 customers; now five queries in all); every quote measured the whole book
for the accumulation limit (now a snapshot every `refresh_minutes`; quotes went from 84 to 26 ms
at p50); every store write was a three-statement transaction sending a NOTIFY, which serialises
commits (now one statement, a notice only for payouts); the pool of 8 connections queued 16
clients (now `DATABASE_POOL_SIZE`).

Open, in order: the partner's results read every customer of the partner per request and grow with
them; as management information they belong in a daily job, read as of last night. The ops queue
reads every claim and task to find the open ones; open work items need their own index. Binding is
five writes (policy, quote, ledger, decision log, reads): one transaction would halve it. Then a
run on the target hardware (RDS, Fargate) at the insurer's expected volumes.
