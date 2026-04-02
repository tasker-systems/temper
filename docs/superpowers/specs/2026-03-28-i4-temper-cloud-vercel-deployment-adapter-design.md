# I4: temper-cloud — Vercel Deployment Adapter

**Date:** 2026-03-28
**Ticket:** 2026-03-27-i4-temper-cloud-vercel-deployment-adapter
**Branch:** jcoletaylor/temper-cloud
**Scope:** feature

## Summary

Build `temper-cloud` as a thin Rust binary that wraps `temper-api` for Vercel serverless deployment. Deploy to Vercel and validate the full stack: Vercel → temper-cloud → temper-api → Neon. Prepare the R2 file storage schema and configuration for the follow-up I4a integration ticket.

## Architecture

### Crate Relationship

```
temper-cloud (binary)
  └── temper-api (library)
        ├── create_app(AppState) -> Router
        ├── ApiConfig::from_env()
        └── AppState::new(pool, jwks_store, config)
```

`temper-cloud` is a standalone binary crate that depends on `temper-api` as a library. It constructs the same `AppState` and calls `create_app()`, then bridges the axum Router to Vercel's serverless function interface via `VercelLayer` and `vercel_runtime::run()`. The only differences from `temper-api`'s own `main.rs` are: no migrations, and the Vercel runtime bridge instead of a TCP listener.

The binary entry point is `api/axum.rs` at the repo root (Vercel convention), declared as a `[[bin]]` in the temper-cloud crate's `Cargo.toml`. This means the file lives outside `crates/temper-cloud/` — it's at the workspace root in `api/` — but it belongs to the temper-cloud crate via the bin path declaration.

### Why No Migrations in temper-cloud

- Cold start latency: migration check adds unnecessary overhead (target: <100ms cold start)
- Concurrent cold starts could contend on the migration lock
- Neon database is persistent — migrations are a deployment concern, not a runtime concern
- sqlx remains the migration authority; migrations run locally or via CI (I11) against Neon connection URIs

## temper-cloud Crate

### `crates/temper-cloud/Cargo.toml`

Dependencies:
- `temper-api` (path dependency — brings axum, sqlx, etc.)
- `vercel_runtime = { version = "2", features = ["axum"] }` — official Vercel Rust runtime with axum adapter
- `tokio` (async runtime)
- `tracing` + `tracing-subscriber` (structured logging)
- `sqlx` (pool connection only — no `migrate` feature needed)
- `tower` (for `ServiceBuilder` to apply `VercelLayer`)

Note: The `vercel_runtime` v2 with `axum` feature is the **official** Vercel Rust runtime (not the deprecated community `vercel-rust` builder). It provides `VercelLayer` which adapts the Vercel serverless function interface to axum's `Router`.

### Entry Point: `api/axum.rs`

Following the official Vercel Rust + axum pattern, the entry point lives at `api/axum.rs` (a Vercel convention — the builder looks for files in `api/`). This file is declared as a `[[bin]]` in the temper-cloud `Cargo.toml`.

~40 lines:
1. Initialize `tracing_subscriber` with `EnvFilter` (structured logging for Vercel log parsing)
2. Load `ApiConfig::from_env()`
3. Connect to Neon via `PgPoolOptions` — no migrations
4. Construct `JwksKeyStore` and `AppState`
5. Call `temper_api::create_app(state)` to get the axum `Router`
6. Wrap with `ServiceBuilder::new().layer(VercelLayer::new()).service(app)`
7. Execute via `vercel_runtime::run(service)`

This differs from `temper-api`'s `main.rs` in two ways: no migrations, and `VercelLayer` + `vercel_runtime::run()` instead of `axum::serve` with a TCP listener.

### `crates/temper-cloud/src/lib.rs`

Remains as a doc comment — the binary entry point is `api/axum.rs`.

## Vercel Configuration

### `vercel.json` (repo root)

```json
{
  "$schema": "https://openapi.vercel.sh/vercel.json",
  "rewrites": [
    { "source": "/(.*)", "destination": "/api/axum" }
  ]
}
```

The official Vercel Rust runtime auto-detects `api/*.rs` files declared as `[[bin]]` entries in `Cargo.toml`. No explicit `functions` or `runtime` block is needed — the runtime is built-in. The rewrite routes all requests to the single axum handler.

### `api/axum.rs`

The binary entry point for the Vercel function, declared in `crates/temper-cloud/Cargo.toml` as:

```toml
[[bin]]
name = "axum"
path = "../../api/axum.rs"
```

The path is relative to the crate root (`crates/temper-cloud/`), reaching up to the workspace root where `api/` lives. This follows the official Vercel Rust + axum example pattern, adapted for the workspace layout.

### Build Configuration

The existing `.cargo/config.toml` release profile applies:
- `codegen-units = 1`
- `lto = "fat"`
- `opt-level = 3`
- `strip = true`
- `panic = "abort"`

Cross-compilation target: `x86_64-unknown-linux-gnu` (handled by the Vercel build system).

### `.vercelignore`

```
target/**
!target/release
!target/x86_64-unknown-linux-gnu/release/**
!target/aarch64-unknown-linux-gnu/release/**
```

## Vercel Project Setup

Manual steps (documented, not automated):
1. `vercel link` — create new project linked to the temper repo
2. Framework preset: "Other"
3. Root directory: repo root
4. Set environment variables in Vercel project settings

### Environment Variables

**Required for API:**
- `DATABASE_URL` — Neon pooled connection string
- `JWKS_URL` — Neon Auth JWKS endpoint
- `AUTH_ISSUER` — JWT issuer URL
- `AUTH_AUDIENCE` — JWT audience (production: set; dev: optional)
- `AUTH_PROVIDER_NAME` — Provider identifier (default: `neon_auth`)
- `CORS_ORIGINS` — Allowed origins (Vercel preview/production URLs)
- `PORT` — Provided by Vercel automatically
- `ENABLE_SWAGGER` — `false` in production

**R2 (prep for I4a, not required at launch):**
- `R2_ACCOUNT_ID`
- `R2_ACCESS_KEY_ID`
- `R2_SECRET_ACCESS_KEY`
- `R2_BUCKET_NAME`
- `R2_PUBLIC_BASE_URL`

### Preview Deployments

Every branch push generates a unique URL (`temper-<hash>-<team>.vercel.app`). The Neon Previews integration auto-creates database branches per preview deployment with copy-on-write from production.

## ApiConfig R2 Extension

Add R2 fields to `ApiConfig` in `temper-api/src/config.rs`:

```rust
pub r2_account_id: Option<String>,
pub r2_access_key_id: Option<String>,
pub r2_secret_access_key: Option<String>,
pub r2_bucket_name: Option<String>,
pub r2_public_base_url: Option<String>,
```

All `Option<String>` — R2 is not required for the API to start. Startup log when R2 is not configured:

```
tracing::info!("R2 not configured — file upload endpoints will be unavailable");
```

## R2 Files Migration

Migration `20260328000003_r2_files.sql` — prepares the table for I4a endpoint work.

```sql
CREATE TABLE r2_files (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    profile_id      UUID NOT NULL REFERENCES profiles(id),
    resource_id     UUID REFERENCES resources(id),
    object_key      TEXT NOT NULL UNIQUE,
    file_url        TEXT NOT NULL,
    content_type    TEXT,
    file_size_bytes BIGINT,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX idx_r2_files_profile ON r2_files(profile_id);
CREATE INDEX idx_r2_files_resource ON r2_files(resource_id);
```

### Design Decisions

- **`profile_id` only, no `team_id`**: A file is always uploaded by one authenticated profile. Access control for who sees the file flows through the resource it's attached to — roles/permissions/ownership are at the `kb_resources` level, not the file level.
- **Optional `resource_id`**: Files can exist unattached (e.g., during upload before association) but the FK is there for "get all files for this resource" queries.
- **No access control functions yet**: `r2_files_visible_to()` and similar deferred to I4a when endpoints land.

## .env.template Update

Update `.env.template` to be the single reference for all env vars:

```
RUST_LOG=info
LOG_LEVEL=info

# Database
DATABASE_URL=postgresql://temper:temper@localhost:5437/temper_development

# Auth
JWKS_URL=https://your-neon-auth-jwks-url/.well-known/jwks.json
AUTH_ISSUER=https://your-neon-auth-issuer
AUTH_AUDIENCE=your-auth-audience
AUTH_PROVIDER_NAME=neon_auth

# CORS
CORS_ORIGINS=http://localhost:3000

# API
PORT=3000
ENABLE_SWAGGER=true

# R2 (Cloudflare) — required for file upload endpoints
R2_ACCOUNT_ID=your_cloudflare_account_id
R2_ACCESS_KEY_ID=your_r2_api_token_access_key_id
R2_SECRET_ACCESS_KEY=your_r2_api_token_secret_access_key
R2_BUCKET_NAME=your_r2_bucket_name
R2_PUBLIC_BASE_URL=https://your-bucket-public-url.r2.dev
```

## Validation Plan

Manual validation after deployment:

1. **`GET /api/health`** — unauthenticated, confirms function boots and connects to Neon
2. **`GET /api/profile`** with valid Neon Auth JWT — confirms auth middleware, JWKS fetching, profile reconciliation
3. **`GET /api/search?q=test`** — confirms query handling returns empty results
4. **Cold start measurement** — hit endpoint after inactivity, target <100ms
5. **Preview deployment** — push a test branch, confirm preview URL is generated and functional

Automated health checks in CI come with I11.

## Follow-up Tickets

### I4a: Cloudflare R2 Integration

- `/api/presign-upload` and `/api/save-metadata` endpoints in temper-api
- R2 CORS configuration using Vercel URLs from I4
- S3-compatible client setup (aws-sdk-s3 or similar)
- Integration testing of upload flow
- Access control for R2 files through resource association

### I11 Addition: OTel Review

Vercel's native OpenTelemetry support is Node.js only. Rust functions get no automatic observability. Investigate:
- `tracing-opentelemetry` + `opentelemetry-otlp` with external collector (Grafana Cloud, Honeycomb, Axiom)
- Flush-on-return lifecycle in serverless (span loss risk)
- Binary size impact (~5-10MB from tonic/prost) vs Vercel's 50MB limit
- Cold start overhead from OTel initialization

For now, structured logging via `tracing` is sufficient — Vercel captures stdout/stderr.

## Out of Scope

- R2 endpoint implementation (I4a)
- CI/CD pipeline (I11)
- OpenTelemetry instrumentation (I11)
- Neon Previews integration setup (manual for now, automated in I11)
- Automated migration running in deployment (I11)
