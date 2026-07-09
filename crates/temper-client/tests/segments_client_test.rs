//! `IngestClient`'s segmented (multi-block) ingest sub-methods (Beat 2 Task 2.4):
//! `begin_segmented` / `append_block` / `finalize` / `list_blocks`.
//!
//! Uses `wiremock` to assert the exact method + path each call hits and that the response body
//! round-trips through the typed wire structs — the same pattern `retry_tests.rs` establishes
//! for `HttpClient::send` in this crate.

use std::sync::Arc;

use uuid::Uuid;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use temper_client::auth::MemoryTokenStore;
use temper_client::TemperClient;
use temper_core::types::ingest::{
    AppendBlockPayload, BlocksResponse, FinalizePayload, IngestPayload, SegmentInfo,
    SegmentedBegin, SegmentedBeginResponse,
};

fn test_client(base_url: &str) -> TemperClient {
    TemperClient::with_token(
        base_url,
        None,
        "test-token".to_string(),
        Arc::new(MemoryTokenStore::empty()),
    )
}

fn segmented_payload() -> IngestPayload {
    IngestPayload {
        title: "Big Doc".to_string(),
        origin_uri: "test://big-doc".to_string(),
        context_ref: "@me/temper".to_string(),
        home_cogmap_id: None,
        doc_type_name: "research".to_string(),
        goal: None,
        content_hash: None,
        content: "first segment".to_string(),
        metadata: None,
        managed_meta: None,
        open_meta: None,
        chunks_packed: Some("b64chunks".to_string()),
        sources: Vec::new(),
        act: Default::default(),
        segmented: Some(SegmentedBegin {
            total_blocks_hint: Some(3),
            block_budget: 262_144,
            source_hash: Some("deadbeef".to_string()),
        }),
    }
}

#[tokio::test]
async fn begin_segmented_posts_to_ingest_and_parses_response() {
    let server = MockServer::start().await;
    let resource_id = Uuid::now_v7();
    let correlation_id = Uuid::now_v7();
    let response = SegmentedBeginResponse {
        resource_id,
        correlation_id,
        blocks: vec![SegmentInfo {
            seq: 0,
            content_hash: "h0".to_string(),
        }],
    };

    Mock::given(method("POST"))
        .and(path("/api/ingest"))
        .respond_with(ResponseTemplate::new(200).set_body_json(&response))
        .expect(1)
        .mount(&server)
        .await;

    let client = test_client(&server.uri());
    let got = client
        .ingest()
        .begin_segmented(&segmented_payload())
        .await
        .expect("begin_segmented should succeed");
    assert_eq!(got.resource_id, resource_id);
    assert_eq!(got.correlation_id, correlation_id);
    assert_eq!(got.blocks.len(), 1);
    assert_eq!(got.blocks[0].seq, 0);
}

#[tokio::test]
async fn append_block_posts_to_resources_blocks_path() {
    let server = MockServer::start().await;
    let resource_id = Uuid::now_v7();
    let response = BlocksResponse {
        blocks: vec![
            SegmentInfo {
                seq: 0,
                content_hash: "h0".to_string(),
            },
            SegmentInfo {
                seq: 1,
                content_hash: "h1".to_string(),
            },
        ],
        body_hash: "sha256:live".to_string(),
    };

    Mock::given(method("POST"))
        .and(path(format!("/api/resources/{resource_id}/blocks")))
        .respond_with(ResponseTemplate::new(200).set_body_json(&response))
        .expect(1)
        .mount(&server)
        .await;

    let client = test_client(&server.uri());
    let payload = AppendBlockPayload {
        seq: 1,
        content: "second segment".to_string(),
        content_hash: temper_core::hash::sha256_hex(b"second segment"),
        chunks_packed: "b64chunks".to_string(),
    };
    let got = client
        .ingest()
        .append_block(resource_id, &payload)
        .await
        .expect("append_block should succeed");
    assert_eq!(got.blocks.len(), 2);
    assert_eq!(got.blocks[1].seq, 1);
    assert_eq!(
        got.body_hash, "sha256:live",
        "the echo-back body_hash must be parsed off the append response"
    );
}

#[tokio::test]
async fn finalize_posts_to_resources_finalize_path_and_ignores_empty_body() {
    let server = MockServer::start().await;
    let resource_id = Uuid::now_v7();

    Mock::given(method("POST"))
        .and(path(format!("/api/resources/{resource_id}/finalize")))
        .respond_with(ResponseTemplate::new(204))
        .expect(1)
        .mount(&server)
        .await;

    let client = test_client(&server.uri());
    client
        .ingest()
        .finalize(
            resource_id,
            &FinalizePayload {
                expected_blocks: 2,
                expected_body_hash: "sha256:abc".to_string(),
            },
        )
        .await
        .expect("finalize should succeed on 204 with no body");
}

#[tokio::test]
async fn list_blocks_gets_resources_blocks_path() {
    let server = MockServer::start().await;
    let resource_id = Uuid::now_v7();
    let response = BlocksResponse {
        blocks: vec![SegmentInfo {
            seq: 0,
            content_hash: "h0".to_string(),
        }],
        body_hash: "sha256:live".to_string(),
    };

    Mock::given(method("GET"))
        .and(path(format!("/api/resources/{resource_id}/blocks")))
        .respond_with(ResponseTemplate::new(200).set_body_json(&response))
        .expect(1)
        .mount(&server)
        .await;

    let client = test_client(&server.uri());
    let got = client
        .ingest()
        .list_blocks(resource_id)
        .await
        .expect("list_blocks should succeed");
    assert_eq!(got.blocks.len(), 1);
    assert_eq!(got.blocks[0].seq, 0);
}
