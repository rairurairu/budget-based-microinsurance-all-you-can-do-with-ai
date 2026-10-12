# Customer app

The customer's side of the monthly loop on the API (ADR-011): framework-free TypeScript, each
screen a pure function from the API's data to escaped HTML (`src/views.ts`), a thin runtime
calling the generated client (`src/main.ts`). Every word comes from the customer's language pack
in `config/copy/<tag>.yaml`; the customer's language setting picks it.

| Screen | Loop stage | What it shows |
|---|---|---|
| Your price | Protect | every plan, in configured order, none recommended (R45); a quote: the optimiser's split or the customer's own (R34), one sentence per cover (R23), why this split (R49); one tap binds from tomorrow (R03, R58) |
| Your cover | Protect | the schedule, premium status (R53), changes for next period (R04), next period's sums insured with a sentence each (R35); a plan change is the customer's own choice |
| About you | Understand | working hours, routes, upcoming events, items, earnings; optional rating answers, kept apart (R19, R33) |
| Prevention tasks | Prevent / Verify | tasks for upcoming events, accept, evidence (camera or file), verdict, ask a person (R10, R15); points per pool (R11, R12, R36) |
| Rewards | Reward | the reward balance; cash for all of it first, then partner rewards as ranked (R16) |
| Payouts | Recover | automatic payouts, live as they are decided; cash or an offered alternative (R16, R28) |
| Claims | Recover | notification of loss with any urgent circumstance and first response (R46), evidence, outcome, review by another person (R13, R15); decisions about the customer to put to a person |
| Privacy | — | opt-ins per data type (R20), identity check (R25), cancel (R54) |

The page reads its token from its fragment (`/app/#token=…`, never sent to a server), as the
partner's app or the insurer's sign-in opens it; in demo mode it offers the synthetic customers.

```bash
pnpm --filter @microinsurance/customer-app build   # dist/, served by the API at /app
pnpm --filter @microinsurance/customer-app test    # src/views.test.ts on Node's test runner
```
