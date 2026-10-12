//! What every request shares: the validated config, the policy store, the ledger, the AI layer,
//! the market data the engine rates with, the clock and the signing key. Built once at start-up
//! (`main.rs`, or `demo::state` in demo mode); a request never reads config files.
use crate::assistance::AssistanceProvider;
use crate::auth::TokenKeys;
use crate::copy::CopyPacks;
use crate::identity::KycProvider;
use chrono::{DateTime, NaiveDate, Utc};
use microinsurance_ai::client::AnyModel;
use microinsurance_ai::harness::Harness;
use microinsurance_ai::vocab::Vocabulary;
use microinsurance_config::{ChannelSettings, ConfigDir};
use microinsurance_engine::config::EngineConfig;
use microinsurance_engine::money::Money;
use microinsurance_engine::optimiser::rating::RatingBasis;
use microinsurance_engine::prevention::PreventionBasis;
use microinsurance_ledger::{
    Account, LedgerError, LedgerStore, MemoryLedger, PgLedger, PostOutcome, Posting,
};
use microinsurance_store::Store;
use microinsurance_workers::desk::TriggerDesk;
use microinsurance_workers::ops::AuditStore;
use microinsurance_workers::queue::Queue;
use microinsurance_workers::state::PgState;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::ops::Deref;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

/// The ledger a process posts to.
pub enum Ledger {
    Memory(MemoryLedger),
    Pg(PgLedger),
}

impl LedgerStore for Ledger {
    async fn post(
        &self,
        idempotency_key: &str,
        entries: &[Posting],
    ) -> Result<PostOutcome, LedgerError> {
        match self {
            Ledger::Memory(l) => l.post(idempotency_key, entries).await,
            Ledger::Pg(l) => l.post(idempotency_key, entries).await,
        }
    }

    async fn balance(&self, account: Account) -> Result<Money, LedgerError> {
        match self {
            Ledger::Memory(l) => l.balance(account).await,
            Ledger::Pg(l) => l.balance(account).await,
        }
    }
}

/// The time the API works in. Demo mode starts at a set moment of the synthetic world and runs
/// on from there in real time, so the demo book's dates line up with the saved feeds.
#[derive(Debug, Clone, Copy)]
pub enum Clock {
    System,
    From { start: DateTime<Utc>, at: Instant },
}

impl Clock {
    pub fn from(start: DateTime<Utc>) -> Clock {
        Clock::From {
            start,
            at: Instant::now(),
        }
    }

    pub fn now(&self) -> DateTime<Utc> {
        match self {
            Clock::System => Utc::now(),
            Clock::From { start, at } => {
                *start + chrono::Duration::from_std(at.elapsed()).unwrap_or_default()
            }
        }
    }
}

pub struct Inner {
    pub cfg: EngineConfig,
    pub channels: ChannelSettings,
    pub copy: CopyPacks,
    pub store: Store,
    pub ledger: Ledger,
    pub ai: Harness<AnyModel>,
    pub vocab: Vocabulary,
    /// Expected claim costs and factor tables (`python/economics`, the insurer's filed rates).
    pub rating: RatingBasis,
    /// Each measure's claims data (the insurer's claims data, ADR-009).
    pub prevention: PreventionBasis,
    /// The audit store's group counts and earlier findings: never AI inputs (R19).
    pub audit_store: AuditStore,
    pub clock: Clock,
    pub keys: TokenKeys,
    /// The insurer's secret audit seed, recorded with each sample (R14).
    pub audit_seed: u64,
    /// One trigger desk per data source that pushes its reports here.
    pub desks: tokio::sync::Mutex<BTreeMap<String, TriggerDesk>>,
    /// With PostgreSQL: the desks' saved state, shared with the workers.
    pub worker_state: Option<PgState>,
    pub kyc: Box<dyn KycProvider + Send + Sync>,
    pub assistance: Box<dyn AssistanceProvider + Send + Sync>,
    /// The market's config (`config/` and its overlay), for each data source's adapter section.
    pub config: ConfigDir,
    /// The repository root: `fixtures/` for demo mode.
    pub root: PathBuf,
    /// The work areas a customer can declare (`areas` of the market's sources, ADR-012). Never
    /// an AI input (R19).
    pub areas: BTreeSet<String>,
    /// The job queue the workers run batch jobs from (ADR-012); shared through PostgreSQL.
    pub queue: Queue,
    /// Demo mode: the synthetic principals' tokens are offered at `/demo/sessions`.
    pub demo: bool,
    pub ids: AtomicU64,
}

#[derive(Clone)]
pub struct AppState(pub Arc<Inner>);

impl Deref for AppState {
    type Target = Inner;

    fn deref(&self) -> &Inner {
        &self.0
    }
}

impl Inner {
    /// Today in market local time.
    pub fn today(&self) -> NaiveDate {
        self.clock
            .now()
            .with_timezone(&self.cfg.local_offset())
            .date_naive()
    }

    /// A new id: `prefix-` and twelve hex digits, unique across processes.
    pub fn new_id(&self, prefix: &str) -> String {
        let n = self.ids.fetch_add(1, Ordering::Relaxed);
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let digest = Sha256::digest(format!("{nanos}/{n}/{}", std::process::id()).as_bytes());
        let hex: String = digest[..6].iter().map(|b| format!("{b:02x}")).collect();
        format!("{prefix}-{hex}")
    }
}
