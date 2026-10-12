# Regulatory model (Hong Kong first)

- **Carrier model.** A licensed insurer underwrites every cover and holds the licence. The platform
  bundles only products the insurer has approved (R24). A policy is bound only through a
  distribution partner with a valid licence on record; an unlicensed partner may only refer the
  customer to the insurer's own journey (R52). The licence is checked on the day of the customer's
  tap, and cover incepts at 00:00 the next day, never for a loss already known (R58).
- **Parametric cover.** Hong Kong has no dedicated parametric legislation; such contracts are
  treated as ordinary insurance. Wording states the cover pays on the trigger, not on proven
  loss.
- **Sandbox.** The pilot can run through the Insurance Authority's Insurtech Sandbox, with its
  wait-list control group stated in the key features (R59).
- **Privacy (PDPO).** Explicit opt-in per data type; data used only for the cover it supports;
  optional data deleted on cancellation; prevention works without location sharing (R20).
  Policy, premium, claims and ledger records are kept for the retention period, access-restricted
  and used only for compliance and underwriting (R54).
- **Fair treatment.** Age, gender and district only where the industry rates on them, never in
  AI inputs; outcomes audited by group (R19). No reward is conditional on not claiming (R26).
- **Advice.** The optimiser's suggested allocation is product information under the insurer's
  licence, with its factors shown and the record kept: every quote is kept as shown, with its
  suggestion, factors, sentences and price; no plan is ever suggested (R45, R49).
- **Rebates.** Premium rebates are insurer-funded, set by rule in the product terms and applied
  identically; no intermediary can grant or share one (R50).
- **Identity.** Licensed eKYC provider (R25).

Per-market settings live in the market's insurer profile (`regulator`, `privacy_regime`,
`kyc_provider`): `config/insurer.example.yaml` for Hong Kong, `config/markets/<market>/insurer.yaml`
for another (ADR-012).

## Open compliance questions (confirm with counsel before the pilot)

| Question | Design position | Rule |
|---|---|---|
| Is the suggested allocation regulated advice? | Product information under the insurer's licence | R49 |
| Are premium rebates a permitted inducement? | Insurer-funded, in the product terms, never through an intermediary | R50 |
| Which licence does each partner need to embed the cover? | Licensed agency or broker to bind; otherwise referral only | R52 |
| How long must policy and claims records be kept? | `record_retention_years`, set with counsel | R54 |
| Is a two-stage, pro-rata parametric payout acceptable in the wording? | Limits stated in the key features | R51 |
| May the partner's wallet collect the premium? | As the appointed agent, accounting to the insurer; confirmed per policy period | R53 |
| Is the quote kept long enough to show what the customer was told? | Kept as shown in the policy store for `record_retention_years` | R49, R54 |

## Taiwan (the second market, ADR-012; confirm with counsel)

- **Regulator and product line.** The Financial Supervisory Commission (Insurance Bureau); the
  covers would be filed by the carrier, small sums insured fitting microinsurance (微型保險)
  practice. Distribution through an insurance agent or broker company; referral otherwise (R52).
- **Trigger.** Weather income cover pays on the local government's work suspension for a natural
  disaster where the customer declared they work, not on a forecast or a model (R55).
- **Privacy (PDPA).** As in Hong Kong: opt-in per data type, erasure of optional data on
  cancellation with a record, records kept for the retention period (R20, R54). Data stays in the
  market's AWS region.

| Question | Design position | Rule |
|---|---|---|
| May a parametric cover pay on a work-suspension announcement? | Yes, as the official trigger stated in the key features | R55 |
| Is a wait-list control group in the pilot fair treatment? | Disclosed, time-limited, drawn at random by the insurer's seed | R59 |
