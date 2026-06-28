#![cfg(feature = "artifact-tests")]
//! `readback::cogmap_shape` — the surface-tier region read. Proves: non-folded regions surface for a
//! readable principal; the in-SQL access gate (`cogmap_readable_by_profile`) denies a non-member
//! (zero rows, not an error); folded regions are excluded; the lens filter narrows by lens.

use sqlx::PgPool;
use temper_substrate::ids::{CogmapId, ProfileId};
use uuid::Uuid;

mod common;

/// One region to seed. A params struct (not a long arg list) per the >5-domain-args rule. `cogmap`/
/// `lens`/`event` are the shared fixture context; `salience`/`label`/`member_count`/`is_folded` vary
/// per region.
struct RegionSeed<'a> {
    cogmap: Uuid,
    lens: Uuid,
    /// An arbitrary existing event id, reused for both NOT NULL event FKs.
    event: Uuid,
    salience: f64,
    label: &'a str,
    member_count: i32,
    is_folded: bool,
}

/// Insert one region from a `RegionSeed`. `centroid` is an all-zero 768-vector (cogmap_shape never
/// reads it). Returns the new region id.
async fn insert_region(pool: &PgPool, seed: RegionSeed<'_>) -> Uuid {
    sqlx::query_scalar::<_, Uuid>(
        "INSERT INTO kb_cogmap_regions
           (cogmap_id, lens_id, centroid, salience, content_cohesion, label, member_count,
            asserted_by_event_id, last_event_id, is_folded)
         VALUES ($1, $2, array_fill(0::double precision, ARRAY[768])::vector, $3, NULL, $4, $5, $6, $6, $7)
         RETURNING id",
    )
    .bind(seed.cogmap)
    .bind(seed.lens)
    .bind(seed.salience)
    .bind(seed.label)
    .bind(seed.member_count)
    .bind(seed.event)
    .bind(seed.is_folded)
    .fetch_one(pool)
    .await
    .expect("insert region")
}

#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn cogmap_shape_surfaces_unfolded_regions_and_gates_by_readability(pool: PgPool) {
    common::seed_system(&pool).await; // boot the canonical `system` actor (see common/mod.rs)

    // Genesis a cogmap (creates the cogmap + telos + events). Reuse the helper pattern in common/mod.rs.
    let (cogmap, _telos) = common::genesis_cogmap(&pool, "shape-test", "Shape Test").await;

    // A fresh NON-root team + two profiles: P1 a member (readable), P2 not (denied).
    let team = common::create_team(&pool, "shape-team").await;
    let p1 = common::create_profile(&pool, "member@example.com").await;
    let p2 = common::create_profile(&pool, "outsider@example.com").await;
    common::add_team_member(&pool, team, p1).await;
    sqlx::query("INSERT INTO kb_team_cogmaps (team_id, cogmap_id) VALUES ($1, $2)")
        .bind(team)
        .bind(cogmap)
        .execute(&pool)
        .await
        .expect("join cogmap to team");

    // Global telos-default lens (cogmap_id IS NULL), seeded by bootseed.
    let lens: Uuid = sqlx::query_scalar(
        "SELECT id FROM kb_cogmap_lenses WHERE name='telos-default' AND cogmap_id IS NULL",
    )
    .fetch_one(&pool)
    .await
    .expect("global telos-default lens");
    let event: Uuid = sqlx::query_scalar("SELECT id FROM kb_events LIMIT 1")
        .fetch_one(&pool)
        .await
        .expect("any event for FK");

    let kept = insert_region(
        &pool,
        RegionSeed {
            cogmap,
            lens,
            event,
            salience: 0.9,
            label: "kept",
            member_count: 3,
            is_folded: false,
        },
    )
    .await;
    let _folded = insert_region(
        &pool,
        RegionSeed {
            cogmap,
            lens,
            event,
            salience: 0.8,
            label: "folded-out",
            member_count: 2,
            is_folded: true,
        },
    )
    .await;

    // Readable principal sees exactly the one non-folded region.
    let rows = temper_substrate::readback::cogmap_shape(
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
    assert_eq!(rows[0].label.as_deref(), Some("kept"));
    assert_eq!(rows[0].member_count, 3);
    assert_eq!(rows[0].content_cohesion, None);

    // Non-member is denied by the in-SQL gate: zero rows, NOT an error.
    let denied = temper_substrate::readback::cogmap_shape(
        &pool,
        CogmapId::from(cogmap),
        ProfileId::from(p2),
        None,
    )
    .await
    .expect("gate denial is empty, not an error");
    assert!(
        denied.is_empty(),
        "non-member must see no regions: {denied:?}"
    );

    // Lens filter: a non-matching lens id narrows to empty for the readable principal.
    let other_lens = Uuid::now_v7();
    let filtered = temper_substrate::readback::cogmap_shape(
        &pool,
        CogmapId::from(cogmap),
        ProfileId::from(p1),
        Some(other_lens),
    )
    .await
    .expect("lens-filtered read");
    assert!(
        filtered.is_empty(),
        "wrong lens yields no regions: {filtered:?}"
    );
}
