#![cfg(feature = "test-db")]
//! The ledger + graph family's attribution witness — the per-family bite the
//! network door demands (learning 8 of the beat).
//!
//! A facet write through the driven path IS an authored act: the tool stamps
//! `origin: Surface::Mcp` on the command (`crates/temper-mcp/src/tools/facets.rs:100`),
//! the backend resolves the durable per-surface emitter entity
//! `<handle>@mcp` (`temper-substrate/src/writes.rs:52-63`, `e.name = p.handle || '@mcp'`),
//! and the act lands on the ledger under that entity. This witness writes a facet
//! through the family's own driver and reads the ledger back through the family's
//! own read tool — `element_trail`, whose rows carry the acting event's emitter
//! name — asserting the facet act's actor is `<handle>@mcp`.
//!
//! The assertion holds in BOTH binding states by construction: pre-swap the
//! `Surface::Mcp` comes from the tool's own command, post-swap from the relay's
//! planted carrier at the door — the witness is authored now so the swap cannot
//! silently re-attribute the family's acts.
//!
//! The bite: drop the `Surface::Mcp` stamp and the backend resolves a different
//! emitter (or the write fails outright) — the trail read reddens. A witness that
//! cannot fail is decoration.

mod common;

use serde_json::json;
use sqlx::PgPool;
use temper_core::types::ingest::{pack_chunks, IngestPayload};
use temper_mcp::service::TemperMcpService;

/// The harness principal's handle — the `<handle>` half of the per-surface emitter.
async fn handle_of(pool: &PgPool) -> String {
    sqlx::query_scalar("SELECT handle FROM kb_profiles WHERE email = $1")
        .bind("e2e@test.example.com")
        .fetch_one(pool)
        .await
        .expect("the harness principal's handle")
}

async fn harness(
    pool: PgPool,
) -> (
    common::E2eTestApp,
    TemperMcpService,
    axum::http::request::Parts,
) {
    let app = common::setup_relay(pool).await;
    let svc = app.mcp_relay_service(app.pool.clone()).await;
    let parts = app.relay_parts();
    (app, svc, parts)
}

/// Ingest a resource through the harness's own client and return its id.
async fn ingest(app: &common::E2eTestApp, title: &str, content: &str) -> uuid::Uuid {
    let chunks = common::chunked(content, 0.1);
    let payload = IngestPayload {
        idempotency_key: None,
        segmented: None,
        goal: None,
        title: title.to_string(),
        origin_uri: format!("test://ledger-graph-witness/{title}"),
        context_ref: "@me/default".to_string(),
        home_cogmap_id: None,
        doc_type_name: "research".to_string(),
        content_hash: Some(temper_core::hash::sha256_hex(content.as_bytes())),
        content: content.to_string(),
        metadata: None,
        managed_meta: None,
        open_meta: None,
        chunks_packed: Some(pack_chunks(&chunks).expect("pack chunks")),
        act: Default::default(),
        sources: Vec::new(),
    };
    app.client
        .ingest()
        .create(&payload)
        .await
        .expect("ingest lands")
        .id
        .uuid()
}

/// A facet WRITE through the driven path lands `<handle>@mcp` at the ledger —
/// read back through the family's own `element_trail` door, on the real
/// `property_asserted` row the write produced.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn a_facet_write_through_the_driven_path_lands_at_mcp_in_the_ledger(pool: PgPool) {
    let (app, svc, parts) = harness(pool).await;
    let handle = handle_of(&app.pool).await;
    let resource = ingest(&app, "Attributed facet", "The attributed body.").await;

    let res = temper_mcp::tools::facets::facet_set(
        &svc,
        &parts,
        serde_json::from_value(json!({
            "resource": resource.to_string(),
            "values": {"status": "open"},
        }))
        .expect("input deserializes from its wire shape"),
    )
    .await
    .expect("the facet write lands");

    let ack: serde_json::Value =
        serde_json::from_str(res.content[0].as_text().expect("a text part").text.as_str())
            .expect("the part is the tool's JSON response");
    assert_eq!(
        ack["property_ids"].as_array().expect("ids").len(),
        1,
        "the write lands one row: {ack}"
    );

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
    let facet_act = events
        .iter()
        .find(|e| e["kind"] == "property_asserted")
        .expect("the facet act is on the resource's trail: {v}");
    assert_eq!(
        facet_act["actor_name"],
        json!(format!("{handle}@mcp")),
        "the MCP-surface emitter attributed the act, read back through the trail door: {v}"
    );
}
