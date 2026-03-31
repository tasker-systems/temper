#![allow(dead_code)]
//! Shared test helpers for temper-api integration tests.
//!
//! Provides `TestApp` — a running server bound to a random port, backed by
//! the `temper_test` Docker Postgres database — and JWT generation utilities
//! signed with the local Ed25519 test key pair.

pub mod fixtures;

use std::net::SocketAddr;

use jsonwebtoken::{Algorithm, EncodingKey, Header};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use tokio::net::TcpListener;

use temper_api::{
    config::ApiConfig,
    create_app,
    state::{AppState, JwksKeyStore},
};

/// A live test server with its backing pool and HTTP client.
pub struct TestApp {
    pub addr: SocketAddr,
    pub pool: PgPool,
    pub client: reqwest::Client,
}

impl TestApp {
    /// Base URL for the running server (e.g. `http://127.0.0.1:54321`).
    pub fn base_url(&self) -> String {
        format!("http://{}", self.addr)
    }

    /// Construct a full URL for the given path (e.g. `/api/health`).
    pub fn url(&self, path: &str) -> String {
        format!("{}{}", self.base_url(), path)
    }
}

/// Claim shape used for test JWT encoding.
#[derive(Debug, Serialize, Deserialize)]
struct TestClaims {
    sub: String,
    email: String,
    email_verified: bool,
    iss: String,
    iat: i64,
    exp: i64,
}

/// Sign a JWT with the test RSA private key (matches Auth0 RS256 production flow).
///
/// The token is valid for 1 hour from `now`, issued by `"test-issuer"`.
pub fn generate_test_jwt(sub: &str, email: &str) -> String {
    let encoding_key = EncodingKey::from_rsa_pem(include_bytes!("test_rsa.key"))
        .expect("Failed to load test RSA private key");

    let now = chrono::Utc::now().timestamp();
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

/// Sign a JWT that expired 1 hour ago.
pub fn generate_expired_jwt(sub: &str, email: &str) -> String {
    let encoding_key = EncodingKey::from_rsa_pem(include_bytes!("test_rsa.key"))
        .expect("Failed to load test RSA private key");

    let now = chrono::Utc::now().timestamp();
    let claims = TestClaims {
        sub: sub.to_string(),
        email: email.to_string(),
        email_verified: true,
        iss: "test-issuer".to_string(),
        iat: now - 7200,
        exp: now - 3600,
    };

    jsonwebtoken::encode(&Header::new(Algorithm::RS256), &claims, &encoding_key)
        .expect("Failed to sign expired test JWT")
}

/// Create a pool, run migrations, seed test data, start the server on a
/// random port, and return a `TestApp` ready for requests.
pub async fn setup_test_app() -> TestApp {
    let database_url = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgresql://temper:temper@localhost:5437/temper_test".to_string());

    // Build pool and run migrations.
    let pool = PgPool::connect(&database_url)
        .await
        .expect("Failed to connect to test database");

    sqlx::migrate!("../../migrations")
        .run(&pool)
        .await
        .expect("Failed to run migrations");

    // Seed / clean test data.
    fixtures::clean_and_seed(&pool).await;

    // Build AppState with a static test key.
    let decoding_key = jsonwebtoken::DecodingKey::from_rsa_pem(include_bytes!("test_rsa.pub"))
        .expect("Failed to load test RSA public key");
    let jwks_store = JwksKeyStore::with_static_key(decoding_key);

    let config = ApiConfig {
        database_url: database_url.clone(),
        jwks_url: "unused".to_string(),
        auth_issuer: "test-issuer".to_string(),
        auth_audience: None,
        auth_provider_name: "test-provider".to_string(),
        cors_origins: vec![],
        port: 0,
        enable_swagger: false,
    };

    let state = AppState::new(pool.clone(), jwks_store, config);
    let app = create_app(state);

    // Bind to any available port.
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("Failed to bind test listener");
    let addr = listener.local_addr().expect("Failed to get local addr");

    tokio::spawn(async move {
        axum::serve(listener, app)
            .await
            .expect("Test server failed");
    });

    TestApp {
        addr,
        pool,
        client: reqwest::Client::new(),
    }
}
