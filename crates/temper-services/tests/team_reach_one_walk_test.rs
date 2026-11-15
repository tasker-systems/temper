#![cfg(feature = "test-db")]

//! Team reach in one walk (migration `20261129000010`): `team_ancestors` and
//! `profile_reachable_teams` both delegate to `team_ancestors_of(uuid[])`, and both must return
//! exactly the sets their pre-migration bodies returned.
//!
//! # The oracle is frozen, never live
//!
//! After the migration `team_ancestors` is itself a wrapper over the new walk. So an oracle that
//! calls `team_ancestors` (or `profile_effective_teams`) would compare the new walk with itself,
//! and a broken walk would move both sides together. [`ORACLE`] therefore carries the
//! pre-migration bodies **verbatim**, self-contained: `team_ancestors` from `20260703000001`, and
//! `profile_reachable_teams` from `20260804000010` with `profile_effective_teams` (also
//! `20260703000001`) inlined. They read only base tables.
//!
//! # What is compared
//!
//! [`closure_diffs`] compares, as ordered arrays with duplicates kept, every profile's
//! `profile_reachable_teams` with the oracle's, and every team's `team_ancestors` with the
//! oracle's. Dead teams, a NULL and an id with no row are included on both axes. It runs over:
//!
//! * a curated world ([`curated`]) holding one cell per way the walk can be cut: a dead member team
//!   (the seed floor), a dead intermediate (the step floor), a diamond with one dead path, a dead
//!   top, overlapping memberships and a cycle;
//! * seeded random worlds ([`random_world`]): random DAGs with random liveness, random
//!   memberships, and back-edges inserted with the cycle trigger disabled, because a cycle is a state
//!   the schema admits (the trigger can be sidestepped).
//!
//! # The bites
//!
//! [`the_step_floor_bites`] and [`the_seed_floor_bites`] each plant one mutation of
//! `team_ancestors_of` in the test's own database, require the comparison to go red, and require
//! it to go green again after restoring the body. The seed-floor bite also shows the leak it
//! guards against: with only that floor dropped, a dead team's public_within row becomes
//! effective and a person under the anchor reads the dead team's resource.

use sqlx::PgPool;
use uuid::Uuid;

// =================================================================================================
// The frozen oracle.
// =================================================================================================

/// The pre-migration bodies, verbatim, under oracle names. `oracle_team_ancestors` is
/// `team_ancestors` from `20260703000001`; `oracle_reachable_teams` is `profile_reachable_teams`
/// from `20260804000010` with `profile_effective_teams` from `20260703000001` inlined, calling the
/// oracle walk rather than the live one.
const ORACLE: &str = r#"
CREATE FUNCTION oracle_team_ancestors(p_team uuid)
RETURNS TABLE(team_id uuid) LANGUAGE sql STABLE AS $$
    WITH RECURSIVE up AS (
        SELECT t.id AS team_id
        FROM kb_teams t
        WHERE t.id = p_team AND t.is_active
        UNION
        SELECT tp.parent_id
        FROM kb_teams_parents tp
        JOIN up ON tp.child_id = up.team_id
        JOIN kb_teams pt ON pt.id = tp.parent_id AND pt.is_active
    )
    SELECT team_id FROM up;
$$;

CREATE FUNCTION oracle_reachable_teams(p_profile uuid)
RETURNS TABLE(team_id uuid) LANGUAGE sql STABLE AS $$
    SELECT DISTINCT a.team_id
    FROM (SELECT tm.team_id
            FROM kb_team_members tm
            JOIN kb_teams t ON t.id = tm.team_id
           WHERE tm.profile_id = p_profile AND t.is_active) e
    CROSS JOIN LATERAL oracle_team_ancestors(e.team_id) a;
$$;
"#;

async fn install_oracle(pool: &PgPool) {
    sqlx::raw_sql(ORACLE)
        .execute(pool)
        .await
        .expect("install the frozen oracle");
}

/// Every subject whose live closure differs from the oracle's, described for the failure message.
/// Empty means equal everywhere.
async fn closure_diffs(pool: &PgPool) -> Vec<String> {
    closure_diffs_within(pool, "").await
}

/// [`closure_diffs`] restricted to the profiles and teams whose handle or slug starts with
/// `prefix` (plus the NULL and the id with no row). The random worlds share one database, so each
/// round checks only its own world rather than re-checking every earlier one.
async fn closure_diffs_within(pool: &PgPool, prefix: &str) -> Vec<String> {
    sqlx::query_scalar(
        "WITH profiles AS (
             SELECT id FROM kb_profiles WHERE starts_with(handle, $1)
             UNION ALL SELECT NULL::uuid UNION ALL SELECT gen_random_uuid()
         ), teams AS (
             SELECT id FROM kb_teams WHERE starts_with(slug, $1)
             UNION ALL SELECT NULL::uuid UNION ALL SELECT gen_random_uuid()
         )
         SELECT 'profile ' || coalesce(p.id::text, 'NULL') || ': live '
                || live::text || ' vs oracle ' || oracle::text
           FROM profiles p
           CROSS JOIN LATERAL (SELECT coalesce(array_agg(x.team_id ORDER BY x.team_id), '{}') AS live
                                 FROM profile_reachable_teams(p.id) x) l
           CROSS JOIN LATERAL (SELECT coalesce(array_agg(x.team_id ORDER BY x.team_id), '{}') AS oracle
                                 FROM oracle_reachable_teams(p.id) x) o
          WHERE live IS DISTINCT FROM oracle
         UNION ALL
         SELECT 'team ' || coalesce(t.id::text, 'NULL') || ': live '
                || live::text || ' vs oracle ' || oracle::text
           FROM teams t
           CROSS JOIN LATERAL (SELECT coalesce(array_agg(x.team_id ORDER BY x.team_id), '{}') AS live
                                 FROM team_ancestors(t.id) x) l
           CROSS JOIN LATERAL (SELECT coalesce(array_agg(x.team_id ORDER BY x.team_id), '{}') AS oracle
                                 FROM oracle_team_ancestors(t.id) x) o
          WHERE live IS DISTINCT FROM oracle",
    )
    .bind(prefix)
    .fetch_all(pool)
    .await
    .expect("closure comparison")
}

// =================================================================================================
// Row helpers. Raw rows, not doors: the oracle is defined over every state the schema admits.
// =================================================================================================

async fn team(pool: &PgPool, slug: &str, live: bool) -> Uuid {
    sqlx::query_scalar(
        "INSERT INTO kb_teams (slug, name, is_active) VALUES ($1, $1, $2) RETURNING id",
    )
    .bind(slug)
    .bind(live)
    .fetch_one(pool)
    .await
    .expect("seed team")
}

async fn profile(pool: &PgPool, handle: &str) -> Uuid {
    sqlx::query_scalar(
        "INSERT INTO kb_profiles (handle, display_name) VALUES ($1, $1) RETURNING id",
    )
    .bind(handle)
    .fetch_one(pool)
    .await
    .expect("seed profile")
}

async fn link(pool: &PgPool, child: Uuid, parent: Uuid) {
    sqlx::query("INSERT INTO kb_teams_parents (child_id, parent_id) VALUES ($1, $2)")
        .bind(child)
        .bind(parent)
        .execute(pool)
        .await
        .expect("seed parent link");
}

/// A parent link the cycle trigger would refuse, inserted with the trigger off. Cycles are
/// admitted states (the trigger can be sidestepped), so the walk must terminate on one and agree.
async fn link_past_the_cycle_trigger(pool: &PgPool, child: Uuid, parent: Uuid) {
    let mut tx = pool.begin().await.expect("begin");
    sqlx::query("ALTER TABLE kb_teams_parents DISABLE TRIGGER trg_team_parent_link_refuses_cycle")
        .execute(&mut *tx)
        .await
        .expect("disable the cycle trigger");
    sqlx::query("INSERT INTO kb_teams_parents (child_id, parent_id) VALUES ($1, $2)")
        .bind(child)
        .bind(parent)
        .execute(&mut *tx)
        .await
        .expect("seed a cycle link");
    sqlx::query("ALTER TABLE kb_teams_parents ENABLE TRIGGER trg_team_parent_link_refuses_cycle")
        .execute(&mut *tx)
        .await
        .expect("re-enable the cycle trigger");
    tx.commit().await.expect("commit");
}

async fn join(pool: &PgPool, team_id: Uuid, who: Uuid) {
    sqlx::query(
        "INSERT INTO kb_team_members (team_id, profile_id, role) VALUES ($1, $2, 'member')",
    )
    .bind(team_id)
    .bind(who)
    .execute(pool)
    .await
    .expect("seed membership");
}

async fn reach(pool: &PgPool, who: Uuid) -> Vec<Uuid> {
    sqlx::query_scalar("SELECT team_id FROM profile_reachable_teams($1) ORDER BY team_id")
        .bind(who)
        .fetch_all(pool)
        .await
        .expect("reach")
}

// =================================================================================================
// The curated world.
// =================================================================================================

struct Curated {
    top: Uuid,
    mid: Uuid,
    leaf: Uuid,
    ghost: Uuid,
    d_top: Uuid,
    d_left: Uuid,
    d_right: Uuid,
    d_leaf: Uuid,
    c1: Uuid,
    c2: Uuid,
    c3: Uuid,
    /// Member of `leaf`, under `mid` (dead) under `top`: the step floor's cell.
    m_cut: Uuid,
    /// Member of `ghost` (dead) only, under `top`: the seed floor's cell.
    m_ghost: Uuid,
    /// Member of `d_leaf`: the diamond, `d_left` dead and `d_right` live, both under `d_top`.
    m_diamond: Uuid,
    /// Member of `c1` in the cycle c1 ⇄ c2, with c3 above c2.
    m_cycle: Uuid,
    /// The bottom of a 70-team chain whose 67th team is dead, with a back-edge inside the live
    /// stretch that the walk must run round: reach is exactly the 66 teams below the dead one. Deeper than any depth cap a
    /// mutation might add, and wider than a LIMIT on the walk's output.
    deep: Vec<Uuid>,
    m_deep: Uuid,
    /// A member of 300 leaf teams under one live parent: reach is 301 teams. Holds the profile
    /// path to its full seed set (a truncated seed array would drop teams).
    wide: Vec<Uuid>,
    m_wide: Uuid,
    /// leaf2 under live1 under dead2 under top2: a dead team two steps up, so a floor applied only
    /// on the first step cannot pass.
    later: [Uuid; 4],
    m_later: Uuid,
}

const DEEP: usize = 70;
const DEEP_DEAD_AT: usize = 66;
const WIDE: usize = 300;

/// ```text
///   top ─▸ mid (dead) ─▸ leaf ◂─ m_cut            top ─▸ ghost (dead) ◂─ m_ghost
///   d_top ─▸ d_left (dead) ─▸ d_leaf ◂─ m_diamond
///   d_top ─▸ d_right ─────▸ d_leaf
///   c3 ─▸ c2 ⇄ c1 ◂─ m_cycle                       dead_top (dead) ─▸ under_dead_top
///   m_many: member of leaf, ghost, d_leaf and under_dead_top        m_none: no team
/// ```
async fn curated(pool: &PgPool) -> Curated {
    let top = team(pool, "cw-top", true).await;
    let mid = team(pool, "cw-mid", false).await;
    let leaf = team(pool, "cw-leaf", true).await;
    let ghost = team(pool, "cw-ghost", false).await;
    let d_top = team(pool, "cw-d-top", true).await;
    let d_left = team(pool, "cw-d-left", false).await;
    let d_right = team(pool, "cw-d-right", true).await;
    let d_leaf = team(pool, "cw-d-leaf", true).await;
    let dead_top = team(pool, "cw-dead-top", false).await;
    let under_dead_top = team(pool, "cw-under-dead-top", true).await;
    let c1 = team(pool, "cw-c1", true).await;
    let c2 = team(pool, "cw-c2", true).await;
    let c3 = team(pool, "cw-c3", true).await;

    link(pool, mid, top).await;
    link(pool, leaf, mid).await;
    link(pool, ghost, top).await;
    link(pool, d_left, d_top).await;
    link(pool, d_right, d_top).await;
    link(pool, d_leaf, d_left).await;
    link(pool, d_leaf, d_right).await;
    link(pool, under_dead_top, dead_top).await;
    link(pool, c1, c2).await;
    link(pool, c2, c3).await;
    link_past_the_cycle_trigger(pool, c2, c1).await;

    let m_cut = profile(pool, "cw-m-cut").await;
    let m_ghost = profile(pool, "cw-m-ghost").await;
    let m_diamond = profile(pool, "cw-m-diamond").await;
    let m_cycle = profile(pool, "cw-m-cycle").await;
    let m_many = profile(pool, "cw-m-many").await;
    profile(pool, "cw-m-none").await;
    join(pool, leaf, m_cut).await;
    join(pool, ghost, m_ghost).await;
    join(pool, d_leaf, m_diamond).await;
    join(pool, c1, m_cycle).await;
    for t in [leaf, ghost, d_leaf, under_dead_top] {
        join(pool, t, m_many).await;
    }

    let mut deep = Vec::with_capacity(DEEP);
    for i in 0..DEEP {
        deep.push(team(pool, &format!("cw-deep-{i}"), i != DEEP_DEAD_AT).await);
    }
    for i in 1..DEEP {
        link(pool, deep[i - 1], deep[i]).await;
    }
    // A back-edge inside the live stretch, 60 up to 5, so the walk runs round a cycle of 56 teams.
    link_past_the_cycle_trigger(pool, deep[60], deep[5]).await;
    let m_deep = profile(pool, "cw-m-deep").await;
    join(pool, deep[0], m_deep).await;

    let wide_top = team(pool, "cw-wide-top", true).await;
    let mut wide = vec![wide_top];
    let m_wide = profile(pool, "cw-m-wide").await;
    for i in 0..WIDE {
        let t = team(pool, &format!("cw-wide-{i}"), true).await;
        link(pool, t, wide_top).await;
        join(pool, t, m_wide).await;
        wide.push(t);
    }

    let later = [
        team(pool, "cw-later-leaf", true).await,
        team(pool, "cw-later-live1", true).await,
        team(pool, "cw-later-dead2", false).await,
        team(pool, "cw-later-top", true).await,
    ];
    for w in later.windows(2) {
        link(pool, w[0], w[1]).await;
    }
    let m_later = profile(pool, "cw-m-later").await;
    join(pool, later[0], m_later).await;

    Curated {
        top,
        mid,
        leaf,
        ghost,
        d_top,
        d_left,
        d_right,
        d_leaf,
        c1,
        c2,
        c3,
        m_cut,
        m_ghost,
        m_diamond,
        m_cycle,
        deep,
        m_deep,
        wide,
        m_wide,
        later,
        m_later,
    }
}

/// A profile's raw-row personal team (if the profile trigger made one) is not part of a cell, so
/// the cell assertions read reach restricted to the world's own teams.
fn within(reach: &[Uuid], world: &[Uuid]) -> Vec<Uuid> {
    let mut v: Vec<Uuid> = reach
        .iter()
        .copied()
        .filter(|t| world.contains(t))
        .collect();
    v.sort();
    v
}

fn sorted(mut v: Vec<Uuid>) -> Vec<Uuid> {
    v.sort();
    v
}

#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn the_one_walk_returns_the_pre_migration_sets_on_every_curated_cell(pool: PgPool) {
    install_oracle(&pool).await;
    let w = curated(&pool).await;

    let diffs = closure_diffs(&pool).await;
    assert!(
        diffs.is_empty(),
        "live closure differs from the frozen oracle: {diffs:#?}"
    );

    // The cells pin the rule itself, not only the agreement: a change that moved the live walk and
    // the oracle together would still have to keep these.
    let world = [
        w.top, w.mid, w.leaf, w.ghost, w.d_top, w.d_left, w.d_right, w.d_leaf, w.c1, w.c2, w.c3,
    ];
    assert_eq!(
        within(&reach(&pool, w.m_cut).await, &world),
        vec![w.leaf],
        "a dead intermediate cuts reach above it"
    );
    assert!(
        within(&reach(&pool, w.m_ghost).await, &world).is_empty(),
        "membership in a dead team confers nothing, not even its live parent"
    );
    assert_eq!(
        within(&reach(&pool, w.m_diamond).await, &world),
        sorted(vec![w.d_leaf, w.d_right, w.d_top]),
        "a diamond reaches the top through its live path, never the dead team"
    );
    assert_eq!(
        within(&reach(&pool, w.m_cycle).await, &world),
        sorted(vec![w.c1, w.c2, w.c3]),
        "a cycle terminates and reaches what is above it"
    );
    assert_eq!(
        within(&reach(&pool, w.m_deep).await, &w.deep),
        sorted(w.deep[..DEEP_DEAD_AT].to_vec()),
        "a {DEEP}-team chain is walked to the dead team at {DEEP_DEAD_AT} and no further, and the \
         cycle over it adds nothing"
    );
    assert_eq!(
        within(&reach(&pool, w.m_wide).await, &w.wide).len(),
        WIDE + 1,
        "a member of {WIDE} teams reaches every one of them and their parent"
    );
    assert_eq!(
        within(&reach(&pool, w.m_later).await, &w.later),
        sorted(w.later[..2].to_vec()),
        "a dead team two steps up stops the walk there"
    );
}

// =================================================================================================
// Seeded random worlds.
// =================================================================================================

/// xorshift64*, the harness precedent: reproducible from one u64.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
    fn chance(&mut self, percent: u64) -> bool {
        self.below(100) < percent
    }
}

/// One random world under a slug prefix: 20–59 teams, about a fifth dead, each team linked to up to
/// three earlier teams (a DAG), a few back-edges forming cycles, and twelve profiles with zero to
/// six memberships each.
async fn random_world(pool: &PgPool, seed: u64) {
    let mut rng = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
    let n = 20 + rng.below(40) as usize;
    let mut teams = Vec::with_capacity(n);
    for i in 0..n {
        let live = !rng.chance(20);
        teams.push(team(pool, &format!("rw{seed}-t{i}"), live).await);
    }
    for i in 1..n {
        let mut parents = Vec::new();
        for _ in 0..rng.below(4) {
            let j = rng.below(i as u64) as usize;
            if !parents.contains(&j) {
                parents.push(j);
                link(pool, teams[i], teams[j]).await;
            }
        }
    }
    for _ in 0..rng.below(4) {
        let lo = rng.below(n as u64 - 1) as usize;
        let hi = lo + 1 + rng.below((n - lo - 1) as u64) as usize;
        // hi was linked under earlier teams only; an edge from lo up to hi closes a cycle when hi
        // already reaches lo, and is a plain extra link otherwise. Either is an admitted state.
        let exists: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM kb_teams_parents WHERE child_id = $1 AND parent_id = $2)",
        )
        .bind(teams[lo])
        .bind(teams[hi])
        .fetch_one(pool)
        .await
        .expect("probe link");
        if !exists {
            link_past_the_cycle_trigger(pool, teams[lo], teams[hi]).await;
        }
    }
    for p in 0..12 {
        let who = profile(pool, &format!("rw{seed}-p{p}")).await;
        let mut joined = Vec::new();
        for _ in 0..rng.below(7) {
            let t = rng.below(n as u64) as usize;
            if !joined.contains(&t) {
                joined.push(t);
                join(pool, teams[t], who).await;
            }
        }
    }
}

const RANDOM_WORLDS: u64 = 32;

#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn the_one_walk_returns_the_pre_migration_sets_on_random_worlds(pool: PgPool) {
    install_oracle(&pool).await;
    let base: u64 = std::env::var("HARNESS_SEED")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0x5EED_2026_1129);
    for k in 0..RANDOM_WORLDS {
        let seed = base.wrapping_add(k);
        random_world(&pool, seed).await;
        let diffs = closure_diffs_within(&pool, &format!("rw{seed}-")).await;
        assert!(
            diffs.is_empty(),
            "world seed {seed} (HARNESS_SEED={base}, world {k}): live closure differs from the \
             frozen oracle: {diffs:#?}"
        );
    }
    // One full pass at the end, over every world together (and the personal teams the profile
    // trigger made), so nothing outside a world's prefix escapes the comparison.
    let diffs = closure_diffs(&pool).await;
    assert!(
        diffs.is_empty(),
        "the accumulated worlds differ from the frozen oracle: {diffs:#?}"
    );
}

// =================================================================================================
// The bites.
// =================================================================================================

/// The live definition of `team_ancestors_of`, to mutate and to restore from.
async fn walk_definition(pool: &PgPool) -> String {
    sqlx::query_scalar("SELECT pg_get_functiondef('team_ancestors_of(uuid[])'::regprocedure)")
        .fetch_one(pool)
        .await
        .expect("team_ancestors_of's definition")
}

/// Replace `team_ancestors_of` with `definition` minus exactly one occurrence of `floor`.
async fn plant_without(pool: &PgPool, definition: &str, floor: &str) {
    assert_eq!(
        definition.matches(floor).count(),
        1,
        "the floor `{floor}` must occur exactly once in team_ancestors_of, or this bite plants \
         the wrong mutation:\n{definition}"
    );
    sqlx::raw_sql(&definition.replacen(floor, "", 1))
        .execute(pool)
        .await
        .expect("plant the mutation");
}

async fn restore(pool: &PgPool, definition: &str) {
    sqlx::raw_sql(definition)
        .execute(pool)
        .await
        .expect("restore team_ancestors_of");
}

/// B7: the step floor (`pt.is_active`) dropped. The frozen oracle must see it, and the dead
/// intermediate's cell must open.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn the_step_floor_bites(pool: PgPool) {
    install_oracle(&pool).await;
    let w = curated(&pool).await;
    assert!(
        closure_diffs(&pool).await.is_empty(),
        "precondition: green before the plant"
    );

    let definition = walk_definition(&pool).await;
    plant_without(&pool, &definition, "AND pt.is_active").await;

    let diffs = closure_diffs(&pool).await;
    assert!(
        !diffs.is_empty(),
        "dropping the step floor left the frozen oracle comparison green"
    );
    assert!(
        reach(&pool, w.m_cut).await.contains(&w.top),
        "with the step floor gone the walk passes through the dead intermediate, so m_cut \
         reaches top: the mutation was planted where intended"
    );

    restore(&pool, &definition).await;
    let diffs = closure_diffs(&pool).await;
    assert!(diffs.is_empty(), "restored body is not green: {diffs:#?}");
}

/// B8: the seed floor (`t.is_active`) dropped ALONE, `profile_effective_teams` left as it is. On
/// the profile path that floor is redundant, so a profile-only check would stay green. The test
/// also asserts the profile path stays closed under this mutation: that is defence in depth, and it
/// is what makes seeding `profile_reachable_teams` from raw memberships (equivalent today, because
/// the seed floor drops dead seeds) go red here too, so the two floors never come to rest on one. Team-seeded
/// callers are what it protects, and the leak is concrete: a dead T's public_within row becomes
/// effective, and a person under the anchor reads T's resource.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn the_seed_floor_bites(pool: PgPool) {
    install_oracle(&pool).await;
    let w = curated(&pool).await;

    // public_within(T, A): T is dead, A is its live parent, the reader is a person in A's subtree.
    let opener = profile(&pool, "pw-opener").await;
    let anchor = team(&pool, "pw-anchor", true).await;
    let dead_t = team(&pool, "pw-dead-t", true).await;
    link(&pool, dead_t, anchor).await;
    let ctx: Uuid = sqlx::query_scalar(
        "INSERT INTO kb_contexts (owner_table, owner_id, slug, name) \
         VALUES ('kb_teams', $1, 'pw-dead-t-ctx', 'pw-dead-t-ctx') RETURNING id",
    )
    .bind(dead_t)
    .fetch_one(&pool)
    .await
    .expect("T's context");
    let resource: Uuid = sqlx::query_scalar(
        "INSERT INTO kb_resources (title, origin_uri) VALUES ('pw-r', '') RETURNING id",
    )
    .fetch_one(&pool)
    .await
    .expect("T's resource");
    sqlx::query(
        "INSERT INTO kb_resource_homes \
           (resource_id, anchor_table, anchor_id, originator_profile_id, owner_profile_id) \
         VALUES ($1, 'kb_contexts', $2, $3, $3)",
    )
    .bind(resource)
    .bind(ctx)
    .bind(opener)
    .execute(&pool)
    .await
    .expect("home");
    sqlx::query(
        "INSERT INTO kb_team_public_within (team_id, anchor_id, opened_by) VALUES ($1, $2, $3)",
    )
    .bind(dead_t)
    .bind(anchor)
    .bind(opener)
    .execute(&pool)
    .await
    .expect("public_within row");
    let reader = profile(&pool, "pw-reader").await;
    sqlx::query(
        "INSERT INTO kb_profile_auth_links (id, profile_id, auth_provider, auth_provider_user_id) \
         VALUES (gen_random_uuid(), $1, 'auth0', 'pw-reader')",
    )
    .bind(reader)
    .execute(&pool)
    .await
    .expect("a human auth link: the reader is a person");
    join(&pool, anchor, reader).await;

    let reads = || async {
        sqlx::query_scalar::<_, bool>(
            "SELECT EXISTS (SELECT 1 FROM resources_visible_to($1) v WHERE v.resource_id = $2)",
        )
        .bind(reader)
        .bind(resource)
        .fetch_one(&pool)
        .await
        .expect("read probe")
    };
    assert!(reads().await, "fixture: the row confers while T is live");
    sqlx::query("UPDATE kb_teams SET is_active = false WHERE id = $1")
        .bind(dead_t)
        .execute(&pool)
        .await
        .expect("kill T");
    assert!(!reads().await, "precondition: a dead T's row is dormant");
    assert!(
        closure_diffs(&pool).await.is_empty(),
        "precondition: green before the plant"
    );

    let definition = walk_definition(&pool).await;
    plant_without(&pool, &definition, "AND t.is_active").await;

    let diffs = closure_diffs(&pool).await;
    assert!(
        diffs.iter().any(|d| d.starts_with("team ")),
        "dropping the seed floor left team_ancestors(dead team) equal to the oracle: {diffs:#?}"
    );
    assert!(
        within(&reach(&pool, w.m_ghost).await, &[w.ghost, w.top]).is_empty(),
        "the profile path stays closed (profile_effective_teams floors liveness), which is why a \
         profile-only check cannot see this mutation"
    );
    assert!(
        reads().await,
        "with the seed floor gone, a dead T's public_within row confers: the leak this floor stops"
    );

    restore(&pool, &definition).await;
    assert!(
        closure_diffs(&pool).await.is_empty(),
        "restored body is not green"
    );
    assert!(
        !reads().await,
        "restored: the dead T's row is dormant again"
    );
}
