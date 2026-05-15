# Wave 1 Phase 4 Completion — Audit-Gate Pull-In + Resource Extraction + Helper Deletions

**Date:** 2026-05-13
**Context:** `temper`
**Mode:** build
**Effort:** large (multi-session; single PR)
**Branch:** `jct/wave1-phase4-completion`

**Spec:** `docs/superpowers/specs/2026-05-11-wave1-phase4-vaultbackend-design.md`
**Predecessor (merged):** PR #77 — Wave 1 Phase 4a, VaultBackend foundation (dark-launched)
**Vault tasks absorbed by this plan:**
- `2026-05-11-complete-per-doctype-write-dispatch-for-task-goal-session-research` (Phase A)
- `2026-05-11-wave-1-phase-4b-extract-commands-resource-rs-local-mode-writes-through-vaultbackend` (Phase B)
- `2026-05-11-delete-actions-frontmatter-build-managed-meta-for-create-after-phase-4b` (Phase C, Task C1)
- `2026-05-11-delete-commands-resource-rs-resolve-resource-id-after-phase-4b` (Phase C, Task C2)

---

## Goal

Land the rest of Wave 1 Phase 4 as one cohesive PR: complete the per-doctype write dispatch (audit-gate pull-in), migrate the Local-mode write arms of `commands/resource.rs::{create,update,delete}` through `VaultBackend`, and delete the two helpers (`build_managed_meta_for_create`, `resolve_resource_id`) whose deletion was previously a downstream backlog task.

The end state:
- All 6 doctype write paths (concept, decision, task, goal, session, research) live in `vault_backend/per_doctype.rs`.
- `commands/resource.rs::{create,update,delete}` Local-mode arms dispatch through `VaultBackend` — doctype-agnostic, no interstitial states.
- Cloud-mode arms unchanged except where inlining `build_managed_meta_for_create` substitutes the helper call at the cloud-create call site.
- The `match VaultState` branch stays in each function — Phase 5 collapses it.
- `actions::frontmatter::build_managed_meta_for_create` + `NewResourceArgs` deleted.
- `commands/resource.rs::resolve_resource_id` deleted; all 7 callers migrated.

## User-confirmed design choices

1. **Full pull-in** for the audit-gate fix (not publish-extraction). Per-doctype write logic moves into `vault_backend/per_doctype.rs`. Existing creators in `actions/{task,goal}.rs` and `commands/{session,research}.rs` become thin wrappers that call `per_doctype::write_for` for the file-write half, then publish + emit discovery events + return their existing return shape.

2. **Hard-error-on-exists** for session and research via `temper resource create`. Matches concept/decision behavior. The dedicated `temper session save` and `temper research save` subcommands keep their save-or-update overload by dispatching at the surface (check exists → call create wrapper or call update wrapper) — the overload becomes surface-level logic, not creator-internal logic.

3. **Single PR**, multi-session execution. The branch holds the whole body of work; PR opens after Phase D.

## Non-Goals

- **Surface dispatch unification** (collapse `match VaultState`) — Phase 5.
- **`commands/{task,goal,session,research}.rs` write paths** (i.e. their dedicated clap subcommands' UX) — preserved unchanged. The pull-in changes their *implementation* (now delegates through `per_doctype.rs`), not their *interface*.
- **`actions::sync.rs`** (4369 lines) — sync orchestration stays put.
- **Local search reinstatement** — `search_resources` stays a passthrough to client.
- **`show_resource` migration** — Local-mode show paths stay surface-direct (Phase 5).

## Approach summary

| Phase | Scope | Tasks | Sessions est. |
|---|---|---|---|
| **A. Audit-gate pull-in** | Pull task/goal/session/research write logic into `per_doctype.rs`; existing creators become thin wrappers | 5 | 1-2 |
| **B. Resource extraction** | Migrate `commands/resource.rs::{create,update,delete}` Local-mode arms through VaultBackend (now doctype-agnostic) | 5 | 1-2 |
| **C. Deletions** | Inline + delete `build_managed_meta_for_create`; migrate + delete `resolve_resource_id` | 2 | 1 |
| **D. Review + PR** | Spec-compliance, opus code review, single PR open | 3 | <1 |

## Conventions for every task

Inherited verbatim from 4a's plan (per memory rules `feedback_prefer_subagent`, `feedback_plan_code_quality`, `feedback_subagent_check_before_commit`, `feedback_plan_verification`, `feedback_workspace_test_surfaces_pipeline_bugs`, `feedback_nextest_summary_lies`, `feedback_subagent_escalate_not_soften`, `feedback_no_premature_backward_compat`):

- **TDD.** Write the test first. Confirm it fails for the right reason. Implement. Confirm it passes.
- **Verify named APIs before dispatch.** Every API name in this plan is a hypothesis until grep-confirmed at task time. The code is ground truth.
- **`cargo make check` before claiming complete.** Pre-commit hook is the backstop.
- **Pair filter-by-name runs with full-crate runs before commit.**
- **No `#[allow(...)]` for clippy.** Use `#[expect(name, reason = "...")]` or fix the underlying issue.
- **Workspace-feature-unification awareness.** `cargo nextest --workspace` activates `ingest-pipeline` via `temper-cloud`'s feature graph — surface that won't appear in standalone crate runs.
- **Don't trust nextest's per-binary `Summary` line.** Trust exit code or grep for `error: test run failed` / `FAIL [`.
- **Escalate, don't soften.** If a test requires loosening a contract, STOP and report BLOCKED.
- **No "for now" workarounds.** Capture as a task; don't ship a TODO comment.
- **No premature backward-compat.** Phase 5 is where unused surface code lives or dies; don't pre-empt those decisions, but don't keep dead code "just in case" either.
- **Existing CLI integration tests + e2e tests pass unmodified** is the regression guard. A test re-write to accommodate the migration is a smell — STOP and report.

---

# Phase A — Audit-gate completion via full pull-in

**Goal of Phase A.** Move template + frontmatter + file-write logic for task, goal, session, research from their current homes (`actions/task.rs`, `actions/goal.rs`, `commands/session.rs`, `commands/research.rs`) into `vault_backend/per_doctype.rs`. Existing creator wrappers stay (preserving subcommand UX); they delegate to `per_doctype::write_for` for the bare write, then publish + emit discovery + render output as before. Result: `VaultBackend.create_resource` works for all 6 doctypes — the audit gate's BadRequest fallback is removed.

**Key invariant.** `per_doctype::write_for` is the bare write — no publish, no discovery event, no output. Publish/discovery/output stays in the calling wrapper. This lets VaultBackend (which has its own push-as-tail-action via `push_create`) call `per_doctype::write_for` without double-publishing.

### Task A1 — Pull task write into `per_doctype::write_task`

**Owner:** subagent (sonnet)

**Goal.** Move the template + frontmatter + write logic from `actions/task.rs::create` (lines 174-209) into a new `write_task` function in `per_doctype.rs`. Existing `actions::task::create` becomes the wrapper: calls `per_doctype::write_for`, then publishes, emits discovery, calls `output::success`.

**Verification of API names** (grep at task start):
- `grep -n "fn next_seq\|ensure_maintenance\|find_goal" crates/temper-cli/src/actions/goal.rs`
- `grep -n "pub fn slugify\|fn validate_mode\|fn validate_effort" crates/temper-cli/src/vault.rs`
- Confirm `TaskTemplate` is in `crates/temper-cli/src/templates/` and its field shape matches lines 184-194 of `actions/task.rs`.

**Implementation outline:**
```rust
// In vault_backend/per_doctype.rs — extend write_for + add helper:
fn write_task(args: WriteArgs<'_>) -> Result<WriteResult, TemperError> {
    // Per-doctype concerns kept here:
    // - Date stamp + slug computation: `{date}-{slugify(title)}`
    // - Sequence number via next_seq (needs config + goal)
    // - Goal validation/maintenance — NOTE: this is task-specific business
    //   logic. Decide at task time whether to keep it in the wrapper (where
    //   it lives today) or move it here. Default: keep validation in the
    //   wrapper since it depends on goal lookup; per_doctype only writes.
    //
    // The bare write:
    // 1. Render TaskTemplate
    // 2. Append stdin body if present (preserves current behavior at
    //    actions/task.rs:199-202)
    // 3. Compute path; ERROR-ON-EXISTS via `if abs_path.exists() { Err(...) }`
    // 4. create_dir_all + vault::write_note
    // 5. Parse temper-id from the rendered template (matches concept/decision
    //    behavior in write_concept_or_decision)
    // 6. Return WriteResult { resource_id, abs_path, rel_path }
}

pub(crate) fn write_for(args: WriteArgs<'_>) -> Result<WriteResult, TemperError> {
    match args.doctype {
        "concept" | "decision" => write_concept_or_decision(args),
        "task" => write_task(args),
        // ... other doctypes added in subsequent tasks
        other => Err(TemperError::BadRequest(format!(
            "unsupported doctype for create: '{other}'"
        ))),
    }
}
```

**Wrapper transformation** in `actions/task.rs::create`:
```rust
pub fn create(...) -> Result<String> {
    // Existing pre-write concerns kept:
    // - goal::ensure_maintenance / find_goal
    // - vault::validate_mode / validate_effort
    // - context match verification (lines 152-162)

    // Bare write via per_doctype:
    let result = crate::vault_backend::per_doctype::write_for(WriteArgs {
        doctype: "task",
        title,
        slug: &slug,
        context,
        body: stdin_content.unwrap_or(""),
        open_meta: None,
        vault_root: &config.vault_root,
        owner: &config.owner_for_context(context),
        config,
    })?;

    // Post-write tail actions:
    crate::actions::runtime::publish_local_write_best_effort(
        &config.vault_root, &result.abs_path,
    )?;

    let event = discovery::Event::ResourceCreate { /* existing */ };
    let _ = discovery::append_event(&config.state_dir, &event);

    output::success(format!("Created task: {}", result.slug.unwrap_or_default()));
    Ok(/* slug from result */)
}
```

**Open question for the implementer:** `actions::task::create` needs goal/mode/effort fields on the written frontmatter. Today those are passed into TaskTemplate (lines 184-194). For `per_doctype::write_for`, the WriteArgs struct only carries doctype/title/slug/context/body/open_meta — no goal/mode/effort. Decision:
- **(a)** Add `task_specific: Option<TaskFields>` (or similar) to `WriteArgs` as a doctype-specific extension.
- **(b)** Compute the rendered template at the surface (in the wrapper), pass the rendered string as `body` to `per_doctype::write_for`. But then `per_doctype` is no longer template-aware for tasks, which is asymmetric vs concept/decision.
- **(c)** Pre-populate the open_meta with goal/mode/effort and have `per_doctype::write_task` look it up there. Hacky.

**Recommended: (a) — add a `task_fields: Option<TaskFields>` field on `WriteArgs`** (or similar shape — verify naming at task time). Mirrors how concept/decision don't need it (None) and task/goal/session/research need it (Some). Each doctype's WriteFor function picks the fields it needs. The shape can be a single enum `DoctypeFields { Task { goal, mode, effort, seq }, Goal { ... }, Session { ... }, Research { ... } }` or per-doctype optional structs.

The implementer picks the cleanest decomposition at task time; escalate if neither shape is clean.

**Tests (in `vault_backend/per_doctype.rs::tests`):**
- `write_for_task_creates_file_with_correct_frontmatter` — assert `temper-id`, `temper-title`, `temper-mode`, `temper-effort`, `temper-goal` all present on disk
- `write_for_task_errors_on_existing_slug` — write twice; second errors
- `write_for_task_writes_body_when_stdin_provided`
- Existing `actions::task::create` integration tests pass unmodified (via the wrapper)

**Verification.**
- `cargo nextest run -p temper-cli vault_backend::per_doctype::tests::write_for_task --features test-db`
- `cargo nextest run -p temper-cli actions::task` (existing tests)
- `cargo nextest run -p temper-cli --features test-db` (full crate)
- `cargo make check` clean.

**Commit message.** `phase4-completion A1: pull task write into per_doctype::write_task`

---

### Task A2 — Pull goal write into `per_doctype::write_goal`

**Owner:** subagent (sonnet)

**Goal.** Same shape as A1 for goal. `actions/goal.rs::create` (lines 120-153) writes a goal file via `vault_layout.doc_file(owner, context, "goal", &slug)` then publishes (line 150). Goals also have an `ensure_maintenance` helper (around line 87) that writes the goal "maintenance" entry. Both write paths get the same treatment.

**Verification of API names** (grep at task start):
- `grep -n "fn ensure_maintenance\|fn create\|fn next_seq" crates/temper-cli/src/actions/goal.rs`
- Confirm `GoalTemplate` exists and check its field shape.

**Implementation:**
- Add `write_goal(args: WriteArgs<'_>) -> Result<WriteResult, TemperError>` in `per_doctype.rs`.
- Move template render + frontmatter + write logic from `actions::goal::create` lines 120-149 into it.
- ERROR-ON-EXISTS check (today there's no such check at line 128 — pre-existing slug behavior depends on the renderer/template; verify at task time and preserve behavior. If the existing code overwrites silently, this is a behavior change to flag; the new error-on-exists matches concept/decision/task).
- `actions::goal::create` becomes the wrapper: validation → per_doctype::write_for → publish → output.
- `ensure_maintenance` (line 87) follows the same pattern but with its specialized maintenance entry; rewrites to delegate through `per_doctype::write_for` as well.

**WriteArgs extension** for goal-specific fields: title-only (no mode/effort/goal-ref). The `DoctypeFields` enum from A1 gains a `Goal { /* status, seq */ }` variant.

**Tests:**
- `write_for_goal_creates_file_with_correct_frontmatter`
- `write_for_goal_errors_on_existing_slug`
- `ensure_maintenance_creates_or_reuses` — preserves existing behavior
- Existing `actions::goal::create` tests pass unmodified.

**Verification.** Same shape as A1.

**Commit message.** `phase4-completion A2: pull goal write into per_doctype::write_goal`

---

### Task A3 — Pull session write into `per_doctype::write_session`

**Owner:** subagent (sonnet)

**Goal.** Move the new-file-create branch of `commands::session::save` (lines 84-120 of `commands/session.rs`) into `per_doctype.rs::write_session`. The "already exists" branch (lines 64-82) becomes surface-side dispatch: the wrapper checks `note_path.exists()` first; if yes, calls the existing update path (no change); if no, calls `per_doctype::write_for`.

**Verification of API names** (grep at task start):
- `grep -n "SessionTemplate" crates/temper-cli/src/templates/`
- Confirm field shape of `SessionTemplate`.

**Implementation:**
- Add `write_session(args: WriteArgs<'_>) -> Result<WriteResult, TemperError>` with hard-error on exists.
- `commands::session::save` wrapper structure:
  ```rust
  pub fn save(...) -> Result<()> {
      // Path computation (existing lines 55-62)
      if note_path.exists() {
          // Existing body-replace path preserved (lines 64-82)
          // — this is the save-or-update overload, surface-side
      } else {
          let result = per_doctype::write_for(WriteArgs { /* session */ })?;
          publish_local_write_best_effort(&config.vault_root, &result.abs_path)?;
          // discovery, output (lines 122-156)
      }
      Ok(())
  }
  ```

**WriteArgs extension** for session-specific fields: title, date (computed in write_session or passed in). The `DoctypeFields::Session` variant covers session-specific frontmatter.

**Tests:**
- `write_for_session_creates_file_with_correct_frontmatter`
- `write_for_session_errors_on_existing_slug`
- Existing `commands::session::save` tests pass unmodified — including the save-or-update overload tests (the overload is preserved at the surface).

**Verification.** Same as A1.

**Commit message.** `phase4-completion A3: pull session write into per_doctype::write_session; preserve save-or-update at surface`

---

### Task A4 — Pull research write into `per_doctype::write_research`

**Owner:** subagent (sonnet)

**Goal.** Mirror of A3 for research. Move the new-file-create branch of `commands::research::save` (lines 48-85 of `commands/research.rs`) into `per_doctype.rs::write_research`. The "already exists" branch (lines 29-46) stays at the surface as the save-or-update overload.

**Verification of API names** at task start: `grep -n "ResearchTemplate" crates/temper-cli/src/templates/`.

**Implementation:** same shape as A3.

**Tests:**
- `write_for_research_creates_file_with_correct_frontmatter`
- `write_for_research_errors_on_existing_slug`
- Existing `commands::research::save` tests pass unmodified.

**Verification.** Same as A1.

**Commit message.** `phase4-completion A4: pull research write into per_doctype::write_research; preserve save-or-update at surface`

---

### Task A5 — Remove audit-gate BadRequest fallback

**Owner:** subagent (haiku)

**Goal.** Mechanical cleanup. With A1-A4 done, `per_doctype::write_for` has explicit branches for task/goal/session/research that no longer fall through to the audit-gate BadRequest at lines 73-80. Remove the BadRequest arm and the audit-gate comment block at lines 1-15 of `per_doctype.rs`. Final `write_for` dispatches all 6 doctypes uniformly.

Also remove the `#[expect(dead_code)]` `derive_rel_path` helper at `per_doctype.rs:189-205` if it's still unused after A1-A4 (it was placeholder for "when task/goal/session/research dispatch is added"); promote it to live code or delete.

**Tests:** `write_for_task_returns_bad_request` (currently at line 279 of per_doctype.rs) flips to assert success. Other "unsupported doctype" tests stay (e.g., `write_for_unsupported_doctype_returns_bad_request` for "widget").

**Verification.**
- `cargo nextest run -p temper-cli vault_backend::per_doctype --features test-db` — all green.
- `cargo make check` clean.

**Commit message.** `phase4-completion A5: remove audit-gate fallback; all 6 doctypes uniform`

---

# Phase B — Resource extraction (Local-mode arm migration)

**Goal of Phase B.** With per_doctype.rs now doctype-agnostic, migrate `commands/resource.rs::{create,update,delete}` Local-mode arms through `VaultBackend`. Doctype-agnostic — no more interstitial states. Cloud-mode arms preserved.

### Task B1 — Promote `build_partial_*` helpers; add `build_move_spec_from_args`

**Owner:** subagent (haiku)

**Goal.** Remove `#[cfg(feature = "embed")]` gate from `build_partial_managed_meta_from_args` (`commands/resource.rs:1332-1360`) and `build_partial_open_meta_from_args` (lines 1362-1414); add `build_move_spec_from_args(params: &UpdateParams<'_>) -> Option<MoveSpec>`.

Implementation + tests as in the original 4b plan draft (same content; carry forward).

**Verification.** `cargo build -p temper-cli` (both with and without `--features embed`); `cargo nextest run -p temper-cli commands::resource::tests::build_`; `cargo make check`.

**Commit message.** `phase4-completion B1: promote build_partial_* helpers; add build_move_spec_from_args`

---

### Task B2 — `assemble_vault_backend_ctx` helper

**Owner:** subagent (sonnet)

**Goal.** Shared `VaultBackendCtx` builder for the three Local arms. Lives in `vault_backend/mod.rs`.

Signature, behavior, tests as in the original 4b plan draft.

**Verification.** `cargo nextest run -p temper-cli vault_backend::ctx_tests --features test-db`; `cargo make check`.

**Commit message.** `phase4-completion B2: assemble_vault_backend_ctx helper`

---

### Task B3 — Migrate `commands/resource.rs::delete` Local arm

**Owner:** subagent (sonnet)

Same content as original 4b plan draft Task 3.

**Commit message.** `phase4-completion B3: migrate commands/resource.rs::delete Local arm through VaultBackend`

---

### Task B4 — Migrate `commands/resource.rs::update` Local arm

**Owner:** subagent (sonnet); dedicated spec-compliance reviewer (sonnet) before commit

Same content as original 4b plan draft Task 4.

**Commit message.** `phase4-completion B4: migrate commands/resource.rs::update Local arm through VaultBackend`

---

### Task B5 — Migrate `commands/resource.rs::create` Local arm (all doctypes)

**Owner:** subagent (sonnet); dedicated reviewer (sonnet) before commit

**Goal.** With the audit gate gone, the migration is now doctype-agnostic. The match at `resource.rs:146-205` collapses entirely — all 6 doctypes dispatch through `VaultBackend.create_resource`.

**Implementation outline:**
```rust
// commands/resource.rs::create — Local-mode arm
// (after cloud-mode early return at line 137)

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
    // VaultBackend.create_resource applies defaults + identity keys.
    // Task-specific args (goal, mode, effort) flow through managed_meta
    // — populate from clap args here.
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

let backend_ctx = crate::vault_backend::assemble_vault_backend_ctx(config, &ctx)?;
let backend = crate::vault_backend::VaultBackend::new(backend_ctx);
let (runtime, _) = actions::runtime::build_runtime_and_client()?;
let output = runtime.block_on(backend.create_resource(cmd))?;

// Render based on output.value (ResourceRow) and format flag — matches
// existing JSON output shapes per doctype.
```

**Open question for the implementer:** the existing task creation has `actions::task::create` (line 148) which does `next_seq` and goal validation BEFORE writing. After A1, that logic moved to the actions::task::create wrapper (calling per_doctype::write_for for the write). When VaultBackend.create_resource is the dispatch path, the per-doctype validation (next_seq, goal exists check) needs to happen somewhere:
- **(a)** Inside `per_doctype::write_task` — but per_doctype is meant to be bare write logic.
- **(b)** Inside `VaultBackend.create_resource` before calling `per_doctype::write_for` — but VaultBackend shouldn't have per-doctype business logic.
- **(c)** Inside `temper_core::operations::actions::validate_create` — the operations-layer validation, runs before any backend dispatch. Per-doctype validation expands at the shared-action layer.
- **(d)** At the surface (`commands/resource.rs::create`), branched by doctype.

Recommended: **(c)** — move per-doctype validation into `validate_create` in the operations layer. Goal-exists, mode-valid, effort-valid for tasks become validation calls that any backend (Vault or DB) runs. Falls out cleanly when each branch needs to validate.

This expands the validation contract — flag if too large; consider falling back to (d) as a more localized fix.

**Tests:**
- All existing `create_concept_*`, `create_decision_*`, `create_task_*` etc. integration tests pass unmodified — this is the regression guard.
- New unit test: `create_local_dispatches_through_vault_backend_for_all_doctypes`.

**Verification.**
- `cargo nextest run -p temper-cli commands::resource::tests::create_` (all green)
- `cargo nextest run -p temper-cli --features test-db` (full crate)
- `cargo nextest run --manifest-path tests/e2e/Cargo.toml --features test-db,test-embed`
- `cargo make check` clean.

**Commit message.** `phase4-completion B5: migrate commands/resource.rs::create Local arm through VaultBackend (doctype-agnostic)`

---

# Phase C — Helper deletions

### Task C1 — Inline `build_managed_meta_for_create`; delete helper + `NewResourceArgs`

**Owner:** subagent (sonnet)

**Goal.** Three callers (`resource.rs:76`, `research.rs:61`, `session.rs:96`) inline the helper body at the call site. Then delete `actions::frontmatter::build_managed_meta_for_create` + `NewResourceArgs` struct.

**Inline pattern** (cloud-mode create at resource.rs:76 example):
```rust
let managed_meta = ManagedMeta {
    mode: mode.map(String::from),
    effort: effort.map(String::from),
    goal: goal.map(String::from),
    ..ManagedMeta::default()
};
```

For research and session: the helper is currently passed all `None` for mode/effort/goal/stage/seq/status/provenance/llm_model/llm_run. So inlining is `ManagedMeta::default()` plus title (which the helper sets from `args.title` — verify at task time). 3 lines per call site.

**Caveat:** if any caller passes meaningful non-None args, the inline gets richer. Verify each call site reads its inputs and inlines the same fields the helper would have set.

**File touches:**
- `crates/temper-cli/src/commands/resource.rs` (cloud-mode create at line 76)
- `crates/temper-cli/src/commands/research.rs` (line 61)
- `crates/temper-cli/src/commands/session.rs` (find the call site — grep at task time)
- `crates/temper-cli/src/actions/frontmatter.rs` (delete `build_managed_meta_for_create` + `NewResourceArgs` + tests)

**Tests.** Existing integration tests in cloud-mode create, research save, session save all pass unmodified.

**Verification.** `cargo nextest run -p temper-cli --features test-db,embed`; `cargo make check`.

**Commit message.** `phase4-completion C1: inline build_managed_meta_for_create at 3 callers; delete helper`

---

### Task C2 — Migrate `resolve_resource_id` callers; delete function

**Owner:** subagent (sonnet)

**Goal.** 7 callers across 3 files (`resource.rs:784, 1114, 1183`, `task.rs:70, 100`, `session.rs:367, 413`). The callers are show-path UUID lookups. Each replaces with a direct `client.resources().resolve_by_uri(...)` call (which is what `resolve_resource_id`'s fallback does anyway).

After B3 (delete migration), the call at `resource.rs:784` is already gone. The remaining 6 are show paths.

**Migration pattern** (`task.rs:70` example):
```rust
// Before
let id = super::resource::resolve_resource_id(
    &config_clone, client, "task", &task_slug, Some(&task_ctx), VaultState::Local,
).await?;

// After
let id = client
    .resources()
    .resolve_by_uri("@me", &task_ctx, "task", &task_slug)
    .await
    .map_err(crate::actions::runtime::client_err_to_temper)?
    .id;
```

**Owner placement:** the `@me` literal above should come from `config.owner_for_context(ctx)` to match how vault paths are computed elsewhere (per the `owner_for_context` rule from PR #75).

**File touches:**
- `crates/temper-cli/src/commands/resource.rs` (callers at 1114, 1183; remove `resolve_resource_id` function at 902-936 after all callers gone)
- `crates/temper-cli/src/commands/task.rs` (callers at 70, 100)
- `crates/temper-cli/src/commands/session.rs` (callers at 367, 413)

**Tests.** Existing show-path tests for task, session, generic resources pass unmodified.

**Verification.** `cargo nextest run -p temper-cli --features test-db`; `cargo make check`.

**Commit message.** `phase4-completion C2: migrate resolve_resource_id callers; delete function`

---

# Phase D — Review + PR

### Task D1 — Spec-compliance + verification matrix

**Owner:** subagent (opus for spec review; sonnet for verification)

**Verification commands:**
```bash
cargo make check
cargo make test
cargo make test-db
cargo nextest run --workspace --no-fail-fast 2>&1 | tee /tmp/phase4-workspace.log
grep -E "FAIL \[|error: test run failed" /tmp/phase4-workspace.log
cargo nextest run --manifest-path tests/e2e/Cargo.toml --features test-db,test-embed
```

**Cloud-mode preservation diff check** (after Phase C inlining touched cloud-create at resource.rs:76, the cloud-mode arms are no longer byte-for-byte; instead, the contract is "cloud-mode behavior preserved"):
```bash
git diff main...HEAD -- crates/temper-cli/src/commands/resource.rs \
  | awk '/^@@/{in_cloud=0} /VaultState::Cloud/{in_cloud=1} in_cloud{print}'
# Manually scan: only diff should be the inlining of build_managed_meta_for_create.
# Cloud delete arm (lines 795-806 today) and cloud_mode_update (1416-1500) stay
# byte-for-byte.
```

**Spec-compliance review prompt (opus):**
> Confirm the branch implements Wave 1 Phase 4 completion per spec at `docs/superpowers/specs/2026-05-11-wave1-phase4-vaultbackend-design.md` and unified plan at `docs/superpowers/plans/2026-05-13-wave1-phase4-completion.md`. Focus on:
> 1. Per-doctype write logic lives in `vault_backend/per_doctype.rs` for all 6 doctypes (concept, decision, task, goal, session, research).
> 2. Existing creator wrappers (`actions::task::create`, `actions::goal::create`, `commands::session::save`, `commands::research::save`) delegate to `per_doctype::write_for` for the file-write half and handle publish/discovery/output at the wrapper layer.
> 3. `commands/resource.rs::{create,update,delete}` Local-mode arms exclusively dispatch through `VaultBackend`. Grep for direct `Frontmatter::write_to`, `manifest_io::save_manifest`, `publish_local_write_best_effort` in Local arms — zero expected.
> 4. Cloud-mode delete arm (resource.rs:795-806) and `cloud_mode_update` byte-for-byte unchanged.
> 5. Cloud-mode create arm changed only by the inlining of `build_managed_meta_for_create`. No other diff.
> 6. `actions::frontmatter::build_managed_meta_for_create` + `NewResourceArgs` deleted.
> 7. `commands/resource.rs::resolve_resource_id` deleted; all 7 callers migrated.
> 8. Schema-required defaults symmetric defense still works through dispatch.
> 9. `temper-updated` not double-set.
> Report under 500 words. Categorize: drift-to-correct vs spec-ambiguity-to-clarify.

**Address findings inline** if quick; roll into follow-up otherwise.

**Commit (if any).** `phase4-completion D1: spec-compliance fixups`

---

### Task D2 — Final code review

**Owner:** subagent (opus)

**Dispatch prompt:**
> Review branch `jct/wave1-phase4-completion` against the spec at `docs/superpowers/specs/2026-05-11-wave1-phase4-vaultbackend-design.md` and the unified plan at `docs/superpowers/plans/2026-05-13-wave1-phase4-completion.md`. Focus:
> - Per-doctype pull-in (Phase A): each wrapper delegates cleanly to per_doctype::write_for; no per-doctype business logic leaked into VaultBackend or per_doctype's bare-write functions; surface UX preserved for `temper task create`, `temper goal create`, `temper session save`, `temper research save`.
> - Save-or-update overload at the session/research surface preserved; hard-error-on-exists semantics for `temper resource create --type {session,research}` correctly diverges.
> - Resource-arm migration (Phase B): cloud-mode delete + `cloud_mode_update` byte-for-byte; cloud-mode create differs only by inlining the deleted helper.
> - Helper deletions clean — `build_managed_meta_for_create` + `NewResourceArgs` gone; `resolve_resource_id` gone; no dangling imports.
> - Existing tests pass unmodified — any tests touched in the diff that are not strict additions is a regression signal.
> - No `#[allow]` clippy; no "for now" TODOs; no surface-side direct `Frontmatter::write_to`/`manifest_io::*`/`publish_local_write_best_effort` calls in Local arms.
> - Concurrency: Arc<Mutex<Manifest>> lock windows are tight (no long I/O held under lock).
> Return READY_WITH_FOLLOWUPS or REQUEST_CHANGES with critical/important/nit categorization.

**Address critical and important findings** inline. Nits roll into follow-up commit.

**Commit (if any).** `phase4-completion D2: code-review fixups`

---

### Task D3 — Open PR

**Owner:** main agent

`git merge origin/main` first (per `feedback_merge_main_before_pushing_pr`).

**PR title.** `Wave 1 Phase 4 completion: audit-gate pull-in + resource extraction + helper deletions`

**PR body template.**
```
## Summary
Completes Wave 1 Phase 4 in one body of work:
- **Audit-gate pull-in**: per-doctype write logic for all 6 doctypes (concept, decision,
  task, goal, session, research) now lives in `vault_backend/per_doctype.rs`. Existing
  creators in `actions/{task,goal}.rs` and `commands/{session,research}.rs` become thin
  wrappers that delegate to per_doctype for the bare write and handle publish/discovery/
  output themselves. Subcommands (`temper task create`, `temper session save`, etc.)
  preserve their UX.
- **Resource extraction**: `commands/resource.rs::{create,update,delete}` Local-mode arms
  dispatch through `VaultBackend` — doctype-agnostic. Cloud-mode arms preserved (cloud
  delete + cloud_mode_update byte-for-byte; cloud create differs only by inlining the
  deleted helper).
- **Helper deletions**: `actions::frontmatter::build_managed_meta_for_create` +
  `NewResourceArgs` removed; `commands/resource.rs::resolve_resource_id` removed (all 7
  callers migrated to `client.resources().resolve_by_uri`).
- **Hard-error-on-exists** for session/research via `temper resource create` (matches
  concept/decision); the `temper session save`/`temper research save` subcommands keep
  their save-or-update overload at the surface.

The `match VaultState` branch in resource.rs stays in place — Phase 5 collapses it.

## Test plan
- [x] cargo make check
- [x] cargo make test
- [x] cargo make test-db
- [x] cargo nextest run --workspace --no-fail-fast
- [x] cargo nextest run --manifest-path tests/e2e/Cargo.toml --features test-db,test-embed
- [x] Cloud-mode arms diff check (zero net change outside cloud-create's inlining)
- [x] Spec-compliance review (opus)
- [x] Code review (opus)

## Vault tasks absorbed by this PR
- complete-per-doctype-write-dispatch-for-task-goal-session-research → done
- wave-1-phase-4b-extract-commands-resource-rs-local-mode-writes-through-vaultbackend → done
- delete-actions-frontmatter-build-managed-meta-for-create-after-phase-4b → done
- delete-commands-resource-rs-resolve-resource-id-after-phase-4b → done

## Reference
- Spec: docs/superpowers/specs/2026-05-11-wave1-phase4-vaultbackend-design.md
- Plan: docs/superpowers/plans/2026-05-13-wave1-phase4-completion.md
- Predecessor: PR #77 (Wave 1 Phase 4a foundation)
```

Push branch; open PR via `gh pr create`. Return the URL.

---

## Plan-writer self-review checklist

- [x] Every absorbed vault task has an explicit phase mapping (Phase A subsumes audit-gate task; B subsumes 4b; C subsumes both deletion tasks).
- [x] User-confirmed design choices documented: full pull-in, hard-error-on-exists.
- [x] Audit-gate constraint disappears explicitly (Task A5).
- [x] Cloud-mode preservation reframed as "behavior preserved" (since cloud-create gets inlined helper) but cloud-delete + cloud-update remain byte-for-byte. The diff check in D1 enforces this.
- [x] Save-or-update overload at session/research surface preserved (not in per_doctype).
- [x] Each task has explicit verification commands.
- [x] Embed-gated tests called out in B5, D1.
- [x] Workspace-feature-unification verification in D1.
- [x] Subagent prompts have escalate-not-soften baked in (A1 open question on WriteArgs extension; B5 open question on validation placement).
- [x] No "for now" workarounds tolerated.
- [x] No duplicate cleanup work — phases reference each other; no overlap.
- [x] PR body template enumerates all 4 absorbed vault tasks.
- [x] Branch-merge-before-push reminder in D3.
- [x] Cross-references to spec + memory rules.

## Risk register

- **WriteArgs extension shape (A1).** `WriteArgs` today carries doctype-agnostic fields. Pulling task/goal/session/research in requires either a per-doctype fields struct (recommended) or surface-side template pre-render. Bad shape choices propagate across A1-A4. Detected when the WriteArgs change feels like it's growing.
- **`validate_create` per-doctype expansion (B5).** Moving goal-exists + mode-valid + effort-valid validation into the operations layer may expand the validate_create contract more than expected. If the surface-side fallback (option (d) in B5) ships instead, ensure both VaultBackend and DbBackend paths honor it — DbBackend may need a sibling update.
- **`ensure_maintenance` (A2).** Goals have two write paths (`create` + `ensure_maintenance`). Both need to delegate to per_doctype::write_for, but ensure_maintenance has an "idempotent existing" semantic that doesn't fit hard-error-on-exists. Decide at A2 task time: extract a separate `write_maintenance` that's idempotent, or keep ensure_maintenance's write path inline at actions/goal.rs.
- **Session/research save-or-update tests.** Existing tests asserting "session save with existing file silently updates" stay green only if the wrapper (commands/session::save) keeps that overload. Verify the test suite covers both paths (create-new + update-existing).
- **Body content in cloud-mode create after inlining (C1).** The inlining at resource.rs:76 substitutes the helper but the surrounding cloud-mode flow (lines 65-137) calls `build_ingest_payload` (line 93) with the result. Verify the payload shape is byte-identical pre/post inlining.

---

## Per-session execution plan

**Session 1 (this session): Phase A — audit-gate pull-in.** Tasks A1-A5. Get the per-doctype pull-in landed and the audit gate removed. Save session note at end with the actual outcome and any deviations.

**Session 2: Phase B — resource extraction.** Tasks B1-B5. With audit gate gone, create-arm migration is doctype-agnostic.

**Session 3: Phase C + D.** Tasks C1-C2 (deletions), then D1-D3 (review + PR).

If a phase runs longer than expected, split across sessions — branch stays open; PR opens at D3.

## Estimated PR shape

12-15 commits on `jct/wave1-phase4-completion` (one per task + 0-3 review fixup commits). Reviewable as one PR with phase headers in the description. Mirrors PR #77's commit cadence (13 commits) but with more substantive changes per commit.
