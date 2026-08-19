# Wave 1 Phase 3a — DbBackend Foundation Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Land the architectural foundation for `DbBackend` — the first concrete impl of the `temper-core::operations::Backend` trait — wrapping existing `temper-api` services without rewiring any HTTP handler or MCP tool. All six trait methods implemented, dark-launched, verified by trait-impl integration tests against a real `test-db` Postgres.

**Architecture:** `DbBackend` is constructed per request with `(pool, profile_id, device_id, surface)`. Each trait method is a thin translator that converts a `temper-core::operations::*Resource` command into the existing service function's request shape, calls the service, synthesizes one coarse `DomainEvent` on success, and returns `CommandOutput<T>`. No SQL moves. No service internals change. Phase 3b (HTTP handler migration) and Phase 3c (MCP tool migration) are separate, subsequent plans.

**Tech Stack:** Rust 2024 edition, sqlx with compile-time-checked queries, async-trait, axum (consumer in 3b), tokio, sqlx::test for isolated per-test DB.

**Spec:** `docs/superpowers/specs/2026-05-07-wave1-phase3-dbbackend-design.md`
**Predecessors (merged in PR #65):** Phase 1 (operations scaffolding) and Phase 2 (shared pure actions).

**Branch convention (this repo):** `jct/wave1-phase3a-dbbackend-foundation`. Create the branch before Task 1 if not already created.

---

## File Structure

| File | Action | Responsibility |
|---|---|---|
| `crates/temper-core/src/error.rs` | Modify | Extend `TemperError` with `Forbidden`, `BadRequest`, `Conflict`, `Unauthorized` variants. |
| `crates/temper-core/src/operations/actions.rs` | Modify | Add `apply_defaults_value(&str, &mut serde_json::Value)` sibling to existing `apply_defaults`. |
| `crates/temper-core/src/operations/mod.rs` | Modify | Re-export `apply_defaults_value`. |
| `crates/temper-api/src/error.rs` | Modify | Add `impl From<ApiError> for TemperError` so DbBackend methods can convert service errors at the trait boundary. |
| `crates/temper-api/src/services/ingest_service.rs` | Modify | Replace two `apply_managed_defaults` call sites (lines 403, 655) with `temper_core::operations::apply_defaults_value`. Drop the local `use` of `apply_managed_defaults`. |
| `crates/temper-api/src/lib.rs` | Modify | Add `pub mod backend;` so `DbBackend` is reachable from handlers (3b) and MCP (3c). |
| `crates/temper-api/src/backend/mod.rs` | Create | Module root: re-export `DbBackend` and submodules. |
| `crates/temper-api/src/backend/db_backend.rs` | Create | `DbBackend` struct + `impl Backend for DbBackend` (all 6 methods). |
| `crates/temper-api/src/backend/translators.rs` | Create | Pure cmd → service-request translators: `create_resource_to_ingest_payload`, `update_resource_to_request`, `list_filter_to_params`, `search_query_to_params`, `resource_row_to_summary`, `unified_hit_to_search_hit`, `resolve_resource_ref` helper. |
| `crates/temper-api/src/backend/tests.rs` | Create | Integration tests against real `test-db` Postgres using `#[sqlx::test(migrator = "crate::MIGRATOR")]`. Happy-path + one error-path per method. Object-safety promotion for the dyn Backend smoke test. |

---

## Caller-side Notes (do not consult during execution; just for orientation)

- The trait already exists at `crates/temper-core/src/operations/backend.rs` (Phase 1) and is object-safe. Don't redefine it.
- `ResourceRef::Scoped { slug, doctype, context }` carries doctype + context but NOT owner. The resolver uses `resolve_by_uri` with `owner: "@me"` (the self-scope idiom — see `crates/temper-api/src/services/resource_service.rs::push_owner`).
- `get_by_slug` in `resource_service` does NOT filter by doctype, so `resolve_by_uri` is the correct primitive for `Scoped` lookups.
- Existing test fixtures in `crates/temper-api/tests/common/fixtures.rs` provide `SYSTEM_PROFILE_ID`, `TEMPER_CONTEXT_ID`, `RESEARCH_DOC_TYPE_ID` UUID constants and a `clean_and_seed` helper. The plan re-uses them via the `tests/common` test crate convention; if `src/backend/tests.rs` needs them, copy the constants into the test module rather than reaching across the test/src boundary.

---

## Task 1: Extend TemperError variants

**Why:** The `Backend` trait returns `Result<_, TemperError>`. Today's `temper-api` services return `Result<_, ApiError>`. There is no conversion. To bridge cleanly without information loss, `TemperError` needs the missing variants `Forbidden`, `BadRequest`, `Conflict`, `Unauthorized`. Variants are additive (no breakage). Per `feedback_no_premature_backward_compat`: project is one month old; tighten the type rather than smushing through `Api(String)`.

**Files:**
- Modify: `crates/temper-core/src/error.rs`
- Test: covered indirectly by Task 2's From-impl test.

- [ ] **Step 1: Read current TemperError**

Run: `Read crates/temper-core/src/error.rs`
Confirm the variants present and the `Result<T>` alias.

- [ ] **Step 2: Add the four new variants**

Edit `crates/temper-core/src/error.rs` — add the variants alphabetically grouped near the existing `NotFound`. Replace the `pub enum TemperError { ... }` block to include:

```rust
#[derive(Error, Debug)]
pub enum TemperError {
    #[error("Vault not found — run `temper init` or set TEMPER_VAULT")]
    VaultNotFound,

    #[error("Config error: {0}")]
    Config(String),

    #[error("Vault error: {0}")]
    Vault(String),

    #[error("Project error: {0}")]
    Project(String),

    #[error("Embedding error: {0}")]
    Embedding(String),

    #[error("Index error: {0}")]
    Index(String),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("YAML error: {0}")]
    Yaml(#[from] serde_yaml::Error),

    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("TOML parse error: {0}")]
    Toml(#[from] toml::de::Error),

    #[error("Extraction error: {0}")]
    Extraction(String),

    #[error("API error: {0}")]
    Api(String),

    #[error("network error: {0}")]
    Network(String),

    #[error("Not found: {0}")]
    NotFound(String),

    #[error("Bad request: {0}")]
    BadRequest(String),

    #[error("Conflict: {0}")]
    Conflict(String),

    #[error("Forbidden")]
    Forbidden,

    #[error("Unauthorized: {0}")]
    Unauthorized(String),

    #[error("system access required")]
    SystemAccessRequired(Box<CliAccessDetails>),
}
```

- [ ] **Step 3: Verify temper-core still compiles**

Run: `cargo build -p temper-core`
Expected: success.

If any existing match on `TemperError` becomes non-exhaustive, the compile error will name the file/line — add the new variants to those matches with the same default behavior they would have given `TemperError::Api`. (Verify each manually, do not soften error handling. If a match arm requires real semantic handling, escalate per `feedback_subagent_escalate_not_soften`.)

- [ ] **Step 4: Run cargo make check**

Run: `cargo make check`
Expected: clean.

- [ ] **Step 5: Commit**

```bash
git add crates/temper-core/src/error.rs
git commit -m "feat(core): extend TemperError with Forbidden/BadRequest/Conflict/Unauthorized variants

Phase 3a prerequisite: DbBackend's Backend trait impl returns
Result<_, TemperError>, but temper-api services return ApiError.
Adding these variants lets the From<ApiError> impl preserve
error semantics without smushing through Api(String).

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

## Task 2: Add From<ApiError> for TemperError

**Why:** The conversion is the explicit boundary between `temper-api`'s service-layer error type and the `Backend` trait's return type. It must be a typed conversion (no stringly mapping) and must lose as little detail as possible. Lives in `temper-api/src/error.rs` because that's where `ApiError` is defined; a `From<ApiError> for temper_core::TemperError` impl belongs to the producing crate per Rust convention.

**Files:**
- Modify: `crates/temper-api/src/error.rs`
- Test: inline `#[cfg(test)] mod tests` in same file.

- [ ] **Step 1: Write the failing test**

Append to `crates/temper-api/src/error.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use temper_core::error::TemperError;

    #[test]
    fn api_error_not_found_maps_to_temper_not_found() {
        let api: ApiError = ApiError::NotFound;
        let t: TemperError = api.into();
        assert!(matches!(t, TemperError::NotFound(_)));
    }

    #[test]
    fn api_error_forbidden_maps_to_temper_forbidden() {
        let t: TemperError = ApiError::Forbidden.into();
        assert!(matches!(t, TemperError::Forbidden));
    }

    #[test]
    fn api_error_bad_request_carries_message() {
        let t: TemperError = ApiError::BadRequest("missing field".into()).into();
        match t {
            TemperError::BadRequest(s) => assert_eq!(s, "missing field"),
            other => panic!("expected BadRequest, got {other:?}"),
        }
    }

    #[test]
    fn api_error_conflict_carries_message() {
        let t: TemperError = ApiError::Conflict("duplicate".into()).into();
        match t {
            TemperError::Conflict(s) => assert_eq!(s, "duplicate"),
            other => panic!("expected Conflict, got {other:?}"),
        }
    }

    #[test]
    fn api_error_unauthorized_carries_message() {
        let t: TemperError = ApiError::Unauthorized("no token".into()).into();
        match t {
            TemperError::Unauthorized(s) => assert_eq!(s, "no token"),
            other => panic!("expected Unauthorized, got {other:?}"),
        }
    }

    #[test]
    fn api_error_internal_maps_to_temper_api() {
        let t: TemperError = ApiError::Internal("oops".into()).into();
        match t {
            TemperError::Api(s) => assert!(s.contains("oops")),
            other => panic!("expected Api(_), got {other:?}"),
        }
    }

    #[test]
    fn api_error_system_access_required_preserves_field_set() {
        use temper_core::types::access_gate::SystemAccessDetails;
        let api = ApiError::SystemAccessRequired {
            details: Box::new(SystemAccessDetails {
                email: Some("a@b.co".into()),
                display_name: Some("A".into()),
                access_mode: "join_request".into(),
                join_request_status: None,
                request_url: Some("https://x".into()),
                cli_command: Some("temper join".into()),
            }),
        };
        let t: TemperError = api.into();
        match t {
            TemperError::SystemAccessRequired(details) => {
                assert_eq!(details.email.as_deref(), Some("a@b.co"));
                assert_eq!(details.display_name.as_deref(), Some("A"));
                assert_eq!(details.access_mode, "join_request");
                assert_eq!(details.request_url.as_deref(), Some("https://x"));
                assert_eq!(details.cli_command.as_deref(), Some("temper join"));
            }
            other => panic!("expected SystemAccessRequired, got {other:?}"),
        }
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo nextest run -p temper-api error::tests --no-fail-fast`
Expected: compile failure — `From<ApiError>` not yet impl'd.

- [ ] **Step 3: Implement the From impl**

Append to `crates/temper-api/src/error.rs` (above the `#[cfg(test)] mod tests`):

```rust
impl From<ApiError> for temper_core::error::TemperError {
    fn from(err: ApiError) -> Self {
        use temper_core::error::{CliAccessDetails, TemperError};
        match err {
            ApiError::NotFound => TemperError::NotFound("resource not found".to_string()),
            ApiError::Forbidden => TemperError::Forbidden,
            ApiError::Unauthorized(s) => TemperError::Unauthorized(s),
            ApiError::BadRequest(s) => TemperError::BadRequest(s),
            ApiError::Conflict(s) => TemperError::Conflict(s),
            ApiError::Internal(s) => TemperError::Api(format!("internal: {s}")),
            ApiError::SystemAccessRequired { details } => {
                let join_request_status = details
                    .join_request_status
                    .as_ref()
                    .map(|s| format!("{s:?}").to_lowercase());
                TemperError::SystemAccessRequired(Box::new(CliAccessDetails {
                    email: details.email,
                    display_name: details.display_name,
                    access_mode: details.access_mode,
                    join_request_status,
                    request_url: details.request_url,
                    cli_command: details.cli_command,
                }))
            }
        }
    }
}
```

Note on `join_request_status`: the API's `JoinRequestStatus` is an enum, the CLI's `CliAccessDetails` field is `Option<String>`. The conversion stringifies via `Debug` → lowercase. If this is wrong for any caller, the implementer escalates per `feedback_subagent_escalate_not_soften` — do not silently lose state.

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo nextest run -p temper-api error::tests --no-fail-fast`
Expected: 7 passed.

- [ ] **Step 5: Run the full crate test suite (regression guard per `feedback_plan_regression_guard_after_filter_test`)**

Run: `cargo nextest run -p temper-api --features test-db --no-fail-fast`
Expected: green. Watch for `error: test run failed` or `FAIL [` in the output (`feedback_nextest_summary_lies` — don't trust the per-binary summary line).

- [ ] **Step 6: Commit**

```bash
git add crates/temper-api/src/error.rs
git commit -m "feat(api): add From<ApiError> for TemperError

Phase 3a foundation: DbBackend wraps temper-api services that return
ApiError, but the Backend trait returns TemperError. This conversion
is the explicit error boundary. Preserves SystemAccessRequired details
field-for-field via CliAccessDetails.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

## Task 3: Add operations::apply_defaults_value

**Why:** ingest_service's pre-validation pipeline operates on `serde_json::Value`. Phase 2's `operations::apply_defaults` takes `&mut ManagedMeta`. Adding the Value-shaped sibling closes the parent spec's acceptance criterion ("operations is the only path applying doctype defaults in temper-api") without forcing ingest_service into a typed round-trip.

**Files:**
- Modify: `crates/temper-core/src/operations/actions.rs`
- Modify: `crates/temper-core/src/operations/mod.rs`

- [ ] **Step 1: Write the failing test**

Append to the existing `#[cfg(test)] mod tests` block in `crates/temper-core/src/operations/actions.rs`:

```rust
    #[test]
    fn apply_defaults_value_task_sets_stage_when_missing() {
        let mut meta = serde_json::json!({});
        apply_defaults_value("task", &mut meta);
        assert_eq!(meta["temper-stage"], "backlog");
    }

    #[test]
    fn apply_defaults_value_task_does_not_overwrite_existing_stage() {
        let mut meta = serde_json::json!({"temper-stage": "in-progress"});
        apply_defaults_value("task", &mut meta);
        assert_eq!(meta["temper-stage"], "in-progress");
    }

    #[test]
    fn apply_defaults_value_unknown_doctype_is_noop() {
        let mut meta = serde_json::json!({});
        apply_defaults_value("nonexistent", &mut meta);
        assert!(meta.as_object().unwrap().is_empty());
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo nextest run -p temper-core operations::actions::tests::apply_defaults_value --no-fail-fast`
Expected: compile failure — `apply_defaults_value` not in scope.

- [ ] **Step 3: Implement apply_defaults_value**

Add to `crates/temper-core/src/operations/actions.rs` (immediately after `apply_defaults`):

```rust
/// Apply managed-tier doctype defaults to a `serde_json::Value` in place.
///
/// Sibling to [`apply_defaults`] for callers that work with `Value` directly
/// (e.g. ingest_service's pre-validation pipeline). Both functions are thin
/// wrappers over the same underlying default-application — pick the variant
/// that matches your call site's natural type.
pub fn apply_defaults_value(doctype: &str, meta: &mut serde_json::Value) {
    crate::defaults::apply_managed_defaults(doctype, meta);
}
```

- [ ] **Step 4: Re-export from operations/mod.rs**

Edit `crates/temper-core/src/operations/mod.rs`. Replace the `pub use actions::{...}` line with:

```rust
pub use actions::{
    apply_defaults, apply_defaults_value, ensure_managed_identity_keys, merge_managed_meta,
    merge_open_meta, validate_create, validate_doctype, validate_slug, validate_update,
    ActionError,
};
```

- [ ] **Step 5: Run test to verify it passes**

Run: `cargo nextest run -p temper-core operations::actions::tests --no-fail-fast`
Expected: all `apply_defaults_value_*` tests pass.

- [ ] **Step 6: Run full temper-core suite (regression guard)**

Run: `cargo nextest run -p temper-core --no-fail-fast`
Expected: green.

- [ ] **Step 7: Commit**

```bash
git add crates/temper-core/src/operations/actions.rs crates/temper-core/src/operations/mod.rs
git commit -m "feat(core): add operations::apply_defaults_value sibling

Value-shaped sibling to apply_defaults for callers that operate on
serde_json::Value directly (ingest_service pre-validation pipeline).
Both wrappers around the same underlying default application.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

## Task 4: Migrate ingest_service to operations::apply_defaults_value

**Why:** Closes the parent spec's acceptance criterion. After this commit `temper-api` has no direct `temper_core::defaults::*` imports.

**Files:**
- Modify: `crates/temper-api/src/services/ingest_service.rs` (lines 10, 403, 655 as of commit `90f6634` — verify with grep before editing).

- [ ] **Step 1: Verify the call sites still match**

Run: `grep -n "apply_managed_defaults\|temper_core::defaults" crates/temper-api/src/services/ingest_service.rs`
Expected output (line numbers may have drifted; the file content is the contract):
```
10:use temper_core::defaults::{apply_managed_defaults, apply_open_defaults};
403:    apply_managed_defaults(&payload.doc_type_name, &mut managed);
655:    apply_managed_defaults(&payload.doc_type_name, &mut managed);
```

If `apply_open_defaults` is the only remaining default fn used after this task, leave its import — it's tier-2 (open meta) and not in scope here.

- [ ] **Step 2: Migrate the import**

Edit `crates/temper-api/src/services/ingest_service.rs` line 10:

```rust
use temper_core::defaults::apply_open_defaults;
```

(Drops `apply_managed_defaults` from the import since we're switching to the operations entrypoint. `apply_open_defaults` stays — open-tier defaults are a separate concern.)

- [ ] **Step 3: Migrate the two call sites**

Replace each occurrence of:
```rust
apply_managed_defaults(&payload.doc_type_name, &mut managed);
```
with:
```rust
temper_core::operations::apply_defaults_value(&payload.doc_type_name, &mut managed);
```

There are exactly two such call sites. Use `Edit` with `replace_all: true` if the surrounding lines are identical at both call sites; otherwise edit them with surrounding context, one at a time.

- [ ] **Step 4: Run cargo make check**

Run: `cargo make check`
Expected: clean.

- [ ] **Step 5: Run full temper-api test suite**

Run: `cargo nextest run -p temper-api --features test-db --no-fail-fast`
Expected: green. The migration is byte-equivalent (both functions wrap the same implementation), so no behavior change.

- [ ] **Step 6: Verify no direct defaults imports remain in temper-api**

Run: `grep -rn "temper_core::defaults" crates/temper-api/src/`
Expected: only `apply_open_defaults` references in `ingest_service.rs`. No `apply_managed_defaults`. No `apply_doc_type_defaults`.

- [ ] **Step 7: Commit**

```bash
git add crates/temper-api/src/services/ingest_service.rs
git commit -m "refactor(api): route ingest_service defaults through operations module

Migrate the two apply_managed_defaults call sites in ingest_service.rs
to use temper_core::operations::apply_defaults_value. After this commit
temper-api has no direct temper_core::defaults imports for managed-tier
defaults — the operations module is the canonical entrypoint, completing
the parent-spec acceptance criterion.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

## Task 5: Scaffold backend/ module with empty DbBackend struct

**Why:** Land the module skeleton so subsequent tasks can fill in trait methods incrementally without a single mega-commit. The struct exists, is reachable from `temper_api::backend`, and a smoke test confirms it compiles.

**Files:**
- Create: `crates/temper-api/src/backend/mod.rs`
- Create: `crates/temper-api/src/backend/db_backend.rs`
- Modify: `crates/temper-api/src/lib.rs`

- [ ] **Step 1: Confirm temper-api lib.rs current shape**

Run: `grep -n "^pub mod\|^mod " crates/temper-api/src/lib.rs`
Note the existing module declarations so the new `backend` module slots in alphabetically.

- [ ] **Step 2: Create the backend module root**

Write `crates/temper-api/src/backend/mod.rs`:

```rust
//! `DbBackend` — Postgres-backed impl of [`temper_core::operations::Backend`].
//!
//! Per-request construction: handlers (3b) and MCP tools (3c) build a
//! `DbBackend` from their auth context and dispatch one command through it.
//! Each trait method is a thin translator over an existing service function;
//! events are synthesized post-hoc on success.
//!
//! See `docs/superpowers/specs/2026-05-07-wave1-phase3-dbbackend-design.md`.

mod db_backend;
mod translators;

#[cfg(test)]
mod tests;

pub use db_backend::DbBackend;
```

- [ ] **Step 3: Create the empty DbBackend struct**

Write `crates/temper-api/src/backend/db_backend.rs`:

```rust
//! `DbBackend` struct + `impl Backend`. Per-request construction.

use sqlx::PgPool;

use temper_core::operations::Surface;
use temper_core::types::ids::ProfileId;

/// Postgres-backed backend impl. Constructed per inbound request.
///
/// Carries the request-scoped auth context (`profile_id`, `device_id`) and the
/// originating `Surface` so each command can be threaded into the existing
/// service-layer functions and so emitted events can be tagged appropriately.
pub struct DbBackend {
    pool: PgPool,
    profile_id: ProfileId,
    device_id: String,
    /// Origin of the inbound command. Stored for forward-compat (Phase 6
    /// telemetry/event tagging); not used by Phase 3a's coarse events.
    #[allow(dead_code)]
    surface: Surface,
}

impl DbBackend {
    pub fn new(pool: PgPool, profile_id: ProfileId, device_id: String, surface: Surface) -> Self {
        Self {
            pool,
            profile_id,
            device_id,
            surface,
        }
    }

    pub(crate) fn pool(&self) -> &PgPool {
        &self.pool
    }

    pub(crate) fn profile_id(&self) -> ProfileId {
        self.profile_id
    }

    pub(crate) fn device_id(&self) -> &str {
        &self.device_id
    }
}
```

- [ ] **Step 4: Create the empty translators module**

Write `crates/temper-api/src/backend/translators.rs`:

```rust
//! Pure cmd → service-request translators.
//!
//! Each function is total (no I/O) and infallible at the type level; runtime
//! validation is the caller's responsibility (it lives in the operations
//! module's pure actions).
//!
//! Translators are added incrementally as their consumers come online.
```

(Empty for now. Subsequent tasks add functions one at a time.)

- [ ] **Step 5: Create the empty tests module placeholder**

Write `crates/temper-api/src/backend/tests.rs`:

```rust
//! Trait-impl integration tests for `DbBackend`.
//!
//! Each test uses `#[sqlx::test(migrator = "crate::MIGRATOR")]` for an
//! isolated per-test database. Happy path + one error path per trait method.
//! Object-safety is verified by promoting Phase 1's smoke test to a real
//! `Box::new(DbBackend) as Box<dyn Backend>`.

#![cfg(test)]
```

- [ ] **Step 6: Wire backend/ into temper-api/src/lib.rs**

Edit `crates/temper-api/src/lib.rs`. Add `pub mod backend;` alphabetically among the existing top-level module declarations (likely between `auth` and `config` or similar — match the existing alphabetical convention).

- [ ] **Step 7: Run cargo make check**

Run: `cargo make check`
Expected: clean. No clippy warnings (the `#[allow(dead_code)]` on `surface` is intentional and time-bound; remove it in Phase 6 when surfaces start tagging events).

- [ ] **Step 8: Commit**

```bash
git add crates/temper-api/src/backend/ crates/temper-api/src/lib.rs
git commit -m "feat(api): scaffold backend/ module with empty DbBackend

Phase 3a foundation: DbBackend struct + translators module + tests
placeholder. No Backend trait impl yet. Per-request construction with
(pool, profile_id, device_id, surface). Subsequent tasks fill in one
trait method at a time.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

## Task 6: Implement DbBackend::create_resource

**Why:** First trait method — the most involved one (delegates to the rich `ingest_service::ingest`). Lands the translator pattern and the post-hoc event-emission pattern that the next five methods reuse.

**Files:**
- Modify: `crates/temper-api/src/backend/db_backend.rs`
- Modify: `crates/temper-api/src/backend/translators.rs`
- Modify: `crates/temper-api/src/backend/tests.rs`

- [ ] **Step 1: Write the failing test**

Replace `crates/temper-api/src/backend/tests.rs` with:

```rust
//! Trait-impl integration tests for `DbBackend`.
//!
//! Each test uses `#[sqlx::test(migrator = "crate::MIGRATOR")]` for an
//! isolated per-test database.

#![cfg(test)]

use sqlx::PgPool;
use uuid::Uuid;

use temper_core::operations::{
    Backend, BodyUpdate, CreateResource, DomainEvent, Surface,
};
use temper_core::types::ids::ProfileId;
use temper_core::types::managed_meta::ManagedMeta;

use crate::backend::DbBackend;

// Well-known UUIDs from the R2 seed migration. Mirrors the constants in
// `crates/temper-api/tests/common/fixtures.rs`; copied here because src/
// can't depend on the integration-test crate's helpers.
const SYSTEM_PROFILE_ID: &str = "00000000-0000-0000-0004-000000000001";
const TEMPER_CONTEXT_NAME: &str = "temper";

fn system_profile() -> ProfileId {
    ProfileId(Uuid::parse_str(SYSTEM_PROFILE_ID).unwrap())
}

fn make_backend(pool: PgPool) -> DbBackend {
    DbBackend::new(pool, system_profile(), "test".to_string(), Surface::ApiHttp)
}

#[sqlx::test(migrator = "crate::MIGRATOR")]
async fn create_resource_inserts_row_and_emits_event(pool: PgPool) {
    let backend = make_backend(pool);
    let cmd = CreateResource {
        slug: "create-test-1".to_string(),
        doctype: "task".to_string(),
        context: TEMPER_CONTEXT_NAME.to_string(),
        title: "Create test 1".to_string(),
        body: Some(BodyUpdate::new("# body")),
        managed_meta: ManagedMeta::default(),
        open_meta: None,
        origin: Surface::ApiHttp,
    };

    let out = backend.create_resource(cmd).await.expect("create succeeds");

    assert_eq!(out.value.slug.as_deref(), Some("create-test-1"));
    assert_eq!(out.value.title, "Create test 1");
    assert_eq!(out.events.len(), 1);
    match &out.events[0] {
        DomainEvent::DbResourceCreated { resource_id } => {
            assert_eq!(*resource_id, out.value.id);
        }
        other => panic!("expected DbResourceCreated, got {other:?}"),
    }
}

#[sqlx::test(migrator = "crate::MIGRATOR")]
async fn create_resource_unknown_doctype_returns_temper_error(pool: PgPool) {
    let backend = make_backend(pool);
    let cmd = CreateResource {
        slug: "create-test-bad".to_string(),
        doctype: "widget".to_string(),
        context: TEMPER_CONTEXT_NAME.to_string(),
        title: "Bad doctype".to_string(),
        body: None,
        managed_meta: ManagedMeta::default(),
        open_meta: None,
        origin: Surface::ApiHttp,
    };

    let err = backend.create_resource(cmd).await.unwrap_err();
    // Whatever specific TemperError variant ingest_service returns for an
    // unknown doctype (likely BadRequest after the From<ApiError> conversion).
    // Asserting it's an error of any non-Internal kind is the contract.
    use temper_core::error::TemperError;
    assert!(
        !matches!(err, TemperError::Api(_)),
        "expected typed variant for unknown doctype, got generic Api: {err:?}"
    );
}
```

(If `ResourceRow.slug` is `String` rather than `Option<String>` in the current code, drop the `.as_deref()` — verify with `grep "pub struct ResourceRow" crates/temper-core/src/types/resource.rs`.)

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo nextest run -p temper-api --features test-db backend::tests --no-fail-fast`
Expected: compile failure — `Backend` not yet impl'd for `DbBackend`.

- [ ] **Step 3: Add the translator**

Append to `crates/temper-api/src/backend/translators.rs`:

```rust
use temper_core::operations::CreateResource;
use temper_core::types::ingest::IngestPayload;

/// Translate `CreateResource` → `IngestPayload` for `ingest_service::ingest`.
///
/// `content_hash` and `chunks_packed` are left `None` so the server runs the
/// shared pipeline (when the `ingest-pipeline` feature is enabled). `metadata`
/// is the legacy unstructured field — left absent for new commands.
pub(crate) fn create_resource_to_ingest_payload(cmd: CreateResource) -> IngestPayload {
    let body = cmd
        .body
        .map(|b| b.content)
        .unwrap_or_default();

    IngestPayload {
        title: cmd.title,
        origin_uri: String::new(),
        context_name: cmd.context,
        doc_type_name: cmd.doctype,
        content_hash: None,
        slug: cmd.slug,
        content: body,
        metadata: None,
        managed_meta: Some(serde_json::to_value(&cmd.managed_meta).unwrap_or_default()),
        open_meta: cmd.open_meta,
        chunks_packed: None,
    }
}
```

- [ ] **Step 4: Implement DbBackend::create_resource**

Replace the contents of `crates/temper-api/src/backend/db_backend.rs` with:

```rust
//! `DbBackend` struct + `impl Backend`. Per-request construction.

use async_trait::async_trait;
use sqlx::PgPool;

use temper_core::error::TemperError;
use temper_core::operations::{
    Backend, CommandOutput, CreateResource, DeleteResource, DomainEvent, ListResources,
    ResourceSummary, SearchHit, SearchResources, ShowResource, Surface, UpdateResource,
};
use temper_core::types::ids::ProfileId;
use temper_core::types::resource::ResourceRow;

use crate::services::ingest_service;

use super::translators::create_resource_to_ingest_payload;

/// Postgres-backed backend impl. Constructed per inbound request.
pub struct DbBackend {
    pool: PgPool,
    profile_id: ProfileId,
    device_id: String,
    #[allow(dead_code)]
    surface: Surface,
}

impl DbBackend {
    pub fn new(pool: PgPool, profile_id: ProfileId, device_id: String, surface: Surface) -> Self {
        Self {
            pool,
            profile_id,
            device_id,
            surface,
        }
    }

    pub(crate) fn pool(&self) -> &PgPool {
        &self.pool
    }

    pub(crate) fn profile_id(&self) -> ProfileId {
        self.profile_id
    }

    pub(crate) fn device_id(&self) -> &str {
        &self.device_id
    }
}

#[async_trait]
impl Backend for DbBackend {
    async fn create_resource(
        &self,
        cmd: CreateResource,
    ) -> Result<CommandOutput<ResourceRow>, TemperError> {
        let payload = create_resource_to_ingest_payload(cmd);
        let row = ingest_service::ingest(self.pool(), self.profile_id(), self.device_id(), payload)
            .await
            .map_err(TemperError::from)?;
        let event = DomainEvent::DbResourceCreated {
            resource_id: row.id,
        };
        Ok(CommandOutput::with_events(row, vec![event]))
    }

    async fn show_resource(
        &self,
        _cmd: ShowResource,
    ) -> Result<CommandOutput<ResourceRow>, TemperError> {
        Err(TemperError::Api("show_resource not yet implemented".to_string()))
    }

    async fn update_resource(
        &self,
        _cmd: UpdateResource,
    ) -> Result<CommandOutput<ResourceRow>, TemperError> {
        Err(TemperError::Api("update_resource not yet implemented".to_string()))
    }

    async fn delete_resource(
        &self,
        _cmd: DeleteResource,
    ) -> Result<CommandOutput<()>, TemperError> {
        Err(TemperError::Api("delete_resource not yet implemented".to_string()))
    }

    async fn list_resources(
        &self,
        _cmd: ListResources,
    ) -> Result<CommandOutput<Vec<ResourceSummary>>, TemperError> {
        Err(TemperError::Api("list_resources not yet implemented".to_string()))
    }

    async fn search_resources(
        &self,
        _cmd: SearchResources,
    ) -> Result<CommandOutput<Vec<SearchHit>>, TemperError> {
        Err(TemperError::Api("search_resources not yet implemented".to_string()))
    }
}
```

The five not-yet-implemented stubs intentionally return `TemperError::Api`. They are the explicit "this method will be filled in by Task N" markers — they fail loudly rather than panicking, and each subsequent task replaces one stub with a real impl. This is NOT a "for now" workaround per `feedback_no_ship_for_now_workarounds`: every stub has a named replacing task in this same plan, and the final verification task (Task 12) fails if any stub remains.

- [ ] **Step 5: Run test to verify create succeeds and unknown-doctype errors**

Run: `cargo nextest run -p temper-api --features test-db backend::tests::create_resource --no-fail-fast`
Expected: 2 passed (`create_resource_inserts_row_and_emits_event`, `create_resource_unknown_doctype_returns_temper_error`).

- [ ] **Step 6: Run full temper-api suite**

Run: `cargo nextest run -p temper-api --features test-db --no-fail-fast`
Expected: green. The new tests pass; existing tests unaffected (no production code path changed yet).

- [ ] **Step 7: Run cargo make check**

Run: `cargo make check`
Expected: clean.

- [ ] **Step 8: Commit**

```bash
git add crates/temper-api/src/backend/
git commit -m "feat(api): impl DbBackend::create_resource

First trait method — wraps ingest_service::ingest with a CreateResource
→ IngestPayload translator and synthesizes DbResourceCreated post-hoc.
Other five methods stub TemperError::Api 'not yet implemented' and are
replaced one-by-one by subsequent tasks in this plan.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

## Task 7: Implement DbBackend::show_resource

**Why:** First read method. Lands the `ResourceRef` resolution helper that `update_resource` and `delete_resource` reuse.

**Files:**
- Modify: `crates/temper-api/src/backend/translators.rs`
- Modify: `crates/temper-api/src/backend/db_backend.rs`
- Modify: `crates/temper-api/src/backend/tests.rs`

- [ ] **Step 1: Write the failing test**

Append to `crates/temper-api/src/backend/tests.rs`:

```rust
use temper_core::operations::{ResourceRef, ShowResource};
use temper_core::types::ids::ResourceId;

#[sqlx::test(migrator = "crate::MIGRATOR")]
async fn show_resource_by_uuid_returns_row(pool: PgPool) {
    let backend = make_backend(pool);

    // Seed via create_resource so we have a real row to look up.
    let created = backend
        .create_resource(CreateResource {
            slug: "show-by-uuid".to_string(),
            doctype: "task".to_string(),
            context: TEMPER_CONTEXT_NAME.to_string(),
            title: "Show by uuid".to_string(),
            body: None,
            managed_meta: ManagedMeta::default(),
            open_meta: None,
            origin: Surface::ApiHttp,
        })
        .await
        .unwrap();

    let cmd = ShowResource {
        resource: ResourceRef::Uuid {
            id: ResourceId(created.value.id),
        },
        origin: Surface::ApiHttp,
    };
    let out = backend.show_resource(cmd).await.expect("show succeeds");
    assert_eq!(out.value.id, created.value.id);
    assert!(out.events.is_empty(), "read methods emit no events");
}

#[sqlx::test(migrator = "crate::MIGRATOR")]
async fn show_resource_by_scoped_slug_returns_row(pool: PgPool) {
    let backend = make_backend(pool);

    backend
        .create_resource(CreateResource {
            slug: "show-by-slug".to_string(),
            doctype: "task".to_string(),
            context: TEMPER_CONTEXT_NAME.to_string(),
            title: "Show by slug".to_string(),
            body: None,
            managed_meta: ManagedMeta::default(),
            open_meta: None,
            origin: Surface::ApiHttp,
        })
        .await
        .unwrap();

    let cmd = ShowResource {
        resource: ResourceRef::scoped("show-by-slug", "task", TEMPER_CONTEXT_NAME),
        origin: Surface::ApiHttp,
    };
    let out = backend.show_resource(cmd).await.expect("show succeeds");
    assert_eq!(out.value.slug.as_deref(), Some("show-by-slug"));
}

#[sqlx::test(migrator = "crate::MIGRATOR")]
async fn show_resource_missing_uuid_returns_not_found(pool: PgPool) {
    let backend = make_backend(pool);
    let cmd = ShowResource {
        resource: ResourceRef::Uuid {
            id: ResourceId(Uuid::new_v4()),
        },
        origin: Surface::ApiHttp,
    };
    let err = backend.show_resource(cmd).await.unwrap_err();
    use temper_core::error::TemperError;
    assert!(matches!(err, TemperError::NotFound(_)), "got {err:?}");
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo nextest run -p temper-api --features test-db backend::tests::show_resource --no-fail-fast`
Expected: 3 tests fail (the stub `TemperError::Api("show_resource not yet implemented")`).

- [ ] **Step 3: Implement the show_resource method**

In `crates/temper-api/src/backend/db_backend.rs`:

a) Replace the `use crate::services::ingest_service;` line with:

```rust
use crate::services::{ingest_service, resource_service};
```

b) Add to imports (in the `use temper_core::operations::*` line):

```rust
use temper_core::operations::{
    Backend, CommandOutput, CreateResource, DeleteResource, DomainEvent, ListResources,
    ResourceRef, ResourceSummary, SearchHit, SearchResources, ShowResource, Surface,
    UpdateResource,
};
```

c) Replace the `show_resource` stub with:

```rust
    async fn show_resource(
        &self,
        cmd: ShowResource,
    ) -> Result<CommandOutput<ResourceRow>, TemperError> {
        let row = match cmd.resource {
            ResourceRef::Uuid { id } => {
                resource_service::get_visible(self.pool(), *self.profile_id(), *id)
                    .await
                    .map_err(TemperError::from)?
            }
            ResourceRef::Scoped {
                slug,
                doctype,
                context,
            } => {
                let params = crate::services::resource_service::ResolveByUriParams {
                    owner: "@me".to_string(),
                    context,
                    doc_type: doctype,
                    ident: slug,
                };
                resource_service::resolve_by_uri(self.pool(), *self.profile_id(), &params)
                    .await
                    .map_err(TemperError::from)?
            }
        };
        Ok(CommandOutput::new(row))
    }
```

Note: `*self.profile_id()` deref pattern works iff `ProfileId` derives `Deref<Target = Uuid>`. Verify with `grep "pub struct ProfileId" crates/temper-core/src/types/ids.rs`. If it doesn't deref-to-Uuid, the call becomes `self.profile_id().0` (tuple-struct field access) — pick whichever the existing service callers use.

d) `ResolveByUriParams` is a public struct in `crates/temper-api/src/services/resource_service.rs` — import it via the path used in step (c). If it isn't `pub`, the implementer either (i) makes it `pub` and notes it in the commit message, or (ii) escalates per `feedback_subagent_escalate_not_soften`.

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo nextest run -p temper-api --features test-db backend::tests::show_resource --no-fail-fast`
Expected: 3 passed.

- [ ] **Step 5: Run full temper-api suite (regression guard)**

Run: `cargo nextest run -p temper-api --features test-db --no-fail-fast`
Expected: green.

- [ ] **Step 6: Commit**

```bash
git add crates/temper-api/src/backend/db_backend.rs crates/temper-api/src/backend/tests.rs
git commit -m "feat(api): impl DbBackend::show_resource

Branches on ResourceRef:
  - Uuid     → resource_service::get_visible
  - Scoped   → resource_service::resolve_by_uri (owner=\"@me\")

Read methods emit no events per Phase 3a's coarse-events contract.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

## Task 8: Implement DbBackend::update_resource

**Why:** Second write method. Reuses the `ResourceRef` resolution from Task 7 — extracted into a private helper to keep `update`/`delete` DRY.

**Files:**
- Modify: `crates/temper-api/src/backend/translators.rs`
- Modify: `crates/temper-api/src/backend/db_backend.rs`
- Modify: `crates/temper-api/src/backend/tests.rs`

- [ ] **Step 1: Write the failing test**

Append to `crates/temper-api/src/backend/tests.rs`:

```rust
use temper_core::operations::UpdateResource;

#[sqlx::test(migrator = "crate::MIGRATOR")]
async fn update_resource_changes_title_and_emits_event(pool: PgPool) {
    let backend = make_backend(pool);

    let created = backend
        .create_resource(CreateResource {
            slug: "update-test".to_string(),
            doctype: "task".to_string(),
            context: TEMPER_CONTEXT_NAME.to_string(),
            title: "Original title".to_string(),
            body: None,
            managed_meta: ManagedMeta::default(),
            open_meta: None,
            origin: Surface::ApiHttp,
        })
        .await
        .unwrap();

    let cmd = UpdateResource {
        resource: ResourceRef::Uuid {
            id: ResourceId(created.value.id),
        },
        body: None,
        managed_meta: Some(ManagedMeta {
            title: Some("New title".to_string()),
            ..ManagedMeta::default()
        }),
        open_meta: None,
        origin: Surface::ApiHttp,
    };
    let out = backend.update_resource(cmd).await.expect("update succeeds");

    assert_eq!(out.value.id, created.value.id);
    assert_eq!(out.value.title, "New title");
    match &out.events[..] {
        [DomainEvent::DbResourceUpdated { resource_id }] => {
            assert_eq!(*resource_id, created.value.id);
        }
        other => panic!("expected single DbResourceUpdated event, got {other:?}"),
    }
}

#[sqlx::test(migrator = "crate::MIGRATOR")]
async fn update_resource_unknown_uuid_returns_not_found(pool: PgPool) {
    let backend = make_backend(pool);
    let cmd = UpdateResource {
        resource: ResourceRef::Uuid {
            id: ResourceId(Uuid::new_v4()),
        },
        body: None,
        managed_meta: None,
        open_meta: None,
        origin: Surface::ApiHttp,
    };
    let err = backend.update_resource(cmd).await.unwrap_err();
    use temper_core::error::TemperError;
    assert!(
        matches!(err, TemperError::NotFound(_) | TemperError::Forbidden),
        "got {err:?}"
    );
    // resource_service::update returns Forbidden when can_modify_resource()
    // is false, which is what an unknown id produces. Either is acceptable.
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo nextest run -p temper-api --features test-db backend::tests::update_resource --no-fail-fast`
Expected: 2 failures.

- [ ] **Step 3: Add the update translator**

Append to `crates/temper-api/src/backend/translators.rs`:

```rust
use temper_core::operations::UpdateResource;
use temper_core::types::resource::ResourceUpdateRequest;

/// Translate `UpdateResource` → `ResourceUpdateRequest` for
/// `resource_service::update`. The body trio is all-or-nothing in the service
/// layer; the operations command surfaces it via `body: Option<BodyUpdate>`,
/// so when a body is present we recompute its hash and pack chunks here.
///
/// 3a-only behavior: if `body` is `Some`, the translator leaves
/// `content_hash` and `chunks_packed` as `None`. This forces the service to
/// reject the update with `BadRequest` because today's `resource_service::update`
/// requires the trio to be all-Some-or-all-None and the handler-layer guard
/// asserts that. **The 3b handler migration must take over hash/chunk
/// computation before passing through DbBackend** — until then, body-bearing
/// UpdateResource commands cannot be fulfilled. This is acceptable for 3a
/// because no caller dispatches through DbBackend yet (it's dark-launched).
pub(crate) fn update_resource_to_request(cmd: UpdateResource) -> ResourceUpdateRequest {
    let (title, slug) = cmd
        .managed_meta
        .as_ref()
        .map(|m| (m.title.clone(), m.slug.clone()))
        .unwrap_or((None, None));

    ResourceUpdateRequest {
        title,
        slug,
        managed_meta: cmd.managed_meta,
        open_meta: cmd.open_meta,
        content: cmd.body.as_ref().map(|b| b.content.clone()),
        content_hash: None,
        chunks_packed: None,
    }
}
```

- [ ] **Step 4: Add ResourceRef → resource_id resolution helper**

Append to `crates/temper-api/src/backend/translators.rs`:

```rust
use sqlx::PgPool;
use temper_core::error::TemperError;
use temper_core::operations::ResourceRef;
use temper_core::types::ids::{ProfileId, ResourceId};

use crate::services::resource_service;

/// Resolve a `ResourceRef` to a concrete `ResourceId`.
///
/// `Uuid` short-circuits without I/O; `Scoped` queries via `resolve_by_uri`
/// with `owner="@me"` (the self-scope idiom — see `push_owner` in
/// `resource_service.rs`).
pub(crate) async fn resolve_resource_ref(
    pool: &PgPool,
    profile_id: ProfileId,
    rref: ResourceRef,
) -> Result<ResourceId, TemperError> {
    match rref {
        ResourceRef::Uuid { id } => Ok(id),
        ResourceRef::Scoped {
            slug,
            doctype,
            context,
        } => {
            let params = resource_service::ResolveByUriParams {
                owner: "@me".to_string(),
                context,
                doc_type: doctype,
                ident: slug,
            };
            let row = resource_service::resolve_by_uri(pool, *profile_id, &params)
                .await
                .map_err(TemperError::from)?;
            Ok(ResourceId(row.id))
        }
    }
}
```

(The `*profile_id` deref usage assumes `ProfileId: Deref<Target = Uuid>`. Confirm via `grep`; switch to `.0` field access if not.)

- [ ] **Step 5: Implement update_resource**

Replace the `update_resource` stub in `crates/temper-api/src/backend/db_backend.rs` with:

```rust
    async fn update_resource(
        &self,
        cmd: UpdateResource,
    ) -> Result<CommandOutput<ResourceRow>, TemperError> {
        let resource_id =
            super::translators::resolve_resource_ref(self.pool(), self.profile_id(), cmd.resource.clone())
                .await?;
        let req = super::translators::update_resource_to_request(cmd);
        let row = resource_service::update(
            self.pool(),
            *self.profile_id(),
            *resource_id,
            self.device_id(),
            req,
        )
        .await
        .map_err(TemperError::from)?;
        let event = DomainEvent::DbResourceUpdated {
            resource_id: row.id,
        };
        Ok(CommandOutput::with_events(row, vec![event]))
    }
```

(`UpdateResource` doesn't currently impl `Clone`. Either (i) derive `Clone` on it in `crates/temper-core/src/operations/commands.rs` — additive, no risk — or (ii) destructure the cmd before resolution. Option (i) is the cleaner shape and lets future tasks share commands across resolution + translation. If choosing (i), include the derive in this commit.)

- [ ] **Step 6: Run test to verify it passes**

Run: `cargo nextest run -p temper-api --features test-db backend::tests::update_resource --no-fail-fast`
Expected: 2 passed.

- [ ] **Step 7: Run full temper-api suite**

Run: `cargo nextest run -p temper-api --features test-db --no-fail-fast`
Expected: green.

- [ ] **Step 8: Commit**

```bash
git add crates/temper-api/src/backend/ crates/temper-core/src/operations/commands.rs
git commit -m "feat(api): impl DbBackend::update_resource

Resolves ResourceRef via resolve_by_uri (Scoped) or short-circuits
(Uuid), translates UpdateResource → ResourceUpdateRequest, calls
resource_service::update, emits DbResourceUpdated.

Adds Clone derive to operations::UpdateResource so the command can
be reused across resolution + translation steps.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

## Task 9: Implement DbBackend::delete_resource

**Why:** Reuses `resolve_resource_ref` from Task 8.

**Files:**
- Modify: `crates/temper-api/src/backend/db_backend.rs`
- Modify: `crates/temper-api/src/backend/tests.rs`

- [ ] **Step 1: Write the failing test**

Append to `crates/temper-api/src/backend/tests.rs`:

```rust
use temper_core::operations::DeleteResource;

#[sqlx::test(migrator = "crate::MIGRATOR")]
async fn delete_resource_soft_deletes_and_emits_event(pool: PgPool) {
    let backend = make_backend(pool);

    let created = backend
        .create_resource(CreateResource {
            slug: "delete-test".to_string(),
            doctype: "task".to_string(),
            context: TEMPER_CONTEXT_NAME.to_string(),
            title: "Delete test".to_string(),
            body: None,
            managed_meta: ManagedMeta::default(),
            open_meta: None,
            origin: Surface::ApiHttp,
        })
        .await
        .unwrap();

    let cmd = DeleteResource {
        resource: ResourceRef::Uuid {
            id: ResourceId(created.value.id),
        },
        force: false,
        origin: Surface::ApiHttp,
    };
    let out = backend.delete_resource(cmd).await.expect("delete succeeds");

    match &out.events[..] {
        [DomainEvent::DbResourceSoftDeleted { resource_id }] => {
            assert_eq!(*resource_id, created.value.id);
        }
        other => panic!("expected single DbResourceSoftDeleted event, got {other:?}"),
    }

    // Confirm the row is no longer visible.
    let show_err = backend
        .show_resource(ShowResource {
            resource: ResourceRef::Uuid {
                id: ResourceId(created.value.id),
            },
            origin: Surface::ApiHttp,
        })
        .await
        .unwrap_err();
    use temper_core::error::TemperError;
    assert!(matches!(show_err, TemperError::NotFound(_)));
}

#[sqlx::test(migrator = "crate::MIGRATOR")]
async fn delete_resource_unknown_uuid_returns_error(pool: PgPool) {
    let backend = make_backend(pool);
    let cmd = DeleteResource {
        resource: ResourceRef::Uuid {
            id: ResourceId(Uuid::new_v4()),
        },
        force: false,
        origin: Surface::ApiHttp,
    };
    let err = backend.delete_resource(cmd).await.unwrap_err();
    use temper_core::error::TemperError;
    assert!(
        matches!(err, TemperError::NotFound(_) | TemperError::Forbidden),
        "got {err:?}"
    );
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo nextest run -p temper-api --features test-db backend::tests::delete_resource --no-fail-fast`
Expected: 2 failures.

- [ ] **Step 3: Implement delete_resource**

Replace the `delete_resource` stub in `crates/temper-api/src/backend/db_backend.rs` with:

```rust
    async fn delete_resource(
        &self,
        cmd: DeleteResource,
    ) -> Result<CommandOutput<()>, TemperError> {
        let resource_id =
            super::translators::resolve_resource_ref(self.pool(), self.profile_id(), cmd.resource)
                .await?;
        resource_service::delete(self.pool(), self.profile_id(), resource_id, self.device_id())
            .await
            .map_err(TemperError::from)?;
        let event = DomainEvent::DbResourceSoftDeleted { resource_id };
        Ok(CommandOutput::with_events((), vec![event]))
    }
```

The `cmd.force` field is not consumed — it's a CLI-side TTY-confirmation concern that `VaultBackend` (Phase 4) will use. Per spec §Components, `DbBackend` ignores it. No `#[allow(unused)]` needed; the destructured `cmd` is fully consumed.

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo nextest run -p temper-api --features test-db backend::tests::delete_resource --no-fail-fast`
Expected: 2 passed.

- [ ] **Step 5: Run full temper-api suite**

Run: `cargo nextest run -p temper-api --features test-db --no-fail-fast`
Expected: green.

- [ ] **Step 6: Commit**

```bash
git add crates/temper-api/src/backend/
git commit -m "feat(api): impl DbBackend::delete_resource

Resolves ResourceRef → resource_id, calls resource_service::delete
(soft-delete), emits DbResourceSoftDeleted. The cmd.force field is
a CLI-side concern and not consumed by DbBackend.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

## Task 10: Implement DbBackend::list_resources

**Files:**
- Modify: `crates/temper-api/src/backend/translators.rs`
- Modify: `crates/temper-api/src/backend/db_backend.rs`
- Modify: `crates/temper-api/src/backend/tests.rs`

- [ ] **Step 1: Write the failing test**

Append to `crates/temper-api/src/backend/tests.rs`:

```rust
use temper_core::operations::{ListFilter, ListResources};

#[sqlx::test(migrator = "crate::MIGRATOR")]
async fn list_resources_returns_summaries(pool: PgPool) {
    let backend = make_backend(pool);

    for n in 1..=3 {
        backend
            .create_resource(CreateResource {
                slug: format!("list-test-{n}"),
                doctype: "task".to_string(),
                context: TEMPER_CONTEXT_NAME.to_string(),
                title: format!("List test {n}"),
                body: None,
                managed_meta: ManagedMeta::default(),
                open_meta: None,
                origin: Surface::ApiHttp,
            })
            .await
            .unwrap();
    }

    let cmd = ListResources {
        filter: ListFilter {
            doctype: Some("task".to_string()),
            context: Some(TEMPER_CONTEXT_NAME.to_string()),
            stage: None,
            goal: None,
            limit: Some(10),
        },
        origin: Surface::ApiHttp,
    };
    let out = backend.list_resources(cmd).await.expect("list succeeds");

    let slugs: Vec<&str> = out
        .value
        .iter()
        .map(|s| s.slug.as_str())
        .collect();
    assert!(slugs.contains(&"list-test-1"), "slugs: {slugs:?}");
    assert!(slugs.contains(&"list-test-2"), "slugs: {slugs:?}");
    assert!(slugs.contains(&"list-test-3"), "slugs: {slugs:?}");
    assert!(out.events.is_empty(), "list emits no events");
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo nextest run -p temper-api --features test-db backend::tests::list_resources --no-fail-fast`
Expected: 1 failure.

- [ ] **Step 3: Add the list translator + summary mapper**

Append to `crates/temper-api/src/backend/translators.rs`:

```rust
use temper_core::operations::{ListFilter, ResourceSummary};
use temper_core::types::resource::{ResourceListParams, ResourceRow};

/// Translate `ListFilter` → `ResourceListParams`.
///
/// Only the filters represented in both shapes are forwarded. `stage` and
/// `goal` are not first-class params on `ResourceListParams` today and would
/// require a `q`-string extension or a service-layer change — captured in
/// the spec's "Open Questions" as a follow-up; for 3a they're ignored.
pub(crate) fn list_filter_to_params(filter: ListFilter) -> ResourceListParams {
    ResourceListParams {
        kb_context_id: None,
        kb_doc_type_id: None,
        context_name: filter.context,
        doc_type_name: filter.doctype,
        owner: Some("@me".to_string()),
        q: None,
        sort: None,
        order: None,
        limit: filter.limit.map(|n| n as i64),
        offset: None,
    }
}

/// Project a `ResourceRow` into the trait's `ResourceSummary`.
pub(crate) fn resource_row_to_summary(row: &ResourceRow) -> ResourceSummary {
    ResourceSummary {
        slug: row.slug.clone().unwrap_or_default(),
        doctype: row.doc_type_name.clone(),
        context: row.context_name.clone(),
        title: row.title.clone(),
    }
}
```

(If `ResourceRow.slug` is `String` rather than `Option<String>`, replace `.clone().unwrap_or_default()` with `.clone()`. Verify with `grep "pub struct ResourceRow" crates/temper-core/src/types/resource.rs`.)

- [ ] **Step 4: Implement list_resources**

Replace the `list_resources` stub in `crates/temper-api/src/backend/db_backend.rs` with:

```rust
    async fn list_resources(
        &self,
        cmd: ListResources,
    ) -> Result<CommandOutput<Vec<ResourceSummary>>, TemperError> {
        let params = super::translators::list_filter_to_params(cmd.filter);
        let response = resource_service::list_visible(self.pool(), *self.profile_id(), params)
            .await
            .map_err(TemperError::from)?;
        let summaries: Vec<ResourceSummary> = response
            .rows
            .iter()
            .map(super::translators::resource_row_to_summary)
            .collect();
        Ok(CommandOutput::new(summaries))
    }
```

- [ ] **Step 5: Run test to verify it passes**

Run: `cargo nextest run -p temper-api --features test-db backend::tests::list_resources --no-fail-fast`
Expected: 1 passed.

- [ ] **Step 6: Run full temper-api suite**

Run: `cargo nextest run -p temper-api --features test-db --no-fail-fast`
Expected: green.

- [ ] **Step 7: Commit**

```bash
git add crates/temper-api/src/backend/
git commit -m "feat(api): impl DbBackend::list_resources

Translates ListFilter → ResourceListParams (with owner=\"@me\"),
calls resource_service::list_visible, projects ResourceRow rows
into ResourceSummary. Stage/goal filters are not yet routed —
captured as a Phase 3a open question.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

## Task 11: Implement DbBackend::search_resources

**Files:**
- Modify: `crates/temper-api/src/backend/translators.rs`
- Modify: `crates/temper-api/src/backend/db_backend.rs`
- Modify: `crates/temper-api/src/backend/tests.rs`

- [ ] **Step 1: Verify SearchParams + UnifiedSearchResultRow shapes**

Run: `grep -A12 "pub struct SearchParams\b" crates/temper-core/src/types/api.rs`
Run: `grep -B1 -A10 "pub struct UnifiedSearchResultRow" crates/temper-api/src/services/search_service.rs crates/temper-core/src/types/`
Note the exact field set so the translator and `unified_hit_to_search_hit` mapper are correct.

- [ ] **Step 2: Write the failing test**

Append to `crates/temper-api/src/backend/tests.rs`:

```rust
use temper_core::operations::{SearchQuery, SearchResources};

#[sqlx::test(migrator = "crate::MIGRATOR")]
async fn search_resources_returns_hits_or_empty(pool: PgPool) {
    let backend = make_backend(pool);

    backend
        .create_resource(CreateResource {
            slug: "search-test".to_string(),
            doctype: "task".to_string(),
            context: TEMPER_CONTEXT_NAME.to_string(),
            title: "Searchable thing".to_string(),
            body: Some(BodyUpdate::new("Body about rust ownership")),
            managed_meta: ManagedMeta::default(),
            open_meta: None,
            origin: Surface::ApiHttp,
        })
        .await
        .unwrap();

    let cmd = SearchResources {
        query: SearchQuery {
            query: "rust".to_string(),
            doctype: None,
            context: Some(TEMPER_CONTEXT_NAME.to_string()),
            limit: Some(5),
        },
        origin: Surface::ApiHttp,
    };
    let out = backend.search_resources(cmd).await.expect("search succeeds");

    // The full-text search may or may not match without an embedding; the
    // contract this test enforces is that the call succeeds and returns a
    // well-shaped Vec<SearchHit>. Match on length only — actual hit content
    // is search-implementation-detail.
    assert!(out.events.is_empty(), "search emits no events");
    let _ = out.value.len();
}
```

- [ ] **Step 3: Run test to verify it fails**

Run: `cargo nextest run -p temper-api --features test-db backend::tests::search_resources --no-fail-fast`
Expected: 1 failure.

- [ ] **Step 4: Add the search translator + hit mapper**

Append to `crates/temper-api/src/backend/translators.rs`. Use the actual `SearchParams` field set verified in Step 1 (the field list below is the shape of SearchParams as of commit `90f6634`; update if Step 1's grep returns a different shape):

```rust
use temper_core::operations::{SearchHit, SearchQuery};
use temper_core::types::api::SearchParams;

pub(crate) fn search_query_to_params(q: SearchQuery) -> SearchParams {
    SearchParams {
        embedding: None,
        query: Some(q.query),
        search_config: "english".to_string(),
        context_name: q.context,
        doc_type: q.doctype,
        limit: q.limit.map(|n| n as i64),
        // Other SearchParams fields default to None / false; verify the
        // exact field list and add `..Default::default()` if SearchParams
        // implements Default. Otherwise enumerate explicitly.
        ..Default::default()
    }
}

/// Project a search-service row into the trait's `SearchHit`.
///
/// `UnifiedSearchResultRow` is defined in `temper-core/src/types/api.rs`.
/// Field set as of commit `90f6634`: `resource_id`, `title`, `slug: String`
/// (not Option), `kb_uri`, `origin_uri`, `context: Option<String>`,
/// `doc_type: String` (not doc_type_name), `fts_score`, `vector_score`,
/// `combined_score: f32`, `origin: String`. The summary's `context` falls
/// back to empty when absent.
pub(crate) fn unified_hit_to_search_hit(
    row: &temper_core::types::api::UnifiedSearchResultRow,
) -> SearchHit {
    SearchHit {
        summary: ResourceSummary {
            slug: row.slug.clone(),
            doctype: row.doc_type.clone(),
            context: row.context.clone().unwrap_or_default(),
            title: row.title.clone(),
        },
        score: row.combined_score,
    }
}
```

- [ ] **Step 5: Implement search_resources**

Replace the `search_resources` stub in `crates/temper-api/src/backend/db_backend.rs` with:

```rust
    async fn search_resources(
        &self,
        cmd: SearchResources,
    ) -> Result<CommandOutput<Vec<SearchHit>>, TemperError> {
        let params = super::translators::search_query_to_params(cmd.query);
        let rows = crate::services::search_service::search(self.pool(), *self.profile_id(), params)
            .await
            .map_err(TemperError::from)?;
        let hits: Vec<SearchHit> = rows
            .iter()
            .map(super::translators::unified_hit_to_search_hit)
            .collect();
        Ok(CommandOutput::new(hits))
    }
```

- [ ] **Step 6: Run test to verify it passes**

Run: `cargo nextest run -p temper-api --features test-db backend::tests::search_resources --no-fail-fast`
Expected: 1 passed.

- [ ] **Step 7: Run full temper-api suite**

Run: `cargo nextest run -p temper-api --features test-db --no-fail-fast`
Expected: green.

- [ ] **Step 8: Commit**

```bash
git add crates/temper-api/src/backend/
git commit -m "feat(api): impl DbBackend::search_resources

Translates SearchQuery → SearchParams, calls search_service::search,
projects UnifiedSearchResultRow → SearchHit. Closes the six-method
Backend trait impl for DbBackend.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

## Task 12: Promote Phase 1's object-safety smoke test

**Why:** Phase 1's `assert_object_safe(_: &dyn Backend)` is a static assertion. With `DbBackend` now real, we can promote it to a runtime test that actually constructs a `Box<dyn Backend>` from a `DbBackend` and calls a method through dynamic dispatch. Confirms object-safety end-to-end.

**Files:**
- Modify: `crates/temper-api/src/backend/tests.rs`

- [ ] **Step 1: Write the test**

Append to `crates/temper-api/src/backend/tests.rs`:

```rust
#[sqlx::test(migrator = "crate::MIGRATOR")]
async fn db_backend_dispatches_via_dyn_backend(pool: PgPool) {
    let concrete = make_backend(pool);
    let boxed: Box<dyn Backend> = Box::new(concrete);

    let cmd = ListResources {
        filter: ListFilter::default(),
        origin: Surface::ApiHttp,
    };
    let out = boxed.list_resources(cmd).await.expect("dyn dispatch ok");
    let _ = out.value;
}
```

- [ ] **Step 2: Run test to verify it passes (no failing-test step needed for compile-only confirmation)**

Run: `cargo nextest run -p temper-api --features test-db backend::tests::db_backend_dispatches_via_dyn_backend --no-fail-fast`
Expected: passed. (If it fails to compile, it indicates a non-object-safe trait method — escalate, do not soften.)

- [ ] **Step 3: Commit**

```bash
git add crates/temper-api/src/backend/tests.rs
git commit -m "test(api): promote Backend object-safety assertion to runtime check

DbBackend now exists, so we can box it as dyn Backend and dispatch
list_resources through dynamic dispatch. Confirms the trait stays
object-safe end-to-end.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

## Task 13: Final verification + create cleanup backlog tasks

**Why:** Phase 3a is dark-launched — no production code path uses `DbBackend` yet. The acceptance gate is "all suites green; the trait foundation works against a real DB." We also create the explicit backlog tasks the spec promised so the divergent-sibling cleanup (`resource_service::create`, `ingest_service::update`) doesn't get forgotten. Per `feedback_no_premature_backward_compat`.

**Files:**
- Vault tasks (via `temper resource create`).

- [ ] **Step 1: Run all relevant test suites**

```bash
cargo make check
cargo nextest run --workspace --no-fail-fast
cargo nextest run -p temper-api --features test-db --no-fail-fast
cargo nextest run --manifest-path tests/e2e/Cargo.toml --features test-db,test-embed --no-fail-fast
```

Expected: every command exits 0. Watch for `error: test run failed` or `FAIL [` — don't trust nextest's per-binary `Summary` line (`feedback_nextest_summary_lies`).

If e2e with `test-embed` is not runnable in the local environment (ONNX runtime not installed), record that as a known limitation and rely on CI's Embed job — but `test-db` alone must pass locally.

- [ ] **Step 2: Verify no `temper_core::defaults::*` direct imports remain in temper-api**

Run: `grep -rn "temper_core::defaults" crates/temper-api/src/`
Expected: only `apply_open_defaults` references in `ingest_service.rs`. Anything else is a regression to investigate.

- [ ] **Step 3: Investigate ingest_service::update redundancy (spec Open Question)**

Run: `grep -rn "ingest_service::update\b" crates/`
Document the finding in the cleanup backlog task created in Step 5b. Likely candidates: sync-pull machinery, MCP tools. If the function turns out to be unreachable from any caller, that's an immediate `cargo machete`-style cleanup; capture it.

- [ ] **Step 4: Verify temper-cli has no new dep on temper-api**

Run: `grep -A2 "name = \"temper-cli\"" crates/temper-cli/Cargo.toml`
Run: `cargo metadata --format-version 1 -q | jq -r '.packages[] | select(.name=="temper-cli") | .dependencies[].name' | sort`
Expected: no `temper-api` in the list. The dep-graph constraint is preserved.

- [ ] **Step 5a: Create the resource_service::create deprecation backlog task**

```bash
cat <<'EOF' | temper resource create --type task --title "Deprecate resource_service::create after Phase 3b" --context temper --mode build --effort small
# Deprecate resource_service::create

After Phase 3b lands, the `POST /api/resources` HTTP handler dispatches through
DbBackend.create_resource (which routes to ingest_service::ingest). Once that
migration is verified by the existing crates/temper-api/tests/resources_*.rs
integration tests, delete `resource_service::create`.

## Why

Two divergent create paths today:
- `ingest_service::ingest` — full pipeline (defaults, validation, dedupe,
  chunks, edges); used by `POST /api/ingest`, CLI cloud writes, MCP create.
- `resource_service::create` — thin SQL insert; used only by
  `POST /api/resources` and ~6 integration tests.

After Phase 3b unifies the create dispatch through DbBackend, the second path
becomes dead code. Per `feedback_no_premature_backward_compat`: project is one
month old; remove rather than keep.

## Acceptance

- `crates/temper-api/src/services/resource_service.rs` no longer defines
  `pub async fn create`.
- All resources_*.rs integration tests pass against the unified path.
- `cargo machete` (or equivalent) confirms no callers remain.
EOF
```

- [ ] **Step 5b: Create the ingest_service::update redundancy backlog task**

```bash
cat <<'EOF' | temper resource create --type task --title "Investigate ingest_service::update redundancy" --context temper --mode plan --effort small
# Investigate ingest_service::update redundancy

Phase 3a's plan flagged that there are two update paths today:
- `resource_service::update` — partial-merge (PATCH /api/resources/:id
  handler; what DbBackend.update_resource wraps).
- `ingest_service::update` — re-ingest path that takes IngestPayload.

Find every caller of `ingest_service::update`, document its purpose, and
decide whether it should be:
1. Collapsed into resource_service::update (preferred if the use cases are
   actually equivalent), or
2. Surfaced as a distinct operations command (e.g., `ReingestResource`)
   on the Backend trait if it has a real semantic difference.

## Why

The DbBackend.update_resource path is now canonical. `ingest_service::update`
diverging from it is a drift risk per the schema-driven managed-meta and
shared-execution-paths reframes.

## Acceptance

A short research note in `docs/superpowers/specs/` documenting the call graph
and recommending (1) or (2). If (1), follow-up cleanup task created.
EOF
```

- [ ] **Step 6: Mark this Phase 3a task done**

```bash
temper resource update 2026-05-03-wave-1-phase-3-write-the-dbbackend-implementation-plan --type task --stage done
```

(The originating task — "write the DbBackend implementation plan" — is complete once this plan is reviewed by the user. Marking it done is the *plan-writer's* terminal step, not the implementer's; if the implementer is a different session, leave this for the user.)

- [ ] **Step 7: Final commit (if any backlog-task files were vault-tracked)**

If the two `temper resource create` calls produced new files in the vault and the vault is tracked in git (it isn't in this repo, but verify):

```bash
git status
```

If there are no untracked vault files, this step is a no-op. The implementation plan is complete.

---

## Self-Review Notes (plan-writer ran this; implementer can skim)

**Spec coverage check:**

| Spec acceptance criterion | Plan task |
|---|---|
| `crates/temper-api/src/backend/` exists with `DbBackend` impl of `Backend` covering all 6 methods | Tasks 5–11 |
| `temper_core::operations::apply_defaults_value` exists; ingest_service migrated; no direct `temper_core::defaults::*` calls | Tasks 3, 4, plus verification in Task 13 Step 2 |
| Trait-impl unit tests in `backend/tests.rs` cover happy + error path per method against test-db | Tasks 6–11 each include both, plus Task 12 for object-safety |
| Existing test suites pass | Task 13 Step 1 |
| `temper-cli` has no new dep on `temper-api` | Task 13 Step 4 |
| Backlog tasks for deprecate-create + investigate-update | Task 13 Steps 5a / 5b |
| HTTP handler migration | **Out of scope for 3a — Phase 3b** |
| MCP tool migration | **Out of scope for 3a — Phase 3c** |
| CLAUDE.md update | **Deferred to 3c or follow-on doc commit** |

**Placeholder scan:** No "TBD" / "TODO" / vague "add error handling" steps. The not-yet-implemented stubs in Task 6 are explicit, time-bound markers replaced by named subsequent tasks.

**Type consistency check:** `ResourceSummary { slug, doctype, context, title }` (Phase 1 backend.rs) used consistently in Tasks 10, 11. `SearchHit { summary, score }` likewise. `DomainEvent` variants `DbResourceCreated/Updated/SoftDeleted` match Phase 1 events.rs. `ResourceRef::Uuid { id }` and `ResourceRef::Scoped { slug, doctype, context }` match Phase 1 resource_ref.rs (note: the variant uses `id` field name, not `resource_id` — Phase 1 had a `#[serde(rename = "resource_id")]` for wire compat).

**Reality-check verifications performed during plan writing:**
- ingest_service.rs:403 and :655 grep-confirmed.
- ingest_service::ingest signature — confirmed `(pool, ProfileId, &str, IngestPayload) -> ApiResult<ResourceRow>`.
- resource_service::resolve_by_uri exists and filters by owner+context+doctype+slug — confirmed.
- get_by_slug does NOT filter by doctype — flagged in Task 7 design.
- ApiError ↔ TemperError have no conversion — Task 2 adds From.
- TemperError missing variants Forbidden/BadRequest/Conflict/Unauthorized — Task 1 adds.
- `#[sqlx::test(migrator = "...")]` is the existing test pattern — Task 6+ uses it.
- `MIGRATOR` is exposed at `temper_api::MIGRATOR` (existing tests reference it) — Tasks use `crate::MIGRATOR` from src/.

**Reality-checks resolved during plan writing (no implementer guesswork required):**
- `ProfileId` and `ResourceId` both impl `Deref<Target = Uuid>` (via the `define_id!` macro in `crates/temper-core/src/types/ids.rs`). The plan's `*self.profile_id()` and `*resource_id` deref patterns are correct.
- `ResourceRow.slug` is `Option<String>` — the plan's `.clone().unwrap_or_default()` and `.as_deref()` patterns reflect this.
- `SearchParams` impls `Default` — the plan's `..Default::default()` works.
- `UnifiedSearchResultRow` lives at `temper_core::types::api::UnifiedSearchResultRow` (NOT in services::search_service::). Field set as cited in Task 11.
- `UpdateResource` does NOT currently derive `Clone`. Task 8 Step 5 adds the derive — additive, no risk.
