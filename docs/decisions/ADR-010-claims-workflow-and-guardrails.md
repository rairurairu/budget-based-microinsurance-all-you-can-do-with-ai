# ADR-010: Claims workflow, review requests, fairness audit and human care

- Status: Accepted
- Date: 2026-10-09
- Decider: engineering, building Phase 5 on the criteria below (delegated design decisions, as for
  ADR-006 and ADR-009); for Tsang Cheuk Nam's review

## Context

Phase 5 (docs/08-roadmap.md) builds the claims workflow for indemnity covers, the claim
assessor, review requests, the fairness audit and the ops console. R13, R14, R15, R19, R46 and
R48 say what must hold; ADR-002 fixes the decision rights (AI proposes, code decides money,
people decide adverse outcomes); ADR-009 left open the anomaly score behind risk review, the
people's audit queue and voiding rewards for confirmed fraud. Still open were: when a claim may
be approved without a person and for how much, what the AI's estimate is for, how a decline is
reviewed, how fairness is measured, and how an urgent case reaches help before a person.

Criteria, in order: a customer is never declined by a machine and is paid quickly when the
evidence is clear; nothing the AI writes sets an amount; every adverse step names a person and
can be reviewed by another; the mechanisms read like existing claims-handling, audit and
fair-treatment practice, so the insurer, its auditors and the Insurance Authority recognise them;
every threshold is config the insurer can file.

## Decision

| Question | Decision | Industry basis |
|---|---|---|
| The claim's path | First notification of loss (`ClaimNotice`) → evidence the cover's terms ask for → the assessor → `claims::decide`: approved straight through, or with a person. Every notification is registered, even on a cover not held: only a person declines (R13). Parametric covers take no claim; their payouts are reviewed instead (R08) | FNOL, then triage into straight-through processing or a handler's queue |
| Straight-through processing (STP) authority (R57) | Approved without a person only when, in order: cover in force for the loss and past its waiting periods (R07, R47, R53); no human-care need (R46); the anomaly score below `risk_review_threshold` (R14); the assessor recommends approve with confidence at least `claims.stp_min_confidence`; the amount claimed within `claims.estimate_tolerance` of its estimate; the claim payment at most `claims.stp_limit`; and not drawn in the 1-in-20 sample. Anything else goes to a person with the reason, due within `claims.referral_sla_hours` | Claims authority limits: automated settlement only for small, clean claims; referral reasons recorded for the handler |
| What the assessor is | The AI reads the photos and documents and returns approve or refer, an estimate of the loss, its confidence and whether the evidence fits the description. Its reply type has no decline; a reply that tries one fails the schema and the claim goes to a person (R13, R22). It never sees the amount claimed, the sum insured or the premium, so it cannot anchor on them | AI damage estimation used as a triage and leakage check, with handlers deciding exceptions |
| What is paid | Straight through: the amount claimed, up to the sum insured (first loss, R48). The estimate only decides whether a person looks; it never becomes the amount paid (R02). A person approves the loss they assessed, again up to the sum insured. Cash first; settlement in kind the cover permits, at fair value (R16, R28); net of premium due in the grace period (R53); one payment per claim (R27) | Indemnity on a first-loss basis; the claimed, documented loss is the measure, not a model's guess |
| Audit before payment (R14) | A claim drawn in the random sample, or at the risk review threshold, waits for a person before payment, as a prevention pass does before credit (ADR-009). Parametric payouts stay audited after payment | Pre-payment quality sampling of straight-through claims |
| The anomaly score | Rules-based fraud indicators, each adding its configured weight, at most 1: early claim after cover began, late notification, frequent claims in the class, amount near the sum insured, evidence seen before (by fingerprint). Every indicator present is named for the person. No model and no protected attribute | Red-flag indicator scoring in claims fraud practice; explainable to the handler and the auditor |
| Declines | Only `claims::person_decision`, by a named person, with a reason from a closed list tied to the wording (not in force, waiting period, not an insured loss, policy exclusion, evidence not provided, fraud) | Declinature letters cite the policy term; adverse findings are a handler's |
| Review requests (R15) | Any decision by the AI or the rules can be put to a person; a person's adverse decision, such as a decline, is reviewed by a different person (four eyes). Each review is due within `appeal_sla_hours`; a declined claim under review returns to a person's queue with the name of the person who declined it, who cannot decide it. A customer's own choices are not reviewed: they change them. The complaints route to the regulator's scheme is outside the system | Internal dispute resolution before an external complaints scheme (in Hong Kong, the Insurance Complaints Bureau) |
| Fairness audit (R19) | Per decision kind and audit attribute (`guardrails.fairness.groups`), each group's approval rate; a group whose rate is below `min_parity_ratio` of the best group's is flagged, counting only groups with `min_group_decisions`. Group attributes come from a separate audit store and never reach the AI. A flag is a finding for people to investigate, never an automatic change | Adverse impact ratio (the four-fifths rule), as used in algorithmic fairness testing of insurance practices |
| Audit findings | A person agrees with or overturns each audited decision; the overturn rate of the random sample, per decision kind, estimates the automated path's error and shows drift. Risk-reviewed decisions are left out of the rate because they were chosen for looking odd | Quality-assurance sampling with an unbiased error estimate |
| Urgent cases (R46) | When a case is notified the engine computes its triage at once: urgent for a configured circumstance (injury, hospitalisation, emergency abroad), a loss at least `large_loss_share_of_monthly_income` of declared earnings, or the AI's escalation. An urgent case gets the configured first response by `urgent_response_seconds` (emergency numbers, the assistance provider, a guarantee of payment), then a person is offered and the customer chooses. A guarantee of payment is automatic up to `auto_guarantee_cap` and only with active cover in the class; the assistance provider confirms the rest. A customer who asks for a person, or whom the AI sees in distress, gets one. The AI's flags only add: rules ∨ AI | 24-hour emergency assistance and guarantee-of-payment practice in travel and accident insurance; fair treatment of vulnerable customers |
| Voiding rewards | A reward is voided only on a person's finding of fraud: what is left of it in the reward balance is taken back and the loss-prevention expense reversed by that much; what was already spent is recorded as unrecovered. Points go with the activity | Clawback of loyalty credit on proven fraud, never on suspicion |
| The ops console | One queue for people: claim referrals (the only place a decline can be made), prevention tasks with a person, decisions drawn for audit after payment and review requests. The engine sets the order (urgent first, then due time) and the actions each item allows; the console shows them and adds none. Until the API (Phase 6) it renders a snapshot the workers write | A single work queue with service levels, as in claims-handling systems |

## Consequences

- R13, R14, R15, R19, R46 and R48 are amended to say the above; R57 is new (STP authority).
- The insurer profile gains `claims` (STP authority, referral service level, anomaly indicators)
  and `guardrails.fairness`; `guardrails.human_care` is now parsed and validated
  (`person_on_request` must stay true).
- `crates/ai` gains the assessor (`assess`), its prompt, saved answers in every language pack and
  a golden set; the engine gains `claims`, `guardrails::{anomaly, appeals, fairness, human_care,
  queue}` and `rewards::void`; the ledger posts claim payments like payouts and reverses voided
  rewards.
- Open: real (consented) claim photos from the pilot to score the assessor live; the insurer's
  claims data to calibrate the anomaly weights and the STP thresholds; recovery of payouts an
  audit finds paid in error is a person's decision outside the system (parametric payouts paid in
  good faith are not recovered); the guarantee-of-payment exchange with the assistance provider
  and the API routes for the ops console (Phase 6).
