//! Read known events for the next months (a trip, a concert, a new device, a change of hours or
//! commute) from text the customer shares, as registered upcoming event kinds for the customer
//! to confirm. The model never sets a value: a ticket price or booked cost is declared by the
//! customer (R02).
use crate::client::Model;
use crate::harness::{Ask, Harness, Harnessed};
use crate::settings::Tier;
use crate::vocab::Vocabulary;
use chrono::{Months, NaiveDate};
use microinsurance_engine::types::UpcomingEvent;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::json;

pub const COMPONENT: &str = "foresee";

/// The most events one reading returns.
pub const MAX_EVENTS: usize = 10;
const MAX_REFERENCE_CHARS: usize = 64;

#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ForeseenEvent {
    /// A registered upcoming event kind, such as `event.concert` or `trip.air`.
    pub kind: String,
    pub starts: NaiveDate,
    pub ends: Option<NaiveDate>,
    /// A booking or ticket reference the customer wrote, as written.
    pub reference: Option<String>,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ForeseeReply {
    pub events: Vec<ForeseenEvent>,
}

/// The last day an event may start: `horizon_months` after `today`.
pub fn horizon_end(today: NaiveDate, horizon_months: u8) -> NaiveDate {
    today
        .checked_add_months(Months::new(u32::from(horizon_months)))
        .unwrap_or(NaiveDate::MAX)
}

/// The checks a reading must pass: registered kinds, dates inside the horizon.
pub fn check(
    reply: ForeseeReply,
    today: NaiveDate,
    horizon_months: u8,
    vocab: &Vocabulary,
) -> Result<Vec<UpcomingEvent>, String> {
    if reply.events.len() > MAX_EVENTS {
        return Err(format!(
            "{} events; at most {MAX_EVENTS}",
            reply.events.len()
        ));
    }
    let last = horizon_end(today, horizon_months);
    reply
        .events
        .into_iter()
        .map(|e| {
            if !vocab.upcoming_event_kinds.contains(&e.kind) {
                return Err(format!(
                    "R55: {:?} is not a registered upcoming event kind",
                    e.kind
                ));
            }
            if e.starts < today || e.starts > last {
                return Err(format!(
                    "{} starts {} outside {today}..{last}",
                    e.kind, e.starts
                ));
            }
            if e.ends.is_some_and(|end| end < e.starts) {
                return Err(format!("{} ends before it starts", e.kind));
            }
            let reference = e
                .reference
                .map(|r| r.trim().to_string())
                .filter(|r| !r.is_empty());
            if reference
                .as_ref()
                .is_some_and(|r| r.chars().count() > MAX_REFERENCE_CHARS)
            {
                return Err("reference too long".into());
            }
            Ok(UpcomingEvent {
                kind: e.kind,
                starts: e.starts,
                ends: e.ends,
                reference,
                value: None,
            })
        })
        .collect()
}

/// Upcoming events in `text`, for the customer to confirm; none when the reading fails.
pub async fn extract_upcoming_events<M: Model>(
    h: &Harness<M>,
    text: &str,
    today: NaiveDate,
    horizon_months: u8,
    vocab: &Vocabulary,
) -> Harnessed<Vec<UpcomingEvent>> {
    let input = json!({
        "today": today,
        "last_start_date": horizon_end(today, horizon_months),
        "upcoming_event_kinds": vocab.upcoming_event_kinds,
        "text": text,
    });
    let ask = Ask {
        component: COMPONENT,
        tier: Tier::Fast,
        input,
        images: &[],
    };
    h.ask(
        ask,
        |r: ForeseeReply| check(r, today, horizon_months, vocab),
        Vec::new,
    )
    .await
}
