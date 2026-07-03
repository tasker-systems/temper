#![cfg(feature = "test-db")]
//! Stage 4b: a machine (`client_credentials`) token, driven through the real mcp
//! gate `ensure_profile_from_parts`, provisions a dedicated agent profile under
//! the `auth0-m2m` link namespace with a NULL email — never the email-reconcile
//! path.

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
async fn machine_token_provisions_agent_profile_via_mcp(pool: sqlx::PgPool) {
    let _app = common::setup(pool.clone()).await;
    let svc = build_mcp_service(&pool).await;

    svc.ensure_profile_from_parts(&machine_parts("steward-client-1"))
        .await
        .expect("mcp gate must admit + provision the agent");

    let link = sqlx::query!(
        "SELECT auth_provider, email FROM kb_profile_auth_links \
         WHERE auth_provider = $1 AND auth_provider_user_id = $2",
        "auth0-m2m",
        "steward-client-1",
    )
    .fetch_one(&pool)
    .await
    .expect("agent link row exists");
    assert_eq!(link.auth_provider, "auth0-m2m");
    assert!(link.email.is_none(), "agent link email must be NULL");
}
