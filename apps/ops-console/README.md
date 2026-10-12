# Insurer operations console

What people at the insurer work on (ADR-010). The engine decides the order and the actions each
item allows (`engine::guardrails::queue`); the console shows them and adds none, so a decline
button exists only on a claim referral (R13).

| View | Shows |
|---|---|
| Work queue | One queue, urgent first (R46), then by due time: claims referred to a person, with the reason (the only place a decline can be made); prevention tasks with a person; decisions drawn for audit after payment (agree / overturn, R14); review requests (uphold / overturn, R15) |
| Fairness audit | Approval rate by audit group per decision kind; groups below `min_parity_ratio` of the best group flagged (R19) |
| Audit overturn rate | How often people overturn the automated path in the random sample, per decision kind (R14) |

Built (Phase 5): a static page rendered from an ops snapshot (`OpsSnapshot`, generated types in
`apps/api-client`). `demo/snapshot.json` is what demo mode writes
(`DEMO_MODE=1 OPS_SNAPSHOT=apps/ops-console/demo/snapshot.json cargo run -p microinsurance-workers`).

Built (Phase 6, ADR-011): the live console. `dist/index.html`, served by the API at `/ops`, reads the
queue from `GET /ops/queue` and posts each button as the signed-in person's decision
(`POST /ops/decisions`): the person is the token's, never a field the page sends (R15), and the API
refuses any action the item does not offer (R13). A decline carries a reason from the closed list,
an approval the loss assessed (R48). `dist/snapshot.html` is the snapshot page, which opens offline.

```bash
pnpm --filter @microinsurance/ops-console build
pnpm --filter @microinsurance/ops-console test
```

runs the checks in `src/console.test.ts` (Node's test runner on the TypeScript sources).
