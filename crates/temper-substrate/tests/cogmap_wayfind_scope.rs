#![cfg(feature = "artifact-tests")]
//! `readback::wayfind_scope_ids` — Surface B Half 2's lens-driven region-salience scope funnel
//! (spec §4/§5/§7). Determinism: region rows are inserted DIRECTLY with hand-chosen
//! centroids/salience/components (no `materialize_cogmap`, no ONNX), so the
//! `region_score = α·salience_norm + β·query_centroid_cosine` blend is exactly predictable.
//!
//! Proves: top-N region selection (1); the §9 regression — a sparse high-cosine region beats a large
//! high-salience low-cosine one, i.e. relevance buys a top-N slot (2); cold-start — a region-less map
//! degrades to its direct homed scope, never errors (3); deny — a non-member of the map's team gets
//! zero ids (4); lens override recomputes salience from the stored components, reordering selection (5).

use sqlx::PgPool;
use temper_core::types::ids::{LensId, ProfileId};
use temper_substrate::readback::{wayfind_scope_ids, WayfindScopeQuery};
use uuid::Uuid;

mod common;

/// Build a 768-dim pgvector text literal with the given `(index, value)` entries; all others zero.
/// The query embedding points along axis 0, so a centroid with mass on axis 0 has high query-cosine
/// and one on axis 1 has zero — fully controllable cosine.
fn vec768(entries: &[(usize, f64)]) -> String {
    let mut v = vec![0.0_f64; 768];
    for &(i, x) in entries {
        v[i] = x;
    }
    let mut s = String::with_capacity(768 * 4 + 2);
    s.push('[');
    for (i, x) in v.iter().enumerate() {
        if i > 0 {
            s.push(',');
        }
        s.push_str(&x.to_string());
    }
    s.push(']');
    s
}

/// Query embedding pointing along axis 0 (cosine 1 to an axis-0 centroid, 0 to an axis-1 centroid).
fn query_axis0() -> Vec<f32> {
    let mut q = vec![0.0_f32; 768];
    q[0] = 1.0;
    q
}

/// Shared fixture: a genesis cogmap joined to a fresh team; `p1` is a member (its maps are visible),
/// `p2` is not (deny). `sys` (the boot-seeded system profile) owns the seeded member resources — they
/// are visible to `p1` purely through the A0 cogmap-membership read clause, which is what the funnel
/// dereferences members through.
struct Fx {
    cogmap: Uuid,
    lens: Uuid,
    event: Uuid,
    p1: Uuid,
    p2: Uuid,
    sys: Uuid,
}

async fn fixture(pool: &PgPool) -> Fx {
    common::seed_system(pool).await;
    let (cogmap, _telos) = common::genesis_cogmap(pool, "wayfind-test", "Wayfind Test").await;
    let team = common::create_team(pool, "wayfind-team").await;
    let p1 = common::create_profile(pool, "member@wayfind.test").await;
    let p2 = common::create_profile(pool, "outsider@wayfind.test").await;
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
    let sys: Uuid = sqlx::query_scalar("SELECT id FROM kb_profiles WHERE handle='system'")
        .fetch_one(pool)
        .await
        .expect("system profile");
    Fx {
        cogmap,
        lens,
        event,
        p1,
        p2,
        sys,
    }
}

struct RegionSeed<'a> {
    cogmap: Uuid,
    lens: Uuid,
    event: Uuid,
    salience: f64,
    telos_alignment: Option<f64>,
    reference_standing: Option<f64>,
    centrality: Option<f64>,
    centroid: &'a str,
    member_count: i32,
}

/// Plant a region on the fixture cogmap.
///
/// The **anchor pair is mandatory**: since T7 the wayfind pool is keyed on
/// `(home_anchor_table, home_anchor_id)` — not `cogmap_id` — and `kb_cogmap_regions` has **no trigger**
/// deriving one from the other. A fixture writing only `cogmap_id` plants regions the funnel cannot
/// see, and the map then looks region-*less*, silently falling through to the cold-start branch that
/// returns its whole homed scope. Every one of these tests would still "pass a search" while asserting
/// nothing about region selection. The producer dual-writes both (spec §3.6 M1); so does this.
async fn insert_region(pool: &PgPool, s: RegionSeed<'_>) -> Uuid {
    sqlx::query_scalar::<_, Uuid>(
        "INSERT INTO kb_cogmap_regions
           (cogmap_id, home_anchor_table, home_anchor_id, lens_id, centroid, salience,
            telos_alignment, reference_standing, centrality, member_count,
            asserted_by_event_id, last_event_id)
         VALUES ($1, 'kb_cogmaps', $1, $2, $3::vector, $4, $5, $6, $7, $8, $9, $9)
         RETURNING id",
    )
    .bind(s.cogmap)
    .bind(s.lens)
    .bind(s.centroid)
    .bind(s.salience)
    .bind(s.telos_alignment)
    .bind(s.reference_standing)
    .bind(s.centrality)
    .bind(s.member_count)
    .bind(s.event)
    .fetch_one(pool)
    .await
    .expect("insert region")
}

/// Insert a resource homed to the fixture cogmap (so it is visible to `p1` via the A0
/// cogmap-membership clause), returning its id.
async fn insert_homed_resource(pool: &PgPool, cogmap: Uuid, owner: Uuid, title: &str) -> Uuid {
    let rid: Uuid = sqlx::query_scalar(
        "INSERT INTO kb_resources (title, origin_uri) VALUES ($1, $2) RETURNING id",
    )
    .bind(title)
    .bind(format!("temper://wayfind/{title}"))
    .fetch_one(pool)
    .await
    .expect("insert resource");
    sqlx::query(
        "INSERT INTO kb_resource_homes
           (resource_id, anchor_table, anchor_id, originator_profile_id, owner_profile_id)
         VALUES ($1, 'kb_cogmaps', $2, $3, $3)",
    )
    .bind(rid)
    .bind(cogmap)
    .bind(owner)
    .execute(pool)
    .await
    .expect("home resource to cogmap");
    rid
}

async fn add_member(pool: &PgPool, region: Uuid, resource: Uuid) {
    sqlx::query(
        "INSERT INTO kb_cogmap_region_members (region_id, member_table, member_id)
         VALUES ($1, 'kb_resources', $2)",
    )
    .bind(region)
    .bind(resource)
    .execute(pool)
    .await
    .expect("add region member");
}

/// Seed one region (salience + optional `(telos_alignment, reference_standing, centrality)` components
/// for override-lens recompute; `None` ⇒ all NULL), create+home one member resource per title, attach
/// them. Returns the member resource ids.
async fn seed_region(
    pool: &PgPool,
    fx: &Fx,
    salience: f64,
    components: Option<(f64, f64, f64)>,
    centroid: &str,
    member_titles: &[&str],
) -> Vec<Uuid> {
    let (ta, rs, ce) = match components {
        Some((a, b, c)) => (Some(a), Some(b), Some(c)),
        None => (None, None, None),
    };
    let region = insert_region(
        pool,
        RegionSeed {
            cogmap: fx.cogmap,
            lens: fx.lens,
            event: fx.event,
            salience,
            telos_alignment: ta,
            reference_standing: rs,
            centrality: ce,
            centroid,
            member_count: member_titles.len() as i32,
        },
    )
    .await;
    let mut ids = Vec::new();
    for t in member_titles {
        let rid = insert_homed_resource(pool, fx.cogmap, fx.sys, t).await;
        add_member(pool, region, rid).await;
        ids.push(rid);
    }
    ids
}

// 1. top-N selection: 3 regions, regions=2 → only the 2 top-scoring regions' members are in scope.
//    A: salience 1.0 + cos 1.0 → score 1.0; B: salience 0.5 + cos 0.0 → 0.2; C: salience 0.0 + cos 0.0
//    → 0.0. Top-2 = {A,B}; C excluded. (α=0.4, β=0.6, min-max norm over the pool.)
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn wayfind_selects_top_n_regions(pool: PgPool) {
    let fx = fixture(&pool).await;
    let high_cos = vec768(&[(0, 1.0)]);
    let low_cos = vec768(&[(1, 1.0)]);
    let a = seed_region(&pool, &fx, 1.0, None, &high_cos, &["a"]).await;
    let b = seed_region(&pool, &fx, 0.5, None, &low_cos, &["b"]).await;
    let c = seed_region(&pool, &fx, 0.0, None, &low_cos, &["c"]).await;
    let q = query_axis0();

    let scope = wayfind_scope_ids(
        &pool,
        WayfindScopeQuery {
            principal: ProfileId::from(fx.p1),
            lens_id: None,
            embedding: Some(&q),
            regions: Some(2),
            anchor: None, // unscoped: pool every visible anchor (T7)
        },
    )
    .await
    .expect("wayfind scope");

    assert!(
        scope.contains(&a[0]),
        "region A (score 1.0) in top-2: {scope:?}"
    );
    assert!(
        scope.contains(&b[0]),
        "region B (score 0.2) in top-2: {scope:?}"
    );
    assert!(
        !scope.contains(&c[0]),
        "region C (score 0.0) excluded by top-2: {scope:?}"
    );
}

// 1b. Regression (review finding): a negative / zero / overflow-wrapped N must never reach
//     `LIMIT <negative>` — Postgres rejects that. The SQL `k`/`n` CTE clamps N into [1, max_n], so
//     regions=-1 behaves like regions=1 (top region only) and never errors.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn wayfind_regions_below_one_clamps_to_one(pool: PgPool) {
    let fx = fixture(&pool).await;
    let high_cos = vec768(&[(0, 1.0)]);
    let low_cos = vec768(&[(1, 1.0)]);
    let a = seed_region(&pool, &fx, 1.0, None, &high_cos, &["a"]).await;
    let b = seed_region(&pool, &fx, 0.5, None, &low_cos, &["b"]).await;
    let q = query_axis0();

    let scope = wayfind_scope_ids(
        &pool,
        WayfindScopeQuery {
            principal: ProfileId::from(fx.p1),
            lens_id: None,
            embedding: Some(&q),
            regions: Some(-1),
            anchor: None, // unscoped: pool every visible anchor (T7)
        },
    )
    .await
    .expect("negative regions must clamp into range, not error");

    assert!(
        scope.contains(&a[0]),
        "clamped to top-1: region A (score 1.0) present: {scope:?}"
    );
    assert!(
        !scope.contains(&b[0]),
        "clamped to top-1: region B (lower score) excluded: {scope:?}"
    );
}

// 2. THE §9 REGRESSION: region B is thin (1 member, salience 0.0) but high query-cosine; region A is
//    large (3 members, salience 1.0) but low query-cosine. regions=1. Scores: A = 0.4·1 + 0.6·0 = 0.4;
//    B = 0.4·0 + 0.6·1 = 0.6. B wins the single slot — relevance buys it. Margin 0.6 vs 0.4 (clear).
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn sparse_high_cosine_region_beats_large_low_cosine(pool: PgPool) {
    let fx = fixture(&pool).await;
    let high_cos = vec768(&[(0, 1.0)]);
    let low_cos = vec768(&[(1, 1.0)]);
    let large = seed_region(&pool, &fx, 1.0, None, &low_cos, &["a1", "a2", "a3"]).await;
    let sparse = seed_region(&pool, &fx, 0.0, None, &high_cos, &["b1"]).await;
    let q = query_axis0();

    let scope = wayfind_scope_ids(
        &pool,
        WayfindScopeQuery {
            principal: ProfileId::from(fx.p1),
            lens_id: None,
            embedding: Some(&q),
            regions: Some(1),
            anchor: None, // unscoped: pool every visible anchor (T7)
        },
    )
    .await
    .expect("wayfind scope");

    assert!(
        scope.contains(&sparse[0]),
        "sparse high-cosine region wins the single slot: {scope:?}"
    );
    for id in &large {
        assert!(
            !scope.contains(id),
            "large high-salience low-cosine region excluded from the single slot: {scope:?}"
        );
    }
}

// 3. cold-start: a region-less map in the visible set contributes its direct homed participants
//    (the cogmap_scope_ids fallback). thin_threshold=0 ⇒ a 0-region map is "thin". regions=N is a
//    silent no-op for it; never errors.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn region_less_map_degrades_to_direct_scope(pool: PgPool) {
    let fx = fixture(&pool).await; // no regions seeded → the map is region-less
    let direct = insert_homed_resource(&pool, fx.cogmap, fx.sys, "homed-direct").await;
    let q = query_axis0();

    let scope = wayfind_scope_ids(
        &pool,
        WayfindScopeQuery {
            principal: ProfileId::from(fx.p1),
            lens_id: None,
            embedding: Some(&q),
            regions: Some(3), // no-op against a region-less map; must not error
            anchor: None,     // unscoped: pool every visible anchor (T7)
        },
    )
    .await
    .expect("region-less map degrades, never errors");

    assert!(
        scope.contains(&direct),
        "region-less map degrades to its direct homed scope: {scope:?}"
    );
}

// 4. deny / per-map gating ("no view from nowhere"): a principal who is NOT a member of the fixture
//    map's team must not see that map's region member. The funnel still resolves without error — and a
//    legitimately-public, region-less map the principal DOES belong to (the L0 kernel `system-default`,
//    auto-joined via `temper-system`) correctly contributes its public telos through the cold-start
//    direct path. So the invariant under test is the per-map gate (the private member is excluded), NOT
//    a blanket empty set: the principal sees public maps it belongs to and nothing from maps it does not.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn wayfind_excludes_unreadable_maps(pool: PgPool) {
    let fx = fixture(&pool).await;
    let high_cos = vec768(&[(0, 1.0)]);
    let private = seed_region(&pool, &fx, 1.0, None, &high_cos, &["a"]).await;
    let q = query_axis0();

    let scope = wayfind_scope_ids(
        &pool,
        WayfindScopeQuery {
            principal: ProfileId::from(fx.p2), // not a member of the fixture map's team
            lens_id: None,
            embedding: Some(&q),
            regions: Some(3),
            anchor: None, // unscoped: pool every visible anchor (T7)
        },
    )
    .await
    .expect("deny yields no error");

    assert!(
        !scope.contains(&private[0]),
        "a non-member of the fixture map's team never sees its region member: {scope:?}"
    );
    // The L0 public-kernel telos is the only thing a fresh principal legitimately sees (region-less
    // public map via the auto-joined root team) — never the private fixture map's resources.
    let l0_telos: Uuid = "00000000-0000-0000-0005-000000000002".parse().unwrap();
    assert!(
        scope.iter().all(|id| *id == l0_telos),
        "p2's scope is bounded to the public kernel; no private map leaks: {scope:?}"
    );
}

// 5. lens override recompute: two regions, both zero query-cosine (so selection is salience-driven).
//    Under the DEFAULT (memoized) salience, A (0.5) > B (0.2) → A wins regions=1. Under an OVERRIDE
//    lens with s_central=1 (s_telos=s_ref=0), salience is recomputed FROM the stored components:
//    A = 0.0 (centrality 0), B = 1.0 (centrality 1) → B wins. Proves recompute-from-components, not a
//    lens_id filter.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn lens_override_recomputes_salience_from_components(pool: PgPool) {
    let fx = fixture(&pool).await;
    let low_cos = vec768(&[(1, 1.0)]); // zero query-cosine for both regions
                                       // A: high memoized salience, but centrality 0.
    let a = seed_region(&pool, &fx, 0.5, Some((1.0, 0.0, 0.0)), &low_cos, &["a"]).await;
    // B: low memoized salience, but centrality 1.
    let b = seed_region(&pool, &fx, 0.2, Some((0.0, 0.0, 1.0)), &low_cos, &["b"]).await;

    let override_lens: Uuid = sqlx::query_scalar(
        "INSERT INTO kb_cogmap_lenses
           (name, w_express, w_contains, w_leads_to, w_near, w_prop,
            s_telos, s_ref, s_central, resolution, asserted_by_event_id)
         VALUES ('central-heavy', 0,0,0,0,0, 0.0, 0.0, 1.0, 1.0, $1)
         RETURNING id",
    )
    .bind(fx.event)
    .fetch_one(&pool)
    .await
    .expect("insert override lens");

    let q = query_axis0();

    // Default lens (memoized salience) → A wins.
    let def = wayfind_scope_ids(
        &pool,
        WayfindScopeQuery {
            principal: ProfileId::from(fx.p1),
            lens_id: None,
            embedding: Some(&q),
            regions: Some(1),
            anchor: None, // unscoped: pool every visible anchor (T7)
        },
    )
    .await
    .expect("default-lens scope");
    assert!(
        def.contains(&a[0]),
        "default lens selects high-memoized-salience region A: {def:?}"
    );
    assert!(
        !def.contains(&b[0]),
        "region B excluded under default lens: {def:?}"
    );

    // Override lens (recompute from centrality) → B wins.
    let ov = wayfind_scope_ids(
        &pool,
        WayfindScopeQuery {
            principal: ProfileId::from(fx.p1),
            lens_id: Some(LensId::from(override_lens)),
            embedding: Some(&q),
            regions: Some(1),
            anchor: None, // unscoped: pool every visible anchor (T7)
        },
    )
    .await
    .expect("override-lens scope");
    assert!(
        ov.contains(&b[0]),
        "override (s_central=1) recomputes salience from components → region B wins: {ov:?}"
    );
    assert!(
        !ov.contains(&a[0]),
        "region A excluded under override lens: {ov:?}"
    );
}
