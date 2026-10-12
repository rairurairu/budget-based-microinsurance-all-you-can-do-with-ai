//! Errors as the caller sees them: a status and a body naming the rule that refused the request,
//! so an app can tell the customer why (R23) and a partner can tell a refusal from a fault.
use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use microinsurance_engine::EngineError;
use microinsurance_store::StoreError;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// The body of every error response.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ErrorBody {
    /// `rule`, `state`, `not_found`, `conflict`, `unauthorised`, `forbidden`, `bad_request` or
    /// `internal`.
    pub code: String,
    /// The business rule that refused the request (`docs/rules.md`), when one did.
    pub rule: Option<String>,
    pub detail: String,
}

#[derive(Debug)]
pub struct ApiError {
    pub status: StatusCode,
    pub body: ErrorBody,
}

pub type ApiResult<T> = Result<Json<T>, ApiError>;

impl ApiError {
    fn new(status: StatusCode, code: &str, detail: impl Into<String>) -> ApiError {
        ApiError {
            status,
            body: ErrorBody {
                code: code.into(),
                rule: None,
                detail: detail.into(),
            },
        }
    }

    pub fn not_found(detail: impl Into<String>) -> ApiError {
        ApiError::new(StatusCode::NOT_FOUND, "not_found", detail)
    }

    pub fn bad_request(detail: impl Into<String>) -> ApiError {
        ApiError::new(StatusCode::BAD_REQUEST, "bad_request", detail)
    }

    pub fn unauthorised(detail: impl Into<String>) -> ApiError {
        ApiError::new(StatusCode::UNAUTHORIZED, "unauthorised", detail)
    }

    pub fn forbidden(detail: impl Into<String>) -> ApiError {
        ApiError::new(StatusCode::FORBIDDEN, "forbidden", detail)
    }

    pub fn conflict(detail: impl Into<String>) -> ApiError {
        ApiError::new(StatusCode::CONFLICT, "conflict", detail)
    }

    pub fn internal(detail: impl Into<String>) -> ApiError {
        ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, "internal", detail)
    }

    /// A request a business rule refuses.
    pub fn rule(rule: &str, detail: impl Into<String>) -> ApiError {
        let mut e = ApiError::new(StatusCode::UNPROCESSABLE_ENTITY, "rule", detail);
        e.body.rule = Some(rule.to_string());
        e
    }
}

impl From<EngineError> for ApiError {
    fn from(e: EngineError) -> ApiError {
        match e {
            EngineError::Rule { rule, detail } => ApiError::rule(rule, detail),
            EngineError::State(detail) => ApiError::new(StatusCode::CONFLICT, "state", detail),
            // At request time, an id the config does not know (a cover, a plan) came from the caller.
            EngineError::Config(detail) => ApiError::bad_request(detail),
        }
    }
}

impl From<StoreError> for ApiError {
    fn from(e: StoreError) -> ApiError {
        match e {
            StoreError::Conflict { .. } => ApiError::conflict(e.to_string()),
            other => ApiError::internal(other.to_string()),
        }
    }
}

impl From<microinsurance_ledger::LedgerError> for ApiError {
    fn from(e: microinsurance_ledger::LedgerError) -> ApiError {
        ApiError::internal(e.to_string())
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.status, Json(self.body)).into_response()
    }
}
