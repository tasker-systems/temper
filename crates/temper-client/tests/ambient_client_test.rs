//! An ambient client (`TEMPER_AUTH=ambient`) against a real HTTP server.
//!
//! The credential is held outside the process and attached in transit, so what these pin is what
//! goes on the wire: no `Authorization` header unless a placeholder is configured, the placeholder
//! verbatim when it is, and a `401` that names the ambient credential instead of `temper auth
//! login`. Same `wiremock` pattern as `profile_entitlements_test.rs`.

use serde_json::json;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use temper_client::auth::AmbientAuth;
use temper_client::error::ClientError;
use temper_client::TemperClient;

fn ambient_client(base_url: &str, auth: &AmbientAuth) -> TemperClient {
    TemperClient::ambient(
        base_url,
        temper_workflow::operations::Surface::CliCloud,
        auth,
    )
    .expect("test server URL (loopback) validates")
}

fn profile_body() -> serde_json::Value {
    json!({
        "id": "019d4add-f49d-7c43-a87d-dda470e5dd9c",
        "display_name": "Test Person",
        "slug": "test-person",
        "email": "test@example.com",
        "avatar_url": null,
        "preferences": {},
        "vault_config": {},
        "created": "2026-01-01T00:00:00Z",
        "updated": "2026-01-01T00:00:00Z"
    })
}

async fn profile_server(status: u16, body: serde_json::Value) -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/profile"))
        .respond_with(ResponseTemplate::new(status).set_body_json(body))
        .mount(&server)
        .await;
    server
}

async fn only_request(server: &MockServer) -> wiremock::Request {
    let mut received = server.received_requests().await.expect("recording on");
    assert_eq!(received.len(), 1, "exactly one request");
    received.remove(0)
}

#[tokio::test]
async fn without_a_placeholder_no_authorization_header_is_sent() {
    let server = profile_server(200, profile_body()).await;
    let client = ambient_client(&server.uri(), &AmbientAuth::default());

    assert!(client.is_ambient());
    let profile = client.profile().get().await.expect("the server answered");
    assert_eq!(profile.slug, "test-person");

    let req = only_request(&server).await;
    assert!(
        req.headers.get("authorization").is_none(),
        "the credential is attached in transit, never by this process"
    );
    assert!(
        req.headers.get("x-temper-device-id").is_none(),
        "no device id is minted"
    );
}

#[tokio::test]
async fn a_placeholder_is_sent_verbatim_as_the_bearer() {
    let server = profile_server(200, profile_body()).await;
    let auth = AmbientAuth::new(
        Some("placeholder-not-a-jwt".to_owned().into()),
        Some("dev-1".to_owned()),
    );
    let client = ambient_client(&server.uri(), &auth);

    client.profile().get().await.expect("the server answered");

    let req = only_request(&server).await;
    assert_eq!(
        req.headers
            .get("authorization")
            .map(|v| v.to_str().unwrap()),
        Some("Bearer placeholder-not-a-jwt")
    );
    assert_eq!(
        req.headers
            .get("x-temper-device-id")
            .map(|v| v.to_str().unwrap()),
        Some("dev-1")
    );
}

#[tokio::test]
async fn a_401_with_an_envelope_names_the_ambient_credential() {
    let body =
        json!({"error": {"code": "UNAUTHORIZED", "message": "Missing Authorization header"}});
    let server = profile_server(401, body).await;
    let client = ambient_client(&server.uri(), &AmbientAuth::default());

    let err = client.profile().get().await.expect_err("refused");
    match &err {
        ClientError::AmbientRefused { message } => {
            assert_eq!(message.as_deref(), Some("Missing Authorization header"));
        }
        other => panic!("expected AmbientRefused, got {other:?}"),
    }
    let rendered = err.to_string();
    assert!(rendered.contains("TEMPER_AUTH=ambient"), "{rendered}");
    assert!(
        !rendered.contains("temper auth login"),
        "logging in cannot fix an ambient credential: {rendered}"
    );
}

#[tokio::test]
async fn a_bare_401_names_the_ambient_credential() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/profile"))
        .respond_with(ResponseTemplate::new(401).set_body_string("Authentication required"))
        .mount(&server)
        .await;
    let client = ambient_client(&server.uri(), &AmbientAuth::default());

    let err = client.profile().get().await.expect_err("refused");
    assert!(
        matches!(err, ClientError::AmbientRefused { message: None }),
        "got {err:?}"
    );
}

/// Only the 401 is rewritten: every other refusal keeps its own variant.
#[tokio::test]
async fn other_refusals_pass_through_unchanged() {
    let server = profile_server(403, json!({})).await;
    let client = ambient_client(&server.uri(), &AmbientAuth::default());

    let err = client.profile().get().await.expect_err("refused");
    assert!(matches!(err, ClientError::Forbidden), "got {err:?}");
}

/// The facade's store is empty by construction: nothing to report, refresh or log out of.
#[test]
fn the_facade_holds_no_credential() {
    let client = ambient_client("https://api.example.com", &AmbientAuth::default());
    let status = client.auth_status().expect("status reads the empty store");
    assert!(!status.authenticated);
}
