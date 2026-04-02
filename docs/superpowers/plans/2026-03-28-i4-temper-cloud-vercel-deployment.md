# I4: temper-cloud Vercel Deployment Adapter — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Deploy temper-api to Vercel as a serverless function via the temper-cloud adapter crate, and prepare R2 file storage schema for follow-up integration work.

**Architecture:** temper-cloud is a thin binary that imports `temper_api::create_app()`, wraps the resulting axum Router with `VercelLayer` from the official `vercel_runtime` v2 crate, and executes via `vercel_runtime::run()`. No migrations at runtime — the serverless function just connects and serves. R2 configuration fields are added as optional to ApiConfig so the API boots without them.

**Tech Stack:** Rust, axum 0.8, vercel_runtime 2 (axum feature), sqlx 0.8, tower, Vercel, Neon Postgres

---

## File Map

| Action | Path | Responsibility |
|--------|------|----------------|
| Modify | `crates/temper-cloud/Cargo.toml` | Add dependencies (vercel_runtime, temper-api, tokio, tower, tracing, sqlx) and [[bin]] entry |
| Modify | `crates/temper-cloud/src/lib.rs` | Update doc comment |
| Create | `api/axum.rs` | Vercel entry point — builds AppState, wraps create_app with VercelLayer |
| Modify | `crates/temper-api/src/config.rs` | Add R2 optional fields to ApiConfig + from_env() |
| Modify | `crates/temper-api/tests/common/mod.rs` | Add R2 fields to test ApiConfig construction |
| Create | `migrations/20260328000003_r2_files.sql` | R2 file metadata table |
| Modify | `.env.template` | Complete env var reference |
| Create | `vercel.json` | Vercel deployment configuration |
| Create | `.vercelignore` | Exclude build artifacts |
| Modify | `Cargo.toml` (workspace root) | No changes needed — `crates/*` glob already includes temper-cloud |

---

### Task 1: R2 Fields in ApiConfig

**Files:**
- Modify: `crates/temper-api/src/config.rs`
- Modify: `crates/temper-api/tests/common/mod.rs`

- [ ] **Step 1: Add R2 fields to ApiConfig struct**

In `crates/temper-api/src/config.rs`, add five optional fields after `enable_swagger`:

```rust
pub r2_account_id: Option<String>,
pub r2_access_key_id: Option<String>,
pub r2_secret_access_key: Option<String>,
pub r2_bucket_name: Option<String>,
pub r2_public_base_url: Option<String>,
```

- [ ] **Step 2: Read R2 env vars in from_env()**

In `ApiConfig::from_env()`, before the final `Ok(Self { ... })`, add:

```rust
let r2_account_id = env::var("R2_ACCOUNT_ID").ok().filter(|s| !s.is_empty());
let r2_access_key_id = env::var("R2_ACCESS_KEY_ID").ok().filter(|s| !s.is_empty());
let r2_secret_access_key = env::var("R2_SECRET_ACCESS_KEY").ok().filter(|s| !s.is_empty());
let r2_bucket_name = env::var("R2_BUCKET_NAME").ok().filter(|s| !s.is_empty());
let r2_public_base_url = env::var("R2_PUBLIC_BASE_URL").ok().filter(|s| !s.is_empty());

if r2_account_id.is_none() || r2_bucket_name.is_none() {
    tracing::info!(
        "R2 not configured — file upload endpoints will be unavailable"
    );
}
```

Add these fields to the `Ok(Self { ... })` return block:

```rust
r2_account_id,
r2_access_key_id,
r2_secret_access_key,
r2_bucket_name,
r2_public_base_url,
```

- [ ] **Step 3: Update test ApiConfig in tests/common/mod.rs**

In the `setup_test_app()` function, add the R2 fields to the `ApiConfig` struct literal (after `enable_swagger: false`):

```rust
r2_account_id: None,
r2_access_key_id: None,
r2_secret_access_key: None,
r2_bucket_name: None,
r2_public_base_url: None,
```

- [ ] **Step 4: Run tests to verify nothing breaks**

Run: `cargo nextest run --workspace --all-features`

Expected: All 214+ tests pass. The new fields are all `None` so existing behavior is unchanged.

- [ ] **Step 5: Commit**

```bash
git add crates/temper-api/src/config.rs crates/temper-api/tests/common/mod.rs
git commit -m "config: add optional R2 fields to ApiConfig for file upload prep"
```

---

### Task 2: R2 Files Migration

**Files:**
- Create: `migrations/20260328000003_r2_files.sql`

- [ ] **Step 1: Create the migration file**

Create `migrations/20260328000003_r2_files.sql`:

```sql
-- R2 file metadata: tracks files uploaded to Cloudflare R2.
-- Access control flows through the associated resource, not the file itself.

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

- [ ] **Step 2: Run migrations against local Docker Postgres**

Run: `cargo sqlx migrate run --source migrations`

Expected: Migration applies successfully. Verify:

```bash
psql postgresql://temper:temper@localhost:5437/temper_development -c "\d r2_files"
```

Expected output shows the table with all columns, indexes, and FK constraints.

- [ ] **Step 3: Run tests to verify migrations don't break existing tests**

Run: `cargo nextest run --workspace --all-features`

Expected: All tests pass. The new table exists but nothing references it yet.

- [ ] **Step 4: Commit**

```bash
git add migrations/20260328000003_r2_files.sql
git commit -m "schema: add r2_files table for Cloudflare R2 file metadata"
```

---

### Task 3: Update .env.template

**Files:**
- Modify: `.env.template`

- [ ] **Step 1: Replace .env.template with complete env var reference**

Replace the entire contents of `.env.template`:

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

- [ ] **Step 2: Commit**

```bash
git add .env.template
git commit -m "config: update .env.template with all env vars including auth and R2"
```

---

### Task 4: temper-cloud Crate Setup

**Files:**
- Modify: `crates/temper-cloud/Cargo.toml`
- Modify: `crates/temper-cloud/src/lib.rs`
- Create: `api/axum.rs`

- [ ] **Step 1: Update temper-cloud Cargo.toml**

Replace the entire contents of `crates/temper-cloud/Cargo.toml`:

```toml
[package]
name = "temper-cloud"
version = "0.1.0"
edition = "2021"
description = "Vercel deployment adapter for temper-api"

[[bin]]
name = "axum"
path = "../../api/axum.rs"

[dependencies]
temper-api = { path = "../temper-api" }
vercel_runtime = { version = "2", features = ["axum"] }
tokio = { version = "1", features = ["full"] }
tower = "0.5"
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter"] }
sqlx = { version = "0.8", features = ["postgres", "runtime-tokio-rustls"] }
```

- [ ] **Step 2: Update lib.rs doc comment**

Replace the contents of `crates/temper-cloud/src/lib.rs`:

```rust
//! temper-cloud — Vercel serverless adapter for temper-api.
//!
//! Wraps [`temper_api::create_app`] with the official `vercel_runtime` v2
//! VercelLayer to serve the axum Router as a Vercel serverless function.
//! No migrations at runtime — connect, serve, done.
```

- [ ] **Step 3: Create api/axum.rs entry point**

Create `api/axum.rs` at the workspace root:

```rust
//! Vercel serverless function entry point for temper-api.
//!
//! This binary bridges the axum Router from temper-api to Vercel's
//! serverless function interface via VercelLayer.

use sqlx::postgres::PgPoolOptions;
use tower::ServiceBuilder;
use tracing_subscriber::EnvFilter;
use vercel_runtime::VercelLayer;

use temper_api::config::ApiConfig;
use temper_api::state::{AppState, JwksKeyStore};

#[tokio::main]
async fn main() -> Result<(), vercel_runtime::Error> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .init();

    let config = ApiConfig::from_env().expect("Failed to load config from environment");

    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(&config.database_url)
        .await
        .expect("Failed to connect to database");

    let jwks_store = JwksKeyStore::new(config.jwks_url.clone());
    let state = AppState::new(pool, jwks_store, config);
    let app = temper_api::create_app(state);

    let service = ServiceBuilder::new()
        .layer(VercelLayer::new())
        .service(app);

    tracing::info!("temper-cloud: Vercel function initialized");

    vercel_runtime::run(service).await
}
```

- [ ] **Step 4: Verify it compiles**

Run: `cargo check -p temper-cloud`

Expected: Compiles successfully. Note: `vercel_runtime` will be downloaded from crates.io on first build.

If compilation fails due to `vercel_runtime` API differences (the exact import paths may vary), check the crate docs:

```bash
cargo doc -p vercel_runtime --open
```

Adjust imports as needed based on actual API surface.

- [ ] **Step 5: Commit**

```bash
git add crates/temper-cloud/Cargo.toml crates/temper-cloud/src/lib.rs api/axum.rs
git commit -m "feat: temper-cloud Vercel adapter with vercel_runtime v2 and VercelLayer"
```

---

### Task 5: Vercel Configuration Files

**Files:**
- Create: `vercel.json`
- Create: `.vercelignore`

- [ ] **Step 1: Create vercel.json**

Create `vercel.json` at the workspace root:

```json
{
  "$schema": "https://openapi.vercel.sh/vercel.json",
  "rewrites": [
    { "source": "/(.*)", "destination": "/api/axum" }
  ]
}
```

- [ ] **Step 2: Create .vercelignore**

Create `.vercelignore` at the workspace root:

```
target/**
!target/release
!target/x86_64-unknown-linux-gnu/release/**
!target/aarch64-unknown-linux-gnu/release/**
```

- [ ] **Step 3: Commit**

```bash
git add vercel.json .vercelignore
git commit -m "config: add Vercel deployment configuration and ignore rules"
```

---

### Task 6: Full Workspace Verification

**Files:** None (verification only)

- [ ] **Step 1: Run full workspace check**

Run: `cargo make check`

Expected: Formatting, clippy, docs, and machete all pass. If machete flags `temper-core` in temper-cloud (since lib.rs is just a doc comment), add to the existing `[package.metadata.cargo-machete]` section if needed — but temper-cloud now depends on `temper-api`, not `temper-core`, so the old machete ignore for temper-core can be removed.

- [ ] **Step 2: Run full test suite**

Run: `cargo nextest run --workspace --all-features`

Expected: All tests pass. temper-cloud has no tests of its own (it's a ~30 line binary that can only be meaningfully tested via deployment).

- [ ] **Step 3: Verify temper-cloud binary builds in release mode**

Run: `cargo build -p temper-cloud --release`

Expected: Compiles successfully. Check binary size:

```bash
ls -lh target/release/axum
```

Note the size — should be well under Vercel's 50MB limit even with fat LTO.

- [ ] **Step 4: Commit any fixes from verification**

If any fixes were needed, commit them:

```bash
git add -A
git commit -m "fix: address workspace verification issues"
```

---

### Task 7: Vercel Project Setup and Deployment

**Files:** None (infrastructure setup)

This task involves manual steps in the Vercel dashboard and CLI. The implementer must have the Vercel CLI installed (`npm i -g vercel`) and be logged in.

- [ ] **Step 1: Link the project to Vercel**

From the workspace root:

```bash
vercel link
```

When prompted:
- Set up new project: **Yes**
- Scope: select your Vercel team/account
- Project name: `temper-cloud` (or similar)
- Root directory: `.` (repo root)
- Framework: **Other**
- Build command: leave empty (Vercel Rust runtime handles this)
- Output directory: leave empty

- [ ] **Step 2: Set environment variables in Vercel**

In the Vercel project dashboard (Settings > Environment Variables), add:

| Variable | Value | Environments |
|----------|-------|--------------|
| `DATABASE_URL` | Neon pooled connection string | Production, Preview |
| `JWKS_URL` | Neon Auth JWKS endpoint URL | Production, Preview |
| `AUTH_ISSUER` | Neon Auth issuer URL | Production, Preview |
| `AUTH_AUDIENCE` | Your auth audience value | Production |
| `AUTH_PROVIDER_NAME` | `neon_auth` | Production, Preview |
| `CORS_ORIGINS` | Production + preview URLs | Production, Preview |
| `ENABLE_SWAGGER` | `false` | Production |
| `ENABLE_SWAGGER` | `true` | Preview (optional) |

R2 variables are not needed yet — add them when I4a work begins.

- [ ] **Step 3: Deploy**

```bash
vercel deploy
```

Or push the branch to trigger a preview deployment. Monitor the build logs for compilation success.

- [ ] **Step 4: Validate the deployment**

Test the live endpoint:

```bash
# Health check (unauthenticated)
curl -s https://your-deployment-url.vercel.app/api/health | jq .

# Expected:
# { "status": "ok", "version": "0.1.0" }
```

For authenticated endpoints, you need a valid Neon Auth JWT. Test with whatever auth flow your Neon Auth setup provides.

- [ ] **Step 5: Measure cold start**

Wait 10+ minutes for the function to go cold, then:

```bash
time curl -s https://your-deployment-url.vercel.app/api/health > /dev/null
```

Target: <100ms. Note: first deployment may have higher cold starts due to binary size.

---

### Task 8: Update I4 Ticket and Create I4a

**Files:** None (ticket management)

- [ ] **Step 1: Update I4 ticket body to match design**

Run:

```bash
temper ticket show 2026-03-27-i4-temper-cloud-vercel-deployment-adapter --project temper
```

Update the ticket body to reflect the actual implementation: official vercel_runtime v2 (not the community builder), VercelLayer (not api/axum.rs shim), no migrations, R2 prep split to I4a. Pipe updated content via stdin to `temper ticket create` or edit the file directly.

- [ ] **Step 2: Create I4a ticket for R2 integration**

```bash
cat <<'TICKET' | temper ticket create --title "I4a: Cloudflare R2 Integration — Endpoints and Testing" --project temper --scope feature
# I4a: Cloudflare R2 Integration — Endpoints and Testing

## What

Build the presigned upload and metadata endpoints, configure R2 CORS using the Vercel URLs from I4, and integration test the full upload flow.

## Why

The r2_files table and ApiConfig R2 fields are in place from I4. This ticket wires up the actual S3-compatible client, presigned URL generation, and metadata recording so files can be uploaded from the CLI or web UI.

## Scope

### Endpoints
- `POST /api/presign-upload` — generates a temporary presigned URL for direct R2 upload
- `POST /api/save-metadata` — records file metadata in r2_files after successful upload

### R2 CORS Configuration
- Configure R2 bucket CORS in Cloudflare dashboard using Vercel production and preview URLs
- Allowed methods: PUT (uploads), GET (retrieval)

### S3-Compatible Client
- Use aws-sdk-s3 or similar crate with R2 endpoint configuration
- Initialize from ApiConfig R2 fields
- Presigned URL generation with configurable expiry (default: 300s)

### Access Control
- Both endpoints require authentication (behind require_auth middleware)
- presign-upload creates r2_files record with authenticated profile_id
- resource_id association optional at upload time, can be set later

### Testing
- Unit tests for presigned URL generation
- Integration tests for upload + metadata flow
- Verify R2 CORS allows uploads from Vercel preview URLs

## Depends On
- I4: temper-cloud deployed to Vercel (for CORS URL configuration)

## Deliverable
- Working file upload flow: presign → upload to R2 → save metadata
- R2 CORS configured for production and preview origins
- Integration tests passing
TICKET
```

- [ ] **Step 3: Add OTel review note to I11**

Read the current I11 ticket and append an OTel review section noting:
- Vercel's native OTel is Node.js only
- Investigate tracing-opentelemetry + external collector
- Evaluate flush-on-return overhead, binary size impact, cold start cost
- For now, structured logging via tracing is sufficient

- [ ] **Step 4: Commit any CLAUDE.md updates if needed**

If any CLAUDE.md files were modified during the ticket work (the git status showed some modified), ensure they are committed or reverted as appropriate.

---

### Task 9: Session Save

**Files:** None (documentation)

- [ ] **Step 1: Save the session note**

Pipe session content summarizing I4 work:

```bash
cat <<'EOF' | temper session save "I4 temper-cloud Vercel Deployment Adapter" --ticket 2026-03-27-i4-temper-cloud-vercel-deployment-adapter --state done --project temper
## Goal
Deploy temper-api to Vercel as a serverless function via the temper-cloud adapter crate.

## What happened
[Fill in: actual implementation details, any issues encountered, deployment results]

## Decisions
[Fill in: any decisions made during implementation]

## What connected
[Fill in: cross-project patterns, learnings]

## To pick up
- I4a: Cloudflare R2 Integration
- I5: temper-client auth-aware HTTP client
EOF
```
