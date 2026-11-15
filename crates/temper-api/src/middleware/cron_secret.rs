//! The `CronSecret` tier: the gate on the Vercel cron doors (`routes::embed_internal`), applied as a
//! layer over the whole group rather than called by each handler.
//!
//! Two checks, in order, as one gate:
//!
//! 1. **Host pin.** On Vercel, the request must arrive on the deployment's own generated URL
//!    ([`ApiConfig::cron_host`](temper_services::config::ApiConfig::cron_host)), which is what Vercel
//!    Cron calls and what sits behind Vercel Authentication. Any other host (the public
//!    `<project>.vercel.app` alias, a custom domain, the UI's `/api` proxy) is refused before the
//!    bearer is read, so a holder of the secret on the public internet runs no door.
//! 2. **Bearer.** `EMBED_DISPATCH_SECRET`, compared in constant time. No secret configured ⇒ every
//!    door is *disabled*, never open: they run server-side work (a drain pass, an ONNX warmup, a
//!    retention sweep, erasure's byte deletes) fed by trust, not user auth.
//!
//! Every refusal is the uniform pre-identity refusal: the wrong host, an unset secret and a wrong
//! secret answer the same bytes as every other unauthenticated request, and the cause is logged.
//!
//! **Why a layer, not a call in each handler.** A tier layer wraps the method router's fallback and
//! every extractor (`Router::layer` maps over both). Called from the handler, the gate ran after
//! axum had already answered an unmounted method `405` with the path's `Allow`, and after `Query`
//! had already answered a malformed `?shard=` `400` — each an answer an unauthenticated caller got
//! from a door it held no secret for. As the layer, nothing on these paths runs before the gate.
//!
//! Every door shares this one gate and `EMBED_DISPATCH_SECRET` rather than introducing a second
//! secret-shaped env var: a new fail-closed variable would become a deploy-time prerequisite, the
//! same hazard that took the T3 deploy dark.

use axum::body::Body;
use axum::extract::{MatchedPath, State};
use axum::http::{HeaderMap, Request};
use axum::middleware::Next;
use axum::response::Response;

use temper_services::error::{ApiError, UnauthCause};
use temper_services::state::AppState;

/// Constant-time equality over the presented bearer token and the configured secret — avoids a
/// byte-by-byte early-exit timing oracle on the shared secret.
pub(crate) fn secret_matches(presented: &str, expected: &str) -> bool {
    let (a, b) = (presented.as_bytes(), expected.as_bytes());
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

/// Whether this request arrived on the one host the cron doors answer on.
///
/// `None` (off Vercel) pins nothing. Otherwise the request's `Host`, without a port and any
/// trailing dot, must equal the deployment's own generated host. The comparison is not
/// constant-time because the host is not a secret: it is the deployment URL Vercel prints in
/// every build log, and it is reachable only through Vercel Authentication.
fn arrived_on_cron_host(cron_host: Option<&str>, headers: &HeaderMap) -> bool {
    let Some(expected) = cron_host else {
        return true;
    };
    let Some(host) = headers
        .get(axum::http::header::HOST)
        .and_then(|v| v.to_str().ok())
    else {
        return false;
    };
    let host = host.split(':').next().unwrap_or(host);
    let host = host.strip_suffix('.').unwrap_or(host);
    host.eq_ignore_ascii_case(expected)
}

/// The `CronSecret` tier's gate. See the module doc.
pub async fn require_cron_secret(
    State(state): State<AppState>,
    request: Request<Body>,
    next: Next,
) -> Result<Response, ApiError> {
    // The door, for the log line: the matched route template, never the URI (a deployment may
    // reach the function through a rewrite). Not a secret: the cron paths are in `vercel.json`.
    let door = request
        .extensions()
        .get::<MatchedPath>()
        .map(|p| p.as_str().to_owned())
        .unwrap_or_default();
    check(&state, request.headers(), &door)?;
    Ok(next.run(request).await)
}

fn check(state: &AppState, headers: &HeaderMap, door: &str) -> Result<(), ApiError> {
    if !arrived_on_cron_host(state.config.cron_host.as_deref(), headers) {
        // Both hosts are logged so a misfiring pin is diagnosable from the log alone. Neither is a
        // secret. The received one is caller-supplied, so it is Debug-quoted and truncated.
        let received: String = headers
            .get(axum::http::header::HOST)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .chars()
            .take(255)
            .collect();
        tracing::warn!(
            door,
            host = ?received,
            expected = ?state.config.cron_host,
            "cron door: rejected (request did not arrive on the deployment's cron host)"
        );
        return Err(ApiError::Unauthenticated(UnauthCause::CronWrongHost));
    }
    let expected = match state.config.embed_dispatch_secret.as_deref() {
        Some(s) if !s.is_empty() => s,
        _ => {
            tracing::warn!(
                door,
                "cron door: rejected (endpoint disabled — secret unset)"
            );
            return Err(ApiError::Unauthenticated(UnauthCause::CronDisabled));
        }
    };
    let presented = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .unwrap_or("");
    if !secret_matches(presented, expected) {
        tracing::warn!(door, "cron door: rejected (bad or missing bearer secret)");
        return Err(ApiError::Unauthenticated(UnauthCause::CronBadSecret));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The shared cron gate compares the presented bearer against the configured secret in constant
    /// time. It must accept ONLY an exact match and reject on any length or content difference — a
    /// truthy-on-prefix or truthy-on-empty bug here would open the ONNX/drain endpoints to anyone.
    #[test]
    fn secret_matches_is_true_only_on_exact_equality() {
        assert!(secret_matches("s3cret-value", "s3cret-value"));
        assert!(!secret_matches("s3cret-value", "s3cret-valuE")); // one byte differs
        assert!(!secret_matches("s3cret-valu", "s3cret-value")); // presented is a prefix
        assert!(!secret_matches("s3cret-valuex", "s3cret-value")); // presented is longer
        assert!(!secret_matches("", "s3cret-value")); // missing bearer vs a real secret

        // Two empty strings compare equal here, but the gate rejects an empty CONFIGURED secret
        // before ever calling this (the `!s.is_empty()` guard in `check`), so an
        // unconfigured deploy still fails closed.
        assert!(secret_matches("", ""));
    }

    fn host(value: &str) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(
            axum::http::header::HOST,
            value.parse().expect("valid header value"),
        );
        headers
    }

    /// The cron doors answer only on the deployment's own generated host. The public alias and a
    /// custom domain are the two ways the internet reaches the same function; both must miss, as
    /// must a request with no `Host` at all, and a lookalike that merely contains the pinned host.
    #[test]
    fn the_cron_host_pin_admits_only_the_deployment_url() {
        let pinned = Some("temper-cloud-a1b2c3d4e-team.vercel.app");

        assert!(arrived_on_cron_host(
            pinned,
            &host("temper-cloud-a1b2c3d4e-team.vercel.app")
        ));
        assert!(arrived_on_cron_host(
            pinned,
            &host("Temper-Cloud-a1b2c3d4e-Team.vercel.app:443")
        ));
        assert!(arrived_on_cron_host(
            pinned,
            &host("temper-cloud-a1b2c3d4e-team.vercel.app.")
        ));

        assert!(!arrived_on_cron_host(
            pinned,
            &host("temper-cloud.vercel.app")
        ));
        assert!(!arrived_on_cron_host(pinned, &host("temperkb.io")));
        assert!(!arrived_on_cron_host(
            pinned,
            &host("temper-cloud-a1b2c3d4e-team.vercel.app.evil.example")
        ));
        assert!(!arrived_on_cron_host(pinned, &HeaderMap::new()));
    }

    /// Off Vercel there is no deployment URL to pin to, and the bearer alone gates, as before.
    #[test]
    fn no_cron_host_pins_nothing() {
        assert!(arrived_on_cron_host(None, &host("anything.example")));
        assert!(arrived_on_cron_host(None, &HeaderMap::new()));
    }
}
