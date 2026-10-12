//! The trigger data sources this worker can poll, built from their config files: each file names
//! the adapter that reads its feed's format (`adapter:`, ADR-012), so a market swaps sources by
//! config, never by code. Each source is polled by its own trigger desk, with its own saved state
//! and lead (`poll_triggers/<id>`).
use crate::config_dir::ConfigDir;
use microinsurance_engine::config::EngineConfig;
use microinsurance_triggers::Feed;
use microinsurance_triggers::adapters::cap::CapAlerts;
use microinsurance_triggers::adapters::rail::{Lines, RailStatus};
use microinsurance_triggers::adapters::transit::Normalise;
use microinsurance_triggers::adapters::{
    Adapter, flight, flight::FlightStatus, hko, hko::HkoWarnings, organiser,
    organiser::OrganiserNotices, rail, transit, transit::TransitDisruptions,
};

/// The adapters built, by the name a source's config gives them, with the source id an adapter
/// written for one feed reports under (`None`: a standard format, any source id).
pub const ADAPTERS: &[(&str, Option<&str>)] = &[
    ("hko_warnsum", Some(hko::SOURCE_ID)),
    ("td_special_traffic_news", Some(transit::SOURCE_ID)),
    ("mtr_line_status", Some(rail::SOURCE_ID)),
    ("hkia_flight_info", Some(flight::SOURCE_ID)),
    ("organiser_notice", Some(organiser::SOURCE_ID)),
    ("cap", None),
];

/// The desk's saved state and lead lock for a source.
pub fn desk_key(source: &str) -> String {
    format!("poll_triggers/{source}")
}

/// The market's sources with a built adapter, in id order.
pub fn built(config: &ConfigDir) -> Result<Vec<String>, String> {
    Ok(config
        .sources()?
        .into_iter()
        .filter(|s| {
            s.adapter
                .as_deref()
                .is_some_and(|a| ADAPTERS.iter().any(|(name, _)| *name == a))
        })
        .map(|s| s.id)
        .collect())
}

/// The adapter for the source `id`, reading `feed`, as its config file names it.
pub fn adapter(
    id: &str,
    feed: Feed,
    config: &ConfigDir,
    cfg: &EngineConfig,
) -> Result<Adapter, String> {
    let access = config.source_access(id)?;
    let kind = access
        .adapter
        .ok_or_else(|| format!("{id}: its config names no adapter"))?;
    let (_, own) = ADAPTERS
        .iter()
        .find(|(name, _)| *name == kind)
        .ok_or_else(|| format!("{id}: no adapter {kind} built"))?;
    if let Some(own) = own
        && *own != id
    {
        return Err(format!("{id}: the {kind} adapter reads only {own}"));
    }
    let offset = cfg.local_offset();
    let text = || config.source_text(id);
    Ok(match kind.as_str() {
        "hko_warnsum" => Adapter::Hko(HkoWarnings {
            live: feed,
            history: None,
        }),
        "td_special_traffic_news" => Adapter::Transit(TransitDisruptions {
            live: feed,
            rules: Normalise::from_source_yaml(&text()?).map_err(|e| e.to_string())?,
            utc_offset: offset,
        }),
        "mtr_line_status" => Adapter::Rail(RailStatus {
            live: feed,
            lines: Lines::from_source_yaml(&text()?).map_err(|e| e.to_string())?,
            utc_offset: offset,
        }),
        "hkia_flight_info" => Adapter::Flight(FlightStatus {
            live: feed,
            utc_offset: offset,
        }),
        "organiser_notice" => Adapter::Organiser(OrganiserNotices { live: feed }),
        "cap" => Adapter::Cap(
            CapAlerts::from_source_yaml(id, feed, &text()?).map_err(|e| e.to_string())?,
        ),
        other => return Err(format!("{id}: no adapter {other} built")),
    })
}

/// The live feed of a source, when its config names a reachable URL (not a placeholder).
pub fn live_feed(config: &ConfigDir, id: &str) -> Result<Option<(Feed, u64)>, String> {
    let access = config.source_access(id)?;
    Ok(access
        .live
        .filter(|u| u.starts_with("https://") || u.starts_with("http://"))
        .map(|u| (Feed::Live(u), access.poll_seconds.unwrap_or(60))))
}
