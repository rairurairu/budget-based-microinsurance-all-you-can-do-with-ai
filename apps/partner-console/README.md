# Partner console

What a distribution partner's staff see (ADR-011), on the partner's token.

| View | Shows |
|---|---|
| Results | Aggregates over the customers who came through the partner, and everyone who took one of its rewards: subscribers, cover in force, prevention tasks completed, automatic payouts, rewards taken. A figure from fewer than `channels.partner_insights_min_cohort` customers is withheld, never estimated (R21) |
| Spare capacity | The partner's reward offers, with price and stock, published to customers' menus after cash (R16, R56); only a supplier registered for rewards may publish |
| Embed | The token exchange and the element to drop into the partner's app |

The reward calculator (profit per coupon, break-even visits) and segment insights come with the
data loop in Phase 7.

```bash
pnpm --filter @microinsurance/partner-console build   # dist/, served by the API at /partner
pnpm --filter @microinsurance/partner-console test
```
