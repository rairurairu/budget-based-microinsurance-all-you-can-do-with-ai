//! Every market loads: the base config and each overlay in `config/markets/` (ADR-012), through
//! the engine's validation (R24, R33, R37, R51, R55). Run by `just check`.
use microinsurance_config::ConfigDir;
use std::path::{Path, PathBuf};

fn config() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../config")
}

#[test]
fn every_market_loads() {
    let markets = ConfigDir::markets(&config()).expect("markets");
    assert!(markets.contains(&"tw".to_string()), "{markets:?}");
    for market in std::iter::once(None).chain(markets.iter().map(|m| Some(m.as_str()))) {
        let dir = ConfigDir::new(&config(), market, None).expect("market directory");
        let cfg = dir.load().unwrap_or_else(|e| panic!("{market:?}: {e}"));
        dir.channel_settings()
            .unwrap_or_else(|e| panic!("{market:?}: {e}"));
        for source in dir.sources().expect("sources") {
            assert!(
                cfg.triggers.sources.contains_key(&source.id),
                "{market:?}: {} is not registered",
                source.id
            );
        }
    }
}

#[test]
fn taiwan_swaps_the_sources_and_keeps_the_product_design() {
    let base = ConfigDir::base(&config()).load().expect("base");
    let dir = ConfigDir::new(&config(), Some("tw"), None).expect("tw");
    let tw = dir.load().expect("tw loads");
    assert_eq!((tw.market.as_str(), tw.currency.as_str()), ("TW", "TWD"));
    // The data sources are the market's own: none of Hong Kong's cross the border.
    assert!(tw.triggers.sources.contains_key("dgpa_work_suspension"));
    assert!(!tw.triggers.sources.contains_key("hko_warnings"));
    // The cover keeps its class of business and payout basis; its trigger is local.
    let (hk_cover, tw_cover) = (
        base.cover("weather_income").expect("hk"),
        tw.cover("weather_income").expect("tw"),
    );
    assert_eq!(hk_cover.class_of_business, tw_cover.class_of_business);
    assert_eq!(
        hk_cover.payout.as_ref().map(|p| p.basis),
        tw_cover.payout.as_ref().map(|p| p.basis)
    );
    let events: Vec<&str> = tw_cover
        .trigger
        .iter()
        .flat_map(|t| t.events.iter().map(|e| e.event_type.as_str()))
        .collect();
    assert_eq!(events, ["work.suspension"]);
    // Covers it does not replace come from the base catalogue.
    assert_eq!(
        tw.cover("device_damage").expect("tw").class_of_business,
        base.cover("device_damage").expect("hk").class_of_business
    );
    // The typhoon keeps its id; its stages are Taiwan's official warnings.
    assert_eq!(
        tw.triggers.event_types["weather.tropical_cyclone"].stages,
        ["sea_warning", "sea_and_land_warning"]
    );
    // A customer declares a city or county as their work area; routes stay Hong Kong's.
    let areas = dir.declared_areas().expect("areas");
    assert!(areas.contains(&"tw.taipei_city".to_string()));
    assert_eq!(areas.len(), 22);
    assert!(dir.declared_routes().expect("routes").is_empty());
    assert!(
        ConfigDir::base(&config())
            .declared_areas()
            .expect("hk")
            .is_empty()
    );
    // The language packs are Taiwan's: one replaced per tag, both offered.
    let packs = dir.language_files().expect("packs");
    let zh = packs.iter().find(|(t, _)| t == "zh-Hant").expect("zh-Hant");
    assert!(zh.1.contains("捷運"));
}
