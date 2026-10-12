//! Road closures, from the Transport Department's Special Traffic News.
//!
//! Live: Special Traffic News (data dictionary v4.0), one `message` per incident with
//! `INCIDENT_NUMBER`, `INCIDENT_STATUS_EN` (`NEW | UPDATED | CLOSED`), `ANNOUNCEMENT_DATE` in
//! local time without offset, `DISTRICT_EN` and the free text `CONTENT_EN` / `CONTENT_CN`.
//!
//! The feed has no closure codes, so the adapter reads the text with the phrase rules in the
//! `normalise` section of `config/triggers/transit_disruptions.yaml` (`Normalise`): a message
//! reports a code for its district when its text contains one of the code's phrases, none of its
//! exceptions and none of `not_in_force`. A closure is in force while TD lists it; when it is
//! closed, re-opened or leaves the feed, it is no longer reported and the engine closes the
//! trigger event (R55). Rail disruptions come from the operator's own status feed (`rail`), not
//! from this text. No phrase is in this code.
//!
//! History: none (the source's `history.available` is false; covers on it are priced on the
//! insurer's filed rate).
use crate::{Feed, SourceError, TriggerSource};
use chrono::{FixedOffset, NaiveDate, NaiveDateTime, TimeZone, Utc};
use microinsurance_engine::triggers::SourceReport;
use serde::Deserialize;
use std::collections::BTreeSet;

pub const SOURCE_ID: &str = "transit_disruptions";

/// One code and the phrases that report it.
#[derive(Debug, Clone, Deserialize)]
pub struct PhraseCode {
    pub code: String,
    /// Any one of these, case-insensitive.
    pub phrases: Vec<String>,
    /// None of these.
    #[serde(default)]
    pub except: Vec<String>,
}

/// The `normalise` section of the source's config file.
#[derive(Debug, Clone, Deserialize)]
pub struct Normalise {
    /// Text that means the closure is over or not yet in force (re-opened, will be).
    #[serde(default)]
    pub not_in_force: Vec<String>,
    /// The zone is the message's district (`DISTRICT_EN`); a message without one is skipped.
    /// The first code whose phrases match wins.
    pub codes: Vec<PhraseCode>,
}

#[derive(Deserialize)]
struct SourceFile {
    normalise: Normalise,
}

impl Normalise {
    /// Reads the `normalise` section of `config/triggers/transit_disruptions.yaml`.
    pub fn from_source_yaml(text: &str) -> Result<Normalise, SourceError> {
        let file: SourceFile = serde_saphyr::from_str(text)
            .map_err(|e| SourceError::Payload(format!("{SOURCE_ID}.yaml normalise: {e}")))?;
        Ok(file.normalise)
    }
}

pub struct TransitDisruptions {
    pub live: Feed,
    pub rules: Normalise,
    /// Market local time: the feed's times carry no offset.
    pub utc_offset: FixedOffset,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
struct Message {
    incident_number: String,
    #[serde(default)]
    incident_heading_en: String,
    #[serde(default)]
    location_en: String,
    #[serde(default)]
    location_cn: String,
    #[serde(default)]
    district_en: String,
    announcement_date: String,
    incident_status_en: String,
    #[serde(default)]
    content_en: String,
    #[serde(default)]
    content_cn: String,
}

#[derive(Debug, Default, Deserialize)]
struct List {
    #[serde(default)]
    message: Vec<Message>,
}

fn contains_any(text: &str, phrases: &[String]) -> bool {
    phrases
        .iter()
        .any(|p| !p.is_empty() && text.contains(&p.to_lowercase()))
}

fn code_for<'a>(text: &str, codes: &'a [PhraseCode]) -> Option<&'a str> {
    codes
        .iter()
        .find(|c| contains_any(text, &c.phrases) && !contains_any(text, &c.except))
        .map(|c| c.code.as_str())
}

/// The feed as reports: each message in force, for its district.
pub fn parse_feed(
    xml: &str,
    rules: &Normalise,
    offset: FixedOffset,
) -> Result<Vec<SourceReport>, SourceError> {
    let list: List =
        quick_xml::de::from_str(xml).map_err(|e| SourceError::Payload(e.to_string()))?;
    let mut out = Vec::new();
    let mut seen = BTreeSet::new();
    for m in list.message {
        if m.incident_status_en.trim().eq_ignore_ascii_case("CLOSED") {
            continue;
        }
        let text = [
            &m.incident_heading_en,
            &m.location_en,
            &m.location_cn,
            &m.content_en,
            &m.content_cn,
        ]
        .map(|s| s.trim())
        .join("\n")
        .to_lowercase();
        if contains_any(&text, &rules.not_in_force) {
            continue;
        }
        let local = NaiveDateTime::parse_from_str(m.announcement_date.trim(), "%Y-%m-%dT%H:%M:%S")
            .map_err(|e| SourceError::Payload(format!("{}: {e}", m.incident_number)))?;
        let starts = offset
            .from_local_datetime(&local)
            .single()
            .ok_or_else(|| SourceError::Payload(format!("{}: local time", m.incident_number)))?
            .with_timezone(&Utc);
        let incident = m.incident_number.trim();
        let mut report = |code: &str, zone: &str| {
            if seen.insert((incident.to_string(), zone.to_string())) {
                out.push(SourceReport {
                    report_id: format!("{incident}:{zone}"),
                    source: SOURCE_ID.into(),
                    code: code.to_string(),
                    starts,
                    ends: None,
                    zone: Some(zone.to_string()),
                    delay_minutes: None,
                    reference: None,
                });
            }
        };
        let district = m.district_en.trim();
        if !district.is_empty()
            && let Some(code) = code_for(&text, &rules.codes)
        {
            report(code, district);
        }
    }
    Ok(out)
}

impl TriggerSource for TransitDisruptions {
    fn id(&self) -> &str {
        SOURCE_ID
    }

    async fn poll(&self) -> Result<Vec<SourceReport>, SourceError> {
        parse_feed(&self.live.text().await?, &self.rules, self.utc_offset)
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
