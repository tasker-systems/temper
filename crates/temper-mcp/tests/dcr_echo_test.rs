//! `POST /oauth/register` is an echo, and what it will not echo is the property worth witnessing.
//!
//! The endpoint is deliberately **a thin static-client echo, not RFC 7591 dynamic registration**.
//! It hands back a pre-registered client id and filters the redirect URIs a client proposes against
//! an allowlist it did not get from the client. The load-bearing invariant: **registration never
//! writes to the authorization server's client allowlist**, and open-redirect protection stays
//! enforced at `/oauth/authorize` against `AS_CLIENTS`.
//!
//! Nothing witnessed that before. The filter was three lines in `discovery.rs` with a comment
//! saying what they were for, and a comment is not a test — a future change that persisted
//! client-supplied URIs, in the belief it was completing a half-built DCR implementation, would have
//! reintroduced the redirect-to-code-capture chain and broken no test at all.
//!
//! No auth, no database, no port: every probe targets the public registration route.
//!
//! `cargo nextest run -p temper-mcp --test dcr_echo_test`

use axum::body::{to_bytes, Body};
use axum::http::{header, Request, StatusCode};
use tower::ServiceExt;

mod common;

use temper_mcp::config::{McpConfig, OAuthStaticConfig};

const ALLOWED: &str = "https://temper.invalid/api/auth/mcp-callback";
const ATTACKER: &str = "https://attacker.example/collect";

/// A router whose registration endpoint is configured and whose allowlist is exactly `ALLOWED`.
fn router(allow_localhost: bool) -> axum::Router {
    temper_mcp::build_router(
        common::state_with_cors_origins(vec![]),
        McpConfig {
            mcp_base_url: "https://temper.invalid".to_string(),
            mcp_client_id: Some("temper-mcp".to_string()),
            oauth: OAuthStaticConfig {
                redirect_uris: vec![ALLOWED.to_string()],
                allow_localhost,
            },
        },
    )
}

async fn register(app: axum::Router, proposed: &[&str]) -> (StatusCode, serde_json::Value) {
    let body = serde_json::json!({
        "client_name": "witness",
        "redirect_uris": proposed,
    })
    .to_string();

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/oauth/register")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(body))
                .expect("request builds"),
        )
        .await
        .expect("router answers");

    let status = response.status();
    let bytes = to_bytes(response.into_body(), 64 * 1024)
        .await
        .expect("body reads");
    (
        status,
        serde_json::from_slice(&bytes).expect("registration answers JSON"),
    )
}

/// A redirect URI the client invented is not echoed back, and the allowlisted one still is.
///
/// **Both halves matter.** A filter that dropped everything would pass a test asserting only the
/// attacker URI's absence, and would break every real client while looking correct.
#[tokio::test]
async fn a_client_proposed_redirect_uri_is_not_echoed_back() {
    let (status, body) = register(router(false), &[ALLOWED, ATTACKER]).await;

    assert_eq!(status, StatusCode::CREATED);

    let echoed: Vec<&str> = body["redirect_uris"]
        .as_array()
        .expect("redirect_uris is an array")
        .iter()
        .map(|v| v.as_str().expect("each URI is a string"))
        .collect();

    assert!(
        !echoed.contains(&ATTACKER),
        "a URI the client supplied and the allowlist does not hold must never be echoed — that is \
         the first step of the redirect-to-code-capture chain this endpoint refuses to build; got \
         {echoed:?}"
    );
    assert_eq!(
        echoed,
        vec![ALLOWED],
        "the allowlisted URI must survive, or every real client breaks while the filter looks \
         correct"
    );
}

/// Registration hands back the pre-registered client id, never one the client asked for.
#[tokio::test]
async fn registration_returns_the_pre_registered_client_id() {
    let (_, body) = register(router(false), &[ALLOWED]).await;

    assert_eq!(
        body["client_id"].as_str(),
        Some("temper-mcp"),
        "the endpoint echoes the instance's own client id; a client that could choose its own \
         would be doing real dynamic registration, which this deliberately is not"
    );
}

/// A loopback callback is admitted only where the instance opted into it.
///
/// RFC 8252 callbacks use an ephemeral port and cannot be enumerated, so `allow_localhost` is the
/// one rule that admits a URI no allowlist contains. Witnessed in both directions, because the
/// permissive half alone would pass against a filter that admitted loopback unconditionally.
#[tokio::test]
async fn a_loopback_callback_is_admitted_only_when_the_instance_allows_it() {
    const LOOPBACK: &str = "http://127.0.0.1:49152/callback";

    let (_, permitted) = register(router(true), &[LOOPBACK]).await;
    assert_eq!(
        permitted["redirect_uris"].as_array().map(Vec::len),
        Some(1),
        "with allow_localhost set, a desktop or CLI client's ephemeral-port callback must be \
         echoed — otherwise it cannot register at all"
    );

    let (_, refused) = register(router(false), &[LOOPBACK]).await;
    assert_eq!(
        refused["redirect_uris"].as_array().map(Vec::len),
        Some(0),
        "with allow_localhost unset, loopback is not a standing exception; a filter that admitted \
         it regardless would make the flag decorative"
    );
}
