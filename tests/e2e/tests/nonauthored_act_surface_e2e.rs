#![cfg(feature = "test-db")]
//! Chunk 6 acceptance: per-act authorship + invocation correlation through the **non-authored** write
//! surfaces (update / delete / retype / reweight), the follow-on to `act_authorship_surface_e2e`. Acts
//! performed under an open invocation via the production temper-client (HTTP API) + the `temper` CLI
//! land in the substrate with `kb_events.invocation_id` set and authorship in `kb_events.metadata`, and
//! surface through `invocation_show`.
//!
//! Embed-free: the update is open_meta-only (a `property_set` sub-event, no body re-chunk), delete/retype/
//! reweight carry no body. The body-only (`block_mutated`) correlation shares the SAME `ctx.clone()`
//! threading in `update_resource_in_tx` (proven at the substrate + backend tiers) and the embedding path
//! is exercised by the Embed CI job.
//!
//! Projection-invisibility is NOT re-tested here: these acts write authorship to the same
//! `kb_events.metadata` column the authored acts do, which the parent's Chunk D acceptance already proved
//! invisible to affinity/region projections by construction (projections read the payload, never the
//! metadata). The invariant holds for these acts by the same structural guarantee.

mod common;

use uuid::Uuid;

use temper_core::types::authorship::ActInput;
use temper_core::types::graph::{EdgeKind, Polarity};
use temper_core::types::ids::InvocationId;
use temper_core::types::ingest::IngestPayload;
use temper_core::types::invocation_requests::OpenInvocationRequest;
use temper_core::types::relationship_requests::{
    AssertRelationshipRequest, RetypeRelationshipRequest, ReweightRelationshipRequest,
};
use temper_core::types::ConfidenceBand;
use temper_workflow::types::resource::ResourceUpdateRequest;

/// The L0 kernel cognitive map reserved id (birth migration `20260625000001`).
const L0_COGMAP: Uuid = Uuid::from_u128(0x00000000_0000_0000_0005_000000000001);

/// Metadata of the single act stamped with `invocation_id` for the given event kind.
async fn act_metadata(pool: &sqlx::PgPool, invocation_id: Uuid, kind: &str) -> serde_json::Value {
    use sqlx::Row;
    let row = sqlx::query(
        "SELECT e.metadata FROM kb_events e JOIN kb_event_types et ON et.id = e.event_type_id \
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
fn empty_payload(title: &str, ctx: &str) -> IngestPayload {
    IngestPayload {
        title: title.to_string(),
        origin_uri: format!("test://nonauth/{title}"),
        context_ref: format!("@me/{ctx}"),
        doc_type_name: "research".to_string(),
        content_hash: None,
        slug: title.to_lowercase().replace(' ', "-"),
        content: String::new(),
        metadata: None,
        managed_meta: None,
        open_meta: None,
        chunks_packed: None,
        act: ActInput::default(),
    }
}

fn stamped(invocation_id: Uuid, reasoning: &str, confidence: ConfidenceBand) -> ActInput {
    ActInput {
        invocation_id: Some(InvocationId::from(invocation_id)),
        reasoning: Some(reasoning.to_string()),
        confidence: Some(confidence),
        ..Default::default()
    }
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn api_nonauthored_writes_under_invocation_stamp_authorship(pool: sqlx::PgPool) {
    let app = common::setup(pool.clone()).await;
    let principal = app.client.profile().get().await.expect("profile").id;
    common::enable_invite_only(&pool, principal).await;
    app.client
        .contexts()
        .create("nonauth-api")
        .await
        .expect("context create");

    // Seed: a doc to update, a doc to delete, and an edge to retype/reweight (all un-attributed).
    let doc = app
        .client
        .ingest()
        .create(&empty_payload("Doc To Update", "nonauth-api"))
        .await
        .expect("create doc");
    let victim = app
        .client
        .ingest()
        .create(&empty_payload("Doc To Delete", "nonauth-api"))
        .await
        .expect("create victim");
    let src = app
        .client
        .ingest()
        .create(&empty_payload("Edge Src", "nonauth-api"))
        .await
        .expect("create src");
    let tgt = app
        .client
        .ingest()
        .create(&empty_payload("Edge Tgt", "nonauth-api"))
        .await
        .expect("create tgt");
    let edge_handle = app
        .client
        .relationships()
        .assert(&AssertRelationshipRequest {
            source: src.id,
            target: tgt.id,
            edge_kind: EdgeKind::Near,
            polarity: Polarity::Forward,
            label: "relates".to_string(),
            weight: 1.0,
            act: ActInput::default(),
        })
        .await
        .expect("assert edge")
        .edge_handle;

    // The run under which the four non-authored acts are performed.
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

    // UPDATE (open_meta-only → property_set) under the invocation.
    app.client
        .resources()
        .update(
            Uuid::from(doc.id),
            &ResourceUpdateRequest {
                open_meta: Some(serde_json::json!({ "reviewed_by": "qa" })),
                act: stamped(invocation_id, "api update act", ConfidenceBand::Probable),
                ..Default::default()
            },
        )
        .await
        .expect("api update under invocation");
    let update_meta = act_metadata(&pool, invocation_id, "property_set").await;
    assert_eq!(update_meta["confidence"], "probable");
    assert_eq!(update_meta["reasoning"], "api update act");

    // RETYPE + REWEIGHT the edge under the invocation.
    app.client
        .relationships()
        .retype(
            edge_handle,
            &RetypeRelationshipRequest {
                edge_kind: EdgeKind::LeadsTo,
                polarity: Polarity::Forward,
                act: stamped(invocation_id, "api retype act", ConfidenceBand::Confident),
            },
        )
        .await
        .expect("api retype under invocation");
    assert_eq!(
        act_metadata(&pool, invocation_id, "relationship_retyped").await["reasoning"],
        "api retype act"
    );
    app.client
        .relationships()
        .reweight(
            edge_handle,
            &ReweightRelationshipRequest {
                weight: 0.5,
                act: stamped(invocation_id, "api reweight act", ConfidenceBand::Tentative),
            },
        )
        .await
        .expect("api reweight under invocation");
    assert_eq!(
        act_metadata(&pool, invocation_id, "relationship_reweighted").await["confidence"],
        "tentative"
    );

    // DELETE under the invocation (authorship rides query params).
    app.client
        .resources()
        .delete(
            Uuid::from(victim.id),
            &stamped(invocation_id, "api delete act", ConfidenceBand::Confident),
        )
        .await
        .expect("api delete under invocation");
    assert_eq!(
        act_metadata(&pool, invocation_id, "resource_deleted").await["reasoning"],
        "api delete act"
    );

    // invocation_show surfaces all four non-authored acts with decoded authorship + correlator.
    let view = app
        .client
        .invocations()
        .show(invocation_id)
        .await
        .expect("show invocation");
    for kind in [
        "property_set",
        "relationship_retyped",
        "relationship_reweighted",
        "resource_deleted",
    ] {
        let act = view
            .acts
            .iter()
            .find(|a| a.event_kind == kind)
            .unwrap_or_else(|| panic!("{kind} act present in invocation_show"));
        assert_eq!(act.invocation_id, Some(invocation_id), "{kind} correlates");
        assert!(
            act.authorship.is_some(),
            "{kind} act carries decoded authorship in show"
        );
    }
}

/// CLI path: drive `temper resource delete --invocation … --confidence …` under an open invocation.
/// Proves the clap → ActArgs → ActInput → query-param → handler vertical for a non-authored write.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn cli_delete_under_invocation_stamps_authorship(pool: sqlx::PgPool) {
    let app = common::setup(pool.clone()).await;
    let principal = app.client.profile().get().await.expect("profile").id;
    common::enable_invite_only(&pool, principal).await;
    app.client
        .contexts()
        .create("nonauth-cli")
        .await
        .expect("context create");

    let victim = app
        .client
        .ingest()
        .create(&empty_payload("Cli Delete Target", "nonauth-cli"))
        .await
        .expect("create cli victim");
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

    let victim_ref = victim.id.to_string();
    let inv_ref = invocation_id.to_string();
    let out = common::run_temper_cli(
        &app,
        &[
            "resource",
            "delete",
            &victim_ref,
            "--invocation",
            &inv_ref,
            "--confidence",
            "probable",
            "--reasoning",
            "cli delete act",
        ],
    )
    .await
    .expect("run temper resource delete");
    assert!(
        out.status.success(),
        "CLI resource delete failed: stdout={} stderr={}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );

    let meta = act_metadata(&pool, invocation_id, "resource_deleted").await;
    assert_eq!(meta["confidence"], "probable");
    assert_eq!(meta["reasoning"], "cli delete act");
}
