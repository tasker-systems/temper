//! HTTP e2e for GET /api/graph/cogmaps/{id}/panorama.
//! A reader sees the cogmap interior (TerritoryOverview); a non-reader gets 404.
#![cfg(feature = "test-db")]

mod common;

use uuid::Uuid;

// Helpers mirror graph_atlas_home_e2e.rs (integration test binaries don't share
// code except via `common`, so these are copied rather than imported).

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

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn panorama_denies_non_reader_as_absence(pool: sqlx::PgPool) {
    let app = common::setup(pool.clone()).await;
    let _profile = provision_profile(&app, &app.token).await;

    // A cogmap joined to NO team the caller belongs to → not readable.
    let orphan_map = create_cogmap(&pool, "unreachable-map").await;

    let status = app
        .reqwest_client
        .get(app.url(&format!("/api/graph/cogmaps/{orphan_map}/panorama")))
        .header("Authorization", format!("Bearer {}", app.token))
        .send()
        .await
        .unwrap()
        .status();
    assert_eq!(
        status,
        reqwest::StatusCode::NOT_FOUND,
        "deny-as-absence, not 403"
    );
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn panorama_returns_overview_for_reader(pool: sqlx::PgPool) {
    let app = common::setup(pool.clone()).await;
    let profile = provision_profile(&app, &app.token).await;
    let team = create_team(&pool, "pano-team").await;
    add_member(&pool, team, profile).await;
    let map = create_cogmap(&pool, "readable-map").await;
    join_cogmap(&pool, map, team).await;

    let resp = app
        .reqwest_client
        .get(app.url(&format!("/api/graph/cogmaps/{map}/panorama")))
        .header("Authorization", format!("Bearer {}", app.token))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::OK);
    let _body: temper_core::types::graph_territory::TerritoryOverview = resp.json().await.unwrap();
    // shape decodes = renderer-compatible
}
