# Risk controls

## Adverse selection

| Risk | Control | Rule |
|---|---|---|
| Buying weather cover when a typhoon is forecast | No new weather cover once HKO Signal 1 or above is in force | R06 |
| Buying cover for a loss already happening | Cover incepts at 00:00 the day after the tap; a quote is valid only on the day it was made | R58 |
| Subscribe for one month, then cancel | Minimum 1-month subscription per cover | R05 |
| Travel cover bought the day before a flight | Minimum 1-month subscription | R05 |
| Pre-damaged device, then instant claim | 14-day waiting period on device cover | R07 |
| Shifting budget into a cover mid-event | Changes take effect from the next policy period, never mid-event | R04 |
| Joining just before a known high-risk season | Each policy period is rated on the next three policy periods' average risk | R32 |
| Joining or re-entering for the peak month alone | New and re-entering customers pay max(this period, horizon average) | R32 |
| Loyal customers paying for others' timing | Continuously covered customers keep the plain horizon average | R32 |
| Withholding data to avoid a higher rate | Rated at the undeclared group's own average | R33 |
| Underinsurance after a rate rise | First-loss settlement; renewal notice flags a sum insured below the item's value | R48 |
| Fraud through automatic guarantees of payment | Automatic guarantee capped per case; above it the assistance provider confirms | R46 |
| Behaviour in one line earning better terms in another | Points and rewards stay within their points pool | R36 |

## Underwriting (high-risk customers)

| Risk | Control | Rule |
|---|---|---|
| A customer far riskier than the portfolio in one class of business | Re-underwritten each policy period into a risk class, including credibility-weighted claims experience; sum insured, benefit amount and rewards move with the class | R31, R39, R40 |
| One unlucky claim swinging the class | Peer-relative experience above a threshold, credibility-weighted; claims under review count only once decided | R39 |
| A market-wide event (typhoon) raising everyone's class | Experience is compared with peers in the same periods, so common shocks cancel out | R39 |
| A claim in one class restricting an unrelated class | Cross-class effect only through a data-supported `experience_links` entry | R39 |
| Above the postponement threshold | The policy renews; advance notice that the class will be excluded, then postponement for N policy periods if that period's re-underwriting still supports it; other classes stay open | R41 |
| Re-entering a postponed class just before a known loss | Waiting period on re-entry | R47 |
| Usable cover runs out before the plan price | The customer is asked to update in the app or web, shown covers with room and what makes them useful; without an update, the optimiser's default applies | R42 |
| Postponements leave the plan price beyond the covers' limits | Plan change or cancellation without penalty available; no plan is recommended | R42, R45 |

## Accumulation (parametric)

| Risk | Control | Rule |
|---|---|---|
| One typhoon triggers every weather policy at once | Event limit per trigger from the reinsurance programme; guaranteed share paid at once, balance pro rata after the event closes | R51 |
| Benefit in force piling up in one zone | New allocation into the zone stops above the accumulation limit | R51 |
| Buying just before the event | No new weather cover once Signal 1 or above is in force | R06 |

## Payment and lapse

| Risk | Control | Rule |
|---|---|---|
| A failed card payment ends cover without warning | Grace period with cover in force; claims paid net of premium due | R53 |
| Lapse then rejoin just before a loss | Reinstatement only within the window; later, waiting periods apply again | R53, R07, R47 |

## Basis risk (parametric)

| Risk | Control | Rule |
|---|---|---|
| Signal raised but the customer was not working | Payout only during declared working hours | R08 |
| Income lost with no warning issued | Not covered; wording says it pays on the signal, not on proven loss | R08 |
| Payout larger than the loss (arbitrage) | Payout sized below typical daily earnings | R09 |

## Fraud

Fraud cannot be zero, so the platform manages **frequency** and **severity**.

| Threat | Control |
|---|---|
| Stock, reused or edited prevention photos | AI image checks for reuse/edits; live capture; dedup hashes |
| Faked diagnostics screenshots | Diagnostics read directly from the device; screenshots rejected |
| Staged damage | Named fraud indicators (early claim, late notification, frequent claims, amount near the sum insured, reused evidence) sum to an anomaly score; at the threshold a person reviews before payment (R14) |
| Inflated claim amount | The amount claimed is checked against the assessor's estimate; beyond the tolerance, or above the STP limit, a person decides (R57) |
| The same photo for two claims | Fingerprint check before any model call; always to risk review (R14) |
| Farming rewards | Points only for verified prevention activities (R10); each measure once per repeat window (R10); 1 in 20 passes checked by a person before credit (R14); monthly reward cap (R17) |
| A prevention task the insurer cannot price | Only registered measures with claims data; reward below the claims avoided (R18, R40) |
| Settlement in kind worth less than the cash | Offered only at fair value: retail value at least the cash, tariff not above it (R28) |
| Paying a supplier for nothing | Suppliers paid only for deliveries, on a monthly statement; unused orders go back to the customer (R56) |
| Residual fraud severity | Low-marginal-cost partner settlement in kind and non-cash alternatives |

Rewards voided for confirmed fraud; denial decided by a person.

## Channels (ADR-011)

| Risk | Control | Rule |
|---|---|---|
| Two taps, or two devices, binding twice | A quote is accepted once; the store refuses a write on a version already changed | R03, R58 |
| A partner binding without a licence | The licence is checked on the day of the tap; a referral-only partner only shows the way to the insurer | R52 |
| A person deciding in someone else's name, or reviewing their own decline | The person is the token's staff id, never a request field; a different person reviews a decline | R15 |
| A decline slipped onto an item that cannot be declined | The API takes only the actions the engine offers the item | R13 |
| A partner re-identifying customers from its results | Aggregates only; any figure over too few customers is withheld | R21 |
| A customer token leaking through logs or referrers | Tokens travel in the Authorization header, never a URL; the app reads its token from the page fragment; customer tokens are short-lived | — |
