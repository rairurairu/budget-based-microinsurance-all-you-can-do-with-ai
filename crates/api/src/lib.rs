//! The HTTP API (ADR-011). It asks `ai` for recommendations and `engine` for every decision,
//! posts the engine's amounts to `ledger`, keeps records in `store` and streams payouts to the
//! customer. Routes are listed once, in `routes::ROUTES`, which also writes the OpenAPI paths.
pub mod assistance;
pub mod auth;
pub mod bodies;
pub mod copy;
pub mod demo;
pub mod error;
pub mod handlers;
pub mod identity;
pub mod routes;
pub mod state;

use crate::auth::TokenKeys;
use crate::copy::CopyPacks;
use crate::state::{AppState, Clock, Inner, Ledger};
use axum::Router;
use axum::extract::{DefaultBodyLimit, Request, State};
use axum::http::{HeaderValue, Method, header};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Redirect, Response};
use chrono::{DateTime, Utc};
use microinsurance_ai::client::{AnthropicClient, AnyModel};
use microinsurance_ai::harness::Harness;
use microinsurance_ai::language::Languages;
use microinsurance_ai::prompts::Prompts;
use microinsurance_ai::replay::ReplayModel;
use microinsurance_ai::settings::AiSettings;
use microinsurance_ai::vocab::Vocabulary;
use microinsurance_config::ConfigDir;
use microinsurance_ledger::{MemoryLedger, PgLedger};
use microinsurance_store::{PgStore, Store};
use microinsurance_workers::ops::AuditStore;
use microinsurance_workers::queue::{PgQueue, Queue};
use microinsurance_workers::state::PgState;
use serde::de::DeserializeOwned;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tower_http::cors::{AllowOrigin, CorsLayer};
use tower_http::services::ServeDir;

/// Evidence uploads are base64 JSON: a few photos.
const BODY_LIMIT: usize = 12 * 1024 * 1024;

/// How to start the API.
#[derive(Debug, Clone)]
pub struct Settings {
    /// The repository root: `config/`, `fixtures/`, `crates/ai/prompts/`, `apps/`.
    pub root: PathBuf,
    /// A market overlay in `config/markets/` (ADR-012); none: the base market.
    pub market: Option<String>,
    /// The insurer profile; by default the market's.
    pub insurer: Option<String>,
    /// Offline: saved AI replies, a synthetic book in memory, demo sessions (ADR-011).
    pub demo: bool,
    pub database_url: Option<String>,
    /// Connections to PostgreSQL for the store and for the ledger, each: about the requests one
    /// process serves at once (`DATABASE_POOL_SIZE`, default 32).
    pub pool_size: u32,
    /// Signs tokens; at least 32 bytes outside demo mode.
    pub token_secret: Option<Vec<u8>>,
    /// The insurer's secret audit seed (R14).
    pub audit_seed: Option<u64>,
    /// Market data files: the rating basis, the prevention basis and the audit store.
    pub rating_basis: Option<PathBuf>,
    pub prevention_basis: Option<PathBuf>,
    pub audit_store: Option<PathBuf>,
    /// Demo mode's clock starts here.
    pub demo_start: Option<DateTime<Utc>>,
    /// Demo mode with the live model (`AI_LIVE=1` and `ANTHROPIC_API_KEY`): the synthetic book,
    /// but every AI component calls the provider; a failed call still gives its saved answer (R22).
    pub ai_live: bool,
}

impl Settings {
    /// From the environment: `REPO_ROOT`, `MARKET`, `INSURER_PROFILE`, `DEMO_MODE=1`, `DATABASE_URL`,
    /// `API_TOKEN_SECRET`, `AUDIT_SEED`, `RATING_BASIS`, `PREVENTION_BASIS`, `AUDIT_STORE`,
    /// `DEMO_START`, `DATABASE_POOL_SIZE`, `AI_LIVE=1`.
    pub fn from_env() -> Result<Settings, String> {
        let var = |k: &str| std::env::var(k).ok().filter(|v| !v.is_empty());
        Ok(Settings {
            root: var("REPO_ROOT").map_or_else(|| PathBuf::from("."), PathBuf::from),
            market: var("MARKET"),
            insurer: var("INSURER_PROFILE"),
            demo: var("DEMO_MODE").is_some_and(|v| v == "1"),
            database_url: var("DATABASE_URL"),
            pool_size: var("DATABASE_POOL_SIZE")
                .map(|s| s.parse().map_err(|_| "DATABASE_POOL_SIZE is a number"))
                .transpose()?
                .unwrap_or(32),
            token_secret: var("API_TOKEN_SECRET").map(String::into_bytes),
            audit_seed: var("AUDIT_SEED")
                .map(|s| s.parse().map_err(|_| "AUDIT_SEED is a number"))
                .transpose()?,
            rating_basis: var("RATING_BASIS").map(PathBuf::from),
            prevention_basis: var("PREVENTION_BASIS").map(PathBuf::from),
            audit_store: var("AUDIT_STORE").map(PathBuf::from),
            demo_start: var("DEMO_START")
                .map(|s| {
                    DateTime::parse_from_rfc3339(&s)
                        .map(|t| t.with_timezone(&Utc))
                        .map_err(|e| format!("DEMO_START: {e}"))
                })
                .transpose()?,
            ai_live: var("AI_LIVE").is_some_and(|v| v == "1"),
        })
    }
}

fn json_file<T: DeserializeOwned>(path: &Path) -> Result<T, String> {
    let text = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    serde_json::from_slice(&text).map_err(|e| format!("{}: {e}", path.display()))
}

/// A data file: the one named, or in demo mode its synthetic fixture, or `fallback` when
/// neither applies.
fn data_file<T: DeserializeOwned>(
    named: Option<&PathBuf>,
    demo: bool,
    fixture: &Path,
    fallback: Option<T>,
    what: &str,
) -> Result<T, String> {
    match (named, demo, fallback) {
        (Some(p), _, _) => json_file(p),
        (None, true, _) => json_file(fixture),
        (None, false, Some(f)) => Ok(f),
        (None, false, None) => Err(format!("{what} is needed outside demo mode")),
    }
}

/// Builds everything a request shares, validating config, copy and data at start-up.
pub async fn build(s: Settings) -> Result<AppState, String> {
    let config = ConfigDir::new(
        &s.root.join("config"),
        s.market.as_deref(),
        s.insurer.as_deref(),
    )?;
    let fixtures = s.root.join("fixtures");
    let cfg = config.load()?;
    let channels = config.channel_settings()?;
    let copy = CopyPacks::load(
        &config.copy_files()?,
        &cfg.languages.default,
        &cfg.languages.offered,
    )?;
    let settings = AiSettings::from_insurer_yaml(&config.insurer_text()?)?;
    let languages = Languages::load(&config.language_files()?, &cfg)?;
    let model = if s.demo && !s.ai_live {
        AnyModel::Replay(ReplayModel::load(&fixtures.join("ai_replies"))?)
    } else {
        match AnthropicClient::from_env(settings.clone()) {
            Ok(client) => {
                eprintln!("ai      live: {}", settings.provider);
                AnyModel::Live(client)
            }
            Err(e) if s.demo => {
                eprintln!("ai      {e}: demo mode replays the saved replies");
                AnyModel::Replay(ReplayModel::load(&fixtures.join("ai_replies"))?)
            }
            Err(e) => {
                eprintln!("ai      {e}: every AI component gives its saved answer (R22)");
                AnyModel::Replay(ReplayModel::default())
            }
        }
    };
    let ai = Harness::new(model, Prompts::in_repo(&s.root), settings, languages);
    let vocab = Vocabulary::from_config(&cfg, config.declared_routes()?);
    let areas = config.declared_areas()?.into_iter().collect();
    // Demo mode's synthetic data: the market's own file in fixtures/markets/<market>/, if it has
    // one (amounts in its currency), otherwise the base market's.
    let synthetic = |name: &str| {
        config
            .market()
            .map(|m| fixtures.join("markets").join(m).join(name))
            .filter(|p| p.is_file())
            .unwrap_or_else(|| fixtures.join(name))
    };
    let rating = data_file(
        s.rating_basis.as_ref(),
        s.demo,
        &synthetic("rating_basis_synthetic.json"),
        None,
        "RATING_BASIS",
    )?;
    let prevention = data_file(
        s.prevention_basis.as_ref(),
        s.demo,
        &synthetic("prevention_basis_synthetic.json"),
        Some(Default::default()),
        "PREVENTION_BASIS",
    )?;
    let audit_store: AuditStore = data_file(
        s.audit_store.as_ref(),
        s.demo,
        &synthetic("guardrails_synthetic.json"),
        Some(AuditStore {
            fairness: Vec::new(),
            audit_findings: Vec::new(),
        }),
        "AUDIT_STORE",
    )?;
    let secret = match (&s.token_secret, s.demo) {
        (Some(k), _) if k.len() >= 32 => k.clone(),
        (Some(_), _) => return Err("API_TOKEN_SECRET needs at least 32 bytes".into()),
        (None, true) => demo::TOKEN_SECRET.to_vec(),
        (None, false) => return Err("API_TOKEN_SECRET is needed outside demo mode".into()),
    };
    let audit_seed = match (s.audit_seed, s.demo) {
        (Some(seed), _) => seed,
        (None, true) => demo::AUDIT_SEED,
        (None, false) => {
            return Err("AUDIT_SEED, the insurer's secret, is needed outside demo mode".into());
        }
    };
    let (store, ledger, worker_state, queue) = match (&s.database_url, s.demo) {
        (Some(url), false) => {
            let store = PgStore::connect_with(url, s.pool_size)
                .await
                .map_err(|e| e.to_string())?;
            store.migrate().await.map_err(|e| e.to_string())?;
            let ledger = PgLedger::connect_with(url, s.pool_size)
                .await
                .map_err(|e| e.to_string())?;
            ledger.migrate().await.map_err(|e| e.to_string())?;
            let state = PgState::connect(url).await?;
            state.migrate().await?;
            let queue = PgQueue::connect(url).await?;
            queue.migrate().await?;
            (
                Store::Pg(store),
                Ledger::Pg(ledger),
                Some(state),
                Queue::Pg(queue),
            )
        }
        _ => (
            Store::memory(),
            Ledger::Memory(MemoryLedger::new()),
            None,
            Queue::memory(),
        ),
    };
    let clock = if s.demo {
        Clock::from(s.demo_start.unwrap_or_else(demo::start))
    } else {
        Clock::System
    };
    let state = AppState(std::sync::Arc::new(Inner {
        cfg,
        channels,
        copy,
        store,
        ledger,
        ai,
        vocab,
        rating,
        prevention,
        audit_store,
        clock,
        keys: TokenKeys::new(&secret),
        audit_seed,
        desks: tokio::sync::Mutex::default(),
        worker_state,
        kyc: if s.demo {
            Box::new(identity::DemoKyc)
        } else {
            Box::new(identity::NoKyc)
        },
        assistance: Box::new(assistance::RecordOnly),
        areas,
        queue,
        config,
        root: s.root.clone(),
        demo: s.demo,
        ids: std::sync::atomic::AtomicU64::new(0),
    }));
    if s.demo {
        demo::seed(&state, &s.root).await?;
    }
    Ok(state)
}

/// The front ends, each built into `apps/<app>/dist` (`pnpm -r build`), at their own path.
const APPS: &[(&str, &str)] = &[
    ("/app", "customer-app"),
    ("/embed", "embed-sdk"),
    ("/partner", "partner-console"),
    ("/ops", "ops-console"),
];

/// The router: every route of the table, demo routes in demo mode, the partners' origins for
/// the embed (CORS), and the built front ends.
pub fn app(state: AppState, root: &Path) -> Router {
    let origins: Vec<HeaderValue> = state
        .channels
        .partner_origins
        .values()
        .flatten()
        .filter_map(|o| HeaderValue::from_str(o).ok())
        .collect();
    let cors = CorsLayer::new()
        .allow_origin(AllowOrigin::list(origins))
        .allow_methods([Method::GET, Method::POST, Method::PUT, Method::PATCH])
        .allow_headers([header::AUTHORIZATION, header::CONTENT_TYPE]);
    let mut router = routes::router();
    if state.demo {
        router = router.merge(demo::router());
    }
    let mut app = router
        .layer(DefaultBodyLimit::max(BODY_LIMIT))
        .layer(cors)
        .with_state(state);
    let mut mounted = Vec::new();
    for (path, dir) in APPS {
        let dist = root.join("apps").join(dir).join("dist");
        if dist.is_dir() {
            app = app.nest_service(path, ServeDir::new(dist));
            mounted.push(*path);
        }
    }
    app.layer(middleware::from_fn_with_state(
        Arc::new(mounted),
        trailing_slash,
    ))
}

/// An app's files are linked relative to its directory, so `/app` redirects to `/app/`.
async fn trailing_slash(
    State(mounted): State<Arc<Vec<&'static str>>>,
    req: Request,
    next: Next,
) -> Response {
    let path = req.uri().path();
    if mounted.contains(&path) {
        let query = req
            .uri()
            .query()
            .map(|q| format!("?{q}"))
            .unwrap_or_default();
        return Redirect::permanent(&format!("{path}/{query}")).into_response();
    }
    next.run(req).await
}
