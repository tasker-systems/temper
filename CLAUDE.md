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
- **temper-ingest** — Embedding (ort/ONNX with BAAI/bge-base-en-v1.5, 768-dim) and document extraction (kreuzberg). Both behind feature flags: `embed`, `extract`.
- **temper-mcp** — Remote MCP server (Streamable HTTP via rmcp). Deployed as a Vercel serverless function alongside temper-api. Auth0 JWT validation, OAuth discovery endpoints (RFC 8414/9728). Tools delegate to temper-api services for DB access. Config in `src/config.rs`, tools in `src/tools/`.

### TypeScript Packages (packages/)
- **temper-cloud** — Vercel serverless functions: file upload (Vercel Blob), background processing workflows, document extraction. Uses Neon serverless Postgres, Vitest, Biome.
- **temper-ui** — SvelteKit app at temperkb.io. Uses Tailwind CSS v4, deployed to Vercel. TypeScript types are code-generated from Rust via ts-rs.

### Deployment Glue (api/)
- `api/axum.rs` — Vercel runtime adapter that wraps the Axum app as a Vercel Function.
- `api/mcp.rs` — Vercel runtime adapter for the MCP server (same pattern as axum.rs).
- `api/auth/`, `api/workflows/` — Vercel serverless endpoints (TypeScript).

**Release ≠ deploy.** Cutting a `v*` tag produces CLI binaries + a GitHub Release ([RELEASING.md](RELEASING.md)) — it deploys nothing. Each running site (temperkb.io, enterprise self-hosted) is an **independent Vercel project** consuming the repo on its own cadence, with its own Neon DB + env; CI does not deploy. Auto-deploy of `main` stays safe via the **additive-only-on-`main`** invariant; big-bang schema changes are operator-run per target via the cutover runbook. See [DEPLOYING.md](DEPLOYING.md).

### End-to-End Tests (tests/e2e/)
Standalone test crate (not in `crates/`) that exercises the full stack: spawns a real Axum server, hits a real Postgres test database, and drives flows through the actual `temper-cli` and `temper-client` code paths. Use this layer for tests that span CLI ↔ API ↔ DB or that need real auth (JWT, JWKS fixtures in `tests/e2e/tests/fixtures/`). Test files in `tests/e2e/tests/`, shared harness in `tests/e2e/tests/common/`. Run with `cargo make test-e2e`.

### Database
- PostgreSQL with pgvector. Local dev/CI runs **PostgreSQL 18** (Docker `pgvector/pgvector:…-pg18`); **Neon cloud runs PostgreSQL 17**. The schema and sqlx migrations are written to run on both — version-portable across 17/18, with no version-specific SQL — so the same `migrations/` apply locally and in cloud. Migrations live in `migrations/` and use sqlx.
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

> **Gotcha:** a bare `cargo nextest run -p temper-api` (no test filter) **hangs** at test-list enumeration — nextest lists the `temper-api` **bin** target, whose `main()` ignores `--list` and blocks (the slow-timeout doesn't cover the list step). Always scope to the integration test target(s): `cargo nextest run -p temper-api --features test-db --test relationship_handler_test`. Also export `DATABASE_URL=postgresql://temper:temper@localhost:5437/temper_development` for `#[sqlx::test]` under bare `cargo` (the `cargo make` tasks set it for you).

### Embed-gated e2e tests
`cargo make test-e2e` only enables `--features test-db`. CI's separate "Embed & MCP Round-Trip Tests" job additionally enables `test-embed`, which gates push-body and ingest-pipeline tests that exercise the embed pipeline (10 extra tests at last count). When touching push-body, ingest-pipeline, or YAML fixture loading code, run with both features locally to match CI:
```bash
cargo make test-e2e-embed
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

## Branch and Commit Conventions

These patterns are observed in recent history rather than rigidly enforced. Match the existing style when in doubt.

### Branch naming

`<initials>/<scope>` — current author uses `jct/<scope>` with kebab-case scope. Examples: `jct/wave1-phase3a-dbbackend-foundation`, `jct/post-cloud-only-qol-trivial-trio`. Keep scopes terse but specific enough to disambiguate parallel branches.

### Commit and PR title prefixes

| Prefix | Use for |
|--------|---------|
| `wave N phase X[a]:` or `Wave N Phase X:` | Numbered phases inside a multi-PR feature plan |
| `cloud-only(<scope>):` | Commits in a multi-chunk migration; `<scope>` is the chunk or PR-letter |
| `QoL:` | Polish, ergonomics, dead-code drops, small cleanups |
| `post-PR-<n>:` | Follow-up to review feedback on PR #n that didn't land inline |
| `audit:` | Output of an audit sweep — rationalization comments, threading fixes |
| `fix(<scope>):` / `refactor(<scope>):` / `docs(<scope>):` / `test:` / `chore:` / `mcp:` | Conventional-Commits style for narrow scoped changes |

Self-contained features sometimes use a plain narrative title with no prefix (e.g. "Limb 1 — relationship events + edge projection", "Add offline_access scope and refresh_token grant support"). That's fine when the PR is its own story; reach for a prefix when the change is one beat of a longer arc.

### Bundling fixes into the PR that surfaced them

If a fix's story is "this PR's tests / new code path surfaced a pre-existing bug," bundle it into the same PR rather than extracting. The narrative stays cohesive: one PR, one explanation. Examples in history: PR #69 bundled the empty-body dedup fix into Phase 3a's PR because workspace feature unification first exposed it under that test suite.

Conversely, if the fix is unrelated to the PR's narrative — even if you noticed it while working — extract it. Mixed-narrative PRs are harder to review and harder to revert.

## Feature Flags

Rust crates use feature flags to gate heavy dependencies:
- `test-db` — enables database integration tests (temper-api, tests/e2e)
- `test-embed` — enables embedding tests (temper-ingest)
- `embed` / `extract` — gates ONNX and kreuzberg dependencies (temper-ingest)
- `web-api` — enables utoipa OpenAPI derives (temper-core)
- `typescript` — enables ts-rs type generation (temper-core)
- `mcp` — enables schemars JsonSchema derives for MCP tool parameters (temper-core)
- `artifact-tests` — enables temper-next's **scenario write-path** integration tests (the `temper-next-write` group in `.config/nextest.toml` — bootseed, seed/scenario load + roundtrip + equivalence, charter, content, ledger, replay) plus ONNX. They build an isolated `temper_next` test namespace from the **canonical baseline** (`migrations/20260624000001_canonical_schema.sql`+`…02_canonical_functions.sql`, loaded under a `search_path=temper_next,public` wrapper via the `00_namespace_reset` fixture — the production schema is `public`, this is a parallel test namespace). Each test OWNS the namespace (resets it to a clean schema+functions then seeds), serialized via the `temper-next-write` nextest group. **No CI job enables it**; run locally with **`cargo make test-next`** (it sets the search_path-option `DATABASE_URL` the test pool needs — post-collapse `substrate::connect()` defaults to `public`, so a bare `cargo nextest run` would resolve against the public substrate and collide). temper-next's pure core tests (affinity, cluster) are ungated and run in CI.
- `artifact-tests-legacy` — the **legacy read-path** tests (`materialize`/`substrate_read`/`embed_job`) that instead need `03_seed.sql` loaded and do NOT reset. Kept on a SEPARATE feature so they can never co-run with the self-resetting write-path tests (a reset's `DROP SCHEMA` would pull the seed out from under them). Run after loading 01+02+03_seed: `cargo nextest run -p temper-next --features artifact-tests-legacy`. M2 retires this path.
- `scenario-schema` — enables `schemars::JsonSchema` derives on the scenario YAML model (temper-next) for the JSON-Schema snapshot test (`tests/scenario_schema.rs`).

### temper-next sqlx macros target the `temper_next` namespace (offline cache)

temper-next is the only crate whose `sqlx::query!` macros resolve against the `temper_next` artifact namespace (not the dev DB's default `public` search_path). Consequences:
- It carries a per-crate `crates/temper-next/.sqlx` cache. Regenerate it after changing temper-next SQL (or the artifact functions it calls) with **`cargo make prepare-next`** (loads the artifact, prepares with `search_path=temper_next`). Per-crate — never `cargo sqlx prepare --workspace` (clobbers per-crate caches).
- All `cargo make` tasks set `SQLX_OFFLINE=true` (matches CI; the dev DB can't validate temper_next queries live). Raw `cargo` against other crates still validates live. pgvector `::vector` queries stay runtime `query()` (the `search_service` exception) — macros are for non-vector queries.
- The **scenario write-path** artifact tests (the `temper-next-write` group in `.config/nextest.toml`) OWN the namespace: each resets it to a clean `01_schema`+`02_functions` then seeds, so they're serialized via the `temper-next-write` nextest test-group and run **separately** from the legacy read-path tests (`materialize`/`substrate_read`/`embed_job`), which instead need `03_seed.sql` loaded. Declarative documents are TWO kinds: **seeds** (`schema-artifact/seeds/` — the template a foundational cogmap is born from) and **scenarios** (`schema-artifact/scenarios/` — a seed reference/embed + the `steps` runbook), each with a snapshot-tested JSON Schema; the reusable mutation functions (`resource_create`/`relationship_assert`/`facet_set`/`lens_create`) live in `02_functions.sql`.

## Key Patterns

- **Vault** — A directory of markdown files with YAML frontmatter. The vault path is resolved via temper-core config (`~/.config/temper/config.toml` or per-project `.temper/config.toml`).
- **UUID v7** — All entity IDs use UUIDv7 (time-sortable).
- **Auth** — Auth0 device authorization PKCE flow. Tokens cached locally. API validates JWTs via JWKS.
- **CI** — GitHub Actions: `code-quality.yml` (fmt, clippy, machete), `test-rust.yml`, `test-typescript.yml`, `ci-success.yml` (merge gate).
- **Addressing is by ref (UUID or decorated)** — `resource show`/`update`/`delete` and `edge assert` source/target take a single positional **ref**: a bare UUID or the decorated form `sluggify(title)-<uuid>`. Resolution is **trailing-UUID-only** — the slug half is parsed off and ignored (a stale/wrong slug half is harmless), so there is no by-slug lookup and no `--type`/`--context`/`--owner` scoping on these commands. Every printed resource carries a `ref` field (list/show/search) — copy it, paste it. `create` keeps `--type`/`--context` (it creates *into* a context); `list` keeps them as filters. The one resolver is `temper_core::operations::parse_ref` (pure string, no DB). See [docs/superpowers/specs/2026-06-17-ws6-surface-completeness-spec-a-addressing-collapse-design.md](docs/superpowers/specs/2026-06-17-ws6-surface-completeness-spec-a-addressing-collapse-design.md).
- **Cloud operations** — All write paths route directly through the API: `temper resource create` POSTs to `/api/ingest`; `temper resource update` PATCHes `/api/resources/{id}` with a partial-merge payload (managed_meta + open_meta + optional body trio). The local vault is a read-only projection cache — files on disk are derivative artifacts, never authoritative. Body edits work via three forms: `--body @<path>` reads from a file, `--body -` reads from stdin explicitly, and implicit stdin is auto-detected when stdin is non-TTY (e.g. `cat tmpfile.md | temper resource update <ref>`). Explicit empty input (`--body @empty.md` or piping no bytes via `--body -`) errors rather than writing an empty body; implicit empty stdin is treated as "no body update requested" so frontmatter-only updates work without piping. The show-edit-cat idiom — `temper resource show <ref>` writes the current body to a temp path, modify it, then `cat tmpfile.md | temper resource update <ref> --stage done` — PATCHes the body trio (content + content_hash + chunks_packed) in one call alongside any frontmatter flags.
- **Resource deletion is always explicit** — Use `temper resource delete <ref> [--force]`. API soft-delete (`is_active = false`, server-side row preserved) is the authoritative action. Removing a projected file from disk with `rm` is just a local cache miss — it has no server effect. To delete a resource from the server, run `temper resource delete <ref>`. To recover a projected file you removed by accident (or that's missing on a fresh device), run `temper pull <context>` — the projection re-materializes from server state. `temper resource delete` is **non-interactive on all surfaces** — there is no confirmation prompt (the pre-cloud local-mode TTY gate was removed by the cloud-only migration). The `--force` flag is therefore vestigial: agents and CI may pass it for clarity, but it changes nothing. See [docs/vault-projection-cache-design.md](docs/vault-projection-cache-design.md#the---force-flag-is-vestigial).
- **Agent-first output defaults** — Temper is agent-first: with a non-TTY stdout (how agents invoke it) and nothing configured, output defaults to **JSON** and **ANSI-free**. Two global flags control presentation: `--format json|toon` and `--color auto|always|never` (both `global = true` on the top-level `Cli`, alongside `--vault`). Each resolves through the same precedence: **CLI flag → env var → `[cli]` config → tty-aware default**. Format env is `TEMPER_FORMAT`; color env is `TEMPER_COLOR`, and the `NO_COLOR` convention is honored at the default layer (an explicit flag/env/config color overrides it). Resolution happens **once** in `main` — format via `OutputFormat::resolve_with` (`temper-cli/src/format.rs`), color via `color::apply_color_choice` which installs anstream's process-global `ColorChoice` so every `output::*` helper obeys it. Config defaults live in the optional `[cli]` section (`format`/`color`) of `~/.config/temper/config.toml` (`CliSection` in temper-core). Never emit raw ANSI — all styled output routes through `output/` (anstream/anstyle).

## Code Quality Rules

These rules apply to all code in this repository. Subagents and implementation plans must follow them.

- **Typed structs over inline JSON** — Never use `serde_json::json!()` for data with a known structure. Define a struct. Compile-time type checking catches errors that runtime serialization silently passes.
- **Shared types at boundaries** — When Rust calls TypeScript (or vice versa), the wire type lives in `temper-core` with `ts-rs` derives. Both sides share the generated type. Never define a zod schema that mirrors a Rust struct manually.
- **Service layer owns SQL; surfaces dispatch through `DbBackend`** — All SQL lives in `temper-api/src/services/`. The `DbBackend` (in `temper-api/src/backend/`) composes services into the `Backend` trait methods defined in `temper-core::operations`. Surfaces (HTTP handlers, MCP tools, CLI actions) build a backend per request and dispatch one operations command per inbound call — they do not call services directly for **writes**. Read paths (list, show, get_meta, search) stay service-direct on both surfaces by design (the trait projections are lossy; reads are passthroughs). Never inline `sqlx::query!()` outside a service. Never call write services directly from a surface — go through the backend trait. All vault writes route through `temper-client` to `temper-api` — there is no local-write surface.
- **Params structs** — Functions with more than 5 domain-related parameters get a params struct. `#[expect(clippy::too_many_arguments)]` is a smell to fix, not suppress.
- **Auth before writes** — Authorization checks go before any mutations. Never write-then-check.
- **Profile scoping** — All data queries scope through `resources_visible_to`, `can_modify_resource`, or equivalent. Even async workflows verify the profile can access the resource before writing.
- **Pino structured logging** — TypeScript uses pino (`packages/temper-cloud/src/logger.ts`) with contextual field objects. No `console.log`.
- **Schema-required defaults at create/update, not later** — Doc-type schemas in `temper-core/types/schemas/` declare required frontmatter fields. Resource creation paths (templated file write, cloud-mode ingest, MCP create) and update paths must populate every schema-required field at write time, not rely on a downstream pass to backfill. Use `apply_doc_type_defaults` and `Frontmatter::set_managed_meta` (which honors the typed `ManagedMeta` shape) to keep this consistent. For the canonical identity keys (`temper-title` and `temper-slug`), call `temper_core::operations::ensure_managed_identity_keys(meta, title, slug)` on **both** send-side and receive-side — this is Phase 5's symmetric defense pattern; both ends inject canonical keys from a typed source so wire payloads can never drift between them. The receive-side variant fills missing keys without overwriting present ones, so any send-side mis-call (e.g. passing `slug` to the `title` parameter) will silently propagate to storage. Pre-existing files without these fields stay valid until their next round-trip; new writes never produce them.

## SQL Query Checking

Production SQL queries use `sqlx::query!()` / `sqlx::query_as!()` / `sqlx::query_scalar!()` macros for compile-time verification against the actual schema. Exception: the `unified_search` query in `search_service.rs` uses runtime `query_as` due to pgvector `::vector` type cast incompatibility. Trivial test-fixture lookups may use runtime `sqlx::query()`; substantive test queries keep macros, cached per-crate (below).

- **Local dev:** Set `DATABASE_URL` — macros check against the live database. Note `cargo make` tasks force `SQLX_OFFLINE=true`, so `cargo make check` is the honest local probe of the committed caches.
- **CI builds:** `SQLX_OFFLINE=true` with committed `.sqlx/` cache for test jobs; the `code-quality` clippy job compiles against a **live** DB, so it will NOT catch a missing cache entry — only offline `cargo make check` does.
- **After changing any SQL:** Regenerate the workspace cache with `cargo sqlx prepare --workspace -- --all-features`
- **Test-target macro queries** (e.g. temper-api's `relationship_*_test`, the e2e suite) are NOT captured by the workspace ritual — plain `cargo sqlx prepare` skips test targets. They live in per-crate caches regenerated with `--all-targets`: `cargo make prepare-api` (`crates/temper-api/.sqlx`) and `cargo make prepare-e2e` (`tests/e2e/.sqlx`), alongside `cargo make prepare-next` for temper-next. Run the matching task after changing test SQL or schema it touches.
- **Tests always run against a real database** (Docker Postgres locally, CI database in GitHub Actions)

## Environment

- Docker Postgres on port **5437** (not 5432, to avoid conflicts).
- `DATABASE_URL=postgresql://temper:temper@localhost:5437/temper_development`
- Linting: Rust uses clippy with `-D warnings`; TypeScript uses Biome.
- Pre-commit hook in `githooks/pre-commit`.

## Cloud Agents

For tasks delegated to cloud-based Claude Code sessions, see [docs/guides/cloud-agents.md](docs/guides/cloud-agents.md) for the task preparation guide and environment setup.
