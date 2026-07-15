#![cfg(feature = "artifact-tests")]
//! W2 PR 3 — verbatim block content storage (`kb_block_content` + coverage-derived `body_storage`).
//!
//! The create/update tests construct through the **chunks arm** (`CreateParams.chunks = Some(..)` /
//! `UpdateParams.chunks = Some(..)`) — the arm EVERY CLI write takes — so they prove the raw bytes are
//! threaded on the production path, not merely on the server-embed prose arm. Isolated ephemeral DB via
//! `MIGRATOR`.

mod common;

use temper_substrate::content::{IncomingChunk, PreparedBlock, PreparedChunk};
use temper_substrate::events::{fire, SeedAction};
use temper_substrate::ids::{BlockId, ChunkId, ContextId, EntityId, ProfileId, ResourceId};
use temper_substrate::payloads::AnchorRef;
use temper_substrate::scenario::bootseed;
use temper_substrate::writes::{create_resource, update_resource, CreateParams, UpdateParams};
use uuid::Uuid;

// ── harness ────────────────────────────────────────────────────────────────────

async fn system_actor(pool: &sqlx::PgPool) -> (ProfileId, EntityId) {
    let profile: Uuid = sqlx::query_scalar("SELECT id FROM kb_profiles WHERE handle='system'")
        .fetch_one(pool)
        .await
        .unwrap();
    let entity: Uuid =
        sqlx::query_scalar("SELECT id FROM kb_entities WHERE profile_id=$1 AND name='system'")
            .bind(profile)
            .fetch_one(pool)
            .await
            .unwrap();
    (ProfileId::from(profile), EntityId::from(entity))
}

async fn make_context(pool: &sqlx::PgPool, owner: ProfileId, slug: &str) -> ContextId {
    let id = common::insert_context(pool, "kb_profiles", owner.uuid(), slug, slug)
        .await
        .unwrap();
    ContextId::from(id)
}

/// A single fake-embedded chunk carrying `content`. ONNX-free: the verbatim-bytes assertions key off
/// the block's raw_text (the whole body), never the chunk text, so the chunk content is immaterial.
fn one_incoming_chunk(content: &str) -> IncomingChunk {
    let mut embedding = vec![0.0_f32; 768];
    embedding[0] = 1.0;
    IncomingChunk {
        chunk_index: 0,
        content_hash: format!("{:064x}", Uuid::now_v7().as_u128()),
        content: content.to_string(),
        embedding,
        embedded_with: None,
        header_path: String::new(),
        heading_depth: 0,
    }
}

/// Create one resource through the CHUNKS arm (the CLI's path), returning its id. `body` is stored
/// verbatim by the write path (`block.raw_text = Some(p.body)`), independent of the chunk plan.
async fn create_via_chunks_arm(
    pool: &sqlx::PgPool,
    ctx: ContextId,
    owner: ProfileId,
    emitter: EntityId,
    body: &str,
) -> ResourceId {
    create_resource(
        pool,
        CreateParams {
            title: "t",
            origin_uri: "test://t",
            body,
            doc_type: "concept",
            home: AnchorRef::context(ctx),
            owner,
            originator: owner,
            emitter,
            properties: &[],
            chunks: Some(vec![one_incoming_chunk(body)]),
            sources: vec![],
        },
    )
    .await
    .unwrap()
}

async fn body_storage(pool: &sqlx::PgPool, id: ResourceId) -> String {
    sqlx::query_scalar("SELECT body_storage FROM kb_resources WHERE id=$1")
        .bind(id.uuid())
        .fetch_one(pool)
        .await
        .unwrap()
}

// ── tests ──────────────────────────────────────────────────────────────────────

#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn a_chunks_arm_create_stores_verbatim_bytes(pool: sqlx::PgPool) {
    bootseed::seed_system(&pool).await.unwrap();
    let (owner, emitter) = system_actor(&pool).await;
    let ctx = make_context(&pool, owner, "verbatim").await;

    let body = "# T\r\n\r\nalpha\nbeta\n"; // CRLF + trailing newline: nothing may be normalized
    let id = create_via_chunks_arm(&pool, ctx, owner, emitter, body).await;

    let (stored, hash): (String, String) = sqlx::query_as(
        "SELECT bc.content, bc.content_hash FROM kb_content_blocks b \
           JOIN kb_block_content bc ON bc.block_revision_id = b.current_revision_id \
          WHERE b.resource_id = $1",
    )
    .bind(id.uuid())
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(stored, body, "the raw bytes must be stored verbatim");
    // The stored hash is the bare-hex sha256 of the raw bytes (Rust `sha256_hex` == SQL twin).
    let expected_hash: String = sqlx::query_scalar("SELECT encode(sha256($1::bytea), 'hex')")
        .bind(body.as_bytes())
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(hash, expected_hash);

    assert_eq!(body_storage(&pool, id).await, "verbatim");
}

#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn mixed_coverage_is_derived_not_verbatim(pool: sqlx::PgPool) {
    // A resource with ONE block carrying bytes and ONE without — the shape a legacy multi-block
    // resource takes when only some of its blocks have been re-written. It must be 'derived', never
    // 'verbatim': a 'verbatim' flag over partial coverage is exactly how an INNER-JOIN readback returns
    // a short body that looks complete.
    bootseed::seed_system(&pool).await.unwrap();
    let (owner, emitter) = system_actor(&pool).await;
    let ctx = make_context(&pool, owner, "mixed").await;

    let with_bytes = block_with_raw(0, Some("block zero body\n"));
    let without_bytes = block_with_raw(1, None);
    let blocks = [with_bytes, without_bytes];
    let mut tx = pool.begin().await.unwrap();
    let id = fire(
        &mut tx,
        SeedAction::ResourceCreate {
            title: "mixed",
            origin_uri: "test://mixed",
            resource_id: None,
            home: AnchorRef::context(ctx),
            owner,
            originator: None,
            blocks: &blocks,
            doc_type: Some("concept"),
            emitter,
            segmented: false,
        },
    )
    .await
    .unwrap()
    .resource()
    .unwrap();
    tx.commit().await.unwrap();

    assert_eq!(
        body_storage(&pool, id).await,
        "derived",
        "partial coverage must NEVER be verbatim — that is how a short body looks complete"
    );
}

#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn superseded_revisions_keep_their_own_bytes(pool: sqlx::PgPool) {
    bootseed::seed_system(&pool).await.unwrap();
    let (owner, emitter) = system_actor(&pool).await;
    let ctx = make_context(&pool, owner, "supersede").await;

    let id = create_via_chunks_arm(&pool, ctx, owner, emitter, "v1 body\n").await;
    update_resource(
        &pool,
        UpdateParams {
            resource: id,
            body: Some("v2 body\n"),
            title: None,
            origin_uri: None,
            properties: &[],
            chunks: Some(vec![one_incoming_chunk("v2 body\n")]),
            sources: vec![],
            content_block: None,
            rehome_to: None,
            emitter,
        },
    )
    .await
    .unwrap();

    // Both revisions retain their own bytes (keyed by revision, superseded rows are never dropped),
    // ordered by the revision's created timestamp.
    let all: Vec<String> = sqlx::query_scalar(
        "SELECT bc.content FROM kb_block_content bc \
           JOIN kb_block_revisions r ON r.id = bc.block_revision_id \
           JOIN kb_content_blocks b  ON b.id = r.block_id \
          WHERE b.resource_id = $1 ORDER BY r.created",
    )
    .bind(id.uuid())
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(all, vec!["v1 body\n".to_string(), "v2 body\n".to_string()]);

    // The live revision (current_revision_id) carries the latest bytes, and the resource stays verbatim.
    let current: String = sqlx::query_scalar(
        "SELECT bc.content FROM kb_content_blocks b \
           JOIN kb_block_content bc ON bc.block_revision_id = b.current_revision_id \
          WHERE b.resource_id = $1",
    )
    .bind(id.uuid())
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(current, "v2 body\n");
    assert_eq!(body_storage(&pool, id).await, "verbatim");
}

#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn empty_body_with_real_chunks_is_derived_not_empty_verbatim(pool: sqlx::PgPool) {
    // The cognitive-map reconcile path creates/updates kernel resources with body="" — the content
    // rides in `chunks`, and the empty string is a "reblock from chunks, no whole-body prose" sentinel.
    // That MUST NOT store an empty verbatim row: `body_storage = 'verbatim'` over zero bytes is a
    // silent-empty-body trap that PR 4's coverage-verified readback would surface as an empty body for
    // a resource that actually has content. It must be 'derived'.
    bootseed::seed_system(&pool).await.unwrap();
    let (owner, emitter) = system_actor(&pool).await;
    let ctx = make_context(&pool, owner, "emptybody").await;

    let id = create_resource(
        &pool,
        CreateParams {
            title: "t",
            origin_uri: "test://empty",
            body: "", // the reconcile sentinel — real content is in the chunk below
            doc_type: "concept",
            home: AnchorRef::context(ctx),
            owner,
            originator: owner,
            emitter,
            properties: &[],
            chunks: Some(vec![one_incoming_chunk("real distilled content")]),
            sources: vec![],
        },
    )
    .await
    .unwrap();

    assert_eq!(
        body_storage(&pool, id).await,
        "derived",
        "an empty whole-body must be 'derived' — 'verbatim' over zero bytes is a silent-empty-body trap"
    );
    let content_rows: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM kb_content_blocks b \
           JOIN kb_block_content bc ON bc.block_revision_id = b.current_revision_id \
          WHERE b.resource_id = $1",
    )
    .bind(id.uuid())
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        content_rows, 0,
        "an empty body stores no verbatim content row (the chunk carries the real content instead)"
    );
}

/// One prepared block at `seq` whose raw bytes are `raw` (`None` ⇒ a legacy/derived block with no
/// stored bytes). A fixed non-degenerate 768-d unit embedding keeps it ONNX-free.
fn block_with_raw(seq: i32, raw: Option<&str>) -> PreparedBlock {
    let mut embedding = vec![0.0_f32; 768];
    embedding[0] = 1.0;
    PreparedBlock {
        block_id: BlockId::from(Uuid::now_v7()),
        seq,
        role: None,
        chunks: vec![PreparedChunk {
            chunk_id: ChunkId::from(Uuid::now_v7()),
            chunk_index: 0,
            content_hash: format!("{:064x}", Uuid::now_v7().as_u128()),
            content: format!("chunk text for block {seq}"),
            embedding: Some(embedding),
            embedded_with: None,
            header_path: None,
            heading_depth: None,
        }],
        incorporated: vec![],
        raw_text: raw.map(str::to_owned),
    }
}
