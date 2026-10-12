# Roadmap (build order)

Each phase ends with tests green and the components it lists working end to end.

## Phase 1: deterministic core (Rust)
Tooling (`just`, CI) → `engine` config loader → plans → catalogue → optimiser → consent → score
→ `ledger` → contracts export. Economic models (Python): unit economics, loss prevention cost-benefit test, reward
economics; PyO3 bindings.

Status (2026-10-08): built. Engine rules R01–R07, R09–R12, R18, R20, R24, R29, R31–R37, R39,
R41–R45, R47, R48 (renewal notice), R50, R52–R54 pass; the ledger posts idempotently (R27) in its
reference in-memory store. Carried into Phase 2: the PostgreSQL ledger store, with payouts, and
the payout rounding fixed in ADR-004.

## Phase 2: automatic recovery
Trigger engine → HKO adapter (live + historical) → payouts (idempotent) → trigger pricing model
on the historical record → workers (polling, monthly renewal).

Status (2026-10-09): built. Engine rules R08, R16 and R28 (payouts), R27, R51, R53 (payout set-off)
and R55 (tracking, history, zone matching) pass; ADR-006 (Accepted) pays by loss occurrence per
peril, with resumption time and a minimum of disrupted work for daily benefits. The HKO adapter
reads the live warning summary and a warnings-database CSV export; trigger pricing replays the
record through the production matcher; `PollTriggers` and the monthly renewal run in
`crates/workers`, and `DEMO_MODE=1 cargo run -p microinsurance-workers` replays a typhoon
offline. Trigger fan-out: 1,000,000 policies in 1.6 s on one core. With `DATABASE_URL`, the
ledger (8,400 postings/s) and the trigger desk's state are in PostgreSQL, one worker leads each poll,
and a restart mid-event resumes without repaying; CI runs these tests against PostgreSQL.
Open: the real HKO history export (the fixture is synthetic). The transit, flight and organiser
adapters followed in Phase 3 (ADR-008); the policy store in Phase 6, the workers reading their book
from it; the job queue for the other jobs moves to Phase 7.

## Phase 3: AI layer
Harness → understand → explain → foresee → prevention task writer → verifier → reward recommender.
Golden-set evaluations for every AI component.

Status (2026-10-09): built. ADR-007 (Accepted): each component asks for a typed reply
(`contracts/schemas/ai/`), checks it with fixed rules (one plain sentence, no amount, no protected
attribute, registered ids, a ranking that keeps every offer) and otherwise uses its saved answer
(R22); the AI never sets an amount (`foresee` returns no value, the prevention writer returns a
`PreventionTaskProposal`) and never sees a premium, a sum insured or a personal rating factor
(R02, R19). Prevention tasks only for loss-based covers and consented evidence (R20, R38); reused
evidence is caught by fingerprint before any model call, and no verdict means a person looks (R10,
R15). Prompts are full; `just eval` runs 27 golden cases through the production code against saved
replies, and `EVAL_LIVE=1` against the provider. Customer text follows the customer's language
setting; each offered language is a pack in `config/languages/` with its check terms and saved
answers, checked at start-up. The rail (the MTR's own line status), road (TD Special Traffic
News), flight (HKIA departures) and organiser (notice) adapters are built (ADR-008), each source
polled by its own desk, and demo mode replays a rail suspension and a concert cancellation
offline. Open: a labelled set of pilot evidence photos for the verifier's live golden set; the
claim assessor followed in Phase 5.

## Phase 4: prevention and rewards
Prevention task lifecycle → audit sampling → rewards engine → partner settlement in kind.

Status (2026-10-09): built. ADR-009 (Accepted): a prevention task is one of the insurer's
registered loss prevention measures (`config/prevention_measures.yaml`), each for a class with a
loss-based cover; the AI writes the event-specific action, and the engine sizes the customer's
reward from the measure's claims data in the prevention basis and their rated relativity, capped
by first loss (R18, R38, R40). The lifecycle runs offered → accepted → evidence → verified, not
verified or with a person, with expiry, a repeat window per measure, appeals, and suspected reuse
always to a person (R10, R13, R15). 1 in 20 passes is held for a person before credit and every
decision at the risk review threshold goes to one; payouts are sampled after payment, all from a
secret, recorded seed (R14). Rewards: granted within the monthly cap (R17), a loyalty streak for
sustained participation with no claim-free variant (R26), a reward balance spent on cash, always
first, or partner rewards the plan opens (R16, R45). Suppliers (`config/suppliers.yaml`) deliver
settlement in kind and partner rewards at an agreed tariff, offered in place of cash only at fair
value (R28), paid monthly for what they delivered, with anything unused returned to the customer
(R56); the ledger books rewards as a loss-prevention expense. `crates/workers` runs the task from
the AI's saved proposal to the supplier's statement in demo mode. The anomaly score behind risk
review, the ops console's audit queue and voiding rewards for fraud a person confirmed followed in
Phase 5 (ADR-010). Open: real claims data for the prevention basis (the fixture is synthetic).

## Phase 5: claims and guardrails
Claims workflow → assessor → appeals → fairness audit → ops console.

Status (2026-10-09): built. ADR-010 (Accepted): every notification of loss on an indemnity cover
is registered and triaged at once, so an urgent case (injury, hospitalisation, an emergency abroad,
a loss large against declared earnings, or the AI's escalation) gets its first response within
`urgent_response_seconds` and a guarantee of payment up to `auto_guarantee_cap`, then a person is
offered (R46). The assessor (`ai::assess`) reads the evidence and recommends approve or refer with
an estimate; its reply has no decline, it never sees the amount claimed or the sum insured, and
reused evidence is caught by fingerprint (R13, R22). `claims::decide` approves straight through
only within the STP authority (R57): cover in force and past its waiting periods (R07, R47, R53),
no human-care need, an anomaly score below the risk review threshold, the assessor's approval and
confidence, the amount claimed within tolerance of the estimate, the payment within `stp_limit`,
and not drawn in the 1-in-20 sample, which waits for a person before payment (R14). It pays the
amount claimed up to the sum insured (first loss, R48), cash first or in kind at fair value. Only
a person declines, with a reason from a closed list; a different person reviews a decline, and any
decision by the AI or the rules can be put to a person (R15). The anomaly score is the sum of named
fraud indicators; the random sample's overturn rate is reported per decision kind; the fairness
audit flags groups below four-fifths of the best group's approval rate (R19); a reward is voided
only on a person's finding of fraud. `crates/workers` runs four claims on saved replies in demo
mode and writes the people's queue, which `apps/ops-console` renders offline (urgent first, only
claim referrals offering a decline). Open: consented pilot claim photos to score the assessor
live; the insurer's claims data to calibrate the anomaly weights and STP thresholds. The API
routes behind the console's buttons followed in Phase 6, with the guarantee of payment recorded
for the assistance provider (ADR-011).

## Phase 6: channels
`api` routes + generated client → customer app → embed SDK → partner console.

Status (2026-10-10): built. ADR-011 (Accepted; its front-end and contract rows amend ADR-001 and
ADR-004, for confirmation): the policy store (`crates/store`, PostgreSQL or memory, versioned
writes) is the system of record; every call carries a token naming one principal (customer,
partner, a named person, a data source), so a person decides in their own name and a decline is
reviewed by someone else (R15). New business is a quote, kept as shown with the suggestion, its
factors, the sentences and the price (R49), then one tap that binds through a licensed partner or
the insurer's own journey, from 00:00 the next day (R52, R58, new); rates are effective-dated so a
period may start any day. 48 routes, each in one table that builds the router and writes the
OpenAPI paths; the TypeScript client is typed from them. Payouts are recorded for the customer
and streamed live, from a pushed feed (`POST /webhooks/triggers/{source}`) or a worker, which now
reads its book from the store (closing Phase 2's open item). The customer app (plans, quote and
own split, cover, renewal, declarations and rating answers, prevention tasks, score and rewards,
payouts with the cash-or-alternative choice, claims, review requests, privacy and cancellation),
the embed (a Web Component; the partner's server exchanges its own customer reference for a
short-lived customer token; a referral-only partner shows the way to the insurer), the partner
console (results withheld below `partner_insights_min_cohort`, R21; spare capacity as rewards) and
the ops console (the live queue; decisions only as offered, R13) are framework-free TypeScript on
the generated client, with customer copy per language in `config/copy/`. `just dev` builds them and
runs the API in demo mode offline: a synthetic book seeded from the engine's own suggestions and a
clock at the organiser's cancellation. Open: hosting for front ends (ADR-001); the insurer's
identity provider, payment provider and assistance provider integrations; the job queue for batch
jobs and a load test of the non-AI routes (Phase 7).

## Phase 7: adoption
Data loop + partner insights → second market config (swap adapters) → pilot evaluation →
Terraform deploy, with the PostgreSQL job queue for batch jobs over the book and a load test of
the API.

Status (2026-10-10): built. ADR-012 (Accepted; the second market, the wait-list control group R59
and front-end hosting are flagged for confirmation). **Second market:** Taiwan, as an overlay on
`config/` (`config/markets/tw/`, `MARKET=tw`): its insurer profile (TWD, the Financial Supervisory
Commission, PDPA, zh-Hant first), its official stages for the typhoon id, its language packs and
its weather income cover, triggered by the local government's work suspension where the customer
declared they work (R55 amended). One generic CAP 1.2 adapter reads its three sources (work
suspensions, typhoon warnings, rain advisories); every source now names its adapter in config.
Building it found what was still Hong Kong: routes read from one named source, adapters chosen by
id, HK$ and the allocation step in the apps, and no way to pay an event reported per area, nor to
count one typhoon across counties as one loss occurrence (R51 amended, `across_zones`). `MARKET=tw
DEMO_MODE=1 cargo run -p microinsurance-workers` replays a typhoon's suspensions offline.
**Job queue:** one PostgreSQL table claimed with `FOR UPDATE SKIP LOCKED`, keys enqueued once,
leases, backoff and failed jobs for a person (`GET /ops/jobs`); the daily jobs issue renewal
notices as issued (R35), open periods on them or the customer's change and bill the premium (R03,
R04), erase a cancelled customer's optional data with a record (R54), measure accumulation every
`refresh_minutes` (R51) and run the monthly experience study. **Data loop:** experience studies
(A/E by class and period, limited-fluctuation credibility, prevention frequency with and without
each measure; `GET /ops/experience`) and `microinsurance_economics.experience`, which turns one
into a proposed rating and prevention basis for the actuary to file; partner results gain the
funnel and retention (R21). **Pilot:** `config/pilot.yaml` with gates fixed before the start,
the wait-list control (R59), the metrics from the book (`GET /ops/pilot`, shown in the ops
console) and `microinsurance_economics.pilot_evaluation` (scale, hold or stop).
**Deployment:** Terraform for one AWS deployment per market (network, RDS, evidence bucket,
secrets, ECS for the API and workers, CloudFront with WAF, locked backups for record retention,
alarms) and a Dockerfile that runs as the code expects. **Load test** against PostgreSQL:
customer reads and quotes meet p99 < 50 ms; binding and the back-office reads do not yet
(`docs/performance.md`). Open: the real CAP feed URLs and codes; Taiwan's filed rates; the gates'
values; the partner results as a daily job, an index of open work items and binding in one
transaction; `terraform validate` against the AWS provider.

## Product rollout (for adopters)
0–3 months: one partner, three automatic covers. 3–6 months: closed pilot (sandbox). 6–12
months: add claim-based covers (device, small medical), more reward partners. 12 months +:
other markets by swapping data feeds.
