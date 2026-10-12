# ADR-008: Rail, road, flight and organiser feeds

- Status: Accepted
- Date: 2026-10-09
- Decider: Tsang Cheuk Nam (Phase 3 go-ahead and the choice of the operator's rail feed on
  2026-10-09; built by engineering within R55 and ADR-006)

## Context

Phase 2 left the transit, flight and organiser adapters open (docs/08-roadmap.md). Each turns a
feed into `SourceReport`s in the source's own codes; classification, matching and payouts stay in
the engine (R55, ADR-006). What was open is which feed each reads, how a report names the zone a
customer declares, and when a report is in force.

Criteria: a payout can be checked against the public record; the zone a customer declares matches
the zone a feed reports exactly; no code, phrase or threshold is in code (R55); no AI on the
payout path. Where a structured feed exists, it is read instead of free text.

## Decision

| Source | Feed | Zone and reference | In force | Industry basis |
|---|---|---|---|---|
| `mtr_line_status` | The MTR's own line status file (the one behind its app): one status code per line | The route id a customer declares for the line (`lines`: `TWL` → `mtr.tsuen_wan`) | From the file's `lastBuildDate` (local time of the operator's change) while the line's code is one the config maps (`YELLOW` delay, `RED` suspension); back to `GREEN`, it is no longer in force | Public-transport disruption covers trigger on the operator's published service status |
| `transit_disruptions` | Transport Department Special Traffic News (data dictionary v4.0), road closures only | TD's district | While TD lists the closure and does not mark it `CLOSED`; re-opened or removed, it ends | The authority's published closure notice |
| `flight_status` | Hong Kong International Airport flight information, departures, polled with the local date | `<flight number>/<scheduled local date>` (`CX880/2026-11-02`), one report per flight number so a codeshare booking matches | A delay from the scheduled time, with its minutes late (estimated, then actual at departure, when it ends); a cancellation while listed | Flight-delay covers measure from the scheduled departure in the airport's record, per flight and date |
| `organiser_notices` | Ticketing notices in the engine's notice format (feed or webhook) | The `event_id` the customer's ticket carries; a per-ticket `ticket_reference` when sent | From the announcement until the event's scheduled start | Event-cancellation covers trigger on the organiser's official notice |

- Rail disruptions come only from the operator's codes; no text is parsed. Every configured line
  is reported with its status and the source's config registers the codes that pay, so the adapter
  holds no notion of "normal".
- TD's feed has no closure codes, so road closures are read from its text with the phrases in the
  `normalise` section of `config/triggers/transit_disruptions.yaml` (English and Chinese): a code
  needs one of its phrases, none of its exceptions (a lane closure is not a road closure) and none
  of `not_in_force` (re-opened, "will be"). No cover pays on road closures yet.
- Feeds report what they say; covers decide what counts. The flight adapter reports every late
  departure; `flight_delay` pays from `min_minutes: 120`. Unregistered statuses are reported as is
  and ignored by the engine (R55).
- Each source is polled by its own trigger desk with its own saved state and lead
  (`poll_triggers/<source>`); a placeholder `live` URL is skipped.
- None of these sources has a historical record in config, so covers on them are priced on the
  insurer's filed rate (R55).
- The organiser notice format is this engine's own; a partner's format is out of scope for a
  demonstration of the method.

## Consequences

- `DEMO_MODE=1 cargo run -p microinsurance-workers` replays a Tsuen Wan Line suspension from the
  operator's status file and a concert cancellation besides the typhoon, offline.
- The route ids a customer can declare are the operator's lines in config
  (`microinsurance_config::declared_routes`).
