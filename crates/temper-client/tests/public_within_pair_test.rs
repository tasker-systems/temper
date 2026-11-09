//! `TeamsClient::resolve_public_within_pair`, the (team, anchor) resolution `temper team
//! public-within` and the MCP `public_within_open`/`public_within_close` acts share. Same
//! `wiremock` pattern as `team_effects_answer_test.rs`.

use std::sync::Arc;

use uuid::Uuid;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use temper_client::auth::MemoryTokenStore;
use temper_client::teams::{PublicWithinPairError, PublicWithinSide};
use temper_client::TemperClient;

fn test_client(base_url: &str) -> TemperClient {
    TemperClient::with_token(
        base_url,
        None,
        temper_workflow::operations::Surface::CliCloud,
        "test-token".to_string(),
        Arc::new(MemoryTokenStore::empty()),
    )
    .expect("test server URL (loopback) validates")
}

fn team_row(id: Uuid, slug: &str) -> serde_json::Value {
    serde_json::json!({
        "id": id, "slug": slug, "name": slug, "description": null,
        "created": "2026-11-08T00:00:00Z", "auto_join_role": null,
    })
}

fn team_ref(id: Uuid, slug: &str) -> serde_json::Value {
    serde_json::json!({ "id": id, "slug": slug, "name": slug })
}

/// A team detail with the given `parents` and `public_within_in` rows, and nothing else.
fn detail(
    id: Uuid,
    slug: &str,
    parents: serde_json::Value,
    opened_in: serde_json::Value,
) -> serde_json::Value {
    serde_json::json!({
        "id": id, "slug": slug, "name": slug, "description": null,
        "created": "2026-11-08T00:00:00Z", "auto_join_role": null, "members": [],
        "parents": parents, "public_within_in": opened_in,
    })
}

async fn mount_list(server: &MockServer, rows: serde_json::Value) {
    Mock::given(method("GET"))
        .and(path("/api/teams"))
        .respond_with(ResponseTemplate::new(200).set_body_json(rows))
        .mount(server)
        .await;
}

/// Two ids resolve offline: no listing, no detail.
///
/// FAILS IF an id-bearing ref (bare UUID or decorated `slug-<uuid>`) costs a request.
#[tokio::test]
async fn ids_resolve_without_the_network() {
    let server = MockServer::start().await;
    let (t, a) = (Uuid::now_v7(), Uuid::now_v7());
    let pair = test_client(&server.uri())
        .teams()
        .resolve_public_within_pair(&t.to_string(), &format!("+anchor-{a}"))
        .await
        .expect("both ids resolve");
    assert_eq!(pair, (t, a));
    assert!(server
        .received_requests()
        .await
        .expect("the mock server records requests")
        .is_empty());
}

/// Two slugs on the caller's own teams resolve from one listing.
#[tokio::test]
async fn slugs_resolve_from_one_listing() {
    let server = MockServer::start().await;
    let (t, a) = (Uuid::now_v7(), Uuid::now_v7());
    Mock::given(method("GET"))
        .and(path("/api/teams"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(serde_json::json!([team_row(t, "eng"), team_row(a, "org")])),
        )
        .expect(1)
        .mount(&server)
        .await;
    let pair = test_client(&server.uri())
        .teams()
        .resolve_public_within_pair(" +eng ", "org")
        .await
        .expect("both slugs are the caller's");
    assert_eq!(pair, (t, a));
}

/// An anchor slug the caller is not on resolves through the team's parents.
#[tokio::test]
async fn an_anchor_resolves_through_the_teams_parents() {
    let server = MockServer::start().await;
    let (t, a) = (Uuid::now_v7(), Uuid::now_v7());
    mount_list(&server, serde_json::json!([team_row(t, "eng")])).await;
    Mock::given(method("GET"))
        .and(path(format!("/api/teams/{t}")))
        .respond_with(ResponseTemplate::new(200).set_body_json(detail(
            t,
            "eng",
            serde_json::json!([team_ref(a, "org")]),
            serde_json::json!([]),
        )))
        .mount(&server)
        .await;
    let pair = test_client(&server.uri())
        .teams()
        .resolve_public_within_pair("eng", "org")
        .await
        .expect("the anchor is the team's parent");
    assert_eq!(pair, (t, a));
}

/// A team slug the anchor's manager is not on resolves through the anchor's `public_within_in`.
#[tokio::test]
async fn a_team_resolves_through_the_anchors_openings() {
    let server = MockServer::start().await;
    let (t, a) = (Uuid::now_v7(), Uuid::now_v7());
    mount_list(&server, serde_json::json!([team_row(a, "org")])).await;
    Mock::given(method("GET"))
        .and(path(format!("/api/teams/{a}")))
        .respond_with(ResponseTemplate::new(200).set_body_json(detail(
            a,
            "org",
            serde_json::json!([]),
            serde_json::json!([{ "team": team_ref(t, "eng"), "effective": true }]),
        )))
        .mount(&server)
        .await;
    let pair = test_client(&server.uri())
        .teams()
        .resolve_public_within_pair("eng", "org")
        .await
        .expect("the team is opened to the anchor");
    assert_eq!(pair, (t, a));
}

/// A slug nothing readable names is `NotATeam`, naming the side and the bare slug.
#[tokio::test]
async fn an_unknown_slug_is_not_a_team() {
    let server = MockServer::start().await;
    let a = Uuid::now_v7();
    mount_list(&server, serde_json::json!([])).await;
    Mock::given(method("GET"))
        .and(path(format!("/api/teams/{a}")))
        .respond_with(ResponseTemplate::new(200).set_body_json(detail(
            a,
            "org",
            serde_json::json!([]),
            serde_json::json!([]),
        )))
        .mount(&server)
        .await;
    let err = test_client(&server.uri())
        .teams()
        .resolve_public_within_pair("+ghost", &a.to_string())
        .await
        .expect_err("no readable team is called ghost");
    match err {
        PublicWithinPairError::NotATeam { side, slug } => {
            assert_eq!(side, PublicWithinSide::Team);
            assert_eq!(slug, "ghost");
        }
        other => panic!("expected NotATeam, got {other:?}"),
    }
}

/// A failed listing is the caller's own error, never "not a team".
///
/// FAILS IF a refused or failed listing is reported as an unresolved ref.
#[tokio::test]
async fn a_listing_failure_is_the_callers_error() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/teams"))
        .respond_with(ResponseTemplate::new(403))
        .mount(&server)
        .await;
    let err = test_client(&server.uri())
        .teams()
        .resolve_public_within_pair("eng", "org")
        .await
        .expect_err("the listing was refused");
    assert!(
        matches!(err, PublicWithinPairError::Client(_)),
        "expected Client, got {err:?}"
    );
}

/// A failed read of the resolved side's detail is the caller's own error, never "not a team".
///
/// FAILS IF a refused detail read is reported as an unresolved ref.
#[tokio::test]
async fn a_detail_failure_is_the_callers_error() {
    let server = MockServer::start().await;
    let a = Uuid::now_v7();
    mount_list(&server, serde_json::json!([])).await;
    Mock::given(method("GET"))
        .and(path(format!("/api/teams/{a}")))
        .respond_with(ResponseTemplate::new(403))
        .expect(1)
        .mount(&server)
        .await;
    let err = test_client(&server.uri())
        .teams()
        .resolve_public_within_pair("eng", &a.to_string())
        .await
        .expect_err("the anchor's detail was refused");
    assert!(
        matches!(err, PublicWithinPairError::Client(_)),
        "expected Client, got {err:?}"
    );
}

/// The `NotATeam` refusal text, which the CLI prints and the MCP prefixes with its act.
///
/// FAILS IF the refusal both surfaces speak changes.
#[tokio::test]
async fn not_a_team_renders_the_surfaces_refusal() {
    let server = MockServer::start().await;
    let t = Uuid::now_v7();
    mount_list(&server, serde_json::json!([])).await;
    Mock::given(method("GET"))
        .and(path(format!("/api/teams/{t}")))
        .respond_with(ResponseTemplate::new(200).set_body_json(detail(
            t,
            "eng",
            serde_json::json!([]),
            serde_json::json!([]),
        )))
        .mount(&server)
        .await;
    let err = test_client(&server.uri())
        .teams()
        .resolve_public_within_pair(&t.to_string(), "+ghost")
        .await
        .expect_err("no readable team is called ghost");
    assert_eq!(
        err.to_string(),
        "not a team: \"ghost\" (the anchor; expected a team slug, a decorated `slug-<uuid>` ref, \
         or a team UUID — name a team you are not a member of by its UUID)"
    );
}
