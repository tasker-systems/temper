#![cfg(feature = "test-db")]
//! Cross-surface auth parity: the gate a per-surface implementation would have
//! missed. Proves `is_active` (deactivation) and `system_access` are enforced
//! identically on temper-api (HTTP) and temper-mcp (`TemperMcpService`) — both
//! now routing through the shared `temper-services::auth` seam.
//!
//! The API surface is driven over HTTP through the real middleware stack via
//! `app.reqwest_client`. The MCP surface is driven by constructing a
//! `TemperMcpService` over the same test pool and calling the production gate
//! `ensure_profile_from_parts` with hand-built request `Parts` carrying
//! `RawJwtClaims` (mirroring the construction block in `act_authorship_mcp_e2e.rs`,
//! minus the profile-cache seed — here the gate call is the thing under test).

mod common;

use reqwest::StatusCode;

use temper_services::config::ApiConfig;
use temper_services::state::{AppState, JwksKeyStore};

/// Construct a `TemperMcpService` over the test pool. CONSTRUCTION ONLY — no
/// profile-cache seeding, because these tests assert REFUSAL: the gate call
/// (`ensure_profile_from_parts`) is the thing under test and is made by the
/// test body.
async fn build_mcp_service(pool: &sqlx::PgPool) -> temper_mcp::service::TemperMcpService {
    let decoding_key =
        jsonwebtoken::DecodingKey::from_rsa_pem(include_bytes!("fixtures/test_rsa.pub"))
            .expect("decoding key");
    let jwks_store = JwksKeyStore::with_static_key(decoding_key, jsonwebtoken::Algorithm::RS256);
    let api_config = ApiConfig {
        database_url: "unused".to_string(),
        jwks_url: "unused".to_string(),
        auth_issuer: "test-issuer".to_string(),
        auth_audience: None,
        auth_provider_name: "test-provider".to_string(),
        cors_origins: vec![],
        port: 0,
        enable_swagger: false,
        internal_reconcile_secret: None,
        embed_dispatch_secret: None,
    };
    let state = AppState::new(pool.clone(), jwks_store, api_config);
    temper_mcp::service::TemperMcpService::new(state)
}

/// Build request `Parts` carrying `RawJwtClaims` for `sub`, to drive the MCP
/// surface's production gate. `exp: 0` is fine — the JWT was already validated
/// by middleware in prod; here we inject claims directly and
/// `ensure_profile_from_parts` does not re-check `exp`.
fn mcp_parts(sub: &str) -> axum::http::request::Parts {
    axum::http::Request::builder()
        .extension(temper_services::auth::RawJwtClaims {
            sub: sub.to_string(),
            email: None,
            email_verified: None,
            azp: None,
            gty: None,
            exp: 0,
            iat: 0,
        })
        .body(())
        .expect("build request")
        .into_parts()
        .0
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn active_approved_allowed_on_both_surfaces(pool: sqlx::PgPool) {
    // Positive control: the happy path is admitted identically on both surfaces,
    // so the parity truth-table is self-contained (refusal cases below prove the
    // negatives). Open mode → an active, authenticated profile has system access.
    let app = common::setup(pool.clone()).await;

    // API surface: a gated endpoint succeeds.
    let api = app
        .reqwest_client
        .get(app.url("/api/resources"))
        .header("Authorization", format!("Bearer {}", app.token))
        .send()
        .await
        .expect("api request");
    assert_eq!(
        api.status(),
        StatusCode::OK,
        "API must admit an active, approved profile"
    );

    // MCP surface: the production gate resolves + authorizes without error.
    let svc = build_mcp_service(&pool).await;
    svc.ensure_profile_from_parts(&mcp_parts("e2e-test-user"))
        .await
        .expect("MCP must admit an active, approved profile");
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn deactivated_profile_refused_on_both_surfaces(pool: sqlx::PgPool) {
    // Open mode — deactivation is gated BEFORE system_access in the seam, so it
    // refuses even without invite-only.
    let app = common::setup(pool.clone()).await;

    // Preflight to create the `e2e-test-user` profile.
    let resp = app
        .reqwest_client
        .get(app.url("/api/profile"))
        .header("Authorization", format!("Bearer {}", app.token))
        .send()
        .await
        .expect("preflight");
    assert_eq!(resp.status(), StatusCode::OK, "preflight should succeed");

    // Deactivate the profile at the persistence layer.
    sqlx::query(
        "UPDATE kb_profiles SET is_active = false WHERE id IN \
         (SELECT profile_id FROM kb_profile_auth_links WHERE auth_provider_user_id = 'e2e-test-user')",
    )
    .execute(&pool)
    .await
    .expect("deactivate");

    // API surface: refused with UNAUTHORIZED.
    let api = app
        .reqwest_client
        .get(app.url("/api/resources"))
        .header("Authorization", format!("Bearer {}", app.token))
        .send()
        .await
        .expect("api request");
    assert_eq!(
        api.status(),
        StatusCode::UNAUTHORIZED,
        "API must refuse deactivated profile"
    );

    // MCP surface: refused (terminal rmcp error) through the real service gate.
    let svc = build_mcp_service(&pool).await;
    let err = svc
        .ensure_profile_from_parts(&mcp_parts("e2e-test-user"))
        .await
        .expect_err("MCP must refuse deactivated profile");
    assert!(
        err.message.contains("deactivated"),
        "MCP deactivation message expected, got: {}",
        err.message
    );
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn no_system_access_refused_on_both_surfaces(pool: sqlx::PgPool) {
    let app = common::setup(pool.clone()).await;

    // Preflight to create the admin profile and resolve its id.
    let profile = app
        .reqwest_client
        .get(app.url("/api/profile"))
        .header("Authorization", format!("Bearer {}", app.token))
        .send()
        .await
        .expect("preflight")
        .json::<serde_json::Value>()
        .await
        .expect("profile json");
    let admin_id: uuid::Uuid = profile["id"]
        .as_str()
        .expect("profile id")
        .parse()
        .expect("profile id parse");

    // Flip to invite-only, adding the first user as a gating member. A second,
    // non-member user now lacks system access.
    common::enable_invite_only(&pool, admin_id).await;

    let second = common::generate_second_user_jwt();

    // API surface FIRST — this also creates the `e2e-second-user` profile, so it
    // exists when the MCP path resolves it below. Refused with FORBIDDEN.
    let api = app
        .reqwest_client
        .get(app.url("/api/resources"))
        .header("Authorization", format!("Bearer {second}"))
        .send()
        .await
        .expect("api request");
    assert_eq!(
        api.status(),
        StatusCode::FORBIDDEN,
        "API must refuse profile with no system access"
    );

    // MCP surface: the `e2e-second-user` profile now exists; the gate must
    // refuse it for lack of system access through the real service gate.
    let svc = build_mcp_service(&pool).await;
    let err = svc
        .ensure_profile_from_parts(&mcp_parts("e2e-second-user"))
        .await
        .expect_err("MCP must refuse profile with no system access");
    assert!(
        err.message.contains("requires approval"),
        "MCP system-access message expected, got: {}",
        err.message
    );
}
