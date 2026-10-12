//! Rail line status from the operator's own feed, matched to declared routes.
//!
//! Live: the MTR line status file (`ryg_line_status.json`), one entry per line with the
//! operator's `line_code` and a `status` code, and `lastBuildDate`, the local time the operator
//! last changed the file. Every line is reported with its status, upper-cased, as the source's
//! code; the source's config maps the codes that pay (`codes`) and the engine ignores the rest
//! (R55), so a line back to normal is simply no longer in force. The zone is the route id a
//! customer declares for the line (`lines` in `config/triggers/mtr_line_status.yaml`); a line
//! the config does not list is skipped. A disruption starts at the file's `lastBuildDate`.
//!
//! History: none (the source's `history.available` is false; covers on it are priced on the
//! insurer's filed rate).
use crate::{Feed, SourceError, TriggerSource};
use chrono::{FixedOffset, NaiveDate, NaiveDateTime, TimeZone, Utc};
use microinsurance_engine::triggers::SourceReport;
use serde::Deserialize;
use std::collections::BTreeMap;

pub const SOURCE_ID: &str = "mtr_line_status";

/// The operator's line codes and the route id a customer declares for each.
#[derive(Debug, Clone, Deserialize)]
pub struct Lines {
    pub lines: BTreeMap<String, String>,
}

impl Lines {
    /// Reads the `lines` section of `config/triggers/mtr_line_status.yaml`.
    pub fn from_source_yaml(text: &str) -> Result<Lines, SourceError> {
        serde_saphyr::from_str(text)
            .map_err(|e| SourceError::Payload(format!("{SOURCE_ID}.yaml lines: {e}")))
    }
}

pub struct RailStatus {
    pub live: Feed,
    pub lines: Lines,
    /// Market local time: the feed's time carries no offset.
    pub utc_offset: FixedOffset,
}

#[derive(Deserialize)]
struct File {
    ryg_status: Status,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Status {
    last_build_date: String,
    #[serde(default)]
    line: Vec<Line>,
}

#[derive(Deserialize)]
struct Line {
    line_code: String,
    status: String,
}

/// The status file as reports, one per configured line.
pub fn parse_status(
    json: &str,
    lines: &Lines,
    offset: FixedOffset,
) -> Result<Vec<SourceReport>, SourceError> {
    let file: File = serde_json::from_str(json).map_err(|e| SourceError::Payload(e.to_string()))?;
    let built = file.ryg_status.last_build_date.trim();
    let local = NaiveDateTime::parse_from_str(built, "%Y-%m-%d %H:%M:%S")
        .map_err(|e| SourceError::Payload(format!("lastBuildDate {built:?}: {e}")))?;
    let starts = offset
        .from_local_datetime(&local)
        .single()
        .ok_or_else(|| SourceError::Payload(format!("lastBuildDate {built:?}")))?
        .with_timezone(&Utc);
    Ok(file
        .ryg_status
        .line
        .iter()
        .filter_map(|l| {
            let zone = lines.lines.get(l.line_code.trim())?;
            Some(SourceReport {
                report_id: format!("{}:{built}", l.line_code.trim()),
                source: SOURCE_ID.into(),
                code: l.status.trim().to_uppercase(),
                starts,
                ends: None,
                zone: Some(zone.clone()),
                delay_minutes: None,
                reference: None,
            })
        })
        .collect())
}

impl TriggerSource for RailStatus {
    fn id(&self) -> &str {
        SOURCE_ID
    }

    async fn poll(&self) -> Result<Vec<SourceReport>, SourceError> {
        parse_status(&self.live.text().await?, &self.lines, self.utc_offset)
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
