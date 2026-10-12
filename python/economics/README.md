# Economic models (offline)

Equations are fixed by the method; inputs come from the adopting insurer. Formulas and default
values: `docs/03-economic-models.md`. Outputs are written to `config/`.

Where a model needs a production rule (for example the loss prevention cost-benefit test), it calls the engine
through `microinsurance_py` (PyO3), so the model and the live system can't disagree. `just test`
builds the bindings first; `microinsurance_py.Engine(...).suggest(...)` runs the production rating
and optimiser for backtests (see `crates/python-bindings/README.md`).

| Module | Answers |
|---|---|
| `trigger_pricing` | What does HK$1 of parametric benefit cost per policy period, from the historical trigger record? |
| `prevention_test` | Is this prevention task type worth rewarding? (R18) |
| `reward_economics` | Does this partner coupon make the partner money? Break-even new-visit share? |
| `unit_economics` | Per-subscriber monthly P&L for a plan |
| `insurer_value` | Incremental value vs cost of running the method |
| `partner_value` | Value to a distribution or reward partner |
| `pilot_evaluation` | Do the pilot metrics pass the gates fixed before it? Scale, hold or stop (`python -m microinsurance_economics.pilot_evaluation report.json`, ADR-012) |
| `experience` | What does the book's experience indicate for the next rating basis and prevention basis, weighted by its credibility? (`GET /ops/experience`; a proposal the actuary files, R31) |
