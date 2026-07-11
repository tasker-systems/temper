#![cfg(feature = "test-db")]
//! G3 Phase A: the machine-principal registration gate, proven end to end.
//!
//! `test-db` alone is a false signal for a change to authentication semantics, so this
//! drives a real Axum server. The MCP side of the same gate is covered by
//! `auth_seam_m2m_e2e.rs` — both surfaces resolve machines through the one
//! `temper-services` function, which is the point (D4).

mod common;

use uuid::Uuid;

/// Register a machine client against a freshly created agent profile.
async fn register(pool: &sqlx::PgPool, client_id: &str) -> Uuid {
    let profile_id = Uuid::now_v7();
    sqlx::query!(
        "INSERT INTO kb_profiles (id, handle, display_name, email, preferences) \
         VALUES ($1, $2, $2, NULL, '{}')",
        profile_id,
        format!("agent-{client_id}"),
    )
    .execute(pool)
    .await
    .expect("seed profile");
    sqlx::query!(
        "INSERT INTO kb_machine_clients (client_id, label, profile_id, registered_by_profile_id) \
         VALUES ($1, 'e2e', $2, $2)",
        client_id,
        profile_id,
    )
    .execute(pool)
    .await
    .expect("seed registration");
    profile_id
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn unregistered_machine_is_rejected_by_the_http_surface(pool: sqlx::PgPool) {
    let app = common::setup(pool.clone()).await;
    let token = common::generate_machine_jwt("ghost-client");

    let response = reqwest::Client::new()
        .get(app.url("/api/resources"))
        .bearer_auth(&token)
        .send()
        .await
        .expect("request");

    assert_eq!(
        response.status(),
        401,
        "an unregistered machine must not reach the data plane"
    );
    let body = response.text().await.expect("body");
    assert!(
        body.contains("not registered"),
        "the rejection names the reason: {body}"
    );
    assert!(
        body.contains("ghost-client"),
        "the rejection names the client id: {body}"
    );
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn registered_machine_reaches_the_data_plane(pool: sqlx::PgPool) {
    let app = common::setup(pool.clone()).await;
    register(&pool, "live-client").await;
    let token = common::generate_machine_jwt("live-client");

    let response = reqwest::Client::new()
        .get(app.url("/api/resources"))
        .bearer_auth(&token)
        .send()
        .await
        .expect("request");

    assert_eq!(
        response.status(),
        200,
        "a registered machine authenticates and passes the system gate"
    );
}

/// Register a temper-ISSUED machine client (issuer='temper', with a secret hash), as Phase B1's
/// `issue` path produces. The gate is issuer-agnostic, so this must authenticate exactly like an
/// auth0-m2m row — proving B1 changed no verifier (D4).
async fn register_temper_issued(pool: &sqlx::PgPool, client_id: &str) -> Uuid {
    let profile_id = Uuid::now_v7();
    sqlx::query!(
        "INSERT INTO kb_profiles (id, handle, display_name, email, preferences) \
         VALUES ($1, $2, $2, NULL, '{}')",
        profile_id,
        format!("agent-{client_id}"),
    )
    .execute(pool)
    .await
    .expect("seed profile");
    sqlx::query!(
        "INSERT INTO kb_machine_clients \
           (client_id, issuer, label, profile_id, registered_by_profile_id, secret_hash) \
         VALUES ($1, 'temper', 'e2e', $2, $2, 'deadbeef')",
        client_id,
        profile_id,
    )
    .execute(pool)
    .await
    .expect("seed temper-issued registration");
    profile_id
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn temper_issued_machine_reaches_the_data_plane(pool: sqlx::PgPool) {
    let app = common::setup(pool.clone()).await;
    register_temper_issued(&pool, "tmpr_live").await;
    let token = common::generate_machine_jwt("tmpr_live");

    let response = reqwest::Client::new()
        .get(app.url("/api/resources"))
        .bearer_auth(&token)
        .send()
        .await
        .expect("request");

    assert_eq!(
        response.status(),
        200,
        "a temper-issued machine authenticates through the unchanged gate (D4)"
    );
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn non_admin_cannot_issue_a_machine_credential(pool: sqlx::PgPool) {
    // A registered machine authenticates and passes the (open-mode) system gate — but it is not
    // the gating-team owner, so `is_system_admin` is false. That check is load-bearing on the new
    // issuer routes (D9): the system-gated router admits everyone under access_mode='open', so
    // only the explicit admin check protects `POST /issue`.
    let app = common::setup(pool.clone()).await;
    register(&pool, "not-admin").await;
    let token = common::generate_machine_jwt("not-admin");

    let response = reqwest::Client::new()
        .post(app.url("/api/machine-clients/issue"))
        .bearer_auth(&token)
        .json(&serde_json::json!({
            "label": "sneaky",
            "owner_team_id": null,
            "teams": [],
            "grants": [],
        }))
        .send()
        .await
        .expect("request");

    assert_eq!(
        response.status(),
        403,
        "a non-admin must not be able to issue a machine credential (D9)"
    );
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn revoked_machine_is_rejected_immediately(pool: sqlx::PgPool) {
    let app = common::setup(pool.clone()).await;
    register(&pool, "doomed-client").await;
    let token = common::generate_machine_jwt("doomed-client");
    let http = reqwest::Client::new();

    let before = http
        .get(app.url("/api/resources"))
        .bearer_auth(&token)
        .send()
        .await
        .expect("request");
    assert_eq!(before.status(), 200);

    sqlx::query!(
        "UPDATE kb_machine_clients SET revoked_at = now() WHERE client_id = 'doomed-client'"
    )
    .execute(&pool)
    .await
    .expect("revoke");

    // The SAME token — still cryptographically valid, still unexpired — is now dead.
    // Revocation does not wait for the token to expire, and does not need Auth0.
    let after = http
        .get(app.url("/api/resources"))
        .bearer_auth(&token)
        .send()
        .await
        .expect("request");
    assert_eq!(
        after.status(),
        401,
        "revocation takes effect on the next call"
    );
    assert!(after.text().await.expect("body").contains("revoked"));
}
