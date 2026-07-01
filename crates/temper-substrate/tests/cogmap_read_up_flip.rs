#![cfg(feature = "artifact-tests")]
//! Deliverable 4 of the generalized access-capability arc (design doc §3.4 + §4 step 3): the
//! cogmap-read direction lockstep flip. The three flat cogmap-read predicates
//! (`cogmap_readable_by_profile`, `cogmap_visible_maps`, `resources_visible_to`'s cogmap branch) move
//! their MEMBERSHIP join from the flat `profile_effective_teams(p)` to the UP+union
//! `profile_effective_teams(p) ⋈ team_ancestors(·)` — so a member of a CHILD team now reads a map
//! joined to an ANCESTOR (parent) team.
//!
//! This is a visibility EXPANSION, which (per #219's lesson) needs its own scenario proof over a REAL
//! team hierarchy — synthetic anchors cannot exercise the membership/ancestor paths. Coverage:
//!   • EXPANSION — a child-team member reads a parent-joined map's shape, homed resource, and wayfind
//!     admission (all three agreeing at the up-expanded level);
//!   • REVERSE NON-LEAK — a parent-team member does NOT reach a child-only map (ancestors go up, not
//!     down);
//!   • UNRELATED — a member of an unrelated team sees none of it.
//!
//! Every resource is owned+originated by a distinct `owner` profile (never one of the principals under
//! test), so a green is genuinely the cogmap-membership branch, never the ownership branch.

use uuid::Uuid;

async fn insert_profile(pool: &sqlx::PgPool, handle: &str) -> Uuid {
    // system_access 'none' → no auto-join to the temper-system root, so a profile's effective teams are
    // exactly the memberships we seed (the ancestor walk is the only reach under test).
    sqlx::query_scalar(
        "INSERT INTO kb_profiles (handle, display_name, system_access) \
         VALUES ($1, $1, 'none') RETURNING id",
    )
    .bind(handle)
    .fetch_one(pool)
    .await
    .unwrap()
}

async fn create_team(pool: &sqlx::PgPool, slug: &str) -> Uuid {
    sqlx::query_scalar("INSERT INTO kb_teams (slug, name) VALUES ($1, $1) RETURNING id")
        .bind(slug)
        .fetch_one(pool)
        .await
        .unwrap()
}

/// Make `child` a child of `parent` in the teams DAG, so `team_ancestors(child) = {child, parent}`.
async fn link_parent(pool: &sqlx::PgPool, parent: Uuid, child: Uuid) {
    sqlx::query("INSERT INTO kb_teams_parents (parent_id, child_id) VALUES ($1, $2)")
        .bind(parent)
        .bind(child)
        .execute(pool)
        .await
        .unwrap();
}

async fn add_member(pool: &sqlx::PgPool, team: Uuid, profile: Uuid) {
    sqlx::query(
        "INSERT INTO kb_team_members (team_id, profile_id, role) VALUES ($1, $2, 'member')",
    )
    .bind(team)
    .bind(profile)
    .execute(pool)
    .await
    .unwrap();
}

/// A real cognitive map (with the NOT-NULL telos resource) joined to `team`. Returns the cogmap id.
async fn create_map_joined_to(pool: &sqlx::PgPool, name: &str, team: Uuid) -> Uuid {
    let telos: Uuid = sqlx::query_scalar(
        "INSERT INTO kb_resources (title, origin_uri) VALUES ($1, $2) RETURNING id",
    )
    .bind(format!("{name}-telos"))
    .bind(format!("temper://d4/{name}/telos"))
    .fetch_one(pool)
    .await
    .unwrap();
    let cogmap: Uuid = sqlx::query_scalar(
        "INSERT INTO kb_cogmaps (name, telos_resource_id) VALUES ($1, $2) RETURNING id",
    )
    .bind(name)
    .bind(telos)
    .fetch_one(pool)
    .await
    .unwrap();
    sqlx::query("INSERT INTO kb_team_cogmaps (cogmap_id, team_id) VALUES ($1, $2)")
        .bind(cogmap)
        .bind(team)
        .execute(pool)
        .await
        .unwrap();
    cogmap
}

/// A resource homed in `cogmap`, owned+originated by `owner` (distinct from every principal under
/// test), so ownership never confers visibility to the readers.
async fn insert_cogmap_homed_resource(
    pool: &sqlx::PgPool,
    title: &str,
    cogmap: Uuid,
    owner: Uuid,
) -> Uuid {
    let rid: Uuid = sqlx::query_scalar(
        "INSERT INTO kb_resources (title, origin_uri) VALUES ($1, $2) RETURNING id",
    )
    .bind(title)
    .bind(format!("temper://d4/{title}"))
    .fetch_one(pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO kb_resource_homes (resource_id, anchor_table, anchor_id, originator_profile_id, owner_profile_id) \
         VALUES ($1, 'kb_cogmaps', $2, $3, $3)",
    )
    .bind(rid)
    .bind(cogmap)
    .bind(owner)
    .execute(pool)
    .await
    .unwrap();
    rid
}

async fn cogmap_readable(pool: &sqlx::PgPool, profile: Uuid, cogmap: Uuid) -> bool {
    sqlx::query_scalar("SELECT cogmap_readable_by_profile($1, $2)")
        .bind(profile)
        .bind(cogmap)
        .fetch_one(pool)
        .await
        .unwrap()
}

async fn resource_visible(pool: &sqlx::PgPool, profile: Uuid, resource: Uuid) -> bool {
    sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM resources_visible_to($1) v WHERE v.resource_id=$2)",
    )
    .bind(profile)
    .bind(resource)
    .fetch_one(pool)
    .await
    .unwrap()
}

async fn map_in_visible_maps(pool: &sqlx::PgPool, profile: Uuid, cogmap: Uuid) -> bool {
    sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM cogmap_visible_maps($1) m WHERE m=$2)")
        .bind(profile)
        .bind(cogmap)
        .fetch_one(pool)
        .await
        .unwrap()
}

#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn child_member_reads_ancestor_joined_map(pool: sqlx::PgPool) {
    let owner = insert_profile(&pool, "d4_owner").await; // owns the resources; never a reader
    let child_member = insert_profile(&pool, "d4_child_member").await;
    let parent_member = insert_profile(&pool, "d4_parent_member").await;
    let unrelated = insert_profile(&pool, "d4_unrelated").await;

    let parent = create_team(&pool, "d4-parent").await;
    let child = create_team(&pool, "d4-child").await;
    let unrel = create_team(&pool, "d4-unrelated").await;
    link_parent(&pool, parent, child).await; // team_ancestors(child) = {child, parent}

    add_member(&pool, child, child_member).await;
    add_member(&pool, parent, parent_member).await;
    add_member(&pool, unrel, unrelated).await;

    // The map + its homed resource live on the PARENT team.
    let parent_map = create_map_joined_to(&pool, "d4-parent-map", parent).await;
    let resource = insert_cogmap_homed_resource(&pool, "d4-parent-doc", parent_map, owner).await;

    // ── EXPANSION: the child-team member reads the parent-joined map, all three predicates agreeing.
    assert!(
        cogmap_readable(&pool, child_member, parent_map).await,
        "child-team member reads the ancestor-joined map's shape"
    );
    assert!(
        resource_visible(&pool, child_member, resource).await,
        "child-team member reads a resource homed in the ancestor-joined map"
    );
    assert!(
        map_in_visible_maps(&pool, child_member, parent_map).await,
        "child-team member's wayfind admits the ancestor-joined map"
    );

    // ── REVERSE NON-LEAK: a map joined to the CHILD is NOT reachable by the PARENT member.
    let child_map = create_map_joined_to(&pool, "d4-child-map", child).await;
    let child_resource =
        insert_cogmap_homed_resource(&pool, "d4-child-doc", child_map, owner).await;
    assert!(
        !cogmap_readable(&pool, parent_member, child_map).await,
        "parent-team member does NOT read a child-only map (ancestors go up, not down)"
    );
    assert!(
        !resource_visible(&pool, parent_member, child_resource).await,
        "parent-team member does NOT read a child-only map's resource"
    );
    assert!(
        !map_in_visible_maps(&pool, parent_member, child_map).await,
        "parent-team member's wayfind does NOT admit a child-only map"
    );

    // ── UNRELATED: a member of an unrelated team sees none of the parent map.
    assert!(!cogmap_readable(&pool, unrelated, parent_map).await);
    assert!(!resource_visible(&pool, unrelated, resource).await);
    assert!(!map_in_visible_maps(&pool, unrelated, parent_map).await);

    // Sanity: the parent member DOES read their own parent-joined map (the flip never narrows).
    assert!(
        cogmap_readable(&pool, parent_member, parent_map).await,
        "parent-team member still reads the parent-joined map (no narrowing)"
    );
}
