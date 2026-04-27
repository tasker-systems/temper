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

### End-to-End Tests (tests/e2e/)
Standalone test crate (not in `crates/`) that exercises the full stack: spawns a real Axum server, hits a real Postgres test database, and drives flows through the actual `temper-cli` and `temper-client` code paths. Use this layer for tests that span CLI ↔ API ↔ DB or that need real auth (JWT, JWKS fixtures in `tests/e2e/tests/fixtures/`). Test files in `tests/e2e/tests/`, shared harness in `tests/e2e/tests/common/`. Run with `cargo make test-e2e`.

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

# E2E tests (CLI ↔ API ↔ DB through real Axum + Postgres; lives at tests/e2e/, not crates/)
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
- `test-db` — enables database integration tests (temper-api, tests/e2e)
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

## Code Quality Rules

These rules apply to all code in this repository. Subagents and implementation plans must follow them.

- **Typed structs over inline JSON** — Never use `serde_json::json!()` for data with a known structure. Define a struct. Compile-time type checking catches errors that runtime serialization silently passes.
- **Shared types at boundaries** — When Rust calls TypeScript (or vice versa), the wire type lives in `temper-core` with `ts-rs` derives. Both sides share the generated type. Never define a zod schema that mirrors a Rust struct manually.
- **Service layer owns SQL** — All SQL lives in `temper-api/src/services/`. MCP tools, CLI actions, and HTTP handlers call service functions. If inline `sqlx::query!()` appears outside a service, extract it first.
- **Params structs** — Functions with more than 5 domain-related parameters get a params struct. `#[expect(clippy::too_many_arguments)]` is a smell to fix, not suppress.
- **Auth before writes** — Authorization checks go before any mutations. Never write-then-check.
- **Profile scoping** — All data queries scope through `resources_visible_to`, `can_modify_resource`, or equivalent. Even async workflows verify the profile can access the resource before writing.
- **Pino structured logging** — TypeScript uses pino (`packages/temper-cloud/src/logger.ts`) with contextual field objects. No `console.log`.
- **Schema-required defaults at create/update, not later** — Doc-type schemas in `temper-core/types/schemas/` declare required frontmatter fields. Resource creation paths (templated file write, cloud-mode ingest, MCP create) and update paths must populate every schema-required field at write time, not rely on a downstream pass to backfill. Use `apply_doc_type_defaults` and `Frontmatter::set_managed_meta` (which honors the typed `ManagedMeta` shape) to keep this consistent. Pre-existing files without these fields stay valid until their next round-trip; new writes never produce them.

## SQL Query Checking

Production SQL queries use `sqlx::query!()` / `sqlx::query_as!()` / `sqlx::query_scalar!()` macros for compile-time verification against the actual schema. Exceptions: the `unified_search` query in `search_service.rs` uses runtime `query_as` due to pgvector `::vector` type cast incompatibility, and test fixtures use runtime `sqlx::query()` because `cargo sqlx prepare` cannot cache queries from test targets.

- **Local dev:** Set `DATABASE_URL` — macros check against the live database
- **CI builds:** `SQLX_OFFLINE=true` with committed `.sqlx/` cache (no database needed for compilation)
- **After changing any SQL:** Regenerate cache with `cargo sqlx prepare --workspace -- --all-features`
- **Tests always run against a real database** (Docker Postgres locally, CI database in GitHub Actions)

## Environment

- Docker Postgres on port **5437** (not 5432, to avoid conflicts).
- `DATABASE_URL=postgresql://temper:temper@localhost:5437/temper_development`
- Linting: Rust uses clippy with `-D warnings`; TypeScript uses Biome.
- Pre-commit hook in `githooks/pre-commit`.

## Cloud Agents

For tasks delegated to cloud-based Claude Code sessions, see [docs/guides/cloud-agents.md](docs/guides/cloud-agents.md) for the task preparation guide and environment setup.
