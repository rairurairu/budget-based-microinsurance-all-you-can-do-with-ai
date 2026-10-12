# Agent instructions

Read this first. It applies to every coding agent working in this repository.

## What this is

An insurer-adoptable microinsurance engine. The method and its rules are in `docs/`; the rules
in `docs/rules.md` are not negotiable. Parameters live in `config/`.

## Commands (run from the repo root)

| Command | Use |
|---|---|
| `just check` | Before every commit. Must pass with zero warnings |
| `just test` | All tests; `cargo test -p microinsurance-engine` for the engine only |
| `just gen` | After changing any type that crosses a boundary |
| `just bench` | After touching `optimiser` or trigger matching |
| `just eval` | After changing any prompt or AI schema |
| `just dev` | Local stack, demo mode (offline fixtures, saved AI replies) |
| `just load` | After touching an API route or the store: p99 against `docs/performance.md` |
| `just demo-market tw` | Another market's demo (config/markets/<market>, ADR-012) |

## Boundaries (do not cross)

1. **Money is decided only in `crates/engine`.** It has no I/O, no clock reads, no randomness
   and no AI. Time, seeds and data are passed in. Same input, same output.
2. **Money is integer minor units** (`Money(i64)` cents, ratios as `Bps`). Floating-point
   arithmetic is denied by clippy in the engine (R29).
3. **Use `BTreeMap`/`BTreeSet` in the engine**, never `HashMap`, so iteration order is
   deterministic.
4. **The AI returns recommendations.** `crates/ai` deserialises model output into a typed recommendation;
   anything that fails to parse, times out or errors uses the saved fallback (R22).
   `crates/engine` never depends on `crates/ai`.
5. **Prompts are files** in `crates/ai/prompts/*.md`, loaded at runtime, so they change without
   recompiling. Every prompt change runs `just eval`.
6. **Make illegal states unrepresentable.** Example: `claims::StpOutcome` has no
   `Declined` variant (R13). Prefer a type over a runtime check.
7. **Contracts are generated.** Never hand-edit `contracts/` or `apps/api-client/src/generated`.
   Change the Rust type (or a route in `crates/api/src/routes.rs`, the only list of routes), then
   `just gen`.
8. **Protected attributes stay out of AI.** Age, gender and district may be rating factors in
   `crates/engine` only where `config/rating_factors.yaml` registers them (R19); ethnicity,
   religion and nationality never. None of them ever reaches `crates/ai` or any model input.

## Adding or changing behaviour

1. Find the rule in `docs/rules.md` (add one if the behaviour is new, with its source).
2. Write the test first, named `rNN_<what>` (Rust) or `test_rNN_<what>` (Python).
3. Implement in the module listed in the rule's "Enforced in" column.
4. Update `docs/traceability.md` if a mechanism's location changed.
5. `just check && just test`.

Reference pattern: `engine::prevention::admit` and its tests in `crates/engine/tests/rules.rs`.

## Definition of done

- Rule tests pass; no `#[ignore]` left on the rules you touched.
- `just check` clean; contracts regenerated and committed.
- Hot paths benchmarked against `docs/performance.md` targets.
- Demo mode still runs offline.

## Naming

The project is **Budget-Based Microinsurance**. Never use an earlier working name in code, docs,
commits or UI copy. Names must be clear to people in the
insurance industry at a glance: use the industry terms in `docs/glossary.md`.

## Don't

- Don't add a dependency to `crates/engine` beyond `serde`, `chrono`, `thiserror` without an ADR.
- Don't let `crates/ai` write to the ledger, set an amount, or decline anything.
- Don't call a model provider from any crate other than `crates/ai`.
- Don't use real customer data in fixtures.
