# microinsurance-engine

The deterministic core. Every money decision lives here.

- No I/O, no clock reads, no randomness, no AI. Time, seeds and data are arguments (R30).
- Money is `Money(i64)` cents; ratios are `Bps` (10,000 = 100%). Floats are denied by clippy (R29).
- `BTreeMap`/`BTreeSet` only, for deterministic iteration.
- Illegal states are unrepresentable: `claims::StpOutcome` has no `Declined` (R13).

| Module | Mechanism | Rules | Status |
|---|---|---|---|
| `money`, `types`, `error` | shared | R29 | done (rounding: ADR-004) |
| `config` | shared | R19, R24, R33, R37, R43, R52, R54, R55 | done: `from_yaml` parses and validates `config/` text, including the registries (ADR-005) |
| `plans` | M1 | R01, R05, R44, R45 | done |
| `optimiser` (`rating`, `underwriting`) | M2 | R02, R31, R32, R33, R34, R39, R41, R42, R43 | done; R49 recording with the ledger |
| `catalogue` | M3 | R06, R07, R09, R24, R47 | done |
| `consent` | M10 | R03, R04, R05, R20, R34, R35, R48 (notice), R52, R53, R54 | done |
| `score` | M6, M7 | R10, R11, R12, R36, R50, R53 | done |
| `prevention` | M6 | R10, R18 (reference pattern), R36, R38, R40 | done (ADR-009) |
| `triggers` | M5 | R08, R55 | done (ADR-006) |
| `payouts` | M5, M7 | R16, R27, R28, R51, R53 | done; `settle_amount` and `settlement_options` shared with claim payments |
| `claims` | M4, M10 | R07, R08, R13, R14, R46, R47, R48, R53, R57 | done (ADR-010): notify, evidence, `decide` (approve or refer only), `person_decision` (the only decline), review of a decline, settle |
| `rewards` | M7 | R10 (void), R16, R17, R26, R40 | done (ADR-009; `void` ADR-010) |
| `guardrails` | M10 | R14, R15, R19, R46 | done (ADR-010): `audit` (sample, findings, overturn rate), `anomaly` (fraud indicators), `appeals` (review requests, four eyes), `fairness` (adverse impact ratio), `human_care` (triage, first response), `queue` (the ops console's work items and actions) |

A policy period, in order: `optimiser::rating::rate_covers` (R31–R33, R39, R43) →
`optimiser::underwriting::review` per class (R39, R41, R47) → `optimiser::renew` (unchanged
allocation, R34) or `optimiser::suggest` (R02) → `consent::renewal_notice` (R35; asks the
customer to update when premium buys nothing usable, R42) → `consent::schedule_for_period` (the
customer's update, or the optimiser's default) → `score::apply` (R11). Rating inputs about the customer (`RatingInputs`) are kept apart from
`Profile`, which reaches the AI layer (R19).

Rule tests: `tests/rules.rs` (synthetic config in `tests/common`), `tests/optimiser_props.rs`
(proptest), `tests/config_files.rs` (the real `config/`). Benchmarks: `benches/engine.rs`
(targets in `docs/performance.md`; `optimise_one_customer` ≈ 20 µs against a 50 µs target).
