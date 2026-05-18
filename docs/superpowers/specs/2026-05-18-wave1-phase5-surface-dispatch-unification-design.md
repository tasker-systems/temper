# Wave 1 Phase 5 — Surface Dispatch Unification Design

**Date:** 2026-05-18
**Context:** `temper`
**Mode:** plan
**Effort:** medium (decomposed into 5a/5b/5c sub-phases; single PR)
**Predecessors (merged):**
- Phase 1+2 (PR #65): `temper-core::operations` scaffolding + shared actions
- Phase 3a (PR #69) / 3b+3c (PR #71): `DbBackend` foundation + HTTP/MCP write dispatch
- Phase 4a (PR #77): `VaultBackend` foundation, dark-launched
- Phase 4 completion (PR #80): `commands/resource.rs::{create,update,delete}` Local-mode arms migrated through `VaultBackend`; per-doctype write pull-in; C1/C2 helper deletions
- Event substrate foundations (PR #81): `temper-events` crate, `event_substrate` schema

**Parent spec:** `docs/superpowers/specs/2026-05-01-shared-core-execution-paths-design.md` (#4)
**Sibling specs:**
- `docs/superpowers/specs/2026-05-11-wave1-phase4-vaultbackend-design.md` (Phase 4)
- `docs/superpowers/specs/2026-05-07-wave1-phase3-dbbackend-design.md` (Phase 3)

---

## Problem

Five write surfaces in `crates/temper-cli/src/commands/` still fork on `VaultState::from_env()`:

| Surface | Forks at | Cloud arm behavior |
|---|---|---|
| `commands/resource.rs::create` | line 207 | builds `IngestPayload`; POSTs `/api/ingest` |
| `commands/resource.rs::update` | line 1617 | calls `cloud_mode_update` → PATCH `/api/resources/{id}` |
| `commands/resource.rs::delete` | line 794 | cloud-first DELETE then (in Local) local file removal |
| `commands/session.rs::save` | line 246 | inline cloud-mode save logic |

And a second class of write subcommands has **no `match VaultState` at all** — they call `actions::*` directly, which is local-only:

| Subcommand | Action call | Cloud behavior today |
|---|---|---|
| `temper task done <slug>` | `actions::task::done` | local-only; no cloud equivalent |
| `temper task move-to <slug>` | `actions::task::move_task` | local-only |
| `temper goal create <slug>` | `actions::goal::create` | local-only |
| `temper goal update <slug>` | `actions::goal::update` | local-only |
| `temper research save <slug>` | `actions::research::save` (existing wrapper) | mode-aware but inconsistent |
| `temper research finish <slug>` | `actions::research::finish` | local-only |

The result is a two-axis asymmetry:

1. **Within `resource.rs`:** the Local arm (now ~5 lines, dispatching through `VaultBackend`) sits next to a Cloud arm of ~40 lines that builds wire types inline. Adding a feature means landing it twice.
2. **Across surfaces:** `resource.rs` works in both modes; `task.rs`/`goal.rs`/`research.rs` work only in local mode. Cloud-mode users hit "command not implemented" surprises on the dedicated subcommands.

Phase 4 closed the loop on the **local-side trait abstraction** (`VaultBackend` exists and works for all 6 doctypes). Phase 5 closes it on the **mode-selection** half: introduce `CloudBackend` as a second `Backend` impl, route every surface through a uniform 3-line `match VaultState` dispatcher, and wire the local-only subcommands into the dispatcher so they work in cloud mode for free.

## The Reframe

`CloudBackend` is constructed per inbound CLI invocation, mirrors `VaultBackend`'s shape, and implements the same `Backend` trait. Each trait method translates the `temper-core::operations::commands::*` cmd into the appropriate wire type (`IngestPayload`, `ResourceUpdateRequest`, delete args), calls `temper-client`, and projects the wire response back into `CommandOutput<ResourceRow>`.

```
  Surface (clap parser — commands/{resource,task,goal,session,research}.rs)
        │  build operations::*Resource command from inbound clap args
        │  let backend: Box<dyn Backend> = match VaultState::from_env() {
        │      Local => Box::new(VaultBackend::new(...)),
        │      Cloud => Box::new(CloudBackend::new(...)),
        │  };
        ▼
  backend.<method>(cmd).await
        │
        ├─ VaultBackend (Phase 4) — vault file + manifest + push-as-tail-action
        └─ CloudBackend (Phase 5) — cmd → wire → temper-client → CommandOutput
                                          │
                                          └─→ HTTP API → DbBackend (Phase 3)
```

The dispatch direction across the CLI boundary changes; the cloud-mode semantics (cloud-first delete ordering, partial-merge PATCH, schema-required defaults) are preserved verbatim — they live inside `CloudBackend`'s trait methods instead of inline in surfaces.

## Architectural Decisions

### D1. `Box<dyn Backend>` for mode selection

Each surface acquires a backend via a single `build_backend` helper:

```rust
pub(crate) fn build_backend(
    config: &Config,
    ctx: &str,
) -> Result<(tokio::runtime::Handle, Box<dyn Backend>)> {
    match VaultState::from_env() {
        VaultState::Local => {
            let (runtime, backend_ctx) = assemble_vault_backend(config, ctx)?;
            Ok((runtime, Box::new(VaultBackend::new(backend_ctx))))
        }
        VaultState::Cloud => {
            let cloud_ctx = assemble_cloud_backend(config, ctx)?;
            let runtime = runtime::current();
            Ok((runtime, Box::new(CloudBackend::new(cloud_ctx))))
        }
    }
}
```

**Why `Box<dyn>` over enum wrapper or generics:**
- The `Backend` trait was made object-safe in Phase 1 specifically for runtime dispatch.
- `async-trait` already boxes futures, so the per-call allocation is unchanged. The `Box<dyn>` itself is one additional allocation at surface construction — negligible for CLI processes that run once.
- Generics can't solve runtime selection: the match arms produce different concrete types. Pushing the match into call sites is what we have today and what Phase 5 is removing.
- An `enum Backends { Vault(VaultBackend), Cloud(CloudBackend) }` would avoid the allocation but require ~30 lines of `impl Backend for Backends` delegation boilerplate for zero measurable benefit in CLI usage.

### D2. CloudBackend module structure mirrors VaultBackend

```
crates/temper-cli/src/cloud_backend/
  mod.rs            // pub re-exports of CloudBackend, CloudBackendCtx, assemble_cloud_backend
  cloud_backend.rs  // struct CloudBackend + impl Backend for CloudBackend
  translators.rs    // pure cmd → wire functions; unit-tested
  ctx.rs            // CloudBackendCtx { client: Arc<Client>, owner, surface, profile_id, ... }
```

The translators surface intentionally mirrors `vault_backend/translators.rs` so the same patterns (partial-merge handling, `move_to → managed_meta` synthesis from PR #79, body-trio handling) carry over.

### D3. Feature gating

`CloudBackend` is `#[cfg(feature = "embed")]` (matches today's cloud-mode arm gating). The non-embed build path provides a stub `CloudBackend` whose every method returns `TemperError::BadRequest("cloud mode requires --features embed")` — same fallback shape today's cloud-mode arms use when embed is off. `build_backend` itself stays ungated; the cfg lives inside the Cloud arm.

### D4. Event shape and surface rendering

VaultBackend emits `DomainEvent::VaultFileWritten { path }`, `RemoteSynced { resource_id }`, `PushDeferred { reason }`. CloudBackend emits only `RemoteSynced { resource_id }` — there is no local file.

**Rendering uses an events-aware helper** so user-visible CLI output stays exactly as it is today:

```rust
fn render_write_success(verb: &str, output: &CommandOutput<ResourceRow>) {
    let rel_path = output.events.iter().find_map(|e| match e {
        DomainEvent::VaultFileWritten { path } => Some(path.as_str()),
        _ => None,
    });
    match rel_path {
        Some(p) => output::success(format!("{verb}: {p}")),     // local mode — preserves today's "Updated: <rel_path>" output
        None    => output::success(format!("{verb}: {slug}", slug = output.value.slug.as_deref().unwrap_or("(no slug)"))),  // cloud mode — slug-based
    }
}
```

The helper is mode-implicit: it switches on the presence of `VaultFileWritten` in events, not on `VaultState`. Local mode renders "Updated: <rel_path>" verbatim (matches today). Cloud mode renders "Updated: <slug>" (matches today's cloud-arm output). This is a centralization refactor across 4 spots in `resource.rs` + 1 spot in `session.rs::save`, not a user-visible behavior change.

### D5. Save-or-update overload stays surface-level

`temper session save` and `temper research save` carry a save-or-update overload (create if absent, update if present). Per CLAUDE.md and the 4b plan's established pattern, this overload lives in the surface, not the backend:

```rust
// in commands/session.rs::save
let exists = match VaultState::from_env() {
    VaultState::Local => manifest_lookup_session_for_date(...)?.is_some(),
    VaultState::Cloud => client.resources().resolve_by_uri(...).await.is_ok(),
};

let backend = build_backend(config, &ctx)?;
if exists {
    backend.update_resource(update_cmd).await?
} else {
    backend.create_resource(create_cmd).await?
}
```

Reads stay service-direct (parent spec rule). The backend trait stays write-focused.

## Architecture Diagram (post-Phase 5)

```
┌──────────────────────────────────────────────────────────────┐
│ Surface — commands/{resource,task,goal,session,research}.rs  │
│                                                              │
│   clap args                                                  │
│      │                                                       │
│      ▼                                                       │
│   build_*_resource_cmd(args) -> CreateResource | Update | ..│
│      │                                                       │
│      │   build_backend(config, ctx) -> Box<dyn Backend>     │
│      ▼                                                       │
│   backend.<method>(cmd).await -> CommandOutput<ResourceRow> │
│      │                                                       │
│      ▼                                                       │
│   render_create_output / discovery_event / output::success  │
└──────────────────────────────────────────────────────────────┘
                              │
                ┌─────────────┴─────────────┐
                ▼                           ▼
        ┌───────────────┐           ┌──────────────────┐
        │ VaultBackend  │           │ CloudBackend     │
        │ (Phase 4)     │           │ (Phase 5, new)   │
        │               │           │                  │
        │ • Vault IO    │           │ • cmd → wire     │
        │ • Manifest    │           │ • temper-client  │
        │ • tail push   │           │ • wire → row     │
        └───────────────┘           └──────────────────┘
                                            │
                                            ▼
                                    HTTP API → DbBackend
                                          (Phase 3)
```

## Scope

**In scope:**
- New `crates/temper-cli/src/cloud_backend/` module.
- `CloudBackend` struct + `impl Backend for CloudBackend`.
- `cloud_backend/translators.rs`: pure `cmd → wire` functions.
- `assemble_cloud_backend` constructor (mirror of `assemble_vault_backend`).
- `build_backend` helper at `crates/temper-cli/src/backend_select.rs` (sibling of `vault_backend/` and `cloud_backend/`, neither owns the selector).
- Migrate `match VaultState` in: `commands/resource.rs::{create, update, delete}`, `commands/session.rs::save`.
- Route through `build_backend`: `commands/task.rs::done`, `commands/task.rs::move_task`, `commands/goal.rs::create`, `commands/goal.rs::update`, `commands/research.rs::save`, `commands/research.rs::finish`.
- Refactor surface rendering to source primary message from `output.value` (5 spots).

## Non-Goals

- **Read paths** (`show`, `list`, `search`, `show_via_api_fallback`) — parent spec says service-direct; reading remains surface-direct in both modes.
- **`VaultState::from_env()` removal** — still used by read paths and by mode-gate commands (`auth.rs`, `sync_cmd.rs`). Phase 6 territory.
- **`temper-core::operations::body::prepare_body_trio` lift** — still tracked as a separate small task. Phase 5 keeps the duplication.
- **Mode-gate commands** (`commands/auth.rs::login` if Cloud, `commands/sync_cmd.rs::run` if Cloud) — their `matches!(VaultState::from_env(), VaultState::Cloud)` patterns are gates that error out, not write dispatch. Not in scope.
- **New HTTP routes** — CloudBackend translates to the routes that already exist.
- **Surface trait abstraction** — surfaces stay as plain functions on `commands/*` modules; the unification is at the backend selector, not at the surface signature.

## Decomposition

**Single PR** (`jct/wave1-phase5-surface-dispatch-unification`), three sub-phases:

| Sub-phase | Scope | Tasks (rough) | Risk |
|---|---|---|---|
| **5a — CloudBackend foundation** | Module, ctx, translators (unit-tested), Backend impl. Dark-launched — no surface callers yet. | ~6 TDD tasks | Low. Pattern is well-established from 4a. |
| **5b — Resource & session surface migration** | `build_backend` helper. Collapse `match VaultState` in `resource.rs::{create,update,delete}` and `session.rs::save`. Refactor output rendering to be value-first. | ~5 tasks | Medium. Existing e2e tests are the regression guard; surface-rendering changes need careful audit of test assertions. |
| **5c — Local-only subs cloud-enabled** | Route `task.rs::{done,move_task}`, `goal.rs::{create,update}`, `research.rs::{save,finish}` through `build_backend`. Adds cloud-mode capability where none existed. | ~6 tasks | Medium-high. These subcommands have surface-specific logic (e.g. `task done` finds the task first, then updates its stage); each one needs an audit pass to confirm the cmd shape it should produce. |

## Test Plan

### Unit tests (5a)
- `cloud_backend/translators.rs`: ~10 tests mirroring `vault_backend/translators.rs`:
  - `cmd_to_ingest_payload_round_trips_managed_meta`
  - `cmd_to_ingest_payload_includes_body_trio_when_present`
  - `cmd_to_resource_update_request_handles_partial_merge`
  - `cmd_to_resource_update_request_synthesizes_managed_meta_from_move_to` (port the 4b move-to-wire test)
  - `cmd_to_resource_update_request_omits_unset_fields`
  - `cmd_to_delete_args_extracts_uuid_from_resource_ref`
  - regression guards for the body-trio dedup paths

### Integration regression (5b + 5c)
- Existing `tests/e2e/` cloud-mode tests are the regression guard:
  - `tests/e2e/tests/cloud_mode_*.rs` (whichever files cover create/update/delete via cloud mode)
  - Any test that runs `TEMPER_VAULT_STATE=cloud temper resource create ...` exercises the new CloudBackend path on the new branch.
- For task/goal/research cloud-enablement (5c), add a small e2e per subcommand:
  - `temper task done <slug>` under cloud mode succeeds + updates the server row.
  - `temper goal create <slug>` under cloud mode succeeds.
  - `temper research finish <slug>` under cloud mode succeeds.
  - These e2e additions are the only "new" tests outside 5a translators.

### Embed-gate
- All cloud-mode tests run under the existing `--features test-db,test-embed` CI job.
- Non-embed builds (`cargo build -p temper-cli` bare) verify CloudBackend's stub returns the documented "embed required" error rather than panicking.

## Risk Register

| Risk | Mitigation |
|---|---|
| **Surface-rendering regression** — centralizing "Updated: …" into `render_write_success` could change the exact string if the helper's event-presence switch is wrong. | Helper picks rel_path when `VaultFileWritten` is present (local), slug otherwise (cloud). Audit e2e assertions for exact message text after migration. Both modes' current strings must round-trip unchanged. |
| **save-or-update exists-check perf** — adding a network round-trip before every cloud-mode `session save` is real latency. | Acceptable for v1; users running `temper session save` aren't latency-sensitive. Add a fast-path local hash-check before the network call if telemetry shows it matters. |
| **Cloud-enabled task/goal subcommands diverge from server semantics** — e.g., `temper task done` updates `temper-stage` to "done", but the server may have richer state-machine rules. | 5c per-subcommand audit confirms each cmd shape matches what the server expects. If divergence surfaces, escalate per `feedback_subagent_escalate_not_soften`. |
| **Workspace feature unification** — `cargo nextest --workspace` activates `ingest-pipeline`, exercising paths standalone runs miss (same risk surfaced in 4a, 4b). | Run `cargo nextest --manifest-path tests/e2e/Cargo.toml --features test-db,test-embed` as the end-of-branch regression gate. |
| **DomainEvent enum incompat** — CloudBackend emits `RemoteSynced { resource_id }`; if the variant shape differs from what surfaces expect, surfaces break silently. | The variant already exists per VaultBackend's current emit (PR #80). Translators tests assert exact variant shape. |

## Connections

- **Predecessor:** Phase 4 completion (PR #80) — established VaultBackend, per_doctype, render_create_output.
- **Follow-on:** Phase 6 — Surface trait abstraction or removal of `VaultState::from_env()` from read paths. Out of Phase 5 scope.
- **Adjacent:** body-trio temper-core lift (separate vault task) — independent of Phase 5; can land before, during, or after.
- **Vault task:** `wire-valid-task-modes---valid-task-efforts-to-schema-enums--ssot` — separate, plan-mode; cleanup of the `VALID_TASK_MODES`/`VALID_TASK_EFFORTS` constants in `temper-core/operations/actions.rs`.

## Execution Pattern

This spec's implementation plan will use **subagent-driven-development** with two project-specific deviations from the default flow:

1. **Per-task tests run only what the task wrote.** Implementer subagents run the tests they wrote in the task + `cargo make check`, not the full crate suite. Full crate + workspace + e2e suites run once at end-of-branch (PR-prep time), not per task. Matches `feedback_workspace_tests_at_pr_only` in user memory.
2. **Code review happens on subagent task resolution, not inline.** Implementer subagents do not invoke a reviewer subagent. Pete reviews each task's diff after the implementer reports complete; the next task dispatches only after Pete's signoff. Final opus code review at end-of-branch as usual.

This pattern keeps subagent budget focused on writing code, defers integration validation to where it's authoritative (CI + end-of-branch), and keeps the human in the loop where judgment matters most (task-boundary review).

## Plan-Writer Checklist

When this spec graduates to an implementation plan:

- [ ] Grep-verify every API name referenced (`Backend` trait methods, `temper-client` method names, `IngestPayload` field shape, `ResourceUpdateRequest` field shape).
- [ ] Confirm `async-trait` macro is still in use on `Backend` (and not stabilized `async fn in trait`) before planning the impl.
- [ ] Audit per-subcommand cmd shape for 5c (each of `task::done`, `task::move_task`, `goal::create`, `goal::update`, `research::save`, `research::finish` produces a different `UpdateResource` or `CreateResource` shape).
- [ ] Decide commit boundaries: dark-launch foundation as one commit, surface migration per-surface as separate commits, or all-in-one. (Recommendation: foundation + 1 commit per surface migrated.)
- [ ] Include the execution-pattern deviations from "Execution Pattern" above in the plan's conventions section, baked into every subagent prompt.
