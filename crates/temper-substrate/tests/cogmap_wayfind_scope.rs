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
//!
//! Per-map fairness (issue #585, Task 2): a relevant sibling map, volume-crowded by a dominant map,
//! reaches the top-N under per-map round-robin — the monopoly a global `LIMIT` would produce is broken
//! without a tuning constant (6). The diagnostics and the scope funnel read ONE scoring home
//! (`wayfind_region_scores`), so their selection cannot drift (7), and the reported scores are the real
//! Stage-1 blend (8).

use std::collections::HashSet;

use sqlx::PgPool;
use temper_core::types::ids::{LensId, ProfileId};
use temper_substrate::readback::{
    wayfind_region_diagnostics, wayfind_scope_ids, wayfind_scope_reach, WayfindScopeQuery,
};
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

// ---------------------------------------------------------------------------
// wayfind_region_diagnostics — the Stage-1 competition instrumentation (issue #585).
//
// These reuse the same deterministic, hand-planted regions (no materialize, no ONNX) so the reported
// sal_norm/query_cos/region_score are exactly predictable. They prove the diagnostics report the REGION
// stage keyed by map (not unified_search's per-hit scores), and — critically — that the diagnostics
// stay in lockstep with `wayfind_scope_ids`' actual selection.
// ---------------------------------------------------------------------------

/// Plant a region on an ARBITRARY cogmap (not just `fx.cogmap`), reusing the fixture's lens/event/sys.
/// Returns `(region_id, member_ids)` so a caller can map a diagnostics `region_id` back to the members
/// it would put in scope — the join the equivalence test needs.
async fn seed_region_on(
    pool: &PgPool,
    fx: &Fx,
    cogmap: Uuid,
    salience: f64,
    centroid: &str,
    member_titles: &[&str],
) -> (Uuid, Vec<Uuid>) {
    let region = insert_region(
        pool,
        RegionSeed {
            cogmap,
            lens: fx.lens,
            event: fx.event,
            salience,
            telos_alignment: None,
            reference_standing: None,
            centrality: None,
            centroid,
            member_count: member_titles.len() as i32,
        },
    )
    .await;
    let mut ids = Vec::new();
    for t in member_titles {
        let rid = insert_homed_resource(pool, cogmap, fx.sys, t).await;
        add_member(pool, region, rid).await;
        ids.push(rid);
    }
    (region, ids)
}

/// Genesis a SECOND cogmap named `name` and make it visible to `p1` (a fresh team `p1` joins), with
/// `slug` giving the team its slug. Returns its id. The fixture's own map is `fx.cogmap`; this is how a
/// test puts two sibling maps in one wayfind pool — the setup the #585 monopoly needs.
///
/// NB `genesis_cogmap(pool, name, telos_title)` takes the cogmap NAME first — so `name` is what lands in
/// `kb_cogmaps.name` and what `wayfind_region_diagnostics.home_anchor_name` resolves to.
async fn add_visible_map(pool: &PgPool, p1: Uuid, slug: &str, name: &str) -> Uuid {
    let (cogmap, _telos) = common::genesis_cogmap(pool, name, name).await;
    let team = common::create_team(pool, &format!("{slug}-team")).await;
    common::add_team_member(pool, team, p1).await;
    sqlx::query("INSERT INTO kb_team_cogmaps (team_id, cogmap_id) VALUES ($1, $2)")
        .bind(team)
        .bind(cogmap)
        .execute(pool)
        .await
        .expect("join second cogmap to team");
    cogmap
}

// 6. THE #585 WITNESS (Task 2 — per-map fairness): a RELEVANT sibling map, volume-crowded by a
//    dominant map, now reaches the top-N. It MUST fail against the pre-fix global-`LIMIT` selection
//    and pass under per-map round-robin — the register's required witness for `no-single-map-monopoly`.
//
//    MAP_A (fx.cogmap) has THREE regions, all high query-cosine (axis 0 = query): scores ≈ a1 0.917,
//    a2 0.783, a3 0.65. MAP_B has ONE region whose centroid mixes axes 0+1 (cos ≈ 0.707) — genuinely
//    relevant, but its champion b1 ≈ 0.741 sits BELOW MAP_A's top two. regions=2.
//      - Under the pre-fix global `ORDER BY region_score DESC LIMIT 2`, the top-2 are {a1, a2}: both
//        MAP_A, MAP_B shut out purely because MAP_A has more high regions. THE #585 MONOPOLY.
//      - Under per-map round-robin, round 1 admits each map's champion by score → {a1, b1}: MAP_B's
//        competitive champion reaches the scope. The monopoly is broken WITHOUT a tuning constant.
//    The witness asserts both the diagnostics flag AND the real scope funnel (`wayfind_scope_ids`)
//    surface MAP_B, and encodes the crowding (≥ regions_n of MAP_A outscore b1) that made it a monopoly.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn per_map_round_robin_admits_a_crowded_relevant_sibling(pool: PgPool) {
    let fx = fixture(&pool).await;
    let map_b = add_visible_map(&pool, fx.p1, "wayfind-map-b", "Wayfind Map B").await;
    let hi = vec768(&[(0, 1.0)]); // high query-cosine (axis 0 = query)
    let mid = vec768(&[(0, 1.0), (1, 1.0)]); // cos ≈ 0.707: relevant, but below MAP_A's axis-0 regions

    // MAP_A (fx.cogmap): three relevant regions — the "volume" that monopolizes under a global LIMIT.
    let (a1, a1m) = seed_region_on(&pool, &fx, fx.cogmap, 1.0, &hi, &["a1"]).await;
    let (a2, _) = seed_region_on(&pool, &fx, fx.cogmap, 0.9, &hi, &["a2"]).await;
    let (a3, _) = seed_region_on(&pool, &fx, fx.cogmap, 0.8, &hi, &["a3"]).await;
    // MAP_B: one relevant-but-lower champion. Global top-2 shuts it out; round-robin admits it.
    let (b1, b1m) = seed_region_on(&pool, &fx, map_b, 1.0, &mid, &["b1"]).await;
    let q = query_axis0();

    let query = WayfindScopeQuery {
        principal: ProfileId::from(fx.p1),
        lens_id: None,
        embedding: Some(&q),
        regions: Some(2),
        anchor: None, // unscoped: pool every visible anchor
    };
    let diag = wayfind_region_diagnostics(&pool, query.clone())
        .await
        .expect("region diagnostics");

    // Every candidate region is reported, keyed by map — the losers (a3) too.
    let row = |rid: Uuid| {
        diag.iter()
            .find(|r| r.region_id == rid)
            .unwrap_or_else(|| panic!("region {rid} must appear in diagnostics: {diag:?}"))
    };
    for rid in [a1, a2, a3, b1] {
        let _ = row(rid);
    }
    assert_eq!(row(a1).home_anchor_id, fx.cogmap);
    assert_eq!(row(b1).home_anchor_id, map_b);
    assert_eq!(
        row(b1).home_anchor_name.as_deref(),
        Some("Wayfind Map B"),
        "the home map name is resolved so a sweep reads by map, not UUID"
    );

    // THE CROWDING that made this a #585 monopoly: at least `regions_n` (=2) of MAP_A's regions
    // strictly outscore MAP_B's champion, so a global `ORDER BY region_score DESC LIMIT 2` would fill
    // both slots with MAP_A and exclude b1. This is what per-map fairness overrides.
    let a_beats_b1 = [a1, a2, a3]
        .iter()
        .filter(|&&r| row(r).region_score > row(b1).region_score)
        .count();
    assert!(
        a_beats_b1 >= 2,
        "fixture invalid: MAP_A must out-score b1 on ≥2 regions so a global LIMIT would monopolize \
         (a1={:.4} a2={:.4} a3={:.4} b1={:.4})",
        row(a1).region_score,
        row(a2).region_score,
        row(a3).region_score,
        row(b1).region_score
    );

    // THE FIX: the monopoly is broken. MAP_B's champion clears the cut; MAP_A does not sweep both slots.
    let top_by_map = |map: Uuid| {
        diag.iter()
            .filter(|r| r.in_top_n && r.home_anchor_id == map)
            .count()
    };
    assert!(
        row(b1).in_top_n,
        "MAP_B's competitive champion reaches the top-N under per-map fairness: {diag:?}"
    );
    assert_eq!(
        top_by_map(fx.cogmap),
        1,
        "MAP_A no longer sweeps: 1/2 slots"
    );
    assert_eq!(top_by_map(map_b), 1, "MAP_B reaches: 1/2 slots");

    // And the REAL scope funnel — not just the diagnostics mirror — surfaces MAP_B's member, so its
    // content actually reaches Stage-2 ranking. Both read the single scoring home, so they agree.
    let scope: HashSet<Uuid> = wayfind_scope_ids(&pool, query)
        .await
        .expect("wayfind scope")
        .into_iter()
        .collect();
    assert!(
        scope.contains(&b1m[0]),
        "MAP_B's member enters the wayfind scope (no longer volume-crowded out): {scope:?}"
    );
    assert!(
        scope.contains(&a1m[0]),
        "MAP_A's champion is still in scope — fairness is not a swap, both maps are reachable: {scope:?}"
    );
}

// 7. EQUIVALENCE GUARD: the diagnostics' `in_top_n` selection must equal `wayfind_scope_ids`' actual
//    scope. Each region gets exactly one member, so a diagnostics `region_id` maps to one scope id.
//    In a no-cold-start fixture the only extra id `wayfind_scope_ids` returns is the public L0 kernel
//    telos (region-less, reached via the auto-joined root team) — which the diagnostics correctly do
//    NOT report (no candidate region to score). Subtract that one known id and the two must coincide.
//    Since Task 2 the two functions read ONE scoring home (`wayfind_region_scores`), so this equivalence
//    is true by construction — the test now guards the WIRING (scope_ids' member-deref of the shared
//    winners matches diagnostics' region rows), across the same round-robin cut sizes, not two copies of
//    a blend that could drift.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn wayfind_region_diagnostics_matches_scope_ids(pool: PgPool) {
    let fx = fixture(&pool).await;
    let map_b = add_visible_map(&pool, fx.p1, "wayfind-eq-b", "Wayfind Eq B").await;

    // Distinct multi-axis centroids give four STRICTLY distinct region_scores (cosines 1.0 / 0.894 /
    // 0.707 / 0.447), so no tie straddles a top-N LIMIT boundary. A boundary tie would let the two
    // functions' independent `ORDER BY region_score DESC LIMIT n` break it differently and flake this
    // guard — the equivalence we want is of the SCORING, not of Postgres' arbitrary tiebreak.
    // Resulting order: a1 (1.05) > b1 (0.987) > a2 (0.474) > b2 (0.318).
    let (a1, a1m) = seed_region_on(&pool, &fx, fx.cogmap, 1.0, &vec768(&[(0, 1.0)]), &["a1"]).await;
    let (a2, a2m) = seed_region_on(
        &pool,
        &fx,
        fx.cogmap,
        0.5,
        &vec768(&[(0, 1.0), (1, 1.0)]),
        &["a2"],
    )
    .await;
    let (b1, b1m) = seed_region_on(
        &pool,
        &fx,
        map_b,
        0.8,
        &vec768(&[(0, 1.0), (1, 0.5)]),
        &["b1"],
    )
    .await;
    let (b2, b2m) = seed_region_on(
        &pool,
        &fx,
        map_b,
        0.1,
        &vec768(&[(0, 1.0), (1, 2.0)]),
        &["b2"],
    )
    .await;
    let member_of = |rid: Uuid| -> Uuid {
        match rid {
            r if r == a1 => a1m[0],
            r if r == a2 => a2m[0],
            r if r == b1 => b1m[0],
            r if r == b2 => b2m[0],
            other => panic!("unexpected region {other}"),
        }
    };
    let q = query_axis0();
    let query = |regions| WayfindScopeQuery {
        principal: ProfileId::from(fx.p1),
        lens_id: None,
        embedding: Some(&q),
        regions: Some(regions),
        anchor: None,
    };

    // Sweep a few region counts so the equivalence is pinned across cut sizes, not one lucky N.
    for regions in [1, 2, 3] {
        let diag = wayfind_region_diagnostics(&pool, query(regions))
            .await
            .expect("diagnostics");
        let scope: HashSet<Uuid> = wayfind_scope_ids(&pool, query(regions))
            .await
            .expect("scope ids")
            .into_iter()
            .collect();

        let expected: HashSet<Uuid> = diag
            .iter()
            .filter(|r| r.in_top_n)
            .map(|r| member_of(r.region_id))
            .collect();

        // `wayfind_scope_ids` also emits the public L0 kernel telos via cold-start (region-less map);
        // the diagnostics report only scored candidate regions, so remove that one known id.
        let l0_telos: Uuid = "00000000-0000-0000-0005-000000000002".parse().unwrap();
        let scope_regions: HashSet<Uuid> = scope.into_iter().filter(|id| *id != l0_telos).collect();

        assert_eq!(
            scope_regions, expected,
            "regions={regions}: diagnostics in_top_n must match wayfind_scope_ids' actual scope \
             (cold-start L0 telos excluded); a mismatch means the two functions have drifted"
        );
        assert!(
            !expected.is_empty(),
            "regions={regions}: the fixture must select at least one region, else the guard is vacuous"
        );
    }
}

// 8. SCORES ARE REAL: the reported sal_norm/query_cos/region_score are the actual Stage-1 blend, not
//    placeholders. Region A (axis-0 centroid, salience 1.0) vs region B (axis-1, salience 0.0), query
//    on axis 0. Hand-computed: A → sal_norm 1.0, query_cos 1.0, score 0.4·1 + 0.6·1 + 0.05·1(cogmap
//    prior) = 1.05; B → sal_norm 0.0, query_cos 0.0, score 0.05. regions=1 ⇒ only A clears the cut.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn diagnostics_report_the_real_stage1_scores(pool: PgPool) {
    let fx = fixture(&pool).await;
    let (a, _) = seed_region_on(&pool, &fx, fx.cogmap, 1.0, &vec768(&[(0, 1.0)]), &["a"]).await;
    let (b, _) = seed_region_on(&pool, &fx, fx.cogmap, 0.0, &vec768(&[(1, 1.0)]), &["b"]).await;
    let q = query_axis0();

    let diag = wayfind_region_diagnostics(
        &pool,
        WayfindScopeQuery {
            principal: ProfileId::from(fx.p1),
            lens_id: None,
            embedding: Some(&q),
            regions: Some(1),
            anchor: None,
        },
    )
    .await
    .expect("diagnostics");

    let ra = diag
        .iter()
        .find(|r| r.region_id == a)
        .expect("region A row");
    let rb = diag
        .iter()
        .find(|r| r.region_id == b)
        .expect("region B row");
    let close = |x: f64, y: f64| (x - y).abs() < 1e-6;

    assert!(close(ra.sal_norm, 1.0), "A sal_norm=1.0: {ra:?}");
    assert!(close(ra.query_cos, 1.0), "A query_cos=1.0: {ra:?}");
    assert!(close(ra.region_score, 1.05), "A score=1.05: {ra:?}");
    assert!(ra.in_top_n, "A clears the top-1 cut: {ra:?}");

    assert!(close(rb.sal_norm, 0.0), "B sal_norm=0.0: {rb:?}");
    assert!(close(rb.query_cos, 0.0), "B query_cos=0.0: {rb:?}");
    assert!(
        close(rb.region_score, 0.05),
        "B score=0.05 (κ·prior only): {rb:?}"
    );
    assert!(!rb.in_top_n, "B excluded by the top-1 cut: {rb:?}");

    // Rows come back highest-score first.
    assert_eq!(diag.first().map(|r| r.region_id), Some(a), "winner first");
}

// ---------------------------------------------------------------------------
// REACH LEGIBILITY (issue #585, Task 4) — `wayfind_scope_reach`.
//
// `scope_size` counts RESOURCES, so a wayfind confined to one map and one spread across many report
// the same healthy-looking figure. These pin the signal that can tell them apart.
// ---------------------------------------------------------------------------

/// Probe helper: the reach scalars for a query, so a test reads as an assertion about reach rather
/// than about struct plumbing.
async fn reach_at(
    pool: &PgPool,
    fx: &Fx,
    regions: Option<i32>,
    q: &[f32],
) -> temper_substrate::readback::WayfindScopeReach {
    wayfind_scope_reach(
        pool,
        WayfindScopeQuery {
            principal: ProfileId::from(fx.p1),
            lens_id: None,
            embedding: Some(q),
            regions,
            anchor: None,
        },
    )
    .await
    .expect("wayfind scope reach")
}

// 9. THE TASK-4 WITNESS: a narrow wayfind is legible AS narrow. With `regions=1` a single map wins the
//    only region slot, so the result is single-map by construction — and `scope_size` cannot say so.
//    `anchors_reached` against `anchors_visible` can. This fails against the pre-Task-4 surface
//    vacuously (the fields did not exist) and, more usefully, would fail against any implementation
//    that reported a resource count under a reach-shaped name.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn reach_distinguishes_a_narrow_wayfind_from_a_broad_one(pool: PgPool) {
    let fx = fixture(&pool).await;
    let map_b = add_visible_map(&pool, fx.p1, "wayfind-reach-b", "Wayfind Reach B").await;
    let map_c = add_visible_map(&pool, fx.p1, "wayfind-reach-c", "Wayfind Reach C").await;
    let hi = vec768(&[(0, 1.0)]);
    // Each map gets a region with SEVERAL members, so the resource count stays large while the number
    // of anchors actually drawn on varies — the exact confusion `scope_size` cannot resolve.
    seed_region_on(&pool, &fx, fx.cogmap, 1.0, &hi, &["a1", "a2", "a3"]).await;
    seed_region_on(&pool, &fx, map_b, 0.9, &hi, &["b1", "b2", "b3"]).await;
    seed_region_on(&pool, &fx, map_c, 0.8, &hi, &["c1", "c2", "c3"]).await;
    let q = query_axis0();

    let narrow = reach_at(&pool, &fx, Some(1), &q).await;
    let broad = reach_at(&pool, &fx, Some(3), &q).await;

    // Both see the same anchors — visibility is a property of the principal, not of the query width.
    assert_eq!(
        narrow.anchors_visible, broad.anchors_visible,
        "anchors_visible is the principal's reach ceiling and must not move with the width"
    );
    assert!(
        narrow.anchors_visible >= 3,
        "fixture invalid: need ≥3 visible anchors to have a reach to miss; got {}",
        narrow.anchors_visible
    );

    // THE POINT: the narrow pass draws on strictly fewer anchors than the broad one...
    assert!(
        narrow.anchors_reached < broad.anchors_reached,
        "a width-1 wayfind must be legible as reaching fewer anchors than a width-3 one \
         (narrow={} broad={})",
        narrow.anchors_reached,
        broad.anchors_reached
    );
    // ...and neither is honest about it through `scope_size`, which is why this signal exists. The
    // narrow pass still admits a substantial resource count, so a caller reading only the resource
    // figure would see health in both.
    assert!(
        narrow.scope_ids.len() >= 3,
        "the narrow pass still looks 'big' by resource count — that is the flattering read this \
         replaces; got {}",
        narrow.scope_ids.len()
    );
    // The narrow pass genuinely missed anchors it could have reached.
    assert!(
        narrow.anchors_reached < narrow.anchors_visible,
        "width 1 over ≥3 visible anchors must leave some unreached (reached={} visible={})",
        narrow.anchors_reached,
        narrow.anchors_visible
    );

    // And the scope the reach describes IS the scope the funnel uses — one home, not two.
    let ids: HashSet<Uuid> = wayfind_scope_ids(
        &pool,
        WayfindScopeQuery {
            principal: ProfileId::from(fx.p1),
            lens_id: None,
            embedding: Some(&q),
            regions: Some(1),
            anchor: None,
        },
    )
    .await
    .expect("wayfind scope")
    .into_iter()
    .collect();
    assert_eq!(
        ids,
        narrow.scope_ids.iter().copied().collect::<HashSet<Uuid>>(),
        "wayfind_scope_ids is a wrapper over wayfind_scope_reach — the id sets must be identical"
    );
}

// 10. THE CLAMP PIN. `wayfind_scope_reach` re-derives the effective width from literals that MUST
//     track `wayfind_region_scores`' own `k`.default_n / `k`.max_n. Nothing structural ties the two,
//     so this test does: it compares the REPORTED width against the number of regions the scoring
//     function actually admits, with enough candidates planted that the clamp is what binds. Change
//     `default_n` or `max_n` in one place and this goes red.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn wayfind_effective_width_tracks_the_scoring_clamp(pool: PgPool) {
    let fx = fixture(&pool).await;
    let hi = vec768(&[(0, 1.0)]);
    // 25 candidate regions on ONE map: more than the ceiling, so at every width below it the cut —
    // not the candidate supply — is what limits selection. One map keeps round-robin from
    // interleaving, so admitted-count is exactly the width.
    for i in 0..25 {
        let salience = 1.0 - (f64::from(i) * 0.01);
        seed_region_on(&pool, &fx, fx.cogmap, salience, &hi, &[&format!("r{i}")]).await;
    }
    let q = query_axis0();

    // (requested, expected effective) — the default (None), the floor, and the ceiling.
    for (requested, expected) in [(None, 3), (Some(0), 1), (Some(2), 2), (Some(999), 20)] {
        let reach = reach_at(&pool, &fx, requested, &q).await;
        assert_eq!(
            reach.regions_effective, expected,
            "reported effective width for requested={requested:?}"
        );
        let admitted = wayfind_region_diagnostics(
            &pool,
            WayfindScopeQuery {
                principal: ProfileId::from(fx.p1),
                lens_id: None,
                embedding: Some(&q),
                regions: requested,
                anchor: None,
            },
        )
        .await
        .expect("region diagnostics")
        .iter()
        .filter(|r| r.in_top_n && r.home_anchor_id == fx.cogmap)
        .count();
        assert_eq!(
            admitted, expected as usize,
            "the SCORING function admitted {admitted} regions for requested={requested:?} but the \
             reported effective width says {expected} — the clamp literals in wayfind_scope_reach \
             have drifted from wayfind_region_scores' `k` CTE"
        );
    }
}

// 11. THE COLD-START ARM. An anchor with no regions contributes its directly-homed resources (§5) and
//     is therefore REACHED — it just did not get there by winning a region slot. Counting only region
//     winners would report the fresh, thin anchors as unreached in exactly the case the substrate went
//     out of its way to reach them, so this pins the arm that is easiest to drop.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn a_region_less_anchor_reached_by_cold_start_counts_as_reached(pool: PgPool) {
    let fx = fixture(&pool).await;
    let thin = add_visible_map(&pool, fx.p1, "wayfind-thin", "Wayfind Thin").await;
    let hi = vec768(&[(0, 1.0)]);
    // The fat map has the only regions, and enough of them to take every slot at width 1.
    seed_region_on(&pool, &fx, fx.cogmap, 1.0, &hi, &["fat1"]).await;
    // `thin` gets a homed resource but NO region — the cold-start shape.
    let thin_member = insert_homed_resource(&pool, thin, fx.sys, "thin-doc").await;
    let q = query_axis0();

    let reach = reach_at(&pool, &fx, Some(1), &q).await;
    assert!(
        reach.scope_ids.contains(&thin_member),
        "cold-start must still admit the region-less anchor's homed resource: {:?}",
        reach.scope_ids
    );
    // Reach EXCEEDS the width: one region slot, but two anchors contributed. This is why the hint may
    // not attribute a partial reach to the width whenever `reached < visible`.
    assert!(
        reach.anchors_reached > reach.regions_effective,
        "an anchor reached by cold-start does not consume a region slot, so reach can exceed the \
         width (reached={} width={})",
        reach.anchors_reached,
        reach.regions_effective
    );
}

// 12. THE COLD-START FLOOR, and why one reach number could not carry the clause. Reproduces the
//     production shape: one map holds every live region and wins the whole width, while several
//     region-less anchors are admitted wholesale on every query. `anchors_reached` is then high and
//     reads as broad reach; `anchors_selected` is 1 and names the monopoly. Measured on prod
//     2026-08-01: 6 of 10 visible anchors were region-less, so a total monopoly still reported 7 of
//     10 reached — this test is that case, shrunk.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn selection_names_a_monopoly_that_the_cold_start_floor_hides(pool: PgPool) {
    let fx = fixture(&pool).await;
    let hi = vec768(&[(0, 1.0)]);
    // The fat map holds every live region, so it wins every slot the width offers.
    for i in 0..6 {
        let salience = 1.0 - (f64::from(i) * 0.01);
        seed_region_on(&pool, &fx, fx.cogmap, salience, &hi, &[&format!("fat{i}")]).await;
    }
    // Three region-less anchors holding content — the cold-start floor.
    for n in ["floor-a", "floor-b", "floor-c"] {
        let thin = add_visible_map(&pool, fx.p1, n, n).await;
        insert_homed_resource(&pool, thin, fx.sys, &format!("{n}-doc")).await;
    }
    let q = query_axis0();

    let reach = reach_at(&pool, &fx, Some(3), &q).await;

    // THE MONOPOLY: one anchor took the entire width.
    assert_eq!(
        reach.anchors_selected, 1,
        "the fat map holds every region, so it must win every slot — selected should be 1, got {}",
        reach.anchors_selected
    );
    // AND THE MASK: reach is materially higher, because the region-less anchors came in regardless.
    assert!(
        reach.anchors_reached >= 4,
        "the three region-less anchors plus the winner must all be 'reached' (got {})",
        reach.anchors_reached
    );
    assert!(
        reach.anchors_reached > reach.anchors_selected,
        "this is the whole point: reach ({}) overstates competitive breadth ({}), so a caller \
         reading reach alone would see health where there is a monopoly",
        reach.anchors_reached,
        reach.anchors_selected
    );
    // The wholesale count is recoverable, which is what lets a caller discount it.
    assert!(
        reach.anchors_reached - reach.anchors_selected >= 3,
        "reached - selected must expose the unconditional admissions"
    );
}
