# Wave 1 Phase 4 Completion — B5b + C1 + C2 Spec Addendum

**Date:** 2026-05-13
**Branch:** `jct/wave1-phase4-completion`
**Predecessors:**
- Spec: `docs/superpowers/specs/2026-05-11-wave1-phase4-vaultbackend-design.md`
- Plan: `docs/superpowers/plans/2026-05-13-wave1-phase4-completion.md`
- Last session note: `phase-4-completion-phase-b-b1-b5a-landed--per-doctype-dispatch-task-closed`

## Purpose

Resolve B5's open question (per-doctype create-validation placement) and lock the implementation strategy for B5b, C1, and C2 before subagent dispatch. The parent plan recommended option (c) (operations-layer validation) and the prior session recommended option (d) (surface-side validation); this addendum specifies a two-layer hybrid that captures (c)'s shared semantics without dragging filesystem-walker concerns into the operations contract.

## Scope

- **B5b** — collapse `commands/resource.rs::create` Local-arm to uniform `VaultBackend::create_resource` dispatch for all 6 doctypes.
- **C1** — inline `actions::frontmatter::build_managed_meta_for_create` at 3 callers; delete helper + `NewResourceArgs`.
- **C2** — migrate 6 `resolve_resource_id` callers to `client.resources().resolve_by_uri(...)`; delete function.

Out of scope: migrating `commands::session::save`, `commands::research::save`, `commands::goal::create` themselves to backend-dispatch. Those remain Phase 5 work; B5b only stops calling them from `resource.rs::create`.

## Validation Strategy — Two-Layer Hybrid

### Layer 1: pure invariants in operations

Add `temper_core::operations::actions::validate_create_pure(cmd: &CreateResource) -> Result<()>` that performs all I/O-free checks. Branching on doctype happens via a `match` on the typed `DocType` enum (parsed once via `DocType::from_str(&cmd.doctype)?` at entry) — string literal comparisons like `"task"` below are spec-shorthand for the corresponding `DocType::Task` arm; the implementation uses the enum.

- mode whitelist (`plan` / `build`) for `DocType::Task`
- effort whitelist (`small` / `medium` / `large`) for `DocType::Task`
- slug shape (kebab-case, no slashes, non-empty) — applies to all doctypes
- identity-key presence in `managed_meta` (covered by `ensure_managed_identity_keys` already; confirm wired in)
- per-doctype required-managed_meta-field presence (verify per arm at implementation time; current expectation: `DocType::Task` needs nothing additional in pure layer; `DocType::Session` needs ctx + title; `DocType::Concept` / `DocType::Decision` need nothing extra)

This function is backend-free, takes no I/O dependencies, and runs before any backend dispatch. Both `VaultBackend` and the future `DbBackend` will call it from their respective `create_resource` implementations.

### Layer 2: backend-specific compute inside `VaultBackend::create_resource`

After `validate_create_pure` passes, `VaultBackend::create_resource` performs the backend-specific compute steps before delegating to `per_doctype::write_for`. As above, branching is via `match DocType` on the typed enum:

- For `DocType::Task`:
  - Filesystem walk to compute `next_seq` against `Vault::doctype_dir(ctx, DocType::Task)`. Lift logic from existing `actions::task` helpers if present, otherwise reimplement.
  - If `cmd.managed_meta.goal.is_some()`, stat `Vault::resource_path(ctx, DocType::Goal, goal_slug)` to verify referent exists. Return `BadRequest` referencing the missing goal slug if not.
- For other `DocType` arms: no backend-specific compute required.

These steps are confined to the backend implementation. The `Backend` trait surface does NOT grow new methods. `temper_core` gains no filesystem-walker dependency. The future `DbBackend::create_resource` will mirror this pattern with `SELECT MAX(seq)+1` and `SELECT EXISTS(...)` queries — same logical flow, different access primitives.

## Surface Migration (B5b)

`commands/resource.rs::create` Local-arm (current lines 144-205) collapses to a single uniform path:

```rust
// after cloud-mode early return at line 137

let stdin_content = vault::read_stdin_if_piped();

let slug_resolved = slug.map(String::from).unwrap_or_else(|| {
    let today = Local::now().format("%Y-%m-%d").to_string();
    let base_slug = vault::slugify(title);
    match doc_type {
        "concept" => base_slug,
        _ => format!("{today}-{base_slug}"),
    }
});

let body = stdin_content.unwrap_or_default().to_string();

let cmd = CreateResource {
    slug: slug_resolved.clone(),
    doctype: doc_type.to_string(),
    context: ctx.to_string(),
    title: title.to_string(),
    body: if body.is_empty() {
        None
    } else {
        Some(BodyUpdate { content: body, content_hash: None })
    },
    managed_meta: ManagedMeta {
        mode: mode.map(String::from),
        effort: effort.map(String::from),
        goal: goal.map(String::from),
        ..ManagedMeta::default()
    },
    open_meta: None,
    origin_uri: None,
    chunks_packed: None,
    content_hash: None,
    origin: Surface::CliLocalVault,
};

let (runtime, ctx_for_backend) = crate::vault_backend::assemble_vault_backend(config, &ctx)?;
let backend = crate::vault_backend::VaultBackend::new(ctx_for_backend);
let output = runtime.block_on(backend.create_resource(cmd))?;

render_create_output(&output, doc_type, format)?;
Ok(())
```

The `match doc_type { ... }` dispatch at lines 146-205 disappears as a dispatch-decision. A smaller per-doctype switch survives inside `render_create_output` to preserve existing JSON output shapes (see next section) — the doctype-awareness migrates from "which path do I take?" to "which output shape do I emit?". The existing per-command callees (`actions::task::create`, `commands::goal::create`, `commands::session::save`, `commands::research::save`) are no longer invoked from this path. They remain as separate command entry points (`temper goal create`, `temper session save`, `temper research save`) untouched in B5b.

## Output Rendering Preservation

Each doctype currently emits a different JSON shape for `--format json`. B5b preserves all existing shapes by routing the returned `ResourceRow` through a doctype-aware `render_create_output(output, doc_type, format)` helper at the surface. The helper switches on `doc_type` and emits the JSON shape matching the pre-B5b output for that doctype.

This is required for backward compatibility: existing CLI tests assert specific JSON shapes per doctype (e.g., the task arm at lines 158-167 emits `{"type": "task", "temper-slug": ..., "temper-title": ..., "temper-context": ...}`). The render helper is the single point that knows about per-doctype output shapes; the create path itself stays uniform.

Audit the existing per-doctype JSON shapes during implementation by grepping CLI integration tests for assertions on `temper resource create` JSON output. Any shape mismatch surfaces as a test failure.

## Save-or-Update Semantics

Documented behavior change at the unified `temper resource create` entry point:

- `temper resource create --type session` — hard-error-on-exists (matches concept/decision behavior).
- `temper resource create --type research` — same.

The dedicated `temper session save` and `temper research save` subcommands continue to support save-or-update via their existing wrapper logic, which is **not modified in B5b**. Their migration to backend-dispatch (with surface-side check-then-route using `Vault::resource_path(...).exists()`) is deferred to Phase 5.

This means no test for `temper session save` or `temper research save` should change in B5b. The only tests at risk are those invoking `temper resource create --type session/research` against an already-existing slug and asserting silent update — those tests need to be reframed (or moved to use the dedicated subcommand) at implementation time.

## Dead-Code Handling

After B5b's surface migration, several functions become unreferenced:

- `actions::task::create` (callable surface gone)
- Possibly `commands::goal::create`, `commands::session::save`'s create branch, `commands::research::save`'s create branch (depends on whether other callers exist — verify at task time)

Project policy (`-D warnings` in clippy):

- Do NOT add `#[expect(dead_code, reason = "...")]` to silence warnings.
- Do NOT pre-emptively delete during the dispatch-collapse commit.
- Within the same B5b task, after the dispatch-collapse commit, run `cargo make check`, read the dead-code warnings, and delete the flagged items in a follow-up commit.
- Final B5b commit boundary always has `cargo make check` clean.
- If session context exhausts mid-B5b before cleanup, the working-tree clippy warnings serve as the punchlist for the next session to pick up.

## Test Strategy

Per superpowers TDD discipline:

- B5b's primary regression guard is the **existing** `create_*` integration test suite. Refactor passes when all stay green; no new test needed for the dispatch-collapse itself.
- B5b's **new** unit test (`create_local_dispatches_through_vault_backend_for_all_doctypes`, per parent plan) is written first (red), then implementation makes it green. It asserts that the local-mode arm constructs a `CreateResource` cmd with the correct shape and dispatches via `VaultBackend::create_resource` for each of the 6 doctypes.
- C1 and C2 are pure mechanical refactors guarded entirely by existing tests; no new tests written.

## Verification Matrix

For each of B5b, C1, C2:

| Step | Command | Pass criterion |
|------|---------|----------------|
| Unit | `cargo nextest run -p temper-cli` | All green |
| Integration | `cargo nextest run -p temper-cli --features test-db` | All green |
| E2E | `cargo nextest run --manifest-path tests/e2e/Cargo.toml --features test-db,test-embed` | All green |
| Lint | `cargo make check` | Clean (no warnings) at final commit boundary |

## Descope Checkpoints

User-confirmed: aim for B5b + C1 + C2 in this session, with permission to stop after B5b or B5b + C1 if scope expands unexpectedly.

- **Stop after B5b** if: dead-code cleanup reveals more than ~5 unreferenced functions; or save-or-update test reframing surfaces non-trivial test rewrites; or the JSON output preservation requires more than a thin doctype switch at the surface.
- **Stop after B5b + C1** if: C2's `resolve_resource_id` callers turn out to need different return shapes per call site (some want id, some want ResourceRow), expanding the per-call adapter complexity.

Either descope path defers the remaining work to a fresh session; the parent plan's Phase D (review + PR open) waits until all three tasks land.

## Connections

- Parent spec: `docs/superpowers/specs/2026-05-11-wave1-phase4-vaultbackend-design.md`
- Parent plan: `docs/superpowers/plans/2026-05-13-wave1-phase4-completion.md`
- Predecessor session: `phase-4-completion-phase-b-b1-b5a-landed--per-doctype-dispatch-task-closed`
- Active task: `2026-05-11-wave-1-phase-4b-extract-commands-resource-rs-local-mode-writes-through-vaultbackend`
- Related backlog tasks (Phase D PR will close):
  - `2026-05-11-delete-actions-frontmatter-build-managed-meta-for-create-after-phase-4b` (closed by C1)
  - `2026-05-11-delete-commands-resource-rs-resolve-resource-id-after-phase-4b` (closed by C2)
