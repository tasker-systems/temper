//! HMAC-signature gates for internal, non-JWT server-to-server callers.
//!
//! Two principals use the identical scheme with two different keys: the co-deployed
//! Authorization Server (SAML reconcile, `INTERNAL_RECONCILE_SECRET`) and the Slack mention
//! agent (link-intent minting, `SLACK_LINK_SECRET`). Each signs its request with
//! `HMAC(secret, "{timestamp}.{raw_body}")`. The secret never crosses the wire, and a
//! captured request is replay-proof (a stale timestamp is rejected). The signing scheme +
//! the shared known-answer vector live in `temper_core::internal_sig`.
//!
//! Fail-closed: if a given gate's secret is unset, that endpoint is disabled and every
//! request to it is rejected.

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

/// The shared gate: fresh timestamp + valid HMAC over the exact bytes received.
///
/// `secret` is passed in rather than read from state because two different principals use
/// this scheme with two different keys — the co-deployed AS (`INTERNAL_RECONCILE_SECRET`)
/// and the Slack mention agent (`SLACK_LINK_SECRET`). One scheme, one implementation, two
/// keys; a shared key would let either forge the other's calls.
async fn require_signature_with(
    secret: &str,
    label: &'static str,
    request: Request<Body>,
    next: Next,
) -> Result<Response, ApiError> {
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
        tracing::warn!("{label}: rejected (missing or malformed signature headers)");
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
        tracing::warn!(timestamp, now, "{label}: rejected (stale timestamp)");
        return Err(ApiError::Unauthorized(
            "stale internal signature".to_string(),
        ));
    }

    // Buffer the body so we can MAC the exact bytes received, then hand them downstream.
    let bytes = axum::body::to_bytes(body, MAX_BODY_BYTES)
        .await
        .map_err(|_| ApiError::Unauthorized("internal reconcile body too large".to_string()))?;

    if !verify(secret.as_bytes(), timestamp, &bytes, &signature) {
        tracing::warn!("{label}: rejected (signature mismatch)");
        return Err(ApiError::Unauthorized(
            "invalid internal signature".to_string(),
        ));
    }

    // Rebuild the request with the buffered body for the handler's `Json` extractor.
    let request = Request::from_parts(parts, Body::from(bytes));
    Ok(next.run(request).await)
}

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
    require_signature_with(&secret, "internal reconcile", request, next).await
}

/// Rejects the request unless it carries a fresh, valid HMAC signature over its body, keyed
/// on `SLACK_LINK_SECRET`.
///
/// This gate is what makes Slack-side hijack expensive. Slack user ids are visible in a
/// workspace, so an open intent endpoint would let anyone mint a link URL for any user's
/// principal, bind it to their own profile, and silently receive that user's future @temper
/// writes. The gate means the URL is only ever minted in response to a real mention and
/// delivered ephemerally — the attacker must steal a message only the victim can see.
///
/// Fail-closed: no secret ⇒ endpoint disabled.
pub async fn require_slack_link_signature(
    State(state): State<AppState>,
    request: Request<Body>,
    next: Next,
) -> Result<Response, ApiError> {
    let secret = match state
        .config
        .slack_link
        .as_ref()
        .map(|c| c.hmac_secret.clone())
    {
        Some(s) if !s.is_empty() => s,
        _ => {
            tracing::warn!("slack link: rejected (endpoint disabled — SLACK_LINK_SECRET unset)");
            return Err(ApiError::Unauthorized("slack link disabled".to_string()));
        }
    };
    require_signature_with(&secret, "slack link", request, next).await
}
