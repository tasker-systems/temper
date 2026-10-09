#![cfg(feature = "test-db")]
//! Build order 2e witnesses: the block history scrub in SQL (resource erasure spec 2026-09-28,
//! D11; migration 20261003000210). The act (`block_history_scrub_execute`) runs against a world
//! built through the REAL write paths; its effect is the one redaction body narrowed by its block
//! scope, never a second body.
//!
//! Spec witness 16 and the rulings of 2e, by letter:
//!   * **16a** — a live block keeps its present and loses its past: the current revision, its
//!     chunks, their embeddings and the search vector are unchanged; every older revision and
//!     every non-current chunk is empty and vectorless; an unnamed block of the same resource and
//!     the resource row, properties and search vector are untouched.
//!   * **16b** — edit and revert (A → B → A): the current revision still reads A, and the first
//!     revision, byte-identical to it, is emptied: keep-current is by revision id, never by bytes.
//!   * **16c** — a folded block empties entirely, both a block folded by a `replaces_body` mutate
//!     and a folded block whose chunks are still current; the embed drain's candidate set is the
//!     same before and after.
//!   * **16c, the drain** — the embed drain's write-backs re-check currency at write time: a
//!     vector computed from prose read before a supersede and a scrub lands nowhere, and a current
//!     chunk of a folded block takes none either.
//!   * **16d** — custody: a sibling resource in another home with byte-identical old content keeps
//!     every row, and `kb_erased_content` gains none.
//!   * **16e** — one `block_history_scrubbed` event and nothing else; its payload is the typed
//!     `BlockHistoryScrubbed`, keyed `kb_content_blocks` / `subject_ids`, one target per block.
//!   * **16f** — an in-flight ingest is cancelled, not refused (rulings 3 and 6): `cancelled`,
//!     `cancelled_ingest = true`, the ingest target line verbatim, and a later append refuses with
//!     TF004; a complete resource's payload carries neither.
//!   * **16f, then the erasure** — an erasure after the scrub cancelled the ingest names no
//!     ingest it ended; an erasure of an ingest still in flight does.
//!   * **16g** — the refusals RAISE (ruling 5) and change nothing; the widened
//!     `resource_erasure_refuse` records `act` and `blocks`, and its six-argument call still
//!     appends a payload without `act`.
//!   * **16h** — a tombstone is scrubbable (ruling 8) and stays a tombstone, never a husk.
//!   * **16i** — a per-block mutate racing the scrub serializes on R's row: the scrub waits for a
//!     mutate whose event is already minted, and live and replay stay byte-identical.
//!   * **16j** — the same race with a whole-body replace (`resource_reblock` folding the block).
//!   * **D10** — the survey reports exactly the counts the act's `targets` name, and those are the
//!     rows the act empties.

mod common;

use sha2::Digest;
use sqlx::PgPool;
use temper_core::types::ids::EntityId;
use temper_substrate::content::{prepare_block_from_chunks, IncomingChunk};
use temper_substrate::events::{fire, EventContext, SeedAction};
use temper_substrate::ids::{BlockId, ContextId, ProfileId, ResourceId};
use temper_substrate::payloads::{
    self, AnchorRef, AnchorTable, BlockHistoryScrubbed, ErasureAct, RecordedRefusalReason,
    ResourceErasureRefused,
};
use temper_substrate::replay;
use temper_substrate::writes::{self, AppendParams, CreateMode, CreateParams, UpdateParams};
use uuid::Uuid;

const SECRET: &str = "the plan and SSN 123-45-6789";
const REV_TWO: &str = "revision two: SSN 987-65-4321";
const REV_THREE: &str = "revision three: nothing sensitive";
const SIDE: &str = "an unnamed block naming jane smith";

/// The scrub's ingest target line, byte for byte (ruling 6 of 2e).
const INGEST_LINE: &str = "ingest in_progress; cancelled by block history scrub";

/// The embed drain's candidate read for one resource: `embed_resource_chunks`'s SELECT, built on
/// the production `STALE_CHUNK_PREDICATE` ($1 the resource, $2 the model this build embeds with).
fn drain_read_sql() -> String {
    format!(
        "SELECT ch.id AS chunk_id, cc.content \
         FROM kb_chunks ch \
         JOIN kb_chunk_content cc ON cc.chunk_id = ch.id \
         JOIN kb_content_blocks b ON b.id = ch.block_id \
         WHERE ch.resource_id = $1 AND {} \
         ORDER BY ch.chunk_index",
        temper_substrate::embed::STALE_CHUNK_PREDICATE
    )
}

fn chunk_hash(prose: &str) -> String {
    format!("{:x}", sha2::Sha256::digest(prose.trim()))
}

fn chunk(prose: &str, header: &str) -> IncomingChunk {
    IncomingChunk {
        chunk_index: 0,
        content_hash: chunk_hash(prose),
        content: prose.to_string(),
        embedding: vec![0.1; 768],
        embedded_with: Some("model-sha-1".to_string()),
        header_path: header.to_string(),
        heading_depth: if header.is_empty() { 0 } else { 1 },
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

async fn make_home(pool: &PgPool, owner: ProfileId, slug: &str) -> ContextId {
    ContextId::from(
        common::insert_context(pool, "kb_profiles", owner.uuid(), slug, slug)
            .await
            .unwrap(),
    )
}

async fn setup(pool: &PgPool) -> (ProfileId, EntityId) {
    common::reset_schema(pool).await;
    temper_substrate::scenario::bootseed::seed_system(pool)
        .await
        .unwrap();
    system_actor(pool).await
}

/// The SQLSTATE of the database error under a substrate write's `anyhow` chain, if any.
fn sqlstate(e: &anyhow::Error) -> Option<String> {
    e.chain()
        .find_map(|cause| cause.downcast_ref::<sqlx::Error>())
        .and_then(|sqlx_err| sqlx_err.as_database_error())
        .and_then(|db| db.code())
        .map(|code| code.into_owned())
}

/// A complete resource with one block, through the create path.
async fn create(
    pool: &PgPool,
    owner: ProfileId,
    emitter: EntityId,
    home: ContextId,
    slug: &str,
    body: &str,
) -> ResourceId {
    let origin = format!("test://{slug}");
    writes::create_resource_with(
        pool,
        CreateParams {
            idempotency_key: None,
            title: slug,
            origin_uri: &origin,
            body,
            doc_type: "research",
            home: AnchorRef::context(home),
            owner,
            originator: owner,
            emitter,
            properties: &[],
            chunks: Some(vec![chunk(body, "first")]),
            sources: vec![],
        },
        EventContext::default(),
    )
    .await
    .expect("create through the create path")
}

/// One per-block revise: a new revision of `block`, superseding its chunk.
async fn revise(pool: &PgPool, resource: ResourceId, block: Uuid, prose: &str, emitter: EntityId) {
    writes::update_resource(
        pool,
        UpdateParams {
            resource,
            body: Some(prose),
            title: None,
            origin_uri: None,
            properties: &[],
            unset_keys: &[],
            chunks: Some(vec![chunk(prose, "revised")]),
            sources: vec![],
            content_block: Some(block),
            rehome_to: None,
            emitter,
        },
    )
    .await
    .expect("a per-block revise");
}

/// Append one block at `seq` through the segmented-ingest write.
async fn append(
    pool: &PgPool,
    resource: ResourceId,
    seq: i32,
    prose: &str,
    emitter: EntityId,
) -> Uuid {
    let mut block = prepare_block_from_chunks(seq, None, vec![chunk(prose, "side")]);
    block.raw_text = Some(prose.to_owned());
    writes::append_block(
        pool,
        AppendParams {
            resource,
            block: &block,
            sources: vec![],
            emitter,
        },
    )
    .await
    .expect("append a block")
    .uuid()
}

async fn first_live_block(pool: &PgPool, resource: ResourceId) -> Uuid {
    sqlx::query_scalar(
        "SELECT id FROM kb_content_blocks WHERE resource_id = $1 AND NOT is_folded \
          ORDER BY seq LIMIT 1",
    )
    .bind(resource.uuid())
    .fetch_one(pool)
    .await
    .unwrap()
}

/// Every row of one block, every column: the block, its revisions and their bytes, its chunks
/// (embedding as text) and their prose.
async fn block_snapshot(pool: &PgPool, block: Uuid) -> String {
    sqlx::query_scalar(
        "SELECT jsonb_build_object(
            'block', (SELECT to_jsonb(b) FROM kb_content_blocks b WHERE b.id = $1),
            'revisions', (SELECT jsonb_agg(to_jsonb(br) ORDER BY br.id)
                            FROM kb_block_revisions br WHERE br.block_id = $1),
            'bytes', (SELECT jsonb_agg(to_jsonb(bc) ORDER BY bc.block_revision_id)
                        FROM kb_block_content bc
                        JOIN kb_block_revisions br ON br.id = bc.block_revision_id
                       WHERE br.block_id = $1),
            'chunks', (SELECT jsonb_agg(to_jsonb(c) || jsonb_build_object('embedding', c.embedding::text)
                                        ORDER BY c.id)
                         FROM kb_chunks c WHERE c.block_id = $1),
            'prose', (SELECT jsonb_agg(to_jsonb(cc) ORDER BY cc.chunk_id)
                        FROM kb_chunk_content cc JOIN kb_chunks c ON c.id = cc.chunk_id
                       WHERE c.block_id = $1))::text",
    )
    .bind(block)
    .fetch_one(pool)
    .await
    .unwrap()
}

/// Every block of `resource`, snapshotted.
async fn resource_blocks_snapshot(pool: &PgPool, resource: ResourceId) -> Vec<String> {
    let blocks: Vec<Uuid> =
        sqlx::query_scalar("SELECT id FROM kb_content_blocks WHERE resource_id = $1 ORDER BY id")
            .bind(resource.uuid())
            .fetch_all(pool)
            .await
            .unwrap();
    let mut out = Vec::new();
    for b in blocks {
        out.push(block_snapshot(pool, b).await);
    }
    out
}

/// The resource-grain state steps (4)–(9) would touch: the resource row, its properties and its
/// search vector.
async fn resource_snapshot(
    pool: &PgPool,
    resource: ResourceId,
) -> (String, String, Option<String>) {
    sqlx::query_as(
        "SELECT (SELECT to_jsonb(r)::text FROM kb_resources r WHERE r.id = $1),
                coalesce((SELECT jsonb_agg(to_jsonb(p) ORDER BY to_jsonb(p)::text)::text
                            FROM kb_properties p
                           WHERE p.owner_table = 'kb_resources' AND p.owner_id = $1), '[]'),
                (SELECT si.search_vector::text FROM kb_resource_search_index si
                  WHERE si.resource_id = $1)",
    )
    .bind(resource.uuid())
    .fetch_one(pool)
    .await
    .unwrap()
}

async fn event_ids(pool: &PgPool) -> Vec<Uuid> {
    sqlx::query_scalar("SELECT id FROM kb_events")
        .fetch_all(pool)
        .await
        .unwrap()
}

/// The type names of the events appended since `before`, sorted.
async fn new_event_types(pool: &PgPool, before: &[Uuid]) -> Vec<String> {
    sqlx::query_scalar(
        "SELECT t.name FROM kb_events e JOIN kb_event_types t ON t.id = e.event_type_id \
          WHERE NOT (e.id = ANY($1)) ORDER BY t.name",
    )
    .bind(before.to_vec())
    .fetch_all(pool)
    .await
    .unwrap()
}

/// The act, as the system admin, under a fresh request reference.
async fn try_scrub(
    pool: &PgPool,
    resource: Uuid,
    blocks: &[Uuid],
) -> Result<serde_json::Value, sqlx::Error> {
    let (owner, emitter) = system_actor(pool).await;
    sqlx::query_scalar::<_, serde_json::Value>(
        "SELECT block_history_scrub_execute($1, $2, $3, $4, $5)",
    )
    .bind(resource)
    .bind(blocks.to_vec())
    .bind(owner.uuid())
    .bind(emitter)
    .bind(Uuid::now_v7())
    .fetch_one(pool)
    .await
}

/// [`try_scrub`] that must complete; returns `{event_id, targets}`.
async fn scrub(pool: &PgPool, resource: ResourceId, blocks: &[Uuid]) -> serde_json::Value {
    try_scrub(pool, resource.uuid(), blocks)
        .await
        .expect("the scrub completes")
}

fn event_of(result: &serde_json::Value) -> Uuid {
    Uuid::parse_str(result["event_id"].as_str().expect("event_id is a string"))
        .expect("event_id parses")
}

async fn payload_of(pool: &PgPool, event: Uuid) -> serde_json::Value {
    sqlx::query_scalar("SELECT payload FROM kb_events WHERE id = $1")
        .bind(event)
        .fetch_one(pool)
        .await
        .unwrap()
}

async fn survey(pool: &PgPool, resource: ResourceId, blocks: &[Uuid]) -> serde_json::Value {
    sqlx::query_scalar("SELECT block_history_scrub_survey($1, $2)")
        .bind(resource.uuid())
        .bind(blocks.to_vec())
        .fetch_one(pool)
        .await
        .unwrap()
}

/// The `kb_content_blocks` target line the act writes for one block.
fn block_line(block: Uuid, folded: bool, revisions: i64, chunks: i64) -> serde_json::Value {
    let state = if folded { "folded" } else { "live" };
    serde_json::json!({
        "target": "kb_content_blocks",
        "outcome": format!(
            "block {block} {state}: {revisions} revisions emptied, {chunks} chunks emptied"
        ),
    })
}

/// (non-current revisions with bytes, non-current chunks with prose, a header_path, an embedding
/// or embedded_with) of one block: what the scrub must leave at zero for a live block.
async fn history_left(pool: &PgPool, block: Uuid) -> (i64, i64) {
    sqlx::query_as(
        "SELECT (SELECT count(*) FROM kb_block_revisions br
                   JOIN kb_content_blocks b ON b.id = br.block_id
                   JOIN kb_block_content bc ON bc.block_revision_id = br.id
                  WHERE br.block_id = $1 AND br.id IS DISTINCT FROM b.current_revision_id
                    AND bc.content <> ''),
                (SELECT count(*) FROM kb_chunks c
                   LEFT JOIN kb_chunk_content cc ON cc.chunk_id = c.id
                  WHERE c.block_id = $1 AND NOT c.is_current
                    AND (cc.content <> '' OR c.header_path IS NOT NULL
                         OR c.embedding IS NOT NULL OR c.embedded_with IS NOT NULL))",
    )
    .bind(block)
    .fetch_one(pool)
    .await
    .unwrap()
}

/// (revisions with bytes, chunks with prose, a header_path, an embedding or embedded_with) of one
/// block, current or not: what the scrub must leave at zero for a folded block.
async fn anything_left(pool: &PgPool, block: Uuid) -> (i64, i64) {
    sqlx::query_as(
        "SELECT (SELECT count(*) FROM kb_block_revisions br
                   JOIN kb_block_content bc ON bc.block_revision_id = br.id
                  WHERE br.block_id = $1 AND bc.content <> ''),
                (SELECT count(*) FROM kb_chunks c
                   LEFT JOIN kb_chunk_content cc ON cc.chunk_id = c.id
                  WHERE c.block_id = $1
                    AND (cc.content <> '' OR c.header_path IS NOT NULL
                         OR c.embedding IS NOT NULL OR c.embedded_with IS NOT NULL))",
    )
    .bind(block)
    .fetch_one(pool)
    .await
    .unwrap()
}

/// The present of one live block: its current revision's bytes, and each current chunk's prose,
/// header_path, embedding (as text) and embedded_with.
async fn present_of(pool: &PgPool, block: Uuid) -> (Option<String>, String) {
    sqlx::query_as(
        "SELECT (SELECT bc.content FROM kb_content_blocks b
                   JOIN kb_block_content bc ON bc.block_revision_id = b.current_revision_id
                  WHERE b.id = $1),
                (SELECT jsonb_agg(jsonb_build_object(
                            'id', c.id, 'content', cc.content, 'header_path', c.header_path,
                            'embedding', c.embedding::text, 'embedded_with', c.embedded_with)
                        ORDER BY c.id)::text
                   FROM kb_chunks c JOIN kb_chunk_content cc ON cc.chunk_id = c.id
                  WHERE c.block_id = $1 AND c.is_current)",
    )
    .bind(block)
    .fetch_one(pool)
    .await
    .unwrap()
}

/// Every revision's bytes of one block, oldest first.
async fn revision_bytes(pool: &PgPool, block: Uuid) -> Vec<String> {
    sqlx::query_scalar(
        "SELECT bc.content FROM kb_block_revisions br
           JOIN kb_block_content bc ON bc.block_revision_id = br.id
          WHERE br.block_id = $1 ORDER BY br.created, br.id",
    )
    .bind(block)
    .fetch_all(pool)
    .await
    .unwrap()
}

/// The embed drain's candidates across `resources`, read as the drain reads them, sorted.
async fn drain_candidates(pool: &PgPool, resources: &[ResourceId]) -> Vec<(Uuid, String)> {
    let sql = drain_read_sql();
    let mut rows: Vec<(Uuid, String)> = Vec::new();
    for resource in resources {
        let read: Vec<(Uuid, String)> = sqlx::query_as(&sql)
            .bind(resource.uuid())
            .bind(temper_ingest::embed::EXPECTED_MODEL_SHA256)
            .fetch_all(pool)
            .await
            .unwrap();
        rows.extend(read);
    }
    rows.sort();
    rows
}

async fn erased_content_rows(pool: &PgPool) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM kb_erased_content")
        .fetch_one(pool)
        .await
        .unwrap()
}

async fn ingest_state(pool: &PgPool, resource: ResourceId) -> String {
    sqlx::query_scalar("SELECT ingest_state FROM kb_resources WHERE id = $1")
        .bind(resource.uuid())
        .fetch_one(pool)
        .await
        .unwrap()
}

/// A resource whose first block has three revisions (create, then two per-block revises, each
/// superseding the prior chunk) and a second live block, unnamed by the scrubs that use it.
struct History {
    resource: ResourceId,
    block: Uuid,
    unnamed: Uuid,
}

async fn seed_history(pool: &PgPool, owner: ProfileId, emitter: EntityId, slug: &str) -> History {
    let home = make_home(pool, owner, slug).await;
    let resource = create(pool, owner, emitter, home, slug, SECRET).await;
    let block = first_live_block(pool, resource).await;
    // Revise before the second block exists, as the act's history witness does: a per-block
    // revise re-applies the blocking policy to the whole resource.
    for prose in [REV_TWO, REV_THREE] {
        revise(pool, resource, block, prose, emitter).await;
    }
    let unnamed = append(pool, resource, 1, SIDE, emitter).await;
    let revisions: i64 =
        sqlx::query_scalar("SELECT count(*) FROM kb_block_revisions WHERE block_id = $1")
            .bind(block)
            .fetch_one(pool)
            .await
            .unwrap();
    assert_eq!(revisions, 3, "setup: three revisions of the named block");
    let (old_revisions, old_chunks) = history_left(pool, block).await;
    assert_eq!(
        (old_revisions, old_chunks),
        (2, 2),
        "setup: two older revisions with bytes and two superseded chunks with prose, heading \
         and embedding"
    );
    History {
        resource,
        block,
        unnamed,
    }
}

/// (16a) The scrub keeps the present and empties the past.
///
/// FAILS IF: the current revision or a current chunk of the named block changes in any of
/// content, header_path, embedding or embedded_with; an older revision or a superseded chunk keeps
/// anything; the unnamed block changes in any column; or the resource row, its properties or its
/// search vector change (steps (4)–(9) ran).
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn the_scrub_keeps_the_present_and_empties_the_past(pool: PgPool) {
    let (owner, emitter) = setup(&pool).await;
    let h = seed_history(&pool, owner, emitter, "scrub-present").await;

    let present_before = present_of(&pool, h.block).await;
    assert_eq!(
        present_before.0.as_deref(),
        Some(REV_THREE),
        "setup: the current revision's bytes are the last revise"
    );
    let unnamed_before = block_snapshot(&pool, h.unnamed).await;
    let resource_before = resource_snapshot(&pool, h.resource).await;
    assert!(
        resource_before.2.as_deref().is_some_and(|v| !v.is_empty()),
        "setup: the resource has a search vector to keep; got {:?}",
        resource_before.2
    );

    scrub(&pool, h.resource, &[h.block]).await;

    assert_eq!(
        present_of(&pool, h.block).await,
        present_before,
        "the current revision and its current chunks (prose, header_path, embedding, \
         embedded_with) are unchanged"
    );
    assert_eq!(
        history_left(&pool, h.block).await,
        (0, 0),
        "both older revisions are '' and every superseded chunk has empty prose, NULL \
         header_path, and NULL embedding and embedded_with together"
    );
    let older: Vec<String> = sqlx::query_scalar(
        "SELECT bc.content FROM kb_block_revisions br
           JOIN kb_content_blocks b ON b.id = br.block_id
           JOIN kb_block_content bc ON bc.block_revision_id = br.id
          WHERE br.block_id = $1 AND br.id <> b.current_revision_id",
    )
    .bind(h.block)
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(
        older,
        vec![String::new(), String::new()],
        "emptied revisions stay rows"
    );
    assert_eq!(
        block_snapshot(&pool, h.unnamed).await,
        unnamed_before,
        "the unnamed block is untouched in every column"
    );
    assert_eq!(
        resource_snapshot(&pool, h.resource).await,
        resource_before,
        "the resource row (title, is_active, erased_at), its properties and its search vector \
         are unchanged"
    );
}

/// (16b) Edit and revert, A → B → A: keep-current is by revision id, not by bytes.
///
/// FAILS IF: the current revision no longer reads A, or the first revision (byte-identical to the
/// current one) keeps its bytes.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn an_edit_and_revert_keeps_the_current_and_empties_the_identical_first(pool: PgPool) {
    let (owner, emitter) = setup(&pool).await;
    let home = make_home(&pool, owner, "scrub-revert").await;
    let resource = create(&pool, owner, emitter, home, "scrub-revert", SECRET).await;
    let block = first_live_block(&pool, resource).await;
    revise(&pool, resource, block, REV_TWO, emitter).await;
    revise(&pool, resource, block, SECRET, emitter).await;

    assert_eq!(
        revision_bytes(&pool, block).await,
        vec![SECRET.to_string(), REV_TWO.to_string(), SECRET.to_string()],
        "setup: revisions A, B, A, each with its bytes"
    );

    scrub(&pool, resource, &[block]).await;

    assert_eq!(
        revision_bytes(&pool, block).await,
        vec![String::new(), String::new(), SECRET.to_string()],
        "the first revision is '' though its bytes equal the current one; the current reads A"
    );
    assert_eq!(
        present_of(&pool, block).await.0.as_deref(),
        Some(SECRET),
        "the current revision is the one kept"
    );
}

/// (16c) A folded block empties entirely, whichever way it was folded, and the embed drain's
/// candidate set does not move.
///
/// Two folds. One is a `replaces_body` block mutate, the real path that folds siblings and
/// retires their chunks. The other must be a folded block whose chunks are still `is_current`,
/// and no write path reachable here leaves one on a scrubbable resource: `replaces_body` (the
/// mutate and the re-block) retires the folded chunks, the re-block's op shape reparents every
/// current chunk of a folded block onto the created ones, and the charter set, which does leave
/// current chunks on folded blocks, only ever folds a charter resource, which the scrub refuses.
/// So that state is seeded directly, with a plain `UPDATE kb_content_blocks SET is_folded = true`
/// on a live block, to pin the body's `b.is_folded` arm on chunks that `NOT c.is_current` would
/// miss.
///
/// FAILS IF: any revision or chunk of either folded block keeps prose, bytes, a header_path, an
/// embedding or embedded_with; or the drain's candidates differ before and after.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn a_folded_block_empties_entirely(pool: PgPool) {
    let (owner, emitter) = setup(&pool).await;
    let home = make_home(&pool, owner, "scrub-folded").await;

    // A block folded by a `replaces_body` mutate of its sibling.
    let resource = create(&pool, owner, emitter, home, "scrub-folded", SECRET).await;
    let block = first_live_block(&pool, resource).await;
    let folded = append(&pool, resource, 1, SIDE, emitter).await;
    const WHOLE: &str = "the whole body, replaced";
    let replacement = prepare_block_from_chunks(0, None, vec![chunk(WHOLE, "")]);
    let mut tx = pool.begin().await.unwrap();
    fire(
        &mut tx,
        SeedAction::BlockMutate {
            block: BlockId::from(block),
            chunks: &replacement.chunks,
            raw: Some(WHOLE),
            incorporated: &[],
            replaces_body: true,
            emitter,
        },
    )
    .await
    .unwrap();
    tx.commit().await.unwrap();
    let (is_folded, current_chunks): (bool, i64) = sqlx::query_as(
        "SELECT b.is_folded, (SELECT count(*) FROM kb_chunks c WHERE c.block_id = b.id AND c.is_current)
           FROM kb_content_blocks b WHERE b.id = $1",
    )
    .bind(folded)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(
        is_folded && current_chunks == 0,
        "setup: the replaces_body mutate folded the block and retired its chunks"
    );

    // A folded block whose chunk is still current (seeded directly; see the doc comment).
    let still_current = create(&pool, owner, emitter, home, "scrub-folded-current", SECRET).await;
    let kept_live = first_live_block(&pool, still_current).await;
    let folded_current = append(&pool, still_current, 1, SIDE, emitter).await;
    sqlx::query("UPDATE kb_content_blocks SET is_folded = true WHERE id = $1")
        .bind(folded_current)
        .execute(&pool)
        .await
        .unwrap();
    let current_on_folded: i64 =
        sqlx::query_scalar("SELECT count(*) FROM kb_chunks WHERE block_id = $1 AND is_current")
            .bind(folded_current)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(
        current_on_folded, 1,
        "setup: the folded block's chunk is still current"
    );
    for b in [folded, folded_current] {
        let (revisions, chunks) = anything_left(&pool, b).await;
        assert!(
            revisions > 0 && chunks > 0,
            "setup: block {b} has bytes and chunk content to empty"
        );
    }
    let kept_before = block_snapshot(&pool, kept_live).await;

    let drain_before = drain_candidates(&pool, &[resource, still_current]).await;
    assert!(
        !drain_before.is_empty(),
        "setup: the live blocks' current chunks are drain candidates (declared with another model)"
    );

    let first = scrub(&pool, resource, &[folded]).await;
    let second = scrub(&pool, still_current, &[folded_current]).await;

    for b in [folded, folded_current] {
        assert_eq!(
            anything_left(&pool, b).await,
            (0, 0),
            "folded block {b}: every revision is '' and every chunk, current or not, is emptied"
        );
    }
    assert_eq!(
        drain_candidates(&pool, &[resource, still_current]).await,
        drain_before,
        "the embed drain's candidates are the same before and after: nothing re-embeds"
    );
    assert_eq!(
        block_snapshot(&pool, kept_live).await,
        kept_before,
        "the live block beside the folded one is untouched"
    );
    assert_eq!(first["targets"][0], block_line(folded, true, 1, 1));
    assert_eq!(second["targets"][0], block_line(folded_current, true, 1, 1));
}

/// A 768-dim vector literal, as the drain formats one.
fn vector_literal() -> String {
    format!("[{}]", vec!["0.1"; 768].join(","))
}

/// Run the drain's vector write-back (`CHUNK_EMBEDDING_WRITE_BACK`) and its blank stamp
/// (`stamp_blank_chunks`) for one chunk; returns the rows each affected.
async fn drain_write_backs(pool: &PgPool, chunk: Uuid) -> (u64, u64) {
    let model = temper_ingest::embed::EXPECTED_MODEL_SHA256;
    let vector = sqlx::query(temper_substrate::embed::CHUNK_EMBEDDING_WRITE_BACK)
        .bind(vector_literal())
        .bind(model)
        .bind(chunk)
        .execute(pool)
        .await
        .unwrap()
        .rows_affected();
    let stamp = temper_substrate::embed::stamp_blank_chunks(pool, model, &[chunk])
        .await
        .unwrap();
    (vector, stamp)
}

/// One chunk's (embedding IS NOT NULL, embedded_with).
async fn chunk_vector(pool: &PgPool, chunk: Uuid) -> (bool, Option<String>) {
    sqlx::query_as("SELECT embedding IS NOT NULL, embedded_with FROM kb_chunks WHERE id = $1")
        .bind(chunk)
        .fetch_one(pool)
        .await
        .unwrap()
}

/// The current chunks of `block`, with neither a vector nor provenance: the state a create whose
/// embedding the drain has not reached yet leaves (both columns NULL together).
async fn unembed_current_chunks(pool: &PgPool, block: Uuid) {
    sqlx::query(
        "UPDATE kb_chunks SET embedding = NULL, embedded_with = NULL \
          WHERE block_id = $1 AND is_current",
    )
    .bind(block)
    .execute(pool)
    .await
    .unwrap();
}

/// The current chunk ids of `block`.
async fn current_chunks(pool: &PgPool, block: Uuid) -> Vec<Uuid> {
    sqlx::query_scalar("SELECT id FROM kb_chunks WHERE block_id = $1 AND is_current ORDER BY id")
        .bind(block)
        .fetch_all(pool)
        .await
        .unwrap()
}

/// (16c, the drain) A write-back that read its candidate before a supersede and a scrub writes
/// nothing onto the emptied chunk.
///
/// The drain reads candidates under `STALE_CHUNK_PREDICATE`, runs inference, and only then writes.
/// In between, the block is mutated (the chunk becomes history) and scrubbed (the chunk's prose,
/// header path and vector are emptied). The drain's own statements then run for that chunk.
///
/// FAILS IF: `CHUNK_EMBEDDING_WRITE_BACK` or `stamp_blank_chunks` affects the
/// scrubbed chunk, or its embedding or embedded_with is no longer NULL; or the same write-back does
/// not write the block's new current chunk (the refusal is the currency guard's, not a statement
/// that never writes).
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn a_drain_write_back_after_a_supersede_and_a_scrub_writes_nothing(pool: PgPool) {
    let (owner, emitter) = setup(&pool).await;
    let home = make_home(&pool, owner, "scrub-drain").await;
    let resource = create(&pool, owner, emitter, home, "scrub-drain", SECRET).await;
    let block = first_live_block(&pool, resource).await;
    unembed_current_chunks(&pool, block).await;
    let read_chunks = current_chunks(&pool, block).await;
    assert_eq!(read_chunks.len(), 1, "setup: one current chunk");
    let read_chunk = read_chunks[0];

    // The drain reads its candidate.
    let candidates = drain_candidates(&pool, &[resource]).await;
    assert_eq!(
        candidates,
        vec![(read_chunk, SECRET.to_string())],
        "setup: the drain reads the unembedded current chunk and its prose"
    );

    // The block is mutated through the real write path: the read chunk becomes history.
    let edited = prepare_block_from_chunks(0, None, vec![chunk(REV_THREE, "edited")]);
    let mut tx = pool.begin().await.unwrap();
    fire(
        &mut tx,
        SeedAction::BlockMutate {
            block: BlockId::from(block),
            chunks: &edited.chunks,
            raw: Some(REV_THREE),
            incorporated: &[],
            replaces_body: false,
            emitter,
        },
    )
    .await
    .unwrap();
    tx.commit().await.unwrap();
    let now_current = current_chunks(&pool, block).await;
    assert!(
        !now_current.is_empty() && !now_current.contains(&read_chunk),
        "setup: the mutate superseded the read chunk"
    );

    // The scrub empties it.
    scrub(&pool, resource, &[block]).await;
    assert_eq!(
        history_left(&pool, block).await,
        (0, 0),
        "setup: the scrub emptied the block's history"
    );

    // The drain's write-backs arrive.
    assert_eq!(
        drain_write_backs(&pool, read_chunk).await,
        (0, 0),
        "neither the vector write-back nor the blank stamp reaches a chunk that is no longer \
         current"
    );
    assert_eq!(
        chunk_vector(&pool, read_chunk).await,
        (false, None),
        "the scrubbed chunk keeps a NULL embedding and a NULL embedded_with"
    );

    // The same statement writes the block's present.
    unembed_current_chunks(&pool, block).await;
    let (vector, _) = drain_write_backs(&pool, now_current[0]).await;
    assert_eq!(
        vector, 1,
        "the write-back writes a current chunk of a live block"
    );
}

/// (16c, the drain, the fold conjunct) A write-back onto a current chunk of a folded block writes
/// nothing. The chunk is still `is_current`, so only the block's fold refuses it; the state is
/// seeded directly, as in `a_folded_block_empties_entirely`.
///
/// FAILS IF: `CHUNK_EMBEDDING_WRITE_BACK` or `stamp_blank_chunks` affects the
/// folded block's current chunk, or its embedding or embedded_with is no longer NULL.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn a_drain_write_back_onto_a_folded_block_writes_nothing(pool: PgPool) {
    let (owner, emitter) = setup(&pool).await;
    let home = make_home(&pool, owner, "scrub-drain-fold").await;
    let resource = create(&pool, owner, emitter, home, "scrub-drain-fold", SECRET).await;
    let folded = append(&pool, resource, 1, SIDE, emitter).await;
    unembed_current_chunks(&pool, folded).await;
    let chunks = current_chunks(&pool, folded).await;
    assert_eq!(chunks.len(), 1, "setup: one current chunk");
    assert!(
        drain_candidates(&pool, &[resource])
            .await
            .iter()
            .any(|(id, _)| *id == chunks[0]),
        "setup: the drain reads the chunk while its block is live"
    );
    sqlx::query("UPDATE kb_content_blocks SET is_folded = true WHERE id = $1")
        .bind(folded)
        .execute(&pool)
        .await
        .unwrap();
    scrub(&pool, resource, &[folded]).await;

    assert_eq!(
        drain_write_backs(&pool, chunks[0]).await,
        (0, 0),
        "neither write-back reaches a current chunk of a folded block"
    );
    assert_eq!(
        chunk_vector(&pool, chunks[0]).await,
        (false, None),
        "the folded block's chunk keeps a NULL embedding and a NULL embedded_with"
    );
}

/// (16d) Custody is never decided by bytes: a sibling resource in another home carrying
/// byte-identical old content keeps every row, and `kb_erased_content` gains none.
///
/// FAILS IF: any row of the twin's blocks changes, `kb_erased_content` grows, or the scrubbed
/// resource's older revision keeps the shared bytes.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn a_twin_with_identical_bytes_keeps_every_row(pool: PgPool) {
    let (owner, emitter) = setup(&pool).await;
    let home = make_home(&pool, owner, "scrub-custody").await;
    let twin_home = make_home(&pool, owner, "scrub-custody-twin").await;
    let resource = create(&pool, owner, emitter, home, "scrub-custody", SECRET).await;
    let block = first_live_block(&pool, resource).await;
    revise(&pool, resource, block, REV_THREE, emitter).await;
    let twin = create(
        &pool,
        owner,
        emitter,
        twin_home,
        "scrub-custody-twin",
        SECRET,
    )
    .await;

    let twin_before = resource_blocks_snapshot(&pool, twin).await;
    assert!(
        twin_before.iter().any(|s| s.contains(SECRET)),
        "setup: the twin carries the same bytes"
    );
    let erased_before = erased_content_rows(&pool).await;

    scrub(&pool, resource, &[block]).await;

    assert_eq!(
        history_left(&pool, block).await,
        (0, 0),
        "R's history is emptied"
    );
    assert_eq!(
        resource_blocks_snapshot(&pool, twin).await,
        twin_before,
        "the twin keeps every row in every column"
    );
    assert_eq!(
        erased_content_rows(&pool).await,
        erased_before,
        "kb_erased_content gains no row"
    );
}

/// (16e) The scrub appends exactly one event, `block_history_scrubbed`, whose payload is the typed
/// `BlockHistoryScrubbed`.
///
/// FAILS IF: any other event is appended (a `block_mutated`, `block_created` or `block_folded`
/// among them), the payload does not deserialize, it carries a key the type does not name, its
/// subject is not `kb_content_blocks` / the named blocks, or `targets` is not one line per block
/// naming the plan's counts.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn the_scrub_records_one_event_and_fires_nothing_else(pool: PgPool) {
    let (owner, emitter) = setup(&pool).await;
    let h = seed_history(&pool, owner, emitter, "scrub-record").await;
    let before = event_ids(&pool).await;

    let result = scrub(&pool, h.resource, &[h.block, h.unnamed]).await;

    assert_eq!(
        new_event_types(&pool, &before).await,
        vec!["block_history_scrubbed".to_string()],
        "exactly one event, and no block_mutated, block_created or block_folded"
    );
    let raw = payload_of(&pool, event_of(&result)).await;
    let mut keys: Vec<&str> = raw
        .as_object()
        .expect("the payload is an object")
        .keys()
        .map(String::as_str)
        .collect();
    keys.sort_unstable();
    assert_eq!(
        keys,
        vec!["actor", "subject_ids", "subject_table", "targets"],
        "no other keys; cancelled_ingest is absent on a complete resource"
    );
    let payload: BlockHistoryScrubbed =
        serde_json::from_value(raw.clone()).expect("the payload is a BlockHistoryScrubbed");
    assert_eq!(payload.subject_table, AnchorTable::ContentBlocks);
    assert_eq!(payload.subject_ids, vec![h.block, h.unnamed]);
    assert_eq!(payload.actor, Some(owner));
    assert!(!payload.cancelled_ingest);
    assert_eq!(
        raw["targets"],
        serde_json::json!([
            block_line(h.block, false, 2, 2),
            block_line(h.unnamed, false, 0, 0),
        ]),
        "one kb_content_blocks line per block, in the named order, with the plan's counts"
    );
    assert_eq!(
        result["targets"], raw["targets"],
        "the act returns the recorded targets"
    );
    payloads::verify_ledger_roundtrip(&pool)
        .await
        .expect("every typed payload on the ledger roundtrips, the scrub's included");
}

/// (16f) An in-flight ingest is cancelled by the scrub, not a refusal; a complete one is left alone.
///
/// FAILS IF: the scrubbed in-flight resource is not `cancelled`; its payload lacks
/// `cancelled_ingest = true` or the ingest target line, verbatim; an append after the scrub lands
/// instead of refusing with TF004; or a complete resource's payload carries the key or the line.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn an_in_flight_ingest_is_cancelled_by_the_scrub(pool: PgPool) {
    let (owner, emitter) = setup(&pool).await;
    let home = make_home(&pool, owner, "scrub-ingest").await;
    let in_flight = writes::create_resource_with_mode(
        &pool,
        CreateParams {
            idempotency_key: None,
            title: "mid-ingest",
            origin_uri: "test://scrub-mid-ingest",
            body: "block zero",
            doc_type: "research",
            home: AnchorRef::context(home),
            owner,
            originator: owner,
            emitter,
            properties: &[],
            chunks: Some(vec![chunk("block zero", "")]),
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
    append(&pool, in_flight, 1, "segment one", emitter).await;
    assert_eq!(
        ingest_state(&pool, in_flight).await,
        "in_progress",
        "setup: an ingest actually in flight"
    );
    let block = first_live_block(&pool, in_flight).await;

    let result = scrub(&pool, in_flight, &[block]).await;

    assert_eq!(
        ingest_state(&pool, in_flight).await,
        "cancelled",
        "the scrub cancelled the ingest"
    );
    let raw = payload_of(&pool, event_of(&result)).await;
    assert_eq!(raw["cancelled_ingest"], serde_json::json!(true));
    let payload: BlockHistoryScrubbed =
        serde_json::from_value(raw.clone()).expect("the payload is a BlockHistoryScrubbed");
    assert!(payload.cancelled_ingest, "replay's field is set");
    assert_eq!(
        raw["targets"],
        serde_json::json!([
            block_line(block, false, 0, 0),
            {"target": "kb_resources.ingest_state", "outcome": INGEST_LINE},
        ]),
        "the block's line, then the ingest line verbatim"
    );

    let late = prepare_block_from_chunks(2, None, vec![chunk("segment two", "")]);
    let err = writes::append_block(
        &pool,
        AppendParams {
            resource: in_flight,
            block: &late,
            sources: vec![],
            emitter,
        },
    )
    .await
    .expect_err("an append after the cancel is refused");
    assert_eq!(
        sqlstate(&err).as_deref(),
        Some("TF004"),
        "the cancelled ingest refuses the append with TF004; got {err:#}"
    );

    // A complete resource: neither the key nor the line.
    let complete = create(&pool, owner, emitter, home, "scrub-complete", SECRET).await;
    let complete_block = first_live_block(&pool, complete).await;
    let result = scrub(&pool, complete, &[complete_block]).await;
    let raw = payload_of(&pool, event_of(&result)).await;
    assert!(
        raw.get("cancelled_ingest").is_none(),
        "a complete resource's payload omits cancelled_ingest; got {raw}"
    );
    assert!(
        !raw["targets"]
            .to_string()
            .contains("kb_resources.ingest_state"),
        "a complete resource's targets carry no ingest line; got {}",
        raw["targets"]
    );
    assert_eq!(ingest_state(&pool, complete).await, "complete");
}

/// A segmented create left in flight: block zero and one appended segment, never finalized.
async fn in_flight_ingest(
    pool: &PgPool,
    owner: ProfileId,
    emitter: EntityId,
    slug: &str,
) -> ResourceId {
    let home = make_home(pool, owner, slug).await;
    let origin = format!("test://{slug}");
    let resource = writes::create_resource_with_mode(
        pool,
        CreateParams {
            idempotency_key: None,
            title: slug,
            origin_uri: &origin,
            body: "block zero",
            doc_type: "research",
            home: AnchorRef::context(home),
            owner,
            originator: owner,
            emitter,
            properties: &[],
            chunks: Some(vec![chunk("block zero", "")]),
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
    append(pool, resource, 1, "segment one", emitter).await;
    assert_eq!(
        ingest_state(pool, resource).await,
        "in_progress",
        "setup: an ingest actually in flight"
    );
    resource
}

/// The `kb_resources.ingest_state` target lines of one `resource_erased` event.
async fn erasure_ingest_lines(pool: &PgPool, event: Uuid) -> Vec<serde_json::Value> {
    payload_of(pool, event).await["targets"]
        .as_array()
        .expect("the resource_erased payload carries targets")
        .iter()
        .filter(|t| t["target"] == "kb_resources.ingest_state")
        .cloned()
        .collect()
}

/// (16f, then the erasure) An erasure after the scrub cancelled the ingest does not claim to have
/// ended it: the scrub's event already records the cancel.
///
/// FAILS IF: the `resource_erased` event of a scrub-cancelled resource carries a
/// `kb_resources.ingest_state` target line; or the erasure of an ingest still in flight does not
/// carry its "ended by erasure" line (the line exists, so its absence above is the condition's).
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn an_erasure_after_a_scrub_cancel_does_not_claim_the_ingest(pool: PgPool) {
    let (owner, emitter) = setup(&pool).await;

    let cancelled = in_flight_ingest(&pool, owner, emitter, "scrub-then-erase").await;
    let block = first_live_block(&pool, cancelled).await;
    scrub(&pool, cancelled, &[block]).await;
    assert_eq!(
        ingest_state(&pool, cancelled).await,
        "cancelled",
        "setup: the scrub cancelled the ingest"
    );
    let erased = erase(&pool, cancelled).await;
    assert_eq!(
        erasure_ingest_lines(&pool, erased).await,
        Vec::<serde_json::Value>::new(),
        "the erasure names no ingest it ended: the scrub ended it"
    );

    let in_flight = in_flight_ingest(&pool, owner, emitter, "erase-in-flight").await;
    let erased = erase(&pool, in_flight).await;
    assert_eq!(
        erasure_ingest_lines(&pool, erased).await,
        vec![serde_json::json!({
            "target": "kb_resources.ingest_state",
            "outcome": "ingest in_progress; ended by erasure; erased_at is authoritative",
        })],
        "an erasure of an ingest still in flight names the ingest it ended"
    );
}

/// (16g) The refusals RAISE with the texts the service parses, and change nothing; the widened
/// refusal records which act was refused and which blocks it named.
///
/// FAILS IF: a refusal does not raise its text; the foreign-block refusal changes any row or
/// appends any event; the eight-argument refusal's payload lacks `act` or `blocks`; the
/// six-argument call stops resolving or writes `act`; or `p_blocks` is accepted with another act.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn the_refusals_raise_and_the_refusal_names_the_act(pool: PgPool) {
    let (owner, emitter) = setup(&pool).await;
    let home = make_home(&pool, owner, "scrub-refuse").await;
    let other_home = make_home(&pool, owner, "scrub-refuse-other").await;
    let raised = |r: Result<serde_json::Value, sqlx::Error>| r.unwrap_err().to_string();

    // Not found.
    let missing = Uuid::now_v7();
    let msg = raised(try_scrub(&pool, missing, &[Uuid::now_v7()]).await);
    assert!(
        msg.contains(&format!(
            "block_history_scrub_execute: resource {missing} not found"
        )),
        "got {msg}"
    );

    // Erased.
    let erased = create(&pool, owner, emitter, home, "scrub-erased", SECRET).await;
    let erased_block = first_live_block(&pool, erased).await;
    sqlx::query("SELECT resource_erasure_execute($1, $2, $3, $4)")
        .bind(erased.uuid())
        .bind(owner.uuid())
        .bind(emitter)
        .bind(Uuid::now_v7())
        .execute(&pool)
        .await
        .expect("the erasure completes");
    let msg = raised(try_scrub(&pool, erased.uuid(), &[erased_block]).await);
    assert!(
        msg.contains("block_history_scrub_execute: already erased"),
        "got {msg}"
    );

    // A charter resource, made through the real cogmap-genesis path.
    let telos = {
        let mut conn = pool.acquire().await.unwrap();
        fire(
            &mut conn,
            SeedAction::CogmapGenesis {
                name: "scrub-refusal-map",
                telos_title: "telos-for-scrub-refusal",
                charter: &[],
                cogmap_id: None,
                telos_resource_id: None,
                owner,
                emitter,
            },
        )
        .await
        .unwrap()
        .cogmap_genesis()
        .unwrap()
        .1
    };
    let msg = raised(try_scrub(&pool, telos.uuid(), &[Uuid::now_v7()]).await);
    assert!(
        msg.contains(
            "block_history_scrub_execute: charter resource (map-grain erasure is filed task"
        ),
        "got {msg}"
    );

    // A block of another resource: refused, naming it, with nothing changed.
    let resource = create(&pool, owner, emitter, home, "scrub-refuse-r", SECRET).await;
    let block = first_live_block(&pool, resource).await;
    revise(&pool, resource, block, REV_TWO, emitter).await;
    let other = create(&pool, owner, emitter, other_home, "scrub-refuse-s", SECRET).await;
    let foreign = first_live_block(&pool, other).await;
    let r_before = resource_blocks_snapshot(&pool, resource).await;
    let s_before = resource_blocks_snapshot(&pool, other).await;
    let events_before = event_ids(&pool).await;
    let msg = raised(try_scrub(&pool, resource.uuid(), &[block, foreign]).await);
    assert!(
        msg.contains(&format!(
            "block_history_scrub_execute: block {foreign} is not a block of resource {}",
            resource.uuid()
        )),
        "got {msg}"
    );
    assert_eq!(resource_blocks_snapshot(&pool, resource).await, r_before);
    assert_eq!(resource_blocks_snapshot(&pool, other).await, s_before);
    assert!(
        new_event_types(&pool, &events_before).await.is_empty(),
        "a refused scrub appends nothing"
    );

    // An empty list, and a repeated block.
    let msg = raised(try_scrub(&pool, resource.uuid(), &[]).await);
    assert!(
        msg.contains("block_history_scrub_execute: p_blocks is empty"),
        "got {msg}"
    );
    let msg = raised(try_scrub(&pool, resource.uuid(), &[block, block]).await);
    assert!(
        msg.contains(&format!(
            "block_history_scrub_execute: block {block} is named more than once"
        )),
        "got {msg}"
    );
    assert_eq!(resource_blocks_snapshot(&pool, resource).await, r_before);

    // The recorded refusal, eight arguments: act and blocks ride the payload.
    let refused: Uuid = sqlx::query_scalar(
        "SELECT resource_erasure_refuse($1, $2, $3, $4, 'already_erased', NULL, \
                'block_history_scrub', $5)",
    )
    .bind(erased.uuid())
    .bind(owner.uuid())
    .bind(emitter)
    .bind(Uuid::now_v7())
    .bind(vec![erased_block, block])
    .fetch_one(&pool)
    .await
    .expect("the eight-argument refusal appends");
    let payload: ResourceErasureRefused =
        serde_json::from_value(payload_of(&pool, refused).await).expect("a ResourceErasureRefused");
    assert_eq!(payload.act, Some(ErasureAct::BlockHistoryScrub));
    assert_eq!(payload.blocks, vec![erased_block, block]);
    assert_eq!(payload.reason, RecordedRefusalReason::AlreadyErased);
    assert_eq!(payload.subject_id, erased.uuid());

    // The six-argument call a deployed binary makes still resolves, and writes no act.
    let six: Uuid =
        sqlx::query_scalar("SELECT resource_erasure_refuse($1, $2, $3, $4, 'already_erased', $5)")
            .bind(erased.uuid())
            .bind(owner.uuid())
            .bind(emitter)
            .bind(Uuid::now_v7())
            .bind(Option::<String>::None)
            .fetch_one(&pool)
            .await
            .expect("the six-argument refusal still resolves");
    let raw = payload_of(&pool, six).await;
    assert!(
        raw.get("act").is_none() && raw.get("blocks").is_none(),
        "got {raw}"
    );
    let payload: ResourceErasureRefused =
        serde_json::from_value(raw).expect("a ResourceErasureRefused");
    assert_eq!(payload.act, None, "absent act reads as the erasure");
    assert!(payload.blocks.is_empty());

    // Blocks only ride a scrub refusal; an unknown act is refused.
    let bad = sqlx::query_scalar::<_, Uuid>(
        "SELECT resource_erasure_refuse($1, $2, $3, $4, 'already_erased', NULL, 'erasure', $5)",
    )
    .bind(erased.uuid())
    .bind(owner.uuid())
    .bind(emitter)
    .bind(Uuid::now_v7())
    .bind(vec![block])
    .fetch_one(&pool)
    .await;
    assert!(
        bad.unwrap_err()
            .to_string()
            .contains("p_blocks is carried only by a block_history_scrub refusal"),
        "blocks with the erasure act are refused"
    );
    let bad = sqlx::query_scalar::<_, Uuid>(
        "SELECT resource_erasure_refuse($1, $2, $3, $4, 'already_erased', NULL, 'reblock')",
    )
    .bind(erased.uuid())
    .bind(owner.uuid())
    .bind(emitter)
    .bind(Uuid::now_v7())
    .fetch_one(&pool)
    .await;
    assert!(
        bad.unwrap_err()
            .to_string()
            .contains("reblock is not a refusable act"),
        "an unknown act is refused"
    );
}

/// (16h) A tombstone is scrubbable (in D11 "live" means not erased) and stays a tombstone.
///
/// FAILS IF: the scrub refuses a soft-deleted resource, leaves its history, or sets `erased_at`.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn a_tombstone_is_scrubbed_and_stays_a_tombstone(pool: PgPool) {
    let (owner, emitter) = setup(&pool).await;
    let home = make_home(&pool, owner, "scrub-tombstone").await;
    let resource = create(&pool, owner, emitter, home, "scrub-tombstone", SECRET).await;
    let block = first_live_block(&pool, resource).await;
    revise(&pool, resource, block, REV_THREE, emitter).await;
    // Tombstone through the REAL delete path.
    let mut tx = pool.begin().await.unwrap();
    fire(&mut tx, SeedAction::ResourceDelete { resource, emitter })
        .await
        .unwrap();
    tx.commit().await.unwrap();
    assert_eq!(
        history_left(&pool, block).await,
        (1, 1),
        "setup: the tombstone has history"
    );

    scrub(&pool, resource, &[block]).await;

    assert_eq!(
        history_left(&pool, block).await,
        (0, 0),
        "the history is emptied"
    );
    let (is_active, erased_at): (bool, Option<chrono::DateTime<chrono::Utc>>) =
        sqlx::query_as("SELECT is_active, erased_at FROM kb_resources WHERE id = $1")
            .bind(resource.uuid())
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(!is_active, "still soft-deleted");
    assert!(erased_at.is_none(), "a scrubbed tombstone is not a husk");
}

/// (D10) Survey equals act: the survey taken before the act reports exactly the counts the act's
/// `targets` name, and those counts are the rows the act empties.
///
/// FAILS IF: the plan's counts disagree with the rows the redaction body changes (the plan's
/// predicates drifted from the body's), or the act's `targets` disagree with the survey.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn the_survey_reports_the_counts_the_act_names(pool: PgPool) {
    let (owner, emitter) = setup(&pool).await;
    let h = seed_history(&pool, owner, emitter, "scrub-survey").await;
    let named = [h.block, h.unnamed];

    let plan = survey(&pool, h.resource, &named).await;
    assert_eq!(plan["ingest_state"], serde_json::json!("complete"));
    assert_eq!(plan["cancels_ingest"], serde_json::json!(false));
    let mut expected = Vec::new();
    let mut emptied_before = Vec::new();
    for (i, b) in named.iter().enumerate() {
        let entry = &plan["blocks"][i];
        assert_eq!(entry["block"], serde_json::json!(b.to_string()));
        let folded = entry["folded"].as_bool().expect("folded is a bool");
        let revisions = entry["revisions_to_empty"].as_i64().expect("a count");
        let chunks = entry["chunks_to_empty"].as_i64().expect("a count");
        expected.push(block_line(*b, folded, revisions, chunks));
        emptied_before.push((revisions, chunks));
    }
    assert_eq!(
        emptied_before,
        vec![(2, 2), (0, 0)],
        "the survey counts the named block's history and nothing on the unnamed one"
    );
    let measured_before = [
        history_left(&pool, h.block).await,
        history_left(&pool, h.unnamed).await,
    ];

    let result = scrub(&pool, h.resource, &named).await;

    assert_eq!(
        result["targets"],
        serde_json::Value::Array(expected),
        "the act's targets name the survey's counts"
    );
    let measured_after = [
        history_left(&pool, h.block).await,
        history_left(&pool, h.unnamed).await,
    ];
    assert_eq!(
        measured_before.to_vec(),
        emptied_before,
        "the survey's counts are the rows carrying history"
    );
    assert_eq!(
        measured_after,
        [(0, 0), (0, 0)],
        "and the act empties exactly those"
    );
}

/// Snapshot, reset, replay, and diff every projection table.
async fn assert_replay_byte_identical(pool: &PgPool, after_what: &str) {
    let before = replay::dump_projections(pool).await.unwrap();
    let snap = replay::snapshot(pool).await.unwrap();
    common::reset_schema(pool).await;
    replay::replay(pool, &snap).await.unwrap();
    let after = replay::dump_projections(pool).await.unwrap();
    for ((ta, a), (tb, b)) in before.iter().zip(after.iter()) {
        assert_eq!(ta, tb);
        assert_eq!(
            a, b,
            "projection table {ta} diverged under replay {after_what}"
        );
    }
}

/// The erasure act, as the system operator under a fresh request reference; returns the
/// `resource_erased` event id.
async fn erase(pool: &PgPool, resource: ResourceId) -> Uuid {
    let (_, operator_entity) = system_actor(pool).await;
    let raw: String =
        sqlx::query_scalar("SELECT (resource_erasure_execute($1,$2,$3,$4)->>'event_id')::text")
            .bind(resource.uuid())
            .bind(operator_entity)
            .bind(operator_entity)
            .bind(Uuid::now_v7())
            .fetch_one(pool)
            .await
            .expect("the erasure completes");
    Uuid::parse_str(&raw).expect("the event id parses")
}

/// (replay) The scrub replays at its position through the one apply function: a live block, a
/// folded block and an in-flight ingest, scrubbed, then replayed from the ledger.
///
/// FAILS IF: any projection table differs after replay (the arm is a no-op, or re-implements a
/// step differently); or `ingest_state` does not read `cancelled` after the replay
/// (`cancelled_ingest` was not honoured).
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn replay_of_a_block_history_scrub_is_byte_identical(pool: PgPool) {
    let (owner, emitter) = setup(&pool).await;

    // A live block with history, and a sibling folded by a `replaces_body` mutate of it.
    let h = seed_history(&pool, owner, emitter, "scrub-replay").await;
    const WHOLE: &str = "the whole body, replaced";
    let replacement = prepare_block_from_chunks(0, None, vec![chunk(WHOLE, "")]);
    let mut tx = pool.begin().await.unwrap();
    fire(
        &mut tx,
        SeedAction::BlockMutate {
            block: BlockId::from(h.block),
            chunks: &replacement.chunks,
            raw: Some(WHOLE),
            incorporated: &[],
            replaces_body: true,
            emitter,
        },
    )
    .await
    .unwrap();
    tx.commit().await.unwrap();
    let folded: bool = sqlx::query_scalar("SELECT is_folded FROM kb_content_blocks WHERE id = $1")
        .bind(h.unnamed)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert!(folded, "setup: the replaces_body mutate folded the sibling");
    scrub(&pool, h.resource, &[h.block, h.unnamed]).await;

    // An in-flight ingest.
    let home = make_home(&pool, owner, "scrub-replay-ingest").await;
    let in_flight = writes::create_resource_with_mode(
        &pool,
        CreateParams {
            idempotency_key: None,
            title: "mid-ingest",
            origin_uri: "test://scrub-replay-mid-ingest",
            body: "block zero",
            doc_type: "research",
            home: AnchorRef::context(home),
            owner,
            originator: owner,
            emitter,
            properties: &[],
            chunks: Some(vec![chunk("block zero", "")]),
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
    append(&pool, in_flight, 1, "segment one", emitter).await;
    assert_eq!(ingest_state(&pool, in_flight).await, "in_progress");
    let ingest_block = first_live_block(&pool, in_flight).await;
    scrub(&pool, in_flight, &[ingest_block]).await;
    assert_eq!(ingest_state(&pool, in_flight).await, "cancelled");

    assert_replay_byte_identical(
        &pool,
        "of a scrub over a live block, a folded block and an ingest",
    )
    .await;
    assert_eq!(
        ingest_state(&pool, in_flight).await,
        "cancelled",
        "the cancelled ingest survives replay"
    );
}

/// (replay, review focus 5) Scrub, scrub the same blocks again, then erase: replay is
/// byte-identical after each of the three. The second scrub is a completed act: its `targets`
/// name zero revisions and zero chunks emptied, and the ledger holds two scrub events.
///
/// FAILS IF: replay diverges after any step; the second scrub raises or names a nonzero count;
/// or the ledger does not hold exactly two `block_history_scrubbed` events.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn a_scrub_scrubbed_again_and_then_erased_replays_byte_identical_at_each_step(pool: PgPool) {
    let (owner, emitter) = setup(&pool).await;
    let h = seed_history(&pool, owner, emitter, "scrub-twice-erase").await;
    let blocks = [h.block, h.unnamed];

    scrub(&pool, h.resource, &blocks).await;
    assert_replay_byte_identical(&pool, "of the first scrub").await;

    let second = scrub(&pool, h.resource, &blocks).await;
    assert_eq!(
        second["targets"],
        serde_json::json!([
            block_line(h.block, false, 0, 0),
            block_line(h.unnamed, false, 0, 0),
        ]),
        "the second scrub names zero revisions and zero chunks emptied"
    );
    let scrubs: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM kb_events e JOIN kb_event_types t ON t.id = e.event_type_id \
          WHERE t.name = 'block_history_scrubbed'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(scrubs, 2, "two block_history_scrubbed events");
    assert_replay_byte_identical(&pool, "of the second scrub").await;

    erase(&pool, h.resource).await;
    assert_replay_byte_identical(&pool, "of an erasure after two scrubs").await;
}

/// The replay arm refuses a `block_history_scrubbed` event whose payload names another table,
/// planted by a raw ledger append (the act would never write it).
///
/// FAILS IF: replay succeeds, or the refusal does not name the offending table.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn replay_refuses_a_block_history_scrubbed_naming_another_table(pool: PgPool) {
    let (_, emitter) = setup(&pool).await;
    sqlx::query(
        "SELECT _event_append('block_history_scrubbed', $1, NULL, NULL, \
                jsonb_build_object('subject_table', 'kb_resources', \
                                   'subject_ids', jsonb_build_array($2::uuid)), \
                p_correlation => $3)",
    )
    .bind(emitter)
    .bind(Uuid::now_v7())
    .bind(Uuid::now_v7())
    .execute(&pool)
    .await
    .expect("the raw append lands");

    let snap = replay::snapshot(&pool).await.unwrap();
    common::reset_schema(&pool).await;
    let err = replay::replay(&pool, &snap)
        .await
        .expect_err("replay must refuse a block_history_scrubbed naming another table");
    let chain = format!("{err:#}");
    assert!(
        chain.contains("names subject_table \"kb_resources\", not kb_content_blocks"),
        "{chain}"
    );
}

/// The race 16i and 16j share. Holds `block`'s current chunks, starts `writer`, and waits until
/// the writer's call to `entry_fn` is parked on them: past its mint, at its chunk supersede. Then
/// it runs the scrub over `block` and gives it 2 seconds. Returns whether the scrub was still
/// waiting when they ran out, and the scrub's event id, once the holder has released the writer
/// and both have committed.
async fn race_the_scrub(
    pool: &PgPool,
    resource: ResourceId,
    block: Uuid,
    entry_fn: &str,
    writer: impl std::future::Future<Output = ()> + Send + 'static,
) -> (bool, Uuid) {
    let mut holder = pool.begin().await.unwrap();
    let held: Vec<Uuid> = sqlx::query_scalar(
        "SELECT id FROM kb_chunks WHERE block_id = $1 AND is_current FOR UPDATE",
    )
    .bind(block)
    .fetch_all(&mut *holder)
    .await
    .unwrap();
    assert!(
        !held.is_empty(),
        "setup: the block has current chunks to hold"
    );

    let writing = tokio::spawn(writer);
    let mut parked = false;
    for _ in 0..100 {
        parked = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM pg_stat_activity
                             WHERE datname = current_database() AND wait_event_type = 'Lock'
                               AND query LIKE '%' || $1 || '%')",
        )
        .bind(entry_fn)
        .fetch_one(pool)
        .await
        .unwrap();
        if parked {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    assert!(parked, "setup: {entry_fn} is parked on the held chunks");

    let pool_for_scrub = pool.clone();
    let mut scrubbing =
        tokio::spawn(async move { try_scrub(&pool_for_scrub, resource.uuid(), &[block]).await });
    let raced = tokio::time::timeout(std::time::Duration::from_secs(2), &mut scrubbing).await;
    let scrub_waited = raced.is_err();

    holder.rollback().await.unwrap();
    writing.await.expect("the writer completes");
    let scrubbed = match raced {
        Ok(joined) => joined,
        Err(_) => scrubbing.await,
    }
    .unwrap()
    .expect("the scrub completes");
    (scrub_waited, event_of(&scrubbed))
}

/// (16i) A per-block mutate racing the scrub serializes on R's row, so live and replay agree.
///
/// A third transaction holds the block's current chunks, which parks the mutate at its chunk
/// supersede: its event is minted and it holds R FOR KEY SHARE, taken before the mint. The scrub
/// then runs and waits on R until the mutate commits, so it keeps the mutate's revision as current
/// and empties REV_THREE. The 2-second timeout that expires is the serialization signal, as in the
/// act's witness 20.
///
/// Without that lock (`block_mutate` before 20261004000020), nothing held R at the park, so the
/// scrub completed past the mutate: it kept REV_THREE as current and committed an event that sorts
/// after the mutate's. The mutate then made REV_THREE history with its prose intact, while replay,
/// walking the mutate first, emptied it.
///
/// FAILS IF: the scrub completes while the mutate is parked; the projection diverges under
/// replay; or REV_THREE keeps its bytes.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn a_mutate_racing_the_scrub_serializes_on_the_resource(pool: PgPool) {
    const REV_FOUR: &str = "revision four: written while the scrub ran";
    let (owner, emitter) = setup(&pool).await;
    // One block with three revisions. No second block: a per-block revise on a multi-block
    // resource re-partitions the whole body, and this witness needs the plain mutate.
    let home = make_home(&pool, owner, "scrub-race").await;
    let resource = create(&pool, owner, emitter, home, "scrub-race", SECRET).await;
    let block = first_live_block(&pool, resource).await;
    for prose in [REV_TWO, REV_THREE] {
        revise(&pool, resource, block, prose, emitter).await;
    }

    let pool_for_mutate = pool.clone();
    let (scrub_waited, scrubbed) =
        race_the_scrub(&pool, resource, block, "block_mutate", async move {
            revise(&pool_for_mutate, resource, block, REV_FOUR, emitter).await;
        })
        .await;

    assert_replay_byte_identical(&pool, "of a mutate that raced the scrub").await;
    assert!(
        scrub_waited,
        "the scrub waited for the parked mutate instead of completing past it"
    );
    let mutated: Uuid = sqlx::query_scalar(
        "SELECT e.id FROM kb_events e JOIN kb_event_types t ON t.id = e.event_type_id \
          WHERE t.name = 'block_mutated' ORDER BY e.id DESC LIMIT 1",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(
        mutated < scrubbed,
        "the mutate, minted first, sorts before the scrub it held up"
    );
    assert_eq!(
        present_of(&pool, block).await.0.as_deref(),
        Some(REV_FOUR),
        "the mutate's revision is the present"
    );
    assert_eq!(
        revision_bytes(&pool, block).await,
        vec![
            String::new(),
            String::new(),
            String::new(),
            REV_FOUR.to_owned()
        ],
        "the scrub saw the mutate's commit: every revision before REV_FOUR is empty, REV_THREE \
         included"
    );
}

/// (16j) A whole-body replace racing the scrub serializes on R's row, so live and replay agree.
///
/// A whole-body write over a single-block resource goes through `resource_reblock`, which folds
/// the incumbent block B. Holding B's current chunks parks the reblock at its chunk supersede: its
/// event is minted and it holds R FOR KEY SHARE, taken before the mint. The scrub over B waits on
/// R until the reblock commits, finds B folded, and empties all of it.
///
/// Without that lock (`resource_reblock` before 20261004000020), the scrub completed past the
/// reblock and kept B's current revision. The reblock then committed B folded with that prose,
/// while replay, walking the reblock first, emptied it.
///
/// FAILS IF: the scrub completes while the reblock is parked; the projection diverges under
/// replay; or B is not folded with every revision empty.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn a_reblock_racing_the_scrub_serializes_on_the_resource(pool: PgPool) {
    const REPLACED: &str = "a whole new body, written while the scrub ran";
    let (owner, emitter) = setup(&pool).await;
    let home = make_home(&pool, owner, "scrub-reblock-race").await;
    let resource = create(&pool, owner, emitter, home, "scrub-reblock-race", SECRET).await;
    let block = first_live_block(&pool, resource).await;
    revise(&pool, resource, block, REV_TWO, emitter).await;

    let pool_for_reblock = pool.clone();
    let (scrub_waited, scrubbed) =
        race_the_scrub(&pool, resource, block, "resource_reblock", async move {
            writes::update_resource(
                &pool_for_reblock,
                UpdateParams {
                    resource,
                    body: Some(REPLACED),
                    title: None,
                    origin_uri: None,
                    properties: &[],
                    unset_keys: &[],
                    chunks: Some(vec![chunk(REPLACED, "replaced")]),
                    sources: vec![],
                    content_block: None,
                    rehome_to: None,
                    emitter,
                },
            )
            .await
            .expect("a whole-body replace");
        })
        .await;

    assert_replay_byte_identical(&pool, "of a reblock that raced the scrub").await;
    assert!(
        scrub_waited,
        "the scrub waited for the parked reblock instead of completing past it"
    );
    let reblocked: Uuid = sqlx::query_scalar(
        "SELECT e.id FROM kb_events e JOIN kb_event_types t ON t.id = e.event_type_id \
          WHERE t.name = 'resource_reblocked' ORDER BY e.id DESC LIMIT 1",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(
        reblocked < scrubbed,
        "the reblock, minted first, sorts before the scrub it held up"
    );
    let folded: bool = sqlx::query_scalar("SELECT is_folded FROM kb_content_blocks WHERE id = $1")
        .bind(block)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert!(folded, "the reblock folded B");
    assert_eq!(
        revision_bytes(&pool, block).await,
        vec![String::new(), String::new()],
        "the scrub saw B folded and emptied every revision, the current one included"
    );
}
