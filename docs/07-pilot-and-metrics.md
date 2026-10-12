# Pilot and metrics

## Design

One partner app, automatic covers first (weather, transit, event ticket), so no claims team is
needed. One typhoon season. Control group for claim frequency.

## Metrics

| Area | Metric |
|---|---|
| Customer | Conversion among previous insurance non-buyers; subscription and renewal rate; retention; perceived value / satisfaction |
| Prevention | Prevention task completion; completion by reward type; claims avoided vs control |
| Claims | Frequency; severity; payout speed; automated-case rate vs human-review rate; audit disagreement rate |
| Fraud | Frequency; severity; cash payout cost vs settlement in kind cost |
| Rewards | Redemption rate; partner reward engagement |
| Partner | Incremental revenue / conversion; partner retention and expansion |
| Insurer | Loss ratio; insurer value equation (`docs/03-economic-models.md` §6) |
| Fairness | Approval-rate parity across audit groups |

## Gates

Each phase starts only when the gate before it passes. Criteria for scaling up or stopping are
set before the pilot starts and evaluated by `microinsurance_economics.pilot_evaluation`.

## How it runs (ADR-012)

- **Gates first.** `config/pilot.yaml` holds the pilot's dates, the day its gates were fixed and
  each gate: a metric, a bound in the metric's own terms (a rate as a fraction, hours as hours)
  and its kind. The engine refuses gates fixed on or after the start (R59).
- **Control group.** A wait-list control: a share of the customers bound during the pilot, drawn
  from the customer id and the insurer's recorded seed, gets no prevention tasks for their first
  policy periods, then joins; the key features say so (R59). Claim frequency on loss-based covers
  is compared between the groups: the claims avoided.
- **Measure.** `GET /ops/pilot` (people at the insurer; also in the ops console) reports the book's
  metrics for the customers bound in the pilot, each with the sample it rests on. Metrics from
  outside the book (satisfaction from the survey, the partner's incremental revenue from
  `partner_value`, the insurer value equation) are added when evaluating.
- **Decide.** `python -m microinsurance_economics.pilot_evaluation report.json external.json`:
  `stop` when any stop gate is breached; `scale` when every scale gate passes, measured; `hold`
  otherwise. A metric not measured never passes a gate.

| Metric (from the book) | Unit | Rests on |
|---|---|---|
| `conversion_rate` | quoted customers who bound | quoted customers |
| `renewal_rate`, `retention_rate` | customers past their first period who renewed; bound and not cancelled | customers |
| `prevention_completion_rate` | tasks verified ÷ offered | customers offered tasks |
| `claim_frequency_treatment`, `claim_frequency_control`, `claims_avoided_vs_control` | claims per policy period on loss-based covers; the control's minus the treatment's, relative | each group |
| `straight_through_rate`, `claim_payment_hours_p95` | claims approved without a person; notification to approval | claims decided |
| `audit_disagreement_rate` | the random sample overturned (R14) | findings |
| `fraud_rate` | declines for fraud ÷ decisions logged | decisions |
| `reward_redemption_rate` | rewards taken ÷ granted | rewards granted |
| `loss_ratio` | payouts and claim payments ÷ premium billed | customers |
| `fairness_min_parity` | the lowest adverse impact ratio among credible groups (R19) | decisions |
