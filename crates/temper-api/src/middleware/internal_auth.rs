//! HMAC-signature gates for internal, non-JWT server-to-server callers.
//!
//! Three gates use the identical scheme with three different keys: the co-deployed
//! Authorization Server (SAML reconcile, `INTERNAL_RECONCILE_SECRET`), the Slack mention agent
//! asking what to say (link state, `SLACK_LINK_SECRET`), and the same agent asking for an
//! act-as-the-human token (mint, `SLACK_MINT_SECRET`). The last two share a *caller* but not a
//! *key*, because they differ enormously in what their key is worth: one answers a question, the
//! other confers a human's full reach. Each signs its request with
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

/// Cap on the buffered body, for BOTH gates. The payloads are small — a membership list for
/// reconcile, a single opaque principal for a link intent — and a real one of either is well
/// under a kilobyte, so 64 KiB is generous while bounding the read.
const MAX_BODY_BYTES: usize = 64 * 1024;

/// The shared gate: fresh timestamp + valid HMAC over the exact bytes received.
///
/// `secret` is passed in rather than read from state because three gates use this scheme with
/// three different keys — the co-deployed AS (`INTERNAL_RECONCILE_SECRET`), the mention agent's
/// link-state call (`SLACK_LINK_SECRET`), and its mint call (`SLACK_MINT_SECRET`). One scheme,
/// one implementation, three keys; a shared key would let any holder forge another's calls.
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
        .map_err(|_| ApiError::Unauthorized(format!("{label} body too large")))?;

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

/// Rejects the request unless it carries a fresh, valid HMAC signature over its body, keyed on
/// `SLACK_MINT_SECRET`.
///
/// **This is the highest-privilege gate in the file, and the reason it has its own key.** The
/// other two guard endpoints that *report* something — a membership reconcile, a link-state
/// answer. This one guards an endpoint that hands back an **act-as-the-human access token**: a
/// bearer that resolves to a real profile and carries that human's full reach, personal contexts
/// included. `resources_visible_to` takes a profile and nothing else, so there is no narrowing to
/// fall back on — whoever holds the minted token is, to temper, that person.
///
/// Sharing `SLACK_LINK_SECRET` here would mean that compromising the ability to ask *"is this
/// principal linked?"* also confers *"give me a token for any linked human."* The endpoint that
/// **confers reach** cannot share a key with one that merely answers a question, however
/// convenient one variable would be. Same scheme, third key.
///
/// The gate is also what makes the principal trustworthy at all: `mint_access_token` enforces no
/// authorization and mints for whatever principal it is handed, so the rule *"naming a principal
/// must not be sufficient to mint its token"* is enforced HERE, by possession of this secret, and
/// nowhere else. See `slack_mint_service` for why that cannot be a predicate in the service.
///
/// Fail-closed: no secret ⇒ endpoint disabled. Note that an instance can legitimately run with
/// the link flow configured and minting off (the secret is independent of `SlackLinkConfig`), so
/// this rejection is not evidence of misconfiguration on its own.
pub async fn require_slack_mint_signature(
    State(state): State<AppState>,
    request: Request<Body>,
    next: Next,
) -> Result<Response, ApiError> {
    let secret = match state.config.slack_mint_secret.as_deref() {
        Some(s) if !s.is_empty() => s.to_string(),
        _ => {
            tracing::warn!("slack mint: rejected (endpoint disabled — SLACK_MINT_SECRET unset)");
            return Err(ApiError::Unauthorized("slack mint disabled".to_string()));
        }
    };
    require_signature_with(&secret, "slack mint", request, next).await
}
