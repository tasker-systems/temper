#![cfg(feature = "artifact-tests")]
//! Deliverable-1 acceptance: `resource_create` persists the production content nesting
//! (resource ⊃ content_blocks ⊃ chunks ⊃ chunk_content) for a **multi-block, multi-chunk-per-block**
//! resource, with real sha256 content hashes, inline bge-768 embeddings, and a correct merkle body_hash.
//!
//! Chunking + embedding happen Rust-side via `content::prepare_blocks` (borrowing temper-ingest); the SQL
//! function only persists. ONNX-dependent. Isolated ephemeral DB via `temper_substrate::MIGRATOR`.
mod common;

use temper_substrate::content;
use temper_substrate::events::{fire, SeedAction};
use temper_substrate::ids::{CogmapId, EntityId, ProfileId};
use temper_substrate::scenario::bootseed;
use uuid::Uuid;

/// Minimal owner profile + emitter entity so `cogmap_genesis` (unchanged) can mint a home cogmap.
async fn seed_actor(pool: &sqlx::PgPool) -> (Uuid, Uuid) {
    let profile: Uuid = sqlx::query_scalar(
        "INSERT INTO kb_profiles (handle, display_name) \
         VALUES ('owner', 'Owner') RETURNING id",
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

#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn resource_create_persists_multi_block_multi_chunk_nesting(pool: sqlx::PgPool) {
    bootseed::seed_system(&pool).await.unwrap();
    let (owner, emitter) = seed_actor(&pool).await;

    // a home cogmap via the genesis function (resource_create homes into it), through the single
    // firing surface (payload-first).
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

    // three blocks: short / LONG (multi-chunk) / short. The long block exceeds one ~1785-char window.
    let short_a = "A short framing note about first-week confidence.";
    let para =
        "This paragraph explains one facet of reaching first-merge confidence in onboarding week one.\n\n";
    let long = para.repeat(30); // ~2700 chars ⇒ splits into >1 chunk
    let short_b = "A closing note on sharp edges that scar newcomers.";
    let blocks =
        content::prepare_blocks(&[(None, short_a), (None, long.as_str()), (None, short_b)])
            .unwrap();

    // sanity on the prepared shape: 3 blocks, middle one multi-chunk
    assert_eq!(blocks.len(), 3, "three content blocks prepared");
    assert_eq!(blocks[0].chunks.len(), 1);
    assert!(
        blocks[1].chunks.len() > 1,
        "the long middle block must be multi-chunk, got {}",
        blocks[1].chunks.len()
    );
    assert_eq!(blocks[2].chunks.len(), 1);
    let total_chunks: usize = blocks.iter().map(|b| b.chunks.len()).sum();

    let resource = fire(
        &mut conn,
        SeedAction::ResourceCreate {
            title: "Onboarding doc",
            origin_uri: "temper://c/multi",
            resource_id: None,
            home: temper_substrate::payloads::AnchorRef::cogmap(CogmapId::from(cogmap.uuid())),
            owner: ProfileId::from(owner),
            originator: None,
            blocks: &blocks,
            doc_type: Some("concept"),
            emitter: EntityId::from(emitter),
            segmented: false,
        },
    )
    .await
    .unwrap()
    .resource()
    .unwrap();
    let resource: Uuid = resource.uuid();
    drop(conn);

    // exactly three content blocks, with the declared seqs 0,1,2
    let block_seqs: Vec<i32> =
        sqlx::query_scalar("SELECT seq FROM kb_content_blocks WHERE resource_id=$1 ORDER BY seq")
            .bind(resource)
            .fetch_all(&pool)
            .await
            .unwrap();
    assert_eq!(block_seqs, vec![0, 1, 2], "three blocks at seq 0,1,2");

    // total chunk rows match the prepared count (multi-chunk-per-block persisted)
    let chunk_rows: i64 = sqlx::query_scalar("SELECT count(*) FROM kb_chunks WHERE resource_id=$1")
        .bind(resource)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(
        chunk_rows, total_chunks as i64,
        "all prepared chunks persisted"
    );
    assert!(chunk_rows > 3, "more chunks than blocks ⇒ a block split");

    // every chunk has prose (chunk_content) and a non-null bge-768 embedding written inline
    let content_rows: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM kb_chunks ch JOIN kb_chunk_content cc ON cc.chunk_id=ch.id \
         WHERE ch.resource_id=$1",
    )
    .bind(resource)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        content_rows, chunk_rows,
        "every chunk has chunk_content prose"
    );

    let embedded: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM kb_chunks WHERE resource_id=$1 AND embedding IS NOT NULL",
    )
    .bind(resource)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        embedded, chunk_rows,
        "every chunk embedded inline (bge-768)"
    );

    // content hashes are the chunker's sha256 (64 hex chars), not md5 (32)
    let hash_lens: Vec<i32> =
        sqlx::query_scalar("SELECT length(content_hash) FROM kb_chunks WHERE resource_id=$1")
            .bind(resource)
            .fetch_all(&pool)
            .await
            .unwrap();
    assert!(
        hash_lens.iter().all(|&l| l == 64),
        "content_hash is sha256 hex (64 chars), got {hash_lens:?}"
    );

    // one block_revision per block, with the right per-block chunk_count (the long block > 1)
    let rev_counts: Vec<i32> = sqlx::query_scalar(
        "SELECT br.chunk_count FROM kb_block_revisions br \
         JOIN kb_content_blocks b ON b.id=br.block_id \
         WHERE b.resource_id=$1 ORDER BY b.seq",
    )
    .bind(resource)
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(rev_counts.len(), 3, "one revision per block");
    assert_eq!(rev_counts[0], 1);
    assert!(
        rev_counts[1] > 1,
        "long block revision records its multi-chunk count"
    );
    assert_eq!(rev_counts[2], 1);

    // resource body_hash is the merkle (non-null sha256 hex)
    let body_hash: Option<String> =
        sqlx::query_scalar("SELECT body_hash FROM kb_resources WHERE id=$1")
            .bind(resource)
            .fetch_one(&pool)
            .await
            .unwrap();
    let body_hash = body_hash.expect("body_hash populated");
    assert_eq!(body_hash.len(), 64, "body_hash is sha256 hex merkle");

    // the doc_type property landed
    let doc_type: Option<serde_json::Value> = sqlx::query_scalar(
        "SELECT property_value FROM kb_properties \
         WHERE owner_table='kb_resources' AND owner_id=$1 AND property_key='doc_type'",
    )
    .bind(resource)
    .fetch_optional(&pool)
    .await
    .unwrap();
    assert_eq!(doc_type, Some(serde_json::json!("concept")));
}
