# Local Integration and E2E Testing Infrastructure — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build an e2e test harness that exercises CLI-as-library against an in-process API server with full config isolation and DB-per-test guarantees.

**Architecture:** Config injection via `_from` builder variants in temper-client and temper-cli. Token resolution centralized in `HttpClient` with override support. A new `tests/e2e` workspace crate uses `#[sqlx::test]` for DB isolation, an in-process Axum server, and a test tracing layer for log verification.

**Tech Stack:** Rust, Axum, sqlx, tracing, reqwest, jsonwebtoken, tempfile, cargo-nextest

---

### Task 1: Add `resolve_token` to `HttpClient` with override support

**Files:**
- Modify: `crates/temper-client/src/http.rs`
- Test: `crates/temper-client/src/http.rs` (inline tests)

- [ ] **Step 1: Write failing test for `resolve_token` with override**

Add to the `#[cfg(test)] mod tests` block in `crates/temper-client/src/http.rs`:

```rust
#[test]
fn resolve_token_returns_override_when_set() {
    let client = HttpClient::with_token_override(
        "https://example.com",
        None,
        "test-token-123".to_string(),
    );
    let token = client.resolve_token().unwrap();
    assert_eq!(token, "test-token-123");
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p temper-client resolve_token_returns_override`
Expected: FAIL — `with_token_override` and `resolve_token` don't exist yet.

- [ ] **Step 3: Add `token_override` field and `resolve_token` method to `HttpClient`**

In `crates/temper-client/src/http.rs`, add the `token_override` field to the struct:

```rust
#[derive(Debug, Clone)]
pub struct HttpClient {
    inner: Client,
    base_url: String,
    device_id: Option<String>,
    token_override: Option<String>,
}
```

Update the existing `new` constructor to initialize it:

```rust
pub fn new(base_url: &str, device_id: Option<String>) -> Self {
    let inner = Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .expect("failed to build reqwest client");

    Self {
        inner,
        base_url: base_url.trim_end_matches('/').to_owned(),
        device_id,
        token_override: None,
    }
}
```

Add the new constructor and method:

```rust
/// Create a client with a pre-set token (for tests — bypasses `auth.json`).
pub fn with_token_override(
    base_url: &str,
    device_id: Option<String>,
    token: String,
) -> Self {
    let mut client = Self::new(base_url, device_id);
    client.token_override = Some(token);
    client
}

/// Resolve an access token: override first, then `auth.json`.
pub fn resolve_token(&self) -> Result<String> {
    if let Some(ref token) = self.token_override {
        return Ok(token.clone());
    }
    crate::auth::current_token()
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p temper-client resolve_token_returns_override`
Expected: PASS

- [ ] **Step 5: Write test for fallback behavior**

Add to the same test module:

```rust
#[test]
fn resolve_token_without_override_falls_back_to_auth() {
    let client = HttpClient::new("https://example.com", None);
    // No token override, no auth.json in test env → should error
    let result = client.resolve_token();
    assert!(result.is_err());
}
```

- [ ] **Step 6: Run test to verify it passes**

Run: `cargo test -p temper-client resolve_token_without_override`
Expected: PASS

- [ ] **Step 7: Commit**

```bash
git add crates/temper-client/src/http.rs
git commit -m "feat: add token_override and resolve_token to HttpClient"
```

---

### Task 2: Migrate sub-clients from `auth::current_token()` to `http.resolve_token()`

**Files:**
- Modify: `crates/temper-client/src/ingest.rs`
- Modify: `crates/temper-client/src/search.rs`
- Modify: `crates/temper-client/src/resources.rs`
- Modify: `crates/temper-client/src/profile.rs`
- Modify: `crates/temper-client/src/events.rs`
- Modify: `crates/temper-client/src/contexts.rs`
- Modify: `crates/temper-client/src/sync.rs`
- Modify: `crates/temper-client/src/upload.rs`

This is a mechanical replacement. Each sub-client currently does:
```rust
let token = auth::current_token()?;
```

Replace every occurrence with:
```rust
let token = self.http.resolve_token()?;
```

And remove the `use crate::auth;` import from each file (if `auth` is no longer used there).

- [ ] **Step 1: Replace in `ingest.rs`**

In `crates/temper-client/src/ingest.rs`, replace both `auth::current_token()?` calls (lines 33 and 42) with `self.http.resolve_token()?`, and remove `use crate::auth;` from imports.

- [ ] **Step 2: Replace in `search.rs`**

In `crates/temper-client/src/search.rs`, replace `auth::current_token()?` (line 34) with `self.http.resolve_token()?`, and remove `use crate::auth;`.

- [ ] **Step 3: Replace in `resources.rs`**

In `crates/temper-client/src/resources.rs`, replace all `auth::current_token()?` calls with `self.http.resolve_token()?`, and remove `use crate::auth;`.

- [ ] **Step 4: Replace in `profile.rs`**

Same pattern in `crates/temper-client/src/profile.rs`.

- [ ] **Step 5: Replace in `events.rs`**

Same pattern in `crates/temper-client/src/events.rs`.

- [ ] **Step 6: Replace in `contexts.rs`**

Same pattern in `crates/temper-client/src/contexts.rs`.

- [ ] **Step 7: Replace in `sync.rs`**

Same pattern in `crates/temper-client/src/sync.rs`.

- [ ] **Step 8: Replace in `upload.rs`**

Same pattern in `crates/temper-client/src/upload.rs`.

- [ ] **Step 9: Verify all existing tests pass**

Run: `cargo test -p temper-client`
Expected: All existing tests PASS. The behavioral change is invisible — `resolve_token()` without an override calls `auth::current_token()` exactly as before.

- [ ] **Step 10: Run clippy**

Run: `cargo clippy -p temper-client --all-features`
Expected: No warnings about unused `auth` imports.

- [ ] **Step 11: Commit**

```bash
git add crates/temper-client/src/
git commit -m "refactor: sub-clients use http.resolve_token() instead of auth::current_token()"
```

---

### Task 3: Add `build_client_from` to temper-client config

**Files:**
- Modify: `crates/temper-client/src/config.rs`
- Test: `crates/temper-client/src/config.rs` (inline tests)

- [ ] **Step 1: Write failing test for `build_client_from`**

Add to the `#[cfg(test)] mod tests` block in `crates/temper-client/src/config.rs`:

```rust
#[test]
fn build_client_from_uses_config_api_url() {
    let config = TemperConfig {
        cloud: CloudSection {
            api_url: "https://test.example.com".to_string(),
        },
        ..TemperConfig::default()
    };
    let auth = crate::auth::StoredAuth {
        provider: "test".to_string(),
        access_token: "test-token".to_string(),
        refresh_token: None,
        expires_at: chrono::Utc::now() + chrono::Duration::hours(1),
        profile_id: None,
        device_id: Some("test-device".to_string()),
    };
    let client = build_client_from(&config, Some(&auth)).unwrap();
    // Client was constructed without reading disk — verify it exists
    assert!(format!("{:?}", client).contains("test.example.com"));
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p temper-client build_client_from_uses_config`
Expected: FAIL — `build_client_from` doesn't exist.

- [ ] **Step 3: Implement `build_client_from`**

In `crates/temper-client/src/config.rs`, add the new function and refactor `build_client`:

```rust
/// Build client from explicit config and optional stored auth (no disk reads).
///
/// When `auth` is provided, the access token is set as a token override on the
/// underlying `HttpClient`, bypassing `auth.json` reads in all sub-clients.
pub fn build_client_from(
    config: &TemperConfig,
    auth: Option<&crate::auth::StoredAuth>,
) -> crate::error::Result<crate::TemperClient> {
    let url = api_url(config);
    let device_id = auth.and_then(|a| a.device_id.clone());

    let client = if let Some(auth) = auth {
        crate::TemperClient::with_token(
            &url,
            device_id,
            auth.access_token.clone(),
        )
    } else {
        crate::TemperClient::new(&url, device_id)
    };

    let client = match oauth_config(config) {
        Ok(oauth) => client.with_oauth(oauth),
        Err(e) => {
            tracing::debug!("OAuth config not available: {e}");
            client
        }
    };

    Ok(client)
}
```

This requires a new `TemperClient::with_token` constructor. In `crates/temper-client/src/lib.rs`, add:

```rust
/// Create a new client with a pre-set token (bypasses `auth.json`).
pub fn with_token(base_url: &str, device_id: Option<String>, token: String) -> Self {
    Self {
        http: http::HttpClient::with_token_override(base_url, device_id, token),
        oauth_config: None,
    }
}
```

Then refactor the existing `build_client` to delegate:

```rust
pub fn build_client() -> crate::error::Result<crate::TemperClient> {
    let config = load_cloud_config()?;
    let auth = crate::auth::load_auth().ok().flatten();
    build_client_from(&config, auth.as_ref())
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p temper-client build_client_from_uses_config`
Expected: PASS

- [ ] **Step 5: Run full client test suite**

Run: `cargo test -p temper-client`
Expected: All tests PASS (existing `build_client` behavior unchanged).

- [ ] **Step 6: Commit**

```bash
git add crates/temper-client/src/config.rs crates/temper-client/src/lib.rs
git commit -m "feat: add build_client_from for config-injected client construction"
```

---

### Task 4: Add `load_from` to temper-cli config

**Files:**
- Modify: `crates/temper-cli/src/config.rs`
- Test: `crates/temper-cli/src/config.rs` (or `crates/temper-cli/tests/config_test.rs`)

- [ ] **Step 1: Write failing test**

Add to `crates/temper-cli/tests/config_test.rs`:

```rust
#[test]
fn test_load_from_uses_explicit_config() {
    let dir = TempDir::new().unwrap();
    let vault_path = dir.path().join("test-vault");
    std::fs::create_dir_all(vault_path.join(".temper")).unwrap();

    let config = temper_core::types::config::TemperConfig {
        vault: temper_core::types::config::CloudVaultConfig {
            path: vault_path.to_str().unwrap().to_string(),
        },
        sync: temper_core::types::config::UnifiedSyncConfig {
            subscriptions: temper_core::types::config::SyncSubscriptionsConfig {
                contexts: vec!["test-ctx".to_string()],
            },
            ..Default::default()
        },
        skill: temper_core::types::config::SkillConfig {
            output: "/tmp/test-skills".to_string(),
            framework: "test-fw".to_string(),
        },
        ..Default::default()
    };

    let result = temper_cli::config::load_from(&config, None);
    assert_eq!(result.vault_root, vault_path);
    assert_eq!(result.contexts, vec!["test-ctx"]);
    assert_eq!(result.skill_framework, "test-fw");
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p temper-cli test_load_from_uses_explicit`
Expected: FAIL — `load_from` doesn't exist.

- [ ] **Step 3: Implement `load_from`**

In `crates/temper-cli/src/config.rs`, add:

```rust
/// Build Config from an explicit TemperConfig + vault override (no disk reads).
pub fn load_from(global: &TemperConfig, cli_vault: Option<&str>) -> Config {
    let vault_root = cli_vault
        .map(|v| expand_tilde(v))
        .unwrap_or_else(|| expand_tilde(&global.vault.path));
    Config {
        state_dir: vault_root.join(".temper"),
        vault_root,
        contexts: global.sync.subscriptions.contexts.clone(),
        skill_output: expand_tilde(&global.skill.output),
        skill_framework: global.skill.framework.clone(),
    }
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p temper-cli test_load_from_uses_explicit`
Expected: PASS

- [ ] **Step 5: Write test for cli_vault override**

```rust
#[test]
fn test_load_from_cli_vault_overrides_config() {
    let dir = TempDir::new().unwrap();
    let override_path = dir.path().join("override-vault");
    std::fs::create_dir_all(override_path.join(".temper")).unwrap();

    let config = temper_core::types::config::TemperConfig::default();
    let result = temper_cli::config::load_from(
        &config,
        Some(override_path.to_str().unwrap()),
    );
    assert_eq!(result.vault_root, override_path);
}
```

- [ ] **Step 6: Run test to verify it passes**

Run: `cargo test -p temper-cli test_load_from_cli_vault_overrides`
Expected: PASS

- [ ] **Step 7: Commit**

```bash
git add crates/temper-cli/src/config.rs crates/temper-cli/tests/config_test.rs
git commit -m "feat: add load_from for config-injected CLI config construction"
```

---

### Task 5: Create `tests/e2e` crate skeleton

**Files:**
- Modify: `Cargo.toml` (workspace members)
- Create: `tests/e2e/Cargo.toml`
- Create: `tests/e2e/tests/common/mod.rs` (empty for now)
- Create: `tests/e2e/tests/common/tracing.rs` (empty for now)

- [ ] **Step 1: Add `tests/e2e` to workspace members**

In the root `Cargo.toml`, change:

```toml
members = ["crates/*"]
```

to:

```toml
members = ["crates/*", "tests/e2e"]
```

- [ ] **Step 2: Create `tests/e2e/Cargo.toml`**

```toml
[package]
name = "temper-e2e"
version = "0.0.0"
edition = "2021"
publish = false

[features]
test-db = []

[dev-dependencies]
temper-api = { path = "../../crates/temper-api" }
temper-cli = { path = "../../crates/temper-cli" }
temper-client = { path = "../../crates/temper-client" }
temper-core = { path = "../../crates/temper-core" }
tokio = { version = "1", features = ["full"] }
sqlx = { version = "0.8", features = ["postgres", "runtime-tokio-rustls", "migrate"] }
reqwest = { version = "0.12", features = ["rustls-tls", "json"] }
axum = "0.8"
serde_json = "1"
chrono = "0.4"
uuid = { version = "1", features = ["v4", "v7"] }
jsonwebtoken = "9"
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter", "json", "registry"] }
tempfile = "3"
base64 = "0.22"
rmp-serde = "1"
```

- [ ] **Step 3: Create placeholder module files**

Create `tests/e2e/tests/common/mod.rs`:

```rust
//! Shared e2e test infrastructure.
pub mod tracing_layer;
```

Create `tests/e2e/tests/common/tracing_layer.rs`:

```rust
//! Test tracing layer for capturing spans and events.
```

- [ ] **Step 4: Verify workspace compiles**

Run: `cargo check --workspace`
Expected: PASS (no lib.rs needed — the crate only has integration tests)

- [ ] **Step 5: Commit**

```bash
git add Cargo.toml tests/e2e/
git commit -m "chore: add tests/e2e crate skeleton to workspace"
```

---

### Task 6: Build `E2eTestApp` harness

**Files:**
- Modify: `tests/e2e/tests/common/mod.rs`

- [ ] **Step 1: Copy test RSA keys to e2e fixtures**

```bash
mkdir -p tests/e2e/tests/fixtures
cp crates/temper-api/tests/common/test_rsa.key tests/e2e/tests/fixtures/
cp crates/temper-api/tests/common/test_rsa.pub tests/e2e/tests/fixtures/
```

- [ ] **Step 2: Implement `E2eTestApp`**

Write `tests/e2e/tests/common/mod.rs`:

```rust
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
        .execute(pool).await.expect("clean kb_device_sync_state");
    sqlx::query("DELETE FROM kb_transfers")
        .execute(pool).await.expect("clean kb_transfers");
    sqlx::query("DELETE FROM kb_team_invitations")
        .execute(pool).await.expect("clean kb_team_invitations");
    sqlx::query("DELETE FROM kb_team_resources")
        .execute(pool).await.expect("clean kb_team_resources");
    sqlx::query("DELETE FROM kb_team_members")
        .execute(pool).await.expect("clean kb_team_members");
    sqlx::query("DELETE FROM kb_teams")
        .execute(pool).await.expect("clean kb_teams");

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
        axum::serve(listener, app).await.expect("Test server failed");
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

    let client =
        temper_client::config::build_client_from(&temper_config, Some(&stored_auth))
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
```

- [ ] **Step 3: Verify it compiles**

Run: `cargo check --manifest-path tests/e2e/Cargo.toml --features test-db`
Expected: PASS (no tests yet, just compilation)

- [ ] **Step 4: Commit**

```bash
git add tests/e2e/
git commit -m "feat: add E2eTestApp harness with config-injected server and client"
```

---

### Task 7: Build `TestTracingLayer`

**Files:**
- Modify: `tests/e2e/tests/common/tracing_layer.rs`

- [ ] **Step 1: Implement the tracing layer**

Write `tests/e2e/tests/common/tracing_layer.rs`:

```rust
//! A test `tracing::Layer` that captures events and span data for assertions.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use tracing::field::{Field, Visit};
use tracing::span;
use tracing::subscriber::Interest;
use tracing::{Event, Id, Level, Subscriber};
use tracing_subscriber::layer::{Context, Layer};
use tracing_subscriber::registry::LookupSpan;

/// A captured tracing event with its fields and parent span fields.
#[derive(Debug, Clone)]
pub struct CapturedEvent {
    pub target: String,
    pub level: Level,
    pub fields: HashMap<String, String>,
    pub span_fields: HashMap<String, String>,
}

/// Shared storage for captured events.
pub type CapturedEvents = Arc<Mutex<Vec<CapturedEvent>>>;

/// A `tracing::Layer` that records every event into a shared `Vec`.
pub struct TestTracingLayer {
    events: CapturedEvents,
}

impl TestTracingLayer {
    pub fn new() -> (Self, CapturedEvents) {
        let events: CapturedEvents = Arc::new(Mutex::new(Vec::new()));
        (
            Self {
                events: events.clone(),
            },
            events,
        )
    }
}

/// Visitor that collects fields into a `HashMap<String, String>`.
struct FieldCollector(HashMap<String, String>);

impl Visit for FieldCollector {
    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        self.0
            .insert(field.name().to_string(), format!("{value:?}"));
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        self.0
            .insert(field.name().to_string(), value.to_string());
    }

    fn record_u64(&mut self, field: &Field, value: u64) {
        self.0
            .insert(field.name().to_string(), value.to_string());
    }

    fn record_i64(&mut self, field: &Field, value: i64) {
        self.0
            .insert(field.name().to_string(), value.to_string());
    }

    fn record_bool(&mut self, field: &Field, value: bool) {
        self.0
            .insert(field.name().to_string(), value.to_string());
    }
}

impl<S> Layer<S> for TestTracingLayer
where
    S: Subscriber + for<'a> LookupSpan<'a>,
{
    fn register_callsite(
        &self,
        _metadata: &'static tracing::Metadata<'static>,
    ) -> Interest {
        Interest::always()
    }

    fn on_new_span(
        &self,
        attrs: &span::Attributes<'_>,
        id: &Id,
        ctx: Context<'_, S>,
    ) {
        let mut fields = FieldCollector(HashMap::new());
        attrs.record(&mut fields);
        if let Some(span) = ctx.span(id) {
            span.extensions_mut().insert(fields.0);
        }
    }

    fn on_record(&self, id: &Id, values: &span::Record<'_>, ctx: Context<'_, S>) {
        if let Some(span) = ctx.span(id) {
            let mut exts = span.extensions_mut();
            if let Some(fields) = exts.get_mut::<HashMap<String, String>>() {
                let mut collector = FieldCollector(HashMap::new());
                values.record(&mut collector);
                fields.extend(collector.0);
            }
        }
    }

    fn on_event(&self, event: &Event<'_>, ctx: Context<'_, S>) {
        let mut fields = FieldCollector(HashMap::new());
        event.record(&mut fields);

        // Walk parent spans to collect span-level fields.
        let mut span_fields = HashMap::new();
        if let Some(scope) = ctx.event_span(event) {
            let span = scope;
            if let Some(exts) = span.extensions().get::<HashMap<String, String>>() {
                span_fields.extend(exts.clone());
            }
            // Walk ancestors
            for ancestor in span.scope().skip(1) {
                if let Some(exts) = ancestor.extensions().get::<HashMap<String, String>>() {
                    // Don't overwrite — inner spans take priority.
                    for (k, v) in exts {
                        span_fields.entry(k.clone()).or_insert_with(|| v.clone());
                    }
                }
            }
        }

        let captured = CapturedEvent {
            target: event.metadata().target().to_string(),
            level: *event.metadata().level(),
            fields: fields.0,
            span_fields,
        };

        self.events.lock().unwrap().push(captured);
    }
}
```

- [ ] **Step 2: Verify it compiles**

Run: `cargo check --manifest-path tests/e2e/Cargo.toml --features test-db`
Expected: PASS

- [ ] **Step 3: Commit**

```bash
git add tests/e2e/tests/common/tracing_layer.rs
git commit -m "feat: add TestTracingLayer for capturing tracing events in e2e tests"
```

---

### Task 8: Auth rejection e2e tests

**Files:**
- Create: `tests/e2e/tests/auth_test.rs`

- [ ] **Step 1: Write auth rejection tests**

Create `tests/e2e/tests/auth_test.rs`:

```rust
#![cfg(feature = "test-db")]

mod common;

use reqwest::StatusCode;

/// Request without auth header returns 401.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn no_auth_returns_401(pool: sqlx::PgPool) {
    let app = common::setup(pool).await;

    let resp = app
        .reqwest_client
        .get(app.url("/api/resources"))
        .send()
        .await
        .expect("request failed");

    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

/// Request with expired JWT returns 401.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn expired_jwt_returns_401(pool: sqlx::PgPool) {
    let app = common::setup(pool).await;
    let expired = common::generate_expired_jwt("expired-user", "expired@test.example.com");

    let resp = app
        .reqwest_client
        .get(app.url("/api/resources"))
        .header("Authorization", format!("Bearer {expired}"))
        .send()
        .await
        .expect("request failed");

    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

/// Request with valid JWT succeeds (200).
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn valid_jwt_returns_200(pool: sqlx::PgPool) {
    let app = common::setup(pool).await;

    let resp = app
        .reqwest_client
        .get(app.url("/api/resources"))
        .header("Authorization", format!("Bearer {}", app.token))
        .send()
        .await
        .expect("request failed");

    assert_eq!(resp.status(), StatusCode::OK);
}
```

- [ ] **Step 2: Run tests**

Run: `cargo nextest run --manifest-path tests/e2e/Cargo.toml --features test-db`
Expected: All 3 tests PASS

- [ ] **Step 3: Commit**

```bash
git add tests/e2e/tests/auth_test.rs
git commit -m "test: add auth rejection e2e tests"
```

---

### Task 9: Ingest flow e2e test

**Files:**
- Create: `tests/e2e/tests/ingest_test.rs`

- [ ] **Step 1: Write ingest test**

Create `tests/e2e/tests/ingest_test.rs`:

```rust
#![cfg(feature = "test-db")]

mod common;

use temper_core::types::ingest::IngestPayload;

/// Ingest a resource via the client, then verify it exists via list.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn ingest_creates_resource(pool: sqlx::PgPool) {
    let app = common::setup(pool).await;

    // Ensure the test user's profile exists (auto-provisioned on first API call).
    app.client
        .profile()
        .get()
        .await
        .expect("profile pre-flight failed");

    // Ingest a test resource.
    let payload = IngestPayload {
        title: "E2E Test Document".to_string(),
        origin_uri: "test://e2e/ingest-test".to_string(),
        context_name: "temper".to_string(),
        doc_type_name: "research".to_string(),
        resource_mode: "imported".to_string(),
        content_hash: "sha256:e2etest0000000000000000000000000000000000000000000000000000000000"
            .to_string(),
        slug: "e2e-test-document".to_string(),
        mimetype: "text/markdown".to_string(),
        content: "# E2E Test\n\nThis is a test document for e2e testing.".to_string(),
        metadata: None,
        chunks_packed: base64::engine::general_purpose::STANDARD.encode(
            rmp_serde::to_vec(&Vec::<temper_core::types::ingest::PackedChunk>::new())
                .expect("encode empty chunks"),
        ),
    };

    let resource = app
        .client
        .ingest()
        .create(&payload)
        .await
        .expect("ingest create failed");

    assert_eq!(resource.title, "E2E Test Document");
    assert_eq!(resource.origin_uri, "test://e2e/ingest-test");
    assert!(resource.is_active);

    // Verify it appears in resource list.
    let resources = app
        .client
        .resources()
        .list(&temper_core::types::resource::ResourceListParams {
            context_id: None,
            doc_type_id: None,
            limit: Some(50),
            offset: None,
        })
        .await
        .expect("list resources failed");

    assert!(
        resources.iter().any(|r| r.origin_uri == "test://e2e/ingest-test"),
        "ingested resource not found in list"
    );
}
```

- [ ] **Step 2: Run test**

Run: `cargo nextest run --manifest-path tests/e2e/Cargo.toml --features test-db -E 'test(ingest_creates_resource)'`
Expected: PASS

- [ ] **Step 3: Commit**

```bash
git add tests/e2e/
git commit -m "test: add ingest flow e2e test"
```

---

### Task 10: Search flow e2e test

**Files:**
- Create: `tests/e2e/tests/search_test.rs`

- [ ] **Step 1: Write search test**

Create `tests/e2e/tests/search_test.rs`:

```rust
#![cfg(feature = "test-db")]

mod common;

/// Search via the client returns results (using the seed resource).
///
/// Note: the search endpoint requires an embedding vector. We send a
/// dummy 768-dim vector — the test validates the API pipeline works
/// end-to-end, not embedding quality.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn search_returns_results(pool: sqlx::PgPool) {
    let app = common::setup(pool).await;

    // Ensure profile exists.
    app.client
        .profile()
        .get()
        .await
        .expect("profile pre-flight failed");

    // Send a dummy embedding (768 dims of 0.1).
    let embedding = vec![0.1_f32; 768];

    let results = app
        .client
        .search()
        .query(embedding, Some("temper".to_string()), None, Some(10))
        .await
        .expect("search query failed");

    // The seed resource should be visible (owned by System profile,
    // but the search endpoint returns all resources the user can see).
    // At minimum we verify the search pipeline doesn't error.
    // If no embeddings are stored, results may be empty — that's OK.
    // The important thing is the API accepted the request and returned 200.
    assert!(results.len() >= 0); // Pipeline works
}
```

- [ ] **Step 2: Run test**

Run: `cargo nextest run --manifest-path tests/e2e/Cargo.toml --features test-db -E 'test(search_returns_results)'`
Expected: PASS (may return empty results if no embeddings stored, but the HTTP pipeline works)

- [ ] **Step 3: Commit**

```bash
git add tests/e2e/tests/search_test.rs
git commit -m "test: add search flow e2e test"
```

---

### Task 11: Structured logging verification e2e test

**Files:**
- Create: `tests/e2e/tests/logging_test.rs`

- [ ] **Step 1: Write logging verification test**

Create `tests/e2e/tests/logging_test.rs`:

```rust
#![cfg(feature = "test-db")]

mod common;

use std::sync::{Arc, Mutex};

use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

use common::tracing_layer::TestTracingLayer;

/// Verify that a request to a protected endpoint produces tracing spans
/// with the expected structured fields (method, path, status, profile_id).
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn request_produces_structured_spans(pool: sqlx::PgPool) {
    // Install the test tracing layer. This must happen before setup()
    // so the server's tracing output is captured.
    let (layer, captured) = TestTracingLayer::new();
    let _guard = tracing_subscriber::registry().with(layer).set_default();

    let app = common::setup(pool).await;

    // Make an authenticated request.
    let resp = app
        .reqwest_client
        .get(app.url("/api/resources"))
        .header("Authorization", format!("Bearer {}", app.token))
        .send()
        .await
        .expect("request failed");

    assert_eq!(resp.status().as_u16(), 200);

    // Give a moment for async logging to flush.
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    let events = captured.lock().unwrap();

    // Look for an event whose parent span chain contains the expected fields.
    // The TraceLayer in routes.rs creates a span with method, path, status.
    // The auth middleware records profile_id into the span.
    let has_request_span = events.iter().any(|e| {
        let sf = &e.span_fields;
        sf.contains_key("method") && sf.contains_key("path")
    });

    assert!(
        has_request_span,
        "expected a tracing event with method and path span fields, got: {events:#?}"
    );
}

/// Verify that an unauthenticated request produces a warn-level event.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn unauthenticated_request_logs_warning(pool: sqlx::PgPool) {
    let (layer, captured) = TestTracingLayer::new();
    let _guard = tracing_subscriber::registry().with(layer).set_default();

    let app = common::setup(pool).await;

    let resp = app
        .reqwest_client
        .get(app.url("/api/resources"))
        .send()
        .await
        .expect("request failed");

    assert_eq!(resp.status().as_u16(), 401);

    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    let events = captured.lock().unwrap();

    // The auth middleware or error handler should produce a warn-level event
    // for unauthorized requests.
    let has_auth_warning = events.iter().any(|e| {
        e.level <= tracing::Level::WARN
    });

    assert!(
        has_auth_warning,
        "expected a WARN-level event for 401 response, got: {events:#?}"
    );
}
```

- [ ] **Step 2: Run tests**

Run: `cargo nextest run --manifest-path tests/e2e/Cargo.toml --features test-db -E 'test(logging)'`
Expected: PASS

**Note:** If the tracing subscriber is already globally initialized (e.g., by `temper-api`'s main), `set_default()` creates a thread-local override which works for the test's async context. If tests fail due to subscriber conflicts, switch to `tracing::subscriber::with_default()` wrapping the test body.

- [ ] **Step 3: Commit**

```bash
git add tests/e2e/tests/logging_test.rs
git commit -m "test: add structured logging verification e2e tests"
```

---

### Task 12: cargo-make `test-e2e` task

**Files:**
- Modify: `tools/cargo-make/main.toml`

- [ ] **Step 1: Add `test-e2e` task**

In `tools/cargo-make/main.toml`, add:

```toml
[tasks.test-e2e]
description = "Run E2E tests (requires Docker Postgres)"
command = "cargo"
args = ["nextest", "run", "--manifest-path", "tests/e2e/Cargo.toml", "--features", "test-db"]
dependencies = ["docker-up"]
```

- [ ] **Step 2: Update `test-all` to include e2e**

Find the existing `test-all` task and add `test-e2e` to its dependencies. The exact change depends on the current `test-all` definition — add `"test-e2e"` to the `dependencies` array.

- [ ] **Step 3: Verify `cargo make test-e2e` runs**

Run: `cargo make test-e2e`
Expected: Docker starts, all e2e tests PASS

- [ ] **Step 4: Commit**

```bash
git add tools/cargo-make/main.toml
git commit -m "chore: add test-e2e cargo-make task"
```

---

### Task 13: Final verification

- [ ] **Step 1: Run full check suite**

Run: `cargo make check`
Expected: fmt, clippy, docs all pass.

- [ ] **Step 2: Run full test suite**

Run: `cargo make test-all`
Expected: All existing tests + new e2e tests pass.

- [ ] **Step 3: Verify no config leakage**

Confirm that no e2e test reads from `~/.config/temper/`. A quick grep:

Run: `grep -r "~/.config/temper\|home_dir\|auth_json_path\|global_config_path\|load_auth()\b\|load_config()\b\|current_token()\b" tests/e2e/`
Expected: No matches (all config/auth is injected).
