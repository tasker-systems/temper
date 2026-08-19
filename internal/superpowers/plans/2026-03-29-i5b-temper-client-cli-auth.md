# I5b: temper-client Crate + CLI Auth — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build the temper-client crate as the auth-aware HTTP client for the temper cloud API, and implement `temper auth login/logout/status` in the CLI.

**Architecture:** temper-client is a standalone crate with sub-clients (resources, upload, search, profile, events) sharing a base HTTP client with automatic auth injection and retry. OAuth PKCE flow for login, token storage at `~/.config/temper/auth.json`. API request/response types extracted from temper-api into temper-core so both server and client share the same contract.

**Tech Stack:** reqwest + rustls, tokio, serde, hyper (callback server), open (browser launch)

**Spec:** `docs/superpowers/specs/2026-03-29-i5-temper-developer-experience-design.md`

---

### Task 1: Extract API Types to temper-core

The API request/response types currently live in temper-api (services/ and handlers/). The client needs them too. Extract them to temper-core so both crates share the contract.

**Files:**
- Create: `crates/temper-core/src/types/resource.rs`
- Create: `crates/temper-core/src/types/api.rs`
- Modify: `crates/temper-core/src/types/mod.rs`
- Modify: `crates/temper-api/src/services/resource_service.rs` (use from temper-core)
- Modify: `crates/temper-api/src/services/event_service.rs` (use from temper-core)
- Modify: `crates/temper-api/src/services/search_service.rs` (use from temper-core)
- Modify: `crates/temper-api/src/handlers/resources.rs` (use from temper-core)
- Modify: `crates/temper-api/src/handlers/profiles.rs` (use from temper-core)
- Modify: `crates/temper-api/src/handlers/health.rs` (use from temper-core)

- [ ] **Step 1: Create resource.rs in temper-core**

Extract these types from temper-api into `crates/temper-core/src/types/resource.rs`:

```rust
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "db", derive(sqlx::FromRow))]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
pub struct ResourceRow {
    pub id: Uuid,
    pub title: String,
    pub uri: Option<String>,
    pub slug: Option<String>,
    pub mimetype: Option<String>,
    pub owner_profile_id: Uuid,
    pub originator_profile_id: Uuid,
    pub created: DateTime<Utc>,
    pub updated: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::IntoParams))]
pub struct ResourceListParams {
    pub kb_context_id: Option<Uuid>,
    pub kb_doc_type_id: Option<Uuid>,
    pub limit: Option<i64>,
    pub offset: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
pub struct ResourceCreateRequest {
    pub kb_context_id: Uuid,
    pub kb_doc_type_id: Option<Uuid>,
    pub uri: Option<String>,
    pub title: String,
    pub slug: Option<String>,
    pub mimetype: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
pub struct ResourceUpdateRequest {
    pub title: Option<String>,
    pub slug: Option<String>,
    pub mimetype: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
pub struct ContentResponse {
    pub resource_id: Uuid,
    pub markdown: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
pub struct DeleteResponse {
    pub deleted: bool,
}
```

Note: Read the actual types from temper-api first — the exact field names, types, and derives may differ from this sketch. Match them exactly.

- [ ] **Step 2: Create api.rs in temper-core**

Extract remaining API types into `crates/temper-core/src/types/api.rs`:

```rust
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HealthResponse {
    pub status: String,
    pub version: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventRow {
    pub id: Uuid,
    pub profile_id: Uuid,
    pub client_id: Option<String>,
    pub kb_context_id: Option<Uuid>,
    pub resource_id: Option<Uuid>,
    pub event_type: String,
    pub payload: Option<serde_json::Value>,
    pub created: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventListParams {
    pub resource_id: Option<Uuid>,
    pub event_type: Option<String>,
    pub limit: Option<i64>,
    pub offset: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchParams {
    pub q: String,
    pub kb_context_id: Option<Uuid>,
    pub limit: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResultRow {
    pub resource_id: Uuid,
    pub title: String,
    pub uri: Option<String>,
    pub snippet: Option<String>,
    pub score: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProfileUpdateRequest {
    pub display_name: Option<String>,
    pub preferences: Option<serde_json::Value>,
    pub vault_config: Option<serde_json::Value>,
}
```

Again — read the actual types first and match exactly.

- [ ] **Step 3: Register new modules in types/mod.rs**

Add `pub mod resource;` and `pub mod api;` to `crates/temper-core/src/types/mod.rs` and re-export all types.

- [ ] **Step 4: Update temper-api to use types from temper-core**

In each temper-api service and handler file, replace the local type definitions with imports from temper-core. The types should be identical so this is a mechanical change. If a type uses `sqlx::FromRow`, gate it behind `#[cfg_attr(feature = "db", ...)]` in temper-core so the client doesn't need sqlx.

- [ ] **Step 5: Verify compilation**

```bash
cargo check --all-features
cargo test --all-features
```

- [ ] **Step 6: Commit**

```bash
git add -A
git commit -m "refactor: extract API request/response types to temper-core

ResourceRow, ResourceListParams, ResourceCreateRequest, ResourceUpdateRequest,
ContentResponse, DeleteResponse, EventRow, EventListParams, SearchParams,
SearchResultRow, ProfileUpdateRequest, HealthResponse — shared between
temper-api and temper-client."
```

---

### Task 2: Add Dependencies to temper-client

**Files:**
- Modify: `crates/temper-client/Cargo.toml`

- [ ] **Step 1: Add dependencies**

```toml
[package]
name = "temper-client"
version = "0.1.0"
edition = "2021"
description = "Auth-aware HTTP client for the temper cloud API"

[dependencies]
temper-core = { path = "../temper-core" }
reqwest = { version = "0.12", features = ["json", "rustls-tls"], default-features = false }
tokio = { version = "1", features = ["rt", "macros", "sync", "net"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
chrono = { version = "0.4", features = ["serde"] }
uuid = { version = "1", features = ["v7", "serde"] }
thiserror = "2"
tracing = "0.1"
url = "2"
open = "5"
rand = "0.8"
sha2 = "0.10"
base64 = "0.22"
dirs = "5"
hyper = { version = "1", features = ["server", "http1"] }
hyper-util = { version = "0.1", features = ["tokio"] }
http-body-util = "0.1"

[dev-dependencies]
tokio = { version = "1", features = ["rt-multi-thread", "macros"] }
```

Check workspace Cargo.toml for pinned versions — use those instead of the versions above if they exist.

- [ ] **Step 2: Verify it compiles**

```bash
cargo check -p temper-client
```

- [ ] **Step 3: Commit**

```bash
git add -A
git commit -m "deps: add reqwest, tokio, hyper, and supporting deps to temper-client"
```

---

### Task 3: ClientError and Base HTTP Client

**Files:**
- Create: `crates/temper-client/src/error.rs`
- Create: `crates/temper-client/src/http.rs`
- Modify: `crates/temper-client/src/lib.rs`

- [ ] **Step 1: Create error.rs**

```rust
use std::time::Duration;

#[derive(Debug, thiserror::Error)]
pub enum ClientError {
    #[error("not authenticated — run `temper auth login`")]
    NotAuthenticated,

    #[error("token expired")]
    TokenExpired,

    #[error("forbidden")]
    Forbidden,

    #[error("{resource} not found")]
    NotFound { resource: String },

    #[error("conflict: {message}")]
    Conflict { message: String },

    #[error("rate limited — retry after {retry_after:?}")]
    RateLimited { retry_after: Duration },

    #[error("server error ({status}): {message}")]
    Server { status: u16, message: String },

    #[error("network error: {0}")]
    Network(#[from] reqwest::Error),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("{0}")]
    Other(String),
}

pub type Result<T> = std::result::Result<T, ClientError>;
```

- [ ] **Step 2: Create http.rs — base HTTP client**

The base client handles:
- Constructing a reqwest::Client with auth header injection
- Base URL from config or `TEMPER_API_URL` env var
- `X-Temper-Client-Id` header from device identity
- Response status → ClientError mapping
- Retry on 429/503 (simple: 1 retry with backoff)

```rust
use crate::error::{ClientError, Result};
use reqwest::{Client, RequestBuilder, Response, StatusCode};
use std::time::Duration;

pub struct HttpClient {
    client: Client,
    base_url: String,
    device_id: Option<String>,
}

impl HttpClient {
    pub fn new(base_url: &str, device_id: Option<String>) -> Result<Self> { ... }

    pub fn get(&self, path: &str) -> RequestBuilder { ... }
    pub fn post(&self, path: &str) -> RequestBuilder { ... }
    pub fn patch(&self, path: &str) -> RequestBuilder { ... }
    pub fn delete(&self, path: &str) -> RequestBuilder { ... }
    pub fn put(&self, path: &str) -> RequestBuilder { ... }

    /// Add auth token if available, execute request, map errors
    pub async fn send(&self, req: RequestBuilder, token: Option<&str>) -> Result<Response> { ... }

    /// send + deserialize JSON body
    pub async fn send_json<T: serde::de::DeserializeOwned>(
        &self, req: RequestBuilder, token: Option<&str>
    ) -> Result<T> { ... }
}
```

Map response status codes:
- 401 → `ClientError::NotAuthenticated`
- 403 → `ClientError::Forbidden`
- 404 → `ClientError::NotFound` (parse body for resource)
- 409 → `ClientError::Conflict` (parse body for message)
- 429 → `ClientError::RateLimited` (parse Retry-After header)
- 5xx → `ClientError::Server`

- [ ] **Step 3: Update lib.rs with module declarations**

```rust
pub mod error;
pub mod http;
```

- [ ] **Step 4: Write tests**

Test error mapping with a mock or by testing the mapping logic directly (unit test the status-to-error conversion function, no actual HTTP calls needed).

- [ ] **Step 5: Verify and commit**

```bash
cargo check -p temper-client && cargo test -p temper-client
git add -A
git commit -m "feat(temper-client): ClientError and base HTTP client with auth/retry"
```

---

### Task 4: Auth Module — Token Storage and Refresh

**Files:**
- Create: `crates/temper-client/src/auth.rs`
- Modify: `crates/temper-client/src/lib.rs`

- [ ] **Step 1: Create auth.rs**

Auth handles:
- Loading/saving tokens from `~/.config/temper/auth.json`
- Token refresh check (is expired within 5 min?)
- Token refresh request (POST to token_url with refresh_token grant)
- Auth status (provider, email, expiry)
- Logout (delete auth.json)

```rust
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use crate::error::{ClientError, Result};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredAuth {
    pub provider: String,
    pub access_token: String,
    pub refresh_token: Option<String>,
    pub expires_at: DateTime<Utc>,
    pub profile_id: Option<uuid::Uuid>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthStatus {
    pub authenticated: bool,
    pub provider: Option<String>,
    pub expires_at: Option<DateTime<Utc>>,
    pub profile_id: Option<uuid::Uuid>,
}

/// Load stored auth from ~/.config/temper/auth.json
pub fn load_auth() -> Result<Option<StoredAuth>> { ... }

/// Save auth to ~/.config/temper/auth.json
pub fn save_auth(auth: &StoredAuth) -> Result<()> { ... }

/// Delete auth.json (logout)
pub fn clear_auth() -> Result<()> { ... }

/// Get auth status
pub fn auth_status() -> Result<AuthStatus> { ... }

/// Check if token needs refresh (within 5 min of expiry)
pub fn needs_refresh(auth: &StoredAuth) -> bool { ... }

/// Refresh the access token using refresh_token grant
pub async fn refresh_token(auth: &StoredAuth, token_url: &str, client_id: &str) -> Result<StoredAuth> { ... }

/// Get a valid access token, refreshing if needed
pub async fn get_valid_token(token_url: &str, client_id: &str) -> Result<String> { ... }
```

The `auth_json_path()` helper returns `~/.config/temper/auth.json` using the `dirs` crate.

- [ ] **Step 2: Write tests**

- Test `needs_refresh` with various expiry times
- Test `save_auth` / `load_auth` roundtrip (use tempdir)
- Test `clear_auth` removes file
- Test `auth_status` when no file exists

- [ ] **Step 3: Verify and commit**

```bash
cargo check -p temper-client && cargo test -p temper-client
git add -A
git commit -m "feat(temper-client): auth token storage, refresh, and status"
```

---

### Task 5: OAuth PKCE Login Flow

**Files:**
- Create: `crates/temper-client/src/login.rs`
- Modify: `crates/temper-client/src/lib.rs`

- [ ] **Step 1: Create login.rs**

The login flow:
1. Read provider config (authorize_url, token_url, client_id, scopes)
2. Generate PKCE code_verifier (random 43-128 char string) and code_challenge (SHA256 + base64url)
3. Generate random `state` parameter
4. Start ephemeral HTTP server on random port (127.0.0.1:0)
5. Open browser to authorize_url with: response_type=code, client_id, redirect_uri, scope, code_challenge, code_challenge_method=S256, state
6. Wait for callback on local server — extract `code` and `state` from query params
7. Verify state matches
8. POST to token_url with: grant_type=authorization_code, code, redirect_uri, client_id, code_verifier
9. Parse response: access_token, refresh_token, expires_in
10. Save to auth.json via `save_auth()`
11. Return success

```rust
use crate::error::Result;
use crate::auth::StoredAuth;

#[derive(Debug, Clone)]
pub struct OAuthConfig {
    pub authorize_url: String,
    pub token_url: String,
    pub client_id: String,
    pub scopes: Vec<String>,
}

/// Run the full OAuth PKCE login flow
pub async fn login(config: &OAuthConfig) -> Result<StoredAuth> { ... }
```

For the local HTTP callback server, use `hyper` to bind a TCP listener on `127.0.0.1:0`, accept exactly one request, extract the code, respond with a success HTML page, then shut down.

- [ ] **Step 2: Write PKCE helper tests**

Test code_verifier generation (length, charset), code_challenge computation (known test vector), state generation.

- [ ] **Step 3: Verify and commit**

```bash
cargo check -p temper-client && cargo test -p temper-client
git add -A
git commit -m "feat(temper-client): OAuth PKCE login flow with local callback server"
```

---

### Task 6: TemperClient and Sub-Clients

**Files:**
- Create: `crates/temper-client/src/resources.rs`
- Create: `crates/temper-client/src/search.rs`
- Create: `crates/temper-client/src/profile.rs`
- Create: `crates/temper-client/src/events.rs`
- Create: `crates/temper-client/src/upload.rs`
- Modify: `crates/temper-client/src/lib.rs`

- [ ] **Step 1: Create the main TemperClient struct in lib.rs**

```rust
pub struct TemperClient {
    http: http::HttpClient,
    oauth_config: Option<login::OAuthConfig>,
    token_url: Option<String>,
    client_id: Option<String>,
}

impl TemperClient {
    pub fn new(base_url: &str, device_id: Option<String>) -> error::Result<Self> { ... }

    pub fn with_oauth(mut self, config: login::OAuthConfig) -> Self { ... }

    /// Get a valid token, refreshing if needed
    async fn token(&self) -> error::Result<String> { ... }

    pub fn resources(&self) -> resources::ResourceClient<'_> { ... }
    pub fn search(&self) -> search::SearchClient<'_> { ... }
    pub fn profile(&self) -> profile::ProfileClient<'_> { ... }
    pub fn events(&self) -> events::EventClient<'_> { ... }
    pub fn upload(&self) -> upload::UploadClient<'_> { ... }

    pub async fn auth_login(&self) -> error::Result<auth::StoredAuth> { ... }
    pub fn auth_logout(&self) -> error::Result<()> { ... }
    pub fn auth_status(&self) -> error::Result<auth::AuthStatus> { ... }
}
```

- [ ] **Step 2: Create resources.rs**

```rust
use crate::{error::Result, http::HttpClient};
use temper_core::types::{
    resource::{ResourceRow, ResourceListParams, ResourceCreateRequest, ResourceUpdateRequest, ContentResponse, DeleteResponse},
};
use uuid::Uuid;

pub struct ResourceClient<'a> {
    http: &'a HttpClient,
    token_fn: &'a dyn Fn() -> /* future producing token */,
}

impl<'a> ResourceClient<'a> {
    pub async fn list(&self, params: &ResourceListParams) -> Result<Vec<ResourceRow>> { ... }
    pub async fn get(&self, id: Uuid) -> Result<ResourceRow> { ... }
    pub async fn create(&self, req: &ResourceCreateRequest) -> Result<ResourceRow> { ... }
    pub async fn update(&self, id: Uuid, req: &ResourceUpdateRequest) -> Result<ResourceRow> { ... }
    pub async fn delete(&self, id: Uuid) -> Result<DeleteResponse> { ... }
    pub async fn content(&self, id: Uuid) -> Result<ContentResponse> { ... }
}
```

The sub-client borrows from TemperClient and calls `self.http.send_json()` with the token. Design the borrow so it's ergonomic — the sub-client needs access to both the HTTP client and a way to get a valid token. A simple approach: pass `&HttpClient` and `token: &str` to each method, where the caller (TemperClient method) gets the token first. Or store a reference to the parent.

Use whatever pattern is cleanest. The API surface from the caller's perspective should be:
```rust
let resources = client.resources().list(&params).await?;
```

- [ ] **Step 3: Create search.rs**

```rust
pub struct SearchClient<'a> { ... }

impl<'a> SearchClient<'a> {
    pub async fn query(&self, params: &SearchParams) -> Result<Vec<SearchResultRow>> { ... }
}
```

- [ ] **Step 4: Create profile.rs**

```rust
pub struct ProfileClient<'a> { ... }

impl<'a> ProfileClient<'a> {
    pub async fn get(&self) -> Result<Profile> { ... }
    pub async fn update(&self, req: &ProfileUpdateRequest) -> Result<Profile> { ... }
    pub async fn auth_links(&self) -> Result<Vec<ProfileAuthLink>> { ... }
}
```

- [ ] **Step 5: Create events.rs**

```rust
pub struct EventClient<'a> { ... }

impl<'a> EventClient<'a> {
    pub async fn list(&self, params: &EventListParams) -> Result<Vec<EventRow>> { ... }
}
```

- [ ] **Step 6: Create upload.rs (stub)**

The upload client handles the two-step flow. For now, create the struct and method signatures. The actual multipart upload to the TypeScript endpoint will be implemented when we have the full flow tested.

```rust
pub struct UploadClient<'a> { ... }

impl<'a> UploadClient<'a> {
    /// Tier 1: upload file content, get resource_id back
    pub async fn add(&self, resource_id: Uuid, content: &[u8], filename: &str) -> Result<UploadResponse> { ... }
}
```

- [ ] **Step 7: Update lib.rs with all modules**

```rust
pub mod auth;
pub mod error;
pub mod events;
pub mod http;
pub mod login;
pub mod profile;
pub mod resources;
pub mod search;
pub mod upload;
```

- [ ] **Step 8: Verify compilation**

```bash
cargo check -p temper-client
```

- [ ] **Step 9: Commit**

```bash
git add -A
git commit -m "feat(temper-client): TemperClient with typed sub-clients for all API endpoints"
```

---

### Task 7: CLI Auth Commands

**Files:**
- Create: `crates/temper-cli/src/commands/auth.rs`
- Modify: `crates/temper-cli/src/commands/mod.rs`
- Modify: `crates/temper-cli/src/cli.rs`
- Modify: `crates/temper-cli/src/main.rs`
- Modify: `crates/temper-cli/Cargo.toml` (add temper-client dep + tokio)

- [ ] **Step 1: Add temper-client and tokio to CLI Cargo.toml**

```toml
temper-client = { path = "../temper-client" }
tokio = { version = "1", features = ["rt-multi-thread", "macros"] }
```

- [ ] **Step 2: Add AuthAction enum to cli.rs**

```rust
/// Authentication commands
#[derive(Subcommand, Debug)]
pub enum AuthAction {
    /// Log in via browser OAuth
    Login,
    /// Clear stored credentials
    Logout,
    /// Show current auth status
    Status,
}
```

Add to Commands enum:
```rust
/// Authenticate with temper cloud
Auth {
    #[command(subcommand)]
    action: AuthAction,
},
```

- [ ] **Step 3: Create commands/auth.rs**

```rust
use temper_client::{auth, login::OAuthConfig, TemperClient};

pub fn login() -> Result<()> {
    // Read provider config from ~/.config/temper/config.toml
    // Construct OAuthConfig
    // Create tokio runtime, run login flow
    // Print success message with profile info as JSON
}

pub fn logout() -> Result<()> {
    auth::clear_auth()?;
    println!("{{\"status\": \"logged_out\"}}");
    Ok(())
}

pub fn status() -> Result<()> {
    let status = auth::auth_status()?;
    println!("{}", serde_json::to_string_pretty(&status)?);
    Ok(())
}
```

- [ ] **Step 4: Add auth dispatch to main.rs**

```rust
Commands::Auth { action } => match action {
    AuthAction::Login => commands::auth::login(),
    AuthAction::Logout => commands::auth::logout(),
    AuthAction::Status => commands::auth::status(),
},
```

- [ ] **Step 5: Add module to commands/mod.rs**

```rust
pub mod auth;
```

- [ ] **Step 6: Verify compilation and test**

```bash
cargo check -p temper-cli && cargo test -p temper-cli
```

- [ ] **Step 7: Commit**

```bash
git add -A
git commit -m "feat(temper-cli): temper auth login/logout/status commands"
```

---

### Task 8: Provider Config in config.toml

**Files:**
- Modify: `crates/temper-cli/src/config.rs` (or create a new cloud config module)
- Modify: `crates/temper-cli/src/commands/auth.rs` (load config)

- [ ] **Step 1: Add auth provider config types**

Either in temper-client or temper-cli config, add types for reading the provider config from `~/.config/temper/config.toml`:

```toml
[auth]
provider = "neon_auth"

[auth.providers.neon_auth]
authorize_url = "https://auth.neon.tech/authorize"
token_url = "https://auth.neon.tech/token"
client_id = "temper-cli"
scopes = ["openid", "email", "profile"]
```

Create a `CloudConfig` struct (or extend existing) that deserializes this.

- [ ] **Step 2: Update auth login to load from config**

The `commands/auth.rs` login function should:
1. Read `~/.config/temper/config.toml`
2. Find the active provider
3. Construct `OAuthConfig` from the provider settings
4. Pass to `login()` flow

- [ ] **Step 3: Read TEMPER_API_URL from config or env**

The base URL for the TemperClient should come from:
1. `TEMPER_API_URL` env var (highest priority)
2. `~/.config/temper/config.toml` `[cloud] api_url = "..."` (fallback)
3. Default: `https://temperkb.io` (production)

- [ ] **Step 4: Verify and commit**

```bash
cargo check -p temper-cli && cargo test -p temper-cli
git add -A
git commit -m "feat(temper-cli): auth provider config from ~/.config/temper/config.toml"
```

---

### Task 9: Integration Tests

**Files:**
- Create: `crates/temper-client/tests/client_test.rs`

- [ ] **Step 1: Write integration tests gated behind a feature flag**

Add to `crates/temper-client/Cargo.toml`:
```toml
[features]
integration-tests = []
```

Create `tests/client_test.rs` gated behind `#[cfg(feature = "integration-tests")]`:

```rust
#[cfg(feature = "integration-tests")]
mod integration {
    use temper_client::TemperClient;

    // These tests require:
    // 1. A running API (TEMPER_API_URL or default)
    // 2. Valid auth credentials (auth.json)

    #[tokio::test]
    async fn test_health() {
        let client = TemperClient::new(&api_url(), None).unwrap();
        // Health doesn't need auth
        // Hit /api/health and verify response
    }

    #[tokio::test]
    async fn test_profile_get() {
        let client = authenticated_client();
        let profile = client.profile().get().await.unwrap();
        assert!(!profile.display_name.is_empty());
    }

    #[tokio::test]
    async fn test_resource_crud() {
        let client = authenticated_client();
        // Create, get, update, delete a test resource
    }

    #[tokio::test]
    async fn test_search() {
        let client = authenticated_client();
        let results = client.search().query(&SearchParams { q: "test".into(), ..Default::default() }).await.unwrap();
        // Just verify it doesn't error
    }
}
```

- [ ] **Step 2: Verify unit tests still pass**

```bash
cargo test -p temper-client
```

(Integration tests won't run without the feature flag)

- [ ] **Step 3: Commit**

```bash
git add -A
git commit -m "test(temper-client): integration test suite (gated behind feature flag)"
```

---

### Task 10: Final Verification

- [ ] **Step 1: Full workspace check**

```bash
cargo check --all-features
cargo test --all-features
cargo clippy --all-features -- -D warnings
cargo make check
```

- [ ] **Step 2: Verify temper auth commands exist**

```bash
cargo install --path crates/temper-cli --force
temper auth status
temper auth --help
```

- [ ] **Step 3: Commit any fixes**

```bash
git add -A
git commit -m "fix: final cleanup for I5b temper-client + CLI auth"
```
