//! HTTP e2e for GET /api/graph/regions/{region_id}/slice (R3) — the acceptance gate.
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

async fn any_event(pool: &sqlx::PgPool) -> Uuid {
    sqlx::query_scalar("SELECT id FROM kb_events LIMIT 1")
        .fetch_one(pool)
        .await
        .unwrap()
}

async fn telos_default_lens(pool: &sqlx::PgPool) -> Uuid {
    sqlx::query_scalar(
        "SELECT id FROM kb_cogmap_lenses WHERE name = 'telos-default' AND cogmap_id IS NULL LIMIT 1",
    )
    .fetch_one(pool)
    .await
    .unwrap()
}

async fn create_cogmap(pool: &sqlx::PgPool, name: &str, telos_resource: Uuid) -> Uuid {
    sqlx::query_scalar(
        "INSERT INTO kb_cogmaps (name, telos_resource_id) VALUES ($1, $2) RETURNING id",
    )
    .bind(name)
    .bind(telos_resource)
    .fetch_one(pool)
    .await
    .unwrap()
}

async fn join_cogmap_team(pool: &sqlx::PgPool, cogmap: Uuid, team: Uuid) {
    sqlx::query("INSERT INTO kb_team_cogmaps (cogmap_id, team_id) VALUES ($1, $2)")
        .bind(cogmap)
        .bind(team)
        .execute(pool)
        .await
        .unwrap();
}

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
    .unwrap()
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
    .unwrap();
}

async fn slice(
    app: &common::E2eTestApp,
    token: &str,
    region: Uuid,
) -> (StatusCode, serde_json::Value) {
    let resp = app
        .reqwest_client
        .get(app.url(&format!("/api/graph/regions/{region}/slice")))
        .header("Authorization", format!("Bearer {token}"))
        .send()
        .await
        .expect("graph/regions/{region}/slice request failed");
    let status = resp.status();
    let body = resp
        .json::<serde_json::Value>()
        .await
        .unwrap_or(serde_json::Value::Null);
    (status, body)
}

/// A member of the region's cogmap-team gets a 200 territory slice with the
/// resource it owns (and that is a region member) present; a non-member is
/// denied as absence (404), as is a nonexistent region id.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn slice_returns_members_for_readers_and_denies_outsiders(pool: sqlx::PgPool) {
    let app = common::setup(pool.clone()).await;
    let member = provision_profile(&app, &app.token).await;
    let outsider_token =
        common::generate_test_jwt("gts-e2e-outsider", "gts-e2e-outsider@test.example.com");
    let _outsider = provision_profile(&app, &outsider_token).await;

    let team = create_team(&pool, "gts-e2e-team").await;
    add_member(&pool, team, member).await;

    let event = any_event(&pool).await;
    let lens = telos_default_lens(&pool).await;
    let telos = create_resource(&pool, "gts-e2e telos", "temper://gts-e2e/telos").await;
    let cogmap = create_cogmap(&pool, "gts-e2e-cogmap", telos).await;
    join_cogmap_team(&pool, cogmap, team).await;

    let region = insert_region(&pool, cogmap, lens, "GTS E2E Region", 1, 0.5, event).await;

    let member_res = create_resource(&pool, "gts-e2e member resource", "temper://gts-e2e/mr").await;
    home_resource(&pool, member_res, "kb_cogmaps", cogmap, member).await;
    add_region_member(&pool, region, member_res, Some(0.8)).await;

    // Happy path: member of the cogmap's team gets a 200 slice with the member resource.
    let (status, body) = slice(&app, &app.token, region).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "member of the cogmap's team gets a 200 territory slice: {body:?}"
    );
    assert_eq!(body["region_id"], region.to_string());
    let members = body["members"].as_array().expect("members array");
    assert!(
        members.iter().any(|m| m["id"] == member_res.to_string()),
        "the visible region member is present: {body:?}"
    );
    // A3: the slice no longer carries a components array; members are the
    // only interior grain. Exactly the one region member inserted above.
    assert_eq!(
        members.len(),
        1,
        "exactly the one visible region member surfaces: {body:?}"
    );

    // Outsider — 404 (deny-as-absence; not a member of the region's cogmap team).
    let (status, _) = slice(&app, &outsider_token, region).await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "a non-member of the cogmap's team is denied as absence"
    );

    // Nonexistent region — also 404 (deny-as-absence).
    let (status, _) = slice(&app, &app.token, Uuid::now_v7()).await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "a nonexistent region id is denied as absence"
    );
}
