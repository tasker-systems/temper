# I3: temper-api — Axum Server Design Spec

**Date:** 2026-03-27
**Ticket:** 2026-03-27-i3-temper-api-axum-server-with-auth-middleware
**Depends on:** I1 (workspace restructure), I2 (Neon deployment)
**Reference:** R5 API contract spec, tasker-core web layer patterns

## Overview

Build the temper-api crate as a standalone axum HTTP server implementing the R5 API contract. Platform-agnostic: runs locally via `cargo run -p temper-api` and exports `create_app(state) -> Router` for composition by temper-cloud (Vercel adapter). Full cut: auth middleware, resource CRUD, profiles, events, search, and OpenAPI documentation.

## Design Decisions

| Area | Decision |
|------|----------|
| Architecture | Thin handlers / fat services (tasker-core pattern) |
| Entry point | `create_app(state) -> Router` factory, composable by temper-cloud |
| Auth model | JWT identity resolution only — no permissions in claims, access control in SQL |
| Auto-provisioning | First API call from new identity creates profile + auth link |
| Error handling | `ApiError` enum implementing `IntoResponse` with consistent JSON |
| OpenAPI | utoipa with `#[derive(ToSchema)]` on temper-core types behind `web-api` feature |
| Test DB | Workspace Docker Postgres on port 5437, `temper_test` database, `test-db` feature gate |
| Search modes | Semantic implemented; keyword and graph stub to semantic initially |

---

## 1. File Layout

```
crates/temper-api/
├── Cargo.toml
├── src/
│   ├── lib.rs                  # create_app(state) -> Router, re-exports
│   ├── main.rs                 # Binary: load config, create pool, serve
│   ├── config.rs               # ApiConfig from environment
│   ├── state.rs                # AppState: PgPool, JwksKeyStore, ApiConfig
│   ├── error.rs                # ApiError implementing IntoResponse
│   ├── middleware/
│   │   ├── mod.rs
│   │   └── auth.rs             # JWT verify → profile resolve → AuthenticatedProfile extractor
│   ├── routes.rs               # Route registration: public + protected
│   ├── handlers/
│   │   ├── mod.rs
│   │   ├── health.rs           # GET /api/health
│   │   ├── resources.rs        # Resource CRUD (6 endpoints)
│   │   ├── profiles.rs         # Profile endpoints (3 endpoints)
│   │   ├── events.rs           # GET /api/events
│   │   └── search.rs           # GET /api/search
│   ├── services/
│   │   ├── mod.rs
│   │   ├── resource_service.rs # CRUD with access control scoping
│   │   ├── profile_service.rs  # Lookup, auto-provision, email reconciliation
│   │   ├── event_service.rs    # Time-bounded visibility queries
│   │   └── search_service.rs   # Semantic search with access scoping
│   └── openapi.rs              # ApiDoc, SecurityAddon, Swagger UI routes
├── tests/
│   ├── common/
│   │   ├── mod.rs              # Test helpers: server bootstrap, test JWT, seed
│   │   └── fixtures.rs         # Seed profiles, resources, teams, chunks
│   ├── health_test.rs
│   ├── auth_test.rs
│   ├── resources_test.rs
│   ├── profiles_test.rs
│   ├── events_test.rs
│   └── search_test.rs
└── .env.template
```

---

## 2. Application State & Configuration

### ApiConfig

```rust
pub struct ApiConfig {
    pub database_url: String,
    pub jwks_url: String,
    pub auth_issuer: String,
    pub auth_audience: Option<String>,
    pub cors_origins: Vec<String>,
    pub port: u16,  // default 3000
}
```

All values from environment variables. `.env.template` documents them.

### AppState

```rust
#[derive(Clone)]
pub struct AppState {
    pool: PgPool,
    jwks_store: Arc<JwksKeyStore>,
    config: Arc<ApiConfig>,
}
```

Single pool — Neon's connection pooler handles read/write routing. No circuit breaker initially.

### JwksKeyStore

Fetches EdDSA public keys from Neon Auth JWKS endpoint. Cache with TTL (default 1 hour). Refetch on cache miss or expiry. Simple implementation — no thundering herd protection needed at temper's scale.

### create_app Factory

```rust
pub fn create_app(state: AppState) -> Router {
    let public = Router::new()
        .route("/api/health", get(health::health_check));

    let protected = Router::new()
        .route("/api/resources", get(resources::list).post(resources::create))
        .route("/api/resources/:id", get(resources::get).patch(resources::update).delete(resources::delete))
        .route("/api/resources/:id/content", get(resources::get_content))
        .route("/api/profile", get(profiles::get).patch(profiles::update))
        .route("/api/profile/auth-links", get(profiles::list_auth_links))
        .route("/api/events", get(events::list))
        .route("/api/search", get(search::search))
        .layer(middleware::from_fn_with_state(state.clone(), auth::require_auth));

    Router::new()
        .merge(public)
        .merge(protected)
        .merge(openapi::docs_routes())
        .layer(TraceLayer::new_for_http())
        .layer(cors_layer(&state.config))
        .with_state(state)
}
```

This is the seam temper-cloud composes — it receives the Router and bridges it to Vercel's function interface.

---

## 3. Auth Middleware & Profile Resolution

### Flow

1. Extract `Authorization: Bearer <jwt>` header
2. Verify JWT signature via JWKS (EdDSA/Ed25519 from Neon Auth)
3. Validate claims: issuer, expiry
4. Extract: `sub` (external user ID), `email`
5. Look up `kb_profile_auth_links` by `(auth_provider='neon_auth', auth_provider_user_id=sub)`
6. If found → load profile from `kb_profiles`
7. If not found → auto-provision:
   - Check email reconciliation: existing auth_link with same email?
   - If yes → link new provider to existing profile
   - If no → create new profile + auth_link
8. Extract optional `X-Temper-Client-Id` header
9. Inject `AuthenticatedProfile` into request extensions

### AuthenticatedProfile Extractor

```rust
pub struct AuthenticatedProfile {
    pub profile: Profile,
    pub claims: AuthClaims,
    pub client_id: Option<String>,
}

impl<S> FromRequestParts<S> for AuthenticatedProfile {
    // Extract from request extensions, return 401 if missing
}
```

Handlers declare `auth: AuthenticatedProfile` as a parameter. No explicit permission checks — access control is in the SQL queries via `resources_visible_to(auth.profile.id)`.

### Auto-Provisioning

First request from a new Neon Auth identity creates:
1. `kb_profiles` row: display_name from email prefix, is_active = true
2. `kb_profile_auth_links` row: linking auth_provider + auth_provider_user_id to profile

Email reconciliation: if another auth_link exists with the same email, link to that existing profile instead of creating a new one. This supports the multi-provider scenario (Google + GitHub with same email = same temper profile).

---

## 4. Error Handling

### ApiError

```rust
#[derive(Debug, thiserror::Error)]
pub enum ApiError {
    NotFound,
    Unauthorized(String),
    Forbidden,
    BadRequest(String),
    Conflict(String),
    Internal(String),
}
```

### HTTP Mapping

| Variant | Status | Code |
|---------|--------|------|
| NotFound | 404 | NOT_FOUND |
| Unauthorized | 401 | UNAUTHORIZED |
| Forbidden | 403 | FORBIDDEN |
| BadRequest | 400 | BAD_REQUEST |
| Conflict | 409 | CONFLICT |
| Internal | 500 | INTERNAL_ERROR |

### Response Shape

```json
{ "error": { "code": "NOT_FOUND", "message": "Resource not found" } }
```

### Automatic Conversions

- `From<sqlx::Error>`: RowNotFound→404, UniqueViolation→409, PoolTimedOut→500
- `From<jsonwebtoken::errors::Error>`: ExpiredSignature→401, InvalidToken→401
- `From<serde_json::Error>`: →400

### ApiResult

```rust
pub type ApiResult<T> = Result<T, ApiError>;
```

---

## 5. Handlers & Services

### Handler Pattern

Thin — extract params, call service, map to JSON:

```rust
pub async fn list(
    State(state): State<AppState>,
    auth: AuthenticatedProfile,
    Query(params): Query<ResourceListParams>,
) -> ApiResult<Json<Vec<ResourceResponse>>> {
    state.resource_service()
        .list_visible(auth.profile.id, params)
        .await
        .map(Json)
        .map_err(Into::into)
}
```

### Service Layer

Services take `&PgPool` and compose SQL with access control:

**ResourceService:**
- `list_visible(profile_id, params)` → `resources_visible_to` CTE + context/doc_type filters
- `get_visible(profile_id, resource_id)` → single resource with access check
- `get_content(profile_id, resource_id)` → reconstitute markdown from `kb_current_chunks`
- `create(profile_id, request)` → insert with originator = owner = profile_id
- `update(profile_id, resource_id, request)` → `can_modify_resource` check, then update
- `delete(profile_id, resource_id)` → `can_modify_resource` check, soft-delete

**ProfileService:**
- `get_by_id(profile_id)` → direct lookup
- `update(profile_id, request)` → update preferences, vault_config, display_name
- `list_auth_links(profile_id)` → linked providers
- `resolve_from_claims(claims)` → auth middleware calls this: lookup or auto-provision

**EventService:**
- `list_visible(profile_id, params)` → time-bounded resource visibility + actor-own

**SearchService:**
- `search(profile_id, query, mode, filters)` → `resources_visible_to` CTE + pgvector `<=>` cosine
- Keyword and graph modes: return semantic results initially, evolve later

### Request/Response Types

Request types defined in temper-api (handler-specific params):
- `ResourceListParams`: context, doc_type, limit, offset
- `ResourceCreateRequest`: title, context, doc_type, content, tags
- `ResourceUpdateRequest`: title, tags (partial update)
- `SearchParams`: q, mode, context, doc_type, team, depth, limit

Response types: reuse temper-core types where possible (Profile, Team, etc.), add API-specific wrappers (ResourceResponse with access_level, SearchResultResponse with score+snippet).

---

## 6. OpenAPI / Swagger

### utoipa Integration

temper-core types get `#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]` — enabled by temper-api, ignored by temper-cli.

temper-api's `openapi.rs`:

```rust
#[derive(OpenApi)]
#[openapi(
    modifiers(&SecurityAddon),
    paths(
        handlers::health::health_check,
        handlers::resources::list,
        handlers::resources::get,
        // ... all handlers
    ),
    components(schemas(
        ResourceResponse,
        SearchParams,
        // ... all types
    )),
    tags(
        (name = "resources", description = "Resource CRUD operations"),
        (name = "profiles", description = "Profile management"),
        (name = "events", description = "Event stream"),
        (name = "search", description = "Unified search"),
    ),
    info(title = "Temper Cloud API", version = "1.0.0"),
)]
pub struct ApiDoc;
```

SecurityAddon adds `bearer_auth` (EdDSA JWT) scheme.

Swagger UI at `/api-docs/ui`, OpenAPI JSON at `/api-docs/openapi.json`.

---

## 7. Test Infrastructure

### Database

Uses the existing workspace Docker Postgres on port 5437 (`docker-compose.yml`). Tests create/use `temper_test` database alongside `temper_development`.

`.env.template`:
```
DATABASE_URL=postgresql://temper:temper@localhost:5437/temper_test
```

### Feature Gate

All integration tests behind `#[cfg(feature = "test-db")]`. Unit tests (no DB) always run.

```
cargo test -p temper-api                    # unit tests only
cargo test -p temper-api --features test-db # full suite with Postgres
```

### Test Helpers

**`tests/common/mod.rs`:**
- `setup_test_db()` → create pool, run migrations, seed fixtures
- `create_test_app()` → setup_test_db + create_app with test config
- `generate_test_jwt(profile_id, email)` → sign with known Ed25519 key pair
- `TestJwksServer` → tiny axum server on random port serving static JWKS (the test public key)

**`tests/common/fixtures.rs`:**
- System + Anonymous profiles (from migration seed)
- Test user profile with neon_auth link
- Second test profile (for access control tests)
- Resources across contexts with chunks + dummy 768-dim embeddings
- Team with members at different roles + shared resources at different access levels

### Test Categories

- **health_test.rs**: unauthenticated health check returns 200
- **auth_test.rs**: valid JWT → 200, expired → 401, missing → 401, auto-provision on first request
- **resources_test.rs**: CRUD scoped by ownership, soft-delete, content reconstitution
- **profiles_test.rs**: get/update profile, list auth links
- **events_test.rs**: visibility scoping, actor-own events
- **search_test.rs**: semantic search returns results scoped by visibility

---

## 8. Dependencies

```toml
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
utoipa = { version = "5", features = ["axum_extras"] }
utoipa-swagger-ui = { version = "8", features = ["axum"] }

[features]
test-db = []

[dev-dependencies]
tempfile = "3"
```

---

## Endpoints Summary

| Method | Endpoint | Auth | Handler | Description |
|--------|----------|------|---------|-------------|
| GET | /api/health | No | health::health_check | Liveness check |
| GET | /api/resources | Yes | resources::list | List visible resources |
| GET | /api/resources/:id | Yes | resources::get | Get resource metadata |
| GET | /api/resources/:id/content | Yes | resources::get_content | Reconstitute markdown |
| POST | /api/resources | Yes | resources::create | Create resource |
| PATCH | /api/resources/:id | Yes | resources::update | Update metadata |
| DELETE | /api/resources/:id | Yes | resources::delete | Soft-delete |
| GET | /api/profile | Yes | profiles::get | Current profile |
| PATCH | /api/profile | Yes | profiles::update | Update profile |
| GET | /api/profile/auth-links | Yes | profiles::list_auth_links | Linked providers |
| GET | /api/events | Yes | events::list | Event stream |
| GET | /api/search | Yes | search::search | Unified search |
| GET | /api-docs/ui | No | openapi::docs_routes | Swagger UI |
| GET | /api-docs/openapi.json | No | openapi::docs_routes | OpenAPI spec |
