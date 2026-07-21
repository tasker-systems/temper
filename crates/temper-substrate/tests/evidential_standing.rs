#![cfg(feature = "artifact-tests")]
//! Set 3 — evidential-standing maturity projection, Phase A (SQL substrate).
//!
//! Exercises the producers/memos in `migrations/20260721000010_evidential_standing_memo.sql`
//! against an ephemeral DB. Standing is NOT truth (spec 019f81e8 preamble): these assert the
//! *shape of the evidence*, never a truth claim. Grounding for the seeding helpers:
//! `content_mutation.rs` (provenance via `writes`) and `write_path_mutations.rs` (edges via
//! `SeedAction::RelationshipAssert`).

mod common;

use sqlx::Row;
use temper_substrate::affinity::EdgeKind;
use temper_substrate::events::{fire, EdgeHome, EventContext, SeedAction};
use temper_substrate::ids::{ContextId, EdgeId, EntityId, ProfileId, ResourceId};
use temper_substrate::payloads::{AnchorRef, EdgePolarity, Incorporation, ProvenanceSource};
use temper_substrate::scenario::bootseed;
use temper_substrate::write;
use temper_substrate::writes::{self, CreateParams, UpdateParams};
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

// ── Task 2 — independence memo + silence default ──────────────────────────────────────────────────

#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn silence_default_is_correlated(pool: sqlx::PgPool) {
    bootseed::seed_system(&pool).await.unwrap();
    let (owner, emitter) = system_actor(&pool).await;
    let home = make_home(&pool, owner, "es-sil").await;
    let (finding, _bases) = seed_finding_with_n_provenance(&pool, owner, emitter, home, 3).await;

    sqlx::query("SELECT refresh_independence_pairs($1)")
        .bind(finding.uuid())
        .execute(&pool)
        .await
        .unwrap();
    let breadth: f64 = sqlx::query_scalar("SELECT resource_independence_breadth($1)")
        .bind(finding.uuid())
        .fetch_one(&pool)
        .await
        .unwrap();
    assert!(
        breadth <= 1.0,
        "silence default: 3 unasserted bases are one correlated cluster, not 3 (got {breadth})"
    );
}

#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn affirmed_independence_raises_breadth(pool: sqlx::PgPool) {
    bootseed::seed_system(&pool).await.unwrap();
    let (owner, emitter) = system_actor(&pool).await;
    let home = make_home(&pool, owner, "es-ind").await;
    let (finding, bases) = seed_finding_with_n_provenance(&pool, owner, emitter, home, 3).await;

    assert_edge(
        &pool,
        bases[0],
        bases[1],
        "independent-of",
        1.0,
        home,
        emitter,
    )
    .await;
    sqlx::query("SELECT refresh_independence_pairs($1)")
        .bind(finding.uuid())
        .execute(&pool)
        .await
        .unwrap();
    let breadth: f64 = sqlx::query_scalar("SELECT resource_independence_breadth($1)")
        .bind(finding.uuid())
        .fetch_one(&pool)
        .await
        .unwrap();
    assert!(
        breadth > 1.0,
        "one affirmed independent pair raises effective independent rank above 1 (got {breadth})"
    );
}

// ── Task 3 — contradiction balance + adversarial survival ─────────────────────────────────────────

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

#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn zero_challenges_is_not_survival(pool: sqlx::PgPool) {
    bootseed::seed_system(&pool).await.unwrap();
    let (owner, emitter) = system_actor(&pool).await;
    let home = make_home(&pool, owner, "es-adv").await;
    let (finding, _) = seed_finding_with_n_provenance(&pool, owner, emitter, home, 1).await;

    let row =
        sqlx::query("SELECT challenge_count, survived FROM resource_adversarial_survival($1)")
            .bind(finding.uuid())
            .fetch_one(&pool)
            .await
            .unwrap();
    let c: i32 = row.get("challenge_count");
    let s: f64 = row.get("survived");
    assert_eq!(
        (c, s),
        (0, 0.0),
        "no challenges: 0 count, 0 survival — distinct from N-withstood"
    );
}

// ── Task 4 — band, refresh parity, gated read ─────────────────────────────────────────────────────

#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn band_is_read_time_over_components(pool: sqlx::PgPool) {
    bootseed::seed_system(&pool).await.unwrap();

    let provisional: String = sqlx::query_scalar("SELECT standing_band(1.0, 0, 0.0, -2.0, 0.5)")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(provisional, "provisional");

    let reinforced: String = sqlx::query_scalar("SELECT standing_band(2.0, 0, 0.0, 0.0, 0.5)")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(reinforced, "reinforced");

    let near: String = sqlx::query_scalar("SELECT standing_band(4.0, 2, 3.0, 5.0, 0.9)")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(near, "near-canonical");
}

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
