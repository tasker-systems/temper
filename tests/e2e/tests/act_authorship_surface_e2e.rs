#![cfg(feature = "test-db")]
//! Chunk C acceptance: per-act authorship + invocation correlation through the **HTTP API**
//! (temper-client → Axum handlers) and the **CLI** binary, mirroring the MCP proof in
//! `act_authorship_mcp_e2e`. Authored acts created under an open invocation land in the substrate
//! with `kb_events.invocation_id` set and the authorship serialized into `kb_events.metadata`.
//!
//! Both paths here are embed-free: resource creates send empty content (handler builds no body, so
//! no chunk/embed work), and edge asserts carry no body at all. The CLI **create** path always
//! embeds (its translator synthesizes a placeholder body), so the CLI vertical is proven via
//! `edge assert` — the embedding create path is covered by the MCP/API proofs here plus the embed
//! CI job.

mod common;

use uuid::Uuid;

use temper_core::types::authorship::ActInput;
use temper_core::types::graph::{EdgeKind, Polarity};
use temper_core::types::ids::InvocationId;
use temper_core::types::ingest::IngestPayload;
use temper_core::types::invocation_requests::OpenInvocationRequest;
use temper_core::types::relationship_requests::AssertRelationshipRequest;
use temper_core::types::ConfidenceBand;

/// The L0 kernel cognitive map reserved id (birth migration `20260625000001`).
const L0_COGMAP: Uuid = Uuid::from_u128(0x00000000_0000_0000_0005_000000000001);

/// Fetch the metadata of the single act stamped with `invocation_id` for the given event kind.
async fn act_metadata(pool: &sqlx::PgPool, invocation_id: Uuid, kind: &str) -> serde_json::Value {
    use sqlx::Row;
    let row = sqlx::query(
        "SELECT e.metadata
           FROM kb_events e
           JOIN kb_event_types et ON et.id = e.event_type_id
          WHERE e.invocation_id = $1 AND et.name = $2",
    )
    .bind(invocation_id)
    .bind(kind)
    .fetch_one(pool)
    .await
    .unwrap_or_else(|e| panic!("expected a stamped {kind} act for the invocation: {e}"));
    row.get("metadata")
}

/// An empty-content create payload (no body → no embed) addressed to `@me/{ctx}`.
fn empty_payload(title: &str, ctx: &str, act: ActInput) -> IngestPayload {
    IngestPayload {
        title: title.to_string(),
        origin_uri: format!("test://act/{title}"),
        context_ref: format!("@me/{ctx}"),
        doc_type_name: "research".to_string(),
        content_hash: None,
        slug: title.to_lowercase().replace(' ', "-"),
        content: String::new(),
        metadata: None,
        managed_meta: None,
        open_meta: None,
        chunks_packed: None,
        act,
    }
}

/// API path: open an invocation, then create a resource and assert an edge *under it* through the
/// production temper-client. Both authored acts carry the invocation_id + authorship into the substrate.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn api_create_and_assert_under_invocation_stamp_authorship(pool: sqlx::PgPool) {
    let app = common::setup(pool.clone()).await;
    let principal = app
        .client
        .profile()
        .get()
        .await
        .expect("profile pre-flight")
        .id;
    common::enable_invite_only(&pool, principal).await;
    app.client
        .contexts()
        .create("act-api")
        .await
        .expect("context create");

    let invocation_id = app
        .client
        .invocations()
        .open(&OpenInvocationRequest {
            trigger_kind: "e2e".into(),
            originating_cogmap: L0_COGMAP,
            parent_cogmap: None,
        })
        .await
        .expect("open invocation")
        .invocation_id;

    // create_resource under the invocation, via POST /api/ingest.
    let src = app
        .client
        .ingest()
        .create(&empty_payload(
            "Source Doc",
            "act-api",
            ActInput {
                invocation_id: Some(InvocationId::from(invocation_id)),
                reasoning: Some("api create act".to_string()),
                confidence: Some(ConfidenceBand::Probable),
                ..Default::default()
            },
        ))
        .await
        .expect("api create under invocation");
    // A second resource to be the edge target (no authorship needed).
    let tgt = app
        .client
        .ingest()
        .create(&empty_payload("Target Doc", "act-api", ActInput::default()))
        .await
        .expect("api create target");

    let create_meta = act_metadata(&pool, invocation_id, "resource_created").await;
    assert_eq!(create_meta["confidence"], "probable");
    assert_eq!(create_meta["reasoning"], "api create act");

    // assert_relationship under the same invocation, via POST /api/relationships.
    app.client
        .relationships()
        .assert(&AssertRelationshipRequest {
            source: src.id,
            target: tgt.id,
            edge_kind: EdgeKind::LeadsTo,
            polarity: Polarity::Forward,
            label: "depends_on".to_string(),
            weight: 1.0,
            act: ActInput {
                invocation_id: Some(InvocationId::from(invocation_id)),
                reasoning: Some("api assert act".to_string()),
                confidence: Some(ConfidenceBand::Confident),
                ..Default::default()
            },
        })
        .await
        .expect("api assert under invocation");

    let assert_meta = act_metadata(&pool, invocation_id, "relationship_asserted").await;
    assert_eq!(assert_meta["confidence"], "confident");
    assert_eq!(assert_meta["reasoning"], "api assert act");
}

/// CLI path: drive the `temper` binary's `edge assert` with the discrete authorship flags under an
/// open invocation. Proves the clap → ActArgs → ActInput → wire DTO → handler vertical end-to-end.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn cli_edge_assert_under_invocation_stamps_authorship(pool: sqlx::PgPool) {
    let app = common::setup(pool.clone()).await;
    let principal = app
        .client
        .profile()
        .get()
        .await
        .expect("profile pre-flight")
        .id;
    common::enable_invite_only(&pool, principal).await;
    app.client
        .contexts()
        .create("act-cli")
        .await
        .expect("context create");

    // Two resources to connect (empty content → no embed).
    let src = app
        .client
        .ingest()
        .create(&empty_payload("Cli Source", "act-cli", ActInput::default()))
        .await
        .expect("create cli source");
    let tgt = app
        .client
        .ingest()
        .create(&empty_payload("Cli Target", "act-cli", ActInput::default()))
        .await
        .expect("create cli target");

    let invocation_id = app
        .client
        .invocations()
        .open(&OpenInvocationRequest {
            trigger_kind: "e2e".into(),
            originating_cogmap: L0_COGMAP,
            parent_cogmap: None,
        })
        .await
        .expect("open invocation")
        .invocation_id;

    let src_ref = src.id.to_string();
    let tgt_ref = tgt.id.to_string();
    let inv_ref = invocation_id.to_string();
    let out = common::run_temper_cli(
        &app,
        &[
            "edge",
            "assert",
            &src_ref,
            &tgt_ref,
            "--kind",
            "near",
            "--polarity",
            "forward",
            "--label",
            "relates_to",
            "--invocation",
            &inv_ref,
            "--confidence",
            "tentative",
            "--reasoning",
            "cli assert act",
            "--persona",
            "steward",
        ],
    )
    .await
    .expect("run temper edge assert");
    assert!(
        out.status.success(),
        "CLI edge assert failed: stdout={} stderr={}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );

    let meta = act_metadata(&pool, invocation_id, "relationship_asserted").await;
    assert_eq!(meta["confidence"], "tentative");
    assert_eq!(meta["reasoning"], "cli assert act");
    assert_eq!(meta["persona"], "steward");
}
