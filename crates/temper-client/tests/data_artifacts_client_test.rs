//! `DataArtifactsClient::list_all` against a server that predates `limit`.
//!
//! Uses `wiremock`, the pattern `reblock_client_test.rs` establishes in this crate. A server
//! without the bounded list ignores `limit` and answers the bare array of every artifact; the
//! client must read that as the final page rather than fail to decode it.

use std::sync::Arc;

use uuid::Uuid;
use wiremock::matchers::{method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

use temper_client::auth::MemoryTokenStore;
use temper_client::TemperClient;
use temper_core::types::data_artifact::{
    ArtifactListParams, ArtifactView, ARTIFACT_DRAIN_PAGE_ROWS,
};
use temper_core::types::ids::{DataArtifactId, ResourceId};

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

fn artifact(resource: Uuid, n: i64) -> ArtifactView {
    ArtifactView {
        artifact_id: DataArtifactId::new(),
        resource_id: ResourceId::from(resource),
        kind_owner_table: "kb_profiles".to_string(),
        kind_owner_id: Uuid::now_v7(),
        artifact_kind: "measurement".to_string(),
        intent: "member".to_string(),
        precedence: 0.0,
        content_hash: format!("hash-{n}"),
        content_bytes: 1,
        shape_state: "never_declared".to_string(),
        is_folded: false,
        created: chrono::Utc::now(),
        content: Some(serde_json::json!({ "n": n })),
    }
}

/// A new client against an old server: the old route ignores `limit` and answers every artifact
/// as a bare array. `list_all` returns all of them and asks exactly once, because that answer is
/// read as the final page (`next: None`), neither truncated nor followed.
#[tokio::test]
async fn list_all_reads_an_older_servers_bare_array_as_the_final_page() {
    let resource = Uuid::now_v7();
    // More than one drain page: an old server returns everything regardless of `limit`.
    let everything: Vec<ArtifactView> = (0..i64::from(ARTIFACT_DRAIN_PAGE_ROWS) + 3)
        .map(|n| artifact(resource, n))
        .collect();

    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(format!("/api/resources/{resource}/artifacts")))
        .and(query_param("limit", ARTIFACT_DRAIN_PAGE_ROWS.to_string()))
        .respond_with(ResponseTemplate::new(200).set_body_json(&everything))
        .expect(1)
        .mount(&server)
        .await;

    let drained = test_client(&server.uri())
        .data_artifacts()
        .list_all(resource, &ArtifactListParams::default())
        .await
        .expect("a bare array is a readable answer");

    assert_eq!(
        drained.iter().map(|a| a.artifact_id).collect::<Vec<_>>(),
        everything.iter().map(|a| a.artifact_id).collect::<Vec<_>>(),
        "every element, in order, none dropped"
    );
    // wiremock verifies `.expect(1)` on drop: exactly one request was made.
}
