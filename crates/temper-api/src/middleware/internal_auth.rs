//! HMAC-signature gate for the internal SAML reconcile endpoint (AS -> temper-api).
//! Not a JWT path: the caller is the co-deployed Authorization Server, which signs each
//! request with `HMAC(secret, "{timestamp}.{raw_body}")` over the shared
//! `INTERNAL_RECONCILE_SECRET`. The secret never crosses the wire, and a captured request
//! is replay-proof (a stale timestamp is rejected). The signing scheme + the shared
//! known-answer vector live in `temper_core::internal_sig`.
//!
//! Fail-closed: if the secret is unset the endpoint is disabled and every request is
//! rejected.

use std::time::{SystemTime, UNIX_EPOCH};

use axum::body::Body;
use axum::extract::State;
use axum::http::Request;
use axum::middleware::Next;
use axum::response::Response;

use temper_core::internal_sig::{timestamp_is_fresh, verify, SIGNATURE_HEADER, TIMESTAMP_HEADER};
use temper_services::error::ApiError;
use temper_services::state::AppState;

/// Cap on the buffered reconcile body. The payload is a small membership list; a real one
/// is well under a kilobyte, so 64 KiB is generous while bounding the read.
const MAX_BODY_BYTES: usize = 64 * 1024;

/// Rejects the request unless it carries a fresh, valid HMAC signature over its body.
pub async fn require_internal_signature(
    State(state): State<AppState>,
    request: Request<Body>,
    next: Next,
) -> Result<Response, ApiError> {
    // Fail-closed when unconfigured: no secret ⇒ endpoint disabled.
    let secret = match state.config.internal_reconcile_secret.as_deref() {
        Some(s) if !s.is_empty() => s.to_string(),
        _ => {
            tracing::warn!("internal reconcile: rejected (endpoint disabled — secret unset)");
            return Err(ApiError::Unauthorized(
                "internal reconcile disabled".to_string(),
            ));
        }
    };

    // Pull the signature headers before consuming the body.
    let (parts, body) = request.into_parts();
    let timestamp = parts
        .headers
        .get(TIMESTAMP_HEADER)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.parse::<i64>().ok());
    let signature = parts
        .headers
        .get(SIGNATURE_HEADER)
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);

    let (Some(timestamp), Some(signature)) = (timestamp, signature) else {
        tracing::warn!("internal reconcile: rejected (missing or malformed signature headers)");
        return Err(ApiError::Unauthorized(
            "invalid internal signature".to_string(),
        ));
    };

    // Reject replays: the timestamp must be within the allowed skew of now.
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    if !timestamp_is_fresh(timestamp, now) {
        tracing::warn!(
            timestamp,
            now,
            "internal reconcile: rejected (stale timestamp)"
        );
        return Err(ApiError::Unauthorized(
            "stale internal signature".to_string(),
        ));
    }

    // Buffer the body so we can MAC the exact bytes received, then hand them downstream.
    let bytes = axum::body::to_bytes(body, MAX_BODY_BYTES)
        .await
        .map_err(|_| ApiError::Unauthorized("internal reconcile body too large".to_string()))?;

    if !verify(secret.as_bytes(), timestamp, &bytes, &signature) {
        tracing::warn!("internal reconcile: rejected (signature mismatch)");
        return Err(ApiError::Unauthorized(
            "invalid internal signature".to_string(),
        ));
    }

    // Rebuild the request with the buffered body for the handler's `Json` extractor.
    let request = Request::from_parts(parts, Body::from(bytes));
    Ok(next.run(request).await)
}
