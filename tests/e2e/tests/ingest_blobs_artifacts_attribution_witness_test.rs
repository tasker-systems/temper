#![cfg(feature = "test-db")]
//! The reblock/blobs/ingest/data_artifacts(+shapes) families' attribution witness
//! (beat G4 — the G3c/G3d witness idiom): a write act through the tool stamps the
//! CALLER'S surface into the ledger — read `<handle>@mcp` back through the
//! already-crossed `element_trail` door (that tool rides the door since G3c; the
//! read is out of this beat's migration scope).
//!
//! The commit goes through THIS beat's direct-bound tool (`commit_data_artifact`):
//! pre-swap the direct tool threads `Surface::Mcp` into the backend command; at the
//! swap the wire request rides the relay's `RelayedSurface` carrier instead and the
//! backend's emitter derives from the request's resolved surface. The assertion is
//! the SAME on both sides — that is the witness.
//!
//! Act-envelope trap (task trap 8): the per-act authorship fields (`confidence`,
//! `reasoning`) map straight into the request body, never defaulted — so the write
//! carries its authorship through the door. The `commit`'s trail row is the
//! `data_artifact_committed` event on the resource's node trail.
//!
//! Bite contract: a swap's relayed commit that dropped the carrier (or mislabeled
//! it) emits `<handle>@web`, reddening the `@mcp` assertion — probed from the
//! relay-client builder, the one place the origin then lives.

mod common;

use serde_json::json;
use sqlx::PgPool;
use temper_mcp::service::TemperMcpService;
use uuid::Uuid;

/// The witness harness, once per test: the relay-ready app, the MCP service, and
/// the harness principal's relay parts — the same vehicle for every leg (a single
/// door; direct parts are gone from this family).
async fn harness(
    pool: PgPool,
) -> (
    common::E2eTestApp,
    TemperMcpService,
    axum::http::request::Parts,
) {
    let app = common::setup_relay(pool).await;
    let svc = app.mcp_relay_service().await;
    let parts = app.relay_parts();
    (app, svc, parts)
}

/// Seed a resource through the app client (the production-true path) and commit one
/// artifact to it THROUGH THE TOOL, with an authored act envelope — answering the
/// resource id for the trail read.
async fn commit_an_authored_act(
    app: &common::E2eTestApp,
    svc: &TemperMcpService,
    parts: &axum::http::request::Parts,
) -> Uuid {
    let ctx_id: Uuid = sqlx::query_scalar(
        "SELECT c.id FROM kb_contexts c JOIN kb_profiles p ON p.id = c.owner_id \
         WHERE p.email = $1 AND c.name = 'default'",
    )
    .bind("e2e@test.example.com")
    .fetch_one(&app.pool)
    .await
    .expect("the auto-provisioned default context");
    let resource = app
        .client
        .resources()
        .create(&temper_workflow::types::resource::ResourceCreateRequest {
            kb_context_id: ctx_id,
            idempotency_key: Some(Uuid::now_v7()),
            doc_type: "research".to_string(),
            origin_uri: format!("test://e2e/g4-witness/{}", Uuid::now_v7()),
            title: "G4 attribution witness".to_string(),
            act: Default::default(),
        })
        .await
        .expect("resource seed through the API");

    let res = temper_mcp::tools::data_artifacts::commit_artifact(
        svc,
        parts,
        serde_json::from_value(json!({
            "resource_id": resource.id.to_string(),
            "kind": "measurement",
            "intent": "current",
            "content": {"reading": 42},
            "confidence": "confident",
            "reasoning": "the witness carries an authored act envelope",
            "persona": "parity-author",
        }))
        .expect("input deserializes from its wire shape"),
    )
    .await
    .expect("the commit lands through the tool");
    let ack: serde_json::Value =
        serde_json::from_str(res.content[0].as_text().expect("a text part").text.as_str())
            .expect("the part is the tool's JSON response");
    assert!(
        ack["artifact_id"].as_str().is_some(),
        "the commit answers its artifact id: {ack}"
    );
    resource.id.into()
}

/// A `commit_data_artifact` through the tool lands `<handle>@mcp` at the ledger: the
/// resource's trail carries the `data_artifact_committed` act, and its
/// `actor_name` names the MCP surface — the write the swap carries unchanged.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn a_commit_artifact_through_the_tool_lands_at_mcp_in_the_ledger(pool: PgPool) {
    let (app, svc, parts) = harness(pool).await;
    let resource = commit_an_authored_act(&app, &svc, &parts).await;

    let trail = temper_mcp::tools::trail::element_trail(
        &svc,
        &parts,
        serde_json::from_value(json!({
            "kind": "node",
            "element": resource.to_string(),
        }))
        .expect("input deserializes from its wire shape"),
    )
    .await
    .expect("the trail answers");
    let v: serde_json::Value = serde_json::from_str(
        trail.content[0]
            .as_text()
            .expect("a text part")
            .text
            .as_str(),
    )
    .expect("the part is the tool's JSON response");
    let events = v["events"].as_array().expect("the events array");
    let commit_act = events
        .iter()
        .find(|e| e["kind"] == "data_artifact_committed")
        .expect("the artifact commit is on the resource's trail: {v}");

    let handle: String = sqlx::query_scalar("SELECT handle FROM kb_profiles WHERE email = $1")
        .bind("e2e@test.example.com")
        .fetch_one(&app.pool)
        .await
        .expect("the harness handle");

    assert_eq!(
        commit_act["actor_name"],
        json!(format!("{handle}@mcp")),
        "the MCP-surface emitter attributed the act, read back through the trail door: {v}"
    );
}
