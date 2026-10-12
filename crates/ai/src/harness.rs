//! The fixed harness around every model call: protected attributes stripped (R19), output
//! parsed into a typed recommendation and checked, a timeout, and a saved fallback for anything
//! else (R22). Every component goes through `Harness::ask`; none calls a model directly.
use crate::client::{Image, Model, ModelRequest};
use crate::language::Languages;
use crate::prompts::Prompts;
use crate::schema;
use crate::settings::{AiSettings, Tier};
use schemars::JsonSchema;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use std::future::Future;
use std::time::Duration;

pub const PROTECTED_FIELDS: &[&str] = &[
    "age",
    "gender",
    "ethnicity",
    "district",
    "religion",
    "nationality",
];

#[derive(Debug, thiserror::Error)]
pub enum AiError {
    #[error("model call failed: {0}")]
    Call(String),
    #[error("output did not match the schema: {0}")]
    Schema(String),
    #[error("output failed a check: {0}")]
    Check(String),
}

/// Where a recommendation came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Source {
    /// The model's reply, parsed and checked.
    Model,
    /// The saved answer: the call was slow, failed, or its reply was invalid (R22).
    Fallback,
    /// A fixed rule decided without a model call (a parametric cover has no prevention task, a
    /// reused photo is flagged).
    Rule,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Harnessed<T> {
    pub value: T,
    pub source: Source,
    /// Why the fallback or rule was used, for the decision log.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

impl<T> Harnessed<T> {
    pub fn rule(value: T, reason: impl Into<String>) -> Harnessed<T> {
        Harnessed {
            value,
            source: Source::Rule,
            reason: Some(reason.into()),
        }
    }

    pub fn map<U>(self, f: impl FnOnce(T) -> U) -> Harnessed<U> {
        Harnessed {
            value: f(self.value),
            source: self.source,
            reason: self.reason,
        }
    }
}

/// The schema check: model text either parses into `T` or is rejected.
pub fn parse<T: DeserializeOwned>(text: &str) -> Result<T, AiError> {
    serde_json::from_str(text).map_err(|e| AiError::Schema(e.to_string()))
}

/// Run a model call with a timeout. On timeout, error or invalid output, use the fallback.
pub async fn run<T, Fut>(call: Fut, timeout: Duration, fallback: impl FnOnce() -> T) -> Harnessed<T>
where
    Fut: Future<Output = Result<T, AiError>>,
{
    match tokio::time::timeout(timeout, call).await {
        Ok(Ok(value)) => Harnessed {
            value,
            source: Source::Model,
            reason: None,
        },
        Ok(Err(e)) => Harnessed {
            value: fallback(),
            source: Source::Fallback,
            reason: Some(e.to_string()),
        },
        Err(_) => Harnessed {
            value: fallback(),
            source: Source::Fallback,
            reason: Some(format!("timed out after {} ms", timeout.as_millis())),
        },
    }
}

/// Remove protected attributes, at any depth, before input reaches a model.
pub fn strip_protected(input: &mut serde_json::Value) {
    match input {
        serde_json::Value::Object(map) => {
            for field in PROTECTED_FIELDS {
                map.remove(*field);
            }
            for v in map.values_mut() {
                strip_protected(v);
            }
        }
        serde_json::Value::Array(items) => {
            for v in items {
                strip_protected(v);
            }
        }
        _ => {}
    }
}

/// One call: what to ask, of which model, and the input.
pub struct Ask<'a> {
    /// The component, which names its prompt file (`prompts/<component>.md`).
    pub component: &'static str,
    pub tier: Tier,
    pub input: serde_json::Value,
    pub images: &'a [Image],
}

/// The model, the prompts, the settings and the language packs every component shares.
pub struct Harness<M> {
    pub model: M,
    pub prompts: Prompts,
    pub settings: AiSettings,
    pub languages: Languages,
}

impl<M: Model> Harness<M> {
    pub fn new(
        model: M,
        prompts: Prompts,
        settings: AiSettings,
        languages: Languages,
    ) -> Harness<M> {
        Harness {
            model,
            prompts,
            settings,
            languages,
        }
    }

    /// The request a call sends: the prompt, and the input with protected attributes removed.
    pub fn request<R: JsonSchema>(&self, ask: &Ask<'_>) -> Result<ModelRequest, AiError> {
        let mut input = ask.input.clone();
        strip_protected(&mut input);
        let system = self
            .prompts
            .load(ask.component)
            .map_err(|e| AiError::Call(format!("prompt {}: {e}", ask.component)))?;
        Ok(ModelRequest {
            component: ask.component,
            tier: ask.tier,
            system,
            user: serde_json::to_string(&input).map_err(|e| AiError::Call(e.to_string()))?,
            images: ask.images.to_vec(),
            schema: schema::output_schema::<R>(),
        })
    }

    /// Asks the model for a reply of type `R`, turns it into the recommendation with `check`
    /// (which rejects anything the rules forbid), and falls back to `fallback` on any failure
    /// or after the tier's timeout (R22).
    pub async fn ask<R, T>(
        &self,
        ask: Ask<'_>,
        check: impl FnOnce(R) -> Result<T, String>,
        fallback: impl FnOnce() -> T,
    ) -> Harnessed<T>
    where
        R: DeserializeOwned + JsonSchema,
    {
        let timeout = self.settings.timeout(ask.tier);
        let call = async {
            let req = self.request::<R>(&ask)?;
            let text = self.model.complete(&req).await?;
            let reply: R = parse(&text)?;
            check(reply).map_err(AiError::Check)
        };
        run(call, timeout, fallback).await
    }
}
