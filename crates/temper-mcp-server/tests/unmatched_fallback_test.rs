//! What the MCP surface answers on a path no route matches.
//!
//! The rule: an unauthenticated caller learns nothing except from the designated public routes.
//! The edge serves this function at its own path (`/api/mcp`) as well as at its doors, so a path
//! outside its public families can reach it. Its 404 there told such a path apart from temper-api,
//! which answers every unmatched path, unauthenticated, with the uniform pre-identity 401.
//!
//! So outside the discovery and OAuth families (`/.well-known/`, `/oauth/`) the fallback is that
//! same refusal, byte for byte, whoever asks. Inside them an unmatched path keeps the structured
//! 404, because RFC 8615 discovery and MCP clients' fallback probes depend on it. `/mcp` and its
//! subpaths never reach the fallback: they keep the edge gate's 401 and its challenge.

use axum::body::{to_bytes, Body};
use axum::http::{header, Request, StatusCode};
use axum::response::{IntoResponse, Response};
use jsonwebtoken::{encode, Algorithm, EncodingKey, Header};
use serde::Serialize;
use tower::ServiceExt;

use temper_services::error::{ApiError, UnauthCause};

mod common;

/// temper-api's pre-identity refusal, as `temper-services`' own byte test pins it.
const U401: &str =
    r#"{"error":{"code":"UNAUTHORIZED","message":"Unauthorized: authentication required"}}"#;

async fn send(method: &str, uri: &str, bearer: Option<&str>) -> Response {
    let mut builder = Request::builder().method(method).uri(uri);
    if let Some(token) = bearer {
        builder = builder.header(header::AUTHORIZATION, format!("Bearer {token}"));
    }
    common::build_router(
        common::state_with_distinct_audiences(),
        common::discovery_config(),
    )
    .oneshot(builder.body(Body::empty()).expect("request builds"))
    .await
    .expect("router answers")
}

async fn body_of(response: Response) -> String {
    let bytes = to_bytes(response.into_body(), 64 * 1024)
        .await
        .expect("body reads");
    String::from_utf8(bytes.to_vec()).expect("body is UTF-8")
}

#[derive(Serialize)]
struct Claims<'a> {
    sub: &'a str,
    iss: &'a str,
    aud: &'a str,
    exp: i64,
    iat: i64,
}

/// A bearer the fixture's gate accepts, so "whoever asks" is witnessed with a caller that is ours.
fn valid_token() -> String {
    encode(
        &Header::new(Algorithm::HS256),
        &Claims {
            sub: "auth|test-user",
            iss: "https://as.test",
            aud: "https://inst.test/mcp",
            exp: i64::MAX / 2,
            iat: 0,
        },
        &EncodingKey::from_secret(b"witness"),
    )
    .expect("token signs")
}

/// Paths outside the public families, including the function's own edge path and case variants of
/// its doors (which reach it if the platform matches routes without regard to case).
const OUTSIDE: &[&str] = &[
    "/api/mcp",
    "/api/mcp/x",
    "/x",
    "/MCPX",
    "/mcpx",
    "/MCP",
    "/OAUTH/register",
    "/.WELL-KNOWN/oauth-protected-resource",
    "/oauth",
    "/.well-known",
    "/",
];

/// Outside the families, an unmatched path answers temper-api's refusal: the same status, the same
/// body bytes as `ApiError::Unauthenticated` renders, the same content type, no `Allow` and no MCP
/// challenge. Every method but `OPTIONS` (see below), with and without a valid bearer.
///
/// Bite: the fallback back to `fallback_handler` (a 404), or the prefix test made
/// case-insensitive (`/OAUTH/register` a 404).
#[tokio::test]
async fn an_unmatched_path_outside_the_families_is_the_uniform_refusal() {
    let reference = ApiError::Unauthenticated(UnauthCause::UnmatchedPath).into_response();
    let reference_type = reference.headers().get(header::CONTENT_TYPE).cloned();
    let reference_body = body_of(reference).await;
    assert_eq!(
        reference_body, U401,
        "the render this test compares against"
    );

    // The values temper-api's answers carry, from the one definition both surfaces apply.
    let mut security = axum::http::HeaderMap::new();
    temper_services::transport::insert_security_headers(&mut security);
    assert_eq!(security.len(), 5, "the five baseline headers");

    let token = valid_token();
    for uri in OUTSIDE {
        for method in ["GET", "HEAD", "POST", "PUT", "DELETE", "PATCH"] {
            for bearer in [None, Some(token.as_str())] {
                let label = format!("{method} {uri} (bearer: {})", bearer.is_some());
                let response = send(method, uri, bearer).await;
                assert_eq!(response.status(), StatusCode::UNAUTHORIZED, "{label}");
                assert_eq!(
                    response.headers().get(header::CONTENT_TYPE).cloned(),
                    reference_type,
                    "{label}"
                );
                // axum stamps `Allow` only on a matched route's method fallback, so this holds by
                // construction; it pins the property temper-api's refusal has, not this fix.
                assert!(response.headers().get(header::ALLOW).is_none(), "{label}");
                for (name, value) in &security {
                    assert_eq!(
                        response.headers().get(name),
                        Some(value),
                        "{label}: the refusal carries the API's security header {name}"
                    );
                }
                assert!(
                    response.headers().get(header::WWW_AUTHENTICATE).is_none(),
                    "{label}: the MCP challenge belongs to /mcp, not to a refused path"
                );
                let expected = if method == "HEAD" { "" } else { U401 };
                assert_eq!(body_of(response).await, expected, "{label}");
            }
        }
    }
}

/// Inside the discovery and OAuth families an unmatched path keeps the structured 404, including
/// the probes MCP clients make before falling back to the root metadata.
///
/// Pinned behaviour (the 404 is what they answered before the change); it bites if
/// `NOT_FOUND_PREFIXES` is emptied, when each becomes the 401.
#[tokio::test]
async fn an_unmatched_path_inside_the_families_keeps_its_404() {
    for uri in [
        "/.well-known/unknown",
        "/.well-known/oauth-protected-resource/mcp",
        "/.well-known/openid-configuration",
        "/.well-known/openid-configuration/mcp",
        "/oauth/unknown",
        "/oauth/register/x",
    ] {
        let response = send("GET", uri, None).await;
        assert_eq!(response.status(), StatusCode::NOT_FOUND, "GET {uri}");
        let body: serde_json::Value =
            serde_json::from_str(&body_of(response).await).expect("structured body");
        assert_eq!(body["error"]["code"], "NOT_FOUND", "GET {uri}");
    }
}

/// `/mcp` and its subpaths are the edge gate's, not the fallback's: an anonymous caller gets the
/// 401 with the `WWW-Authenticate` challenge MCP clients discover the authorization server by.
///
/// Bite: `/mcp` mounted with `route_service` instead of `nest_service` (`/mcp/x` becomes the
/// fallback's refusal, with no challenge).
#[tokio::test]
async fn the_mcp_door_keeps_its_challenge() {
    for uri in ["/mcp", "/mcp/", "/mcp/x"] {
        let response = send("POST", uri, None).await;
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED, "POST {uri}");
        let challenge = response
            .headers()
            .get(header::WWW_AUTHENTICATE)
            .unwrap_or_else(|| panic!("POST {uri}: the 401 must carry the challenge"))
            .to_str()
            .expect("ASCII")
            .to_owned();
        assert!(challenge.contains("resource_metadata"), "POST {uri}");
    }
}

/// The designated public routes answer for themselves, unchanged. Pinned rather than a witness of
/// the fix: they answered the same before it.
#[tokio::test]
async fn the_public_routes_answer_for_themselves() {
    for (method, uri) in [
        ("GET", "/mcp/health"),
        ("GET", "/.well-known/oauth-protected-resource"),
        ("POST", "/oauth/register"),
    ] {
        let status = send(method, uri, None).await.status();
        assert_ne!(status, StatusCode::UNAUTHORIZED, "{method} {uri}");
        assert_ne!(status, StatusCode::NOT_FOUND, "{method} {uri}");
    }
}

/// The declared residue, pinned so a change to it is seen: the CORS layer, a `Router::layer` here,
/// answers every `OPTIONS` itself, before routing, so every path on this function answers it alike
/// (`200`), where temper-api's outer service routes a non-preflight `OPTIONS` to the refusal. On an
/// unmatched path it carries no `Allow`; on a matched public route axum stamps that route's methods
/// (`/oauth/register` answers `Allow: POST`), which is the public door's own answer. So it sorts
/// this function's paths from temper-api's, and names only the public routes' methods.
/// `docs/concepts/trust-boundary.md` states it.
#[tokio::test]
async fn every_options_is_answered_alike_by_the_cors_layer() {
    for uri in ["/api/mcp", "/x", "/.well-known/unknown", "/oauth/unknown"] {
        let response = send("OPTIONS", uri, None).await;
        assert_eq!(response.status(), StatusCode::OK, "OPTIONS {uri}");
        assert!(
            response.headers().get(header::ALLOW).is_none(),
            "OPTIONS {uri}"
        );
    }
    let response = send("OPTIONS", "/oauth/register", None).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response
            .headers()
            .get(header::ALLOW)
            .map(|v| v.to_str().unwrap().to_owned()),
        Some("POST".to_owned()),
        "a public route's own methods, and only on a public route"
    );
}
