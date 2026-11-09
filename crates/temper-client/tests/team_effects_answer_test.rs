//! The three team acts that ask for their `public_within` effects (`?answer=effects`), against a
//! server that answers with them and one that predates the parameter. Same `wiremock` pattern as
//! `invocations_client_test.rs`.

use std::sync::Arc;

use uuid::Uuid;
use wiremock::matchers::{method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

use temper_client::auth::MemoryTokenStore;
use temper_client::TemperClient;
use temper_core::types::team::TeamCreateRequest;

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

fn team_row(slug: &str) -> serde_json::Value {
    serde_json::json!({
        "id": Uuid::now_v7(), "slug": slug, "name": slug, "description": null,
        "created": "2026-11-08T00:00:00Z", "auto_join_role": null,
    })
}

/// A server that predates the parameter ignores it: `204` on the deletes, the bare row on create.
/// The client reports "no effects reported" (`None`), never an error and never "nothing happened".
///
/// FAILS IF the client stops asking for the effects, or treats an older server's `204` or bare row
/// as a failure.
#[tokio::test]
async fn an_older_server_degrades_to_no_effects_reported() {
    let server = MockServer::start().await;
    let (team, child) = (Uuid::now_v7(), Uuid::now_v7());
    Mock::given(method("DELETE"))
        .and(path(format!("/api/teams/{team}")))
        .and(query_param("answer", "effects"))
        .respond_with(ResponseTemplate::new(204))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("DELETE"))
        .and(path(format!("/api/teams/{team}/children/{child}")))
        .and(query_param("answer", "effects"))
        .respond_with(ResponseTemplate::new(204))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/teams"))
        .and(query_param("answer", "effects"))
        .respond_with(ResponseTemplate::new(201).set_body_json(team_row("old")))
        .expect(1)
        .mount(&server)
        .await;

    let teams = test_client(&server.uri());
    let teams = teams.teams();
    assert_eq!(teams.delete(team).await.expect("delete"), None);
    assert_eq!(teams.detach_child(team, child).await.expect("detach"), None);
    let row = teams
        .create(&TeamCreateRequest {
            slug: "old".to_owned(),
            ..Default::default()
        })
        .await
        .expect("create");
    assert!(row.public_within.is_none() && row.public_within_in_reach.is_none());
}

/// A current server answers the effects, and the client returns them.
#[tokio::test]
async fn a_current_server_answers_the_effects() {
    let server = MockServer::start().await;
    let team = Uuid::now_v7();
    Mock::given(method("DELETE"))
        .and(path(format!("/api/teams/{team}")))
        .and(query_param("answer", "effects"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "lapsed": [], "dormant": [], "revived": [],
            "lapsed_unnamed": 0, "dormant_unnamed": 2, "revived_unnamed": 0,
        })))
        .mount(&server)
        .await;
    let effects = test_client(&server.uri())
        .teams()
        .delete(team)
        .await
        .expect("delete")
        .expect("the effects");
    assert_eq!(effects.dormant_unnamed, 2);
}
