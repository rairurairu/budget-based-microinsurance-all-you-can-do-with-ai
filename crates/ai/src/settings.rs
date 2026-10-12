//! The `ai` section of the insurer profile.
use serde::Deserialize;
use std::time::Duration;

/// Which model a component uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tier {
    /// Short text tasks on the customer's path (understand, foresee, explain, rewards).
    Fast,
    /// Judgement and images (prevention tasks, verify).
    Strong,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AiSettings {
    pub provider: String,
    pub fast_model: String,
    pub strong_model: String,
    /// Budget for a fast call, retries included (R22).
    pub timeout_ms: u64,
    /// Budget for a strong call.
    #[serde(default = "default_strong_timeout")]
    pub strong_timeout_ms: u64,
    /// `output_config.effort`: these are short, bounded tasks.
    #[serde(default = "default_effort")]
    pub effort: String,
    #[serde(default = "default_max_tokens")]
    pub max_tokens: u32,
    /// The provider's server-side refusal fallback for the strong model (`default`), or `none`.
    #[serde(default = "default_refusal_fallback")]
    pub refusal_fallback: String,
}

fn default_strong_timeout() -> u64 {
    15_000
}

fn default_effort() -> String {
    "low".into()
}

fn default_max_tokens() -> u32 {
    2_048
}

fn default_refusal_fallback() -> String {
    "default".into()
}

#[derive(Deserialize)]
struct InsurerFile {
    ai: AiSettings,
}

impl AiSettings {
    /// Reads the `ai` section of an insurer profile (`config/insurer.*.yaml`).
    pub fn from_insurer_yaml(text: &str) -> Result<AiSettings, String> {
        let file: InsurerFile =
            serde_saphyr::from_str(text).map_err(|e| format!("insurer profile ai: {e}"))?;
        if file.ai.provider != "anthropic" {
            return Err(format!("ai.provider {:?} has no client", file.ai.provider));
        }
        Ok(file.ai)
    }

    pub fn model(&self, tier: Tier) -> &str {
        match tier {
            Tier::Fast => &self.fast_model,
            Tier::Strong => &self.strong_model,
        }
    }

    pub fn timeout(&self, tier: Tier) -> Duration {
        Duration::from_millis(match tier {
            Tier::Fast => self.timeout_ms,
            Tier::Strong => self.strong_timeout_ms,
        })
    }
}
