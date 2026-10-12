# ADR-011: Channels: the API, the policy store, identity, binding and the front ends

- Status: Accepted
- Date: 2026-10-10
- Decider: engineering, building Phase 6 on the criteria below (delegated design decisions, as for
  ADR-006, ADR-009 and ADR-010); for Tsang Cheuk Nam's review. Two rows amend owner decisions and
  are flagged for confirmation: the front ends (ADR-001) and how the OpenAPI paths are written
  (ADR-004).

## Context

Phase 6 (docs/08-roadmap.md) puts the method in front of customers, partners and the insurer's
people: the API routes and the generated client, the customer app, the embed SDK and the partner
console. Left open by earlier phases: the policy store and the job queue (Phase 2), recording
what the optimiser's suggestion showed (R49, Phase 1), the API behind the ops console's buttons
and the guarantee-of-payment exchange (Phase 5), and the OpenAPI paths (ADR-004). Not yet
decided anywhere: who may call what, how a policy is bound (new business), how a partner's app
gets a customer onto the insurer's journey without holding their credentials, and how a payout
reaches the customer's screen.

Criteria, in order: nothing changes without the customer's tap and a quote binds once (R03);
nobody can act in another's name, so four eyes cannot be bypassed (R15); a partner never sees an
individual (R21) and binds only with a licence (R52); every route is in the contract; the
mechanisms read like existing policy administration, intermediary and data-protection practice,
so the insurer, its auditors and the Insurance Authority recognise them; demo mode runs offline.

## Decision

| Question | Decision | Industry basis |
|---|---|---|
| The system of record | The policy store (`crates/store`): one JSON record per id in named collections (customers, rating answers, quotes, policies, endorsements, payouts, tasks, claims, review requests, the decision log, audits…), filed by customer and partner. Every write names the version it read (`UPDATE … WHERE version = n`), so two taps on one quote bind one policy. PostgreSQL in production, with `pg_notify` so every API process hears every write; an in-memory store passes the same tests. Records are kept for `record_retention_years`; cancellation rewrites the customer's record, never deletes a policy's (R54) | A policy administration system as the single system of record; optimistic locking against lost updates |
| Who is calling | A bearer token names one principal: a customer (and the partner channel they came through), a distribution partner's server, a named person at the insurer, or a data source. Tokens are HMAC-SHA-256 signed (`v1.<claims>.<signature>`) and expire. The principal's kind fixes what it may do: a customer acts only on their own records, a partner sees only its own aggregates, a data source pushes only its own reports. The person deciding a work item is the token's staff id, never a field of the request, so a decline cannot be reviewed by the person who made it (R15) | OAuth 2.0 bearer tokens; segregation of duties enforced by identity, not by trust in the client |
| The embedded journey | Token exchange: the partner's server sends its own token and its reference for the customer and gets a short-lived customer token (`customer_token_minutes`) for the embed. The customer's id is the insurer's (a hash of partner and reference), never the partner's own. Whether the channel binds or refers is decided at the tap (R52) and shown before (`GET /channels/{partner}`); a referral-only partner shows the way to `own_journey_url` | OAuth 2.0 token exchange (RFC 8693); appointed agent or broker binds, an introducer only refers |
| New business (R58, new) | A quote, then the customer's tap. The quote is valid on the market-local day it was made, at that day's rates; the tap binds through a partner licensed that day or the insurer's own journey (R52), for a schedule of bundled covers totalling the plan price (R24, R34); cover starts at 00:00 local the next day and every waiting period runs from then (R07). The first period's premium is billed at once | Cover commences the day after acceptance, never for a loss already known; a quotation is valid for a stated period |
| What was shown (R49) | Every quote is kept as shown: the offer, the schedule, the suggestion with the factors behind it, the sentence per cover and who wrote it (model or saved answer), the price with any promotional discount, the policy period. Accepting it is recorded on it; a quote is accepted once | Record keeping for product information issued under the insurer's licence; the client file an intermediary keeps |
| Rates for a period starting mid-month | A rate is effective from its date: a period takes the rating basis entry in force on its first day, and an entry is in force for one policy period at most, so a curve that has run out is an error, never a stale rate (`rating::horizon_cost`) | Filed rates apply to policies incepting or renewing on or after their effective date |
| Premium collection | The partner's wallet collects the premium and confirms it (`POST /partners/me/premium-payments`), posted as premium received (R53). The insurer's own payment provider is the insurer's integration. Demo mode records the first premium as paid at binding | Premium collected by the appointed agent and accounted to the insurer |
| A payout on the customer's screen | Each payout the trigger desk decides is recorded once for the customer, with its decision-log row (so it can be put to a person, R15) and any audit draw (R14), whether a worker polled the source or the source pushed it to `POST /webhooks/triggers/{source}` (`workers::book::record`). The customer's page streams them (`GET /me/payouts/stream`, server-sent events read with fetch, so the token stays in a header, never a URL) and chooses cash or an alternative when one is offered (R16, R28). With PostgreSQL the workers read the book from the store for every poll | Event notification from the system of record; the customer, not the system, chooses settlement in kind |
| The contract | The API's route table (`api::routes::ROUTES`) is the only list of routes: the router is built from it and `crates/contracts` writes the OpenAPI paths from it, referencing the same schemars schemas. utoipa is not used: its derives would describe every type a second time. A test checks every listed route is served and guarded. This amends ADR-004's "utoipa paths come with the routes in Phase 6" | One source of truth for an interface; drift fails CI |
| The front ends | Framework-free TypeScript compiled by `tsc`: each screen is a pure function from the API's data to escaped HTML, tested with Node's test runner; a thin runtime calls the generated client. The API serves the built apps. Customer-facing words are config (`config/copy/<tag>.yaml`), one file per offered language with the same keys and placeholders, checked at start-up; the customer's language setting picks the file. The embed is a Web Component in a shadow root. This amends ADR-001's front-end row (React 19, Vite, TanStack Query): the reference apps are small, run offline in demo mode with no extra toolchain, and adopters build their own front ends on the generated client; moving a screen to React later is a rendering change, as its logic is already pure | Reference implementations an insurer's own channel teams replace; approved customer wording held as controlled config |
| Partner insights (R21) | A partner sees aggregates over the customers who came through it, and over everyone who took one of its rewards: subscribers, cover in force, prevention completion, payouts, redemptions. A figure from fewer than `channels.partner_insights_min_cohort` customers is withheld, not rounded (was a constant) | Small-cell suppression in statistical disclosure control |
| The people's queue | `GET /ops/queue` builds the queue from the store in the engine's order (urgent first, R46) with the fairness audit and overturn rates; `POST /ops/decisions` takes a person's action on an item and refuses any action the engine did not offer it (R13): a decline needs a reason from the closed list, an approval the loss assessed (R48) | A claims work queue with authority limits enforced by the system |
| Guarantees of payment and identity | A guarantee above `auto_guarantee_cap` is recorded with the assistance provider's reference for a person to follow (R46); identity checks start and are read through the eKYC provider (R25). Both are adapters (`AssistanceProvider`, `KycProvider`) each insurer connects; demo mode records the guarantee and verifies at once | Third-party administrators and eKYC providers behind a defined interface |
| AI steps and batch jobs | The AI steps a request needs (explain, verify, assess, rank, understand) run inline within the harness's timeout, with the saved answer on any failure (R22), so no request waits on a queue. The PostgreSQL job queue for batch jobs over the book (renewal, fading, erasure) moves to Phase 7 with the deployment | Bounded synchronous calls with a fallback; batch work scheduled where it is operated |
| Demo mode | `DEMO_MODE=1` runs the API offline on a synthetic book seeded from the engine's own suggestions, with saved AI replies, a clock starting at 15:00 Hong Kong time on 18 November 2026 (when the organiser's saved notice cancels a concert), synthetic principals' tokens (`GET /demo/sessions`) and a replay of each source's saved response (`POST /demo/replay/{source}`); none of these routes exists outside demo mode | A sandbox that cannot be mistaken for production |

## Consequences

- R58 is new; R49 is enforced (the quote record, tested in `crates/api/tests`); R21's cohort is
  config; R52 is also checked at the tap; R31's rates are effective-dated.
- New crate `crates/store`; `crates/api` builds on `crates/workers` (desks, claims and prevention
  jobs, `book`); `crates/learning` gains `partner_results`; the ledger gains `premium_received`;
  `crates/triggers` takes a pushed payload (`Feed::Inline`).
- Config gains `channels` (own journey URL, token lifetime, the insights cohort, partners' web
  origins), `config/copy/` (app words per language), illustrative distribution partners (a
  licensed agency that binds, a referral-only ticketing partner that also supplies rewards) and,
  illustratively, the claim-based covers opening first on the larger plans as plan privileges
  (R45). `fixtures/rating_basis_synthetic.json` is the synthetic rating basis for demo mode.
- `just dev` builds the apps and runs the API in demo mode; `just gen` writes the paths.
- Open: hosting for front ends (ADR-001); the insurer's identity provider for sign-in on its own
  journey; the payment provider and the assistance provider integrations; the job queue for batch
  jobs (Phase 7); a load test of the non-AI routes against `docs/performance.md`.
