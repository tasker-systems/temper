#![cfg(feature = "artifact-tests")]
//! `readback::anchor_shape` — the surface tier plus its anchor-level envelope. Proves each of the
//! five outcomes the envelope distinguishes, including the two that were byte-identical before it.
//!
//! Every case is built on the COGMAP anchor kind, because that is the arm the substrate fixtures
//! already reach (`common::genesis_cogmap` + `kb_team_cogmaps`, as
//! `cogmap_shape_readback.rs:96-110` establishes) — the one exception is the absent-anchor case,
//! which needs an id no row carries and so uses a context anchor. The envelope is computed once,
//! above the anchor-kind switch, so an outcome proven on one kind is the same code on the other;
//! the *visibility* arms differ per kind and are proven per kind in `cogmap_shape_readback.rs` and
//! `temper-api/tests/context_orientation_test.rs`.

use sqlx::PgPool;
use temper_core::types::home::HomeAnchor;
use temper_substrate::ids::{CogmapId, ContextId, LensId, ProfileId};
use uuid::Uuid;

mod common;

/// A resource homed in `cogmap`. A profile who can reach the cogmap's team can read it
/// (`resources_visible_to` → "resources homed in a cognitive map joined to a REACHABLE team").
/// Modelled on `cogmap_shape_readback.rs:15-33`.
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

/// One region to seed, homed on a COGMAP anchor. Params struct rather than a long arg list, as in
/// `cogmap_shape_readback.rs:52-61`.
struct RegionSeed {
    cogmap: Uuid,
    lens: Uuid,
    /// An arbitrary existing event id, reused for both NOT NULL event FKs.
    event: Uuid,
    salience: f64,
    member_count: i32,
}

/// Insert one region from a `RegionSeed`. `centroid` is an all-zero 768-vector (the shape read never
/// reads it). The anchor pair is written alongside the vestigial `cogmap_id` because that is what
/// the real producer writes (M1 dual-write) and what the reads are keyed on.
async fn insert_region(pool: &PgPool, seed: RegionSeed) -> Uuid {
    sqlx::query_scalar::<_, Uuid>(
        "INSERT INTO kb_cogmap_regions
           (cogmap_id, home_anchor_table, home_anchor_id, lens_id, centroid, salience,
            content_cohesion, label, member_count, asserted_by_event_id, last_event_id, is_folded)
         VALUES ($1, 'kb_cogmaps', $1, $2, array_fill(0::double precision, ARRAY[768])::vector, $3,
            NULL, 'seeded', $4, $5, $5, false)
         RETURNING id",
    )
    .bind(seed.cogmap)
    .bind(seed.lens)
    .bind(seed.salience)
    .bind(seed.member_count)
    .bind(seed.event)
    .fetch_one(pool)
    .await
    .expect("insert region")
}

/// Stamp the FORMATION watermark the envelope's clock reads. `cogmap_genesis` leaves
/// `shape_materialized_event_id` NULL (`20260624000002_canonical_functions.sql:689` inserts only
/// `id, name, telos_resource_id, created`), so an anchor is never-clustered until this runs — which
/// is exactly the premise `a_never_clustered_anchor_says_so` rests on.
async fn mark_materialized(pool: &PgPool, cogmap: Uuid, event: Uuid) {
    sqlx::query("UPDATE kb_cogmaps SET shape_materialized_event_id = $2 WHERE id = $1")
        .bind(cogmap)
        .bind(event)
        .execute(pool)
        .await
        .expect("stamp shape watermark");
}

/// Two cogmaps and two profiles. `reader`'s team is joined to `mine` only, so what is homed in
/// `theirs` is unreadable to them, and `outsider` is on no team at all — the same two-cogmap shape
/// `cogmap_shape_readback.rs:220-240` uses to build a visibility boundary through a region.
struct Fx {
    /// The anchor under test: readable by `reader`.
    mine: Uuid,
    /// An anchor `reader` cannot read, used to home members that must be invisible.
    theirs: Uuid,
    reader: Uuid,
    outsider: Uuid,
    /// The boot-seeded global `telos-default` lens (`cogmap_id IS NULL`).
    lens: Uuid,
    event: Uuid,
    system: Uuid,
}

async fn fixture(pool: &PgPool) -> Fx {
    common::seed_system(pool).await; // boot the canonical `system` actor (see common/mod.rs)

    let (mine, _) = common::genesis_cogmap(pool, "mine", "Mine").await;
    let (theirs, _) = common::genesis_cogmap(pool, "theirs", "Theirs").await;

    let team = common::create_team(pool, "envelope-team").await;
    let reader = common::create_profile(pool, "reader@example.com").await;
    let outsider = common::create_profile(pool, "outsider@example.com").await;
    common::add_team_member(pool, team, reader).await;
    sqlx::query("INSERT INTO kb_team_cogmaps (team_id, cogmap_id) VALUES ($1, $2)")
        .bind(team)
        .bind(mine)
        .execute(pool)
        .await
        .expect("join MY cogmap to the team");

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
    let system: Uuid = sqlx::query_scalar("SELECT id FROM kb_profiles WHERE handle='system'")
        .fetch_one(pool)
        .await
        .expect("system profile");

    Fx {
        mine,
        theirs,
        reader,
        outsider,
        lens,
        event,
        system,
    }
}

fn anchor(cogmap: Uuid) -> HomeAnchor {
    HomeAnchor::Cogmap(CogmapId::from(cogmap))
}

#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn a_never_clustered_anchor_says_so(pool: PgPool) {
    // An anchor the principal CAN read, with shape_materialized_event_id NULL.
    let fx = fixture(&pool).await;

    let out = temper_substrate::readback::anchor_shape(
        &pool,
        anchor(fx.mine),
        ProfileId::from(fx.reader),
        None,
    )
    .await
    .expect("readable read");

    assert!(
        out.regions.is_empty(),
        "no regions were ever formed: {out:?}"
    );
    assert_eq!(out.population, 0);
    assert_eq!(out.emptiness.as_deref(), Some("never_clustered"));
    assert!(out.materialized_at.is_none());
}

#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn nothing_visible_is_not_never_clustered(pool: PgPool) {
    // Readable anchor, materialized, but every region's members are invisible to this principal.
    // THIS IS THE PAIR THE OLD READ COULD NOT SEPARATE — both were `[]`.
    let fx = fixture(&pool).await;
    mark_materialized(&pool, fx.mine, fx.event).await;

    let region = insert_region(
        &pool,
        RegionSeed {
            cogmap: fx.mine,
            lens: fx.lens,
            event: fx.event,
            salience: 0.9,
            member_count: 2,
        },
    )
    .await;
    // Both members are homed in the OTHER cogmap, which `reader`'s team is not joined to.
    for (i, affinity) in [0.9_f64, 0.8].iter().enumerate() {
        let hidden =
            insert_cogmap_resource(&pool, fx.theirs, fx.system, &format!("SECRET {i}")).await;
        add_member(&pool, region, hidden, *affinity).await;
    }

    let out = temper_substrate::readback::anchor_shape(
        &pool,
        anchor(fx.mine),
        ProfileId::from(fx.reader),
        None,
    )
    .await
    .expect("readable read");

    assert!(
        out.regions.is_empty(),
        "a region with no visible members is not returned: {out:?}"
    );
    assert_eq!(out.emptiness.as_deref(), Some("nothing_visible"));
    assert_eq!(out.population, 0);
    assert!(
        out.materialized_at.is_some(),
        "the clock is disclosed to a reader: {out:?}"
    );
}

#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn an_unreadable_anchor_discloses_neither_population_nor_clock(pool: PgPool) {
    // Materialized, populated, but this principal cannot read the anchor at all.
    let fx = fixture(&pool).await;
    mark_materialized(&pool, fx.mine, fx.event).await;

    let region = insert_region(
        &pool,
        RegionSeed {
            cogmap: fx.mine,
            lens: fx.lens,
            event: fx.event,
            salience: 0.9,
            member_count: 1,
        },
    )
    .await;
    let member = insert_cogmap_resource(&pool, fx.mine, fx.system, "a member").await;
    add_member(&pool, region, member, 0.9).await;

    let out = temper_substrate::readback::anchor_shape(
        &pool,
        anchor(fx.mine),
        ProfileId::from(fx.outsider),
        None,
    )
    .await
    .expect("gate denial is an envelope, not an error");

    assert!(out.regions.is_empty(), "deny returns no regions: {out:?}");
    assert_eq!(out.emptiness.as_deref(), Some("unreadable_or_absent"));
    assert_eq!(
        out.population, 0,
        "must not leak the size of an anchor it cannot read: {out:?}"
    );
    assert!(
        out.materialized_at.is_none(),
        "must not leak the clock either: {out:?}"
    );
}

#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn an_absent_anchor_is_indistinguishable_from_an_unreadable_one(pool: PgPool) {
    let fx = fixture(&pool).await;

    let absent = HomeAnchor::Context(ContextId::from(Uuid::now_v7()));
    let out =
        temper_substrate::readback::anchor_shape(&pool, absent, ProfileId::from(fx.reader), None)
            .await
            .expect("an absent anchor is an envelope, not an error");

    assert!(out.regions.is_empty());
    assert_eq!(out.emptiness.as_deref(), Some("unreadable_or_absent"));
    assert_eq!(out.population, 0);
    assert!(out.materialized_at.is_none());
}

#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn population_is_all_lenses_while_rows_are_lens_narrowed(pool: PgPool) {
    // Two regions under two DIFFERENT lenses, both visible. Read with one lens.
    let fx = fixture(&pool).await;
    mark_materialized(&pool, fx.mine, fx.event).await;

    // A second lens, cloned in spirit from `cogmap_wayfind_scope.rs:441-449`. Only its id matters
    // here — the shape read filters on `lens_id`, it never evaluates the weights.
    let other_lens: Uuid = sqlx::query_scalar(
        "INSERT INTO kb_cogmap_lenses
           (name, w_express, w_contains, w_leads_to, w_near, w_prop,
            s_telos, s_ref, s_central, resolution, asserted_by_event_id)
         VALUES ('second-lens', 0,0,0,0,0, 0.0, 0.0, 1.0, 1.0, $1)
         RETURNING id",
    )
    .bind(fx.event)
    .fetch_one(&pool)
    .await
    .expect("insert second lens");

    for (lens, salience) in [(fx.lens, 0.9_f64), (other_lens, 0.8)] {
        let region = insert_region(
            &pool,
            RegionSeed {
                cogmap: fx.mine,
                lens,
                event: fx.event,
                salience,
                member_count: 1,
            },
        )
        .await;
        let member =
            insert_cogmap_resource(&pool, fx.mine, fx.system, &format!("member {salience}")).await;
        add_member(&pool, region, member, 0.9).await;
    }

    let out = temper_substrate::readback::anchor_shape(
        &pool,
        anchor(fx.mine),
        ProfileId::from(fx.reader),
        Some(LensId::from(fx.lens)),
    )
    .await
    .expect("lens-filtered read");

    assert_eq!(out.regions.len(), 1, "the lens narrows the rows: {out:?}");
    assert_eq!(
        out.population, 2,
        "the denominator is ALL lenses — this is what makes it a denominator: {out:?}"
    );
    assert_eq!(
        out.emptiness, None,
        "the row set is non-empty, so there is nothing to explain: {out:?}"
    );
}

#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn a_lens_that_matches_nothing_says_so_rather_than_going_silent(pool: PgPool) {
    // Regions exist and are visible, but none under the requested lens.
    let fx = fixture(&pool).await;
    mark_materialized(&pool, fx.mine, fx.event).await;

    let region = insert_region(
        &pool,
        RegionSeed {
            cogmap: fx.mine,
            lens: fx.lens,
            event: fx.event,
            salience: 0.9,
            member_count: 1,
        },
    )
    .await;
    let member = insert_cogmap_resource(&pool, fx.mine, fx.system, "a visible member").await;
    add_member(&pool, region, member, 0.9).await;

    let out = temper_substrate::readback::anchor_shape(
        &pool,
        anchor(fx.mine),
        ProfileId::from(fx.reader),
        Some(LensId::from(Uuid::now_v7())),
    )
    .await
    .expect("lens-filtered read");

    assert!(out.regions.is_empty(), "no region under that lens: {out:?}");
    assert!(
        out.population > 0,
        "the anchor is not empty — the lens is: {out:?}"
    );
    assert_eq!(out.emptiness.as_deref(), Some("lens_narrowed"));
}

#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn a_populated_read_carries_no_emptiness(pool: PgPool) {
    let fx = fixture(&pool).await;
    mark_materialized(&pool, fx.mine, fx.event).await;

    let region = insert_region(
        &pool,
        RegionSeed {
            cogmap: fx.mine,
            lens: fx.lens,
            event: fx.event,
            salience: 0.9,
            member_count: 2,
        },
    )
    .await;
    for (i, affinity) in [0.9_f64, 0.8].iter().enumerate() {
        let member =
            insert_cogmap_resource(&pool, fx.mine, fx.system, &format!("member {i}")).await;
        add_member(&pool, region, member, *affinity).await;
    }

    let out = temper_substrate::readback::anchor_shape(
        &pool,
        anchor(fx.mine),
        ProfileId::from(fx.reader),
        None,
    )
    .await
    .expect("readable read");

    assert!(!out.regions.is_empty(), "the region surfaces: {out:?}");
    assert_eq!(out.emptiness, None);
    assert_eq!(
        out.population as usize,
        out.regions.len(),
        "without a lens filter the denominator equals the row count: {out:?}"
    );
}

#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn a_lens_narrowed_read_of_a_never_materialized_anchor_is_not_never_clustered(pool: PgPool) {
    // The two conditions that used to collide: regions exist and are VISIBLE (so `population > 0`),
    // and the anchor's `shape_materialized_event_id` is NULL. Deliberately NO `mark_materialized`.
    //
    // The clock arm used to be evaluated before the row-count arms, so it fired on the NULL
    // watermark alone and returned `population: 1` alongside `never_clustered` — an anchor
    // simultaneously non-empty and never-clustered. `ShapeEmptiness::LensNarrowed`'s own contract
    // ("`population` is > 0 — the caller is looking at a narrowed view, not an empty anchor") is met
    // here, and it is the arm that must fire: the caller's fix is to drop `--lens`, not to run
    // `context materialize`.
    let fx = fixture(&pool).await;

    let region = insert_region(
        &pool,
        RegionSeed {
            cogmap: fx.mine,
            lens: fx.lens,
            event: fx.event,
            salience: 0.9,
            member_count: 1,
        },
    )
    .await;
    let member = insert_cogmap_resource(&pool, fx.mine, fx.system, "a visible member").await;
    add_member(&pool, region, member, 0.9).await;

    let out = temper_substrate::readback::anchor_shape(
        &pool,
        anchor(fx.mine),
        ProfileId::from(fx.reader),
        Some(LensId::from(Uuid::now_v7())),
    )
    .await
    .expect("lens-filtered read");

    assert!(out.regions.is_empty(), "no region under that lens: {out:?}");
    assert!(
        out.population > 0,
        "the anchor holds a visible region — it is the lens that is empty: {out:?}"
    );
    assert_eq!(
        out.emptiness.as_deref(),
        Some("lens_narrowed"),
        "a non-empty population must never be reported as never_clustered: {out:?}"
    );
    // The clock fact is not lost by the arm that did not fire — it is carried by the field that is
    // actually about the clock.
    assert!(
        out.materialized_at.is_none(),
        "the anchor really was never materialized; that is `materialized_at`'s job to say: {out:?}"
    );
}

#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn population_equals_the_row_count_when_the_lens_matches_every_region(pool: PgPool) {
    // `population` is the ALL-LENS denominator, so under a lens filter it is >= the row count —
    // NOT strictly greater. An anchor whose every region sits under the requested lens returns
    // equality, and that is the ordinary case for an anchor with one lens. Pinned here so the
    // relation is asserted by a test rather than restated in prose.
    let fx = fixture(&pool).await;
    mark_materialized(&pool, fx.mine, fx.event).await;

    for (i, salience) in [0.9_f64, 0.7].iter().enumerate() {
        let region = insert_region(
            &pool,
            RegionSeed {
                cogmap: fx.mine,
                lens: fx.lens,
                event: fx.event,
                salience: *salience,
                member_count: 1,
            },
        )
        .await;
        let member =
            insert_cogmap_resource(&pool, fx.mine, fx.system, &format!("member {i}")).await;
        add_member(&pool, region, member, 0.9).await;
    }

    let out = temper_substrate::readback::anchor_shape(
        &pool,
        anchor(fx.mine),
        ProfileId::from(fx.reader),
        Some(LensId::from(fx.lens)),
    )
    .await
    .expect("lens-filtered read");

    assert_eq!(
        out.regions.len(),
        2,
        "both regions are under that lens: {out:?}"
    );
    assert_eq!(
        out.population as usize,
        out.regions.len(),
        "a lens matching every region leaves the denominator EQUAL to the row count: {out:?}"
    );
    assert_eq!(
        out.emptiness, None,
        "the row set is non-empty, so there is nothing to explain: {out:?}"
    );
}

/// **The cogmap self-read arm must not admit a uuid no map carries.**
///
/// That disjunct is a tautology over two values the CALLER supplies — `p_principal_id =
/// p_anchor_id` — so before the `EXISTS` on `kb_cogmaps` it verified nothing. That was harmless
/// while this function returned a bare row set, because a fabricated uuid and a real-but-empty map
/// both answered a byte-identical `[]`. The envelope changed it: the arm began answering
/// `never_clustered`, a fact about an anchor, and for a materialized map it would have disclosed the
/// clock — an existence-and-clock oracle over any uuid, from a gate that checked nothing.
///
/// Not reachable through `readback::anchor_shape`, which hardcodes `'profile'`, so this goes to the
/// SQL directly — which is exactly the surface the hole was on. Runtime-checked `query_as` rather
/// than the macro, so this needs no offline-cache entry.
///
/// Both halves matter: the fabrication must be refused, AND a real map reading itself must still be
/// admitted. A gate that closed the first by breaking the second would pass a one-sided test.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn a_fabricated_cogmap_identity_is_refused_by_the_self_read_arm(pool: PgPool) {
    let fx = fixture(&pool).await;

    let invented = Uuid::from_u128(0xffff_ffff_ffff_4fff_8fff_ffff_ffff_ffffu128);
    let (population, emptiness): (i32, Option<String>) = sqlx::query_as(
        "SELECT population, emptiness \
           FROM anchor_shape('kb_cogmaps', $1, 'cogmap', $1, NULL)",
    )
    .bind(invented)
    .fetch_one(&pool)
    .await
    .expect("the sentinel row is returned even for an anchor that does not exist");

    assert_eq!(
        emptiness.as_deref(),
        Some("unreadable_or_absent"),
        "a uuid no kb_cogmaps row carries must disclose nothing, not answer a fact about an anchor",
    );
    assert_eq!(population, 0, "and it must disclose no population either");

    // The legitimate path is unchanged: a real map reading its own shape still passes the gate.
    let (_, mine): (i32, Option<String>) = sqlx::query_as(
        "SELECT population, emptiness \
           FROM anchor_shape('kb_cogmaps', $1, 'cogmap', $1, NULL)",
    )
    .bind(fx.mine)
    .fetch_one(&pool)
    .await
    .expect("a real cogmap self-read");

    assert_ne!(
        mine.as_deref(),
        Some("unreadable_or_absent"),
        "the EXISTS must not lock a real map out of its own shape",
    );
}
