# ADR-004: Engine config parsing, rounding, rates and contract schemas

- Status: Accepted
- Date: 2026-10-08
- Decider: Tsang Cheuk Nam (delegated to engineering on 2026-10-08, decided on the criteria below)

## Context

Phase 1 (docs/08-roadmap.md) needs the engine to load `config/`, round money, express rates and
export contracts. ADR-001 left two points open (YAML crate, rounding rule), and AGENTS.md
requires an ADR for any engine dependency beyond `serde`, `chrono` and `thiserror`.

Criteria, in order: the same input always gives the same money (R30); every amount a customer
sees can be explained and audited by the insurer and the Insurance Authority; no rounding works
against the customer beyond what the product terms disclose; the fewest and safest dependencies.

## Options

| Question | Options | Chosen |
|---|---|---|
| YAML parser in the engine | `serde_yaml` (archived 2024) · `serde_norway` (fork, wraps C-derived `unsafe-libyaml`) · `serde-saphyr` (pure Rust, panic-free, budgeted) · parse outside the engine | **`serde-saphyr`**, `deserialize` feature only. Maintained, pure Rust, MIT/Apache-2.0, no file includes. The engine still does no I/O: callers pass the text in. Used only inside `config`, through `from_str`, so swapping it is a one-file change |
| Decimals in config (`0.55`, `0.20`) | float fields · exact decimal parsing | **Exact**: the shortest round-trip text of the number is parsed into integer `Bps` or `Money`; more decimal places than the unit holds is a config error, never rounded (R29) |
| Rounding | half-up · banker's (half-even) | **Half-up, away from zero**, with the direction fixed per amount below. Banker's rounding suits sums of many rounded figures in accounting; here each customer amount is rounded once and shown on a notice, where commercial rounding is what customers and the regulator expect. The ledger itself never rounds |
| Rate unit | `Bps` · parts per million | **`Rate`, parts per million** of sum insured. A personal accident rate near 0.04% would be 4 bps, a 25% jump per step; in millionths it is 400 |
| Contract schemas | `utoipa` derives now · `schemars` derives behind a feature | **`schemars`** behind the engine's `schema` feature, enabled only by `crates/contracts`. OpenAPI 3.1 uses JSON Schema, so `contracts/openapi.json` carries the same schemas as components; the paths come with the routes in Phase 6 (written from the API's route table, not utoipa: ADR-011) |
| Property tests | none · `proptest` (dev-dependency) | **`proptest`**, dev-dependency only |

## Decision

As chosen above. Default engine builds depend on `serde`, `chrono`, `thiserror` and
`serde-saphyr` (deserialize only). `schemars` is compiled only with `--features schema`.

### Rounding by amount

| Amount | Rule | Why |
|---|---|---|
| Money × ratio: promotional discounts, premium rebates, shares of premium | Half-up to the cent (`Money::mul_bps`) | Commercial rounding, as on a premium notice |
| Rates and relativities | Half-up; a rate is never below 1 ppm | Deterministic; a sum insured is always finite |
| Sums insured and parametric benefit amounts | Down to whole HK$ (`Money::floor_major`) | The schedule shows round amounts and never more cover than the premium buys; the difference is under HK$1 and is stated in the product terms |
| Earnings cap on a parametric benefit (R09) | Down to whole HK$ | Keeps the benefit strictly below earnings |
| Payouts and claim payments (Phase 2) | The contractual sum insured or assessed amount, unrounded | A customer is never paid less than the contract states |
| Pro-rata reduction above an event limit (R51, Phase 2) | Each payout down to the cent; the cents left go one each to the largest remainders, ties by policy id | The total equals the event limit exactly and reconciles with the reinsurance programme |
| Ledger postings | No rounding: integer cents, every posting set balances to zero | Rounding happens once, before posting |

## Consequences

- `EngineConfig` is built from text by `config::from_yaml`; a binary or test reads the files.
  `just check` loads the real `config/` through `crates/engine/tests/config_files.rs`.
- A value such as `target_loss_ratio: 0.55555` fails to load instead of being rounded.
- Contracts regenerate with `just gen`; CI fails on drift (`just gen-check`).
- The product terms must state that sums insured are shown in whole HK$, rounded down.
- Revisit if `serde-saphyr` stops being maintained.
