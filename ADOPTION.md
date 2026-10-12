# Adopting the platform

The platform separates **what is fixed by the method** from **what each business configures**.
An insurer or partner adopts it by filling in `config/`, plugging in its own data sources and
running the economic models on its own portfolio data.

## Roles

| Party | Role | What it provides |
|---|---|---|
| Licensed insurer (carrier) | Underwrites the risk, holds the licence, approves products | Approved product wordings, loss data, target loss ratio, claims team for referrals and appeals |
| Distribution partner | Embeds the platform in its app; supplies rewards from spare capacity | Channel, customer context (with consent), reward inventory |
| Platform operator | Runs the engines, AI layer, triggers and prevention loop | This repository |

## Adoption steps

1. **Market and insurer profile.** For a new market, make `config/markets/<market>/` (ADR-012):
   its `insurer.yaml` (market, currency, local time, regulator, privacy law, languages, plans,
   policy period, target loss ratio, audit rate, waiting periods, accumulation limits, payout
   destinations), and only the files it changes: event types with its official stages, its own
   `triggers/` (always), a cover whose trigger differs, its language packs. Everything else is
   read from `config/`. `MARKET=<market>` selects it; `config/markets/global/` (the international demo) and
   `config/markets/tw/` are worked examples.
   For the base market, copy `config/insurer.example.yaml`.
2. **Catalogue.** Enable the covers your licence and approved products allow
   (`config/catalogue/*.yaml`). Only pre-approved products may be bundled (R24).
3. **Triggers.** Covers pay on registered event types (`config/event_types.yaml`), never on a
   feed's codes (R55). For each feed, add `config/triggers/<source>.yaml` mapping its codes to an
   event type and stage, naming its `adapter`. A feed in the Common Alerting Protocol (CAP 1.2),
   as many national weather and civil-protection services publish, needs only the file
   (`adapter: cap`, which event code and geocode it reads, and its `areas`); another format needs
   an adapter in
   `crates/triggers/src/adapters`. Hong Kong ships with HKO warnings, heat stress at work, air
   quality, transit, flight and ticketing sources; Taiwan with CAP work suspensions, typhoon
   warnings and rain advisories. A market keeps the catalogue's product design and states each
   cover's trigger on its own official signals; where they are announced per area, the cover
   matches the customer's declared work area and the peril counts one event across areas. What
   customers can declare (event kinds, items) and the optional data types are registries too
   (`config/exposure_types.yaml`, `config/data_types.yaml`).
4. **Price.** Run `microinsurance_economics.trigger_pricing` on the historical trigger record and your
   target loss ratio to set each cover's rate per HK$ of sum insured. Run
   `microinsurance_economics.unit_economics` with your real premium, claims, commission and
   operations figures.
5. **Rewards.** Partners list spare capacity in the partner console; run
   `microinsurance_economics.reward_economics` to set face values that are profitable for the partner.
6. **Prevention.** Check every prevention task type against the loss prevention cost-benefit test (R18) with your claim
   frequency and severity data.
7. **Guardrails.** Set the audit sampling rate (default 1 in 20), the fairness-audit groups and
   the appeal SLA.
8. **Channels.** Record each distribution partner and its licence (R52) and its web origins and
   the insurer's own journey under `channels`; replace the illustrative wording in `config/copy/`
   with approved wording in every offered language; connect the eKYC provider, the assistance
   provider, the payment provider and the identity provider for the insurer's own journey
   (`crates/api`, ADR-011). Partners embed `<microinsurance-embed>` after a token exchange.
9. **Pilot.** Fix the gates and the wait-list control in `pilot.yaml` before the start (R59).
   Launch narrow (one partner, automatic covers), read `GET /ops/pilot`, evaluate it with
   `microinsurance_economics.pilot_evaluation` and scale only when it says so
   (`docs/07-pilot-and-metrics.md`).
10. **Run.** Deploy one market per AWS region with `infra/terraform` (README there), run the
   load test against `docs/performance.md` at the expected volumes, and each month give the
   actuary the experience study (`GET /ops/experience`, `microinsurance_economics.experience`):
   rates change only when the actuary files a new rating basis (R31).

## What stays fixed

The rules in `docs/rules.md` are the method. Adopters change parameters, never the decision
rights: AI never prices or declines, nothing changes without the customer's tap, and points are
only earned for verified prevention activities.
