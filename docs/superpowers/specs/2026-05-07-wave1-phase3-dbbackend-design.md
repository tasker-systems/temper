# Wave 1 Phase 3 — DbBackend Implementation Design

**Date:** 2026-05-07
**Context:** `temper`
**Mode:** plan
**Effort:** medium (in spec; implementation splits across three sub-PRs)
**Predecessors (merged in PR #65):**
- `docs/superpowers/plans/2026-05-02-wave1-phase1-operations-scaffolding.md`
- `docs/superpowers/plans/2026-05-02-wave1-phase2-shared-actions.md`

**Parent spec:** `docs/superpowers/specs/2026-05-01-shared-core-execution-paths-design.md` (#4)
**Companion spec:** `docs/superpowers/specs/2026-05-01-cloud-first-reframe-and-manifest-redefinition-design.md` (#3 — Phase 6)
**Backlog task:** `2026-05-03-wave-1-phase-3-write-the-dbbackend-implementation-plan`

---

## Problem

Phases 1+2 landed `temper-core::operations` (commands, actions, events, `Backend` trait, `Surface`, `ResourceRef`, `CommandOutput`, `BodyUpdate`/`ListFilter`/`SearchQuery`) and the shared pure actions (`validate_*`, `apply_defaults` for `ManagedMeta`, `merge_managed_meta`, `merge_open_meta`, `ensure_managed_identity_keys`). The `Backend` trait has no impls — every existing call site still reaches into `temper-api::services::*` directly, both from Axum HTTP handlers and from MCP tools. The architectural foundation exists in code but is dark.

Phase 3 closes that loop on the DB side: implement `Backend` for a `DbBackend` struct in `temper-api`, migrate every relevant HTTP handler and every MCP tool to dispatch through it, and unify the lone remaining direct call to `apply_managed_defaults` (in `ingest_service`) onto the shared actions module.

The original parent spec carried a hard constraint that "Phase 3 must ship in the same PR as Phase 1+2." That constraint was already violated — PR #65 merged Phases 1+2 alone while the schema-driven managed-meta umbrella took priority. Phase 3 is now a follow-on, decomposed across three sub-PRs (3a/3b/3c) so each is independently shippable.

## The Reframe (Phase 3 specifics)

`DbBackend` is constructed per request: surfaces (Axum handlers, MCP tools) build it from their own auth context and dispatch one command through it. Each trait method is a thin translator — it converts an `operations::*Resource` command into the existing service function's request shape, calls the service, synthesizes one coarse `DomainEvent` on success, and returns `CommandOutput<T>`. No SQL moves. No service internals change. The migration is structure-tightening, not behavior change.

Three layers stay where the parent spec put them. Phase 3 changes only the dispatch direction across the API boundary:

```
  Surface (Axum handler / MCP tool)
        │  build per-request DbBackend { pool, profile_id, device_id, surface }
        │  build operations::*Resource command from inbound input
        ▼
  DbBackend.<method>(cmd)              ← Phase 1 trait (object-safe, async_trait)
        │  translator: cmd → service request
        ▼
  ingest_service / resource_service / search_service  ← unchanged from today
        │  (existing SQL, existing chunks/embedding pipeline, existing edges)
        ▼
  Postgres
```

## Components

### `crates/temper-api/src/backend/`

New directory:

```
crates/temper-api/src/backend/
├── mod.rs           // pub use db_backend::DbBackend;
├── db_backend.rs    // struct + impl Backend for DbBackend
├── translators.rs   // cmd → service-request mappers
└── tests.rs         // trait-impl unit tests against a real test-db
```

The directory choice (vs. a single file) reflects the 6 trait methods + per-method translators + trait-impl tests — splitting these keeps each file in the focused-size range the project favors.

### `DbBackend` struct

Per-request construction. Auth context is part of the value, not threaded through method args.

```rust
// crates/temper-api/src/backend/db_backend.rs (sketch)
pub struct DbBackend {
    pool: PgPool,
    profile_id: ProfileId,
    device_id: String,
    surface: Surface,
}

impl DbBackend {
    pub fn new(pool: PgPool, profile_id: ProfileId, device_id: String, surface: Surface) -> Self {
        Self { pool, profile_id, device_id, surface }
    }
}

#[async_trait]
impl Backend for DbBackend { /* 6 methods */ }
```

Surfaces build it once per inbound call:

- **HTTP handler (Axum):** `DbBackend::new(state.pool.clone(), auth.0.profile.id.into(), device_id, Surface::ApiHttp)` — `device_id` from the `Extension<DeviceId>` already extracted by middleware.
- **MCP tool:** `DbBackend::new(state.pool.clone(), profile_id_from_jwt, "mcp".to_string(), Surface::Mcp)`.

Constructing per request keeps the `Backend` trait signature exactly as Phase 1 defined it (no auth-context method parameter, no command-struct auth fields), aligns with how Axum extractors already produce request-scoped values, and matches `feedback_sql_query_patterns`'s spirit ("compose, don't enumerate") — surfaces compose a backend from their primitives instead of the trait enumerating all auth-shape variations.

### Trait method → service mapping (canonical)

| Trait method | Wraps | Translator notes |
|---|---|---|
| `create_resource` | `ingest_service::ingest` | `CreateResource → IngestPayload`. Builds `IngestPayload { context_name, doc_type_name, slug, title, content, managed_meta, open_meta, content_hash: None, chunks_packed: None, origin_uri: "" }` and lets `ingest` run defaults + identity injection + validate + pipeline + dedupe. Body bytes come from `cmd.body.as_ref().map(|b| b.content.clone()).unwrap_or_default()`. |
| `show_resource` | `resource_service::get_visible` (Uuid) / `resource_service::get_by_slug` (Scoped) | Branches on `ResourceRef`. `get_by_slug` already handles the scoped case end-to-end. |
| `update_resource` | `resource_service::update` | `UpdateResource → ResourceUpdateRequest`. Resolves `ResourceRef` to a `resource_id` first (via `get_visible`/`get_by_slug` for the resolve), then forwards the partial-merge payload. Body trio derived from `cmd.body.is_some()`. |
| `delete_resource` | `resource_service::delete` | Resolves `ResourceRef` to `resource_id`, then forwards. The `force` flag is a CLI-side concern (TTY confirmation); DbBackend ignores it. |
| `list_resources` | `resource_service::list_visible` | `ListFilter → ResourceListParams`. |
| `search_resources` | `search_service::search` | `SearchQuery → SearchParams`. |

The siblings — `resource_service::create` (thin SQL insert behind `POST /api/resources`) and `ingest_service::update` (re-ingest path used by sync-pull machinery) — stay live in 3a/3b/3c and are addressed by an explicit follow-up cleanup task captured below in *Forward-Looking Constraints*. They are not "for now" workarounds — Phase 3's design picks the canonical mapping unambiguously, and the siblings exist on a known-finite leash.

### Coarse post-hoc event emission

Each `DbBackend` method synthesizes one event on success from the service return value:

- `create_resource` → `DbResourceCreated { resource_id: row.id }`
- `update_resource` → `DbResourceUpdated { resource_id: row.id }`
- `delete_resource` → `DbResourceSoftDeleted { resource_id }`
- `show_resource`, `list_resources`, `search_resources` → empty events vec

Fine-grained body-changed events (`DbChunksGenerated`, `DbEmbeddingTriggered`) are deferred to Phase 6, where the state-machine work in companion spec #3 wires events at the moment they actually happen inside the ingest pipeline. Phase 3's coarse events compose with — they do not replace — the future fine-grained set, and they map 1:1 onto the `DbBackend Lifecycle` transitions in companion spec #3 §3.1, so Phase 6 can refine without rewriting Phase 3's emission sites.

### Phase-2 follow-on: `apply_defaults_value`

Add to `crates/temper-core/src/operations/actions.rs`:

```rust
/// Apply managed-tier doctype defaults to a `serde_json::Value` in place.
/// Sibling to `apply_defaults` for callers that operate on `Value` directly
/// (e.g. ingest_service's pre-validation pipeline). Both functions are thin
/// wrappers over `crate::defaults::apply_managed_defaults`.
pub fn apply_defaults_value(doctype: &str, meta: &mut serde_json::Value) {
    crate::defaults::apply_managed_defaults(doctype, meta);
}
```

Re-export through `operations/mod.rs`. Then:

- `crates/temper-api/src/services/ingest_service.rs:403` — replace `apply_managed_defaults(&payload.doc_type_name, &mut managed)` with `temper_core::operations::apply_defaults_value(&payload.doc_type_name, &mut managed)`.
- `crates/temper-api/src/services/ingest_service.rs:655` — same replacement.
- Drop the `use temper_core::defaults::apply_managed_defaults` line in favor of the existing `temper_core::operations` import already present in that file.

After this, `temper-api` has no direct `temper_core::defaults::*` calls — operations is the canonical entry. The acceptance criterion in the parent spec ("operations::apply_defaults is the only path applying doctype defaults in temper-api") is met, with the function-name discrepancy resolved (the parent spec referenced `apply_doc_type_defaults`, which never existed under that name; the real function is `apply_managed_defaults` and the canonical operations entry is now `apply_defaults_value`).

### Auth, profile scoping, and existing rules — preserved

- **Auth before writes** (CLAUDE.md rule, `feedback_sql_query_patterns`): preserved unchanged. `resource_service::update` still calls `can_modify_resource` before any mutation; `delete` likewise; `ingest_service::ingest` operates only against the caller's `profile_id`.
- **Profile scoping**: every query continues to scope through `resources_visible_to` / `can_modify_resource`. `DbBackend` forwards the request-scoped `profile_id` into existing service signatures unchanged.
- **Service layer owns SQL**: `DbBackend` itself contains no SQL — every SQL path stays in `services/`. Translators are pure data-shape adapters.
- **Schema-required defaults at create/update** (CLAUDE.md): `apply_defaults_value` and existing `ensure_managed_identity_keys` call sites in `ingest_service`, `resource_service`, `meta_service` (Phase 5 symmetric defense) all preserved.
- **Typed structs over inline JSON**: translators use typed fields. `cmd.managed_meta: ManagedMeta` round-trips into the JSON shape only at the service boundary (where service signatures already require `Value`).
- **Params structs** (`feedback_no_premature_backward_compat`'s sibling rule): `DbBackend::new` takes 4 params (pool, profile_id, device_id, surface) — at the project's threshold. If a fifth field is added during 3a, introduce `DbBackendCtx`.

### Sub-phase decomposition

One spec, three plans, three PRs. Each is independently shippable.

**3a — Foundation (next session, written from this spec).** Lands the architectural foundation; no behavior change visible from any surface.
- Add `crates/temper-api/src/backend/{mod,db_backend,translators,tests}.rs`.
- `DbBackend` struct + all 6 trait method impls (delegating to existing services + synthesizing coarse events).
- Add `temper_core::operations::apply_defaults_value`; migrate the two `ingest_service` call sites.
- Trait-impl unit tests in `backend/tests.rs` against a `test-db` Postgres: each method exercised once happy-path + one error path, plus the object-safe-`dyn Backend` smoke test promoted from Phase 1.
- `cargo make check`, `cargo make test`, `cargo make test-db` clean.
- No HTTP handlers rewired; no MCP tools rewired. The trait is dark-launched, verified by trait-impl tests.

**3b — HTTP handler migration (subsequent session).** Mechanical rewire of the 7 handlers in `crates/temper-api/src/handlers/{resources,ingest,search}.rs`. Each handler builds a `DbBackend` from `state.pool` + `auth` + `device_id` + `Surface::ApiHttp` and dispatches one command through it. Existing integration tests in `crates/temper-api/tests/*.rs` and `tests/e2e/` pass unmodified — that pass-without-modification is the regression guard.

**3c — MCP tool migration (subsequent session).** Same pattern in `crates/temper-mcp/src/tools/resources.rs`: each tool constructs `DbBackend` from its in-process state with `Surface::Mcp` and dispatches. The existing `ensure_managed_identity_keys` send-side wiring stays as-is (Phase 5 symmetric defense — DbBackend is on the receive side of MCP). Round-trip tests in `tests/e2e/` (under the embed-gated CI job) pass unmodified.

Each sub-phase is a separate plan document under `docs/superpowers/plans/`, written when its session begins.

## Forward-Looking Constraints & Inherited Guidance

This section makes the "out of scope" boundary explicit so the constraints we already developed don't get dropped. Phase 3 is **not** allowed to make any of these harder.

### Phase 4 (VaultBackend) — must be able to mirror Phase 3's shape

Phase 4 lifts the 2125-line `crates/temper-cli/src/commands/resource.rs` into a `crates/temper-cli/src/vault_backend/` module that implements the same `Backend` trait against vault-file persistence. Phase 3's design choices set the template:

- The per-request struct shape (`DbBackend { pool, profile_id, device_id, surface }`) gives `VaultBackend { vault_root, manifest, client, surface }` an obvious analogue.
- The translator pattern (cmd → backend-native request) gives `VaultBackend` a place to put cmd → file-write/manifest-update orchestration.
- Coarse post-hoc events give `VaultBackend` the contract for `VaultFileWritten`, `VaultManifestUpdated`, `RemoteSynced`, `PushDeferred`.
- For the `CliLocalVault` push-as-tail-action chain, `VaultBackend` will call `DbBackend` *via `temper-client` over HTTP*. Phase 3's `DbBackend` idempotency and dedupe semantics — specifically `find_by_body_hash` short-circuit in `ingest_service::ingest` — are load-bearing for sync-recovery from `PendingPush`. Preserved.

### Phase 5 (Surface dispatch unification) — preserved by per-request construction

Phase 5 collapses `commands/resource.rs`'s `match VaultState` branches into a single `Surface::dispatch` that picks `VaultBackend` or invokes `temper-client` (which hits API → `DbBackend`). Phase 3's per-request `DbBackend` shape supports this directly: the dispatch site picks one backend and calls trait methods. No retrofit needed.

### Phase 6 (state machines, fine-grained events, manifest narrowing — companion spec #3)

Phase 6 wires the lifecycle state machines in companion spec #3:

- `DbBackend` lifecycle is `Active(N)` → `Active(N+1)` per command. **Phase 3's coarse events `DbResourceCreated/Updated/SoftDeleted` are exactly the transition triggers companion spec #3 §3.1 specifies.** They do not get replaced in Phase 6 — they get augmented with body-changed sub-events (`DbChunksGenerated`, `DbEmbeddingTriggered`) and conflict events.
- Server-side versioning is unchanged — existing optimistic-lock pattern (last-write-wins at the SQL transaction level, plus `can_modify_resource` profile scoping). Phase 3 does not touch versioning semantics.
- **Phase 3 must preserve existing conflict-detection behavior on the server side.** `VaultBackend`'s `Conflicting` state in companion spec #3 §3.2 depends on `DbBackend` returning a conflict on stale-base-hash pushes. The current ingest path doesn't accept a base-hash parameter; this is a Phase-6 wire-in, not a Phase 3 task — but Phase 3 shouldn't introduce any change that makes adding it harder (e.g., don't lose the ability to thread a `base_hash` argument from cmd into the ingest path; the `IngestPayload.content_hash` field is the existing channel and stays).
- Manifest narrowing (companion spec #3 §5) does not affect `DbBackend` — the manifest is a `VaultBackend` concern.

### Out-of-scope cleanups (high-priority follow-ups, not vague-future)

Per `feedback_no_premature_backward_compat` (project is one month old; remove dead code rather than keeping it "for compat") — the divergent siblings get explicit cleanup tasks, not indefinite leashes:

1. **Deprecate `resource_service::create`.** It's used only by the `POST /api/resources` handler and a handful of `crates/temper-api/tests/*.rs` tests that exercise that endpoint. After 3b, migrate the handler to dispatch through `DbBackend.create_resource` (which routes to `ingest_service::ingest`). The endpoint's surface stays the same; the body-less / managed-meta-less callers will start exercising the full ingest path with empty inputs (which is harmless — defaults apply, dedupe is a no-op when there's no body). Once handler + tests confirm parity, delete `resource_service::create`.
2. **Investigate `ingest_service::update` redundancy with `resource_service::update`.** Phase 3a's plan-writer should grep its callers (likely sync-pull machinery) and decide whether it's a genuinely separate path or a duplicate. If duplicate, plan a follow-up to collapse it.

Both cleanups become backlog tasks at the moment 3a's plan is written (so the task list reflects the deferred work).

### Inherited guidance for implementer subagents (3a/3b/3c plans)

The 3a/3b/3c plan documents must embed these rules in implementer prompts (per `feedback_subagent_check_before_commit`, `feedback_subagent_escalate_not_soften`, `feedback_plan_regression_guard_after_filter_test`):

- **Run `cargo make check` before claiming work complete.** Pre-commit hook is a backstop, not first line.
- **Pair every filter-by-name test run with a full crate suite run before commit.** A passing single test does not imply a passing crate.
- **Embed-gated tests:** when 3b or 3c touches push-body / ingest-pipeline code, run with `--features test-db,test-embed` locally to match CI's Embed job (`cargo nextest --manifest-path tests/e2e/Cargo.toml --features test-db,test-embed`).
- **Don't trust nextest's per-binary `Summary` line** with `--no-fail-fast` (`feedback_nextest_summary_lies`). Trust exit code or grep for `error: test run failed` / `FAIL [`.
- **Escalate, don't soften.** If passing a test requires loosening an error path or a contract, STOP and report BLOCKED.
- **No "for now" workarounds.** If `DbBackend.delete_resource` discovers `cmd.force` has a real semantic on the server side that wasn't anticipated here, capture it as a task — don't ship a TODO comment.
- **Verify named APIs before dispatch.** Per `feedback_pre_propose_arch_review` and `feedback_plan_verification`: every API name in the 3a/3b/3c plans gets grep-verified against the current code at dispatch time. The plan is a hypothesis; the code is ground truth.

## Acceptance Criteria

- [ ] `crates/temper-api/src/backend/` exists with `DbBackend` impl of `Backend` covering all 6 trait methods, each delegating to existing services and emitting coarse events.
- [ ] All 7 HTTP handlers in `crates/temper-api/src/handlers/{resources,ingest,search}.rs` dispatch through `DbBackend` (lands in 3b).
- [ ] All MCP tools in `crates/temper-mcp/src/tools/resources.rs` dispatch through `DbBackend` (lands in 3c).
- [ ] `temper_core::operations::apply_defaults_value(&str, &mut Value)` exists; both ingest_service call sites migrated; `temper-api` has no direct `temper_core::defaults::*` calls.
- [ ] Trait-impl unit tests in `crates/temper-api/src/backend/tests.rs` cover happy-path + one error path per method against `test-db`.
- [ ] All existing `cargo make test`, `test-db`, `test-e2e` (with `test-embed` feature) suites pass at every sub-phase boundary.
- [ ] `temper-cli` has no new dep on `temper-api` (verifies the dep-graph constraint is preserved).
- [ ] Backlog tasks created for: (a) deprecate `resource_service::create`; (b) investigate `ingest_service::update` redundancy.
- [ ] CLAUDE.md updated: "service layer owns operations" rule rewritten as "operations layer (`temper-core/operations/`) defines commands; backends implement; surfaces adapt." (May land in 3c or as a follow-on doc commit.)

## Out of Scope (with cross-references to where each lives)

- VaultBackend impl — Phase 4.
- Surface dispatch unification — Phase 5.
- Lifecycle state machines, conflict semantics, manifest narrowing, fine-grained body-changed events — Phase 6 / companion spec #3.
- Auth/authz refactor — preserved unchanged; existing plumbing consumed as parameters.
- Surface output formatting refactor — each surface keeps its current output style.
- Splitting `temper-services` out of `temper-api` so `temper-mcp` can drop its `temper-api` dep — future architectural unit per parent spec.

## Open Questions

- **`ingest_service::update` callers** — confirmed redundant with `resource_service::update`, or genuinely separate? Plan-writer for 3a greps and decides; if separate, plan documents why and `update_resource` mapping may need a second variant.
- **`base_hash` threading for Phase 6 conflict-on-push** — should 3a anticipate this by defining `IngestPayload.content_hash`'s role in `cmd_to_ingest_payload` translation, or is it fully Phase 6's problem? Default: fully Phase 6's problem; Phase 3's translator passes `None` for `content_hash` and `chunks_packed` (let `ingest_service` recompute), which is what the existing `POST /api/ingest` callers already do.
