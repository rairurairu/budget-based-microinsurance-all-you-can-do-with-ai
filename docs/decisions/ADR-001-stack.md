# ADR-001: Technology stack

- Status: Accepted (option A: runtime all-Rust)
- Decider: Tsang Cheuk Nam

## Context

Two requirements: the code must be easy for coding agents to build and change safely, and the
money paths must be fast and stable enough for a typhoon-day burst across a whole insurer's book.

## Decision

| Layer | Choice | Why |
|---|---|---|
| Deterministic engine | **Rust** (edition 2024), pure library | Hot paths (trigger fan-out, optimiser); rules become compile-time guarantees; precise compiler feedback for agents |
| AI layer | **Rust** (`crates/ai`): reqwest + serde against the model API; prompts as runtime-loaded files | One runtime and one type system; model output is parsed straight into typed recommendations; no extra service to deploy |
| API, workers, triggers, ledger | **Rust**: axum, tokio, sqlx (compile-time checked SQL), tracing | Every runtime process in one workspace and one binary |
| Database and queue | **PostgreSQL** only (jobs via `SKIP LOCKED`) | Fewer moving parts |
| Economic models | **Python** offline (polars, numpy) + **PyO3** bindings to the engine | Actuaries read Python; backtests run the production engine |
| AI evaluations | **Python** offline (`python/evals`) against the running API | Fast iteration on golden sets without touching runtime code |
| Front ends | **TypeScript**: React 19, Vite, TanStack Query; embed SDK as a Web Component. *Amended by ADR-011, for confirmation: framework-free TypeScript, screens as pure functions, compiled by tsc* | Works inside any partner app or webview |
| Contracts | Rust types → OpenAPI (paths from the API's route table, ADR-011) + JSON Schema (schemars) → TS client (openapi-typescript) | One source of truth; CI fails on drift |
| Tooling | `just`, `rust-toolchain.toml`, `uv`, `pnpm`, clippy `-D warnings`, rustfmt, ruff, pyright, Biome, cargo-nextest, proptest, insta, criterion | One command per task; strict linters catch agent errors early |
| Delivery | Docker, GitHub Actions, AWS (ECS + RDS) via Terraform | Standard for insurers |

## Options compared (8 Oct 2026)

| | A runtime all-Rust (chosen) | B Rust + Python AI service | C everything Rust |
|---|---|---|---|
| Runtime stability | one binary, one type system | extra service hop and cross-language schema | as A; WASM front end harder to debug |
| Agent-friendliness | one server language | two runtimes to reason about | little training data for Leptos |
| Prompt iteration | prompts as files + `just eval` | fastest | as A |
| Adopter fit | one container + Postgres; actuaries use Python via PyO3 | two containers | actuaries can't read the models |
| Partner embed | Web Component | Web Component | larger WASM bundle |

## Decide

- [x] YAML crate for config parsing: `serde-saphyr` (ADR-004)
- [x] Rounding rule for `Money::mul_bps`: half-up, away from zero (ADR-004)
- [x] Hosting for front ends: AWS, served by the API behind CloudFront (ADR-012, for confirmation)
