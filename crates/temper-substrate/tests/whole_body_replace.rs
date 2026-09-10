#![cfg(feature = "artifact-tests")]
//! Witnesses for the whole-body replace arm (the ergonomics design pass, 2026-09-08, goal
//! Anchor addressability — the identity clause's named remainder). One behavior per test, each
//! through the real write path (`update_resource` whole-body semantics): sections are compared
//! against PRE-update incumbents by CONTENT-hash, kept sections survive with identity, folded
//! incumbents' provenance redistributes under absorbed/carried, and content-gone rows stay
//! history on the folded block (ruled). Fixtures duplicate this suite's convention: resources
//! are seeded through the real write machinery (segmented trio for chosen block boundaries), so
//! every incumbent carries real stored bytes and real chunk rows.
//!
//! ONNX-dependent. Isolated ephemeral DB via `temper_substrate::MIGRATOR`.

mod common;

use temper_substrate::payloads::{AnchorRef, Incorporation, ProvenanceSource};
use temper_substrate::writes::{
    self, AppendParams, CreateMode, CreateParams, FinalizeParams, UpdateParams,
};
use temper_substrate::{replay, scenario::bootseed};
use uuid::Uuid;

const SECTION_A: &str = "# Alpha\n\nAlpha body paragraph.\n";
const SECTION_B: &str = "## Beta\n\nBeta body paragraph.\n";
const BODY_A_B: &str = "# Alpha\n\nAlpha body paragraph.\n## Beta\n\nBeta body paragraph.\n";

// ── fixture helpers (duplicated per file, per this suite's convention) ──────────────────────

async fn system_actor(
    pool: &sqlx::PgPool,
) -> (
    temper_substrate::ids::ProfileId,
    temper_substrate::ids::EntityId,
) {
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
    (
        temper_substrate::ids::ProfileId::from(profile),
        temper_substrate::ids::EntityId::from(entity),
    )
}

async fn make_home(
    pool: &sqlx::PgPool,
    owner: temper_substrate::ids::ProfileId,
    slug: &str,
) -> AnchorRef {
    let ctx = common::insert_context(pool, "kb_profiles", owner.uuid(), slug, slug)
        .await
        .unwrap();
    AnchorRef::context(temper_substrate::ids::ContextId::from(ctx))
}

/// A two-block resource through the segmented trio (block boundaries chosen by the fixture, not
/// the policy — the one honest way a resource gets blocks that are not section-aligned, or
/// multi-section blocks).
async fn create_two_block(
    pool: &sqlx::PgPool,
    owner: temper_substrate::ids::ProfileId,
    emitter: temper_substrate::ids::EntityId,
    home: &AnchorRef,
    first: &str,
    rest: &str,
    block0_sources: Vec<Incorporation>,
) -> temper_substrate::ids::ResourceId {
    use temper_substrate::events::EventContext;
    let resource = writes::create_resource_with_mode(
        pool,
        CreateParams {
            title: "whole-body fixture",
            origin_uri: "temper://whole-body/fixture",
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
        EventContext::default(),
        CreateMode {
            defer: false,
            segmented: true,
        },
    )
    .await
    .unwrap();
    let breadcrumb: Vec<String> = temper_ingest::chunk::chunk_markdown(first)
        .last()
        .filter(|c| !c.header_path.is_empty())
        .map(|c| c.header_path.split(" > ").map(str::to_owned).collect())
        .unwrap_or_default();
    let mut block1 =
        temper_substrate::content::prepare_block_with_prefix(1, None, rest, &breadcrumb).unwrap();
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
            expected_body_hash: temper_substrate::content::body_hash_from_block_chunk_hashes(&[
                h0, h1,
            ]),
            expected_content_hash: None,
            emitter,
        },
    )
    .await
    .unwrap();
    resource
}

async fn update_body(
    pool: &sqlx::PgPool,
    emitter: temper_substrate::ids::EntityId,
    resource: temper_substrate::ids::ResourceId,
    body: &str,
    sources: Vec<Incorporation>,
) {
    writes::update_resource(
        pool,
        UpdateParams {
            resource,
            body: Some(body),
            title: None,
            origin_uri: None,
            properties: &[],
            chunks: None,
            sources,
            content_block: None,
            rehome_to: None,
            emitter,
        },
    )
    .await
    .unwrap();
}

/// (id, seq) of the LIVE blocks, seq order. Folded rows stay history — they are asserted
/// separately where a witness cares.
async fn blocks_of(
    pool: &sqlx::PgPool,
    resource: temper_substrate::ids::ResourceId,
) -> Vec<(Uuid, i32)> {
    sqlx::query_as(
        "SELECT id, seq FROM kb_content_blocks WHERE resource_id=$1 AND NOT is_folded ORDER BY seq, id",
    )
    .bind(resource.uuid())
    .fetch_all(pool)
    .await
    .unwrap()
}

/// (source locator, block_seq, is_carried, contributed_by_event_id) for the resource's LIVE
/// blocks' uncorrected provenance rows.
async fn provenance_rows(
    pool: &sqlx::PgPool,
    resource: temper_substrate::ids::ResourceId,
) -> Vec<(String, i32, bool, Uuid)> {
    sqlx::query_as(
        "SELECT coalesce(r.uri, p.source_kind || ':' || p.source_id::text), b.seq, p.is_carried, \
                p.contributed_by_event_id \
           FROM kb_block_provenance p \
           JOIN kb_content_blocks b ON b.id = p.block_id \
           LEFT JOIN kb_remote_sources r ON p.source_kind='remote' AND r.id = p.source_id \
          WHERE b.resource_id=$1 AND NOT b.is_folded AND NOT p.is_corrected \
          ORDER BY b.seq, p.accretion_seq",
    )
    .bind(resource.uuid())
    .fetch_all(pool)
    .await
    .unwrap()
}

fn source(url: &str, seq: i32) -> Incorporation {
    Incorporation {
        source: ProvenanceSource::Remote(url.to_owned()),
        seq,
    }
}

// ── the witnesses ───────────────────────────────────────────────────────────

/// THE shipped defect, closed: a byte-identical whole-body rewrite carrying sources used to fold
/// every sibling and re-mint fresh ids for every section (the short-circuit is broken by
/// sources; kept-detection post-mutate could match nothing). Under the replace-shaped partition
/// every section KEEPS its incumbent — same ids, same revisions — and the caller's body-grain
/// sources land as carried rows on each section, appended under the new event.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn identical_body_with_sources_keeps_every_block_and_appends_carried_sources(
    pool: sqlx::PgPool,
) {
    bootseed::seed_system(&pool).await.unwrap();
    let (owner, emitter) = system_actor(&pool).await;
    let home = make_home(&pool, owner, "wb-identical").await;
    let resource =
        create_two_block(&pool, owner, emitter, &home, SECTION_A, SECTION_B, vec![]).await;
    let before = blocks_of(&pool, resource).await;
    assert_eq!(before.len(), 2);

    update_body(
        &pool,
        emitter,
        resource,
        BODY_A_B,
        vec![source("https://ex.com/a", 0)],
    )
    .await;

    let after = blocks_of(&pool, resource).await;
    assert_eq!(after.len(), 2, "no block may fold or be re-minted");
    for (b, a) in before.iter().zip(after.iter()) {
        assert_eq!(b.0, a.0, "block identity must survive an identical body");
        assert_eq!(b.1, a.1, "seq is unchanged");
    }
    let folded: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM kb_content_blocks WHERE resource_id=$1 AND is_folded",
    )
    .bind(resource.uuid())
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(folded, 0, "an identical body folds nothing");
    // The event fired (sources break the no-op) and the source landed carried = true on EACH
    // section — body-grain sources never read as direct on a multi-section body.
    let rows = provenance_rows(&pool, resource).await;
    assert_eq!(rows.len(), 2, "one carried row per section");
    for (loc, _, carried, _) in &rows {
        assert_eq!(loc, "https://ex.com/a");
        assert!(carried, "a body-grain source is carried, never direct");
    }
    let event_ids: std::collections::HashSet<Uuid> = rows.iter().map(|r| r.3).collect();
    assert_eq!(event_ids.len(), 1, "both rows ride the ONE replace event");
}

/// A whole-body edit that rewrites section A's prose folds A — and A's annotation must
/// redistribute onto the section that now holds its content, ABSORBED (carried = false: the
/// content IS in the block), never dropped, never presented as carried when it is whole.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn a_folded_incumbents_annotation_is_absorbed_by_the_section_that_holds_it(
    pool: sqlx::PgPool,
) {
    bootseed::seed_system(&pool).await.unwrap();
    let (owner, emitter) = system_actor(&pool).await;
    let home = make_home(&pool, owner, "wb-absorbed").await;
    let resource = create_two_block(
        &pool,
        owner,
        emitter,
        &home,
        SECTION_A,
        SECTION_B,
        vec![source("https://ex.com/alpha-src", 0)],
    )
    .await;
    // The rewrite appends a full extra page of new prose to A's section — the section's bytes
    // differ (A folds) but the alpha paragraph itself survives VERBATIM, so its chunk hash is
    // preserved as one chunk of the new section (the chunker splits at the budget): the
    // absorbed case, not content-gone.
    let filler = "Brand new prose. ".repeat(120);
    let new_body =
        format!("# Alpha\n\nAlpha body paragraph.\n\n{filler}\n## Beta\n\nBeta body paragraph.\n");
    update_body(&pool, emitter, resource, &new_body, vec![]).await;

    let blocks = blocks_of(&pool, resource).await;
    assert_eq!(
        blocks.len(),
        2,
        "A folds, its section re-creates; B is kept"
    );
    // The folded A's row redistributes onto the section holding the alpha prose — absorbed,
    // carried = false (direct-quality: the content IS in the block).
    let rows = provenance_rows(&pool, resource).await;
    let alpha_rows: Vec<_> = rows
        .iter()
        .filter(|(l, _, _, _)| *l == "https://ex.com/alpha-src")
        .collect();
    assert_eq!(
        alpha_rows.len(),
        1,
        "exactly one live row for the absorbed source"
    );
    assert!(
        !alpha_rows[0].2,
        "absorbed is asserted-quality, never carried"
    );
}

/// An incumbent whose content SPANS the new partition (its chunk hashes land in several
/// sections) is carried by each section holding part — distinguishable at row grain, never
/// re-united as direct. The incumbent is a genuinely multi-chunk block: one heading section
/// whose prose exceeds the chunk budget, so the chunker splits it into two chunks; the rewrite
/// then splits that prose across two headings, and no section contains both hashes.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn a_spanning_incumbents_rows_carry_to_every_section_holding_part(pool: sqlx::PgPool) {
    bootseed::seed_system(&pool).await.unwrap();
    let (owner, emitter) = system_actor(&pool).await;
    let home = make_home(&pool, owner, "wb-spanning").await;
    // One heading section, two long single-line paragraphs: the chunker accumulates P1 (under
    // budget), then refuses P2 (would exceed) and splits — exactly two chunks, split between
    // the paragraphs.
    let p1 = "Span paragraph one sentence. ".repeat(48); // ~1344 chars: one line, under budget
    let p2 = "Span paragraph two sentence. ".repeat(16); // adding it would exceed the budget
    let span_block = format!("## Span\n\n{p1}\n{p2}\n");
    let resource =
        create_two_block(&pool, owner, emitter, &home, SECTION_A, &span_block, vec![]).await;
    let block1_id: Uuid = sqlx::query_scalar(
        "SELECT id FROM kb_content_blocks WHERE resource_id=$1 AND seq=1 AND NOT is_folded",
    )
    .bind(resource.uuid())
    .fetch_one(&pool)
    .await
    .unwrap();
    let chunk_count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM kb_chunks WHERE block_id=$1 AND is_current")
            .bind(block1_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(
        chunk_count, 2,
        "fixture: the span block must hold TWO chunks"
    );
    writes::annotate_block_sources(
        &pool,
        writes::AnnotateParams {
            resource,
            sources: vec![source("https://ex.com/span-src", 0)],
            content_block: Some(block1_id),
            emitter,
        },
    )
    .await
    .unwrap();

    // The rewrite splits the span prose across two headings: the incumbent folds, and each of
    // its chunk hashes lands in a DIFFERENT section — carried to both, absorbed nowhere.
    let new_body = format!("## Span\n\n{p1}\n\n## Tail\n\n{p2}\n");
    update_body(&pool, emitter, resource, &new_body, vec![]).await;

    let rows = provenance_rows(&pool, resource).await;
    let span_rows: Vec<_> = rows
        .iter()
        .filter(|(l, _, _, _)| *l == "https://ex.com/span-src")
        .collect();
    assert_eq!(span_rows.len(), 2, "carried to every section holding part");
    for (_, _, carried, _) in &span_rows {
        assert!(carried, "a spanning copy is carried, never direct");
    }
    // The two sections hold DIFFERENT parts — the copies sit on different blocks.
    assert_ne!(
        span_rows[0].1, span_rows[1].1,
        "carried copies land on distinct sections"
    );
}

/// Content-gone (ruled, 2026-09-08): when the rewrite deletes an incumbent's content outright,
/// its rows do NOT redistribute — there is no honest live target, and stamping them onto
/// unrelated prose would fabricate. They remain history on the folded row.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn content_gone_rows_stay_history_on_the_folded_block(pool: sqlx::PgPool) {
    bootseed::seed_system(&pool).await.unwrap();
    let (owner, emitter) = system_actor(&pool).await;
    let home = make_home(&pool, owner, "wb-gone").await;
    let resource = create_two_block(
        &pool,
        owner,
        emitter,
        &home,
        SECTION_A,
        SECTION_B,
        vec![source("https://ex.com/doomed-src", 0)],
    )
    .await;
    // The rewrite deletes B's section entirely and rewrites A's prose so A folds too: every
    // incumbent is content-gone.
    let new_body = "# Alpha\n\nA wholly different body with none of the old prose.\n";
    update_body(&pool, emitter, resource, new_body, vec![]).await;

    let blocks = blocks_of(&pool, resource).await;
    assert_eq!(blocks.len(), 1);
    // No LIVE block carries either old source: the doomed row stays history on the folded rows.
    let rows = provenance_rows(&pool, resource).await;
    assert!(
        rows.iter()
            .all(|(l, _, _, _)| l != "https://ex.com/doomed-src"),
        "content-gone rows must not fabricate onto live prose"
    );
    let folded_rows: Vec<(String, bool)> = sqlx::query_as(
        "SELECT coalesce(r.uri, ''), p.is_carried \
           FROM kb_block_provenance p \
           JOIN kb_content_blocks b ON b.id = p.block_id \
           LEFT JOIN kb_remote_sources r ON p.source_kind='remote' AND r.id=p.source_id \
          WHERE b.resource_id=$1 AND b.is_folded",
    )
    .bind(resource.uuid())
    .fetch_all(&pool)
    .await
    .unwrap();
    assert!(
        folded_rows
            .iter()
            .any(|(l, _)| l == "https://ex.com/doomed-src"),
        "the row survives as the folded block's reference"
    );
}

/// Duplicate identical sections: kept detection claims incumbents in seq order, deterministically;
/// the folded duplicate's annotation unions onto the kept survivor — SKIPPED where the survivor
/// already holds the source (the union adds nothing), landed otherwise.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn duplicate_sections_keep_in_order_and_a_folded_duplicate_unions_onto_the_survivor(
    pool: sqlx::PgPool,
) {
    bootseed::seed_system(&pool).await.unwrap();
    let (owner, emitter) = system_actor(&pool).await;
    let home = make_home(&pool, owner, "wb-duplicate").await;
    // Block 0 = A (no sources); block 1 = an identical copy of A, annotated with S.
    let resource =
        create_two_block(&pool, owner, emitter, &home, SECTION_A, SECTION_A, vec![]).await;
    // Annotate ONLY the second copy (block seq 1) with S — the annotate path addresses the
    // sole... it needs the block id; do it via a direct annotate call.
    let block1_id: Uuid = sqlx::query_scalar(
        "SELECT id FROM kb_content_blocks WHERE resource_id=$1 AND seq=1 AND NOT is_folded",
    )
    .bind(resource.uuid())
    .fetch_one(&pool)
    .await
    .unwrap();
    writes::annotate_block_sources(
        &pool,
        writes::AnnotateParams {
            resource,
            sources: vec![source("https://ex.com/dup-src", 0)],
            content_block: Some(block1_id),
            emitter,
        },
    )
    .await
    .unwrap();

    // The rewrite collapses the duplicate pair into ONE section byte-equal to copy 1: copy 1
    // is KEPT (first match in seq order), copy 2 folds, and its S union lands on the survivor —
    // which does NOT hold S, so the union lands (carried = false, direct-quality: the content
    // IS the survivor's).
    update_body(&pool, emitter, resource, SECTION_A, vec![]).await;

    let rows = provenance_rows(&pool, resource).await;
    let dup_rows: Vec<_> = rows
        .iter()
        .filter(|(l, _, _, _)| *l == "https://ex.com/dup-src")
        .collect();
    assert_eq!(
        dup_rows.len(),
        1,
        "the folded duplicate's source unions onto the survivor"
    );
    assert!(
        !dup_rows[0].2,
        "an absorbed union reads direct — the content IS the survivor's"
    );
}

/// A section permutation is a legal edit: both incumbents keep their identity, only their seqs
/// move, and the replace event fires (a kept-only manifest is a real act — the entry guard
/// admits it).
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn a_section_permutation_keeps_identity_and_moves_seqs(pool: sqlx::PgPool) {
    bootseed::seed_system(&pool).await.unwrap();
    let (owner, emitter) = system_actor(&pool).await;
    let home = make_home(&pool, owner, "wb-permute").await;
    let resource =
        create_two_block(&pool, owner, emitter, &home, SECTION_A, SECTION_B, vec![]).await;
    let before = blocks_of(&pool, resource).await;

    update_body(
        &pool,
        emitter,
        resource,
        "## Beta\n\nBeta body paragraph.\n# Alpha\n\nAlpha body paragraph.\n",
        vec![],
    )
    .await;

    let after = blocks_of(&pool, resource).await;
    assert_eq!(after.len(), 2, "nothing folds on a pure permutation");
    let ids_before: Vec<Uuid> = before.iter().map(|b| b.0).collect();
    let ids_after: Vec<Uuid> = after.iter().map(|b| b.0).collect();
    for id in &ids_before {
        assert!(ids_after.contains(id), "both incumbents survive");
    }
    // seqs swapped: the alpha block now sits at seq 1, beta at seq 0.
    let alpha_seq = after.iter().find(|b| b.0 == before[0].0).unwrap().1;
    let beta_seq = after.iter().find(|b| b.0 == before[1].0).unwrap().1;
    assert_eq!((alpha_seq, beta_seq), (1, 0), "the permutation moved seqs");
}

/// An identical whole-body rewrite with no sources is SILENT. This witness exercises the
/// PARTITION-level no-op (a single-block resource has no multi-block short-circuit: the replace
/// arm runs, computes an everything-kept no-move partition, and must fire NOTHING — the twin of
/// the SQL entry guard). The multi-block short-circuit has its own coverage in
/// content_mutation's byte_identical witness.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn an_identical_body_without_sources_is_silent(pool: sqlx::PgPool) {
    bootseed::seed_system(&pool).await.unwrap();
    let (owner, emitter) = system_actor(&pool).await;
    let home = make_home(&pool, owner, "wb-silent").await;
    let resource =
        create_two_block(&pool, owner, emitter, &home, SECTION_A, SECTION_A, vec![]).await;
    // Collapse to the single-section body equal to BOTH blocks' content: the first incumbent
    // is kept, the duplicate folds content-gone — the partition then re-run over the SAME body
    // is a pure no-op.
    update_body(&pool, emitter, resource, SECTION_A, vec![]).await;
    let stable = blocks_of(&pool, resource).await;
    assert_eq!(stable.len(), 1);
    let events_before: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM kb_events e \
          WHERE (e.payload->>'resource_id')::uuid = $1 \
             OR EXISTS (SELECT 1 FROM kb_content_blocks b \
                         WHERE b.id = (e.payload->>'block_id')::uuid AND b.resource_id = $1)",
    )
    .bind(resource.uuid())
    .fetch_one(&pool)
    .await
    .unwrap();
    let before = blocks_of(&pool, resource).await;

    update_body(&pool, emitter, resource, SECTION_A, vec![]).await;

    assert_eq!(blocks_of(&pool, resource).await, before, "nothing moves");
    let events_after: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM kb_events e \
          WHERE (e.payload->>'resource_id')::uuid = $1 \
             OR EXISTS (SELECT 1 FROM kb_content_blocks b \
                         WHERE b.id = (e.payload->>'block_id')::uuid AND b.resource_id = $1)",
    )
    .bind(resource.uuid())
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        events_before, events_after,
        "a no-op write appends no event"
    );
}

/// The dedup-fallback: when an update both ABSORBS a folded duplicate holding source S onto a
/// kept survivor that already holds S, AND the caller re-asserts S, the caller's append still
/// lands (a second row under the replace event) — resolution never depends on seq ordering,
/// and a held union never silently swallows the caller's assertion.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn a_held_union_does_not_silently_swallow_the_callers_reassertion(pool: sqlx::PgPool) {
    bootseed::seed_system(&pool).await.unwrap();
    let (owner, emitter) = system_actor(&pool).await;
    let home = make_home(&pool, owner, "wb-fallback").await;
    // Two byte-identical copies, EACH annotated with S.
    let resource =
        create_two_block(&pool, owner, emitter, &home, SECTION_A, SECTION_A, vec![]).await;
    for seq in 0..=1 {
        let block_id: Uuid = sqlx::query_scalar(
            "SELECT id FROM kb_content_blocks WHERE resource_id=$1 AND seq=$2 AND NOT is_folded",
        )
        .bind(resource.uuid())
        .bind(seq)
        .fetch_one(&pool)
        .await
        .unwrap();
        writes::annotate_block_sources(
            &pool,
            writes::AnnotateParams {
                resource,
                sources: vec![source("https://ex.com/dup-src", 0)],
                content_block: Some(block_id),
                emitter,
            },
        )
        .await
        .unwrap();
    }
    // Collapse to ONE byte-equal section: copy 1 keeps, copy 2 folds (its S unions onto the
    // survivor, which holds S — dropped), AND the caller re-asserts S. The append must land.
    update_body(
        &pool,
        emitter,
        resource,
        SECTION_A,
        vec![source("https://ex.com/dup-src", 5)],
    )
    .await;

    let rows = provenance_rows(&pool, resource).await;
    let dup_rows: Vec<_> = rows
        .iter()
        .filter(|(l, _, _, _)| *l == "https://ex.com/dup-src")
        .collect();
    assert_eq!(
        dup_rows.len(),
        2,
        "the caller's re-assertion appended alongside the held row"
    );
    let event_ids: std::collections::HashSet<Uuid> = dup_rows.iter().map(|r| r.3).collect();
    assert_eq!(
        event_ids.len(),
        2,
        "two rows, two events: the annotate and the replace"
    );
}

/// A single-SECTION body asserts caller sources DIRECT (carried = false — the section covers
/// the whole span, matching the single-block posture the arm replaced), on a fully-kept
/// partition: the assertion-only manifest fires and appends.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn a_single_section_body_asserts_caller_sources_direct(pool: sqlx::PgPool) {
    bootseed::seed_system(&pool).await.unwrap();
    let (owner, emitter) = system_actor(&pool).await;
    let home = make_home(&pool, owner, "wb-direct").await;
    let resource =
        create_two_block(&pool, owner, emitter, &home, SECTION_A, SECTION_B, vec![]).await;
    // Collapse to one section equal to block 0's bytes... the body must compose to ONE section
    // whose bytes+merkle keep an incumbent: use exactly SECTION_A. Block 1 (beta) folds
    // content-gone.
    update_body(
        &pool,
        emitter,
        resource,
        SECTION_A,
        vec![source("https://ex.com/direct", 0)],
    )
    .await;

    let rows = provenance_rows(&pool, resource).await;
    let direct: Vec<_> = rows
        .iter()
        .filter(|(l, _, _, _)| *l == "https://ex.com/direct")
        .collect();
    assert_eq!(direct.len(), 1);
    assert!(
        !direct[0].2,
        "single-section span is covered wholly: direct, not carried"
    );
}

/// Replay round-trip: a replace-shaped event's snapshot re-supplies its created chunks, so the
/// replayed projection reproduces the partition byte-for-byte (the shipped op's replay witness,
/// re-earned for the new shape).
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn replay_reproduces_a_whole_body_replace(pool: sqlx::PgPool) {
    bootseed::seed_system(&pool).await.unwrap();
    let (owner, emitter) = system_actor(&pool).await;
    let home = make_home(&pool, owner, "wb-replay").await;
    let resource =
        create_two_block(&pool, owner, emitter, &home, SECTION_A, SECTION_B, vec![]).await;
    let new_body = "# Alpha\n\nAlpha body paragraph, rewritten.\n## Beta\n\nBeta body paragraph.\n";
    update_body(&pool, emitter, resource, new_body, vec![]).await;
    let expected = blocks_of(&pool, resource).await;

    let before = replay::dump_projections(&pool).await.unwrap();
    let snap = replay::snapshot(&pool).await.unwrap();
    common::reset_schema(&pool).await;
    replay::replay(&pool, &snap).await.unwrap();
    let after = replay::dump_projections(&pool).await.unwrap();

    for ((table_a, a), (table_b, b)) in before.iter().zip(after.iter()) {
        assert_eq!(table_a, table_b);
        assert_eq!(a, b, "projection table {table_a} diverged under replay");
    }
    assert_eq!(
        expected,
        blocks_of(&pool, resource).await,
        "the replace partition survives replay"
    );
}

/// A KEPT-ONLY replace (section permutation — `created` empty, so the payload serializes no
/// `created` key at all) replays: the snapshot arm treats the absent key as an empty manifest,
/// and the round-trip reproduces the partition. This is the flagship shape the marker was
/// written to admit — a permutation anywhere must never poison the replay proof.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn replay_reproduces_a_kept_only_replace(pool: sqlx::PgPool) {
    bootseed::seed_system(&pool).await.unwrap();
    let (owner, emitter) = system_actor(&pool).await;
    let home = make_home(&pool, owner, "wb-replay-kept").await;
    let resource =
        create_two_block(&pool, owner, emitter, &home, SECTION_A, SECTION_B, vec![]).await;
    // Pure permutation: both incumbents keep, only seqs move, `created` is empty.
    update_body(
        &pool,
        emitter,
        resource,
        "## Beta\n\nBeta body paragraph.\n# Alpha\n\nAlpha body paragraph.\n",
        vec![],
    )
    .await;
    let expected = blocks_of(&pool, resource).await;
    assert_eq!(expected.len(), 2);

    let before = replay::dump_projections(&pool).await.unwrap();
    let snap = replay::snapshot(&pool).await.unwrap();
    common::reset_schema(&pool).await;
    replay::replay(&pool, &snap).await.unwrap();
    let after = replay::dump_projections(&pool).await.unwrap();

    for ((table_a, a), (table_b, b)) in before.iter().zip(after.iter()) {
        assert_eq!(table_a, table_b);
        assert_eq!(a, b, "projection table {table_a} diverged under replay");
    }
    assert_eq!(expected, blocks_of(&pool, resource).await);
}

/// The replace shape advances the content clock: the region readout-refresh gate must see the
/// resource after a whole-body replace (the arm no longer fires `block_mutated`, which was the
/// clock's only witness to prose movement).
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn the_replace_shape_advances_the_content_clock(pool: sqlx::PgPool) {
    bootseed::seed_system(&pool).await.unwrap();
    let (owner, emitter) = system_actor(&pool).await;
    let home = make_home(&pool, owner, "wb-clock").await;
    let resource =
        create_two_block(&pool, owner, emitter, &home, SECTION_A, SECTION_B, vec![]).await;
    let watermark: Uuid = sqlx::query_scalar(
        "SELECT coalesce((SELECT id FROM kb_events ORDER BY id DESC LIMIT 1), \
                         '00000000-0000-0000-0000-000000000000'::uuid)",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    update_body(
        &pool,
        emitter,
        resource,
        "# Alpha\n\nRewritten alpha prose.\n## Beta\n\nBeta body paragraph.\n",
        vec![],
    )
    .await;

    let anchor = temper_core::types::home::HomeAnchor::from_parts("kb_contexts", home.id)
        .expect("the fixture home is a context");
    let touched = replay::content_touched_resources_since(&pool, anchor, watermark)
        .await
        .unwrap();
    assert!(
        touched.contains(&resource.uuid()),
        "a whole-body replace is a content touch: the readout gate must see it"
    );
}
