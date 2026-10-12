//! Model provider clients. The live client calls the Anthropic Messages API over HTTPS (reqwest +
//! serde; there is no official Rust SDK) with structured outputs, so the reply is JSON in the
//! component's schema. `replay::ReplayModel` answers from saved replies in demo mode and tests.
//! Retries and rate limits are handled here; the harness handles timeouts and fallbacks (R22).
use crate::harness::AiError;
use crate::replay::ReplayModel;
use crate::settings::{AiSettings, Tier};
use base64::Engine as _;
use serde_json::{Value, json};
use std::future::Future;
use std::time::Duration;

/// Evidence passed to the model as an image.
#[derive(Debug, Clone)]
pub struct Image {
    /// `image/jpeg`, `image/png`, `image/gif` or `image/webp`.
    pub media_type: String,
    pub bytes: Vec<u8>,
}

#[derive(Debug, Clone)]
pub struct ModelRequest {
    /// The component, for replay keys and logs.
    pub component: &'static str,
    pub tier: Tier,
    /// The prompt file's text.
    pub system: String,
    /// The component's input as JSON, protected attributes already stripped (R19).
    pub user: String,
    pub images: Vec<Image>,
    /// The JSON Schema the reply must match (structured outputs).
    pub schema: Value,
}

/// Anything that turns a request into the model's text. The future is `Send`, so a call can run
/// in any request handler.
pub trait Model {
    fn complete(&self, req: &ModelRequest) -> impl Future<Output = Result<String, AiError>> + Send;
}

pub struct AnthropicClient {
    pub http: reqwest::Client,
    pub api_key: String,
    pub base_url: String,
    pub settings: AiSettings,
    pub max_retries: u32,
}

const API_VERSION: &str = "2023-06-01";
const FALLBACK_BETA: &str = "server-side-fallback-2026-07-01";

impl AnthropicClient {
    /// From `ANTHROPIC_API_KEY` (and `ANTHROPIC_BASE_URL`, if set).
    pub fn from_env(settings: AiSettings) -> Result<AnthropicClient, AiError> {
        let api_key = std::env::var("ANTHROPIC_API_KEY")
            .map_err(|_| AiError::Call("ANTHROPIC_API_KEY is not set".into()))?;
        let base_url = std::env::var("ANTHROPIC_BASE_URL")
            .unwrap_or_else(|_| "https://api.anthropic.com".into());
        let http = reqwest::Client::builder()
            .build()
            .map_err(|e| AiError::Call(e.to_string()))?;
        Ok(AnthropicClient {
            http,
            api_key,
            base_url,
            settings,
            max_retries: 2,
        })
    }

    /// The request body: the prompt as the system prompt, images then the input as the user
    /// turn, and the component's schema as the output format.
    pub fn body(&self, req: &ModelRequest) -> Value {
        let mut content: Vec<Value> = req
            .images
            .iter()
            .map(|i| {
                json!({
                    "type": "image",
                    "source": {
                        "type": "base64",
                        "media_type": i.media_type,
                        "data": base64::engine::general_purpose::STANDARD.encode(&i.bytes),
                    }
                })
            })
            .collect();
        content.push(json!({"type": "text", "text": req.user}));
        let mut body = json!({
            "model": self.settings.model(req.tier),
            "max_tokens": self.settings.max_tokens,
            "system": req.system,
            "messages": [{"role": "user", "content": content}],
            "output_config": {
                "effort": self.settings.effort,
                "format": {"type": "json_schema", "schema": req.schema},
            },
        });
        if self.uses_fallback(req.tier) {
            body["fallbacks"] = json!(self.settings.refusal_fallback);
        }
        body
    }

    fn uses_fallback(&self, tier: Tier) -> bool {
        tier == Tier::Strong && self.settings.refusal_fallback != "none"
    }

    async fn once(&self, req: &ModelRequest) -> Result<Result<String, AiError>, Retry> {
        let mut call = self
            .http
            .post(format!("{}/v1/messages", self.base_url))
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", API_VERSION)
            .json(&self.body(req));
        if self.uses_fallback(req.tier) {
            call = call.header("anthropic-beta", FALLBACK_BETA);
        }
        let resp = match call.send().await {
            Ok(r) => r,
            Err(e) if e.is_connect() || e.is_timeout() => return Err(Retry(None)),
            Err(e) => return Ok(Err(AiError::Call(e.to_string()))),
        };
        let status = resp.status();
        if status.as_u16() == 429 || status.as_u16() == 529 || status.is_server_error() {
            let after = resp
                .headers()
                .get("retry-after")
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.parse::<u64>().ok())
                .map(Duration::from_secs);
            return Err(Retry(after));
        }
        let body: Value = match resp.json().await {
            Ok(b) => b,
            Err(e) => return Ok(Err(AiError::Call(e.to_string()))),
        };
        if !status.is_success() {
            return Ok(Err(AiError::Call(format!("HTTP {status}: {body}"))));
        }
        Ok(reply_text(&body))
    }
}

/// A failure worth retrying, with the provider's `retry-after` when given.
struct Retry(Option<Duration>);

/// The text of a Messages API response, or why there is none.
pub fn reply_text(body: &Value) -> Result<String, AiError> {
    match body["stop_reason"].as_str() {
        Some("end_turn") => {}
        Some(other) => return Err(AiError::Call(format!("stopped: {other}"))),
        None => return Err(AiError::Call("no stop_reason".into())),
    }
    let text: String = body["content"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|b| b["type"] == "text")
        .filter_map(|b| b["text"].as_str())
        .collect();
    if text.is_empty() {
        return Err(AiError::Call("no text in the reply".into()));
    }
    Ok(text)
}

impl Model for AnthropicClient {
    async fn complete(&self, req: &ModelRequest) -> Result<String, AiError> {
        let mut attempt = 0;
        loop {
            match self.once(req).await {
                Ok(result) => return result,
                Err(Retry(after)) if attempt < self.max_retries => {
                    let backoff = Duration::from_millis(250 << attempt);
                    tokio::time::sleep(after.unwrap_or(backoff).min(Duration::from_secs(5))).await;
                    attempt += 1;
                }
                Err(_) => return Err(AiError::Call("provider unavailable".into())),
            }
        }
    }
}

/// The model a process uses: live, or saved replies (demo mode, evaluations).
pub enum AnyModel {
    Live(AnthropicClient),
    Replay(ReplayModel),
}

impl Model for AnyModel {
    async fn complete(&self, req: &ModelRequest) -> Result<String, AiError> {
        match self {
            AnyModel::Live(m) => m.complete(req).await,
            AnyModel::Replay(m) => m.complete(req).await,
        }
    }
}
