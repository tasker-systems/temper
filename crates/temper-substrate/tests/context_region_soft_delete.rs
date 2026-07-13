#![cfg(feature = "artifact-tests")]
//! A SOFT-DELETED RESOURCE MUST LEAVE THE REGION IT WAS IN.
//!
//! `temper resource delete` is a soft delete: the `kb_resources` row survives with `is_active =
//! false`, and — crucially — its `kb_resource_homes` row survives untouched. `Substrate::load` builds
//! its candidate node set straight off `kb_resource_homes` with no join to `kb_resources`, so a
//! tombstone stayed in the producer's input forever: it kept getting clustered, kept contributing its
//! vector to the region centroid, and kept counting toward `member_count` and `centrality`.
//!
//! Measured on prod (2026-07-13): **every** dead-but-homed resource was still a region member — 40 for
//! 40, across both anchor regimes. Six regions had NO live member at all, and surfaced in the
//! orientation read as nameless rows carrying salience (the region label declines to name a region
//! from a tombstone, correctly — which is how this was found).
//!
//! The fix is two coordinated halves, and this file pins both:
//!
//! 1. **Formation is honest** — `Substrate::load` filters its node set to `is_active`. Because `nodes`
//!    is the root of every downstream input (facets, kNN, edges, clusters, centroids, centrality),
//!    filtering it once fixes all of them. It is deliberately NOT a prune-on-delete side-write into
//!    `kb_cogmap_region_members`: membership is a PROJECTION, and a projection table is written only by
//!    an event projection.
//!
//! 2. **Re-formation is prompt** — `resource_deleted` counts as a formation touch. Without this the
//!    first half is inert in practice: `STRUCTURAL_EVENTS` listed `resource_created` but not
//!    `resource_deleted`, so a delete ticked no clock and the stale region simply sat there until some
//!    unrelated structural event happened to fire on that anchor. That is exactly why the prod ghosts
//!    were *stable* rather than self-healing.
//!
//! Both tests drive the real production write path (`writes::delete_resource` → the `resource_delete`
//! SQL function → `_project_resource_deleted`), not a hand-rolled `UPDATE ... SET is_active = false`.

mod common;

use sqlx::PgPool;
use temper_core::types::home::HomeAnchor;
use temper_core::types::ids::ResourceId;
use temper_substrate::scenario::{bootseed, loader, model::Seed};
use temper_substrate::{embed, replay, write, writes};

const L0_KERNEL_SEED: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/seeds/l0-kernel.yaml"
);

fn seed() -> Seed {
    serde_yaml::from_str(&std::fs::read_to_string(L0_KERNEL_SEED).unwrap()).unwrap()
}

/// How many live regions currently carry `resource` as a member.
async fn memberships(pool: &PgPool, resource: uuid::Uuid) -> i64 {
    sqlx::query_scalar(
        "SELECT count(*) FROM kb_cogmap_region_members m \
           JOIN kb_cogmap_regions r ON r.id = m.region_id AND NOT r.is_folded \
          WHERE m.member_table = 'kb_resources' AND m.member_id = $1",
    )
    .bind(resource)
    .fetch_one(pool)
    .await
    .expect("membership count")
}

/// The newest event id — a watermark to ask "did anything formation-affecting happen since?".
async fn latest_event(pool: &PgPool) -> uuid::Uuid {
    sqlx::query_scalar("SELECT id FROM kb_events ORDER BY id DESC LIMIT 1")
        .fetch_one(pool)
        .await
        .expect("latest event")
}

/// Seed the kernel corpus, re-home it into a fresh context, and embed — the
/// `context_region_smoke` fixture shape (a production context has no facets and no
/// anchor-homed edges, so the embedding is the only clustering signal).
///
/// Returns `(anchor, emitter, one member resource)`.
async fn context_with_regions(pool: &PgPool) -> (HomeAnchor, uuid::Uuid, uuid::Uuid) {
    bootseed::seed_system(pool).await.unwrap();
    let loaded = loader::load_seed(pool, &seed()).await.unwrap();
    embed::embed_chunks(pool).await.unwrap();

    let ctx = common::insert_context(pool, "kb_profiles", loaded.owner, "ghost-ctx", "Ghost Ctx")
        .await
        .expect("context");

    sqlx::query(
        "UPDATE kb_resource_homes SET anchor_table = 'kb_contexts', anchor_id = $1 \
         WHERE anchor_table = 'kb_cogmaps' AND anchor_id = $2",
    )
    .bind(ctx)
    .bind(loaded.cogmap)
    .execute(pool)
    .await
    .expect("re-home");

    sqlx::query(
        "UPDATE kb_properties SET is_folded = true \
         WHERE owner_table = 'kb_resources' AND property_key = 'facet'",
    )
    .execute(pool)
    .await
    .expect("fold facets");

    let anchor = HomeAnchor::Context(ctx.into());
    write::materialize(pool, anchor, "workflow-default", loaded.emitter.into())
        .await
        .expect("materialize");

    // Any resource the producer actually placed in a region. Picking one that IS a member is the
    // precondition the delete assertion rests on — an unclustered resource would pass vacuously.
    let victim: uuid::Uuid = sqlx::query_scalar(
        "SELECT m.member_id FROM kb_cogmap_region_members m \
           JOIN kb_cogmap_regions r ON r.id = m.region_id AND NOT r.is_folded \
          WHERE r.home_anchor_table = 'kb_contexts' AND r.home_anchor_id = $1 \
          ORDER BY m.member_id LIMIT 1",
    )
    .bind(ctx)
    .fetch_one(pool)
    .await
    .expect("a clustered resource to delete");

    (anchor, loaded.emitter, victim)
}

/// Half 1 — formation is honest: re-materializing after a soft delete must evict the tombstone.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn a_soft_deleted_resource_is_not_a_region_member(pool: PgPool) {
    let (anchor, emitter, victim) = context_with_regions(&pool).await;

    assert!(
        memberships(&pool, victim).await > 0,
        "precondition: the victim must be a region member BEFORE the delete, or the post-delete \
         assertion passes vacuously"
    );

    writes::delete_resource(&pool, ResourceId::from(victim), emitter.into())
        .await
        .expect("soft-delete");

    // The delete alone does not touch the projection — membership is written only by the materialize
    // projection. This asserts the pre-fix state is genuinely reproduced rather than papered over.
    assert!(
        memberships(&pool, victim).await > 0,
        "a soft delete must NOT side-write the projection table; the stale row is expected to survive \
         until the next materialize re-forms the region"
    );

    write::materialize(&pool, anchor, "workflow-default", emitter.into())
        .await
        .expect("re-materialize");

    assert_eq!(
        memberships(&pool, victim).await,
        0,
        "a soft-deleted resource must be gone from every region after re-formation — it is not \
         readable, so it must not carry membership, centroid weight, or salience"
    );

    // `member_count` is a STORED column, written at region-assert from the producer's member list — so
    // it inherits the filter rather than needing its own. Pinned because it is the number the surface
    // reads actually show: a region claiming 7 members with 0 readable ones is the visible symptom.
    let overcounting: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM kb_cogmap_regions reg \
          WHERE NOT reg.is_folded \
            AND reg.home_anchor_table = 'kb_contexts' AND reg.home_anchor_id = $1 \
            AND reg.member_count <> ( \
                  SELECT count(*) FROM kb_cogmap_region_members m \
                    JOIN kb_resources r ON r.id = m.member_id \
                   WHERE m.region_id = reg.id AND m.member_table = 'kb_resources' AND r.is_active)",
    )
    .bind(anchor.uuid())
    .fetch_one(&pool)
    .await
    .expect("member_count audit");

    assert_eq!(
        overcounting, 0,
        "every region's stored `member_count` must equal its count of LIVE members — an \
         over-reporting count is what surfaces a ghost region as a populated one"
    );
}

/// Half 2 — re-formation is prompt: a delete is a formation touch, so the region re-forms rather than
/// sitting stale behind an un-ticked clock.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn a_soft_delete_is_a_formation_touch(pool: PgPool) {
    let (anchor, emitter, victim) = context_with_regions(&pool).await;

    let watermark = latest_event(&pool).await;
    assert!(
        !replay::formation_touched_since(&pool, anchor, watermark)
            .await
            .expect("clock read"),
        "precondition: nothing formation-affecting has happened since the watermark"
    );

    writes::delete_resource(&pool, ResourceId::from(victim), emitter.into())
        .await
        .expect("soft-delete");

    assert!(
        replay::formation_touched_since(&pool, anchor, watermark)
            .await
            .expect("clock read"),
        "`resource_deleted` must count as a FORMATION touch — a delete changes the producer's node \
         set, exactly as `resource_created` does. If it ticks no clock, the region never re-forms and \
         the tombstone keeps its membership, centroid weight and salience indefinitely (the prod \
         ghosts)"
    );
}
