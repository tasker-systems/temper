#![cfg(feature = "artifact-tests")]
//! `wayfind_region_scores` — Surface B Half 2's lens-driven region-salience competition — read
//! through `wayfind_region_diagnostics`, its only read surface. Determinism: region rows are
//! inserted DIRECTLY with hand-chosen centroids/salience/components (no `materialize_cogmap`, no
//! ONNX), so the `region_score = α·sal_norm + β·query_cos + κ·prior` blend is exactly predictable.
//!
//! Proves: top-N region selection (1); the width clamp (1b); the §9 regression — a sparse
//! high-cosine region beats a large high-salience low-cosine one, i.e. relevance buys a top-N slot
//! (2); deny — a non-member of the map's team gets no candidate rows from it (4); lens override
//! recomputes salience from the stored components, reordering selection (5); per-map fairness
//! (issue #585, Task 2) — a relevant sibling map, volume-crowded by a dominant map, reaches the
//! top-N under round-robin (6); the wrapper projects the scoring function verbatim (7); the
//! reported scores are the real Stage-1 blend (8); and the `k` CTE's clamp literals (10).
//!
//! ## What this file used to test, and no longer can
//!
//! It was named for `readback::wayfind_scope_ids`, which with `wayfind_scope_reach` is dropped by
//! the commit that retires the wayfind scope funnel. Those two functions did TWO things:
//! selection, which lives in `wayfind_region_scores` and is still witnessed below; and the
//! member/cold-start DEREFERENCE from winning regions to a resource-id scope, which has no
//! surviving home. Everything that turned on the dereference is a declared coverage loss, named
//! here rather than left to be inferred from a shorter file:
//!
//! * **Cold start** — a region-less anchor contributing its directly-homed resources (§5). Both the
//!   plain case (`region_less_map_degrades_to_direct_scope`) and the reach-accounting case
//!   (`a_region_less_anchor_reached_by_cold_start_counts_as_reached`). `wayfind_region_scores`
//!   scores regions and a region-less anchor has none, so it cannot observe this arm at all.
//! * **The scope BOUND on a denied principal** — that `p2`'s ids are exactly the public L0 kernel
//!   telos and nothing from a map it cannot read. Test 4 below keeps the region-level half (the
//!   private map contributes no candidate row); the id-level half needs the dereference.
//! * **Reach legibility** (`wayfind_scope_reach`'s `anchors_visible` / `anchors_reached` /
//!   `anchors_selected` / `regions_effective`, issue #585 Task 4) — the narrow-vs-broad witness and
//!   the cold-start-floor monopoly mask. These were properties of the reach reporter, which is
//!   dropped whole. Test 10 keeps the half that pins the CLAMP against the scoring function; the
//!   half that pinned the REPORTER's literals against it has nothing left to compare.
//! * **The equivalence guard's WIRING half** — that a diagnostics `in_top_n` region's members are
//!   the ids the funnel actually returns. Test 7 re-homes the guard onto `wayfind_region_scores`
//!   directly, which pins the projection but not the dereference, because the dereference is gone.
//!
//! When the `survey` act gets a door onto `wayfind_region_scores` and a scope is derived again,
//! those are the properties to re-home — alongside the two already-orphaned ones from the earlier
//! commit on this branch (`salience_is_normalized_per_anchor_kind`,
//! `zero_centroid_region_does_not_hijack_the_top_n`).

use std::collections::HashSet;

use sqlx::PgPool;
use temper_core::types::ids::{LensId, ProfileId};
use temper_substrate::readback::{wayfind_region_diagnostics, WayfindScopeQuery};
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
/// are visible to `p1` purely through the A0 cogmap-membership read clause.
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
/// deriving one from the other. A fixture writing only `cogmap_id` plants regions the scoring function
/// cannot see, and the map then looks region-*less*, contributing no candidate rows at all. Every one
/// of these tests would still "pass a query" while asserting nothing about region selection. The
/// producer dual-writes both (spec §3.6 M1); so does this.
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

/// Insert a resource homed to `cogmap` (so it is visible to `p1` via the A0 cogmap-membership
/// clause), returning its id.
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

/// Plant a region on an ARBITRARY cogmap, with optional
/// `(telos_alignment, reference_standing, centrality)` components for override-lens recompute
/// (`None` ⇒ all NULL), creating and homing one member resource per title.
///
/// Returns the region id. Members are still planted even though nothing dereferences them any more:
/// `member_count` and the membership rows are part of the shape the producer writes, and a region
/// with no members is a different fixture than the one these tests mean.
async fn seed_region_on(
    pool: &PgPool,
    fx: &Fx,
    cogmap: Uuid,
    salience: f64,
    components: Option<(f64, f64, f64)>,
    centroid: &str,
    member_titles: &[&str],
) -> Uuid {
    let (ta, rs, ce) = match components {
        Some((a, b, c)) => (Some(a), Some(b), Some(c)),
        None => (None, None, None),
    };
    let region = insert_region(
        pool,
        RegionSeed {
            cogmap,
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
    for t in member_titles {
        let rid = insert_homed_resource(pool, cogmap, fx.sys, t).await;
        add_member(pool, region, rid).await;
    }
    region
}

/// [`seed_region_on`] against the fixture's own map.
async fn seed_region(
    pool: &PgPool,
    fx: &Fx,
    salience: f64,
    components: Option<(f64, f64, f64)>,
    centroid: &str,
    member_titles: &[&str],
) -> Uuid {
    seed_region_on(
        pool,
        fx,
        fx.cogmap,
        salience,
        components,
        centroid,
        member_titles,
    )
    .await
}

/// Genesis a SECOND cogmap named `name` and make it visible to `p1` (a fresh team `p1` joins), with
/// `slug` giving the team its slug. Returns its id — this is how a test puts two sibling maps in one
/// wayfind pool, the setup the #585 monopoly needs.
///
/// NB `genesis_cogmap(pool, name, telos_title)` takes the cogmap NAME first — so `name` is what lands
/// in `kb_cogmaps.name` and what `wayfind_region_diagnostics.home_anchor_name` resolves to.
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

/// The diagnostics rows for a query, as the tests read them.
async fn diagnostics(
    pool: &PgPool,
    principal: Uuid,
    lens: Option<Uuid>,
    q: &[f32],
    regions: Option<i32>,
) -> Vec<temper_substrate::readback::WayfindRegionDiagnosticRow> {
    wayfind_region_diagnostics(
        pool,
        WayfindScopeQuery {
            principal: ProfileId::from(principal),
            lens_id: lens.map(LensId::from),
            embedding: Some(q),
            regions,
            anchor: None, // unscoped: pool every visible anchor (T7)
        },
    )
    .await
    .expect("region diagnostics")
}

/// The region ids that cleared the top-N cut, as a set.
fn selected(rows: &[temper_substrate::readback::WayfindRegionDiagnosticRow]) -> HashSet<Uuid> {
    rows.iter()
        .filter(|r| r.in_top_n)
        .map(|r| r.region_id)
        .collect()
}

// 1. top-N selection: 3 regions, regions=2 → only the 2 top-scoring regions clear the cut.
//    A: salience 1.0 + cos 1.0 → score 1.05; B: salience 0.5 + cos 0.0 → 0.25; C: salience 0.0 +
//    cos 0.0 → 0.05. Top-2 = {A,B}; C excluded. (α=0.4, β=0.6, κ=0.05 cogmap prior, min-max norm
//    over the pool.)
//
//    Re-homed from `wayfind_selects_top_n_regions`, which read the same selection through
//    `wayfind_scope_ids`' dereferenced member ids.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn wayfind_selects_top_n_regions(pool: PgPool) {
    let fx = fixture(&pool).await;
    let high_cos = vec768(&[(0, 1.0)]);
    let low_cos = vec768(&[(1, 1.0)]);
    let a = seed_region(&pool, &fx, 1.0, None, &high_cos, &["a"]).await;
    let b = seed_region(&pool, &fx, 0.5, None, &low_cos, &["b"]).await;
    let c = seed_region(&pool, &fx, 0.0, None, &low_cos, &["c"]).await;
    let q = query_axis0();

    let top = selected(&diagnostics(&pool, fx.p1, None, &q, Some(2)).await);

    assert!(top.contains(&a), "region A (score 1.05) in top-2: {top:?}");
    assert!(top.contains(&b), "region B (score 0.25) in top-2: {top:?}");
    assert!(
        !top.contains(&c),
        "region C (score 0.05) excluded by top-2: {top:?}"
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

    let top = selected(&diagnostics(&pool, fx.p1, None, &q, Some(-1)).await);

    assert!(
        top.contains(&a),
        "clamped to top-1: region A (score 1.05) present: {top:?}"
    );
    assert!(
        !top.contains(&b),
        "clamped to top-1: region B (lower score) excluded: {top:?}"
    );
}

// 2. THE §9 REGRESSION: region B is thin (1 member, salience 0.0) but high query-cosine; region A is
//    large (3 members, salience 1.0) but low query-cosine. regions=1. Scores: A = 0.4·1 + 0.6·0 +
//    0.05 = 0.45; B = 0.4·0 + 0.6·1 + 0.05 = 0.65. B wins the single slot — relevance buys it.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn sparse_high_cosine_region_beats_large_low_cosine(pool: PgPool) {
    let fx = fixture(&pool).await;
    let high_cos = vec768(&[(0, 1.0)]);
    let low_cos = vec768(&[(1, 1.0)]);
    let large = seed_region(&pool, &fx, 1.0, None, &low_cos, &["a1", "a2", "a3"]).await;
    let sparse = seed_region(&pool, &fx, 0.0, None, &high_cos, &["b1"]).await;
    let q = query_axis0();

    let top = selected(&diagnostics(&pool, fx.p1, None, &q, Some(1)).await);

    assert!(
        top.contains(&sparse),
        "sparse high-cosine region wins the single slot: {top:?}"
    );
    assert!(
        !top.contains(&large),
        "large high-salience low-cosine region excluded from the single slot: {top:?}"
    );
}

// 4. DENY / per-map gating ("no view from nowhere"): a principal who is NOT a member of the fixture
//    map's team gets no candidate row from that map at all — the gate is `visible_region_anchors`
//    inside `wayfind_region_scores`, so an unreadable map's regions never enter the competition,
//    let alone win it. The function still resolves without error.
//
//    The membership half of this witness (that p2's resulting SCOPE was exactly the public L0 kernel
//    telos) went with `wayfind_scope_ids` and is a declared loss — see the header. The p1 assertion
//    below is what keeps this from passing because the fixture planted nothing.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn wayfind_excludes_unreadable_maps(pool: PgPool) {
    let fx = fixture(&pool).await;
    let high_cos = vec768(&[(0, 1.0)]);
    let private = seed_region(&pool, &fx, 1.0, None, &high_cos, &["a"]).await;
    let q = query_axis0();

    let mine = diagnostics(&pool, fx.p1, None, &q, Some(3)).await;
    assert!(
        mine.iter().any(|r| r.region_id == private),
        "precondition: a member of the map's team DOES see its region as a candidate, or the \
         exclusion below proves nothing"
    );

    let theirs = diagnostics(&pool, fx.p2, None, &q, Some(3)).await;
    assert!(
        theirs.iter().all(|r| r.home_anchor_id != fx.cogmap),
        "a non-member of the map's team gets no candidate row from it: {theirs:?}"
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
    let def = selected(&diagnostics(&pool, fx.p1, None, &q, Some(1)).await);
    assert!(
        def.contains(&a),
        "default lens selects high-memoized-salience region A: {def:?}"
    );
    assert!(
        !def.contains(&b),
        "region B excluded under default lens: {def:?}"
    );

    // Override lens (recompute from centrality) → B wins.
    let ov = selected(&diagnostics(&pool, fx.p1, Some(override_lens), &q, Some(1)).await);
    assert!(
        ov.contains(&b),
        "override (s_central=1) recomputes salience from components → region B wins: {ov:?}"
    );
    assert!(
        !ov.contains(&a),
        "region A excluded under override lens: {ov:?}"
    );
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
//        competitive champion reaches the cut. The monopoly is broken WITHOUT a tuning constant.
//
//    The tail that also asserted MAP_B's MEMBER entered the funnel's scope went with
//    `wayfind_scope_ids` (header). The flag is where the fairness lives; the dereference was the
//    wiring, and test 7 carries what remains of that guard.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn per_map_round_robin_admits_a_crowded_relevant_sibling(pool: PgPool) {
    let fx = fixture(&pool).await;
    let map_b = add_visible_map(&pool, fx.p1, "wayfind-map-b", "Wayfind Map B").await;
    let hi = vec768(&[(0, 1.0)]); // high query-cosine (axis 0 = query)
    let mid = vec768(&[(0, 1.0), (1, 1.0)]); // cos ≈ 0.707: relevant, but below MAP_A's axis-0 regions

    // MAP_A (fx.cogmap): three relevant regions — the "volume" that monopolizes under a global LIMIT.
    let a1 = seed_region_on(&pool, &fx, fx.cogmap, 1.0, None, &hi, &["a1"]).await;
    let a2 = seed_region_on(&pool, &fx, fx.cogmap, 0.9, None, &hi, &["a2"]).await;
    let a3 = seed_region_on(&pool, &fx, fx.cogmap, 0.8, None, &hi, &["a3"]).await;
    // MAP_B: one relevant-but-lower champion. Global top-2 shuts it out; round-robin admits it.
    let b1 = seed_region_on(&pool, &fx, map_b, 1.0, None, &mid, &["b1"]).await;
    let q = query_axis0();

    let diag = diagnostics(&pool, fx.p1, None, &q, Some(2)).await;

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
}

/// Read `wayfind_region_scores` — the surviving scoring home — directly, as `(region_id, in_top_n)`
/// for EVERY scored candidate, winners and losers alike.
///
/// There is no `readback` wrapper for it; `wayfind_region_diagnostics` is its only Rust surface,
/// which is exactly why test 7 must reach past that surface to have anything to compare against.
/// One read, so the winner-set and row-count assertions cannot disagree about which snapshot they
/// are looking at.
async fn scored_regions(
    pool: &PgPool,
    principal: Uuid,
    q: &[f32],
    regions: i32,
) -> Vec<(Uuid, bool)> {
    use sqlx::Row;
    let parts: Vec<String> = q.iter().map(f32::to_string).collect();
    sqlx::query(
        "SELECT region_id, in_top_n
           FROM wayfind_region_scores($1, NULL::uuid, $2::vector, $3, NULL::varchar, NULL::uuid)",
    )
    .bind(principal)
    .bind(format!("[{}]", parts.join(",")))
    .bind(regions)
    .fetch_all(pool)
    .await
    .expect("wayfind_region_scores")
    .iter()
    .map(|r| (r.get::<Uuid, _>("region_id"), r.get::<bool, _>("in_top_n")))
    .collect()
}

// 7. EQUIVALENCE GUARD, re-homed. It used to tie the diagnostics' `in_top_n` to `wayfind_scope_ids`'
//    actually-returned scope, by mapping each winning region to its single member. That comparator is
//    dropped, and the SELECTION it was guarding now lives entirely in `wayfind_region_scores` — so the
//    guard is re-pointed at that function directly.
//
//    What it still pins: `wayfind_region_diagnostics` is a PROJECTION of `wayfind_region_scores`, not a
//    second opinion about it. It re-states its own `ORDER BY` deliberately ("so the diagnostics' own
//    contract does not depend on the callee's"), and a re-stated clause is exactly the place a `WHERE
//    in_top_n` or a dropped loser row would arrive unnoticed. So: identical winner sets AND identical
//    row counts, swept across cut sizes rather than pinned at one lucky N.
//
//    What it can no longer pin — and this is the loss, not a weaker version of the same thing — is the
//    WIRING: that a winning region's members are the ids a caller receives. The dereference lives in
//    `wayfind_scope_reach`, which is dropped whole.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn wayfind_region_diagnostics_projects_the_scoring_function_verbatim(pool: PgPool) {
    let fx = fixture(&pool).await;
    let map_b = add_visible_map(&pool, fx.p1, "wayfind-eq-b", "Wayfind Eq B").await;

    // Distinct multi-axis centroids give four STRICTLY distinct region_scores (cosines 1.0 / 0.894 /
    // 0.707 / 0.447), so no tie straddles a top-N boundary. A boundary tie would be broken by `id` in
    // both functions and so could not actually flake — but keeping the scores distinct means a
    // mismatch is a real drift rather than a tiebreak artifact anyone has to reason about.
    seed_region_on(
        &pool,
        &fx,
        fx.cogmap,
        1.0,
        None,
        &vec768(&[(0, 1.0)]),
        &["a1"],
    )
    .await;
    seed_region_on(
        &pool,
        &fx,
        fx.cogmap,
        0.5,
        None,
        &vec768(&[(0, 1.0), (1, 1.0)]),
        &["a2"],
    )
    .await;
    seed_region_on(
        &pool,
        &fx,
        map_b,
        0.8,
        None,
        &vec768(&[(0, 1.0), (1, 0.5)]),
        &["b1"],
    )
    .await;
    seed_region_on(
        &pool,
        &fx,
        map_b,
        0.1,
        None,
        &vec768(&[(0, 1.0), (1, 2.0)]),
        &["b2"],
    )
    .await;
    let q = query_axis0();

    // Sweep a few region counts so the projection is pinned across cut sizes, not one lucky N.
    for regions in [1, 2, 3] {
        let diag = diagnostics(&pool, fx.p1, None, &q, Some(regions)).await;
        let scored = scored_regions(&pool, fx.p1, &q, regions).await;
        let winners: HashSet<Uuid> = scored
            .iter()
            .filter(|(_, top)| *top)
            .map(|(id, _)| *id)
            .collect();

        assert_eq!(
            selected(&diag),
            winners,
            "regions={regions}: the diagnostics' in_top_n set must be the scoring function's own; \
             a mismatch means the wrapper has started deciding rather than reporting"
        );
        assert!(
            !winners.is_empty(),
            "regions={regions}: the fixture must select a region, else the guard is vacuous"
        );

        // Every CANDIDATE is reported, not just the winners — the losers are the whole point of the
        // instrumentation, and a `WHERE in_top_n` in the wrapper would satisfy the set equality
        // above while silently turning the diagnostics into a winners-only view.
        assert_eq!(
            diag.len(),
            scored.len(),
            "regions={regions}: the diagnostics must report every scored candidate region, losers \
             included — {} reported against {} scored",
            diag.len(),
            scored.len()
        );
        assert!(
            scored.len() > winners.len(),
            "regions={regions}: the fixture must contain a LOSER, or the row-count assertion above \
             cannot tell a projection from a winners-only filter"
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
    let a = seed_region_on(
        &pool,
        &fx,
        fx.cogmap,
        1.0,
        None,
        &vec768(&[(0, 1.0)]),
        &["a"],
    )
    .await;
    let b = seed_region_on(
        &pool,
        &fx,
        fx.cogmap,
        0.0,
        None,
        &vec768(&[(1, 1.0)]),
        &["b"],
    )
    .await;
    let q = query_axis0();

    let diag = diagnostics(&pool, fx.p1, None, &q, Some(1)).await;

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

// 10. THE CLAMP PIN. `wayfind_region_scores`' `k` CTE carries three literals that decide the
//     effective width — `default_n` (the width when the caller names none), the floor of 1, and
//     `max_n` (the per-call ceiling). Nothing structural stops any of them being retuned in the SQL
//     without anyone noticing downstream, so this pins the answer for all three at once: with more
//     candidate regions planted than the ceiling, the number admitted IS the effective width.
//
//     Re-homed from `wayfind_effective_width_tracks_the_scoring_clamp`, which compared these same
//     admitted counts against `wayfind_scope_reach.regions_effective`. That comparator was a SECOND
//     copy of the literals in a second function, and it is dropped; the literals it duplicated are
//     what remains, so the test now pins them directly rather than pinning two copies to each other.
//     One map keeps round-robin from interleaving, so admitted-count is exactly the width.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn the_region_width_clamp_holds_its_default_floor_and_ceiling(pool: PgPool) {
    let fx = fixture(&pool).await;
    let hi = vec768(&[(0, 1.0)]);
    // 25 candidate regions on ONE map: more than the ceiling, so at every width below it the cut —
    // not the candidate supply — is what limits selection.
    for i in 0..25 {
        let salience = 1.0 - (f64::from(i) * 0.01);
        seed_region_on(
            &pool,
            &fx,
            fx.cogmap,
            salience,
            None,
            &hi,
            &[&format!("r{i}")],
        )
        .await;
    }
    let q = query_axis0();

    // (requested, expected effective) — the default (None), the floor, an ordinary width, the ceiling.
    for (requested, expected) in [(None, 3), (Some(0), 1), (Some(2), 2), (Some(999), 20)] {
        let admitted = diagnostics(&pool, fx.p1, None, &q, requested)
            .await
            .iter()
            .filter(|r| r.in_top_n && r.home_anchor_id == fx.cogmap)
            .count();
        assert_eq!(
            admitted, expected as usize,
            "requested={requested:?}: the scoring function admitted {admitted} regions where the \
             `k` CTE's clamp (default_n=3, floor=1, max_n=20) says {expected} — a clamp literal has \
             moved"
        );
    }
}
