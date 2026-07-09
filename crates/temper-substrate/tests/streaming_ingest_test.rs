#![cfg(feature = "artifact-tests")]
//! Streaming/segmented ingest: append + finalize + idempotency (Beat 1 — persistence + events).
//!
//! `block_append`/`resource_finalize` are ONNX-dependent through the shared `prepare_block_from_chunks`
//! / `_project_blocks` path (the crate's `artifact-tests` feature gates `#[sqlx::test]` DB tests, each
//! on its own ephemeral database via `temper_substrate::MIGRATOR`).

use temper_substrate::content::{self, IncomingChunk};
use temper_substrate::events::{fire, SeedAction};
use temper_substrate::ids::{EntityId, ProfileId, ResourceId};
use temper_substrate::payloads;
use temper_substrate::scenario::bootseed;
use uuid::Uuid;

/// Shared seed context + fixture helpers for the streaming-ingest tests below. Builds a resource
/// with one landed block (seq 0) via the ordinary create path, homed to a fresh cogmap, owned by
/// (and visible to) `principal` — the shape every append/finalize test starts from.
mod streaming_test_support {
    use super::*;

    /// The ids a seeded fixture hands back: the resource under test, its emitter entity (for
    /// firing further appends), and the owning profile (for `readback::body` visibility).
    pub struct SeedCtx {
        pub resource: ResourceId,
        pub emitter: EntityId,
        pub principal: ProfileId,
    }

    /// Minimal owner profile + emitter entity so `cogmap_genesis`/`resource_create` can mint a
    /// home — mirrors `content_multichunk.rs`'s `seed_actor`.
    async fn seed_actor(pool: &sqlx::PgPool) -> (Uuid, Uuid) {
        let profile: Uuid = sqlx::query_scalar(
            "INSERT INTO kb_profiles (handle, display_name, system_access) \
             VALUES ('owner', 'Owner', 'approved'::system_access) RETURNING id",
        )
        .fetch_one(pool)
        .await
        .unwrap();
        let entity: Uuid = sqlx::query_scalar(
            "INSERT INTO kb_entities (profile_id, name, metadata) VALUES ($1, 'agent#1', '{}'::jsonb) RETURNING id",
        )
        .bind(profile)
        .fetch_one(pool)
        .await
        .unwrap();
        (profile, entity)
    }

    /// Build a single caller-supplied chunk for `prepare_block_from_chunks`. The embedding is a
    /// full-width (768-dim) fake vector — `kb_chunks.embedding` is a fixed `vector(768)` column, so
    /// even a throwaway test fixture must carry an exact-dimension vector to persist.
    pub fn one_chunk(text: &str) -> Vec<IncomingChunk> {
        use sha2::{Digest, Sha256};
        let mut hasher = Sha256::new();
        hasher.update(text.as_bytes());
        let content_hash = format!("{:x}", hasher.finalize());
        vec![IncomingChunk {
            chunk_index: 0,
            content_hash,
            content: text.to_owned(),
            embedding: vec![0.1_f32; 768],
            header_path: String::new(),
            heading_depth: 0,
        }]
    }

    /// Seed a resource with block 0 landed via the ordinary create path (homed to a fresh
    /// cogmap), returning the ids the streaming-ingest tests append/finalize against.
    pub async fn seed_resource_with_block0(pool: &sqlx::PgPool) -> SeedCtx {
        bootseed::seed_system(pool).await.unwrap();
        let (owner, emitter) = seed_actor(pool).await;
        let charter = content::prepare_blocks(&[(None, "seed statement")]).unwrap();
        let mut conn = pool.acquire().await.unwrap();
        let (cogmap, _telos) = fire(
            &mut conn,
            SeedAction::CogmapGenesis {
                name: "home",
                telos_title: "Charter",
                charter: &charter,
                cogmap_id: None,
                telos_resource_id: None,
                owner: ProfileId::from(owner),
                emitter: EntityId::from(emitter),
            },
        )
        .await
        .unwrap()
        .cogmap_genesis()
        .unwrap();

        let block0 = content::prepare_block_from_chunks(0, None, one_chunk("first segment"));
        let blocks = [block0];
        let resource = fire(
            &mut conn,
            SeedAction::ResourceCreate {
                title: "Streaming doc",
                origin_uri: "temper://streaming/doc",
                resource_id: None,
                home: payloads::AnchorRef::cogmap(cogmap),
                owner: ProfileId::from(owner),
                originator: None,
                blocks: &blocks,
                doc_type: Some("concept"),
                emitter: EntityId::from(emitter),
            },
        )
        .await
        .unwrap()
        .resource()
        .unwrap();

        SeedCtx {
            resource,
            emitter: EntityId::from(emitter),
            principal: ProfileId::from(owner),
        }
    }
}

// ── Task 1.1: resource_finalized event type + block_append/resource_finalize SQL ────────────

#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn resource_finalized_event_type_is_seeded(pool: sqlx::PgPool) {
    let name: Option<String> =
        sqlx::query_scalar("SELECT name FROM kb_event_types WHERE name = 'resource_finalized'")
            .fetch_optional(&pool)
            .await
            .unwrap();
    assert_eq!(name.as_deref(), Some("resource_finalized"));
}

// ── Task 1.2: BlockCreated + ResourceFinalized payloads ──────────────────────────────────────

#[test]
fn block_created_payload_serializes_with_resource_and_block() {
    use temper_substrate::payloads::{BlockCreated, BlockManifest};
    // BlockManifest::from(&PreparedBlock) is the existing constructor; build a
    // minimal PreparedBlock via content::prepare_block_from_chunks (ONNX-free).
    let block = temper_substrate::content::prepare_block_from_chunks(
        3,
        None,
        vec![temper_substrate::content::IncomingChunk {
            chunk_index: 0,
            content_hash: "abc".into(),
            content: "hi".into(),
            embedding: vec![],
            header_path: String::new(),
            heading_depth: 0,
        }],
    );
    let p = BlockCreated {
        resource_id: temper_substrate::ids::ResourceId::from(uuid::Uuid::now_v7()),
        block: BlockManifest::from(&block),
    };
    let v = serde_json::to_value(&p).unwrap();
    assert!(v.get("resource_id").is_some());
    assert_eq!(v["block"]["seq"], 3);
    assert!(v["block"]["chunks"].is_array());
}

// ── Task 1.3/1.4: SeedAction::BlockAppend + writes::append_block ────────────────────────────

#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn append_block_lands_second_block_and_fires_block_created(pool: sqlx::PgPool) {
    // Seed a resource with block 0 via the ordinary create path, then append seq 1.
    let ctx = streaming_test_support::seed_resource_with_block0(&pool).await;
    let block1 = temper_substrate::content::prepare_block_from_chunks(
        1,
        None,
        streaming_test_support::one_chunk("second segment"),
    );
    let block_id = temper_substrate::writes::append_block(
        &pool,
        temper_substrate::writes::AppendParams {
            resource: ctx.resource,
            block: &block1,
            sources: vec![],
            emitter: ctx.emitter,
        },
    )
    .await
    .unwrap();
    // A block_created event exists for this resource.
    let n: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM kb_events e JOIN kb_event_types t ON t.id=e.event_type_id \
         WHERE t.name='block_created' AND (e.payload->>'resource_id')::uuid = $1",
    )
    .bind(ctx.resource.uuid())
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(n, 1);
    // The body now reassembles both segments in seq order.
    let body = temper_substrate::readback::body(&pool, ctx.principal, ctx.resource)
        .await
        .unwrap();
    assert!(body.contains("second segment"));
    let _ = block_id;
}

#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn append_block_is_idempotent_on_reappend(pool: sqlx::PgPool) {
    let ctx = streaming_test_support::seed_resource_with_block0(&pool).await;
    let block1 = temper_substrate::content::prepare_block_from_chunks(
        1,
        None,
        streaming_test_support::one_chunk("segment one"),
    );
    let a = temper_substrate::writes::append_block(
        &pool,
        temper_substrate::writes::AppendParams {
            resource: ctx.resource,
            block: &block1,
            sources: vec![],
            emitter: ctx.emitter,
        },
    )
    .await
    .unwrap();
    // Re-append the SAME prepared block (same chunk content_hash → same merkle).
    let b = temper_substrate::writes::append_block(
        &pool,
        temper_substrate::writes::AppendParams {
            resource: ctx.resource,
            block: &block1,
            sources: vec![],
            emitter: ctx.emitter,
        },
    )
    .await
    .unwrap();
    assert_eq!(a, b, "re-append is a no-op returning the same block id");
    let live: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM kb_content_blocks WHERE resource_id=$1 AND seq=1 AND NOT is_folded",
    )
    .bind(ctx.resource.uuid())
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(live, 1, "no duplicate block at seq 1");
}

// ── Task 1.5: writes::finalize_ingest ────────────────────────────────────────────────────────

#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn finalize_validates_block_count_and_hash(pool: sqlx::PgPool) {
    let ctx = streaming_test_support::seed_resource_with_block0(&pool).await;
    let block1 = temper_substrate::content::prepare_block_from_chunks(
        1,
        None,
        streaming_test_support::one_chunk("segment one"),
    );
    temper_substrate::writes::append_block(
        &pool,
        temper_substrate::writes::AppendParams {
            resource: ctx.resource,
            block: &block1,
            sources: vec![],
            emitter: ctx.emitter,
        },
    )
    .await
    .unwrap();
    let actual_hash: String = sqlx::query_scalar("SELECT body_hash FROM kb_resources WHERE id=$1")
        .bind(ctx.resource.uuid())
        .fetch_one(&pool)
        .await
        .unwrap();

    // Wrong count → error.
    let bad = temper_substrate::writes::finalize_ingest(
        &pool,
        temper_substrate::writes::FinalizeParams {
            resource: ctx.resource,
            expected_blocks: 5,
            expected_body_hash: actual_hash.clone(),
            emitter: ctx.emitter,
        },
    )
    .await;
    assert!(bad.is_err());

    // Correct count + hash → a resource_finalized event lands.
    temper_substrate::writes::finalize_ingest(
        &pool,
        temper_substrate::writes::FinalizeParams {
            resource: ctx.resource,
            expected_blocks: 2,
            expected_body_hash: actual_hash,
            emitter: ctx.emitter,
        },
    )
    .await
    .unwrap();
    let n: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM kb_events e JOIN kb_event_types t ON t.id=e.event_type_id \
         WHERE t.name='resource_finalized' AND (e.payload->>'resource_id')::uuid=$1",
    )
    .bind(ctx.resource.uuid())
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(n, 1);
}

// ── Task 1.6: writes::upsert_ingestion_record ────────────────────────────────────────────────

#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn ingestion_record_upserts_source_provenance(pool: sqlx::PgPool) {
    let ctx = streaming_test_support::seed_resource_with_block0(&pool).await;
    temper_substrate::writes::upsert_ingestion_record(
        &pool,
        temper_substrate::writes::IngestionRecord {
            resource: ctx.resource,
            source_uri: "vault://big.md",
            source_mimetype: Some("text/markdown"),
            conversion_tool: "passthrough",
            conversion_version: "1",
            source_hash: Some("deadbeef"),
        },
    )
    .await
    .unwrap();
    let hash: Option<String> =
        sqlx::query_scalar("SELECT source_hash FROM kb_ingestion_records WHERE resource_id=$1")
            .bind(ctx.resource.uuid())
            .fetch_optional(&pool)
            .await
            .unwrap();
    assert_eq!(hash.as_deref(), Some("deadbeef"));
}
