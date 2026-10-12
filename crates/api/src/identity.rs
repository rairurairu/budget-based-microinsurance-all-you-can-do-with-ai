//! Identity checks through a licensed eKYC provider (R25). The API starts a check and reads its
//! status; the provider holds the documents, the API keeps only the session and its outcome.
//! Each insurer connects its own provider; demo mode uses `DemoKyc`, which verifies at once.
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum KycStatus {
    Pending,
    Verified,
    Failed,
}

pub trait KycProvider {
    /// Starts a check for the customer; returns the provider's session id.
    fn start(&self, customer_id: &str) -> Result<String, String>;
    fn status(&self, session_id: &str) -> Result<KycStatus, String>;
}

/// Demo mode's provider: every check is verified. Never used live.
pub struct DemoKyc;

impl KycProvider for DemoKyc {
    fn start(&self, customer_id: &str) -> Result<String, String> {
        Ok(format!("kyc-demo-{customer_id}"))
    }

    fn status(&self, _session_id: &str) -> Result<KycStatus, String> {
        Ok(KycStatus::Verified)
    }
}

/// No provider connected: a check cannot start, and the API says so.
pub struct NoKyc;

impl KycProvider for NoKyc {
    fn start(&self, _customer_id: &str) -> Result<String, String> {
        Err("no eKYC provider is connected (R25)".into())
    }

    fn status(&self, _session_id: &str) -> Result<KycStatus, String> {
        Err("no eKYC provider is connected (R25)".into())
    }
}
