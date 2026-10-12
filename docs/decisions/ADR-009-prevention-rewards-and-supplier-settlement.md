# ADR-009: Prevention task lifecycle, rewards and supplier settlement

- Status: Accepted
- Date: 2026-10-09
- Decider: engineering, building Phase 4 on the criteria below; for Tsang Cheuk Nam's review

## Context

Phase 4 (docs/08-roadmap.md) takes a prevention task from the AI's proposal to points, a
reward the customer chooses and a supplier paid for what it delivered. R10, R14, R16–R18, R26,
R28, R36, R38, R40 and R50 say what must hold. They leave open where the claims data behind a
task comes from, when a task can be repeated, whether an audited activity is credited before or
after the audit, how a reward is spent, and how and when a supplier that delivers settlement in
kind or a partner reward is paid.

Criteria, in order: the customer is rewarded only for prevention that is real and verified, and
never loses a benefit they are owed; rewards cost less than the claims they avoid (R18); every
amount comes from data the insurer holds, never from the AI (R02); the mechanisms read like
existing loss-control, loyalty and supplier-panel practice, so the insurer, its auditors and the
Insurance Authority recognise them; every value is config the insurer can file.

## Decision

| Question | Decision | Industry basis |
|---|---|---|
| What a prevention task can be | A registered **loss prevention measure** (`config/prevention_measures.yaml`): an id, one class of business with a loss-based cover (R38), the evidence kinds that prove it and a repeat window. The AI writes the event-specific action for one registered measure; a task type the insurer has not registered is never offered | Recognised loss-control (risk improvement) measures: insurers credit approved locks, alarms or telematics only where their claims data shows the effect. An unregistered measure has no claims data, so R18 could not be tested |
| Where the numbers come from | A **prevention basis** per policy period, beside the rating basis: for each measure, the class's claim probability per policy period without and with it, and the class's average claim cost, from the insurer's claims data (`python/economics`). Market data, never config and never AI (R02, R18, R36) | Loss-control credits are filed from the insurer's own claims experience of the class |
| This customer's claims avoided (R40) | Claim probability × the customer's rated relativity in the class (R39), at most 100%; severity is the class's average claim cost, at most the customer's sum insured (first loss, R48). Reward = `reward_share_of_claims_avoided` × that, in cents, rounded down; offered only if below the claims avoided (R18) and the customer holds the cover | Individual risk rating: the same measure is worth more where the rated exposure is higher |
| Repeat window | A measure earns once per `repeat_after_days` per customer, counted from the last verified activity; a task is offered only outside that window | Loyalty-scheme earning rules ("once per qualifying activity"), against farming |
| Task lifecycle | Offered → accepted (customer's tap) → evidence submitted → verified, not verified or with a person. A pass credits points and a reward; a fail can be sent again before the due date or appealed to a person (R15); suspected reuse or edits always go to a person, who alone decides fraud. A task not submitted by its due date (the event's start, or the policy period's end) expires | Claims-style triage: straight-through where clear, referral otherwise; only a person makes an adverse finding (R13) |
| Audit timing (R14) | One in 20 of the verifier's passes, chosen by `audit::should_audit` on the task id with a secret seed, is held for a person **before** points and the reward are credited; a decision whose anomaly score is at least `risk_review_threshold` always goes to a person. Payouts are audited **after** payment: a parametric payout rests on the index, and holding it would break R51's prompt guaranteed share | Pre-payment quality sampling of straight-through claims; post-payment audit of parametric settlements |
| The audit seed | Passed in by the caller, kept secret and recorded with each sample so it can be replayed. A public seed would let a customer compute which decisions are checked | Audit sampling practice: selection must be unpredictable to the audited and reproducible by the auditor |
| Monthly cap (R17) | The rewards granted to a customer in one calendar month of market local time, prevention and loyalty together, stay within `monthly_cap_per_customer`; a grant over the room left is reduced, and the customer is told | Loyalty-programme earning caps |
| Loyalty (R26) | A loyalty reward of `loyalty.reward` each time a points pool has verified activity in `loyalty.streak_periods` consecutive policy periods. What a reward can be earned for is a closed type with no claim-free variant, so no reward can be made conditional on not claiming | Participation (engagement) rewards; claims-free bonuses are excluded by R26 |
| How a reward is spent (R16) | Each customer has a **reward balance** in money. The menu is always cash for the whole balance first, then partner offers in the families the customer's plan opens (R45) with stock left and a price within the balance. An offer's price is what the insurer pays the supplier when it is delivered; the supplier's spare capacity funds the value above it | Points-to-reward redemption catalogues with a cash-out option |
| Who can deliver in kind | Every in-kind option a cover's terms name is registered with exactly one **supplier** (`config/suppliers.yaml`) with its tariff (fixed, or invoiced up to the amount), how long the customer has to use it and, for a fixed tariff, its retail value. An option is offered in place of a cash payout only if its retail value is at least the cash it replaces (R28) | Supplier panels (repair networks, assistance providers) under agreed tariffs; fair value of non-cash settlement |
| Paying suppliers | A supplier is paid only for what it delivered: the tariff, or its invoice up to the amount reserved, on a monthly statement per supplier (`supplier/<id>/<month>`, idempotent). The difference between a payout and a lower tariff goes back to the insurer's risk account | Network settlement against delivery; repairer invoices up to the authorised amount |
| Not used in time | Anything issued in kind and not delivered by its expiry goes back to the customer: a payout as cash, a reward to the reward balance. The supplier is paid nothing for it | A benefit the customer is owed is never lost to breakage; R16 keeps cash as the default |

## Consequences

- `config/prevention_measures.yaml` and `config/suppliers.yaml` are new registries, validated at
  load: a measure names a leaf class with at least one indemnity cover (R38) and registered
  evidence; every cover's `settlement_in_kind` option has one supplier (R28).
- R10, R14, R16, R17, R18, R26, R28 and R40 are amended to say the above; R56 is new (supplier
  settlement).
- `crates/ai/src/prevention.rs` gives the model the registered measures for the cover's class and
  rejects any other task type; the golden set and saved replies follow.
- The ledger gains `reward_expense` (prevention and loyalty rewards are a loss-prevention expense,
  not claims). Reward balances sit in `rewards_payable`; anything awaiting delivery in
  `in_kind_payable` until it is delivered, returned or paid to the supplier.
- Open (Phase 5): the person's audit queue and decisions in the ops console; post-payment audit
  findings and their correction; voiding rewards for fraud a person has confirmed.
