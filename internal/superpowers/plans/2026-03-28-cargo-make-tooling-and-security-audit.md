# Cargo-Make Tooling & Security/Architecture Audit Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Establish cargo-make quality gates and verify architectural soundness before Vercel deployment.

**Architecture:** Two sequential workstreams. Part 1 creates the cargo-make tooling (Makefile.toml, .cargo/config.toml, nextest config). Part 2 runs four parallel OWASP security scans followed by a conversational architecture review. Findings from Part 2 are fixed in-session where possible.

**Tech Stack:** cargo-make, cargo-nextest, cargo-audit, cargo-machete, PostgreSQL 18, pgvector, axum, sqlx, jsonwebtoken

---

## Part 1: Cargo-Make Tooling

### Task 1: Create base-tasks.toml

**Files:**
- Create: `tools/cargo-make/base-tasks.toml`

- [ ] **Step 1: Create the tools/cargo-make directory and base-tasks.toml**

```toml
# =============================================================================
# Temper — Base cargo-make Task Templates
# =============================================================================
# Shared task definitions extended by per-crate Makefile.toml files.

[tasks.base-rust-format-check]
description = "Check Rust formatting"
command = "cargo"
args = ["fmt", "--all", "--", "--check"]

[tasks.base-rust-format-fix]
description = "Fix Rust formatting"
command = "cargo"
args = ["fmt", "--all"]

[tasks.base-rust-lint]
description = "Run Clippy lints"
command = "cargo"
args = ["clippy", "--all-targets", "--all-features", "--", "-D", "warnings"]

[tasks.base-rust-lint-fix]
description = "Fix Clippy issues"
command = "cargo"
args = ["clippy", "--all-targets", "--all-features", "--fix", "--allow-dirty", "--allow-staged"]

[tasks.base-rust-test]
description = "Run tests"
command = "cargo"
args = ["nextest", "run", "--workspace"]

[tasks.base-rust-docs]
description = "Check documentation builds"
command = "cargo"
args = ["doc", "--workspace", "--no-deps", "--document-private-items"]
```

- [ ] **Step 2: Verify the file was created**

Run: `cat tools/cargo-make/base-tasks.toml | head -5`
Expected: The file header comment

- [ ] **Step 3: Commit**

```bash
git add tools/cargo-make/base-tasks.toml
git commit -m "build: add cargo-make base-tasks.toml with shared task templates"
```

---

### Task 2: Create main.toml with all workspace tasks

**Files:**
- Create: `tools/cargo-make/main.toml`

- [ ] **Step 1: Create main.toml extending base-tasks.toml**

```toml
# =============================================================================
# Temper — Workspace-Level cargo-make Tasks
# =============================================================================
# Composite tasks that operate across the entire workspace.
# Extended by the root Makefile.toml.

extend = "./base-tasks.toml"

# =============================================================================
# Composite Tasks
# =============================================================================

[tasks.check]
description = "Run all quality checks"
dependencies = ["rust-fmt-check", "rust-clippy", "rust-docs", "rust-machete"]

[tasks.test]
description = "Run tests (unit tests, no feature flags)"
command = "cargo"
args = ["nextest", "run", "--workspace"]

[tasks.test-db]
description = "Run tests including database integration tests (requires Docker Postgres)"
command = "cargo"
args = ["nextest", "run", "--workspace", "--features", "test-db"]

[tasks.test-all]
description = "Run all test tiers including embedder tests"
command = "cargo"
args = ["nextest", "run", "--workspace", "--features", "test-db,test-embedder"]

[tasks.fix]
description = "Auto-fix formatting and lint issues"
dependencies = ["rust-fmt-fix", "rust-clippy-fix"]

[tasks.build]
description = "Build all workspace crates"
command = "cargo"
args = ["build", "--workspace", "--all-features"]

[tasks.audit]
description = "Run security audit on dependencies"
command = "cargo"
args = ["audit"]

[tasks.run]
description = "Run the temper-api server locally"
command = "cargo"
args = ["run", "-p", "temper-api"]

# =============================================================================
# Sub-Tasks
# =============================================================================

[tasks.rust-fmt-check]
description = "Check Rust formatting"
command = "cargo"
args = ["fmt", "--all", "--", "--check"]

[tasks.rust-fmt-fix]
description = "Fix Rust formatting"
command = "cargo"
args = ["fmt", "--all"]

[tasks.rust-clippy]
description = "Run Clippy lints"
command = "cargo"
args = ["clippy", "--all-targets", "--all-features", "--", "-D", "warnings"]

[tasks.rust-clippy-fix]
description = "Fix Clippy issues"
command = "cargo"
args = ["clippy", "--all-targets", "--all-features", "--fix", "--allow-dirty", "--allow-staged"]

[tasks.rust-docs]
description = "Check documentation builds"
command = "cargo"
args = ["doc", "--workspace", "--no-deps", "--document-private-items"]

[tasks.rust-machete]
description = "Check for unused Cargo dependencies"
command = "cargo"
args = ["machete", "--with-metadata"]

# =============================================================================
# Docker Tasks
# =============================================================================

[tasks.docker-up]
description = "Start PostgreSQL development database"
script = ["docker compose up -d"]

[tasks.docker-down]
description = "Stop PostgreSQL development database"
script = ["docker compose down"]

[tasks.docker-down-volumes]
description = "Stop PostgreSQL and remove data volumes"
script = ["docker compose down -v"]
```

- [ ] **Step 2: Verify the file was created**

Run: `cat tools/cargo-make/main.toml | head -5`
Expected: The file header comment

- [ ] **Step 3: Commit**

```bash
git add tools/cargo-make/main.toml
git commit -m "build: add cargo-make main.toml with workspace composite tasks"
```

---

### Task 3: Create root Makefile.toml

**Files:**
- Create: `Makefile.toml`

- [ ] **Step 1: Create root Makefile.toml**

```toml
# =============================================================================
# Temper — cargo-make Root Task Definitions
# =============================================================================
#
# Unified task runner for the temper workspace.
#
# Quick Start:
#   cargo make check       Run all quality checks
#   cargo make test        Run all tests (unit only)
#   cargo make test-db     Run tests with database (requires Docker)
#   cargo make test-all    Run all test tiers
#   cargo make fix         Auto-fix all issues
#   cargo make build       Build everything
#   cargo make audit       Security audit
#   cargo make run         Run temper-api locally
#
# Docker:
#   cargo make docker-up   Start PostgreSQL
#   cargo make docker-down Stop PostgreSQL
#
# See: docs/superpowers/specs/2026-03-28-cargo-make-tooling-and-security-audit-design.md

extend = "./tools/cargo-make/main.toml"

[config]
default_to_workspace = false
skip_core_tasks = true

[tasks.default]
description = "Default: show available tasks"
script = '''
echo "Temper — cargo-make Tasks"
echo "========================="
echo ""
echo "Quality:"
echo "  cargo make check       Run all quality checks (fmt, clippy, docs, machete)"
echo "  cargo make fix         Auto-fix formatting and lint issues"
echo "  cargo make audit       Security audit on dependencies"
echo ""
echo "Testing:"
echo "  cargo make test        Unit tests (no feature flags)"
echo "  cargo make test-db     + database integration tests (requires Docker Postgres)"
echo "  cargo make test-all    + embedder tests (downloads HF model)"
echo ""
echo "Build & Run:"
echo "  cargo make build       Build all workspace crates"
echo "  cargo make run         Run temper-api server locally"
echo ""
echo "Docker:"
echo "  cargo make docker-up   Start PostgreSQL"
echo "  cargo make docker-down Stop PostgreSQL"
echo ""
echo "Run 'cargo make --list-all-steps' for all tasks"
'''
```

- [ ] **Step 2: Run `cargo make` to verify it works**

Run: `cargo make`
Expected: The help text listing all available tasks

- [ ] **Step 3: Commit**

```bash
git add Makefile.toml
git commit -m "build: add root Makefile.toml — cargo-make entry point"
```

---

### Task 4: Create .cargo/config.toml

**Files:**
- Create: `.cargo/config.toml`

- [ ] **Step 1: Create .cargo/config.toml**

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

- [ ] **Step 2: Verify cargo still builds**

Run: `cargo check --workspace 2>&1 | tail -3`
Expected: `Finished` with no errors

- [ ] **Step 3: Commit**

```bash
git add .cargo/config.toml
git commit -m "build: add .cargo/config.toml with dev/release profiles"
```

---

### Task 5: Create .config/nextest.toml

**Files:**
- Create: `.config/nextest.toml`

- [ ] **Step 1: Create nextest config**

```toml
[profile.default]
slow-timeout = { period = "60s", terminate-after = 2 }
status-level = "skip"
final-status-level = "flaky"

[profile.ci]
retries = 1
fail-fast = false
```

- [ ] **Step 2: Verify nextest runs with the config**

Run: `cargo nextest run --workspace 2>&1 | tail -10`
Expected: All unit tests pass (temper-cli tests)

- [ ] **Step 3: Commit**

```bash
git add .config/nextest.toml
git commit -m "build: add nextest config with default and CI profiles"
```

---

### Task 6: Validate the full cargo-make surface

No new files — verify everything works end-to-end.

- [ ] **Step 1: Run `cargo make check`**

Run: `cargo make check`
Expected: fmt-check, clippy, docs, and machete all pass. If machete finds unused deps, fix them before proceeding.

- [ ] **Step 2: Run `cargo make test`**

Run: `cargo make test`
Expected: All unit tests pass via nextest

- [ ] **Step 3: Run `cargo make test-db`**

Run: `cargo make test-db`
Expected: All tests including temper-api integration tests pass (Docker Postgres must be running)

- [ ] **Step 4: Run `cargo make audit`**

Run: `cargo make audit`
Expected: No critical vulnerabilities. Note any advisories.

- [ ] **Step 5: Run `cargo make build`**

Run: `cargo make build`
Expected: All crates build with all features

- [ ] **Step 6: Fix any issues found, then commit**

```bash
git add -A
git commit -m "build: fix issues found during cargo-make validation"
```

Skip this commit if no issues were found.

---

### Task 7: Update pre-commit hook to use cargo-make

**Files:**
- Modify: `githooks/pre-commit`

- [ ] **Step 1: Update the pre-commit hook**

Replace the contents of `githooks/pre-commit` with:

```bash
#!/usr/bin/env bash
set -euo pipefail

echo "==> Running pre-commit checks..."

echo "  Checking formatting..."
cargo fmt --all -- --check
echo "  ✓ Formatting OK"

echo "  Running Clippy..."
cargo clippy --all-targets --all-features -- -D warnings
echo "  ✓ Clippy OK"

echo "  Checking docs..."
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --document-private-items --quiet
echo "  ✓ Docs OK"

echo "==> All pre-commit checks passed."
```

Note: We add `--all-features` to clippy and `--workspace` to doc to match the cargo-make tasks. The hook runs direct cargo commands (not cargo-make) so it works even if cargo-make isn't installed — pre-commit hooks should have minimal tool dependencies.

- [ ] **Step 2: Verify the hook passes**

Run: `bash githooks/pre-commit`
Expected: All checks pass

- [ ] **Step 3: Commit**

```bash
git add githooks/pre-commit
git commit -m "build: update pre-commit hook to match cargo-make check flags"
```

---

## Part 2: Security & Architecture Audit

### Task 8: Run parallel code-level OWASP sweep

**No file changes initially — this is a research task that produces findings.**

Launch four parallel agents, one per scan area:

- [ ] **Step 1: Launch parallel security scan agents**

Launch four agents simultaneously:

**Agent 1 — Auth & Session:**
Scan `crates/temper-api/src/middleware/auth.rs`, `crates/temper-api/src/state.rs`, `crates/temper-api/src/services/profile_service.rs`, and `crates/temper-api/src/config.rs`. Check:
- JWT algorithm restriction (must be EdDSA only — verify `Validation::new(Algorithm::EdDSA)`)
- Audience/issuer validation (check all code paths)
- Token expiry enforcement (verify `exp` is validated by jsonwebtoken)
- Middleware bypass: verify every route in `routes.rs` that should be protected goes through `require_auth`
- Key rotation: verify JWKS cache TTL refresh works correctly
- Claims handling: verify no unvalidated JWT claims flow directly into SQL

**Agent 2 — SQL & Data:**
Scan all files in `crates/temper-api/src/services/`. Check:
- Every SQL query uses bind parameters (`$1`, `$2`, etc.) — no string interpolation
- Every resource query composes with `resources_visible_to()` CTE
- Every mutation checks `can_modify_resource()` before modifying
- Request bodies are deserialized into typed structs (not raw `serde_json::Value` passed to SQL)
- No query path returns data belonging to a different profile without access control

**Agent 3 — Error Handling & Info Leakage:**
Scan `crates/temper-api/src/error.rs`, all handlers in `crates/temper-api/src/handlers/`, and `crates/temper-api/src/main.rs`. Check:
- `From<sqlx::Error>` never leaks database error details (verify the tracing::error + generic message pattern)
- No handler returns raw error strings that reveal schema or internal paths
- Panic handling: verify axum catches panics (it does by default, but confirm no `unwrap()` on user input)
- Error response shape is always `ErrorBody` (no raw strings as responses)

**Agent 4 — Dependencies:**
Run `cargo audit` and `cargo machete --with-metadata`. Also:
- Review `Cargo.toml` files for unnecessary feature flags that expand attack surface
- Check for any `unsafe` blocks in temper's own code
- Review dependency tree for vendored C code (`cargo tree --prefix none | sort -u | wc -l` for scale)

- [ ] **Step 2: Collect findings from all four agents**

Compile findings into a single list, classified by severity (Critical/High/Medium/Low).

- [ ] **Step 3: Fix any Critical or High findings immediately**

Apply code changes for any Critical or High severity findings. Commit each fix individually with a descriptive message.

- [ ] **Step 4: Document Medium and Low findings**

Add Medium/Low findings to the audit output document for tracking. These don't block deployment.

---

### Task 9: Architecture review — tenant isolation and schema audit

**Files:**
- Read: `migrations/20260326000001_r2_schema.sql`
- Read: `migrations/20260326000002_r2_seed.sql`

- [ ] **Step 1: Audit kb_contexts for tenant isolation**

Read the schema and verify:
- `kb_contexts` table has no `profile_id` or `team_id` foreign key — **this is the known gap**
- Document the required migration: add `owner_profile_id UUID NOT NULL REFERENCES kb_profiles(id)` to `kb_contexts`
- Note impact on `resources_visible_to()` — contexts should be visible based on their owner or team association
- Note impact on seed data — the seeded contexts (temper, storyteller, tasker, etc.) are currently user-specific project names, confirming they should be profile-scoped

- [ ] **Step 2: Audit all other entities for scoping correctness**

Walk through each table in the schema:

| Entity | Check | Expected Result |
|--------|-------|-----------------|
| `kb_doc_types` | Global (no owner FK) | Correct — system-level shared vocabulary |
| `kb_behaviors` | Global (no owner FK) | Correct — system-level |
| `kb_doc_type_behaviors` | Global (FK to doc_types + behaviors) | Correct |
| `kb_lifecycle_stages` | FK to doc_types | Correct — system-level |
| `resources` | `owner_profile_id` FK | Correct |
| `kb_chunks` | FK to resources (cascade) | Correct — scoped via resource |
| `kb_current_chunks` | View on kb_chunks | Correct |
| `kb_ingestion_records` | FK to resources (cascade) | Correct |
| `kb_profiles` | Self-scoping | Correct |
| `kb_profile_auth_links` | FK to profiles (cascade) | Correct |
| `kb_teams` | `created_by_profile_id` FK | Correct |
| `kb_team_members` | FK to teams + profiles | Correct |
| `kb_team_resources` | FK to teams + resources | Correct |
| `kb_team_invitations` | FK to teams | Correct |
| `kb_transfers` | `from_profile_id` + `to_profile_id` FKs | Correct |
| `kb_device_sync_state` | `profile_id` FK | Correct |
| `kb_events` | `profile_id` FK | Correct |
| `kb_workflowable_states` | FK to resources (cascade) | Correct |
| `kb_sequenceable_states` | FK to resources (cascade) | Correct |
| `kb_assignable_states` | FK to resources (cascade) | Correct — but `author`/`assignee` are VARCHAR, not profile FKs — note for future |
| `kb_taggable_states` | FK to resources (cascade) | Correct |

- [ ] **Step 3: Audit seed data for single-tenant assumptions**

Review `migrations/20260326000002_r2_seed.sql`:
- Seeded `kb_contexts` (temper, storyteller, tasker, knowledge, writing) — these are user-specific project names, confirming contexts need profile scoping
- Seeded `System` and `Anonymous` profiles — these are sentinel values, acceptable
- Seeded doc types, behaviors, lifecycle stages — system-level, correct

Document: seed contexts will need to be removed or converted to per-profile creation once `kb_contexts` is scoped.

- [ ] **Step 4: Audit access control functions**

Review `resources_visible_to()`, `can_modify_resource()`, `can_manage_team()`:
- Verify `resources_visible_to()` checks both `owner_profile_id` and team membership — **confirmed correct**
- Verify `can_modify_resource()` checks ownership and team role (not watcher) — **confirmed correct**
- Verify `can_manage_team()` role hierarchy (owner for delete, owner/maintainer for invite/remove/change_role) — **confirmed correct**
- Note: none of these functions check `is_active` on the resource — verify the calling code filters `is_active = true`

- [ ] **Step 5: Document architecture findings**

Compile all findings with severity and recommended fixes. Known issues to document:
1. `kb_contexts` lacks profile/team scoping (High — needs migration in upcoming task)
2. `kb_assignable_states.author`/`assignee` are VARCHAR not profile FKs (Low — future concern)
3. Seed contexts are user-specific data hardcoded in migrations (Medium — remove when contexts are scoped)
4. `resources_visible_to()` doesn't filter `is_active` internally — relies on callers (Medium — consider adding to the function)

---

### Task 10: Write audit findings document and commit all changes

**Files:**
- Create: `docs/security/2026-03-28-pre-deployment-audit.md`

- [ ] **Step 1: Write the audit findings document**

Create `docs/security/2026-03-28-pre-deployment-audit.md` with all findings from Tasks 8 and 9, organized by severity:

```markdown
# Pre-Deployment Security & Architecture Audit

**Date:** 2026-03-28
**Scope:** temper-api crate + full schema, pre-I4 (Vercel deployment)

## Summary

[Total findings by severity]

## Critical Findings

[Any critical findings, or "None"]

## High Findings

[High findings with fix status]

## Medium Findings

[Medium findings with recommendations]

## Low Findings

[Low findings for future tracking]

## Architecture Notes

[Key observations about tenant isolation, trust boundaries, access control]
```

- [ ] **Step 2: Commit the findings document and any remaining fixes**

```bash
git add docs/security/2026-03-28-pre-deployment-audit.md
git commit -m "docs: pre-deployment security and architecture audit findings"
```

- [ ] **Step 3: Run `cargo make check` and `cargo make test-db` to verify everything is green**

Run: `cargo make check && cargo make test-db`
Expected: All checks and tests pass

- [ ] **Step 4: Final commit if needed**

```bash
git add -A
git commit -m "fix: address audit findings"
```

Skip if no changes needed.
