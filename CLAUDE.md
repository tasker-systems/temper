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

  **Both surfaces must embed with the same model**, and this is enforced, not assumed — see [crates/temper-ingest/CLAUDE.md](crates/temper-ingest/CLAUDE.md).
- **temper-substrate** — Persistence write/readback core (`writes`/`readback`) plus the cognitive-map / telos-lens region producer and the YAML scenario DSL. Pulls `temper-ingest(embed)` unconditionally, so every crate depending on it links ort.
- **temper-migrate** — The migration chain and the runner that records what happens to it: the **one** `sqlx::migrate!("../../migrations")` declaration, the `Migrate` decorator that brackets each apply with a `kb_migration_ledger` entry, the additive-only router, and the `temper-migrate` binary the deploy and `cargo make db-migrate` both run. temper-substrate, temper-api and temper-services each `pub use temper_migrate::MIGRATOR`, so all ~930 `#[sqlx::test(migrator = "temper_api::MIGRATOR")]`-style call sites resolve through a re-export and none of them name this crate.

  **Its dependency list is the point of the crate — sqlx, tokio, clap, anyhow, futures-core, and nothing else.** The deploy applies additive schema *before* the API functions are built, and while this lived in temper-substrate that step compiled ort and tokenizers to decide whether a migration was additive: 2m55s of a 9m build, paid every time, because every build ends `Build cache size 1.61 GB exceeds limit of 1.50 GB. Invalidating cache.` and the next one starts cold `[observed — 2026-07-31]`. A feature flag on temper-substrate would not have worked — temper-api and temper-services need `embed`, so feature unification links ort into any workspace build regardless. Anything added here lands on the deploy's critical path ahead of the schema.

  Two consequences worth knowing. `sqlx::migrate!` embeds `migrations/` at **compile time** and cargo's tracking of that directory is unreliable, so after adding or editing a migration the crate to clean is now **`cargo clean -p temper-migrate`** — one crate, not three. And a `migrations/` path that *exists but holds no `.sql`* compiles fine and embeds zero, which would make every declaration check pass vacuously; `the_real_migration_set_is_fully_declared_and_entirely_routable` asserts non-emptiness first for exactly that reason (a missing directory, by contrast, is a compile error — measured, not assumed).
- **temper-telemetry** — How a temper process observes itself: `init`, inbound trace-context extraction, the owned request root span (`request_span`, which replaced `TraceLayer`), OTLP `export`, `link` (joining a trusted caller's trace post-auth), and `propagate` (injecting `traceparent` on **outbound** calls — the mirror of `link`, and what makes a link name a span that actually exists rather than dangling). `init` is the single logging seam for all five binaries (`init_server_logging()` JSON/stdout/`info`, `init_cli_logging()` plain/**stderr**/`warn`; the CLI's divergence is an output contract, since its stdout belongs to `temper … | jq`).

  **The OTLP exporter MUST use reqwest's *blocking* client** (`reqwest-blocking-client`, never `reqwest-client`). `BatchSpanProcessor` exports from a dedicated OS thread with no Tokio reactor, so the async client panics there with *"there is no reactor running"* and **every span is silently dropped** — the only symptom a `warn` from our own flush. Being inside a runtime at the call site is irrelevant, which is why this shipped in #535 and survived review: the in-memory flush test needs neither HTTP nor a runtime, so it could not see it. `crates/temper-telemetry/tests/live_export_client.rs` posts to a real socket and is what keeps the fix in place.

  **The CLI exports behind two switches, both required**: `TEMPER_CLI_TRACE=true` *and* an OTLP endpoint. The second switch exists because `OTEL_EXPORTER_OTLP_ENDPOINT` is often already set in a developer's shell for another project. `cli_stack` filters **per-layer** (fmt at `RUST_LOG`/`warn`, export at `info`) — a subscriber-wide `warn` starves the export layer and exports nothing, which `tests/cli_export_filter.rs` asserts. Spans drain via `shutdown_telemetry()` on **both** `main` exit paths, since the failure arm ends in `std::process::exit` and runs no destructors. Extraction is `ROOT_TRACE_FIELDS` + `TraceParent::parse` + `record_inbound_trace_context`, called from both root-span constructors via the `root_span!` macro. Built on `Registry` + layers, not `tracing_subscriber::fmt()`, so the OTLP exporter attaches as one more layer rather than a rewrite. The OTLP exporter and the post-auth span link have since shipped — see [docs/guides/open-telemetry-setup.md](docs/guides/open-telemetry-setup.md), and note that Vercel hosts no collector to export *to*: its OTel product is a drain that pushes to your vendor, and its span channel is a JS call unreachable from Rust. Inbound trace context is never a parent (decision `019f95ff-e216-7dd1-b2aa-a49d20b1cd6c`).
- **temper-mcp** — Remote MCP server (Streamable HTTP via rmcp). Deployed as a Vercel serverless function alongside temper-api. Auth0 JWT validation, OAuth discovery endpoints (RFC 8414/9728). Tools delegate to temper-services for DB access (services-direct reads, `DbBackend` writes) — it no longer depends on temper-api. Config in `src/config.rs`, tools in `src/tools/`.

### TypeScript Packages (packages/)
- **temper-cloud** — Vercel serverless functions: file upload (Vercel Blob), background processing workflows, document extraction. Uses Neon serverless Postgres, Vitest, Biome.
- **temper-ui** — SvelteKit app at temperkb.io. Uses Tailwind CSS v4, deployed to Vercel. TypeScript types are code-generated from Rust via ts-rs.
- **agent-workflows** — Deployed agent runtimes over temper-mcp (Eve now, Claude Managed Agents later). Each agent is a **self-contained Eve project** (its own TS 7 toolchain, npm lockfile) that is **workspace-isolated** — deliberately NOT a bun `workspaces` member, so it never collides with temper-cloud's TS 5.8 and the repo pre-commit never touches it. Install/run tooling from inside each agent dir (`cd steward && npm install`; a root `npm install` inherits the root's bun `overrides` and fails). First agent: `steward/` (team self-cognition steward; MCP connection with env-driven `TEMPER_MCP_URL` + platform-carried auth).

### Deployment Glue (api/)
- `api/axum.rs` — Vercel runtime adapter that wraps the Axum app (`create_app`) as a Vercel Function; serves the public API, `maxDuration: 60`.
- `api/mcp.rs` — Vercel runtime adapter for the MCP server (same pattern as axum.rs).
- `api/internal.rs` — Vercel runtime adapter for the internal/system surface (`create_internal_app`): the embed crons (`/api/embed/dispatch`, `/api/embed/warm`) and server-to-server `/internal/*`. A **separate function only so it can carry a longer `maxDuration` (300)** — Vercel timeouts are per-function, and the embed crons run ONNX work that exceeds the public 60s ceiling. `create_app` still mounts these routes too, so single-process deploys (local, e2e, self-hosted) serve the full surface from one binary; the split matters only for Vercel's per-function timeout. See [DEPLOYING.md](DEPLOYING.md#function-timeouts-per-function-not-per-route).
- `api/auth/`, `api/workflows/` — Vercel serverless endpoints (TypeScript).

**Release ≠ deploy.** Cutting a `v*` tag produces CLI binaries + a GitHub Release ([RELEASING.md](RELEASING.md)) — it deploys nothing. Each running site (temperkb.io, enterprise self-hosted) is an **independent Vercel project** consuming the repo on its own cadence, with its own Neon DB + env; CI does not deploy. Auto-deploy of `main` stays safe via the **additive-only-on-`main`** invariant, which the **build now enforces rather than assuming**: `vercel.json`'s `buildCommand` runs `temper-migrate --additive-only`, which applies the pending set only while every member declares itself `additive` and **halts at the first shape-breaking or undeclared one, failing the deploy** (`[decided — 2026-07-31, Pete]`; a warn would ship a binary ahead of its schema). Big-bang schema changes are still operator-run per target via the cutover runbook — the build refuses to deploy until the operator has taken one. The class is read from the SQL the deploying binary carries, not from `migration_current`, because `declare_migration` runs inside the migration's own transaction and a *pending* migration's class is therefore not in the database yet; that second parser is held to `scripts/migration-declaration-corpus.txt` alongside CI's awk one. See [DEPLOYING.md](DEPLOYING.md).

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

> **`cargo make openapi` and `cargo make generate-ts-types` each restale multiple committed
> artifacts, and `cargo make check` gates every one of them.** Read the `generated-artifacts`
> skill before changing a response DTO, a route, or a ts-rs-derived type.

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

> **Never add a `-E 'binary(...)'` filter to a CI test job.** Selection is `--workspace` so a new crate or test is picked up with no CI edit. A filter that makes CI green is hiding a test, not fixing one. CI jobs are split by **intention** (what they need from the environment), never by feature flag — see [.github/workflows/CLAUDE.md](.github/workflows/CLAUDE.md).

### TypeScript & UI checks

> **`cargo make check` does NOT cover temper-ui.** Its TypeScript step runs `tsc` on temper-cloud, not
> `svelte-check` on temper-ui. So a change to a **generated shared type** (`cargo make generate-ts-types`
> → `src/lib/types/generated/*.ts`) that restales a UI fixture — e.g. adding a required field to
> `ResourceRow`, which then breaks a hand-built `makeRow` test helper — passes `cargo make check` and
> fails only in CI's UI job. After any shared-type change, run `cd packages/temper-ui && bun run check`
> yourself. (If it reds on `d3-*` "implicit any" / "cannot find package" in `graph/atlas/layout/*`, that
> is a stale local `node_modules`, not your change — `bun install` first; CI installs fresh. See
> [[project_ci_flake_signatures]].)

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
- `scenario-schema` — enables `schemars::JsonSchema` derives for temper-substrate's **two** JSON-Schema snapshot suites: `tests/scenario_schema.rs` (the scenario YAML model) and `tests/payload_schema.rs` (the **event payload wire contract** — the boot-seed stamps those fixtures into `kb_event_types.payload_schema`, so repo == registry == Rust types). Runs in the **Unit** CI job and via **`cargo make test-schema`** (which `cargo make test` depends on). Regenerate with `UPDATE_SCHEMA=1 cargo make test-schema`.

  > **Run it package-scoped — `-p temper-substrate`, never `--workspace`.** Feature unification changes the emitted schema; `-p` is what the regen emits and what the boot-seed stamps. See [crates/temper-substrate/CLAUDE.md](crates/temper-substrate/CLAUDE.md).

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
- **Evidential standing is three axes and a band, never one number** — `kb_resource_standing` carries `citation_magnitude` (count of distinct **live** resource-kind sources a finding cites — monotone, the *findability* axis), `audit_coverage` (how many of those carry ≥1 audit — the *evaluated-ness* axis), and `citation_quality` (mean over the **audited subset only** of each source's decay-weighted audit value, in `[-1,1]`). `resource_standing_shape` recomputes them live at read; the memo columns are a write-cost optimization, never the read's authority. **Skepticism toward unevaluated evidence lives in the band's coverage-ratio gate, not in a poisoned mean** — quality is computed over audited sources only, so adding good-faith unaudited citations moves the *coverage* axis, never pulls a positive verdict down. `citation_quality`'s aggregation is **three-stage and the order matters**: collapse within an **auditor** first (decay-weighted, so one principal's N audits on a citation count once), then across auditors per source **weighted by each auditor's freshest audit** — NOT a plain mean, which would give one vote per principal but let a two-year-old verdict count equally with today's, breaking *"decay only arbitrates between competing audits"* and regressing a case the earlier two-stage body got right — then the plain mean across distinct audited sources. Each stage exists because of the same actor-count fallacy at a different level: a naive `LEFT JOIN` of audits onto provenance lets a source cited by three blocks vote three times, and grouping by source alone lets one principal vote N times by auditing N times (the auditor persona is structurally the biggest repeater — same credential, same model family, same instructions each run). `r_parent` is deliberately NOT `citation_magnitude`: it counts all uncorrected provenance rows including duplicates, so ten citations of one source is `r_parent = 10, citation_magnitude = 1`, and collapsing them reintroduces the actor-count fallacy. Set 3's `indep_breadth` / `adversarial_survival` / `challenge_count` and the whole `kb_independence_pairs` pairwise model are **retired** (Set 5, `20260724000120`) — they were provably inert, since nothing ever wrote the `'independent-of'` / `'challenged'` edge labels they read.
- **Citation audits are append-only events; the auditor may not grade its own work** — an audit is `(block, source, signed value ∈ [-1,1])` in `kb_citation_audits`. It **cannot be an edge** (`kb_edges` admits only `kb_resources`/`kb_cogmaps` endpoints), so it is a `citation_audited` event with a projector. There is **no supersession and no `is_superseded` column**: multiple audits of one citation, including opposite-signed ones, are the point, and the visible standing is a decay-weighted aggregate recomputed fresh from the trail rather than incrementally from its own prior value. The only uniqueness is `UNIQUE (audited_by_event_id)`, which is what makes replay idempotent instead of duplicating; `kb_citation_audits.id` has no inbound references, so it is a **masked-surrogate** table — replay mints a fresh id and the replay-stable identity is `audited_by_event_id`. Writes gate on `AuditAuthority` (`temper-services/src/authz/audit_gate.rs`): readability **minus a self-audit denial arm**, since readability alone would let the citer grade its own work and the whole adversarial premise would collapse. Both denial arms render `NotFound`, matching the evidence read's zero-rows→404 so the write never becomes an existence oracle. The auditor runs as its **own** registered machine principal with **read-only** cogmap reach — `--cogmap <ref>` defaults to *write*, which would make it `can_modify_resource` on every finding in the map and 404 every audit it attempts (see `docs/auth/machine-token-contract.md` §C). Every audit carries `audited_by_profile_id`, filled by the projector from the **owning event** (never an ambient principal — a replay must not re-attribute history to whoever ran it), and `GET /api/resources/{id}/citation-audits` reads the attributed trail: for any audit on a resource you may already see, you may see who applied it. Not a leak — transparency, and the disclosure the aggregate hides.
- **`ingest_state` — an interrupted ingest is not a document** — `kb_resources.ingest_state` is `complete` | `in_progress`. Every ordinary create is **atomic** and is born `complete`; only a **segmented begin** (`begin_segmented_ingest`) births a resource `in_progress`, and only `resource_finalize` — after validating `expected_blocks` + `expected_body_hash` — flips it to `complete`. An `in_progress` resource is **excluded from list and search** but stays fully addressable and readable via `show` (which reports the state): hidden is not deleted, and the owner must be able to see and resume it. The exclusion lives in `substrate_read::filtered_visible_page` and in three SQL functions — **not** in `resources_visible_to`: visibility is an *authorization* predicate, completeness is a *content* predicate. The rule that places the search gates is **"`ingest_state = 'complete'` goes exactly where `r.is_active` already goes"** → `unified_search`'s `corpus` CTE (the sufficient gate; every scored candidate funnels through it), `search_vector_candidates` (anti-starvation — a partial must not eat slots in the global top-k ANN), and `search_fts_candidates` (seed hygiene — `blend0` feeds `seeds`, which anchors graph expansion). Orthogonal to `embedding_status` (`pending`/`ready`): that asks *are the vectors ready?*, this asks *are the bytes all here?*
- **MULTI-BLOCK DOES NOT MEAN SEGMENTED** — `_project_charter_set` projects a multi-block, role-tagged set and **never** fires `resource_finalized`, because a charter is not an upload. So the tempting heuristic "more than one live block AND no finalize event ⇒ an incomplete ingest" matches **every cognitive map's charter, including the L0 kernel** — a backfill on it would hide them all from list and search. There is **no `ingest_state` backfill** for exactly this reason: every pre-existing row keeps the `complete` default, and only new segmented begins are ever born `in_progress`.
- **Resource deletion is always explicit** — Use `temper resource delete <ref> [--force]`. API soft-delete (`is_active = false`, server-side row preserved) is the authoritative action. Removing a projected file from disk with `rm` is just a local cache miss — it has no server effect. To delete a resource from the server, run `temper resource delete <ref>`. To recover a projected file you removed by accident (or that's missing on a fresh device), run `temper pull <context>` — the projection re-materializes from server state. `temper resource delete` is **non-interactive on all surfaces** — there is no confirmation prompt (the pre-cloud local-mode TTY gate was removed by the cloud-only migration). The `--force` flag is therefore vestigial: agents and CI may pass it for clarity, but it changes nothing. See [docs/vault-projection-cache-design.md](docs/vault-projection-cache-design.md#the---force-flag-is-vestigial).
- **Agent-first output defaults** — Temper is agent-first: with a non-TTY stdout (how agents invoke it) and nothing configured, output defaults to **JSON** and **ANSI-free**. Two global flags control presentation: `--format json|toon` and `--color auto|always|never` (both `global = true` on the top-level `Cli`, alongside `--vault`). Each resolves through the same precedence: **CLI flag → env var → `[cli]` config → tty-aware default**. Format env is `TEMPER_FORMAT`; color env is `TEMPER_COLOR`, and the `NO_COLOR` convention is honored at the default layer (an explicit flag/env/config color overrides it). Resolution happens **once** in `main` — format via `OutputFormat::resolve_with` (`temper-cli/src/format.rs`), color via `color::apply_color_choice` which installs anstream's process-global `ColorChoice` so every `output::*` helper obeys it. Config defaults live in the optional `[cli]` section (`format`/`color`) of `~/.config/temper/config.toml` (`CliSection` in temper-core). Never emit raw ANSI — all styled output routes through `output/` (anstream/anstyle).
- **L0 kernel cognitive map (`system-default`)** — the public, root-team-joined kernel "what is temper" cogmap, born deterministically by migration `20260625000001_l0_kernel_cogmap.sql` via `cogmap_genesis` under the `system` actor. Reserved ids: cogmap `00000000-0000-0000-0005-000000000001`, telos resource `00000000-0000-0000-0005-000000000002`; root team slug `temper-system` (this migration also closes a latent gap — functions referenced that team but no production migration created it). L0 is a *living* map but **release/operator-governed, not operationally-stewarded** — it evolves by shipping **new additive migrations** that call the same mutation functions (`facet_set`/`relationship_assert`/`block_mutated`) against L0's reserved id (never by editing the immutable birth migration). Its charter declares ambient steward wake = never. See [docs/superpowers/specs/2026-06-25-cognitive-map-agent-invocation-architecture-design.md](docs/superpowers/specs/2026-06-25-cognitive-map-agent-invocation-architecture-design.md).
- **Release binaries carry a per-file manifest, verified against a pinned trust root, in a verdict trichotomy that is never binary** — each macOS/Linux release publishes a `temper-v<ver>-<triple>.manifest.json` (sha256+size per shipped file, `crates/temper-cli/src/manifest.rs`) alongside the existing archive-level `.sha256`, because that sidecar measures the archive, not the installed binary. `install.sh` verifies every extracted file against it before the atomic swap, rolling back through the same run-gate machinery a merely-unrunnable binary already used. Verification is never pass/fail: it is **`verified` / `mismatch` / `unverifiable`**, and **`unverifiable` is not `mismatch`** — a `cargo install` build, a Windows install (no manifest today, by design — see below), a network-fetch failure, a release published *before* manifests existed, or a self-update performed by a pre-manifest binary all mean "we cannot tell," never "it is wrong," so none of them are ever collapsed into a false verdict in either direction. **That trichotomy binds `install.sh` itself, which is why an absent published manifest does not abort the install.** The script is served from `main` (unversioned) but installs *versioned* artifacts, so it is routinely newer than the release it is pointed at; hard-failing would break `--version v0.2.6` permanently and buy nothing, since `install.sh` verifies no attestation and the manifest it fetches shares the archive's credential — anyone able to delete it could upload one matching a tampered archive. So absence warns and proceeds (archive `.sha256` still mandatory), while a manifest that *is* published and disagrees stays fatal. The invariant lives on the **producer**: `create-github-release.sh` refuses to publish when any archive lacks its manifest, *before* creating the release, so "no manifest" can only ever mean "predates them", never "lost one". The matching recovery is that **`--verify --online` plants the offline baseline on success** (never overwriting an existing one; a failed write never changes the verdict) — legitimate precisely because the bytes it persists just cleared both the per-file comparison and the attestation check, which is what distinguishes it from retro-fitting manifests onto never-attested old releases (deliberately not done: that would manufacture provenance). This is what lets a v0.2.6 → new `temper update` reach full parity without a fresh `curl | sh`. The trust root itself is **compiled in from a committed JSON file** (`crates/temper-cli/trust/sigstore-public-good-trusted-root.json`, `attest.rs`), not fetched live via TUF at verify time — the Rust TUF ecosystem is unsettled ([rust-lang/rfcs#3724](https://github.com/rust-lang/rfcs/pull/3724) is still open), so pinning trades an open ecosystem problem for a closed, auditable one, at the cost of a **standing release obligation**: when Sigstore rotates its root, cut a release promptly, or older installs start failing attestation checks against newer releases (see [docs/guides/releasing.md](docs/guides/releasing.md#standing-obligation-sigstore-root-rotation)). Two levels carry genuinely different weight and must never be described as equivalent: offline `temper version --verify` compares an install dir against a manifest *in that same directory* (catches corruption/drift, not an attacker who replaced both), while `--verify --online` and `temper update` re-fetch the published manifest/archive and verify GitHub's build-provenance attestation over **the digest of the exact object each path just compared** — the manifest's digest for `--online` (since the manifest is the object its comparison actually depends on), the archive's digest for `update` (since the archive is what gets installed) — never the sibling object's digest, which would let a tampered manifest borrow a genuine archive's attestation. **The manifest path-containment rule exists twice and is held to one corpus** — `manifest.rs::is_contained_relative` and `install.sh::is_contained_relative_sh` decide the same security question in two languages, and neither is removable (the installer must refuse before the atomic swap and cannot call Rust). Both are checked against `scripts/install/containment-corpus.txt`, so agreement with the corpus is agreement with each other. It caught a live divergence on its first run, in **both** directions: sh accepted a leading `./x` Rust refused, and Rust accepted an interior `a/./b` sh refused — because `Path::components()` **normalizes `.` away unless it leads the path**, making the `CurDir` match arm unreachable for interior cases. Neither was dangerous, which is why both survived; the shared rule is now the stricter *no `.` component anywhere*, which on the Rust side needs a raw-segment scan since the component iterator cannot see them. The **`--archive`/`--manifest` handoff** (`update.rs`'s `download_and_verify_release` → `install.sh --archive <path> --manifest <path>`) is what keeps "one installer, one truth" (`update.rs` embeds and shells out to the same `install.sh` a fresh `curl | sh` runs) intact while closing a TOCTOU gap: without it, Rust would verify one download and the script would independently re-download and install a second, unverified one of "the same" release.

## Code Quality Rules

These rules apply to all code in this repository. Subagents and implementation plans must follow them. The canonical, fuller statement — the **explicit lens for code review** (opinionated best-practice, not just correctness) — lives in [docs/development/code-quality-best-practices.md](docs/development/code-quality-best-practices.md). The structural invariants below are the load-bearing summary; read the doc for the rationale, the worked examples, and the opinionated lens (single-responsibility/function-length, keys-not-loose-markers, parse-don't-validate, error-escalation, testing).

- **Typed structs over inline JSON** — Never use `serde_json::json!()` for data with a known structure. Define a struct. Compile-time type checking catches errors that runtime serialization silently passes.
- **Shared types at boundaries** — When Rust calls TypeScript (or vice versa), the wire type lives in `temper-core` with `ts-rs` derives. Both sides share the generated type. Never define a zod schema that mirrors a Rust struct manually.
- **Persistence is its own layer; surfaces dispatch through `DbBackend`** — SQL/persistence CRUD lives in a dedicated persistence layer (`temper-services/src/services/` for service logic; the lower-level write/readback core in `temper-substrate`'s `writes`/`readback`), never inline in a surface or mixed into behavior code. The `DbBackend` (in `temper-services/src/backend/`) composes the persistence layer into the `Backend` trait methods defined in `temper-workflow::operations`. Surfaces (HTTP handlers, MCP tools, CLI actions) build a backend per request and dispatch one operations command per inbound call — they do not call persistence directly for **writes**. Read paths (list, show, get_meta, search) stay service-direct on both surfaces by design (the trait projections are lossy; reads are passthroughs). Never inline `sqlx::query!()` in a surface. Never call write persistence directly from a surface — go through the backend trait. All vault writes route through `temper-client` to `temper-api` — there is no local-write surface.
- **Params structs** — Functions with more than 5 domain-related parameters get a params struct. `#[expect(clippy::too_many_arguments)]` is a smell to fix, not suppress.
- **Auth before writes** — Authorization checks go before any mutations. Never write-then-check.
- **Profile scoping** — All data queries scope through `resources_visible_to`, `can_modify_resource`, or equivalent. Even async workflows verify the profile can access the resource before writing.
- **Pino structured logging** — TypeScript uses pino (`packages/temper-cloud/src/logger.ts`) with contextual field objects. No `console.log`.
- **Span fields: acts get their own span; never record onto the root** — Every request gets a root span (`http_request` in temper-api, **`mcp_request`** in temper-mcp — deliberately different names, since temper-client's outbound span is *also* `http_request` and three things under one name are unreadable once exported). An act's ids (`correlation_id`, `invocation_id`) arrive in the **request body** (`ActInput` → `ActContext`), so the transport-level root span cannot carry them (built by the `root_span` middleware in each surface, which runs before the body is read — it replaced `tower_http`'s `TraceLayer`, whose span is cloned into the response body and so can never be observed closed by a flush). Write commands in `db_backend.rs` therefore carry **`#[act_span]`** (`temper-macros`), which expands to the `#[tracing::instrument]` with the field set and a mandatory `skip_all` — commands hold bodies and secrets — and build their `EventContext` via `act_context(&cmd.act)`, which does the mapping **and** records the ids. The attribute is one declaration on purpose: eleven hand-written copies drifted, and `record_citation_audit` shipped with none of it. **The trap:** with no child spans, `Span::current().record(...)` in a handler resolves to the root span, so recording act ids there *works* — right up until the first nested span makes it silently wrong. The e2e gate (`tests/e2e/tests/logging_test.rs`) asserts the carrying span is **not** the root for exactly this reason. Acts are conditional by design: C/U/D are Acts, a read is just a request, so clause 2 fires only where an act exists. Field set has one definition — `temper_services::backend::ACT_SPAN_FIELDS`, tied to the attribute by `act_span_declares_every_act_field` (a constant nothing asserts its **consumers** against prevents no drift at all). Inbound trace context is never a parent; a **trusted** caller — one that passed an authentication gate — is joined by span *link* post-auth (`temper_telemetry::link_trusted_caller`). See [docs/development/span-field-conventions.md](docs/development/span-field-conventions.md).
- **Schema-required defaults at create/update, not later** — Doc-type schemas in `temper-workflow/schemas/` declare required frontmatter fields. Resource creation paths (templated file write, cloud-mode ingest, MCP create) and update paths must populate every schema-required field at write time, not rely on a downstream pass to backfill. Use `apply_doc_type_defaults` and `Frontmatter::set_managed_meta` (which honors the typed `ManagedMeta` shape) to keep this consistent. For the canonical identity keys (`temper-title` and `temper-slug`), call `temper_workflow::operations::ensure_managed_identity_keys(meta, title, slug)` on **both** send-side and receive-side — this is Phase 5's symmetric defense pattern; both ends inject canonical keys from a typed source so wire payloads can never drift between them. The receive-side variant fills missing keys without overwriting present ones, so any send-side mis-call (e.g. passing `slug` to the `title` parameter) will silently propagate to storage. Pre-existing files without these fields stay valid until their next round-trip; new writes never produce them.

## SQL Query Checking

Production SQL uses `sqlx::query!()`-family macros, verified at compile time against the real schema. **After changing any SQL or a migration, the `.sqlx` caches must be regenerated — and the workspace ritual does NOT cover test-target queries.** Read the `sqlx-query-cache` skill before touching a query macro or a migration. Tests always run against a real database (Docker Postgres locally, CI database in GitHub Actions).

> **`error[E0282]: type annotations needed` on a `query!` you did not touch means your dev DB is
> behind `migrations/` — not that the `.sqlx` cache is stale.** Regenerating the cache will not fix
> it. sqlx's compile-time macros read `.env` (`sqlx-macros-core` → `dotenvy`), so a **bare**
> `cargo check`/`cargo nextest` picks up `DATABASE_URL` and verifies against the **live** dev
> database, bypassing the committed cache. `cargo make` tasks are immune — Makefile.toml sets
> `SQLX_OFFLINE = "true"` globally — which is why this only ever bites outside cargo-make and looks
> like a cache problem. Fix: `cargo make docker-up`, which now waits for health and applies pending
> migrations (or `cargo make db-migrate` alone, without cycling the container).

## Environment

- Docker Postgres on port **5437** (not 5432, to avoid conflicts).
- `DATABASE_URL=postgresql://temper:temper@localhost:5437/temper_development`
- Pre-commit hook in `githooks/pre-commit`.

## Cloud Agents

For tasks delegated to cloud-based Claude Code sessions, see [docs/guides/cloud-agents.md](docs/guides/cloud-agents.md) for the task preparation guide and environment setup.
