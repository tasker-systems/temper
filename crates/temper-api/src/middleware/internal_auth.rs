//! HMAC-signature gates for internal, non-JWT server-to-server callers.
//!
//! Three gates use the identical scheme with three different keys: the co-deployed
//! Authorization Server (SAML reconcile, `INTERNAL_RECONCILE_SECRET`), the Slack mention agent
//! asking what to say (link state, `SLACK_LINK_SECRET`), and the same agent asking for an
//! act-as-the-human token (mint, `SLACK_MINT_SECRET`). The last two share a *caller* but not a
//! *key*, because they differ enormously in what their key is worth: one answers a question, the
//! other confers a human's full reach. Each signs its request with the v2 construction,
//! `HMAC(secret, "v2\n{METHOD}\n{door}\n{timestamp}\n{nonce}\n{raw_body}")`. The secret never
//! crosses the wire; the signature is good only for the route it names; and a request is accepted
//! once, because its nonce is recorded (`internal_sig_service`). The construction + the shared
//! known-answer vector live in `temper_core::internal_sig`.
//!
//! The original `X-Temper-Signature` header is ignored. Signers still send it, for verifiers that
//! predate v2, but a verifier that accepted it would reopen the replay and cross-door gaps to anyone
//! who stripped the v2 headers from a capture.
//!
//! Fail-closed: if a given gate's secret is unset, that endpoint is disabled and every
//! request to it is rejected.

use std::time::{SystemTime, UNIX_EPOCH};

use axum::body::{Body, Bytes};
use axum::extract::{MatchedPath, State};
use axum::http::request::Parts;
use axum::http::Request;
use axum::middleware::Next;
use axum::response::Response;

use temper_core::internal_sig::{DOORS, NONCE_HEADER, SIGNATURE_V2_HEADER, TIMESTAMP_HEADER};
use temper_services::error::ApiError;
use temper_services::services::internal_sig_service::{self, SignedRequest};
use temper_services::state::AppState;

/// Cap on the buffered body, for BOTH gates. The payloads are small — a membership list for
/// reconcile, a single opaque principal for a link intent — and a real one of either is well
/// under a kilobyte, so 64 KiB is generous while bounding the read.
const MAX_BODY_BYTES: usize = 64 * 1024;

/// The current unix time in seconds, saturating to 0 before the epoch (unreachable in practice).
fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// A request's signature headers and buffered body, before any verdict.
struct Buffered {
    parts: Parts,
    method: String,
    door: String,
    timestamp: i64,
    nonce: String,
    signature: String,
    body: Bytes,
}

impl Buffered {
    fn signed(&self) -> SignedRequest<'_> {
        SignedRequest {
            method: &self.method,
            door: &self.door,
            timestamp: self.timestamp,
            nonce: &self.nonce,
            signature: &self.signature,
            body: &self.body,
        }
    }

    /// Rebuild the request with the buffered body for the handler's extractor.
    fn into_request(self) -> Request<Body> {
        Request::from_parts(self.parts, Body::from(self.body))
    }
}

/// Pull the v2 signature headers, name the door, and buffer the body — the transport plumbing every
/// signature gate shares. It does **not** verify: each gate verifies how it must.
/// `require_signature_with` accepts the request through `internal_sig_service::accept_once` for the
/// report gates; the mint gate hands it to `verify_mint_request`, which verifies AND seals a proof
/// inside temper-services — the split that makes that proof un-forgeable in this crate (spec §5.1).
///
/// The door is the route the request matched, never the URI it arrived on: a deployment may reach
/// the API through a prefix or a rewrite, and the route is the one name the signer also holds.
/// A matched route that is not one of `internal_sig::DOORS` is refused rather than verified: a route
/// mounted under a signature tier without a door of its own must fail closed, not accept a
/// signature over whatever path it happens to have.
async fn buffer_signed_request(request: Request<Body>, label: &str) -> Result<Buffered, ApiError> {
    let (parts, body) = request.into_parts();
    let header = |name: &str| {
        parts
            .headers
            .get(name)
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned)
    };
    let timestamp = header(TIMESTAMP_HEADER).and_then(|s| s.parse::<i64>().ok());
    let nonce = header(NONCE_HEADER);
    let signature = header(SIGNATURE_V2_HEADER);
    let door = parts
        .extensions
        .get::<MatchedPath>()
        .map(|p| p.as_str().to_owned());

    let (Some(timestamp), Some(nonce), Some(signature), Some(door)) =
        (timestamp, nonce, signature, door)
    else {
        tracing::warn!(reason = "missing_headers", "{label}: rejected");
        return Err(ApiError::Unauthorized(
            "invalid internal signature".to_string(),
        ));
    };

    if !DOORS.contains(&door.as_str()) {
        tracing::error!(door, reason = "unknown_door", "{label}: rejected");
        return Err(ApiError::Unauthorized(
            "invalid internal signature".to_string(),
        ));
    }

    let body = axum::body::to_bytes(body, MAX_BODY_BYTES)
        .await
        .map_err(|_| ApiError::Unauthorized(format!("{label} body too large")))?;

    Ok(Buffered {
        method: parts.method.as_str().to_owned(),
        parts,
        door,
        timestamp,
        nonce,
        signature,
        body,
    })
}

/// The shared gate for the signature surfaces that guard a *report*: a v2 signature over the exact
/// bytes received, for this route, accepted once.
///
/// `secret` is passed in rather than read from state because these gates use one scheme with
/// different keys — the co-deployed AS (`INTERNAL_RECONCILE_SECRET`) and the mention agent's
/// link-state call (`SLACK_LINK_SECRET`). A shared key would let any holder forge another's calls.
/// The **mint** gate deliberately does NOT use this helper: it seals a proof via
/// `verify_mint_request` instead (see `require_slack_mint_signature`), which is why no proof can be
/// minted behind this path.
async fn require_signature_with(
    state: &AppState,
    secret: &str,
    label: &'static str,
    request: Request<Body>,
    next: Next,
) -> Result<Response, ApiError> {
    let buffered = buffer_signed_request(request, label).await?;
    internal_sig_service::accept_once(
        &state.pool,
        secret.as_bytes(),
        &buffered.signed(),
        now_secs(),
        label,
    )
    .await?;

    // The signature verified against a secret only our own services hold, which is the strongest
    // form of "one of ours" temper has — so join the caller's trace by link (decision `019f95ff`
    // rule 2). These are the hops the whole goal exists to make navigable: the mention agent asking
    // what to say, then asking for a token, are two of the three spans in that story.
    temper_telemetry::link_trusted_caller(&tracing::Span::current(), &buffered.parts.headers);

    Ok(next.run(buffered.into_request()).await)
}

/// Rejects the request unless it carries a fresh v2 signature over its body, for this route, not
/// accepted before.
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
    require_signature_with(&state, &secret, "internal reconcile", request, next).await
}

/// Rejects the request unless it carries a fresh v2 signature over its body, for this route, not
/// accepted before, keyed
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
    require_signature_with(&state, &secret, "slack link", request, next).await
}

/// Rejects the request unless it carries a fresh v2 signature over its body, for this route, not
/// accepted before, keyed on
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

    let buffered = buffer_signed_request(request, "slack mint").await?;

    // Verify AND seal in temper-services — the ONLY place a `VerifiedSlackPrincipal` is constructed,
    // so this gate is the only one that can produce one, and it can only produce one for a request
    // that verifies against the mint secret. The verify lives THERE, not here, which is what makes
    // the proof un-forgeable by temper-api (spec §5.1). A malformed principal surfaces as a 400, a
    // bad signature, stale timestamp or replay as a 401 — all before the handler runs.
    let verified = temper_services::services::slack_mint_service::verify_mint_request(
        &state.pool,
        &secret,
        &buffered.signed(),
        now_secs(),
    )
    .await?;
    // Verified against the mint secret — one of ours, so link its trace (decision `019f95ff`
    // rule 2). Spelled out here rather than inherited from `require_signature_with` because this
    // gate deliberately does not use that helper; see the doc comment above.
    temper_telemetry::link_trusted_caller(&tracing::Span::current(), &buffered.parts.headers);
    let mut request = buffered.into_request();
    request.extensions_mut().insert(verified);
    Ok(next.run(request).await)
}
