# microinsurance-workers

Scheduled jobs on a PostgreSQL queue (`FOR UPDATE SKIP LOCKED`).

Built (Phase 2): `PollTriggers` (`src/desk.rs`) and the monthly renewal, `ProposeRenewal` and
`OpenPolicyPeriod` (`src/renewal.rs`). `PollTriggers` polls every source with a built adapter and
a live URL (`src/sources.rs`: HKO warnings, MTR line status, TD road closures, flight status,
organiser notices), each with its own desk. `DEMO_MODE=1 cargo run -p microinsurance-workers` (from the
repository root) replays a Signal 8, a Tsuen Wan Line suspension and a concert cancellation from
saved responses against a synthetic book and prints each payout. With `DATABASE_URL`, the ledger and the trigger desk's state
(`worker_state`) are in PostgreSQL, saved after each poll before posting, and one worker leads
each source's poll (advisory lock).

Built (Phase 4): the prevention task jobs (`src/prevention.rs`: `PreventionDesk::propose` and
`check`) and supplier settlement (`src/suppliers.rs`). Demo mode then runs one prevention task on
saved AI replies: proposal, sizing, the verifier's pass, the reward granted and spent on a partner
reward, and the supplier's statement.

Built (Phase 5): the claims jobs (`src/claims.rs`: `ClaimsDesk::notify`, `assess`, `person_decides`, each
logged as a `DecisionRecord`) and the people's queue (`src/ops.rs`). Demo mode then runs four claims
on saved AI replies (paid straight through; declined in its waiting period and put to a second
person; an injury with its first response and guarantee of payment; a reused photo held for risk
review), a review request on a payout and the fairness report on a synthetic audit store
(`fixtures/guardrails_synthetic.json`). With `OPS_SNAPSHOT=<path>` it writes the ops console's
snapshot there (`apps/ops-console/demo/snapshot.json` is that output, checked by
`tests/workers.rs`).

| Job | Schedule | Does |
|---|---|---|
| `PollTriggers` | every 60 s per source | adapters → engine matching → payouts (idempotent, R27) |
| `OpenPolicyPeriod` | renewal | apply accepted offers (R04), bill plans, post ledger |
| `ProposeRenewal` | 7 days before renewal, and on new upcoming events | foresee → optimiser → explain → offer |
| `RefreshPreventionTasks` | renewal, and on new upcoming events | prevention task writer → size (R40) → admit (R18) → offer (`src/prevention.rs`) |
| `CheckEvidence` | on evidence | fingerprint → verifier → review: credit, a person, or not verified (R10, R14) (`src/prevention.rs`) |
| `ExpireInKindOrders` | daily | orders not delivered in time go back to the customer (R56) (`src/suppliers.rs`) |
| `SettleSuppliers` | monthly | each supplier's statement for what it delivered, posted once (R56) (`src/suppliers.rs`) |
| `NotifyClaim` | on notification | `claims::notify`: registered and triaged at once; an urgent case's first response goes out within seconds (R46) (`src/claims.rs`) |
| `AssessClaim` | on complete evidence | fingerprint → assessor → `claims::decide`: paid straight through within the STP authority, or to a person (R13, R14, R57) (`src/claims.rs`) |
| `ScheduleDay` | daily, local time | enqueues the day's jobs over the book (`src/jobs.rs`) |
| `EraseCancelled` | daily | erase a cancelled customer's optional data, with a record (R20, R54) (`src/jobs.rs`) |
| `AccumulationSnapshot` | every `accumulation.refresh_minutes` | benefit in force per zone, read by quotes (R51) (`src/jobs.rs`) |
| `ExperienceStudy` | monthly | the experience study for the actuary (`src/jobs.rs`, ADR-012) |

Points fade by the date they are read on (R12), payouts are sampled for audit as they are recorded
(R14, `book`) and the fairness report is computed with the people's queue (R19, `ops`): none needs a
batch job.

Built (Phase 6): the book in the policy store (`src/book.rs`, ADR-011). With `DATABASE_URL`,
`PollTriggers` reads every policy from the store the API keeps, afresh for each poll, and records
what each poll decided where the customer sees it: the payout (streamed to them by the API), its
decision-log row (so they can ask a person to review it, R15) and the payouts drawn for audit after
payment (R14, `AUDIT_SEED`). The API records the reports a source pushes to its webhook the same
way. The job queue for the batch jobs below moves to Phase 7.

Built (Phase 7, ADR-012): the job queue (`src/queue.rs`, `migrations/0002_jobs.sql`): one
PostgreSQL table claimed with `FOR UPDATE SKIP LOCKED`, a key enqueued once, a lease that a
stopped worker's job outlives, backoff, and failed jobs kept for a person (`GET /ops/jobs`); the
in-memory queue passes the same tests (`tests/queue.rs`). The batch jobs (`src/jobs.rs`) run from
it: `ProposeRenewal` issues the renewal notice and keeps it as issued (R35), `OpenPolicyPeriod`
opens the period on the customer's accepted change or the issued default and bills the premium
(R03, R04; unchanged without a notice in time; not for a cancelled or lapsed policy), and the jobs
in the table above (`tests/jobs.rs`). Renewals rate through `src/rating.rs`, shared with the API.
Sources are built from each config file's `adapter` (`src/sources.rs`); `MARKET` picks the market.
Demo mode reads the market's script (`fixtures/demo_script.yaml`,
`fixtures/markets/<market>/demo_script.yaml`, `src/demo.rs`): `MARKET=tw DEMO_MODE=1 cargo run -p
microinsurance-workers` replays a typhoon's work suspensions in Taiwan (`tests/markets.rs`).
Live, every worker schedules the day's jobs and runs those due; `WORKER_POLLS=0` runs a worker
for the jobs only.
