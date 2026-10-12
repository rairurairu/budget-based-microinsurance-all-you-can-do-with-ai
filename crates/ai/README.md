# microinsurance-ai

The harnessed AI layer (M4), and the only crate that talks to a model provider. It returns typed
recommendations; it never sets an amount, writes to the ledger or declines anything (ADR-002).

| Module | Prompt (component) | Tier | Returns | Saved fallback |
|---|---|---|---|---|
| `understand` | `understand_form`, `understand_facts` | fast | `ProposalForm`; `ProfileFacts` in registered ids | the pack's five-question form; nothing mapped |
| `foresee` | `foresee` | fast | `Vec<UpcomingEvent>`, never a value (R02) | no events |
| `explain` | `explain`, `explain_change` | fast | one plain sentence per cover (R23); one per renewal change (R35) | the pack's sentence for the cover; the pack's change sentence |
| `prevention` | `prevention_tasks` | strong | `PreventionTaskProposal` for one of the cover class's registered measures the customer can prove, or none; parametric covers, and customers with no consented evidence for any measure, get none by rule (R18, R20, R38) | no task |
| `verify` | `verify` | strong | `VerifyOutcome`: a `Verdict`, or refer to a person; reused files caught by fingerprint first | refer to a person |
| `rewards` | `rewards` | fast | offer ids, best first: a permutation of the offers (R16) | cash first, then by perceived value |
| `assess` | `assess` | strong | `AssessOutcome`: an `engine::claims::Assessment` (approve or refer, never decline; an estimate that checks the amount claimed and never sets the amount paid; escalation to urgent or a person, R46), or refer to a person; reused files caught by fingerprint first. Never sees the amount claimed, the sum insured or the premium (ADR-010) | refer to a person |

`Harness::ask` is the only way a component reaches a model: it strips protected attributes from
the input (R19), loads the prompt, asks for the reply's JSON Schema (structured outputs), parses
it strictly, runs the component's fixed checks (`text`: one sentence, no money, nothing protected;
registered ids) and otherwise returns the saved answer, with the reason (R22). Every result says
whether it came from the model, the fallback or a fixed rule. See ADR-007.

Models: `client::AnthropicClient` (Messages API over HTTPS, retries on 429/5xx), or
`replay::ReplayModel`, which answers from `fixtures/ai_replies` for demo mode and evaluations.
Settings are the `ai` section of the insurer profile. Customer text is in the customer's
language setting; the language packs (`config/languages/`, `language::Languages`) hold each
language's check terms and saved answers and are checked when loaded.

Prompts live in `prompts/*.md` and are read at runtime, so they change without recompiling.
After any prompt change run `just eval` (offline), and `EVAL_LIVE=1 just eval` against the
provider. The reply schemas are generated into `contracts/schemas/ai/` (`just gen`).
