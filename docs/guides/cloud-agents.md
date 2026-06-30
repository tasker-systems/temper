# Cloud Agent Development Guide

This document describes how to prepare tasks for and work as a cloud-based Claude Code agent on the temper project. Cloud agents run in Anthropic's remote infrastructure without access to the local projection cache, IDE, or persistent filesystem.

## What is a Cloud Agent?

A cloud agent is a Claude Code session running on Anthropic's cloud infrastructure (not the developer's local machine). These sessions are launched for focused implementation tasks — typically a single migration, service change, or feature — and produce a PR or set of commits that are reviewed and merged by the project owner.

Cloud agents have access to:
- The git repository (cloned into the remote environment)
- Docker for PostgreSQL
- Cargo, Rust toolchain, and standard dev tools (installed via setup scripts)
- The `.sqlx/` offline query cache (allows compilation without a live database in degraded mode)
- Context provided in the task prompt

Cloud agents do NOT have access to:
- The local projection cache (`~/projects/kb-vault` — a derivative of cloud state)
- `~/.config/temper/config.toml` or other local config
- The Temper MCP server (can't use it to help build it)
- External URLs (can't curl production endpoints to verify)
- Previous conversation history

## Environment Variables for Ephemeral Sessions

Cloud and ephemeral sessions can bootstrap temper without running the browser OAuth flow by exporting the following variables. See the design spec at `docs/superpowers/specs/2026-04-18-cloud-mode-and-portable-memory-design.md` for the broader cloud-mode design these env vars belong to.

| Variable | Purpose | Notes |
|----------|---------|-------|
| `TEMPER_TOKEN` | JWT access token for the temper API | When set, the client uses this in-memory and does not read `~/.config/temper/auth.json`. Malformed tokens error rather than silently falling through. |
| `TEMPER_PROVIDER` | Auth0 provider name that issued the token | Defaults to `auth0`. Typically only needed when a non-default provider is configured. |
| `TEMPER_DEVICE_ID` | Stable device id for this session | When unset, a fresh UUIDv7 is generated per session. Set explicitly if you want a stable device id across session restarts. |
| `TEMPER_API_URL` | API base URL override | Existing variable; takes precedence over config. |

For a SessionStart hook (`.claude/settings.local.json`), export `TEMPER_TOKEN` alongside `cargo install --path crates/temper-cli --locked`, and the temper CLI will authenticate without any interactive step or disk state.

## Environment Setup Scripts

The project provides setup scripts for cloud agent environments:

### `tools/bin/setup-claude-web.sh` (lightweight, runs on SessionStart)
- Configures PATH, environment variables, git hooks
- Generates root `.env` with Rust/API dev values
- Generates `packages/temper-ui/.env` with SvelteKit stub values (so
  `bun run check` / `build` resolve `$env/static/private` imports — real
  credentials come from your local `.env` or Vercel project env in prod)
- Runs `bun install` at the workspace root to populate `node_modules`
- Wired via the SessionStart hook in the committed `.claude/settings.json`
  — applies to every cloud session automatically. Per-user overrides go
  in the gitignored `.claude/settings.local.json`.
- Must complete in seconds

### `tools/bin/setup-claude-web-full.sh` (heavy, run manually)
- Installs system dependencies, Rust toolchain, cargo tools
- Starts PostgreSQL via Docker (pgvector on port 5437)
- Runs database migrations
- Run this when the task requires database access or integration tests

### Individual setup modules (`tools/cargo-make/scripts/claude-web/`)
- `setup-common.sh` — shared helpers (`log_ok`, `log_warn`, `persist_env`, `command_exists`)
- `setup-system-deps.sh` — apt packages
- `setup-rust.sh` — rustup, stable toolchain
- `setup-cargo-tools.sh` — cargo-make, sqlx-cli, cargo-nextest
- `setup-postgres.sh` — Docker PostgreSQL with pgvector
- `setup-db-migrations.sh` — runs sqlx migrations
- `setup-gh.sh` — GitHub CLI

## Writing a Cloud Agent Task

A well-structured task prompt has two parts: the **task description** (what to build and why) and the **Cloud Agent Context** section (everything needed to build it without vault access).

### Cloud Agent Context Section

Include the following in every cloud agent task:

#### 1. Project Overview
Brief description of temper and where this task fits. The cloud agent starts cold — it doesn't know the project.

#### 2. Repository Layout
Show the relevant crate structure. Don't include the full tree — focus on the directories the agent will touch.

```
crates/
  temper-api/src/
    handlers/         # Axum route handlers (thin: extract, call service, respond)
    services/         # Business logic (SQL queries, transactions)
    middleware/       # JWT auth, CORS
    routes.rs         # Router wiring
    state.rs          # AppState (PgPool, config)
    error.rs          # ApiError enum → HTTP status codes
  temper-core/src/
    types/            # Shared domain types (serde + sqlx::FromRow + optional ts-rs)
  temper-mcp/src/     # MCP server (rmcp, Streamable HTTP transport)
    tools/            # Tool implementations
    discovery.rs      # OAuth well-known endpoints + DCR
    middleware.rs     # JWT validation for MCP
    service.rs        # TemperMcpService handler
    router.rs         # Axum router assembly
migrations/           # sqlx migrations (sequential timestamps)
api/
  axum.rs             # Vercel entry point for temper-api
  mcp.rs              # Vercel entry point for temper-mcp
```

#### 3. Database Schema
Include CREATE TABLE statements for every table the task touches. Include relevant indexes, constraints, and SQL functions. The agent can't query the live database to discover the schema.

#### 4. Existing Code Patterns
Show the patterns the agent should follow. Include actual code snippets from the codebase — transaction patterns, error handling, query-as patterns, hash computation, etc. The agent should produce code that looks like it belongs in the project.

#### 5. Services and Types to Modify
List every file that needs changes, with the current function signatures and what needs to change. Include enough context that the agent can find the right insertion point.

#### 6. Build and Test Commands
```bash
# Start PostgreSQL
docker compose up -d

# Run migrations
DATABASE_URL=postgresql://temper:temper@localhost:5437/temper_development sqlx migrate run

# Check compilation
cargo check --all-features

# Run clippy
cargo clippy --all-features -- -D warnings

# Run unit tests (no DB)
cargo test

# Run DB integration tests
DATABASE_URL=postgresql://temper:temper@localhost:5437/temper_test cargo test -p temper-api --features test-db

# Update sqlx offline cache after query changes
DATABASE_URL=postgresql://temper:temper@localhost:5437/temper_development cargo sqlx prepare --workspace

# Full quality check
cargo make check
```

#### 7. Feature Flags
- `test-db` — enables database integration tests
- `web-api` — enables utoipa OpenAPI derives (temper-core)
- `typescript` — enables ts-rs type generation (temper-core)
- `mcp` — enables schemars JsonSchema derives for MCP tool parameters (temper-core)

#### 8. Migration Conventions
- File name: `YYYYMMDDHHMMSS_description.sql` (e.g., `20260406000001_resource_audits.sql`)
- Place in `migrations/` directory
- Pure SQL, no procedural wrappers
- Tables use `kb_` prefix
- Indexes named `idx_tablename_column`
- UUIDs: `gen_random_uuid()` for non-time-sorted, application-side `Uuid::now_v7()` for time-sorted
- After migration, run `cargo sqlx prepare --workspace` to update `.sqlx/` offline cache

#### 9. Acceptance Criteria
Numbered list of specific, verifiable outcomes. Include compilation, clippy, test requirements.

### Example Structure

```markdown
# Task: [descriptive title]

## Problem
What's broken or missing and why it matters.

## Design
How to fix it — tables, API changes, code changes.

## Cloud Agent Context

### Project overview
[brief description]

### Repository layout
[relevant tree]

### Database schema
[CREATE TABLE statements]

### Services to modify
[file paths + current signatures + what to change]

### Code patterns
[snippets showing conventions]

### Build and test commands
[standard block]

### Acceptance criteria
1. [specific outcome]
2. [specific outcome]
...
```

## Key Project Conventions

### Deployment
- **Two separate Vercel projects**: `temperkb.io` (SvelteKit UI) and `temper-cloud.vercel.app` (Rust API + MCP)
- UI `vercel.json` at `packages/temper-ui/vercel.json` contains rewrites proxying `/api/*`, `/mcp`, `/.well-known/*`, `/oauth/*` to the API project
- Root `vercel.json` at repo root routes to the Rust binaries (`/api/axum`, `/api/mcp`)

### Auth
- Auth0 at `temperkb.us.auth0.com` is the sole OAuth provider
- Neon Auth references in old `.env` files are stale — ignore them
- `AUTH_PROVIDER_NAME` should be `auth0`
- MCP uses a static DCR proxy (returns pre-registered client_id) at `/oauth/register`

### UUIDs
All entity IDs use UUIDv7 (time-sortable). Generate with `Uuid::now_v7()`.

### Error handling
```rust
use crate::error::{ApiError, ApiResult};
// ApiError::NotFound → 404
// ApiError::Forbidden → 403
// ApiError::BadRequest(String) → 400
```

### Transaction pattern
```rust
let mut tx = pool.begin().await?;
// ... operations ...
tx.commit().await?;
```

### SQLx offline mode
The `.sqlx/` directory contains cached query metadata. When a live database isn't available, sqlx compiles queries against this cache. Always run `cargo sqlx prepare --workspace` after changing any SQL queries to keep the cache in sync.

## Communicating Results

Cloud agents should:
1. Create a feature branch with descriptive name
2. Make atomic commits with clear messages
3. Run `cargo make check` before considering the task complete
4. If tests require a live database and Docker isn't available, ensure compilation passes with the `.sqlx/` offline cache and note which tests remain to be run
5. Create a PR with a description that references the task

## Revision History

When a task is written and then the codebase changes (e.g., PRs merge that affect the same files), add a **Revision Notes** section to the task documenting what changed and confirming whether the original design still holds. This prevents the cloud agent from working against stale assumptions.
