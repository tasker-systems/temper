//! The client methods added to close temper-client's gap against `openapi.json`, each pinned to
//! the wire: the verb, the path, and whatever rides the query string, body, or headers.
//!
//! The registry's parity test (`src/ops.rs`) proves each operation HAS a constant that some method
//! uses. It cannot prove the method sends the right request with it. These tests do. Same `wiremock`
//! pattern as `segments_client_test.rs`.

use std::sync::Arc;

use serde_json::json;
use uuid::Uuid;
use wiremock::matchers::{body_json, header, method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

use temper_client::auth::MemoryTokenStore;
use temper_client::TemperClient;
use temper_core::types::auditor::AuditorDispatchTickRequest;
use temper_core::types::authorship::ActInput;
use temper_core::types::graph_atlas::SliceRequest;
use temper_core::types::steward::DispatchTickRequest;

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

#[tokio::test]
async fn blob_delete_sends_delete_with_the_act_on_the_query() {
    let server = MockServer::start().await;
    let blob_id = Uuid::now_v7();
    Mock::given(method("DELETE"))
        .and(path(format!("/api/blobs/{blob_id}")))
        .and(query_param("rationale", "superseded"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({ "blob_id": blob_id, "released": true })),
        )
        .expect(1)
        .mount(&server)
        .await;

    let act = ActInput {
        rationale: Some("superseded".to_string()),
        ..Default::default()
    };
    let ack = test_client(&server.uri())
        .blobs()
        .delete(blob_id, &act)
        .await
        .expect("delete succeeds");
    assert_eq!(ack.blob_id, blob_id);
    assert!(ack.released);
}

#[tokio::test]
async fn list_citation_audits_reads_the_findings_audit_rows() {
    let server = MockServer::start().await;
    let resource_id = Uuid::now_v7();
    Mock::given(method("GET"))
        .and(path(format!(
            "/api/resources/{resource_id}/citation-audits"
        )))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([])))
        .expect(1)
        .mount(&server)
        .await;

    let rows = test_client(&server.uri())
        .resources()
        .list_citation_audits(resource_id)
        .await
        .expect("list succeeds");
    assert!(rows.is_empty());
}

#[tokio::test]
async fn graph_home_reads_the_atlas_home() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/graph/home"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({ "build": [], "research": [] })),
        )
        .expect(1)
        .mount(&server)
        .await;

    let home = test_client(&server.uri())
        .graph()
        .home()
        .await
        .expect("home succeeds");
    assert!(home.build.is_empty() && home.research.is_empty());
}

#[tokio::test]
async fn cogmap_slice_posts_the_slice_request() {
    let server = MockServer::start().await;
    let cogmap_id = Uuid::now_v7();
    let seed = Uuid::now_v7();
    let request = SliceRequest {
        seeds: vec![seed],
        depth: 2,
        edge_kinds: vec![],
    };
    Mock::given(method("POST"))
        .and(path(format!("/api/cogmaps/{cogmap_id}/graph/slice")))
        .and(body_json(
            serde_json::to_value(&request).expect("serializes"),
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "nodes": [], "edges": [] })))
        .expect(1)
        .mount(&server)
        .await;

    let slice = test_client(&server.uri())
        .graph()
        .cogmap_slice(cogmap_id, &request)
        .await
        .expect("slice succeeds");
    assert!(slice.nodes.is_empty());
}

#[tokio::test]
async fn steward_candidates_reads_the_candidate_maps() {
    let server = MockServer::start().await;
    let map = Uuid::now_v7();
    Mock::given(method("GET"))
        .and(path("/api/steward/candidates"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([map])))
        .expect(1)
        .mount(&server)
        .await;

    let ids = test_client(&server.uri())
        .steward()
        .candidates()
        .await
        .expect("candidates succeeds");
    assert_eq!(ids, vec![map]);
}

#[tokio::test]
async fn steward_dispatch_posts_the_tick_with_its_correlation_header() {
    let server = MockServer::start().await;
    let correlation = Uuid::now_v7();
    Mock::given(method("POST"))
        .and(path("/api/steward/dispatch"))
        .and(header(
            "x-steward-correlation-id",
            correlation.to_string().as_str(),
        ))
        .and(body_json(json!({ "cap": 3 })))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({ "claimed": [], "correlation_id": correlation })),
        )
        .expect(1)
        .mount(&server)
        .await;

    let request = DispatchTickRequest {
        threshold: None,
        cap: Some(3),
    };
    let tick = test_client(&server.uri())
        .steward()
        .dispatch(&request, Some(correlation))
        .await
        .expect("dispatch succeeds");
    assert_eq!(tick.correlation_id, Some(correlation));
}

#[tokio::test]
async fn auditor_dispatch_posts_the_tick_with_its_own_correlation_header() {
    let server = MockServer::start().await;
    let correlation = Uuid::now_v7();
    Mock::given(method("POST"))
        .and(path("/api/auditor/dispatch"))
        .and(header(
            "x-auditor-correlation-id",
            correlation.to_string().as_str(),
        ))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({ "claimed": [], "correlation_id": correlation })),
        )
        .expect(1)
        .mount(&server)
        .await;

    let tick = test_client(&server.uri())
        .auditor()
        .dispatch(&AuditorDispatchTickRequest::default(), Some(correlation))
        .await
        .expect("dispatch succeeds");
    assert_eq!(tick.correlation_id, Some(correlation));
}

#[tokio::test]
async fn auditor_complete_posts_to_the_cogmaps_complete_door() {
    let server = MockServer::start().await;
    let cogmap = Uuid::now_v7();
    Mock::given(method("POST"))
        .and(path(format!("/api/auditor/{cogmap}/complete")))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({ "cogmap_id": cogmap, "job_id": null })),
        )
        .expect(1)
        .mount(&server)
        .await;

    let ack = test_client(&server.uri())
        .auditor()
        .complete(cogmap)
        .await
        .expect("complete succeeds");
    assert_eq!(ack.cogmap_id, cogmap);
    assert_eq!(ack.job_id, None);
}
