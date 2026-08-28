//! Operator identity, and the JSON body extractor (ADR 0001 D1).
//!
//! There is no login and there will not be one. Each tablet takes a name the first time it
//! opens `/operator` or `/checkin` -- "FRONT DESK TABLET" -- keeps it in local storage, and
//! sends it with every write. That name is the `operator` column of the audit trail
//! (CLAUDE.md 20).
//!
//! The rule this module exists to enforce: a write with no name is **rejected**. Defaulting
//! to an empty string would produce an audit row that looks like a record of who did
//! something and is not one, and the whole point of D1's trade -- device-level traceability
//! instead of personal -- is that the device name is actually there.
//!
//! Traceability is to a device, not to a person. If that ever has to change, this extractor
//! and the `operator` field become an identity without the audit shape changing (D1).

use crate::error::ApiError;
use axum::extract::{FromRequest, FromRequestParts, Request};
use axum::http::request::Parts;
use serde::de::DeserializeOwned;

/// Where the device name travels. A header rather than a body field so it applies
/// identically to every write, including ones whose body is a domain document
/// (a course, a reader registration) that should not have an operator field welded into it.
pub const OPERATOR_HEADER: &str = "x-operator-device";

/// The audit identity of whoever is making this write.
///
/// Present in the signature of every mutating handler, which is also how a reviewer can see
/// at a glance that no write path skips it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OperatorDevice(pub String);

impl<S: Send + Sync> FromRequestParts<S> for OperatorDevice {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, ApiError> {
        // Decode the raw bytes as UTF-8 rather than using `to_str`, which rejects anything
        // outside visible ASCII. ADR 0001 D1 makes the device's own name the audit identity,
        // and this gym names its tablets in Chinese -- 「櫃檯平板」 would otherwise read as
        // no operator at all, which is exactly the anonymous audit row D1 exists to prevent.
        let name = parts
            .headers
            .get(OPERATOR_HEADER)
            .and_then(|value| std::str::from_utf8(value.as_bytes()).ok())
            .map(str::trim)
            .unwrap_or_default();
        // Whitespace is not a name. A header of `"   "` would satisfy "present" and tell a
        // later reader of the audit trail exactly as much as no header at all.
        if name.is_empty() {
            return Err(ApiError::operator_required());
        }
        Ok(Self(name.to_string()))
    }
}

/// A JSON body that fails with this API's error shape rather than axum's.
///
/// An empty body is read as `{}`. Several writes -- arming, closing, returning to draft --
/// have nothing to say beyond the intent, and a 400 for a `fetch` that sent no body would
/// be a confusing answer to a well-formed request. A field that is genuinely required is
/// still required: `{}` fails to deserialise into a type that has one, and says which.
pub struct Body<T>(pub T);

impl<T, S> FromRequest<S> for Body<T>
where
    T: DeserializeOwned,
    S: Send + Sync,
{
    type Rejection = ApiError;

    async fn from_request(req: Request, state: &S) -> Result<Self, ApiError> {
        let bytes = axum::body::Bytes::from_request(req, state)
            .await
            .map_err(|e| ApiError::invalid_body(e.to_string()))?;
        let slice: &[u8] = if bytes.is_empty() { b"{}" } else { &bytes };
        serde_json::from_slice(slice)
            .map(Body)
            .map_err(|e| ApiError::invalid_body(e.to_string()))
    }
}
