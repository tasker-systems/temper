# Wave 1 Phase 3c — MCP Tool Migration Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Migrate every **write-path** MCP resource tool in `crates/temper-mcp/src/tools/resources.rs` (`create_resource`, `update_resource`, `update_resource_meta`, `delete_resource`) to dispatch through `DbBackend` (the trait impl landed in 3a). Read-only MCP tools (`get_resource`, `list_resources`) and the search MCP tool stay service-direct, mirroring 3b's A5 deviation (wire-shape mismatch between MCP's rich return types and the Backend trait's lossy projections). After 3c, write-path MCP tools no longer call `temper_api::services::*` directly — they construct a `DbBackend` from `(state.pool, profile_id, "mcp", Surface::Mcp)` and dispatch one `temper-core::operations::*Resource` command per inbound tool call. The `update_resource_meta` MCP tool collapses into the unified `DbBackend::update_resource` dispatch (the translator's meta-only branch built in 3b). After both 3b and 3c land, the divergent service siblings (`resource_service::create`, `ingest_service::update`, `meta_service::update_meta`) are deleted.

**Architecture (locked, do not re-debate):**
1. MCP `update_resource_meta` folds into `DbBackend::update_resource` — no 7th trait method. The translator (built in 3b) branches on cmd shape: body trio | meta-only | title/slug. The MCP tool's typed `MetaUpdatePayload` (typed `ManagedMeta` + `Value` open_meta + caller-supplied hashes) translates into an `UpdateResource` cmd carrying just `managed_meta` and `open_meta`.
2. `ensure_managed_identity_keys` send-side wiring is preserved as Phase 5 symmetric defense. The two existing call sites at `tools/resources.rs:264-268` (create_resource) and `tools/resources.rs:509-513` (update_resource — the user-quoted 508-513 has shifted by 1 line in the verified file) stay where they are. They run BEFORE the translator builds the cmd. Do not remove or relocate them.
3. Read-only MCP tools (`get_resource`, `list_resources`) migrate through `DbBackend::show_resource` / `DbBackend::list_resources` for consistency with 3b's HTTP read-only handler treatment.
4. The 3 retirement targets (`resource_service::create`, `ingest_service::update`, `meta_service::update_meta`) land at end of branch as their own delete tasks — each preceded by a grep confirming zero callers.

**Tech Stack:** Rust 2024, async-trait (already in DbBackend), tokio, sqlx with compile-time-checked queries (unchanged by this phase — 3c moves dispatch, not SQL), rmcp for MCP transport, schemars for tool param schemas.

**Spec:** `docs/superpowers/specs/2026-05-07-wave1-phase3-dbbackend-design.md`
**Predecessor (3a, merged):** `docs/superpowers/plans/2026-05-07-wave1-phase3a-dbbackend-foundation.md`
**Predecessor (3b, same branch — MUST land before 3c starts):** `docs/superpowers/plans/2026-05-07-wave1-phase3b-http-handler-migration.md`

**Branch:** `jct/wave1-phase3bc-handler-mcp-migration` (same branch as 3b — both phases ship as one PR or two stacked PRs from the same branch).

**Backlog task:** task `#3` ("Write 3c plan (MCP tool migration)") and task `#4` ("Execute 3b + 3c via subagent-driven development").

---

## Why

After 3a, `DbBackend` is dark-launched: trait impl exists, services unchanged, no surface dispatches through it. After 3b, every Axum handler for resources/ingest/search/meta dispatches through `DbBackend`; the translator covers all three update shapes (body trio | meta-only | title/slug). 3c brings the second surface — MCP tools — onto the same dispatch path so both surfaces share one canonical write path. This unifies:

- **Create dispatch** — MCP `create_resource` and HTTP `POST /api/ingest` both go through `DbBackend::create_resource → ingest_service::ingest`.
- **Update dispatch** — MCP `update_resource` (formerly two-phase: title/slug via `resource_service::update`, content via `ingest_service::update`) collapses to one `DbBackend::update_resource` call. MCP `update_resource_meta` (formerly `meta_service::update_meta`) becomes the meta-only branch of the same dispatch.
- **Delete dispatch** — MCP `delete_resource` and HTTP `DELETE /api/resources/:id` both go through `DbBackend::delete_resource → resource_service::delete`.
- **Delete + write paths** are unified across surfaces; **read paths stay service-direct** on both HTTP (3b A5) and MCP (this plan, after deviation).

Once 3c lands, every resource **mutation** flows through `temper-core::operations::*Resource` commands, every command flows through the `Backend` trait, every backend call emits the same coarse events. Read-paths remain a service-direct passthrough on both surfaces (the unification value is on writes; reads are lossless passthroughs that re-coupling the trait to surface shapes would not improve). Phase 4 (VaultBackend) and Phase 5 (Surface dispatch unification) inherit this exact shape — write-unified, read-direct.

---

## Architecture (locked decisions, restated)

| Decision | Rationale |
|---|---|
| `update_resource_meta` folds into `DbBackend::update_resource` (no 7th trait method) | The trait surface stays at 6 methods; the meta-only update is a degenerate `UpdateResource` with `body=None` and `managed_meta=Some(...)` + `open_meta=Some(...)` — the translator's meta-only branch handles it. Adds zero trait surface. |
| `ensure_managed_identity_keys` stays in MCP tools (send-side) | Preserves Phase 5's symmetric defense pattern. The MCP tool is the send side; `meta_service::update_meta` (and after retirement, `resource_service::update`) is the receive side. Both fill canonical identity keys from a typed source. Removing the send-side call would weaken the symmetric defense. |
| Read-only MCP tools (`get_resource`, `list_resources`, `search`) stay service-direct | Mirrors 3b's A5 deviation: today these tools return rich shapes (`EnrichedResource`, `UnifiedSearchResultRow`); DbBackend's trait projects to lossy types (`ResourceSummary`, `SearchHit`). Routing through the trait would either narrow the contract (visible to MCP test fixtures and downstream agent prompt examples) or grow the trait with surface-shaped types (re-couples it). The unification value is on writes (defaults, validation, dedupe, pipeline); reads are passthroughs. Out-of-scope cleanup if ever needed. |
| Service retirements happen in 3c (not 3b) | 3b removes HTTP handler callers but tests + MCP tools are still callers. 3c is the last surface migration; once it lands, all in-tree callers are gone and the deletes are safe. Each delete is a separate task with grep-verified zero-caller proof immediately before. |
| Branch shared with 3b | 3b builds the unified translator; 3c consumes it. Stacking on one branch lets 3c verify against 3b's translator changes without a separate merge. |

---

## Verified APIs (grep-confirmed at plan-write time)

**MCP tools in scope** (`crates/temper-mcp/src/tools/resources.rs` and `tools/search.rs`):

| Tool | File:line (verified) | Today's service call(s) | After 3c: `DbBackend` method |
|---|---|---|---|
| `create_resource` | `resources.rs:226` | `ingest_service::ingest(pool, profile_id, "mcp", payload)` (line 286) | `DbBackend::create_resource(cmd)` with `Surface::Mcp` |
| ~~`get_resource`~~ | ~~`resources.rs:312`~~ | (stays service-direct — wire-shape mismatch; mirrors 3b A5) | **NOT MIGRATED** |
| ~~`list_resources`~~ | ~~`resources.rs:382`~~ | (stays service-direct — wire-shape mismatch; mirrors 3b A5) | **NOT MIGRATED** |
| `update_resource` | `resources.rs:440` | TWO calls: `resource_service::update` (line 483) for title/slug, `ingest_service::update` (line 533) for content | ONE call: `DbBackend::update_resource(cmd)` — translator routes body to body-trio path and title/slug to title/slug path. Two-phase split disappears. |
| `update_resource_meta` | `resources.rs:559` | `meta_service::update_meta(pool, profile_id, resource_id, "mcp", payload)` (line 581) | `DbBackend::update_resource(cmd)` — cmd carries only `managed_meta` + `open_meta`; translator routes to meta-only branch |
| `delete_resource` | `resources.rs:609` | `resource_service::delete(pool, profile_id, resource_id, "mcp")` (line 615) | `DbBackend::delete_resource(cmd)` |
| ~~`search`~~ | ~~`search.rs:9`~~ | (stays service-direct — wire-shape mismatch; mirrors 3b A5) | **NOT MIGRATED** |

**MCP tools NOT in scope** (read-only, distinct shapes — verified each does not touch resources):

| Tool file | Verified service call(s) | Why excluded |
|---|---|---|
| `tools/contexts.rs` | `context_service::list_visible/get_visible/create` | Operates on contexts (kb_contexts table), not resources. No `Backend` trait method covers context CRUD; out of Wave 1 Phase 3 scope. |
| `tools/doc_types.rs` | `doc_type_service::list_all` | Operates on doc types, not resources. Out of scope. |
| `tools/profiles.rs` | (no service::; reads via `svc.require_profile()`) | Profile read-only via auth context; out of scope. |
| `tools/events.rs` | `event_service::list_visible` | Operates on events, not resources. Out of scope. |

**`ensure_managed_identity_keys` call sites that MUST be preserved** (verified by re-reading the file before this plan was written):

| Call site | Verified file:line | Stays as-is in 3c |
|---|---|---|
| Inside `create_resource`, before building the cmd | `crates/temper-mcp/src/tools/resources.rs:264-268` | Yes — runs on `managed_meta_value` (a `serde_json::Value`) before the cmd is built. The translator consumes the cmd's typed `ManagedMeta`, but the JSONB-shape canonical keys must already be set on the wire — that's what this call ensures. |
| Inside `update_resource`, when content is present | `crates/temper-mcp/src/tools/resources.rs:509-513` (user prompt said 508-513; verified shifted by 1) | Yes — same reason. Symmetric defense on the send side. |

The `update_resource` tool also sets `ManagedMeta.title` / `.slug` directly when title/slug args are present (lines 472-476). That typed-shape mirroring continues to work in 3c — the translator picks them up from `cmd.managed_meta`.

**Where MCP tools get their inputs** (verified):
- `profile_id`: `let profile = svc.require_profile().await?;` then `ProfileId::from(profile.id)` — used pervasively in `tools/resources.rs`.
- Pool: `&svc.api_state.pool` — `TemperMcpService` has `pub api_state: AppState` (verified `crates/temper-mcp/src/service.rs:33-34`); `AppState.pool: PgPool` (verified `crates/temper-api/src/state.rs:152`).
- `device_id`: literal string `"mcp"` (used today in service calls — see lines 286, 483, 533, 581, 619).
- `Surface::Mcp` for cmd `origin` field and for `DbBackend::new(...)`.

**Service signatures used by translators (built in 3b, consumed in 3c) — verified:**
- `ingest_service::ingest(&PgPool, ProfileId, &str, IngestPayload) -> ApiResult<ResourceRow>` — line 384 of `ingest_service.rs`. Wrapped by 3a's `DbBackend::create_resource` already.
- `resource_service::update(&PgPool, Uuid, Uuid, &str, ResourceUpdateRequest) -> ApiResult<ResourceRow>` — line 527 of `resource_service.rs`. Wrapped by 3a's `DbBackend::update_resource` (3b extends translator to handle body trio + meta-only branches).
- `meta_service::update_meta(&PgPool, ProfileId, ResourceId, &str, MetaUpdatePayload) -> ApiResult<Value>` — line 87 of `meta_service.rs`. **3b's translator must subsume the meta-only path.** After 3c migrates `update_resource_meta`, this fn has no remaining callers and gets deleted.

**Tests that must keep passing unmodified** (the regression guard):
- All MCP integration tests in `tests/e2e/tests/mcp_*.rs` — specifically `mcp_round_trip_test.rs`, `mcp_ingest_test.rs`, `mcp_resource_parity_test.rs`. Verified these tests directly call `ingest_service::create_resource_with_manifest` and `ingest_service::update` as **test fixtures** (not as the SUT) — those direct calls stay (they seed test rows). The MCP tool dispatch path is the SUT, and that's what 3c rewires.
- `crates/temper-api/tests/meta_reconcile_test.rs` directly calls `meta_service::update_meta` as a fixture (lines 72, 114). **These calls must be migrated to use the new dispatch path before deleting `meta_service::update_meta`** (Task 9b). Otherwise the delete will break the test crate.
- `cargo make check`, `cargo make test-db`, `cargo make test-e2e`, `cargo nextest --manifest-path tests/e2e/Cargo.toml --features test-db,test-embed`.

---

## Inherited Project Guidance (verbatim — embed in EVERY implementer subagent prompt)

The following clauses are mandatory in every subagent prompt for this phase. Copy them verbatim:

- **`feedback_subagent_check_before_commit`** — Run `cargo make check` before claiming work complete. The pre-commit hook is a backstop, not the first line.
- **`feedback_subagent_escalate_not_soften`** — If passing a test requires loosening an error path or a contract, STOP and report BLOCKED. Do not weaken assertions. Do not "ignore for now".
- **`feedback_no_premature_backward_compat`** — This project is one month old. Don't keep dead code "for compat". Remove rather than retain.
- **`feedback_no_ship_for_now_workarounds`** — No "TODO" stubs. No "ship it for now and clean up later" comments. If a problem can't be solved cleanly, escalate; don't ship a workaround.
- **`feedback_plan_regression_guard_after_filter_test`** — Pair every filter-by-name test run with a full crate suite run before commit. A passing single test does not imply a passing crate.
- **`feedback_nextest_summary_lies`** — Don't trust nextest's per-binary `Summary` line with `--no-fail-fast`. Trust the exit code or grep for `error: test run failed` / `FAIL [` in the output.
- **`feedback_pre_propose_arch_review`** + **`feedback_plan_verification`** — Verify named APIs against the current code at dispatch time. The plan is a hypothesis; the code is ground truth. Re-grep before each task; line numbers may have shifted between when this plan was written and when you execute.
- **`project_workspace_feature_unification_ort`** — MCP tool tests run under the embed-gated CI job. When 3c touches push-body / ingest-pipeline code paths (Tasks 2 and 5 in particular), run with `--features test-db,test-embed` locally to match CI's Embed job before claiming complete: `cargo nextest --manifest-path tests/e2e/Cargo.toml --features test-db,test-embed --no-fail-fast`. The Embed job is the only one with ONNX Runtime installed and is the one that catches workspace-feature-unification surprises.

---

## TDD Tasks

Each task is independently committable. RED → GREEN → REFACTOR. Subagents executing this plan: do NOT batch tasks. One task per subagent dispatch unless the subagent explicitly verifies it can hold all task constraints in mind. Re-grep the verified file:line citations before editing — they may have shifted.

### Task 1: Identify the MCP test harness and confirm the regression baseline

**Why:** Before rewiring any tool, confirm which tests exercise the MCP dispatch path end-to-end so the regression guard is set. If the existing harness already drives every tool through real MCP transport (rmcp), no new fixtures are needed — the existing tests are the contract.

**Files:** read-only.

- [ ] **Step 1: Inventory existing MCP integration tests**

```bash
ls tests/e2e/tests/mcp_*.rs
grep -l "create_resource\|update_resource\|delete_resource\|update_resource_meta\|list_resources\|get_resource" tests/e2e/tests/mcp_*.rs
```

Expected: at minimum `mcp_round_trip_test.rs`, `mcp_ingest_test.rs`, `mcp_resource_parity_test.rs`. Confirm each exercises one or more of the in-scope tools via `crate::tools::resources::*` or via the rmcp transport surface.

- [ ] **Step 2: Distinguish SUT calls from fixture calls**

Many of these tests call `ingest_service::create_resource_with_manifest` and `ingest_service::update` directly. Verify which calls are **fixtures** (seed test data) vs. **SUT** (the path under test). Fixture calls stay unchanged in 3c — they're outside the dispatch path being rewired. Re-running the inventory below should confirm the SUT calls are the MCP tool functions in `crates/temper-mcp/src/tools/`:

```bash
grep -n "tools::resources::\|tools::search::\|crate::tools::" tests/e2e/tests/mcp_*.rs
```

- [ ] **Step 3: Capture baseline (run before any code changes)**

```bash
cargo nextest run --manifest-path tests/e2e/Cargo.toml --features test-db --no-fail-fast
cargo nextest run --manifest-path tests/e2e/Cargo.toml --features test-db,test-embed --no-fail-fast
```

Expected: green at the head of the 3b-finished branch. Watch for `error: test run failed` / `FAIL [` per `feedback_nextest_summary_lies`. Record the test count for each binary so the post-3c run can be compared.

If the embed-gated run cannot run locally (no ONNX runtime), record that as a known limitation and rely on CI's Embed job. The non-embed run MUST pass locally before proceeding.

- [ ] **Step 4: No commit (this is a verification task)**

This task produces no diff. Its output is the implementer's confidence that the regression baseline is captured. Subsequent tasks compare against this baseline.

---

### Task 2: Migrate `tools::resources::create_resource` to DbBackend

**Why:** First MCP tool migration. Lands the construction pattern (`DbBackend::new(state.pool.clone(), ProfileId::from(profile.id), "mcp".to_string(), Surface::Mcp)`) and the cmd-build pattern that the next six tools reuse. Preserves `ensure_managed_identity_keys` send-side wiring at lines 264-268.

**Files:**
- Modify: `crates/temper-mcp/src/tools/resources.rs` (function at line 226)

⚠️ Plan/reality gap: line numbers may have shifted between plan-write and execution. Re-grep `pub async fn create_resource` in `crates/temper-mcp/src/tools/resources.rs` before editing.

- [ ] **Step 1: Re-verify the file shape**

```bash
grep -n "pub async fn create_resource\|ensure_managed_identity_keys\|ingest_service::ingest" crates/temper-mcp/src/tools/resources.rs
```

Confirm:
- `pub async fn create_resource` exists.
- `temper_core::operations::ensure_managed_identity_keys(...)` is called BEFORE the `IngestPayload` is built (lines 264-268 in the as-written file; may have shifted).
- `ingest_service::ingest(pool, profile_id, "mcp", payload)` is the dispatch (line 286 in the as-written file).

- [ ] **Step 2: Confirm regression baseline for create_resource**

```bash
cargo nextest run --manifest-path tests/e2e/Cargo.toml --features test-db -E 'test(mcp) and test(create)' --no-fail-fast
```

Record the passing tests as the contract.

- [ ] **Step 3: Rewire to DbBackend dispatch**

Edit `crates/temper-mcp/src/tools/resources.rs` `create_resource`:

a) **Keep** the auth + slug/origin_uri build + `ensure_managed_identity_keys` call exactly as-is (lines 230-268). These are pre-cmd preparation and are **not** part of the dispatch.

b) **Replace** the `IngestPayload { ... }` block + `ingest_service::ingest(...)` call with:

```rust
use temper_api::backend::DbBackend;
use temper_core::operations::{Backend, BodyUpdate, CreateResource, Surface};

let cmd = CreateResource {
    slug,
    doctype: input.doc_type_name,
    context: input.context_name,
    title: input.title,
    body: if content.is_empty() { None } else { Some(BodyUpdate::new(content)) },
    // managed_meta on the command is typed ManagedMeta; the call to
    // ensure_managed_identity_keys above mutated the JSON form, so deserialize
    // it back into the typed shape. The extra-bucket on ManagedMeta preserves
    // any unknown keys; serde renames produce canonical temper-* keys on
    // round-trip.
    managed_meta: serde_json::from_value(managed_meta_value)
        .map_err(|e| rmcp::ErrorData::invalid_params(
            format!("invalid managed_meta: {e}"), None))?,
    open_meta: input.open_meta,
    origin: Surface::Mcp,
};

let backend = DbBackend::new(
    pool.clone(),
    profile_id,
    "mcp".to_string(),
    Surface::Mcp,
);
let out = backend.create_resource(cmd).await.map_err(|e| {
    use temper_core::error::TemperError;
    match e {
        TemperError::NotFound(_) => rmcp::ErrorData::invalid_params(
            "Context or doc_type not found. Use create_context / list_doc_types to verify."
                .to_string(),
            None,
        ),
        TemperError::BadRequest(msg) => rmcp::ErrorData::invalid_params(msg, None),
        other => rmcp::ErrorData::internal_error(
            format!("Failed to create resource: {other}"), None,
        ),
    }
})?;
let resource = out.value;
```

The error-translation block preserves the existing user-facing messages exactly (verify against the original `match e { ApiError::NotFound => ... }` block at line 288). The `TemperError` variants come from 3a's `From<ApiError> for TemperError` (Task 2 in 3a plan).

c) The `enrich_resource(...)` call at line 302 stays unchanged — it consumes the `ResourceRow` and is independent of the dispatch path.

- [ ] **Step 4: Update the `use temper_api::services::*` import**

The original imports `use temper_api::services::{context_service, doc_type_service, ingest_service, meta_service, resource_service};`. Drop `ingest_service` from the list (still needed by other tools in the same file until later tasks). The compiler will catch any leftover use; if `ingest_service::resolve_doc_type` (line 410) is the only remaining use, leave the import.

⚠️ Plan/reality gap: subsequent tasks remove more services from this import. The implementer should NOT pre-emptively remove imports — only remove an entry the moment the last in-file caller goes away. Each task ends with `cargo build` to catch unused-imports.

- [ ] **Step 5: Run targeted MCP create tests**

```bash
cargo nextest run --manifest-path tests/e2e/Cargo.toml --features test-db -E 'test(mcp) and test(create)' --no-fail-fast
```

Expected: same green count as Step 2's baseline. Behavior is byte-equivalent (DbBackend.create_resource wraps `ingest_service::ingest` — the path 3a's translator already locked in).

- [ ] **Step 6: Run full e2e suite (regression guard per `feedback_plan_regression_guard_after_filter_test`)**

```bash
cargo nextest run --manifest-path tests/e2e/Cargo.toml --features test-db --no-fail-fast
cargo nextest run --manifest-path tests/e2e/Cargo.toml --features test-db,test-embed --no-fail-fast
```

Expected: green. The embed-gated run is mandatory because create_resource exercises the ingest pipeline (chunks + embedding).

- [ ] **Step 7: Run cargo make check**

```bash
cargo make check
```

Expected: clean. Watch for clippy `unused_imports` warnings if the `services::*` import shrunk.

- [ ] **Step 8: Commit**

```bash
git add crates/temper-mcp/src/tools/resources.rs
git commit -m "refactor(mcp): dispatch create_resource through DbBackend

Replaces the direct ingest_service::ingest call with a CreateResource
command dispatched through DbBackend. Preserves the send-side
ensure_managed_identity_keys call before cmd-build (Phase 5 symmetric
defense; the receive side runs inside the service unchanged).

First MCP tool on the unified dispatch path; six more follow.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

### Task 3: Migrate `tools::resources::get_resource` to DbBackend — REMOVED (out of 3c scope)

**Why removed:** Read-only/search MCP tools stay service-direct, mirroring 3b's A5 deviation. Wire-shape mismatch (today returns rich shapes; DbBackend trait projects to lossy types) makes trivial migration either narrow the contract (visible to MCP test fixtures and downstream agent-prompt examples) or expand the trait with surface-shaped types. Reads are passthroughs; the unification value is on writes.

**Implication for downstream tasks:**
- Task 9a's grep for read-path callers in `crates/temper-mcp/` will continue to find `resource_service::get_visible`, `resource_service::get_by_slug`, `resource_service::get_content`, `resource_service::list_visible`, `search_service::search` — these are EXPECTED, not regressions.
- Task 10 verification step 2 should treat read-path service calls in MCP as in-spec.

If a future phase decides to unify reads, this can land as a follow-up.

---

### Task 4: Migrate `tools::resources::list_resources` to DbBackend — REMOVED (out of 3c scope)

**Why removed:** Read-only/search MCP tools stay service-direct, mirroring 3b's A5 deviation. Wire-shape mismatch (today returns rich shapes; DbBackend trait projects to lossy types) makes trivial migration either narrow the contract (visible to MCP test fixtures and downstream agent-prompt examples) or expand the trait with surface-shaped types. Reads are passthroughs; the unification value is on writes.

**Implication for downstream tasks:**
- Task 9a's grep for read-path callers in `crates/temper-mcp/` will continue to find `resource_service::get_visible`, `resource_service::get_by_slug`, `resource_service::get_content`, `resource_service::list_visible`, `search_service::search` — these are EXPECTED, not regressions.
- Task 10 verification step 2 should treat read-path service calls in MCP as in-spec.

If a future phase decides to unify reads, this can land as a follow-up.

---

### Task 5: Migrate `tools::resources::update_resource` to DbBackend (collapse two-phase)

**Why:** **The headline migration.** Today's `update_resource` is a two-phase split: title/slug via `resource_service::update`, content via `ingest_service::update`. After 3c, both collapse into ONE `DbBackend::update_resource(cmd)` call — 3b's translator routes body to body-trio path and title/slug to title/slug path within the same dispatch. This is the change that retires `ingest_service::update`.

Preserves `ensure_managed_identity_keys` send-side wiring at lines 509-513 (the user prompt cited 508-513; verified shifted by 1 in current file).

**Files:**
- Modify: `crates/temper-mcp/src/tools/resources.rs` (function at line 440)

⚠️ Depends on 3b's translator: 3b must have extended `update_resource_to_request` to handle the body trio (compute `content_hash` and `chunks_packed` when `body.is_some()`) and the meta-only path. If 3b's translator still leaves `content_hash: None, chunks_packed: None` when body is present (the 3a-only behavior documented in `translators.rs` lines 50-57), this task BLOCKS until 3b completes that work. Verify before starting:

```bash
grep -A20 "pub(crate) fn update_resource_to_request" crates/temper-api/src/backend/translators.rs
```

Expected: 3b's version of the translator computes `content_hash` (sha256 of the body) and `chunks_packed` (via the ingest chunking pipeline) when `cmd.body.is_some()`. If those fields are still `None`, escalate to controller — 3c cannot proceed.

- [ ] **Step 1: Re-verify the file shape and 3b state**

```bash
grep -n "pub async fn update_resource\b\|resource_service::update\|ingest_service::update\|resource_service::check_can_modify\|ensure_managed_identity_keys" crates/temper-mcp/src/tools/resources.rs
grep -A30 "pub(crate) fn update_resource_to_request" crates/temper-api/src/backend/translators.rs
```

Confirm 3b's translator handles the body trio and the meta-only path.

- [ ] **Step 2: Confirm regression baseline**

```bash
cargo nextest run --manifest-path tests/e2e/Cargo.toml --features test-db -E 'test(mcp) and test(update)' --no-fail-fast
cargo nextest run --manifest-path tests/e2e/Cargo.toml --features test-db,test-embed -E 'test(mcp) and test(update)' --no-fail-fast
```

- [ ] **Step 3: Rewire to a single DbBackend dispatch**

Edit `tools::resources::update_resource`. The function shape becomes:

```rust
pub async fn update_resource(
    svc: &TemperMcpService,
    input: UpdateResourceInput,
) -> Result<CallToolResult, rmcp::ErrorData> {
    let profile = svc.require_profile().await?;
    let pool = &svc.api_state.pool;
    let profile_id = ProfileId::from(profile.id);
    let resource_id = ResourceId::from(input.id);

    // The auth check at lines 450-461 is preserved as-is. resource_service::check_can_modify
    // runs an explicit can_modify_resource SQL check before mutation. DbBackend.update_resource
    // ALSO runs can_modify_resource via resource_service::update — so the explicit check
    // here is redundant. Remove it: the dispatch path is the single source of truth.
    //
    // ⚠️ Per feedback_subagent_escalate_not_soften: removing the pre-check is a behavior change.
    // It does NOT loosen the contract (the inner check is still mandatory at the SQL level),
    // but it does change the user-facing error mapping. Verify the test assertions don't
    // depend on the specific error mapping at lines 452-461.

    // If body content is present, fetch the existing row to derive payload context + slug.
    // The MCP tool today fetches existing because IngestPayload requires context_name +
    // doc_type_name + origin_uri. After DbBackend dispatch, the cmd is keyed by
    // ResourceRef::Uuid; the translator builds the partial-update inside resource_service::update
    // and recomputes whatever the body trio needs.
    //
    // **Still required pre-cmd:** ensure_managed_identity_keys send-side fill, because the
    // typed ManagedMeta on the cmd carries the canonical title/slug keys. We need the
    // existing row's slug to fill the canonical key when input.slug is None.

    let (payload_title, payload_slug_opt) = if input.title.is_some() || input.slug.is_some() || input.content.is_some() {
        // Fetch existing for fallbacks (title for canonical keys, slug for canonical keys).
        // This call stays; it's pre-cmd preparation, not a service-layer dispatch in the
        // sense being retired.
        let existing = resource_service::get_visible(pool, profile.id, input.id)
            .await
            .map_err(|e| rmcp::ErrorData::internal_error(
                format!("Failed to get resource: {e}"), None,
            ))?;
        let title = input.title.clone().unwrap_or(existing.title);
        let slug = input.slug.clone().or(existing.slug);
        (title, slug)
    } else {
        // Pure meta update with no title/slug change — caller provided neither.
        (String::new(), None)
    };

    // Inject canonical temper-title / temper-slug into managed_meta JSONB (Phase 5
    // symmetric defense; preserved from the today's tool, lines 509-513).
    let mut managed_meta_value = input.managed_meta.unwrap_or_else(|| serde_json::json!({}));
    if !payload_title.is_empty() {
        temper_core::operations::ensure_managed_identity_keys(
            &mut managed_meta_value,
            &payload_title,
            payload_slug_opt.as_deref(),
        );
    }

    use temper_api::backend::DbBackend;
    use temper_core::operations::{Backend, BodyUpdate, ResourceRef, Surface, UpdateResource};

    let cmd = UpdateResource {
        resource: ResourceRef::Uuid { id: resource_id },
        body: input.content.map(BodyUpdate::new),
        // Mirror title/slug into the typed managed_meta partial too — serde renames
        // produce canonical temper-title/temper-slug keys. The translator picks
        // them up from cmd.managed_meta.
        managed_meta: Some({
            let mut m: ManagedMeta = serde_json::from_value(managed_meta_value)
                .map_err(|e| rmcp::ErrorData::invalid_params(
                    format!("invalid managed_meta: {e}"), None,
                ))?;
            if input.title.is_some() { m.title = input.title.clone(); }
            if input.slug.is_some() { m.slug = input.slug.clone(); }
            m
        }),
        open_meta: input.open_meta,
        origin: Surface::Mcp,
    };

    let backend = DbBackend::new(pool.clone(), profile_id, "mcp".to_string(), Surface::Mcp);
    backend.update_resource(cmd).await.map_err(|e| {
        use temper_core::error::TemperError;
        match e {
            TemperError::Forbidden => rmcp::ErrorData::invalid_params(
                "Resource not found or not modifiable".to_string(), None,
            ),
            TemperError::NotFound(msg) => rmcp::ErrorData::invalid_params(
                format!("Resource not found: {msg}"), None,
            ),
            TemperError::BadRequest(msg) => rmcp::ErrorData::invalid_params(msg, None),
            other => rmcp::ErrorData::internal_error(
                format!("Failed to update resource: {other}"), None,
            ),
        }
    })?;

    // Return enriched current state (existing pattern, unchanged).
    let row = resource_service::get_visible(pool, profile.id, input.id)
        .await
        .map_err(|e| rmcp::ErrorData::internal_error(
            format!("Failed to get resource: {e}"), None,
        ))?;

    let enriched = enrich_resource(pool, profile_id, &row).await?;
    Ok(CallToolResult::success(vec![rmcp::model::Content::text(
        to_text(&enriched),
    )]))
}
```

Key behavior preserved:
- Auth check (was explicit `resource_service::check_can_modify`; now via inner `resource_service::update` which gates on `can_modify_resource`).
- Send-side `ensure_managed_identity_keys` (was at original lines 509-513).
- Typed-meta mirroring of title/slug (was at original lines 472-476).

Key behavior NEWLY UNIFIED:
- Title/slug update + content update are ONE dispatch instead of TWO service calls. Atomicity improves; the original two-phase split could leave the DB in an inconsistent state on partial failure.

⚠️ Plan/reality gap: the original tool only did the existing-row fetch when `content` was provided (line 494). The plan above fetches when title or slug or content is provided. This is because the typed `ManagedMeta` shape needs a title for the `ensure_managed_identity_keys` canonical-key fill. If the implementer can avoid the existing-row fetch when `content.is_none() && input.title.is_some()`, that's a perf win — but verify the canonical-key fill produces the right output without it.

- [ ] **Step 4: Run targeted MCP update tests**

```bash
cargo nextest run --manifest-path tests/e2e/Cargo.toml --features test-db -E 'test(mcp) and test(update)' --no-fail-fast
cargo nextest run --manifest-path tests/e2e/Cargo.toml --features test-db,test-embed -E 'test(mcp) and test(update)' --no-fail-fast
```

Expected: green. The embed-gated run is **mandatory** because content updates exercise the chunk + embed pipeline.

- [ ] **Step 5: Run full e2e suite**

```bash
cargo nextest run --manifest-path tests/e2e/Cargo.toml --features test-db --no-fail-fast
cargo nextest run --manifest-path tests/e2e/Cargo.toml --features test-db,test-embed --no-fail-fast
cargo make check
```

- [ ] **Step 6: Commit**

```bash
git add crates/temper-mcp/src/tools/resources.rs
git commit -m "refactor(mcp): collapse update_resource two-phase dispatch through DbBackend

The today's MCP tool made two service calls: resource_service::update
for title/slug and ingest_service::update for content. After this
commit, one DbBackend::update_resource dispatch handles both — the
3b translator routes body to the body-trio path and title/slug to
the title/slug path within the same call.

Atomicity improves: the original split could leave the DB inconsistent
on partial failure. Now the path is single-transactional.

Preserves the send-side ensure_managed_identity_keys call (Phase 5
symmetric defense). Removes the redundant pre-flight check_can_modify
call — the inner resource_service::update already gates on
can_modify_resource at the SQL level.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

### Task 6: Migrate `tools::resources::update_resource_meta` to DbBackend

**Why:** Folds the meta-only update path into the unified `DbBackend::update_resource` dispatch. The MCP tool's typed `MetaUpdatePayload` (typed `ManagedMeta` + `Value` open_meta + `managed_hash` + `open_hash` strings) translates into an `UpdateResource` cmd carrying only `managed_meta` + `open_meta`. The translator (3b) routes to the meta-only branch.

⚠️ Critical question: **the caller-supplied `managed_hash` and `open_hash` strings have no analog in the `UpdateResource` cmd shape today.** The today's `meta_service::update_meta` writes them verbatim into `kb_resource_manifests.managed_hash` / `open_hash`. Routing through `DbBackend::update_resource` either:

1. **Drops the hashes silently** — server recomputes on receive. This breaks the contract: callers that compute hashes for sync-protocol reasons rely on the server preserving their value verbatim.
2. **Threads the hashes through** — requires extending `UpdateResource` (or `BodyUpdate`-style sibling) with `managed_hash` / `open_hash` fields and updating 3b's translator to thread them into `ResourceUpdateRequest` (or whatever the meta-only path consumes).

This is a translator-shape decision that 3b technically owns, but 3c is the first consumer. **3b's plan must specify which shape the translator commits to.**

For this plan, the implementer's resolution path:

a) Read the 3b plan section "Architecture (locked decisions)" and the translator-shape spec. Confirm whether `managed_hash`/`open_hash` threading is in scope.

b) If 3b's translator already threads them: dispatch through `DbBackend::update_resource` with cmd fields populated. This is the simple path.

c) If 3b's translator does NOT thread them: **escalate to controller before proceeding.** Either 3b is amended to add the threading (ideal), or this MCP tool keeps calling `meta_service::update_meta` directly until a future phase adds the threading (ship-for-now, violates `feedback_no_ship_for_now_workarounds`). The implementer must NOT silently drop the hashes.

**Files:**
- Modify: `crates/temper-mcp/src/tools/resources.rs` (function at line 559)
- Possibly: `crates/temper-core/src/operations/commands.rs` (if 3b deferred the field addition)
- Possibly: `crates/temper-api/src/backend/translators.rs` (if 3b deferred the threading)

- [ ] **Step 1: Verify 3b's translator state**

```bash
grep -B2 -A30 "pub(crate) fn update_resource_to_request\|managed_hash" crates/temper-api/src/backend/translators.rs
grep -B2 -A20 "pub struct UpdateResource\b" crates/temper-core/src/operations/commands.rs
```

Confirm whether `cmd.managed_hash` / `cmd.open_hash` exist on `UpdateResource` and whether the translator threads them into the meta-only branch's downstream call.

- [ ] **Step 2: If gaps exist, escalate or implement (per Step 0's resolution path)**

If `UpdateResource` lacks the hash fields and 3b owns adding them, escalate. If the controller approves 3c picking up the slack, implement them here as a precursor commit:

a) Add `managed_hash: Option<String>, open_hash: Option<String>` to `UpdateResource` (Phase 1 cmd struct). Additive; default to `None` for non-meta-only updates.

b) Extend the translator's meta-only branch to thread the hashes into a meta-only-shaped service call. **The cleanest receiver is still `meta_service::update_meta`** — until that's deleted, the translator can dispatch into it for the meta-only path. After deletion (Task 9c), the hash-write logic must be inlined into `resource_service::update` (or a new method). Either way, hashes are not dropped.

c) Test: `#[sqlx::test]` in `crates/temper-api/src/backend/tests.rs` exercising the meta-only path with caller-supplied hashes; assert the resulting manifest row has the supplied hashes verbatim.

- [ ] **Step 3: Confirm regression baseline**

```bash
cargo nextest run --manifest-path tests/e2e/Cargo.toml --features test-db -E 'test(mcp) and test(meta)' --no-fail-fast
cargo nextest run -p temper-api --features test-db meta::tests --no-fail-fast
```

- [ ] **Step 4: Rewire `update_resource_meta`**

Replace the body of `update_resource_meta` with:

```rust
pub async fn update_resource_meta(
    svc: &TemperMcpService,
    input: UpdateResourceMetaInput,
) -> Result<CallToolResult, rmcp::ErrorData> {
    let profile = svc.require_profile().await?;
    let pool = &svc.api_state.pool;
    let profile_id = ProfileId::from(profile.id);
    let resource_id = ResourceId::from(input.id);

    use temper_api::backend::DbBackend;
    use temper_core::operations::{Backend, ResourceRef, Surface, UpdateResource};

    let cmd = UpdateResource {
        resource: ResourceRef::Uuid { id: resource_id },
        body: None,
        managed_meta: Some(input.managed_meta),
        open_meta: Some(input.open_meta),
        // Iff Step 2 added these:
        managed_hash: Some(input.managed_hash),
        open_hash: Some(input.open_hash),
        origin: Surface::Mcp,
    };

    let backend = DbBackend::new(pool.clone(), profile_id, "mcp".to_string(), Surface::Mcp);
    backend.update_resource(cmd).await.map_err(|e| {
        use temper_core::error::TemperError;
        match e {
            TemperError::Forbidden => rmcp::ErrorData::invalid_params(
                "Resource not found or not modifiable".to_string(), None,
            ),
            TemperError::NotFound(msg) => rmcp::ErrorData::invalid_params(
                format!("Resource not found: {msg}"), None,
            ),
            TemperError::BadRequest(msg) => rmcp::ErrorData::invalid_params(msg, None),
            other => rmcp::ErrorData::internal_error(
                format!("Failed to update resource meta: {other}"), None,
            ),
        }
    })?;

    let response = UpdateResourceMetaResponse { updated: true, id: input.id };
    Ok(CallToolResult::success(vec![rmcp::model::Content::text(
        to_text(&response),
    )]))
}
```

If the hash-threading was deferred (Step 2(b)), the translator currently dispatches into `meta_service::update_meta` for the meta-only path, so the call still works — but `meta_service::update_meta` does NOT get deleted in Task 9c until the threading is fully inlined.

- [ ] **Step 5: Run targeted tests**

```bash
cargo nextest run --manifest-path tests/e2e/Cargo.toml --features test-db -E 'test(mcp) and test(meta)' --no-fail-fast
cargo nextest run -p temper-api --features test-db meta::tests --no-fail-fast
```

Hash-verbatim assertion: the targeted test must verify `kb_resource_manifests.managed_hash` ends up exactly equal to the caller-supplied input. If it doesn't, the hashes are being dropped or recomputed — STOP and escalate.

- [ ] **Step 6: Run full e2e + check**

```bash
cargo nextest run --manifest-path tests/e2e/Cargo.toml --features test-db --no-fail-fast
cargo nextest run --manifest-path tests/e2e/Cargo.toml --features test-db,test-embed --no-fail-fast
cargo make check
```

- [ ] **Step 7: Commit**

```bash
git add crates/temper-mcp/src/tools/resources.rs crates/temper-core/src/operations/commands.rs crates/temper-api/src/backend/
git commit -m "refactor(mcp): dispatch update_resource_meta through DbBackend

The MCP update_resource_meta tool's typed MetaUpdatePayload (typed
ManagedMeta + Value open_meta + caller-supplied managed_hash/open_hash)
translates into an UpdateResource cmd carrying only managed_meta and
open_meta. The translator routes to the meta-only branch.

Caller-supplied hashes thread through verbatim, preserving the today's
contract that hash strings round-trip into kb_resource_manifests
without recomputation.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

### Task 7: Migrate `tools::resources::delete_resource` to DbBackend

**Why:** Mechanical. Today's tool calls `resource_service::delete(pool, profile_id, resource_id, "mcp")` directly. After 3c, dispatch through `DbBackend::delete_resource`.

**Files:**
- Modify: `crates/temper-mcp/src/tools/resources.rs` (function at line 609)

- [ ] **Step 1: Re-verify**

```bash
grep -n "pub async fn delete_resource\|resource_service::delete" crates/temper-mcp/src/tools/resources.rs
```

- [ ] **Step 2: Confirm regression baseline**

```bash
cargo nextest run --manifest-path tests/e2e/Cargo.toml --features test-db -E 'test(mcp) and test(delete)' --no-fail-fast
```

- [ ] **Step 3: Rewire**

```rust
pub async fn delete_resource(
    svc: &TemperMcpService,
    input: DeleteResourceInput,
) -> Result<CallToolResult, rmcp::ErrorData> {
    let profile = svc.require_profile().await?;
    let pool = &svc.api_state.pool;
    let profile_id = ProfileId::from(profile.id);

    use temper_api::backend::DbBackend;
    use temper_core::operations::{Backend, DeleteResource, ResourceRef, Surface};
    use temper_core::types::ids::ResourceId;

    let cmd = DeleteResource {
        resource: ResourceRef::Uuid { id: ResourceId::from(input.id) },
        force: false, // CLI-side concern; DbBackend ignores per spec
        origin: Surface::Mcp,
    };

    let backend = DbBackend::new(pool.clone(), profile_id, "mcp".to_string(), Surface::Mcp);
    backend.delete_resource(cmd).await.map_err(|e| {
        rmcp::ErrorData::internal_error(format!("Failed to delete resource: {e}"), None)
    })?;

    let response = DeleteResourceResponse { deleted: true, id: input.id };
    Ok(CallToolResult::success(vec![rmcp::model::Content::text(
        to_text(&response),
    )]))
}
```

- [ ] **Step 4: Run targeted + full e2e + check**

```bash
cargo nextest run --manifest-path tests/e2e/Cargo.toml --features test-db -E 'test(mcp) and test(delete)' --no-fail-fast
cargo nextest run --manifest-path tests/e2e/Cargo.toml --features test-db --no-fail-fast
cargo make check
```

- [ ] **Step 5: Commit**

```bash
git add crates/temper-mcp/src/tools/resources.rs
git commit -m "refactor(mcp): dispatch delete_resource through DbBackend

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

### Task 8: Migrate `tools::search::search` to DbBackend — REMOVED (out of 3c scope)

**Why removed:** Read-only/search MCP tools stay service-direct, mirroring 3b's A5 deviation. Wire-shape mismatch (today returns rich shapes; DbBackend trait projects to lossy types) makes trivial migration either narrow the contract (visible to MCP test fixtures and downstream agent-prompt examples) or expand the trait with surface-shaped types. Reads are passthroughs; the unification value is on writes.

**Implication for downstream tasks:**
- Task 9a's grep for read-path callers in `crates/temper-mcp/` will continue to find `resource_service::get_visible`, `resource_service::get_by_slug`, `resource_service::get_content`, `resource_service::list_visible`, `search_service::search` — these are EXPECTED, not regressions.
- Task 10 verification step 2 should treat read-path service calls in MCP as in-spec.

If a future phase decides to unify reads, this can land as a follow-up.

---

### Task 9a: Final verification before retiring services

**Why:** Before deleting `resource_service::create`, `ingest_service::update`, and `meta_service::update_meta`, **prove they have zero remaining callers** — and re-confirm 3b's deletes (if any) match what 3c assumed.

**Files:** read-only.

- [ ] **Step 1: Confirm 3b's actions on `resource_service::create`**

3b is responsible for migrating `POST /api/resources` (which calls `resource_service::create` at `handlers/resources.rs:121`). Read 3b's plan: did it queue `resource_service::create`'s deletion? If yes, verify the delete already happened on this branch:

```bash
grep -n "pub async fn create\b" crates/temper-api/src/services/resource_service.rs
```

If absent: 3b deleted it. Skip Task 9b.
If present: 3b deferred to 3c per the spec; proceed to Task 9b.

- [ ] **Step 2: Confirm `ingest_service::update` callers**

```bash
grep -rn "ingest_service::update\b" crates/ tests/
```

Expected after Task 5: zero callers in `crates/` (handlers + MCP migrated). The remaining hits should be in `tests/e2e/tests/mcp_ingest_test.rs` and `tests/e2e/tests/mcp_round_trip_test.rs` — these use `ingest_service::update` as a TEST FIXTURE (seeding test rows / direct service-layer test of the ingest re-process path) rather than as the SUT. Read each call site to confirm.

If any remaining caller is **production code** (not in `tests/e2e/`), STOP and escalate — Task 9c blocks.

If all remaining callers are tests, the test fixtures stay; `ingest_service::update` stays alive **as a tested-but-not-dispatched-through path**. Per `feedback_no_premature_backward_compat`: a function used only by tests is dead production code. The right move is to **inline the test fixture's logic** (likely just `sqlx::query!` plus a `pipeline::process` call) into the test file or a test helper, then delete the production function.

Implementer's choice (escalate to controller):

a) Inline the test-fixture usage (clean; ~20 lines of test-helper code per test file). Recommended.
b) Keep `ingest_service::update` alive marked `#[cfg(any(test, feature="test-db"))]`. Compromise; the function stays in production binaries' debug builds.
c) Keep it alive unmarked. Worst option; defeats the point of the cleanup.

For this plan: pick (a). Per `feedback_subagent_escalate_not_soften`, escalate if (a) reveals an unanticipated dependency.

- [ ] **Step 3: Confirm `meta_service::update_meta` callers**

```bash
grep -rn "meta_service::update_meta\b" crates/ tests/
```

Expected after Task 6: zero callers in `crates/temper-mcp` (3c migrated) and zero in `crates/temper-api/src/handlers/meta.rs` (3b migrated). Remaining hits:

- `crates/temper-api/tests/meta_reconcile_test.rs:72,114` — TEST FIXTURE callers (verified in plan-write).
- `tests/e2e/tests/mcp_round_trip_test.rs:579` — TEST FIXTURE caller.

Same disposition as Task 9a Step 2: inline the test usage or escalate.

- [ ] **Step 4: No commit (verification task)**

The output of this task is the implementer's confidence in proceeding. Document the findings in the Task 9b/9c/9d commit messages as evidence.

---

### Task 9b: Delete `resource_service::create` (if 3b deferred)

**Why:** `feedback_no_premature_backward_compat`: zero callers, no reason to keep.

**Files:**
- Modify: `crates/temper-api/src/services/resource_service.rs` (remove `pub async fn create` at line 485)
- Possibly modify: `crates/temper-api/tests/*.rs` — if any integration test calls `resource_service::create` directly as a fixture, inline the fixture or delete the test if it tested the now-removed path explicitly.

- [ ] **Step 1: Final grep**

```bash
grep -rn "resource_service::create\b" crates/ tests/
```

Expected: zero. If hits remain, do not proceed — they need migration first.

- [ ] **Step 2: Delete the function**

Edit `crates/temper-api/src/services/resource_service.rs`. Remove the entire `pub async fn create(...) -> ApiResult<ResourceRow> { ... }` block (lines 485-525 in the as-written file; verify before edit).

If there's an associated `pub struct ResourceCreateRequest` (or similar) used only by the deleted function, delete that too. Run `cargo check -p temper-api` to surface any remaining usage.

- [ ] **Step 3: Run tests**

```bash
cargo nextest run -p temper-api --features test-db --no-fail-fast
cargo nextest run --manifest-path tests/e2e/Cargo.toml --features test-db --no-fail-fast
cargo make check
```

- [ ] **Step 4: Commit**

```bash
git add crates/temper-api/src/services/resource_service.rs
git commit -m "refactor(api): delete resource_service::create

Final caller (POST /api/resources) was migrated to DbBackend.create_resource
in 3b. No production or test callers remain. Per feedback_no_premature_backward_compat,
remove rather than keep alive.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

### Task 9c: Delete `ingest_service::update`

**Why:** Same. The MCP `update_resource` tool was the last production caller; Task 5 migrated it.

⚠️ This task is the most likely to be blocked by test-fixture entanglement. Re-run Task 9a Step 2 immediately before starting.

**Files:**
- Modify: `crates/temper-api/src/services/ingest_service.rs` (remove `pub async fn update` at line 623)
- Possibly modify: `tests/e2e/tests/mcp_ingest_test.rs`, `tests/e2e/tests/mcp_round_trip_test.rs` — inline the test fixtures.

- [ ] **Step 1: Final grep + decision**

```bash
grep -rn "ingest_service::update\b" crates/ tests/
```

For each remaining hit:
- If in `tests/e2e/tests/`: inline the fixture into a test helper. The fixture's job is "update an existing resource's body and re-process it" — that's `ingest_service::create_resource_with_manifest` for the row + pipeline call. Read each call site, replicate inline, delete the call.
- If in any other location: STOP, escalate.

- [ ] **Step 2: Inline test fixtures**

For each test file with `ingest_service::update`:
1. Read the call site + ~20 lines around it.
2. Identify what it does (probably: `update_resource_manifest` + `pipeline::process_revision` or similar).
3. Either replace with a direct call to the lower-level helpers, OR — if there are many call sites and they share a shape — extract a `test_helper::ingest_update(...)` in `tests/e2e/tests/common/`.

- [ ] **Step 3: Delete the function**

Remove `pub async fn update(...)` from `crates/temper-api/src/services/ingest_service.rs`. Run `cargo check` to surface any remaining usage.

- [ ] **Step 4: Run tests**

```bash
cargo nextest run -p temper-api --features test-db --no-fail-fast
cargo nextest run --manifest-path tests/e2e/Cargo.toml --features test-db --no-fail-fast
cargo nextest run --manifest-path tests/e2e/Cargo.toml --features test-db,test-embed --no-fail-fast
cargo make check
```

- [ ] **Step 5: Commit**

```bash
git add crates/temper-api/src/services/ingest_service.rs tests/e2e/tests/
git commit -m "refactor(api): delete ingest_service::update; inline test fixtures

Final production caller (MCP update_resource tool) was migrated to
DbBackend.update_resource in 3c Task 5. Test-fixture callers in
tests/e2e/tests/mcp_*.rs were the only remaining users; this commit
inlines those into local helpers and removes the now-unused function.

Per feedback_no_premature_backward_compat: a function used only by
tests is dead production code; remove rather than mark cfg(test).

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

### Task 9d: Delete `meta_service::update_meta` (or block on hash threading)

**Why:** Same pattern. The MCP `update_resource_meta` tool was the last production caller; Task 6 migrated it.

⚠️ Conditional on Task 6's hash-threading state. If Task 6's translator currently dispatches into `meta_service::update_meta` for the meta-only path (the deferred-threading workaround), `meta_service::update_meta` is STILL a production caller via the translator — DO NOT DELETE. In that case, this task is renamed to "Inline meta-only hash-write logic into resource_service::update or new method, then delete meta_service::update_meta" and becomes a follow-up task, not part of 3c's terminal cleanup.

If Task 6 fully threaded the hashes via `cmd.managed_hash`/`cmd.open_hash` and the translator inlines the manifest hash-write logic itself (or routes it through `resource_service::update`), then `meta_service::update_meta` is genuinely dead and can be deleted.

**Files:**
- Modify: `crates/temper-api/src/services/meta_service.rs` (remove `pub async fn update_meta`)
- Possibly modify: `crates/temper-api/tests/meta_reconcile_test.rs`, `tests/e2e/tests/mcp_round_trip_test.rs` — inline test fixtures.

- [ ] **Step 1: Final grep + decision**

```bash
grep -rn "meta_service::update_meta\b" crates/ tests/
```

Filter as in Task 9a Step 3. If the translator still calls it: STOP, this task is deferred to follow-up.

- [ ] **Step 2-5: Same shape as Task 9c (inline fixtures, delete function, run tests, commit)**

```bash
git commit -m "refactor(api): delete meta_service::update_meta; inline test fixtures

Final production caller (MCP update_resource_meta tool) was migrated
to DbBackend.update_resource (meta-only branch) in 3c Task 6.
Test-fixture callers were the only remaining users; this commit inlines
those into local helpers and removes the now-unused function.

The hash-write logic (kb_resource_manifests.managed_hash/open_hash) is
now part of the meta-only translator branch in DbBackend; caller-supplied
hashes thread through verbatim per the original contract.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

If this task is deferred (translator still calls `meta_service::update_meta`), capture as a follow-up backlog task:

```bash
cat <<'EOF' | temper resource create --type task --title "Inline meta-only hash-write into resource_service::update; delete meta_service::update_meta" --context temper --mode plan --effort small
After 3c, the DbBackend.update_resource translator's meta-only branch
still dispatches into meta_service::update_meta because the hash-write
logic (kb_resource_manifests.managed_hash/open_hash + cascade_identity_fields
+ event/audit/edge reconciliation) hadn't been inlined yet.

This task: pick the right home for that logic (most likely
resource_service::update with a meta-only fast-path, OR a new
resource_service::update_meta_inline helper), inline it, route the
translator there, delete meta_service::update_meta.

## Why
Eliminates the last divergent service sibling. After this, the unified
DbBackend dispatch path covers every resource mutation with no service-layer
ramps left.

## Acceptance
- meta_service::update_meta is deleted.
- All meta-related tests (meta_reconcile_test.rs, mcp_round_trip_test.rs,
  mcp_resource_parity_test.rs) pass with the inlined logic.
- The hash-verbatim contract is preserved (caller-supplied hashes round-trip
  to kb_resource_manifests without recomputation).
EOF
```

---

### Task 10: Final regression sweep

**Why:** Spec acceptance gate. Confirms the unified dispatch path is fully wired and every test surface is green.

**Files:** none modified.

- [ ] **Step 1: Run the full battery**

```bash
cargo make check
cargo nextest run --workspace --no-fail-fast
cargo nextest run -p temper-api --features test-db --no-fail-fast
cargo nextest run --manifest-path tests/e2e/Cargo.toml --features test-db --no-fail-fast
cargo nextest run --manifest-path tests/e2e/Cargo.toml --features test-db,test-embed --no-fail-fast
```

Each command: exit 0. Watch for `error: test run failed` / `FAIL [` per `feedback_nextest_summary_lies`.

⚠️ The embed-gated run (`--features test-db,test-embed`) is **mandatory** per `project_workspace_feature_unification_ort`. The Embed CI job is the only one with ONNX Runtime installed and is the one that catches workspace-feature-unification surprises (e.g., temper-cloud enabling ingest-pipeline on temper-api).

If embed-gated tests cannot run locally (no ONNX runtime), document in the PR description and rely on CI's Embed job — but the non-embed runs MUST pass.

- [ ] **Step 2: Verify no service callers remain**

```bash
grep -rn "ingest_service::ingest\b\|ingest_service::update\b\|resource_service::create\b\|resource_service::update\b\|resource_service::delete\b\|resource_service::list_visible\b\|resource_service::get_visible\b\|resource_service::get_by_slug\b\|resource_service::resolve_by_uri\b\|resource_service::get_content\b\|resource_service::check_can_modify\b\|meta_service::update_meta\b\|meta_service::get_meta\b\|search_service::search\b" crates/temper-mcp/ crates/temper-cli/
```

Expected hits in `crates/temper-mcp/`:
- `resource_service::get_visible` — used in `get_resource` (read-only, stays service-direct) and post-update enrichment in `update_resource`. OK.
- `resource_service::get_by_slug` — used in `get_resource` slug branch (read-only, stays service-direct). OK.
- `resource_service::get_content` — used in `get_resource` (when `include_content=true`, read-only, stays service-direct). OK.
- `resource_service::list_visible` — used in `list_resources` (read-only, stays service-direct per Task 4 removal). OK.
- `search_service::search` — used in `tools/search.rs::search` (stays service-direct per Task 8 removal). OK.
- `context_service::*`, `doc_type_service::*` — out of scope; OK.

Expected hits in `crates/temper-cli/`: TBD by 3b's scope (CLI may or may not be migrated separately). If 3b/3c are scoped to API+MCP, CLI hits are OK.

Hits that indicate a regression:
- `ingest_service::ingest` in `crates/temper-mcp/` — should be ZERO (Task 2 migrated it).
- `ingest_service::update` in `crates/temper-mcp/` — should be ZERO (Task 5 migrated it).
- `meta_service::update_meta` in `crates/temper-mcp/` — should be ZERO (Task 6 migrated it).
- `resource_service::delete` in `crates/temper-mcp/` — should be ZERO (Task 7 migrated it).

If any of these regressions show up: STOP, the migration is incomplete.

- [ ] **Step 3: Verify the dispatch path is the canonical create path**

```bash
grep -n "DbBackend::new\|backend\.create_resource\|backend\.update_resource\|backend\.delete_resource" crates/temper-mcp/src/tools/
```

Expected: at least one DbBackend dispatch in each migrated **write-path** tool function (`create_resource`, `update_resource`, `update_resource_meta`, `delete_resource`). Read-only tools (`get_resource`, `list_resources`, `search`) will NOT appear in this grep — they stay service-direct per the A5 deviation.

- [ ] **Step 4: Update CLAUDE.md (deferred from 3a's spec § Acceptance Criteria)**

Edit `/Users/petetaylor/projects/tasker-systems/temper/CLAUDE.md`. Replace the "Service layer owns SQL" rule with:

> - **Service layer owns SQL** — All SQL lives in `temper-api/src/services/`. Backends (`temper-api/src/backend/DbBackend`) compose service calls into trait methods; surfaces (HTTP handlers, MCP tools, CLI actions) build a backend per request and dispatch one operations command per inbound call. Never inline `sqlx::query!()` outside a service. Never call services directly from a surface — go through the backend trait.

This is a doc commit; no code changes. The implementer can fold it into Task 10's commit if convenient.

- [ ] **Step 5: Final commit (if doc updated)**

```bash
git add CLAUDE.md
git commit -m "docs: update service-layer rule to reference DbBackend dispatch

After Wave 1 Phase 3 (3a/3b/3c), surfaces compose a backend per request
and dispatch operations commands; they no longer call services directly.
Updates the architectural rule in CLAUDE.md to reflect this.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

- [ ] **Step 6: Mark plan tasks done**

```bash
temper resource update 2026-05-07-wave1-phase3c-mcp-tool-migration --type task --stage done
```

---

## Acceptance Criteria

- [ ] Every **write-path** MCP tool in `tools/resources.rs` (`create_resource`, `update_resource`, `update_resource_meta`, `delete_resource`) dispatches through `DbBackend`. Verified by grep in Task 10 Step 3.
- [ ] Read-only MCP tools (`get_resource`, `list_resources`) and the search MCP tool stay service-direct (Tasks 3, 4, 8 removed; mirrors 3b's A5).
- [ ] No `ingest_service::ingest`, `ingest_service::update`, `resource_service::create`, `resource_service::delete`, or `meta_service::update_meta` calls remain in `crates/temper-mcp/`. Verified by grep in Task 10 Step 2. (`search_service::search`, `resource_service::get_visible/get_by_slug/get_content/list_visible` calls in MCP are EXPECTED — read-path stays service-direct.)
- [ ] `resource_service::create`, `ingest_service::update` deleted (Tasks 9b, 9c).
- [ ] `meta_service::update_meta` deleted (Task 9d) OR follow-up backlog task created (Task 9d's escape hatch).
- [ ] `ensure_managed_identity_keys` send-side calls preserved at the entry of `create_resource` and `update_resource` MCP tools.
- [ ] `cargo make check` clean.
- [ ] `cargo nextest --workspace --no-fail-fast` exits 0.
- [ ] `cargo nextest -p temper-api --features test-db --no-fail-fast` exits 0.
- [ ] `cargo nextest --manifest-path tests/e2e/Cargo.toml --features test-db --no-fail-fast` exits 0.
- [ ] `cargo nextest --manifest-path tests/e2e/Cargo.toml --features test-db,test-embed --no-fail-fast` exits 0 (or documented limitation + CI Embed job green).
- [ ] CLAUDE.md updated to reference DbBackend dispatch (Task 10 Step 4).

---

## Risks & Open Questions

1. **`update_resource_meta` hash threading (Task 6).** The most architecturally load-bearing decision. If 3b's translator does NOT thread `managed_hash`/`open_hash` through to the manifest write, 3c either blocks or ships with a dispatch-into-`meta_service::update_meta` workaround. The plan above documents the escalation path; the implementer must NOT silently drop the hashes.

2. ~~**`get_resource` slug-without-doctype.**~~ — Resolved: `get_resource` cut from 3c (Task 3 removed). The `ResourceRef::Slug { slug, context }` variant is **not** added in this phase; if a future phase migrates read-only MCP tools, that's where the variant lands.

3. ~~**List/search response shape narrowing.**~~ — Resolved: tasks 4 and 8 removed. Read paths stay service-direct.

4. **Test-fixture entanglement (Task 9a–9d).** Three of the retirement targets are used by test fixtures. The plan recommends inlining the fixture logic and deleting the production function. If inlining reveals an unanticipated dependency, the deletion task is deferred to a follow-up; the function stays alive but unused-in-production with `#[cfg(any(test, feature="test-db"))]` as a temporary state. Per `feedback_no_ship_for_now_workarounds`, this is NOT acceptable as a terminal state — it MUST be captured as a backlog task with a hard deadline.

5. **3b → 3c branch coordination.** Both phases share `jct/wave1-phase3bc-handler-mcp-migration`. 3c starts ONLY after 3b is fully landed on the branch. The first action of any 3c implementer subagent is to verify 3b's terminal commit is the current HEAD parent. If branch is shared and 3b is incomplete, STOP.

6. **Auth pre-check removal in update_resource (Task 5).** The today's tool calls `resource_service::check_can_modify` BEFORE building the cmd. The new dispatch relies on the inner `resource_service::update`'s `can_modify_resource` SQL gate. This is functionally equivalent at the SQL level but the user-facing error mapping changes. Verify test assertions are not error-message-specific. If any test fails on the message change, escalate per `feedback_subagent_escalate_not_soften`.

7. **`enrich_resource` post-call pattern (Tasks 2, 5, possibly 7).** Today's tools call `enrich_resource` after the dispatch to get `EnrichedResource` (with context_name, doc_type_name resolved). After migration, `DbBackend::create_resource` returns `ResourceRow` which already has these names — `enrich_resource` is technically redundant for those fields. The plan keeps it for output-format stability (no test assertion breakage), but a follow-up cleanup could remove it.
