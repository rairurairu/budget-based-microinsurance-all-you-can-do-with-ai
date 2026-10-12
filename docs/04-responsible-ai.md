# Responsible AI

A fixed harness around the AI. The AI explains and suggests; deterministic code sets money;
people review a sample and every appeal.

| Safeguard | Rule | Enforced in |
|---|---|---|
| AI never sets prices, rates or sums insured; plan prices are the same for everyone; sums insured come from registered rating factors only | R01, R02, R31, R33 | `engine::plans`, `engine::optimiser` |
| Every AI reply is schema-checked; slow, failed or invalid replies fall back to a safe saved answer | R22 | `ai::harness` |
| AI can approve or refer a claim, never decline: the assessor's reply has no decline, a person declines with a reason, and the AI's estimate never sets the amount paid | R13, R48 | `engine::claims`, `ai::assess` |
| A claim is paid without a person only within the insurer's STP authority: cover in force, assessor confident, amount within tolerance of its estimate, payment within the limit | R57 | `engine::claims` |
| 1-in-20 random human audit of automated decisions, and risk review at the anomaly threshold (named fraud indicators), before credit or payment; the sample's overturn rate tracks drift | R14 | `engine::guardrails::audit`, `engine::guardrails::anomaly` |
| Any AI decision can be appealed to a person; a decline is reviewed by a different person | R15 | `engine::guardrails::appeals`, `engine::claims` |
| Customer changes need a tap; an unchanged allocation renews automatically with advance notice of new sums insured; cash is always offered next to partner rewards | R03, R16, R34, R35 | `engine::consent`, `engine::rewards` |
| Fairness audit: approval rates compared across groups as an adverse impact ratio (a group below four-fifths of the best is flagged for people); protected attributes never reach the AI, and only industry-used ones (age, gender, district) may be engine rating factors | R19, R43 | `engine::guardrails::fairness` |
| Partners receive consented, aggregated insights only; personal data never sold | R21 | `learning::insights` |
| Identity checks via a licensed eKYC provider | R25 | `api::identity` |
| Every recommendation explained in one plain sentence | R23 | `ai::explain` |
| Urgent cases get an automatic first response within seconds, then the choice of a person; customers who need human care get one; the AI can escalate, never de-escalate | R46 | `engine::guardrails::human_care`, `engine::claims`, `ai::assess` |

## Random audit: four layers

1. AI automated verification
2. Random human audit (deters gaming, estimates AI false negatives, monitors drift, produces
   labelled data)
3. Risk-based human review (anomaly score above threshold)
4. Appeal and exception review
