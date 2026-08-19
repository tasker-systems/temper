# Wave 1 Phase 4 — VaultBackend, RemoteBackend, and CLI Surface Dispatch Unification

**Status:** Draft for plan-writing
**Date:** 2026-05-09
**Parent spec:** [`docs/superpowers/specs/2026-05-01-shared-core-execution-paths-design.md`](2026-05-01-shared-core-execution-paths-design.md)
**Predecessor spec:** [`docs/superpowers/specs/2026-05-07-wave1-phase3-dbbackend-design.md`](2026-05-07-wave1-phase3-dbbackend-design.md)
**Goal:** `path-to-alpha`

## Summary

Phase 4 delivers the second and third Backend trait implementations (after Phase 3's `DbBackend`), unifies the CLI's local- and cloud-mode write paths under a single dispatch point, and lifts ~2137 lines of mode-branching orchestration out of `commands/resource.rs`. After Phase 4 ships:

- `VaultBackend` (in `temper-cli`) implements `temper-core::operations::Backend` against vault-file persistence with best-effort tail-action push.
- `RemoteBackend` (in `temper-client`) implements the same trait by translating commands into `temper-client` HTTP calls.
- `Surface::dispatch` (a CLI-side helper) collapses every write-path `match VaultState` branch in `commands/resource.rs` into a single dispatch call.
- The architectural payoff originally split as Phase 4 + Phase 5 in the parent spec lands as one cohesive phase.

## Scope

In scope:
- New module `crates/temper-cli/src/vault_backend/` — `VaultBackend` impl.
- New module `crates/temper-client/src/backend.rs` — `RemoteBackend` impl.
- New module `crates/temper-cli/src/dispatch.rs` — `build_backend(...) -> Box<dyn Backend>`.
- Rewire of `commands/resource.rs` write functions (`create`, `update`, `delete`) to dispatch through the trait.
- Manifest schema additions to support PendingPush hooks for Phase 6.

Out of scope:
- Read-path unification. `show`, `list`, `search`, `get_meta` continue to mode-branch inline at the command site, matching the precedent set by Phase 3 (HTTP and MCP read endpoints stayed service-direct on those surfaces). The Backend trait's read methods are still implemented on both new backends for symmetry, but `Surface::dispatch` only routes writes.
- `temper sync` commands (`push`, `pull`, `refresh`, `reset`, `status`). They are vault-bulk-orchestration, not single-resource writes, and continue to live in `actions/sync.rs`.
- `update_resource_meta` is not currently in the Backend trait shape (the trait has `create`/`show`/`update`/`delete`/`list`/`search`). If meta-updates are dispatched via `update_resource` with a meta-only `UpdateResource` cmd, no trait change is needed; otherwise plan-writing surfaces the gap.
- Phase 6's per-resource state machine (`Local`, `Synced`, `Conflicting`, `PendingPush`). Phase 4 introduces only the *hooks* (manifest field, `PendingPush` event payload). The state machine wires up later.
- `temper auth`, `temper config`, `temper goal`, `temper task`, `temper search`, etc. — out of resource-write surface.

## Architecture

### Trait + impl placement

```
temper-core::operations::Backend            (trait — Phase 1)
├── DbBackend          temper-api/src/backend/         [Phase 3a]
├── VaultBackend       temper-cli/src/vault_backend/   [Phase 4a]
└── RemoteBackend      temper-client/src/backend.rs    [Phase 4a]
```

The trait is dyn-safe (verified by `assert_object_safe(_: &dyn Backend)` in `temper-core/src/operations/backend.rs`). All three impls are `Box<dyn Backend>`-compatible.

**Why RemoteBackend in `temper-client`, not `temper-cli`:**
1. Future `temper-sdk` and similar non-CLI consumers (agents, automation, alternative clients) get a typed Backend API for free.
2. Composition payoff: `VaultBackend` can hold an `Option<Box<dyn Backend>>` for its tail-action push, and the natural concrete value to inject is `RemoteBackend`. With RemoteBackend in `temper-client`, `temper-cli` code can simply construct one and hand it to `VaultBackend::new` — no new dep edges, no wire-translation logic in `temper-cli`.
3. Dependency cost is small: `temper-client` already depends on `temper-core` for types; adding `temper-core::operations` (commands + output + the trait) is type-level extension only, no new heavy crates.

### Surface::dispatch

A CLI-side helper, not a method on the `Surface` enum (which already exists in `temper-core/src/operations/surface.rs` as a pure identity tag for cross-backend logging).

```rust
// temper-cli/src/dispatch.rs
pub fn build_backend(
    config: &Config,
    vault_state: VaultState,
    profile_id: Uuid,
    device_id: String,
) -> Result<Box<dyn Backend>> {
    match vault_state {
        VaultState::Local => {
            let remote = build_remote_backend(config, profile_id, device_id.clone())?;
            Ok(Box::new(VaultBackend::new(
                config.vault_root().to_path_buf(),
                /* manifest */ load_manifest_arc(config, &device_id)?,
                /* remote */ Some(Box::new(remote)),
                profile_id,
                device_id,
                Surface::CliLocalVault,
            )))
        }
        VaultState::Cloud => Ok(Box::new(build_remote_backend(
            config, profile_id, device_id,
        )?)),
    }
}
```

### CLI command shape after Phase 4c

```rust
pub async fn create(config: &Config, params: CreateParams<'_>) -> Result<()> {
    let cmd         = build_create_resource_cmd(&params)?;        // pure: clap → cmd
    let vault_state = VaultState::from_env();
    let backend     = dispatch::build_backend(config, vault_state, /* profile, device */)?;
    let output      = backend.create_resource(cmd).await?;
    format_create_output(&output, params.format)?;                // pure: output → terminal
    Ok(())
}
```

Three sections per write-command function: cmd build, dispatch, output format. The 2137-line `resource.rs` becomes mostly cmd-builders and output-formatters; orchestration moves into the backend impls.

## VaultBackend internals

### Per-request struct

```rust
pub struct VaultBackend {
    pub vault_root: PathBuf,
    pub manifest: Arc<ManifestManager>,      // shared manager, not shared resource
    pub remote: Option<Box<dyn Backend>>,    // None disables tail-action push (tests, vault-only mode)
    pub profile_id: Uuid,
    pub device_id: String,
    pub surface: Surface,                    // CliLocalVault
}
```

### ManifestManager — the SWMR-delegating chokepoint

VaultBackend does **not** hold `Arc<RwLock<Manifest>>` directly. Doing so would leak the manifest's mutability into every call site and force compound operations (read record → mutate → save) to rely on convention rather than construction. Instead, `ManifestManager` (in `temper-cli/src/manifest_manager.rs`) owns the `Manifest` exclusively and exposes only intent-shaped methods:

```rust
pub struct ManifestManager {
    inner: Mutex<Manifest>,                  // encapsulated, never exposed
    save_path: PathBuf,
    device_id: String,
}

impl ManifestManager {
    pub fn load(temper_dir: &Path, device_id: &str) -> Result<Self> { /* wraps manifest_io::load_manifest */ }

    // intent-shaped mutators — invariants enforced inside; addressing
    // through ResourceRef so callers don't mix UUID and slug-tuple forms.
    // Internal storage is always UUID-keyed; Scoped variants resolve via
    // the manifest's slug index as the manager's first step.
    pub fn record_local_write(&self, row: &ResourceRow, hash: &str) -> Result<()>;
    pub fn record_push_outcome(&self, r: &ResourceRef, outcome: PushOutcome) -> Result<()>;
    pub fn record_deletion(&self, r: &ResourceRef) -> Result<()>;

    // read APIs — no caller ever sees Manifest
    pub fn read_record(&self, r: &ResourceRef) -> Result<Option<ManifestRecord>>;
    pub fn snapshot_for_sync(&self) -> ManifestSnapshot;     // immutable view for bulk sync

    // slug → UUID resolution as a standalone primitive for callers that
    // want the UUID without performing an action.
    pub fn resolve(
        &self,
        owner: &str,
        context: &str,
        doctype: &str,
        slug: &str,
    ) -> Result<Option<ResourceId>>;

    // explicit save — manager flushes at end-of-command
    pub fn flush(&self) -> Result<()>;
}
```

The canonical `kb://<owner>/<context>/<doctype>/<uuid>` URI form is the **Display/serialization** of `ResourceRef::Uuid` (built via the existing `temper-core::vault::Vault::build_uri` helper) — used for event payloads, audit log entries, and structured-log fields. It is not a separate parameter type for the manager. `ResourceRef::Scoped` is the slug-form input the manager canonicalizes via its index; UUID is the internal key everything resolves to.

Three things this buys that `Arc<RwLock<Manifest>>` does not:
1. **Compound operations are atomic by construction.** `record_push_outcome` reads + mutates + (optionally) saves inside one method — no caller takes a write lock around a sequence.
2. **Manifest invariants live in one place.** Rules like "PendingPush.attempt_count must monotonically increase", "Synced state clears any prior pending_push", "deletion record cleared on successful re-create" live in the manager's methods, not in caller code.
3. **Phase 6's state machine plugs in cleanly.** New methods (`record_conflict_detected`, `record_retry_attempt`) add to the manager; backends and surfaces don't change.

The `Mutex` inside the manager is a true implementation detail. If contention ever becomes real (rare for a single-process CLI; possible in test harnesses spawning multiple `VaultBackend`s), the manager can swap to a sharded mutex or actor channel without touching VaultBackend.

`actions/sync.rs` continues to work directly with `Manifest` for now (it has its own sync-orchestration semantics that predate this manager). Phase 4 does not migrate sync.rs to the manager; that's a Phase 6 concern alongside the state machine.

### Trait impl shape (write methods)

Each write method follows a consistent decomposition. Using `create_resource` as the model:

```rust
async fn create_resource(&self, cmd: CreateResource) -> Result<CommandOutput<ResourceRow>> {
    let validated   = self.validate_create(&cmd)?;
    let path        = self.resolve_vault_path(&validated)?;
    let frontmatter = self.build_frontmatter(&validated)?;
    let row         = self.write_vault_file(&path, &frontmatter, &cmd.body).await?;
    self.manifest.record_local_write(&row, &row.content_hash)?;
    let push_outcome = self.try_tail_push_create(&row, &cmd).await;       // Result, not ?
    self.manifest.record_push_outcome(&ResourceRef::uuid(row.id), push_outcome.clone())?;
    self.manifest.flush()?;
    let events = build_events(&row, &push_outcome);
    Ok(CommandOutput::with_events(row, events))
}
```

Each helper is private and individually unit-testable:
1. **Validation** — pure functions over cmd structs (doctype, context, slug). No I/O.
2. **File I/O** — wraps existing `vault::write_note` primitive with VaultBackend-aware error mapping.
3. **Manifest mutation** — delegated to `ManifestManager`; backend code never reaches the inner `Manifest`. Invariants are enforced inside the manager, not by the backend.
4. **Tail-action push** — calls `self.remote.as_ref().map(|r| r.create_resource(cmd.clone()))`. Failure does not abort the trait method; the manager records the outcome and `build_events` emits the matching event.

### Tail-action push semantics

| Push outcome | Manifest state | Event | Trait return |
|---|---|---|---|
| `remote = None` | `Local` (never attempted) | `VaultFileWritten`, `VaultManifestUpdated` | `Ok` |
| Network/auth failure | `PendingPush { kind: Network, last_attempt_at, attempt_count: 1 }` | `PushDeferred` | `Ok` |
| 4xx server validation | `PendingPush { kind: ServerValidation { status, body }, ... }` | `PushDeferred` | `Ok` |
| 409 Conflict | `PendingPush { kind: Conflict, ... }` | `PushDeferred` | `Ok` |
| 2xx success | `Synced { server_uuid, server_hash }` | `RemoteSynced` | `Ok` |

The vault write succeeds in all cases (the local file is real). Push failure never blocks. Phase 6's state machine reads the manifest's `PendingPush` field to drive recovery.

### Events emitted

Always:
- `VaultFileWritten { path, hash }`
- `VaultManifestUpdated { resource_id, state }`

Exactly one of:
- `RemoteSynced { resource_id, server_hash }` (push succeeded)
- `PushDeferred { resource_id, reason }` (push skipped, failed, or conflicted)

### Manifest schema additions

```rust
pub struct ManifestRecord {
    // ...existing fields...
    pub pending_push: Option<PendingPushRecord>,    // NEW in Phase 4
}

pub struct PendingPushRecord {
    pub kind: PendingPushKind,
    pub last_attempt_at: DateTime<Utc>,
    pub attempt_count: u32,
}

pub enum PendingPushKind {
    Network,
    Auth,
    ServerValidation { status: u16, body: String },
    Conflict,
}
```

Adding the field bumps the manifest format version. `manifest_io::load_manifest` gains a one-way upgrade path: pre-4a manifests load with `pending_push: None` for every record. Backward-compatible read; forward-incompatible write.

### Existing module relationships

| Module | Role | Phase 4 treatment |
|---|---|---|
| `temper-cli/src/vault.rs` (186 LOC) | File-system primitives (`write_note`, `slugify`, `get_template`) | Kept intact. VaultBackend uses as a primitives library. |
| `temper-cli/src/actions/sync.rs` (4167 LOC) | Bulk sync orchestration (`sync_orchestration`, `push_one_resource`, `publish_local_write`) | Kept intact. `temper sync` commands continue to use it. VaultBackend does **not** call `publish_local_write` — it composes `RemoteBackend` directly via the trait. |
| `temper-cli/src/actions/ingest.rs` (1567 LOC) | Local-mode ingest pipeline (chunking, embedding) | Kept intact for the `local-mode-with-test-embed` test path. VaultBackend's create_resource handles file write; ingest pipeline primitives remain available. |
| `temper-cli/src/actions/frontmatter.rs` | Managed_meta + open_meta serialization | Kept intact, used as-is. |
| `temper-cli/src/manifest_io.rs` | `load_manifest`, `save_manifest` | Extended for the `pending_push` field migration. Used by `ManifestManager::load` and `ManifestManager::flush`. |
| `temper-cli/src/manifest_manager.rs` (NEW) | `ManifestManager` — SWMR-delegating wrapper around `Manifest` | Owns `Manifest` exclusively; exposes intent-shaped methods. VaultBackend holds `Arc<ManifestManager>`. |

## RemoteBackend internals

### Per-request struct

```rust
pub struct RemoteBackend {
    pub client: Client,
    pub profile_id: Uuid,
    pub device_id: String,
    pub surface: Surface,    // CliCloud
}
```

### Trait impl shape

Mechanical translation — each method ~10 lines. Wire payloads use the existing `temper-core` types that `temper-client` already serializes.

```rust
async fn create_resource(&self, cmd: CreateResource) -> Result<CommandOutput<ResourceRow>> {
    let req = translate_create_to_ingest_request(&cmd)?;       // cmd → IngestRequest
    let row = self.client.ingest(req).await?;                  // POST /api/ingest
    Ok(CommandOutput::with_events(
        row,
        vec![Event::RemoteSynced { /* ... */ }],
    ))
}
```

The `translate_*` functions are the inverse of the translators in `temper-api/src/backend/translators.rs`. One per write method (`create`, `update`, `delete`). Reads delegate to existing `client.show`, `client.list`, `client.search`.

## Sub-phase decomposition

Three sub-phases mirroring Phase 3's cadence (3a / 3b / 3c), preceded by a small prep PR for the cross-cutting `ResourceRef::Scoped` change. Each lands as its own PR.

### Phase 4-prep — `ResourceRef::Scoped` gains owner field

- Extend `temper-core::operations::resource_ref::ResourceRef::Scoped` with an `owner: String` field.
- Update every constructor and call site (HTTP handlers, MCP tools, CLI commands, sync code, tests). All current callers pass the implicit owner (`@me` for solo use; the profile's owner handle for the request).
- Strict prerequisite for both the `ManifestManager` API shape and team support.
- Lands as a small isolated PR before 4a — keeps the Backend impl work in 4a from being mixed with a cross-cutting type extension.
- Acceptance: workspace + e2e green; no behavioral change (owner today is always the caller's `@me` handle).

### Phase 4a — Foundation (dark-launched)

- Implement `VaultBackend` in `temper-cli/src/vault_backend/`.
- Implement `RemoteBackend` in `temper-client/src/backend.rs`.
- Implement `Surface::dispatch::build_backend` in `temper-cli/src/dispatch.rs`.
- Add `pending_push: Option<PendingPushRecord>` to manifest schema with backward-compatible load.
- Unit-test all 6 trait methods on both new backends.
- No callers rewired. `commands/resource.rs` unchanged.
- Workspace + `temper-cli` (with `test-db`) green; e2e baseline unchanged.

### Phase 4b — Local-mode rewire

- Rewire `commands/resource.rs::create`, `update`, `delete` `VaultState::Local` arms to dispatch through `VaultBackend` (constructed via `dispatch::build_backend`).
- Cloud arms (`VaultState::Cloud`) untouched (still hit `temper-client` directly).
- Verify `commands/resource.rs` LOC drops by ≥30% (target ~1500 from 2137; the cloud arms remain).
- All four test suites green:
  - `cargo nextest run --workspace`
  - `cargo nextest run -p temper-cli --features test-db`
  - `cargo nextest run --manifest-path tests/e2e/Cargo.toml --features test-db`
  - `cargo nextest run --manifest-path tests/e2e/Cargo.toml --features test-db,test-embed`

### Phase 4c — Surface::dispatch unification

- Collapse remaining `match VaultState` branches in `commands/resource.rs` write functions (`create`, `update`, `delete`) into single `build_backend → backend.method(cmd)` calls.
- Cloud arms now route through `RemoteBackend` (via `dispatch::build_backend`'s cloud branch) instead of direct `temper-client` calls.
- Read commands, sync commands, and `update_meta` (if not in trait) untouched.
- All four test suites green. Manual sanity check in both `local` and `cloud` `TEMPER_VAULT_STATE` modes.

## Testing strategy

### 4a — unit tests

VaultBackend (`temper-cli/src/vault_backend/tests.rs`):
1. Per-method happy paths for `create`, `update`, `delete`, `show`, `list`, `search`. Run each twice: with `remote: None` (vault-only) and with `remote: Some(MockBackend)` (full pipeline). Assert file written, manifest updated, events emitted.
2. Tail-action push failure modes — `MockBackend` returns `Network`, `Auth`, `Validation { 422 }`, `Conflict`. For each: assert `PendingPush { kind }` recorded, `PushDeferred` event emitted, command returns `Ok`.
3. `ManifestManager` invariants — `record_local_write` for new vs. existing slug, `record_push_outcome` clearing prior pending_push on Synced, monotonic `attempt_count`, mutual exclusion of `Synced` and `PendingPush`, `record_deletion` removing the record cleanly, `resolve(owner, ctx, doctype, slug) → uuid` slug-index correctness across owner boundaries. Tested independently of any backend.
4. Path resolution — `{vault_root}/{ctx}/{type}/{slug}.md`, including doctype-qualified lookups.
5. Idempotency — repeated `create_resource` with same slug matches existing `commands/resource.rs::create_simple_resource` semantics. Plan-writing pins the contract.
6. Reads — fixture vault directory via `tempfile`.

RemoteBackend (`temper-client/src/backend/tests.rs`):
- `wiremock`- or `mockito`-based tests asserting wire shape per method. Inverse-symmetry check against `temper-api/src/backend/translators.rs` shapes.

### 4b + 4c — regression target

The lesson from Phase 3b's mid-3c surprise (workspace + crate-level green ≠ e2e green; three contract regressions hid in the gap): include all four suites in the per-PR verification block, not just workspace + crate.

```bash
cargo make check
cargo nextest run --workspace
cargo nextest run -p temper-cli --features test-db
cargo nextest run --manifest-path tests/e2e/Cargo.toml --features test-db
cargo nextest run --manifest-path tests/e2e/Cargo.toml --features test-db,test-embed
```

For 4c, additionally: manual `temper resource create --type task ...` in both `TEMPER_VAULT_STATE=local` and `cloud` modes against a real Postgres. Dispatch boundary is the highest-risk point.

## Acceptance criteria

- [ ] **4-prep**: `ResourceRef::Scoped` carries `owner: String`. Every existing call site (HTTP, MCP, CLI, sync, tests) passes the caller's profile-owner handle. Workspace + four test suites green. No behavioral change.
- [ ] **4a**: `Backend` is implemented by `VaultBackend` (in `temper-cli/src/vault_backend/`) and `RemoteBackend` (in `temper-client/src/backend.rs`). `ManifestManager` lives in `temper-cli/src/manifest_manager.rs` and is the only mutator of `Manifest` reachable from `VaultBackend`. ManifestManager addressing is uniform via `&ResourceRef`; UUID is the internal storage key. Unit tests cover all 6 trait methods on both backends and `ManifestManager`'s invariants (slug-resolution, owner-aware lookup, etc.) independently. No callers rewired. `cargo make check` clean. All four test suites at the baseline.
- [ ] **4b**: `commands/resource.rs` local-mode arms for `create`, `update`, `delete` dispatch through `VaultBackend` via `dispatch::build_backend`. Cloud arms unchanged. `commands/resource.rs` LOC dropped ≥30%. All four test suites green.
- [ ] **4c**: `dispatch::build_backend` is the only construction site for backend trait objects in CLI write paths. `match VaultState` branches removed from `commands/resource.rs::create`, `update`, `delete`. Reads, sync, `update_meta` untouched. All four test suites green plus manual mode-flip sanity check.
- [ ] Manifest format upgraded with `pending_push` field. Load is backward-compatible; pre-4a manifests load with `pending_push: None`.
- [ ] Phase 6 hooks present: `PendingPushRecord` written by VaultBackend; `PushDeferred` event emitted on every non-success push outcome.

## Risks

1. **Wire-format drift between RemoteBackend and the API.** Mitigated by inverse-symmetry tests against `temper-api/src/backend/translators.rs`. If a wire-field name diverges, both sides' tests catch it.
2. **`actions/sync.rs::publish_local_write` semantics not matching `RemoteBackend.create_resource`.** Phase 4 keeps both paths; `temper sync push` continues to use `publish_local_write`, VaultBackend uses `RemoteBackend`. Drift is theoretical and surfaces under e2e if it manifests. Reconciliation deferred to Phase 6 if needed.
3. **`ManifestManager` introduces a chokepoint that didn't exist.** Today, `commands/resource.rs` mutates `Manifest` directly through `manifest_io::save_manifest`. The manager funnels every write through one type, which is good for invariants but means a manager bug affects every write path. Mitigation: extensive unit tests on `ManifestManager` itself (independent of any backend) — `record_local_write`, `record_push_outcome`, `record_deletion`, plus invariant tests (monotonic `attempt_count`, mutual exclusion of `Synced` and `PendingPush`, etc.).
4. **Manifest schema migration.** Adding `pending_push` bumps the format version. `manifest_io::load_manifest` handles backward-compatible upgrade. New writes are forward-incompatible — older `temper` binaries will fail to parse a 4a-or-later manifest. Acceptable given single-developer cadence.
5. **`update_resource_meta` not currently in the Backend trait.** If Phase 4 needs to dispatch meta-only updates through `update_resource` with a `meta_only: true` cmd flag, the existing `UpdateResource` cmd shape must support it. Plan-writing surfaces the gap if it exists.

## Migration notes

- **PR cadence:** 4a, 4b, 4c land as separate PRs to `main`. Each green per-PR per the regression target. Stacked branches optional.
- **No production callers in 4a:** dark-launch property means a 4a regression cannot reach production. 4b is the first caller-reachable PR; review attention concentrates there.
- **Phase 5 retired:** the parent spec's Phase 5 ("surface dispatch unification") is subsumed into 4c. Update the parent spec when this Phase 4 spec is committed.

## Open questions for plan-writing

These are intentionally left for the implementation plan to resolve, not the spec:

1. Exact shape of `Surface` carrying — is `surface: Surface` field on each backend used for logging/events only, or also for authorization? Both DbBackend and the new backends should agree.
2. Whether `update_resource_meta` (currently dispatched in HTTP/MCP via a meta-only `UpdateResource` cmd) needs trait-level changes for the CLI. The trait has `update_resource` only. Plan-writing reads `commands/resource.rs::update` to confirm meta-only semantics fit the existing cmd shape.
3. The `MockBackend` test fixture — fresh impl in `temper-cli/src/vault_backend/tests.rs`, or a shared fixture in `temper-core/src/operations/test_support.rs` for cross-backend reuse? Plan-writing decides based on whether DbBackend tests would also benefit.
4. `ManifestManager` save semantics — flush after every mutation, flush at end-of-command, or both? Plan-writing decides. Trade-off: per-mutation flush is safer (crash mid-command preserves all completed steps) but adds I/O; end-of-command flush is faster but loses partial work on crash. The methods exposed by `ManifestManager` allow either.
5. Whether sync.rs migrates to `ManifestManager` later (Phase 6) is intentionally deferred. Phase 4 leaves sync.rs's direct manifest access intact; the two paths coexist.

## Connections

- Parent spec: `docs/superpowers/specs/2026-05-01-shared-core-execution-paths-design.md`
- Predecessor spec: `docs/superpowers/specs/2026-05-07-wave1-phase3-dbbackend-design.md`
- Companion spec (Phase 6 state machine): `docs/superpowers/specs/2026-05-01-cloud-first-reframe-and-manifest-redefinition-design.md`
- Predecessor session: `wave-1-phase-3c-mcp-migration-complete-3b-regressions-fixed-ready-for-pr`
- Goal: `path-to-alpha`
