#![cfg(feature = "test-db")]
//! Stage 4b: a machine (`client_credentials`) token, driven through the real mcp
//! gate `ensure_profile_from_parts`. Since G3 Phase A, registration is fail-closed:
//! an unregistered client is rejected and creates nothing; a registered one resolves
//! to its pre-created agent profile. temper-mcp inherits the gate from
//! `temper-services` — it has no gate of its own (D4).

mod common;

use temper_services::config::ApiConfig;
use temper_services::state::{AppState, JwksKeyStore};

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

fn machine_parts(client_id: &str) -> axum::http::request::Parts {
    axum::http::Request::builder()
        .extension(temper_services::auth::RawJwtClaims {
            sub: format!("{client_id}@clients"),
            email: None,
            email_verified: None,
            azp: Some(client_id.to_string()),
            gty: Some("client-credentials".to_string()),
            exp: 0,
            iat: 0,
        })
        .body(())
        .expect("build request")
        .into_parts()
        .0
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn unregistered_machine_token_is_rejected_by_the_mcp_gate(pool: sqlx::PgPool) {
    let _app = common::setup(pool.clone()).await;
    let svc = build_mcp_service(&pool).await;

    let err = svc
        .ensure_profile_from_parts(&machine_parts("steward-client-1"))
        .await
        .expect_err("an unregistered machine must be rejected at the mcp gate");
    let rendered = format!("{err:?}");
    assert!(
        rendered.contains("not registered"),
        "the mcp surface inherits the services-layer gate (D4): {rendered}"
    );

    let links = sqlx::query_scalar!(
        "SELECT count(*) FROM kb_profile_auth_links WHERE auth_provider = 'auth0-m2m'",
    )
    .fetch_one(&pool)
    .await
    .expect("count links");
    assert_eq!(links, Some(0), "rejection creates no auth link");
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn registered_machine_token_is_admitted_by_the_mcp_gate(pool: sqlx::PgPool) {
    let _app = common::setup(pool.clone()).await;
    let svc = build_mcp_service(&pool).await;

    let profile_id = uuid::Uuid::now_v7();
    sqlx::query!(
        "INSERT INTO kb_profiles (id, handle, display_name, email, preferences) \
         VALUES ($1, 'agent-steward', 'agent-steward', NULL, '{}')",
        profile_id,
    )
    .execute(&pool)
    .await
    .expect("seed profile");
    sqlx::query!(
        "INSERT INTO kb_machine_clients (client_id, label, profile_id, registered_by_profile_id) \
         VALUES ('steward-client-1', 'test', $1, $1)",
        profile_id,
    )
    .execute(&pool)
    .await
    .expect("seed registration");

    svc.ensure_profile_from_parts(&machine_parts("steward-client-1"))
        .await
        .expect("mcp gate must admit a registered machine");
}
