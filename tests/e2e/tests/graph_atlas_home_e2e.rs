//! HTTP e2e for GET /api/graph/home (Atlas Home build/research read, Beat B).
//! Access-tier gate: `build` = the caller's visible contexts (personal + member
//! team), owner-scoped; `research` = visible cogmaps with a derived held-by scope
//! and member-team edges. Deny direction: a context/cogmap reachable only via a
//! non-member team must not appear.
#![cfg(feature = "test-db")]

mod common;

use uuid::Uuid;

// Harness pattern verified against graph_atlas_slice_e2e.rs:
//   #[sqlx::test(migrator = "temper_api::MIGRATOR")] fn(pool: sqlx::PgPool)
//   let app = common::setup(pool.clone()).await;  // app.token = a member JWT
//   common::generate_test_jwt(sub, email) for other identities.

async fn provision_profile(app: &common::E2eTestApp, token: &str) -> Uuid {
    let resp = app
        .reqwest_client
        .get(app.url("/api/profile"))
        .header("Authorization", format!("Bearer {token}"))
        .send()
        .await
        .expect("profile request failed");
    resp.json::<serde_json::Value>().await.unwrap()["id"]
        .as_str()
        .unwrap()
        .parse()
        .unwrap()
}

async fn create_team(pool: &sqlx::PgPool, slug: &str) -> Uuid {
    sqlx::query_scalar("INSERT INTO kb_teams (slug, name) VALUES ($1, $1) RETURNING id")
        .bind(slug)
        .fetch_one(pool)
        .await
        .unwrap()
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

async fn create_cogmap(pool: &sqlx::PgPool, name: &str) -> Uuid {
    // kb_cogmaps requires a telos_resource_id; create a throwaway resource for it.
    let telos: Uuid = sqlx::query_scalar(
        "INSERT INTO kb_resources (title, origin_uri) VALUES ($1, '') RETURNING id",
    )
    .bind(format!("{name}-telos"))
    .fetch_one(pool)
    .await
    .unwrap();
    sqlx::query_scalar(
        "INSERT INTO kb_cogmaps (name, telos_resource_id) VALUES ($1, $2) RETURNING id",
    )
    .bind(name)
    .bind(telos)
    .fetch_one(pool)
    .await
    .unwrap()
}

async fn join_cogmap(pool: &sqlx::PgPool, cogmap: Uuid, team: Uuid) {
    sqlx::query("INSERT INTO kb_team_cogmaps (cogmap_id, team_id) VALUES ($1, $2)")
        .bind(cogmap)
        .bind(team)
        .execute(pool)
        .await
        .unwrap();
}

async fn create_personal_context(pool: &sqlx::PgPool, profile: Uuid, slug: &str) -> Uuid {
    sqlx::query_scalar(
        "INSERT INTO kb_contexts (owner_table, owner_id, slug, name) \
         VALUES ('kb_profiles', $1, $2, $2) RETURNING id",
    )
    .bind(profile)
    .bind(slug)
    .fetch_one(pool)
    .await
    .unwrap()
}

async fn create_team_context(pool: &sqlx::PgPool, team: Uuid, slug: &str) -> Uuid {
    sqlx::query_scalar(
        "INSERT INTO kb_contexts (owner_table, owner_id, slug, name) \
         VALUES ('kb_teams', $1, $2, $2) RETURNING id",
    )
    .bind(team)
    .bind(slug)
    .fetch_one(pool)
    .await
    .unwrap()
}

async fn home_resource(pool: &sqlx::PgPool, context: Uuid, owner: Uuid, title: &str) {
    home_resource_returning(pool, context, owner, title).await;
}

/// Same as `home_resource` but returns the created resource id (for soft-delete).
async fn home_resource_returning(
    pool: &sqlx::PgPool,
    context: Uuid,
    owner: Uuid,
    title: &str,
) -> Uuid {
    let rid: Uuid = sqlx::query_scalar(
        "INSERT INTO kb_resources (title, origin_uri) VALUES ($1, '') RETURNING id",
    )
    .bind(title)
    .fetch_one(pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO kb_resource_homes \
         (resource_id, anchor_table, anchor_id, originator_profile_id, owner_profile_id) \
         VALUES ($1, 'kb_contexts', $2, $3, $3)",
    )
    .bind(rid)
    .bind(context)
    .bind(owner)
    .execute(pool)
    .await
    .unwrap();
    rid
}

async fn soft_delete_resource(pool: &sqlx::PgPool, resource: Uuid) {
    sqlx::query("UPDATE kb_resources SET is_active = false WHERE id = $1")
        .bind(resource)
        .execute(pool)
        .await
        .unwrap();
}

/// Stamp a resource's `updated` timestamp to a specific instant (for recency
/// ordering tests). `kb_resources.updated` is the column `graph_home_contexts`'s
/// `last_active_at` subquery reads (aliased `last_active_at` on the way out).
async fn set_resource_updated_at(
    pool: &sqlx::PgPool,
    resource: Uuid,
    updated_at: chrono::DateTime<chrono::Utc>,
) {
    sqlx::query("UPDATE kb_resources SET updated = $1 WHERE id = $2")
        .bind(updated_at)
        .bind(resource)
        .execute(pool)
        .await
        .unwrap();
}

/// Link `child` under `parent` in the teams DAG.
async fn add_parent(pool: &sqlx::PgPool, child: Uuid, parent: Uuid) {
    sqlx::query("INSERT INTO kb_teams_parents (child_id, parent_id) VALUES ($1, $2)")
        .bind(child)
        .bind(parent)
        .execute(pool)
        .await
        .unwrap();
}

/// Grant a profile explicit read on a context.
async fn grant_context_read(
    pool: &sqlx::PgPool,
    context: Uuid,
    principal_profile: Uuid,
    granted_by: Uuid,
) {
    sqlx::query(
        "INSERT INTO kb_access_grants \
         (subject_table, subject_id, principal_table, principal_id, can_read, granted_by_profile_id) \
         VALUES ('kb_contexts', $1, 'kb_profiles', $2, true, $3)",
    )
    .bind(context)
    .bind(principal_profile)
    .bind(granted_by)
    .execute(pool)
    .await
    .unwrap();
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn home_returns_member_teams_and_shared_cogmap_edges(pool: sqlx::PgPool) {
    let app = common::setup(pool.clone()).await;
    let profile = provision_profile(&app, &app.token).await;

    let team_a = create_team(&pool, "home-a").await;
    let team_b = create_team(&pool, "home-b").await;
    add_member(&pool, team_a, profile).await;
    add_member(&pool, team_b, profile).await;

    let shared = create_cogmap(&pool, "shared-map").await;
    join_cogmap(&pool, shared, team_a).await;
    join_cogmap(&pool, shared, team_b).await;

    // A personal context should surface on the build lens, owner-scoped `@me`.
    create_personal_context(&pool, profile, "my-notes").await;

    let body: temper_core::types::graph_home::AtlasHome = app
        .reqwest_client
        .get(app.url("/api/graph/home"))
        .header("Authorization", format!("Bearer {}", app.token))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    assert!(
        body.build
            .iter()
            .any(|c| c.name == "my-notes" && c.owner_ref == "@me"),
        "personal context appears on the build lens as @me"
    );
    let sc = body
        .research
        .iter()
        .find(|c| c.name == "shared-map")
        .expect("shared cogmap present");
    assert_eq!(
        sc.team_ids.len(),
        2,
        "shared cogmap lists both member teams"
    );
    assert!(
        sc.owner_ref.starts_with('+'),
        "a team-held cogmap derives a +team held-by scope, got {}",
        sc.owner_ref
    );
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn home_excludes_cogmaps_visible_only_via_non_member_team(pool: sqlx::PgPool) {
    let app = common::setup(pool.clone()).await;
    let profile = provision_profile(&app, &app.token).await;

    let team_x = create_team(&pool, "home-x").await;
    let team_y = create_team(&pool, "home-y").await;
    add_member(&pool, team_x, profile).await;
    // Caller is deliberately NOT added to team_y.

    let only_y = create_cogmap(&pool, "only-y").await;
    join_cogmap(&pool, only_y, team_y).await;

    let mixed = create_cogmap(&pool, "mixed").await;
    join_cogmap(&pool, mixed, team_x).await;
    join_cogmap(&pool, mixed, team_y).await;

    let body: temper_core::types::graph_home::AtlasHome = app
        .reqwest_client
        .get(app.url("/api/graph/home"))
        .header("Authorization", format!("Bearer {}", app.token))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    assert!(
        !body.research.iter().any(|c| c.name == "only-y"),
        "cogmap joined only to a non-member team must not appear in the home response"
    );

    let mc = body
        .research
        .iter()
        .find(|c| c.name == "mixed")
        .expect("cogmap joined to a member team is present");
    assert!(
        mc.team_ids.contains(&team_x),
        "mixed cogmap's team_ids includes the caller's member team"
    );
    assert!(
        !mc.team_ids.contains(&team_y),
        "mixed cogmap's team_ids must not leak the non-member team's id"
    );
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn build_lens_scopes_contexts_to_membership(pool: sqlx::PgPool) {
    let app = common::setup(pool.clone()).await;
    let profile = provision_profile(&app, &app.token).await;

    let team_in = create_team(&pool, "ctx-in").await;
    let team_out = create_team(&pool, "ctx-out").await;
    add_member(&pool, team_in, profile).await;
    // Caller is deliberately NOT a member of team_out.

    let personal = create_personal_context(&pool, profile, "personal-ctx").await;
    let team_ctx = create_team_context(&pool, team_in, "member-team-ctx").await;
    let hidden = create_team_context(&pool, team_out, "non-member-ctx").await;

    let body: temper_core::types::graph_home::AtlasHome = app
        .reqwest_client
        .get(app.url("/api/graph/home"))
        .header("Authorization", format!("Bearer {}", app.token))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    let build_ids: Vec<Uuid> = body.build.iter().map(|c| c.id).collect();
    assert!(build_ids.contains(&personal), "personal context is visible");
    assert!(
        build_ids.contains(&team_ctx),
        "member-team context is visible"
    );
    assert!(
        !build_ids.contains(&hidden),
        "a context owned by a non-member team must NOT appear on the build lens"
    );

    // Owner-scope decoration.
    let p = body.build.iter().find(|c| c.id == personal).unwrap();
    assert_eq!(p.owner_ref, "@me");
    let t = body.build.iter().find(|c| c.id == team_ctx).unwrap();
    assert_eq!(t.owner_ref, "+ctx-in");
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn build_lens_resource_count_reflects_visible_homed_resources(pool: sqlx::PgPool) {
    let app = common::setup(pool.clone()).await;
    let profile = provision_profile(&app, &app.token).await;

    let ctx = create_personal_context(&pool, profile, "sized-ctx").await;
    home_resource(&pool, ctx, profile, "r1").await;
    home_resource(&pool, ctx, profile, "r2").await;

    let body: temper_core::types::graph_home::AtlasHome = app
        .reqwest_client
        .get(app.url("/api/graph/home"))
        .header("Authorization", format!("Bearer {}", app.token))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    let c = body
        .build
        .iter()
        .find(|c| c.id == ctx)
        .expect("sized context present on the build lens");
    assert_eq!(
        c.resource_count, 2,
        "resource_count reflects the two resources homed in (and visible via) the context"
    );
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn build_lens_resource_count_excludes_soft_deleted(pool: sqlx::PgPool) {
    let app = common::setup(pool.clone()).await;
    let profile = provision_profile(&app, &app.token).await;

    let ctx = create_personal_context(&pool, profile, "trimmed-ctx").await;
    home_resource_returning(&pool, ctx, profile, "keep").await;
    let doomed = home_resource_returning(&pool, ctx, profile, "drop").await;
    soft_delete_resource(&pool, doomed).await;

    let body: temper_core::types::graph_home::AtlasHome = app
        .reqwest_client
        .get(app.url("/api/graph/home"))
        .header("Authorization", format!("Bearer {}", app.token))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    let c = body
        .build
        .iter()
        .find(|c| c.id == ctx)
        .expect("context present on the build lens");
    assert_eq!(
        c.resource_count, 1,
        "resource_count excludes the soft-deleted resource, counting only the active one"
    );
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn build_lens_grant_owned_by_other_profile_reads_shared(pool: sqlx::PgPool) {
    let app = common::setup(pool.clone()).await;
    let profile = provision_profile(&app, &app.token).await;

    // A second identity owns a personal context and grants read on it to the caller.
    let other_token = common::generate_test_jwt("e2e-other-user", "other@test.example.com");
    let other_profile = provision_profile(&app, &other_token).await;
    let granted = create_personal_context(&pool, other_profile, "granted-ctx").await;
    grant_context_read(&pool, granted, profile, other_profile).await;

    let body: temper_core::types::graph_home::AtlasHome = app
        .reqwest_client
        .get(app.url("/api/graph/home"))
        .header("Authorization", format!("Bearer {}", app.token))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    let c = body
        .build
        .iter()
        .find(|c| c.id == granted)
        .expect("a context read-granted to the caller appears on the build lens");
    assert_eq!(
        c.owner_ref, "shared",
        "a context owned by another profile, visible only via an explicit read-grant, \
         is owner-scoped 'shared' — not @me, not +team"
    );
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn research_lens_ancestor_held_cogmap_derives_ancestor_team(pool: sqlx::PgPool) {
    let app = common::setup(pool.clone()).await;
    let profile = provision_profile(&app, &app.token).await;

    // Caller is a member of the CHILD only; the cogmap is joined to the PARENT,
    // reachable only via the ancestor up-walk.
    let parent = create_team(&pool, "home-parent").await;
    let child = create_team(&pool, "home-child").await;
    add_parent(&pool, child, parent).await;
    add_member(&pool, child, profile).await;

    let map = create_cogmap(&pool, "ancestor-held").await;
    join_cogmap(&pool, map, parent).await;

    let body: temper_core::types::graph_home::AtlasHome = app
        .reqwest_client
        .get(app.url("/api/graph/home"))
        .header("Authorization", format!("Bearer {}", app.token))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    let c = body
        .research
        .iter()
        .find(|c| c.name == "ancestor-held")
        .expect("a cogmap held by an ancestor team is visible on the research lens");
    assert_eq!(
        c.owner_ref, "+home-parent",
        "held-by scope derives the ancestor team that made the map reachable, \
         not the universal 'temper' marker"
    );
    assert!(
        c.team_ids.contains(&parent),
        "team_ids includes the ancestor team the map is joined to"
    );
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn build_lens_last_active_at_is_visibility_scoped(pool: sqlx::PgPool) {
    let app = common::setup(pool.clone()).await;
    let profile = provision_profile(&app, &app.token).await;

    let ctx = create_personal_context(&pool, profile, "recency-ctx").await;

    let t1 = chrono::Utc::now() - chrono::Duration::days(2);
    let t2 = chrono::Utc::now() - chrono::Duration::days(1); // newer than t1
    let t3 = chrono::Utc::now(); // newer than both t1 and t2

    // R1: visible + active, stamped t1.
    let visible = home_resource_returning(&pool, ctx, profile, "visible-r1").await;
    set_resource_updated_at(&pool, visible, t1).await;

    // R2: homed in the SAME context but soft-deleted — excluded from the
    // resource_count join set, so its newer stamp (t2) must not leak into
    // last_active_at either (the two subqueries share the same counted set).
    let hidden = home_resource_returning(&pool, ctx, profile, "hidden-r2").await;
    set_resource_updated_at(&pool, hidden, t2).await;
    soft_delete_resource(&pool, hidden).await;

    // R3: homed in the SAME context, active, but owned/originated by a SECOND
    // profile with no grant back to the caller — invisible via
    // `resources_visible_to(profile)` even though `ctx` itself (owned by
    // `profile`) is visible. Stamped t3 (newest of all three) so a leak would
    // be unambiguous. Proves the `resources_visible_to` conjunct specifically,
    // distinct from R2's soft-delete/is_active conjunct.
    let other_token =
        common::generate_test_jwt("e2e-other-recency", "other-recency@test.example.com");
    let other_profile = provision_profile(&app, &other_token).await;
    let invisible = home_resource_returning(&pool, ctx, other_profile, "invisible-r3").await;
    set_resource_updated_at(&pool, invisible, t3).await;

    // A second context with no homed resources at all — last_active_at is None.
    let empty_ctx = create_personal_context(&pool, profile, "empty-ctx").await;

    let body: temper_core::types::graph_home::AtlasHome = app
        .reqwest_client
        .get(app.url("/api/graph/home"))
        .header("Authorization", format!("Bearer {}", app.token))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    let c = body
        .build
        .iter()
        .find(|c| c.id == ctx)
        .expect("recency context present on the build lens");
    let last_active_at = c
        .last_active_at
        .expect("context with a visible resource has a last_active_at");
    assert!(
        (last_active_at - t1).num_milliseconds().abs() < 1000,
        "last_active_at reflects the visible resource's updated stamp (t1), got {last_active_at:?}"
    );
    assert!(
        (last_active_at - t2).num_milliseconds().abs() > 1000,
        "the soft-deleted resource's newer stamp (t2) must NOT leak into last_active_at, got {last_active_at:?}"
    );
    assert!(
        (last_active_at - t3).num_milliseconds().abs() > 1000,
        "a resource invisible to the caller (owned by another profile, no grant) must NOT \
         advance last_active_at even though its stamp (t3) is the newest of all three, \
         got {last_active_at:?}"
    );

    let empty = body
        .build
        .iter()
        .find(|c| c.id == empty_ctx)
        .expect("empty context present on the build lens");
    assert_eq!(
        empty.last_active_at, None,
        "a context with no visible resources has last_active_at == None"
    );
}
