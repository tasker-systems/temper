# Cargo-Make Tooling & Security/Architecture Audit

**Date:** 2026-03-28
**Status:** Pre-I4 checkpoint — establish quality gates and verify architectural soundness before Vercel deployment

## Context

temper is approaching its first production deployment via Vercel + Neon (I4). The sibling projects (tasker-core, storyteller) both use cargo-make as their unified task runner. temper currently has CI workflows (fmt/clippy/docs/audit) and pre-commit hooks but no cargo-make setup, no nextest, and no machete.

This spec covers two independent workstreams executed in sequence:
1. **Cargo-make tooling** — establish the quality gate commands
2. **Security & architecture audit** — verify the codebase is production-ready

## Part 1: Cargo-Make Tooling

### Design Decisions

- **Storyteller-weight, not tasker-core-weight.** temper is a pure-Rust workspace with a single database. No polyglot workers, no split-db modes, no cluster testing. The two-file pattern (`main.toml` + `base-tasks.toml`) is the right complexity level.
- **No aliases.** Full command names only — clarity over brevity.
- **nextest from the start.** Better parallel execution, output formatting, and failure reporting.
- **No deploy tasks.** Vercel's git integration handles preview/production deployment automatically (see I11). Cargo-make focuses on quality gates.

### File Structure

```
temper/
├── Makefile.toml                     # Root entry point, extends main.toml
├── .cargo/config.toml                # Dev/release profiles
├── .config/nextest.toml              # Nextest profiles
└── tools/cargo-make/
    ├── main.toml                     # Workspace composite tasks
    └── base-tasks.toml               # Primitives for crate-level extension
```

### Task Surface

| Command | Dependencies / Implementation |
|---------|-------------------------------|
| `cargo make check` | `rust-fmt-check` + `rust-clippy` + `rust-docs` + `rust-machete` |
| `cargo make test` | `nextest run --workspace` (no feature flags — unit tests only) |
| `cargo make test-db` | `nextest run --workspace --features test-db` (requires Docker Postgres) |
| `cargo make test-all` | `nextest run --workspace --features test-db,test-embedder` |
| `cargo make fix` | `rust-fmt-fix` + `rust-clippy-fix` |
| `cargo make build` | `cargo build --workspace --all-features` |
| `cargo make audit` | `cargo audit` |
| `cargo make run` | `cargo run -p temper-api` |
| `cargo make docker-up` | `docker compose up -d` |
| `cargo make docker-down` | `docker compose down` |

### Sub-Tasks (in main.toml)

| Task | Command |
|------|---------|
| `rust-fmt-check` | `cargo fmt --all -- --check` |
| `rust-fmt-fix` | `cargo fmt --all` |
| `rust-clippy` | `cargo clippy --all-targets --all-features -- -D warnings` |
| `rust-clippy-fix` | `cargo clippy --all-targets --all-features --fix --allow-dirty --allow-staged` |
| `rust-docs` | `cargo doc --workspace --no-deps --document-private-items` |
| `rust-machete` | `cargo machete --with-metadata` |

### .cargo/config.toml

```toml
[env]
WORKSPACE_PATH = { value = ".", relative = true }

[profile.dev]
incremental = true
split-debuginfo = "unpacked"

[profile.release]
incremental = false
opt-level = 3
strip = true
lto = "fat"
panic = "abort"
codegen-units = 1
```

### .config/nextest.toml

```toml
[profile.default]
slow-timeout = { period = "60s", terminate-after = 2 }
status-level = "skip"
final-status-level = "flaky"

[profile.ci]
retries = 1
fail-fast = false
```

### Tool Prerequisites

| Tool | Install |
|------|---------|
| cargo-make | `cargo install cargo-make` |
| cargo-nextest | `cargo install cargo-nextest --locked` |
| cargo-audit | `cargo install cargo-audit` |
| cargo-machete | `cargo install cargo-machete` |

### What's Deferred (to I11)

- CI-specific tasks (ci-check, ci-test, ci-scope-detect)
- Release tasks (release-prepare, tag, publish)
- Coverage tasks (cargo-llvm-cov)
- Env-file generation (temper uses a simple .env)

---

## Part 2: Security & Architecture Audit

### Code-Level OWASP Sweep

Four parallel scan areas. Each finding classified as Critical / High / Medium / Low.

#### Area 1: Auth & Session
- JWT validation completeness: algorithm restriction, audience/issuer checks, expiry enforcement
- Key rotation handling in JWKS store
- Middleware bypass: verify no protected route accidentally skips `require_auth`
- Token claims handling: ensure no unvalidated claims flow into business logic

#### Area 2: SQL & Data
- SQL injection: all queries must use bind parameters, no string interpolation
- Access control bypass: every resource query must go through `resources_visible_to()`, every mutation through `can_modify_resource()`
- Mass assignment: verify request bodies are deserialized into typed structs (not raw Value passthrough to SQL)
- Data leakage across tenants: no query path that returns another user's data

#### Area 3: Error Handling & Info Leakage
- Database error details in responses (previously fixed — verify no regression)
- Stack traces or internal paths in error bodies
- Verbose error messages that reveal schema structure or business logic
- Panic handling: ensure no unhandled panics crash the server

#### Area 4: Dependencies
- `cargo audit` for known CVEs
- `cargo machete` for unused dependencies (reduce attack surface)
- Review dependency feature flags for unnecessary capabilities
- Check for vendored C code or unsafe blocks in dependency tree

### Architecture Review

#### 1. Tenant Isolation — Schema Audit

Systematically verify every entity for proper scoping:

| Entity | Expected Scoping | Check |
|--------|-----------------|-------|
| `kb_contexts` | Profile or team-owned | **Known gap** — currently unscoped, needs migration |
| `kb_doc_types` | System-level (global) | **Intentionally unscoped** — shared vocabulary |
| `resources` | Profile-owned, team-visible via `kb_team_resources` | Verify FK chain |
| `kb_current_chunks` | Scoped via resource ownership | Verify no direct access path |
| `kb_events` | Profile + resource visibility | Verify event service uses CTE |
| `kb_teams` | Creator-owned | Verify ownership chain |
| `kb_team_members` | Team-scoped | Verify join integrity |
| `kb_team_resources` | Team-scoped | Verify access level enforcement |
| `kb_team_invitations` | Team-scoped | Verify invitation doesn't leak team data |
| `kb_transfers` | Source/target profile scoped | Verify both parties' visibility |
| `kb_device_sync_state` | Profile-scoped | Verify no cross-profile leakage |
| `kb_profile_auth_links` | Profile-scoped | Verify auth reconciliation doesn't merge wrong profiles |

#### 2. Trust Boundaries

Map untrusted input entry points:
- JWT claims (from Neon Auth — trusted issuer but untrusted content)
- Request bodies (JSON deserialization)
- Query parameters (pagination, filters)
- URL path parameters (resource IDs)

Verify each boundary has validation before business logic.

#### 3. Access Control Model

Verify `resources_visible_to()` and `can_modify_resource()` are the sole gatekeepers:
- No service bypasses these functions for "convenience"
- No admin/system path that skips access control
- Soft-delete (`is_active = false`) is consistently enforced

#### 4. API Surface

- HTTP method semantics (GET is safe, DELETE is idempotent)
- Response shape consistency (all errors use ErrorBody)
- Rate limiting considerations (note: Vercel provides edge-level rate limiting)
- CORS configuration review

#### 5. Schema Design for Multi-Tenancy

Beyond the entity-by-entity check:
- Seed data audit: do migrations create data that assumes single-tenant?
- Foreign key chains: does every ownership path terminate at a profile or team?
- Indexes: are access-control queries properly indexed?

### Output

Findings document with:
- Severity classification (Critical/High/Medium/Low)
- What's wrong
- Why it matters
- Concrete fix (code, migration, or architectural decision)

Immediate fixes applied in this session. Schema-level changes documented as actionable items for upcoming implementation tasks.
