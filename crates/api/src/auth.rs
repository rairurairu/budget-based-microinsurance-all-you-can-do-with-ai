//! Who is calling (ADR-011). Every route except the public ones takes a bearer token naming one
//! principal: a customer, a distribution partner's server, a named person at the insurer, or a
//! data source pushing its reports. A token is `v1.<claims>.<signature>`: the claims in base64url
//! JSON, signed with HMAC-SHA-256 under the API's secret, so the API checks it without a lookup.
//!
//! What a principal may do is fixed by its kind, never by the request: a customer acts only on
//! their own records; a partner sees only aggregates for itself (R21); the person deciding a work
//! item is the staff member the token names, so nobody can decide in another's name and a decline
//! is reviewed by someone else (R15). In the embedded journey the partner's server exchanges its
//! own token and its reference for the customer for a short-lived customer token (token
//! exchange): the partner app never holds the customer's credentials with the insurer, and the
//! customer's id is the insurer's, not the partner's.
use crate::error::ApiError;
use crate::state::AppState;
use axum::extract::FromRequestParts;
use axum::http::header::AUTHORIZATION;
use axum::http::request::Parts;
use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use chrono::{DateTime, Utc};
use hmac::{Hmac, Mac};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::sync::Arc;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Principal {
    /// A customer, on a partner's channel or the insurer's own journey (`partner_id` none).
    Customer {
        customer_id: String,
        partner_id: Option<String>,
    },
    /// A distribution partner's server (R21, R52).
    Partner { partner_id: String },
    /// A named person at the insurer: the reviewer on every decision they make (ADR-010).
    Staff { staff_id: String },
    /// A data source pushing its reports (R55).
    Feed { source: String },
}

#[derive(Serialize, Deserialize)]
struct Claims {
    sub: Principal,
    /// Expiry, seconds since the Unix epoch.
    exp: i64,
}

/// The API's signing secret.
#[derive(Clone)]
pub struct TokenKeys {
    secret: Arc<Vec<u8>>,
}

const VERSION: &str = "v1";

impl TokenKeys {
    pub fn new(secret: &[u8]) -> TokenKeys {
        TokenKeys {
            secret: Arc::new(secret.to_vec()),
        }
    }

    fn sign(&self, payload: &str) -> String {
        let mut mac = Hmac::<Sha256>::new_from_slice(&self.secret).expect("any key length");
        mac.update(VERSION.as_bytes());
        mac.update(b".");
        mac.update(payload.as_bytes());
        URL_SAFE_NO_PAD.encode(mac.finalize().into_bytes())
    }

    pub fn issue(&self, principal: &Principal, expires: DateTime<Utc>) -> String {
        let claims = Claims {
            sub: principal.clone(),
            exp: expires.timestamp(),
        };
        let json = serde_json::to_vec(&claims).unwrap_or_default();
        let payload = URL_SAFE_NO_PAD.encode(json);
        let signature = self.sign(&payload);
        format!("{VERSION}.{payload}.{signature}")
    }

    pub fn verify(&self, token: &str, now: DateTime<Utc>) -> Result<Principal, ApiError> {
        let bad = || ApiError::unauthorised("the token is not valid");
        let mut parts = token.split('.');
        let (Some(VERSION), Some(payload), Some(signature), None) =
            (parts.next(), parts.next(), parts.next(), parts.next())
        else {
            return Err(bad());
        };
        let mut mac = Hmac::<Sha256>::new_from_slice(&self.secret).map_err(|_| bad())?;
        mac.update(VERSION.as_bytes());
        mac.update(b".");
        mac.update(payload.as_bytes());
        let given = URL_SAFE_NO_PAD.decode(signature).map_err(|_| bad())?;
        mac.verify_slice(&given).map_err(|_| bad())?;
        let json = URL_SAFE_NO_PAD.decode(payload).map_err(|_| bad())?;
        let claims: Claims = serde_json::from_slice(&json).map_err(|_| bad())?;
        if claims.exp <= now.timestamp() {
            return Err(ApiError::unauthorised("the token has expired"));
        }
        Ok(claims.sub)
    }
}

/// The insurer's customer id for a partner's own customer reference: stable for the same
/// partner and reference, and telling the partner nothing it did not already know.
pub fn customer_id_for(partner_id: &str, customer_ref: &str) -> String {
    let digest = Sha256::digest(format!("{partner_id}\u{1f}{customer_ref}").as_bytes());
    let hex: String = digest[..8].iter().map(|b| format!("{b:02x}")).collect();
    format!("cust-{hex}")
}

fn bearer(parts: &Parts) -> Option<&str> {
    parts
        .headers
        .get(AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(str::trim)
}

/// The caller, any kind.
pub struct Caller(pub Principal);

impl FromRequestParts<AppState> for Caller {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, ApiError> {
        let token =
            bearer(parts).ok_or_else(|| ApiError::unauthorised("a bearer token is needed"))?;
        state.keys.verify(token, state.clock.now()).map(Caller)
    }
}

/// A customer acting on their own records.
pub struct CustomerCaller {
    pub customer_id: String,
    /// The channel they came through, when a partner's (R52).
    pub partner_id: Option<String>,
}

impl FromRequestParts<AppState> for CustomerCaller {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, ApiError> {
        match Caller::from_request_parts(parts, state).await?.0 {
            Principal::Customer {
                customer_id,
                partner_id,
            } => Ok(CustomerCaller {
                customer_id,
                partner_id,
            }),
            _ => Err(ApiError::forbidden("customers only")),
        }
    }
}

/// A distribution partner's server, acting for itself.
pub struct PartnerCaller {
    pub partner_id: String,
}

impl FromRequestParts<AppState> for PartnerCaller {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, ApiError> {
        match Caller::from_request_parts(parts, state).await?.0 {
            Principal::Partner { partner_id } => Ok(PartnerCaller { partner_id }),
            _ => Err(ApiError::forbidden("distribution partners only")),
        }
    }
}

/// A named person at the insurer.
pub struct StaffCaller {
    pub staff_id: String,
}

impl FromRequestParts<AppState> for StaffCaller {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, ApiError> {
        match Caller::from_request_parts(parts, state).await?.0 {
            Principal::Staff { staff_id } if !staff_id.trim().is_empty() => {
                Ok(StaffCaller { staff_id })
            }
            _ => Err(ApiError::forbidden("the insurer's people only")),
        }
    }
}

/// A data source pushing its own reports.
pub struct FeedCaller {
    pub source: String,
}

impl FromRequestParts<AppState> for FeedCaller {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, ApiError> {
        match Caller::from_request_parts(parts, state).await?.0 {
            Principal::Feed { source } => Ok(FeedCaller { source }),
            _ => Err(ApiError::forbidden("registered data sources only")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(s: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(s)
            .expect("time")
            .with_timezone(&Utc)
    }

    #[test]
    fn a_token_names_its_principal_until_it_expires() {
        let keys = TokenKeys::new(b"secret");
        let p = Principal::Staff {
            staff_id: "handler-amy".into(),
        };
        let token = keys.issue(&p, at("2026-11-18T08:00:00Z"));
        assert_eq!(
            keys.verify(&token, at("2026-11-18T07:59:59Z")).ok(),
            Some(p)
        );
        assert!(keys.verify(&token, at("2026-11-18T08:00:00Z")).is_err());
    }

    #[test]
    fn a_changed_token_or_another_key_is_refused() {
        let keys = TokenKeys::new(b"secret");
        let token = keys.issue(
            &Principal::Staff {
                staff_id: "handler-amy".into(),
            },
            at("2027-01-01T00:00:00Z"),
        );
        let now = at("2026-11-18T07:00:00Z");
        assert!(TokenKeys::new(b"other").verify(&token, now).is_err());
        // Claim to be someone else with the same signature.
        let forged = keys.issue(
            &Principal::Staff {
                staff_id: "handler-ben".into(),
            },
            at("2027-01-01T00:00:00Z"),
        );
        let mixed = format!(
            "{}.{}",
            forged.rsplit_once('.').map_or("", |(head, _)| head),
            token.rsplit_once('.').map_or("", |(_, sig)| sig)
        );
        assert!(keys.verify(&mixed, now).is_err());
        assert!(keys.verify("v1.e30", now).is_err());
    }

    #[test]
    fn the_insurers_customer_id_is_stable_per_partner() {
        let a = customer_id_for("delivery_app", "rider-77");
        assert_eq!(a, customer_id_for("delivery_app", "rider-77"));
        assert_ne!(a, customer_id_for("ticketing_app", "rider-77"));
        assert!(!a.contains("rider-77"));
    }
}
