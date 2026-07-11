#![cfg(feature = "test-db")]
//! G3 Phase B2 — machine-client registration authorization and reach containment, over HTTP.
//!
//! `test-db` green is a false signal for access semantics: these assertions have to run through
//! the real router, the real auth middleware, and real JWTs. The bite test here
//! (`gating_team_maintainer_cannot_mint_a_system_admin`) is the one that matters — it asserts a
//! privilege-escalation path is closed, not merely that a predicate returns false.

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
    assert_eq!(resp.status(), 201, "team create: {:?}", resp.text().await);
    let body: serde_json::Value = resp.json().await.expect("team json");
    body["id"].as_str().expect("team id").parse().expect("uuid")
}

/// `POST /api/machine-clients/issue`. Returns (status, body).
async fn issue(
    app: &common::E2eTestApp,
    token: &str,
    body: serde_json::Value,
) -> (reqwest::StatusCode, serde_json::Value) {
    let resp = app
        .reqwest_client
        .post(app.url("/api/machine-clients/issue"))
        .bearer_auth(token)
        .json(&body)
        .send()
        .await
        .expect("POST /issue");
    let status = resp.status();
    let json = resp.json().await.unwrap_or(serde_json::Value::Null);
    (status, json)
}

async fn machine_count(pool: &PgPool) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM kb_machine_clients")
        .fetch_one(pool)
        .await
        .expect("count machines")
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn team_owner_can_issue_for_their_own_team(pool: PgPool) {
    let app = common::setup(pool.clone()).await;
    let token = common::generate_test_jwt("b2-owner", "b2-owner@example.com");
    provision_profile(&app, &token).await;
    let team = create_team(&app, &token, "b2-owner-team").await;

    let (status, body) = issue(
        &app,
        &token,
        json!({ "label": "team agent", "owner_team_id": team, "teams": [], "grants": [] }),
    )
    .await;

    assert_eq!(
        status, 200,
        "a team owner may issue for their own team: {body:?}"
    );
    assert!(
        body["client_secret"]
            .as_str()
            .is_some_and(|s| !s.is_empty()),
        "the plaintext secret is returned once"
    );
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn non_owner_cannot_issue_for_a_team(pool: PgPool) {
    let app = common::setup(pool.clone()).await;

    let owner_token = common::generate_test_jwt("b2-o", "b2-o@example.com");
    provision_profile(&app, &owner_token).await;
    let team = create_team(&app, &owner_token, "b2-nonowner-team").await;

    // A maintainer of the very same team is still not an owner.
    let maint_token = common::generate_test_jwt("b2-m", "b2-m@example.com");
    let maint_id = provision_profile(&app, &maint_token).await;
    let resp = app
        .reqwest_client
        .post(app.url(&format!("/api/teams/{team}/members")))
        .bearer_auth(&owner_token)
        .json(&json!({ "profile_id": maint_id, "role": "maintainer" }))
        .send()
        .await
        .expect("add maintainer");
    assert_eq!(resp.status(), 201);

    let (status, _) = issue(
        &app,
        &maint_token,
        json!({ "label": "nope", "owner_team_id": team, "teams": [], "grants": [] }),
    )
    .await;
    assert_eq!(
        status, 403,
        "a maintainer is not an owner; registration needs OWNER"
    );

    // And a total stranger.
    let stranger = common::generate_test_jwt("b2-s", "b2-s@example.com");
    provision_profile(&app, &stranger).await;
    let (status, _) = issue(
        &app,
        &stranger,
        json!({ "label": "nope", "owner_team_id": team, "teams": [], "grants": [] }),
    )
    .await;
    assert_eq!(
        status, 403,
        "a non-member cannot register for someone else's team"
    );
}

/// Spec D2 — a teamless registration is admin-only. NULL must deny, not fall open.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn non_admin_cannot_issue_a_teamless_machine(pool: PgPool) {
    let app = common::setup(pool.clone()).await;
    let token = common::generate_test_jwt("b2-null", "b2-null@example.com");
    provision_profile(&app, &token).await;
    create_team(&app, &token, "b2-null-team").await; // owns a team, but doesn't name it

    let (status, _) = issue(
        &app,
        &token,
        json!({ "label": "teamless", "owner_team_id": null, "teams": [], "grants": [] }),
    )
    .await;
    assert_eq!(status, 403, "owner_team_id: null is admin-only (D2)");
}

/// Spec D4 + auth-before-writes: reach into an unmanaged team is refused AND writes nothing.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn reach_into_an_unmanaged_team_is_denied_and_writes_nothing(pool: PgPool) {
    let app = common::setup(pool.clone()).await;

    let alice = common::generate_test_jwt("b2-alice", "b2-alice@example.com");
    provision_profile(&app, &alice).await;
    let alice_team = create_team(&app, &alice, "b2-alice-team").await;

    let bob = common::generate_test_jwt("b2-bob", "b2-bob@example.com");
    provision_profile(&app, &bob).await;
    let bob_team = create_team(&app, &bob, "b2-bob-team").await;

    let before = machine_count(&pool).await;

    let (status, _) = issue(
        &app,
        &alice,
        json!({
            "label": "reaching too far",
            "owner_team_id": alice_team,
            "teams": [{ "team_id": bob_team, "role": "member" }],
            "grants": []
        }),
    )
    .await;

    assert_eq!(status, 403, "Alice cannot walk a machine into Bob's team");
    assert_eq!(
        machine_count(&pool).await,
        before,
        "auth before writes: a rejected registration leaves NO row behind"
    );
}

/// Spec D4 + auth-before-writes: a grant on a cogmap the caller cannot administer is refused,
/// and nothing is written. The L0 kernel cogmap is the natural subject — a fresh team owner
/// holds no `can_grant` on it.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn grant_without_can_grant_is_denied_and_writes_nothing(pool: PgPool) {
    let app = common::setup(pool.clone()).await;

    let alice = common::generate_test_jwt("b2-g-alice", "b2-g-alice@example.com");
    provision_profile(&app, &alice).await;
    let alice_team = create_team(&app, &alice, "b2-grant-team").await;

    let l0 = "00000000-0000-0000-0005-000000000001";
    let before = machine_count(&pool).await;

    let (status, _) = issue(
        &app,
        &alice,
        json!({
            "label": "over-granted",
            "owner_team_id": alice_team,
            "teams": [],
            "grants": [{ "cogmap_id": l0, "can_write": true }]
        }),
    )
    .await;

    assert_eq!(
        status, 403,
        "a machine cannot be granted write on a cogmap its minter cannot administer"
    );
    assert_eq!(
        machine_count(&pool).await,
        before,
        "auth before writes: a rejected grant leaves NO row behind"
    );
}

/// **The bite test (spec D4a).** A gating-team MAINTAINER is not a system admin, but clears
/// `can_manage` on the gating team. Without the role bar they could mint a machine at
/// `role = owner` on the gating team — and that machine WOULD be `is_system_admin`.
/// This asserts the escalation is closed, not merely that a predicate said no.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn gating_team_maintainer_cannot_mint_a_system_admin(pool: PgPool) {
    let app = common::setup(pool.clone()).await;

    let alice = common::generate_test_jwt("b2-esc", "b2-esc@example.com");
    let alice_id = provision_profile(&app, &alice).await;
    let alice_team = create_team(&app, &alice, "b2-esc-team").await;

    // Alice is a MAINTAINER of the gating team — emphatically not an owner, so not an admin.
    common::add_to_gating_team(&pool, alice_id, "maintainer").await;

    let is_admin: Option<bool> = sqlx::query_scalar("SELECT is_system_admin($1)")
        .bind(alice_id)
        .fetch_one(&pool)
        .await
        .expect("is_system_admin");
    assert!(
        !is_admin.unwrap_or(false),
        "precondition: a gating-team maintainer is NOT a system admin"
    );

    let gating_id: Uuid =
        sqlx::query_scalar("SELECT id FROM kb_teams WHERE slug = 'temper-system'")
            .fetch_one(&pool)
            .await
            .expect("gating team id");

    let (status, _) = issue(
        &app,
        &alice,
        json!({
            "label": "trojan",
            "owner_team_id": alice_team,
            "teams": [{ "team_id": gating_id, "role": "owner" }],
            "grants": []
        }),
    )
    .await;

    assert_eq!(
        status, 403,
        "minting a gating-team OWNER is an escalation to system admin"
    );

    let admin_machines: i64 = sqlx::query_scalar(
        "SELECT count(*)
           FROM kb_machine_clients mc
           JOIN kb_team_members tm ON tm.profile_id = mc.profile_id
          WHERE tm.team_id = $1 AND tm.role = 'owner'",
    )
    .bind(gating_id)
    .fetch_one(&pool)
    .await
    .expect("count admin machines");
    assert_eq!(
        admin_machines, 0,
        "no machine may hold owner on the gating team"
    );
}

/// Spec D5 — a system admin retains full, unchecked reach.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn system_admin_retains_full_reach(pool: PgPool) {
    let app = common::setup(pool.clone()).await;

    let admin = common::generate_test_jwt("b2-admin", "b2-admin@example.com");
    let admin_id = provision_profile(&app, &admin).await;
    common::make_system_admin(&pool, admin_id).await;

    let bob = common::generate_test_jwt("b2-bob2", "b2-bob2@example.com");
    provision_profile(&app, &bob).await;
    let bob_team = create_team(&app, &bob, "b2-admin-foreign").await;

    let (status, body) = issue(
        &app,
        &admin,
        json!({
            "label": "operator agent",
            "owner_team_id": null,
            "teams": [{ "team_id": bob_team, "role": "member" }],
            "grants": []
        }),
    )
    .await;

    assert_eq!(
        status, 200,
        "an admin may confer any reach (Phase A D5): {body:?}"
    );
}

/// Spec D5 — reads and per-row lifecycle are scoped to the owning team.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn reads_and_lifecycle_are_scoped_to_the_owning_team(pool: PgPool) {
    let app = common::setup(pool.clone()).await;

    let alice = common::generate_test_jwt("b2-r-alice", "b2-r-alice@example.com");
    provision_profile(&app, &alice).await;
    let alice_team = create_team(&app, &alice, "b2-r-alice-team").await;

    let bob = common::generate_test_jwt("b2-r-bob", "b2-r-bob@example.com");
    provision_profile(&app, &bob).await;
    create_team(&app, &bob, "b2-r-bob-team").await;

    let (status, body) = issue(
        &app,
        &alice,
        json!({ "label": "alice agent", "owner_team_id": alice_team, "teams": [], "grants": [] }),
    )
    .await;
    assert_eq!(status, 200);
    let machine_id = body["client"]["id"]
        .as_str()
        .expect("machine id")
        .to_string();

    // Alice sees her machine; Bob sees none.
    let mine: serde_json::Value = app
        .reqwest_client
        .get(app.url("/api/machine-clients"))
        .bearer_auth(&alice)
        .send()
        .await
        .expect("list as alice")
        .json()
        .await
        .expect("json");
    assert_eq!(
        mine.as_array().expect("array").len(),
        1,
        "Alice sees her own machine"
    );

    let theirs: serde_json::Value = app
        .reqwest_client
        .get(app.url("/api/machine-clients"))
        .bearer_auth(&bob)
        .send()
        .await
        .expect("list as bob")
        .json()
        .await
        .expect("json");
    assert!(
        theirs.as_array().expect("array").is_empty(),
        "Bob owns a team, but none of Alice's machines"
    );

    // Bob cannot GET, revoke, or rotate Alice's machine.
    let resp = app
        .reqwest_client
        .get(app.url(&format!("/api/machine-clients/{machine_id}")))
        .bearer_auth(&bob)
        .send()
        .await
        .expect("get as bob");
    assert_eq!(resp.status(), 403, "Bob cannot read Alice's machine");

    let resp = app
        .reqwest_client
        .post(app.url(&format!("/api/machine-clients/{machine_id}/rotate-secret")))
        .bearer_auth(&bob)
        .json(&json!({ "grace_seconds": 0 }))
        .send()
        .await
        .expect("rotate as bob");
    assert_eq!(
        resp.status(),
        403,
        "Bob cannot rotate Alice's machine's secret"
    );

    let resp = app
        .reqwest_client
        .delete(app.url(&format!("/api/machine-clients/{machine_id}")))
        .bearer_auth(&bob)
        .send()
        .await
        .expect("revoke as bob");
    assert_eq!(resp.status(), 403, "Bob cannot revoke Alice's machine");

    // Alice can do all three — this is the point of the phase: no operator in the loop.
    let resp = app
        .reqwest_client
        .post(app.url(&format!("/api/machine-clients/{machine_id}/rotate-secret")))
        .bearer_auth(&alice)
        .json(&json!({ "grace_seconds": 0 }))
        .send()
        .await
        .expect("rotate as alice");
    assert_eq!(resp.status(), 200, "Alice rotates her own machine's secret");

    let resp = app
        .reqwest_client
        .delete(app.url(&format!("/api/machine-clients/{machine_id}")))
        .bearer_auth(&alice)
        .send()
        .await
        .expect("revoke as alice");
    assert_eq!(resp.status(), 200, "Alice revokes her own machine");
}
