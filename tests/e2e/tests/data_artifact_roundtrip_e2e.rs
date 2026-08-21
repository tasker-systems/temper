#![cfg(feature = "test-db")]

//! E2E: data artifact commit → show round-trip through the real API stack.
//!
//! Drives the typed `TemperClient` sub-client (`data_artifacts().commit` / `.get`) against
//! the in-process Axum server backed by an isolated `#[sqlx::test]` database — the same
//! harness pattern `resource_crud_test.rs` uses. Asserts byte-identical content round-trip
//! and field-level correctness, then exercises `supersedes` folding.

mod common;

use serde_json::json;
use sqlx::PgPool;
use temper_core::types::data_artifact::{ArtifactCommitRequest, ArtifactListParams, ArtifactView};
use temper_workflow::types::resource::ResourceCreateRequest;

/// Commit a data artifact via the API, then get it back and verify:
///
/// - content round-trips byte-identical (structural `serde_json::Value` equality)
/// - `content_hash` and `content_bytes` are stable across the round-trip
/// - all `ArtifactView` fields match the commit request (kind, intent, precedence)
/// - `is_folded` is `false` for a freshly committed artifact with no supersedes
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn data_artifact_commit_show_round_trip(pool: PgPool) {
    let app = common::setup(pool).await;

    app.client
        .profile()
        .get()
        .await
        .expect("profile pre-flight failed");

    let context = app
        .client
        .contexts()
        .create("e2e-artifact-rt", None)
        .await
        .expect("context create failed");

    let resource = app
        .client
        .resources()
        .create(&ResourceCreateRequest {
            kb_context_id: context.id.into(),
            idempotency_key: None,
            doc_type: "research".to_string(),
            origin_uri: "test://e2e/artifact-rt".to_string(),
            title: "Artifact Round-Trip Test".to_string(),
            act: Default::default(),
        })
        .await
        .expect("resource create failed");

    let content = json!({
        "measurement": "temperature",
        "value": 42.5,
        "unit": "celsius",
        "metadata": {
            "sensor": "thermocouple-1",
            "calibrated": true,
            "tags": ["lab", "upstairs"]
        }
    });

    let committed = app
        .client
        .data_artifacts()
        .commit(
            resource.id.into(),
            &ArtifactCommitRequest {
                kind: "measurement".to_string(),
                kind_owner: None,
                intent: "current".to_string(),
                precedence: 0.0,
                content: content.clone(),
                supersedes: Vec::new(),
                act: Default::default(),
            },
        )
        .await
        .expect("artifact commit failed");

    // Commit response fields
    assert_eq!(committed.artifact.artifact_kind, "measurement");
    assert_eq!(committed.artifact.intent, "current");
    assert_eq!(committed.artifact.precedence, 0.0);
    assert!(
        !committed.artifact.is_folded,
        "a freshly committed artifact must not be folded"
    );
    assert_eq!(
        committed.artifact.resource_id, resource.id,
        "the artifact must be owned by the resource it was committed to"
    );

    // Get it back
    let retrieved = app
        .client
        .data_artifacts()
        .get(resource.id.into(), committed.artifact_id.into())
        .await
        .expect("artifact get failed");

    // Byte-identical content: structural equality of the JSON value
    assert_eq!(
        retrieved.content,
        Some(content.clone()),
        "content must round-trip byte-identical through the API"
    );

    // Hash and byte-count stability across the round-trip
    assert_eq!(
        retrieved.content_hash, committed.artifact.content_hash,
        "content_hash must be stable across commit → get"
    );
    assert_eq!(
        retrieved.content_bytes, committed.artifact.content_bytes,
        "content_bytes must be stable across commit → get"
    );

    // All fields match between commit and get responses
    assert_eq!(retrieved.artifact_id, committed.artifact_id);
    assert_eq!(retrieved.resource_id, committed.artifact.resource_id);
    assert_eq!(retrieved.artifact_kind, committed.artifact.artifact_kind);
    assert_eq!(retrieved.intent, committed.artifact.intent);
    assert_eq!(retrieved.precedence, committed.artifact.precedence);
    assert_eq!(retrieved.is_folded, committed.artifact.is_folded);
    assert_eq!(retrieved.shape_state, committed.artifact.shape_state);
}

/// Commit two artifacts of the same family with intent `current`; the second declares
/// `supersedes: [first]`. Assert:
///
/// - the first artifact becomes `is_folded: true`
/// - the second artifact is `is_folded: false`
/// - the default list (no `include_folded`) returns only the live one
/// - the list with `include_folded: true` returns both
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn data_artifact_supersedes_folding(pool: PgPool) {
    let app = common::setup(pool).await;

    app.client
        .profile()
        .get()
        .await
        .expect("profile pre-flight failed");

    let context = app
        .client
        .contexts()
        .create("e2e-artifold", None)
        .await
        .expect("context create failed");

    let resource = app
        .client
        .resources()
        .create(&ResourceCreateRequest {
            kb_context_id: context.id.into(),
            idempotency_key: None,
            doc_type: "research".to_string(),
            origin_uri: "test://e2e/artifold".to_string(),
            title: "Artifact Fold Test".to_string(),
            act: Default::default(),
        })
        .await
        .expect("resource create failed");

    // First artifact
    let first = app
        .client
        .data_artifacts()
        .commit(
            resource.id.into(),
            &ArtifactCommitRequest {
                kind: "measurement".to_string(),
                kind_owner: None,
                intent: "current".to_string(),
                precedence: 0.0,
                content: json!({ "value": 1 }),
                supersedes: Vec::new(),
                act: Default::default(),
            },
        )
        .await
        .expect("first artifact commit failed");

    assert!(
        !first.artifact.is_folded,
        "first artifact must start unfolded"
    );

    // Second artifact that supersedes the first
    let second = app
        .client
        .data_artifacts()
        .commit(
            resource.id.into(),
            &ArtifactCommitRequest {
                kind: "measurement".to_string(),
                kind_owner: None,
                intent: "current".to_string(),
                precedence: 0.0,
                content: json!({ "value": 2 }),
                supersedes: vec![first.artifact_id],
                act: Default::default(),
            },
        )
        .await
        .expect("second artifact commit failed");

    assert!(
        !second.artifact.is_folded,
        "the superseding artifact must not be folded"
    );

    // Get the first back — it must now be folded
    let first_after = app
        .client
        .data_artifacts()
        .get(resource.id.into(), first.artifact_id.into())
        .await
        .expect("get first artifact after supersession");

    assert!(
        first_after.is_folded,
        "the superseded artifact must be folded after a declared supersession"
    );

    // Default list (include_folded = false) → only the live (second) artifact
    let live = app
        .client
        .data_artifacts()
        .list(
            resource.id.into(),
            &ArtifactListParams {
                kind: Some("measurement".to_string()),
                intent: None,
                include_folded: Some(false),
                counts: None,
            },
        )
        .await
        .expect("list live artifacts failed");

    let live_artifacts: Vec<ArtifactView> =
        serde_json::from_value(live).expect("deserialize live artifact list");
    assert_eq!(
        live_artifacts.len(),
        1,
        "default list must return only the live (non-folded) artifact"
    );
    assert_eq!(
        live_artifacts[0].artifact_id, second.artifact_id,
        "the live artifact must be the superseding one"
    );

    // List with include_folded = true → both artifacts
    let all = app
        .client
        .data_artifacts()
        .list(
            resource.id.into(),
            &ArtifactListParams {
                kind: Some("measurement".to_string()),
                intent: None,
                include_folded: Some(true),
                counts: None,
            },
        )
        .await
        .expect("list all artifacts failed");

    let all_artifacts: Vec<ArtifactView> =
        serde_json::from_value(all).expect("deserialize all artifact list");
    assert_eq!(
        all_artifacts.len(),
        2,
        "list with include_folded must return both the folded and live artifacts"
    );
}
