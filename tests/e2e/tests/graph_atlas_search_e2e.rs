//! HTTP e2e for GET /api/teams/{id}/graph/search (C3) — access-tier gate.
//! Proves the full stack (auth + handler + deny code) agrees, at the e2e access tier;
//! test-db predicate tests alone are a false signal for access changes.
#![cfg(feature = "test-db")]

mod common;

use reqwest::StatusCode;
use uuid::Uuid;

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

async fn create_resource(pool: &sqlx::PgPool, title: &str, origin: &str) -> Uuid {
    sqlx::query_scalar("INSERT INTO kb_resources (title, origin_uri) VALUES ($1, $2) RETURNING id")
        .bind(title)
        .bind(origin)
        .fetch_one(pool)
        .await
        .unwrap()
}

/// A context owned by a team — passes `resources_in_team_scope`'s team-owned-context branch.
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

async fn home_resource(
    pool: &sqlx::PgPool,
    resource: Uuid,
    anchor_table: &str,
    anchor_id: Uuid,
    profile: Uuid,
) {
    sqlx::query(
        "INSERT INTO kb_resource_homes \
             (resource_id, anchor_table, anchor_id, originator_profile_id, owner_profile_id) \
         VALUES ($1, $2, $3, $4, $4)",
    )
    .bind(resource)
    .bind(anchor_table)
    .bind(anchor_id)
    .bind(profile)
    .execute(pool)
    .await
    .unwrap();
}

/// `unified_search`'s FTS candidates come from the stored `kb_resource_search_index`,
/// which production populates via the ingest event pipeline (`_rebuild_resource_search_vector`,
/// see `migrations/20260626000001_fts_search_index.sql`). Raw-inserted test resources
/// (via `create_resource` above) never run that pipeline, so the search index row must
/// be seeded directly — mirrors the production tsvector recipe (title only; no body chunks
/// in this harness).
async fn index_resource_for_search(pool: &sqlx::PgPool, resource: Uuid, title: &str) {
    sqlx::query(
        "INSERT INTO kb_resource_search_index (resource_id, search_vector, search_config) \
         VALUES ($1, setweight(to_tsvector('english', $2), 'A'), 'english')",
    )
    .bind(resource)
    .bind(title)
    .execute(pool)
    .await
    .unwrap();
}

async fn search(
    app: &common::E2eTestApp,
    token: &str,
    team: Uuid,
    q: &str,
) -> (StatusCode, serde_json::Value) {
    let resp = app
        .reqwest_client
        .get(app.url(&format!("/api/teams/{team}/graph/search?q={q}")))
        .bearer_auth(token)
        .send()
        .await
        .unwrap();
    let status = resp.status();
    let body = resp
        .json::<serde_json::Value>()
        .await
        .unwrap_or(serde_json::Value::Null);
    (status, body)
}

/// (a) a member finds an in-scope resource by title token; (b) a resource that
/// exists but is NOT homed in the team's scope does not surface; (c) a non-member
/// of the team is denied as absence (404).
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn atlas_search_scopes_to_team_and_denies_outsiders(pool: sqlx::PgPool) {
    let app = common::setup(pool.clone()).await;
    let member = provision_profile(&app, &app.token).await;
    let outsider_token =
        common::generate_test_jwt("as-e2e-outsider", "as-e2e-outsider@test.example.com");
    let _outsider = provision_profile(&app, &outsider_token).await;

    let team = create_team(&pool, "as-e2e-team").await;
    add_member(&pool, team, member).await;
    let ctx = create_team_context(&pool, team, "as-e2e-ctx").await;

    let in_scope = create_resource(&pool, "Findable Widget", "temper://as-e2e/findable").await;
    home_resource(&pool, in_scope, "kb_contexts", ctx, member).await;
    index_resource_for_search(&pool, in_scope, "Findable Widget").await;

    // A resource NOT homed in the team scope — must not surface.
    let out_of_scope =
        create_resource(&pool, "Findable Widget Hidden", "temper://as-e2e/hidden").await;
    index_resource_for_search(&pool, out_of_scope, "Findable Widget Hidden").await;

    // (a) member finds the in-scope resource by title token
    let (status, body) = search(&app, &app.token, team, "Findable").await;
    assert_eq!(status, StatusCode::OK, "member gets 200: {body:?}");
    let hits = body.as_array().expect("array of hits");
    let ids: Vec<&str> = hits.iter().filter_map(|h| h["node_id"].as_str()).collect();
    assert!(
        ids.contains(&in_scope.to_string().as_str()),
        "in-scope resource surfaces: {body:?}"
    );

    // (b) out-of-scope resource does not appear
    assert!(
        !ids.contains(&out_of_scope.to_string().as_str()),
        "out-of-scope resource is not returned: {body:?}"
    );

    // (c) outsider → 404 deny-as-absence
    let (status, _) = search(&app, &outsider_token, team, "Findable").await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "non-member denied as absence"
    );
}
