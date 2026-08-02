#![cfg(feature = "artifact-tests")]
//! Evidential-standing maturity projection (SQL substrate).
//!
//! Exercises the producers/memos in `migrations/20260721000010_evidential_standing_memo.sql` as
//! amended by `migrations/20260724000120_standing_citation_components.sql` (Set 5, Task 3) against
//! an ephemeral DB. Standing is NOT truth (spec 019f81e8 preamble): these assert the *shape of the
//! evidence*, never a truth claim. Grounding for the seeding helpers: `content_mutation.rs`
//! (provenance via `writes`), `write_path_mutations.rs` (edges via `SeedAction::RelationshipAssert`),
//! `streaming_ingest_test.rs` (a second live block via `writes::append_block`), and
//! `citation_audits.rs` (audits via `writes::record_citation_audit`).
//!
//! Set 5 retired the pairwise-independence model (`kb_independence_pairs`,
//! `resource_independence_breadth`, `refresh_independence_pairs`) and the edge-based
//! `resource_adversarial_survival` reader — spec
//! `docs/superpowers/specs/2026-07-23-set5-adversary-citation-audit-design.md` §3.4. The four Set-3
//! tests that drove those objects (`silence_default_is_correlated`,
//! `affirmed_independence_raises_breadth`, `zero_challenges_is_not_survival`,
//! `band_is_read_time_over_components`) were DELETED with them rather than left to fail: a test of
//! a retired model is not a regression guard, it is a fossil.

mod common;

use sqlx::Row;
use temper_substrate::affinity::EdgeKind;
use temper_substrate::content::{self, IncomingChunk};
use temper_substrate::events::{fire, EdgeHome, EventContext, SeedAction};
use temper_substrate::ids::{BlockId, ContextId, EdgeId, EntityId, ProfileId, ResourceId};
use temper_substrate::payloads::{AnchorRef, EdgePolarity, Incorporation, ProvenanceSource};
use temper_substrate::scenario::bootseed;
use temper_substrate::write;
use temper_substrate::writes::{
    self, AppendParams, CitationAuditParams, CreateParams, UpdateParams,
};
use uuid::Uuid;

// ── fixtures ──────────────────────────────────────────────────────────────────────────────────

/// The canonical `system` actor as typed newtypes (pattern from write_path_mutations.rs:19).
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

async fn make_home(pool: &sqlx::PgPool, owner: ProfileId, slug: &str) -> ContextId {
    ContextId::from(
        common::insert_context(pool, "kb_profiles", owner.uuid(), slug, slug)
            .await
            .unwrap(),
    )
}

async fn make_resource(
    pool: &sqlx::PgPool,
    owner: ProfileId,
    emitter: EntityId,
    home: ContextId,
    title: &str,
    uri: &str,
) -> ResourceId {
    writes::create_resource_with(
        pool,
        CreateParams {
            idempotency_key: None,
            title,
            origin_uri: uri,
            body: "seed body",
            doc_type: "research",
            home: AnchorRef::context(home),
            owner,
            originator: owner,
            emitter,
            properties: &[],
            chunks: None,
            sources: vec![],
        },
        EventContext::default(),
    )
    .await
    .unwrap()
}

/// A finding with `n` uncorrected provenance rows, each contributed by a distinct resource-base.
/// One create (seq 0) + (n-1) revises (seq i), mirroring content_mutation.rs:290-344. Returns the
/// finding and its `n` base resources (the provenance sources).
async fn seed_finding_with_n_provenance(
    pool: &sqlx::PgPool,
    owner: ProfileId,
    emitter: EntityId,
    home: ContextId,
    n: usize,
) -> (ResourceId, Vec<ResourceId>) {
    assert!(n >= 1);
    let mut bases = Vec::new();
    for i in 0..n {
        bases.push(
            make_resource(
                pool,
                owner,
                emitter,
                home,
                &format!("src{i}"),
                &format!("temper://es/src{i}"),
            )
            .await,
        );
    }
    let finding = writes::create_resource_with(
        pool,
        CreateParams {
            idempotency_key: None,
            title: "finding",
            origin_uri: "temper://es/finding",
            body: "the claim under standing",
            doc_type: "research",
            home: AnchorRef::context(home),
            owner,
            originator: owner,
            emitter,
            properties: &[],
            chunks: None,
            sources: vec![Incorporation {
                source: ProvenanceSource::Resource(bases[0].uuid()),
                seq: 0,
            }],
        },
        EventContext::default(),
    )
    .await
    .unwrap();
    for (i, base) in bases.iter().enumerate().skip(1) {
        writes::update_resource_with(
            pool,
            UpdateParams {
                resource: finding,
                body: Some(&format!("revised body incorporating source {i}")),
                title: None,
                origin_uri: None,
                properties: &[],
                chunks: None,
                sources: vec![Incorporation {
                    source: ProvenanceSource::Resource(base.uuid()),
                    seq: i as i32,
                }],
                content_block: None,
                rehome_to: None,
                emitter,
            },
            EventContext::default(),
        )
        .await
        .unwrap();
    }
    (finding, bases)
}

/// Fire an `express` edge with a label between two resources (wrapper from write_path_mutations.rs:112).
async fn assert_edge(
    pool: &sqlx::PgPool,
    src: ResourceId,
    tgt: ResourceId,
    label: &str,
    weight: f64,
    home: ContextId,
    emitter: EntityId,
) -> EdgeId {
    let mut tx = pool.begin().await.unwrap();
    let id = fire(
        &mut tx,
        SeedAction::RelationshipAssert {
            src,
            tgt,
            kind: EdgeKind::Express,
            polarity: EdgePolarity::Forward,
            label: Some(label),
            weight,
            home: EdgeHome::Context(home),
            emitter,
        },
    )
    .await
    .unwrap()
    .relationship()
    .unwrap();
    tx.commit().await.unwrap();
    id
}

/// The finding's live blocks in `seq` order — the citation grain an audit targets. Same query the
/// write path's own block resolver uses (`writes.rs:355-358`).
async fn live_blocks(pool: &sqlx::PgPool, resource: ResourceId) -> Vec<Uuid> {
    sqlx::query_scalar(
        "SELECT id FROM kb_content_blocks WHERE resource_id=$1 AND NOT is_folded ORDER BY seq",
    )
    .bind(resource.uuid())
    .fetch_all(pool)
    .await
    .unwrap()
}

/// Append a SECOND live block to a resource, citing `sources` — the only production path that makes
/// one finding's blocks plural (`writes::append_block`, mirrored from
/// `streaming_ingest_test.rs:167-186`). A resource `update` cannot do this: it revises the single
/// non-folded body block in place (`writes.rs:353-362` errors on >1 live block), so every source in
/// `seed_finding_with_n_provenance` cites the SAME block. Block multiplicity is exactly what
/// `citation_quality`'s two-stage aggregate exists to survive, so it has to be constructed here.
///
/// The 768-dim placeholder embedding is required, not decorative: `kb_chunks.embedding` is a fixed
/// `vector(768)` column (`streaming_ingest_test.rs:49-51`).
async fn append_second_block(
    pool: &sqlx::PgPool,
    resource: ResourceId,
    emitter: EntityId,
    text: &str,
    sources: Vec<Incorporation>,
) {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(text.as_bytes());
    let chunk = IncomingChunk {
        chunk_index: 0,
        content_hash: format!("{:x}", hasher.finalize()),
        content: text.to_owned(),
        embedding: vec![0.1_f32; 768],
        embedded_with: None,
        header_path: String::new(),
        heading_depth: 0,
    };
    let block = content::prepare_block_from_chunks(1, None, vec![chunk]);
    writes::append_block(
        pool,
        AppendParams {
            resource,
            block: &block,
            sources,
            emitter,
        },
    )
    .await
    .unwrap();
}

/// Record one citation audit through the PRODUCTION Rust write path
/// (`writes::record_citation_audit`, Set 5 Task 2 — `citation_audits.rs:395-406`), never a
/// hand-rolled `INSERT INTO kb_citation_audits`: a fixture that writes the row itself tests the
/// fixture, not the system.
async fn audit(
    pool: &sqlx::PgPool,
    block: Uuid,
    source: ResourceId,
    value: f64,
    emitter: EntityId,
) -> Uuid {
    writes::record_citation_audit(
        pool,
        CitationAuditParams {
            block: BlockId::from(block),
            source: ProvenanceSource::Resource(source.uuid()),
            value,
            reason: Some("test audit"),
            emitter,
        },
    )
    .await
    .unwrap()
}

/// One row of `resource_standing_shape` at the Set-5 return shape.
#[derive(Debug)]
struct Shape {
    citation_magnitude: i32,
    audit_coverage: i32,
    citation_quality: f64,
    contradiction_balance: f64,
    r_parent: f64,
    band: String,
}

/// Read the finding's standing shape as `principal` — through the access gate, which is where the
/// production read goes. `None` = gated out (zero rows), never an error.
async fn shape(pool: &sqlx::PgPool, principal: ProfileId, finding: ResourceId) -> Option<Shape> {
    sqlx::query(
        "SELECT citation_magnitude, audit_coverage, citation_quality, contradiction_balance, \
                r_parent, band \
           FROM resource_standing_shape($1, 'profile', $2)",
    )
    .bind(finding.uuid())
    .bind(principal.uuid())
    .fetch_optional(pool)
    .await
    .unwrap()
    .map(|r| Shape {
        citation_magnitude: r.get("citation_magnitude"),
        audit_coverage: r.get("audit_coverage"),
        citation_quality: r.get("citation_quality"),
        contradiction_balance: r.get("contradiction_balance"),
        r_parent: r.get("r_parent"),
        band: r.get("band"),
    })
}

/// Accrete one more citation of `source` onto the finding's single live body block. Each update is a
/// distinct event, so each lands a distinct `kb_block_provenance` row even for a source already
/// cited — the UNIQUE key includes `contributed_by_event_id`
/// (`20260624000001_canonical_schema.sql:612`). This is how `r_parent` climbs while
/// `citation_magnitude` does not.
async fn cite_again(
    pool: &sqlx::PgPool,
    finding: ResourceId,
    source: ResourceId,
    seq: i32,
    emitter: EntityId,
) {
    writes::update_resource_with(
        pool,
        UpdateParams {
            resource: finding,
            body: Some(&format!("revision {seq}")),
            title: None,
            origin_uri: None,
            properties: &[],
            chunks: None,
            sources: vec![Incorporation {
                source: ProvenanceSource::Resource(source.uuid()),
                seq,
            }],
            content_block: None,
            rehome_to: None,
            emitter,
        },
        EventContext::default(),
    )
    .await
    .unwrap();
}

// ── Task 1 — R_parent ───────────────────────────────────────────────────────────────────────────

#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn r_parent_counts_uncorrected_provenance(pool: sqlx::PgPool) {
    bootseed::seed_system(&pool).await.unwrap();
    let (owner, emitter) = system_actor(&pool).await;
    let home = make_home(&pool, owner, "es-rp").await;
    let (finding, _) = seed_finding_with_n_provenance(&pool, owner, emitter, home, 3).await;

    let r: f64 = sqlx::query_scalar("SELECT resource_r_parent($1)")
        .bind(finding.uuid())
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(
        r, 3.0,
        "r_parent counts uncorrected provenance over the finding's live blocks"
    );
}

// ── contradiction balance ─────────────────────────────────────────────────────────────────────────

#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn contradiction_balance_is_vector_sum(pool: sqlx::PgPool) {
    bootseed::seed_system(&pool).await.unwrap();
    let (owner, emitter) = system_actor(&pool).await;
    let home = make_home(&pool, owner, "es-con").await;
    let (finding, _) = seed_finding_with_n_provenance(&pool, owner, emitter, home, 1).await;
    let t1 = make_resource(&pool, owner, emitter, home, "t1", "temper://es/t1").await;
    let t2 = make_resource(&pool, owner, emitter, home, "t2", "temper://es/t2").await;
    let t3 = make_resource(&pool, owner, emitter, home, "t3", "temper://es/t3").await;

    assert_edge(&pool, finding, t1, "supports", 1.0, home, emitter).await;
    assert_edge(&pool, finding, t2, "supports", 1.0, home, emitter).await;
    assert_edge(&pool, finding, t3, "contradicts", 1.0, home, emitter).await;

    let bal: f64 = sqlx::query_scalar("SELECT resource_contradiction_balance($1)")
        .bind(finding.uuid())
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(
        bal, 1.0,
        "2 supports − 1 contradicts = +1.0 (vector-sum, not a headcount)"
    );
}

// ── refresh parity, gated read ────────────────────────────────────────────────────────────────────

#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn refresh_lands_where_recompute_would(pool: sqlx::PgPool) {
    bootseed::seed_system(&pool).await.unwrap();
    let (owner, emitter) = system_actor(&pool).await;
    let home = make_home(&pool, owner, "es-ref").await;
    let (finding, _) = seed_finding_with_n_provenance(&pool, owner, emitter, home, 2).await;

    sqlx::query("SELECT refresh_resource_standing($1)")
        .bind(finding.uuid())
        .execute(&pool)
        .await
        .unwrap();
    let memo: f64 =
        sqlx::query_scalar("SELECT r_parent FROM kb_resource_standing WHERE finding_id=$1")
            .bind(finding.uuid())
            .fetch_one(&pool)
            .await
            .unwrap();
    let live: f64 = sqlx::query_scalar("SELECT resource_r_parent($1)")
        .bind(finding.uuid())
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(
        memo, live,
        "memoized r_parent == live recompute (refresh lands where recompute would)"
    );

    // spec §1.3 AMEND: no stored band/maturity column on kb_resources.
    let has_band: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM information_schema.columns \
         WHERE table_name='kb_resources' AND column_name IN ('maturity','standing','band'))",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(
        !has_band,
        "spec §1.3 AMEND: standing is never a stored band on kb_resources"
    );
}

#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn shape_read_is_gated_and_carries_band(pool: sqlx::PgPool) {
    bootseed::seed_system(&pool).await.unwrap();
    let (owner, emitter) = system_actor(&pool).await;
    let home = make_home(&pool, owner, "es-shape").await;
    let (finding, _) = seed_finding_with_n_provenance(&pool, owner, emitter, home, 2).await;

    // owner reads: exactly one shape row, band present.
    let rows = sqlx::query("SELECT band, r_parent FROM resource_standing_shape($1, 'profile', $2)")
        .bind(finding.uuid())
        .bind(owner.uuid())
        .fetch_all(&pool)
        .await
        .unwrap();
    assert_eq!(rows.len(), 1, "owner reads its finding's shape");
    let band: String = rows[0].get("band");
    assert!(!band.is_empty(), "the band chip is carried WITH the shape");

    // a non-existent finding is not in the read-set → gated out (0 rows).
    let none = sqlx::query("SELECT band FROM resource_standing_shape($1, 'profile', $2)")
        .bind(Uuid::from_u128(0))
        .bind(owner.uuid())
        .fetch_all(&pool)
        .await
        .unwrap();
    assert!(none.is_empty(), "the gate admits only readable findings");
}

// ── Phase B — Rust wrapper over refresh_resource_standing ───────────────────────────────────────

/// Same intent as `refresh_lands_where_recompute_would` (memo == live recompute), but going through
/// the Rust wrapper `temper_substrate::write::refresh_resource_standing` instead of calling the SQL
/// function directly — the wrapper Task 6's write-path clock will call.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn rust_wrapper_refreshes_standing_memo(pool: sqlx::PgPool) {
    bootseed::seed_system(&pool).await.unwrap();
    let (owner, emitter) = system_actor(&pool).await;
    let home = make_home(&pool, owner, "es-wrap").await;
    let n = 3;
    let (finding, _) = seed_finding_with_n_provenance(&pool, owner, emitter, home, n).await;

    write::refresh_resource_standing(&pool, finding)
        .await
        .unwrap();

    let memo: f64 =
        sqlx::query_scalar("SELECT r_parent FROM kb_resource_standing WHERE finding_id=$1")
            .bind(finding.uuid())
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(
        memo, n as f64,
        "the memo's r_parent counts the seeded provenance"
    );

    let live: f64 = sqlx::query_scalar("SELECT resource_r_parent($1)")
        .bind(finding.uuid())
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(
        memo, live,
        "the Rust wrapper lands the memo exactly where a live recompute would"
    );
}

// ── Task 7 — readback::resource_standing (gated read through Rust) ───────────────────────────────

/// Same intent as `shape_read_is_gated_and_carries_band`, but going through the Rust readback
/// producer `temper_substrate::readback::resource_standing` instead of calling
/// `resource_standing_shape` directly, and gating against a genuinely unrelated second profile
/// (not just a non-existent finding).
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn readback_resource_standing_is_gated_and_carries_band(pool: sqlx::PgPool) {
    bootseed::seed_system(&pool).await.unwrap();
    let (owner, emitter) = system_actor(&pool).await;
    let home = make_home(&pool, owner, "es-readback").await;
    let n = 2;
    let (finding, _bases) = seed_finding_with_n_provenance(&pool, owner, emitter, home, n).await;

    // readable principal (the owner) → Some(row), band non-empty, r_parent matches the seeded
    // provenance count.
    let shape = temper_substrate::readback::resource_standing(&pool, owner, finding)
        .await
        .expect("readable read")
        .expect("owner can read its own finding's standing shape");
    assert_eq!(shape.finding_id, finding);
    assert_eq!(
        shape.r_parent, n as f64,
        "r_parent matches the seeded provenance count"
    );
    // Set 5 Task 3/4 (spec §3.1): the two integer citation axes over the readback shape. `n`
    // distinct live sources were seeded (one per base), none audited, so magnitude == n and
    // coverage == 0.
    assert_eq!(
        shape.citation_magnitude, n as i32,
        "citation_magnitude counts the n distinct live seeded sources"
    );
    assert_eq!(
        shape.audit_coverage, 0,
        "no citation audits were recorded for this finding, so coverage is 0"
    );
    assert!(
        !shape.band.is_empty(),
        "the band chip is carried WITH the shape"
    );

    // an unreadable principal (a second, unrelated profile with no ownership/team access) → None,
    // never an error — the gate is inside the SQL.
    let outsider = ProfileId::from(common::insert_profile(&pool, "es-readback-outsider").await);
    let denied = temper_substrate::readback::resource_standing(&pool, outsider, finding)
        .await
        .expect("gate denial is empty, not an error");
    assert!(
        denied.is_none(),
        "an unrelated profile must not read the finding's standing shape: {denied:?}"
    );
}

/// LOAD-BEARING (`20260727000020`). An in-flight SOURCE is not a citable source.
///
/// `resource_live_citations` gated the source on `is_active` alone, so a segmented upload still
/// arriving counted toward `citation_magnitude` — against CLAUDE.md's rule that
/// "`ingest_state = 'complete'` goes exactly where `r.is_active` already goes".
///
/// `r_parent` IS THE CONTROL, and it is what makes this test discriminating rather than merely
/// green. `resource_r_parent` reads `kb_block_provenance` directly and never calls the shared
/// producer, so it must stay at 2 while `citation_magnitude` falls to 1. A change that suppressed
/// the provenance row itself — or that gated too widely — would move both, and this asserts it does
/// not. Bite probe: drop `src.ingest_state = 'complete'` from `resource_live_citations` and this is
/// the test that fails.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn an_in_progress_source_leaves_provenance_but_not_magnitude(pool: sqlx::PgPool) {
    bootseed::seed_system(&pool).await.unwrap();
    let (owner, emitter) = system_actor(&pool).await;
    let home = make_home(&pool, owner, "es-ingest-gate").await;

    let (finding, bases) = seed_finding_with_n_provenance(&pool, owner, emitter, home, 2).await;

    let before = shape(&pool, owner, finding).await.expect("readable");
    assert_eq!(
        before.citation_magnitude, 2,
        "both seeded sources are complete, so magnitude counts both"
    );
    assert_eq!(before.r_parent, 2.0, "two uncorrected provenance rows");

    // Exactly the state a segmented `begin_segmented_ingest` leaves a source in before
    // `resource_finalize` flips it.
    sqlx::query("UPDATE kb_resources SET ingest_state = 'in_progress' WHERE id = $1")
        .bind(bases[1].uuid())
        .execute(&pool)
        .await
        .unwrap();

    let after = shape(&pool, owner, finding).await.expect("still readable");
    assert_eq!(
        after.citation_magnitude, 1,
        "an in-flight source must not count toward findability: {after:?}"
    );
    assert_eq!(
        after.r_parent, 2.0,
        "r_parent counts provenance rows and does NOT route through resource_live_citations, so \
         the gate must not have moved it: {after:?}"
    );

    // Exclusion is not deletion: `ingest_state` only moves in_progress -> complete, so finalizing
    // returns the source to the axis. This is why gating a monotone axis is safe.
    sqlx::query("UPDATE kb_resources SET ingest_state = 'complete' WHERE id = $1")
        .bind(bases[1].uuid())
        .execute(&pool)
        .await
        .unwrap();

    let refinalized = shape(&pool, owner, finding).await.expect("readable");
    assert_eq!(
        refinalized.citation_magnitude, 2,
        "a finalized source re-enters the axis it was withheld from: {refinalized:?}"
    );
}

// ── Task 4 — readback::is_resource_visible (the extracted, shared predicate) ───────────────────
//
// `is_resource_visible` is the one Rust-callable spelling of `resources_visible_to`, extracted out
// of `ensure_visible` so this read and Task 6's authorization gate ask the identical question. These
// two tests exercise it directly rather than through `resource_standing`, since `resource_standing`'s
// gate runs `resources_readable_by('profile', …)` inside SQL and never calls the extracted Rust
// function at all — a green `resource_standing` test does not prove `is_resource_visible` itself is
// wired correctly.

/// Falsifies a `false`-always stub: the owner of a finding must be reported visible.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn is_resource_visible_true_for_a_readable_finding(pool: sqlx::PgPool) {
    bootseed::seed_system(&pool).await.unwrap();
    let (owner, emitter) = system_actor(&pool).await;
    let home = make_home(&pool, owner, "es-visible-true").await;
    let (finding, _bases) = seed_finding_with_n_provenance(&pool, owner, emitter, home, 1).await;

    let visible = temper_substrate::readback::is_resource_visible(&pool, owner, finding)
        .await
        .expect("visibility check must not fault");
    assert!(visible, "the owner must see its own finding");
}

/// Falsifies a `true`-always stub: an unrelated profile with no ownership/team access to the
/// finding's home must be reported NOT visible.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn is_resource_visible_false_for_an_unreadable_one(pool: sqlx::PgPool) {
    bootseed::seed_system(&pool).await.unwrap();
    let (owner, emitter) = system_actor(&pool).await;
    let home = make_home(&pool, owner, "es-visible-false").await;
    let (finding, _bases) = seed_finding_with_n_provenance(&pool, owner, emitter, home, 1).await;

    let outsider = ProfileId::from(common::insert_profile(&pool, "es-visible-outsider").await);
    let visible = temper_substrate::readback::is_resource_visible(&pool, outsider, finding)
        .await
        .expect("visibility check must not fault");
    assert!(
        !visible,
        "an unrelated profile with no reach into the finding's home must not see it"
    );
}

// ── Set 5 Task 3 — the three citation axes and the re-thresholded band ──────────────────────────
//
// Spec `docs/superpowers/specs/2026-07-23-set5-adversary-citation-audit-design.md` §3.1/§3.2/§4.1,
// migration `20260724000120_standing_citation_components.sql`. Every audit below goes through the
// production write path (`writes::record_citation_audit`), never a hand-rolled INSERT.

/// LOAD-BEARING. Falsifies the collapse spec §3.1's mapping table names in as many words — "an
/// implementer who collapses them reintroduces the actor-count fallacy", of `r_parent` into
/// `citation_magnitude`. Ten citations of ONE source must read `r_parent = 10`,
/// `citation_magnitude = 1`.
///
/// Also falsifies the missing liveness join: soft-delete only flips `kb_resources.is_active` and
/// does NOT fold blocks or provenance (`_project_resource_deleted`,
/// `20260624000002_canonical_functions.sql:1051-1061`), so a producer without
/// `JOIN kb_resources src ON src.id = p.source_id AND src.is_active` keeps a deleted source
/// conferring standing forever. `r_parent` deliberately does NOT change — it is unamended and has
/// no liveness join, which is precisely why the two numbers diverge here.
///
/// The magnitude-0 state at the end is also the band's divide-by-zero probe: the coverage ratio
/// `audit_coverage / citation_magnitude` must not raise, and must land `provisional`.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn magnitude_counts_distinct_live_sources_not_provenance_rows(pool: sqlx::PgPool) {
    bootseed::seed_system(&pool).await.unwrap();
    let (owner, emitter) = system_actor(&pool).await;
    let home = make_home(&pool, owner, "es-mag").await;
    let (finding, bases) = seed_finding_with_n_provenance(&pool, owner, emitter, home, 1).await;
    let source = bases[0];

    // nine more citations of the SAME source → ten provenance rows, one distinct source.
    for seq in 1..10 {
        cite_again(&pool, finding, source, seq, emitter).await;
    }

    let s = shape(&pool, owner, finding).await.expect("owner reads");
    assert_eq!(
        s.r_parent, 10.0,
        "r_parent counts every uncorrected provenance row — the echo included"
    );
    assert_eq!(
        s.citation_magnitude, 1,
        "magnitude counts DISTINCT sources: ten citations of one source is ONE source"
    );
    assert_eq!(s.audit_coverage, 0, "nothing audited yet");

    // soft-delete the source: is_active flips, blocks/provenance are untouched.
    writes::delete_resource(&pool, source, emitter)
        .await
        .unwrap();

    let s = shape(&pool, owner, finding).await.expect("owner reads");
    assert_eq!(
        s.citation_magnitude, 0,
        "a soft-deleted source stops conferring magnitude — the liveness join is not optional"
    );
    assert_eq!(
        s.r_parent, 10.0,
        "r_parent is unamended and carries no liveness join: it still counts the ten rows"
    );
    assert_eq!(
        s.band, "provisional",
        "magnitude 0 must land provisional through the ratio guard, not raise divide-by-zero"
    );
}

/// LOAD-BEARING. Falsifies the naive single-stage aggregate — the exact echo/actor-count fallacy
/// spec §3.1 warns re-enters "through block multiplicity, in the very function written to exclude
/// it." Source A is cited by TWO of the finding's blocks and audited on each (`+1.0`, `-1.0`);
/// source B is cited once and audited `+1.0`.
///
/// Two-stage (correct): A collapses to ~0.0 within itself, B is +1.0, mean over the two distinct
/// audited sources = **0.5**.
/// Naive flat weighted mean over the three joined audit rows: `(1 - 1 + 1) / 3` = **0.333…** — A
/// votes twice because it is cited twice. The 0.45 floor below is what separates them.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn a_source_cited_by_two_blocks_counts_once_in_quality(pool: sqlx::PgPool) {
    bootseed::seed_system(&pool).await.unwrap();
    let (owner, emitter) = system_actor(&pool).await;
    let home = make_home(&pool, owner, "es-two-block").await;
    let a = make_resource(&pool, owner, emitter, home, "srcA", "temper://es/q-a").await;
    let b = make_resource(&pool, owner, emitter, home, "srcB", "temper://es/q-b").await;

    let finding = writes::create_resource_with(
        &pool,
        CreateParams {
            idempotency_key: None,
            title: "finding",
            origin_uri: "temper://es/q-finding",
            body: "the claim under standing",
            doc_type: "research",
            home: AnchorRef::context(home),
            owner,
            originator: owner,
            emitter,
            properties: &[],
            chunks: None,
            sources: vec![
                Incorporation {
                    source: ProvenanceSource::Resource(a.uuid()),
                    seq: 0,
                },
                Incorporation {
                    source: ProvenanceSource::Resource(b.uuid()),
                    seq: 1,
                },
            ],
        },
        EventContext::default(),
    )
    .await
    .unwrap();
    append_second_block(
        &pool,
        finding,
        emitter,
        "a second segment, also distilled from source A",
        vec![Incorporation {
            source: ProvenanceSource::Resource(a.uuid()),
            seq: 0,
        }],
    )
    .await;

    let blocks = live_blocks(&pool, finding).await;
    assert_eq!(
        blocks.len(),
        2,
        "the finding must genuinely have two blocks"
    );

    audit(&pool, blocks[0], a, 1.0, emitter).await;
    audit(&pool, blocks[1], a, -1.0, emitter).await;
    audit(&pool, blocks[0], b, 1.0, emitter).await;

    let s = shape(&pool, owner, finding).await.expect("owner reads");
    assert_eq!(s.citation_magnitude, 2, "two distinct live sources");
    assert_eq!(
        s.audit_coverage, 2,
        "coverage is per distinct source: A counts once despite being audited on two blocks"
    );
    assert!(
        (s.citation_quality - 0.5).abs() < 1e-6,
        "two-stage: A collapses to ~0 within itself, B is +1, mean = 0.5 (got {})",
        s.citation_quality
    );
    assert!(
        s.citation_quality > 0.45,
        "a naive flat mean over the three audit rows gives (1-1+1)/3 = 0.333 — A voting twice \
         because two blocks cite it. Got {}",
        s.citation_quality
    );
}

/// LOAD-BEARING (regression). Falsifies the perverse gradient spec §3.2 rejects: an earlier draft
/// gave unaudited citations a `-0.5` contribution to the mean, so adding good-faith evidence
/// *destroyed a verdict already earned*. Quality is computed over the AUDITED SUBSET ONLY, so
/// adding four unaudited sources must leave it at exactly `+1.0`, and the finding must never read
/// `disputed` (which would say the adversary examined it and it failed).
///
/// The BAND does move, and that is the design, not a regression: unaudited-ness lives on the
/// coverage axis (spec §3.1 — "citing more sources lowers the ratio and re-enters it into the
/// auditor's queue, without ever destroying a verdict already earned"). What §3.2 forbids is the
/// *quality* falling, and that is what this asserts. See the task report for why "stays its band"
/// is unsatisfiable under the specified arms.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn adding_unaudited_citations_does_not_demote_the_quality_axis(pool: sqlx::PgPool) {
    bootseed::seed_system(&pool).await.unwrap();
    let (owner, emitter) = system_actor(&pool).await;
    let home = make_home(&pool, owner, "es-perverse").await;
    let (finding, bases) = seed_finding_with_n_provenance(&pool, owner, emitter, home, 2).await;
    let block = live_blocks(&pool, finding).await[0];

    audit(&pool, block, bases[0], 1.0, emitter).await;
    audit(&pool, block, bases[1], 1.0, emitter).await;

    let before = shape(&pool, owner, finding).await.expect("owner reads");
    assert_eq!(before.citation_quality, 1.0);
    assert_eq!(before.band, "near-canonical");

    for seq in 2..6 {
        let extra = make_resource(
            &pool,
            owner,
            emitter,
            home,
            &format!("extra{seq}"),
            &format!("temper://es/extra{seq}"),
        )
        .await;
        cite_again(&pool, finding, extra, seq, emitter).await;
    }

    let after = shape(&pool, owner, finding).await.expect("owner reads");
    assert_eq!(
        after.citation_quality, 1.0,
        "adding unaudited evidence must NOT pull the earned verdict down (spec §3.2)"
    );
    assert_eq!(after.citation_magnitude, 6, "magnitude moved");
    assert_eq!(after.audit_coverage, 2, "coverage did not");
    assert_ne!(
        after.band, "disputed",
        "unaudited evidence is not adverse evidence — never 'disputed'"
    );
    assert_eq!(
        after.band, "provisional",
        "the coverage ratio (2/6) drops it to the floor, which is what re-enters it into the \
         auditor's queue — the axis that moved is coverage, not quality"
    );
}

/// Falsifies "a large citation set alone can promote." Magnitude 5, coverage 0: the finding is
/// well-connected in the graph and nobody has weighed any of it, so it sits at the floor with a
/// neutral (not negative) quality — spec §3.1's "not from a poisoned mean."
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn unaudited_finding_is_provisional_regardless_of_magnitude(pool: sqlx::PgPool) {
    bootseed::seed_system(&pool).await.unwrap();
    let (owner, emitter) = system_actor(&pool).await;
    let home = make_home(&pool, owner, "es-unaudited").await;
    let (finding, _bases) = seed_finding_with_n_provenance(&pool, owner, emitter, home, 5).await;

    let s = shape(&pool, owner, finding).await.expect("owner reads");
    assert_eq!(s.citation_magnitude, 5);
    assert_eq!(s.audit_coverage, 0);
    assert_eq!(
        s.citation_quality, 0.0,
        "an unaudited finding makes no quality claim — neutral 0.0, not a negative prior"
    );
    assert_eq!(s.band, "provisional");
}

/// LOAD-BEARING. Falsifies the three-arm band — the arm that keeps "never evaluated" and "evaluated
/// and found wanting" from being flattened together (spec §3.1, §1 on its negative side). Without
/// the fourth arm this finding reads `provisional`, indistinguishable from one nobody has looked at.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn a_negative_audit_yields_disputed(pool: sqlx::PgPool) {
    bootseed::seed_system(&pool).await.unwrap();
    let (owner, emitter) = system_actor(&pool).await;
    let home = make_home(&pool, owner, "es-disputed").await;
    let (finding, bases) = seed_finding_with_n_provenance(&pool, owner, emitter, home, 1).await;
    let block = live_blocks(&pool, finding).await[0];

    audit(&pool, block, bases[0], -1.0, emitter).await;

    let s = shape(&pool, owner, finding).await.expect("owner reads");
    assert_eq!(s.audit_coverage, 1, "it WAS evaluated");
    assert_eq!(s.citation_quality, -1.0);
    assert_eq!(
        s.band, "disputed",
        "coverage > 0 with negative quality is 'disputed', distinct from the unaudited floor"
    );
}

/// LOAD-BEARING. **The `quality == 0.0` boundary.** Two sources, both audited, verdicts that
/// exactly cancel: `coverage == magnitude == 2`, `quality == 0.0`. Under the strict `< 0.0` arm this
/// fell through every band to `provisional` — byte-identical to a finding nobody has ever opened —
/// and `audit_drift_sweep` selects on `coverage < magnitude`, so it could never be re-queued to
/// escape. That state is ordinary, not a corner: `0.0` is an explicit anchor on the auditor's own
/// scale and the persona is told to prefer it when a citation is undecidable.
///
/// Asserts the two preconditions first (`coverage == magnitude`, `quality == 0.0`) so the band
/// assertion cannot pass for some other reason — and change the arm back to `< 0.0` and only the
/// band line reds.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn cancelling_verdicts_read_disputed_not_provisional(pool: sqlx::PgPool) {
    bootseed::seed_system(&pool).await.unwrap();
    let (owner, emitter) = system_actor(&pool).await;
    let home = make_home(&pool, owner, "es-net-zero").await;
    let (finding, bases) = seed_finding_with_n_provenance(&pool, owner, emitter, home, 2).await;
    let block = live_blocks(&pool, finding).await[0];

    audit(&pool, block, bases[0], 1.0, emitter).await;
    audit(&pool, block, bases[1], -1.0, emitter).await;

    let s = shape(&pool, owner, finding).await.expect("owner reads");
    assert_eq!(s.citation_magnitude, 2);
    assert_eq!(
        s.audit_coverage, 2,
        "EVERY source was evaluated — this is the opposite of unaudited"
    );
    assert!(
        s.citation_quality.abs() < 1e-9,
        "the two verdicts cancel: expected ~0.0, got {}",
        s.citation_quality
    );
    assert_eq!(
        s.band, "disputed",
        "'we looked and it did not hold up' must not be flattened into 'nobody tried' — and the \
         sweep can never re-queue this finding to correct it"
    );
}

/// Falsifies "the aggregate is a latest-wins column" and "decay erases the trail." An old `-1.0`
/// and a recent `+1.0` of the same citation both remain permanent rows (append-only, spec §4.1 —
/// "a later +1.0 never erases an earlier -1.0"), while the decay-weighted projection reads
/// positive because the recent verdict weighs more. The back-dated `created` is the only way to
/// exercise decay without sleeping; `kb_citation_audits` carries no append-only trigger.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn recent_audit_outweighs_older_opposite(pool: sqlx::PgPool) {
    bootseed::seed_system(&pool).await.unwrap();
    let (owner, emitter) = system_actor(&pool).await;
    let home = make_home(&pool, owner, "es-decay").await;
    let (finding, bases) = seed_finding_with_n_provenance(&pool, owner, emitter, home, 1).await;
    let block = live_blocks(&pool, finding).await[0];

    let old = audit(&pool, block, bases[0], -1.0, emitter).await;
    sqlx::query("UPDATE kb_citation_audits SET created = now() - interval '365 days' WHERE id=$1")
        .bind(old)
        .execute(&pool)
        .await
        .unwrap();
    audit(&pool, block, bases[0], 1.0, emitter).await;

    let s = shape(&pool, owner, finding).await.expect("owner reads");
    assert!(
        s.citation_quality > 0.0,
        "a year-old -1.0 has faded to ~2e-4 of the weight of today's +1.0 (30-day half-life), so \
         the projection reads positive (got {})",
        s.citation_quality
    );

    let rows: i64 = sqlx::query_scalar("SELECT count(*) FROM kb_citation_audits WHERE block_id=$1")
        .bind(block)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(
        rows, 2,
        "both audits persist — decay changes influence, never the ledger"
    );
}

/// The access gate at the new return shape. The `gated` CTE over `resources_readable_by` is carried
/// byte-for-byte from the shipped function (`20260721000010:239-242`); a changed return type must
/// not quietly change what leaks. An unrelated profile gets ZERO rows, never an error.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn standing_shape_returns_none_for_an_unreadable_finding(pool: sqlx::PgPool) {
    bootseed::seed_system(&pool).await.unwrap();
    let (owner, emitter) = system_actor(&pool).await;
    let home = make_home(&pool, owner, "es-gate5").await;
    let (finding, _bases) = seed_finding_with_n_provenance(&pool, owner, emitter, home, 2).await;

    let mine = shape(&pool, owner, finding).await.expect("owner reads");
    assert_eq!(mine.citation_magnitude, 2);
    assert_eq!(mine.contradiction_balance, 0.0);
    assert!(!mine.band.is_empty(), "the band chip rides WITH the shape");

    let outsider = ProfileId::from(common::insert_profile(&pool, "es-gate5-outsider").await);
    assert!(
        shape(&pool, outsider, finding).await.is_none(),
        "an unrelated profile must read zero rows at the new return shape too"
    );
}

/// LOAD-BEARING. Falsifies "the top band is unreachable without repeated rounds or a `supports`-edge
/// writer" — the recalibration spec §3.1 demands. Two distinct live resource-kind citations, both
/// audited positively in ONE sequence, no `supports` edge (nothing writes those), so
/// `contradiction_balance` stays at its `0.0` default and satisfies the `>= 0.0` conjunct.
///
/// If this test cannot pass, the thresholds are wrong, not the test.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn near_canonical_is_reachable_in_one_pass(pool: sqlx::PgPool) {
    bootseed::seed_system(&pool).await.unwrap();
    let (owner, emitter) = system_actor(&pool).await;
    let home = make_home(&pool, owner, "es-onepass").await;
    let (finding, bases) = seed_finding_with_n_provenance(&pool, owner, emitter, home, 2).await;
    let block = live_blocks(&pool, finding).await[0];

    audit(&pool, block, bases[0], 1.0, emitter).await;
    audit(&pool, block, bases[1], 1.0, emitter).await;

    let s = shape(&pool, owner, finding).await.expect("owner reads");
    assert_eq!(s.citation_magnitude, 2, "the Landmesser line, exactly");
    assert_eq!(
        s.audit_coverage, 2,
        "one thorough pass audits every cited source, reaching full coverage at once"
    );
    assert_eq!(
        s.contradiction_balance, 0.0,
        "no supports-edge writer exists; the top band must not depend on one"
    );
    assert_eq!(
        s.band, "near-canonical",
        "magnitude 2 + full coverage + positive quality reaches the top band in one pass"
    );
}

/// LOAD-BEARING. **The cross-finding leak.** One source S is cited by two DIFFERENT findings; an
/// auditor audits S on F1 only. F2 must be completely untouched — `coverage 0`, `quality 0.0`,
/// `provisional` — because an audit is keyed on the CITATION (the `(block, source)` pair), not on the
/// source.
///
/// This is the probe the rest of the quality suite could not supply. Every other test here audits
/// within one finding, so deleting `AND a.block_id = c.block_id` from the quality join
/// (`…020…sql`) and from the coverage `EXISTS` leaves them all green — hand-checked on
/// `a_source_cited_by_two_blocks_counts_once_in_quality`, whose 0.5 is unchanged by the deletion
/// because both of A's citing rows then join both of A's audits. The real consequence is here: with
/// the conjunct gone, F2 reads `coverage 1, quality -1.0 → disputed` with nobody having audited F2 at
/// all — one principal's verdict silently condemning another's finding for sharing a source.
///
/// The F1 half is asserted first and is not decoration: it proves the audit landed and is being read,
/// so F2's zeros cannot be "nothing works".
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn an_audit_on_one_finding_does_not_cover_the_same_source_on_another(pool: sqlx::PgPool) {
    bootseed::seed_system(&pool).await.unwrap();
    let (owner, emitter) = system_actor(&pool).await;
    let home = make_home(&pool, owner, "es-shared-src").await;
    let source = make_resource(&pool, owner, emitter, home, "shared", "temper://es/shared").await;

    // Two findings, each citing the SAME source at create time.
    let mut findings = Vec::new();
    for tag in ["f1", "f2"] {
        findings.push(
            writes::create_resource_with(
                &pool,
                CreateParams {
                    idempotency_key: None,
                    title: tag,
                    origin_uri: &format!("temper://es/shared-{tag}"),
                    body: "a claim resting on the shared source",
                    doc_type: "research",
                    home: AnchorRef::context(home),
                    owner,
                    originator: owner,
                    emitter,
                    properties: &[],
                    chunks: None,
                    sources: vec![Incorporation {
                        source: ProvenanceSource::Resource(source.uuid()),
                        seq: 0,
                    }],
                },
                EventContext::default(),
            )
            .await
            .unwrap(),
        );
    }
    let (f1, f2) = (findings[0], findings[1]);

    audit(
        &pool,
        live_blocks(&pool, f1).await[0],
        source,
        -1.0,
        emitter,
    )
    .await;

    let s1 = shape(&pool, owner, f1).await.expect("owner reads f1");
    assert_eq!(s1.audit_coverage, 1, "f1's citation WAS audited");
    assert_eq!(s1.citation_quality, -1.0);
    assert_eq!(s1.band, "disputed");

    let s2 = shape(&pool, owner, f2).await.expect("owner reads f2");
    assert_eq!(
        s2.citation_magnitude, 1,
        "f2 genuinely cites the same source — that is what makes the leak possible"
    );
    assert_eq!(
        s2.audit_coverage, 0,
        "nobody audited f2's citation of it: an audit is keyed on the (block, source) pair, not on \
         the source"
    );
    assert_eq!(
        s2.citation_quality, 0.0,
        "and no verdict from another finding may reach f2's mean"
    );
    assert_eq!(
        s2.band, "provisional",
        "f2 is unaudited, not disputed — a shared source must not carry a neighbour's verdict"
    );
}

/// LOAD-BEARING. **The band's cut points and its `contradiction_balance` conjuncts**, exercised
/// against `standing_band` directly with literal arguments.
///
/// Direct, not end-to-end, and that is the point: every other band test in this file drives quality
/// to `±1.0` or `0.0` through real audits, so nothing sits near the `> 0.3` cut and no test ever
/// supplies a negative balance. Both were unfalsified — `> 0.3` could become `>= 0.3` or `> 0.0`,
/// and `AND p_contradiction_balance >= 0.0` could be deleted from arms 1 and 2, with the whole suite
/// still green. Reaching those inputs through fixtures is not possible *exactly*: a per-source mean
/// is `(w·v)/w`, which is `v` only up to a rounding step, so an end-to-end probe pinned at the `0.3`
/// boundary would flip on a 1-ulp wobble. `standing_band` is `IMMUTABLE` and total, so calling it
/// with literals is both exact and the honest grain for an arm test.
///
/// The `0.3` cut is not arbitrary — it is co-calibrated with the auditor persona's published scale
/// (`subagents/auditor/instructions.md`: `+0.3` = "partial" ⇒ **not** near-canonical, `+0.6` =
/// "sound" ⇒ near-canonical). That coupling was asserted in prose and nowhere in code.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn band_arms_hold_at_their_boundaries(pool: sqlx::PgPool) {
    // `freshness` is accepted by the signature and DELIBERATELY not gated on, so it is pinned at a
    // constant here — a band that started reading it would not change any expectation below.
    async fn band(
        pool: &sqlx::PgPool,
        magnitude: i32,
        coverage: i32,
        quality: f64,
        balance: f64,
    ) -> String {
        sqlx::query_scalar("SELECT standing_band($1, $2, $3, $4, $5)")
            .bind(magnitude)
            .bind(coverage)
            .bind(quality)
            .bind(balance)
            .bind(1.0_f64)
            .fetch_one(pool)
            .await
            .unwrap()
    }

    // The `> 0.3` cut, from both sides.
    assert_eq!(
        band(&pool, 2, 2, 0.3, 0.0).await,
        "reinforced",
        "the cut is STRICT: an all-partial pass (every source at the persona's +0.3 anchor) must \
         not reach the top band. `>= 0.3` or `> 0.0` would."
    );
    assert_eq!(
        band(&pool, 2, 2, 0.6, 0.0).await,
        "near-canonical",
        "an all-sound pass (+0.6) must reach it — raising the cut past 0.6 silently retires the \
         top band for the distribution the persona actually emits"
    );
    assert_eq!(
        band(&pool, 2, 2, 0.31, 0.0).await,
        "near-canonical",
        "and the cut is 0.3, not something between 0.3 and 0.6"
    );

    // `AND p_contradiction_balance >= 0.0` on arms 1 and 2, and the arm that now catches what they
    // refuse. Delete either conjunct and the first two of these read 'near-canonical'/'reinforced'.
    assert_eq!(
        band(&pool, 2, 2, 1.0, -1.0).await,
        "disputed",
        "a live contradiction blocks the top band"
    );
    assert_eq!(
        band(&pool, 1, 1, 1.0, -1.0).await,
        "disputed",
        "the same conjunct on arm 2: a contradicted, audited finding is not 'reinforced'"
    );
    assert_eq!(
        band(&pool, 3, 3, 0.9, -1.0).await,
        "disputed",
        "fully covered, strongly positive, one contradicts edge — this fell through every arm to \
         'provisional' before, indistinguishable from a finding nobody opened AND terminal, since \
         coverage == magnitude keeps it out of audit_drift_sweep"
    );
    assert_eq!(
        band(&pool, 3, 0, 0.0, -1.0).await,
        "provisional",
        "but an UNAUDITED contradicted finding stays at the floor: nobody looked, which is what \
         the floor means"
    );

    // The coverage ratio and its divide guard.
    assert_eq!(
        band(&pool, 2, 1, 1.0, 0.0).await,
        "reinforced",
        "the ratio admits exactly 0.5 — `>= 0.5`, not `> 0.5`"
    );
    assert_eq!(
        band(&pool, 3, 1, 1.0, 0.0).await,
        "provisional",
        "and refuses 1/3: below half-covered is the floor however good the audited third is"
    );
    assert_eq!(
        band(&pool, 0, 0, 0.0, 0.0).await,
        "provisional",
        "magnitude 0 divides by nothing and falls through the guard to the floor"
    );
}

/// LOAD-BEARING. Falsifies dropping the magnitude floor to 1. A single source audited `+1.0` has
/// perfect coverage and perfect quality and STILL must not be near-canonical — one source is not
/// diverse evidence (spec §3.1's Landmesser line: "a lone source can never be near-canonical no
/// matter how well-audited — that is the whole point"). It lands `reinforced`.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn one_source_never_reaches_near_canonical(pool: sqlx::PgPool) {
    bootseed::seed_system(&pool).await.unwrap();
    let (owner, emitter) = system_actor(&pool).await;
    let home = make_home(&pool, owner, "es-lone").await;
    let (finding, bases) = seed_finding_with_n_provenance(&pool, owner, emitter, home, 1).await;
    let block = live_blocks(&pool, finding).await[0];

    audit(&pool, block, bases[0], 1.0, emitter).await;

    let s = shape(&pool, owner, finding).await.expect("owner reads");
    assert_eq!(s.citation_magnitude, 1);
    assert_eq!(s.audit_coverage, 1, "coverage ratio is a perfect 1.0");
    assert_eq!(s.citation_quality, 1.0, "quality is the maximum");
    assert_eq!(
        s.band, "reinforced",
        "a perfectly-audited lone source is reinforced, never near-canonical"
    );
}
