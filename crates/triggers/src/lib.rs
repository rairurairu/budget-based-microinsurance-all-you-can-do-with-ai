//! Trigger data sources. Each adapter turns a raw feed into `SourceReport`s in the source's own
//! codes; classification, tracking and matching happen in the engine (R55). Swap adapters per
//! market (ADOPTION.md step 3). In demo mode an adapter reads a saved response from `fixtures/`
//! and never calls the network.
#![allow(async_fn_in_trait)]

pub mod adapters;

use chrono::NaiveDate;
use microinsurance_engine::triggers::SourceReport;
use std::path::PathBuf;

#[derive(Debug, thiserror::Error)]
pub enum SourceError {
    #[error("fetch failed: {0}")]
    Fetch(String),
    #[error("unexpected payload: {0}")]
    Payload(String),
    #[error("not available: {0}")]
    Unavailable(String),
}

/// Where a feed is read from.
#[derive(Debug, Clone)]
pub enum Feed {
    /// The live endpoint.
    Live(String),
    /// A saved response, for demo mode and tests.
    Fixture(PathBuf),
    /// A payload the source pushed (a webhook), already received.
    Inline(String),
}

impl Feed {
    async fn text(&self) -> Result<String, SourceError> {
        match self {
            Feed::Live(url) => {
                let client = reqwest::Client::builder()
                    .timeout(std::time::Duration::from_secs(10))
                    .build()
                    .map_err(|e| SourceError::Fetch(e.to_string()))?;
                client
                    .get(url)
                    .send()
                    .await
                    .and_then(reqwest::Response::error_for_status)
                    .map_err(|e| SourceError::Fetch(e.to_string()))?
                    .text()
                    .await
                    .map_err(|e| SourceError::Fetch(e.to_string()))
            }
            Feed::Fixture(path) => std::fs::read_to_string(path)
                .map_err(|e| SourceError::Fetch(format!("{}: {e}", path.display()))),
            Feed::Inline(text) => Ok(text.clone()),
        }
    }
}

pub trait TriggerSource {
    /// The registered data source id (`config/triggers/*.yaml`).
    fn id(&self) -> &str;
    /// Everything the source reports in force now.
    async fn poll(&self) -> Result<Vec<SourceReport>, SourceError>;
    /// Reports that started in `[from, to]`, each with its end, for trigger pricing and
    /// backtests (`triggers::from_history`).
    async fn history(
        &self,
        from: NaiveDate,
        to: NaiveDate,
    ) -> Result<Vec<SourceReport>, SourceError>;
}
