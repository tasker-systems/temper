#![cfg(feature = "artifact-tests")]
//! Deliverable-2 (charter-as-content-blocks) acceptance: `cogmap_genesis` persists the telos charter as
//! REAL content-blocks via the shared block→chunk path — block-0 statement, blocks 1..n
//! questions-with-context, then framing blocks (positional by seq, no block_kind) — multi-block AND
//! multi-chunk-per-block, with sha256 hashes + inline bge-768 embeddings, and returns BOTH the cogmap id
//! and its telos resource id (sparing the loader a re-fetch).
//!
//! Chunking + embedding happen Rust-side via `TelosDef::block_specs` + `content::prepare_blocks`
//! (borrowing temper-ingest); the SQL function only persists. Resets the artifact, ONNX-dependent,
//! serialized via the temper-substrate-write group.
mod common;

use temper_substrate::events::{fire, SeedAction};
use temper_substrate::ids::{EntityId, ProfileId};
use temper_substrate::scenario::bootseed;
use temper_substrate::scenario::model::{QuestionDef, TelosDef};
use temper_substrate::content;
use uuid::Uuid;

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

#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn cogmap_genesis_persists_multi_block_multi_chunk_charter(pool: sqlx::PgPool) {
    bootseed::seed_system(&pool).await.unwrap();
    let (owner, emitter) = seed_actor(&pool).await;

    // A charter that exercises BOTH multi-block (statement + 2 questions + 1 framing = 4 blocks) and
    // multi-chunk-per-block (the second question's context is long enough to split one 510-token window).
    let para =
        "This context explains one facet of reaching first-merge confidence in onboarding week one.\n\n";
    let long_context = para.repeat(30); // ~2700 chars ⇒ the question block splits into >1 chunk
    let telos = TelosDef {
        title: "Onboarding charter".into(),
        statement: "Help a new EPD engineer reach first-merge confidence in week one.".into(),
        questions: vec![
            QuestionDef {
                question: "What transfers?".into(),
                context: String::new(),
            },
            QuestionDef {
                question: "What is the smallest real change?".into(),
                context: long_context,
            },
        ],
        framing: vec!["This map situates first-week onboarding.".into()],
    };

    // Rust-side: flatten → chunk → embed, exactly as the loader does.
    let specs = telos.block_specs();
    let refs: Vec<(Option<&str>, &str)> = specs
        .iter()
        .map(|(role, prose)| (Some(*role), prose.as_str()))
        .collect();
    let blocks = content::prepare_blocks(&refs).unwrap();
    assert_eq!(
        blocks.len(),
        4,
        "statement + 2 questions + 1 framing = 4 blocks"
    );
    assert!(
        blocks[2].chunks.len() > 1,
        "the long question-with-context block must be multi-chunk, got {}",
        blocks[2].chunks.len()
    );
    let total_chunks: usize = blocks.iter().map(|b| b.chunks.len()).sum();

    // genesis through the single firing surface (payload-first); returns BOTH ids — the record-set
    // return that retires the loader's telos re-fetch.
    let mut conn = pool.acquire().await.unwrap();
    let (cogmap, telos_resource) = fire(
        &mut conn,
        SeedAction::CogmapGenesis {
            name: "onboarding-cogmap",
            telos_title: "Onboarding charter",
            charter: &blocks,
            owner: ProfileId::from(owner),
            emitter: EntityId::from(emitter),
        },
    )
    .await
    .unwrap()
    .cogmap_genesis()
    .unwrap();
    let (cogmap, telos_resource): (Uuid, Uuid) = (cogmap.uuid(), telos_resource.uuid());
    drop(conn);

    // the returned telos id IS the cogmap's telos_resource_id (no re-fetch divergence)
    let homed_telos: Uuid =
        sqlx::query_scalar("SELECT telos_resource_id FROM kb_cogmaps WHERE id=$1")
            .bind(cogmap)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(
        homed_telos, telos_resource,
        "returned telos id matches the cogmap row"
    );

    // exactly four charter blocks at seq 0..3
    let block_seqs: Vec<i32> =
        sqlx::query_scalar("SELECT seq FROM kb_content_blocks WHERE resource_id=$1 ORDER BY seq")
            .bind(telos_resource)
            .fetch_all(&pool)
            .await
            .unwrap();
    assert_eq!(
        block_seqs,
        vec![0, 1, 2, 3],
        "four charter blocks at seq 0..3"
    );

    // all prepared chunks persisted (multi-chunk-per-block), each with prose + an inline bge-768 embedding
    let chunk_rows: i64 = sqlx::query_scalar("SELECT count(*) FROM kb_chunks WHERE resource_id=$1")
        .bind(telos_resource)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(
        chunk_rows, total_chunks as i64,
        "all prepared charter chunks persisted"
    );
    assert!(
        chunk_rows > 4,
        "more chunks than blocks ⇒ a charter block split"
    );

    let embedded_with_content: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM kb_chunks ch JOIN kb_chunk_content cc ON cc.chunk_id=ch.id \
         WHERE ch.resource_id=$1 AND ch.embedding IS NOT NULL",
    )
    .bind(telos_resource)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        embedded_with_content, chunk_rows,
        "every charter chunk has prose + an inline bge-768 embedding"
    );

    // content hashes are sha256 hex (64 chars), not md5 (32)
    let hash_lens: Vec<i32> =
        sqlx::query_scalar("SELECT length(content_hash) FROM kb_chunks WHERE resource_id=$1")
            .bind(telos_resource)
            .fetch_all(&pool)
            .await
            .unwrap();
    assert!(
        hash_lens.iter().all(|&l| l == 64),
        "charter content_hash is sha256 hex (64), got {hash_lens:?}"
    );

    // resource body_hash is the sha256 merkle
    let body_hash: Option<String> =
        sqlx::query_scalar("SELECT body_hash FROM kb_resources WHERE id=$1")
            .bind(telos_resource)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(
        body_hash.expect("body_hash populated").len(),
        64,
        "body_hash is sha256 hex merkle"
    );

    // doc_type property stamped cogmap_charter
    let doc_type: Option<serde_json::Value> = sqlx::query_scalar(
        "SELECT property_value FROM kb_properties \
         WHERE owner_table='kb_resources' AND owner_id=$1 AND property_key='doc_type'",
    )
    .bind(telos_resource)
    .fetch_optional(&pool)
    .await
    .unwrap();
    assert_eq!(doc_type, Some(serde_json::json!("cogmap_charter")));

    // exactly one cogmap_seeded event, anchored to the new cogmap (the genesis correlation root)
    let seeded_events: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM kb_events e JOIN kb_event_types t ON t.id=e.event_type_id \
         WHERE t.name='cogmap_seeded' AND e.producing_anchor_table='kb_cogmaps' \
           AND e.producing_anchor_id=$1",
    )
    .bind(cogmap)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        seeded_events, 1,
        "one cogmap_seeded genesis event anchored to the cogmap"
    );
}
