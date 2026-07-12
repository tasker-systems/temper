#![cfg(feature = "artifact-tests")]
//! T5 payoff: a context region acquires a telos, and **closing a goal moves salience without changing
//! region membership** — the goal-level acceptance criterion of the whole arc (spec §3.4, §3.5).
//!
//! ## What was broken
//!
//! `populate_readouts` computed `cogmap_region_telos_alignment(r.id, r.cogmap_id)`. For a context
//! region `r.cogmap_id` is **NULL** (vestigial — superseded by the anchor pair), so the function
//! matched no cogmap, returned NULL, and *every context region in existence scored a telos_alignment
//! of NULL*. With `s_telos = 0.6` the dominant term of context salience was permanently coalesced to
//! zero. This file is the proof that it no longer is.
//!
//! ## Why the two clocks genuinely come apart
//!
//! Not by assertion — by construction. Formation reads members, edges, facets (`property_key='facet'`
//! ONLY — `substrate.rs:99`) and embeddings. Liveness reads `temper-stage` property rows and
//! `advances` edges. Closing a task rewrites a `temper-stage` row, which is in the second set and not
//! the first. So membership and its fingerprint *cannot* move, while the telos *must*. That is spec
//! §3.5's claim, and the assertions below hold it to it.
//!
//! Region ids survive a recompute only because of the region-id stability fix (PR #389). Before it,
//! every region was re-minted each pass and "salience moved on region X" was not a statement one
//! could even make.

mod common;

use common::context_fixture::{
    close_task, readouts, rehome_corpus_into_context, seed_goal, seed_task, telos_centroid,
};

use sqlx::PgPool;
use temper_core::types::home::HomeAnchor;
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

#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn closing_a_goal_moves_salience_without_changing_region_membership(pool: PgPool) {
    bootseed::seed_system(&pool).await.unwrap();
    let loaded = loader::load_seed(&pool, &seed()).await.unwrap();
    embed::embed_chunks(&pool).await.unwrap();

    let ctx = common::insert_context(&pool, "kb_profiles", loaded.owner, "telos-ctx", "Telos Ctx")
        .await
        .expect("context");

    rehome_corpus_into_context(&pool, ctx, loaded.cogmap).await;

    // Two goals pointing in near-orthogonal directions, one live task each.
    let mut va = vec![0.0f32; 768];
    va[0] = 1.0;
    let mut vb = vec![0.0f32; 768];
    vb[1] = 1.0;
    let goal_a = seed_goal(&pool, ctx, loaded.owner, "Goal A", va).await;
    let goal_b = seed_goal(&pool, ctx, loaded.owner, "Goal B", vb).await;
    let task_a = seed_task(&pool, ctx, loaded.owner, goal_a, "in-progress").await;
    let _task_b = seed_task(&pool, ctx, loaded.owner, goal_b, "in-progress").await;

    // ── materialize #1 ───────────────────────────────────────────────────────────────────────────
    let anchor = HomeAnchor::Context(temper_core::types::ids::ContextId::from(ctx));
    let before = write::materialize(
        &pool,
        anchor,
        "workflow-default",
        EntityId::from(loaded.emitter),
    )
    .await
    .expect("materialize #1");
    let r1 = readouts(&pool, ctx).await;
    let telos1 = telos_centroid(&pool, ctx).await;

    // THE CORE T5 CLAIM. Before this change every one of these was NULL, because the readout keyed on
    // `r.cogmap_id` — NULL for a context region — so the telos term of salience was dead.
    assert!(
        r1.iter().any(|(_, _, t)| t.is_some()),
        "a context region must now have a computable telos_alignment; all NULL means the anchor \
         dispatch is not reaching the kb_contexts branch"
    );
    assert!(
        telos1.is_some(),
        "kb_contexts.telos_centroid must be snapshotted — T6's two-clock gate has nothing to \
         compare against without it"
    );

    // ── close Goal A, then materialize #2 ────────────────────────────────────────────────────────
    // Only a `temper-stage` property row changes. Nothing that feeds formation is touched.
    close_task(&pool, task_a).await;

    let after = write::materialize(
        &pool,
        anchor,
        "workflow-default",
        EntityId::from(loaded.emitter),
    )
    .await
    .expect("materialize #2");
    let r2 = readouts(&pool, ctx).await;
    let telos2 = telos_centroid(&pool, ctx).await;

    // ── clock 1: FORMATION did not move ──────────────────────────────────────────────────────────
    assert_eq!(
        before.membership_fingerprint, after.membership_fingerprint,
        "closing a goal must not re-partition the context"
    );
    assert_eq!(
        r1.iter().map(|r| r.0).collect::<Vec<_>>(),
        r2.iter().map(|r| r.0).collect::<Vec<_>>(),
        "the region IDS must survive (region-id stability, PR #389) — otherwise 'salience moved on \
         region X' is not even a statement one can make"
    );
    assert_eq!(
        after.regions_minted, 0,
        "every region should have been REUSED, not re-minted"
    );

    // ── clock 2: SALIENCE did move ───────────────────────────────────────────────────────────────
    // Goal A drops out of the telos (its only task is done, and sw_done = 0.0), so the telos rotates
    // from a blend of A and B onto B alone. Every region's cosine against it changes.
    assert_ne!(
        telos1, telos2,
        "the telos must rotate when a goal's liveness collapses"
    );
    let moved = r1
        .iter()
        .zip(&r2)
        .filter(|((_, s1, _), (_, s2, _))| (s1 - s2).abs() > 1e-9)
        .count();
    assert!(
        moved > 0,
        "closing a goal must move salience on at least one region; membership was identical, so a \
         zero delta means the telos is not reaching the salience blend"
    );
}

/// T5's other acceptance criterion: **a context with zero live goals degrades gracefully.**
///
/// Most contexts have no goals at all, and every context has none on the day it is created. The telos
/// is then undefined — which must mean "fall back", not "fail" and not "score everything zero by
/// dividing by an empty sum". `anchor_telos_embedding` returns NULL, `nullif`/`coalesce` carry it, and
/// salience keeps its other two terms.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn a_context_with_no_goals_falls_back_to_reference_standing_and_centrality(pool: PgPool) {
    bootseed::seed_system(&pool).await.unwrap();
    let loaded = loader::load_seed(&pool, &seed()).await.unwrap();
    embed::embed_chunks(&pool).await.unwrap();

    let ctx = common::insert_context(&pool, "kb_profiles", loaded.owner, "bare-ctx", "Bare Ctx")
        .await
        .expect("context");
    sqlx::query(
        "UPDATE kb_resource_homes SET anchor_table='kb_contexts', anchor_id=$1 \
         WHERE anchor_table='kb_cogmaps' AND anchor_id=$2",
    )
    .bind(ctx)
    .bind(loaded.cogmap)
    .execute(&pool)
    .await
    .expect("re-home");

    // Re-anchor the seed's EDGES into the context too. Edges are anchor-scoped, so a bare re-home
    // leaves them behind at the cogmap — and a context with no edges has zero centrality, no
    // provenance therefore zero reference-standing, and (with no goals) no telos. All three salience
    // terms would be zero, and "falls back to reference-standing + centrality" would be untestable:
    // the assertion would pass against a fixture that has nothing to fall back TO. Give it centrality,
    // so the fallback is actually exercised rather than asserted into a vacuum.
    sqlx::query(
        "UPDATE kb_edges SET home_anchor_table='kb_contexts', home_anchor_id=$1 \
         WHERE home_anchor_table='kb_cogmaps' AND home_anchor_id=$2",
    )
    .bind(ctx)
    .bind(loaded.cogmap)
    .execute(&pool)
    .await
    .expect("re-anchor edges");

    // No goals seeded at all — the telos has nothing to be computed from.
    let anchor = HomeAnchor::Context(temper_core::types::ids::ContextId::from(ctx));
    write::materialize(
        &pool,
        anchor,
        "workflow-default",
        EntityId::from(loaded.emitter),
    )
    .await
    .expect("a goal-less context must still materialize");

    let rows = readouts(&pool, ctx).await;
    assert!(!rows.is_empty(), "regions still form without a telos");
    assert!(
        rows.iter().all(|(_, _, t)| t.is_none()),
        "no goals means no telos: every telos_alignment must be NULL, never NaN and never 0.0 — a \
         stored 0.0 would be a real cosine reading of 'orthogonal to the context's purpose'"
    );
    assert!(
        telos_centroid(&pool, ctx).await.is_none(),
        "and nothing is snapshotted onto kb_contexts.telos_centroid"
    );
    // salience is NOT NULL by constraint, and must still be computable from the surviving terms.
    // Every value must be a real finite number — a NaN here would mean a cosine was taken against a
    // zero/absent telos vector and stored, which is the failure mode `nullif` exists to catch.
    assert!(
        rows.iter().all(|(_, s, _)| s.is_finite()),
        "salience must stay finite with no telos; a NaN means a cosine-vs-nothing reached the blend"
    );
    assert!(
        rows.iter().any(|(_, s, _)| *s > 0.0),
        "salience must fall back to the surviving terms (here: centrality), not collapse to zero \
         just because the telos is absent"
    );
}
