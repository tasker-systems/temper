#![cfg(feature = "artifact-tests")]
//! THE REGRESSION FLOOR for the anchor-generalization arc (plan §"The One Test That Governs
//! Everything"): with `w_cos = 0.0`, the anchor-keyed producer must form *the same regions over the
//! same corpus* as the cogmap-keyed producer it replaces.
//!
//! **Why this does not assert `membership_fingerprint` directly.** `component_fingerprint` hashes
//! member UUIDs, and the seed loader mints fresh UUIDs into every ephemeral `#[sqlx::test]` database.
//! So a fingerprint literal is not stable run-to-run, and asserting "the fingerprint equals itself
//! across two materializes" tests DETERMINISM, not BEHAVIOR PRESERVATION — it stays green even if the
//! refactor reshuffles which resources cluster together, which is the one thing this floor exists to
//! catch.
//!
//! Instead the floor is pinned on a UUID-free canonical signature: each live region rendered as the
//! sorted set of its member *titles*, regions sorted among themselves. That is stable across runs, so
//! `EXPECTED_MEMBERSHIP` below is a golden captured from the CURRENT (cogmap-keyed) producer and
//! re-asserted against the anchor-keyed one.
//!
//! **If this goes red after the refactor, the refactor changed behavior. Do not adjust the golden.**
mod common;

use sqlx::PgPool;
use temper_core::types::home::HomeAnchor;
use temper_substrate::scenario::{bootseed, loader, model::Seed};
use temper_substrate::{embed, write};

const ONBOARDING_SEED: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/seeds/onboarding-cogmap.yaml"
);

fn seed() -> Seed {
    serde_yaml::from_str(&std::fs::read_to_string(ONBOARDING_SEED).unwrap()).unwrap()
}

/// Captured from the CURRENT (cogmap-keyed) producer on the onboarding seed, BEFORE the
/// anchor-generalization refactor. The anchor-keyed producer must reproduce it exactly.
const EXPECTED_MEMBERSHIP: &[&str] = &[
    "Onboarding charter|regulation: pair on the first PR",
    "concept: big-bang-cutover|concept: blue-green|concept: deploy-confidence-checklist|concept: feature-flags|concept: oncall-handoff|concept: rollback-runbook|concept: staging-rollout",
    "concept: early-confidence-signal|concept: pair-on-first-PR|concept: smallest-real-change",
    "concept: first-build-green|concept: first-day-setup",
    "concept: solo-retro-note",
];

/// Every live region as `title|title|…` (members sorted), regions sorted among themselves. UUID-free,
/// so it is comparable across ephemeral databases and across the refactor.
async fn slug_membership(pool: &PgPool, cogmap: uuid::Uuid) -> Vec<String> {
    let mut rows: Vec<String> = sqlx::query_scalar::<_, String>(
        "SELECT string_agg(res.title, '|' ORDER BY res.title) \
         FROM kb_cogmap_regions r \
         JOIN kb_cogmap_region_members m ON m.region_id = r.id AND m.member_table = 'kb_resources' \
         JOIN kb_resources res ON res.id = m.member_id \
         WHERE r.cogmap_id = $1 AND NOT r.is_folded \
         GROUP BY r.id",
    )
    .bind(cogmap)
    .fetch_all(pool)
    .await
    .expect("membership query");
    rows.sort();
    rows
}

#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn cogmap_region_membership_is_unchanged_under_the_anchor_producer(pool: PgPool) {
    bootseed::seed_system(&pool).await.unwrap();
    let loaded = loader::load_seed(&pool, &seed()).await.unwrap();
    embed::embed_chunks(&pool).await.unwrap();

    let anchor = HomeAnchor::Cogmap(loaded.cogmap.into());
    let first = write::materialize(&pool, anchor, "telos-default", loaded.emitter.into())
        .await
        .expect("materialize");

    let membership = slug_membership(&pool, loaded.cogmap).await;
    assert_eq!(
        membership, EXPECTED_MEMBERSHIP,
        "THE REGRESSION FLOOR: the anchor-keyed producer must form exactly the regions the \
         cogmap-keyed producer formed. A diff here means the refactor CHANGED BEHAVIOR — fix the \
         producer, do not touch EXPECTED_MEMBERSHIP."
    );
    assert_eq!(
        membership.len(),
        first.regions,
        "every asserted region must be live and readable back"
    );

    // An idempotent re-materialize over an unchanged corpus must reproduce the SAME partition and the
    // same membership fingerprint (the fingerprint IS stable within one run — same uuids).
    let second =
        write::incremental_materialize(&pool, anchor, "telos-default", loaded.emitter.into())
            .await
            .expect("re-materialize");
    assert_eq!(
        first.membership_fingerprint, second.membership_fingerprint,
        "an unchanged corpus must re-materialize to the same fingerprint"
    );
    assert_eq!(
        membership,
        slug_membership(&pool, loaded.cogmap).await,
        "an unchanged corpus must re-materialize to the same partition"
    );

    // Exactly one live region set — the fold must not leave duplicates behind.
    let live: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM kb_cogmap_regions WHERE cogmap_id=$1 AND NOT is_folded",
    )
    .bind(loaded.cogmap)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        live as usize,
        membership.len(),
        "no duplicate live regions after a re-materialize"
    );
}

/// The producer must DUAL-WRITE the anchor pair alongside the vestigial `cogmap_id` (spec §3.6 M1) —
/// the pair is what the new fold predicate keys on, and `cogmap_id` is what the previous commit's code
/// still reads. A row missing either half breaks one of the two.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn the_producer_dual_writes_the_anchor_pair_and_cogmap_id(pool: PgPool) {
    bootseed::seed_system(&pool).await.unwrap();
    let loaded = loader::load_seed(&pool, &seed()).await.unwrap();
    embed::embed_chunks(&pool).await.unwrap();
    write::materialize(
        &pool,
        HomeAnchor::Cogmap(loaded.cogmap.into()),
        "telos-default",
        loaded.emitter.into(),
    )
    .await
    .expect("materialize");

    for table in ["kb_cogmap_regions", "kb_cogmap_components"] {
        let bad: i64 = sqlx::query_scalar(&format!(
            "SELECT count(*) FROM {table} WHERE NOT is_folded AND ( \
                 home_anchor_table IS DISTINCT FROM 'kb_cogmaps' \
              OR home_anchor_id IS DISTINCT FROM $1 \
              OR cogmap_id IS DISTINCT FROM $1)"
        ))
        .bind(loaded.cogmap)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(
            bad, 0,
            "{table}: every live row must carry BOTH the anchor pair and cogmap_id"
        );
    }

    // The event payload carries the pair too, and keeps cogmap_id for the pre-T3 ledger probe.
    let payload: serde_json::Value = sqlx::query_scalar(
        "SELECT e.payload FROM kb_events e JOIN kb_event_types et ON et.id = e.event_type_id \
         WHERE et.name = 'region_materialized' ORDER BY e.id DESC LIMIT 1",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(payload["home_anchor_table"], "kb_cogmaps");
    assert_eq!(payload["home_anchor_id"], loaded.cogmap.to_string());
    assert_eq!(payload["cogmap_id"], loaded.cogmap.to_string());

    // ...and the event itself is anchored at the map, which is what `_project_region_materialized`
    // reads to stamp the watermark.
    let stamped: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM kb_cogmaps WHERE id=$1 AND shape_materialized_event_id IS NOT NULL",
    )
    .bind(loaded.cogmap)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        stamped, 1,
        "the materialize event must stamp the anchor's watermark"
    );
}

/// Bug §3.9.1: `kb_cogmap_region_members.affinity` was never written, so the four readers that
/// `ORDER BY m.affinity DESC` were ordering by nothing. It must now be populated — and it must
/// actually MEAN something: a member of a multi-member region has positive average-link affinity to
/// its peers, and the ordering it induces is stable.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn member_affinity_is_persisted_and_orders_the_region(pool: PgPool) {
    bootseed::seed_system(&pool).await.unwrap();
    let loaded = loader::load_seed(&pool, &seed()).await.unwrap();
    embed::embed_chunks(&pool).await.unwrap();
    write::materialize(
        &pool,
        HomeAnchor::Cogmap(loaded.cogmap.into()),
        "telos-default",
        loaded.emitter.into(),
    )
    .await
    .expect("materialize");

    let unwritten: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM kb_cogmap_region_members m \
         JOIN kb_cogmap_regions r ON r.id = m.region_id \
         WHERE NOT r.is_folded AND m.affinity IS NULL",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        unwritten, 0,
        "every live region member must carry a persisted affinity — four readers ORDER BY it"
    );

    // In a region with peers, average-link affinity to those peers is positive: the members are there
    // BECAUSE they have nonzero affinity to each other (that is what connected_components partitions
    // on), so a zero here would mean the score is not measuring what put them in the region.
    let nonpositive: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM kb_cogmap_region_members m \
         JOIN kb_cogmap_regions r ON r.id = m.region_id \
         WHERE NOT r.is_folded AND r.member_count > 1 AND m.affinity <= 0",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        nonpositive, 0,
        "a member of a multi-member region must have positive affinity to its peers"
    );

    // A singleton region has no peers to be central to — 0.0, not NULL.
    let singleton_bad: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM kb_cogmap_region_members m \
         JOIN kb_cogmap_regions r ON r.id = m.region_id \
         WHERE NOT r.is_folded AND r.member_count = 1 AND m.affinity IS DISTINCT FROM 0.0",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(singleton_bad, 0, "a singleton member scores exactly 0.0");
}
