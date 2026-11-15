//! The webhook intake transport — no temper token, gated by the **`BrokerAttested` tier** on the
//! broker attestation, for the same reason `embed_internal_routes` has its own gate: the caller is
//! not a temper principal and holds no temper token. Vercel Connect forwards a remote system's
//! event carrying an RS256 attestation, and the tier's layer verifies it (signature, issuer,
//! audience, the anti-decoy `client_id`, and the signed `trigger` claim) and resolves the
//! connection it names before the handler, or anything that reads the body, runs.
//!
//! A router of its own rather than a route on `embed_internal_routes` because the controls are
//! not the same control. That group's tier checks a shared secret temper issues; this one verifies
//! a third party's signature against a remote JWKS. Folding them together would put a route whose
//! gate is a bearer comparison in the same reviewed set as one whose gate is a JWKS verification,
//! and the tripwire's whole job is that each entry names the control it actually carries.
//!
//! **`create_app` only.** Connect forwards to the public host; `create_internal_app` exists solely
//! to give Vercel crons a longer `maxDuration` and serves no third-party surface. Same reasoning
//! as `slack_link_public_routes`.
//!
//! The body limit is set here and — until `query_routes` `[2026-08-28]` — nowhere else. Nothing in
//! temper-api set `DefaultBodyLimit` at the time, so axum's 2 MB default applied — under GitHub's 25 MB ceiling, above which GitHub *silently drops*
//! the delivery. A payload between the two was refused by temper while GitHub believed it had
//! delivered. Scoped to this route rather than raised globally: no other endpoint has a reason to
//! accept a 25 MB body, and a global raise would hand every one of them the same exposure. Note it
//! bounds the body axum's extractor sees, which is *after* the app-wide
//! `RequestDecompressionLayer` — so it bounds decompressed bytes, which is the direction that
//! matters.
//!
//! Excluded from the OpenAPI contract entirely: the caller is Connect, which reads no spec of ours.

use axum::extract::DefaultBodyLimit;
use axum::routing::post;
use axum::Router;

use crate::handlers;
use temper_services::state::AppState;

pub(super) fn webhook_intake_routes() -> Router<AppState> {
    Router::new()
        .route(
            "/api/intake/webhook",
            post(handlers::webhook_intake::receive),
        )
        .layer(DefaultBodyLimit::max(GITHUB_MAX_WEBHOOK_BYTES))
}

/// GitHub's documented webhook payload ceiling. Above this GitHub does not deliver at all, so
/// accepting more would buy nothing while widening what one request can make temper hold in
/// memory.
const GITHUB_MAX_WEBHOOK_BYTES: usize = 25 * 1024 * 1024;
