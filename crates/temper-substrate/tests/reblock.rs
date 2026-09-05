#![cfg(feature = "artifact-tests")]
//! Witnesses for the re-block substrate (task 2026-09-04, goal Anchor addressability phase 1).
//! One behavior per test, each against the real write paths — resources are seeded through
//! `writes::create_resource` (verbatim bytes present, the op's own precondition), never
//! hand-inserted. The identity/attribution fixtures build multi-block resources through the
//! segmented-ingest trio (create-segmented → append → finalize), which is the one honest way a
//! resource gets blocks whose boundaries are NOT its section boundaries.
//!
//! ONNX-dependent. Isolated ephemeral DB via `temper_substrate::MIGRATOR`.

mod common;

use temper_substrate::content::{body_hash_from_block_chunk_hashes, prepare_block_with_prefix};
use temper_substrate::ids::{BlockId, EntityId, ProfileId, ResourceId};
use temper_substrate::payloads::{AnchorRef, Incorporation, ProvenanceSource};
use temper_substrate::writes::{
    self, AppendParams, CreateMode, CreateParams, FinalizeParams, ReblockOutcome, ReblockParams,
};
use temper_substrate::{replay, scenario::bootseed};
use uuid::Uuid;

const SECTION_A: &str = "# Alpha\n\nAlpha body paragraph.\n";
const SECTION_B: &str = "## Beta\n\nBeta body paragraph.\n";
const SECTION_C: &str = "### Gamma\n\nGamma body paragraph.\n";
const BODY_A_B: &str = "# Alpha\n\nAlpha body paragraph.\n## Beta\n\nBeta body paragraph.\n";

// ── fixture helpers (duplicated per file, per this suite's convention) ──────────────────────

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

async fn make_home(pool: &sqlx::PgPool, owner: ProfileId, slug: &str) -> AnchorRef {
    let ctx = common::insert_context(pool, "kb_profiles", owner.uuid(), slug, slug)
        .await
        .unwrap();
    AnchorRef::context(temper_substrate::ids::ContextId::from(ctx))
}

async fn create_body_resource(
    pool: &sqlx::PgPool,
    owner: ProfileId,
    emitter: EntityId,
    home: &AnchorRef,
    title: &str,
    body: &str,
    sources: Vec<Incorporation>,
) -> ResourceId {
    writes::create_resource(
        pool,
        CreateParams {
            title,
            origin_uri: &format!("temper://reblock/{title}"),
            body,
            doc_type: "concept",
            home: *home,
            owner,
            originator: owner,
            emitter,
            properties: &[],
            chunks: None,
            sources,
            idempotency_key: None,
        },
    )
    .await
    .unwrap()
}

/// A two-block resource built through the segmented-ingest trio (create-segmented → append →
/// finalize), so every block carries real stored bytes and real chunk rows. The caller chooses
/// the block boundaries via `first`/`rest`: aligned with the body's sections when `rest` is one
/// section, misaligned (a block spanning two sections) when it is two.
async fn create_segmented_two_block(
    pool: &sqlx::PgPool,
    owner: ProfileId,
    emitter: EntityId,
    home: &AnchorRef,
    first: &str,
    rest: &str,
    block0_sources: Vec<Incorporation>,
) -> ResourceId {
    let resource = writes::create_resource_with_mode(
        pool,
        CreateParams {
            title: "segmented",
            origin_uri: "temper://reblock/segmented",
            body: first,
            doc_type: "concept",
            home: *home,
            owner,
            originator: owner,
            emitter,
            properties: &[],
            chunks: None,
            sources: block0_sources,
            idempotency_key: None,
        },
        temper_substrate::events::EventContext::default(),
        CreateMode {
            defer: false,
            segmented: true,
        },
    )
    .await
    .unwrap();
    // The trailing heading stack of `first` is segment 2's initial breadcrumb — the streaming
    // segmenter's own rule (stream.rs), so segment 2's chunks carry document-contextual
    // header_paths ("Alpha > Beta"), not segment-relative ones ("Beta").
    let breadcrumb: Vec<String> = temper_ingest::chunk::chunk_markdown(first)
        .last()
        .filter(|c| !c.header_path.is_empty())
        .map(|c| c.header_path.split(" > ").map(str::to_owned).collect())
        .unwrap_or_default();
    let mut block1 = prepare_block_with_prefix(1, None, rest, &breadcrumb).unwrap();
    // The append's verbatim bytes: without raw_text the block stores no kb_block_content row
    // and is honestly `derived` — the op would (correctly) refuse it.
    block1.raw_text = Some(rest.to_owned());
    writes::append_block(
        pool,
        AppendParams {
            resource,
            block: &block1,
            sources: vec![],
            emitter,
        },
    )
    .await
    .unwrap();
    let h0: Vec<String> = temper_ingest::chunk::chunk_markdown(first)
        .iter()
        .map(|c| c.content_hash.clone())
        .collect();
    let h1: Vec<String> = temper_ingest::chunk::chunk_markdown(rest)
        .iter()
        .map(|c| c.content_hash.clone())
        .collect();
    writes::finalize_ingest(
        pool,
        FinalizeParams {
            resource,
            expected_blocks: 2,
            expected_body_hash: body_hash_from_block_chunk_hashes(&[h0, h1]),
            expected_content_hash: None,
            emitter,
        },
    )
    .await
    .unwrap();
    resource
}

/// (id, seq, is_folded, genesis_event_id, last_event_id, current_revision_id), seq order.
async fn blocks_of(
    pool: &sqlx::PgPool,
    resource: ResourceId,
) -> Vec<(Uuid, i32, bool, Uuid, Uuid, Uuid)> {
    sqlx::query_as(
        "SELECT id, seq, is_folded, genesis_event_id, last_event_id, current_revision_id \
           FROM kb_content_blocks WHERE resource_id=$1 ORDER BY seq, id",
    )
    .bind(resource.uuid())
    .fetch_all(pool)
    .await
    .unwrap()
}

/// (content_hash, embedding::text, header_path, heading_depth, is_current) for the chunks hanging
/// off the resource's non-folded blocks — the retrieval-relevant multiset.
async fn live_chunk_rows(
    pool: &sqlx::PgPool,
    resource: ResourceId,
) -> Vec<(String, Option<String>, Option<String>, Option<i16>, bool)> {
    type ChunkRow = (String, Option<String>, Option<String>, Option<i16>, bool);
    let mut rows: Vec<ChunkRow> = sqlx::query_as(
        "SELECT c.content_hash, c.embedding::text, c.header_path, c.heading_depth, c.is_current \
           FROM kb_chunks c JOIN kb_content_blocks b ON b.id=c.block_id \
          WHERE b.resource_id=$1 AND NOT b.is_folded ORDER BY c.content_hash",
    )
    .bind(resource.uuid())
    .fetch_all(pool)
    .await
    .unwrap();
    rows.sort();
    rows
}

async fn body_state(pool: &sqlx::PgPool, resource: ResourceId) -> (Option<String>, String) {
    // (body_hash, verbatim concat of live blocks in seq order)
    let row: (Option<String>, Option<String>) = sqlx::query_as(
        "SELECT r.body_hash, string_agg(bc.content, '' ORDER BY b.seq) \
           FROM kb_resources r \
           JOIN kb_content_blocks b ON b.resource_id=r.id AND NOT b.is_folded \
           LEFT JOIN kb_block_content bc ON bc.block_revision_id=b.current_revision_id \
          WHERE r.id=$1 GROUP BY r.body_hash",
    )
    .bind(resource.uuid())
    .fetch_one(pool)
    .await
    .unwrap();
    (row.0, row.1.unwrap_or_default())
}

async fn event_count(pool: &sqlx::PgPool) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM kb_events")
        .fetch_one(pool)
        .await
        .unwrap()
}

async fn reblocked_event_count(pool: &sqlx::PgPool) -> i64 {
    sqlx::query_scalar(
        "SELECT count(*) FROM kb_events e JOIN kb_event_types t ON t.id=e.event_type_id \
          WHERE t.name='resource_reblocked'",
    )
    .fetch_one(pool)
    .await
    .unwrap()
}

fn emitter_of(actor: &(ProfileId, EntityId)) -> EntityId {
    actor.1
}

// ── the witnesses ────────────────────────────────────────────────────────────────────────────

/// 1. The projector applies the manifest transactionally: fold → insert → reparent in one txn.
/// The same-seq coexistence (a folded seq-0 block and a LIVE seq-0 block) is exactly the state
/// the partial unique index forbids among live rows and allows once one side is folded — its
/// existence after the operation witnesses the fold-before-insert ordering by construction.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn the_projector_applies_the_manifest_transactionally(pool: sqlx::PgPool) {
    bootseed::seed_system(&pool).await.unwrap();
    let actor = system_actor(&pool).await;
    let home = make_home(&pool, actor.0, "reblock-txn").await;
    let resource = create_body_resource(
        &pool,
        actor.0,
        emitter_of(&actor),
        &home,
        "txn",
        BODY_A_B,
        vec![],
    )
    .await;
    let before = blocks_of(&pool, resource).await;
    assert_eq!(
        before.len(),
        1,
        "fixture: one body block holding both sections"
    );

    let outcome = writes::reblock_resource(
        &pool,
        ReblockParams {
            resource,
            emitter: emitter_of(&actor),
        },
    )
    .await
    .unwrap();
    assert!(matches!(outcome, ReblockOutcome::Reblocked { .. }));

    let after = blocks_of(&pool, resource).await;
    assert_eq!(after.len(), 3, "1 folded + 2 live");
    let folded: Vec<_> = after
        .iter()
        .filter(|(_, _, folded, _, _, _)| *folded)
        .collect();
    let live: Vec<_> = after
        .iter()
        .filter(|(_, _, folded, _, _, _)| !*folded)
        .collect();
    assert_eq!(folded.len(), 1);
    assert_eq!(live.len(), 2);
    assert_eq!(folded[0].0, before[0].0, "the incumbent is the folded one");
    assert_eq!(
        folded[0].1, live[0].1,
        "the folded seq-0 block and the live seq-0 block COEXIST — the fold→insert ordering, \
         staged in one transaction, is what makes this state reachable at all"
    );
    assert_eq!(
        live.iter()
            .map(|(_, seq, _, _, _, _)| *seq)
            .collect::<Vec<_>>(),
        vec![0, 1],
        "live blocks renumber from 0"
    );
    for (_, _, _, genesis, last, _) in &live {
        assert_eq!(
            genesis, last,
            "a created block's genesis IS this re-block event"
        );
        let ev_type: String = sqlx::query_scalar(
            "SELECT t.name FROM kb_events e JOIN kb_event_types t ON t.id=e.event_type_id \
              WHERE e.id=$1",
        )
        .bind(last)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(ev_type, "resource_reblocked");
    }
    let chunk_total: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM kb_chunks c JOIN kb_content_blocks b ON b.id=c.block_id \
          WHERE b.resource_id=$1",
    )
    .bind(resource.uuid())
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(chunk_total, 2, "chunks are reparented, never duplicated");
    let renumbered: Vec<(i32, i32)> = sqlx::query_as(
        "SELECT b.seq, c.chunk_index FROM kb_chunks c \
           JOIN kb_content_blocks b ON b.id=c.block_id \
          WHERE b.resource_id=$1 AND NOT b.is_folded ORDER BY b.seq, c.chunk_index",
    )
    .bind(resource.uuid())
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(
        renumbered,
        vec![(0, 0), (1, 0)],
        "each created block's chunks renumber from 0"
    );
}

/// 2. Byte-exactness: the VERBATIM BODY (concat of live blocks in seq order) composes
/// identically before/after, and the chunk multiset (hash, embedding bytes, header metadata,
/// currency) is unchanged — asserted by hash and count, never by absence of error.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn reblocking_is_byte_exact_over_body_and_chunk_multiset(pool: sqlx::PgPool) {
    bootseed::seed_system(&pool).await.unwrap();
    let actor = system_actor(&pool).await;
    let home = make_home(&pool, actor.0, "reblock-bytes").await;
    let resource = create_body_resource(
        &pool,
        actor.0,
        emitter_of(&actor),
        &home,
        "bytes",
        BODY_A_B,
        vec![],
    )
    .await;

    let (_, concat_before) = body_state(&pool, resource).await;
    let chunks_before = live_chunk_rows(&pool, resource).await;
    assert!(
        !concat_before.is_empty(),
        "fixture sanity: the body is stored"
    );

    writes::reblock_resource(
        &pool,
        ReblockParams {
            resource,
            emitter: emitter_of(&actor),
        },
    )
    .await
    .unwrap();

    let (body_hash, concat_after) = body_state(&pool, resource).await;
    let chunks_after = live_chunk_rows(&pool, resource).await;
    assert_eq!(concat_before, concat_after, "verbatim concat identical");
    assert_eq!(
        chunks_before, chunks_after,
        "chunk multiset (hash + embedding bytes + header metadata + currency) identical"
    );
    // The derived-state check: body_hash == sha256(concat of the live blocks' block_body_hash
    // in seq order). The body_hash column LEGITIMATELY changes when the partition changes — it
    // is a block-grain merkle, not a content hash — so what is asserted is that it equals the
    // fresh recompute over the NEW partition (the recompute tail ran, in the pinned order).
    let merkles: Vec<String> = sqlx::query_scalar(
        "SELECT r.block_body_hash FROM kb_block_revisions r \
           JOIN kb_content_blocks b ON b.current_revision_id=r.id \
          WHERE b.resource_id=$1 AND NOT b.is_folded ORDER BY b.seq",
    )
    .bind(resource.uuid())
    .fetch_all(&pool)
    .await
    .unwrap();
    let expected_hash = temper_ingest::merkle::resource_body_hash(&merkles);
    assert_eq!(body_hash.unwrap(), expected_hash);
}

/// 2b. Byte-exactness under a whitespace-straddling kept match: chunk content_hash is over
/// TRIMMED text, so a section's derived merkle can match an incumbent whose stored BYTES
/// differ from the section's slice (a block boundary that straddles edge whitespace). Keeping
/// that row would re-compose the body from the incumbent's old bytes and silently DROP the
/// separator the folded neighbor carried — so a kept match additionally requires the stored
/// bytes to equal the slice; otherwise the section creates and the incumbent folds.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn a_whitespace_straddling_kept_match_creates_rather_than_dropping_bytes(pool: sqlx::PgPool) {
    bootseed::seed_system(&pool).await.unwrap();
    let actor = system_actor(&pool).await;
    let home = make_home(&pool, actor.0, "reblock-wskeep").await;
    // Block 0's bytes end early; the leading "\n" of block 1 is the separator between them.
    // The composed body carries "Alpha.\n\n" — which belongs to NO incumbent's bytes whole.
    let first = "Alpha.\n";
    let rest = "\n# H\nB\n\n# I\nC\n";
    let resource = create_segmented_two_block(
        &pool,
        actor.0,
        emitter_of(&actor),
        &home,
        first,
        rest,
        vec![],
    )
    .await;
    let (body_hash_before, body_before) = body_state(&pool, resource).await;

    writes::reblock_resource(
        &pool,
        ReblockParams {
            resource,
            emitter: emitter_of(&actor),
        },
    )
    .await
    .unwrap();

    let (body_hash_after, body_after) = body_state(&pool, resource).await;
    assert_eq!(
        body_before, body_after,
        "the composed body is byte-identical — no separator dropped"
    );
    // And the derived state recomputes over whatever partition resulted.
    let merkles: Vec<String> = sqlx::query_scalar(
        "SELECT r.block_body_hash FROM kb_block_revisions r \
           JOIN kb_content_blocks b ON b.current_revision_id=r.id \
          WHERE b.resource_id=$1 AND NOT b.is_folded ORDER BY b.seq",
    )
    .bind(resource.uuid())
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(
        body_hash_after.unwrap(),
        temper_ingest::merkle::resource_body_hash(&merkles)
    );
    let _ = body_hash_before;
}

/// 3. Identity preservation: a section whose run hashes identically to an incumbent block KEEPS
/// that block ROW (id equality); the folded block's id differs.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn a_matching_section_keeps_its_block_row(pool: sqlx::PgPool) {
    bootseed::seed_system(&pool).await.unwrap();
    let actor = system_actor(&pool).await;
    let home = make_home(&pool, actor.0, "reblock-identity").await;
    let resource = create_segmented_two_block(
        &pool,
        actor.0,
        emitter_of(&actor),
        &home,
        SECTION_A,
        &format!("{SECTION_B}{SECTION_C}"),
        vec![],
    )
    .await;
    let before = blocks_of(&pool, resource).await;
    let block0 = BlockId::from(before[0].0);
    let block1 = BlockId::from(before[1].0);

    writes::reblock_resource(
        &pool,
        ReblockParams {
            resource,
            emitter: emitter_of(&actor),
        },
    )
    .await
    .unwrap();

    // Read the manifest off the ledger — the mapping is the payload's own content.
    let (payload,): (serde_json::Value,) = sqlx::query_as(
        "SELECT e.payload FROM kb_events e JOIN kb_event_types t ON t.id=e.event_type_id \
          WHERE t.name='resource_reblocked' ORDER BY e.id DESC LIMIT 1",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    let kept: Vec<&serde_json::Value> = payload["kept"]
        .as_array()
        .expect("kept array")
        .iter()
        .collect();
    assert_eq!(kept.len(), 1, "section Alpha keeps its incumbent");
    assert_eq!(
        kept[0]["block_id"].as_str().unwrap(),
        block0.uuid().to_string(),
        "the KEPT row is the SAME row — id equality, not content equality"
    );
    let folded: Vec<&serde_json::Value> = payload["folded"]
        .as_array()
        .expect("folded array")
        .iter()
        .collect();
    assert_eq!(folded.len(), 1);
    assert_eq!(
        folded[0].as_str().unwrap(),
        block1.uuid().to_string(),
        "the misaligned incumbent is the folded one"
    );
    assert_eq!(payload["created"].as_array().unwrap().len(), 2);
}

/// 4. No-op dedup: an identical partition produces no event — the ledger is indistinguishable
/// from the operation never having run — and a re-block of an already-re-blocked resource fires
/// nothing either.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn an_identical_partition_fires_nothing(pool: sqlx::PgPool) {
    bootseed::seed_system(&pool).await.unwrap();
    let actor = system_actor(&pool).await;
    let home = make_home(&pool, actor.0, "reblock-noop").await;
    let resource = create_body_resource(
        &pool,
        actor.0,
        emitter_of(&actor),
        &home,
        "noop",
        BODY_A_B,
        vec![],
    )
    .await;

    // First: the resource is ONE block and its body has TWO sections — not a no-op.
    let first = writes::reblock_resource(
        &pool,
        ReblockParams {
            resource,
            emitter: emitter_of(&actor),
        },
    )
    .await
    .unwrap();
    assert!(matches!(first, ReblockOutcome::Reblocked { .. }));
    let after_first = event_count(&pool).await;
    assert_eq!(reblocked_event_count(&pool).await, 1);

    // Then: the resource is section-aligned — the identical partition fires nothing.
    let second = writes::reblock_resource(
        &pool,
        ReblockParams {
            resource,
            emitter: emitter_of(&actor),
        },
    )
    .await
    .unwrap();
    assert_eq!(second, ReblockOutcome::NoOp, "aligned resource: no-op");
    assert_eq!(
        event_count(&pool).await,
        after_first,
        "the ledger is indistinguishable from the operation never having run"
    );
    assert_eq!(reblocked_event_count(&pool).await, 1);
}

/// 5. Attribution: never lost, never fabricated. A split of an attributed block carries to both
/// halves with is_carried=true; an absorbed block's rows union as DIRECT; the kept block's own
/// source is never re-inserted under the new event; unattributed blocks carry nothing.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn attribution_is_carried_marked_and_never_fabricated(pool: sqlx::PgPool) {
    bootseed::seed_system(&pool).await.unwrap();
    let actor = system_actor(&pool).await;
    let home = make_home(&pool, actor.0, "reblock-attrib").await;

    // (a) SPLIT: one attributed block holding two sections → both halves carry, marked carried.
    let split_resource = create_body_resource(
        &pool,
        actor.0,
        emitter_of(&actor),
        &home,
        "split",
        BODY_A_B,
        vec![Incorporation {
            source: ProvenanceSource::Remote("https://example.test/origin".into()),
            seq: 0,
        }],
    )
    .await;
    writes::reblock_resource(
        &pool,
        ReblockParams {
            resource: split_resource,
            emitter: emitter_of(&actor),
        },
    )
    .await
    .unwrap();
    let split_rows: Vec<(Uuid, bool, i32)> = sqlx::query_as(
        "SELECT p.block_id, p.is_carried, p.accretion_seq \
           FROM kb_block_provenance p \
           JOIN kb_content_blocks b ON b.id=p.block_id AND NOT b.is_folded \
          WHERE b.resource_id=$1",
    )
    .bind(split_resource.uuid())
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(split_rows.len(), 2, "both halves carry the source");
    for (block, carried, seq) in &split_rows {
        assert!(
            *carried,
            "a split copy is CARRIED, never direct (block {block})"
        );
        assert_eq!(*seq, 0, "accretion order preserved");
    }
    let distinct: std::collections::HashSet<Uuid> = split_rows.iter().map(|(b, _, _)| *b).collect();
    assert_eq!(distinct.len(), 2, "two DIFFERENT blocks carry it");
    // The folded incumbent's own rows are history — untouched by the delta rule.
    let folded_rows: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM kb_block_provenance p \
           JOIN kb_content_blocks b ON b.id=p.block_id AND b.is_folded \
          WHERE b.resource_id=$1",
    )
    .bind(split_resource.uuid())
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(folded_rows, 1, "attribution is never LOST from history");

    // (b) KEPT + UNATTRIBUTED: block 0 carries a resource source and is KEPT; the created
    // sections of the unattributed block carry nothing; the kept source is not re-inserted.
    let other = create_body_resource(
        &pool,
        actor.0,
        emitter_of(&actor),
        &home,
        "other-source",
        "source body\n",
        vec![],
    )
    .await;
    let resource = create_segmented_two_block(
        &pool,
        actor.0,
        emitter_of(&actor),
        &home,
        SECTION_A,
        &format!("{SECTION_B}{SECTION_C}"),
        vec![Incorporation {
            source: ProvenanceSource::Resource(other.uuid()),
            seq: 0,
        }],
    )
    .await;
    writes::reblock_resource(
        &pool,
        ReblockParams {
            resource,
            emitter: emitter_of(&actor),
        },
    )
    .await
    .unwrap();
    let kept_rows: Vec<(Uuid, bool, Uuid)> = sqlx::query_as(
        "SELECT p.block_id, p.is_carried, p.contributed_by_event_id \
           FROM kb_block_provenance p \
           JOIN kb_content_blocks b ON b.id=p.block_id AND NOT b.is_folded \
           JOIN kb_events e ON e.id=p.contributed_by_event_id \
           JOIN kb_event_types t ON t.id=e.event_type_id \
          WHERE b.resource_id=$1 AND t.name='resource_reblocked'",
    )
    .bind(resource.uuid())
    .fetch_all(&pool)
    .await
    .unwrap();
    assert!(
        kept_rows.is_empty(),
        "the kept block's own source is NEVER re-inserted under the re-block event"
    );
    let kept_row: (bool, i64) = sqlx::query_as(
        "SELECT NOT p.is_carried, (SELECT count(*) FROM kb_block_provenance q \
                                    WHERE q.block_id=p.block_id AND NOT q.is_corrected) \
           FROM kb_block_provenance p \
           JOIN kb_content_blocks b ON b.id=p.block_id AND NOT b.is_folded \
          WHERE b.resource_id=$1",
    )
    .bind(resource.uuid())
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(
        kept_row.0,
        "the kept row reads DIRECT — it rode along, unmarked"
    );
    assert_eq!(
        kept_row.1, 1,
        "exactly once — no duplicate under the new event"
    );
}

/// 6. Replay symmetry: fire, snapshot, reset the namespace, replay, and the projections come
/// back identical — including kb_block_content's byte-sets and the created blocks' rows.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn replay_reproduces_the_reblocked_partition(pool: sqlx::PgPool) {
    bootseed::seed_system(&pool).await.unwrap();
    let actor = system_actor(&pool).await;
    let home = make_home(&pool, actor.0, "reblock-replay").await;
    let resource = create_body_resource(
        &pool,
        actor.0,
        emitter_of(&actor),
        &home,
        "replay",
        BODY_A_B,
        vec![Incorporation {
            source: ProvenanceSource::Remote("https://example.test/replay".into()),
            seq: 0,
        }],
    )
    .await;
    writes::reblock_resource(
        &pool,
        ReblockParams {
            resource,
            emitter: emitter_of(&actor),
        },
    )
    .await
    .unwrap();
    assert_eq!(reblocked_event_count(&pool).await, 1);

    let before = replay::dump_projections(&pool).await.unwrap();
    let snap = replay::snapshot(&pool).await.unwrap();
    common::reset_schema(&pool).await;
    replay::replay(&pool, &snap).await.unwrap();
    let after = replay::dump_projections(&pool).await.unwrap();

    for ((table_a, a), (table_b, b)) in before.iter().zip(after.iter()) {
        assert_eq!(table_a, table_b);
        assert_eq!(a, b, "projection table {table_a} diverged under replay");
    }
}

/// 7. Differential: re-block output ≡ fresh ingest of the same PARTITION. The twin is ingested
/// FRESH into the section-aligned partition (segmented [A][B]) — the partition a re-block of a
/// one-block body should produce. The re-blocked resource's chunk-hash sequence, header_path
/// sequence, verbatim body, body hash, and block-grain merkle set all equal the twin's.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn reblock_output_equals_a_fresh_ingest_of_the_same_partition(pool: sqlx::PgPool) {
    bootseed::seed_system(&pool).await.unwrap();
    let actor = system_actor(&pool).await;
    let home = make_home(&pool, actor.0, "reblock-diff").await;
    // twin_a: the ordinary one-block create — what a re-block starts from.
    let twin_a = create_body_resource(
        &pool,
        actor.0,
        emitter_of(&actor),
        &home,
        "twin-a",
        BODY_A_B,
        vec![],
    )
    .await;
    // twin_b: the FRESH INGEST of the same partition — aligned [A][B] blocks, bytes and all.
    let twin_b = create_segmented_two_block(
        &pool,
        actor.0,
        emitter_of(&actor),
        &home,
        SECTION_A,
        SECTION_B,
        vec![],
    )
    .await;
    writes::reblock_resource(
        &pool,
        ReblockParams {
            resource: twin_a,
            emitter: emitter_of(&actor),
        },
    )
    .await
    .unwrap();

    let (hash_a, body_a) = body_state(&pool, twin_a).await;
    let (hash_b, body_b) = body_state(&pool, twin_b).await;
    assert_eq!(body_a, body_b, "verbatim bodies equal");
    assert_eq!(
        hash_a, hash_b,
        "body hashes equal — same partition, same merkle"
    );

    let chunks_a = live_chunk_rows(&pool, twin_a).await;
    let chunks_b = live_chunk_rows(&pool, twin_b).await;
    let hashes_a: Vec<&String> = chunks_a.iter().map(|(h, ..)| h).collect();
    let hashes_b: Vec<&String> = chunks_b.iter().map(|(h, ..)| h).collect();
    assert_eq!(hashes_a, hashes_b, "chunk-hash sequences equal");
    let paths_a: Vec<&Option<String>> = chunks_a.iter().map(|(_, _, p, _, _)| p).collect();
    let paths_b: Vec<&Option<String>> = chunks_b.iter().map(|(_, _, p, _, _)| p).collect();
    assert_eq!(paths_a, paths_b, "header_path sequences equal");

    // Block-grain merkle SETS equal: the re-block's created blocks derive exactly the merkles
    // a fresh ingest of the same partition derives — sha256(ordered chunk hashes) is the same
    // function both ways, which is what makes identity-by-derived-hash sound.
    let merkle_a: Vec<String> = sqlx::query_scalar(
        "SELECT r.block_body_hash FROM kb_block_revisions r \
           JOIN kb_content_blocks b ON b.current_revision_id=r.id \
          WHERE b.resource_id=$1 AND NOT b.is_folded ORDER BY b.seq",
    )
    .bind(twin_a.uuid())
    .fetch_all(&pool)
    .await
    .unwrap();
    let merkle_b: Vec<String> = sqlx::query_scalar(
        "SELECT r.block_body_hash FROM kb_block_revisions r \
           JOIN kb_content_blocks b ON b.current_revision_id=r.id \
          WHERE b.resource_id=$1 AND NOT b.is_folded ORDER BY b.seq",
    )
    .bind(twin_b.uuid())
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(merkle_a, merkle_b, "block-grain merkle sequences equal");
}

/// 8. Refusals are well-formed: a derived-shape resource and an in_progress resource each
/// decline with a named reason and NO event row.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn refusals_decline_without_events(pool: sqlx::PgPool) {
    bootseed::seed_system(&pool).await.unwrap();
    let actor = system_actor(&pool).await;
    let home = make_home(&pool, actor.0, "reblock-refuse").await;

    // (a) A derived-shape resource: the cogmap's telos charter carries no verbatim bytes.
    let (cogmap, telos) = common::genesis_cogmap(&pool, "refusal-cogmap", "Refusal").await;
    let telos_resource = ResourceId::from(telos);
    let err = writes::reblock_resource(
        &pool,
        ReblockParams {
            resource: telos_resource,
            emitter: emitter_of(&actor),
        },
    )
    .await
    .unwrap_err();
    assert!(
        err.to_string().contains("verbatim bytes"),
        "the derived-shape refusal names its reason: {err}"
    );
    let _ = cogmap;

    // (b) A still-arriving body: a segmented create is born in_progress.
    let in_progress = writes::create_resource_with_mode(
        &pool,
        CreateParams {
            title: "arriving",
            origin_uri: "temper://reblock/arriving",
            body: SECTION_A,
            doc_type: "concept",
            home,
            owner: actor.0,
            originator: actor.0,
            emitter: emitter_of(&actor),
            properties: &[],
            chunks: None,
            sources: vec![],
            idempotency_key: None,
        },
        temper_substrate::events::EventContext::default(),
        CreateMode {
            defer: false,
            segmented: true,
        },
    )
    .await
    .unwrap();
    let err = writes::reblock_resource(
        &pool,
        ReblockParams {
            resource: in_progress,
            emitter: emitter_of(&actor),
        },
    )
    .await
    .unwrap_err();
    assert!(
        err.to_string().contains("in_progress"),
        "the in_progress refusal names its reason: {err}"
    );

    assert_eq!(
        reblocked_event_count(&pool).await,
        0,
        "a refusal appends nothing"
    );
}

/// 9. Roles are never fabricated: a re-block creates no block_role property rows; a folded
/// block keeps its role (history); created blocks are born roleless.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn block_roles_are_never_fabricated(pool: sqlx::PgPool) {
    bootseed::seed_system(&pool).await.unwrap();
    let actor = system_actor(&pool).await;
    let home = make_home(&pool, actor.0, "reblock-roles").await;
    let resource = create_body_resource(
        &pool,
        actor.0,
        emitter_of(&actor),
        &home,
        "roles",
        BODY_A_B,
        vec![],
    )
    .await;
    let (block, _, _, _, _): (Uuid, i32, bool, Uuid, Uuid) = sqlx::query_as(
        "SELECT id, seq, is_folded, genesis_event_id, current_revision_id \
           FROM kb_content_blocks WHERE resource_id=$1 AND NOT is_folded",
    )
    .bind(resource.uuid())
    .fetch_one(&pool)
    .await
    .unwrap();
    // Fixture: the incumbent carries a role (as a charter/scenario block would).
    sqlx::query(
        "INSERT INTO kb_properties (owner_table, owner_id, property_key, property_value, \
                                    asserted_by_event_id, last_event_id) \
          SELECT 'kb_content_blocks', b.id, 'block_role', to_jsonb('statement'::text), b.genesis_event_id, b.genesis_event_id \
            FROM kb_content_blocks b WHERE b.id=$1",
    )
    .bind(block)
    .execute(&pool)
    .await
    .unwrap();

    writes::reblock_resource(
        &pool,
        ReblockParams {
            resource,
            emitter: emitter_of(&actor),
        },
    )
    .await
    .unwrap();

    let roles: Vec<(Uuid, bool)> = sqlx::query_as(
        "SELECT b.id, b.is_folded FROM kb_properties p \
           JOIN kb_content_blocks b ON b.id=p.owner_id \
          WHERE b.resource_id=$1 AND p.property_key='block_role'",
    )
    .bind(resource.uuid())
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(
        roles.len(),
        1,
        "exactly the folded incumbent keeps its role"
    );
    assert!(
        roles[0].1,
        "the surviving role row belongs to the FOLDED block (history)"
    );
    let live_roles: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM kb_properties p \
           JOIN kb_content_blocks b ON b.id=p.owner_id AND NOT b.is_folded \
          WHERE b.resource_id=$1 AND p.property_key='block_role'",
    )
    .bind(resource.uuid())
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        live_roles, 0,
        "created blocks are born roleless — never fabricated"
    );
}
