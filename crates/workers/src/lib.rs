//! Scheduled jobs (README.md). Each job is glue: it reads data, calls the engine for every
//! decision and posts the engine's amounts to the ledger unchanged. Phase 2 builds
//! `PollTriggers` (`desk`) and the monthly renewal (`renewal`); Phase 4 the prevention task jobs
//! (`prevention`) and supplier settlement (`suppliers`); Phase 5 the claims jobs (`claims`), the
//! weekly fairness report and the ops console's snapshot (`ops`); the other jobs arrive with
//! their roadmap phases.
pub mod book;
pub mod claims;
pub mod demo;
pub mod desk;
pub mod jobs;
pub mod ops;
pub mod prevention;
pub mod queue;
pub mod rating;
pub mod renewal;
pub mod sources;
pub mod state;
pub mod suppliers;

/// Reading `config/` from disk (shared with the AI evaluation runner).
pub use microinsurance_config as config_dir;
