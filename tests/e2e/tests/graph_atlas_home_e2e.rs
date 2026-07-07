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
