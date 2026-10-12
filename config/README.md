# Configuration: the adoption surface

Everything an insurer or partner changes lives here. Engines read config; they never hard-code
market values. Values marked `# Decide` are open design decisions; values marked
`# illustrative` are replaced with the insurer's own data at adoption.

| File | What it sets |
|---|---|
| `insurer.example.yaml` | Market, currency, local time (`utc_offset_minutes`), regulator, policy period, plans, loss-ratio target, score, rewards, `guardrails` (audit sample and risk review, review requests, human care and emergencies, the fairness audit), `claims` (the STP authority for indemnity claims and the fraud indicators behind the anomaly score, R57, ADR-010), privacy, event limits per trigger event type and zone accumulation (R51), `languages` (offered and default), `ai`: models, timeouts and effort for the AI layer (ADR-007), the distribution partners and their licences (R52), and `channels`: the insurer's own journey, customer token lifetime, the partner insights cohort (R21) and partners' web origins (ADR-011) . `accumulation.refresh_minutes`: how often benefit in force per zone is measured for the accumulation limit (R51); `experience.full_credibility_claims` for experience studies (ADR-012) |
| `catalogue/*.yaml` | One file per cover: type, class of business, launch phase, `exposure` (what a customer needs for the cover to protect them), `product_limit` (R34), trigger or claim basis, waiting period, eligibility |
| `lines_of_business.yaml` | Lines and classes of business; the depth at which points and rewards pool (R36, R37) |
| `rating_factors.yaml` | The only market and individual factors the rating formula may use, each with its formula and data source (R33) |
| `event_types.yaml` | Trigger event types: stages, measure, zone unit, pricing basis (R55) |
| `triggers/*.yaml` | One file per data source: status, history, the `adapter` that reads its format, and the mapping of its codes to event types and stages (R55). A CAP source (`adapter: cap`) says which event code and geocode it reads (`cap`) and maps the feed's geocodes to the work areas a customer declares (`areas`, ADR-012). `mtr_line_status.yaml` also holds `lines`, the operator's line codes and the route ids a customer declares; `transit_disruptions.yaml` holds `normalise`, the phrases that turn TD's free text into road closure codes (ADR-008) |
| `copy/<tag>.yaml` | The words the customer app and the embed show, one file per offered language with the same keys and `{placeholders}`, checked when the API starts; illustrative wording the insurer replaces (ADR-011) |
| `languages/<tag>.yaml` | One file per language a customer can choose (`languages.offered` in the insurer profile): sentence shape, the terms the AI checks look for, and the saved answers shown when the AI is not used (R22, ADR-007) |
| `exposure_types.yaml` | What a customer can declare: upcoming event kinds and declared item kinds, as dotted trees (R55) |
| `data_types.yaml` | Optional data types a customer opts into (R20) and evidence kinds (R55) |
| `prevention_measures.yaml` | The loss prevention measures the insurer recognises, each for one class with a loss-based cover, with the evidence that proves it and a repeat window (R10, R18, R38, ADR-009) |
| `suppliers.yaml` | The supplier panel: who delivers each in-kind option a cover names, at what tariff (fixed with a retail value, or invoiced), for how long, and which suppliers offer partner rewards (R28, R56) |

`just check` loads every `insurer.*.yaml` with the catalogue, the class tree and the rating
factors through the engine's validation (`crates/engine/tests/config_files.rs`). Decimals are
exact: a ratio such as `0.55` loads as 5,500 basis points, and a value with more decimal places
than the unit holds is refused (ADR-004).

Market data that changes every policy period (expected claim cost per cover and period, factor
relativity tables) is not config: it is the rating basis (`optimiser::rating::RatingBasis`),
produced by `python/economics` and the insurer's filed rates. Likewise each measure's claim
probability without and with it and its class's average claim cost are the prevention basis
(`prevention::PreventionBasis`), from the insurer's claims data; partner reward offers, with their
price and stock, come from the partners at run time.

## Markets (ADR-012)

`config/` is the base market (Hong Kong). Another market is an overlay, `config/markets/<market>/`,
selected with `MARKET=<market>`: a file there replaces the base file of the same name;
`catalogue/`, `languages/` and `copy/` merge by file name; `triggers/` is the market's own as a
whole, since data sources never cross a border; `event_types.yaml` merges by id, so a market
states its own official stages for a peril. Its insurer profile is `insurer.yaml`. `just check`
loads every market (`crates/config/tests/markets.rs`). `config/markets/global/` is the
international demonstration market `just dev` runs (US dollars, English, weather warnings in CAP
1.2; its synthetic data in `fixtures/markets/global/`); `config/markets/tw/` is Taiwan.

| File | What it sets |
|---|---|
| `pilot.yaml` | The pilot: its dates, the day its gates were fixed (before the start), the wait-list control group (R59) and each gate, a metric with a bound in the metric's own terms and `scale` or `stop` (`microinsurance_economics.pilot_evaluation`) |
| `markets/<market>/` | One market's overlay, as above |
