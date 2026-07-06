//! SQL-level semantics for the R3 territory-slice function
//! (`graph_region_members`). Proves the function directly against the
//! migrated schema: visibility-scoped member deref — a member outside
//! `resources_visible_to` is excluded, and a member with no `doc_type`
//! property still surfaces (LEFT JOIN, no erasure) — before the HTTP
//! endpoint is exercised (that is `graph_territory_slice_e2e.rs`).
//!
//! A3: `graph_region_components` (and its projection) was dropped — components
//! are a region's PARENT grain, not its sub-clusters, and the function had no
//! `reg.id` tie anyway. See `migrations/20260706120100_drop_graph_region_components.sql`.
#![cfg(feature = "test-db")]

mod common;

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

async fn create_resource(pool: &sqlx::PgPool, title: &str, origin: &str) -> Uuid {
    sqlx::query_scalar("INSERT INTO kb_resources (title, origin_uri) VALUES ($1, $2) RETURNING id")
        .bind(title)
        .bind(origin)
        .fetch_one(pool)
        .await
        .expect("insert resource")
}

async fn set_doc_type(pool: &sqlx::PgPool, resource: Uuid, doc_type: &str, event: Uuid) {
    sqlx::query(
        "INSERT INTO kb_properties \
             (owner_table, owner_id, property_key, property_value, asserted_by_event_id, last_event_id) \
         VALUES ('kb_resources', $1, 'doc_type', to_jsonb($2::text), $3, $3)",
    )
    .bind(resource)
    .bind(doc_type)
    .bind(event)
    .execute(pool)
    .await
    .expect("set doc_type");
}

/// Any pre-existing kb_events row (the L0 kernel cogmap genesis migration inserts
/// one) — sufficient FK target for asserted_by_event_id/last_event_id in these tests.
async fn any_event(pool: &sqlx::PgPool) -> Uuid {
    sqlx::query_scalar("SELECT id FROM kb_events LIMIT 1")
        .fetch_one(pool)
        .await
        .expect("at least one kb_events row exists (L0 genesis)")
}

/// The global `telos-default` lens (cogmap_id IS NULL) — seeded by canonical_seed.
async fn telos_default_lens(pool: &sqlx::PgPool) -> Uuid {
    sqlx::query_scalar(
        "SELECT id FROM kb_cogmap_lenses WHERE name = 'telos-default' AND cogmap_id IS NULL LIMIT 1",
    )
    .fetch_one(pool)
    .await
    .expect("telos-default lens seeded by canonical_seed")
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

/// 768-dim zero pgvector text literal — determinism of the region's
/// centroid does not matter for this function (it doesn't cosine-rank).
fn zero_vec768() -> String {
    let v = vec!["0"; 768];
    format!("[{}]", v.join(","))
}

async fn insert_region(
    pool: &sqlx::PgPool,
    cogmap: Uuid,
    lens: Uuid,
    label: &str,
    member_count: i32,
    salience: f64,
    event: Uuid,
) -> Uuid {
    sqlx::query_scalar(
        "INSERT INTO kb_cogmap_regions \
             (cogmap_id, lens_id, centroid, salience, label, member_count, asserted_by_event_id, last_event_id) \
         VALUES ($1, $2, $3::vector, $4, $5, $6, $7, $7) RETURNING id",
    )
    .bind(cogmap)
    .bind(lens)
    .bind(zero_vec768())
    .bind(salience)
    .bind(label)
    .bind(member_count)
    .bind(event)
    .fetch_one(pool)
    .await
    .expect("insert region")
}

async fn add_region_member(pool: &sqlx::PgPool, region: Uuid, member: Uuid, affinity: Option<f64>) {
    sqlx::query(
        "INSERT INTO kb_cogmap_region_members (region_id, member_table, member_id, affinity) \
         VALUES ($1, 'kb_resources', $2, $3)",
    )
    .bind(region)
    .bind(member)
    .bind(affinity)
    .execute(pool)
    .await
    .expect("add region member");
}

/// A region with three candidate members — one visible (with a doc_type),
/// one visible but doc-type-less (must still surface, no INNER-JOIN
/// erasure), and one NOT visible to the caller (must be excluded).
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn slice_functions_scope_members_by_visibility(pool: sqlx::PgPool) {
    let profile = mk_profile(&pool, "gts-tester").await;
    let event = any_event(&pool).await;
    let lens = telos_default_lens(&pool).await;

    let telos = create_resource(&pool, "telos", "temper://gts/telos").await;
    let cogmap = create_cogmap(&pool, "gts-cogmap", telos).await;

    // Make the cogmap readable by `profile`: join it to a team the profile is a member
    // of (cogmap_readable_by_profile gates on kb_team_cogmaps, not resource-home ownership).
    let team: Uuid =
        sqlx::query_scalar("INSERT INTO kb_teams (slug, name) VALUES ($1, $1) RETURNING id")
            .bind("gts-sql-team")
            .fetch_one(&pool)
            .await
            .expect("insert team");
    sqlx::query(
        "INSERT INTO kb_team_members (team_id, profile_id, role) VALUES ($1, $2, 'member')",
    )
    .bind(team)
    .bind(profile)
    .execute(&pool)
    .await
    .expect("add member");
    sqlx::query("INSERT INTO kb_team_cogmaps (cogmap_id, team_id) VALUES ($1, $2)")
        .bind(cogmap)
        .bind(team)
        .execute(&pool)
        .await
        .expect("join cogmap team");

    let region = insert_region(&pool, cogmap, lens, "Region GTS", 2, 0.6, event).await;

    // Visible member with a doc_type — owned by `profile`, so it lands in
    // resources_visible_to(profile) via the owned/originated branch.
    let visible_typed = create_resource(&pool, "visible typed", "temper://gts/vt").await;
    sqlx::query(
        "INSERT INTO kb_resource_homes \
             (resource_id, anchor_table, anchor_id, originator_profile_id, owner_profile_id) \
         VALUES ($1, 'kb_cogmaps', $2, $3, $3)",
    )
    .bind(visible_typed)
    .bind(cogmap)
    .bind(profile)
    .execute(&pool)
    .await
    .expect("home visible_typed");
    set_doc_type(&pool, visible_typed, "concept", event).await;

    // Visible member with NO doc_type — must still surface with doc_type NULL.
    let visible_untyped = create_resource(&pool, "visible untyped", "temper://gts/vu").await;
    sqlx::query(
        "INSERT INTO kb_resource_homes \
             (resource_id, anchor_table, anchor_id, originator_profile_id, owner_profile_id) \
         VALUES ($1, 'kb_cogmaps', $2, $3, $3)",
    )
    .bind(visible_untyped)
    .bind(cogmap)
    .bind(profile)
    .execute(&pool)
    .await
    .expect("home visible_untyped");

    // NOT visible: no resource_home rows tying it to `profile` at all, and no
    // team/context path grants it — resources_visible_to(profile) excludes it.
    let not_visible = create_resource(&pool, "not visible", "temper://gts/nv").await;

    add_region_member(&pool, region, visible_typed, Some(0.9)).await;
    add_region_member(&pool, region, visible_untyped, Some(0.5)).await;
    add_region_member(&pool, region, not_visible, Some(0.1)).await;

    // ── graph_region_members ────────────────────────────────────────────
    let members: Vec<(Uuid, String, Option<String>, Option<f64>)> =
        sqlx::query_as("SELECT id, title, doc_type, affinity FROM graph_region_members($1, $2)")
            .bind(profile)
            .bind(region)
            .fetch_all(&pool)
            .await
            .expect("graph_region_members");

    assert_eq!(
        members.len(),
        2,
        "only the two visible members surface, not_visible excluded: {members:?}"
    );
    assert!(
        members.iter().all(|(id, ..)| *id != not_visible),
        "the non-visible member must never appear: {members:?}"
    );

    let typed_row = members
        .iter()
        .find(|(id, ..)| *id == visible_typed)
        .expect("visible_typed present");
    assert_eq!(typed_row.2.as_deref(), Some("concept"));

    let untyped_row = members
        .iter()
        .find(|(id, ..)| *id == visible_untyped)
        .expect("visible_untyped present");
    assert_eq!(
        untyped_row.2, None,
        "doc-type-less member projects doc_type = NULL, not erased by a LEFT JOIN: {untyped_row:?}"
    );

    // ORDER BY affinity DESC NULLS LAST — typed (0.9) sorts before untyped (0.5).
    assert_eq!(members[0].0, visible_typed);
    assert_eq!(members[1].0, visible_untyped);
}
