//! `AdminClient::reblock` — the corpus re-blocking step.
//!
//! Uses `wiremock` to assert the exact method + path each call hits and that the request body
//! carries the discriminated scope shape on the wire (`{"resource": <id>}` object arm, bare
//! `"all"` string arm) and the response round-trips through the typed wire structs — the same
//! pattern `segments_client_test.rs` establishes in this crate.

use std::sync::Arc;

use uuid::Uuid;
use wiremock::matchers::{body_json, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use temper_client::auth::MemoryTokenStore;
use temper_client::TemperClient;
use temper_core::types::reblock::{
    ReblockCandidate, ReblockOutcome, ReblockReceipt, ReblockRequest, ReblockSummary,
};

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

fn receipt() -> ReblockReceipt {
    ReblockReceipt {
        dry_run: false,
        correlation_id: Uuid::now_v7(),
        outcomes: vec![
            ReblockCandidate {
                resource: Uuid::now_v7(),
                outcome: ReblockOutcome::Reblocked {
                    event: Uuid::now_v7(),
                },
            },
            ReblockCandidate {
                resource: Uuid::now_v7(),
                outcome: ReblockOutcome::Denied,
            },
        ],
        summary: ReblockSummary {
            reblocked: 1,
            would_change: 0,
            no_op: 0,
            declined: 1,
            error: 0,
        },
        after_id: Some(Uuid::now_v7()),
    }
}

/// The resource arm names its target as a single-key object — the externally-tagged scope's
/// wire shape — and the optional fields ride only when present.
#[tokio::test]
async fn reblock_posts_the_resource_scope_body_to_resources_reblock() {
    let server = MockServer::start().await;
    let resource_id = Uuid::now_v7();
    let cursor = Uuid::now_v7();
    let expected_receipt = receipt();

    let expected_body = serde_json::json!({
        "scope": {"resource": resource_id},
        "dry_run": true,
        "limit": 50,
        "after_id": cursor,
    });
    Mock::given(method("POST"))
        .and(path("/api/resources/reblock"))
        .and(body_json(expected_body))
        .respond_with(ResponseTemplate::new(200).set_body_json(&expected_receipt))
        .expect(1)
        .mount(&server)
        .await;

    let client = test_client(&server.uri());
    let body = ReblockRequest {
        scope: temper_core::types::reblock::ReblockScope::Resource(resource_id),
        dry_run: true,
        limit: Some(50),
        after_id: Some(cursor),
    };
    let got = client
        .admin()
        .reblock(&body)
        .await
        .expect("reblock should succeed");
    assert_eq!(got.summary.reblocked, 1);
    assert_eq!(got.summary.declined, 1);
    assert_eq!(got.outcomes.len(), 2);
    assert!(got.after_id.is_some());
}

/// The deployment-wide arm is the bare string `"all"` — no object wrapper — and omitted
/// optionals are dropped from the wire entirely (`skip_serializing_if`).
#[tokio::test]
async fn reblock_all_scope_serializes_as_the_bare_string_and_receipt_round_trips() {
    let server = MockServer::start().await;
    let expected_receipt = receipt();

    Mock::given(method("POST"))
        .and(path("/api/resources/reblock"))
        .and(body_json(serde_json::json!({
            "scope": "all",
            "dry_run": false,
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(&expected_receipt))
        .expect(1)
        .mount(&server)
        .await;

    let client = test_client(&server.uri());
    let body = ReblockRequest {
        scope: temper_core::types::reblock::ReblockScope::All,
        dry_run: false,
        limit: None,
        after_id: None,
    };
    let got = client
        .admin()
        .reblock(&body)
        .await
        .expect("reblock should succeed");
    assert_eq!(got, expected_receipt);
}
