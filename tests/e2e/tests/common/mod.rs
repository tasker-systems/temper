#![allow(dead_code)]
//! Shared e2e test infrastructure.
//!
//! `E2eTestApp` starts an in-process Axum server backed by an isolated
//! per-test database and builds a `TemperClient` with injected config
//! (no disk reads, no env var manipulation).

pub mod tracing_layer;

use std::net::SocketAddr;

use chrono::{Duration, Utc};
use jsonwebtoken::{Algorithm, EncodingKey, Header};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use tempfile::TempDir;
use tokio::net::TcpListener;

use temper_api::{
    config::ApiConfig,
    create_app,
    state::{AppState, JwksKeyStore},
};
use temper_client::auth::StoredAuth;
use temper_core::types::config::{CloudSection, CloudVaultConfig, TemperConfig};

// Well-known UUIDs from seed migration.
pub const SYSTEM_PROFILE_ID: &str = "00000000-0000-0000-0004-000000000001";
pub const TEMPER_CONTEXT_ID: &str = "00000000-0000-0000-0003-000000000001";
pub const RESEARCH_DOC_TYPE_ID: &str = "00000000-0000-0000-0001-000000000004";

/// A running e2e test environment with in-process API server and injected client.
pub struct E2eTestApp {
    pub addr: SocketAddr,
    pub pool: PgPool,
    pub client: temper_client::TemperClient,
    pub reqwest_client: reqwest::Client,
    pub config: TemperConfig,
    pub cli_config: temper_cli::config::Config,
    pub token: String,
    pub vault_dir: TempDir,
}

impl E2eTestApp {
    pub fn base_url(&self) -> String {
        format!("http://{}", self.addr)
    }

    pub fn url(&self, path: &str) -> String {
        format!("{}{}", self.base_url(), path)
    }
}

/// JWT claims for test tokens.
#[derive(Debug, Serialize, Deserialize)]
struct TestClaims {
    sub: String,
    email: String,
    email_verified: bool,
    iss: String,
    iat: i64,
    exp: i64,
}

/// Sign a JWT with the test RSA private key. Valid for 1 hour.
pub fn generate_test_jwt(sub: &str, email: &str) -> String {
    let encoding_key = EncodingKey::from_rsa_pem(include_bytes!("../fixtures/test_rsa.key"))
        .expect("Failed to load test RSA private key");

    let now = Utc::now().timestamp();
    let claims = TestClaims {
        sub: sub.to_string(),
        email: email.to_string(),
        email_verified: true,
        iss: "test-issuer".to_string(),
        iat: now,
        exp: now + 3600,
    };

    jsonwebtoken::encode(&Header::new(Algorithm::RS256), &claims, &encoding_key)
        .expect("Failed to sign test JWT")
}

/// Sign an expired JWT (expired 1 hour ago).
pub fn generate_expired_jwt(sub: &str, email: &str) -> String {
    let encoding_key = EncodingKey::from_rsa_pem(include_bytes!("../fixtures/test_rsa.key"))
        .expect("Failed to load test RSA private key");

    let now = Utc::now().timestamp();
    let claims = TestClaims {
        sub: sub.to_string(),
        email: email.to_string(),
        email_verified: true,
        iss: "test-issuer".to_string(),
        iat: now - 7200,
        exp: now - 3600,
    };

    jsonwebtoken::encode(&Header::new(Algorithm::RS256), &claims, &encoding_key)
        .expect("Failed to sign expired JWT")
}

/// Seed fixtures: delete test data, insert stable seed resource.
async fn clean_and_seed(pool: &PgPool) {
    sqlx::query(
        "DELETE FROM kb_events WHERE profile_id NOT IN (
            '00000000-0000-0000-0004-000000000001',
            '00000000-0000-0000-0004-000000000002'
        )",
    )
    .execute(pool)
    .await
    .expect("clean kb_events");

    sqlx::query("DELETE FROM kb_device_sync_state")
        .execute(pool)
        .await
        .expect("clean kb_device_sync_state");
    sqlx::query("DELETE FROM kb_transfers")
        .execute(pool)
        .await
        .expect("clean kb_transfers");
    sqlx::query("DELETE FROM kb_team_invitations")
        .execute(pool)
        .await
        .expect("clean kb_team_invitations");
    sqlx::query("DELETE FROM kb_team_resources")
        .execute(pool)
        .await
        .expect("clean kb_team_resources");
    sqlx::query("DELETE FROM kb_team_members")
        .execute(pool)
        .await
        .expect("clean kb_team_members");
    sqlx::query("DELETE FROM kb_teams")
        .execute(pool)
        .await
        .expect("clean kb_teams");

    sqlx::query(
        "DELETE FROM kb_resources WHERE owner_profile_id NOT IN (
            '00000000-0000-0000-0004-000000000001',
            '00000000-0000-0000-0004-000000000002'
        )",
    )
    .execute(pool)
    .await
    .expect("clean test resources");

    sqlx::query(
        "DELETE FROM kb_profile_auth_links WHERE profile_id NOT IN (
            '00000000-0000-0000-0004-000000000001',
            '00000000-0000-0000-0004-000000000002'
        )",
    )
    .execute(pool)
    .await
    .expect("clean test auth links");

    sqlx::query(
        "DELETE FROM kb_profiles WHERE id NOT IN (
            '00000000-0000-0000-0004-000000000001',
            '00000000-0000-0000-0004-000000000002'
        )",
    )
    .execute(pool)
    .await
    .expect("clean test profiles");

    sqlx::query(
        r#"
        INSERT INTO kb_resources
            (id, kb_context_id, kb_doc_type_id, origin_uri, title, slug,
             originator_profile_id, owner_profile_id, is_active, created, updated)
        VALUES (
            '00000000-0000-0000-0099-000000000001',
            $1, $2,
            'test://seed-resource',
            'Seed Research Doc',
            'seed-research-doc',
            $3, $3,
            true, now(), now()
        )
        ON CONFLICT (id) DO UPDATE SET updated = now()
        "#,
    )
    .bind(uuid::Uuid::parse_str(TEMPER_CONTEXT_ID).unwrap())
    .bind(uuid::Uuid::parse_str(RESEARCH_DOC_TYPE_ID).unwrap())
    .bind(uuid::Uuid::parse_str(SYSTEM_PROFILE_ID).unwrap())
    .execute(pool)
    .await
    .expect("seed resource");
}

/// Build an `E2eTestApp` from a pool provided by `#[sqlx::test]`.
pub async fn setup(pool: PgPool) -> E2eTestApp {
    clean_and_seed(&pool).await;

    // --- Server setup ---
    let decoding_key =
        jsonwebtoken::DecodingKey::from_rsa_pem(include_bytes!("../fixtures/test_rsa.pub"))
            .expect("Failed to load test RSA public key");
    let jwks_store = JwksKeyStore::with_static_key(decoding_key);

    let api_config = ApiConfig {
        database_url: "unused".to_string(),
        jwks_url: "unused".to_string(),
        auth_issuer: "test-issuer".to_string(),
        auth_audience: None,
        auth_provider_name: "test-provider".to_string(),
        cors_origins: vec![],
        port: 0,
        enable_swagger: false,
    };

    let state = AppState::new(pool.clone(), jwks_store, api_config);
    let app = create_app(state);

    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("Failed to bind test listener");
    let addr = listener.local_addr().expect("Failed to get local addr");

    tokio::spawn(async move {
        axum::serve(listener, app)
            .await
            .expect("Test server failed");
    });

    // --- Config + client setup (no disk reads) ---
    let token = generate_test_jwt("e2e-test-user", "e2e@test.example.com");

    let vault_dir = TempDir::new().expect("Failed to create temp vault");
    std::fs::create_dir_all(vault_dir.path().join(".temper"))
        .expect("Failed to create .temper dir");

    let temper_config = TemperConfig {
        vault: CloudVaultConfig {
            path: vault_dir.path().to_str().unwrap().to_string(),
        },
        cloud: CloudSection {
            api_url: format!("http://{addr}"),
        },
        ..TemperConfig::default()
    };

    let stored_auth = StoredAuth {
        provider: "test".to_string(),
        access_token: token.clone(),
        refresh_token: None,
        expires_at: Utc::now() + Duration::hours(1),
        profile_id: None,
        device_id: Some("e2e-test-device".to_string()),
    };

    let client = temper_client::config::build_client_from(&temper_config, Some(&stored_auth))
        .expect("Failed to build test client");

    let cli_config = temper_cli::config::load_from(&temper_config, None);

    E2eTestApp {
        addr,
        pool,
        client,
        reqwest_client: reqwest::Client::new(),
        config: temper_config,
        cli_config,
        token,
        vault_dir,
    }
}
