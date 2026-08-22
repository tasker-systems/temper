//! Pins which routes on the MCP surface are reachable without a token, and that the rest are not.
//!
//! This exists because the router's public and protected halves are assembled separately and then
//! merged, so "is this route gated?" is a property of assembly order rather than of anything
//! visible at the route definition. Three things can silently change it: a new route merged
//! outside the `require_mcp_auth` layer, a change to which router wins for a path that two of them
//! match, and a fallback (added with the shared transport layers) that answers before auth does.
//!
//! The last one is not hypothetical — it is why this file was written. Adding
//! `transport::apply_base_layers` moved an unmatched path from `401` (the auth middleware wrapping
//! `mcp_routes`' inherited fallback) to `404`. That is the intended shape, but it is a change to
//! what an unauthenticated caller gets, so the gate on the routes that matter is asserted here
//! rather than assumed.
//!
//! `/mcp/health` is the interesting case: it is registered as an exact public route *underneath*
//! `nest_service("/mcp", …)`, which the auth layer wraps. It stays public only because an exact
//! route outranks a nested service. Nothing else states that, so it is stated here.

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use tower::ServiceExt;

mod common;

fn router() -> axum::Router {
    temper_mcp::build_router(
        common::state_with_cors_origins(vec![]),
        common::mcp_config(),
    )
}

async fn status_of(method: &str, uri: &str, bearer: Option<&str>) -> StatusCode {
    let mut builder = Request::builder().method(method).uri(uri);
    if let Some(token) = bearer {
        builder = builder.header(header::AUTHORIZATION, format!("Bearer {token}"));
    }
    router()
        .oneshot(
            builder
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from("{}"))
                .expect("request builds"),
        )
        .await
        .expect("router answers")
        .status()
}

/// The MCP endpoint itself refuses an unauthenticated caller. This is the gate; if it ever
/// answers anything but 401 without a token, the surface is open.
#[tokio::test]
async fn the_mcp_endpoint_refuses_a_caller_with_no_token() {
    assert_eq!(
        status_of("POST", "/mcp", None).await,
        StatusCode::UNAUTHORIZED,
        "POST /mcp must be gated by require_mcp_auth"
    );
}

/// A token that is present but not valid is refused — the gate validates rather than merely
/// checking that the header exists.
///
/// This one needs a key store that can answer, or it proves nothing: against an unreachable JWKS
/// URL the refusal is a `503` from the fetch failing, which would pass a "not 200" assertion
/// without the gate ever having judged the token.
#[tokio::test]
async fn the_mcp_endpoint_refuses_a_malformed_token() {
    let router =
        temper_mcp::build_router(common::state_with_static_jwt_key(), common::mcp_config());

    let status = router
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/mcp")
                .header(header::AUTHORIZATION, "Bearer not-a-jwt")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from("{}"))
                .expect("request builds"),
        )
        .await
        .expect("router answers")
        .status();

    assert_eq!(
        status,
        StatusCode::UNAUTHORIZED,
        "a malformed bearer must be refused by validation, not passed through"
    );
}

/// The refusal carries `WWW-Authenticate` pointing at the protected-resource metadata. MCP clients
/// use this to discover where to authenticate, so a bare 401 would be a broken login flow rather
/// than a closed door.
#[tokio::test]
async fn the_refusal_tells_the_client_where_to_authenticate() {
    let response = router()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/mcp")
                .body(Body::empty())
                .expect("request builds"),
        )
        .await
        .expect("router answers");

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

    let challenge = response
        .headers()
        .get(header::WWW_AUTHENTICATE)
        .expect("a 401 from the MCP gate must carry WWW-Authenticate")
        .to_str()
        .expect("header is ASCII")
        .to_string();

    assert!(
        challenge.contains("resource_metadata"),
        "the challenge must point at the protected-resource metadata; got {challenge:?}"
    );
}

/// The public set, asserted as a set. Each of these is deliberately reachable with no token; a
/// 401 here would break client bootstrap, and a route that stops being listed here has changed
/// posture.
///
/// `/mcp/health` sits under the auth-layered `nest_service("/mcp", …)` and is public only because
/// an exact route outranks a nested service.
#[tokio::test]
async fn the_public_routes_are_reachable_without_a_token() {
    for (method, uri) in [
        ("GET", "/mcp/health"),
        ("GET", "/.well-known/oauth-protected-resource"),
        ("POST", "/oauth/register"),
    ] {
        let status = status_of(method, uri, None).await;
        assert_ne!(
            status,
            StatusCode::UNAUTHORIZED,
            "{method} {uri} is a public bootstrap route and must not be gated"
        );
        assert_ne!(
            status,
            StatusCode::NOT_FOUND,
            "{method} {uri} must exist — a 404 here means the route moved and this assertion \
             stopped covering anything"
        );
    }
}

/// An unmatched path is a 404, not a 401. Answering 401 would tell an unauthenticated prober that
/// every path exists, and would come from the auth middleware rather than from any routing
/// decision.
#[tokio::test]
async fn an_unmatched_path_is_not_answered_by_the_auth_gate() {
    assert_eq!(
        status_of("GET", "/definitely-not-a-route", None).await,
        StatusCode::NOT_FOUND,
        "an unmatched path must be a 404 from the fallback, not a 401 from the auth layer"
    );
}
