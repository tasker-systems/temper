#![cfg(feature = "test-db")]
//! A segmented ingest's search vector takes its body text at finalize, not on every append
//! (migration 20261117000010). The rebuild aggregated the whole body on every append, which made a
//! segmented ingest quadratic.
//!
//! Witnesses, over a real segmented ingest (begin, appends, finalize):
//!   * while the ingest is in progress, the search-index row exists and indexes the title, but no
//!     body term; this pins the guard that keeps appends cheap;
//!   * after every append, the stored `body_hash` equals the client-side merkle over the segments
//!     landed so far, because the append response returns it and an MCP caller echoes it back at
//!     finalize; this pins the hash as still computed per append;
//!   * after finalize, the stored vector equals what a fresh full rebuild produces, and a term only
//!     the LAST block carries matches; this pins the finalize rebuild.

mod common;

use sha2::Digest;
use sqlx::PgPool;
use temper_core::types::ids::EntityId;
use temper_substrate::content::{prepare_block_from_chunks, IncomingChunk, PreparedBlock};
use temper_substrate::events::EventContext;
use temper_substrate::ids::{ContextId, ProfileId, ResourceId};
use temper_substrate::payloads::AnchorRef;
use temper_substrate::writes::{self, AppendParams, CreateMode, CreateParams, FinalizeParams};
use uuid::Uuid;

fn chunk_hash(prose: &str) -> String {
    hex::encode(sha2::Sha256::digest(prose.trim()))
}

fn chunk(prose: &str) -> IncomingChunk {
    IncomingChunk {
        chunk_index: 0,
        content_hash: chunk_hash(prose),
        content: prose.to_string(),
        embedding: vec![0.1; 768],
        embedded_with: Some("model-sha-1".to_string()),
        header_path: String::new(),
        heading_depth: 0,
    }
}

async fn system_actor(pool: &PgPool) -> (ProfileId, EntityId) {
    let profile: Uuid = sqlx::query_scalar("SELECT id FROM kb_profiles WHERE handle='system'")
        .fetch_one(pool)
        .await
        .unwrap();
    let entity: EntityId =
        sqlx::query_scalar("SELECT id FROM kb_entities WHERE profile_id=$1 AND name='system'")
            .bind(profile)
            .fetch_one(pool)
            .await
            .unwrap();
    (ProfileId::from(profile), entity)
}

/// Re-register `resource_finalized`: migration 20260708000012 inserts it; `reset_schema` truncates it.
async fn register_resource_finalized(pool: &PgPool) {
    sqlx::query(
        "INSERT INTO kb_event_types (name, payload_schema, schema_version, category) \
         VALUES ('resource_finalized', NULL, 1, 'domain') \
         ON CONFLICT (name) DO NOTHING",
    )
    .execute(pool)
    .await
    .expect("re-register resource_finalized");
}

/// Whether the stored vector matches `term` under the 'english' config every resource is indexed with.
async fn vector_matches(pool: &PgPool, resource: ResourceId, term: &str) -> bool {
    sqlx::query_scalar(
        "SELECT search_vector @@ plainto_tsquery('english', $2) \
           FROM kb_resource_search_index WHERE resource_id = $1",
    )
    .bind(resource.uuid())
    .bind(term)
    .fetch_one(pool)
    .await
    .expect("the search-index row exists")
}

async fn stored_vector(pool: &PgPool, resource: ResourceId) -> String {
    sqlx::query_scalar(
        "SELECT search_vector::text FROM kb_resource_search_index WHERE resource_id = $1",
    )
    .bind(resource.uuid())
    .fetch_one(pool)
    .await
    .unwrap()
}

async fn body_hash(pool: &PgPool, resource: ResourceId) -> String {
    sqlx::query_scalar("SELECT body_hash FROM kb_resources WHERE id = $1")
        .bind(resource.uuid())
        .fetch_one(pool)
        .await
        .unwrap()
}

/// The client's view of a segment: its chunk hashes, folded the way `temper_ingest::merkle` folds them.
fn client_merkle(prose: &[&str]) -> String {
    let blocks: Vec<String> = prose
        .iter()
        .map(|p| temper_ingest::merkle::block_merkle(&[chunk_hash(p)]))
        .collect();
    temper_ingest::merkle::resource_body_hash(&blocks)
}

#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn a_segmented_ingest_indexes_its_body_at_finalize(pool: sqlx::PgPool) {
    common::reset_schema(&pool).await;
    temper_substrate::scenario::bootseed::seed_system(&pool)
        .await
        .unwrap();
    register_resource_finalized(&pool).await;
    let (owner, emitter) = system_actor(&pool).await;
    let home = ContextId::from(
        common::insert_context(&pool, "kb_profiles", owner.uuid(), "segv", "segv")
            .await
            .unwrap(),
    );

    // Each segment carries a term no other segment or the title has; the last one's term is the
    // finalize witness.
    let segments = [
        "the opening segment names aardvark",
        "the middle segment names bandicoot",
        "the closing segment names chinchilla",
    ];
    let resource = writes::create_resource_with_mode(
        &pool,
        CreateParams {
            idempotency_key: None,
            title: "Quokka survey",
            origin_uri: "test://segmented-search-vector",
            body: segments[0],
            doc_type: "research",
            home: AnchorRef::context(home),
            owner,
            originator: owner,
            emitter,
            properties: &[],
            chunks: Some(vec![chunk(segments[0])]),
            sources: vec![],
        },
        EventContext::default(),
        CreateMode {
            defer: false,
            segmented: true,
        },
    )
    .await
    .unwrap();
    assert_eq!(
        body_hash(&pool, resource).await,
        client_merkle(&segments[..1]),
        "the begin's body_hash is the merkle over block 0"
    );

    for (seq, prose) in segments.iter().enumerate().skip(1) {
        let block: PreparedBlock = prepare_block_from_chunks(seq as i32, None, vec![chunk(prose)]);
        writes::append_block(
            &pool,
            AppendParams {
                resource,
                block: &block,
                sources: vec![],
                emitter,
            },
        )
        .await
        .unwrap();
        assert_eq!(
            body_hash(&pool, resource).await,
            client_merkle(&segments[..=seq]),
            "after appending seq {seq}, the body_hash a reader sees is the merkle over every \
             segment landed so far"
        );
    }

    // In progress: the row exists and holds the title, but no body term.
    assert!(
        vector_matches(&pool, resource, "quokka").await,
        "the in-progress row still indexes the title"
    );
    for term in ["aardvark", "bandicoot", "chinchilla"] {
        assert!(
            !vector_matches(&pool, resource, term).await,
            "the in-progress vector withholds body term {term:?}; the guard keeps appends from \
             re-tokenizing the whole body"
        );
    }

    writes::finalize_ingest(
        &pool,
        FinalizeParams {
            resource,
            expected_blocks: segments.len() as u32,
            expected_body_hash: client_merkle(&segments),
            expected_content_hash: None,
            emitter,
        },
    )
    .await
    .expect("the per-append body_hash validates at finalize");

    // Complete: every body term matches, the last block's included, and the vector is exactly what
    // a fresh full rebuild produces.
    for term in ["quokka", "aardvark", "bandicoot", "chinchilla"] {
        assert!(
            vector_matches(&pool, resource, term).await,
            "the finalized vector indexes {term:?}"
        );
    }
    let at_finalize = stored_vector(&pool, resource).await;
    sqlx::query("SELECT _rebuild_resource_search_vector($1)")
        .bind(resource.uuid())
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(
        at_finalize,
        stored_vector(&pool, resource).await,
        "the vector finalize leaves equals a fresh full rebuild"
    );
}
