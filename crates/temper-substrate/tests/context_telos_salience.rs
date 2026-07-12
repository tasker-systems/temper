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

use sqlx::PgPool;
use temper_core::types::home::HomeAnchor;
use temper_substrate::ids::EntityId;
use temper_substrate::scenario::{bootseed, loader, model::Seed};
use temper_substrate::{embed, write};
use uuid::Uuid;

const L0_KERNEL_SEED: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/seeds/l0-kernel.yaml"
);

fn seed() -> Seed {
    serde_yaml::from_str(&std::fs::read_to_string(L0_KERNEL_SEED).unwrap()).unwrap()
}

fn sha256_hex(s: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(s.as_bytes());
    h.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

/// An event id to satisfy the NOT NULL FKs on fixture rows.
async fn any_event(pool: &PgPool) -> Uuid {
    sqlx::query_scalar("SELECT id FROM kb_events ORDER BY occurred_at LIMIT 1")
        .fetch_one(pool)
        .await
        .expect("event")
}

/// A goal resource homed in `ctx`, carrying one chunk with the **given** synthetic embedding.
///
/// The vector is supplied rather than embedded so the two goals can be made deliberately DISSIMILAR
/// (near-orthogonal). If both goals embedded to roughly the same place — which real prose about one
/// project tends to — the telos would barely rotate when one is closed, and the test could pass while
/// proving nothing. Handing the vectors in makes the telos shift a controlled input.
async fn seed_goal(pool: &PgPool, ctx: Uuid, owner: Uuid, title: &str, v: Vec<f32>) -> Uuid {
    let ev = any_event(pool).await;
    let id: Uuid = sqlx::query_scalar(
        "INSERT INTO kb_resources (title, origin_uri) VALUES ($1,'') RETURNING id",
    )
    .bind(title)
    .fetch_one(pool)
    .await
    .expect("goal");
    sqlx::query(
        "INSERT INTO kb_resource_homes (resource_id, anchor_table, anchor_id, \
           originator_profile_id, owner_profile_id) VALUES ($1,'kb_contexts',$2,$3,$3)",
    )
    .bind(id)
    .bind(ctx)
    .bind(owner)
    .execute(pool)
    .await
    .expect("home");
    set_prop(pool, id, "doc_type", "goal").await;

    let block: Uuid = sqlx::query_scalar(
        "INSERT INTO kb_content_blocks (resource_id, seq, genesis_event_id, last_event_id) \
         VALUES ($1, 0, $2, $2) RETURNING id",
    )
    .bind(id)
    .bind(ev)
    .fetch_one(pool)
    .await
    .expect("block");
    sqlx::query(
        "INSERT INTO kb_chunks (block_id, resource_id, chunk_index, content_hash, embedding) \
         VALUES ($1, $2, 0, $3, $4::vector)",
    )
    .bind(block)
    .bind(id)
    .bind(sha256_hex(title))
    .bind(format!(
        "[{}]",
        v.iter()
            .map(|f| f.to_string())
            .collect::<Vec<_>>()
            .join(",")
    ))
    .execute(pool)
    .await
    .expect("chunk");
    id
}

/// A task homed in `ctx` that `advances` `goal`, at the given stage. No body — a task contributes to
/// the CENSUS, never to the telos vector.
async fn seed_task(pool: &PgPool, ctx: Uuid, owner: Uuid, goal: Uuid, stage: &str) -> Uuid {
    let ev = any_event(pool).await;
    let id: Uuid = sqlx::query_scalar(
        "INSERT INTO kb_resources (title, origin_uri) VALUES ($1,'') RETURNING id",
    )
    .bind(format!("task {stage}"))
    .fetch_one(pool)
    .await
    .expect("task");
    sqlx::query(
        "INSERT INTO kb_resource_homes (resource_id, anchor_table, anchor_id, \
           originator_profile_id, owner_profile_id) VALUES ($1,'kb_contexts',$2,$3,$3)",
    )
    .bind(id)
    .bind(ctx)
    .bind(owner)
    .execute(pool)
    .await
    .expect("home");
    set_prop(pool, id, "doc_type", "task").await;
    set_prop(pool, id, "temper-stage", stage).await;
    sqlx::query(
        "INSERT INTO kb_edges (source_table, source_id, target_table, target_id, edge_kind, \
           polarity, label, home_anchor_table, home_anchor_id, asserted_by_event_id, last_event_id) \
         VALUES ('kb_resources',$1,'kb_resources',$2,'leads_to','forward','advances', \
           'kb_contexts',$3,$4,$4)",
    )
    .bind(id)
    .bind(goal)
    .bind(ctx)
    .bind(ev)
    .execute(pool)
    .await
    .expect("advances");
    id
}

async fn set_prop(pool: &PgPool, owner: Uuid, key: &str, value: &str) {
    let ev = any_event(pool).await;
    sqlx::query(
        "INSERT INTO kb_properties (owner_table, owner_id, property_key, property_value, \
           asserted_by_event_id, last_event_id) \
         VALUES ('kb_resources',$1,$2,to_jsonb($3::text),$4,$4)",
    )
    .bind(owner)
    .bind(key)
    .bind(value)
    .bind(ev)
    .execute(pool)
    .await
    .expect("prop");
}

/// Close a task the way the real path does: FOLD the live stage row, assert a new one. (A blind
/// UPDATE would work here too, but folding is what production does, and `context_goal_liveness`
/// depends on `NOT is_folded` selecting exactly one row.)
async fn close_task(pool: &PgPool, task: Uuid) {
    sqlx::query(
        "UPDATE kb_properties SET is_folded = true \
         WHERE owner_table='kb_resources' AND owner_id=$1 AND property_key='temper-stage'",
    )
    .bind(task)
    .execute(pool)
    .await
    .expect("fold stage");
    set_prop(pool, task, "temper-stage", "done").await;
}

/// (region id, salience, telos_alignment) for every live region of this context, id-ordered.
async fn readouts(pool: &PgPool, ctx: Uuid) -> Vec<(Uuid, f64, Option<f64>)> {
    sqlx::query_as(
        "SELECT id, salience, telos_alignment FROM kb_cogmap_regions \
         WHERE home_anchor_table='kb_contexts' AND home_anchor_id=$1 AND NOT is_folded \
         ORDER BY id",
    )
    .bind(ctx)
    .fetch_all(pool)
    .await
    .expect("readouts")
}

async fn telos_centroid(pool: &PgPool, ctx: Uuid) -> Option<String> {
    sqlx::query_scalar("SELECT telos_centroid::text FROM kb_contexts WHERE id=$1")
        .bind(ctx)
        .fetch_one(pool)
        .await
        .expect("telos_centroid")
}

#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn closing_a_goal_moves_salience_without_changing_region_membership(pool: PgPool) {
    bootseed::seed_system(&pool).await.unwrap();
    let loaded = loader::load_seed(&pool, &seed()).await.unwrap();
    embed::embed_chunks(&pool).await.unwrap();

    let ctx = common::insert_context(&pool, "kb_profiles", loaded.owner, "telos-ctx", "Telos Ctx")
        .await
        .expect("context");

    // Re-home the seeded corpus into the context and fold its facets — the same construction
    // `context_region_smoke` uses, and for the same reason: a production context has no facets, so
    // the embedding must be the only formation signal.
    sqlx::query(
        "UPDATE kb_resource_homes SET anchor_table='kb_contexts', anchor_id=$1 \
         WHERE anchor_table='kb_cogmaps' AND anchor_id=$2",
    )
    .bind(ctx)
    .bind(loaded.cogmap)
    .execute(&pool)
    .await
    .expect("re-home");
    sqlx::query(
        "UPDATE kb_properties SET is_folded=true \
         WHERE owner_table='kb_resources' AND property_key='facet'",
    )
    .execute(&pool)
    .await
    .expect("fold facets");

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
