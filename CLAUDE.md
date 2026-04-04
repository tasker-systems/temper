# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What is Temper

Temper is a knowledge base system for AI-assisted development. It maintains a vault of markdown files with YAML frontmatter that gives agents session continuity — goals, tasks, sessions, research, and decisions persist across conversations. The CLI (`temper`) manages the vault locally; the cloud API syncs it and provides semantic search.

## Architecture

**Monorepo** with a Rust workspace (crates/) and Node/Bun workspace (packages/).

### Rust Crates (crates/)
- **temper-core** — Shared types, config, vault operations. All domain models live here (goals, tasks, sessions, resources). Types derive `sqlx::FromRow` for Postgres and `serde` for serialization. Optional `ts-rs` derives generate TypeScript types.
- **temper-cli** — The `temper` binary. Uses clap for arg parsing. Commands live in `src/commands/`, business logic in `src/actions/`. Templates use askama.
- **temper-api** — Axum HTTP server. Handlers in `src/handlers/`, services in `src/services/`, JWT auth middleware in `src/middleware/`. Uses utoipa for OpenAPI spec generation.
- **temper-client** — Auth-aware HTTP client for the cloud API. Handles Auth0 PKCE device flow, token caching, and all API calls.
- **temper-ingest** — Embedding (ort/ONNX with all-MiniLM-L6-v2) and document extraction (kreuzberg). Both behind feature flags: `embed`, `extract`.
- **temper-mcp** — Remote MCP server (Streamable HTTP via rmcp). Deployed as a Vercel serverless function alongside temper-api. Auth0 JWT validation, OAuth discovery endpoints (RFC 8414/9728). Tools delegate to temper-api services for DB access. Config in `src/config.rs`, tools in `src/tools/`.

### TypeScript Packages (packages/)
- **temper-cloud** — Vercel serverless functions: file upload (Vercel Blob), background processing workflows, document extraction. Uses Neon serverless Postgres, Vitest, Biome.
- **temper-ui** — SvelteKit app at temperkb.io. Uses Tailwind CSS v4, deployed to Vercel. TypeScript types are code-generated from Rust via ts-rs.

### Deployment Glue (api/)
- `api/axum.rs` — Vercel runtime adapter that wraps the Axum app as a Vercel Function.
- `api/mcp.rs` — Vercel runtime adapter for the MCP server (same pattern as axum.rs).
- `api/auth/`, `api/workflows/` — Vercel serverless endpoints (TypeScript).

### Database
- PostgreSQL 18 with pgvector. Migrations in `migrations/` using sqlx.
- Dev database: `postgresql://temper:temper@localhost:5437/temper_development`

## Build & Test Commands

All commands use **cargo-make** (install: `cargo install cargo-make`). Rust tests use **cargo-nextest** (install: `cargo install cargo-nextest`).

```bash
# Quality checks (Rust fmt + clippy + docs + machete, TS typecheck + biome)
cargo make check

# Auto-fix formatting and lint
cargo make fix

# Unit tests (no database needed)
cargo make test

# Integration tests (requires Docker Postgres running)
cargo make docker-up
cargo make test-db

# E2E tests only
cargo make test-e2e

# All tests (Rust + TypeScript + integration)
cargo make test-all

# TypeScript tests only
cargo make ts-test

# Build everything
cargo make build

# Run API server locally
cargo make run

# Generate TypeScript types from Rust structs
cargo make generate-ts-types
```

### Running a single Rust test
```bash
cargo nextest run --workspace test_name
cargo nextest run --workspace -E 'test(test_name)'        # exact filter
cargo nextest run -p temper-api --features test-db test_name  # specific crate with features
```

### TypeScript (temper-cloud)
```bash
cd packages/temper-cloud
bun run test           # unit tests
bun run test:integration  # integration tests
bun run check          # biome lint + format check
bun run check:fix      # auto-fix
bun run typecheck      # tsc
```

### SvelteKit UI (temper-ui)
```bash
cd packages/temper-ui
bun run dev            # dev server
bun run build          # production build
bun run check          # svelte-check
```

## Feature Flags

Rust crates use feature flags to gate heavy dependencies:
- `test-db` — enables database integration tests (temper-api, temper-e2e)
- `test-embed` — enables embedding tests (temper-ingest)
- `embed` / `extract` — gates ONNX and kreuzberg dependencies (temper-ingest)
- `web-api` — enables utoipa OpenAPI derives (temper-core)
- `typescript` — enables ts-rs type generation (temper-core)
- `mcp` — enables schemars JsonSchema derives for MCP tool parameters (temper-core)

## Key Patterns

- **Vault** — A directory of markdown files with YAML frontmatter. The vault path is resolved via temper-core config (`~/.config/temper/config.toml` or per-project `.temper/config.toml`).
- **Sync protocol** — Manifest-based three-way merge (local file vs manifest record vs server). The manifest tracks file hashes to detect local/remote changes. Non-conflicting changes auto-merge at paragraph level using the `similar` crate.
- **UUID v7** — All entity IDs use UUIDv7 (time-sortable).
- **Auth** — Auth0 device authorization PKCE flow. Tokens cached locally. API validates JWTs via JWKS.
- **CI** — GitHub Actions: `code-quality.yml` (fmt, clippy, machete), `test-rust.yml`, `test-typescript.yml`, `ci-success.yml` (merge gate).

## Environment

- Docker Postgres on port **5437** (not 5432, to avoid conflicts).
- `DATABASE_URL=postgresql://temper:temper@localhost:5437/temper_development`
- Linting: Rust uses clippy with `-D warnings`; TypeScript uses Biome.
- Pre-commit hook in `githooks/pre-commit`.
