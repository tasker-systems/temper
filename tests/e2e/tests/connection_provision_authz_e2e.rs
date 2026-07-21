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
    let id: Uuid = body["id"]
        .as_str()
        .expect("profile id")
        .parse()
        .expect("uuid");
    // D11: a fresh principal is born Denied. Approve so this actor clears the front door and the
    // ENDPOINT authz (team ownership, admin-only, etc.) is what these tests actually exercise — not
    // the system-access gate they all sat behind under open mode.
    common::approve(&app.pool, id).await;
    id
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

/// `POST /api/connections/{id}/{sub}`. Returns (status, body).
async fn post_sub(
    app: &common::E2eTestApp,
    token: &str,
    id: &str,
    sub: &str,
    body: serde_json::Value,
) -> (reqwest::StatusCode, serde_json::Value) {
    let path = format!("/api/connections/{id}/{sub}");
    let resp = app
        .reqwest_client
        .post(app.url(&path))
        .bearer_auth(token)
        .json(&body)
        .send()
        .await
        .expect("POST connection sub-resource");
    let status = resp.status();
    let json = resp.json().await.unwrap_or(serde_json::Value::Null);
    (status, json)
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

/// The credential and the two capability tiers, over the real router.
///
/// Each is its own endpoint precisely so a caller cannot grant reach while believing they were
/// only registering a webhook — so the surface must prove they move independently, and that
/// `needs_credential` flips off from the COLUMN, not from a status anyone set.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn the_credential_and_the_capability_tiers_move_independently_over_http(pool: PgPool) {
    let app = common::setup(pool.clone()).await;
    let token = common::generate_test_jwt("conn-owner", "conn-owner@example.com");
    provision_profile(&app, &token).await;
    let team = create_team(&app, &token, "acme").await;

    let (status, born) = provision(
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
    assert_eq!(status, 200, "team owner may provision: {born:?}");
    assert!(born["credential"].is_null(), "born needs_credential");
    let id = born["id"].as_str().expect("connection id").to_string();

    // The credential. No secret crosses the wire — a broker name and a connector id the BROKER
    // holds the secret for.
    let (status, credentialed) = post_sub(
        &app,
        &token,
        &id,
        "credential",
        json!({ "broker": "vercel-connect", "connector": "conn_abc123" }),
    )
    .await;
    assert_eq!(status, 200, "attach credential: {credentialed:?}");
    // The response is now {connection, verification}: the connection plus what
    // minting once at attach time observed.
    let conn = &credentialed["connection"];
    assert_eq!(conn["credential"]["broker"], "vercel-connect");
    assert_eq!(conn["credential"]["connector"], "conn_abc123");
    assert!(
        conn["credential"].get("installation").is_none(),
        "an absent installation must be omitted, not serialized as null"
    );
    // The test deployment configures no broker, so the credential is recorded but
    // NOT verified — and it says so, out loud, rather than silently.
    assert_eq!(
        credentialed["verification"]["verified"], false,
        "with no broker configured the mint cannot happen: {credentialed:?}"
    );
    assert!(
        credentialed["verification"]["note"].is_string(),
        "an unverified attach must carry a note saying why"
    );
    // A credential confers NEITHER tier.
    assert_eq!(
        conn["webhook_events"].as_array().map(Vec::len),
        Some(0),
        "a credential does not make a connection ledger-capable"
    );
    assert_eq!(
        conn["tool_manifest"],
        json!({}),
        "a credential does not make a connection reach-capable"
    );

    // Ledger-capable, and ONLY ledger-capable.
    let (status, ledger) = post_sub(
        &app,
        &token,
        &id,
        "webhook-events",
        json!({ "events": ["pull_request", "push"] }),
    )
    .await;
    assert_eq!(status, 200, "set webhook events: {ledger:?}");
    assert_eq!(ledger["webhook_events"], json!(["pull_request", "push"]));
    assert_eq!(
        ledger["tool_manifest"],
        json!({}),
        "ledger-capable must not imply reach-capable — a ledger-only connection is legal, and \
         inert for judgment"
    );

    // Reach-capable.
    let (status, reach) = post_sub(
        &app,
        &token,
        &id,
        "tool-manifest",
        json!({ "tools": ["get_pull_request"] }),
    )
    .await;
    assert_eq!(status, 200, "set tool manifest: {reach:?}");
    assert_eq!(reach["tool_manifest"], json!(["get_pull_request"]));
    assert_eq!(
        reach["webhook_events"],
        json!(["pull_request", "push"]),
        "declaring tools must not clear the registered webhooks"
    );
}

/// The mutators must reach `machine_authz::authorize` too — not just `provision`. A surface that
/// gates creation and then leaves mutation open is the classic way this goes wrong.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn an_outsider_cannot_attach_a_credential_over_http(pool: PgPool) {
    let app = common::setup(pool.clone()).await;
    let owner = common::generate_test_jwt("conn-owner", "conn-owner@example.com");
    let outsider = common::generate_test_jwt("conn-outsider", "conn-outsider@example.com");
    provision_profile(&app, &owner).await;
    provision_profile(&app, &outsider).await;
    let team = create_team(&app, &owner, "acme").await;

    let (status, born) = provision(
        &app,
        &owner,
        json!({ "provider": "github", "name": "Acme GitHub", "owner_team_id": team }),
    )
    .await;
    assert_eq!(status, 200, "owner provisions: {born:?}");
    let id = born["id"].as_str().expect("connection id").to_string();

    let (status, _) = post_sub(
        &app,
        &outsider,
        &id,
        "credential",
        json!({ "broker": "vercel-connect", "connector": "conn_abc123" }),
    )
    .await;
    assert_eq!(status, 403, "an outsider may not attach a credential");

    let stored: Option<serde_json::Value> =
        sqlx::query_scalar("SELECT credential FROM kb_connections WHERE id = $1")
            .bind(id.parse::<Uuid>().expect("uuid"))
            .fetch_one(&pool)
            .await
            .expect("read credential");
    assert!(
        stored.is_none(),
        "a denied attach wrote a credential anyway"
    );
}
