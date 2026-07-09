#![allow(dead_code)]
//! Shared test helpers for temper-api integration tests.
//!
//! Provides `TestApp` — a running server bound to a random port, backed by
//! an isolated per-test database (via `#[sqlx::test]`) — and JWT generation
//! utilities signed with the local RSA test key pair.

pub mod fixtures;

use std::net::SocketAddr;

use jsonwebtoken::{Algorithm, EncodingKey, Header};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use tokio::net::TcpListener;

use temper_api::create_app;
use temper_services::{
    config::ApiConfig,
    state::{AppState, JwksKeyStore},
};
use uuid::Uuid;

/// Seeds a bare profile with no access to anything. Returns its id.
///
/// A random UUIDv7 handle keeps concurrent `#[sqlx::test]` databases collision-free even
/// though each already runs against its own isolated schema.
pub async fn seed_profile(pool: &PgPool, handle: &str) -> Uuid {
    let id = Uuid::now_v7();
    let unique = format!("{handle}-{id}");
    sqlx::query(
        "INSERT INTO kb_profiles (id, handle, display_name, email) VALUES ($1, $2, $3, $4)",
    )
    .bind(id)
    .bind(&unique)
    .bind(handle)
    .bind(format!("{unique}@test.com"))
    .execute(pool)
    .await
    .expect("insert profile");
    id
}

/// Seeds a profile-owned context holding one `goal` and `n` `task` resources, each linked
/// `goal --parent_of--> task` (edge_kind `contains`, homed in the context). Returns
/// `(profile_id, context_id, goal_id)`.
///
/// A profile-owned context makes every seeded resource visible to the profile (via
/// `kb_resource_homes.owner_profile_id`) AND makes the context's edges readable (via
/// `anchor_readable_by_profile`'s personal-context clause), so the seed satisfies the full
/// canonical edge-visibility predicate. Mirrors `scripts/seed-graph-fixtures.sql` but built
/// programmatically so tests can vary `n`.
pub async fn seed_context_with_goal_and_tasks(pool: &PgPool, n: usize) -> (Uuid, Uuid, Uuid) {
    let profile = seed_profile(pool, "owner").await;

    // Emitter entity + one genesis event, reused as every asserted_by/last FK.
    let entity = Uuid::now_v7();
    sqlx::query("INSERT INTO kb_entities (id, profile_id, name) VALUES ($1, $2, $3)")
        .bind(entity)
        .bind(profile)
        .bind("owner@web")
        .execute(pool)
        .await
        .expect("insert entity");

    let ctx = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO kb_contexts (id, owner_table, owner_id, slug, name) \
         VALUES ($1, 'kb_profiles', $2, $3, $3)",
    )
    .bind(ctx)
    .bind(profile)
    .bind(format!("ctx-{ctx}"))
    .execute(pool)
    .await
    .expect("insert context");

    let event = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO kb_events \
             (id, event_type_id, emitter_entity_id, producing_anchor_table, producing_anchor_id) \
         SELECT $1, (SELECT id FROM kb_event_types WHERE name = 'relationship_asserted'), \
                $2, 'kb_contexts', $3",
    )
    .bind(event)
    .bind(entity)
    .bind(ctx)
    .execute(pool)
    .await
    .expect("insert genesis event");

    // One shared closure for the resource + home + doc_type property triple.
    let goal = seed_resource(pool, ctx, profile, event, "Goal", "goal").await;

    for i in 0..n {
        let task = seed_resource(pool, ctx, profile, event, &format!("Task {i}"), "task").await;
        // goal --parent_of--> task : the historical containment spine (contains, forward),
        // homed in the context. The backfill (20260709000005) rewrites this to advances.
        sqlx::query(
            "INSERT INTO kb_edges \
                 (source_table, source_id, target_table, target_id, edge_kind, polarity, label, \
                  home_anchor_table, home_anchor_id, asserted_by_event_id, last_event_id) \
             VALUES ('kb_resources', $1, 'kb_resources', $2, 'contains', 'forward', 'parent_of', \
                     'kb_contexts', $3, $4, $4)",
        )
        .bind(goal)
        .bind(task)
        .bind(ctx)
        .bind(event)
        .execute(pool)
        .await
        .expect("insert parent_of edge");
    }

    (profile, ctx, goal)
}

/// Insert one resource homed in `ctx` (owned + originated by `profile`) carrying a
/// `doc_type` property. Returns the resource id.
async fn seed_resource(
    pool: &PgPool,
    ctx: Uuid,
    profile: Uuid,
    event: Uuid,
    title: &str,
    doc_type: &str,
) -> Uuid {
    let id = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO kb_resources (id, title, origin_uri, is_active) VALUES ($1, $2, $3, true)",
    )
    .bind(id)
    .bind(title)
    .bind(format!("test://{id}"))
    .execute(pool)
    .await
    .expect("insert resource");
    sqlx::query(
        "INSERT INTO kb_resource_homes \
             (resource_id, anchor_table, anchor_id, originator_profile_id, owner_profile_id) \
         VALUES ($1, 'kb_contexts', $2, $3, $3)",
    )
    .bind(id)
    .bind(ctx)
    .bind(profile)
    .execute(pool)
    .await
    .expect("insert home");
    sqlx::query(
        "INSERT INTO kb_properties \
             (owner_table, owner_id, property_key, property_value, asserted_by_event_id, last_event_id) \
         VALUES ('kb_resources', $1, 'doc_type', to_jsonb($2::text), $3, $3)",
    )
    .bind(id)
    .bind(doc_type)
    .bind(event)
    .execute(pool)
    .await
    .expect("insert doc_type property");
    id
}

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

/// Build a `TestApp` from a pool provided by `#[sqlx::test]`.
///
/// The pool already points at an isolated per-test database with migrations
/// applied. We seed fixtures and start the Axum server on a random port.
pub async fn setup_test_app(pool: PgPool) -> TestApp {
    // Seed test data into the isolated database.
    fixtures::clean_and_seed(&pool).await;

    // Build AppState with a static test key.
    let decoding_key = jsonwebtoken::DecodingKey::from_rsa_pem(include_bytes!("test_rsa.pub"))
        .expect("Failed to load test RSA public key");
    let jwks_store = JwksKeyStore::with_static_key(decoding_key, Algorithm::RS256);

    let config = ApiConfig {
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

/// Like [`setup_test_app`] but lets the caller mutate the `ApiConfig` before the app is built
/// (e.g. to set `internal_reconcile_secret` / `auth_provider_name` for a specific test).
pub async fn setup_test_app_with_config(
    pool: PgPool,
    configure: impl FnOnce(&mut ApiConfig),
) -> TestApp {
    fixtures::clean_and_seed(&pool).await;

    let decoding_key = jsonwebtoken::DecodingKey::from_rsa_pem(include_bytes!("test_rsa.pub"))
        .expect("Failed to load test RSA public key");
    let jwks_store = JwksKeyStore::with_static_key(decoding_key, Algorithm::RS256);

    let mut config = ApiConfig {
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
    configure(&mut config);

    let state = AppState::new(pool.clone(), jwks_store, config);
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

    TestApp {
        addr,
        pool,
        client: reqwest::Client::new(),
    }
}
