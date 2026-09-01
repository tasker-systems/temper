//! Integration test — the blob surfaces attribute their writes to the surface's emitter
//! entity (segment S5 of the surfaces task: MCP blob tools with the on-behalf-of chain).
//!
//! `blob_service` used to hard-wire `resolve_emitter(pool, caller, "web")` at every
//! attributed site, so an MCP commit or relation landed in the ledger under
//! `<handle>@web` — a lie about where the act came from. The widening threads a
//! `Surface` through `commit_blob` / `finalize_upload` / `relate_blob`, and these
//! witnesses pin the emitter join end-to-end: the surface named in the call is the
//! surface named on the event.
//!
//! `Surface::Mcp` is the case the widening exists for (the MCP server dispatches
//! in-process — no HTTP boundary ever re-marks it); `Surface::CliCloud` is ridden too,
//! because a parameter that cannot vary cannot be said to flow.
#![cfg(feature = "test-db")]

use std::sync::Arc;

use sqlx::{PgPool, Row};
use uuid::Uuid;

use temper_core::types::authorship::ActContext;
use temper_core::types::blob::BlobRelationAssertRequest;
use temper_core::types::graph::{EdgeKind, Polarity};
use temper_core::types::ids::ProfileId;
use temper_services::config::{BlobConfig, BlobCredentialMode};
use temper_services::services::blob_service;
use temper_substrate::blob_store::InMemoryBlobStore;
use temper_workflow::operations::Surface;

/// Seed a substrate profile + a profile-owned `temper` context (the minimum the write
/// path's `resolve_emitter` + visibility gate require). Mirrors
/// `segmented_backend_test.rs`'s inlined fixture — each test-target crate keeps its own
/// copy so it has no cross-target test-harness dependency.
async fn seed_profile_with_context(pool: &PgPool, email: &str) -> (Uuid, Uuid, String) {
    let profile_id = Uuid::now_v7();
    let local = email.split('@').next().unwrap_or("test-user");
    let handle = format!("{local}-{}", &profile_id.simple().to_string()[..8]);
    sqlx::query("INSERT INTO kb_profiles (id, handle, display_name, email) VALUES ($1,$2,$3,$4)")
        .bind(profile_id)
        .bind(&handle)
        .bind(email)
        .bind(email)
        .execute(pool)
        .await
        .expect("seed profile");
    for surface in ["web", "cli", "mcp"] {
        sqlx::query(
            "INSERT INTO kb_entities (profile_id, name, metadata) VALUES ($1,$2,'{}'::jsonb)",
        )
        .bind(profile_id)
        .bind(format!("{handle}@{surface}"))
        .execute(pool)
        .await
        .expect("seed emitter entity");
    }
    let context_id = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO kb_contexts (id, owner_table, owner_id, slug, name) \
         VALUES ($1,'kb_profiles',$2,'temper','temper')",
    )
    .bind(context_id)
    .bind(profile_id)
    .execute(pool)
    .await
    .expect("seed context");
    (profile_id, context_id, handle)
}

/// A test-sized `BlobConfig`: static-token posture (no env races), the given D9 numbers.
fn blob_cfg() -> BlobConfig {
    BlobConfig {
        store_id: "store_test".to_string(),
        read_write_token: Some("vercel_rw_test_store_test".to_string()),
        credential_mode: BlobCredentialMode::Token,
        oidc_token_source: Arc::new(|| None),
        max_bytes: 1 << 20,
        allowlist: vec!["image/png".to_string()],
        single_request_max_bytes: 64 * 1024,
    }
}

/// The emitter entity name the blob's assert event carries.
async fn blob_event_emitter(pool: &PgPool, blob_id: Uuid) -> String {
    sqlx::query(
        "SELECT ent.name \
           FROM kb_blobs b \
           JOIN kb_events ev  ON ev.id = b.asserted_by_event_id \
           JOIN kb_entities ent ON ent.id = ev.emitter_entity_id \
          WHERE b.id = $1",
    )
    .bind(blob_id)
    .fetch_one(pool)
    .await
    .expect("blob assert event with its emitter")
    .get(0)
}

/// The emitter entity name the edge's assert event carries.
async fn edge_event_emitter(pool: &PgPool, edge_id: Uuid) -> String {
    sqlx::query(
        "SELECT ent.name \
           FROM kb_edges e \
           JOIN kb_events ev  ON ev.id = e.asserted_by_event_id \
           JOIN kb_entities ent ON ent.id = ev.emitter_entity_id \
          WHERE e.id = $1",
    )
    .bind(edge_id)
    .fetch_one(pool)
    .await
    .expect("edge assert event with its emitter")
    .get(0)
}

/// The witness that fails while the emitter is hard-wired: an MCP commit must be
/// attributed to `<handle>@mcp`, not to the web entity that used to take every commit.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn an_mcp_commit_is_attributed_to_the_mcp_emitter(pool: PgPool) {
    let (profile, ctx, handle) = seed_profile_with_context(&pool, "emitter-mcp@example.com").await;
    let outcome = blob_service::commit_blob(
        &pool,
        &InMemoryBlobStore::default(),
        &blob_cfg(),
        blob_service::BlobCommitCommand {
            caller: ProfileId::from(profile),
            home_table: Some("kb_contexts".to_string()),
            home_id: Some(ctx.to_string()),
            content_type: "image/png".to_string(),
            bytes: bytes::Bytes::from_static(b"mcp-attributed-bytes"),
            surface: Surface::Mcp,
        },
    )
    .await
    .expect("commit through the service");

    assert_eq!(
        blob_event_emitter(&pool, outcome.blob_id.uuid()).await,
        format!("{handle}@mcp"),
        "an MCP commit is attributed to the caller's mcp emitter entity"
    );
}

/// A parameter that cannot vary cannot be said to flow: the same commit through a
/// different surface lands under that surface's entity. Guards against another
/// hard-wire.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn the_commit_emitter_follows_the_surface_named(pool: PgPool) {
    let (profile, ctx, handle) = seed_profile_with_context(&pool, "emitter-cli@example.com").await;
    let outcome = blob_service::commit_blob(
        &pool,
        &InMemoryBlobStore::default(),
        &blob_cfg(),
        blob_service::BlobCommitCommand {
            caller: ProfileId::from(profile),
            home_table: Some("kb_contexts".to_string()),
            home_id: Some(ctx.to_string()),
            content_type: "image/png".to_string(),
            bytes: bytes::Bytes::from_static(b"cli-attributed-bytes"),
            surface: Surface::CliCloud,
        },
    )
    .await
    .expect("commit through the service");

    assert_eq!(
        blob_event_emitter(&pool, outcome.blob_id.uuid()).await,
        format!("{handle}@cli"),
        "the emitter follows the surface the commit arrived on"
    );
}

/// The relate surface attributes through the same seam: an MCP relation's assert event
/// carries the `<handle>@mcp` emitter, with the act context riding alongside. The peer is
/// a second blob in the same home — kb_blobs is one of the three admissible peer tables
/// (the `blob_relate:` vocabulary), and this witness is about ATTRIBUTION, not relation
/// semantics; S4 witnessed those.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn an_mcp_relation_is_attributed_to_the_mcp_emitter(pool: PgPool) {
    let (profile, ctx, handle) =
        seed_profile_with_context(&pool, "emitter-relate@example.com").await;
    let caller = ProfileId::from(profile);
    let store = InMemoryBlobStore::default();
    let cfg = blob_cfg();

    let committed = blob_service::commit_blob(
        &pool,
        &store,
        &cfg,
        blob_service::BlobCommitCommand {
            caller,
            home_table: Some("kb_contexts".to_string()),
            home_id: Some(ctx.to_string()),
            content_type: "image/png".to_string(),
            bytes: bytes::Bytes::from_static(b"relate-attributed-bytes"),
            surface: Surface::Mcp,
        },
    )
    .await
    .expect("commit through the service");

    let peer = blob_service::commit_blob(
        &pool,
        &store,
        &cfg,
        blob_service::BlobCommitCommand {
            caller,
            home_table: Some("kb_contexts".to_string()),
            home_id: Some(ctx.to_string()),
            content_type: "image/png".to_string(),
            bytes: bytes::Bytes::from_static(b"relate-peer-bytes"),
            surface: Surface::Mcp,
        },
    )
    .await
    .expect("commit the peer blob through the service");

    let ack = blob_service::relate_blob(
        &pool,
        caller,
        committed.blob_id,
        &BlobRelationAssertRequest {
            direction: temper_core::types::blob::BlobRelationDirection::BlobAsSource,
            peer_table: "kb_blobs".to_string(),
            peer_id: peer.blob_id.uuid(),
            edge_kind: EdgeKind::Express,
            polarity: Polarity::Forward,
            label: "figure_of".to_string(),
            weight: 1.0,
            act: temper_core::types::authorship::ActInput::default(),
        },
        ActContext::default(),
        Surface::Mcp,
    )
    .await
    .expect("relate through the service");

    assert_eq!(
        edge_event_emitter(&pool, ack.edge_handle).await,
        format!("{handle}@mcp"),
        "an MCP relation is attributed to the caller's mcp emitter entity"
    );
}
