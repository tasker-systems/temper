//! HTTP e2e for GET /api/teams/{id}/graph/territories (R2) — the acceptance gate.
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

async fn territories(
    app: &common::E2eTestApp,
    token: &str,
    team: Uuid,
    lens_id: Option<Uuid>,
) -> (StatusCode, serde_json::Value) {
    let mut url = format!("/api/teams/{team}/graph/territories");
    if let Some(lens) = lens_id {
        url.push_str(&format!("?lens_id={lens}"));
    }
    let resp = app
        .reqwest_client
        .get(app.url(&url))
        .header("Authorization", format!("Bearer {token}"))
        .send()
        .await
        .expect("graph/territories request failed");
    let status = resp.status();
    let body = resp
        .json::<serde_json::Value>()
        .await
        .unwrap_or(serde_json::Value::Null);
    (status, body)
}

/// A member gets a 200 territory overview (default-lens path, no `lens_id`) with at
/// least one context territory for a resource homed in the team's own context; a
/// non-member of the team is denied as absence (404).
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn territories_returns_overview_on_default_lens_and_denies_outsiders(pool: sqlx::PgPool) {
    let app = common::setup(pool.clone()).await;
    let member = provision_profile(&app, &app.token).await;
    let outsider_token =
        common::generate_test_jwt("gto-e2e-outsider", "gto-e2e-outsider@test.example.com");
    let _outsider = provision_profile(&app, &outsider_token).await;

    let team = create_team(&pool, "gto-e2e-team").await;
    add_member(&pool, team, member).await;

    let ctx = create_team_context(&pool, team, "gto-e2e-ctx").await;
    let ctx_res = create_resource(&pool, "context resource", "temper://gto-e2e/ctx-res").await;
    home_resource(&pool, ctx_res, "kb_contexts", ctx, member).await;

    // Happy path: member, no lens_id — exercises the default-lens resolution path.
    let (status, body) = territories(&app, &app.token, team, None).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "member gets a 200 territory overview: {body:?}"
    );
    let territory_list = body["territories"].as_array().expect("territories array");
    assert!(
        !territory_list.is_empty(),
        "at least one territory present: {body:?}"
    );
    let ctx_territory = territory_list
        .iter()
        .find(|t| t["id"] == ctx.to_string())
        .expect("the team-owned context surfaces as a territory");
    assert_eq!(ctx_territory["kind"], "context");
    assert_eq!(ctx_territory["member_count"], 1);
    assert!(
        body["orphan_nodes"].as_array().is_some(),
        "orphan_nodes key present (possibly empty)"
    );
    assert!(
        body["bridges"].as_array().is_some(),
        "bridges key present (possibly empty)"
    );

    // Outsider — 404 (deny-as-absence; not a member of the team).
    let (status, _) = territories(&app, &outsider_token, team, None).await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "a non-member of the team is denied as absence"
    );
}

/// D3: a resource homed in a joined, region-LESS cogmap surfaces as an orphan
/// node carrying the home cogmap's human name as `anchor_label`, so the Atlas
/// sparse-territory label can show a real name instead of a generic fallback.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn territories_orphan_node_carries_home_cogmap_anchor_label(pool: sqlx::PgPool) {
    let app = common::setup(pool.clone()).await;
    let member = provision_profile(&app, &app.token).await;

    let team = create_team(&pool, "gto-e2e-label-team").await;
    add_member(&pool, team, member).await;

    let telos = create_resource(&pool, "telos", "temper://gto-e2e-label/telos").await;
    let cogmap = create_cogmap(&pool, "gto-e2e-label-cogmap", telos).await;
    join_cogmap_team(&pool, cogmap, team).await;

    let orphan = create_resource(&pool, "orphan node", "temper://gto-e2e-label/orphan").await;
    home_resource(&pool, orphan, "kb_cogmaps", cogmap, member).await;

    let (status, body) = territories(&app, &app.token, team, None).await;
    assert_eq!(status, StatusCode::OK, "member gets a 200: {body:?}");

    let orphan_nodes = body["orphan_nodes"].as_array().expect("orphan_nodes array");
    let orphan_row = orphan_nodes
        .iter()
        .find(|n| n["id"] == orphan.to_string())
        .expect("the region-less cogmap's homed resource surfaces as an orphan");
    assert_eq!(
        orphan_row["anchor_id"],
        cogmap.to_string(),
        "anchor_id is the orphan's home cogmap"
    );
    assert_eq!(
        orphan_row["anchor_label"], "gto-e2e-label-cogmap",
        "anchor_label carries the home cogmap's human name: {orphan_row:?}"
    );
}
