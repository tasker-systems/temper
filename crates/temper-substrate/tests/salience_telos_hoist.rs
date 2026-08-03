#![cfg(feature = "artifact-tests")]
//! Hole 6 of goal `019fc46c-5731-75d2-b00f-891069ce6f31`: the salience clock evaluated **one
//! anchor-level value once per region**.
//!
//! ## What was broken
//!
//! `refresh_salience` statement 1 called `anchor_region_telos_alignment(r.id, …)` per row. That
//! function reads the region's *stored* centroid — so the cosine really is cheap, exactly as
//! `region_clocks.rs` claims — but it computes the anchor's telos inside a per-region
//! `CROSS JOIN LATERAL`, and `anchor_telos_embedding` depends only on `(anchor, lens)`. It is the
//! same value for every region of the anchor, and it was evaluated once per region.
//!
//! Measured against production 2026-08-03 on the `temper` context's 429 live regions: **23.186s**
//! for the per-region shape versus **0.027s** for one telos evaluation plus 429 stored-centroid
//! cosines. ~860×, same values. One `anchor_telos_embedding` call costs 26.4ms.
//!
//! ## What this is NOT
//!
//! Not a `projection-cost-tracks-what-changed` violation. When the telos drifts, every region's
//! alignment against it genuinely *is* stale, so recomputing all of them is **correct**. The defect
//! is the repeated evaluation of one anchor-level value — an N+1 of the same family as latency-pass
//! Finding B. That distinction is why the fix hoists an expression rather than tracking which
//! regions moved.
//!
//! ## How the count is observed
//!
//! `anchor_telos_embedding` is renamed aside and replaced with a counting shim that delegates to it.
//! The shim is deliberately **VOLATILE**: a `STABLE` shim could be hoisted by the planner even on
//! the per-region shape, which would make the pre-fix count 1 and the bite vanish. Volatile
//! guarantees the old shape counts once per region, so this test fails against it — verified.
//! Each `#[sqlx::test]` owns an isolated database, so replacing a shipped function is contained.

mod common;

use common::context_fixture::{rehome_corpus_into_context, seed_goal, seed_task};

use sqlx::PgPool;
use temper_core::types::home::HomeAnchor;
use temper_core::types::ids::ContextId;
use temper_substrate::ids::EntityId;
use temper_substrate::scenario::{bootseed, loader, model::Seed};
use temper_substrate::{embed, write};

const L0_KERNEL_SEED: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/seeds/l0-kernel.yaml"
);

fn seed() -> Seed {
    serde_yaml::from_str(&std::fs::read_to_string(L0_KERNEL_SEED).unwrap()).unwrap()
}

/// Rename `anchor_telos_embedding` aside and install a counting shim over it.
async fn install_telos_counter(pool: &PgPool) {
    sqlx::query("CREATE TABLE telos_eval_log (at timestamptz NOT NULL DEFAULT clock_timestamp())")
        .execute(pool)
        .await
        .expect("counter table");
    sqlx::query(
        "ALTER FUNCTION anchor_telos_embedding(varchar, uuid, uuid) \
           RENAME TO anchor_telos_embedding_real",
    )
    .execute(pool)
    .await
    .expect("rename original");
    // VOLATILE on purpose — see the module docs. A STABLE shim can be hoisted by the planner and
    // would under-count the very shape this test exists to catch.
    sqlx::query(
        "CREATE FUNCTION anchor_telos_embedding(p_anchor_table varchar, p_anchor_id uuid, \
                                                p_lens uuid) \
         RETURNS vector LANGUAGE plpgsql VOLATILE AS $fn$ \
         BEGIN \
           INSERT INTO telos_eval_log DEFAULT VALUES; \
           RETURN anchor_telos_embedding_real(p_anchor_table, p_anchor_id, p_lens); \
         END $fn$",
    )
    .execute(pool)
    .await
    .expect("counting shim");
}

async fn telos_eval_count(pool: &PgPool) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM telos_eval_log")
        .fetch_one(pool)
        .await
        .expect("count")
}

async fn reset_telos_counter(pool: &PgPool) {
    sqlx::query("DELETE FROM telos_eval_log")
        .execute(pool)
        .await
        .expect("reset");
}

async fn live_region_count(pool: &PgPool, ctx: uuid::Uuid) -> i64 {
    sqlx::query_scalar(
        "SELECT count(*) FROM kb_cogmap_regions \
         WHERE home_anchor_table='kb_contexts' AND home_anchor_id=$1 AND NOT is_folded",
    )
    .bind(ctx)
    .fetch_one(pool)
    .await
    .expect("live regions")
}

/// Build a context carrying several live regions and a rotating telos.
async fn context_with_regions(pool: &PgPool) -> (uuid::Uuid, HomeAnchor, EntityId) {
    bootseed::seed_system(pool).await.unwrap();
    let loaded = loader::load_seed(pool, &seed()).await.unwrap();
    embed::embed_chunks(pool).await.unwrap();

    let ctx = common::insert_context(pool, "kb_profiles", loaded.owner, "hoist-ctx", "Hoist Ctx")
        .await
        .expect("context");
    rehome_corpus_into_context(pool, ctx, loaded.cogmap).await;

    let mut va = vec![0.0f32; 768];
    va[0] = 1.0;
    let mut vb = vec![0.0f32; 768];
    vb[1] = 1.0;
    let goal_a = seed_goal(pool, ctx, loaded.owner, "Goal A", va).await;
    let goal_b = seed_goal(pool, ctx, loaded.owner, "Goal B", vb).await;
    seed_task(pool, ctx, loaded.owner, goal_a, "in-progress").await;
    seed_task(pool, ctx, loaded.owner, goal_b, "in-progress").await;

    let anchor = HomeAnchor::Context(ContextId::from(ctx));
    let emitter = EntityId::from(loaded.emitter);
    write::materialize(pool, anchor, "workflow-default", emitter)
        .await
        .expect("materialize");
    (ctx, anchor, emitter)
}

/// **The bite.** One `refresh_salience` must evaluate the anchor's telos exactly once, however many
/// regions the anchor carries.
///
/// Against the per-region shape this fails with `left: <region count>, right: 1` — the count *is*
/// the region count, which is the defect stated as a number.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn refresh_salience_evaluates_the_anchor_telos_exactly_once(pool: PgPool) {
    let (ctx, anchor, emitter) = context_with_regions(&pool).await;

    let regions = live_region_count(&pool, ctx).await;
    assert!(
        regions > 1,
        "the fixture must carry more than one live region or the N+1 cannot be distinguished from \
         a single correct evaluation; got {regions}"
    );

    install_telos_counter(&pool).await;
    reset_telos_counter(&pool).await;

    write::refresh_salience(&pool, anchor, "workflow-default", emitter)
        .await
        .expect("refresh_salience");

    assert_eq!(
        telos_eval_count(&pool).await,
        1,
        "the anchor telos is one value for the whole anchor and must be evaluated once per \
         refresh, not once per region ({regions} live regions in this fixture)"
    );
}

/// The safety argument for the hoist: it must produce exactly what the incumbent per-region
/// function produces. `anchor_region_telos_alignment` is deliberately left unchanged and is the
/// oracle here, so this stays meaningful for as long as both exist.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn the_hoisted_alignment_equals_the_incumbent_per_region_function(pool: PgPool) {
    let (ctx, anchor, emitter) = context_with_regions(&pool).await;

    write::refresh_salience(&pool, anchor, "workflow-default", emitter)
        .await
        .expect("refresh_salience");

    let disagreements: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM kb_cogmap_regions r \
          WHERE r.home_anchor_table='kb_contexts' AND r.home_anchor_id=$1 AND NOT r.is_folded \
            AND r.telos_alignment IS DISTINCT FROM \
                nullif(anchor_region_telos_alignment(r.id, r.home_anchor_table, r.home_anchor_id, \
                                                     r.lens_id), 'NaN'::double precision)",
    )
    .bind(ctx)
    .fetch_one(&pool)
    .await
    .expect("compare");

    assert_eq!(
        disagreements, 0,
        "every stored telos_alignment must equal what the unchanged per-region function computes; \
         a performance change that moves a value is not a performance change"
    );
}

/// The **cogmap** branch, which the production measurement never reached — it was taken on a
/// context anchor, and a 398-region cogmap exists in production, so "contexts only" would have been
/// a choice rather than a safe default.
///
/// The branch carries the same N+1 for the same reason: `cogmap_region_telos_alignment`
/// (`20260624000002_canonical_functions.sql:461-472`) computes its telos in a `WITH telos AS (SELECT
/// avg(ch.embedding) …)` CTE *per call*. The fix reaches it without a second code path, because the
/// hoisted value comes from `anchor_telos_embedding`, whose `kb_cogmaps` branch is the same
/// aggregate over the same joins — this test is what holds that equivalence claim to account rather
/// than leaving it as a reading of two SQL bodies.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn the_cogmap_branch_is_hoisted_and_agrees_with_its_own_incumbent(pool: PgPool) {
    bootseed::seed_system(&pool).await.unwrap();
    let loaded = loader::load_seed(&pool, &seed()).await.unwrap();
    embed::embed_chunks(&pool).await.unwrap();

    let anchor = HomeAnchor::Cogmap(loaded.cogmap.into());
    let emitter = EntityId::from(loaded.emitter);
    write::materialize(&pool, anchor, "telos-default", emitter)
        .await
        .expect("materialize");

    let regions: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM kb_cogmap_regions \
         WHERE home_anchor_table='kb_cogmaps' AND home_anchor_id=$1 AND NOT is_folded",
    )
    .bind(loaded.cogmap)
    .fetch_one(&pool)
    .await
    .expect("live regions");
    assert!(
        regions > 1,
        "the cogmap fixture must carry more than one live region; got {regions}"
    );

    install_telos_counter(&pool).await;
    reset_telos_counter(&pool).await;

    write::refresh_salience(&pool, anchor, "telos-default", emitter)
        .await
        .expect("refresh_salience");

    assert_eq!(
        telos_eval_count(&pool).await,
        1,
        "the cogmap branch must be hoisted too — it carries the same per-call telos CTE \
         ({regions} live regions in this fixture)"
    );

    let disagreements: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM kb_cogmap_regions r \
          WHERE r.home_anchor_table='kb_cogmaps' AND r.home_anchor_id=$1 AND NOT r.is_folded \
            AND r.telos_alignment IS DISTINCT FROM \
                nullif(anchor_region_telos_alignment(r.id, r.home_anchor_table, r.home_anchor_id, \
                                                     r.lens_id), 'NaN'::double precision)",
    )
    .bind(loaded.cogmap)
    .fetch_one(&pool)
    .await
    .expect("compare");

    assert_eq!(
        disagreements, 0,
        "the hoisted value comes from anchor_telos_embedding's kb_cogmaps branch while the oracle \
         routes through cogmap_region_telos_alignment; a disagreement means those two are not the \
         same aggregate after all"
    );
}

/// Criterion 3: a region with no usable centroid. `populate_readouts` coalesces a
/// memberless/unembedded region to the zero-vector placeholder, and cosine-vs-zero is `NaN` — the
/// `nullif(…, 'NaN')` guard must still turn that into `NULL`, not store a `NaN` that poisons the
/// salience blend downstream.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn a_region_with_a_zero_centroid_stores_null_alignment_not_nan(pool: PgPool) {
    let (ctx, anchor, emitter) = context_with_regions(&pool).await;

    // Force the zero-centroid edge on one real region.
    let victim: uuid::Uuid = sqlx::query_scalar(
        "SELECT id FROM kb_cogmap_regions \
         WHERE home_anchor_table='kb_contexts' AND home_anchor_id=$1 AND NOT is_folded \
         ORDER BY id LIMIT 1",
    )
    .bind(ctx)
    .fetch_one(&pool)
    .await
    .expect("a region");
    sqlx::query(
        "UPDATE kb_cogmap_regions SET centroid = array_fill(0::real, ARRAY[768])::vector \
         WHERE id = $1",
    )
    .bind(victim)
    .execute(&pool)
    .await
    .expect("zero the centroid");

    write::refresh_salience(&pool, anchor, "workflow-default", emitter)
        .await
        .expect("refresh_salience");

    let (alignment, salience): (Option<f64>, f64) =
        sqlx::query_as("SELECT telos_alignment, salience FROM kb_cogmap_regions WHERE id=$1")
            .bind(victim)
            .fetch_one(&pool)
            .await
            .expect("victim readouts");

    assert_eq!(
        alignment, None,
        "cosine against a zero centroid is NaN; the nullif guard must store NULL so the salience \
         blend can coalesce it to 0"
    );
    assert!(
        salience.is_finite(),
        "a NaN alignment would propagate into salience through the blend; got {salience}"
    );
}
