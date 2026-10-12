# Components

Mechanisms M1–M10 come from the ideation map. Loop stages: U Understand, P Protect, Pv Prevent,
V Verify, Rw Reward, Rc Recover, L Learn.

| # | Component | Path | Lang | Mechanism | Stage |
|---|---|---|---|---|---|
| 1 | Plan plans | `crates/engine/src/plans.rs` | Rust | M1 | P |
| 2 | Cover optimiser | `crates/engine/src/optimiser.rs` | Rust | M2 | P |
| 3 | Cover catalogue | `crates/engine/src/catalogue.rs`, `config/catalogue/` | Rust, YAML | M3 | P |
| 4 | Consent and policy period rules | `crates/engine/src/consent.rs` | Rust | M10 | P |
| 5 | Trigger matching | `crates/engine/src/triggers.rs` | Rust | M5 | Rc |
| 6 | Trigger adapters + fan-out | `crates/triggers` | Rust | M5 | Rc |
| 7 | Payout decisions and settlement in kind | `crates/engine/src/payouts.rs` | Rust | M5, M7 | Rc |
| 8 | Claims workflow | `crates/engine/src/claims.rs` | Rust | M4, M10 | Rc |
| 9 | Prevention task lifecycle | `crates/engine/src/prevention.rs`, `config/prevention_measures.yaml` | Rust, YAML | M6 | Pv, V |
| 10 | Protection Score | `crates/engine/src/score.rs` | Rust | M6, M7 | Rw |
| 11 | Reward engine | `crates/engine/src/rewards.rs` | Rust | M7 | Rw |
| 11b | Supplier settlement (in-kind orders, monthly statements) | `crates/engine/src/suppliers.rs`, `config/suppliers.yaml` | Rust, YAML | M5, M7 | Rc, Rw |
| 12 | Guardrails | `crates/engine/src/guardrails/` | Rust | M10 | all |
| 13 | Money ledger | `crates/ledger` | Rust | — | Rc, Rw |
| 13b | Policy store (system of record for the channels and the workers) | `crates/store` | Rust | M1, M8, M10 | all |
| 14 | Learning loop: partner insights, experience studies, pilot metrics | `crates/learning` | Rust | M9 | L |
| 15 | API (route table, tokens per principal, payout stream, demo mode) | `crates/api` | Rust | all | all |
| 16 | Scheduled workers (the book in the policy store: `book.rs`) | `crates/workers` | Rust | M5, M9, M10 | Rc, L |
| 17 | Python bindings | `crates/python-bindings` | Rust | — | — |
| 18 | Contracts export | `crates/contracts` → `contracts/` | Rust | — | — |
| 18b | Config loader | `crates/config` | Rust | — | — |
| 19 | AI harness | `crates/ai/src/harness.rs` | Rust | M4, M10 | all |
| 20 | Understand | `crates/ai/src/understand.rs` | Rust | M4 | U |
| 21 | Foresee (upcoming events) | `crates/ai/src/foresee.rs` | Rust | M4 | U |
| 22 | Explain (one plain sentence per cover and per renewal change) | `crates/ai/src/explain.rs` | Rust | M4 | P |
| 23 | Prevention task writer | `crates/ai/src/prevention.rs` | Rust | M6 | Pv |
| 24 | Verifier | `crates/ai/src/verify.rs` | Rust | M4 | V |
| 25 | Claim assessor | `crates/ai/src/assess.rs` | Rust | M4 | Rc |
| 26 | Reward ranker | `crates/ai/src/rewards.rs` | Rust | M7 | Rw |
| 27 | Economic models (incl. experience indications, pilot evaluation, partner value) | `python/economics` | Python (offline) | M1, M5, M7, M9 | — |
| 27b | AI evaluations (golden sets; runner `crates/ai/src/bin/ai_eval.rs`) | `python/evals` | Python (offline) | M4 | — |
| 28 | Customer app | `apps/customer-app` | TS | M1–M7 | all |
| 29 | Embed SDK | `apps/embed-sdk` | TS | M8 | U, P |
| 30 | Partner console | `apps/partner-console` | TS | M7, M8, M9 | Rw, L |
| 31 | Ops console | `apps/ops-console` | TS | M10 | V, Rc |
| 32 | Identity (eKYC adapter) | `crates/api/src/identity.rs` | Rust | M10 | U |
| 33 | Assistance provider adapter (guarantee of payment) | `crates/api/src/assistance.rs` | Rust | M10 | Rc |
| 34 | Generated API client | `apps/api-client` | TS | all | all |
| 35 | Market config (a market's overlay on `config/`, ADR-012) | `crates/config`, `config/markets/<market>/` | Rust, YAML | all | all |
| 36 | CAP adapter (Common Alerting Protocol 1.2, any market's public warnings) | `crates/triggers/src/adapters/cap.rs` | Rust | M5 | Rc |
| 37 | Job queue and batch jobs (renewals, period openings, erasure, accumulation, experience studies) | `crates/workers/src/queue.rs`, `crates/workers/src/jobs.rs` | Rust | M1, M9, M10 | P, L |
| 38 | Experience studies and pilot metrics | `crates/learning/src/experience.rs`, `crates/learning/src/pilot.rs`, `config/pilot.yaml` | Rust, YAML | M9 | L |
| 39 | Pilot control group (R59) | `crates/engine/src/pilot.rs` | Rust | M6, M9 | Pv, L |
| 40 | Deployment (one AWS deployment per market) | `infra/terraform`, `infra/Dockerfile` | Terraform | — | — |
| 41 | API load test | `crates/api/examples/load_test.rs` | Rust | — | — |
