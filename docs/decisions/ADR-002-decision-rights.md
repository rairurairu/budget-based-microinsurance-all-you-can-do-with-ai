# ADR-002: Decision rights between AI, rules and people

- Status: Accepted
- Decider: Tsang Cheuk Nam

## Decision

AI proposes and explains; deterministic code decides money; people decide adverse outcomes.
Enforced structurally:

- The engine is a Rust crate with no dependency on the AI service or any I/O.
- AI output reaches the engine only as a typed recommendation deserialised in `crates/api`.
- `claims::StpOutcome` cannot express a decline; only `claims::person_decision` can.

## Consequences

Prices and payouts are reproducible from config + state. AI failures degrade to saved answers,
never to wrong money movements.
