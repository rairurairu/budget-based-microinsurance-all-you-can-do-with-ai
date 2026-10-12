//! Reads `config/` from disk and hands the texts to the engine, which validates them (R30: the
//! engine itself never touches the disk). Used by the workers, the API and the AI evaluation
//! runner.
//!
//! A market other than the base reads its overlay, `config/markets/<market>/`, over `config/`
//! (ADR-012): a file in the overlay replaces the base file of the same name; `catalogue/`,
//! `languages/` and `copy/` merge by file name; `triggers/` is replaced as a whole when the
//! overlay has one (data sources never cross a border); `event_types.yaml` merges by id (the
//! engine reads the base, then the overlay). The overlay's insurer profile is `insurer.yaml`.
use microinsurance_engine::config::{self, ConfigSources, EngineConfig};
use serde::Deserialize;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

fn read(path: &Path) -> Result<String, String> {
    fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))
}

/// The `*.yaml` files of a directory by file name; none when it does not exist.
fn yaml_files(dir: &Path) -> Result<BTreeMap<String, PathBuf>, String> {
    if !dir.is_dir() {
        return Ok(BTreeMap::new());
    }
    let mut out = BTreeMap::new();
    for entry in fs::read_dir(dir).map_err(|e| format!("{}: {e}", dir.display()))? {
        let path = entry.map_err(|e| format!("{}: {e}", dir.display()))?.path();
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        if name.ends_with(".yaml") {
            out.insert(name.to_string(), path);
        }
    }
    Ok(out)
}

/// How the overlay combines with a base directory.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Merge {
    /// The overlay's file replaces the base file of the same name.
    ByFileName,
    /// An overlay directory replaces the base directory as a whole.
    Whole,
}

/// The base directory of the overlay markets.
pub const MARKETS: &str = "markets";

/// Where a process reads its config: `config/`, and a market's overlay over it (ADR-012).
#[derive(Debug, Clone)]
pub struct ConfigDir {
    pub base: PathBuf,
    /// `config/markets/<market>/`, read over the base.
    pub overlay: Option<PathBuf>,
    /// The insurer profile's file name: `insurer.yaml` in a market overlay, otherwise
    /// `insurer.example.yaml` unless named.
    pub insurer: String,
}

impl ConfigDir {
    /// The config in `base` for `market` (none: the base market), with the insurer profile
    /// `insurer` when named.
    pub fn new(
        base: &Path,
        market: Option<&str>,
        insurer: Option<&str>,
    ) -> Result<ConfigDir, String> {
        let overlay = match market {
            None => None,
            Some(m) => {
                let dir = base.join(MARKETS).join(m);
                if !dir.is_dir() {
                    return Err(format!("market {m}: no {}", dir.display()));
                }
                Some(dir)
            }
        };
        let insurer = insurer.map_or_else(
            || {
                if overlay.is_some() {
                    "insurer.yaml".to_string()
                } else {
                    "insurer.example.yaml".to_string()
                }
            },
            str::to_string,
        );
        Ok(ConfigDir {
            base: base.to_path_buf(),
            overlay,
            insurer,
        })
    }

    /// The base market's config in `base`, with the example insurer profile.
    pub fn base(base: &Path) -> ConfigDir {
        ConfigDir {
            base: base.to_path_buf(),
            overlay: None,
            insurer: "insurer.example.yaml".into(),
        }
    }

    /// From the environment: `MARKET` (a directory of `config/markets/`) and `INSURER_PROFILE`,
    /// with `root` the repository root.
    pub fn from_env(root: &Path) -> Result<ConfigDir, String> {
        let var = |k: &str| std::env::var(k).ok().filter(|v| !v.is_empty());
        ConfigDir::new(
            &root.join("config"),
            var("MARKET").as_deref(),
            var("INSURER_PROFILE").as_deref(),
        )
    }

    /// Every market overlay in `base`, by name.
    pub fn markets(base: &Path) -> Result<Vec<String>, String> {
        let dir = base.join(MARKETS);
        if !dir.is_dir() {
            return Ok(Vec::new());
        }
        let mut out = Vec::new();
        for entry in fs::read_dir(&dir).map_err(|e| format!("{}: {e}", dir.display()))? {
            let path = entry.map_err(|e| e.to_string())?.path();
            if path.is_dir()
                && let Some(name) = path.file_name().and_then(|n| n.to_str())
            {
                out.push(name.to_string());
            }
        }
        out.sort();
        Ok(out)
    }

    /// The market's name: the overlay's directory, or `None` for the base.
    pub fn market(&self) -> Option<&str> {
        self.overlay
            .as_ref()
            .and_then(|d| d.file_name())
            .and_then(|n| n.to_str())
    }

    /// The path of a top-level file: the overlay's when it has one.
    pub fn file(&self, name: &str) -> PathBuf {
        self.overlay
            .as_ref()
            .map(|o| o.join(name))
            .filter(|p| p.is_file())
            .unwrap_or_else(|| self.base.join(name))
    }

    /// The files of a subdirectory after the overlay, in file name order.
    fn files(&self, sub: &str, merge: Merge) -> Result<Vec<PathBuf>, String> {
        let mut files = yaml_files(&self.base.join(sub))?;
        if let Some(o) = &self.overlay {
            let dir = o.join(sub);
            if dir.is_dir() {
                if merge == Merge::Whole {
                    files.clear();
                }
                files.extend(yaml_files(&dir)?);
            }
        }
        Ok(files.into_values().collect())
    }

    fn texts(&self, sub: &str, merge: Merge) -> Result<Vec<String>, String> {
        self.files(sub, merge)?.iter().map(|p| read(p)).collect()
    }

    /// Loads and validates the market's config.
    pub fn load(&self) -> Result<EngineConfig, String> {
        let catalogue = self.texts("catalogue", Merge::ByFileName)?;
        let sources = self.texts("triggers", Merge::Whole)?;
        let catalogue: Vec<&str> = catalogue.iter().map(String::as_str).collect();
        let sources: Vec<&str> = sources.iter().map(String::as_str).collect();
        let mut event_types = vec![read(&self.base.join("event_types.yaml"))?];
        if let Some(o) = &self.overlay {
            let more = o.join("event_types.yaml");
            if more.is_file() {
                event_types.push(read(&more)?);
            }
        }
        let event_types: Vec<&str> = event_types.iter().map(String::as_str).collect();
        let pilot = self.pilot_text()?;
        config::from_yaml(&ConfigSources {
            insurer: &self.insurer_text()?,
            lines_of_business: &read(&self.file("lines_of_business.yaml"))?,
            rating_factors: &read(&self.file("rating_factors.yaml"))?,
            event_types: &event_types,
            trigger_sources: &sources,
            exposure_types: &read(&self.file("exposure_types.yaml"))?,
            data_types: &read(&self.file("data_types.yaml"))?,
            catalogue: &catalogue,
            prevention_measures: &read(&self.file("prevention_measures.yaml"))?,
            suppliers: &read(&self.file("suppliers.yaml"))?,
            pilot: pilot.as_deref(),
        })
        .map_err(|e| match self.market() {
            Some(m) => format!("market {m}: {e}"),
            None => e.to_string(),
        })
    }

    /// The text of the insurer profile, for sections the engine does not read (`ai`).
    pub fn insurer_text(&self) -> Result<String, String> {
        read(&self.file(&self.insurer))
    }

    /// The data sources of the market (`triggers/*.yaml`), each with how a worker reaches it.
    pub fn sources(&self) -> Result<Vec<SourceAccess>, String> {
        self.files("triggers", Merge::Whole)?
            .iter()
            .map(|p| serde_saphyr::from_str(&read(p)?).map_err(|e| format!("{}: {e}", p.display())))
            .collect()
    }

    fn source_path(&self, id: &str) -> Result<PathBuf, String> {
        let name = format!("{id}.yaml");
        self.files("triggers", Merge::Whole)?
            .into_iter()
            .find(|p| p.file_name().and_then(|n| n.to_str()) == Some(name.as_str()))
            .ok_or_else(|| format!("no data source {id} in this market"))
    }

    pub fn source_access(&self, id: &str) -> Result<SourceAccess, String> {
        let path = self.source_path(id)?;
        serde_saphyr::from_str(&read(&path)?).map_err(|e| format!("{}: {e}", path.display()))
    }

    /// The text of a data source's config file, for an adapter's own section (`normalise`,
    /// `lines`, `areas`).
    pub fn source_text(&self, id: &str) -> Result<String, String> {
        read(&self.source_path(id)?)
    }

    /// The language packs, `languages/<tag>.yaml`, as (tag, text) in name order.
    pub fn language_files(&self) -> Result<Vec<(String, String)>, String> {
        self.tagged("languages")
    }

    /// The words the customer app and the embed show, one file per offered language
    /// (`copy/<tag>.yaml`), as (tag, text) in name order.
    pub fn copy_files(&self) -> Result<Vec<(String, String)>, String> {
        self.tagged("copy")
    }

    fn tagged(&self, sub: &str) -> Result<Vec<(String, String)>, String> {
        self.files(sub, Merge::ByFileName)?
            .iter()
            .map(|p| {
                let tag = p
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .ok_or_else(|| format!("{}: file name", p.display()))?;
                Ok((tag.to_string(), read(p)?))
            })
            .collect()
    }

    /// The route ids a customer can declare: every data source's `lines` (R55).
    pub fn declared_routes(&self) -> Result<Vec<String>, String> {
        Ok(self.zones(|f| f.lines)?.into_iter().collect())
    }

    /// The work areas a customer can declare: every data source's `areas` (R55, ADR-012).
    pub fn declared_areas(&self) -> Result<Vec<String>, String> {
        Ok(self.zones(|f| f.areas)?.into_iter().collect())
    }

    fn zones(
        &self,
        pick: fn(ZoneFile) -> BTreeMap<String, String>,
    ) -> Result<BTreeSet<String>, String> {
        let mut out = BTreeSet::new();
        for p in self.files("triggers", Merge::Whole)? {
            let file: ZoneFile =
                serde_saphyr::from_str(&read(&p)?).map_err(|e| format!("{}: {e}", p.display()))?;
            out.extend(pick(file).into_values());
        }
        Ok(out)
    }

    /// What the API and the apps need beyond the engine: the `channels` section of the insurer
    /// profile (ADR-011).
    pub fn channel_settings(&self) -> Result<ChannelSettings, String> {
        let section: ChannelsSection = serde_saphyr::from_str(&self.insurer_text()?)
            .map_err(|e| format!("{} channels: {e}", self.insurer))?;
        Ok(section.channels)
    }

    /// The pilot and its gates (`pilot.yaml`, ADR-012), when the market has one.
    pub fn pilot_text(&self) -> Result<Option<String>, String> {
        let path = self.file("pilot.yaml");
        if path.is_file() {
            read(&path).map(Some)
        } else {
            Ok(None)
        }
    }
}

/// How a worker reaches a data source: the fields of `triggers/<id>.yaml` the engine does not
/// need.
#[derive(Debug, Clone, Deserialize)]
pub struct SourceAccess {
    pub id: String,
    /// The adapter that reads the feed's format (ADR-012); none for a source with no adapter
    /// built yet.
    #[serde(default)]
    pub adapter: Option<String>,
    #[serde(default)]
    pub live: Option<String>,
    #[serde(default)]
    pub poll_seconds: Option<u64>,
    #[serde(default)]
    pub demo_mode_fixture: Option<String>,
    #[serde(default)]
    pub history: SourceHistory,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct SourceHistory {
    /// A local export of the source's historical record, relative to the repository root.
    #[serde(default)]
    pub import: Option<String>,
}

/// The zones a source names: rail lines and administrative areas, each from the feed's own code
/// to the id a customer declares.
#[derive(Deserialize, Default)]
struct ZoneFile {
    #[serde(default)]
    lines: BTreeMap<String, String>,
    #[serde(default)]
    areas: BTreeMap<String, String>,
}

/// What the API and the apps need beyond the engine: the `channels` section of the insurer
/// profile (ADR-011).
#[derive(Debug, Clone, Deserialize)]
pub struct ChannelSettings {
    /// Where a referral-only partner sends the customer: the insurer's own journey (R52).
    pub own_journey_url: String,
    /// How long a customer token a partner exchanges for lasts.
    pub customer_token_minutes: u32,
    /// No figure from fewer customers than this reaches a partner (R21).
    pub partner_insights_min_cohort: u32,
    /// The web origins each partner embeds the journey from, by partner id.
    #[serde(default)]
    pub partner_origins: BTreeMap<String, Vec<String>>,
}

#[derive(Deserialize)]
struct ChannelsSection {
    channels: ChannelSettings,
}

/// Loads and validates the base market's config in `dir` with the insurer profile `insurer`
/// (for example `insurer.example.yaml`).
pub fn load(dir: &Path, insurer: &str) -> Result<EngineConfig, String> {
    ConfigDir::new(dir, None, Some(insurer))?.load()
}

/// Experience studies for the actuary: the `experience` section of the insurer profile
/// (ADR-012).
#[derive(Debug, Clone, Copy, Deserialize)]
pub struct ExperienceSettings {
    /// Claims for full credibility (limited fluctuation; 1,082: 90% probability within 5%).
    #[serde(default = "full_credibility_claims")]
    pub full_credibility_claims: u32,
}

fn full_credibility_claims() -> u32 {
    1_082
}

impl Default for ExperienceSettings {
    fn default() -> ExperienceSettings {
        ExperienceSettings {
            full_credibility_claims: full_credibility_claims(),
        }
    }
}

#[derive(Deserialize)]
struct ExperienceSection {
    #[serde(default)]
    experience: Option<ExperienceSettings>,
}

impl ConfigDir {
    pub fn experience_settings(&self) -> Result<ExperienceSettings, String> {
        let section: ExperienceSection = serde_saphyr::from_str(&self.insurer_text()?)
            .map_err(|e| format!("{} experience: {e}", self.insurer))?;
        Ok(section.experience.unwrap_or_default())
    }
}
