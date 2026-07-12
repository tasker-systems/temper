#![cfg(feature = "artifact-tests")]
//! THE KERNEL DOES SOMETHING (spec §3.1, plan Task 4 acceptance).
//!
//! The regression floor (`anchor_refactor_regression.rs`) proves the kernel is INERT on the cogmap
//! path. That is only half the claim. This file proves the other half: that in a **context** — where
//! `w_cos = 1.0` — the kernel forms real regions where the declared-only producer forms none.
//!
//! It is a DIFFERENTIAL test, not a hand-written expectation. The same 22 resources, the same
//! producer, the same anchor; the *only* thing that varies between the two materializes is the lens.
//! So a difference in the partition is attributable to `w_cos` and to nothing else. A typed
//! expectation ("expect 5 regions") would just reproduce the author's guess about the embedding model.
//!
//! ## Why the fixture folds the facets
//!
//! Production contexts carry **zero facets** and their declared edges live elsewhere — that is the
//! whole motivating fact of this arc (1,643 context-homed resources, no facets), and it is why the
//! declared-only kernel clusters a context into all-singletons. The fixture reproduces exactly that:
//! re-home the seed's resources into a context (which leaves the edges behind, since edges are
//! anchor-scoped while facets are resource-scoped) and fold the facets, so the embedding is the ONLY
//! signal available. Anything less would let facet overlap do the clustering and the test would prove
//! nothing about `w_cos`.
//!
//! **T9 replaces this with a proper `ContextDef` in the scenario DSL.** Until then this is a re-homed
//! cogmap seed, built inline and marked as such.
mod common;

use sqlx::PgPool;
use temper_core::types::home::HomeAnchor;
use temper_substrate::scenario::{bootseed, loader, model::Seed};
use temper_substrate::{embed, write};

/// 22 resources with substantive bodies across four topical strata (concept / invariant / reference /
/// boundary) — enough corpus for the ≥20-embedded-resource bar the acceptance sets.
const L0_KERNEL_SEED: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/seeds/l0-kernel.yaml"
);

fn seed() -> Seed {
    serde_yaml::from_str(&std::fs::read_to_string(L0_KERNEL_SEED).unwrap()).unwrap()
}

/// Live regions for this anchor under the named lens.
async fn region_count(pool: &PgPool, ctx: uuid::Uuid, lens: &str) -> i64 {
    sqlx::query_scalar(
        "SELECT count(*) FROM kb_cogmap_regions r \
         JOIN kb_cogmap_lenses l ON l.id = r.lens_id \
         WHERE r.home_anchor_table = 'kb_contexts' AND r.home_anchor_id = $1 \
           AND l.name = $2 AND NOT r.is_folded",
    )
    .bind(ctx)
    .bind(lens)
    .fetch_one(pool)
    .await
    .expect("region count")
}

#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn the_w_cos_kernel_forms_regions_where_the_declared_only_kernel_forms_none(pool: PgPool) {
    bootseed::seed_system(&pool).await.unwrap();
    let loaded = loader::load_seed(&pool, &seed()).await.unwrap();
    embed::embed_chunks(&pool).await.unwrap();

    let ctx = common::insert_context(
        &pool,
        "kb_profiles",
        loaded.owner,
        "kernel-ctx",
        "Kernel Ctx",
    )
    .await
    .expect("context");

    // Re-home every seeded resource into the context. `kb_resource_homes.resource_id` is UNIQUE — a
    // resource has exactly ONE home — so this MOVES them. Edges are anchor-scoped and stay behind at
    // the cogmap; the context inherits none, exactly like a real context.
    let moved = sqlx::query(
        "UPDATE kb_resource_homes SET anchor_table = 'kb_contexts', anchor_id = $1 \
         WHERE anchor_table = 'kb_cogmaps' AND anchor_id = $2",
    )
    .bind(ctx)
    .bind(loaded.cogmap)
    .execute(&pool)
    .await
    .expect("re-home")
    .rows_affected() as i64;

    // Facets are RESOURCE-scoped, so they would follow the resources into the context. Fold them: a
    // production context has none, and leaving them in would let facet overlap (w_prop = 0.4, equal on
    // both lenses) form the regions — which would make this test pass while proving nothing about w_cos.
    sqlx::query(
        "UPDATE kb_properties SET is_folded = true \
         WHERE owner_table = 'kb_resources' AND property_key = 'facet'",
    )
    .execute(&pool)
    .await
    .expect("fold facets");

    let n_resources: i64 =
        sqlx::query_scalar("SELECT count(*) FROM kb_resource_homes WHERE anchor_id = $1")
            .bind(ctx)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(
        moved, n_resources,
        "every seeded resource moved to the context"
    );
    assert!(
        n_resources >= 20,
        "the acceptance bar is a context with >= 20 embedded resources; got {n_resources}"
    );

    let anchor = HomeAnchor::Context(ctx.into());

    // ── ARM A: the declared-only kernel (w_cos = 0) ──────────────────────────────────────────────
    // No edges, no facets, and the lens is blind to the embedding. Every pair has affinity 0, so
    // connected_components yields n singletons. THIS IS THE PRODUCTION BUG the arc exists to fix.
    write::materialize(&pool, anchor, "telos-default", loaded.emitter.into())
        .await
        .expect("declared-only materialize");
    let declared_only = region_count(&pool, ctx, "telos-default").await;
    assert_eq!(
        declared_only, n_resources,
        "the declared-only kernel must collapse a facet-free, edge-free context into ALL SINGLETONS \
         — if this is not n, the fixture is leaking a declared signal and Arm B proves nothing"
    );

    // ── ARM B: the w_cos kernel (w_cos = 1) ──────────────────────────────────────────────────────
    // Same corpus, same producer, same anchor. ONLY the lens changed.
    write::materialize(&pool, anchor, "workflow-default", loaded.emitter.into())
        .await
        .expect("kernel materialize");
    let kernel = region_count(&pool, ctx, "workflow-default").await;

    // Observed at authoring time: 4 regions over 23 resources, none a singleton, and topically
    // coherent in a way that CUTS ACROSS the (folded) facet layers — e.g. {event, invocation,
    // inv_event_primary, inv_attribution} is the ledger/attribution cluster, mixing a concept, an
    // invariant and a reference. That cross-cutting is the tell that the EMBEDDING formed them.
    //
    // The assertions below deliberately do NOT pin that partition: a literal would encode a guess
    // about bge-base-en-v1.5's geometry and would break on any re-embed. What is asserted is the pair
    // of degenerate outcomes the construction exists to prevent.
    assert!(
        kernel > 1,
        "not one blob: a raw (dense) cosine would make the affinity graph complete and collapse the \
         context to a single region. The sparse top-k-above-floor construction is what prevents it. \
         Got {kernel} regions."
    );
    assert!(
        kernel < n_resources,
        "not all singletons: the kernel must actually BIND related resources. Got {kernel} regions \
         over {n_resources} resources — the same degenerate partition the declared-only lens gives, \
         which means w_cos contributed nothing."
    );

    // The differential, stated directly: the lens is the only variable, so this inequality IS the
    // kernel's effect.
    assert_ne!(
        kernel, declared_only,
        "same corpus, same producer, only the lens differs — the partition MUST differ"
    );
}

/// Determinism (acceptance): the same corpus under the same lens re-materializes to the same
/// fingerprint. `membership_fingerprint` depends on formation being reproducible, which is why the kNN
/// is computed exactly and never through an approximate index.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn the_kernel_is_deterministic_across_repeated_materializes(pool: PgPool) {
    bootseed::seed_system(&pool).await.unwrap();
    let loaded = loader::load_seed(&pool, &seed()).await.unwrap();
    embed::embed_chunks(&pool).await.unwrap();

    let ctx = common::insert_context(&pool, "kb_profiles", loaded.owner, "det-ctx", "Det Ctx")
        .await
        .expect("context");
    sqlx::query(
        "UPDATE kb_resource_homes SET anchor_table = 'kb_contexts', anchor_id = $1 \
         WHERE anchor_table = 'kb_cogmaps' AND anchor_id = $2",
    )
    .bind(ctx)
    .bind(loaded.cogmap)
    .execute(&pool)
    .await
    .expect("re-home");

    let anchor = HomeAnchor::Context(ctx.into());
    let first = write::materialize(&pool, anchor, "workflow-default", loaded.emitter.into())
        .await
        .expect("materialize");
    let second =
        write::incremental_materialize(&pool, anchor, "workflow-default", loaded.emitter.into())
            .await
            .expect("re-materialize");

    assert_eq!(
        first.membership_fingerprint, second.membership_fingerprint,
        "an unchanged corpus must re-materialize to the same fingerprint — if the kNN graph were \
         built from an approximate index, or iterated in hash-map order, this is where it would show"
    );
}
