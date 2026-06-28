#![cfg(feature = "artifact-tests")]
//! `readback::cogmap_region_metrics` + `readback::cogmap_analytics` — the analytics read side.
//! Proves: per-region metrics surface (stored columns) for a readable principal, are gated (deny →
//! empty), are lens-filtered, and exclude folded regions; the map-level analytics row carries the
//! telos id + staleness, surfaces a readable regulation edge through the `json_agg` composition, and
//! denies a non-member (None, not an error).

use sqlx::PgPool;
use temper_substrate::ids::{CogmapId, ProfileId};
use uuid::Uuid;

mod common;

/// Insert one region with explicit metric columns (analytics reads stored columns, so seed them
/// directly — no materialization run needed). `centroid` is an all-zero 768-vector. Returns its id.
struct MetricSeed {
    cogmap: Uuid,
    lens: Uuid,
    event: Uuid,
    centrality: f64,
    internal_tension: f64,
    is_folded: bool,
}

async fn insert_region_with_metrics(pool: &PgPool, s: MetricSeed) -> Uuid {
    sqlx::query_scalar::<_, Uuid>(
        "INSERT INTO kb_cogmap_regions
           (cogmap_id, lens_id, centroid, salience, centrality, content_cohesion,
            internal_tension, reference_standing, telos_alignment, label, member_count,
            asserted_by_event_id, last_event_id, is_folded)
         VALUES ($1, $2, array_fill(0::double precision, ARRAY[768])::vector, 0.5, $3, 0.25,
            $4, 7.0, 0.9, 'r', 2, $5, $5, $6)
         RETURNING id",
    )
    .bind(s.cogmap)
    .bind(s.lens)
    .bind(s.centrality)
    .bind(s.internal_tension)
    .bind(s.event)
    .bind(s.is_folded)
    .fetch_one(pool)
    .await
    .expect("insert region with metrics")
}

/// Shared fixture: a genesis cogmap joined to a fresh team; p1 is a member (readable), p2 is not.
/// Returns (cogmap, telos, lens, event, p1, p2).
async fn fixture(pool: &PgPool) -> (Uuid, Uuid, Uuid, Uuid, Uuid, Uuid) {
    common::seed_system(pool).await;
    let (cogmap, telos) = common::genesis_cogmap(pool, "analytics-test", "Analytics Test").await;
    let team = common::create_team(pool, "analytics-team").await;
    let p1 = common::create_profile(pool, "member@example.com").await;
    let p2 = common::create_profile(pool, "outsider@example.com").await;
    common::add_team_member(pool, team, p1).await;
    sqlx::query("INSERT INTO kb_team_cogmaps (team_id, cogmap_id) VALUES ($1, $2)")
        .bind(team)
        .bind(cogmap)
        .execute(pool)
        .await
        .expect("join cogmap to team");
    let lens: Uuid = sqlx::query_scalar(
        "SELECT id FROM kb_cogmap_lenses WHERE name='telos-default' AND cogmap_id IS NULL",
    )
    .fetch_one(pool)
    .await
    .expect("global telos-default lens");
    let event: Uuid = sqlx::query_scalar("SELECT id FROM kb_events LIMIT 1")
        .fetch_one(pool)
        .await
        .expect("any event for FK");
    (cogmap, telos, lens, event, p1, p2)
}

#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn region_metrics_surface_gate_and_lens(pool: PgPool) {
    let (cogmap, _telos, lens, event, p1, p2) = fixture(&pool).await;

    let kept = insert_region_with_metrics(
        &pool,
        MetricSeed {
            cogmap,
            lens,
            event,
            centrality: 4.0,
            internal_tension: 1.5,
            is_folded: false,
        },
    )
    .await;
    let _folded = insert_region_with_metrics(
        &pool,
        MetricSeed {
            cogmap,
            lens,
            event,
            centrality: 9.0,
            internal_tension: 0.0,
            is_folded: true,
        },
    )
    .await;

    // Readable principal sees exactly the non-folded region, with the stored scalars.
    let rows = temper_substrate::readback::cogmap_region_metrics(
        &pool,
        CogmapId::from(cogmap),
        ProfileId::from(p1),
        None,
    )
    .await
    .expect("readable read");
    assert_eq!(
        rows.len(),
        1,
        "only the non-folded region surfaces: {rows:?}"
    );
    assert_eq!(rows[0].region_id, kept);
    assert_eq!(rows[0].centrality, Some(4.0));
    assert_eq!(
        rows[0].internal_tension,
        Some(1.5),
        "tension surfaces from the stored column"
    );
    assert_eq!(rows[0].reference_standing, Some(7.0));

    // Non-member is denied by the in-SQL gate: zero rows, not an error.
    let denied = temper_substrate::readback::cogmap_region_metrics(
        &pool,
        CogmapId::from(cogmap),
        ProfileId::from(p2),
        None,
    )
    .await
    .expect("gate denial is empty, not an error");
    assert!(denied.is_empty(), "non-member sees no metrics: {denied:?}");

    // Wrong lens narrows to empty.
    let filtered = temper_substrate::readback::cogmap_region_metrics(
        &pool,
        CogmapId::from(cogmap),
        ProfileId::from(p1),
        Some(Uuid::now_v7()),
    )
    .await
    .expect("lens-filtered read");
    assert!(
        filtered.is_empty(),
        "wrong lens yields no metrics: {filtered:?}"
    );
}

#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn analytics_telos_staleness_regulation_and_deny(pool: PgPool) {
    let (cogmap, telos, _lens, event, p1, p2) = fixture(&pool).await;

    // Seed a readable regulation edge: a target resource OWNED by p1 (→ visible via
    // resources_visible_to), and an `express` edge telos → target labeled `operationalized_by`.
    let target: Uuid = sqlx::query_scalar(
        "INSERT INTO kb_resources (title, origin_uri) VALUES ('Deploy safely', 'temper://reg/t') RETURNING id",
    )
    .fetch_one(&pool)
    .await
    .expect("insert target resource");
    sqlx::query(
        "INSERT INTO kb_resource_homes
           (resource_id, anchor_table, anchor_id, originator_profile_id, owner_profile_id)
         VALUES ($1, 'kb_cogmaps', $2, $3, $3)",
    )
    .bind(target)
    .bind(cogmap)
    .bind(p1)
    .execute(&pool)
    .await
    .expect("home target to p1");
    sqlx::query(
        "INSERT INTO kb_edges
           (source_table, source_id, target_table, target_id, edge_kind, label,
            home_anchor_table, home_anchor_id, asserted_by_event_id, last_event_id)
         VALUES ('kb_resources', $1, 'kb_resources', $2, 'express', 'operationalized_by',
            'kb_cogmaps', $3, $4, $4)",
    )
    .bind(telos)
    .bind(target)
    .bind(cogmap)
    .bind(event)
    .execute(&pool)
    .await
    .expect("insert express edge");

    // Readable principal: telos id, staleness present, regulation carries the one readable concept.
    let got = temper_substrate::readback::cogmap_analytics(
        &pool,
        CogmapId::from(cogmap),
        ProfileId::from(p1),
    )
    .await
    .expect("readable analytics read")
    .expect("readable principal gets Some");
    assert_eq!(got.telos_resource_id, telos);
    assert!(
        got.staleness.is_stale,
        "never-materialized map reads as stale"
    );
    assert_eq!(
        got.regulation.len(),
        1,
        "one readable regulation concept: {:?}",
        got.regulation
    );
    assert_eq!(got.regulation[0].resource_id, target);
    assert_eq!(got.regulation[0].edge_label, "operationalized_by");
    assert_eq!(got.regulation[0].title, "Deploy safely");

    // Non-member: the in-SQL gate yields zero rows → None.
    let denied = temper_substrate::readback::cogmap_analytics(
        &pool,
        CogmapId::from(cogmap),
        ProfileId::from(p2),
    )
    .await
    .expect("gate denial is None, not an error");
    assert!(denied.is_none(), "non-member must get None");
}
