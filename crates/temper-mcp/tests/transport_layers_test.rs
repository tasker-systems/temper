//! Witness that the MCP router carries the transport layers both HTTP surfaces apply.
//!
//! `create_app` and `create_internal_app` share `apply_transport_layers`. The MCP router assembled
//! its own stack and reproduced only the root span, so two layers were absent on the surface most
//! likely to be consumed by a program rather than a person:
//!
//!   - **request decompression** — a `Content-Encoding: gzip` body reached the JSON extractor still
//!     compressed and was rejected as malformed. This also orphaned a piece of reasoning written as
//!     though it held everywhere: the webhook body limit's doc comment
//!     (`temper-api/src/routes.rs`) explains that it "bounds the body axum's extractor sees, which
//!     is *after* the app-wide `RequestDecompressionLayer` — so it bounds decompressed bytes". On
//!     MCP there was no such layer for that sentence to be true about.
//!   - **the fallback handler** — an unmatched path got axum's bare 404 with an empty body instead
//!     of the structured `ErrorBody` every other temper surface returns.
//!
//! Both probes target public routes, so neither needs auth, a database, or a port.

use axum::body::{to_bytes, Body};
use axum::http::{header, Request, StatusCode};
use flate2::write::GzEncoder;
use flate2::Compression;
use std::io::Write;
use tower::ServiceExt;

mod common;

/// A body past the door's ceiling is refused, and a body under it is not refused FOR ITS SIZE.
///
/// `[added — 2026-08-28, found in review]` `/mcp` is mounted with `nest_service`, whose target is a
/// raw tower service — no axum extractor runs, so `DefaultBodyLimit` is not merely unset here, it is
/// inapplicable. rmcp's `expect_json` reads with a bare `.collect()`, so this door buffered an
/// arbitrarily large body while the sibling reasoning in `temper-api/src/routes.rs` read as though
/// every surface were bounded.
///
/// **The under-limit half is what makes this a witness rather than a tautology.** A test that only
/// sent an oversized body would stay green if the limit were set absurdly low, which is the failure
/// that breaks legitimate ingest — this door carries `ingest`'s inline content and
/// `data_artifacts`' JSON, so refusing too early is the more likely mistake. The small body must
/// therefore get past the limit and fail somewhere else (401 — it is unauthenticated), never 413.
///
/// **`Content-Length` is load-bearing and the reason is worth keeping.** `RequestBodyLimitLayer`
/// refuses eagerly on that header and otherwise only when the body is READ — and `require_mcp_auth`
/// answers 401 without reading it. So a declared oversized body is refused before authentication,
/// while an undeclared (chunked) one is refused when rmcp reads it, after. Both are bounded; only
/// the first is observable without a token, which is why this probe sends the header a real client
/// sends.
#[tokio::test]
async fn a_body_past_the_ceiling_is_refused_and_a_small_one_is_not() {
    // One byte over 25 MB. Built here rather than named from the constant, which is private: a test
    // that imported the number could not disagree with it.
    let over = vec![b'x'; 25 * 1024 * 1024 + 1];
    let over_len = over.len();
    let response = router()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/mcp")
                .header(header::CONTENT_TYPE, "application/json")
                .header(header::CONTENT_LENGTH, over_len)
                .body(Body::from(over))
                .expect("request builds"),
        )
        .await
        .expect("router answers");
    assert_eq!(
        response.status(),
        StatusCode::PAYLOAD_TOO_LARGE,
        "a body past the ceiling must be refused by the transport, not buffered"
    );

    let under = vec![b'x'; 1024];
    let under_len = under.len();
    let response = router()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/mcp")
                .header(header::CONTENT_TYPE, "application/json")
                .header(header::CONTENT_LENGTH, under_len)
                .body(Body::from(under))
                .expect("request builds"),
        )
        .await
        .expect("router answers");
    assert_ne!(
        response.status(),
        StatusCode::PAYLOAD_TOO_LARGE,
        "a 1 KB body is nowhere near the ceiling; a 413 here means the limit is mis-set"
    );
}

/// Assemble the real router over a never-queried pool.
fn router() -> axum::Router {
    temper_mcp::build_router(
        common::state_with_cors_origins(vec![]),
        common::mcp_config(),
    )
}

/// An unmatched path answers with temper's structured error body, not axum's empty 404.
///
/// The status alone is not the witness — axum's default fallback also returns 404. The body is:
/// without `fallback_handler` it is empty, so the `error.code` assertion is what fails.
#[tokio::test]
async fn an_unmatched_path_answers_with_the_structured_error_body() {
    let response = router()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/no-such-route-on-the-mcp-surface")
                .body(Body::empty())
                .expect("request builds"),
        )
        .await
        .expect("router answers");

    assert_eq!(response.status(), StatusCode::NOT_FOUND);

    let bytes = to_bytes(response.into_body(), 64 * 1024)
        .await
        .expect("body reads");
    let parsed: serde_json::Value = serde_json::from_slice(&bytes).unwrap_or_else(|e| {
        panic!(
            "an unmatched path must answer with temper's structured error body; \
             got {} byte(s) that are not JSON ({e}) — axum's default fallback returns an empty \
             body, which is the state this witness guards against",
            bytes.len()
        )
    });

    assert_eq!(
        parsed["error"]["code"], "NOT_FOUND",
        "the fallback must produce the same error shape the HTTP surfaces produce; body was {parsed}"
    );
}

/// A gzip-encoded request body is decompressed before the JSON extractor sees it.
///
/// `/oauth/register` is public and reads `Json<ClientRegistrationRequest>`. With no
/// `mcp_client_id` configured the handler answers `503` — so `503` proves the extractor parsed the
/// body and the handler ran. Without the decompression layer the extractor is handed gzip bytes
/// and rejects them (a 4xx), which is the failure this witness names.
#[tokio::test]
async fn a_gzip_encoded_body_is_decompressed_before_the_json_extractor() {
    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    encoder
        .write_all(br#"{"client_name":"witness","redirect_uris":[]}"#)
        .expect("gzip write");
    let gzipped = encoder.finish().expect("gzip finish");

    let response = router()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/oauth/register")
                .header(header::CONTENT_TYPE, "application/json")
                .header(header::CONTENT_ENCODING, "gzip")
                .body(Body::from(gzipped))
                .expect("request builds"),
        )
        .await
        .expect("router answers");

    assert_eq!(
        response.status(),
        StatusCode::SERVICE_UNAVAILABLE,
        "a gzip body must be decompressed before the JSON extractor; a 4xx here means the \
         extractor was handed compressed bytes, and 503 is what the handler itself returns when \
         no MCP_CLIENT_ID is configured"
    );
}

// ── The request-body ceiling ────────────────────────────────────────────────────────────────────
//
// `MAX_REQUEST_BODY_BYTES` ratifies the ceiling that axum's `DefaultBodyLimit` default was already
// supplying, so it changes no behaviour — and **none of the three witnesses below proves the
// constant was added**, because a probe cannot tell temper's 2 MiB from a dependency's identical
// 2 MiB. They are here for the property the constant exists to protect: the ceiling cannot *move*
// without one of them going red. A framework upgrade that changed the default, an edit to the
// constant, or the decompression ordering being disturbed each break exactly one of them.
//
// Same reasoning, and same limitation, as `a_payload_above_githubs_ceiling_is_refused` in
// temper-api's webhook suite — "here to stop the ceiling being removed, not to prove it was added".
//
// `/oauth/register` is the probe for the same reason the decompression witness above uses it: it is
// public, it reads `Json<_>`, and with no `mcp_client_id` configured the handler answers `503`. So
// `503` means the extractor read the whole body, and `413` means it refused to.
//
// **These cover a different door from `MCP_MAX_BODY_BYTES`' witness above, and the split is not
// cosmetic.** `/mcp` is a raw tower service where `DefaultBodyLimit` is inapplicable, so it carries
// `RequestBodyLimitLayer` and its own 25 MB. The routes below — discovery, registration, health —
// are ordinary axum routes, so `MAX_REQUEST_BODY_BYTES` is what bounds them. One surface, two
// mechanisms, because one of them cannot reach half of it.

/// A JSON body of exactly `n` decompressed bytes, valid enough for the extractor to parse.
fn registration_body_of(n: usize) -> Vec<u8> {
    const PREFIX: &[u8] = br#"{"client_name":""#;
    const SUFFIX: &[u8] = br#"","redirect_uris":[]}"#;
    let padding = n
        .checked_sub(PREFIX.len() + SUFFIX.len())
        .expect("requested body is smaller than the JSON scaffolding around it");
    let mut body = Vec::with_capacity(n);
    body.extend_from_slice(PREFIX);
    body.resize(PREFIX.len() + padding, b'a');
    body.extend_from_slice(SUFFIX);
    assert_eq!(body.len(), n);
    body
}

async fn register_with(body: Vec<u8>, encoding: Option<&'static str>) -> StatusCode {
    let mut request = Request::builder()
        .method("POST")
        .uri("/oauth/register")
        .header(header::CONTENT_TYPE, "application/json");
    if let Some(encoding) = encoding {
        request = request.header(header::CONTENT_ENCODING, encoding);
    }
    router()
        .oneshot(request.body(Body::from(body)).expect("request builds"))
        .await
        .expect("router answers")
        .status()
}

/// A body at the ceiling is read in full.
///
/// The lower bracket. Without it, the refusal witness below would also pass against a ceiling of
/// zero, and "the limit is 2 MiB" would be indistinguishable from "the limit is nothing at all".
#[tokio::test]
async fn a_body_at_the_request_ceiling_is_read() {
    let status = register_with(registration_body_of(2 * 1024 * 1024), None).await;

    assert_eq!(
        status,
        StatusCode::SERVICE_UNAVAILABLE,
        "a body of exactly MAX_REQUEST_BODY_BYTES must reach the handler; 413 here means the \
         ceiling is below the value temper set, which would refuse requests temper accepts today"
    );
}

/// One byte past the ceiling is refused.
///
/// The upper bracket, and the witness that pins the number: raising or lowering
/// `MAX_REQUEST_BODY_BYTES` fails this and nothing else. The size is written out rather than
/// imported from the constant deliberately — a witness that reads the value it is pinning follows
/// an edit instead of catching it.
#[tokio::test]
async fn a_body_one_byte_past_the_request_ceiling_is_refused() {
    let status = register_with(registration_body_of(2 * 1024 * 1024 + 1), None).await;

    assert_eq!(
        status,
        StatusCode::PAYLOAD_TOO_LARGE,
        "the ceiling must be 2 MiB exactly; a 503 here means the limit moved upward, which is the \
         change this witness exists to make loud"
    );
}

/// The ceiling counts decompressed bytes, not bytes on the wire.
///
/// This is the property `apply_base_layers`' ordering comment claims and that nothing held it to.
/// The body below is tens of KB compressed and 64 MiB expanded, so a ceiling measured on the wire
/// would admit it and hand the extractor 64 MiB to parse. Distinct from the decompression witness
/// above: that one proves the body is decompressed at all, this one proves the *limit* is applied
/// after it happens.
#[tokio::test]
async fn the_request_ceiling_is_measured_after_decompression() {
    // Deliberately far above any ceiling this repository would plausibly choose. A size close to
    // MAX_REQUEST_BODY_BYTES would make this witness fail whenever that value moved, which is the
    // bracketing witnesses' job — this one must answer only to the axis the ceiling is measured on.
    let expanded = registration_body_of(64 * 1024 * 1024);
    let mut encoder = GzEncoder::new(Vec::new(), Compression::best());
    encoder.write_all(&expanded).expect("gzip write");
    let compressed = encoder.finish().expect("gzip finish");

    assert!(
        compressed.len() < 2 * 1024 * 1024,
        "the probe is only meaningful if the compressed body is itself under the ceiling; it is {} \
         byte(s)",
        compressed.len()
    );

    let status = register_with(compressed, Some("gzip")).await;

    assert_eq!(
        status,
        StatusCode::PAYLOAD_TOO_LARGE,
        "a 64 MiB body that arrives compressed must be refused; any other status means the ceiling \
         was applied to the size on the wire, and a small request can still expand past it"
    );
}

// ── The baseline response headers ───────────────────────────────────────────────────────────────
//
// Set in `apply_base_layers`, so both public surfaces carry them and neither can be assembled
// without them. Witnessed here for the same reason the layers above are: the MCP surface is the one
// that was previously built without the shared stack, so it is the surface where an omission
// actually happened once.
//
// These bite cleanly, unlike the request-ceiling witnesses: nothing supplied these headers before,
// so removing the layer removes the header and every assertion below fails.

/// Every response carries the baseline, and the policy is the one for a surface that renders
/// nothing.
///
/// Asserted on the 404 path deliberately. A header set on handler responses but not on the
/// fallback would pass a test that probed a real route, and the unmatched-path response is exactly
/// the one a scanner reaches first.
#[tokio::test]
async fn every_response_carries_the_baseline_security_headers() {
    let response = router()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/no-such-route-on-the-mcp-surface")
                .body(Body::empty())
                .expect("request builds"),
        )
        .await
        .expect("router answers");

    let headers = response.headers();
    for (name, expected) in [
        ("x-content-type-options", "nosniff"),
        ("x-frame-options", "DENY"),
        ("referrer-policy", "no-referrer"),
        (
            "strict-transport-security",
            "max-age=63072000; includeSubDomains",
        ),
        (
            "content-security-policy",
            "default-src 'none'; frame-ancestors 'none'; base-uri 'none'",
        ),
    ] {
        assert_eq!(
            headers.get(name).map(|v| v.to_str().expect("header is ascii")),
            Some(expected),
            "{name} must be set in-app; the posture cannot depend on what the hosting edge \
             happens to send, because the edge can be reconfigured without touching this repository"
        );
    }
}

/// The baseline is a floor a response can raise or relax, not a value layered over one.
///
/// `apply_base_layers` sets each header `if_not_present`, which is what lets temper-api's HTML
/// routes carry their own content-security policy. Probed here on the shared layer rather than on
/// those routes: if this ever became `overriding`, the exception on the Slack callback page would
/// stop working and that page would render unstyled — a failure nobody would see in a test that
/// only checked the headers were present.
#[tokio::test]
async fn a_response_that_sets_its_own_policy_keeps_it() {
    use axum::routing::get;

    let app = temper_services::transport::apply_base_layers(axum::Router::new().route(
        "/probe",
        get(|| async {
            (
                [(
                    axum::http::header::CONTENT_SECURITY_POLICY,
                    "default-src 'none'; style-src 'unsafe-inline'",
                )],
                "probe",
            )
        }),
    ));

    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/probe")
                .body(Body::empty())
                .expect("request builds"),
        )
        .await
        .expect("router answers");

    assert_eq!(
        response
            .headers()
            .get("content-security-policy")
            .map(|v| v.to_str().expect("header is ascii")),
        Some("default-src 'none'; style-src 'unsafe-inline'"),
        "a response that set its own policy must keep it; the shared baseline is applied \
         if_not_present precisely so an HTML route can state its own exception next to the markup"
    );
    assert_eq!(
        response
            .headers()
            .get("x-content-type-options")
            .map(|v| v.to_str().expect("header is ascii")),
        Some("nosniff"),
        "relaxing one header must not drop the rest of the baseline"
    );
}
