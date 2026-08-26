//! `ProfileClient` against the real `GET /api/profile` wire shape.
//!
//! The defect these pin is a silent one: `get()` deserializes into a bare `Profile`, which neither
//! declares an `entitlements` field nor denies unknown ones — so serde **discards the entitlements
//! object on every call**, without an error. The CLI was fetching the authoritative access answer
//! and throwing it away, then answering the access question from the join-request queue instead.
//!
//! A type-level change cannot witness that; only a real response body can. Same `wiremock` pattern
//! as `segments_client_test.rs`.

use std::sync::Arc;

use serde_json::json;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use temper_client::auth::MemoryTokenStore;
use temper_client::TemperClient;

fn test_client(base_url: &str) -> TemperClient {
    TemperClient::with_token(
        base_url,
        None,
        temper_workflow::operations::Surface::CliCloud,
        "test-token".to_string(),
        Arc::new(MemoryTokenStore::empty()),
    )
}

/// The profile half is flattened into the same object as `entitlements`, so this body is the shape
/// the server actually sends — not a hand-built approximation of it.
fn profile_body(
    system_access: bool,
    is_admin: bool,
    join_request: serde_json::Value,
) -> serde_json::Value {
    let mut body = bare_profile_body(system_access, is_admin, join_request);
    body["entitlements"]["standing"] = json!("approved");
    body
}

/// The same body **without** `standing` — what a server older than that field sends.
fn bare_profile_body(
    system_access: bool,
    is_admin: bool,
    join_request: serde_json::Value,
) -> serde_json::Value {
    json!({
        "id": "019d4add-f49d-7c43-a87d-dda470e5dd9c",
        "display_name": "Test Person",
        "slug": "test-person",
        "email": "test@example.com",
        "avatar_url": null,
        "preferences": {},
        "vault_config": {},
        "created": "2026-01-01T00:00:00Z",
        "updated": "2026-01-01T00:00:00Z",
        "entitlements": {
            "system_access": system_access,
            "is_admin": is_admin,
            "join_request_status": join_request
        }
    })
}

/// The witness for the original bug. An approved principal who never filed a join request has
/// `join_request_status: null` — the shape the old CLI read as denial — while `system_access` is
/// true. Reading it back through the typed client must preserve both.
#[tokio::test]
async fn entitlements_survive_the_round_trip_when_the_queue_is_empty() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/profile"))
        .respond_with(ResponseTemplate::new(200).set_body_json(profile_body(
            true,
            true,
            json!(null),
        )))
        .mount(&server)
        .await;

    let client = test_client(&server.uri());
    let p = client.profile().get_with_entitlements().await.unwrap();

    assert!(
        p.entitlements.system_access,
        "an empty join-request queue must not read as denial"
    );
    assert!(p.entitlements.is_admin);
    assert_eq!(p.entitlements.join_request_status, None);
}

/// Identity comes back on the same round trip — this is what replaces the stored `profile_id`,
/// which is structurally absent under Auth0.
#[tokio::test]
async fn the_flattened_profile_half_still_deserializes() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/profile"))
        .respond_with(ResponseTemplate::new(200).set_body_json(profile_body(
            false,
            false,
            json!("pending"),
        )))
        .mount(&server)
        .await;

    let client = test_client(&server.uri());
    let p = client.profile().get_with_entitlements().await.unwrap();

    assert_eq!(p.profile.slug, "test-person");
    assert_eq!(
        p.profile.id.to_string(),
        "019d4add-f49d-7c43-a87d-dda470e5dd9c"
    );
    assert_eq!(
        p.entitlements.join_request_status,
        Some(temper_core::types::access_gate::JoinRequestStatus::Pending)
    );
}

/// The discard, made observable. `get()` reads the identical body into a bare `Profile` and the
/// entitlements vanish — there is no error and no missing-field complaint, which is exactly why the
/// bug survived. This test does not assert a bug is fixed; it pins *why the other method exists*,
/// so that deleting `get_with_entitlements` and routing callers back through `get` fails here.
#[tokio::test]
async fn plain_get_silently_drops_entitlements() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/profile"))
        .respond_with(ResponseTemplate::new(200).set_body_json(profile_body(
            true,
            true,
            json!(null),
        )))
        .mount(&server)
        .await;

    let client = test_client(&server.uri());
    // Succeeds. That is the point: nothing signals that two thirds of the answer was thrown away.
    let bare = client.profile().get().await.unwrap();
    assert_eq!(bare.slug, "test-person");
}

/// A server older than the `standing` field must still yield a usable answer.
///
/// This is the version-skew case, and it is tested by **serving a body that genuinely omits the
/// field** rather than by constructing the post-deserialization value. An earlier test of this
/// property did the latter: it hand-built the struct and asserted on serialization, so deleting
/// the `#[serde(default)]` that implements the compatibility left it green. Deleting that
/// attribute now fails here.
#[tokio::test]
async fn a_server_without_standing_still_answers_the_access_question() {
    let server = MockServer::start().await;
    let body = bare_profile_body(true, false, json!(null));
    assert!(
        body["entitlements"].get("standing").is_none(),
        "the premise of this test is that the field is absent from the wire"
    );
    Mock::given(method("GET"))
        .and(path("/api/profile"))
        .respond_with(ResponseTemplate::new(200).set_body_json(&body))
        .mount(&server)
        .await;

    let client = test_client(&server.uri());
    let p = client.profile().get_with_entitlements().await.unwrap();

    assert_eq!(
        p.entitlements.standing, None,
        "absent on the wire must read as absent, never as a standing"
    );
    assert!(
        p.entitlements.system_access,
        "the access answer must survive a server that cannot name the standing"
    );
}

/// The state an earlier revision tried to withhold must survive the round trip.
#[tokio::test]
async fn revoked_is_reported_as_revoked() {
    let server = MockServer::start().await;
    let mut body = bare_profile_body(false, false, json!("approved"));
    body["entitlements"]["standing"] = json!("revoked");
    Mock::given(method("GET"))
        .and(path("/api/profile"))
        .respond_with(ResponseTemplate::new(200).set_body_json(&body))
        .mount(&server)
        .await;

    let client = test_client(&server.uri());
    let p = client.profile().get_with_entitlements().await.unwrap();

    assert_eq!(
        p.entitlements.standing,
        Some(temper_principal::Standing::Revoked),
        "revoked must survive the round trip — the CLI routes it to a different remedy"
    );
    assert!(!p.entitlements.system_access);
}
