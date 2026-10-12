//! Rail, road, flight and organiser adapters on saved (synthetic) feed responses.
use chrono::{DateTime, FixedOffset, NaiveDate, Utc};
use microinsurance_engine::triggers::SourceReport;
use microinsurance_triggers::adapters::flight::{self, FlightStatus};
use microinsurance_triggers::adapters::organiser::{self, OrganiserNotices};
use microinsurance_triggers::adapters::rail::{self, Lines, RailStatus};
use microinsurance_triggers::adapters::transit::{self, Normalise, TransitDisruptions};
use microinsurance_triggers::{Feed, SourceError, TriggerSource};
use std::path::PathBuf;

fn repo(path: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(path)
}

fn hong_kong() -> FixedOffset {
    FixedOffset::east_opt(8 * 3600).expect("offset")
}

fn utc(s: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(s)
        .expect("time")
        .with_timezone(&Utc)
}

fn rules() -> Normalise {
    let text = std::fs::read_to_string(repo("config/triggers/transit_disruptions.yaml"))
        .expect("source config");
    Normalise::from_source_yaml(&text).expect("normalise section")
}

fn transit(fixture: &str) -> TransitDisruptions {
    TransitDisruptions {
        live: Feed::Fixture(repo("fixtures").join(fixture)),
        rules: rules(),
        utc_offset: hong_kong(),
    }
}

fn find<'a>(reports: &'a [SourceReport], zone: &str) -> &'a SourceReport {
    reports
        .iter()
        .find(|r| r.zone.as_deref() == Some(zone))
        .unwrap_or_else(|| panic!("no report for {zone}"))
}

#[tokio::test]
async fn rail_reports_each_line_in_its_declared_route() {
    let text =
        std::fs::read_to_string(repo("config/triggers/mtr_line_status.yaml")).expect("config");
    let source = RailStatus {
        live: Feed::Fixture(repo("fixtures/mtr_line_status_suspended.json")),
        lines: Lines::from_source_yaml(&text).expect("lines"),
        utc_offset: hong_kong(),
    };
    let reports = source.poll().await.expect("feed parses");
    // Every configured line, with the operator's status as the code: the engine registers only
    // the codes that pay, so a line in normal service is not in force.
    assert_eq!(reports.len(), 11);
    let suspended = find(&reports, "mtr.tsuen_wan");
    assert_eq!(suspended.code, "RED");
    assert_eq!(suspended.source, "mtr_line_status");
    assert_eq!(suspended.starts, utc("2026-11-03T00:12:00Z"));
    assert_eq!(suspended.ends, None);
    assert_eq!(find(&reports, "mtr.kwun_tong").code, "YELLOW");
    assert_eq!(find(&reports, "mtr.island").code, "GREEN");

    let only_two = Lines {
        lines: [("TWL".to_string(), "mtr.tsuen_wan".to_string())].into(),
    };
    let json =
        std::fs::read_to_string(repo("fixtures/mtr_line_status_suspended.json")).expect("fixture");
    assert_eq!(
        rail::parse_status(&json, &only_two, hong_kong())
            .expect("parses")
            .len(),
        1
    );
    assert!(matches!(
        rail::parse_status(
            r#"{"ryg_status": {"lastBuildDate": "today", "line": []}}"#,
            &only_two,
            hong_kong()
        ),
        Err(SourceError::Payload(_))
    ));
}

#[tokio::test]
async fn road_closures_are_reported_for_their_district_only() {
    let reports = transit("td_trafficnews.xml")
        .poll()
        .await
        .expect("feed parses");
    // Rail messages are not read from TD text; a lane closure, a closed incident and a planned
    // closure are not in force.
    let seen: Vec<(&str, &str)> = reports
        .iter()
        .map(|r| (r.zone.as_deref().unwrap_or("-"), r.code.as_str()))
        .collect();
    assert_eq!(seen, [("Tsuen Wan", "ROAD_CLOSED")]);
    let road = &reports[0];
    assert_eq!(road.source, "transit_disruptions");
    assert_eq!(road.report_id, "IN-26-90004:Tsuen Wan");
    assert_eq!(road.starts, utc("2026-11-02T22:40:00Z"));
    assert_eq!(road.ends, None);
}

#[test]
fn road_closures_read_chinese_text_and_reject_bad_dates() {
    let xml = |date: &str, text: &str| {
        format!(
            "<list><message><INCIDENT_NUMBER>IN-1</INCIDENT_NUMBER>             <DISTRICT_EN>Sha Tin</DISTRICT_EN>             <ANNOUNCEMENT_DATE>{date}</ANNOUNCEMENT_DATE>             <INCIDENT_STATUS_EN>NEW</INCIDENT_STATUS_EN>             <CONTENT_CN>{text}</CONTENT_CN></message></list>"
        )
    };
    let closed = xml("2026-11-03T08:00:00", "因山泥傾瀉，大埔公路現已全線封閉。");
    let reports = transit::parse_feed(&closed, &rules(), hong_kong()).expect("parses");
    assert_eq!(reports.len(), 1);
    assert_eq!(reports[0].zone.as_deref(), Some("Sha Tin"));
    let reopened = xml("2026-11-03T09:00:00", "較早前全線封閉的大埔公路現已解封。");
    assert!(
        transit::parse_feed(&reopened, &rules(), hong_kong())
            .expect("parses")
            .is_empty()
    );
    assert!(matches!(
        transit::parse_feed(&xml("3 Nov 2026", "全線封閉"), &rules(), hong_kong()),
        Err(SourceError::Payload(_))
    ));
    assert!(
        transit::parse_feed("<list></list>", &rules(), hong_kong())
            .expect("empty")
            .is_empty()
    );
}

#[tokio::test]
async fn flight_reports_late_and_cancelled_departures_per_flight_number() {
    let source = FlightStatus {
        live: Feed::Fixture(repo("fixtures/hkia_departures_delayed.json")),
        utc_offset: hong_kong(),
    };
    let reports = source.poll().await.expect("feed parses");
    let seen: Vec<(&str, &str, Option<u32>)> = reports
        .iter()
        .map(|r| {
            (
                r.zone.as_deref().unwrap_or("-"),
                r.code.as_str(),
                r.delay_minutes,
            )
        })
        .collect();
    // Early, boarding and blank statuses are not reported.
    assert_eq!(
        seen,
        [
            ("CX880/2026-11-01", "DELAYED", Some(165)),
            ("AA8933/2026-11-01", "DELAYED", Some(165)),
            ("CX251/2026-11-02", "DELAYED", Some(195)),
            ("UO624/2026-11-02", "CANCELLED", None),
            ("CX5624/2026-11-02", "CANCELLED", None),
            ("HX035/2026-11-02", "DELAYED", Some(20)),
        ]
    );
    let departed = find(&reports, "CX880/2026-11-01");
    assert_eq!(departed.starts, utc("2026-11-01T15:55:00Z"));
    assert_eq!(departed.ends, Some(utc("2026-11-01T18:40:00Z")));
    // Still waiting: in force, no end yet.
    assert_eq!(find(&reports, "CX251/2026-11-02").ends, None);
}

#[test]
fn flight_rejects_unreadable_times() {
    let bad = r#"[{"date":"2026-11-02","arrival":false,"list":[
        {"time":"10:00","flight":[{"no":"CX 1"}],"status":"Dep 25:99"}]}]"#;
    assert!(matches!(
        flight::parse_departures(bad, hong_kong()),
        Err(SourceError::Payload(_))
    ));
    let arrivals = r#"[{"date":"2026-11-02","arrival":true,"list":[
        {"time":"10:00","flight":[{"no":"CX 1"}],"status":"Cancelled"}]}]"#;
    assert!(
        flight::parse_departures(arrivals, hong_kong())
            .expect("parses")
            .is_empty()
    );
}

#[tokio::test]
async fn organiser_notices_report_the_event_and_status() {
    let source = OrganiserNotices {
        live: Feed::Fixture(repo("fixtures/organiser_cancellation.json")),
    };
    let reports = source.poll().await.expect("feed parses");
    let codes: Vec<(&str, &str)> = reports
        .iter()
        .map(|r| (r.zone.as_deref().unwrap_or("-"), r.code.as_str()))
        .collect();
    // An unregistered status is reported as is; the engine ignores it (R55).
    assert_eq!(
        codes,
        [
            ("EVT-2026-1121-ARENA", "CANCELLED"),
            ("EVT-2026-1128-STADIUM", "POSTPONED"),
            ("EVT-2026-1205-THEATRE", "VENUE_CHANGED"),
        ]
    );
    let cancelled = &reports[0];
    assert_eq!(cancelled.starts, utc("2026-11-18T07:00:00Z"));
    // In force until the show was due to start.
    assert_eq!(cancelled.ends, Some(utc("2026-11-21T12:00:00Z")));
    assert_eq!(reports[2].ends, None);
    assert_eq!(cancelled.report_id, "N-2026-0412");

    let one = organiser::parse_webhook(
        r#"{"notice_id":"N-9","event_id":"EVT-1","status":"cancelled",
            "announced_at":"2026-11-18T15:00:00+08:00","ticket_reference":"T-77"}"#,
    )
    .expect("webhook parses");
    assert_eq!(one.reference.as_deref(), Some("T-77"));
    assert!(matches!(
        organiser::parse_webhook(r#"{"notice_id":"N-9","event_id":"EVT-1","status":"x"}"#),
        Err(SourceError::Payload(_))
    ));
}

#[tokio::test]
async fn sources_without_a_record_have_no_history() {
    let day = |d| NaiveDate::from_ymd_opt(2026, 1, d).expect("date");
    let source = OrganiserNotices {
        live: Feed::Fixture(repo("fixtures/organiser_cancellation.json")),
    };
    assert!(matches!(
        source.history(day(1), day(31)).await,
        Err(SourceError::Unavailable(_))
    ));
}
