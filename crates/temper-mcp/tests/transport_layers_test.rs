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
