# microinsurance-py (PyO3)

The production engine, callable from Python, so the economic models and backtests run the same
code as the live system. Money is in cents, ratios in basis points, rates in parts per million;
structured inputs and outputs are JSON in the shapes of `contracts/`.

| Python | Engine |
|---|---|
| `prevention_test_passes`, `claims_avoided_cents` | `prevention::admit`, `prevention::claims_avoided` (R18) |
| `mul_bps` | `Money::mul_bps` (R29, ADR-004 rounding) |
| `horizon_cost_ppm` | `optimiser::rating::horizon_cost` (R32) |
| `Engine(insurer, lines, factors, event_types, trigger_sources, exposure_types, data_types, catalogue)` | `config::from_yaml` |
| `Engine.base_cost_ppm`, `Engine.rate_ppm`, `Engine.risk_class` | rating (R31, R32, R39) |
| `Engine.suggest` | rating + `optimiser::suggest` (R02) |

`just test` builds it into the Python workspace first. By hand:

```
uv run --project python maturin develop --uv -m crates/python-bindings/Cargo.toml
```

It builds against the workspace's Python (`python/.venv`). To run cargo on this crate directly,
do it inside that environment so PyO3 finds the interpreter:

```
uv run --project python cargo clippy -p microinsurance-python-bindings
```
