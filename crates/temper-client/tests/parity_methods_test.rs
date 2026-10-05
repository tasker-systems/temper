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

mod erasure {
    use super::*;
    use temper_core::types::erasure::{
        BlockHistoryScrubExecuteResponse, BlockHistoryScrubRequestBody, ErasureExecuteRequest,
        ErasureExecuteResponse, ErasureSurveyRequest, ResourceErasureExecuteRequest,
        ResourceErasureExecuteResponse, ResourceErasureRefusalReason, ResourceErasureSurveyRequest,
    };

    #[tokio::test]
    async fn resource_erasure_completed_carries_targets_remainder_and_ledger_remainder() {
        let server = MockServer::start().await;
        let resource = Uuid::now_v7();
        let event = Uuid::now_v7();
        Mock::given(method("POST"))
            .and(path("/api/admin/resources/erasure"))
            .and(body_json(
                json!({ "resource": resource, "also_strike_blobs": null }),
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "status": "completed",
                "request_reference": Uuid::now_v7(),
                "event_id": Uuid::now_v7(),
                "folded_edges": [],
                "targets": [{ "target": "blocks", "outcome": "emptied 3" }],
                "remainder": [{ "target": "blob", "outcome": "independent_obligation" }],
                "ledger_remainder": [{ "event": event, "paths": ["/title"] }],
                "blob_strikes": [],
            })))
            .expect(1)
            .mount(&server)
            .await;

        let body = ResourceErasureExecuteRequest {
            resource,
            also_strike_blobs: None,
        };
        let answer = test_client(&server.uri())
            .admin()
            .erase_resource(&body)
            .await
            .expect("erasure answers");
        match answer {
            ResourceErasureExecuteResponse::Completed {
                targets,
                remainder,
                ledger_remainder,
                ..
            } => {
                assert_eq!(targets[0].outcome, "emptied 3");
                assert_eq!(remainder[0].outcome, "independent_obligation");
                assert_eq!(ledger_remainder[0].paths, vec!["/title".to_string()]);
            }
            other => panic!("expected a completion, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn resource_erasure_refusal_is_an_answer_not_an_error() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/admin/resources/erasure"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "status": "refused",
                "request_reference": Uuid::now_v7(),
                "event_id": Uuid::now_v7(),
                "reason": "charter_resource",
                "detail": "the resource is a cognitive map's charter",
            })))
            .expect(1)
            .mount(&server)
            .await;

        let body = ResourceErasureExecuteRequest {
            resource: Uuid::now_v7(),
            also_strike_blobs: None,
        };
        let answer = test_client(&server.uri())
            .admin()
            .erase_resource(&body)
            .await
            .expect("a refusal still answers 200");
        assert!(matches!(
            answer,
            ResourceErasureExecuteResponse::Refused {
                reason: ResourceErasureRefusalReason::CharterResource,
                ..
            }
        ));
    }

    #[tokio::test]
    async fn resource_erasure_survey_posts_to_the_survey_door() {
        let server = MockServer::start().await;
        let resource = Uuid::now_v7();
        Mock::given(method("POST"))
            .and(path("/api/admin/resources/erasure/survey"))
            .and(body_json(json!({ "resource": resource })))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "resource": resource,
                "already_erased": true,
                "plan": null,
            })))
            .expect(1)
            .mount(&server)
            .await;

        let survey = test_client(&server.uri())
            .admin()
            .survey_resource_erasure(&ResourceErasureSurveyRequest { resource })
            .await
            .expect("survey answers");
        assert!(survey.already_erased && survey.plan.is_none());
    }

    #[tokio::test]
    async fn principal_erasure_and_its_survey_post_to_their_doors() {
        let server = MockServer::start().await;
        let subject = Uuid::now_v7();
        let reference = Uuid::now_v7();
        Mock::given(method("POST"))
            .and(path("/api/admin/erasure/survey"))
            .and(body_json(json!({ "subject": subject })))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "subject": subject,
                "already_erased": false,
                "redacted_hashes": [],
                "targets": [],
                "blob_strikes": [],
            })))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/api/admin/erasure"))
            .and(body_json(
                json!({ "subject": subject, "request_reference": reference }),
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "status": "completed",
                "event_id": Uuid::now_v7(),
                "already_erased": false,
                "redacted_hashes": ["h1"],
                "targets": [],
                "blob_strikes": [],
            })))
            .expect(1)
            .mount(&server)
            .await;

        let client = test_client(&server.uri());
        let survey = client
            .admin()
            .survey_principal_erasure(&ErasureSurveyRequest { subject })
            .await
            .expect("survey answers");
        assert!(!survey.already_erased);
        let ErasureExecuteResponse::Completed {
            redacted_hashes, ..
        } = client
            .admin()
            .erase_principal(&ErasureExecuteRequest {
                subject,
                request_reference: reference,
            })
            .await
            .expect("erasure answers");
        assert_eq!(redacted_hashes, vec!["h1".to_string()]);
    }

    #[tokio::test]
    async fn block_history_scrub_and_its_survey_post_to_their_doors() {
        let server = MockServer::start().await;
        let resource = Uuid::now_v7();
        let block = Uuid::now_v7();
        let body = BlockHistoryScrubRequestBody {
            resource,
            blocks: vec![block],
        };
        Mock::given(method("POST"))
            .and(path("/api/admin/resources/block-history-scrub/survey"))
            .and(body_json(
                json!({ "resource": resource, "blocks": [block] }),
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "resource": resource,
                "refusal": null,
                "detail": null,
                "plan": { "cancels_ingest": false, "blocks": [] },
            })))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/api/admin/resources/block-history-scrub"))
            .and(body_json(
                json!({ "resource": resource, "blocks": [block] }),
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "status": "completed",
                "request_reference": Uuid::now_v7(),
                "event_id": Uuid::now_v7(),
                "targets": [],
                "cancelled_ingest": false,
            })))
            .expect(1)
            .mount(&server)
            .await;

        let client = test_client(&server.uri());
        let survey = client
            .admin()
            .survey_block_history_scrub(&body)
            .await
            .expect("survey answers");
        assert!(survey.plan.is_some());
        let answer = client
            .admin()
            .scrub_block_history(&body)
            .await
            .expect("scrub answers");
        assert!(matches!(
            answer,
            BlockHistoryScrubExecuteResponse::Completed { .. }
        ));
    }
}
