# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What is Temper

Temper is a knowledge base system for AI-assisted development. It maintains a vault of markdown files with YAML frontmatter that gives agents session continuity — goals, tasks, sessions, research, and decisions persist across conversations. The CLI (`temper`) manages the vault locally; the cloud API syncs it and provides semantic search.

## Architecture

**Monorepo** with a Rust workspace (crates/) and Node/Bun workspace (packages/).

### Rust Crates (crates/)
- **temper-core** — Shared types, config, vault operations. All domain models live here (goals, tasks, sessions, resources). Types derive `sqlx::FromRow` for Postgres and `serde` for serialization. Optional `ts-rs` derives generate TypeScript types.
- **temper-cli** — The `temper` binary. Uses clap for arg parsing. Commands live in `src/commands/`, business logic in `src/actions/`. Templates use askama.
- **temper-api** — Axum HTTP transport only. Handlers in `src/handlers/`, routes, JWT auth middleware in `src/middleware/`, OpenAPI (utoipa), and `main`/`create_app`. Business logic + persistence live in temper-services; temper-api just wires transport to them.
- **temper-workflow** — Domain operations layer extracted from temper-core: the `Backend` trait + operations commands (`src/operations/`, including `parse_ref`), frontmatter, doc-type schemas, vault ops, and hashing.
- **temper-services** — Shared business-logic + auth-infra layer for **both** surfaces (temper-api and temper-mcp): the services (`src/services/`), the `DbBackend` (`src/backend/`) that composes persistence into the `Backend` trait, plus ApiError/AppState/JwksKeyStore/ApiConfig. Both surfaces depend on it; neither surface depends on the other.
- **temper-client** — Auth-aware HTTP client for the cloud API. Handles Auth0 PKCE device flow, token caching, and all API calls.
- **temper-ingest** — Embedding (ort/ONNX with BAAI/bge-base-en-v1.5, 768-dim) and document extraction (kreuzberg). Both behind feature flags: `embed`, `extract`. **The CLI is the primary embed path** — it depends on temper-ingest directly and computes embeddings client-side (`compute_body_chunks`). The server does **not** recompute them: chunks supplied by the client ride through **verbatim** (`db_backend.rs`, the `chunks_packed: Some(..)` arm), and the server embeds **only when chunks are absent** (MCP and any programmatic client without an embedder). Because temper-substrate pulls `temper-ingest(embed)` non-optionally, ort is always linked into temper-api and temper-services (there is no embed feature flag to toggle on those crates).

  **Both surfaces must embed with the same model**, and this is enforced, not assumed. `temper-ingest/build.rs` derives the expected model sha256 from the LFS-pinned `model_quantized.onnx` **as committed** (from the git-lfs pointer when the blob is unsmudged — its `oid` *is* the sha256 — from the file when it is), and every model loaded from disk is verified against it. A mismatch is a hard error. This exists because it silently went wrong: the CLI's `embed-download` used to fetch the **fp32** model from Hugging Face `main` while the server used the quantized one, so the index filled with vectors from two different models with nothing recording which. `embed-download` no longer downloads anything — it resolves the model from disk next to the binary (the release archive ships it there, which is why the release checkout needs `lfs: true`).
- **temper-substrate** — Persistence write/readback core (`writes`/`readback`) plus the cognitive-map / telos-lens region producer and the YAML scenario DSL. Pulls `temper-ingest(embed)` unconditionally, so every crate depending on it links ort.
- **temper-mcp** — Remote MCP server (Streamable HTTP via rmcp). Deployed as a Vercel serverless function alongside temper-api. Auth0 JWT validation, OAuth discovery endpoints (RFC 8414/9728). Tools delegate to temper-services for DB access (services-direct reads, `DbBackend` writes) — it no longer depends on temper-api. Config in `src/config.rs`, tools in `src/tools/`.

### TypeScript Packages (packages/)
- **temper-cloud** — Vercel serverless functions: file upload (Vercel Blob), background processing workflows, document extraction. Uses Neon serverless Postgres, Vitest, Biome.
- **temper-ui** — SvelteKit app at temperkb.io. Uses Tailwind CSS v4, deployed to Vercel. TypeScript types are code-generated from Rust via ts-rs.
- **agent-workflows** — Deployed agent runtimes over temper-mcp (Eve now, Claude Managed Agents later). Each agent is a **self-contained Eve project** (its own TS 7 toolchain, npm lockfile) that is **workspace-isolated** — deliberately NOT a bun `workspaces` member, so it never collides with temper-cloud's TS 5.8 and the repo pre-commit never touches it. Install/run tooling from inside each agent dir (`cd steward && npm install`; a root `npm install` inherits the root's bun `overrides` and fails). First agent: `steward/` (team self-cognition steward; MCP connection with env-driven `TEMPER_MCP_URL` + platform-carried auth).

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

# Regenerate openapi.json AND the temper-rb gem AND temper-ts's schema.ts (all products of the router)
cargo make openapi
```

> **OpenAPI + the temper-rb gem + temper-ts's `schema.ts` are all products of the router.** A
> new/changed response DTO (a new field, a renamed type) restales **three** committed artifacts:
> `openapi.json`, the generated Ruby gem under `clients/temper-rb/lib/temper/generated`, and
> `clients/temper-ts/src/generated/schema.ts` (emitted by `openapi-typescript`, pinned exactly —
> no caret — in temper-ts's devDependencies). `cargo make openapi` regenerates all three in one
> step (gem regen needs Docker; the TS schema needs only Node). `cargo make check` gates all
> three: `openapi-check` (spec), `openapi-rb-drift` (gem — Docker-based, **skips** without Docker;
> the `test-ruby` CI job is the never-skipping backstop), and `openapi-ts-drift` (schema — and
> unlike the gem's gate, this one **never skips**: `openapi-typescript` needs only Node, so there
> is no environment in which `cargo make check` would rather guess than check). Never assume that
> because one SDK's gate is best-effort, the other is too — they have different skip semantics for
> different reasons, and `openapi-ts-drift` is the strict one. The generator pin + params for the
> gem live in one place — `.github/scripts/generate-temper-rb.sh` — shared by cargo-make and the
> gem's Rakefile; the TS equivalent is `.github/scripts/generate-temper-ts.sh`, shared by
> `cargo make openapi-ts`, `check-temper-ts-drift.sh`, and the `test-agents-ts` CI job's drift
> step. `detect-ci-scope.sh` carries `^openapi\.json$` in **both** `test-ruby`'s and
> `test-agents-ts`'s trigger sets, for the identical reason: a contract change that does not run
> the job whose gate catches the stale artifact is a gate that runs nowhere. (`test-agents-ts` got
> this later than `test-ruby` did — the same rot the gem discovered in `tests/contracts/`.)
>
> **The drift gate compares against git, not against a fresh build.** Both `check-temper-rb-drift.sh`
> and `check-temper-ts-drift.sh` regenerate their artifact and then run `git diff --exit-code` over
> it. So an artifact you have *just correctly regenerated* still fails `cargo make check` while it
> sits unstaged — the error reads "generated core/schema is out of date with openapi.json", which
> sounds like you forgot to run `cargo make openapi` when in fact you need to `git add` (or commit)
> its output. Stage the regenerated files, then re-run `check`.

### Running a single Rust test
```bash
cargo nextest run --workspace test_name
cargo nextest run --workspace -E 'test(test_name)'        # exact filter
cargo nextest run -p temper-api --features test-db test_name  # specific crate with features
```

> **Gotcha:** a bare `cargo nextest run -p temper-api` (no test filter) **hangs** at test-list enumeration — nextest lists the `temper-api` **bin** target, whose `main()` ignores `--list` and blocks (the slow-timeout doesn't cover the list step). Always scope to the integration test target(s): `cargo nextest run -p temper-api --features test-db --test relationship_handler_test`. Also export `DATABASE_URL=postgresql://temper:temper@localhost:5437/temper_development` for `#[sqlx::test]` under bare `cargo` (the `cargo make` tasks set it for you).

### Embed-gated e2e tests
`cargo make test-e2e` only enables `--features test-db`, so it **silently compiles out every `test-embed`-gated test**. CI does not: **every CI test job enables `test-embed`**, and ONNX is installed in all of them. When touching push-body, ingest-pipeline, or YAML fixture loading code, run with both features locally to match CI:
```bash
cargo make test-e2e-embed
```

> **CI runs everything, by construction.** Jobs are split by **intention** (what they need from the environment), never by feature flag: **Unit** (no DB) · **Integration & E2E** (Postgres + LFS — the whole DB-backed workspace in ONE `--workspace` command) · **Substrate Artifacts** (a different feature set). Coverage is nightly (`coverage.yml`), out of the PR path, so an instrumented-build OOM can never block a merge.
>
> There is **no "the job with ONNX"** any more — that was a historical constraint and it is gone. Confining `test-embed` to one job is precisely what let `streaming_ingest_test` rot: its tests were *compiled out* of the integration job and *filtered out* of the embed job's allowlist, so they ran **nowhere**, and a 484-second test hid behind a green tick for months.
>
> **Never add a `-E 'binary(...)'` filter to a CI test job.** Selection is `--workspace` so a new crate or test is picked up with no CI edit. A filter that makes CI green is hiding a test, not fixing one.
>
> Shared CI behavior lives in composite actions (`.github/actions/install-onnx`, `.github/actions/setup-rust`) rather than being copy-pasted per job — the ONNX install had drifted into **five** near-identical copies.

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
- `artifact-tests` — enables temper-substrate's **scenario write-path** integration tests (bootseed, seed/scenario load + roundtrip + equivalence, charter, content, ledger, replay) plus ONNX. Tests run on ephemeral `public`-schema databases via `#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]` — each test gets its own isolated database. CI runs it in its own **Substrate Artifact Tests** job (a distinct feature set, so it cannot fold into the `--workspace` integration run); run locally with **`cargo make test-artifacts`**. temper-substrate's pure core tests (affinity, cluster) are ungated and run in CI.
- `scenario-schema` — enables `schemars::JsonSchema` derives on the scenario YAML model (temper-substrate) for the JSON-Schema snapshot test (`tests/scenario_schema.rs`).

## Key Patterns

- **Vault** — A directory of markdown files with YAML frontmatter. The vault path is resolved via temper-core config (`~/.config/temper/config.toml` or per-project `.temper/config.toml`).
- **UUID v7** — All entity IDs use UUIDv7 (time-sortable).
- **Auth** — Auth0 device authorization PKCE flow. Tokens cached locally. API validates JWTs via JWKS.
- **CI** — GitHub Actions (`ci.yml` orchestrates): a `detect-scope` job runs `.github/scripts/detect-ci-scope.sh` first, then `code-quality.yml` (fmt, clippy, machete), `test-rust.yml`, and `test-typescript.yml` run **only when the change is not docs-only** (a change touching only `*.md`/`*.txt`/`*.adoc` skips the whole pipeline — pure-docs PRs pay ~zero CI). The `ci-success` job is an inline `if: always()` gate that validates each job's result against whether scope said it should run (a correctly-skipped job still yields a green gate; a failed in-scope job fails it) — it's the single check intended for branch protection. The detection logic is conservative (only ever turns jobs *off* for pure-docs changes; self-referential edits to the script itself force a full run) and unit-tested by `.github/scripts/test-detect-ci-scope.sh` (`bash` it locally). Pattern borrowed from the sibling `tasker-core` repo.
- **Addressing is by ref (UUID or decorated)** — `resource show`/`update`/`delete` and `edge assert` source/target take a single positional **ref**: a bare UUID or the decorated form `sluggify(title)-<uuid>`. Resolution is **trailing-UUID-only** — the slug half is parsed off and ignored (a stale/wrong slug half is harmless), so there is no by-slug lookup and no `--type`/`--context`/`--owner` scoping on these commands. Every printed resource carries a `ref` field (list/show/search) — copy it, paste it. `create` keeps `--type`/`--context` (it creates *into* a context); `list` keeps them as filters. The one resolver is `temper_workflow::operations::parse_ref` (pure string, no DB). See [docs/superpowers/specs/2026-06-17-ws6-surface-completeness-spec-a-addressing-collapse-design.md](docs/superpowers/specs/2026-06-17-ws6-surface-completeness-spec-a-addressing-collapse-design.md).
- **Cloud operations** — All write paths route directly through the API: `temper resource create` POSTs to `/api/ingest`; `temper resource update` PATCHes `/api/resources/{id}` with a partial-merge payload (managed_meta + open_meta + optional body trio). The local vault is a read-only projection cache — files on disk are derivative artifacts, never authoritative. Body edits work via three forms: `--body @<path>` reads from a file, `--body -` reads from stdin explicitly, and implicit stdin is auto-detected when stdin is non-TTY (e.g. `cat tmpfile.md | temper resource update <ref>`). Explicit empty input (`--body @empty.md` or piping no bytes via `--body -`) errors rather than writing an empty body; implicit empty stdin is treated as "no body update requested" so frontmatter-only updates work without piping. The implicit branch polls stdin for readiness (~300ms) before reading, so an open-but-idle non-TTY stdin (e.g. a pipe an agent/CI harness leaves connected with no piped body) resolves to "no body" instead of blocking on a read that never reaches EOF — frontmatter-only updates never hang. For a guaranteed stdin body use `--body -` (which always blocks-reads); a genuine `cat … |` pipe has data ready immediately, so it is unaffected. The show-edit-cat idiom — `temper resource show <ref>` writes the current body to a temp path, modify it, then `cat tmpfile.md | temper resource update <ref> --stage done` — PATCHes the body trio (content + content_hash + chunks_packed) in one call alongside any frontmatter flags.
- **Machine principals are registered, not discovered** — a `client_credentials` token
  authenticates only if its `client_id` appears in `kb_machine_clients` and is not revoked.
  `resolve_machine_from_claims` is lookup-or-401; there is no JIT create branch. The gate lives in
  `temper-services` (not middleware) so temper-api and temper-mcp cannot drift. Register with
  `temper admin machine provision --client-id <id> --label <l> [--team <ref>[:role]]... [--cogmap <ref>[:ro]]...`
  — reach is plural and never inferred from `--owner-team`, which records the machine's *owner* and
  is never consulted for authorization. Rotating the IdP *secret* needs no temper action (the
  `client_id` is unchanged, so authorship history stays continuous); rotating the IdP *application*
  needs `temper admin machine rebind`, which binds the new `client_id` to the existing agent profile.
  `revoke` denies authentication and nothing else — grants and memberships hang off the profile.
  No secret is ever stored. See
  [docs/superpowers/specs/2026-07-10-machine-principal-registration-design.md](docs/superpowers/specs/2026-07-10-machine-principal-registration-design.md).
- **`ingest_state` — an interrupted ingest is not a document** — `kb_resources.ingest_state` is `complete` | `in_progress`. Every ordinary create is **atomic** and is born `complete`; only a **segmented begin** (`begin_segmented_ingest`) births a resource `in_progress`, and only `resource_finalize` — after validating `expected_blocks` + `expected_body_hash` — flips it to `complete`. An `in_progress` resource is **excluded from list and search** but stays fully addressable and readable via `show` (which reports the state): hidden is not deleted, and the owner must be able to see and resume it. The exclusion lives in `substrate_read::filtered_visible_page` and in three SQL functions — **not** in `resources_visible_to`: visibility is an *authorization* predicate, completeness is a *content* predicate. The rule that places the search gates is **"`ingest_state = 'complete'` goes exactly where `r.is_active` already goes"** → `unified_search`'s `corpus` CTE (the sufficient gate; every scored candidate funnels through it), `search_vector_candidates` (anti-starvation — a partial must not eat slots in the global top-k ANN), and `search_fts_candidates` (seed hygiene — `blend0` feeds `seeds`, which anchors graph expansion). Orthogonal to `embedding_status` (`pending`/`ready`): that asks *are the vectors ready?*, this asks *are the bytes all here?*
- **MULTI-BLOCK DOES NOT MEAN SEGMENTED** — `_project_charter_set` projects a multi-block, role-tagged set and **never** fires `resource_finalized`, because a charter is not an upload. So the tempting heuristic "more than one live block AND no finalize event ⇒ an incomplete ingest" matches **every cognitive map's charter, including the L0 kernel** — a backfill on it would hide them all from list and search. There is **no `ingest_state` backfill** for exactly this reason: every pre-existing row keeps the `complete` default, and only new segmented begins are ever born `in_progress`.
- **Resource deletion is always explicit** — Use `temper resource delete <ref> [--force]`. API soft-delete (`is_active = false`, server-side row preserved) is the authoritative action. Removing a projected file from disk with `rm` is just a local cache miss — it has no server effect. To delete a resource from the server, run `temper resource delete <ref>`. To recover a projected file you removed by accident (or that's missing on a fresh device), run `temper pull <context>` — the projection re-materializes from server state. `temper resource delete` is **non-interactive on all surfaces** — there is no confirmation prompt (the pre-cloud local-mode TTY gate was removed by the cloud-only migration). The `--force` flag is therefore vestigial: agents and CI may pass it for clarity, but it changes nothing. See [docs/vault-projection-cache-design.md](docs/vault-projection-cache-design.md#the---force-flag-is-vestigial).
- **Agent-first output defaults** — Temper is agent-first: with a non-TTY stdout (how agents invoke it) and nothing configured, output defaults to **JSON** and **ANSI-free**. Two global flags control presentation: `--format json|toon` and `--color auto|always|never` (both `global = true` on the top-level `Cli`, alongside `--vault`). Each resolves through the same precedence: **CLI flag → env var → `[cli]` config → tty-aware default**. Format env is `TEMPER_FORMAT`; color env is `TEMPER_COLOR`, and the `NO_COLOR` convention is honored at the default layer (an explicit flag/env/config color overrides it). Resolution happens **once** in `main` — format via `OutputFormat::resolve_with` (`temper-cli/src/format.rs`), color via `color::apply_color_choice` which installs anstream's process-global `ColorChoice` so every `output::*` helper obeys it. Config defaults live in the optional `[cli]` section (`format`/`color`) of `~/.config/temper/config.toml` (`CliSection` in temper-core). Never emit raw ANSI — all styled output routes through `output/` (anstream/anstyle).
- **L0 kernel cognitive map (`system-default`)** — the public, root-team-joined kernel "what is temper" cogmap, born deterministically by migration `20260625000001_l0_kernel_cogmap.sql` via `cogmap_genesis` under the `system` actor. Reserved ids: cogmap `00000000-0000-0000-0005-000000000001`, telos resource `00000000-0000-0000-0005-000000000002`; root team slug `temper-system` (this migration also closes a latent gap — functions referenced that team but no production migration created it). L0 is a *living* map but **release/operator-governed, not operationally-stewarded** — it evolves by shipping **new additive migrations** that call the same mutation functions (`facet_set`/`relationship_assert`/`block_mutated`) against L0's reserved id (never by editing the immutable birth migration). Its charter declares ambient steward wake = never. See [docs/superpowers/specs/2026-06-25-cognitive-map-agent-invocation-architecture-design.md](docs/superpowers/specs/2026-06-25-cognitive-map-agent-invocation-architecture-design.md).

## Code Quality Rules

These rules apply to all code in this repository. Subagents and implementation plans must follow them. The canonical, fuller statement — the **explicit lens for code review** (opinionated best-practice, not just correctness) — lives in [docs/development/code-quality-best-practices.md](docs/development/code-quality-best-practices.md). The structural invariants below are the load-bearing summary; read the doc for the rationale, the worked examples, and the opinionated lens (single-responsibility/function-length, keys-not-loose-markers, parse-don't-validate, error-escalation, testing).

- **Typed structs over inline JSON** — Never use `serde_json::json!()` for data with a known structure. Define a struct. Compile-time type checking catches errors that runtime serialization silently passes.
- **Shared types at boundaries** — When Rust calls TypeScript (or vice versa), the wire type lives in `temper-core` with `ts-rs` derives. Both sides share the generated type. Never define a zod schema that mirrors a Rust struct manually.
- **Persistence is its own layer; surfaces dispatch through `DbBackend`** — SQL/persistence CRUD lives in a dedicated persistence layer (`temper-services/src/services/` for service logic; the lower-level write/readback core in `temper-substrate`'s `writes`/`readback`), never inline in a surface or mixed into behavior code. The `DbBackend` (in `temper-services/src/backend/`) composes the persistence layer into the `Backend` trait methods defined in `temper-workflow::operations`. Surfaces (HTTP handlers, MCP tools, CLI actions) build a backend per request and dispatch one operations command per inbound call — they do not call persistence directly for **writes**. Read paths (list, show, get_meta, search) stay service-direct on both surfaces by design (the trait projections are lossy; reads are passthroughs). Never inline `sqlx::query!()` in a surface. Never call write persistence directly from a surface — go through the backend trait. All vault writes route through `temper-client` to `temper-api` — there is no local-write surface.
- **Params structs** — Functions with more than 5 domain-related parameters get a params struct. `#[expect(clippy::too_many_arguments)]` is a smell to fix, not suppress.
- **Auth before writes** — Authorization checks go before any mutations. Never write-then-check.
- **Profile scoping** — All data queries scope through `resources_visible_to`, `can_modify_resource`, or equivalent. Even async workflows verify the profile can access the resource before writing.
- **Pino structured logging** — TypeScript uses pino (`packages/temper-cloud/src/logger.ts`) with contextual field objects. No `console.log`.
- **Schema-required defaults at create/update, not later** — Doc-type schemas in `temper-workflow/schemas/` declare required frontmatter fields. Resource creation paths (templated file write, cloud-mode ingest, MCP create) and update paths must populate every schema-required field at write time, not rely on a downstream pass to backfill. Use `apply_doc_type_defaults` and `Frontmatter::set_managed_meta` (which honors the typed `ManagedMeta` shape) to keep this consistent. For the canonical identity keys (`temper-title` and `temper-slug`), call `temper_workflow::operations::ensure_managed_identity_keys(meta, title, slug)` on **both** send-side and receive-side — this is Phase 5's symmetric defense pattern; both ends inject canonical keys from a typed source so wire payloads can never drift between them. The receive-side variant fills missing keys without overwriting present ones, so any send-side mis-call (e.g. passing `slug` to the `title` parameter) will silently propagate to storage. Pre-existing files without these fields stay valid until their next round-trip; new writes never produce them.

## SQL Query Checking

Production SQL queries use `sqlx::query!()` / `sqlx::query_as!()` / `sqlx::query_scalar!()` macros for compile-time verification against the actual schema. Exception: the `unified_search` query in `search_service.rs` uses runtime `query_as` due to pgvector `::vector` type cast incompatibility. Trivial test-fixture lookups may use runtime `sqlx::query()`; substantive test queries keep macros, cached per-crate (below).

- **Local dev:** Set `DATABASE_URL` — macros check against the live database. Note `cargo make` tasks force `SQLX_OFFLINE=true`, so `cargo make check` is the honest local probe of the committed caches.
- **CI builds:** `SQLX_OFFLINE=true` with committed `.sqlx/` cache for test jobs; the `code-quality` clippy job compiles against a **live** DB, so it will NOT catch a missing cache entry — only offline `cargo make check` does.
- **After changing any SQL:** Regenerate the workspace cache with `cargo sqlx prepare --workspace -- --all-features`
- **Test-target macro queries** (e.g. temper-api's `relationship_*_test`, temper-services' moved service queries, the e2e suite) are NOT captured by the workspace ritual — plain `cargo sqlx prepare` skips test targets. They live in per-crate caches regenerated with `--all-targets`: `cargo make prepare-api` (`crates/temper-api/.sqlx`), `cargo make prepare-services` (`crates/temper-services/.sqlx`), and `cargo make prepare-e2e` (`tests/e2e/.sqlx`). Run the matching task after changing test SQL or schema it touches. After a merge that moves service code between crates, run the full ritual in order: `cargo sqlx prepare --workspace -- --all-features` → `cargo make prepare-services` → `cargo make prepare-api` (per-crate last). Each `prepare` **rewrites its cache directory wholesale** — it prunes entries no longer emitted, so orphans clean themselves up; no manual pruning is needed. The corollary is that a per-crate cache silently rots whenever a *lib* query's signature changes and only the workspace ritual is run (macro resolution falls back to the workspace root `.sqlx`, so nothing fails — the stale entries just sit there until the next per-crate `prepare` sweeps them). Expect an unrelated-looking pile of `.sqlx` churn on the first run after such a drift, and check that each pruned entry has a same-query replacement rather than assuming the diff is noise.
- **Tests always run against a real database** (Docker Postgres locally, CI database in GitHub Actions)

## Environment

- Docker Postgres on port **5437** (not 5432, to avoid conflicts).
- `DATABASE_URL=postgresql://temper:temper@localhost:5437/temper_development`
- Linting: Rust uses clippy with `-D warnings`; TypeScript uses Biome.
- Pre-commit hook in `githooks/pre-commit`.

## Cloud Agents

For tasks delegated to cloud-based Claude Code sessions, see [docs/guides/cloud-agents.md](docs/guides/cloud-agents.md) for the task preparation guide and environment setup.
