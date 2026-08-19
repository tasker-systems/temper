# I3: temper-api — Axum Server Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build the temper-api crate as a working axum HTTP server with JWT auth, resource CRUD, profiles, events, search, and OpenAPI docs.

**Architecture:** Thin handlers / fat services pattern. `create_app(state) -> Router` factory is the composable entry point. Auth middleware resolves JWT → profile via database lookup with auto-provisioning. All resource queries compose with `resources_visible_to()` SQL function for access control. Tests run against local Docker Postgres behind `test-db` feature gate.

**Tech Stack:** Rust, axum 0.8, sqlx 0.8, jsonwebtoken 9, utoipa 5, tower-http 0.6, reqwest 0.12

---

## Task Decomposition

| Task | What | Depends On |
|------|------|------------|
| 1 | Cargo.toml, config, error types, AppState | — |
| 2 | JWKS key store | Task 1 |
| 3 | Auth middleware + ProfileService | Task 2 |
| 4 | ResourceService + handlers | Task 3 |
| 5 | ProfileService handlers, EventService, SearchService | Task 3 |
| 6 | Routes, OpenAPI, main.rs | Tasks 4-5 |
| 7 | Test infrastructure + integration tests | Task 6 |
| 8 | temper-core web-api feature gate for utoipa | Task 6 |

---

### Task 1: Foundation — Cargo.toml, Config, Error, AppState

**Files:**
- Modify: `crates/temper-api/Cargo.toml`
- Create: `crates/temper-api/src/config.rs`
- Create: `crates/temper-api/src/error.rs`
- Create: `crates/temper-api/src/state.rs`
- Modify: `crates/temper-api/src/lib.rs`
- Create: `crates/temper-api/.env.template`

- [ ] **Step 1: Update temper-api Cargo.toml with full dependencies**

Replace the contents of `crates/temper-api/Cargo.toml`:

```toml
[package]
name = "temper-api"
version = "0.1.0"
edition = "2021"
description = "Axum HTTP server implementing the temper cloud API"

[[bin]]
name = "temper-api"
path = "src/main.rs"

[dependencies]
temper-core = { path = "../temper-core" }
axum = { version = "0.8", features = ["json", "query"] }
sqlx = { version = "0.8", features = ["postgres", "runtime-tokio-rustls", "chrono", "json", "uuid", "macros", "migrate"] }
tokio = { version = "1", features = ["full"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
jsonwebtoken = "9"
reqwest = { version = "0.12", default-features = false, features = ["rustls-tls", "json"] }
tower-http = { version = "0.6", features = ["cors", "trace"] }
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter"] }
uuid = { version = "1", features = ["v7", "serde"] }
chrono = { version = "0.4", features = ["serde"] }
thiserror = "2"

[features]
test-db = []

[dev-dependencies]
tempfile = "3"
```

Note: utoipa is deferred to Task 8 to keep the initial build cycle fast.

- [ ] **Step 2: Create .env.template**

Create `crates/temper-api/.env.template`:

```
DATABASE_URL=postgresql://temper:temper@localhost:5437/temper_test
JWKS_URL=http://localhost:9999/.well-known/jwks.json
AUTH_ISSUER=https://neonauth.example.com
AUTH_AUDIENCE=
CORS_ORIGINS=http://localhost:3000,http://localhost:5173
PORT=3000
```

- [ ] **Step 3: Create config.rs**

Create `crates/temper-api/src/config.rs`:

```rust
use std::env;

#[derive(Debug, Clone)]
pub struct ApiConfig {
    pub database_url: String,
    pub jwks_url: String,
    pub auth_issuer: String,
    pub auth_audience: Option<String>,
    pub cors_origins: Vec<String>,
    pub port: u16,
}

impl ApiConfig {
    pub fn from_env() -> Result<Self, env::VarError> {
        let cors_origins = env::var("CORS_ORIGINS")
            .unwrap_or_default()
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();

        Ok(Self {
            database_url: env::var("DATABASE_URL")?,
            jwks_url: env::var("JWKS_URL")?,
            auth_issuer: env::var("AUTH_ISSUER")?,
            auth_audience: env::var("AUTH_AUDIENCE").ok().filter(|s| !s.is_empty()),
            cors_origins,
            port: env::var("PORT")
                .ok()
                .and_then(|p| p.parse().ok())
                .unwrap_or(3000),
        })
    }
}
```

- [ ] **Step 4: Create error.rs**

Create `crates/temper-api/src/error.rs`:

```rust
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::Serialize;

#[derive(Debug, thiserror::Error)]
pub enum ApiError {
    #[error("Not found")]
    NotFound,
    #[error("Unauthorized: {0}")]
    Unauthorized(String),
    #[error("Forbidden")]
    Forbidden,
    #[error("Bad request: {0}")]
    BadRequest(String),
    #[error("Conflict: {0}")]
    Conflict(String),
    #[error("Internal error: {0}")]
    Internal(String),
}

pub type ApiResult<T> = Result<T, ApiError>;

#[derive(Serialize)]
struct ErrorBody {
    error: ErrorDetail,
}

#[derive(Serialize)]
struct ErrorDetail {
    code: &'static str,
    message: String,
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, code) = match &self {
            ApiError::NotFound => (StatusCode::NOT_FOUND, "NOT_FOUND"),
            ApiError::Unauthorized(_) => (StatusCode::UNAUTHORIZED, "UNAUTHORIZED"),
            ApiError::Forbidden => (StatusCode::FORBIDDEN, "FORBIDDEN"),
            ApiError::BadRequest(_) => (StatusCode::BAD_REQUEST, "BAD_REQUEST"),
            ApiError::Conflict(_) => (StatusCode::CONFLICT, "CONFLICT"),
            ApiError::Internal(_) => (StatusCode::INTERNAL_SERVER_ERROR, "INTERNAL_ERROR"),
        };
        let body = ErrorBody {
            error: ErrorDetail {
                code,
                message: self.to_string(),
            },
        };
        (status, axum::Json(body)).into_response()
    }
}

impl From<sqlx::Error> for ApiError {
    fn from(err: sqlx::Error) -> Self {
        match &err {
            sqlx::Error::RowNotFound => ApiError::NotFound,
            sqlx::Error::Database(db_err) if db_err.code().as_deref() == Some("23505") => {
                ApiError::Conflict("Resource already exists".to_string())
            }
            _ => ApiError::Internal(format!("Database error: {err}")),
        }
    }
}

impl From<serde_json::Error> for ApiError {
    fn from(err: serde_json::Error) -> Self {
        ApiError::BadRequest(format!("Invalid JSON: {err}"))
    }
}
```

- [ ] **Step 5: Create state.rs**

Create `crates/temper-api/src/state.rs`:

```rust
use sqlx::PgPool;
use std::sync::Arc;

use crate::config::ApiConfig;

/// Placeholder for JWKS key store — implemented in Task 2.
#[derive(Debug, Clone)]
pub struct JwksKeyStore;

#[derive(Clone)]
pub struct AppState {
    pub pool: PgPool,
    pub jwks_store: Arc<JwksKeyStore>,
    pub config: Arc<ApiConfig>,
}

impl AppState {
    pub fn new(pool: PgPool, jwks_store: JwksKeyStore, config: ApiConfig) -> Self {
        Self {
            pool,
            jwks_store: Arc::new(jwks_store),
            config: Arc::new(config),
        }
    }
}
```

- [ ] **Step 6: Update lib.rs with module declarations**

Replace `crates/temper-api/src/lib.rs`:

```rust
//! temper-api — Axum HTTP server implementing the temper cloud API.
//!
//! Platform-agnostic: runs locally via `cargo run` or wrapped by temper-cloud
//! for Vercel deployment. Exports `create_app(state) -> Router` for composition.

pub mod config;
pub mod error;
pub mod state;
```

- [ ] **Step 7: Create a minimal main.rs**

Create `crates/temper-api/src/main.rs`:

```rust
fn main() {
    println!("temper-api — not yet runnable, see Task 6");
}
```

- [ ] **Step 8: Verify compilation**

Run: `cargo check -p temper-api 2>&1 | tail -5`
Expected: compiles successfully.

- [ ] **Step 9: Commit**

```bash
git add crates/temper-api/
git commit -m "feat(api): foundation — Cargo.toml, config, error types, AppState"
```

---

### Task 2: JWKS Key Store

**Files:**
- Modify: `crates/temper-api/src/state.rs`

Replace the JwksKeyStore placeholder with a real implementation that fetches and caches EdDSA public keys.

- [ ] **Step 1: Implement JwksKeyStore**

Replace the full contents of `crates/temper-api/src/state.rs`:

```rust
use jsonwebtoken::{DecodingKey, Validation, Algorithm};
use reqwest::Client;
use serde::Deserialize;
use sqlx::PgPool;
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

use crate::config::ApiConfig;

#[derive(Debug, Deserialize)]
struct JwksResponse {
    keys: Vec<JwkKey>,
}

#[derive(Debug, Deserialize)]
struct JwkKey {
    kty: String,
    crv: Option<String>,
    x: Option<String>,
    kid: Option<String>,
    alg: Option<String>,
}

#[derive(Debug)]
struct CachedKeys {
    keys: Vec<(Option<String>, DecodingKey)>,
    fetched_at: Instant,
}

#[derive(Debug)]
pub struct JwksKeyStore {
    url: String,
    client: Client,
    cache: RwLock<Option<CachedKeys>>,
    ttl: Duration,
}

impl JwksKeyStore {
    pub fn new(url: String) -> Self {
        Self {
            url,
            client: Client::new(),
            cache: RwLock::new(None),
            ttl: Duration::from_secs(3600),
        }
    }

    /// Create a key store with a pre-loaded key (for testing).
    pub fn with_static_key(key: DecodingKey) -> Self {
        let store = Self {
            url: String::new(),
            client: Client::new(),
            cache: RwLock::new(None),
            ttl: Duration::from_secs(86400),
        };
        let cached = CachedKeys {
            keys: vec![(None, key)],
            fetched_at: Instant::now(),
        };
        *store.cache.write().expect("lock poisoned") = Some(cached);
        store
    }

    pub async fn get_decoding_key(&self) -> Result<DecodingKey, String> {
        // Check cache
        {
            let cache = self.cache.read().expect("lock poisoned");
            if let Some(ref cached) = *cache {
                if cached.fetched_at.elapsed() < self.ttl {
                    if let Some((_, key)) = cached.keys.first() {
                        return Ok(key.clone());
                    }
                }
            }
        }

        // Fetch fresh
        self.refresh().await?;

        let cache = self.cache.read().expect("lock poisoned");
        cache
            .as_ref()
            .and_then(|c| c.keys.first())
            .map(|(_, key)| key.clone())
            .ok_or_else(|| "No keys available after refresh".to_string())
    }

    async fn refresh(&self) -> Result<(), String> {
        let resp: JwksResponse = self
            .client
            .get(&self.url)
            .send()
            .await
            .map_err(|e| format!("JWKS fetch failed: {e}"))?
            .json()
            .await
            .map_err(|e| format!("JWKS parse failed: {e}"))?;

        let mut keys = Vec::new();
        for jwk in &resp.keys {
            if jwk.kty == "OKP" && jwk.crv.as_deref() == Some("Ed25519") {
                if let Some(ref x) = jwk.x {
                    match DecodingKey::from_ed_der(
                        &base64_url_decode(x).map_err(|e| format!("base64 decode: {e}"))?,
                    ) {
                        key => keys.push((jwk.kid.clone(), key)),
                    }
                }
            }
        }

        let cached = CachedKeys {
            keys,
            fetched_at: Instant::now(),
        };
        *self.cache.write().expect("lock poisoned") = Some(cached);
        Ok(())
    }

    pub fn validation(&self, issuer: &str, audience: Option<&str>) -> Validation {
        let mut v = Validation::new(Algorithm::EdDSA);
        v.set_issuer(&[issuer]);
        if let Some(aud) = audience {
            v.set_audience(&[aud]);
        } else {
            v.validate_aud = false;
        }
        v
    }
}

fn base64_url_decode(input: &str) -> Result<Vec<u8>, String> {
    use jsonwebtoken::decode_header;
    // Use a simple base64url decoder
    let padded = match input.len() % 4 {
        2 => format!("{input}=="),
        3 => format!("{input}="),
        _ => input.to_string(),
    };
    let standard = padded.replace('-', "+").replace('_', "/");
    general_purpose_decode(&standard).map_err(|e| format!("base64 error: {e}"))
}

fn general_purpose_decode(input: &str) -> Result<Vec<u8>, String> {
    // Inline base64 decode without adding a dependency
    // jsonwebtoken already depends on base64, but we access it simply
    let bytes: Vec<u8> = input
        .bytes()
        .filter(|&b| b != b'=')
        .map(|b| match b {
            b'A'..=b'Z' => b - b'A',
            b'a'..=b'z' => b - b'a' + 26,
            b'0'..=b'9' => b - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            _ => 255,
        })
        .collect::<Vec<_>>()
        .chunks(4)
        .flat_map(|chunk| {
            let mut buf = [0u8; 3];
            let len = chunk.len();
            if len >= 2 {
                buf[0] = (chunk[0] << 2) | (chunk[1] >> 4);
            }
            if len >= 3 {
                buf[1] = (chunk[1] << 4) | (chunk[2] >> 2);
            }
            if len >= 4 {
                buf[2] = (chunk[2] << 6) | chunk[3];
            }
            buf[..len.saturating_sub(1)].to_vec()
        })
        .collect();
    Ok(bytes)
}

impl Clone for JwksKeyStore {
    fn clone(&self) -> Self {
        // Cloning shares the same cache via Arc in AppState
        // This is only called during AppState construction
        Self {
            url: self.url.clone(),
            client: Client::new(),
            cache: RwLock::new(None),
            ttl: self.ttl,
        }
    }
}

#[derive(Clone)]
pub struct AppState {
    pub pool: PgPool,
    pub jwks_store: Arc<JwksKeyStore>,
    pub config: Arc<ApiConfig>,
}

impl AppState {
    pub fn new(pool: PgPool, jwks_store: JwksKeyStore, config: ApiConfig) -> Self {
        Self {
            pool,
            jwks_store: Arc::new(jwks_store),
            config: Arc::new(config),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_base64_url_decode() {
        // "hello" in base64url
        let result = base64_url_decode("aGVsbG8").unwrap();
        assert_eq!(result, b"hello");
    }

    #[test]
    fn test_validation_with_audience() {
        let store = JwksKeyStore::new("http://example.com".to_string());
        let v = store.validation("issuer", Some("audience"));
        assert_eq!(v.algorithms, vec![Algorithm::EdDSA]);
    }

    #[test]
    fn test_validation_without_audience() {
        let store = JwksKeyStore::new("http://example.com".to_string());
        let v = store.validation("issuer", None);
        assert!(!v.validate_aud);
    }
}
```

- [ ] **Step 2: Verify compilation**

Run: `cargo check -p temper-api 2>&1 | tail -5`
Expected: compiles.

- [ ] **Step 3: Run unit tests**

Run: `cargo test -p temper-api 2>&1 | tail -10`
Expected: 3 tests pass.

- [ ] **Step 4: Commit**

```bash
git add crates/temper-api/src/state.rs
git commit -m "feat(api): JWKS key store — EdDSA key fetching with cache and TTL"
```

---

### Task 3: Auth Middleware + ProfileService

**Files:**
- Create: `crates/temper-api/src/middleware/mod.rs`
- Create: `crates/temper-api/src/middleware/auth.rs`
- Create: `crates/temper-api/src/services/mod.rs`
- Create: `crates/temper-api/src/services/profile_service.rs`
- Modify: `crates/temper-api/src/lib.rs`

- [ ] **Step 1: Create ProfileService**

Create `crates/temper-api/src/services/mod.rs`:

```rust
pub mod profile_service;
```

Create `crates/temper-api/src/services/profile_service.rs`:

```rust
use sqlx::PgPool;
use uuid::Uuid;

use temper_core::types::{AuthClaims, Profile, ProfileAuthLink};

use crate::error::{ApiError, ApiResult};

pub struct ProfileService;

impl ProfileService {
    /// Look up or auto-provision a profile from JWT claims.
    pub async fn resolve_from_claims(pool: &PgPool, claims: &AuthClaims) -> ApiResult<Profile> {
        // Try direct lookup by auth provider + external user ID
        let existing = sqlx::query_as::<_, ProfileAuthLink>(
            "SELECT * FROM kb_profile_auth_links WHERE auth_provider = $1 AND auth_provider_user_id = $2"
        )
        .bind(&claims.provider)
        .bind(&claims.external_user_id)
        .fetch_optional(pool)
        .await?;

        if let Some(link) = existing {
            return Self::get_by_id(pool, link.profile_id).await;
        }

        // Email reconciliation: check if another provider linked with same email
        let email_link = sqlx::query_as::<_, ProfileAuthLink>(
            "SELECT * FROM kb_profile_auth_links WHERE email = $1 LIMIT 1"
        )
        .bind(&claims.email)
        .fetch_optional(pool)
        .await?;

        if let Some(link) = email_link {
            // Link this new provider to the existing profile
            let link_id = Uuid::now_v7();
            sqlx::query(
                "INSERT INTO kb_profile_auth_links (id, profile_id, auth_provider, auth_provider_user_id, email, is_default)
                 VALUES ($1, $2, $3, $4, $5, false)"
            )
            .bind(link_id)
            .bind(link.profile_id)
            .bind(&claims.provider)
            .bind(&claims.external_user_id)
            .bind(&claims.email)
            .execute(pool)
            .await?;

            return Self::get_by_id(pool, link.profile_id).await;
        }

        // No existing profile — create new
        let profile_id = Uuid::now_v7();
        let display_name = claims.email.split('@').next().unwrap_or("user").to_string();

        sqlx::query(
            "INSERT INTO kb_profiles (id, display_name, email) VALUES ($1, $2, $3)"
        )
        .bind(profile_id)
        .bind(&display_name)
        .bind(&claims.email)
        .execute(pool)
        .await?;

        let link_id = Uuid::now_v7();
        sqlx::query(
            "INSERT INTO kb_profile_auth_links (id, profile_id, auth_provider, auth_provider_user_id, email, is_default)
             VALUES ($1, $2, $3, $4, $5, true)"
        )
        .bind(link_id)
        .bind(profile_id)
        .bind(&claims.provider)
        .bind(&claims.external_user_id)
        .bind(&claims.email)
        .execute(pool)
        .await?;

        Self::get_by_id(pool, profile_id).await
    }

    pub async fn get_by_id(pool: &PgPool, id: Uuid) -> ApiResult<Profile> {
        sqlx::query_as::<_, Profile>("SELECT * FROM kb_profiles WHERE id = $1 AND is_active = true")
            .bind(id)
            .fetch_optional(pool)
            .await?
            .ok_or(ApiError::NotFound)
    }

    pub async fn update(
        pool: &PgPool,
        id: Uuid,
        display_name: Option<&str>,
        preferences: Option<&serde_json::Value>,
        vault_config: Option<&serde_json::Value>,
    ) -> ApiResult<Profile> {
        if let Some(name) = display_name {
            sqlx::query("UPDATE kb_profiles SET display_name = $1, updated = now() WHERE id = $2")
                .bind(name)
                .bind(id)
                .execute(pool)
                .await?;
        }
        if let Some(prefs) = preferences {
            sqlx::query("UPDATE kb_profiles SET preferences = $1, updated = now() WHERE id = $2")
                .bind(prefs)
                .bind(id)
                .execute(pool)
                .await?;
        }
        if let Some(vc) = vault_config {
            sqlx::query("UPDATE kb_profiles SET vault_config = $1, updated = now() WHERE id = $2")
                .bind(vc)
                .bind(id)
                .execute(pool)
                .await?;
        }
        Self::get_by_id(pool, id).await
    }

    pub async fn list_auth_links(pool: &PgPool, profile_id: Uuid) -> ApiResult<Vec<ProfileAuthLink>> {
        let links = sqlx::query_as::<_, ProfileAuthLink>(
            "SELECT * FROM kb_profile_auth_links WHERE profile_id = $1 ORDER BY is_default DESC, linked_at"
        )
        .bind(profile_id)
        .fetch_all(pool)
        .await?;
        Ok(links)
    }
}
```

- [ ] **Step 2: Create auth middleware**

Create `crates/temper-api/src/middleware/mod.rs`:

```rust
pub mod auth;
```

Create `crates/temper-api/src/middleware/auth.rs`:

```rust
use axum::extract::{FromRequestParts, Request, State};
use axum::http::request::Parts;
use axum::http::StatusCode;
use axum::middleware::Next;
use axum::response::Response;

use temper_core::types::{AuthClaims, AuthenticatedProfile, Profile};

use crate::error::ApiError;
use crate::services::profile_service::ProfileService;
use crate::state::AppState;

/// Middleware: verify JWT and resolve to AuthenticatedProfile.
pub async fn require_auth(
    State(state): State<AppState>,
    mut request: Request,
    next: Next,
) -> Result<Response, ApiError> {
    let token = extract_bearer_token(request.headers())
        .ok_or_else(|| ApiError::Unauthorized("Missing Authorization header".to_string()))?;

    let decoding_key = state
        .jwks_store
        .get_decoding_key()
        .await
        .map_err(|e| ApiError::Unauthorized(format!("JWKS error: {e}")))?;

    let validation = state
        .jwks_store
        .validation(&state.config.auth_issuer, state.config.auth_audience.as_deref());

    let token_data = jsonwebtoken::decode::<JwtClaims>(&token, &decoding_key, &validation)
        .map_err(|e| ApiError::Unauthorized(format!("Invalid token: {e}")))?;

    let claims = AuthClaims {
        provider: "neon_auth".to_string(),
        external_user_id: token_data.claims.sub,
        email: token_data.claims.email.unwrap_or_default(),
        exp: token_data.claims.exp,
        iat: token_data.claims.iat,
    };

    let profile = ProfileService::resolve_from_claims(&state.pool, &claims).await?;

    let client_id = request
        .headers()
        .get("X-Temper-Client-Id")
        .and_then(|v| v.to_str().ok())
        .map(String::from);

    let authenticated = AuthenticatedProfile {
        profile,
        claims,
    };

    request.extensions_mut().insert(authenticated);
    if let Some(cid) = client_id {
        request.extensions_mut().insert(ClientId(cid));
    }

    Ok(next.run(request).await)
}

/// Newtype for client ID extracted from X-Temper-Client-Id header.
#[derive(Debug, Clone)]
pub struct ClientId(pub String);

/// JWT claims shape from Neon Auth.
#[derive(Debug, serde::Deserialize)]
struct JwtClaims {
    sub: String,
    email: Option<String>,
    exp: i64,
    iat: i64,
}

/// Extract AuthenticatedProfile from request extensions.
impl<S: Send + Sync> FromRequestParts<S> for AuthenticatedProfile {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        parts
            .extensions
            .get::<AuthenticatedProfile>()
            .cloned()
            .ok_or_else(|| ApiError::Unauthorized("Not authenticated".to_string()))
    }
}

fn extract_bearer_token(headers: &axum::http::HeaderMap) -> Option<String> {
    headers
        .get(axum::http::header::AUTHORIZATION)?
        .to_str()
        .ok()?
        .strip_prefix("Bearer ")
        .map(String::from)
}
```

- [ ] **Step 3: Update lib.rs**

Replace `crates/temper-api/src/lib.rs`:

```rust
//! temper-api — Axum HTTP server implementing the temper cloud API.
//!
//! Platform-agnostic: runs locally via `cargo run` or wrapped by temper-cloud
//! for Vercel deployment. Exports `create_app(state) -> Router` for composition.

pub mod config;
pub mod error;
pub mod middleware;
pub mod services;
pub mod state;
```

- [ ] **Step 4: Verify compilation**

Run: `cargo check -p temper-api 2>&1 | tail -10`
Expected: compiles. The `AuthenticatedProfile` from temper-core needs `Clone` (it already has it).

- [ ] **Step 5: Commit**

```bash
git add crates/temper-api/src/
git commit -m "feat(api): auth middleware + profile service — JWT verify, auto-provision, email reconciliation"
```

---

### Task 4: ResourceService + Handlers

**Files:**
- Create: `crates/temper-api/src/services/resource_service.rs`
- Create: `crates/temper-api/src/handlers/mod.rs`
- Create: `crates/temper-api/src/handlers/health.rs`
- Create: `crates/temper-api/src/handlers/resources.rs`
- Modify: `crates/temper-api/src/services/mod.rs`
- Modify: `crates/temper-api/src/lib.rs`

- [ ] **Step 1: Create ResourceService**

Create `crates/temper-api/src/services/resource_service.rs`:

```rust
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::{FromRow, PgPool};
use uuid::Uuid;

use crate::error::{ApiError, ApiResult};

#[derive(Debug, Serialize, FromRow)]
pub struct ResourceRow {
    pub id: Uuid,
    pub kb_context_id: Uuid,
    pub kb_doc_type_id: Uuid,
    pub uri: String,
    pub title: String,
    pub slug: Option<String>,
    pub content_hash: Option<String>,
    pub mimetype: Option<String>,
    pub originator_profile_id: Uuid,
    pub owner_profile_id: Uuid,
    pub is_active: bool,
    pub created: DateTime<Utc>,
    pub updated: DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
pub struct ResourceListParams {
    pub context: Option<String>,
    pub doc_type: Option<String>,
    pub limit: Option<i64>,
    pub offset: Option<i64>,
}

#[derive(Debug, Deserialize)]
pub struct ResourceCreateRequest {
    pub title: String,
    pub context: String,
    pub doc_type: String,
    pub uri: Option<String>,
    pub slug: Option<String>,
    pub content: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ResourceUpdateRequest {
    pub title: Option<String>,
}

pub struct ResourceService;

impl ResourceService {
    pub async fn list_visible(
        pool: &PgPool,
        profile_id: Uuid,
        params: ResourceListParams,
    ) -> ApiResult<Vec<ResourceRow>> {
        let limit = params.limit.unwrap_or(50).min(100);
        let offset = params.offset.unwrap_or(0);

        let rows = sqlx::query_as::<_, ResourceRow>(
            r#"
            WITH visible AS (
                SELECT resource_id FROM resources_visible_to($1)
            )
            SELECT r.*
            FROM resources r
            JOIN visible v ON v.resource_id = r.id
            WHERE r.is_active = true
              AND ($2::text IS NULL OR r.kb_context_id = (SELECT id FROM kb_contexts WHERE name = $2))
              AND ($3::text IS NULL OR r.kb_doc_type_id = (SELECT id FROM kb_doc_types WHERE name = $3))
            ORDER BY r.updated DESC
            LIMIT $4 OFFSET $5
            "#,
        )
        .bind(profile_id)
        .bind(&params.context)
        .bind(&params.doc_type)
        .bind(limit)
        .bind(offset)
        .fetch_all(pool)
        .await?;

        Ok(rows)
    }

    pub async fn get_visible(
        pool: &PgPool,
        profile_id: Uuid,
        resource_id: Uuid,
    ) -> ApiResult<ResourceRow> {
        sqlx::query_as::<_, ResourceRow>(
            r#"
            WITH visible AS (
                SELECT resource_id FROM resources_visible_to($1)
            )
            SELECT r.*
            FROM resources r
            JOIN visible v ON v.resource_id = r.id
            WHERE r.id = $2 AND r.is_active = true
            "#,
        )
        .bind(profile_id)
        .bind(resource_id)
        .fetch_optional(pool)
        .await?
        .ok_or(ApiError::NotFound)
    }

    pub async fn get_content(
        pool: &PgPool,
        profile_id: Uuid,
        resource_id: Uuid,
    ) -> ApiResult<String> {
        // First verify access
        Self::get_visible(pool, profile_id, resource_id).await?;

        // Reconstitute markdown from current chunks
        let chunks: Vec<(String,)> = sqlx::query_as(
            "SELECT content FROM kb_current_chunks WHERE resource_id = $1 ORDER BY chunk_index"
        )
        .bind(resource_id)
        .fetch_all(pool)
        .await?;

        Ok(chunks.into_iter().map(|(c,)| c).collect::<Vec<_>>().join("\n\n"))
    }

    pub async fn create(
        pool: &PgPool,
        profile_id: Uuid,
        request: ResourceCreateRequest,
    ) -> ApiResult<ResourceRow> {
        let id = Uuid::now_v7();
        let now = Utc::now();
        let uri = request
            .uri
            .unwrap_or_else(|| format!("kb://{}/{}/{}", request.context, request.doc_type, id));

        let context_id: (Uuid,) = sqlx::query_as("SELECT id FROM kb_contexts WHERE name = $1")
            .bind(&request.context)
            .fetch_optional(pool)
            .await?
            .ok_or_else(|| ApiError::BadRequest(format!("Unknown context: {}", request.context)))?;

        let doc_type_id: (Uuid,) = sqlx::query_as("SELECT id FROM kb_doc_types WHERE name = $1")
            .bind(&request.doc_type)
            .fetch_optional(pool)
            .await?
            .ok_or_else(|| ApiError::BadRequest(format!("Unknown doc_type: {}", request.doc_type)))?;

        sqlx::query(
            r#"
            INSERT INTO resources (id, kb_context_id, kb_doc_type_id, uri, title, slug,
                                   originator_profile_id, owner_profile_id, created, updated)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $7, $8, $8)
            "#,
        )
        .bind(id)
        .bind(context_id.0)
        .bind(doc_type_id.0)
        .bind(&uri)
        .bind(&request.title)
        .bind(&request.slug)
        .bind(profile_id)
        .bind(now)
        .execute(pool)
        .await?;

        Self::get_visible(pool, profile_id, id).await
    }

    pub async fn update(
        pool: &PgPool,
        profile_id: Uuid,
        resource_id: Uuid,
        request: ResourceUpdateRequest,
    ) -> ApiResult<ResourceRow> {
        // Check modify permission
        let can_modify: (bool,) =
            sqlx::query_as("SELECT can_modify_resource($1, $2)")
                .bind(profile_id)
                .bind(resource_id)
                .fetch_one(pool)
                .await?;

        if !can_modify.0 {
            return Err(ApiError::Forbidden);
        }

        if let Some(ref title) = request.title {
            sqlx::query("UPDATE resources SET title = $1, updated = now() WHERE id = $2")
                .bind(title)
                .bind(resource_id)
                .execute(pool)
                .await?;
        }

        Self::get_visible(pool, profile_id, resource_id).await
    }

    pub async fn delete(
        pool: &PgPool,
        profile_id: Uuid,
        resource_id: Uuid,
    ) -> ApiResult<()> {
        let can_modify: (bool,) =
            sqlx::query_as("SELECT can_modify_resource($1, $2)")
                .bind(profile_id)
                .bind(resource_id)
                .fetch_one(pool)
                .await?;

        if !can_modify.0 {
            return Err(ApiError::Forbidden);
        }

        sqlx::query("UPDATE resources SET is_active = false, updated = now() WHERE id = $1")
            .bind(resource_id)
            .execute(pool)
            .await?;

        Ok(())
    }
}
```

- [ ] **Step 2: Update services/mod.rs**

Replace `crates/temper-api/src/services/mod.rs`:

```rust
pub mod profile_service;
pub mod resource_service;
```

- [ ] **Step 3: Create handlers/mod.rs**

Create `crates/temper-api/src/handlers/mod.rs`:

```rust
pub mod health;
pub mod resources;
```

- [ ] **Step 4: Create health handler**

Create `crates/temper-api/src/handlers/health.rs`:

```rust
use axum::Json;
use serde::Serialize;

#[derive(Serialize)]
pub struct HealthResponse {
    pub status: &'static str,
    pub version: &'static str,
}

pub async fn health_check() -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "ok",
        version: env!("CARGO_PKG_VERSION"),
    })
}
```

- [ ] **Step 5: Create resource handlers**

Create `crates/temper-api/src/handlers/resources.rs`:

```rust
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::Json;
use serde::Serialize;
use uuid::Uuid;

use temper_core::types::AuthenticatedProfile;

use crate::error::{ApiError, ApiResult};
use crate::services::resource_service::{
    ResourceCreateRequest, ResourceListParams, ResourceRow, ResourceService, ResourceUpdateRequest,
};
use crate::state::AppState;

#[derive(Serialize)]
pub struct ResourceResponse {
    #[serde(flatten)]
    pub resource: ResourceRow,
}

pub async fn list(
    State(state): State<AppState>,
    auth: AuthenticatedProfile,
    Query(params): Query<ResourceListParams>,
) -> ApiResult<Json<Vec<ResourceRow>>> {
    ResourceService::list_visible(&state.pool, auth.profile.id, params)
        .await
        .map(Json)
}

pub async fn get(
    State(state): State<AppState>,
    auth: AuthenticatedProfile,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<ResourceRow>> {
    ResourceService::get_visible(&state.pool, auth.profile.id, id)
        .await
        .map(Json)
}

pub async fn get_content(
    State(state): State<AppState>,
    auth: AuthenticatedProfile,
    Path(id): Path<Uuid>,
) -> ApiResult<String> {
    ResourceService::get_content(&state.pool, auth.profile.id, id).await
}

pub async fn create(
    State(state): State<AppState>,
    auth: AuthenticatedProfile,
    Json(request): Json<ResourceCreateRequest>,
) -> ApiResult<(StatusCode, Json<ResourceRow>)> {
    ResourceService::create(&state.pool, auth.profile.id, request)
        .await
        .map(|r| (StatusCode::CREATED, Json(r)))
}

pub async fn update(
    State(state): State<AppState>,
    auth: AuthenticatedProfile,
    Path(id): Path<Uuid>,
    Json(request): Json<ResourceUpdateRequest>,
) -> ApiResult<Json<ResourceRow>> {
    ResourceService::update(&state.pool, auth.profile.id, id, request)
        .await
        .map(Json)
}

pub async fn delete(
    State(state): State<AppState>,
    auth: AuthenticatedProfile,
    Path(id): Path<Uuid>,
) -> ApiResult<StatusCode> {
    ResourceService::delete(&state.pool, auth.profile.id, id)
        .await
        .map(|_| StatusCode::NO_CONTENT)
}
```

- [ ] **Step 6: Update lib.rs**

Replace `crates/temper-api/src/lib.rs`:

```rust
//! temper-api — Axum HTTP server implementing the temper cloud API.

pub mod config;
pub mod error;
pub mod handlers;
pub mod middleware;
pub mod services;
pub mod state;
```

- [ ] **Step 7: Verify compilation**

Run: `cargo check -p temper-api 2>&1 | tail -10`
Expected: compiles.

- [ ] **Step 8: Commit**

```bash
git add crates/temper-api/src/
git commit -m "feat(api): resource service + handlers — CRUD with access control scoping"
```

---

### Task 5: Profile Handlers, EventService, SearchService

**Files:**
- Create: `crates/temper-api/src/handlers/profiles.rs`
- Create: `crates/temper-api/src/handlers/events.rs`
- Create: `crates/temper-api/src/handlers/search.rs`
- Create: `crates/temper-api/src/services/event_service.rs`
- Create: `crates/temper-api/src/services/search_service.rs`
- Modify: `crates/temper-api/src/handlers/mod.rs`
- Modify: `crates/temper-api/src/services/mod.rs`

- [ ] **Step 1: Create profile handlers**

Create `crates/temper-api/src/handlers/profiles.rs`:

```rust
use axum::extract::State;
use axum::Json;
use serde::Deserialize;

use temper_core::types::{AuthenticatedProfile, Profile, ProfileAuthLink};

use crate::error::ApiResult;
use crate::services::profile_service::ProfileService;
use crate::state::AppState;

#[derive(Debug, Deserialize)]
pub struct ProfileUpdateRequest {
    pub display_name: Option<String>,
    pub preferences: Option<serde_json::Value>,
    pub vault_config: Option<serde_json::Value>,
}

pub async fn get(
    State(state): State<AppState>,
    auth: AuthenticatedProfile,
) -> ApiResult<Json<Profile>> {
    ProfileService::get_by_id(&state.pool, auth.profile.id)
        .await
        .map(Json)
}

pub async fn update(
    State(state): State<AppState>,
    auth: AuthenticatedProfile,
    Json(request): Json<ProfileUpdateRequest>,
) -> ApiResult<Json<Profile>> {
    ProfileService::update(
        &state.pool,
        auth.profile.id,
        request.display_name.as_deref(),
        request.preferences.as_ref(),
        request.vault_config.as_ref(),
    )
    .await
    .map(Json)
}

pub async fn list_auth_links(
    State(state): State<AppState>,
    auth: AuthenticatedProfile,
) -> ApiResult<Json<Vec<ProfileAuthLink>>> {
    ProfileService::list_auth_links(&state.pool, auth.profile.id)
        .await
        .map(Json)
}
```

- [ ] **Step 2: Create EventService**

Create `crates/temper-api/src/services/event_service.rs`:

```rust
use chrono::{DateTime, Utc};
use serde::Deserialize;
use sqlx::PgPool;
use uuid::Uuid;

use temper_core::types::EventResponse;

use crate::error::ApiResult;

#[derive(Debug, Deserialize)]
pub struct EventListParams {
    pub since: Option<DateTime<Utc>>,
    pub context: Option<String>,
    pub resource_id: Option<Uuid>,
    pub limit: Option<i64>,
}

pub struct EventService;

impl EventService {
    /// List events with time-bounded visibility + actor-always-sees-own.
    pub async fn list_visible(
        pool: &PgPool,
        profile_id: Uuid,
        params: EventListParams,
    ) -> ApiResult<Vec<EventResponse>> {
        let limit = params.limit.unwrap_or(50).min(200);

        let rows = sqlx::query_as::<_, EventResponse>(
            r#"
            SELECT e.id, e.profile_id, e.client_id,
                   c.name as context, e.resource_id,
                   e.event_type, e.payload, e.created
            FROM kb_events e
            LEFT JOIN kb_contexts c ON c.id = e.kb_context_id
            WHERE (
                e.profile_id = $1
                OR e.resource_id IN (SELECT resource_id FROM resources_visible_to($1))
            )
            AND ($2::timestamptz IS NULL OR e.created >= $2)
            AND ($3::text IS NULL OR c.name = $3)
            AND ($4::uuid IS NULL OR e.resource_id = $4)
            ORDER BY e.created DESC
            LIMIT $5
            "#,
        )
        .bind(profile_id)
        .bind(params.since)
        .bind(&params.context)
        .bind(params.resource_id)
        .bind(limit)
        .fetch_all(pool)
        .await?;

        Ok(rows)
    }
}
```

- [ ] **Step 3: Create event handler**

Create `crates/temper-api/src/handlers/events.rs`:

```rust
use axum::extract::{Query, State};
use axum::Json;

use temper_core::types::{AuthenticatedProfile, EventResponse};

use crate::error::ApiResult;
use crate::services::event_service::{EventListParams, EventService};
use crate::state::AppState;

pub async fn list(
    State(state): State<AppState>,
    auth: AuthenticatedProfile,
    Query(params): Query<EventListParams>,
) -> ApiResult<Json<Vec<EventResponse>>> {
    EventService::list_visible(&state.pool, auth.profile.id, params)
        .await
        .map(Json)
}
```

- [ ] **Step 4: Create SearchService**

Create `crates/temper-api/src/services/search_service.rs`:

```rust
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use uuid::Uuid;

use temper_core::types::SearchMode;

use crate::error::ApiResult;

#[derive(Debug, Deserialize)]
pub struct SearchParams {
    pub q: String,
    #[serde(default)]
    pub mode: SearchMode,
    pub context: Option<String>,
    pub doc_type: Option<String>,
    pub team: Option<String>,
    pub depth: Option<u32>,
    pub limit: Option<i64>,
}

#[derive(Debug, Serialize)]
pub struct SearchResultRow {
    pub resource_id: Uuid,
    pub title: String,
    pub context: String,
    pub doc_type: String,
    pub score: f64,
    pub snippet: String,
}

pub struct SearchService;

impl SearchService {
    /// Unified search — currently semantic only; keyword and graph modes stub to semantic.
    pub async fn search(
        pool: &PgPool,
        profile_id: Uuid,
        _params: SearchParams,
    ) -> ApiResult<Vec<SearchResultRow>> {
        // Semantic search requires an embedding of the query, which needs temper-embed.
        // For now, return an empty result set. The endpoint contract is in place;
        // the implementation will be wired when the embedding pipeline lands in I8.
        //
        // When implemented:
        // 1. Embed the query string (via temper-embed or a pre-computed embedding endpoint)
        // 2. WITH visible AS (SELECT resource_id FROM resources_visible_to($1))
        //    SELECT r.*, c.content, c.embedding <=> $2::vector AS distance
        //    FROM kb_current_chunks c
        //    JOIN resources r ON r.id = c.resource_id
        //    JOIN visible v ON v.resource_id = r.id
        //    ORDER BY distance LIMIT $3
        let _ = (pool, profile_id);
        Ok(vec![])
    }
}
```

- [ ] **Step 5: Create search handler**

Create `crates/temper-api/src/handlers/search.rs`:

```rust
use axum::extract::{Query, State};
use axum::Json;

use temper_core::types::AuthenticatedProfile;

use crate::error::ApiResult;
use crate::services::search_service::{SearchParams, SearchResultRow, SearchService};
use crate::state::AppState;

pub async fn search(
    State(state): State<AppState>,
    auth: AuthenticatedProfile,
    Query(params): Query<SearchParams>,
) -> ApiResult<Json<Vec<SearchResultRow>>> {
    SearchService::search(&state.pool, auth.profile.id, params)
        .await
        .map(Json)
}
```

- [ ] **Step 6: Update module files**

Update `crates/temper-api/src/handlers/mod.rs`:

```rust
pub mod events;
pub mod health;
pub mod profiles;
pub mod resources;
pub mod search;
```

Update `crates/temper-api/src/services/mod.rs`:

```rust
pub mod event_service;
pub mod profile_service;
pub mod resource_service;
pub mod search_service;
```

- [ ] **Step 7: Verify compilation**

Run: `cargo check -p temper-api 2>&1 | tail -10`
Expected: compiles. Note: `EventResponse` from temper-core may need adjustment if its fields don't match the SQL query aliases. If compilation fails on the EventResponse query, create a local `EventRow` struct in event_service.rs with `FromRow` and map it to the response. Fix as needed.

- [ ] **Step 8: Commit**

```bash
git add crates/temper-api/src/
git commit -m "feat(api): profile, event, search handlers and services"
```

---

### Task 6: Routes, main.rs, CORS

**Files:**
- Create: `crates/temper-api/src/routes.rs`
- Modify: `crates/temper-api/src/main.rs`
- Modify: `crates/temper-api/src/lib.rs`

- [ ] **Step 1: Create routes.rs**

Create `crates/temper-api/src/routes.rs`:

```rust
use axum::routing::{delete, get, patch, post};
use axum::Router;
use tower_http::cors::{Any, CorsLayer};
use tower_http::trace::TraceLayer;

use crate::handlers;
use crate::middleware::auth;
use crate::state::AppState;

pub fn create_app(state: AppState) -> Router {
    let public = Router::new().route("/api/health", get(handlers::health::health_check));

    let protected = Router::new()
        .route("/api/resources", get(handlers::resources::list).post(handlers::resources::create))
        .route(
            "/api/resources/{id}",
            get(handlers::resources::get)
                .patch(handlers::resources::update)
                .delete(handlers::resources::delete),
        )
        .route("/api/resources/{id}/content", get(handlers::resources::get_content))
        .route(
            "/api/profile",
            get(handlers::profiles::get).patch(handlers::profiles::update),
        )
        .route("/api/profile/auth-links", get(handlers::profiles::list_auth_links))
        .route("/api/events", get(handlers::events::list))
        .route("/api/search", get(handlers::search::search))
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            auth::require_auth,
        ));

    let cors = cors_layer(&state);

    Router::new()
        .merge(public)
        .merge(protected)
        .layer(TraceLayer::new_for_http())
        .layer(cors)
        .with_state(state)
}

fn cors_layer(state: &AppState) -> CorsLayer {
    if state.config.cors_origins.is_empty() {
        CorsLayer::permissive()
    } else {
        CorsLayer::new()
            .allow_origin(
                state
                    .config
                    .cors_origins
                    .iter()
                    .filter_map(|o| o.parse().ok())
                    .collect::<Vec<_>>(),
            )
            .allow_methods(Any)
            .allow_headers(Any)
    }
}
```

- [ ] **Step 2: Update main.rs**

Replace `crates/temper-api/src/main.rs`:

```rust
use sqlx::postgres::PgPoolOptions;
use std::net::SocketAddr;
use tokio::net::TcpListener;
use tracing_subscriber::EnvFilter;

use temper_api::config::ApiConfig;
use temper_api::routes::create_app;
use temper_api::state::{AppState, JwksKeyStore};

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .init();

    let config = ApiConfig::from_env().expect("Failed to load config from environment");

    let pool = PgPoolOptions::new()
        .max_connections(10)
        .connect(&config.database_url)
        .await
        .expect("Failed to connect to database");

    // Run migrations
    sqlx::migrate!("../../migrations")
        .run(&pool)
        .await
        .expect("Failed to run migrations");

    let jwks_store = JwksKeyStore::new(config.jwks_url.clone());
    let port = config.port;
    let state = AppState::new(pool, jwks_store, config);
    let app = create_app(state);

    let addr = SocketAddr::from(([0, 0, 0, 0], port));
    let listener = TcpListener::bind(addr).await.expect("Failed to bind");
    tracing::info!("temper-api listening on {addr}");

    axum::serve(listener, app).await.expect("Server failed");
}
```

- [ ] **Step 3: Update lib.rs to re-export create_app**

Replace `crates/temper-api/src/lib.rs`:

```rust
//! temper-api — Axum HTTP server implementing the temper cloud API.
//!
//! Platform-agnostic: runs locally or wrapped by temper-cloud for Vercel.
//! Use [`routes::create_app`] to get the composable Router.

pub mod config;
pub mod error;
pub mod handlers;
pub mod middleware;
pub mod routes;
pub mod services;
pub mod state;

pub use routes::create_app;
```

- [ ] **Step 4: Verify compilation**

Run: `cargo check -p temper-api 2>&1 | tail -10`
Expected: compiles.

- [ ] **Step 5: Commit**

```bash
git add crates/temper-api/src/
git commit -m "feat(api): routes, CORS, main.rs — complete server wiring"
```

---

### Task 7: Test Infrastructure + Integration Tests

**Files:**
- Create: `crates/temper-api/tests/common/mod.rs`
- Create: `crates/temper-api/tests/common/fixtures.rs`
- Create: `crates/temper-api/tests/health_test.rs`
- Create: `crates/temper-api/tests/auth_test.rs`
- Create: `crates/temper-api/tests/resources_test.rs`

This task creates the test infrastructure and a representative set of integration tests. The test suite validates: health check (public), auth flow (JWT verification, auto-provisioning), and resource CRUD (access control scoping).

- [ ] **Step 1: Create test common module**

Create `crates/temper-api/tests/common/mod.rs`:

```rust
#![allow(dead_code)]
pub mod fixtures;

use axum::Router;
use jsonwebtoken::{encode, EncodingKey, Header, Algorithm};
use serde::Serialize;
use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::net::TcpListener;

use temper_api::config::ApiConfig;
use temper_api::routes::create_app;
use temper_api::state::{AppState, JwksKeyStore};

/// EdDSA test key pair (Ed25519).
/// Generated deterministically for tests. NOT for production.
pub const TEST_ED25519_PRIVATE_KEY: &[u8] = include_bytes!("test_ed25519.key");

pub struct TestApp {
    pub addr: SocketAddr,
    pub pool: PgPool,
    pub client: reqwest::Client,
}

impl TestApp {
    pub fn url(&self, path: &str) -> String {
        format!("http://{}{}", self.addr, path)
    }

    pub fn auth_header(&self, token: &str) -> (String, String) {
        ("Authorization".to_string(), format!("Bearer {token}"))
    }
}

pub async fn setup_test_app() -> TestApp {
    let database_url =
        std::env::var("DATABASE_URL").unwrap_or_else(|_| {
            "postgresql://temper:temper@localhost:5437/temper_test".to_string()
        });

    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(&database_url)
        .await
        .expect("Failed to connect to test database");

    // Run migrations
    sqlx::migrate!("../../migrations")
        .run(&pool)
        .await
        .expect("Failed to run migrations");

    // Clean and seed
    fixtures::clean_and_seed(&pool).await;

    // Use a static test key for JWT verification
    let jwks_store = JwksKeyStore::with_static_key(
        jsonwebtoken::DecodingKey::from_ed_pem(include_bytes!("test_ed25519.pub"))
            .expect("Failed to load test public key"),
    );

    let config = ApiConfig {
        database_url,
        jwks_url: "unused-in-tests".to_string(),
        auth_issuer: "test-issuer".to_string(),
        auth_audience: None,
        cors_origins: vec![],
        port: 0,
    };

    let state = AppState::new(pool.clone(), jwks_store, config);
    let app = create_app(state);

    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("Failed to bind test server");
    let addr = listener.local_addr().expect("Failed to get local addr");

    tokio::spawn(async move {
        axum::serve(listener, app).await.expect("Test server failed");
    });

    TestApp {
        addr,
        pool,
        client: reqwest::Client::new(),
    }
}

#[derive(Serialize)]
struct TestClaims {
    sub: String,
    email: String,
    iss: String,
    exp: i64,
    iat: i64,
}

pub fn generate_test_jwt(sub: &str, email: &str) -> String {
    let now = chrono::Utc::now().timestamp();
    let claims = TestClaims {
        sub: sub.to_string(),
        email: email.to_string(),
        iss: "test-issuer".to_string(),
        exp: now + 3600,
        iat: now,
    };
    let key = EncodingKey::from_ed_pem(include_bytes!("test_ed25519.key"))
        .expect("Failed to load test private key");
    let header = Header::new(Algorithm::EdDSA);
    encode(&header, &claims, &key).expect("Failed to encode JWT")
}

pub fn generate_expired_jwt(sub: &str, email: &str) -> String {
    let now = chrono::Utc::now().timestamp();
    let claims = TestClaims {
        sub: sub.to_string(),
        email: email.to_string(),
        iss: "test-issuer".to_string(),
        exp: now - 3600, // expired 1 hour ago
        iat: now - 7200,
    };
    let key = EncodingKey::from_ed_pem(include_bytes!("test_ed25519.key"))
        .expect("Failed to load test private key");
    let header = Header::new(Algorithm::EdDSA);
    encode(&header, &claims, &key).expect("Failed to encode JWT")
}
```

- [ ] **Step 2: Generate Ed25519 test key pair**

Run these commands to generate deterministic test keys:

```bash
openssl genpkey -algorithm Ed25519 -out crates/temper-api/tests/common/test_ed25519.key
openssl pkey -in crates/temper-api/tests/common/test_ed25519.key -pubout -out crates/temper-api/tests/common/test_ed25519.pub
```

- [ ] **Step 3: Create test fixtures**

Create `crates/temper-api/tests/common/fixtures.rs`:

```rust
use sqlx::PgPool;
use uuid::Uuid;

/// Well-known test UUIDs
pub const SYSTEM_PROFILE_ID: &str = "00000000-0000-0000-0004-000000000001";
pub const TEMPER_CONTEXT_ID: &str = "00000000-0000-0000-0003-000000000001";
pub const RESEARCH_DOC_TYPE_ID: &str = "00000000-0000-0000-0001-000000000004";
pub const TEST_RESOURCE_ID: &str = "00000000-0000-0000-0099-000000000001";

pub async fn clean_and_seed(pool: &PgPool) {
    // Clean test data (preserve seed data from migrations)
    sqlx::query("DELETE FROM kb_events")
        .execute(pool)
        .await
        .expect("clean events");
    sqlx::query("DELETE FROM kb_chunks")
        .execute(pool)
        .await
        .expect("clean chunks");
    sqlx::query("DELETE FROM resources WHERE id != '00000000-0000-0000-0000-000000000000'::uuid")
        .execute(pool)
        .await
        .expect("clean resources");
    sqlx::query("DELETE FROM kb_team_members")
        .execute(pool)
        .await
        .expect("clean team members");
    sqlx::query("DELETE FROM kb_teams WHERE TRUE")
        .execute(pool)
        .await
        .expect("clean teams");
    sqlx::query(
        "DELETE FROM kb_profile_auth_links WHERE profile_id NOT IN (
            '00000000-0000-0000-0004-000000000001'::uuid,
            '00000000-0000-0000-0004-000000000002'::uuid
        )",
    )
    .execute(pool)
    .await
    .expect("clean auth links");
    sqlx::query(
        "DELETE FROM kb_profiles WHERE id NOT IN (
            '00000000-0000-0000-0004-000000000001'::uuid,
            '00000000-0000-0000-0004-000000000002'::uuid
        )",
    )
    .execute(pool)
    .await
    .expect("clean profiles");

    // Create a test resource owned by System profile
    let resource_id: Uuid = TEST_RESOURCE_ID.parse().unwrap();
    let context_id: Uuid = TEMPER_CONTEXT_ID.parse().unwrap();
    let doc_type_id: Uuid = RESEARCH_DOC_TYPE_ID.parse().unwrap();
    let system_id: Uuid = SYSTEM_PROFILE_ID.parse().unwrap();

    sqlx::query(
        "INSERT INTO resources (id, kb_context_id, kb_doc_type_id, uri, title, slug,
                                originator_profile_id, owner_profile_id, created, updated)
         VALUES ($1, $2, $3, 'kb://research/temper/test', 'Test Research', 'test-research',
                 $4, $4, now(), now())
         ON CONFLICT (uri) DO NOTHING",
    )
    .bind(resource_id)
    .bind(context_id)
    .bind(doc_type_id)
    .bind(system_id)
    .execute(pool)
    .await
    .expect("seed test resource");
}
```

- [ ] **Step 4: Create health test**

Create `crates/temper-api/tests/health_test.rs`:

```rust
#[cfg(feature = "test-db")]
mod common;

#[cfg(feature = "test-db")]
#[tokio::test]
async fn test_health_check() {
    let app = common::setup_test_app().await;
    let resp = app
        .client
        .get(app.url("/api/health"))
        .send()
        .await
        .expect("request failed");
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["status"], "ok");
}
```

- [ ] **Step 5: Create auth test**

Create `crates/temper-api/tests/auth_test.rs`:

```rust
#[cfg(feature = "test-db")]
mod common;

#[cfg(feature = "test-db")]
mod tests {
    use super::common;

    #[tokio::test]
    async fn test_missing_auth_returns_401() {
        let app = common::setup_test_app().await;
        let resp = app
            .client
            .get(app.url("/api/profile"))
            .send()
            .await
            .expect("request failed");
        assert_eq!(resp.status(), 401);
    }

    #[tokio::test]
    async fn test_expired_jwt_returns_401() {
        let app = common::setup_test_app().await;
        let token = common::generate_expired_jwt("test-sub", "test@example.com");
        let resp = app
            .client
            .get(app.url("/api/profile"))
            .header("Authorization", format!("Bearer {token}"))
            .send()
            .await
            .expect("request failed");
        assert_eq!(resp.status(), 401);
    }

    #[tokio::test]
    async fn test_valid_jwt_auto_provisions_profile() {
        let app = common::setup_test_app().await;
        let token = common::generate_test_jwt("new-user-sub", "newuser@example.com");
        let resp = app
            .client
            .get(app.url("/api/profile"))
            .header("Authorization", format!("Bearer {token}"))
            .send()
            .await
            .expect("request failed");
        assert_eq!(resp.status(), 200);
        let body: serde_json::Value = resp.json().await.unwrap();
        assert_eq!(body["display_name"], "newuser");
        assert_eq!(body["email"], "newuser@example.com");
    }
}
```

- [ ] **Step 6: Create resources test**

Create `crates/temper-api/tests/resources_test.rs`:

```rust
#[cfg(feature = "test-db")]
mod common;

#[cfg(feature = "test-db")]
mod tests {
    use super::common;

    #[tokio::test]
    async fn test_create_and_list_resources() {
        let app = common::setup_test_app().await;
        let token = common::generate_test_jwt("resource-test-sub", "resource@example.com");
        let auth = format!("Bearer {token}");

        // Create a resource
        let resp = app
            .client
            .post(app.url("/api/resources"))
            .header("Authorization", &auth)
            .json(&serde_json::json!({
                "title": "Test Resource",
                "context": "temper",
                "doc_type": "research"
            }))
            .send()
            .await
            .expect("create failed");
        assert_eq!(resp.status(), 201);
        let created: serde_json::Value = resp.json().await.unwrap();
        let resource_id = created["id"].as_str().unwrap();

        // List resources — should include the one we just created
        let resp = app
            .client
            .get(app.url("/api/resources?context=temper"))
            .header("Authorization", &auth)
            .send()
            .await
            .expect("list failed");
        assert_eq!(resp.status(), 200);
        let list: Vec<serde_json::Value> = resp.json().await.unwrap();
        assert!(list.iter().any(|r| r["id"].as_str() == Some(resource_id)));
    }

    #[tokio::test]
    async fn test_resource_visibility_scoping() {
        let app = common::setup_test_app().await;

        // User A creates a resource
        let token_a = common::generate_test_jwt("user-a-sub", "usera@example.com");
        let resp = app
            .client
            .post(app.url("/api/resources"))
            .header("Authorization", format!("Bearer {token_a}"))
            .json(&serde_json::json!({
                "title": "User A Private",
                "context": "temper",
                "doc_type": "research"
            }))
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), 201);

        // User B should NOT see User A's resource
        let token_b = common::generate_test_jwt("user-b-sub", "userb@example.com");
        let resp = app
            .client
            .get(app.url("/api/resources"))
            .header("Authorization", format!("Bearer {token_b}"))
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), 200);
        let list: Vec<serde_json::Value> = resp.json().await.unwrap();
        assert!(
            list.iter().all(|r| r["title"].as_str() != Some("User A Private")),
            "User B should not see User A's resources"
        );
    }
}
```

- [ ] **Step 7: Verify tests compile (without running)**

Run: `cargo test -p temper-api --features test-db --no-run 2>&1 | tail -10`
Expected: compiles. May require adjusting types if `EventResponse` or others don't perfectly match `FromRow` expectations from the SQL queries. Fix compilation errors as needed.

- [ ] **Step 8: Run tests against Docker Postgres**

Ensure Docker Postgres is running on port 5437, then:

```bash
# Create temper_test database if it doesn't exist
docker exec temper-postgres psql -U temper -d temper_development -c "CREATE DATABASE temper_test;" 2>/dev/null || true
DATABASE_URL=postgresql://temper:temper@localhost:5437/temper_test cargo test -p temper-api --features test-db -- --nocapture 2>&1 | tail -30
```

Expected: health test passes, auth tests pass, resource CRUD tests pass.

- [ ] **Step 9: Run clippy**

Run: `cargo clippy -p temper-api --all-features -- -D warnings 2>&1 | tail -10`
Expected: no warnings.

- [ ] **Step 10: Commit**

```bash
git add crates/temper-api/tests/ crates/temper-api/.env.template
git commit -m "feat(api): test infrastructure + integration tests — health, auth, resources"
```

---

### Task 8: temper-core web-api Feature Gate + OpenAPI (Optional)

**Files:**
- Modify: `crates/temper-core/Cargo.toml`
- Modify: temper-core type files (add `#[cfg_attr]` for utoipa)
- Create: `crates/temper-api/src/openapi.rs`
- Modify: `crates/temper-api/Cargo.toml`

This task is optional for the initial cut — OpenAPI docs are nice but not blocking. If time permits:

- [ ] **Step 1: Add utoipa as optional dependency to temper-core**

Add to `crates/temper-core/Cargo.toml`:

```toml
[dependencies]
# ... existing deps ...
utoipa = { version = "5", optional = true }

[features]
web-api = ["utoipa"]
```

- [ ] **Step 2: Add ToSchema derives to key types**

Add `#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]` to: Profile, ProfileAuthLink, AuthClaims, SearchMode, SearchResult, EventResponse, AccessLevel, TeamRole, Team, TeamMember in their respective files in `crates/temper-core/src/types/`.

- [ ] **Step 3: Add utoipa to temper-api**

Add to `crates/temper-api/Cargo.toml` dependencies:

```toml
utoipa = { version = "5", features = ["axum_extras"] }
utoipa-swagger-ui = { version = "8", features = ["axum"] }
```

And update temper-core dependency:

```toml
temper-core = { path = "../temper-core", features = ["web-api"] }
```

- [ ] **Step 4: Create openapi.rs**

Create `crates/temper-api/src/openapi.rs` with `ApiDoc` struct deriving `OpenApi`, `SecurityAddon` modifier, and Swagger UI route at `/api-docs/ui`.

- [ ] **Step 5: Wire into routes.rs**

Add `.merge(openapi::docs_routes())` to `create_app`.

- [ ] **Step 6: Verify and commit**

```bash
cargo check -p temper-api 2>&1 | tail -5
git add crates/temper-core/ crates/temper-api/
git commit -m "feat(api): OpenAPI documentation via utoipa + Swagger UI"
```
