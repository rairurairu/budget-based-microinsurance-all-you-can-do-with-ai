# ADR-006: Trigger events, loss occurrences and two-stage payouts

- Status: Accepted
- Date: 2026-10-09
- Decider: Tsang Cheuk Nam (delegated to engineering on 2026-10-09, decided on the criteria below)

## Context

Phase 2 (docs/08-roadmap.md) turns feed reports into payouts. R08, R27, R51 and R55 say what
must hold; they leave open what one "trigger event" is for payouts and limits, which policy
period it attaches to, how declared hours are counted, and how a multi-day event is paid in two
stages.

Criteria, in order: the customer is paid for the disruption the product promises, once, quickly
and in a way they can check against the warning history; the insurer's limits line up with how
its reinsurance programme defines one loss; the terms read like existing parametric and
property-catastrophe wordings so the insurer, its reinsurer and the Insurance Authority
recognise them; every value is config the insurer can file.

## Decision

| Question | Decision | Industry basis |
|---|---|---|
| Unit of tracking | A **trigger event**: one event type in force in one zone (and, for a per-customer report, one reference) from first report until its source stops reporting it. Raised from Signal 8 to 10 and lowered to 3, a typhoon is one trigger event with its stage history | Parametric index definitions: peril, index, threshold, geography, independent of any one feed (ADR-005) |
| Unit of payment and limit | A **loss occurrence**: the trigger events of one **peril** (`accumulation.perils`) in one zone, each starting within the peril's `aggregation_hours` of the last one ending. A typhoon's signals and the black rain it brings are one occurrence: one event limit, one payout per stage (R27, R51) | The loss-occurrence and hours clause of catastrophe reinsurance, where a windstorm's wind and associated rain are one loss. Without it one storm would draw two event limits and could pay a day twice |
| One benefit per day | A cover's event types must all be in one peril, and a peril paid by a daily benefit aggregates over at least 24 hours, so each working day is paid at most once however many warnings touch it | Daily-benefit wordings: "one benefit per day" |
| Which policy period pays | The period in force on the local day the occurrence starts; the cover must have been in force and past its waiting period that day. The whole occurrence is paid on that period's terms, even if the policy ends or is re-rated during it | Losses-occurring basis with an extended-expiration provision; cover bought into an event already in force pays nothing on it (also R06) |
| Declared hours (R08) | A day counts when at least `min_disrupted_minutes` of declared work fall in the disruption; a night shift belongs to the day it starts | Hours clause / minimum duration in parametric income covers: a benefit sized near daily earnings (R09) needs a real loss of working time behind it |
| Resumption time (R08) | The disruption runs `resumption_minutes` past the moment the trigger ends or falls below the cover's stage. Weather income: 120 minutes | The Labour Department's Code of Practice in Times of Typhoons and Rainstorms expects a reasonable time to report back once a signal is lowered; workers on a 09:00 shift lose work when Signal 8 comes down at 08:30 |
| Minutes in force | `min_minutes` is the longest unbroken stretch at the cover's stage or above, not a total | Index triggers and delay covers count consecutive duration ("for at least N consecutive minutes") |
| Hourly benefit | Pro rata to the minute, down to the cent, up to `max_hours` | Linear payout removes the cliff at each whole hour, the main source of basis-risk disputes |
| Stage one: guaranteed share (R51) | Paid once per policy when it first qualifies: the share `event_limit ÷ benefit in force` (at most 100%) of what it is owed then, up to one day's or one event's benefit. Benefit in force: every eligible cover's benefit when the occurrence opens, whether or not it has qualified yet | Prudent: the guaranteed total can never pass the limit, so nothing paid is ever clawed back |
| Stage two: balance (R51) | When the occurrence closes (`aggregation_hours` without anything in force): everything owed less the guaranteed share, reduced pro rata if the total would pass the event limit (ADR-004 largest remainder). Later days and stage rises are paid here | Final settlement once the event's total is known, as for a reinsurer's loss advice |
| R27 key | `payout/{policy}/{occurrence}/{guaranteed or balance}`; a policy with two covers paying on one occurrence gets one payout with a line per cover | |
| Zone accumulation (R51) | Benefit in force per peril and zone; a cover paying on a typhoon and its rain counts once. Above `zone_accumulation_limit`, no new or increased premium goes into the zone; existing cover renews | Capacity management on aggregate exposure, not non-renewal (R41 leaves cancellation to the customer) |
| Settlement (R16, R28, R53) | Cash first, always. In kind only where every cover in the payout permits it, a partner can provide it now and the customer accepts. A cash payout in the grace period is paid net of the premium due; in kind leaves it due | Set-off of outstanding premium against a claim; settlement in kind only by agreement |
| Local time | A fixed UTC offset per market (`utc_offset_minutes`, 480 for Hong Kong); a market with daylight saving needs a time zone table first | |
| District zones | Not offered until a declared work district is modelled, with consent (R20) | |

## Consequences

- `engine::triggers` tracks trigger events (`track`, `from_history`), groups them into loss
  occurrences (`group`, `group_history`) and matches policies (`match_occurrence`);
  `engine::payouts` decides both stages and settlement. `crates/workers` (`desk`) carries the
  state between polls.
- A balance is paid a day after a typhoon season's storm goes quiet. Most customers are paid in
  full at once: the guaranteed share is 100% whenever the benefit in force is within the limit.
- Trigger pricing replays the record through the same occurrences and matcher, so the priced
  and the paid definitions cannot drift.
- Weather income's key features state the daily benefit, the hour of disrupted work, the two
  hours to resume and one benefit per day (`config/catalogue/weather_income.yaml`).
- Measured: 1,000,000 policies matched and their guaranteed payouts decided in 1.6 s on one core.
- Edge left open: resumption time that spills past midnight into the next local day can count
  toward a night shift starting that day. With `aggregation_hours` ≥ 24 this stays within one
  occurrence and so one benefit per day.
