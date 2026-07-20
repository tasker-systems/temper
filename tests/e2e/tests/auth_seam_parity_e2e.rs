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

use temper_services::auth_config::{AuthConfig, AuthMode};
use temper_services::config::ApiConfig;
use temper_services::state::{AppState, JwksKeyStore};

/// The `AppState` both MCP helpers below share — the same auth config temper-api runs with, so a
/// difference in behavior between the surfaces can only come from the surfaces themselves.
fn mcp_app_state(pool: &sqlx::PgPool) -> AppState {
    let decoding_key =
        jsonwebtoken::DecodingKey::from_rsa_pem(include_bytes!("fixtures/test_rsa.pub"))
            .expect("decoding key");
    let jwks_store = JwksKeyStore::with_static_key(decoding_key, jsonwebtoken::Algorithm::RS256);
    let api_config = ApiConfig {
        database_url: "unused".to_string(),
        auth: AuthConfig {
            issuer: "test-issuer".to_string(),
            jwks_url: "unused".to_string(),
            audience: common::TEST_AUDIENCE.to_string(),
            mode: AuthMode::ExternalIdp,
        },
        auth_provider_name: "test-provider".to_string(),
        cors_origins: vec![],
        port: 0,
        enable_swagger: false,
        internal_reconcile_secret: None,
        embed_dispatch_secret: None,
        vercel_connect: None,
        slack_link: None,
        slack_mint_secret: None,
    };
    AppState::new(pool.clone(), jwks_store, api_config)
}

/// Construct a `TemperMcpService` over the test pool. CONSTRUCTION ONLY — no
/// profile-cache seeding, because these tests assert REFUSAL: the gate call
/// (`ensure_profile_from_parts`) is the thing under test and is made by the
/// test body.
async fn build_mcp_service(pool: &sqlx::PgPool) -> temper_mcp::service::TemperMcpService {
    temper_mcp::service::TemperMcpService::new(mcp_app_state(pool))
}

/// Spawn the **real** MCP router — `build_router`, the same one `api/mcp.rs` serves — on a random
/// port, and return its base URL.
///
/// The other MCP helpers here call `ensure_profile_from_parts` with hand-built `RawJwtClaims`,
/// which enters *after* JWT verification. That means no test in this repo has ever driven
/// `require_mcp_auth`, and so MCP's `aud` check has been entirely uncovered — on the one surface
/// where the bug being fixed here originally diverged. This closes that.
async fn spawn_mcp_server(pool: &sqlx::PgPool) -> String {
    let mcp_config = temper_mcp::McpConfig {
        mcp_base_url: "http://localhost".to_string(),
        mcp_client_id: None,
        oauth: temper_mcp::config::OAuthStaticConfig {
            redirect_uris: vec![],
            allow_localhost: true,
        },
    };
    let router = temper_mcp::router::build_router(mcp_app_state(pool), mcp_config);

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind mcp test listener");
    let addr = listener.local_addr().expect("mcp local addr");
    tokio::spawn(async move {
        axum::serve(listener, router).await.expect("serve mcp");
    });

    format!("http://{addr}")
}

/// Build request `Parts` carrying `RawJwtClaims` for `sub`, to drive the MCP
/// surface's production gate. `exp: 0` is fine — the JWT was already validated
/// by middleware in prod; here we inject claims directly and
/// `ensure_profile_from_parts` does not re-check `exp`.
fn mcp_parts(sub: &str) -> axum::http::request::Parts {
    axum::http::Request::builder()
        // The MCP JWT middleware injects the raw bearer alongside the claims; the auth
        // seam needs it for the email ladder's /userinfo rung. Synthetic parts must
        // carry both or the service rejects the request as unwired.
        .extension(temper_mcp::middleware::BearerToken("synthetic".to_string()))
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

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn foreign_audience_token_is_refused_on_both_surfaces(pool: sqlx::PgPool) {
    // A correctly-signed, unexpired token from the trusted issuer, minted for a DIFFERENT API's
    // audience. Everything about it is valid except who it was meant for.
    //
    // It used to be accepted on temper-api: an unset `AUTH_AUDIENCE` set `validate_aud = false`, so
    // no audience was checked at all. Both surfaces must refuse it now — and this asserts BOTH,
    // because a two-surface divergence is the bug this file exists to catch.
    let app = common::setup(pool.clone()).await;
    let foreign = common::generate_jwt_for_other_audience("e2e-test-user", "e2e@test.com");

    // ── temper-api ────────────────────────────────────────────────────────────────
    // Positive control first: without it, a 401 below could just mean a malformed request.
    let ok = app
        .reqwest_client
        .get(app.url("/api/profile"))
        .header("Authorization", format!("Bearer {}", app.token))
        .send()
        .await
        .expect("api request");
    assert_eq!(
        ok.status(),
        StatusCode::OK,
        "positive control: a correctly-audienced token must be admitted on the API"
    );

    let refused = app
        .reqwest_client
        .get(app.url("/api/profile"))
        .header("Authorization", format!("Bearer {foreign}"))
        .send()
        .await
        .expect("api request");
    assert_eq!(
        refused.status(),
        StatusCode::UNAUTHORIZED,
        "API: a token minted for another audience must be refused"
    );

    // ── temper-mcp ────────────────────────────────────────────────────────────────
    // Driven through `build_router` — the real production router, and therefore the real
    // `require_mcp_auth`. This is the surface that had NO coverage of its `aud` check.
    let mcp_url = spawn_mcp_server(&pool).await;
    let client = reqwest::Client::new();

    let mcp_ok = client
        .post(format!("{mcp_url}/mcp"))
        .header("Authorization", format!("Bearer {}", app.token))
        .header("Content-Type", "application/json")
        .header("Accept", "application/json, text/event-stream")
        .body(r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26","capabilities":{},"clientInfo":{"name":"e2e","version":"0"}}}"#)
        .send()
        .await
        .expect("mcp request");
    assert_ne!(
        mcp_ok.status(),
        StatusCode::UNAUTHORIZED,
        "positive control: a correctly-audienced token must clear MCP's JWT gate"
    );

    let mcp_refused = client
        .post(format!("{mcp_url}/mcp"))
        .header("Authorization", format!("Bearer {foreign}"))
        .header("Content-Type", "application/json")
        .header("Accept", "application/json, text/event-stream")
        .body(r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26","capabilities":{},"clientInfo":{"name":"e2e","version":"0"}}}"#)
        .send()
        .await
        .expect("mcp request");
    assert_eq!(
        mcp_refused.status(),
        StatusCode::UNAUTHORIZED,
        "MCP: a token minted for another audience must be refused"
    );
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn token_with_no_audience_claim_is_refused_on_both_surfaces(pool: sqlx::PgPool) {
    // The subtler half of the same hole, and the one the first cut of this work MISSED.
    //
    // Setting an expected audience is not sufficient. `jsonwebtoken` only checks `aud` when the
    // claim is PRESENT — `required_spec_claims` defaults to `{"exp"}`, and the crate's own docs say
    // "Validation only happens if `aud` claim is present in the token." So a token that simply
    // OMITS `aud` was accepted even with `validate_aud = true`.
    //
    // The tell that this went unnoticed: adding `aud` to every fixture token was not load-bearing.
    // The whole suite would have stayed green without it, because an absent `aud` passed. A test
    // suite in which every token carries the claim can never discover that the claim is optional.
    let app = common::setup(pool.clone()).await;
    let audless = common::generate_jwt_without_audience("e2e-test-user", "e2e@test.com");

    let api = app
        .reqwest_client
        .get(app.url("/api/profile"))
        .header("Authorization", format!("Bearer {audless}"))
        .send()
        .await
        .expect("api request");
    assert_eq!(
        api.status(),
        StatusCode::UNAUTHORIZED,
        "API: a token with NO aud claim must be refused, not accepted by default"
    );

    let mcp_url = spawn_mcp_server(&pool).await;
    let mcp = reqwest::Client::new()
        .post(format!("{mcp_url}/mcp"))
        .header("Authorization", format!("Bearer {audless}"))
        .header("Content-Type", "application/json")
        .header("Accept", "application/json, text/event-stream")
        .body(r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26","capabilities":{},"clientInfo":{"name":"e2e","version":"0"}}}"#)
        .send()
        .await
        .expect("mcp request");
    assert_eq!(
        mcp.status(),
        StatusCode::UNAUTHORIZED,
        "MCP: a token with NO aud claim must be refused"
    );
}
