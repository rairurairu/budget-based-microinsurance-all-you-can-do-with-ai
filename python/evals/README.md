# AI evaluations (offline)

Golden sets per AI component in `golden/<component>/*.yaml`: input, expected properties (not
exact text), and the rules each case protects. `just eval` runs every case through the production
code (the `ai-eval` binary in `crates/ai`: harness, fixed checks, fallbacks) against the saved
replies in `fixtures/ai_replies`; nothing leaves the machine. `EVAL_LIVE=1 just eval` runs them
against the configured provider (`ANTHROPIC_API_KEY`) and skips harness cases, whose saved replies
are deliberately bad to prove the fallback.

After changing a case's input, refresh the saved replies' keys with `just eval-rekey`; with
`EVAL_LIVE=1`, `just eval-record` saves the live model's replies instead.

| Component | Checks |
|---|---|
| understand_form | at most five questions; none about a protected attribute (R19) |
| understand_facts | registered device kinds and routes only (R55); protected answers never sent (R19) |
| foresee | events found with the right kind and date; never a value (R02) |
| explain, explain_change | one sentence (R23); no amount sent or stated (R02); market factors only (R19) |
| prevention_tasks | consented evidence (R20); none for parametric covers (R38); no reward named |
| verify | reused files flagged without a model call; edited never passes; no verdict → a person (R10, R15) |
| assess | approve or refer only, a decline reply falls back to a person (R13, R22); inconsistent evidence is never an approval (R57); reused evidence caught before the model (R14); urgent and distress escalated (R46); the model never sees the amount claimed (R02) |
| rewards | every offer kept, cash included (R16) |
| all | protected fields never sent (R19); every component has a case that must fall back (R22) |
