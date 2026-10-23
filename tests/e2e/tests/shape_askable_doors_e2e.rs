#![cfg(feature = "test-db")]

//! E2E: a shape's `askable` vocabulary rides the by-id read at every door, and no listing (spec
//! `temper-artifacts:specs/2026-10-19-asking-artifacts-by-what-they-are-design.md` §2.5).
//!
//! The by-id read is one service read (`substrate_read::get_shape`) behind three doors — `GET
//! /api/shapes/{id}`, MCP `get_data_artifact_shape` (relayed to that route), and `temper
//! data-artifact schema show` (a client of it) — so the three answer one vocabulary, and the
//! listing route answers none: the field absent, not empty.

mod common;

use serde_json::{json, Value};
use sqlx::PgPool;
use temper_core::types::data_artifact_shape::{EnforcementMode, ShapeDeclareRequest};
use temper_workflow::types::resource::ResourceCreateRequest;

/// Extract the text of the first content part of a tool result.
fn text_of(result: rmcp::model::CallToolResult) -> String {
    match result.content.first() {
        Some(rmcp::model::ContentBlock::Text(t)) => t.text.clone(),
        other => panic!("tool returned no text content part: {other:?}"),
    }
}

/// The by-id read at all three doors carries the same `askable` — the one path the schema
/// declares, typed — and the context listing carries none. Bites a door that drops the field (a
/// typed copy of `ShapeView` without it), or a listing that computes it.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn askable_rides_the_by_id_read_at_every_door_and_no_listing(pool: PgPool) {
    let app = common::setup_relay(pool).await;
    let context = app
        .client
        .contexts()
        .create("e2e-shape-askable", None)
        .await
        .expect("context create failed");
    // A resource in the context, so the declare's family owner defaults.
    app.client
        .resources()
        .create(&ResourceCreateRequest {
            kb_context_id: context.id.into(),
            idempotency_key: None,
            doc_type: "research".to_string(),
            origin_uri: "test://e2e/shape-askable".to_string(),
            title: "shape-askable subject".to_string(),
            act: Default::default(),
        })
        .await
        .expect("resource create failed");
    let declared = app
        .client
        .data_artifacts()
        .declare_shape(
            *context.id,
            &ShapeDeclareRequest {
                kind: "measurement".to_string(),
                kind_owner: None,
                schema: json!({
                    "$schema": "https://json-schema.org/draft/2020-12/schema",
                    "type": "object",
                    "properties": { "value": { "type": "string" } }
                }),
                enforcement: EnforcementMode::Advisory,
                act: Default::default(),
            },
        )
        .await
        .expect("declare");
    let shape_id = declared.shape_id.uuid();
    let expected = json!({
        "paths": [{
            "path": ["value"],
            "facts": { "types": ["string"] },
            "operators": ["has_key", "contains", "compare"]
        }]
    });

    // HTTP, read as the raw body, so a field the wire carries as `null` is seen, not defaulted
    let http = raw_json(&app, &format!("/api/shapes/{shape_id}")).await;
    assert_eq!(http["askable"], expected, "{http}");
    assert!(http.get("unaskable").is_none(), "{http}");

    // MCP
    let svc = app.mcp_relay_service().await;
    let result = temper_mcp::tools::data_artifact_shapes::get_shape(
        &svc,
        &app.relay_parts(),
        temper_mcp::tools::data_artifact_shapes::GetShapeInput {
            shape_id: shape_id.to_string(),
        },
    )
    .await
    .expect("get shape over mcp");
    let mcp: Value = serde_json::from_str(&text_of(result)).expect("tool result is ShapeView JSON");
    assert_eq!(mcp["askable"], expected, "{mcp}");

    // CLI
    let output = common::run_temper_cli(
        &app,
        &[
            "data-artifact",
            "schema",
            "show",
            &shape_id.to_string(),
            "--format",
            "json",
        ],
    )
    .await
    .expect("cli run");
    assert!(
        output.status.success(),
        "schema show must succeed: stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    let cli: Value = serde_json::from_slice(&output.stdout).expect("schema show prints JSON");
    assert_eq!(cli["askable"], expected, "{cli}");

    // The listing: absent from the wire, not empty and not null.
    let listed = raw_json(&app, &format!("/api/contexts/{}/shapes", *context.id)).await;
    let listed = listed.as_array().expect("the listing is an array");
    assert_eq!(listed.len(), 1);
    assert!(listed[0].get("askable").is_none(), "{}", listed[0]);
    assert!(listed[0].get("unaskable").is_none(), "{}", listed[0]);
}

/// A GET on the app's own listener with the harness bearer, its body parsed as JSON untyped.
async fn raw_json(app: &common::E2eTestApp, path: &str) -> Value {
    let response = app
        .reqwest_client
        .get(app.url(path))
        .bearer_auth(&app.token)
        .send()
        .await
        .expect("request");
    assert!(
        response.status().is_success(),
        "{path}: {}",
        response.status()
    );
    response.json().await.expect("a JSON body")
}
