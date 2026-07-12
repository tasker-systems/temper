#![cfg(feature = "artifact-tests")]
//! T6 — the two clocks (spec §3.5): **refresh salience without running formation at all.**
//!
//! T5 already proved the clocks come apart, but it proved it *through a full `materialize`* — it ran
//! the whole producer twice and observed that formation was a no-op the second time. That no-op is not
//! free: at live prod dimensions it is a kNN build plus a re-cluster over ~1k resources. T6's job is to
//! make the cheap thing cheap, and these tests hold it to two claims that are easy to *say* and easy to
//! get subtly wrong:
//!
//!  1. **Equivalence** — a salience-only refresh lands on exactly the salience a full materialize would
//!     have produced. If it did not, the cheap path would be a second, divergent definition of
//!     salience, and which one you got would depend on which clock happened to fire.
//!
//!  2. **It really does skip formation** — no re-cluster, no re-mint, no `region_materialized` event.
//!     The last of those is not fussiness: `formation_touched_count_since` counts events, so an event
//!     fired here would advance the very threshold the *expensive* clock gates on, and every cheap trip
//!     would drag the anchor toward a re-cluster it does not need.
//!
//! Both are asserted differentially — against the full producer, which is the shipped, scenario-corpus-
//! verified oracle — rather than against a hand-written expected number, which would only reproduce the
//! author's understanding of the formula.

mod common;

use common::context_fixture::{
    close_task, readouts, rehome_corpus_into_context, seed_goal, seed_task, telos_centroid,
};

use sqlx::PgPool;
use temper_core::types::home::HomeAnchor;
use temper_core::types::ids::ContextId;
use temper_substrate::ids::EntityId;
use temper_substrate::scenario::{bootseed, loader, model::Seed};
use temper_substrate::{embed, substrate, write};
use uuid::Uuid;

const L0_KERNEL_SEED: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/seeds/l0-kernel.yaml"
);

fn seed() -> Seed {
    serde_yaml::from_str(&std::fs::read_to_string(L0_KERNEL_SEED).unwrap()).unwrap()
}

const LENS: &str = "workflow-default";

/// A context with two near-orthogonal goals, one live task each, materialized once.
///
/// Near-orthogonal on purpose: if both goals pointed the same way, closing one would barely rotate the
/// telos and every assertion below could pass while proving nothing.
async fn context_with_two_goals(pool: &PgPool) -> (Uuid, HomeAnchor, EntityId, Uuid, Uuid) {
    bootseed::seed_system(pool).await.unwrap();
    let loaded = loader::load_seed(pool, &seed()).await.unwrap();
    embed::embed_chunks(pool).await.unwrap();

    let ctx = common::insert_context(
        pool,
        "kb_profiles",
        loaded.owner,
        "clocks-ctx",
        "Clocks Ctx",
    )
    .await
    .expect("context");
    rehome_corpus_into_context(pool, ctx, loaded.cogmap).await;

    let mut va = vec![0.0f32; 768];
    va[0] = 1.0;
    let mut vb = vec![0.0f32; 768];
    vb[1] = 1.0;
    let goal_a = seed_goal(pool, ctx, loaded.owner, "Goal A", va).await;
    let goal_b = seed_goal(pool, ctx, loaded.owner, "Goal B", vb).await;
    let task_a = seed_task(pool, ctx, loaded.owner, goal_a, "in-progress").await;
    let _task_b = seed_task(pool, ctx, loaded.owner, goal_b, "in-progress").await;

    let anchor = HomeAnchor::Context(ContextId::from(ctx));
    let emitter = EntityId::from(loaded.emitter);
    write::materialize(pool, anchor, LENS, emitter)
        .await
        .expect("materialize #1");

    (ctx, anchor, emitter, task_a, goal_b)
}

async fn count_materialize_events(pool: &PgPool, ctx: Uuid) -> i64 {
    sqlx::query_scalar(
        "SELECT count(*) FROM kb_events e JOIN kb_event_types et ON et.id = e.event_type_id \
          WHERE et.name = 'region_materialized' \
            AND e.producing_anchor_table = 'kb_contexts' AND e.producing_anchor_id = $1",
    )
    .bind(ctx)
    .fetch_one(pool)
    .await
    .expect("materialize events")
}

async fn lens_id(pool: &PgPool, anchor: HomeAnchor) -> temper_substrate::ids::LensId {
    substrate::load_lens(pool, anchor, LENS)
        .await
        .expect("lens")
        .1
}

/// **The equivalence claim.** After the census moves, the cheap clock must land on precisely the
/// salience the expensive one would have — same telos_alignment, same salience, region for region.
///
/// This is the test that makes the cheap path *safe*, and the only honest way to write it is
/// differentially: run the refresh, snapshot the readouts, then run the full producer over the same
/// substrate and require it to change nothing. Anything else is asserting the formula against itself.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn a_salience_refresh_lands_where_a_full_materialize_would(pool: PgPool) {
    let (ctx, anchor, emitter, task_a, _) = context_with_two_goals(&pool).await;

    // Move the census: Goal A's only task closes, so A drops out of the telos (sw_done = 0.0) and the
    // telos rotates from a blend of A and B onto B alone.
    close_task(&pool, task_a).await;

    // ── the CHEAP clock, alone.
    let refreshed = write::refresh_salience(&pool, anchor, LENS, emitter)
        .await
        .expect("refresh_salience");
    assert!(
        refreshed.regions_refreshed > 0,
        "the refresh must actually touch regions, or the equivalence below is vacuous"
    );
    let cheap = readouts(&pool, ctx).await;
    let cheap_telos = telos_centroid(&pool, ctx).await;

    // ── now the EXPENSIVE clock over the identical substrate. It is the oracle.
    write::materialize(&pool, anchor, LENS, emitter)
        .await
        .expect("materialize #2");
    let full = readouts(&pool, ctx).await;
    let full_telos = telos_centroid(&pool, ctx).await;

    assert_eq!(
        cheap.len(),
        full.len(),
        "the two paths must agree on how many regions there are"
    );
    for (c, f) in cheap.iter().zip(&full) {
        assert_eq!(c.0, f.0, "region ids must line up");
        assert!(
            (c.1 - f.1).abs() < 1e-9,
            "region {}: salience {} from the cheap clock but {} from a full materialize — the two \
             paths have diverged, and which value you get now depends on which clock fired",
            c.0,
            c.1,
            f.1
        );
        match (c.2, f.2) {
            (Some(a), Some(b)) => assert!(
                (a - b).abs() < 1e-9,
                "region {}: telos_alignment {a} vs {b}",
                c.0
            ),
            (a, b) => assert_eq!(a, b, "region {}: telos_alignment NULL-ness differs", c.0),
        }
    }
    assert_eq!(
        cheap_telos, full_telos,
        "and the re-armed telos snapshot must be the same vector either way — otherwise the NEXT \
         drift reading depends on which clock last ran"
    );
}

/// **The skip-formation claim.** The cheap clock must not re-cluster, must not re-mint a region, and
/// must fire no event.
///
/// The event assertion is the load-bearing one. `formation_touched_count_since` counts events against
/// the materialize watermark, so a `region_materialized` fired here would advance the threshold the
/// EXPENSIVE clock gates on — and every cheap trip would drag the anchor toward a re-cluster it does
/// not need, quietly undoing the whole point of separating them.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn the_cheap_clock_runs_no_formation_and_fires_no_event(pool: PgPool) {
    let (ctx, anchor, emitter, task_a, _) = context_with_two_goals(&pool).await;

    let before_ids: Vec<Uuid> = readouts(&pool, ctx).await.iter().map(|r| r.0).collect();
    let before_events = count_materialize_events(&pool, ctx).await;
    let before_components: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM kb_cogmap_components \
          WHERE home_anchor_table='kb_contexts' AND home_anchor_id=$1 AND NOT is_folded",
    )
    .bind(ctx)
    .fetch_one(&pool)
    .await
    .expect("components");

    close_task(&pool, task_a).await;
    write::refresh_salience(&pool, anchor, LENS, emitter)
        .await
        .expect("refresh_salience");

    let after_ids: Vec<Uuid> = readouts(&pool, ctx).await.iter().map(|r| r.0).collect();
    assert_eq!(
        before_ids, after_ids,
        "a salience refresh must not fold, re-mint, or re-partition a single region"
    );
    assert_eq!(
        before_events,
        count_materialize_events(&pool, ctx).await,
        "a salience refresh must fire NO region_materialized event — it is a projection write, and \
         an event here would advance the formation threshold the expensive clock gates on"
    );
    let after_components: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM kb_cogmap_components \
          WHERE home_anchor_table='kb_contexts' AND home_anchor_id=$1 AND NOT is_folded",
    )
    .bind(ctx)
    .fetch_one(&pool)
    .await
    .expect("components");
    assert_eq!(
        before_components, after_components,
        "…and no component was re-clustered"
    );
}

/// **Drift is a reading, and it reads what it should.** Zero right after a materialize; positive once
/// the census moves; back to (under) epsilon once the refresh re-arms the clock.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn telos_drift_is_zero_when_armed_positive_when_the_census_moves_and_rearms(pool: PgPool) {
    let (_, anchor, emitter, task_a, _) = context_with_two_goals(&pool).await;
    let lens = lens_id(&pool, anchor).await;

    // Freshly materialized: the snapshot IS the current telos.
    let armed = write::telos_drift(&pool, anchor, lens)
        .await
        .expect("drift");
    assert!(
        armed
            .distance
            .expect("a context with live goals has a telos")
            < 1e-9,
        "right after a materialize the telos and its snapshot are the same vector: {armed:?}"
    );
    assert!(
        !armed.exceeds_epsilon,
        "so the cheap clock must NOT fire on a write that changed nothing about the census"
    );

    // Close a task: the census moves, so the telos must.
    close_task(&pool, task_a).await;
    let moved = write::telos_drift(&pool, anchor, lens)
        .await
        .expect("drift");
    assert!(
        moved.exceeds_epsilon,
        "closing a task drops its goal out of the telos (sw_done = 0.0) — the telos rotates and the \
         cheap clock must fire. drift = {:?}",
        moved.distance
    );

    // Refreshing re-arms it.
    write::refresh_salience(&pool, anchor, LENS, emitter)
        .await
        .expect("refresh");
    let rearmed = write::telos_drift(&pool, anchor, lens)
        .await
        .expect("drift");
    assert!(
        !rearmed.exceeds_epsilon,
        "the refresh must re-snapshot the telos, or the clock stays stuck ON and every subsequent \
         write refreshes salience forever. drift = {:?}",
        rearmed.distance
    );
}

/// **Drift returns a sane value for BOTH anchor kinds** — T6's stated acceptance criterion.
///
/// T2 gave `telos_centroid` to contexts only, reasoning that a cogmap's telos is a DECLARED resource
/// whose embedding "can be read directly". True for reading it — but drift is not a reading of the
/// telos, it is a reading of how far it has MOVED, and that needs a snapshot on both. Without one this
/// could only ever have returned NULL for a cogmap.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn drift_is_computable_for_a_cogmap_too(pool: PgPool) {
    bootseed::seed_system(&pool).await.unwrap();
    let loaded = loader::load_seed(&pool, &seed()).await.unwrap();
    embed::embed_chunks(&pool).await.unwrap();

    let anchor = HomeAnchor::Cogmap(temper_core::types::ids::CogmapId::from(loaded.cogmap));
    let emitter = EntityId::from(loaded.emitter);

    // Before any materialize there is no snapshot — and NULL must mean "no drift question to ask",
    // never "no drift". (`exceeds_epsilon` false ⇒ the cheap clock declines; the FORMATION clock owns
    // the first trip, because there are no regions to refresh yet.)
    let lens = substrate::load_lens(&pool, anchor, "telos-default")
        .await
        .expect("lens")
        .1;
    let cold = write::telos_drift(&pool, anchor, lens)
        .await
        .expect("drift");
    assert_eq!(
        cold.distance, None,
        "an un-materialized anchor has nothing to compare against"
    );
    assert!(
        !cold.exceeds_epsilon,
        "and a NULL drift must decline to fire the cheap clock, not fire it spuriously"
    );

    write::materialize(&pool, anchor, "telos-default", emitter)
        .await
        .expect("materialize");

    let armed = write::telos_drift(&pool, anchor, lens)
        .await
        .expect("drift");
    let d = armed
        .distance
        .expect("a cogmap declares a telos (its charter), so drift is computable once snapshotted");
    assert!(
        d < 1e-9,
        "right after a materialize a cogmap's telos matches its snapshot: {d}"
    );
    assert!(!armed.exceeds_epsilon);
}

/// **The telos is scale-invariant to pure time passage** — which is why epsilon is tiny rather than a
/// generous deadband, and this is the assertion that says so out loud.
///
/// Liveness is `damper · sqrt(Σ stage_weight · exp(−idle/halflife))`. When wall-clock advances and
/// nothing else happens, EVERY task's idle grows by the same Δt, so every goal's mass scales by one
/// common factor; `sqrt` preserves that, the dampers are time-independent, and a uniform scaling of
/// every weight CANCELS in the centroid's normalisation. So time alone cannot rotate the telos.
///
/// If this ever stops holding — someone adds a nonlinearity to liveness, say — drift would creep on
/// its own, the cheap clock would fire on every write forever, and epsilon would silently become load
/// bearing. Better to fail here.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn wall_clock_alone_does_not_rotate_the_telos(pool: PgPool) {
    let (_, anchor, _emitter, _, _) = context_with_two_goals(&pool).await;
    let lens = lens_id(&pool, anchor).await;

    // Age every task by the same 90 days — exactly what the passage of time does, since `now()`
    // advances uniformly for all of them. (Ageing them by DIFFERENT amounts would be a census change,
    // and would rightly drift.)
    sqlx::query(
        "UPDATE kb_resources SET updated = updated - interval '90 days' \
          WHERE id IN (SELECT p.owner_id FROM kb_properties p \
                        WHERE p.property_key = 'doc_type' AND p.property_value #>> '{}' = 'task' \
                          AND NOT p.is_folded)",
    )
    .execute(&pool)
    .await
    .expect("age tasks");

    let drift = write::telos_drift(&pool, anchor, lens)
        .await
        .expect("drift");
    let d = drift.distance.expect("telos");
    assert!(
        d < 1e-9,
        "90 days of uniform decay rotated the telos by {d} — liveness must have acquired a \
         nonlinearity, and epsilon is now doing work it was never meant to do"
    );
    assert!(
        !drift.exceeds_epsilon,
        "so no write should trigger a salience refresh merely because time has passed"
    );
}
