//! SQL-level semantics for the A2 cogmap-scoped Atlas neighborhood-slice
//! functions (`graph_traverse_cogmap_scoped`, `resources_in_cogmap_scope`) —
//! the cogmap-door analog of `graph_atlas_slice_sql_test.rs`'s team stack.
//! Proves the new functions directly against the migrated schema: an
//! in-cogmap incoming edge is reachable, and a resource not visible to the
//! profile is excluded even though it is homed in the same cogmap.
#![cfg(feature = "test-db")]

mod common;

use temper_core::types::graph::EdgeKind;
use uuid::Uuid;

/// Insert a profile with the given handle, return its id.
async fn mk_profile(pool: &sqlx::PgPool, handle: &str) -> Uuid {
    sqlx::query_scalar(
        "INSERT INTO kb_profiles (handle, display_name) VALUES ($1, $1) RETURNING id",
    )
    .bind(handle)
    .fetch_one(pool)
    .await
    .expect("insert profile")
}

async fn create_team(pool: &sqlx::PgPool, slug: &str) -> Uuid {
    sqlx::query_scalar("INSERT INTO kb_teams (slug, name) VALUES ($1, $1) RETURNING id")
        .bind(slug)
        .fetch_one(pool)
        .await
        .expect("create team")
}

async fn add_member(pool: &sqlx::PgPool, team: Uuid, profile: Uuid) {
    sqlx::query(
        "INSERT INTO kb_team_members (team_id, profile_id, role) VALUES ($1, $2, 'member')",
    )
    .bind(team)
    .bind(profile)
    .execute(pool)
    .await
    .expect("add member");
}

async fn create_resource(pool: &sqlx::PgPool, title: &str, origin: &str) -> Uuid {
    sqlx::query_scalar("INSERT INTO kb_resources (title, origin_uri) VALUES ($1, $2) RETURNING id")
        .bind(title)
        .bind(origin)
        .fetch_one(pool)
        .await
        .expect("insert resource")
}

async fn create_cogmap(pool: &sqlx::PgPool, name: &str, telos_resource: Uuid) -> Uuid {
    sqlx::query_scalar(
        "INSERT INTO kb_cogmaps (name, telos_resource_id) VALUES ($1, $2) RETURNING id",
    )
    .bind(name)
    .bind(telos_resource)
    .fetch_one(pool)
    .await
    .expect("create cogmap")
}

/// Join a team to a cogmap — the mechanism `cogmap_readable_by_profile` reads
/// (`kb_team_cogmaps` ⋈ `profile_effective_teams`). This is what makes an edge
/// HOMED in the cogmap pass `anchor_readable_by_profile`'s `kb_cogmaps` branch,
/// which delegates straight to `cogmap_readable_by_profile`
/// (`migrations/20260624000002_canonical_functions.sql:277`, unchanged by the
/// read-up-flip in `20260701000002_cogmap_read_up_flip.sql`).
async fn join_cogmap(pool: &sqlx::PgPool, cogmap: Uuid, team: Uuid) {
    sqlx::query("INSERT INTO kb_team_cogmaps (cogmap_id, team_id) VALUES ($1, $2)")
        .bind(cogmap)
        .bind(team)
        .execute(pool)
        .await
        .expect("join cogmap");
}

/// Home a resource in the cogmap AND make it visible to `profile` in one shot:
/// `resources_visible_to`'s owned/originated clause
/// (`migrations/20260624000002_canonical_functions.sql`, carried forward
/// unchanged by `20260629000006_cogmap_resource_visibility.sql`) grants
/// visibility whenever `h.owner_profile_id = p_profile OR h.originator_profile_id
/// = p_profile`. Setting both FKs to `profile` on the `kb_resource_homes` row
/// therefore satisfies both halves of `resources_in_cogmap_scope`'s predicate —
/// homed in THIS cogmap (`anchor_table`/`anchor_id` match), AND in
/// `resources_visible_to(profile)` — without a separate grant. Same pattern
/// `graph_territory_slice_sql_test.rs` uses for its visible region members.
async fn home_in_cogmap(pool: &sqlx::PgPool, resource: Uuid, cogmap: Uuid, profile: Uuid) {
    sqlx::query(
        "INSERT INTO kb_resource_homes \
             (resource_id, anchor_table, anchor_id, originator_profile_id, owner_profile_id) \
         VALUES ($1, 'kb_cogmaps', $2, $3, $3)",
    )
    .bind(resource)
    .bind(cogmap)
    .bind(profile)
    .execute(pool)
    .await
    .expect("home resource in cogmap");
}

async fn assert_edge(
    pool: &sqlx::PgPool,
    source: Uuid,
    target: Uuid,
    edge_kind: EdgeKind,
    home_anchor_table: &str,
    home_anchor_id: Uuid,
    event: Uuid,
) -> Uuid {
    sqlx::query_scalar(
        "INSERT INTO kb_edges \
             (source_table, source_id, target_table, target_id, edge_kind, \
              home_anchor_table, home_anchor_id, asserted_by_event_id, last_event_id) \
         VALUES ('kb_resources', $1, 'kb_resources', $2, $3, $4, $5, $6, $6) \
         RETURNING id",
    )
    .bind(source)
    .bind(target)
    .bind(edge_kind)
    .bind(home_anchor_table)
    .bind(home_anchor_id)
    .bind(event)
    .fetch_one(pool)
    .await
    .expect("assert edge")
}

/// Any pre-existing kb_events row (the L0 kernel cogmap genesis migration inserts
/// one) — sufficient FK target for asserted_by_event_id/last_event_id in these tests.
async fn any_event(pool: &sqlx::PgPool) -> Uuid {
    sqlx::query_scalar("SELECT id FROM kb_events LIMIT 1")
        .fetch_one(pool)
        .await
        .expect("at least one kb_events row exists (L0 genesis)")
}

/// A1 (cogmap door): a seed whose ONLY edge is *incoming* (neighbor -> seed)
/// must still be reachable through `graph_traverse_cogmap_scoped` — the walk
/// seeds the frontier NODE set and follows edges in either direction, exactly
/// like the team-scoped `graph_traverse_scoped`.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn cogmap_walk_reaches_in_cogmap_neighbor(pool: sqlx::PgPool) {
    let profile = mk_profile(&pool, "cogmap-walk-tester").await;
    let event = any_event(&pool).await;

    let telos = create_resource(&pool, "telos", "temper://cogmap-walk/telos").await;
    let cogmap = create_cogmap(&pool, "cogmap-walk-map", telos).await;

    // Make the cogmap (hence a `kb_cogmaps`-homed edge) readable by `profile`:
    // join it to a team the profile is a member of.
    let team = create_team(&pool, "cogmap-walk-team").await;
    add_member(&pool, team, profile).await;
    join_cogmap(&pool, cogmap, team).await;

    // seed S and neighbor N, both homed in the cogmap and visible to `profile`;
    // the ONLY edge is N -> S (incoming to S), homed in the cogmap itself.
    let seed = create_resource(&pool, "seed", "temper://cogmap-walk/seed").await;
    let nbr = create_resource(&pool, "neighbor", "temper://cogmap-walk/neighbor").await;
    home_in_cogmap(&pool, seed, cogmap, profile).await;
    home_in_cogmap(&pool, nbr, cogmap, profile).await;

    assert_edge(
        &pool,
        nbr,
        seed,
        EdgeKind::Contains,
        "kb_cogmaps",
        cogmap,
        event,
    )
    .await;

    let rows: Vec<(Uuid, Uuid, Uuid)> = sqlx::query_as(
        "SELECT id, source_id, target_id FROM graph_traverse_cogmap_scoped($1,$2,$3,$4,$5)",
    )
    .bind(profile)
    .bind(cogmap)
    .bind(vec![seed])
    .bind(2_i32)
    .bind(Vec::<EdgeKind>::new()) // empty edge_kinds => all kinds
    .fetch_all(&pool)
    .await
    .expect("graph_traverse_cogmap_scoped");

    assert!(
        rows.iter().any(|(_, s, t)| *s == nbr && *t == seed),
        "incoming in-cogmap edge must be reachable: {rows:?}"
    );
}

/// A resource homed in the SAME cogmap but not visible to the caller (no
/// owner/originator tie, no team/context grant) must be excluded from the
/// cogmap scope entirely — `resources_in_cogmap_scope` intersects
/// `kb_resource_homes` with `resources_visible_to`, not just the anchor match.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn cogmap_scope_excludes_homed_but_not_visible_resource(pool: sqlx::PgPool) {
    let profile = mk_profile(&pool, "cogmap-scope-tester").await;

    let telos = create_resource(&pool, "telos", "temper://cogmap-scope/telos").await;
    let cogmap = create_cogmap(&pool, "cogmap-scope-map", telos).await;

    let visible = create_resource(&pool, "visible", "temper://cogmap-scope/visible").await;
    home_in_cogmap(&pool, visible, cogmap, profile).await;

    // Homed in the same cogmap, but owned/originated by a DIFFERENT profile and
    // granted to nobody — resources_visible_to(profile) excludes it.
    let other_profile = mk_profile(&pool, "cogmap-scope-other").await;
    let not_visible = create_resource(&pool, "not visible", "temper://cogmap-scope/nv").await;
    home_in_cogmap(&pool, not_visible, cogmap, other_profile).await;

    let rows: Vec<(Uuid,)> =
        sqlx::query_as("SELECT resource_id FROM resources_in_cogmap_scope($1, $2)")
            .bind(profile)
            .bind(cogmap)
            .fetch_all(&pool)
            .await
            .expect("resources_in_cogmap_scope");

    let ids: Vec<Uuid> = rows.into_iter().map(|(id,)| id).collect();
    assert!(ids.contains(&visible), "visible resource must be in scope");
    assert!(
        !ids.contains(&not_visible),
        "homed-but-not-visible resource must be excluded: {ids:?}"
    );
}
