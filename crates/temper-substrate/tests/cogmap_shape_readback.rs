#![cfg(feature = "artifact-tests")]
//! `readback::cogmap_shape` — the surface-tier region read. Proves: non-folded regions surface for a
//! readable principal; the in-SQL access gate (`cogmap_readable_by_profile`) denies a non-member
//! (zero rows, not an error); folded regions are excluded; the lens filter narrows by lens; and
//! (D5) the member count is over VISIBLE members only, on the COGMAP anchor kind.

use sqlx::PgPool;
use temper_substrate::ids::{CogmapId, LensId, ProfileId, RegionId};
use uuid::Uuid;

mod common;

/// A resource homed in `cogmap`. A profile who can reach the cogmap's team can read it
/// (`resources_visible_to` → "resources homed in a cognitive map joined to a REACHABLE team").
async fn insert_cogmap_resource(pool: &PgPool, cogmap: Uuid, owner: Uuid, title: &str) -> Uuid {
    let id: Uuid = sqlx::query_scalar(
        "INSERT INTO kb_resources (title, origin_uri) VALUES ($1,'') RETURNING id",
    )
    .bind(title)
    .fetch_one(pool)
    .await
    .expect("insert resource");
    sqlx::query(
        "INSERT INTO kb_resource_homes \
           (resource_id, anchor_table, anchor_id, originator_profile_id, owner_profile_id) \
         VALUES ($1, 'kb_cogmaps', $2, $3, $3)",
    )
    .bind(id)
    .bind(cogmap)
    .bind(owner)
    .execute(pool)
    .await
    .expect("home resource in cogmap");
    id
}

async fn add_member(pool: &PgPool, region: Uuid, resource: Uuid, affinity: f64) {
    sqlx::query(
        "INSERT INTO kb_cogmap_region_members (region_id, member_table, member_id, affinity) \
         VALUES ($1, 'kb_resources', $2, $3)",
    )
    .bind(region)
    .bind(resource)
    .bind(affinity)
    .execute(pool)
    .await
    .expect("add region member");
}

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
        // The anchor pair is written alongside the vestigial `cogmap_id` because that is what the real
        // producer writes (M1 dual-write) and what the reads are keyed on. A fixture that set only
        // `cogmap_id` would fabricate a row shape the system can no longer produce — prod carries zero
        // such rows — and the anchor-keyed reads would (correctly) not see it.
        "INSERT INTO kb_cogmap_regions
           (cogmap_id, home_anchor_table, home_anchor_id, lens_id, centroid, salience,
            content_cohesion, label, member_count, asserted_by_event_id, last_event_id, is_folded)
         VALUES ($1, 'kb_cogmaps', $1, $2, array_fill(0::double precision, ARRAY[768])::vector, $3,
            NULL, $4, $5, $6, $6, $7)
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

    // Three real members, homed in the cogmap: p1 reaches the cogmap's team, so p1 can read all three.
    // (The region's STORED member_count is 3 — so a fully-sighted read must return exactly that.)
    let system: Uuid = sqlx::query_scalar("SELECT id FROM kb_profiles WHERE handle='system'")
        .fetch_one(&pool)
        .await
        .expect("system profile");
    for (i, affinity) in [0.9_f64, 0.5, 0.1].iter().enumerate() {
        let r = insert_cogmap_resource(&pool, cogmap, system, &format!("member-{i}")).await;
        add_member(&pool, kept, r, *affinity).await;
    }

    // Readable principal sees exactly the one non-folded region.
    let rows = temper_substrate::readback::anchor_shape(
        &pool,
        temper_core::types::home::HomeAnchor::Cogmap(CogmapId::from(cogmap)),
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
    assert_eq!(rows[0].region_id, RegionId::from(kept));
    assert_eq!(rows[0].label.as_deref(), Some("kept"));
    assert_eq!(
        rows[0].member_count, 3,
        "a caller who can see every member is handed the stored count, unchanged (D5 differential)"
    );
    assert_eq!(rows[0].content_cohesion, None);

    // Non-member is denied by the in-SQL gate: zero rows, NOT an error.
    let denied = temper_substrate::readback::anchor_shape(
        &pool,
        temper_core::types::home::HomeAnchor::Cogmap(CogmapId::from(cogmap)),
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
    let filtered = temper_substrate::readback::anchor_shape(
        &pool,
        temper_core::types::home::HomeAnchor::Cogmap(CogmapId::from(cogmap)),
        ProfileId::from(p1),
        Some(LensId::from(other_lens)),
    )
    .await
    .expect("lens-filtered read");
    assert!(
        filtered.is_empty(),
        "wrong lens yields no regions: {filtered:?}"
    );
}

/// D5 on the COGMAP anchor kind: the count is over VISIBLE members only.
///
/// The context half of this lives in `temper-api/tests/context_orientation_test.rs`. Both anchor kinds
/// go through the same `anchor_shape`, but they reach visibility by different arms of
/// `resources_visible_to` (cogmap-joined-to-a-reachable-team vs. homed-in-a-readable-context), so a
/// fix proven on one is not proven on the other.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn the_member_count_is_over_visible_members_only(pool: PgPool) {
    common::seed_system(&pool).await;

    // Two cogmaps. p1's team is joined to `mine` only — so what is homed in `theirs` is unreadable.
    let (mine, _) = common::genesis_cogmap(&pool, "mine", "Mine").await;
    let (theirs, _) = common::genesis_cogmap(&pool, "theirs", "Theirs").await;
    let team = common::create_team(&pool, "count-team").await;
    let p1 = common::create_profile(&pool, "member@example.com").await;
    common::add_team_member(&pool, team, p1).await;
    sqlx::query("INSERT INTO kb_team_cogmaps (team_id, cogmap_id) VALUES ($1, $2)")
        .bind(team)
        .bind(mine)
        .execute(&pool)
        .await
        .expect("join MY cogmap to the team");

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
    let system: Uuid = sqlx::query_scalar("SELECT id FROM kb_profiles WHERE handle='system'")
        .fetch_one(&pool)
        .await
        .expect("system profile");

    // The region stores member_count = 4: what materialize wrote, having seen all four members.
    let region = insert_region(
        &pool,
        RegionSeed {
            cogmap: mine,
            lens,
            event,
            salience: 0.9,
            label: "mixed-visibility",
            member_count: 4,
            is_folded: false,
        },
    )
    .await;

    // Two members p1 can read, and two they cannot: one homed in the OTHER cogmap (what a re-home
    // leaves behind), one soft-deleted (invisible on every axis, per the read floor).
    let seen_a = insert_cogmap_resource(&pool, mine, system, "visible one").await;
    let seen_b = insert_cogmap_resource(&pool, mine, system, "visible two").await;
    let unseen = insert_cogmap_resource(&pool, theirs, system, "SECRET in another map").await;
    let deleted = insert_cogmap_resource(&pool, mine, system, "deleted since materialize").await;
    sqlx::query("UPDATE kb_resources SET is_active = false WHERE id = $1")
        .bind(deleted)
        .execute(&pool)
        .await
        .expect("soft-delete");

    add_member(&pool, region, unseen, 0.99).await; // most affine, and unreadable
    add_member(&pool, region, deleted, 0.98).await; // next most affine, and gone
    add_member(&pool, region, seen_a, 0.50).await;
    add_member(&pool, region, seen_b, 0.10).await;

    let rows = temper_substrate::readback::anchor_shape(
        &pool,
        temper_core::types::home::HomeAnchor::Cogmap(CogmapId::from(mine)),
        ProfileId::from(p1),
        None,
    )
    .await
    .expect("readable read");

    assert_eq!(rows.len(), 1, "the region surfaces: {rows:?}");
    assert_eq!(
        rows[0].member_count, 2,
        "stored count is 4 and four member rows exist, but this caller can read exactly two — \
         telling them 4 discloses the cardinality of content they have no read on"
    );
}
