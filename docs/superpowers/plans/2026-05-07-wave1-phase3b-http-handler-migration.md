# Wave 1 Phase 3b — HTTP Handler Migration Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use `superpowers:subagent-driven-development` (recommended) or `superpowers:executing-plans` to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking. **Cannot write to file in this session — copy this content into `docs/superpowers/plans/2026-05-07-wave1-phase3b-http-handler-migration.md`.**

**Goal:** Make `DbBackend` the canonical write-path dispatch target for HTTP handlers. Migrate the four write-path handlers (`POST /api/resources`, `PATCH /api/resources/{id}`, `POST /api/ingest`, `PUT /api/ingest/{id}`, `PUT /api/resources/{id}/meta`) and their three sibling service functions (`resource_service::create`, `ingest_service::update`, `meta_service::update_meta`) into a single unified flow through `DbBackend::create_resource` / `DbBackend::update_resource`. Read-only handlers and the search handler stay service-direct (the trait projects to lossy types — see "Architecture decisions").

**Predecessors:**
- Plan: `docs/superpowers/plans/2026-05-07-wave1-phase3a-dbbackend-foundation.md`
- Spec: `docs/superpowers/specs/2026-05-07-wave1-phase3-dbbackend-design.md` (canonical reference; this plan supersedes the spec's "Sub-phase decomposition" handler list, see *§Verified inventory* below)

**Branch:** `jct/wave1-phase3bc-handler-mcp-migration` — same branch will carry 3c.

**Tech stack:** Rust 2024, axum, sqlx, async-trait, tokio. `cfg(feature = "ingest-pipeline")`-gated body-trio computation.

---

## Why

Phase 3a landed `DbBackend` as a 6-method `Backend` trait impl that wraps existing services. It is dark-launched: every HTTP handler still calls `services::*` directly. Phase 3b cuts the dependency the other way — handlers stop importing `services::*`-write functions and instead build a `DbBackend` and dispatch a typed `operations::*Resource` command through it.

The mechanical rewire is half the value. The other half is the *consolidation* it forces: today there are three update-shape paths (`resource_service::update`, `ingest_service::update`, `meta_service::update_meta`) each entered by a different handler, each with its own quirks. After 3b they collapse into one — `DbBackend::update_resource` — with the translator branching on the cmd shape (`body | meta | title/slug`). The schema-required-defaults-at-create-or-update rule (CLAUDE.md) and the Phase 5 symmetric-defense `ensure_managed_identity_keys` wiring stay in `resource_service::update`, which is the canonical destination for all three branches.

The wire surface stays. CLI sync (`crates/temper-cli/src/actions/sync.rs`) hits `POST /api/ingest`, `PUT /api/ingest/{id}`, `POST /api/resources`, `PATCH /api/resources/{id}`, `PUT /api/resources/{id}/meta` — every endpoint stays alive; only handler bodies rewire.

---

## Architecture (locked decisions; do not re-debate)

These were chosen in a brainstorming session before this plan was written. The plan implements them; it does not litigate them.

### A1. Body-update path: `prepare_body_trio` helper

Add a `prepare_body_trio(body: &str) -> Result<(String, String), TemperError>` helper in `crates/temper-api/src/backend/translators.rs` (or a sibling `body_pipeline.rs` if translators.rs grows past ~200 lines — implementer's call, see Task 2 step 1). It computes:
- `content_hash` via `temper_core::hash::compute_body_hash(body)` (verified to be the same primitive used today at `crates/temper-api/src/services/ingest_service.rs:666`).
- `chunks_packed` via `temper_ingest::pipeline::prepare_markdown(body)` → `temper_core::types::ingest::pack_chunks(&packed_chunks)` (verified at `ingest_service.rs:667-674`).

Gated `#[cfg(feature = "ingest-pipeline")]`. The non-pipeline path returns `TemperError::BadRequest("chunks_packed required when server-side pipeline is not available")` — the exact contract preserved from `ingest_service.rs:678-683`.

The translator `update_resource_to_request` (currently a stub at `crates/temper-api/src/backend/translators.rs:58-74`) calls `prepare_body_trio` when `cmd.body.is_some()` and populates `ResourceUpdateRequest.content_hash` + `chunks_packed`. This makes `DbBackend::update_resource` the first non-stub body-update path.

### A2. Sibling cleanup — three retirements, conditional on caller-elimination

Retire inline in 3b once handlers stop calling them:

- `resource_service::create` — `crates/temper-api/src/services/resource_service.rs:485` (verified).
- `ingest_service::update` — `crates/temper-api/src/services/ingest_service.rs:623` (verified).
- `meta_service::update_meta` — `crates/temper-api/src/services/meta_service.rs:87` (verified).

**Caveat — MCP cross-branch dependency.** 3c lands the MCP migration on the same branch. MCP tools at `crates/temper-mcp/src/tools/resources.rs` (verified: `create_resource`/`update_resource`/`update_resource_meta` etc., lines 226 / 440 / 559) currently call into `temper-api` services directly. Until 3c lands, these tools may still call the to-be-deleted functions. **Resolution:** the deletion tasks (Tasks 12–14) are conditional. Each task ends with `grep -rn "fn_name\b" crates/` verification that zero callers remain. If MCP still calls in, the deletion defers to 3c's final task (a single grep + delete commit). The 3c plan will own the gate.

### A3. Meta-only path folded into `DbBackend::update_resource`

No 7th trait method. The translator branches on `UpdateResource` cmd shape:

1. **`cmd.body.is_some()`** → body-trio path: `prepare_body_trio` populates `(content_hash, chunks_packed)` then calls `resource_service::update` with full trio.
2. **`cmd.managed_meta.is_some() || cmd.open_meta.is_some()`** → meta path: validate via `validate_open_meta_keys` (lifted to `temper-core` per A4), then `resource_service::update` with meta-only fields populated. `resource_service::update` (line 527, verified) already does the partial merge + identity-key injection + recomputes hashes. The current `meta_service::update_meta` reconciles edges on success (line 301 — `super::edge_service::reconcile_edges`); confirm `resource_service::update` performs the equivalent reconciliation (Task 4 verification step), and if not, port that one call into `resource_service::update` rather than keeping a separate path.
3. **else** (title/slug-only) → `resource_service::update`.

`meta_service::update_meta` is retired (Task 14).

### A4. Lift `validate_open_meta_keys` to `temper-core::operations::actions`

**Verified discrepancy from user-supplied requirements:** the user said "Move both `KNOWN_OPEN_FIELDS` registry and `validate_open_meta_keys` from `meta_service`." `KNOWN_OPEN_FIELDS` already lives in `temper-core` (`crates/temper-core/src/frontmatter/registry.rs:45`, re-exported at `frontmatter::mod.rs:25`) and is consumed by `meta_service` via `registry::lookup`. Only `validate_open_meta_keys` (`meta_service.rs:30`) needs to move.

**Action:** lift `validate_open_meta_keys` (and its 7 unit tests at `meta_service.rs:328-409`) into `crates/temper-core/src/operations/actions.rs`. Re-export through `crates/temper-core/src/operations/mod.rs`. Both `DbBackend` and the future `VaultBackend` call it.

### A5. Read-only / search handlers stay service-direct (deviation from user-supplied scope)

**Verified wire-shape mismatches** mean these handlers cannot trivially route through `DbBackend` without losing fields:

| Handler | Returns today | `DbBackend` projection |
|---|---|---|
| `GET /api/resources` (`list`) | `ResourceListResponse { rows, total, facets }` | `Vec<ResourceSummary>` (drops `total`, `facets`) |
| `GET /api/resources/by-uri` (`by_uri`) | `ResourceRow` | (no by-uri trait method; `show_resource` UUID branch resolves first) |
| `GET /api/resources/{id}/content` (`get_content`) | `ContentResponse` | (no trait method) |
| `POST /api/search` (`search`) | `Vec<UnifiedSearchResultRow>` | `Vec<SearchHit>` (drops `kb_uri`, `origin_uri`, `fts_score`, `vector_score`, `origin`) |

**Decision:** these read-only / search handlers stay service-direct. Justification:
- Adding by-uri / get-content trait methods would push us past the "fold meta into update" simplification (A3).
- Routing list/search through `DbBackend` would force returning HTTP-shaped types from the trait (re-coupling the trait to the HTTP wire), or returning the trait's projection from the handler (a wire-contract change visible to existing CLI/UI clients via the typed `temper-client` ↔ `temper-api` round-trip).
- These paths are *not* architecturally load-bearing for unifying writes. The unification value is on writes (defaults, validation, identity injection, edge reconciliation, dedupe, pipeline). Reads are passthroughs.

**The handlers in scope for migration are therefore:**

| Handler | Route | Migration target |
|---|---|---|
| `handlers::resources::get` | `GET /api/resources/{id}` | `DbBackend::show_resource` (UUID branch) |
| `handlers::resources::create` | `POST /api/resources` | `DbBackend::create_resource` (with handler-side ID→name resolve, see A6) |
| `handlers::resources::update` | `PATCH /api/resources/{id}` | `DbBackend::update_resource` |
| `handlers::resources::delete` | `DELETE /api/resources/{id}` | `DbBackend::delete_resource` |
| `handlers::ingest::create` | `POST /api/ingest` | `DbBackend::create_resource` |
| `handlers::ingest::update` | `PUT /api/ingest/{id}` | `DbBackend::update_resource` (body branch) |
| `handlers::meta::update_meta` | `PUT /api/resources/{id}/meta` | `DbBackend::update_resource` (meta branch) |

**Explicitly out of scope (stay service-direct):** `list`, `by_uri`, `get_content`, `get_meta`, `search`. Handler bodies for those 5 routes do not change in 3b. (The user's task list step 5 said "list/by_uri/get/get_content" all migrate; this plan migrates only `get` because the other three have wire-shape mismatches. Documented and accepted.)

### A6. Wire-contract preservation for `POST /api/resources`

**Verified:** `ResourceCreateRequest { kb_context_id: Uuid, kb_doc_type_id: Uuid, origin_uri, title, slug }` (`crates/temper-core/src/types/resource.rs:128-138`) carries IDs, not names. `DbBackend::create_resource` (translator at `backend/translators.rs:27-43`) builds `IngestPayload` from cmd's `context: String` + `doctype: String` (names).

**Resolution:** the migrated `handlers::resources::create` resolves `kb_context_id` → `context_name` and `kb_doc_type_id` → `doc_type_name` in the handler before constructing `CreateResource`. Use existing primitives:
- `context_service::resolve_by_id` (verify exists; if not, do a one-shot `kb_contexts WHERE id = $1` lookup in the handler — but per "Service layer owns SQL", add a thin `resolve_by_id` to the service if missing).
- `kb_doc_types WHERE id = $1` lookup — same approach.

If the lookups fail, return `ApiError::BadRequest`. The `IngestPayload` derived from the result will then be a no-body, no-meta create, which `ingest_service::ingest` handles end-to-end (defaults, dedupe is a no-op when there's no body).

### A7. Wire-contract change in `PUT /api/resources/{id}/meta` response shape

**Verified:** today the handler returns `Json<Value>` shaped `{"updated": true, "resource_id": <uuid>}` (`meta_service.rs:319`). `DbBackend::update_resource` returns `ResourceRow`.

**Verified caller:** `crates/temper-cli/src/actions/sync.rs:997` calls `client.resources().update_meta(...)` and discards the response. The typed return is `Result<serde_json::Value>`. CLI sync does not pattern-match on the shape.

**Decision:** the migrated handler returns `Json<ResourceRow>`. The wire is now `ResourceRow` JSON instead of `{updated, resource_id}` JSON. Update the OpenAPI annotation. Add an integration-test assertion (Task 11) that the response deserializes to `ResourceRow` to lock in the new contract. If a hidden caller breaks (none found in `grep`), fix it forward — do not preserve the old shape per `feedback_no_premature_backward_compat`.

---

## Verified APIs (signatures grep-confirmed against current code)

```rust
// crates/temper-api/src/backend/db_backend.rs (Phase 3a, current)
impl DbBackend {
    pub fn new(pool: PgPool, profile_id: ProfileId, device_id: String, surface: Surface) -> Self
}
impl Backend for DbBackend {
    async fn create_resource(&self, cmd: CreateResource) -> Result<CommandOutput<ResourceRow>, TemperError>
    async fn show_resource(&self, cmd: ShowResource)     -> Result<CommandOutput<ResourceRow>, TemperError>
    async fn update_resource(&self, cmd: UpdateResource) -> Result<CommandOutput<ResourceRow>, TemperError>
    async fn delete_resource(&self, cmd: DeleteResource) -> Result<CommandOutput<()>, TemperError>
    async fn list_resources(&self, cmd: ListResources)   -> Result<CommandOutput<Vec<ResourceSummary>>, TemperError>
    async fn search_resources(&self, cmd: SearchResources) -> Result<CommandOutput<Vec<SearchHit>>, TemperError>
}

// Service entry points that stay (canonical dispatch destinations)
ingest_service::ingest(pool, profile_id: ProfileId, device_id: &str, payload: IngestPayload) -> ApiResult<ResourceRow>     // line 384
resource_service::update(pool, profile_id: Uuid, resource_id: Uuid, device_id: &str, req: ResourceUpdateRequest) -> ApiResult<ResourceRow>  // line 527
resource_service::delete(pool, profile_id: ProfileId, resource_id: ResourceId, device_id: &str) -> ApiResult<...>          // line 820
resource_service::get_visible(pool, profile_id: Uuid, resource_id: Uuid) -> ApiResult<ResourceRow>                          // line 291
resource_service::resolve_by_uri(pool, profile_id: Uuid, params: &ResolveByUriParams) -> ApiResult<ResourceRow>             // line 337
search_service::search(pool, profile_id: Uuid, params: SearchParams) -> ApiResult<Vec<UnifiedSearchResultRow>>              // unchanged

// Service entry points retired in 3b (subject to MCP cross-branch caveat in A2)
resource_service::create(pool, profile_id: Uuid, req: ResourceCreateRequest) -> ApiResult<ResourceRow>   // line 485 — retired Task 12
ingest_service::update(pool, profile_id: ProfileId, resource_id: ResourceId, device_id: &str, payload: IngestPayload) -> ApiResult<ResourceRow>  // line 623 — retired Task 13
meta_service::update_meta(pool, profile_id: ProfileId, resource_id: ResourceId, device_id: &str, payload: MetaUpdatePayload) -> ApiResult<Value>  // line 87 — retired Task 14

// Lifted to temper-core
crate::services::meta_service::validate_open_meta_keys(open_meta: &Value) -> Result<(), String>  // moves to temper_core::operations::validate_open_meta_keys

// Body-pipeline primitives (unchanged; called by new prepare_body_trio helper)
temper_core::hash::compute_body_hash(body: &str) -> String                                                    // grep-confirmed
temper_ingest::pipeline::prepare_markdown(body: &str) -> Result<Vec<PackedChunk>, _>                          // grep-confirmed (feature ingest-pipeline)
temper_core::types::ingest::pack_chunks(chunks: &[PackedChunk]) -> Result<String, _>                          // grep-confirmed

// Existing wire types
ResourceCreateRequest { kb_context_id: Uuid, kb_doc_type_id: Uuid, origin_uri: String, title: String, slug: Option<String> }  // resource.rs:132
ResourceUpdateRequest { title, slug, managed_meta, open_meta, content, content_hash, chunks_packed }  // resource.rs:146
MetaUpdatePayload { resource_id, managed_meta: ManagedMeta, open_meta: Value, managed_hash, open_hash }  // managed_meta.rs:150
IngestPayload { context_name, doc_type_name, slug, title, content, managed_meta, open_meta, content_hash, chunks_packed, origin_uri, metadata }  // verified

// Operations layer cmd types (Phase 1)
CreateResource { context, doctype, slug, title, body: Option<BodyUpdate>, managed_meta, open_meta, origin: Surface }
UpdateResource { resource: ResourceRef, body: Option<BodyUpdate>, managed_meta: Option<ManagedMeta>, open_meta: Option<Value>, origin: Surface }
DeleteResource { resource: ResourceRef, force: bool, origin: Surface }
ShowResource { resource: ResourceRef, origin: Surface }
```

### Wire-route → handler bindings (verified `crates/temper-api/src/routes.rs:48-75`)

```
GET    /api/resources          handlers::resources::list           (NOT migrated — see A5)
POST   /api/resources          handlers::resources::create         (Task 5)
GET    /api/resources/by-uri   handlers::resources::by_uri         (NOT migrated — see A5)
GET    /api/resources/{id}     handlers::resources::get            (Task 6 — show)
PATCH  /api/resources/{id}     handlers::resources::update         (Task 7)
DELETE /api/resources/{id}     handlers::resources::delete         (Task 8)
GET    /api/resources/{id}/content    handlers::resources::get_content (NOT migrated)
GET    /api/resources/{id}/meta       handlers::meta::get_meta         (NOT migrated)
PUT    /api/resources/{id}/meta       handlers::meta::update_meta      (Task 11)
POST   /api/ingest             handlers::ingest::create            (Task 9)
PUT    /api/ingest/{id}        handlers::ingest::update            (Task 10)
POST   /api/search             handlers::search::search            (NOT migrated — see A5)
```

---

## Inherited Project Guidance (embed verbatim in EVERY implementer subagent prompt)

The 3a plan embedded these clauses and they worked. Repeat the pattern. Each implementer subagent MUST be told:

- **`feedback_subagent_check_before_commit`** — Run `cargo make check` before staging. The pre-commit hook is the backstop, not the first line of defense.
- **`feedback_subagent_escalate_not_soften`** — If passing a test requires loosening a contract, error path, or test assertion, STOP and report BLOCKED.
- **`feedback_no_premature_backward_compat`** — Project is one month old. Delete dead code; do not keep "for compat" stubs.
- **`feedback_no_ship_for_now_workarounds`** — No `// TODO` comments shipped. No half-baked attempts. Revert to baseline + capture as a follow-up task.
- **`feedback_plan_regression_guard_after_filter_test`** — Every filter-by-name test run (`cargo nextest run -p X test_name`) must be paired with a full crate suite run (`cargo nextest run -p X --features test-db`) before commit.
- **`feedback_nextest_summary_lies`** — Do NOT trust the per-binary `Summary` line with `--no-fail-fast`. Trust exit code, or grep for `error: test run failed` / `FAIL [`.
- **`feedback_pre_propose_arch_review` + `feedback_plan_verification`** — Verify every named API in this plan against current code before writing code. The plan is a hypothesis; the code is ground truth. `Read` and `grep` first.
- **`project_workspace_feature_unification_ort`** — `cargo nextest --workspace` exercises temper-cloud's ingest-pipeline activation. Include it in regression-guard runs especially after touching `ingest_service` / pipeline code (Tasks 2, 9, 10).

---

## TDD Tasks

> Each task names files touched, has a RED → GREEN → REFACTOR script, names the verification commands, and ends with a commit-message template. The plan-writer ran `feedback_plan_verification` against every API name above before dispatch.

---

### Task 1 — Lift `validate_open_meta_keys` to `temper-core::operations::actions`

**Why:** Both `DbBackend` (this phase) and the future `VaultBackend` (Phase 4) need server-side validation of open-meta keys. Lifting it removes a `temper-api` → `temper-core` directional anomaly and unifies the validation entry point with the rest of the actions module. This is the lift the user described in A4. **NB:** `KNOWN_OPEN_FIELDS` is already in `temper-core` (`frontmatter::registry`) — only the `validate_open_meta_keys` function and its tests move.

**Files:**
- Modify: `crates/temper-core/src/operations/actions.rs` — add the function + tests.
- Modify: `crates/temper-core/src/operations/mod.rs` — re-export.
- Modify: `crates/temper-api/src/services/meta_service.rs` — delete local fn + tests, change call site to `temper_core::operations::validate_open_meta_keys`.

- [ ] **Step 1 (RED):** Copy the 7 unit tests from `meta_service.rs:328-409` into `crates/temper-core/src/operations/actions.rs` `#[cfg(test)] mod tests`. They will fail to compile because `validate_open_meta_keys` doesn't exist yet in operations.
- [ ] **Step 2 (GREEN):** Add to `crates/temper-core/src/operations/actions.rs` (above tests):
  ```rust
  pub fn validate_open_meta_keys(open_meta: &serde_json::Value) -> Result<(), String> {
      let Some(obj) = open_meta.as_object() else { return Ok(()); };
      for key in obj.keys() {
          if crate::frontmatter::registry::lookup(key.as_str()).is_none() {
              return Err(key.clone());
          }
      }
      Ok(())
  }
  ```
  Add `validate_open_meta_keys` to the `pub use actions::{...}` line in `operations/mod.rs:18-22`.
- [ ] **Step 3:** `cargo nextest run -p temper-core operations::actions::tests --no-fail-fast` — expect 7 new tests pass.
- [ ] **Step 4 (REFACTOR):** Delete the local `validate_open_meta_keys` from `meta_service.rs:30-40` and the 7 tests at lines 328-409. Update the call site at `meta_service.rs:112` to `temper_core::operations::validate_open_meta_keys`. Drop `use temper_core::frontmatter::registry;` if it becomes unused.
- [ ] **Step 5:** `cargo make check` — expect clean.
- [ ] **Step 6:** `cargo nextest run -p temper-core --no-fail-fast` and `cargo nextest run -p temper-api --features test-db --no-fail-fast` — expect green; watch for `error: test run failed` / `FAIL [` per `feedback_nextest_summary_lies`.
- [ ] **Step 7:** Commit:
  ```
  refactor(core): lift validate_open_meta_keys to operations::actions

  Phase 3b prep: DbBackend's update_resource path needs to call
  open-meta key validation before delegating to resource_service::update
  with meta-only fields. Lifting from meta_service to temper-core makes
  the validation reachable from both DbBackend and the future
  VaultBackend, and aligns with operations being the canonical
  shared-actions home.

  KNOWN_OPEN_FIELDS already lives in temper-core::frontmatter::registry;
  only the validation fn + tests move.
  ```

---

### Task 2 — Add `prepare_body_trio` helper

**Why:** `DbBackend::update_resource` needs to populate the body trio (`content`, `content_hash`, `chunks_packed`) when `cmd.body.is_some()`. The current translator stub at `backend/translators.rs:58-74` leaves `content_hash` and `chunks_packed` as `None`, which forces `resource_service::update`'s handler-layer guard to reject the call. This task lifts the body-pipeline trio computation from `ingest_service.rs:666-674` into a translator-layer helper.

**Files:**
- Modify: `crates/temper-api/src/backend/translators.rs` (or create a sibling `body_pipeline.rs` if translators.rs grows past ~200 lines after this change — implementer's call).

- [ ] **Step 1:** Read `crates/temper-api/src/services/ingest_service.rs:660-685` to lock in the exact pipeline call sequence and feature gating.
- [ ] **Step 2 (RED):** Append a unit test to `backend/translators.rs` (under `#[cfg(test)] mod tests` — create if not present):
  ```rust
  #[test]
  fn prepare_body_trio_computes_hash_and_packs_chunks() {
      let body = "# heading\n\nparagraph text.\n";
      let (hash, packed) = prepare_body_trio(body).expect("pipeline ok");
      assert_eq!(hash.len(), 64); // sha256 hex
      assert!(!packed.is_empty()); // base64 mp
  }

  #[test]
  fn prepare_body_trio_empty_body_ok() {
      let (_hash, _packed) = prepare_body_trio("").expect("empty body still hashable");
  }
  ```
  Run: `cargo nextest run -p temper-api --features test-db,ingest-pipeline backend::translators::tests --no-fail-fast` — expect compile failure (`prepare_body_trio` not defined).
- [ ] **Step 3 (GREEN):** Add the helper to `backend/translators.rs`:
  ```rust
  /// Compute (content_hash, chunks_packed) for an update-resource body. Mirrors
  /// the in-place pipeline at ingest_service.rs:666-674. Gated on the
  /// `ingest-pipeline` feature; without it, returns BadRequest preserving the
  /// contract from ingest_service.rs:678-683.
  #[cfg(feature = "ingest-pipeline")]
  pub(crate) fn prepare_body_trio(body: &str) -> Result<(String, String), TemperError> {
      use temper_core::error::TemperError;
      let hash = temper_core::hash::compute_body_hash(body);
      let packed_chunks = temper_ingest::pipeline::prepare_markdown(body)
          .map_err(|e| TemperError::Api(format!("embed: {e}")))?;
      let packed = temper_core::types::ingest::pack_chunks(&packed_chunks)
          .map_err(|e| TemperError::Api(format!("pack: {e}")))?;
      Ok((hash, packed))
  }

  #[cfg(not(feature = "ingest-pipeline"))]
  pub(crate) fn prepare_body_trio(_body: &str) -> Result<(String, String), TemperError> {
      Err(TemperError::BadRequest(
          "chunks_packed required when server-side pipeline is not available".to_owned(),
      ))
  }
  ```
- [ ] **Step 4:** Run the test from Step 2 — expect green. Then run full crate: `cargo nextest run -p temper-api --features test-db --no-fail-fast`.
- [ ] **Step 5:** Workspace regression run: `cargo nextest run --workspace --no-fail-fast` — catches the temper-cloud feature-unification surface (per `project_workspace_feature_unification_ort`).
- [ ] **Step 6:** Commit:
  ```
  feat(api): add prepare_body_trio helper for body-update path

  Lifts the (compute_body_hash, prepare_markdown, pack_chunks) sequence
  from ingest_service.rs:666-674 into a translator-layer helper so
  DbBackend::update_resource can populate the body trio when
  cmd.body.is_some(). Preserves the non-pipeline BadRequest contract.
  ```

---

### Task 3 — Branch `update_resource_to_request` translator on cmd shape

**Why:** With Task 2 in place, the translator can stop returning a "stub-with-None-trio" and produce real `ResourceUpdateRequest` shapes for all three branches (body / meta / title-or-slug). Removes the 3a `TODO`-equivalent comment at `translators.rs:51-57`.

**Files:**
- Modify: `crates/temper-api/src/backend/translators.rs`

- [ ] **Step 1 (RED):** Add three tests to `backend/translators.rs::tests`:
  ```rust
  #[test]
  fn update_translator_body_branch_populates_trio() {
      let cmd = UpdateResource {
          resource: ResourceRef::Uuid { id: ResourceId::from(uuid::Uuid::new_v4()) },
          body: Some(BodyUpdate { content: "# x".into() }),
          managed_meta: None,
          open_meta: None,
          origin: Surface::ApiHttp,
      };
      let req = update_resource_to_request(cmd).expect("ok");
      assert!(req.content.is_some());
      assert!(req.content_hash.is_some());
      assert!(req.chunks_packed.is_some());
  }

  #[test]
  fn update_translator_meta_branch_leaves_body_fields_none() {
      let cmd = UpdateResource {
          resource: ResourceRef::Uuid { id: ResourceId::from(uuid::Uuid::new_v4()) },
          body: None,
          managed_meta: Some(ManagedMeta::default()),
          open_meta: Some(serde_json::json!({"tags": ["x"]})),
          origin: Surface::ApiHttp,
      };
      let req = update_resource_to_request(cmd).expect("ok");
      assert!(req.content.is_none());
      assert!(req.content_hash.is_none());
      assert!(req.chunks_packed.is_none());
      assert!(req.managed_meta.is_some());
      assert!(req.open_meta.is_some());
  }

  #[test]
  fn update_translator_meta_branch_rejects_unknown_open_key() {
      let cmd = UpdateResource {
          resource: ResourceRef::Uuid { id: ResourceId::from(uuid::Uuid::new_v4()) },
          body: None,
          managed_meta: None,
          open_meta: Some(serde_json::json!({"totally_made_up": 1})),
          origin: Surface::ApiHttp,
      };
      let err = update_resource_to_request(cmd).expect_err("unknown key");
      assert!(matches!(err, TemperError::BadRequest(_)));
  }
  ```
- [ ] **Step 2 (GREEN):** Replace `update_resource_to_request` with a fallible version returning `Result<ResourceUpdateRequest, TemperError>`. Body branch calls `prepare_body_trio`; meta branch calls `temper_core::operations::validate_open_meta_keys` on `cmd.open_meta` (skip if None) and bubbles `BadRequest("unknown open_meta key '...'")`. Update the caller at `db_backend.rs:108` to `?`-propagate.
- [ ] **Step 3:** Run the three tests + full crate suite. Expect green.
- [ ] **Step 4:** Verify `backend/db_backend.rs::update_resource` still compiles after the signature change. The existing test at `backend/tests.rs::update_resource_*` continues to pass — those tests do not exercise the body branch (Task 8 of 3a left body-bearing updates as "3b will take over"; this task fulfills that).
- [ ] **Step 5:** Commit:
  ```
  feat(api): branch update translator on cmd shape; trio populated for body

  Removes the 3a stub. The translator now:
  - body present → prepare_body_trio populates (content, content_hash, chunks_packed)
  - meta present → validate_open_meta_keys before passing through
  - title/slug-only → unchanged

  No 7th trait method; meta path folds into update_resource per A3.
  ```

---

### Task 4 — Verify `resource_service::update` reconciles edges (else port from `meta_service::update_meta`)

**Why:** `meta_service::update_meta` ends with `super::edge_service::reconcile_edges` (line 301). If `resource_service::update` does NOT also reconcile edges, then folding the meta path through it (A3) silently regresses edge reconciliation for meta-only updates. Per `feedback_subagent_escalate_not_soften`, this must be verified before Task 11 lands; if missing, port the call into `resource_service::update` as a single-tx final step.

**Files:**
- Read: `crates/temper-api/src/services/resource_service.rs:527-820` (the full `update` body).
- Possibly modify: `crates/temper-api/src/services/resource_service.rs` (port reconcile_edges).

- [ ] **Step 1:** Read `resource_service::update` end-to-end. Grep for `reconcile_edges` inside the function. Result: either ✅ already calls it (no work) or ❌ missing.
- [ ] **Step 2 (if missing):** Port the reconcile_edges block from `meta_service.rs:298-318` into `resource_service::update`, after `tx.commit()`, gated on "meta touched" (i.e. `req.managed_meta.is_some() || req.open_meta.is_some()`). Use the same warn-and-continue error semantics.
- [ ] **Step 3 (if missing):** Write/extend an integration test in `crates/temper-api/tests/` that exercises the meta-only path through `resource_service::update` and asserts an edge declared in `open_meta.relates_to` materializes in `kb_edges`.
- [ ] **Step 4:** Run `cargo nextest run -p temper-api --features test-db --no-fail-fast`. Expect green.
- [ ] **Step 5:** Commit (only if Step 2 fired). Skip otherwise.

---

### Task 5 — Migrate `handlers::resources::create` (POST /api/resources)

**Why:** First write-handler migration. Routes a wire-shape `ResourceCreateRequest { kb_context_id, kb_doc_type_id, ... }` through the operations layer. Resolves IDs → names in the handler before constructing `CreateResource` (per A6).

**Files:**
- Modify: `crates/temper-api/src/handlers/resources.rs` (the `create` fn, `crates/temper-api/src/handlers/resources.rs:116-124`).
- Possibly modify: `crates/temper-api/src/services/context_service.rs` and a new `resolve_doc_type_by_id` if missing.

- [ ] **Step 1:** Grep for an existing `context_service::resolve_by_id` and `resolve_doc_type` (note: `ingest_service::resolve_doc_type` at line 174 takes a *name*, not an id). Add `resolve_by_id`-shape lookups in the appropriate service if missing.
- [ ] **Step 2 (RED):** Find the existing integration test for `POST /api/resources` (likely `crates/temper-api/tests/resources_create.rs` or similar). Run it pre-change to capture baseline. Don't edit it — passing unchanged is the regression guard.
- [ ] **Step 3 (GREEN):** Rewrite `handlers::resources::create`:
  ```rust
  pub async fn create(
      State(state): State<AppState>,
      auth: AuthUser,
      device_id: Option<Extension<DeviceId>>,
      Json(req): Json<ResourceCreateRequest>,
  ) -> ApiResult<Json<ResourceRow>> {
      let device_id = device_id.map(|d| d.0.0.clone()).unwrap_or_else(|| "api".into());
      // Resolve IDs to names for the operations command
      let context_name = context_service::resolve_by_id(&state.pool, req.kb_context_id).await?;
      let doc_type_name = resource_service::resolve_doc_type_by_id(&state.pool, req.kb_doc_type_id).await?;

      let cmd = CreateResource {
          context: context_name,
          doctype: doc_type_name,
          slug: req.slug.unwrap_or_default(),
          title: req.title,
          body: None,
          managed_meta: ManagedMeta::default(),
          open_meta: None,
          origin: Surface::ApiHttp,
      };
      let backend = DbBackend::new(state.pool.clone(), auth.0.profile.id.into(), device_id, Surface::ApiHttp);
      let out = backend.create_resource(cmd).await.map_err(ApiError::from)?;
      Ok(Json(out.value))
  }
  ```
  Drop the now-unused `use crate::services::resource_service::{..., ResourceCreateRequest, ...}` line if create was the only consumer (verify with grep).
- [ ] **Step 4:** Add `impl From<TemperError> for ApiError` (verify exists; if not, this is a separate prerequisite — flag as BLOCKED if missing). Phase 3a's `From<ApiError> for TemperError` is the inbound conversion; the handler needs the outbound conversion too. **Implementer must verify** before claiming this task done.
- [ ] **Step 5 (REFACTOR + verify):** `cargo make check` then `cargo nextest run -p temper-api --features test-db --no-fail-fast`. The pre-existing `POST /api/resources` integration tests must pass unchanged.
- [ ] **Step 6:** Commit:
  ```
  feat(api): migrate POST /api/resources handler through DbBackend

  Handler resolves kb_context_id → context_name and kb_doc_type_id →
  doc_type_name (the ResourceCreateRequest wire shape carries IDs;
  the operations CreateResource command carries names), constructs
  the command, and dispatches via DbBackend::create_resource which
  routes to ingest_service::ingest. POST /api/resources now
  exercises the full ingest pipeline (defaults, dedupe, identity
  injection); body-less callers are a no-op for the body branches.
  ```

---

### Task 6 — Migrate `handlers::resources::get` (GET /api/resources/{id})

**Why:** The only read-only handler that maps cleanly through `DbBackend` (returns `ResourceRow` on both sides). Read-path migration is low-risk and proves the show path.

**Files:**
- Modify: `crates/temper-api/src/handlers/resources.rs::get`.

- [ ] **Step 1 (GREEN):** Rewrite `get` to construct `ShowResource { resource: ResourceRef::Uuid { id: ResourceId::from(resource_id) }, origin: Surface::ApiHttp }`, build a `DbBackend` (device_id `"api"` since show is read-only and never writes audit), dispatch, return `Json(out.value)`.
- [ ] **Step 2:** Existing integration tests for `GET /api/resources/{id}` pass unchanged.
- [ ] **Step 3:** Commit:
  ```
  refactor(api): route GET /api/resources/{id} through DbBackend::show_resource
  ```

---

### Task 7 — Migrate `handlers::resources::update` (PATCH /api/resources/{id})

**Why:** This is the canonical update path. After this task, the handler-side body-trio guard at `handlers/resources.rs:149-158` is removed (its responsibility moves into the translator, which now PRODUCES the trio rather than rejecting absence).

**Files:**
- Modify: `crates/temper-api/src/handlers/resources.rs::update`.

- [ ] **Step 1:** Run baseline integration tests for `PATCH /api/resources/{id}` and capture results. Do NOT modify them — passing unchanged is the guard.
- [ ] **Step 2 (GREEN):** Rewrite `update`:
  - Drop the body-trio all-or-nothing validation (lines 149-158).
  - Translate `req: ResourceUpdateRequest` → `UpdateResource` cmd. Body present iff `req.content.is_some()`. (Note: the wire `ResourceUpdateRequest` allows caller to send a `content_hash` + `chunks_packed` — those fields are now ignored by the translator, which always recomputes. **Document this as an intentional contract tightening** in the commit message: clients send `content` only; server is the single source of truth for hash + chunks. This matches the spec's "server-side pipeline" model.)
  - Build `DbBackend`, dispatch `update_resource(cmd)`, return `Json(out.value)`.
- [ ] **Step 3:** Existing integration tests pass unchanged. Run `cargo nextest run -p temper-api --features test-db,ingest-pipeline --no-fail-fast` to ensure body-bearing tests exercise the new pipeline path.
- [ ] **Step 4:** Commit:
  ```
  feat(api): migrate PATCH /api/resources/{id} handler through DbBackend

  Drops the handler-side body-trio guard. The translator
  (prepare_body_trio) now produces (content_hash, chunks_packed)
  server-side from `content`; client-supplied hash/chunks are no
  longer load-bearing. Wire shape unchanged.
  ```

---

### Task 8 — Migrate `handlers::resources::delete` (DELETE /api/resources/{id})

**Why:** Smallest write-handler migration. Establishes the delete dispatch pattern.

**Files:**
- Modify: `crates/temper-api/src/handlers/resources.rs::delete`.

- [ ] **Step 1 (GREEN):** Rewrite to construct `DeleteResource { resource: ResourceRef::Uuid { id: ResourceId::from(resource_id) }, force: false, origin: Surface::ApiHttp }`, dispatch via `DbBackend`, return `Json(DeleteResponse { deleted: true })` (the wire response stays).
- [ ] **Step 2:** Existing integration tests pass unchanged.
- [ ] **Step 3:** Commit:
  ```
  refactor(api): route DELETE /api/resources/{id} through DbBackend::delete_resource
  ```

---

### Task 9 — Migrate `handlers::ingest::create` (POST /api/ingest)

**Why:** Canonical create path used by CLI cloud-mode pushes. Routes through the same `DbBackend::create_resource` as Task 5; the difference is the wire input shape (`IngestPayload` carries names directly; no ID→name resolve needed).

**Files:**
- Modify: `crates/temper-api/src/handlers/ingest.rs::create`.

- [ ] **Step 1 (GREEN):** Rewrite to translate `IngestPayload` → `CreateResource` cmd directly (names line up). Build `DbBackend`, dispatch, return `Json(out.value)`. Note: `IngestPayload.content_hash` and `chunks_packed` are now ignored on the create path (server recomputes via `ingest_service::ingest` — verify by reading `ingest_service.rs:428-466`).
- [ ] **Step 2:** Integration tests for `POST /api/ingest` pass unchanged. Run with `--features test-db,ingest-pipeline`.
- [ ] **Step 3:** **Workspace regression:** `cargo nextest run --workspace --no-fail-fast` (per `project_workspace_feature_unification_ort` — POST /api/ingest is the path temper-cloud's feature-unification check exercises).
- [ ] **Step 4:** Commit.

---

### Task 10 — Migrate `handlers::ingest::update` (PUT /api/ingest/{id})

**Why:** The other CLI-cloud sync write path. Body-bearing update — exercises the trio-population branch in the translator from Task 3.

**Files:**
- Modify: `crates/temper-api/src/handlers/ingest.rs::update`.

- [ ] **Step 1 (GREEN):** Translate `IngestPayload` → `UpdateResource` cmd. Body trio: cmd carries `body: Some(BodyUpdate { content: payload.content })`. Managed/open meta extracted from payload. Dispatch via `DbBackend::update_resource`. Return `Json(out.value)`.

  **Note** (BLOCKED-if-discovered): `ingest_service::update` (line 623, currently called by this handler) does meta-defaulting (`apply_defaults_value`) BEFORE the body-pipeline step. `resource_service::update` (the new dispatch destination) — verify it also calls `apply_defaults_value` for the meta branch. If not, per `feedback_subagent_escalate_not_soften`, escalate; do NOT silently drop default application.
- [ ] **Step 2:** Integration tests for `PUT /api/ingest/{id}` pass unchanged. Run with `--features test-db,ingest-pipeline`.
- [ ] **Step 3:** **E2E check** — `cargo nextest run --manifest-path tests/e2e/Cargo.toml --features test-db,test-embed --no-fail-fast`. CLI sync push exercises this exact path.
- [ ] **Step 4:** Commit.

---

### Task 11 — Migrate `handlers::meta::update_meta` (PUT /api/resources/{id}/meta)

**Why:** Folds the meta-only path into `DbBackend::update_resource` per A3. Wire-response shape changes from `{updated, resource_id}` to `ResourceRow` per A7 — sync caller discards the response, no breakage expected.

**Files:**
- Modify: `crates/temper-api/src/handlers/meta.rs::update_meta`.
- Modify: `crates/temper-api/src/handlers/meta.rs` OpenAPI annotation (`#[utoipa::path(...)]` response body type).

- [ ] **Step 1:** Read existing integration tests for `PUT /api/resources/{id}/meta`. They likely assert on the `{updated, resource_id}` JSON shape — those tests need to update to assert on `ResourceRow` shape (this is the rare "tests update because contract tightens" case allowed by `feedback_no_premature_backward_compat`; document why in the commit).
- [ ] **Step 2 (GREEN):** Rewrite `update_meta`:
  ```rust
  let cmd = UpdateResource {
      resource: ResourceRef::Uuid { id: ResourceId::from(resource_id) },
      body: None,
      managed_meta: Some(payload.managed_meta),
      open_meta: Some(payload.open_meta),
      origin: Surface::ApiHttp,
  };
  let backend = DbBackend::new(state.pool.clone(), auth.0.profile.id.into(), device_id, Surface::ApiHttp);
  let out = backend.update_resource(cmd).await.map_err(ApiError::from)?;
  Ok(Json(out.value))  // ResourceRow, NOT the old {updated, resource_id} shape
  ```
- [ ] **Step 3:** Update OpenAPI: `(status = 200, description = "Updated resource", body = ResourceRow)`.
- [ ] **Step 4:** Update integration tests to assert the new `ResourceRow` shape.
- [ ] **Step 5:** Run `cargo nextest run -p temper-api --features test-db --no-fail-fast`. Then `cargo nextest run --manifest-path tests/e2e/Cargo.toml --features test-db,test-embed --no-fail-fast` (CLI sync's meta-only branch hits this path).
- [ ] **Step 6:** Commit:
  ```
  feat(api): migrate PUT /api/resources/{id}/meta through DbBackend; tighten response shape

  Folds the meta-only update path into DbBackend::update_resource (A3
  in the 3b plan). The translator branches on cmd shape: when only
  managed/open_meta present, it validates open keys via
  validate_open_meta_keys and dispatches to resource_service::update
  meta-only.

  Wire response shape changes from {updated:bool, resource_id:uuid}
  to ResourceRow. The sole CLI caller (sync.rs) discards the response;
  no client breakage. Per feedback_no_premature_backward_compat, no
  compat shim.
  ```

---

### Task 12 — Retire `resource_service::create` (conditional on zero-callers)

**Why:** After Task 5, `resource_service::create` should have zero callers. Delete it per `feedback_no_premature_backward_compat`.

**Files:**
- Modify: `crates/temper-api/src/services/resource_service.rs` (delete `create` fn at line 485).
- Modify: any test in `crates/temper-api/tests/` that imports it.

- [ ] **Step 1 (verification gate):** `grep -rn "resource_service::create\b\|\.create(\b" crates/ | grep -v "// "` — confirm only the resource_service self-test references remain (if any). MCP tools (3c not yet landed) may still reference; if so, **defer this task to 3c**'s final sweep (document deferral in plan progress notes; do not delete).
- [ ] **Step 2 (delete):** If zero external callers, delete the `pub async fn create` block at `resource_service.rs:484-510`.
- [ ] **Step 3:** `cargo make check` — expect clean (no dangling import errors).
- [ ] **Step 4:** `cargo nextest run -p temper-api --features test-db --no-fail-fast` — expect green.
- [ ] **Step 5:** Commit:
  ```
  refactor(api): retire resource_service::create after handler migration

  POST /api/resources now dispatches through DbBackend::create_resource
  (Task 5). resource_service::create has zero callers; delete it per
  feedback_no_premature_backward_compat.
  ```

---

### Task 13 — Retire `ingest_service::update` (conditional on zero-callers)

**Files:**
- Modify: `crates/temper-api/src/services/ingest_service.rs` (delete `update` fn at line 623).

- [ ] **Step 1 (verification gate):** `grep -rn "ingest_service::update\b" crates/`. After Task 10, expect callers only inside MCP (`crates/temper-mcp/src/tools/resources.rs::update_resource` at line 440). **Defer to 3c if MCP still calls in.**
- [ ] **Step 2 (delete):** If zero external callers, delete the `pub async fn update` block at `ingest_service.rs:615-770` (verify exact end line during execution).
- [ ] **Step 3:** `cargo make check` + `cargo nextest run -p temper-api --features test-db --no-fail-fast` + `cargo nextest run --manifest-path tests/e2e/Cargo.toml --features test-db,test-embed --no-fail-fast`.
- [ ] **Step 4:** Commit.

---

### Task 14 — Retire `meta_service::update_meta` (conditional on zero-callers)

**Files:**
- Modify: `crates/temper-api/src/services/meta_service.rs` (delete `update_meta` fn at line 87).

- [ ] **Step 1 (verification gate):** `grep -rn "meta_service::update_meta\b\|update_resource_meta\b" crates/`. After Task 11, the HTTP handler no longer calls in. MCP `update_resource_meta` (tools/resources.rs:559) likely does — **defer to 3c**.
- [ ] **Step 2 (delete):** If zero external callers, delete the function block. Note: keep `meta_service::get_meta` — `GET /api/resources/{id}/meta` is out of 3b scope (read-only).
- [ ] **Step 3:** `cargo make check` + full crate suite.
- [ ] **Step 4:** Commit.

---

### Task 15 — Final regression sweep

**Why:** Lock in the migration. Each individual task ran its own `--features test-db` regression; this task adds the embed-gated and workspace passes that catch cross-crate feature-unification issues (`project_workspace_feature_unification_ort`).

- [ ] **Step 1:** `cargo make check` — expect clean.
- [ ] **Step 2:** `cargo nextest run --workspace --all-features --no-fail-fast` — exit 0; `grep -E 'error: test run failed|FAIL \[' nextest.log` returns nothing (per `feedback_nextest_summary_lies`).
- [ ] **Step 3:** `cargo nextest run -p temper-api --features test-db --no-fail-fast`.
- [ ] **Step 4:** `cargo nextest run --manifest-path tests/e2e/Cargo.toml --features test-db,test-embed --no-fail-fast` — matches CI's Embed job. **Document this run explicitly in the final commit message** so reviewers can confirm.
- [ ] **Step 5:** `cargo nextest run --workspace --no-fail-fast` (without `--all-features`) — catches the temper-cloud feature-unification surface that workspace-wide flag activation hides.
- [ ] **Step 6:** Verify no `temper_core::defaults::*` direct imports introduced (`grep -rn "temper_core::defaults" crates/temper-api/src/`).
- [ ] **Step 7:** Verify `temper-cli` still has no dep on `temper-api`: `cargo metadata --format-version 1 -q | jq -r '.packages[] | select(.name=="temper-cli") | .dependencies[].name' | sort | grep -c temper-api` returns 0.
- [ ] **Step 8:** Mark the 3b backlog task (`temper resource list --type task --stage in-progress` to find it) as `done`.
- [ ] **Step 9:** Final commit (only if any verification turned up minor cleanups; otherwise this is a no-op).

---

## Acceptance Criteria

- [ ] `validate_open_meta_keys` lives in `temper-core::operations::actions` (Task 1).
- [ ] `prepare_body_trio` exists at `temper-api/src/backend/translators.rs` (or a sibling) and is gated on `ingest-pipeline`; preserves the BadRequest contract on the non-pipeline path (Task 2).
- [ ] `update_resource_to_request` translator returns `Result<ResourceUpdateRequest, TemperError>` and branches on cmd shape (body / meta / title-or-slug); body branch populates the trio; meta branch validates open keys (Task 3).
- [ ] `resource_service::update` reconciles edges for the meta branch (Task 4 verification — port if absent).
- [ ] HTTP handlers `resources::create`, `resources::get`, `resources::update`, `resources::delete`, `ingest::create`, `ingest::update`, `meta::update_meta` all dispatch through `DbBackend` (Tasks 5–11).
- [ ] HTTP handlers `resources::list`, `resources::by_uri`, `resources::get_content`, `meta::get_meta`, `search::search` remain service-direct (deviation from user's stated scope, justified in §A5).
- [ ] `resource_service::create`, `ingest_service::update`, `meta_service::update_meta` deleted (Tasks 12–14) OR each task's deletion is explicitly deferred to 3c with a written note in plan progress.
- [ ] All HTTP integration tests in `crates/temper-api/tests/*.rs` pass; the only test edits are in Task 11 to update the `PUT /api/resources/{id}/meta` response-shape assertions.
- [ ] All E2E tests with `--features test-db,test-embed` pass.
- [ ] `cargo nextest run --workspace --no-fail-fast` exits 0.
- [ ] No new `temper_core::defaults::*` direct imports in `temper-api`.
- [ ] `temper-cli` still has no dep on `temper-api`.

---

## Risks & Open Questions

1. **`From<TemperError> for ApiError` may not exist.** Phase 3a added `From<ApiError> for TemperError` (the inbound conversion for DbBackend's return path). Outbound — handler converting `TemperError` from `DbBackend` back to the `ApiResult = Result<_, ApiError>` HTTP shape — needs a matching impl. Task 5 Step 4 flags verification; if missing, it's a one-task prerequisite to land before Task 5 (mechanical mirror of Phase 3a's Task 2).

2. **`resource_service::update` defaulting parity with `ingest_service::update`.** Task 10's BLOCKED-if-discovered note flags this. `ingest_service::update` calls `apply_defaults_value` (line 660); `resource_service::update` may or may not. If not, the meta branch silently regresses default application for meta-only updates. Task 4 partially mitigates (edge reconciliation parity); Task 10 must explicitly verify defaults parity too.

3. **Wire-contract change in `PUT /api/resources/{id}/meta` response (A7).** No client today pattern-matches on the `{updated, resource_id}` shape (CLI sync discards). External integrators (MCP, future UI) — if any — would need to update. Mitigation: Task 11 commit message explicitly flags the change; OpenAPI is updated.

4. **MCP cross-branch dependency for retirement Tasks 12–14.** 3c lands on the same branch but in a later session. Each retirement task gates on a `grep` for zero callers; MCP-internal callers force deferral to 3c's final task. The 3c plan-writer should expect to inherit Tasks 12–14 if they didn't fire in 3b.

5. **List/by-uri/get-content/search not migrated (A5 deviation).** The user's scope said all four `resources` GET handlers + search migrate. The verified wire-shape mismatches (list returns `{rows, total, facets}`; search returns `Vec<UnifiedSearchResultRow>`; get_content returns `ContentResponse`) make trivial migration impossible without either (a) lossy projection at the handler boundary (wire change visible to clients) or (b) extending the trait with HTTP-shaped types (re-coupling). The plan keeps them service-direct. **Open question for the user:** is this deviation accepted? If not, Phase 3b expands and we either widen the trait (rejecting the "fold meta into update" simplification) or accept lossy projection (wire change). Default per this plan: accept service-direct.

6. **`validate_open_meta_keys` on the meta branch may double-validate.** `resource_service::update` may already validate (verify in Task 4). If yes, the translator's pre-validation is redundant but harmless; if no, the translator's pre-validation is the only line of defense. Either way, behavior is preserved.

---

### Self-review notes (plan-writer ran `feedback_plan_verification` against named APIs)

- `KNOWN_OPEN_FIELDS` already in `temper-core::frontmatter::registry:45` — corrected from user's "lift both" instruction (only the fn moves).
- `resource_service::create` at line 485 — verified.
- `ingest_service::update` at line 623 — verified.
- `meta_service::update_meta` at line 87 — verified.
- `prepare_body_trio` source body at `ingest_service.rs:660-674` — verified incl. `apply_defaults_value` at line 660 (so meta-defaulting on the body branch happens INSIDE `ingest_service::update`; if Task 10 routes to `resource_service::update` instead, the defaulting parity check in Risk #2 is critical).
- `routes.rs:48-75` — all 10 routes verified; the user's inventory missed `meta::get_meta` (read) and the `resources/{id}/edges` route (handled in `handlers::edges`, not in scope).
- `temper-client/src/resources.rs:139` `update_meta` returns `Result<serde_json::Value>` — caller discards (verified at `temper-cli/src/actions/sync.rs:997-999`). Wire change in A7 is safe.
- `ResourceCreateRequest` carries IDs (`kb_context_id`, `kb_doc_type_id`); `IngestPayload` and `CreateResource` cmd carry names — A6 resolution required.
- Branch name `jct/wave1-phase3bc-handler-mcp-migration` matches user's stated convention (single branch for 3b + 3c).

### Critical Files for Implementation
- /Users/petetaylor/projects/tasker-systems/temper/crates/temper-api/src/backend/translators.rs
- /Users/petetaylor/projects/tasker-systems/temper/crates/temper-api/src/backend/db_backend.rs
- /Users/petetaylor/projects/tasker-systems/temper/crates/temper-api/src/handlers/resources.rs
- /Users/petetaylor/projects/tasker-systems/temper/crates/temper-api/src/handlers/ingest.rs
- /Users/petetaylor/projects/tasker-systems/temper/crates/temper-api/src/handlers/meta.rs
- /Users/petetaylor/projects/tasker-systems/temper/crates/temper-core/src/operations/actions.rs
- /Users/petetaylor/projects/tasker-systems/temper/crates/temper-api/src/services/meta_service.rs
- /Users/petetaylor/projects/tasker-systems/temper/crates/temper-api/src/services/resource_service.rs
- /Users/petetaylor/projects/tasker-systems/temper/crates/temper-api/src/services/ingest_service.rs