#![cfg(feature = "artifact-tests")]
//! REGION-ID STABILITY: a region whose member set did not change keeps its id.
//!
//! `materialize` folds every live region and re-asserts the partition. Before this change every
//! re-assert minted a fresh UUID, so a region that came back bit-for-bit identical came back with a
//! **new id** — a lie to `graph_region_members`, `graph_cogmap_territories`, wayfind, `region_metrics`
//! and `atlas_search`, all of which hold region ids.
//!
//! ## These tests assert on IDS, not counts
//!
//! That is the whole point. The region *count* was stable the entire time this bug was live — the same
//! four regions, the same members, four brand-new ids. A count assertion stays green through total id
//! churn, which is exactly how this hid for as long as it did. Every assertion below compares the
//! `member-set → region-id` mapping across a re-materialize.
//!
//! ## Why removing a resource is the perturbation
//!
//! To exercise the *incremental* path's reuse, a component must actually be classified `changed` —
//! otherwise incremental reuses it wholesale at the component grain and the region grain is never
//! reached. Dropping a resource's home is the one perturbation that is unambiguously a membership
//! change (it moves `members`, so `component_fingerprint` cannot fail to move), and it doubles as the
//! supersession case: the region that LOST the member must get a new id, while its untouched siblings
//! keep theirs.

mod common;

use sqlx::PgPool;
use std::collections::HashMap;
use temper_core::types::home::HomeAnchor;
use temper_substrate::scenario::{bootseed, loader, model::Seed};
use temper_substrate::{embed, write};
use uuid::Uuid;

const ONBOARDING_SEED: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/seeds/onboarding-cogmap.yaml"
);
const L0_KERNEL_SEED: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/seeds/l0-kernel.yaml"
);

fn seed(path: &str) -> Seed {
    serde_yaml::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

/// Every live region under (anchor, lens), keyed by a UUID-free signature of its members: their titles,
/// sorted and joined. Title-keyed rather than uuid-keyed so the key survives across ephemeral
/// `#[sqlx::test]` databases, and — more to the point — so "the same region" means *the same member
/// set*, independent of the id we are trying to prove stable.
async fn regions_by_members(
    pool: &PgPool,
    anchor: HomeAnchor,
    lens: &str,
) -> HashMap<String, Uuid> {
    sqlx::query_as::<_, (String, Uuid)>(
        "SELECT string_agg(res.title, '|' ORDER BY res.title), r.id \
         FROM kb_cogmap_regions r \
         JOIN kb_cogmap_lenses l ON l.id = r.lens_id \
         JOIN kb_cogmap_region_members m ON m.region_id = r.id AND m.member_table = 'kb_resources' \
         JOIN kb_resources res ON res.id = m.member_id \
         WHERE r.home_anchor_table = $1 AND r.home_anchor_id = $2 AND l.name = $3 \
           AND NOT r.is_folded \
         GROUP BY r.id",
    )
    .bind(anchor.table())
    .bind(anchor.uuid())
    .bind(lens)
    .fetch_all(pool)
    .await
    .expect("live regions by member signature")
    .into_iter()
    .collect()
}

/// Drop one resource out of the anchor entirely, by title. Its home is what makes it a substrate node
/// (`substrate::load` selects nodes straight from `kb_resource_homes`), so this removes it from the
/// graph — and therefore from whichever region held it.
async fn unhome_resource(pool: &PgPool, title: &str) {
    let n = sqlx::query(
        "DELETE FROM kb_resource_homes WHERE resource_id = \
           (SELECT id FROM kb_resources WHERE title = $1)",
    )
    .bind(title)
    .execute(pool)
    .await
    .expect("unhome")
    .rows_affected();
    assert_eq!(
        n, 1,
        "fixture drift: expected exactly one resource titled {title:?}"
    );
}

/// The core invariant, asserted after every re-materialize in this file: **a member set that was a live
/// region before, and is a live region after, carries the SAME id**. Stated over member sets rather
/// than over ids, so it holds whether or not the partition moved — a region that genuinely changed is
/// simply a different key, and is allowed (required, even) to have a new id.
fn assert_unchanged_regions_kept_their_ids(
    before: &HashMap<String, Uuid>,
    after: &HashMap<String, Uuid>,
) {
    for (members, before_id) in before {
        if let Some(after_id) = after.get(members) {
            assert_eq!(
                before_id, after_id,
                "region {members:?} has the same member set before and after, so it is the same \
                 region — it MUST keep its id. A fresh id here is the churn this change exists to \
                 stop, and it is invisible to any count-based assertion."
            );
        }
    }
}

/// A full `materialize` over an unchanged cogmap corpus must reuse **every** region id.
///
/// This is the declared-only regime (`telos-default`, `w_cos = 0`) — the regression floor. A full pass
/// folds every region and re-asserts the whole partition, which is precisely the path that used to
/// re-mint every id.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn full_re_materialize_of_an_unchanged_cogmap_reuses_every_region_id(pool: PgPool) {
    bootseed::seed_system(&pool).await.unwrap();
    let loaded = loader::load_seed(&pool, &seed(ONBOARDING_SEED))
        .await
        .unwrap();
    embed::embed_chunks(&pool).await.unwrap();

    let anchor = HomeAnchor::Cogmap(loaded.cogmap.into());
    let first = write::materialize(&pool, anchor, "telos-default", loaded.emitter.into())
        .await
        .expect("materialize");
    let before = regions_by_members(&pool, anchor, "telos-default").await;

    let second = write::materialize(&pool, anchor, "telos-default", loaded.emitter.into())
        .await
        .expect("re-materialize");
    let after = regions_by_members(&pool, anchor, "telos-default").await;

    assert_eq!(
        first.membership_fingerprint, second.membership_fingerprint,
        "an unchanged corpus must re-materialize to the same partition"
    );
    assert_eq!(
        before, after,
        "same partition AND same ids, region for region"
    );
    assert_eq!(
        (second.regions_reused, second.regions_minted),
        (second.regions, 0),
        "nothing changed, so every region must be REUSED and none minted"
    );

    // and the fold left no duplicate live rows behind — a reused region must survive the fold, not be
    // folded and re-inserted (it cannot be: `id` is the PK and the fold is soft).
    let live: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM kb_cogmap_regions WHERE home_anchor_table='kb_cogmaps' \
           AND home_anchor_id=$1 AND NOT is_folded",
    )
    .bind(loaded.cogmap)
    .fetch_one(&pool)
    .await
    .expect("live count");
    assert_eq!(live, second.regions as i64, "exactly one live region set");
}

/// The same claim in the regime the arc exists for: a **context** under `workflow-default`
/// (`w_cos > 0`), where the kNN graph is connected and the whole anchor is ONE component.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn full_re_materialize_of_an_unchanged_context_reuses_every_region_id(pool: PgPool) {
    bootseed::seed_system(&pool).await.unwrap();
    let loaded = loader::load_seed(&pool, &seed(L0_KERNEL_SEED))
        .await
        .unwrap();
    embed::embed_chunks(&pool).await.unwrap();
    let ctx = rehome_into_context(&pool, &loaded, "stability-ctx", "Stability Ctx").await;

    let anchor = HomeAnchor::Context(ctx.into());
    let first = write::materialize(&pool, anchor, "workflow-default", loaded.emitter.into())
        .await
        .expect("materialize");
    let before = regions_by_members(&pool, anchor, "workflow-default").await;
    assert!(
        first.regions > 1,
        "fixture must form real regions under w_cos, else this proves nothing"
    );

    let second = write::materialize(&pool, anchor, "workflow-default", loaded.emitter.into())
        .await
        .expect("re-materialize");
    let after = regions_by_members(&pool, anchor, "workflow-default").await;

    assert_eq!(
        before, after,
        "same partition AND same ids, region for region"
    );
    assert_eq!(
        (second.regions_reused, second.regions_minted),
        (second.regions, 0),
        "nothing changed, so every region must be REUSED and none minted"
    );
}

/// **Supersession stays honest, and the incremental path reuses.**
///
/// Dropping one resource out of the context makes the (single) component `changed`, so
/// `incremental_materialize` re-clusters the entire context — the churn scenario T4's review measured
/// (`REUSED=0`, all regions folded + recreated). The region that lost the member must get a NEW id;
/// every region whose member set is untouched must keep its id.
///
/// The reuse count printed here is the measurement **T6** inherits: it is how much of T6's stated cost
/// ("~500 regions folded + recreated + 3 readout UPDATEs each") region-id stability already removes,
/// leaving only the re-cluster itself.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn a_membership_change_mints_a_new_id_and_leaves_every_other_region_alone(pool: PgPool) {
    bootseed::seed_system(&pool).await.unwrap();
    let loaded = loader::load_seed(&pool, &seed(L0_KERNEL_SEED))
        .await
        .unwrap();
    embed::embed_chunks(&pool).await.unwrap();
    let ctx = rehome_into_context(&pool, &loaded, "churn-ctx", "Churn Ctx").await;

    let anchor = HomeAnchor::Context(ctx.into());
    write::materialize(&pool, anchor, "workflow-default", loaded.emitter.into())
        .await
        .expect("materialize");
    let before = regions_by_members(&pool, anchor, "workflow-default").await;

    // the resource we drop, and the region that holds it — that region's member set is about to change.
    let dropped = pick_a_member(&before);
    let host = before
        .iter()
        .find(|(members, _)| members.split('|').any(|t| t == dropped))
        .map(|(members, id)| (members.clone(), *id))
        .expect("the dropped title belongs to some region");

    unhome_resource(&pool, &dropped).await;

    let out =
        write::incremental_materialize(&pool, anchor, "workflow-default", loaded.emitter.into())
            .await
            .expect("re-materialize after membership change");
    let after = regions_by_members(&pool, anchor, "workflow-default").await;

    assert_unchanged_regions_kept_their_ids(&before, &after);

    // the region that lost a member no longer exists under its old member set...
    assert!(
        !after.contains_key(&host.0),
        "the region that lost {dropped:?} must no longer exist under its old member set"
    );
    // ...and no live region carries its id: a changed member set is a NEW region.
    assert!(
        !after.values().any(|id| *id == host.1),
        "a region whose membership genuinely changed must NOT keep its id — supersession has to stay \
         honest, or a consumer holding that id would silently follow a region it never asked for"
    );

    // the whole point: one member left, and the rest of the context did NOT churn.
    assert!(
        out.regions_reused > 0,
        "re-clustering the one component must still REUSE the regions whose member sets survived \
         intact — that is the entire fix. reused={} minted={}",
        out.regions_reused,
        out.regions_minted
    );
    // Dropping a resource perturbs the kNN graph globally (every node's neighbour list can shift), so
    // some genuine partition movement here is CORRECT, not churn. The 100%-reuse claim belongs to the
    // unchanged-corpus tests above; what this one shows is that a real membership change costs only the
    // regions it actually moved.
    println!(
        "one member dropped from a {}-region context: {} regions reused, {} minted \
         (before this change: 0 reused, {} minted — every id churned regardless)",
        before.len(),
        out.regions_reused,
        out.regions_minted,
        out.regions_reused + out.regions_minted
    );
}

/// Any title in the partition — the fixture's own membership, not a hand-picked literal, so this does
/// not rot when the seed or the embedding model moves.
fn pick_a_member(regions: &HashMap<String, Uuid>) -> String {
    let mut sigs: Vec<&String> = regions.keys().collect();
    sigs.sort();
    // a member of a MULTI-member region: dropping a singleton's only member deletes the region outright
    // rather than changing a member set, which would not exercise supersession.
    let multi = sigs
        .iter()
        .find(|s| s.contains('|'))
        .expect("fixture must contain at least one multi-member region");
    multi
        .split('|')
        .next()
        .expect("non-empty region")
        .to_string()
}

/// Re-home a loaded cogmap seed's resources into a fresh context and fold their facets — the
/// production context shape (zero facets; edges are anchor-scoped and stay behind with the cogmap), so
/// the embedding is the only formation signal. Mirrors `context_region_smoke.rs`, whose comments
/// explain why each half is load-bearing; replaced by a real `ContextDef` in T9.
async fn rehome_into_context(
    pool: &PgPool,
    loaded: &loader::Loaded,
    slug: &str,
    title: &str,
) -> Uuid {
    let ctx = common::insert_context(pool, "kb_profiles", loaded.owner, slug, title)
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
    // Facets are RESOURCE-scoped, so they follow the resources into the context. Fold them: a
    // production context has none, and leaving them in would let facet overlap form the regions — the
    // context would then decompose into many components and the one-component churn this file is about
    // would never arise.
    sqlx::query(
        "UPDATE kb_properties SET is_folded = true \
         WHERE owner_table = 'kb_resources' AND property_key = 'facet'",
    )
    .execute(pool)
    .await
    .expect("fold facets");
    ctx
}
