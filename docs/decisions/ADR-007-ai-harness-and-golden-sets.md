# ADR-007: The AI harness, model contract and golden sets

- Status: Accepted
- Date: 2026-10-09
- Decider: Tsang Cheuk Nam (Phase 3 go-ahead and the language decision on 2026-10-09; built by
  engineering within ADR-002 and R02, R19, R22, R23)

## Context

Phase 3 (docs/08-roadmap.md) builds the AI layer: understand, foresee, explain, the prevention
task writer, the verifier and the reward ranker, each with a golden set. ADR-002 already fixes
the decision rights: AI proposes and explains, deterministic code decides money, people decide
adverse outcomes. What was open is how a model reply becomes something the system may use, what
happens when it should not be used, which data a model may see, and how prompts are tested
without a live model.

Criteria, in order: nothing a model writes can move money, name an amount, decline anything or
expose a protected attribute; a customer always gets a safe answer within the latency budget;
every rule the AI touches is tested offline in CI; prompts change without a release.

## Decision

| Question | Decision | Industry basis |
|---|---|---|
| Reply contract | Each component has a typed reply (`contracts/schemas/ai/<component>.json`, generated). The request asks the provider for that schema (structured outputs); the reply is still parsed strictly (unknown fields refused) and then passes the component's fixed checks before use | Model risk management practice (as in SR 11-7): outputs are validated before they enter a business process |
| Fixed checks | Customer-facing text: one plain sentence in the customer's language (its terminators and length, R23), no currency term or any amount in the input (R02), no protected attribute term (R19), the terms of every offered language checked together. Ids: only registered kinds, routes and evidence kinds (R55). Rankings: a permutation of the offers given (R16). Question ids never name a protected attribute | Fair-treatment and plain-language disclosure standards (key facts statements); controls are deterministic, so the same reply is always accepted or refused |
| Fallback (R22) | Any timeout (`ai.timeout_ms`, `ai.strong_timeout_ms`), provider error, refusal, schema or check failure uses the component's saved answer: the fixed five-question form, no facts mapped, no events read, the cover's key features (or a fixed sentence from its name and events when they are not one sentence), a fixed renewal sentence, no prevention task, verification referred to a person, rewards cash first then by value. Every result says whether it came from the model, the fallback or a fixed rule, and why | Fail-safe defaults: a failed automated step degrades to the conservative manual path, never to a different money outcome |
| Language | The customer's own setting (`Profile::language`), one of the insurer's `languages.offered`, or `languages.default` until they choose. Each language is a pack in `config/languages/<tag>.yaml`: its sentence shape, the currency and protected terms the checks look for, and every saved answer (the form, a sentence per cover, cover names and change sentences, verification messages). Packs are checked at start-up: every offered language has one, every cover is covered, every saved answer passes the checks; otherwise the process does not start. A reply not in the customer's language fails the sentence check and falls back. Nothing about a language is in code | Fair treatment of customers: disclosures in the language the customer chose; copy the insurer approves lives in config |
| What the model sees | Built by the component, then stripped of protected fields at any depth (R19). Never a premium, a sum insured, a benefit amount, a customer id or an individual rating factor; renewal reasons name only market factors (the same for every customer). Reward ranking sees what the customer sees, not what an offer costs its funder | Data minimisation (PDPO DPP1, DPP3): only the data needed for the purpose |
| The AI never sets an amount | `foresee` returns events without a value (the customer declares a ticket price); the prevention writer returns `PreventionTaskProposal` (action, evidence, a registered measure), and the engine sizes and admits it with the measure's claims data (R18, R40; ADR-009) | R02, ADR-002 |
| Verification | A reused file is caught by its SHA-256 fingerprint before any model call. A reply suspecting reuse or editing never passes. No valid verdict means `ReferToPerson`, never a fail (R15); a pass is a recommendation, audited 1 in 20 (R14) | Claims-fraud practice: duplicate-document checks first, then human review of exceptions |
| Prevention scope | Only loss-based covers get prevention tasks (R38), decided by a rule without a model call; evidence only of kinds the customer opted into (R20) | R38, R20 |
| Models | Fast tier (`claude-haiku-5-5`) for understand, foresee, explain and rewards; strong tier (`claude-sonnet-5-5`) for prevention tasks and verification (images). Effort `low`; the strong model uses the provider's server-side refusal fallback (`refusal_fallback: default`). All in `ai` in the insurer profile. Raw HTTPS to the Messages API: there is no official Rust SDK | Choose the least capable model that passes the golden set; values the insurer can change |
| Saved replies | `fixtures/ai_replies/<component>/*.json`, keyed by component and a hash of the exact input and images (not the prompt, so saved replies survive prompt edits). Demo mode and CI use them; nothing leaves the machine | Reproducible evidence for audit; demo mode stays offline (AGENTS.md) |
| Golden sets | `python/evals/golden/<component>/*.yaml`: input, the rules each case protects, and expected properties, not exact text. Offline, every case runs through the production code (`ai-eval`) against saved replies, including deliberately bad replies that must fall back. `EVAL_LIVE=1` runs the same cases against the provider and skips the harness cases; `--record` saves the live replies | Model validation on a held set before release, with the controls tested separately from the model |

## Consequences

- No model call is on the payout path, and no model output reaches the ledger.
- A prompt edit needs no release; `just eval` (offline) still passes, and `EVAL_LIVE=1 just eval`
  shows whether the model still meets the properties.
- A labelled set of real (consented) evidence photos from the pilot is needed before the
  verifier is scored live; the synthetic images test the harness only.
- Adding a language is a config change: offer it in the insurer profile and add its pack.
- The claim assessor (`ai::assess`) follows in Phase 5 with the claims workflow, under this ADR.
