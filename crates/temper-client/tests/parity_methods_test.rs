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

mod reads {
    use super::*;
    use temper_core::types::query_params::{
        CogmapPanoramaQuery, ConnectionsQuery, ContextCompositionQuery, ContextPanoramaQuery,
        DeltaQuery, RegionCompositionQuery, SweepQuery,
    };

    #[tokio::test]
    async fn context_panorama_carries_its_query() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/graph/contexts/panorama"))
            .and(query_param("context_ref", "+team/core"))
            .and(query_param("depth", "2"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "containers": [],
                "residual": { "group_key": "doc_type", "buckets": [] },
                "group_keys": [],
            })))
            .expect(1)
            .mount(&server)
            .await;

        let query = ContextPanoramaQuery {
            context_ref: "+team/core".to_string(),
            group_by: None,
            container_types: None,
            depth: Some(2),
        };
        let panorama = test_client(&server.uri())
            .graph()
            .context_panorama(&query)
            .await
            .expect("panorama answers");
        assert!(panorama.containers.is_empty());
    }

    #[tokio::test]
    async fn context_and_region_composition_carry_their_queries() {
        let server = MockServer::start().await;
        let subgraph = json!({ "nodes": [], "edges": [] });
        Mock::given(method("GET"))
            .and(path("/api/graph/contexts/composition"))
            .and(query_param("context_ref", "@me/temper"))
            .respond_with(ResponseTemplate::new(200).set_body_json(subgraph.clone()))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/api/graph/regions/composition"))
            .and(query_param("ids", "a,b"))
            .respond_with(ResponseTemplate::new(200).set_body_json(subgraph))
            .expect(1)
            .mount(&server)
            .await;

        let client = test_client(&server.uri());
        client
            .graph()
            .context_composition(&ContextCompositionQuery {
                context_ref: "@me/temper".to_string(),
                container: None,
                group: None,
                container_types: None,
                depth: None,
                container_depth: None,
            })
            .await
            .expect("context composition answers");
        client
            .graph()
            .region_composition(&RegionCompositionQuery {
                ids: "a,b".to_string(),
                depth: None,
            })
            .await
            .expect("region composition answers");
    }

    #[tokio::test]
    async fn cogmap_panorama_names_the_map_in_the_path() {
        let server = MockServer::start().await;
        let cogmap = Uuid::now_v7();
        Mock::given(method("GET"))
            .and(path(format!("/api/graph/cogmaps/{cogmap}/panorama")))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "territories": [],
                "orphan_nodes": [],
                "bridges": [],
            })))
            .expect(1)
            .mount(&server)
            .await;

        let overview = test_client(&server.uri())
            .graph()
            .cogmap_panorama(cogmap, &CogmapPanoramaQuery { lens_id: None })
            .await
            .expect("panorama answers");
        assert!(overview.territories.is_empty());
    }

    #[tokio::test]
    async fn list_connections_carries_the_limit() {
        let server = MockServer::start().await;
        let resource = Uuid::now_v7();
        Mock::given(method("GET"))
            .and(path(format!("/api/resources/{resource}/connections")))
            .and(query_param("limit", "10"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "rows": [], "total": 0, "limit": 10, "returned": 0, "truncated": false,
            })))
            .expect(1)
            .mount(&server)
            .await;

        let page = test_client(&server.uri())
            .resources()
            .list_connections(resource, &ConnectionsQuery { limit: Some(10) })
            .await
            .expect("connections answer");
        assert!(!page.truncated);
    }

    #[tokio::test]
    async fn the_sweeps_carry_their_bounds() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/auditor/sweep"))
            .and(query_param("cap", "5"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!([])))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/api/steward/sweep"))
            .and(query_param("threshold", "3"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!([])))
            .expect(1)
            .mount(&server)
            .await;

        let client = test_client(&server.uri());
        assert!(client
            .auditor()
            .sweep(&SweepQuery { cap: Some(5) })
            .await
            .expect("auditor sweep answers")
            .is_empty());
        assert!(client
            .steward()
            .sweep(&DeltaQuery { threshold: Some(3) })
            .await
            .expect("steward sweep answers")
            .is_empty());
    }

    #[tokio::test]
    async fn the_schema_reads_reach_their_doors() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/schema/doc-types"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!([
                { "name": "task", "has_schema": true, "required_fields": ["title"] }
            ])))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/api/schema/doc-types/task"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "name": "task",
                "schema": {},
                "required_fields": [],
                "enum_fields": { "temper-stage": ["backlog", "done"] },
                "example_managed_meta": {},
            })))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/api/schema/open-meta"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "schema": {},
                "discouraged_keys": [{ "key": "status", "use_instead": "temper-status" }],
            })))
            .expect(1)
            .mount(&server)
            .await;

        let client = test_client(&server.uri());
        let schema = client.schema();
        assert_eq!(schema.list_doc_types().await.expect("list")[0].name, "task");
        let task = schema.describe_doc_type("task").await.expect("describe");
        assert_eq!(task.enum_fields["temper-stage"], vec!["backlog", "done"]);
        let convention = schema.describe_open_meta().await.expect("open meta");
        assert_eq!(convention.discouraged_keys[0].use_instead, "temper-status");
    }

    /// FAILS IF a doc-type name can steer the request off its door: a traversal stays one
    /// encoded segment under `/api/schema/doc-types/`, and nothing reaches the admin ledger.
    #[tokio::test]
    async fn a_doc_type_name_cannot_traverse_to_another_door() {
        let server = MockServer::start().await;
        let client = test_client(&server.uri());
        let _ = client
            .schema()
            .describe_doc_type("../../admin/ledger")
            .await;
        let requests = server.received_requests().await.expect("recording is on");
        let paths: Vec<_> = requests.iter().map(|r| r.url.path().to_owned()).collect();
        assert_eq!(
            paths,
            vec!["/api/schema/doc-types/%2E%2E%2F%2E%2E%2Fadmin%2Fledger".to_owned()]
        );
    }

    /// The health door is unauthenticated: the probe must not send a token, so a logged-out
    /// caller can still ask whether the service is up.
    #[tokio::test]
    async fn health_reads_without_a_token() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/health"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "status": "ok", "version": "0.6.0", "commit": null,
            })))
            .expect(1)
            .mount(&server)
            .await;

        let health = test_client(&server.uri())
            .health()
            .get_health()
            .await
            .expect("health answers");
        assert_eq!(health.status, "ok");
        assert_eq!(health.commit, None);
        let requests = server.received_requests().await.expect("recorded");
        assert!(
            requests[0].headers.get("authorization").is_none(),
            "the health probe carries no bearer token"
        );
    }
}
