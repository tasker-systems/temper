#![cfg(feature = "test-db")]
//! Connection provisioning authorization, over HTTP (external systems as subscribed emitters, S1).
//!
//! `test-db` green is a false signal for access semantics: these assertions have to run through
//! the real router, the real auth middleware, and real JWTs. The bite test here is
//! `a_teamless_connection_is_admin_only_over_http` — a teamless connection is the one with no
//! owning team to key a check on, and "no team to check" must never mean "nothing to deny".
//!
//! A connection reuses `machine_authz::authorize` verbatim, so what is being proven at this layer
//! is that the *surface* actually reaches that gate — not that the predicate returns false.

mod common;

use serde_json::json;
use sqlx::PgPool;
use uuid::Uuid;

/// Provision a profile by hitting an authed endpoint (auto-provision on first request).
async fn provision_profile(app: &common::E2eTestApp, token: &str) -> Uuid {
    let resp = app
        .reqwest_client
        .get(app.url("/api/profile"))
        .bearer_auth(token)
        .send()
        .await
        .expect("GET /api/profile");
    assert_eq!(resp.status(), 200, "profile auto-provision");
    let body: serde_json::Value = resp.json().await.expect("profile json");
    body["id"]
        .as_str()
        .expect("profile id")
        .parse()
        .expect("uuid")
}

/// Create a team via the API; the caller becomes its sole owner.
async fn create_team(app: &common::E2eTestApp, token: &str, slug: &str) -> Uuid {
    let resp = app
        .reqwest_client
        .post(app.url("/api/teams"))
        .bearer_auth(token)
        .json(&json!({ "slug": slug, "name": slug }))
        .send()
        .await
        .expect("POST /api/teams");
    assert_eq!(resp.status(), 201, "team create");
    let body: serde_json::Value = resp.json().await.expect("team json");
    body["id"].as_str().expect("team id").parse().expect("uuid")
}

/// `POST /api/connections`. Returns (status, body).
async fn provision(
    app: &common::E2eTestApp,
    token: &str,
    body: serde_json::Value,
) -> (reqwest::StatusCode, serde_json::Value) {
    let resp = app
        .reqwest_client
        .post(app.url("/api/connections"))
        .bearer_auth(token)
        .json(&body)
        .send()
        .await
        .expect("POST /api/connections");
    let status = resp.status();
    let json = resp.json().await.unwrap_or(serde_json::Value::Null);
    (status, json)
}

async fn connection_count(pool: &PgPool) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM kb_connections")
        .fetch_one(pool)
        .await
        .expect("count connections")
}

/// The point of the model: a team runs its own integrations, with no operator in the loop.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn a_team_owner_can_provision_for_their_own_team(pool: PgPool) {
    let app = common::setup(pool.clone()).await;
    let token = common::generate_test_jwt("conn-owner", "conn-owner@example.com");
    provision_profile(&app, &token).await;
    let team = create_team(&app, &token, "acme").await;

    let (status, body) = provision(
        &app,
        &token,
        json!({
            "provider": "github",
            "name": "Acme GitHub",
            "owner_team_id": team,
            "reach_granularity": "repo-set",
            "reach_covers": "acme/temper",
        }),
    )
    .await;

    assert_eq!(status, 200, "team owner may provision: {body:?}");
    assert_eq!(body["provider"], "github");
    assert_eq!(body["slug"], "acme-github");
    // Born needs_credential: the credential is attached separately, so a connection never
    // silently pretends to be more than it is.
    assert!(
        body["credential"].is_null(),
        "born needs_credential, got {body:?}"
    );
    assert_eq!(
        body["webhook_events"].as_array().map(Vec::len),
        Some(0),
        "not ledger-capable at birth"
    );
}

/// Teamless fails closed. There is no owning team to key a check on, and that must deny rather
/// than fall open.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn a_teamless_connection_is_admin_only_over_http(pool: PgPool) {
    let app = common::setup(pool.clone()).await;
    let token = common::generate_test_jwt("conn-nobody", "conn-nobody@example.com");
    provision_profile(&app, &token).await;

    let (status, _body) = provision(
        &app,
        &token,
        json!({ "provider": "github", "name": "Rogue GitHub", "owner_team_id": null }),
    )
    .await;

    assert_eq!(status, 403, "a teamless connection is admin-only");
    assert_eq!(connection_count(&pool).await, 0, "and nothing was written");
}

/// Reach into a team the caller does not own is denied, and writes nothing — the auth-before-writes
/// invariant, asserted at the surface: no orphaned profile, entity, or context is left behind.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn provisioning_for_an_unowned_team_is_denied_and_writes_nothing(pool: PgPool) {
    let app = common::setup(pool.clone()).await;

    let owner = common::generate_test_jwt("conn-owner", "conn-owner@example.com");
    provision_profile(&app, &owner).await;
    let team = create_team(&app, &owner, "acme").await;

    let outsider = common::generate_test_jwt("conn-outsider", "conn-outsider@example.com");
    provision_profile(&app, &outsider).await;

    let profiles_before: i64 = sqlx::query_scalar("SELECT count(*) FROM kb_profiles")
        .fetch_one(&pool)
        .await
        .expect("count profiles");

    let (status, _body) = provision(
        &app,
        &outsider,
        json!({ "provider": "linear", "name": "Acme Linear", "owner_team_id": team }),
    )
    .await;

    assert_eq!(status, 403, "an outsider may not provision for acme");
    assert_eq!(connection_count(&pool).await, 0);

    let profiles_after: i64 = sqlx::query_scalar("SELECT count(*) FROM kb_profiles")
        .fetch_one(&pool)
        .await
        .expect("count profiles");
    assert_eq!(
        profiles_before, profiles_after,
        "a denied provisioning left an orphaned profile behind"
    );
}
