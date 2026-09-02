//! The transport layers every HTTP surface applies below its own root span.
//!
//! Companion to [`crate::cors`], and here for the same reason: temper-api and temper-mcp each
//! assemble their own router, so anything expressed twice is something one of them can be built
//! without. That is not hypothetical — `RequestDecompressionLayer` and the fallback handler were
//! both applied by `apply_transport_layers` (shared by the public and internal API apps) and by
//! neither of MCP's stacks, so on the agent-facing surface a gzip body reached the JSON extractor
//! still compressed, and an unmatched path was answered by the auth middleware rather than by a
//! 404 of any shape.
//!
//! The root span stays per-surface deliberately: temper-api names it `http_request` and temper-mcp
//! names it `mcp_request`, because three different things under one name is unreadable once they
//! are exported together.

use axum::extract::DefaultBodyLimit;
use axum::http::header::{self, HeaderValue};
use axum::Router;
use tower_http::decompression::RequestDecompressionLayer;
use tower_http::set_header::SetResponseHeaderLayer;

/// The response headers both public HTTP surfaces set on every response.
///
/// **These are set in-app, not verified at the edge, and that is the whole point.** The hosting
/// platform may well send some of them; it can also be reconfigured, or replaced, without any
/// change to this repository. A control that only holds while something in front of the instance
/// keeps behaving is not a control this codebase can claim.
///
/// Each is applied `if_not_present`, so the baseline is a **floor a handler can deliberately
/// raise or relax for its own content**, and every relaxation is visible next to the response it
/// belongs to rather than hidden in a wider policy here. One route needs that today: temper-api's
/// Slack callback renders HTML with an inline `<style>`, which the policy below forbids.
///
/// | Header | Value | Why this value |
/// |---|---|---|
/// | `x-content-type-options` | `nosniff` | Both surfaces answer `application/json` almost everywhere. Content sniffing can only ever disagree with a `Content-Type` these surfaces set deliberately |
/// | `x-frame-options` | `DENY` | Neither surface has a page meant to be framed. The Slack callback is a terminal page, not an embed |
/// | `referrer-policy` | `no-referrer` | The Slack callback carries state in its URL. A referrer leak from it is the one path either surface has to leaking a URL to a third party |
/// | `strict-transport-security` | `max-age=63072000; includeSubDomains` | Two years, the usual floor for preload eligibility. `preload` itself is **not** sent — it is a submission to a browser-vendor list that is painful to reverse, and that is an operator's decision, not a default |
/// | `content-security-policy` | `default-src 'none'; frame-ancestors 'none'; base-uri 'none'` | A JSON API loads nothing, so the correct policy is *nothing*. `frame-ancestors` is what actually enforces the framing rule in modern browsers; `x-frame-options` is above it for the ones that never learned |
fn security_header_layers<S>(app: Router<S>) -> Router<S>
where
    S: Clone + Send + Sync + 'static,
{
    const NOSNIFF: HeaderValue = HeaderValue::from_static("nosniff");
    const DENY: HeaderValue = HeaderValue::from_static("DENY");
    const NO_REFERRER: HeaderValue = HeaderValue::from_static("no-referrer");
    const HSTS: HeaderValue = HeaderValue::from_static("max-age=63072000; includeSubDomains");

    app.layer(SetResponseHeaderLayer::if_not_present(
        header::X_CONTENT_TYPE_OPTIONS,
        NOSNIFF,
    ))
    .layer(SetResponseHeaderLayer::if_not_present(
        header::X_FRAME_OPTIONS,
        DENY,
    ))
    .layer(SetResponseHeaderLayer::if_not_present(
        header::REFERRER_POLICY,
        NO_REFERRER,
    ))
    .layer(SetResponseHeaderLayer::if_not_present(
        header::STRICT_TRANSPORT_SECURITY,
        HSTS,
    ))
    .layer(SetResponseHeaderLayer::if_not_present(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static(JSON_CONTENT_SECURITY_POLICY),
    ))
}

/// The policy for a surface that renders nothing — which is every route on both surfaces except
/// the two that answer with HTML.
pub const JSON_CONTENT_SECURITY_POLICY: &str =
    "default-src 'none'; frame-ancestors 'none'; base-uri 'none'";

/// Replace the baseline content-security policy on one sub-router.
///
/// The baseline is applied `if_not_present`, so a route that answers with HTML can set its own and
/// keep it. A handler assembling a single response does that on the response itself; this is for
/// the case where the exception covers a whole sub-router nobody hand-writes — Swagger's bundle,
/// which serves its own scripts, styles and images.
///
/// Kept here rather than layered at the call site so the surfaces do not each grow a `tower-http`
/// dependency to relax a policy this module set.
pub fn override_content_security_policy<S>(app: Router<S>, policy: &'static str) -> Router<S>
where
    S: Clone + Send + Sync + 'static,
{
    app.layer(SetResponseHeaderLayer::overriding(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static(policy),
    ))
}

/// The default request-body ceiling, in **decompressed** bytes, for a door that declares none.
///
/// **This number is ratified, not derived, and that is the point of it being here.** Before this
/// constant existed the ceiling was axum's `DefaultBodyLimit` default — the same 2 MiB, but owned
/// by a dependency, so a framework upgrade could have moved temper's request ceiling with nothing
/// in this repository failing. Setting it explicitly changes no behaviour and moves the decision
/// here, where a change to it is a change someone made on purpose.
///
/// # Read the per-door limits before reasoning about this one
///
/// This is the floor for ordinary requests, not a ceiling over the whole instance. Four doors
/// declare their own, each for a reason recorded next to it, and a limit applied to a single route
/// runs *inside* this one and therefore wins there:
///
/// | Door | Limit | Where |
/// |---|---|---|
/// | signed internal routes | 64 KiB | `temper-api/src/middleware/internal_auth.rs` |
/// | `/api/query` | 4 MB | `QUERY_MAX_BODY_BYTES`, `temper-api/src/routes.rs` |
/// | GitHub webhook intake | 25 MiB | `GITHUB_MAX_WEBHOOK_BYTES`, `temper-api/src/routes.rs` |
/// | `/mcp` | 25 MB | `MCP_MAX_BODY_BYTES`, `temper-mcp/src/router.rs` |
///
/// **`/mcp` is not merely an exception — this constant cannot reach it.** That door is mounted with
/// `nest_service` over a raw tower service, so no axum extractor runs and `DefaultBodyLimit` is
/// inapplicable rather than overridden; it uses `RequestBodyLimitLayer` instead. So this constant
/// governs temper-api's undeclared routes and temper-mcp's *axum* routes (discovery, registration,
/// health) — not the MCP tool surface.
///
/// # Why 2 MiB, and the measurement that bounds the claim
///
/// Ordinary request bodies here are small: the largest first-party document in this repository is
/// ~70 KB and the largest resource body observed in the vault is ~34 KB. 2 MiB is roughly thirty
/// times the former, which is ample for a door carrying one of them.
///
/// **It is emphatically not ample for every payload the contract admits, and that is the reason the
/// table above exists rather than a reason to raise this.** A composition `/api/query` calls legal
/// serializes to **2,194,320 bytes** `[measured — 2026-08-28]` — 97 KB *past* this number. Had that
/// door inherited this ceiling it would have answered a legal plan with a bare 413. The doors
/// carrying large payloads by design — a composition, and the MCP tool surface with `ingest`'s
/// inline content and `data_artifacts`' JSON — each declare their own, which is the shape this
/// constant is the default half of.
///
/// So: **do not raise this to accommodate a door that needs more.** Give that door its own limit and
/// state why, as all four above do.
pub const MAX_REQUEST_BODY_BYTES: usize = 2 * 1024 * 1024;

/// Apply the layers that sit **below** a surface's root span: the fallback handler, request
/// decompression, the request-body ceiling, and the baseline response headers.
///
/// Call this first, then add the surface's own root span and its CORS layer:
///
/// ```ignore
/// apply_base_layers(app)
///     .layer(axum::middleware::from_fn(root_span))
///     .layer(temper_services::cors::cors_layer(&config))
/// ```
///
/// Ordering matters and is preserved from the stack this was extracted from: decompression runs
/// innermost, so the body an extractor sees — and therefore any `DefaultBodyLimit` guarding it —
/// is measured in decompressed bytes.
pub fn apply_base_layers<S>(app: Router<S>) -> Router<S>
where
    S: Clone + Send + Sync + 'static,
{
    let app = app
        .fallback(fallback_handler)
        .layer(RequestDecompressionLayer::new())
        .layer(DefaultBodyLimit::max(MAX_REQUEST_BODY_BYTES));

    security_header_layers(app)
}

/// Bound on the echoed request path in both the `unmatched route` event and the 404
/// body. A path is attacker-chosen input like any other; without a cap its length is
/// whatever the client sent.
const MAX_ECHOED_PATH_BYTES: usize = 512;

/// The path as it may appear in a log event or an error body: redacted through
/// [`temper_telemetry::redact::redact_path`] — the same function the request root
/// spans run — and capped at [`MAX_ECHOED_PATH_BYTES`].
///
/// The fallback sits *outside* the root span (it is the answer when no route exists),
/// so nothing else applies redaction to what it records. An unmatched path can also
/// carry a credential-shaped segment — the invitation-token family this repo once
/// leaked exactly this way — so the event and the body must not become a side door
/// around the deny-by-default guard.
fn sanitize_echoed_path(path: &str) -> std::borrow::Cow<'_, str> {
    let redacted = temper_telemetry::redact::redact_path(path);
    if redacted.len() <= MAX_ECHOED_PATH_BYTES {
        return redacted;
    }
    let mut cut = MAX_ECHOED_PATH_BYTES;
    while !redacted.is_char_boundary(cut) {
        cut -= 1;
    }
    std::borrow::Cow::Owned(format!("{}…", &redacted[..cut]))
}

/// Answer an unmatched route with temper's structured error body.
///
/// Axum's default fallback returns a bare 404 with an empty body. Every other error a caller can
/// receive from temper is an [`crate::error::ErrorBody`], so a client that parses errors would
/// have to special-case exactly this one.
pub async fn fallback_handler(req: axum::extract::Request) -> axum::response::Response {
    use axum::response::IntoResponse;

    let path = sanitize_echoed_path(req.uri().path());
    let method = req.method().to_string();
    tracing::warn!(path = %path, method = %method, "unmatched route");
    let body =
        crate::error::ErrorBody::new("NOT_FOUND", format!("No route matches {method} {path}"));
    (axum::http::StatusCode::NOT_FOUND, axum::Json(body)).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An unmatched request is exactly how a token-in-path would travel if a route ever
    /// reintroduced one — the fallback must run the same redaction the root spans do,
    /// or it becomes the one recording site a credential can pass through unredacted.
    #[test]
    fn the_echoed_path_is_redacted_like_a_span_attribute() {
        let token_path = "/api/invitations/9f8e7d6c5b4a39281706f5e4d3c2b1a0".to_string(); // gitleaks:allow — fake token literal in a redaction regression test
        let sanitized = sanitize_echoed_path(&token_path);
        assert_eq!(sanitized, "/api/invitations/{token}");
        assert!(!sanitized.contains("9f8e7d6c"));
    }

    /// Normal paths — the overwhelming majority — pass through untouched and unallocated.
    #[test]
    fn an_ordinary_path_passes_through_untouched() {
        let sanitized = sanitize_echoed_path("/api/resources/019f97a7-ad61-7e40-b325-73028060ac06");
        assert_eq!(
            sanitized,
            "/api/resources/019f97a7-ad61-7e40-b325-73028060ac06"
        );
        assert!(matches!(sanitized, std::borrow::Cow::Borrowed(_)));
    }

    /// The cap is what makes the echo bounded: a pathological path is cut, not
    /// reflected at full length into the log stream and the response body.
    #[test]
    fn an_oversized_path_is_truncated_on_a_char_boundary() {
        let long = format!("/{}", "a".repeat(MAX_ECHOED_PATH_BYTES * 4));
        let sanitized = sanitize_echoed_path(&long);
        assert!(sanitized.len() <= MAX_ECHOED_PATH_BYTES + '…'.len_utf8());
        assert!(sanitized.ends_with('…'));
        // A multibyte path must not be cut mid-character.
        let multibyte = format!("/{}", "é".repeat(MAX_ECHOED_PATH_BYTES));
        let sanitized = sanitize_echoed_path(&multibyte);
        assert!(sanitized.is_char_boundary(sanitized.len() - '…'.len_utf8()));
    }
}
