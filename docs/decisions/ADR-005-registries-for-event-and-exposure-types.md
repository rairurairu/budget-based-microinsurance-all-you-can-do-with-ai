# ADR-005: Registries for trigger event types, exposures and data types

- Status: Accepted
- Date: 2026-10-08
- Decider: Tsang Cheuk Nam

## Context

The Phase 1 engine carried the first prototype's minimum: catalogue triggers named one data source
and its raw codes (`hko_warnings`, `TC8NE`), only the HKO feed had a config file, and closed Rust
enums listed what a customer can declare (`Exposure`, `UpcomingEventKind`), the optional data
types (`DataType`), evidence (`EvidenceKind`) and reward families (`RewardFamily`). Each new kind
of cover, a second market or a new feed meant a code change. The product's point is many covers,
including kinds traditional insurance does not write, so the catalogue must grow by config.

Industry practice for parametric products defines a trigger by covered peril, index source,
parameter, threshold and geography, independently of any one feed; reinsurance event definitions
do the same. A data source is then one way to observe an event type.

## Options

A. Keep closed enums; add variants as covers are added.
B. Free strings everywhere.
C. **Registries in config, validated at load**, with closed Rust types kept only where the
   engine runs different logic for each variant.

## Decision

C. The test for "enum or registry": does the engine compute something different for it?

| Concept | Where | Why |
|---|---|---|
| Trigger event types: stages, measure, zone unit, pricing basis | `config/event_types.yaml` (registry) | Data: a new peril is a new entry |
| Data sources and their code mapping | `config/triggers/*.yaml` (registry) | Swap per market (ADOPTION.md) |
| Upcoming event kinds, declared item kinds | `config/exposure_types.yaml` (registry, dotted tree) | A concert, a match, a pet, a bicycle are data |
| Exposure form: declared hours, route, item, upcoming event, everyone | `ExposureKind` (enum) | Each computes the customer's need differently |
| Trigger measure: in force, duration, delay | `Measure` (enum) | Each is matched differently |
| Payout basis: fixed per day or event, declared value, tiered by stage, per hour | `PayoutBasis` (enum) | Each sizes a payout differently |
| Optional data types and evidence kinds | `config/data_types.yaml` (registry) | R20 consent applies to any type the same way |
| Reward families | `rewards.families` in the insurer profile | Labels for offers and plan privileges |
| Cover type (parametric, indemnity), claim outcomes, risk classes | enums / config as before | Legal and logical distinctions (R13, R39) |

Rule R55 makes the registry binding: a cover may pay only on registered event types, and is offered
only when its event types have an available source and a stated pricing basis.

## Consequences

- A new peril, feed, market, upcoming-event kind, declared item or data type is a config change
  that `just check` validates; no recompilation.
- Catalogue files name event types and stages, not source codes; the HKO codes live only in
  `config/triggers/hko_warnings.yaml`.
- Profiles and AI recommendations carry registered ids; the engine refuses an unregistered one.
- A cover whose feed has no history (for example a ticketing platform) needs the insurer's filed
  rate as its pricing basis before it can be offered.
