# Wave 1 Phase 4 — VaultBackend Implementation Design

**Date:** 2026-05-11
**Context:** `temper`
**Mode:** plan
**Effort:** large (decomposed into 4a/4b sub-PRs during plan-writing)
**Predecessors (merged):**
- Phase 1+2 (PR #65): `temper-core::operations` scaffolding + shared actions
- Phase 3a (PR #69): `DbBackend` foundation, dark-launched
- Phase 3b+3c (PR #71): every HTTP + MCP write path routed through `DbBackend`
- Phase 4-prep (PR #74): `ResourceRef::Scoped` gains `owner` field
- Owner-threading sweep (PR #75)
- Cloud-only sync handling + `find_resource` refactor (PR #76)

**Parent spec:** `docs/superpowers/specs/2026-05-01-shared-core-execution-paths-design.md` (#4)
**Companion spec:** `docs/superpowers/specs/2026-05-01-cloud-first-reframe-and-manifest-redefinition-design.md` (#3 — Phase 6)
**Originating task:** `2026-05-11-wave-1-phase-4-write-vaultbackend-impl-plan-extract-from-commands-resource-rs`

---

## Problem

`crates/temper-cli/src/commands/resource.rs` is 2135 lines. Every resource
write (`create`, `update`, `delete`) opens with `let vault_state =
VaultState::from_env()` and branches into a Local-mode path or a Cloud-mode
path. The Local-mode paths interleave four concerns inline:

1. **Vault file IO** — `Vault::doc_file(...)`, `std::fs::create_dir_all`,
   `Frontmatter::write_to(&path)`, optionally moving the file across
   `--context-to` / `--type-to` boundaries.
2. **Frontmatter manipulation** — `Frontmatter::set_managed_field`,
   array appends, schema validation, `temper-updated` timestamp refresh.
3. **Manifest IO** — `manifest_io::load_manifest` /
   `manifest_io::save_manifest`, entry insertion / removal.
4. **Best-effort push as tail action** —
   `runtime::publish_local_write_best_effort`, plus (for delete) cloud-first
   ordering via `client.resources().delete(uuid)` *before* local mutation.

The Cloud-mode paths are well-factored already: they build typed structs
(`IngestPayload`, `ResourceUpdateRequest`) and call `temper-client`. The
Local-mode paths are the asymmetry — every new feature has to land twice
(once per branch) and the drift surfaces as bugs (the phantom show-edit-cat,
the silent `✓ Updated` on dropped body, the missing cloud-first delete in
PR #64, the `_role` regression that triggered the symmetric defense work).

Phases 1+2+3 closed the loop on the **shared-action** and **DB-side**
halves of the parent spec's reframe. Phase 4 closes it on the
**vault-side** half: extract the Local-mode logic into a `VaultBackend`
struct that implements the same `Backend` trait `DbBackend` already
implements, so both backends can later be dispatched from a single
`Surface::dispatch` call (Phase 5).

## The Reframe (Phase 4 specifics)

`VaultBackend` is constructed per inbound CLI invocation: `commands/
resource.rs` will (in 4b) build a `VaultBackend` from the resolved config
+ manifest + optional client + owner + surface, and dispatch one command
through it. Each trait method is a thin orchestrator — it composes the
shared `temper-core::operations::actions::*` primitives that Phase 2
extracted, performs the vault-file side effects, updates the manifest,
and (for create/update/delete in `CliLocalVault`) attempts the
push-as-tail-action via `temper-client`. No new SQL. No new HTTP routes.
The migration is structure-tightening, not behavior change.

The dispatch direction across the CLI boundary changes; the local-mode
semantics (cloud-first delete ordering, manifest dual-write, best-effort
publish, schema-required defaults at create/update) are preserved verbatim.

```
  Surface (commands/resource.rs — clap parser)
        │  build per-request VaultBackend from {
        │     vault_root, manifest, client, owner, surface
        │  }; build operations::*Resource command from inbound clap args
        ▼
  VaultBackend.<method>(cmd)             ← Phase 1 trait (object-safe, async_trait)
        │  orchestrator:
        │    1. shared validate / applyDefaults / merge from operations::actions
        │    2. vault-file IO (Vault::doc_file, Frontmatter::*)
        │    3. manifest update (manifest_io::*)
        │    4. tail-action push via temper-client (CliLocalVault only)
        ▼
  filesystem + manifest.json
  + optional `temper-client` → API → DbBackend (already shipped)
```

## Components

### `crates/temper-cli/src/vault_backend/`

New directory. Mirrors Phase 3a's `temper-api/src/backend/` shape:

```
crates/temper-cli/src/vault_backend/
├── mod.rs              // pub use vault_backend::{VaultBackend, VaultBackendCtx};
├── vault_backend.rs    // struct + impl Backend for VaultBackend
├── translators.rs      // cmd → field-update + path orchestrators
├── per_doctype.rs      // dispatch table for create_resource by doctype
└── tests.rs            // trait-impl unit tests against a tmp vault
```

Five files (one more than Phase 3a's four) because per-doctype dispatch
deserves its own module — five doctypes (task, goal, session, research,
concept/decision) with distinct file-write paths.

### `VaultBackend` struct

Per-request construction. State that the trait method needs is part of
the value, not threaded through method args.

```rust
// crates/temper-cli/src/vault_backend/vault_backend.rs (sketch)
pub struct VaultBackend {
    vault_root: PathBuf,
    /// Wrapped in Arc<Mutex<>> because trait methods take `&self` (matching
    /// DbBackend) but the manifest is mutated by create/update/delete.
    /// Per-command lifetime makes the mutex contention-free in practice.
    manifest: Arc<Mutex<Manifest>>,
    /// Some when the caller has an authenticated `temper-client` available;
    /// None when the binary is fully offline (rare for CliLocalVault, but
    /// possible during `temper auth logout`-then-edit). Backend treats
    /// None as "PushDeferred { Offline }" and stops at the local write.
    client: Option<Arc<TemperClient>>,
    /// Owner handle for vault-file path construction. `@me` for solo;
    /// `+team-...` for team contexts when teams ship.
    owner: OwnerHandle,
    /// Origin of the inbound command. Today always `CliLocalVault` for
    /// VaultBackend; stored for forward-compat (Phase 6 telemetry).
    surface: Surface,
}

pub struct VaultBackendCtx {
    pub vault_root: PathBuf,
    pub manifest: Arc<Mutex<Manifest>>,
    pub client: Option<Arc<TemperClient>>,
    pub owner: OwnerHandle,
    pub surface: Surface,
}

impl VaultBackend {
    pub fn new(ctx: VaultBackendCtx) -> Self { /* ... */ }
}

#[async_trait]
impl Backend for VaultBackend { /* 6 methods */ }
```

The `VaultBackendCtx` builder honors the project's params-struct rule
(5 fields ≥ threshold). `DbBackend` got away with positional args at 4
fields; `VaultBackend` opens with the struct from day 1 because (a)
`OwnerHandle` is the next field anyone is going to want to pass, and
(b) the field count is already at threshold, not below it.

The 4b extraction site builds it once per CLI invocation:

```rust
// commands/resource.rs (4b — sketch of the call site)
let device_id = config::load_device_id().unwrap_or_else(|| "unknown".into());
let manifest = Arc::new(Mutex::new(manifest_io::load_manifest(
    &config.state_dir, &device_id,
)?));
let client = actions::runtime::optional_client_arc()?;  // helper added in 4a
let owner = OwnerHandle::from_str(&config.owner_for_context(&ctx))?;
let backend = VaultBackend::new(VaultBackendCtx {
    vault_root: config.vault_root.clone(),
    manifest,
    client,
    owner,
    surface: Surface::CliLocalVault,
});
let cmd = CreateResource { /* from clap args */ };
let output = backend.create_resource(cmd).await?;
// Render output.value via existing output:: helpers; observe output.events.
```

### Trait method → vault-flow mapping (canonical)

| Trait method | Vault-side flow | Notes |
|---|---|---|
| `create_resource` | `validate_create(&cmd)` → `apply_defaults_value(doctype, &mut managed)` → `ensure_managed_identity_keys(&mut managed, title, Some(slug))` → per-doctype file-write dispatch (`per_doctype::write_for(&cmd)`) → manifest entry insert (`Provisional`) → `RemoteSynced` / `PushDeferred` via `client.ingest().create(&payload)` → manifest entry promote-to-Clean on success | Per-doctype dispatch table covers task / goal / session / research / concept / decision. Each entry calls the existing creator (`actions::task::create`, `commands::goal::create`, etc.) for now — full pull-in is a follow-up. Body present? Run `prepare_body_trio` (already in `temper-api/src/backend/translators.rs`; lift the function-shape into a shared `temper-core` helper if both backends need it, or duplicate with a TODO; default: duplicate into vault_backend/translators.rs and capture as cleanup). |
| `update_resource` | resolve `ResourceRef` → load file via `lookup::find_resource` → load `Frontmatter::parse_file` → apply scalar+array updates (existing logic, lifted into `translators::apply_updates`) → optional `--context-to` / `--type-to` move → `Frontmatter::set_body` if `cmd.body.is_some()` → `Frontmatter::write_to(final_path)` → if file moved, remove old → manifest entry rehash → push via `client.resources().update(uuid, &req)` (or `client.ingest().update(uuid, &payload)` for body-bearing) → emit `VaultFileWritten` + `VaultManifestUpdated` + `RemoteSynced`/`PushDeferred` | Reuses `temper_core::operations::actions::merge_managed_meta` and `merge_open_meta` for the in-memory merge before write. The 4b call site stops calling `actions::body_source::resolve_body_source` directly — that becomes a surface-side translation (clap → `BodyUpdate`), and the backend assumes the cmd already has the resolved body. |
| `delete_resource` | resolve `ResourceRef` → cloud-first: `client.resources().delete(uuid)` → on success: load manifest, find file via entry-or-fallback (existing logic), prompt or `--force`, `std::fs::remove_file`, manifest entry remove → emit `RemoteSynced` (cloud delete) + `VaultFileRemoved` + `VaultManifestUpdated` | Cloud-first ordering preserved (parent spec hard rule). The non-TTY guard moves to the surface (clap layer); backend assumes `cmd.force` is correct. |
| `show_resource` | resolve `ResourceRef` → load file via `lookup::find_resource` → `Frontmatter::parse_file(&path)` → return `ResourceRow` projection. On `LocallyMissing` (manifest entry exists but file missing): fall back to `client.resources().content(uuid)` (the existing `show_via_api_fallback`). | Read-path; reuses `lookup::find_resource` from PR #76. No events emitted (Phase 3 read-path precedent). |
| `list_resources` | scan `vault_root/<owner>/<context>/<doctype>/` → parse each file → filter → sort → project to `Vec<ResourceSummary>` | Read-path. Reuses existing `scan_rows`/`filter_rows`/`sort_rows` helpers, lifted from `commands/resource.rs` into `vault_backend/list.rs` (or kept inline in `vault_backend.rs` if the helpers are short). |
| `search_resources` | local search is removed (per `project_offline_indexing_dropped`); delegate to `client.search().search(&params)` when `client.is_some()`; return `BadRequest("search requires authenticated client")` when None. | Local indexing was dropped in I5a. `VaultBackend.search_resources` is therefore a passthrough to the client; this is the one trait method where vault-only mode legitimately can't satisfy the contract. |

### Translators

`translators.rs` holds pure cmd → vault-flow adapters (no I/O):

- `apply_updates(fm: &mut Frontmatter, cmd: &UpdateResource)` — applies
  scalar fields and array appends from `cmd.managed_meta` and
  `cmd.open_meta` to an in-memory frontmatter. Mirrors the inline logic
  at `commands/resource.rs:1545-1634` lifted out.
- `cmd_to_ingest_payload(cmd: &CreateResource, body_trio: BodyTrio) -> IngestPayload`
  — builds the payload for `client.ingest().create`. `body_trio` carries
  pre-computed `(content_hash, chunks_packed)` when the body branch ran
  the pipeline.
- `cmd_to_update_request(cmd: &UpdateResource, body_trio: Option<BodyTrio>) -> ResourceUpdateRequest`
  — builds the update request for `client.resources().update` (meta-only
  branch) or `client.ingest().update` (body-bearing branch).
- `resolve_resource_ref(rref: &ResourceRef, manifest: &Manifest, vault_root: &Path, owner: &str) -> Result<ResolvedResource>`
  — resolves a `ResourceRef` to (`ResourceId`, `PathBuf`) for the
  vault-side. `Uuid` short-circuits via the manifest's
  `entries.get(&id)` lookup; `Scoped` calls into `lookup::find_resource`.

### Coarse post-hoc event emission

Each `VaultBackend` method synthesizes a small set of events on success.
The events are exactly the variants `temper-core::operations::events`
already defines (Phase 1's enum is complete for this purpose):

- `create_resource` → `[VaultFileWritten { path }, VaultManifestUpdated { path }, RemoteSynced { resource_id } | PushDeferred { reason }]`
- `update_resource` → same set
- `delete_resource` → `[RemoteSynced { resource_id }, VaultFileRemoved { path }, VaultManifestUpdated { path }]`
- `show_resource` / `list_resources` / `search_resources` → empty events vec

The push-related event (`RemoteSynced` vs `PushDeferred`) is decided
inside the backend method based on the `client.is_some()` and the
result of the push call. Reasons for `PushDeferred`: `Offline` (no
client present), `NotAuthed` (auth-failed during push), `Other` (any
other client error). The `Other` variant carries the same `Vec<DomainEvent>`
contract — callers can introspect; surfaces today only inspect
event variants for log lines.

### Auth, profile scoping, and existing rules — preserved

- **Auth before writes** (CLAUDE.md): preserved — but the local-vault
  side enforces a different shape than the DB side. Local-vault writes
  are unauthenticated against the local filesystem (the user owns it);
  the *push-as-tail-action* call into `temper-client` reuses the
  cached token from `auth.json`, and the API server enforces auth on
  receipt. No new auth surface in `VaultBackend`.
- **Profile scoping**: vault paths scope through `OwnerHandle`
  (`@me` / `+team-...`), threaded into `Vault::doc_file(owner, ...)`.
  `OwnerHandle` is the Phase 4-prep / PR #75 mechanism; VaultBackend
  consumes it directly.
- **Schema-required defaults at create/update**: `apply_defaults_value`
  and `ensure_managed_identity_keys` (both Phase 2 actions, both already
  used by `DbBackend` translators) are called from `VaultBackend.create_resource`
  and `update_resource` before the file is written. **Symmetric defense
  on the local side closes the existing send-side gap** — today the
  vault-file create paths inject defaults via `actions::frontmatter::
  build_managed_meta_for_create`, which is the **clap-layer** translator,
  not the operations-layer action. After Phase 4 the operations-layer
  action is the authoritative entry; the clap-layer translator either
  goes away or becomes a thin wrapper.
- **Service layer owns SQL** (CLAUDE.md): N/A — VaultBackend has no
  SQL. The CLAUDE.md rule's local-vault analogue is "VaultBackend owns
  vault-file IO and manifest IO; surfaces don't open files directly."
  The 4b extraction enforces this.
- **Typed structs over inline JSON**: `cmd_to_ingest_payload` and
  `cmd_to_update_request` produce typed wire structs; no raw `json!()`
  in the backend.
- **Params structs**: `VaultBackendCtx` introduced from day 1.

### Sub-phase decomposition

One spec, two plans, two PRs. Each is independently shippable.

**4a — Foundation (this session, written from this spec).** Lands the
architectural foundation; no behavior change visible from any surface.

- Add `crates/temper-cli/src/vault_backend/{mod,vault_backend,translators,per_doctype,tests}.rs`.
- `VaultBackend` struct + `VaultBackendCtx` builder + all 6 trait method impls
  delegating to existing creators / lifted helpers.
- `prepare_body_trio` — duplicate the body-pipeline helper from
  `temper-api/src/backend/translators.rs` into `vault_backend/translators.rs`
  (or extract to a shared `temper-core::operations::body` module — TBD
  during 4a Task 1 once the import graph is examined). Captured as
  follow-up either way.
- Trait-impl unit tests in `vault_backend/tests.rs` against a tmp vault:
  each method exercised once happy-path + one error path, plus the
  object-safe `dyn Backend` smoke test (mirroring Phase 3a's promotion).
- `cargo make check`, `cargo make test`, `cargo make test-db` clean.
- No `commands/*.rs` callers rewired; the trait is dark-launched,
  verified by trait-impl tests.

**4b — Resource extraction (subsequent session).** Migrate `commands/
resource.rs`'s Local-mode write paths to dispatch through `VaultBackend`.
- The `match VaultState` branch stays (Phase 5 collapses it); only the
  inside of the `Local` arm changes.
- `commands/resource.rs::create / update / delete` Local-mode bodies
  shrink to: build `VaultBackendCtx`, build cmd from clap args, call
  `backend.<method>(cmd).await`, render output.
- Existing CLI integration tests + e2e tests pass unmodified — the
  pass-without-modification bar is the regression guard.
- The follow-up cleanup (other commands' write paths — `commands/
  task.rs`, `commands/goal.rs`, `commands/session.rs`, `commands/
  research.rs`) is **not** in 4b. Those are touched in Phase 5 when
  the surface dispatch unification rewires every `match VaultState`
  call site through one `Surface::dispatch`. Keeping 4b narrow keeps
  the diff reviewable.

Each sub-phase is a separate plan document under `docs/superpowers/plans/`,
written when its session begins.

## Forward-Looking Constraints & Inherited Guidance

This section makes the "out of scope" boundary explicit so the constraints
already developed don't get dropped. Phase 4 is **not** allowed to make
any of these harder.

### Phase 5 (Surface dispatch unification) — must remain achievable

Phase 5 collapses `commands/resource.rs`'s `match VaultState` branches
into a single `Surface::dispatch` that picks `VaultBackend` (CliLocalVault)
or invokes `temper-client` (CliCloud). Phase 4's per-request `VaultBackend`
shape supports this directly: the dispatch site picks one backend and
calls trait methods. No retrofit needed. The 4b extraction explicitly
keeps the cloud-mode arm untouched so Phase 5's diff is the surgical
collapse, not a co-mingled rewrite.

### Phase 6 (state machines, fine-grained events, manifest narrowing)

Phase 6 wires the lifecycle state machines in companion spec #3:

- `VaultBackend` lifecycle is `Synced ↔ LocalEditing ↔ PendingPush ↔
  Synced` per command. **Phase 4's coarse events `VaultFileWritten` /
  `VaultManifestUpdated` / `VaultFileRemoved` / `RemoteSynced` /
  `PushDeferred` are exactly the transition triggers companion spec
  #3 §3.2 specifies.** They do not get replaced in Phase 6 — they
  drive the state-machine evaluations.
- `LocallyMissing` recovery (the `temper sync run` path that pulls
  back missing-but-tracked files) is owned by `actions::sync` and is
  **outside VaultBackend's scope**. VaultBackend's `show_resource`
  exposes the missing-file fallback to API content, but the manifest
  reclassification + bulk pull stays in `actions::sync`. Phase 4
  doesn't touch `actions::sync.rs` (4369 lines).
- `Conflicting` state in companion spec #3 §3.2 depends on
  `VaultBackend` carrying a base-hash through the push call. Phase 4
  passes `cmd.body.content_hash` if set, else recomputes — same
  contract `DbBackend` already honors. Conflict detection wire-in is
  Phase 6.

### Out-of-scope cleanups (high-priority follow-ups, not vague-future)

Per `feedback_no_premature_backward_compat` (project is one month old;
remove dead code rather than keeping it "for compat") — the divergent
helpers get explicit cleanup tasks, not indefinite leashes:

1. **`actions::frontmatter::build_managed_meta_for_create`**
   becomes redundant when `VaultBackend.create_resource` calls
   `apply_defaults_value` + `ensure_managed_identity_keys` directly.
   After 4b lands, delete the action and inline the clap-arg-mapping
   at the surface where create_resource is dispatched. Captured as a
   backlog task at the moment 4a's plan is written.
2. **`prepare_body_trio` duplication** between `temper-api/src/backend/
   translators.rs` and `vault_backend/translators.rs`. If 4a Task 2's
   import-graph examination shows `temper-core::operations::body` is
   viable (no `temper-ingest` dependency leak into `temper-core`), do
   the lift inside 4a and there's no duplication. If not, ship 4a with
   the duplicate and capture as a backlog task.
3. **`commands/resource.rs::resolve_resource_id`** becomes redundant
   when `VaultBackend::translators::resolve_resource_ref` covers the
   same cases. Delete after 4b's `task::show` and `session::show`
   call-site update.

### Inherited guidance for implementer subagents (4a/4b plans)

The 4a/4b plan documents must embed these rules in implementer prompts
(per the working memory rules `feedback_subagent_check_before_commit`,
`feedback_subagent_escalate_not_soften`, `feedback_plan_regression_guard_after_filter_test`,
`feedback_workspace_test_surfaces_pipeline_bugs`):

- **Run `cargo make check` before claiming work complete.** Pre-commit
  hook is a backstop, not first line.
- **Pair every filter-by-name test run with a full crate suite run
  before commit.** A passing single test does not imply a passing crate.
- **Embed-gated tests:** when 4a or 4b touches push-body / ingest-pipeline
  code (the `prepare_body_trio` path), run with
  `--features test-db,test-embed` locally to match CI's Embed job.
- **Workspace-feature-unification awareness:** the `cargo nextest
  --workspace` run activates `ingest-pipeline` via `temper-cloud`'s
  feature unification, exercising code paths the standalone
  `-p temper-cli` test won't. Always include workspace runs in
  regression-guard verification.
- **Don't trust nextest's per-binary `Summary` line** with
  `--no-fail-fast` (`feedback_nextest_summary_lies`). Trust exit code
  or grep for `error: test run failed` / `FAIL [`.
- **Escalate, don't soften.** If passing a test requires loosening
  an error path or a contract, STOP and report BLOCKED.
- **No "for now" workarounds.** If `VaultBackend.delete_resource`
  discovers `cmd.force` has unexpected non-TTY semantics, capture it
  as a task — don't ship a TODO comment.
- **Verify named APIs before dispatch.** Per
  `feedback_pre_propose_arch_review` and `feedback_plan_verification`:
  every API name in the 4a/4b plans gets grep-verified against the
  current code at dispatch time. The plan is a hypothesis; the code
  is ground truth.
- **Shared types live in `temper-core`.** Don't define a `BodyTrio`
  in two places; if both backends need it, lift to operations.

## Acceptance Criteria

- [ ] `crates/temper-cli/src/vault_backend/` exists with `VaultBackend`
      impl of `Backend` covering all 6 trait methods, each composing
      shared `temper-core::operations::actions::*` primitives + vault-side
      side effects + emitting coarse events.
- [ ] `VaultBackendCtx` builder exists for construction; `VaultBackend::new`
      takes a single `VaultBackendCtx` argument.
- [ ] All Local-mode write paths in `commands/resource.rs::create / update
      / delete` dispatch through `VaultBackend` (lands in 4b).
- [ ] Cloud-mode arms in `commands/resource.rs` are **untouched** by
      Phase 4 — they remain the existing direct `temper-client` calls.
      Phase 5 collapses the branch.
- [ ] Trait-impl unit tests in `crates/temper-cli/src/vault_backend/
      tests.rs` cover happy-path + one error path per method against a
      tmp vault.
- [ ] All existing `cargo make test`, `test-db`, `test-e2e` (with
      `test-embed` feature) suites pass at every sub-phase boundary.
- [ ] `temper-cli` continues to depend on `temper-core` and `temper-client`
      only (no new dep on `temper-api`).
- [ ] Backlog tasks created for: (a) delete `actions::frontmatter::
      build_managed_meta_for_create` after 4b; (b) lift `prepare_body_trio`
      to `temper-core` if duplication ships in 4a; (c) delete
      `commands/resource.rs::resolve_resource_id` after 4b.
- [ ] CLAUDE.md updated: the "Service layer owns SQL; surfaces dispatch
      through `DbBackend`" paragraph gets a sibling for the vault side:
      "Vault file IO and manifest IO live in `vault_backend/`; surfaces
      dispatch through `VaultBackend`."

## Out of Scope (with cross-references to where each lives)

- Surface dispatch unification (collapse `match VaultState`) — Phase 5.
- `commands/{task,goal,session,research}.rs` write paths — touched in
  Phase 5 alongside the surface unification.
- Lifecycle state machines, conflict semantics, manifest narrowing,
  fine-grained body-changed events — Phase 6 / companion spec #3.
- `actions::sync.rs` (4369 lines) — sync orchestration stays in
  `actions::sync`; VaultBackend's job is per-command operations, not
  bulk sync rounds.
- Auth/authz refactor — preserved unchanged; existing plumbing consumed
  as parameters.
- Surface output formatting refactor — clap output stays in
  `commands/resource.rs`; the change is the dispatch path, not the
  formatting.
- Local search reinstatement — `project_offline_indexing_dropped` rules
  this out; `search_resources` is a passthrough to the client.

## Open Questions

- **`prepare_body_trio` placement.** Inside 4a Task 2, examine the
  import graph: can `temper-core::operations::body` host a
  pipeline-feature-gated helper without leaking `temper-ingest` into
  `temper-core`'s default surface? If yes, lift it; if no, duplicate
  and capture cleanup. Default: duplicate, capture cleanup.
- **`per_doctype` dispatch — full pull-in vs delegation.** 4a delegates
  to existing per-doctype creators (`actions::task::create`, etc.).
  Phase 5 (or a follow-up cleanup) may pull those into `vault_backend/
  per_doctype.rs` for full uniformity. Default: delegation in 4a/4b;
  full pull-in deferred.
- **`OwnerHandle` for vault path construction.** PR #75 threaded owner
  through vault-path helpers; confirm `Vault::doc_file` already takes
  an owner string and `OwnerHandle::as_str()` produces the right form.
  If not, a small helper goes into 4a Task 2.
