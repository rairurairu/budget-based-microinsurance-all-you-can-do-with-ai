# Economic models

The platform provides the equations; the adopting insurer provides the real inputs. Defaults in
`config/` are illustrative and are replaced by the insurer's portfolio data at adoption.

## 1. Plan economics (per subscriber per month)

| Line | Default (HK$) | Basis |
|---|---|---|
| Premium | 80 | Essential plan |
| Expected claims | 44 | 55% loss ratio before prevention (target band 50–60%) |
| Prevention rewards | 5 | |
| Partner commission | 12 | 15% of premium |
| AI, cloud and operations | 4 | mostly automated claims and onboarding |
| Insurer margin + platform fee | 15 | shared under the partnership agreement |

Module: `microinsurance_economics.unit_economics`.

## 2. Policy period rating (R31–R33)

The plan price is fixed; the rate decides how much cover each premium HK$ buys. It is
re-rated every policy period, for each customer and cover, from that customer's exposure.

```
expected_claims(cover, customer, c) = frequency × severity, adjusted by the registered factors
horizon_cost(c)            = mean(expected_claims per HK$ of sum insured, periods c … c+N−1)
rate(cover, customer, c)   = horizon_cost(c) / target_loss_ratio                       continuously covered
                           = max(own cost in c, horizon_cost(c)) / target_loss_ratio   new or re-entering
sum_insured                = premium_allocation / rate              N = rating_horizon_periods
```

- Parametric: `frequency = P(trigger in force during declared hours in a policy period)`, from the
  historical record (HKO warnings database: typhoon signals since 1946, rainstorm warnings since
  1998), season and official outlooks. Payout sized below typical daily earnings (R09); one
  event's payouts bounded by its event limit (R51). Module: `microinsurance_economics.trigger_pricing`.
- Indemnity: frequency and severity from the insurer's portfolio data, with individual
  relativities.
- Factors come only from `config/rating_factors.yaml` (R33). A factor the customer withholds is
  rated at the undeclared group's average, credibility-blended with the portfolio average, so
  withholding never pays when the people who withhold are riskier. Age, gender and district only where the industry uses
  them (R19). Novel factors (data the industry does not collect) are rated in shadow until
  validated and approved (R43).
- Averaging over the next N policy periods shows known future risk early: if January knows that
  February–April carry more risk, February's sums insured already reflect it. Customers who
  join or re-enter within `continuous_cover_periods` pay the higher of the period's own cost and
  the average, so joining for a peak alone never pays; continuously covered customers keep the
  plain average, which is what an annual premium would charge them (R32).

### Risk classes and postponement (R39–R42)

Per class of business, `relativity = customer expected claims / portfolio expected claims` places
the customer in one of the insurer's standard classes (`underwriting.risk_classes`), re-underwritten
every policy period. The class moves sums insured and benefit amounts through the rate. Rewards
follow the claims a task avoids for this customer (R40), so they rise with risk, not fall.

Claims experience enters as evidence of exposure, not of fault:

```
experience(class) = own claim frequency / peer claim frequency, same rated exposure, same periods
relativity       ×= 1 + Z × (experience − 1)     only if experience > claims_experience_threshold
```

A typhoon pays every exposed customer, so it moves own and peer frequency together and cancels
out. A customer whose flight-delay payouts run well above peers travels more than rated, and that
counts. `Z` is the credibility weight. A claim in one class moves another only through a
configured `experience_links` entry backed by data. Above
the last threshold the class of business is postponed for this customer. The policy itself always
renews (a monthly subscription ends only on cancellation): the class is excluded from the schedule
after `exclusion_notice_periods` of notice, if that period's re-underwriting still supports it, and
for `postponement_periods` they choose among classes of business where their risk is not high. The
plan price never changes.

## 3. Allocation and the cover optimiser

The customer allocates the plan price across enabled covers each policy period, in fixed steps (default
HK$5). An unchanged allocation renews automatically and is re-rated (R34); the new sums insured are
shown `renewal_notice_days` before the policy period with a one-sentence reason (R35).

The optimiser proposes an allocation that maximises protection value per HK$ for this
customer's profile and upcoming events, subject to minimums, maximums, eligibility and waiting
periods. Deterministic, unit-tested. Module: `engine::optimiser`.

## 4. The loss prevention cost-benefit test

```
reward_cost < claim_probability_reduction × average_claim_cost
```

Example: verified case + screen protector cuts screen claims from 4% to 2.5% a month on a
HK$800 average repair → saves HK$12 per user per month against a HK$3 reward. Module:
`microinsurance_economics.prevention_test`.

Per customer, the reward is `reward_share_of_claims_avoided × claims_avoided(customer)` (R40):
the same verified case on a phone that breaks often is worth more than on one that rarely does.

```
p_before(customer) = min(1, p_without × relativity)       relativity: the customer's rating (R39)
p_after(customer)  = min(p_before, p_with × relativity)
severity(customer) = min(average claim cost, sum insured)  first loss (R48)
reward             = floor(share × (p_before − p_after) × severity)   offered only if < claims avoided
```

The probabilities and claim cost come from the prevention task's own class of business (R36), for
one of the insurer's registered measures (`config/prevention_measures.yaml`), in the prevention
basis built from its claims data. Only indemnity covers get prevention tasks: behaviour cannot
change a parametric trigger (R38). Module: `microinsurance_economics.prevention_test`
(`reward_for_customer` runs the engine).

## 5. Reward economics (partner side)

```
profit_per_coupon = redeemed × (new_visit_share × margin × spend − face_value)
break_even_new_visit_share = face_value / (margin × spend)
```

Why rewards make money: perceived value ≠ actual cost; spare capacity has near-zero marginal
cost; basket uplift; co-funding from marketing budgets; a gamed prevention task still sends a
paying customer. Module: `microinsurance_economics.reward_economics`.

The insurer pays the supplier an offer's price (its tariff) when it is delivered; the partner's
spare capacity funds the value above it. Rewards are spent from a reward balance where cash is
always the first option (R16), and anything not used in time goes back to the balance, so the
insurer earns no breakage on rewards or settlement in kind (R56). Settlement in kind replaces a
payout only at fair value (R28): at a fixed tariff the insurer keeps `payout − tariff`, and the
customer gets something worth at least the cash.

## 6. Insurer value equation

```
value = claims_avoided + retention_value + new_customer_value
      − reward_cost − verification_cost − platform_cost
```

Scale only when incremental value > incremental cost in the pilot. Module:
`microinsurance_economics.insurer_value`.

## 7. Partner value

```
value = incremental_engagement + conversion + retention + use_of_low_marginal_cost_capacity
```

## 8. Protection Score economics

Two layers. The rate (§2) moves both ways with risk, re-underwritten every policy period with
claims experience (R39). The score sits on top and only rewards: a premium rebate credited after
the policy period, or a bonus sum insured (customer chooses), never worse than the period's
rated base, capped at 20%. The plan price itself changes only by the customer's choice or a
promotional discount (R44).

Points are kept per points pool (`config/lines_of_business.yaml`, R36): a verified device
prevention task improves property terms only, never accident or travel terms. Points fade after about three
months, so the score reflects current behaviour. Module: `engine::score`.

## 9. Experience studies and rate indications (ADR-012)

Each month the book is studied against the rating basis that priced it (`learning::experience`,
`GET /ops/experience`):

```
A/E(class)           = Σ incurred / Σ expected            over the study's policy periods
Z(class)             = min(1, sqrt(claims / 1,082))        limited-fluctuation credibility
indicated factor     = Z × A/E + (1 − Z) × 1
p_with, p_without    = Z × observed frequency + (1 − Z) × prevention basis     per measure
```

A typhoon season with a few dozen claims moves the indication little; a credible run of
experience moves it to the experience. `microinsurance_economics.experience` applies the factors
to the next rating basis and blends the measures' probabilities into the next prevention basis:
a proposal the actuary reviews and files (R18, R31). The system never changes a rate on its own.
