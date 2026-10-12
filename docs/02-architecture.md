# Architecture

**AI where judgement is needed, fixed rules where money is decided.** At runtime everything is
one Rust workspace; Python runs only offline.

```
 apps/ (TypeScript)        customer-app · embed-sdk · partner-console · ops-console
        │  generated client from contracts/openapi.json; a bearer token per principal
        ▼
 crates/api (axum) ───────────▶ crates/ai  ──HTTPS──▶ model provider
        │                       understand · foresee · explain · prevention tasks · verify · assess · rewards
        │                       harness: typed output, timeout, saved fallback; prompts/*.md
        │  routes::ROUTES builds the router and the OpenAPI paths
        ▼
 crates/engine (pure) ◀── config/ (YAML)
        │ decisions
        ▼
 crates/store · crates/ledger · crates/learning (PostgreSQL) ◀── crates/workers ◀── crates/triggers (feeds)
                                                   job queue (SKIP LOCKED): renewals, erasure,
                                                   accumulation, experience studies

 offline:  python/economics ── PyO3 ──▶ crates/engine     python/evals ──HTTP──▶ crates/api
```

## Boundaries

- `crates/engine` depends only on `serde`, `chrono`, `thiserror`. No I/O, no clock, no
  randomness, no AI. Floating-point arithmetic is denied by clippy (R29, R30).
- `crates/ai` is the only crate that talks to a model provider. It returns typed recommendations,
  each checked by fixed rules (one sentence, no amount, no protected attribute, registered ids);
  parse failure, check failure, timeout or error → saved fallback (R22). It has no ledger access
  and never sets an amount (ADR-007).
- `crates/api` wires them together: it asks `ai` for a recommendation and `engine` for the decision,
  keeps records in the policy store (`crates/store`, versioned writes) and posts to the ledger.
  Every call carries a token naming one principal (customer, partner, a named person at the
  insurer, a data source); the kind decides what it may do, and a person decides in their own name
  (ADR-011). The route table (`routes::ROUTES`) builds the router and writes the OpenAPI paths.
- `python/economics` calibrates parameters offline and writes them to config. Through PyO3 it
  runs the same optimiser and pricing code as production, so a backtest is the real system.
- `python/evals` scores every AI component on golden sets through the production code
  (`ai-eval`), offline against saved replies or live with `EVAL_LIVE=1` (ADR-007).
- `contracts/` (OpenAPI + JSON Schema) is generated from Rust types; the TypeScript client is
  generated from it.

## Key flows

**One market at a time.** A process reads `config/` and its market's overlay
(`config/markets/<market>/`, `MARKET`): the same code and image in every market; a market changes
its files, its data sources (each naming its adapter: a feed's own format, or CAP) and its
wording, never the code (ADR-012).

**Batch jobs.** Every worker enqueues the day's `ScheduleDay` and the interval's
`AccumulationSnapshot` by key, so each is added once, then claims due jobs from the PostgreSQL job
queue (`FOR UPDATE SKIP LOCKED`) → `ScheduleDay` enqueues one job per policy or customer due →
`ProposeRenewal` (the notice, kept as issued, R35), `OpenPolicyPeriod` (the change or the issued
default, the premium billed, R03, R04), `EraseCancelled` (R54), monthly `ExperienceStudy` → each
writes once (versioned writes, ledger keys); a failed job is retried, then kept for a person
(`GET /ops/jobs`).

**Monthly renewal.** `workers` opens a policy period → `ai::foresee` reads upcoming events →
`engine::optimiser` re-rates every cover over the rating horizon (R31–R33) and proposes the next schedule
→ `ai::explain` writes one sentence per cover → new sums insured are shown before the policy period
(R35) → customer reallocates and taps (R03), or the current allocation renews automatically re-rated
(R34) → change takes effect next policy period (R04).

**Parametric payout (the hot path).** `triggers` adapter reports a new HKO code (`SourceReport`) →
`engine::triggers::track` (source code → registered event type and stage, R55; one trigger event
per event type and zone) → `engine::triggers::group` (the peril's trigger events into one loss
occurrence, R51) → `engine::triggers::match_occurrence` against an index of active policies by
event type (R08) → `engine::payouts::guaranteed`, then `balance` when the occurrence closes
(R51) → `ledger` posts with idempotency key `(policy, occurrence, stage)` (R27) → recorded once in
the policy store for the customer, with its decision-log row and any audit draw (`workers::book`),
and streamed to their app (server-sent events); the customer takes cash or an offered alternative
(ADR-006, ADR-011). A source that pushes its reports does so through the API's webhook, into the
same desk. Target: notified within seconds at a million policies
(`docs/performance.md`). No AI call is on this path.

**Prevention task.** `ai::prevention` proposes an action and its evidence for a known event, for
one of the insurer's registered measures on a loss-based cover (R18, R38), never an amount →
`engine::prevention::size` sets the customer's reward from the measure's claims data and their
rated relativity (R40) and `offer` checks the cost-benefit test (R18) and the repeat window (R10) →
the customer accepts and sends evidence by the due date → a reused file is caught by its
fingerprint and goes to a person, otherwise `ai::verify` gives a verdict or refers to a person →
`engine::prevention::review` holds 1 in 20 passes for a person before crediting (R14) → points
(R11) and the reward, granted within the monthly cap (R17) into the reward balance (ADR-009).

**Reward and settlement in kind.** The reward balance is spent from `engine::rewards::menu`: cash
for all of it first (R16), then partner rewards the plan opens (R45), ranked by `ai::rewards` →
`engine::suppliers` issues an in-kind order to the supplier, the same as for a payout the customer
took in kind (R28) → the supplier is paid its tariff for what it delivered on a monthly statement;
anything not used in time goes back to the customer (R56).

**Claim.** Notification of loss → `engine::claims::notify` registers it and returns its triage, so
an urgent case's first response goes out at once (R46) → the evidence the cover asks for →
`ai::assess` (approve or refer, an estimate, never an amount paid; reused evidence caught by
fingerprint) → `engine::claims::decide` returns `StpOutcome` (`Approved | Referred`, no decline
possible, R13) within the STP authority (R57), holding the 1-in-20 sample and risk review for a
person before payment (R14) → referrals, audits and review requests go to one queue in the ops
console (`guardrails::queue`) → only `claims::person_decision` can decline, and a different person
reviews a decline (R15; ADR-010).

**New business through a channel.** The partner's server exchanges its own customer reference for a
short-lived customer token → the embed shows every plan, none recommended (R45) → `engine::optimiser`
suggests a split, `ai::explain` writes one sentence per cover, the price carries any promotional
discount (R44) → the quote is kept as shown (R49) → the customer's tap: `consent::bind` checks the
partner's licence that day (R52) and binds from 00:00 the next day (R58) → the first premium is
billed; the partner's wallet confirms its payment. A referral-only partner shows the way to the
insurer's own journey instead (ADR-011).

## Stack

See `docs/decisions/ADR-001-stack.md`.
