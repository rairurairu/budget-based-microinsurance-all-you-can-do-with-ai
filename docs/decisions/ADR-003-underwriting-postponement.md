# ADR-003: Rule-based underwriting postponement by class of business

- Status: Accepted
- Date: 2026-10-08
- Decider: Tsang Cheuk Nam

## Context

ADR-002 says people decide adverse outcomes. R41 lets the engine stop a high-risk customer from
allocating premium to one class of business for a set period. This is not new: every policy
period is a renewal, so the design maps existing underwriting practice onto a monthly,
multi-cover policy.

### Industry practice compared

| Practice | 中文 | Where it is used | What it does | In this design |
|---|---|---|---|---|
| Renewal re-underwriting, experience rating | 續保重新核保、經驗費率 | motor, SME, group | Terms reset at each renewal from exposure and claims experience | Every policy period, peer-relative and credibility-weighted (R31, R39) |
| Loading (rated, substandard) | 加費承保 | life, health, accident | Higher premium for a substandard risk | Lower sum insured at the same plan price (R31, R39) |
| No-claim discount, bonus–malus | 無索償折扣 | motor | Discount grows without claims, falls after one | Same effect through the rate, never the plan price (R01, R39) |
| Notice of non-renewal | 不續保通知 | general insurance | Cover runs to the end of the notice, then is not renewed | **Not used for the policy:** a monthly subscription renews until the customer cancels. What remains is advance notice of a change in terms: one class is excluded, `exclusion_notice_periods` (R41) |
| Postponement | 延期承保 | life, health | Not accepted for a set period, then reassessed | `postponement_periods` (R41) |
| Specific exclusion endorsement | 除外批註 | health, property | One named risk excluded; the rest of the policy continues | Postponement applies per class; a deeper sub-class gives a narrower exclusion (R37) |
| Increased excess | 提高自負額 | motor, property, device | Customer bears more of each loss | **Option:** excess by risk class |
| Sub-limit | 分項限額 | property, travel | Cap on one peril or item | **Rejected:** the customer may put the whole plan price into one cover, up to its product limit (R34) |
| Moratorium underwriting | 暫緩期核保 | health | Excluded until claim- or symptom-free for N months | **Rejected:** a monthly flexible plan reassesses every period; a claim in one class never restricts another unless a data-supported link exists (R39) |
| Reinstatement with a new waiting period | 復效及重新等候期 | life, health, device | Lapsed cover restarts with a waiting period | **Adopted:** waiting period on re-entry after postponement (R47) |
| Referral to an underwriter | 轉介核保員 | all lines | Cases beyond authority limits go to a person | Reframed as human care: customers who need a person get one (R46) |
| Emergency assistance | 緊急援助 | travel, accident | 24-hour help: emergency contacts, assistance provider, guarantee of payment | Automatic first response within seconds, then the choice of a person (R46) |
| Vulnerable-customer handling | 弱勢客戶處理 | all lines | A person handles distress and vulnerability | R46 |
| Underwriting moratorium (binding restriction) | 暫停承保 | property, catastrophe | Market-wide stop on new business when a catastrophe is imminent | R06 (Signal 1 or above) |
| High-risk pool | 高風險共保池 | motor, some markets | Risks no insurer accepts go to a shared pool | Not applicable |

## Options

A. A person confirms every postponement.
B. The engine applies the insurer's filed underwriting thresholds; a person hears any review
   request.

## Decision

B. Postponement is an underwriting decision from filed, deterministic rules (R39, R41), not an
AI decision. ADR-002 still holds for claims: only a person declines a claim (R13).

## Consequences

- The same inputs always give the same class and the same postponement (R30).
- The AI never sets a class or a postponement; it may only explain one (R23).
- Every postponement carries a one-sentence reason and can be reviewed by a person (R15).
- The plan price never changes; the customer reallocates to classes where their risk is not
  high, or is offered a lower plan or cancellation (R42).
- Increased excess is not adopted; it needs a rule before it is built.
- 2026-10-08 clarification (product owner): the policy is never non-renewed; only cancellation
  ends it. Postponement is a change in terms at renewal (an exclusion of one class), given with
  advance notice. Each period's terms come from its own re-underwriting, so the exclusion applies
  only if that period still rates the class postponed; no separate withdrawal decision exists.
