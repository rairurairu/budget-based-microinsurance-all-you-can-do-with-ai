//! Flight status, matched to declared bookings.
//!
//! Live: Hong Kong International Airport's flight information (`flightinfo-rest`), departures:
//! one entry per scheduled departure with its local scheduled `time`, the flight numbers under
//! it (codeshares) and a `status` such as `Dep 00:37 (08/10/2026)`, `Est at 01:00`,
//! `Cancelled`, `Boarding`. The request needs the local date (`{date}` in the configured URL);
//! the response covers that day and late departures of the day before.
//!
//! Each flight number is reported in its own zone, `<number>/<scheduled local date>` such as
//! `CX880/2026-11-02`: a customer's declared booking (`trip.air`) carries the same reference, so
//! a codeshare booking matches too (R55). A departure later than scheduled is `DELAYED` with its
//! minutes late (estimated while waiting, actual once departed, when it also ends); a
//! cancellation is `CANCELLED`, in force while the airport lists it. The cover's config, not
//! this adapter, says how late counts.
//!
//! History: none (the source's `history.available` is false; covers on it are priced on the
//! insurer's filed rate).
use crate::{Feed, SourceError, TriggerSource};
use chrono::{DateTime, FixedOffset, NaiveDate, NaiveTime, TimeZone, Utc};
use microinsurance_engine::triggers::SourceReport;
use serde::Deserialize;

pub const SOURCE_ID: &str = "flight_status";

pub struct FlightStatus {
    /// The live URL holds `{date}`, replaced by today's local date at each poll.
    pub live: Feed,
    pub utc_offset: FixedOffset,
}

#[derive(Deserialize)]
struct Day {
    date: NaiveDate,
    #[serde(default)]
    arrival: bool,
    #[serde(default)]
    list: Vec<Departure>,
}

#[derive(Deserialize)]
struct Departure {
    time: String,
    #[serde(default)]
    flight: Vec<FlightNumber>,
    #[serde(default)]
    status: String,
}

#[derive(Deserialize)]
struct FlightNumber {
    no: String,
}

enum Status {
    Departed(DateTime<Utc>),
    Estimated(DateTime<Utc>),
    Cancelled,
    Other,
}

/// `HH:MM` with an optional ` (dd/mm/yyyy)` when the day differs from the scheduled one.
fn local_time(
    text: &str,
    day: NaiveDate,
    offset: FixedOffset,
) -> Result<DateTime<Utc>, SourceError> {
    let bad = || SourceError::Payload(format!("flight time {text:?}"));
    let (hm, date) = match text.split_once('(') {
        Some((hm, rest)) => {
            let d = NaiveDate::parse_from_str(rest.trim_end_matches(')').trim(), "%d/%m/%Y")
                .map_err(|_| bad())?;
            (hm.trim(), d)
        }
        None => (text.trim(), day),
    };
    let t = NaiveTime::parse_from_str(hm, "%H:%M").map_err(|_| bad())?;
    offset
        .from_local_datetime(&date.and_time(t))
        .single()
        .map(|t| t.with_timezone(&Utc))
        .ok_or_else(bad)
}

fn status(text: &str, day: NaiveDate, offset: FixedOffset) -> Result<Status, SourceError> {
    let text = text.trim();
    if let Some(rest) = text.strip_prefix("Dep ") {
        return Ok(Status::Departed(local_time(rest, day, offset)?));
    }
    if let Some(rest) = text.strip_prefix("Est at ") {
        return Ok(Status::Estimated(local_time(rest, day, offset)?));
    }
    if text.eq_ignore_ascii_case("Cancelled") {
        return Ok(Status::Cancelled);
    }
    Ok(Status::Other)
}

/// Departures as reports, one per flight number that is late or cancelled.
pub fn parse_departures(json: &str, offset: FixedOffset) -> Result<Vec<SourceReport>, SourceError> {
    let days: Vec<Day> =
        serde_json::from_str(json).map_err(|e| SourceError::Payload(e.to_string()))?;
    let mut out = Vec::new();
    for day in days.into_iter().filter(|d| !d.arrival) {
        for dep in day.list {
            let scheduled = local_time(&dep.time, day.date, offset)?;
            let (code, ends, late) = match status(&dep.status, day.date, offset)? {
                Status::Departed(at) => ("DELAYED", Some(at), at - scheduled),
                Status::Estimated(at) => ("DELAYED", None, at - scheduled),
                Status::Cancelled => ("CANCELLED", None, chrono::TimeDelta::zero()),
                Status::Other => continue,
            };
            let minutes = u32::try_from(late.num_minutes()).unwrap_or(0);
            if code == "DELAYED" && minutes == 0 {
                continue;
            }
            for f in &dep.flight {
                let number: String = f.no.split_whitespace().collect();
                let zone = format!("{number}/{}", day.date);
                out.push(SourceReport {
                    report_id: format!("{zone}:{}", dep.status.trim()),
                    source: SOURCE_ID.into(),
                    code: code.into(),
                    starts: scheduled,
                    ends,
                    zone: Some(zone),
                    delay_minutes: (code == "DELAYED").then_some(minutes),
                    reference: None,
                });
            }
        }
    }
    Ok(out)
}

impl TriggerSource for FlightStatus {
    fn id(&self) -> &str {
        SOURCE_ID
    }

    async fn poll(&self) -> Result<Vec<SourceReport>, SourceError> {
        let feed = match &self.live {
            Feed::Live(url) => {
                let today = Utc::now().with_timezone(&self.utc_offset).date_naive();
                Feed::Live(url.replace("{date}", &today.to_string()))
            }
            fixture => fixture.clone(),
        };
        parse_departures(&feed.text().await?, self.utc_offset)
    }

    async fn history(
        &self,
        _from: NaiveDate,
        _to: NaiveDate,
    ) -> Result<Vec<SourceReport>, SourceError> {
        Err(SourceError::Unavailable(format!(
            "{SOURCE_ID}: no historical record; priced on the insurer's filed rate"
        )))
    }
}
