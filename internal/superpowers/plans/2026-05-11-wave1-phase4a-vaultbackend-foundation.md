# Wave 1 Phase 4a — VaultBackend Foundation Plan

**Date:** 2026-05-11
**Context:** `temper`
**Mode:** build
**Effort:** medium (foundation; dark-launched, no callers rewired)
**Branch:** `jct/wave1-phase4a-vaultbackend-foundation`

**Spec:** `docs/superpowers/specs/2026-05-11-wave1-phase4-vaultbackend-design.md`
**Parent spec:** `docs/superpowers/specs/2026-05-01-shared-core-execution-paths-design.md` (#4)
**Predecessors merged:** PR #65 (Phases 1+2), PR #69 (Phase 3a), PR #71 (3b+3c), PR #74 (4-prep), PR #75 (owner threading), PR #76 (cloud-only sync fix)

---

## Goal

Land the `VaultBackend` foundation — `crates/temper-cli/src/vault_backend/`
with `VaultBackend` struct, `VaultBackendCtx` builder, full `impl Backend`
covering all 6 trait methods, and trait-impl unit tests against a tmp
vault. Dark-launched: no `commands/*.rs` callers rewired. The 4b plan
(written when that session begins) rewires `commands/resource.rs`'s
Local-mode arm to dispatch through `VaultBackend`.

## Non-Goals (call out so they don't creep in)

- Rewiring any `commands/*.rs` caller — that's 4b.
- Touching `commands/resource.rs`'s `match VaultState` branch — that's 4b
  (Local arm) and Phase 5 (collapse).
- Touching `actions::sync.rs` (4369 lines) — sync orchestration stays
  put.
- Local search reinstatement — `search_resources` is a thin passthrough
  to `client.search().search`.
- Per-doctype full pull-in — 4a delegates to existing creators
  (`actions::task::create`, `commands::goal::create`, etc.).
- Body-pipeline lift to `temper-core` — examined in Task 2; lifted if
  feasible, duplicated with cleanup task otherwise.

## Conventions for every task

Implementer subagents (per `feedback_prefer_subagent`,
`feedback_plan_code_quality`, `feedback_subagent_check_before_commit`):

- **TDD.** Write the test first. Confirm it fails for the right reason.
  Implement. Confirm it passes.
- **Verify named APIs before dispatch.** Every API name in this plan is
  a hypothesis until grep-confirmed at task time. The code is ground
  truth.
- **`cargo make check` before claiming complete.** Pre-commit hook is
  the backstop, not the first line of defense.
- **Pair filter-by-name runs with full-crate runs before commit.** A
  passing single test does not imply a passing crate.
- **No `#[allow(...)]` for clippy.** Use `#[expect(name, reason = "...")]`
  with a real reason, or fix the underlying issue.
- **Workspace-feature-unification awareness.** `cargo nextest --workspace`
  activates `ingest-pipeline` via `temper-cloud`'s feature graph —
  exercises code paths the standalone crate run won't (see
  `feedback_workspace_test_surfaces_pipeline_bugs`). Include workspace
  runs in regression-guard verification.
- **Don't trust nextest's per-binary `Summary` line.** Trust exit code
  or grep for `error: test run failed` / `FAIL [`
  (`feedback_nextest_summary_lies`).
- **Escalate, don't soften.** If a test requires loosening a contract,
  STOP and report BLOCKED.
- **No "for now" workarounds.** If a real semantic surfaces, capture
  it as a task — don't ship a TODO comment.
- **Shared types live in `temper-core`.** Don't define `BodyTrio`
  twice.

Code quality (per `feedback_plan_code_quality` and CLAUDE.md):
- Service layer / operations layer owns the SQL / file IO; backends
  orchestrate but don't inline. For VaultBackend the analogue is:
  shared `operations::actions::*` for validation/defaults/merge,
  `temper-core::vault::Vault` for path construction,
  `temper-core::frontmatter::Frontmatter` for parse/serialize,
  `crate::manifest_io::*` for manifest IO. The backend method bodies
  compose these — no inline `std::fs::*` calls outside the
  `vault_backend/` module.
- Typed structs over `serde_json::json!()`. `BodyTrio { hash, packed }`
  is its own struct, not a tuple.
- Params structs at 5+ args. `VaultBackendCtx` is mandatory from day 1.

## Tasks

Tasks are sequenced; each is one commit on `jct/wave1-phase4a-vaultbackend-foundation`.
Tasks 1-4 set up structure; Tasks 5-10 implement the 6 trait methods;
Tasks 11-13 close out the foundation; Task 14 is the final verification
and code review.

---

### Task 1 — Scaffolding: empty `vault_backend/` module

**Owner:** subagent (haiku is enough)

**Goal.** Add the module structure with empty bodies so the crate
compiles. No behavior. Establishes import paths.

**Files to create:**
- `crates/temper-cli/src/vault_backend/mod.rs`
- `crates/temper-cli/src/vault_backend/vault_backend.rs`
- `crates/temper-cli/src/vault_backend/translators.rs`
- `crates/temper-cli/src/vault_backend/per_doctype.rs`
- `crates/temper-cli/src/vault_backend/tests.rs` (gated under
  `#[cfg(all(test, feature = "test-db"))]` mirroring Phase 3a)

**Files to edit:**
- `crates/temper-cli/src/lib.rs`: add `pub mod vault_backend;` (or
  `mod vault_backend;` if no external re-exports are needed; check
  what `cli.rs` and `main.rs` use today).

**Contents:**
- `mod.rs`: module doc comment mirroring `temper-api/src/backend/mod.rs`,
  `mod vault_backend; mod translators; mod per_doctype;
  #[cfg(all(test, feature = "test-db"))] mod tests;
  pub use vault_backend::{VaultBackend, VaultBackendCtx};`.
- `vault_backend.rs`: file-level doc comment, no struct yet (next task).
- Others: file-level doc comment only.

**Test.** None — this task only verifies the module skeleton compiles.

**Verification.**
- `cargo build -p temper-cli` succeeds.
- `cargo make check` clean.

**Commit message.** `wave1-4a task 1: scaffold vault_backend/ module`

---

### Task 2 — `VaultBackend` struct + `VaultBackendCtx` builder + `BodyTrio` placement decision

**Owner:** subagent (sonnet)

**Goal.** Define the struct + builder. Make the body-trio placement
decision (lift to `temper-core` if viable; else duplicate with
cleanup task). Add the `prepare_body_trio` helper either via lift or
duplication.

**Files to edit / create:**
- `crates/temper-cli/src/vault_backend/vault_backend.rs`: add the
  `VaultBackend` struct + `VaultBackendCtx` struct + `impl VaultBackend
  { pub fn new(ctx: VaultBackendCtx) -> Self; pub(crate) fn <getters> }`.
  Mirror Phase 3a's `db_backend.rs` shape exactly where applicable.
- `crates/temper-cli/src/vault_backend/translators.rs`: add `BodyTrio
  { pub content_hash: String, pub chunks_packed: String }` (struct
  not tuple) and either:
  - **(a) Lift path:** `temper-core::operations::body` module hosts
    `prepare_body_trio(body: &str) -> Result<BodyTrio, TemperError>`
    feature-gated on `ingest-pipeline`. Re-exported through
    `operations/mod.rs`. Both DbBackend translators and vault_backend
    translators import from there. Update
    `temper-api/src/backend/translators.rs` to call the new shared
    function; delete the local copy. **Acceptance gate:** does
    `temper-ingest` already appear in `temper-core`'s `Cargo.toml`
    feature graph? If not, the lift adds a new feature-gated dep and
    is a larger change — defer the lift.
  - **(b) Duplicate path:** add `prepare_body_trio` (private to
    `vault_backend/translators.rs`) with identical signature and
    body to the DbBackend copy. Create a backlog task immediately:
    `lift-prepare-body-trio-to-temper-core-shared-helper`.

**Decision pre-work (do this in the implementer subagent prompt):**
> Examine `crates/temper-core/Cargo.toml` and the `[features]` block
> in `crates/temper-api/Cargo.toml`. Determine whether moving
> `prepare_body_trio` into `temper-core::operations::body` requires
> adding `temper-ingest` as a feature-gated dep of `temper-core`. If
> `temper-core` already has feature-gated deps and the pattern is
> established, lift. Otherwise, duplicate + capture cleanup task.

**Test (unit, in `vault_backend.rs`):**
- `vault_backend_new_constructs_from_ctx` — build a `VaultBackendCtx`
  with a tmp vault root, an empty `Manifest`, `client: None`,
  `owner: OwnerHandle::me()`, `surface: Surface::CliLocalVault`, call
  `VaultBackend::new(ctx)`, assert the struct can be queried via
  getters.

**Verification.**
- `cargo test -p temper-cli vault_backend_new` passes.
- `cargo make check` clean.

**Commit message.** `wave1-4a task 2: VaultBackend struct + VaultBackendCtx + BodyTrio placement`

---

### Task 3 — `resolve_resource_ref` translator (no I/O against client, only manifest + vault)

**Owner:** subagent (sonnet)

**Goal.** Add the pure translator that resolves a `ResourceRef` to
`(ResourceId, PathBuf)` using manifest reverse-index for `Uuid` and
`lookup::find_resource` for `Scoped`. This is the local-side mirror
of Phase 3a's `translators::resolve_resource_ref` (which queries
SQL).

**Files to edit:**
- `crates/temper-cli/src/vault_backend/translators.rs`: add
  ```rust
  pub(crate) struct ResolvedResource {
      pub resource_id: ResourceId,
      pub path: PathBuf,
  }

  pub(crate) fn resolve_resource_ref(
      vault_root: &Path,
      manifest: &Manifest,
      owner: &str,
      config: &Config,  // for lookup::find_resource fallback
      rref: &ResourceRef,
  ) -> Result<ResolvedResource, TemperError>;
  ```
  Branches:
  - `Uuid { id }`: look up `manifest.entries.get(&id)`. If present,
    return `(id, vault_root.join(&entry.path))`. If absent, return
    `TemperError::NotFound(...)` — the caller can decide whether to
    fall back to API for show paths.
  - `Scoped { owner, context, doctype, slug }`: delegate to
    `lookup::find_resource(FindableResource { config, manifest:
    Some(manifest), owner: Some(owner.to_string()), context:
    Some(context.clone()), doc_type: DocType::from_str(&doctype)?,
    slug_or_suffix: slug.clone() })`. Project the result into
    `ResolvedResource`.

**Tests (unit, in `translators.rs`):**
- `resolve_uuid_hits_manifest_entry` — seed a manifest with one
  entry, call with `ResourceRef::Uuid { id }`, assert path returned.
- `resolve_uuid_missing_entry_returns_not_found` — empty manifest;
  assert `Err(NotFound)`.
- `resolve_scoped_delegates_to_find_resource` — set up a tmp vault
  with one `.md` file under `@me/temper/task/`, call with
  `ResourceRef::Scoped`, assert path resolves.

**Verification.**
- `cargo nextest run -p temper-cli vault_backend::translators` clean.
- `cargo make check` clean.

**Commit message.** `wave1-4a task 3: resolve_resource_ref translator + tests`

---

### Task 4 — `cmd_to_ingest_payload` + `cmd_to_update_request` translators

**Owner:** subagent (sonnet)

**Goal.** Pure cmd → wire-type adapters for the push-as-tail-action path.
These convert a `CreateResource` or `UpdateResource` (operations layer)
into `IngestPayload` / `ResourceUpdateRequest` (wire types). Both
adapters take a pre-computed `Option<BodyTrio>` so the caller decides
when to run the pipeline.

**Files to edit:**
- `crates/temper-cli/src/vault_backend/translators.rs`:
  ```rust
  pub(crate) fn cmd_to_ingest_payload(
      cmd: &CreateResource,
      body: &str,
      body_trio: Option<&BodyTrio>,
  ) -> IngestPayload;

  pub(crate) fn cmd_to_update_request(
      cmd: &UpdateResource,
      body_trio: Option<&BodyTrio>,
  ) -> Result<ResourceUpdateRequest, TemperError>;
  ```
  - `cmd_to_ingest_payload` mirrors
    `temper-api/src/backend/translators.rs::create_resource_to_ingest_payload`
    line-for-line, taking the `body` as a separate `&str` since the
    cmd's `body: Option<BodyUpdate>` carries it.
  - `cmd_to_update_request` mirrors the DbBackend translator,
    validating `open_meta` keys via
    `temper_core::operations::validate_open_meta_keys`, populating the
    body trio from `body_trio` when present, leaving them None
    otherwise.

**Tests (unit, in `translators.rs`):**
- `cmd_to_ingest_payload_carries_managed_meta_and_body`
- `cmd_to_ingest_payload_empty_body_when_no_body_update`
- `cmd_to_update_request_meta_only_branch_leaves_body_fields_none`
- `cmd_to_update_request_rejects_unknown_open_meta_key`
- `cmd_to_update_request_body_branch_populates_trio`

Mirror Phase 3a's tests in
`temper-api/src/backend/translators.rs:#[cfg(test)] mod tests` —
those have the right shape.

**Verification.**
- `cargo nextest run -p temper-cli vault_backend::translators` clean.
- `cargo make check` clean.

**Commit message.** `wave1-4a task 4: cmd_to_ingest_payload + cmd_to_update_request translators`

---

### Task 5 — `Backend::show_resource` (read; no manifest mutation)

**Owner:** subagent (sonnet)

**Goal.** First trait method. Read-only; safest place to land the
trait-impl pattern. Branches on `ResourceRef`; on `LocallyMissing`,
falls back to API via `client.resources().content`.

**Implementation:**
```rust
#[async_trait]
impl Backend for VaultBackend {
    async fn show_resource(
        &self,
        cmd: ShowResource,
    ) -> Result<CommandOutput<ResourceRow>, TemperError> {
        let manifest = self.manifest.lock().await;
        let resolved = translators::resolve_resource_ref(
            &self.vault_root, &*manifest, self.owner.as_str(), &self.config, &cmd.resource,
        );
        drop(manifest);  // release before any I/O

        match resolved {
            Ok(r) if r.path.exists() => {
                let fm = temper_core::frontmatter::Frontmatter::parse_file(&r.path)?;
                let row = vault_file_to_resource_row(&r.path, &fm, r.resource_id);
                Ok(CommandOutput::new(row))
            }
            // LocallyMissing or NotFound → API fallback when client available.
            _ if self.client.is_some() => {
                // Existing show_via_api_fallback logic, lifted/adapted.
                let row = self.fallback_show_via_api(&cmd.resource).await?;
                Ok(CommandOutput::new(row))
            }
            Ok(_) => Err(TemperError::NotFound("local file missing, no client".into())),
            Err(e) => Err(e),
        }
    }
    // ... other methods stubbed to unimplemented!() for now
}
```

**Helpers added in this task:**
- `vault_file_to_resource_row(path, fm, id) -> ResourceRow` — projects
  a parsed frontmatter into a `ResourceRow`. Inline as `fn` in
  `vault_backend.rs` for now; lift to `translators.rs` only if reused.
- `VaultBackend::fallback_show_via_api(&self, rref: &ResourceRef) ->
  Result<ResourceRow, TemperError>` — wraps the existing logic at
  `commands/resource.rs:947` (`show_via_api_fallback`). Lift the
  body verbatim into a `VaultBackend` method; the function in
  `commands/resource.rs` stays but its callers will move to
  `VaultBackend` in 4b.

**Config threading note.** `VaultBackend::resolve_resource_ref` needs
a `&Config` for the `lookup::find_resource` fallback. Choices:
- (a) Add `config: Arc<Config>` to `VaultBackendCtx` from day 1.
- (b) Pass `config` as a method argument to `show_resource`.

Decision: **(a)**. The trait method signature stays untouched and
matching `DbBackend`'s shape; the cost is one more `Arc` field. The
spec already accepts that 5 fields = at-threshold; 6 fields is one
over. Update `VaultBackendCtx` (Task 2 retro) — add `config:
Arc<Config>` and accept the params-struct overage. Document in the
struct doc comment: "fields above the project's 5-param threshold;
already using a params struct."

**Tests (in `tests.rs`, gated on `test-db`):**
- `show_resource_uuid_returns_resource_row` — seed a tmp vault with
  one task, build a manifest entry, build `VaultBackend`, call
  `show_resource` with `ResourceRef::Uuid`, assert the row matches.
- `show_resource_scoped_returns_resource_row` — same, but via
  `Scoped` ref.
- `show_resource_locally_missing_falls_back_to_client` — mock client
  (use the existing test-fixture client if available, else skip and
  add a TODO test); assert the API path runs.
- `show_resource_not_found_no_client_returns_error` — empty vault,
  no client; assert `NotFound`.

**Verification.**
- `cargo nextest run -p temper-cli vault_backend::tests::show_resource --features test-db`
- `cargo make check` clean.

**Commit message.** `wave1-4a task 5: VaultBackend::show_resource impl + tests`

---

### Task 6 — `Backend::list_resources` + `Backend::search_resources`

**Owner:** subagent (sonnet)

**Goal.** Both read-only. List walks the filesystem under the
backend's `vault_root`. Search is a passthrough to the client (per
`project_offline_indexing_dropped`).

**Implementation:**
- `list_resources`: lift the existing `scan_rows` / `filter_rows` /
  `sort_rows` helpers (in `commands/resource.rs:394-451`) into
  `vault_backend/list.rs` as `pub(crate) fn` so they're testable.
  In `Backend::list_resources` impl, call them and project
  `ResourceRow` → `ResourceSummary`. The lift is mechanical — the
  helpers take `&Config` today; keep that signature.
- `search_resources`: when `self.client.is_some()`, call
  `client.search().search(&params)` (verify exact API name and
  param-builder signature at task time). When `None`, return
  `Err(TemperError::BadRequest("search requires authenticated client"))`.

**Tests (in `tests.rs`):**
- `list_resources_filters_by_context_and_doctype`
- `list_resources_respects_limit`
- `list_resources_empty_dir_returns_empty_vec`
- `search_resources_no_client_returns_bad_request`
- `search_resources_with_mock_client_passes_query_through` — best-effort;
  if the test fixture is awkward, capture as follow-up and ship the
  no-client test only.

**Verification.**
- `cargo nextest run -p temper-cli vault_backend::tests::list_resources --features test-db`
- `cargo nextest run -p temper-cli vault_backend::tests::search_resources --features test-db`
- `cargo make check` clean.

**Commit message.** `wave1-4a task 6: VaultBackend::list_resources + search_resources impls + tests`

---

### Task 7 — `Backend::create_resource` (file write + manifest insert)

**Owner:** subagent (sonnet); dedicated reviewer (sonnet) before commit

**Goal.** The most consequential trait method. Composes shared
`operations::actions::*` + per-doctype dispatch + manifest insert +
push-as-tail-action.

**Implementation outline:**
```rust
async fn create_resource(
    &self,
    cmd: CreateResource,
) -> Result<CommandOutput<ResourceRow>, TemperError> {
    use temper_core::operations::actions::{
        validate_create, apply_defaults_value, ensure_managed_identity_keys,
    };

    // 1. Validate (shared)
    validate_create(&cmd)?;

    // 2. Apply doctype defaults (shared) — works on Value form
    let mut managed_value = serde_json::to_value(&cmd.managed_meta)?;
    apply_defaults_value(&cmd.doctype, &mut managed_value);
    ensure_managed_identity_keys(&mut managed_value, &cmd.title, Some(&cmd.slug));

    // 3. Per-doctype file-write dispatch
    let body_str = cmd.body.as_ref().map(|b| b.content.as_str()).unwrap_or("");
    let written = per_doctype::write_for(per_doctype::WriteArgs {
        doctype: &cmd.doctype,
        title: &cmd.title,
        slug: &cmd.slug,
        context: &cmd.context,
        body: body_str,
        managed_value: &managed_value,
        open_meta: cmd.open_meta.as_ref(),
        vault_root: &self.vault_root,
        owner: self.owner.as_str(),
    })?;

    let mut events = vec![DomainEvent::VaultFileWritten {
        path: written.rel_path.clone(),
    }];

    // 4. Manifest entry insert (Provisional until push confirms)
    let mut manifest = self.manifest.lock().await;
    let entry = ManifestEntry { /* provisional, with body_hash from
        prepare_body_trio when body is non-empty */ };
    manifest.entries.insert(written.resource_id, entry);
    manifest_io::save_manifest(&self.config.state_dir, &manifest)?;
    events.push(DomainEvent::VaultManifestUpdated {
        path: written.rel_path.clone(),
    });
    drop(manifest);

    // 5. Push as tail action (if client present)
    let push_event = match &self.client {
        None => DomainEvent::PushDeferred { reason: PushDeferReason::Offline },
        Some(client) => {
            let body_trio = if !body_str.is_empty() {
                Some(translators::prepare_body_trio(body_str)?)
            } else {
                None
            };
            let payload = translators::cmd_to_ingest_payload(&cmd, body_str, body_trio.as_ref());
            match client.ingest().create(&payload).await {
                Ok(row) => DomainEvent::RemoteSynced { resource_id: row.id },
                Err(e) if is_offline(&e) => DomainEvent::PushDeferred { reason: PushDeferReason::Offline },
                Err(e) if is_auth_error(&e) => DomainEvent::PushDeferred { reason: PushDeferReason::NotAuthed },
                Err(_) => DomainEvent::PushDeferred { reason: PushDeferReason::Other },
            }
        }
    };
    events.push(push_event);

    // 6. Project file → ResourceRow (read-back from disk to confirm
    //    the write is what it claims to be)
    let fm = temper_core::frontmatter::Frontmatter::parse_file(&written.abs_path)?;
    let row = vault_file_to_resource_row(&written.abs_path, &fm, written.resource_id);
    Ok(CommandOutput::with_events(row, events))
}
```

**`per_doctype::write_for`** — central dispatch. New module
`vault_backend/per_doctype.rs`. Function takes `WriteArgs` and
matches on `doctype`:
- `"concept" | "decision"`: inline the existing `create_simple_resource`
  logic from `commands/resource.rs:209-321`. (Lift verbatim; keep the
  function pure of `Config` if possible — it currently takes a
  `&Config` for `vault_root` and `state_dir`; pass those individually
  in `WriteArgs`.)
- `"task" | "goal" | "session" | "research"`: **delegate** to the
  existing creators (`actions::task::create`, `commands::goal::create`,
  `commands::session::save`, `commands::research::save`). Each takes
  a `&Config` — pass through. They write the file and return
  `Result<String>` (slug) or similar; capture the resource_id and
  rel_path post-hoc by reading back the manifest entry the existing
  creator's `publish_local_write_best_effort` populated, **or** by
  re-deriving via `vault_layout.rel_path(...)`.
  - **Audit gate.** Implementer verifies each existing creator's
    actual return shape and stomach for re-entry from VaultBackend.
    If any creator is too entangled (e.g., reads `VaultState::from_env()`
    internally and would loop), capture as a 4b dependency and ship
    Task 7 with concept/decision only — push the doctype-dispatch
    completeness into a follow-up task. Per `feedback_subagent_escalate_not_soften`.

**Tests (in `tests.rs`):**
- `create_resource_concept_writes_file_and_manifest_entry`
- `create_resource_with_no_client_emits_push_deferred_offline`
- `create_resource_with_client_emits_remote_synced_on_success`
- `create_resource_with_client_emits_push_deferred_on_network_error`
- `create_resource_validate_create_rejects_empty_title` — confirms the
  shared `validate_create` is wired in
- `create_resource_applies_doctype_defaults_at_write_time` — write a
  task with no `temper-mode`/`temper-effort`, assert defaults are in
  the on-disk frontmatter
- `create_resource_invokes_ensure_managed_identity_keys` — assert
  `temper-title` and `temper-slug` are in the on-disk frontmatter

**Verification.**
- `cargo nextest run -p temper-cli vault_backend::tests::create_resource --features test-db`
- `cargo nextest run -p temper-cli vault_backend::tests::create_resource --features test-db,test-embed`
  (when the body-bearing branch runs the pipeline)
- `cargo make check` clean.

**Commit message.** `wave1-4a task 7: VaultBackend::create_resource impl + tests`

---

### Task 8 — `Backend::update_resource`

**Owner:** subagent (sonnet); dedicated reviewer (sonnet) before commit

**Goal.** Mirror of `create_resource` for the update path. Includes
scalar field updates, array appends, body update, `--context-to` /
`--type-to` move (carried via the `cmd.managed_meta.context` and
synthetic mechanism — verify at task time how this is currently
plumbed; may need a `MoveSpec` field on `UpdateResource` or a
follow-up extraction since the move semantics today are clap-only).

**Decision pre-work in subagent prompt:**
> Examine `crates/temper-core/src/operations/commands.rs::UpdateResource`
> and `commands/resource.rs::UpdateParams`. Map every clap field on
> `UpdateParams` to either:
> (a) an existing field on `UpdateResource` (already in operations);
> (b) the `managed_meta`/`open_meta` partial Maps;
> (c) a field that does NOT exist yet on `UpdateResource` and that
>     4a must add (e.g., `context_to`, `type_to`).
>
> For category (c), add the fields to `UpdateResource` in this task
> with the smallest sensible shape — likely a nested `MoveSpec
> { context_to: Option<String>, type_to: Option<String> }`. Update
> Phase 3a's `DbBackend::update_resource` to handle the new fields
> (it'll just ignore them; DB updates don't move files). Re-prepare
> sqlx cache if needed (probably not — these fields are CLI-side).

**Implementation outline:**
```rust
async fn update_resource(
    &self,
    cmd: UpdateResource,
) -> Result<CommandOutput<ResourceRow>, TemperError> {
    use temper_core::operations::actions::{validate_update};

    validate_update(&cmd)?;

    // Resolve target file
    let manifest_guard = self.manifest.lock().await;
    let resolved = translators::resolve_resource_ref(
        &self.vault_root, &*manifest_guard, self.owner.as_str(), &self.config, &cmd.resource,
    )?;
    drop(manifest_guard);

    // Parse, apply updates (translators::apply_updates), optional move
    let mut fm = temper_core::frontmatter::Frontmatter::parse_file(&resolved.path)?;
    let final_path = translators::apply_updates(&mut fm, &cmd, &resolved.path, &self.vault_root, self.owner.as_str())?;

    // Refresh temper-updated, write
    let now = chrono::Local::now().to_rfc3339();
    fm.set_managed_field("temper-updated", serde_json::Value::String(now));
    if let Some(body) = cmd.body.as_ref() {
        fm.set_body(body.content.clone());
    }
    fm.write_to(&final_path)?;
    if final_path != resolved.path && resolved.path.exists() {
        std::fs::remove_file(&resolved.path)?;
    }

    // Manifest rehash (best-effort)
    let mut manifest = self.manifest.lock().await;
    rehash_entry(&mut manifest, resolved.resource_id, &final_path, &self.vault_root)?;
    manifest_io::save_manifest(&self.config.state_dir, &manifest)?;
    drop(manifest);

    // Events + push-as-tail (mirrors create_resource)
    let mut events = vec![
        DomainEvent::VaultFileWritten { path: rel_path(&final_path, &self.vault_root) },
        DomainEvent::VaultManifestUpdated { path: rel_path(&final_path, &self.vault_root) },
    ];

    if let Some(client) = &self.client {
        let body_trio = if let Some(body) = cmd.body.as_ref() {
            Some(translators::prepare_body_trio(&body.content)?)
        } else {
            None
        };
        let req = translators::cmd_to_update_request(&cmd, body_trio.as_ref())?;
        match client.resources().update(*resolved.resource_id, &req).await {
            Ok(_row) => events.push(DomainEvent::RemoteSynced { resource_id: resolved.resource_id }),
            Err(e) if is_offline(&e) => events.push(DomainEvent::PushDeferred { reason: PushDeferReason::Offline }),
            Err(e) if is_auth_error(&e) => events.push(DomainEvent::PushDeferred { reason: PushDeferReason::NotAuthed }),
            Err(_) => events.push(DomainEvent::PushDeferred { reason: PushDeferReason::Other }),
        };
    } else {
        events.push(DomainEvent::PushDeferred { reason: PushDeferReason::Offline });
    }

    let row = vault_file_to_resource_row(&final_path, &fm, resolved.resource_id);
    Ok(CommandOutput::with_events(row, events))
}
```

**`translators::apply_updates`** — lifts the scalar/array logic from
`commands/resource.rs:1545-1634`. Pure function, takes
`&mut Frontmatter` + cmd + path/root/owner for the move computation,
returns the final path.

**Tests (in `tests.rs`):**
- `update_resource_scalar_field_updates`
- `update_resource_array_field_appends`
- `update_resource_body_only_updates`
- `update_resource_context_to_moves_file` — write fixture in
  `@me/temper/task/`, call with managed_meta.context = "writing",
  assert file is at `@me/writing/task/...`, old path removed
- `update_resource_type_to_moves_file`
- `update_resource_no_client_emits_push_deferred`
- `update_resource_with_client_emits_remote_synced`

**Verification.**
- `cargo nextest run -p temper-cli vault_backend::tests::update_resource --features test-db`
- `cargo nextest run -p temper-cli vault_backend::tests::update_resource --features test-db,test-embed`
- `cargo make check` clean.

**Commit message.** `wave1-4a task 8: VaultBackend::update_resource impl + tests`

---

### Task 9 — `Backend::delete_resource`

**Owner:** subagent (sonnet)

**Goal.** Cloud-first delete. API call first; on success, remove
local file and manifest entry. Mirrors the existing logic at
`commands/resource.rs:748-868` with the surface-layer concerns
(TTY guard, prompt rendering) stripped — those move to the surface
adapter in 4b.

**Implementation outline:**
```rust
async fn delete_resource(
    &self,
    cmd: DeleteResource,
) -> Result<CommandOutput<()>, TemperError> {
    // Cloud-first: API delete first.
    let manifest_guard = self.manifest.lock().await;
    let resolved = translators::resolve_resource_ref(
        &self.vault_root, &*manifest_guard, self.owner.as_str(), &self.config, &cmd.resource,
    );
    drop(manifest_guard);

    // Resolve might fail if neither manifest entry nor file exists;
    // in that case there's nothing to delete locally, but we still
    // call the API in case the resource exists remotely.
    let resource_id_opt = resolved.as_ref().ok().map(|r| r.resource_id);

    let mut events = Vec::new();

    if let Some(client) = &self.client {
        if let Some(rid) = resource_id_opt {
            client.resources().delete(*rid).await
                .map_err(client_err_to_temper)?;
            events.push(DomainEvent::RemoteSynced { resource_id: rid });
        }
    }
    // If no client AND no local entry, that's NotFound.
    let resolved = resolved?;

    // Local-tail: confirmation prompt is a SURFACE concern (clap layer).
    // Backend assumes cmd.force is already correct.
    if resolved.path.exists() {
        std::fs::remove_file(&resolved.path)?;
        events.push(DomainEvent::VaultFileRemoved {
            path: rel_path(&resolved.path, &self.vault_root),
        });
    }
    let mut manifest = self.manifest.lock().await;
    if manifest.entries.remove(&resolved.resource_id).is_some() {
        manifest_io::save_manifest(&self.config.state_dir, &manifest)?;
        events.push(DomainEvent::VaultManifestUpdated {
            path: rel_path(&resolved.path, &self.vault_root),
        });
    }
    drop(manifest);

    Ok(CommandOutput::with_events((), events))
}
```

**Confirmation prompt is intentionally NOT inside the backend.** The
backend takes `cmd.force` and does what it's told. The TTY check,
the `[y/N]` render, the `stdin.read_line` — that's surface concern,
moved to the clap-layer adapter in 4b.

**Tests (in `tests.rs`):**
- `delete_resource_cloud_first_calls_api_then_removes_local`
- `delete_resource_no_client_removes_local_only_if_manifest_present`
- `delete_resource_no_local_file_but_client_present_still_calls_api`
- `delete_resource_api_failure_aborts_before_local_mutation` — assert
  on API error the local file is NOT touched

**Verification.**
- `cargo nextest run -p temper-cli vault_backend::tests::delete_resource --features test-db`
- `cargo make check` clean.

**Commit message.** `wave1-4a task 9: VaultBackend::delete_resource impl + tests`

---

### Task 10 — Object-safe smoke test + trait-impl tests module finalization

**Owner:** subagent (haiku)

**Goal.** Promote the `assert_object_safe(_: &dyn Backend)` test
pattern from Phase 1 / Phase 3a into `vault_backend/tests.rs`. Add a
single `fn assert_object_safe(_: &dyn Backend) {}` and a test
constructing a `Box<dyn Backend>` from a `VaultBackend`. Mirror
`temper-api/src/backend/tests.rs::assert_object_safe`.

**Test (in `tests.rs`):**
```rust
#[test]
fn vault_backend_is_object_safe() {
    fn assert_object_safe(_: &dyn temper_core::operations::Backend) {}
    let ctx = /* tmp vault */;
    let backend: Box<dyn temper_core::operations::Backend> =
        Box::new(VaultBackend::new(ctx));
    assert_object_safe(&*backend);
}
```

**Verification.**
- `cargo nextest run -p temper-cli vault_backend::tests::vault_backend_is_object_safe --features test-db`
- `cargo make check` clean.

**Commit message.** `wave1-4a task 10: object-safe smoke test for VaultBackend`

---

### Task 11 — Backlog tasks captured up front

**Owner:** main agent (this session) OR subagent

**Goal.** Per `feedback_no_premature_backward_compat` — create the
follow-up tasks at the moment of plan-writing, not deferred-with-vibes.

Tasks to create via `temper resource create --type task --context temper
--mode build --effort small --goal temper-maintenance`:

1. `wave-1-phase-4b-extract-commands-resource-rs-local-mode-writes-through-vaultbackend`
2. `delete-actions-frontmatter-build-managed-meta-for-create-after-4b`
3. `delete-commands-resource-rs-resolve-resource-id-after-4b`
4. (conditional) `lift-prepare-body-trio-to-temper-core-shared-helper`
   — only if Task 2 picked the duplicate path
5. (conditional) `complete-per-doctype-write-dispatch-for-task-goal-session-research`
   — only if Task 7 audit gate forced concept/decision-only shipping

Each task gets a 5-10 line body capturing the why and the entry point
(file:line where the cleanup needs to start).

**Commit message.** `wave1-4a task 11: capture 4b + cleanup backlog tasks`

(Note: backlog task creation doesn't need to be a commit on the branch
— it's vault work, separate from the code branch. Captured here so it
doesn't get skipped.)

---

### Task 12 — Full-suite verification + spec-compliance review

**Owner:** subagent (opus for spec review; sonnet for the verification)

**Goal.** Run the full local verification matrix and dispatch a
spec-compliance reviewer against the spec doc.

**Verification commands:**
```bash
# Full local suite
cargo make check
cargo make test
cargo make test-db

# Workspace run — surfaces feature-unification issues per
# feedback_workspace_test_surfaces_pipeline_bugs
cargo nextest run --workspace --no-fail-fast 2>&1 | tee /tmp/4a-workspace.log
# Then grep for failures:
grep -E "FAIL \[|error: test run failed" /tmp/4a-workspace.log

# Embed-gated e2e
cargo nextest run --manifest-path tests/e2e/Cargo.toml --features test-db,test-embed
```

**Spec-compliance review.** Dispatch an opus subagent with the spec
doc + the diff (`git diff main...HEAD`). Prompt: "Confirm Phase 4a as
implemented matches the spec at
docs/superpowers/specs/2026-05-11-wave1-phase4-vaultbackend-design.md.
Flag any divergence as either (a) implementer interpretation drift
that should be corrected, or (b) spec ambiguity that should be
clarified in the spec. Report under 400 words with file:line refs."

**Address findings** inline if quick (≤30 lines); roll into 4b otherwise.

**Commit message (if any code changes from review).** `wave1-4a task 12: spec-compliance fixups`

---

### Task 13 — Final code review (READY_WITH_FOLLOWUPS or REQUEST_CHANGES)

**Owner:** subagent (opus)

**Goal.** Independent code review on the full branch.

**Dispatch prompt:**
> Review branch `jct/wave1-phase4a-vaultbackend-foundation` against
> the spec at
> `docs/superpowers/specs/2026-05-11-wave1-phase4-vaultbackend-design.md`.
> Focus on:
> - Backend trait impl correctness (does the trait promise match what
>   the impl delivers, especially for read-asymmetric methods like
>   search)?
> - Event emission completeness (are
>   `VaultFileWritten`/`VaultManifestUpdated`/`RemoteSynced`/`PushDeferred`/`VaultFileRemoved`
>   emitted at the right points and only at the right points?)
> - Concurrency safety (`Arc<Mutex<Manifest>>` — are there any
>   lock-while-doing-I/O windows that should be tighter?)
> - Schema-required defaults symmetric defense (is
>   `apply_defaults_value` + `ensure_managed_identity_keys` called
>   on the create + update paths?)
> - Test coverage (is each method exercised happy + 1 error path?
>   Are the embed-gated paths gated correctly?)
> - Sub-phase boundary integrity (is 4a's no-caller-rewired contract
>   honored? No diff inside `commands/*.rs` other than ambient imports?)
> Return READY_WITH_FOLLOWUPS or REQUEST_CHANGES with a categorized
> bullet list (critical / important / nit).

**Address critical and important findings** inline. Nits roll into
4b unless trivial.

**Commit message.** Code-review fixups commit, if any.

---

### Task 14 — Open PR with the spec link and the dark-launch caveat

**Owner:** main agent

**Goal.** Push branch and open the PR. PR body explicitly notes the
dark-launch (no callers rewired) so the reviewer understands why
existing CLI tests pass unmodified.

**PR title.** `Wave 1 Phase 4a: VaultBackend foundation (dark-launched)`

**PR body (template):**
```
## Summary
- Lands the `VaultBackend` foundation per the spec at
  `docs/superpowers/specs/2026-05-11-wave1-phase4-vaultbackend-design.md`.
- Implements `Backend` for `VaultBackend` covering all 6 trait methods.
- Dark-launched: no callers rewired. `commands/*.rs` is unchanged
  except for ambient imports. 4b (next session) does the wiring.
- Trait-impl unit tests in `vault_backend/tests.rs` against a tmp
  vault cover happy + error paths per method.

## Test plan
- [x] `cargo make check`
- [x] `cargo make test`
- [x] `cargo make test-db`
- [x] `cargo nextest run --workspace --no-fail-fast`
- [x] `cargo nextest run --manifest-path tests/e2e/Cargo.toml --features test-db,test-embed`

## Follow-ups (backlog tasks created)
- 4b: extract `commands/resource.rs` local-mode writes through
  `VaultBackend`.
- Delete `actions::frontmatter::build_managed_meta_for_create` after 4b.
- Delete `commands/resource.rs::resolve_resource_id` after 4b.
- (conditional) Lift `prepare_body_trio` to `temper-core`.
- (conditional) Complete per-doctype write dispatch (task/goal/session/research).
```

Run `git merge origin/main` before pushing (per
`feedback_merge_main_before_pushing_pr`).

**Commit / push.** Push branch, open PR via `gh pr create`.

---

## Plan-writer self-review checklist

Per `feedback_plan_code_quality` and the spec acceptance bar:

- [x] Every trait method on `Backend` has at least one task implementing
      it.
- [x] Read paths (`show`, `list`, `search`) land before write paths
      (`create`, `update`, `delete`) — the simpler trait-impl shape
      is established first, write-path orchestration comes after.
- [x] Each task has explicit verification commands.
- [x] Workspace-feature-unification verification (per
      `feedback_workspace_test_surfaces_pipeline_bugs`) included in
      Task 12.
- [x] Embed-gated tests called out where the body pipeline runs.
- [x] Subagent prompts have escalate-not-soften baked in (Task 7
      audit gate, Task 8 decision pre-work).
- [x] No "for now" workarounds tolerated.
- [x] Backlog tasks captured up front (Task 11).
- [x] PR body template included so the dark-launch contract is
      explicit to the reviewer.
- [x] Branch-merge-before-push reminder in Task 14.
- [x] Cross-references to the spec doc and to memory rules
      (`feedback_*`) so subagents inherit context.
