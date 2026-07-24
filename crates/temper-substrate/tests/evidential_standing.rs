#![cfg(feature = "artifact-tests")]
//! Evidential-standing maturity projection (SQL substrate).
//!
//! Exercises the producers/memos in `migrations/20260721000010_evidential_standing_memo.sql` as
//! amended by `migrations/20260723000020_standing_citation_components.sql` (Set 5, Task 3) against
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

// ── Set 5 Task 3 — the three citation axes and the re-thresholded band ──────────────────────────
//
// Spec `docs/superpowers/specs/2026-07-23-set5-adversary-citation-audit-design.md` §3.1/§3.2/§4.1,
// migration `20260723000020_standing_citation_components.sql`. Every audit below goes through the
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
