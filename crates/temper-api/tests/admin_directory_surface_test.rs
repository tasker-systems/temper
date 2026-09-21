#![cfg(feature = "test-db")]
//! HTTP witnesses for the operator directory's doors:
//!
//! * the gate runs BEFORE any existence lookup — a non-admin asking for a NONEXISTENT profile
//!   UUID gets the same plain 403 as for a real one, so absence never leaks below the gate
//!   (spec §7, pinned ordering). Pinned for BOTH refusal classes: a born-Denied outsider is
//!   refused by the router's Level-2 `require_system_access` before any handler code runs, so
//!   the approved non-admin class — the only one that reaches the handler body — carries its
//!   own pin on the handler's Level-3 `require_system_admin`;
//! * `?email=` is an identity-resolution act: exact, case-insensitive, over verified emails —
//!   one match → the state card; zero → 404; two verified owners → 404 whose BODY NAMES the
//!   collision; a lookalike address is a different identity, never a substring match (§6, C1);
//! * the list carries `total`, clamps its page, and no response body ever carries a token.

mod common;

use serde_json::Value;
use sqlx::PgPool;
use uuid::Uuid;

/// Seed a HUMAN profile with one verified auth link (provider `saml:okta` — deliberately not
/// `test-provider`, so the discriminator is exercised against a non-JWT shape too).
async fn seed_human(pool: &PgPool, handle: &str, email: &str) -> Uuid {
    let id = Uuid::now_v7();
    sqlx::query("INSERT INTO kb_profiles (id, handle, display_name) VALUES ($1, $2, $2)")
        .bind(id)
        .bind(handle)
        .execute(pool)
        .await
        .expect("seed profile");
    sqlx::query(
        "INSERT INTO kb_profile_auth_links \
         (id, profile_id, auth_provider, auth_provider_user_id, email, email_verified, is_default) \
         VALUES ($1, $2, 'saml:okta', $3, $4, true, true)",
    )
    .bind(Uuid::now_v7())
    .bind(id)
    .bind(format!("saml-{}", id.simple()))
    .bind(email)
    .execute(pool)
    .await
    .expect("seed auth link");
    id
}

/// Provision `sub` through a real authenticated request, then grant operator authority
/// (approved standing + governance). JWT first sign-in is born `Denied` (D11).
async fn provision_and_make_operator(app: &common::TestApp, sub: &str, email: &str) -> String {
    let token = common::generate_test_jwt(sub, email);
    let resp = app
        .client
        .get(app.url("/api/profile"))
        .header("Authorization", format!("Bearer {token}"))
        .send()
        .await
        .expect("provisioning request");
    assert_eq!(
        resp.status().as_u16(),
        200,
        "first sign-in must provision: {}",
        resp.text().await.unwrap_or_default()
    );
    let profile: Uuid = sqlx::query_scalar(
        "SELECT profile_id FROM kb_profile_auth_links WHERE auth_provider_user_id = $1",
    )
    .bind(sub)
    .fetch_one(&app.pool)
    .await
    .expect("provisioned profile");
    common::fixtures::make_test_admin(&app.pool, profile).await;
    token
}

/// Provision `sub` and grant standing ONLY — authenticated, but not an operator.
async fn provision_non_operator(app: &common::TestApp, sub: &str, email: &str) -> String {
    let token = common::generate_test_jwt(sub, email);
    let resp = app
        .client
        .get(app.url("/api/profile"))
        .header("Authorization", format!("Bearer {token}"))
        .send()
        .await
        .expect("provisioning request");
    assert_eq!(resp.status().as_u16(), 200);
    token
}

/// Provision `sub` and grant approved standing ONLY — clears the router's Level-2
/// `require_system_access` but holds no governance, so the handler's own `require_system_admin`
/// gate is what must refuse. This is the only refused caller class that reaches a gated
/// handler body.
async fn provision_and_approve(app: &common::TestApp, sub: &str, email: &str) -> String {
    let token = common::generate_test_jwt(sub, email);
    let resp = app
        .client
        .get(app.url("/api/profile"))
        .header("Authorization", format!("Bearer {token}"))
        .send()
        .await
        .expect("provisioning request");
    assert_eq!(resp.status().as_u16(), 200);
    let profile: Uuid = sqlx::query_scalar(
        "SELECT profile_id FROM kb_profile_auth_links WHERE auth_provider_user_id = $1",
    )
    .bind(sub)
    .fetch_one(&app.pool)
    .await
    .expect("provisioned profile");
    common::fixtures::approve_standing(&app.pool, profile).await;
    token
}

async fn get_json(app: &common::TestApp, token: &str, path: &str) -> (u16, Value) {
    let resp = app
        .client
        .get(app.url(path))
        .header("Authorization", format!("Bearer {token}"))
        .send()
        .await
        .expect("request");
    let status = resp.status().as_u16();
    let body: Value =
        serde_json::from_str(&resp.text().await.unwrap_or_default()).unwrap_or(Value::Null);
    (status, body)
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn non_admin_getting_a_nonexistent_uuid_gets_the_uniform_403(pool: PgPool) {
    let app = common::setup_test_app(pool).await;
    let outsider = provision_non_operator(&app, " outsider|1", "outsider@test.example").await;

    // Gate before existence: the SAME refusal for a UUID that names nothing.
    let (real_shaped, body) = get_json(
        &app,
        &outsider,
        &format!("/api/access/admin/profiles/{}", Uuid::now_v7()),
    )
    .await;
    assert_eq!(real_shaped, 403, "non-admin on a nonexistent uuid: {body}");
    let (list, body) = get_json(&app, &outsider, "/api/access/admin/profiles").await;
    assert_eq!(list, 403, "non-admin on the list: {body}");
    // The ?email= identity-resolution door is the SAME gate, no side door.
    let (by_email, body) = get_json(
        &app,
        &outsider,
        "/api/access/admin/profiles?email=anyone%40corp.example",
    )
    .await;
    assert_eq!(by_email, 403, "non-admin on ?email=: {body}");
}

/// §7's ordering pin, second class. The born-Denied pin above is refused by the ROUTER's
/// Level-2 `require_system_access` before any handler body runs, so its green proves nothing
/// about the handler. An APPROVED non-admin is the only refused class that reaches the handler
/// body: the show door's Level-3 `require_system_admin` must run before any lookup of the path
/// UUID, or this caller could distinguish a real profile (404) from an absent one.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn approved_non_admin_getting_a_nonexistent_uuid_gets_the_uniform_403(pool: PgPool) {
    let app = common::setup_test_app(pool).await;
    let approved = provision_and_approve(
        &app,
        "approved-outsider|1",
        "approved-outsider@test.example",
    )
    .await;

    let (status, body) = get_json(
        &app,
        &approved,
        &format!("/api/access/admin/profiles/{}", Uuid::now_v7()),
    )
    .await;
    assert_eq!(
        status, 403,
        "approved non-admin on a nonexistent uuid: {body}"
    );
}

/// The directory is GET-only by ROUTE REGISTRATION, not by handler-side checks: POST, PUT,
/// PATCH and DELETE on both paths must meet 405 (method not allowed), NOT 403 — the point is
/// that no write method door exists at all, so there is nothing a non-admin could even be
/// refused. A 403 here would mean a door exists and only the gate is in front of it.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn the_directory_routes_register_no_write_method(pool: PgPool) {
    use reqwest::Method;

    let app = common::setup_test_app(pool).await;
    let admin = provision_and_make_operator(&app, "operator|1", "operator@test.example").await;
    let human = seed_human(&app.pool, "ro-one", "ro@corp.example").await;
    let paths = [
        "/api/access/admin/profiles".to_string(),
        format!("/api/access/admin/profiles/{human}"),
    ];

    for path in &paths {
        for method in [Method::POST, Method::PUT, Method::PATCH, Method::DELETE] {
            let resp = app
                .client
                .request(method.clone(), app.url(path))
                .header("Authorization", format!("Bearer {admin}"))
                .send()
                .await
                .expect("request");
            let status = resp.status().as_u16();
            assert_eq!(
                status, 405,
                "{method} {path} must have no door at all (got {status})"
            );
        }
    }
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn email_resolution_exact_card_zero_404_and_ambiguity_names_the_collision(pool: PgPool) {
    let app = common::setup_test_app(pool).await;
    let admin = provision_and_make_operator(&app, "operator|1", "operator@test.example").await;
    let alice = seed_human(&app.pool, "alice", "alice@corp.example").await;

    // Exact, case-insensitive → the card.
    let (status, card) = get_json(
        &app,
        &admin,
        "/api/access/admin/profiles?email=ALICE%40corp.example",
    )
    .await;
    assert_eq!(status, 200, "{card}");
    assert_eq!(
        card["profile_id"].as_str(),
        Some(alice.to_string().as_str())
    );
    assert_eq!(card["email"].as_str(), Some("alice@corp.example"));
    assert_eq!(card["standing"].as_str(), Some("denied"));

    // Zero matches → 404.
    let (status, body) = get_json(
        &app,
        &admin,
        "/api/access/admin/profiles?email=nobody%40corp.example",
    )
    .await;
    assert_eq!(status, 404, "{body}");

    // A verified lookalike is a DIFFERENT identity: the original still resolves, exactly.
    seed_human(&app.pool, "lookalike", "alice@corp.example.evil.io").await;
    let (status, card) = get_json(
        &app,
        &admin,
        "/api/access/admin/profiles?email=alice%40corp.example",
    )
    .await;
    assert_eq!(status, 200, "{card}");
    assert_eq!(
        card["profile_id"].as_str(),
        Some(alice.to_string().as_str())
    );

    // Two profiles verified-own the address → 404 whose body NAMES the collision.
    seed_human(&app.pool, "alice-two", "alice@corp.example").await;
    let (status, body) = get_json(
        &app,
        &admin,
        "/api/access/admin/profiles?email=alice%40corp.example",
    )
    .await;
    assert_eq!(status, 404, "{body}");
    let text = body.to_string();
    assert!(text.contains("2 profiles"), "must name the count: {text}");

    // email and email_contains are different axes: naming both is refused, not guessed.
    let (status, body) = get_json(
        &app,
        &admin,
        "/api/access/admin/profiles?email=a%40corp.example&email_contains=corp",
    )
    .await;
    assert_eq!(status, 400, "{body}");
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn the_list_carries_total_and_honors_the_standing_filter(pool: PgPool) {
    let app = common::setup_test_app(pool).await;
    let admin = provision_and_make_operator(&app, "operator|1", "operator@test.example").await;
    let denied_one = seed_human(&app.pool, "denied-one", "d1@corp.example").await;
    let _approved = seed_human(&app.pool, "approved-one", "ap@corp.example").await;
    sqlx::query("INSERT INTO kb_principal_standing (profile_id, state) VALUES ($1, 'approved')")
        .bind(_approved)
        .execute(&app.pool)
        .await
        .expect("approve");

    // Default = needs-access: only the never-approved human.
    let (status, page) = get_json(&app, &admin, "/api/access/admin/profiles").await;
    assert_eq!(status, 200, "{page}");
    assert_eq!(page["total"].as_i64(), Some(1));
    assert_eq!(page["entries"].as_array().map(Vec::len), Some(1));
    assert_eq!(
        page["entries"][0]["profile_id"].as_str(),
        Some(denied_one.to_string().as_str())
    );
    assert_eq!(page["entries"][0]["standing"].as_str(), Some("denied"));

    // `all` → both seeded humans PLUS the operator themself (their JWT first-sign-in
    // provisioned a `test-provider` link — the operator is a human too); a named state
    // narrows; an unknown name is a 400.
    let (status, page) = get_json(&app, &admin, "/api/access/admin/profiles?standing=all").await;
    assert_eq!(status, 200);
    assert_eq!(page["total"].as_i64(), Some(3));
    let (status, _) = get_json(
        &app,
        &admin,
        "/api/access/admin/profiles?standing=unapproved",
    )
    .await;
    assert_eq!(status, 400);
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn list_pagination_clamps_and_the_card_never_carries_a_token(pool: PgPool) {
    let app = common::setup_test_app(pool).await;
    let admin = provision_and_make_operator(&app, "operator|1", "operator@test.example").await;
    let invitee = seed_human(&app.pool, "invitee", "invitee@corp.example").await;
    let inviter = seed_human(&app.pool, "inviter", "inviter@corp.example").await;

    // A pending invitation to a verified address the invitee uniquely owns.
    let team_id: Uuid = sqlx::query_scalar(
        "INSERT INTO kb_teams (slug, name) VALUES ('invited-teams', 'Invited Teams') RETURNING id",
    )
    .fetch_one(&app.pool)
    .await
    .expect("team");
    sqlx::query(
        "INSERT INTO kb_team_invitations \
         (id, team_id, invited_email, invited_by_profile_id, role, token) \
         VALUES ($1, $2, 'invitee@corp.example', $3, 'member', $4)",
    )
    .bind(Uuid::now_v7())
    .bind(team_id)
    .bind(inviter)
    .bind(format!("secret-token-{}", Uuid::now_v7()))
    .execute(&app.pool)
    .await
    .expect("invitation");

    // Page clamps: a huge limit returns everything rather than erroring.
    let (status, page) = get_json(
        &app,
        &admin,
        "/api/access/admin/profiles?standing=all&limit=99999",
    )
    .await;
    assert_eq!(status, 200, "{page}");
    // invitee + inviter + the operator themself (a JWT-provisioned human).
    assert_eq!(page["entries"].as_array().map(Vec::len), Some(3));

    // The card is token-free over the whole serialized body — the invitation's redemption
    // token must not exist as a field, let alone a value.
    let (status, card) = get_json(
        &app,
        &admin,
        &format!("/api/access/admin/profiles/{invitee}"),
    )
    .await;
    assert_eq!(status, 200, "{card}");
    assert_eq!(
        card["pending_invitations"].as_array().map(Vec::len),
        Some(1)
    );
    let text = card.to_string();
    assert!(
        !text.contains("token"),
        "token leaked into the card: {text}"
    );
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn show_by_uuid_returns_the_card_and_404s_an_absent_profile(pool: PgPool) {
    let app = common::setup_test_app(pool).await;
    let admin = provision_and_make_operator(&app, "operator|1", "operator@test.example").await;
    let human = seed_human(&app.pool, "show-one", "show@corp.example").await;

    let (status, card) =
        get_json(&app, &admin, &format!("/api/access/admin/profiles/{human}")).await;
    assert_eq!(status, 200, "{card}");
    assert_eq!(card["handle"].as_str(), Some("show-one"));
    // Machine-legal hints (adversarial review F2): this fixture is standing-row ABSENCE —
    // the card renders it `denied`, but approve refuses from absence and no other access
    // act is legal there either, so the card advertises NO `temper admin access` command.
    // Row-bearing classes keep their hints (pinned by the service class-table test).
    assert!(card["hints"].as_array().expect("hints").iter().all(|h| !h
        .as_str()
        .expect("hint text")
        .starts_with("temper admin access ")));

    let (status, body) = get_json(
        &app,
        &admin,
        &format!("/api/access/admin/profiles/{}", Uuid::now_v7()),
    )
    .await;
    assert_eq!(status, 404, "{body}");
}
