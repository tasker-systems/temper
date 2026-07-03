//! HTTP e2e for GET /api/teams/{id}/graph-scope (R1) — the acceptance gate.
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

async fn graph_scope(
    app: &common::E2eTestApp,
    token: &str,
    team: Uuid,
) -> (StatusCode, serde_json::Value) {
    let resp = app
        .reqwest_client
        .get(app.url(&format!("/api/teams/{team}/graph-scope")))
        .header("Authorization", format!("Bearer {token}"))
        .send()
        .await
        .expect("graph-scope request failed");
    let status = resp.status();
    let body = resp
        .json::<serde_json::Value>()
        .await
        .unwrap_or(serde_json::Value::Null);
    (status, body)
}

/// A member of squad-a, viewing engineering (an ancestor), sees squad-a as an
/// enterable zone; squad-b (a sibling they do not belong to) is NOT shown; a
/// non-member of the whole tree gets 404 (deny-as-absence).
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn scope_shows_reachable_zones_and_denies_outsiders(pool: sqlx::PgPool) {
    let app = common::setup(pool.clone()).await;
    let member = provision_profile(&app, &app.token).await;
    let outsider_token = common::generate_test_jwt("tgs-outsider", "tgs-outsider@test.example.com");
    let _outsider = provision_profile(&app, &outsider_token).await;

    let eng = create_team(&pool, "e2e-eng").await;
    let squad_a = create_team(&pool, "e2e-squad-a").await;
    let squad_b = create_team(&pool, "e2e-squad-b").await;
    link_parent(&pool, eng, squad_a).await;
    link_parent(&pool, eng, squad_b).await;
    add_member(&pool, squad_a, member).await;

    // The member can view engineering (upward access from squad-a).
    let (status, body) = graph_scope(&app, &app.token, eng).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "member of a descendant may view the ancestor scope"
    );
    assert_eq!(body["team"]["slug"], "e2e-eng");
    let zone_slugs: Vec<&str> = body["zones"]
        .as_array()
        .unwrap()
        .iter()
        .map(|z| z["slug"].as_str().unwrap())
        .collect();
    assert!(
        zone_slugs.contains(&"e2e-squad-a"),
        "squad-a is an enterable zone"
    );
    assert!(
        !zone_slugs.contains(&"e2e-squad-b"),
        "squad-b (not a member) is not shown"
    );

    // An outsider (member of nothing under eng) is denied — deny-as-absence.
    let (status, _) = graph_scope(&app, &outsider_token, eng).await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "outsider cannot view the scope"
    );
}
