//! Witness that the MCP surface's cross-origin policy comes from configuration, not from a
//! hardcoded constant.
//!
//! `CORS_ORIGINS` is read by both HTTP surfaces (`create_app` and `create_internal_app` share
//! `apply_transport_layers`, which builds its `CorsLayer` from `state.config.cors_origins`). The
//! MCP router assembled its own stack and ended in a literal `CorsLayer::permissive()`, so the
//! configured value reached the process, was parsed into `ApiConfig`, and was then dropped at the
//! one layer that acts — on the surface that takes the most automated traffic, with nothing
//! reporting the discrepancy.
//!
//! These two halves fail in opposite directions against the hardcoded-permissive state, which is
//! what makes them a witness for *derived from config* rather than for any particular value:
//!   - deny-all config (`cors_origins: []`) must yield **no** `access-control-allow-origin`;
//!     permissive answers `*`.
//!   - allowlist config must echo **that origin**; permissive answers `*` here too.
//!
//! No database and no port: `connect_lazy` builds an `AppState` whose pool is never queried (same
//! device as `dispatch_witness_test.rs`), and the probe targets `/mcp/health`, the one public
//! route on this router — so neither auth nor a tool body is reached.

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use tower::ServiceExt;

mod common;

const PROBE_ORIGIN: &str = "https://app.example.com";

/// Send a cross-origin GET to the router's public health route and return the
/// `access-control-allow-origin` it answered with, if any.
async fn allow_origin_for(cors_origins: Vec<String>) -> Option<String> {
    let router = common::build_router(
        common::state_with_cors_origins(cors_origins),
        common::discovery_config(),
    );

    let response = router
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/mcp/health")
                .header(header::ORIGIN, PROBE_ORIGIN)
                .body(Body::empty())
                .expect("request builds"),
        )
        .await
        .expect("router answers");

    // The probe is only meaningful if it actually reached the public route — a 404 or a 401 would
    // make an absent CORS header prove nothing about the policy.
    assert_eq!(
        response.status(),
        StatusCode::OK,
        "/mcp/health must be reachable without auth for this probe to mean anything"
    );

    response
        .headers()
        .get(header::ACCESS_CONTROL_ALLOW_ORIGIN)
        .map(|v| v.to_str().expect("header is ASCII").to_string())
}

/// No configured origins means deny-all on the HTTP surfaces (`cors_layer`'s first branch says so
/// in as many words). MCP must agree rather than answering `*`.
#[tokio::test]
async fn no_configured_origins_denies_cross_origin_on_mcp() {
    let allow_origin = allow_origin_for(vec![]).await;

    assert_eq!(
        allow_origin, None,
        "an unconfigured CORS_ORIGINS must deny cross-origin on MCP as it does on the HTTP API; \
         answering {allow_origin:?} means the MCP router is not reading the configured value"
    );
}

/// A configured allowlist must be echoed back as itself. `*` here is the same defect as the case
/// above wearing a different answer: it proves the layer ignored the configuration.
#[tokio::test]
async fn configured_origin_is_echoed_rather_than_wildcarded_on_mcp() {
    let allow_origin = allow_origin_for(vec![PROBE_ORIGIN.to_string()]).await;

    assert_eq!(
        allow_origin.as_deref(),
        Some(PROBE_ORIGIN),
        "a configured allowlist must be honored on MCP; `*` or absence means the configured value \
         never reached the layer that acts"
    );
}

/// Send a browser preflight for an authenticated POST to `/mcp` and return the response.
///
/// A preflight carries no credentials by specification, and `cors_layer` is the outermost layer,
/// above `require_mcp_auth`: tower-http answers the preflight itself and the inner stack never
/// runs. That ordering is required — a preflight that met the JWT check would 401 and no browser
/// could ever call the door — so these probes pin what it means rather than move it.
async fn preflight(cors_origins: Vec<String>) -> axum::response::Response {
    common::build_router(
        common::state_with_cors_origins(cors_origins),
        common::discovery_config(),
    )
    .oneshot(
        Request::builder()
            .method("OPTIONS")
            .uri("/mcp")
            .header(header::ORIGIN, PROBE_ORIGIN)
            .header(header::ACCESS_CONTROL_REQUEST_METHOD, "POST")
            .header(header::ACCESS_CONTROL_REQUEST_HEADERS, "authorization")
            .body(Body::empty())
            .expect("request builds"),
    )
    .await
    .expect("router answers")
}

fn header_of(response: &axum::response::Response, name: header::HeaderName) -> Option<String> {
    response
        .headers()
        .get(name)
        .map(|v| v.to_str().expect("header is ASCII").to_string())
}

/// Unconfigured, a preflight is answered before authentication and grants nothing: no origin, so
/// the browser refuses the real request.
/// FAILS IF: the preflight reaches the JWT check (401), or deny-all starts granting an origin.
#[tokio::test]
async fn an_unconfigured_preflight_is_answered_before_auth_and_grants_nothing() {
    let response = preflight(vec![]).await;
    assert!(
        response.status().is_success(),
        "the CORS layer must answer the preflight itself; {} means it reached the stack below \
         (401 is require_mcp_auth)",
        response.status()
    );
    assert_eq!(
        header_of(&response, header::ACCESS_CONTROL_ALLOW_ORIGIN),
        None,
        "deny-all granted an origin to a preflight"
    );
}

/// An allowlisted origin's preflight is granted, naming the `authorization` header the real
/// request carries, and allows no credentials.
/// FAILS IF: the answer is `*`, which the Fetch standard never lets cover `Authorization`, so a
/// browser client could not send its bearer; or credentials become allowed.
#[tokio::test]
async fn an_allowlisted_preflight_grants_the_origin_and_names_the_bearer_header() {
    let response = preflight(vec![PROBE_ORIGIN.to_string()]).await;
    assert_eq!(
        header_of(&response, header::ACCESS_CONTROL_ALLOW_ORIGIN).as_deref(),
        Some(PROBE_ORIGIN)
    );
    let allowed_headers = header_of(&response, header::ACCESS_CONTROL_ALLOW_HEADERS)
        .unwrap_or_default()
        .to_ascii_lowercase();
    assert!(
        allowed_headers
            .split(',')
            .any(|h| h.trim() == "authorization"),
        "the bearer header must be named for an allowlisted origin: {allowed_headers:?}"
    );
    assert_eq!(
        header_of(&response, header::ACCESS_CONTROL_ALLOW_CREDENTIALS),
        None,
        "the allowlist arm must not allow credentials"
    );
}

/// The permissive arm (`CORS_ORIGINS=*`, development only) answers any origin but never allows
/// credentials. That is what keeps it harmless: no door authenticates by cookie, and a browser
/// will not attach a bearer it does not hold, so a cross-origin page gains nothing.
/// FAILS IF: the permissive arm starts sending `access-control-allow-credentials: true`.
#[tokio::test]
async fn the_permissive_preflight_never_allows_credentials() {
    let response = preflight(vec!["*".to_string()]).await;
    assert_eq!(
        header_of(&response, header::ACCESS_CONTROL_ALLOW_ORIGIN).as_deref(),
        Some("*"),
        "the probe must reach the permissive arm to mean anything"
    );
    assert_eq!(
        header_of(&response, header::ACCESS_CONTROL_ALLOW_CREDENTIALS),
        None,
        "the permissive arm must never allow credentials"
    );
}
