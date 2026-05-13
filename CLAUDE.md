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

### Embed-gated e2e tests
`cargo make test-e2e` only enables `--features test-db`. CI's separate "Embed & MCP Round-Trip Tests" job additionally enables `test-embed`, which gates push-body and ingest-pipeline tests that exercise the embed pipeline (10 extra tests at last count). When touching push-body, ingest-pipeline, or YAML fixture loading code, run with both features locally to match CI:
```bash
cargo nextest run --manifest-path tests/e2e/Cargo.toml --features test-db,test-embed
```
The Embed CI job is the only one with ONNX Runtime installed, so it's also the one that catches workspace-feature-unification surprises (see `temper-cloud` enabling `ingest-pipeline` on `temper-api`).

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
- **Cloud mode operations** — When `TEMPER_VAULT_STATE=cloud`, write paths route directly through the API: `temper resource create` POSTs to `/api/ingest`; `temper resource update` PATCHes `/api/resources/{id}` with a partial-merge payload (managed_meta + open_meta + optional body trio). **Do not invoke `temper sync run`** in cloud mode — it errors with a redirect message. Files on disk under cloud mode are derivative — the `show` debounce cache and any scratch tmpfile from the show-edit-cat pattern are read-cache or base-then-update artifacts, never authoritative. Body edits work uniformly in both modes via three forms: `--body @<path>` reads from a file, `--body -` reads from stdin explicitly, and implicit stdin is auto-detected when stdin is non-TTY (e.g. `cat tmpfile.md | temper resource update <slug>`). Explicit empty input (`--body @empty.md` or piping no bytes via `--body -`) errors rather than writing an empty body; implicit empty stdin is treated as "no body update requested" so frontmatter-only updates work without piping. The show-edit-cat idiom — `temper resource show <slug>` writes the current body to a temp path, modify it, then `cat tmpfile.md | temper resource update <slug> --stage done` — works in both local and cloud modes; in local mode the vault file is rewritten and best-effort published, in cloud mode the body trio (content + content_hash + chunks_packed) is PATCHed in one call alongside any frontmatter flags.
- **Resource deletion is always explicit** — Use `temper resource delete <slug> --type <doctype> [--context <ctx>] [--force]`. Cloud-first ordering: API soft-delete (`is_active = false`, server-side row preserved) lands first; in local mode the vault file is removed and the manifest entry cleared as a tail action. API failure means no local mutation in either mode. There is no implicit-delete-via-`rm` path. To delete a resource, run `temper resource delete <slug>`. To recover a file you removed by accident (or that's missing on a fresh device), just run `temper sync run` — the next sync cycle reclassifies missing-but-tracked files as `LocallyMissing` and pulls them back. `temper sync refresh` is for non-destructive manifest rebuilds against the server's view, not for recovering missing files; do not use it for recovery. Non-TTY callers (agents, CI) must pass `--force` because the local-file confirmation prompt won't read from a non-terminal stdin.

## Code Quality Rules

These rules apply to all code in this repository. Subagents and implementation plans must follow them.

- **Typed structs over inline JSON** — Never use `serde_json::json!()` for data with a known structure. Define a struct. Compile-time type checking catches errors that runtime serialization silently passes.
- **Shared types at boundaries** — When Rust calls TypeScript (or vice versa), the wire type lives in `temper-core` with `ts-rs` derives. Both sides share the generated type. Never define a zod schema that mirrors a Rust struct manually.
- **Service layer owns SQL; surfaces dispatch through `DbBackend`** — All SQL lives in `temper-api/src/services/`. The `DbBackend` (in `temper-api/src/backend/`) composes services into the `Backend` trait methods defined in `temper-core::operations`. Surfaces (HTTP handlers, MCP tools, CLI actions) build a backend per request and dispatch one operations command per inbound call — they do not call services directly for **writes**. Read paths (list, show, get_meta, search) stay service-direct on both surfaces by design (the trait projections are lossy; reads are passthroughs). Never inline `sqlx::query!()` outside a service. Never call write services directly from a surface — go through the backend trait.
- **Vault file IO and manifest IO live in `vault_backend/`; CLI surfaces dispatch through `VaultBackend`** — All vault-file mutations (`Frontmatter::write_to`, `std::fs::remove_file`, `std::fs::create_dir_all` on vault paths) and all manifest mutations (`manifest_io::save_manifest`, `Manifest::entries` insert/remove) live in `crates/temper-cli/src/vault_backend/`. The `VaultBackend` composes `temper-core::vault::Vault` (path construction), `temper-core::frontmatter::Frontmatter` (parse/serialize), `temper-core::operations::actions::*` (validate/apply_defaults/merge), and `manifest_io::*` (ledger IO) into the `Backend` trait methods. CLI command handlers (`commands/*.rs`) build a `VaultBackend` per request and dispatch one operations command per inbound call — they do not perform vault-file IO or manifest IO directly for **writes**. Read paths (`show`, `list`, `search`) stay surface-direct on the local side, consistent with the `DbBackend` rule above. Never inline `std::fs::*` against a vault path outside `vault_backend/`. Never call `manifest_io::save_manifest` from a `commands/*.rs` module — go through the backend trait. The push-as-tail-action call into `temper-client` is owned by the backend, not the surface — surfaces inspect the returned `Vec<DomainEvent>` (`RemoteSynced` / `PushDeferred { reason }`) for log lines but do not initiate the push themselves.
- **Params structs** — Functions with more than 5 domain-related parameters get a params struct. `#[expect(clippy::too_many_arguments)]` is a smell to fix, not suppress.
- **Auth before writes** — Authorization checks go before any mutations. Never write-then-check.
- **Profile scoping** — All data queries scope through `resources_visible_to`, `can_modify_resource`, or equivalent. Even async workflows verify the profile can access the resource before writing.
- **Pino structured logging** — TypeScript uses pino (`packages/temper-cloud/src/logger.ts`) with contextual field objects. No `console.log`.
- **Schema-required defaults at create/update, not later** — Doc-type schemas in `temper-core/types/schemas/` declare required frontmatter fields. Resource creation paths (templated file write, cloud-mode ingest, MCP create) and update paths must populate every schema-required field at write time, not rely on a downstream pass to backfill. Use `apply_doc_type_defaults` and `Frontmatter::set_managed_meta` (which honors the typed `ManagedMeta` shape) to keep this consistent. For the canonical identity keys (`temper-title` and `temper-slug`), call `temper_core::operations::ensure_managed_identity_keys(meta, title, slug)` on **both** send-side and receive-side — this is Phase 5's symmetric defense pattern; both ends inject canonical keys from a typed source so wire payloads can never drift between them. The receive-side variant fills missing keys without overwriting present ones, so any send-side mis-call (e.g. passing `slug` to the `title` parameter) will silently propagate to storage. Pre-existing files without these fields stay valid until their next round-trip; new writes never produce them.

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
