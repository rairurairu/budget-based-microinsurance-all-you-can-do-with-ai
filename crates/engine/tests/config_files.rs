//! Loads the real `config/` through the engine's validation (R24, R33, R37, R43, R55). Run by
//! `just check`, so an invalid edit to config fails before it is committed.
use microinsurance_engine::config::{self, ConfigSources};
use std::fs;
use std::path::{Path, PathBuf};

fn config_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../config")
}

fn read(path: &Path) -> String {
    fs::read_to_string(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

/// The texts of every `*.yaml` in a directory whose name passes `keep`, in name order.
fn yaml_files(dir: &Path, keep: impl Fn(&str) -> bool) -> Vec<(PathBuf, String)> {
    let mut paths: Vec<PathBuf> = fs::read_dir(dir)
        .unwrap_or_else(|e| panic!("{}: {e}", dir.display()))
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.ends_with(".yaml") && keep(n))
        })
        .collect();
    paths.sort();
    paths
        .into_iter()
        .map(|p| {
            let text = read(&p);
            (p, text)
        })
        .collect()
}

#[test]
fn every_insurer_profile_loads_with_the_registries_and_catalogue() {
    let dir = config_dir();
    let catalogue = yaml_files(&dir.join("catalogue"), |_| true);
    let catalogue: Vec<&str> = catalogue.iter().map(|(_, t)| t.as_str()).collect();
    let sources = yaml_files(&dir.join("triggers"), |_| true);
    let sources: Vec<&str> = sources.iter().map(|(_, t)| t.as_str()).collect();
    let lines = read(&dir.join("lines_of_business.yaml"));
    let factors = read(&dir.join("rating_factors.yaml"));
    let event_types = read(&dir.join("event_types.yaml"));
    let exposure_types = read(&dir.join("exposure_types.yaml"));
    let data_types = read(&dir.join("data_types.yaml"));
    let prevention_measures = read(&dir.join("prevention_measures.yaml"));
    let suppliers = read(&dir.join("suppliers.yaml"));
    let pilot = read(&dir.join("pilot.yaml"));

    let insurers = yaml_files(&dir, |n| n.starts_with("insurer."));
    assert!(!insurers.is_empty(), "no config/insurer.*.yaml");
    for (path, text) in insurers {
        let cfg = config::from_yaml(&ConfigSources {
            insurer: &text,
            lines_of_business: &lines,
            rating_factors: &factors,
            event_types: &[&event_types],
            trigger_sources: &sources,
            exposure_types: &exposure_types,
            data_types: &data_types,
            catalogue: &catalogue,
            prevention_measures: &prevention_measures,
            suppliers: &suppliers,
            pilot: Some(&pilot),
        })
        .unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        assert_eq!(cfg.catalogue.len(), catalogue.len());
        assert_eq!(cfg.triggers.sources.len(), sources.len());
    }
}
