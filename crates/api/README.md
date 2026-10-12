# microinsurance-api

HTTP API (axum, ADR-011). It asks `ai` for recommendations and `engine` for every decision,
keeps records in the policy store (`crates/store`), posts the engine's amounts to `ledger`
unchanged and streams payouts to the customer. It never decides an amount.

Every route is listed once, in `src/routes.rs` (`ROUTES`): the router is built from the table and
`crates/contracts` writes the OpenAPI paths from it (`contracts/openapi.json`, `just gen`), so a
route cannot be served without being in the contract. `tests/api.rs` checks every listed route is
served and guarded, and runs the journeys end to end in demo mode.

| Area | Routes | Caller | Rules |
|---|---|---|---|
| Catalogue | `GET /plans`, `/covers`, `/channels/{partner_id}`, `/registry`, `/rating-questions`, `/copy/{language}` | public | R01, R23, R24, R44, R45, R52, R55 |
| The customer | `GET /me`, `PATCH /me/profile`, `PUT /me/consents`, `GET/PUT /me/rating-answers`, `POST /me/proposal-form` (+ `/answers`) | customer | R08, R19, R20, R33, R55 |
| Quotes and the tap | `POST /me/quotes`, `GET /me/quotes/{quote_id}`, `POST /me/quotes/{quote_id}/accept` | customer | R02–R05, R34, R44, R45, R49, R52, R58 |
| The policy | `GET /me/policy`, `/me/renewal`, `POST /me/cancel`, `POST/GET /me/identity-checks`, `GET /me/decisions` | customer | R15, R20, R25, R35, R53, R54 |
| Prevention and rewards | `GET /me/prevention-tasks`, `POST …/refresh`, `…/{task_id}/accept`, `…/evidence`, `…/review-request`, `GET /me/score`, `GET /me/rewards`, `POST /me/rewards/redeem` | customer | R10–R12, R14–R18, R36, R38, R40, R45, R56 |
| Recovery | `GET /me/payouts`, `GET /me/payouts/stream` (server-sent events), `POST /me/payouts/{payout_key}/choice`, `POST/GET /me/claims`, `…/{claim_id}/evidence`, `…/choice`, `POST /me/review-requests` | customer | R08, R13–R16, R27, R28, R46, R48, R53, R57 |
| Partners | `POST /partners/me/customer-tokens` (token exchange), `GET /partners/me/results`, `GET/PUT /partners/me/reward-offers`, `POST /partners/me/premium-payments` | partner | R16, R21, R52, R53, R56 |
| People at the insurer | `GET /ops/queue`, `POST /ops/decisions`, `GET /ops/jobs`, `GET /ops/experience`, `GET /ops/pilot` | staff | R13–R15, R19, R46, R48, R59 |
| Data sources | `POST /webhooks/triggers/{source}` (the source's own payload) | feed | R08, R14, R27, R51, R55 |

A token (`Authorization: Bearer …`) names one principal (`src/auth.rs`); the person deciding a
work item is the token's, never a field of the request. Path parameters are percent-encoded: some
ids contain `/`. A refusal is `{ code, rule, detail }`, with the rule from `docs/rules.md` when one
refused it (`src/error.rs`).

## Running

```bash
just dev
```

builds the apps and runs the API in demo mode (`DEMO_MODE=1`): offline, a synthetic book seeded
from the engine's own suggestions, saved AI replies, the clock at 15:00 Hong Kong time on 18
November 2026, the apps at `/app`, `/embed`, `/partner` and `/ops`. Demo mode adds
`GET /demo/sessions` (the synthetic principals' tokens), `POST /demo/replay/{source}` (a source's
saved response, pushed as if the source had sent it) and `GET /demo/evidence/{file}`.

Outside demo mode (`src/lib.rs`, `Settings::from_env`): `API_TOKEN_SECRET` (at least 32 bytes),
`AUDIT_SEED` (the insurer's secret, R14), `RATING_BASIS` (the rating basis JSON from
`python/economics` and the filed rates), optionally `PREVENTION_BASIS`, `AUDIT_STORE`,
`DATABASE_URL` (the store, ledger, desk state and job queue in PostgreSQL; `DATABASE_POOL_SIZE`
connections each for the store and the ledger, default 32), `MARKET` (a market overlay, ADR-012), `ANTHROPIC_API_KEY` (without it
every AI component gives its saved answer, R22), `INSURER_PROFILE`, `PORT`. In demo mode,
`AI_LIVE=1` with `ANTHROPIC_API_KEY` keeps the synthetic book but calls the live model
(`just dev-live`); without a key it replays the saved replies.

Adapters each insurer connects: `src/identity.rs` (eKYC provider, R25) and
`src/assistance.rs` (the assistance provider confirming guarantees of payment, R46).

## Load test

```bash
cargo run --release -p microinsurance-api --example load_test
```

binds new customers through a partner's embedded journey, then drives the non-AI routes with
`CONCURRENCY` clients for `SECONDS` and prints p50, p95 and p99 per route against
`docs/performance.md`, against the API at `BASE_URL` (demo sessions, or tokens signed with
`API_TOKEN_SECRET` outside demo mode). Results: `docs/performance.md`.
