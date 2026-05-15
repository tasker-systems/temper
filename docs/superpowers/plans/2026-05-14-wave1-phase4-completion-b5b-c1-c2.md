# Wave 1 Phase 4 Completion — B5b + C1 + C2 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Collapse `commands/resource.rs::create` Local-arm to uniform `VaultBackend::create_resource` dispatch for all 6 doctypes; then inline `build_managed_meta_for_create` at remaining callers; then migrate and delete `resolve_resource_id`.

**Architecture:** Two-layer validation hybrid — pure invariants extended into `temper_core::operations::actions::validate_create` (mode/effort whitelists for tasks, per-doctype required-field presence), backend-specific compute (next_seq, goal-exists) inside `VaultBackend::create_resource` via a new `compute_task_defaults` helper. The `Backend` trait surface is unchanged. Output shape preservation via a new `render_create_output` surface helper that switches on `DocType` to emit the existing per-doctype JSON shapes.

**Tech Stack:** Rust workspace (temper-core, temper-cli), sqlx, cargo-make, cargo-nextest. Tests use `serial_test` for filesystem-touching cases.

**Reference docs:**
- Spec addendum: `docs/superpowers/specs/2026-05-13-wave1-phase4-completion-b5b-addendum.md`
- Parent spec: `docs/superpowers/specs/2026-05-11-wave1-phase4-vaultbackend-design.md`
- Parent plan: `docs/superpowers/plans/2026-05-13-wave1-phase4-completion.md`

---

## File Structure

**Files modified:**
- `crates/temper-core/src/operations/actions.rs` — extend `validate_create` with per-doctype pure invariants + tests
- `crates/temper-cli/src/vault_backend/vault_backend.rs` — add `compute_task_defaults`, wire into `create_resource`
- `crates/temper-cli/src/commands/resource.rs` — collapse Local-arm match dispatch; add `render_create_output` helper; inline `build_managed_meta_for_create` (C1); migrate `resolve_resource_id` callers (C2)
- `crates/temper-cli/src/actions/frontmatter.rs` — delete `build_managed_meta_for_create` + `NewResourceArgs` + their tests (C1)
- `crates/temper-cli/src/commands/research.rs` — inline build_managed_meta_for_create call (C1)
- `crates/temper-cli/src/commands/session.rs` — inline build_managed_meta_for_create call (C1)
- `crates/temper-cli/src/commands/task.rs` — migrate resolve_resource_id callers (C2)

**Files potentially deleted (clippy-driven):**
- Whatever clippy flags as dead after Task 5 lands. Likely: `actions::task::create` (the create wrapper, not the next_seq helper which we still use). Verify at Task 6.

---

## Task 1: Extend `validate_create` with per-doctype pure invariants

**Files:**
- Modify: `crates/temper-core/src/operations/actions.rs:236-246` (function body) and `:298-end` (test module)

The existing `validate_create` validates slug, doctype, context, title. We extend it to add per-doctype invariants: mode/effort whitelist for `DocType::Task`. Branches via `match DocType::from_str(&cmd.doctype)?` — no string-literal compares.

Mode whitelist: `["plan", "build"]`. Effort whitelist: `["small", "medium", "large"]`. Both are pulled from `temper-core`'s existing managed-meta type definitions if a constant exists there; otherwise defined inline as `const` in `actions.rs`.

- [ ] **Step 1: Locate existing mode/effort whitelist sources (if any)**

Run: `grep -rn "\"plan\"\|\"build\"\|plan_or_build\|task_modes\|MODE_OPTIONS\|EFFORT_OPTIONS" /Users/petetaylor/projects/tasker-systems/temper/crates/temper-core/src/`

Expected: identify whether a canonical whitelist constant exists in `temper-core` already. If yes, reuse it. If no, define `const VALID_TASK_MODES: &[&str] = &["plan", "build"];` and `const VALID_TASK_EFFORTS: &[&str] = &["small", "medium", "large"];` at the top of `actions.rs` (after imports).

- [ ] **Step 2: Write failing test for mode whitelist rejection on Task**

Add to the existing test module at the bottom of `actions.rs`:

```rust
#[test]
fn validate_create_rejects_task_with_unknown_mode() {
    use crate::operations::commands::{CreateResource, Surface};
    use crate::types::ManagedMeta;

    let cmd = CreateResource {
        slug: "2026-05-14-test-task".to_string(),
        doctype: "task".to_string(),
        context: "temper".to_string(),
        title: "Test task".to_string(),
        body: None,
        managed_meta: ManagedMeta {
            mode: Some("nonsense".to_string()),
            effort: Some("small".to_string()),
            goal: Some("temper-maintenance".to_string()),
            ..ManagedMeta::default()
        },
        open_meta: None,
        origin_uri: None,
        chunks_packed: None,
        content_hash: None,
        origin: Surface::CliLocalVault,
    };

    let err = validate_create(&cmd).unwrap_err();
    assert!(
        format!("{err:?}").contains("mode") || format!("{err:?}").contains("nonsense"),
        "expected error mentioning mode/nonsense, got: {err:?}"
    );
}
```

- [ ] **Step 3: Run test to verify it fails**

Run: `cargo nextest run -p temper-core actions::tests::validate_create_rejects_task_with_unknown_mode`
Expected: FAIL — current `validate_create` doesn't check mode.

- [ ] **Step 4: Extend `validate_create` with per-doctype branch**

Modify `validate_create` (lines 236-246) to:

```rust
pub fn validate_create(cmd: &CreateResource) -> Result<(), ActionError> {
    validate_slug(&cmd.slug)?;
    validate_doctype(&cmd.doctype)?;
    if cmd.context.is_empty() {
        return Err(ActionError::MissingRequiredField("context".to_string()));
    }
    if cmd.title.is_empty() {
        return Err(ActionError::MissingRequiredField("title".to_string()));
    }

    let doctype = crate::frontmatter::DocType::from_str(&cmd.doctype)
        .map_err(|e| ActionError::InvalidValue(format!("doctype: {e}")))?;

    match doctype {
        crate::frontmatter::DocType::Task => {
            if let Some(mode) = cmd.managed_meta.mode.as_deref() {
                if !VALID_TASK_MODES.contains(&mode) {
                    return Err(ActionError::InvalidValue(format!(
                        "mode '{mode}' not in {VALID_TASK_MODES:?}"
                    )));
                }
            }
            if let Some(effort) = cmd.managed_meta.effort.as_deref() {
                if !VALID_TASK_EFFORTS.contains(&effort) {
                    return Err(ActionError::InvalidValue(format!(
                        "effort '{effort}' not in {VALID_TASK_EFFORTS:?}"
                    )));
                }
            }
        }
        crate::frontmatter::DocType::Goal
        | crate::frontmatter::DocType::Session
        | crate::frontmatter::DocType::Research
        | crate::frontmatter::DocType::Concept
        | crate::frontmatter::DocType::Decision => {
            // No additional per-doctype pure invariants beyond the generic checks above.
        }
    }

    Ok(())
}
```

If `ActionError::InvalidValue` does not exist, use whichever existing variant fits ("InvalidField" or similar — grep `ActionError` enum to confirm the closest match). If no fitting variant exists, add a new variant `InvalidValue(String)` to the enum.

- [ ] **Step 5: Run test to verify it passes**

Run: `cargo nextest run -p temper-core actions::tests::validate_create_rejects_task_with_unknown_mode`
Expected: PASS.

- [ ] **Step 6: Add corresponding effort-rejection test**

```rust
#[test]
fn validate_create_rejects_task_with_unknown_effort() {
    use crate::operations::commands::{CreateResource, Surface};
    use crate::types::ManagedMeta;

    let cmd = CreateResource {
        slug: "2026-05-14-test-task".to_string(),
        doctype: "task".to_string(),
        context: "temper".to_string(),
        title: "Test task".to_string(),
        body: None,
        managed_meta: ManagedMeta {
            mode: Some("plan".to_string()),
            effort: Some("gigantic".to_string()),
            goal: Some("temper-maintenance".to_string()),
            ..ManagedMeta::default()
        },
        open_meta: None,
        origin_uri: None,
        chunks_packed: None,
        content_hash: None,
        origin: Surface::CliLocalVault,
    };

    let err = validate_create(&cmd).unwrap_err();
    assert!(
        format!("{err:?}").contains("effort") || format!("{err:?}").contains("gigantic"),
        "expected error mentioning effort/gigantic, got: {err:?}"
    );
}
```

Run: `cargo nextest run -p temper-core actions::tests::validate_create_rejects_task_with_unknown_effort`
Expected: PASS.

- [ ] **Step 7: Add a positive test for valid task**

```rust
#[test]
fn validate_create_accepts_valid_task() {
    use crate::operations::commands::{CreateResource, Surface};
    use crate::types::ManagedMeta;

    let cmd = CreateResource {
        slug: "2026-05-14-test-task".to_string(),
        doctype: "task".to_string(),
        context: "temper".to_string(),
        title: "Test task".to_string(),
        body: None,
        managed_meta: ManagedMeta {
            mode: Some("plan".to_string()),
            effort: Some("medium".to_string()),
            goal: Some("temper-maintenance".to_string()),
            ..ManagedMeta::default()
        },
        open_meta: None,
        origin_uri: None,
        chunks_packed: None,
        content_hash: None,
        origin: Surface::CliLocalVault,
    };

    validate_create(&cmd).expect("valid task should pass validation");
}
```

Run: `cargo nextest run -p temper-core actions::tests::validate_create_accepts_valid_task`
Expected: PASS.

- [ ] **Step 8: Add a positive test for non-task doctype with arbitrary mode/effort**

This is the regression-guard ensuring we don't accidentally apply task-only whitelists to other doctypes (e.g., a future research doc carrying `mode: "exploratory"` shouldn't be rejected by the task whitelist).

```rust
#[test]
fn validate_create_accepts_research_with_arbitrary_managed_meta() {
    use crate::operations::commands::{CreateResource, Surface};
    use crate::types::ManagedMeta;

    let cmd = CreateResource {
        slug: "2026-05-14-test-research".to_string(),
        doctype: "research".to_string(),
        context: "temper".to_string(),
        title: "Test research".to_string(),
        body: None,
        managed_meta: ManagedMeta {
            mode: Some("anything".to_string()),
            ..ManagedMeta::default()
        },
        open_meta: None,
        origin_uri: None,
        chunks_packed: None,
        content_hash: None,
        origin: Surface::CliLocalVault,
    };

    validate_create(&cmd).expect("non-task doctype should not be subject to task whitelist");
}
```

Run: `cargo nextest run -p temper-core actions::tests::validate_create_accepts_research_with_arbitrary_managed_meta`
Expected: PASS.

- [ ] **Step 9: Run full temper-core test suite to verify nothing regressed**

Run: `cargo nextest run -p temper-core`
Expected: All green.

- [ ] **Step 10: Run `cargo make check` to verify clippy/fmt clean**

Run: `cargo make check`
Expected: All checks pass clean.

- [ ] **Step 11: Commit**

```bash
git add crates/temper-core/src/operations/actions.rs
git commit -m "$(cat <<'EOF'
phase4-completion B5b-1: extend validate_create with per-doctype invariants

Adds mode/effort whitelist validation for DocType::Task in
temper_core::operations::actions::validate_create. Branches via match
DocType::from_str — no string-literal comparisons. Other doctypes
remain subject to slug/doctype/context/title checks only. Backend
trait surface unchanged.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 2: Add `compute_task_defaults` helper inside `vault_backend/`

**Files:**
- Modify: `crates/temper-cli/src/vault_backend/vault_backend.rs` (add new private function + tests)

Add a new private associated function (or free function in the `vault_backend` module) `compute_task_defaults(config: &Config, cmd: &CreateResource) -> Result<TaskDefaults, TemperError>` that:
1. If `cmd.doctype == "task"` (via `DocType::from_str` match): computes `next_seq` via existing `actions::task::next_seq(config, &cmd.context, &goal_slug)` and verifies goal-exists via `actions::goal::find_goal(config, &cmd.context, &goal_slug)?.is_some()`.
2. For non-task doctypes: returns a no-op `TaskDefaults` with `seq: None`.

`TaskDefaults` is a small new struct with `seq: Option<u32>` (extensible later for other backend-derived defaults).

- [ ] **Step 1: Confirm `actions::task::next_seq` and `actions::goal::find_goal` signatures**

Run: `grep -n "pub fn next_seq\|pub fn find_goal" /Users/petetaylor/projects/tasker-systems/temper/crates/temper-cli/src/actions/task.rs /Users/petetaylor/projects/tasker-systems/temper/crates/temper-cli/src/actions/goal.rs`

Expected output identifies:
- `actions::task::next_seq(config: &Config, context: &str, goal_slug: &str) -> Result<u32>`
- `actions::goal::find_goal(config: &Config, context: &str, slug: &str) -> Result<Option<GoalInfo>>`

If signatures differ, adjust Task 2 code below to match the actual signatures.

- [ ] **Step 2: Write failing test for `compute_task_defaults` returning a seq for a task**

This is a unit test that needs a temporary vault with a goal directory. Add to the existing test module at the bottom of `vault_backend.rs` (if no module exists, create `#[cfg(test)] mod tests { ... }` at file end):

```rust
#[cfg(test)]
mod compute_task_defaults_tests {
    use super::*;
    use temper_core::operations::commands::{CreateResource, Surface};
    use temper_core::types::ManagedMeta;
    use tempfile::TempDir;

    fn make_test_config_with_goal(tmpdir: &TempDir, ctx: &str, goal_slug: &str) -> Config {
        // Create vault structure: <tmpdir>/me/<ctx>/goal/<goal_slug>.md
        let goal_dir = tmpdir.path().join("me").join(ctx).join("goal");
        std::fs::create_dir_all(&goal_dir).unwrap();
        let goal_file = goal_dir.join(format!("{goal_slug}.md"));
        std::fs::write(
            &goal_file,
            format!(
                "---\ntemper-slug: {goal_slug}\ntemper-title: Test Goal\ntemper-type: goal\n---\n\n# Test Goal\n"
            ),
        )
        .unwrap();

        Config {
            vault_root: tmpdir.path().to_path_buf(),
            owner: "me".to_string(),
            // ... fill the rest from Config::default() or existing test helpers
            ..Config::default()
        }
    }

    #[test]
    fn compute_task_defaults_returns_seq_for_task_with_existing_goal() {
        let tmpdir = TempDir::new().unwrap();
        let config = make_test_config_with_goal(&tmpdir, "temper", "temper-maintenance");

        let cmd = CreateResource {
            slug: "2026-05-14-test-task".to_string(),
            doctype: "task".to_string(),
            context: "temper".to_string(),
            title: "Test".to_string(),
            body: None,
            managed_meta: ManagedMeta {
                goal: Some("temper-maintenance".to_string()),
                mode: Some("plan".to_string()),
                effort: Some("small".to_string()),
                ..ManagedMeta::default()
            },
            open_meta: None,
            origin_uri: None,
            chunks_packed: None,
            content_hash: None,
            origin: Surface::CliLocalVault,
        };

        let defaults = compute_task_defaults(&config, &cmd).expect("should succeed");
        assert_eq!(defaults.seq, Some(10), "first task in goal should get seq=10");
    }
}
```

If the `Config` struct shape differs significantly from this sketch, look at existing test helpers in `crates/temper-cli/tests/common/` or sibling unit tests in the crate for the correct construction pattern, and use that.

- [ ] **Step 3: Run test to verify it fails**

Run: `cargo nextest run -p temper-cli vault_backend::vault_backend::compute_task_defaults_tests::compute_task_defaults_returns_seq_for_task_with_existing_goal`
Expected: FAIL — `compute_task_defaults` function does not exist.

- [ ] **Step 4: Implement `compute_task_defaults` and `TaskDefaults`**

Add near the top of `vault_backend.rs` (after existing struct definitions, before `impl VaultBackend`):

```rust
/// Backend-specific compute results for a `CreateResource` command.
///
/// Holds per-doctype default values that require filesystem access
/// (next_seq for tasks). Populated by `compute_task_defaults` before
/// dispatching to `per_doctype::write_for`.
#[derive(Debug, Default)]
pub(crate) struct TaskDefaults {
    pub(crate) seq: Option<u32>,
}

/// Compute backend-specific defaults for a Create cmd.
///
/// For tasks, walks the vault to find `next_seq` and verifies that the
/// referenced goal exists. For non-task doctypes, returns an empty
/// `TaskDefaults`.
///
/// Returns `BadRequest` if the referenced goal is missing.
pub(crate) fn compute_task_defaults(
    config: &Config,
    cmd: &CreateResource,
) -> Result<TaskDefaults, TemperError> {
    let doctype = temper_core::frontmatter::DocType::from_str(&cmd.doctype)
        .map_err(|e| TemperError::BadRequest(format!("invalid doctype: {e}")))?;

    match doctype {
        temper_core::frontmatter::DocType::Task => {
            let goal_slug = cmd.managed_meta.goal.as_deref().ok_or_else(|| {
                TemperError::BadRequest("task requires managed_meta.goal".to_string())
            })?;

            // Goal-exists check: stat-equivalent via find_goal.
            if crate::actions::goal::find_goal(config, &cmd.context, goal_slug)?.is_none() {
                return Err(TemperError::BadRequest(format!(
                    "goal '{goal_slug}' not found in context '{}'",
                    cmd.context
                )));
            }

            let seq = crate::actions::task::next_seq(config, &cmd.context, goal_slug)?;
            Ok(TaskDefaults { seq: Some(seq) })
        }
        _ => Ok(TaskDefaults::default()),
    }
}
```

If `TemperError::BadRequest` doesn't exist, use the closest variant (e.g., `TemperError::Vault` or `TemperError::Project`). Confirm by grepping `enum TemperError` in `crates/temper-cli/src/error.rs`.

- [ ] **Step 5: Run test to verify it passes**

Run: `cargo nextest run -p temper-cli vault_backend::vault_backend::compute_task_defaults_tests::compute_task_defaults_returns_seq_for_task_with_existing_goal`
Expected: PASS.

- [ ] **Step 6: Add tests for missing-goal and non-task doctype paths**

```rust
#[test]
fn compute_task_defaults_errors_when_goal_missing() {
    let tmpdir = TempDir::new().unwrap();
    let config = make_test_config_with_goal(&tmpdir, "temper", "real-goal");

    let cmd = CreateResource {
        slug: "2026-05-14-test-task".to_string(),
        doctype: "task".to_string(),
        context: "temper".to_string(),
        title: "Test".to_string(),
        body: None,
        managed_meta: ManagedMeta {
            goal: Some("nonexistent-goal".to_string()),
            mode: Some("plan".to_string()),
            effort: Some("small".to_string()),
            ..ManagedMeta::default()
        },
        open_meta: None,
        origin_uri: None,
        chunks_packed: None,
        content_hash: None,
        origin: Surface::CliLocalVault,
    };

    let err = compute_task_defaults(&config, &cmd).unwrap_err();
    assert!(
        format!("{err:?}").contains("nonexistent-goal"),
        "expected error mentioning missing goal, got: {err:?}"
    );
}

#[test]
fn compute_task_defaults_is_noop_for_non_task() {
    let tmpdir = TempDir::new().unwrap();
    let config = make_test_config_with_goal(&tmpdir, "temper", "real-goal");

    let cmd = CreateResource {
        slug: "2026-05-14-test-research".to_string(),
        doctype: "research".to_string(),
        context: "temper".to_string(),
        title: "Test".to_string(),
        body: None,
        managed_meta: ManagedMeta::default(),
        open_meta: None,
        origin_uri: None,
        chunks_packed: None,
        content_hash: None,
        origin: Surface::CliLocalVault,
    };

    let defaults = compute_task_defaults(&config, &cmd).expect("should succeed");
    assert_eq!(defaults.seq, None);
}
```

Run: `cargo nextest run -p temper-cli vault_backend::vault_backend::compute_task_defaults_tests`
Expected: All three tests PASS.

- [ ] **Step 7: Run `cargo make check`**

Run: `cargo make check`
Expected: Clean.

- [ ] **Step 8: Commit**

```bash
git add crates/temper-cli/src/vault_backend/vault_backend.rs
git commit -m "$(cat <<'EOF'
phase4-completion B5b-2: add compute_task_defaults to vault_backend

Adds private `compute_task_defaults` helper that walks the vault to
compute next_seq for tasks and verifies goal-exists. Returns BadRequest
when a referenced goal is missing. No-op for non-task doctypes. Not yet
wired into VaultBackend::create_resource — that's the next commit.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 3: Wire `compute_task_defaults` into `VaultBackend::create_resource`

**Files:**
- Modify: `crates/temper-cli/src/vault_backend/vault_backend.rs:351-441` (`create_resource` body)

Insert the `compute_task_defaults` call after `validate_create` succeeds and before defaults-application. Thread the returned `TaskDefaults.seq` into `cmd.managed_meta.seq` (or whatever existing field carries seq — verify by reading the current `apply_doc_type_defaults` call to see what field is populated).

The existing integration tests serve as the regression guard: with `compute_task_defaults` wired in, all existing `create_*` task tests must still pass (since they create tasks with valid goal references and rely on next_seq computation that previously happened in `actions::task::create`).

- [ ] **Step 1: Read the current `create_resource` body**

Read: `/Users/petetaylor/projects/tasker-systems/temper/crates/temper-cli/src/vault_backend/vault_backend.rs` lines 351-441.

Identify:
- Where `validate_create` is called.
- Where managed_meta defaults are applied.
- Whether managed_meta.seq is currently being set somewhere else (e.g., in `apply_doc_type_defaults` or in `per_doctype::write_task`).

- [ ] **Step 2: Identify the current seq population path**

Run: `grep -n "seq\s*[:=]\|\.seq\s*=" /Users/petetaylor/projects/tasker-systems/temper/crates/temper-cli/src/vault_backend/per_doctype.rs`

Expected: locate where `write_task` currently picks seq (likely from `WriteArgs.fields.task.seq` — see B5a's `doctype_fields` wiring).

The exact wiring point depends on B5a's structure: either (a) seq flows in via `cmd.managed_meta.seq` and `per_doctype::write_task` reads it, or (b) seq is computed inside `per_doctype::write_task` itself. Read enough of `write_task` (line 286+) to confirm which.

If (a): in `create_resource`, after `validate_create` passes, call `compute_task_defaults` and assign `cmd.managed_meta.seq = task_defaults.seq` (cmd is owned, so this is a direct field write).

If (b): in `write_task`, the seq is computed internally — meaning Task 2's `compute_task_defaults` would duplicate. In that case, refactor `write_task` to read seq from `cmd.managed_meta.seq` instead, AND wire `compute_task_defaults` at the top of `create_resource` as in (a). This factors backend-compute out of `per_doctype` and into `vault_backend.rs`'s `create_resource`, matching the spec.

- [ ] **Step 3: Wire `compute_task_defaults` into `create_resource`**

After the existing `validate_create(&cmd)?;` line in `create_resource`, insert:

```rust
        let task_defaults = compute_task_defaults(self.ctx.config(), &cmd)?;
        if let Some(seq) = task_defaults.seq {
            cmd.managed_meta.seq = Some(seq);
        }
```

If `cmd.managed_meta.seq` already has a value (e.g., caller explicitly set it — unlikely from CLI but possible from tests), the assignment overwrites. Acceptable for B5b since the surface (post-collapse) won't pre-populate seq.

- [ ] **Step 4: Run the existing temper-cli test suite as the regression guard**

Run: `cargo nextest run -p temper-cli --features test-db`
Expected: All green. Pay attention to any task-create tests.

If any task-create tests fail because seq was previously computed via a different path and the result differs, investigate by reading the test's expected seq value vs the new computation. Either the test had a stale expectation (fix the test) or the wiring is wrong (fix the wiring) — do NOT relax assertions to make tests pass; surface the discrepancy.

- [ ] **Step 5: Run e2e tests**

Run: `cargo nextest run --manifest-path tests/e2e/Cargo.toml --features test-db,test-embed`
Expected: All green.

- [ ] **Step 6: Run `cargo make check`**

Run: `cargo make check`
Expected: Clean.

- [ ] **Step 7: Commit**

```bash
git add crates/temper-cli/src/vault_backend/vault_backend.rs crates/temper-cli/src/vault_backend/per_doctype.rs
git commit -m "$(cat <<'EOF'
phase4-completion B5b-3: wire compute_task_defaults into VaultBackend::create_resource

VaultBackend::create_resource now invokes compute_task_defaults after
validate_create, populating managed_meta.seq for tasks (via filesystem
walk) and erroring on missing-goal references. Per-doctype writers in
vault_backend/per_doctype.rs now read seq from cmd.managed_meta rather
than recomputing internally.

Regression guard: full temper-cli + e2e suites green.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 4: Add `render_create_output` surface helper

**Files:**
- Modify: `crates/temper-cli/src/commands/resource.rs` (add helper near top, before `create` function)

Add a new private helper that takes the `CommandOutput<ResourceRow>` returned by `backend.create_resource(cmd)`, the doctype string, and the format flag, and emits the exact JSON shape each doctype previously produced. The helper matches on `DocType` (parsed once) and emits the appropriate shape.

Before writing, **audit the existing JSON shapes** for each doctype's create path:

- [ ] **Step 1: Inventory existing per-doctype JSON output shapes**

Read each existing output path:
- Task: `commands/resource.rs` lines 158-167 (already shown — `{"type", "temper-slug", "temper-title", "temper-context"}`)
- Goal: read `commands/goal.rs::create` to find its JSON emit
- Session: read `commands/session.rs::save` to find its JSON emit
- Research: read `commands/research.rs::save` to find its JSON emit
- Concept/Decision: read `commands/resource.rs::create_simple_resource` (lines 209+) to find its JSON emit

Run (in parallel):
```bash
grep -n "format\s*==\s*\"json\"\|to_string_pretty\|println!" /Users/petetaylor/projects/tasker-systems/temper/crates/temper-cli/src/commands/goal.rs
grep -n "format\s*==\s*\"json\"\|to_string_pretty\|println!" /Users/petetaylor/projects/tasker-systems/temper/crates/temper-cli/src/commands/session.rs
grep -n "format\s*==\s*\"json\"\|to_string_pretty\|println!" /Users/petetaylor/projects/tasker-systems/temper/crates/temper-cli/src/commands/research.rs
```

Record each shape in a comment block in the new helper (so the spec lives next to the code).

- [ ] **Step 2: Inventory tests that assert JSON output shape**

Run: `grep -rn "\"type\"\s*:\s*\"task\"\|\"temper-slug\"\|\"temper-title\"\|\"type\"\s*:\s*\"goal\"" /Users/petetaylor/projects/tasker-systems/temper/crates/temper-cli/tests/ /Users/petetaylor/projects/tasker-systems/temper/tests/e2e/tests/`

Record the list — these are the tests that B5b's `render_create_output` must keep green.

- [ ] **Step 3: Write a failing test for `render_create_output` task shape**

Add to the test module at the bottom of `commands/resource.rs` (or create one if none exists):

```rust
#[cfg(test)]
mod render_create_output_tests {
    use super::*;
    use temper_core::operations::CommandOutput;
    use temper_core::types::{ManagedMeta, ResourceRow, ResourceId};
    use chrono::Utc;
    use uuid::Uuid;

    fn make_resource_row(slug: &str, doctype: &str, title: &str, context: &str) -> ResourceRow {
        ResourceRow {
            id: ResourceId::from(Uuid::now_v7()),
            slug: Some(slug.to_string()),
            doctype: doctype.to_string(),
            context: context.to_string(),
            title: title.to_string(),
            // ... fill remaining ResourceRow fields with defaults; check the struct definition
            ..ResourceRow::test_default(slug, doctype, title, context)
        }
    }

    #[test]
    fn render_create_output_task_json_matches_legacy_shape() {
        let row = make_resource_row("2026-05-14-test", "task", "Test", "temper");
        let output = CommandOutput { value: row, events: vec![] };
        let json = render_create_output_to_string(&output, "task", "json")
            .expect("rendering task JSON should succeed");

        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["type"], "task");
        assert_eq!(parsed["temper-slug"], "2026-05-14-test");
        assert_eq!(parsed["temper-title"], "Test");
        assert_eq!(parsed["temper-context"], "temper");
    }
}
```

`render_create_output_to_string` is a test-only sibling of `render_create_output` that returns the JSON string instead of writing to stdout. Implement it as part of Step 4 (it's the testable core; the printing wrapper is trivial).

If `ResourceRow::test_default` doesn't exist, construct the full row by hand using whatever the struct requires.

Run: `cargo nextest run -p temper-cli commands::resource::render_create_output_tests::render_create_output_task_json_matches_legacy_shape`
Expected: FAIL — function doesn't exist yet.

- [ ] **Step 4: Implement `render_create_output` + `render_create_output_to_string`**

Add to `commands/resource.rs` (above the `create` function):

```rust
/// Render the result of `VaultBackend::create_resource` to stdout in the
/// shape that each doctype's pre-B5b dispatch path emitted.
///
/// Doctype-aware switch preserves backward-compatible JSON output. The
/// dispatch itself is now uniform; only output shape varies by doctype.
fn render_create_output(
    output: &CommandOutput<ResourceRow>,
    doc_type: &str,
    format: &str,
) -> Result<()> {
    let rendered = render_create_output_to_string(output, doc_type, format)?;
    if !rendered.is_empty() {
        println!("{rendered}");
    }
    Ok(())
}

/// Test-friendly core of `render_create_output` — returns the string that
/// would be printed (empty string for the non-JSON success path).
fn render_create_output_to_string(
    output: &CommandOutput<ResourceRow>,
    doc_type: &str,
    format: &str,
) -> Result<String> {
    let row = &output.value;
    let doctype = temper_core::frontmatter::DocType::from_str(doc_type)
        .map_err(|e| TemperError::Vault(format!("invalid doctype: {e}")))?;

    if format != "json" {
        // Non-JSON path: mimic existing success-line output per doctype.
        let slug_display = row.slug.as_deref().unwrap_or("(no slug)");
        output::success(format!("Created: {slug_display}"));
        return Ok(String::new());
    }

    // IMPORTANT: each branch below MUST emit the exact JSON shape recorded
    // in Step 1's audit. The Task arm is verified from existing source
    // (commands/resource.rs:158-167); the other 5 arms are placeholders
    // that THIS STEP FAILS TO COMPLETE if Step 1 was skipped. After Step 1
    // records each doctype's shape, replace each branch's `serde_json::json!`
    // contents to match exactly. Step 2's test inventory is the gate that
    // catches any divergence.
    let json = match doctype {
        temper_core::frontmatter::DocType::Task => serde_json::json!({
            "type": "task",
            "temper-slug": row.slug,
            "temper-title": row.title,
            "temper-context": row.context,
        }),
        temper_core::frontmatter::DocType::Goal => {
            // Replace with shape recorded from commands/goal.rs::create in Step 1.
            return Err(TemperError::Vault(
                "render_create_output: Goal arm not yet wired from audit (Step 1)".to_string(),
            ));
        }
        temper_core::frontmatter::DocType::Session => {
            // Replace with shape recorded from commands/session.rs::save in Step 1.
            return Err(TemperError::Vault(
                "render_create_output: Session arm not yet wired from audit (Step 1)".to_string(),
            ));
        }
        temper_core::frontmatter::DocType::Research => {
            // Replace with shape recorded from commands/research.rs::save in Step 1.
            return Err(TemperError::Vault(
                "render_create_output: Research arm not yet wired from audit (Step 1)".to_string(),
            ));
        }
        temper_core::frontmatter::DocType::Concept
        | temper_core::frontmatter::DocType::Decision => {
            // Replace with shape recorded from commands/resource.rs::create_simple_resource in Step 1.
            return Err(TemperError::Vault(
                "render_create_output: Concept/Decision arm not yet wired from audit (Step 1)".to_string(),
            ));
        }
    };

    let s = serde_json::to_string_pretty(&json)
        .map_err(|e| TemperError::Vault(format!("json render failed: {e}")))?;
    Ok(s)
}
```

**Important:** The Task arm above is the only one with a verified shape (from `commands/resource.rs:158-167`). The other 5 arms intentionally return errors so that Step 6's per-doctype tests fail loudly until Step 1's audit is performed and each arm's `serde_json::json!` body is filled in with the exact shape that doctype's pre-B5b path emitted.

This is structured so that **the test for each doctype will fail with a clear error message** ("not yet wired from audit") if the implementer skipped Step 1. The test failure points directly at the audit step that needs completion. No silent placeholders.

The "Typed structs over inline JSON" rule in `tasker-systems/temper/CLAUDE.md` applies: if any of these shapes warrants reuse beyond `render_create_output`, define a typed struct. For 6 single-use shapes confined to one helper, inline `serde_json::json!` is acceptable — these aren't wire-protocol contracts, they're CLI output shapes.

- [ ] **Step 5: Run test to verify it passes for task**

Run: `cargo nextest run -p temper-cli commands::resource::render_create_output_tests::render_create_output_task_json_matches_legacy_shape`
Expected: PASS.

- [ ] **Step 6: Add tests for each remaining doctype**

Mirror the task test for goal, session, research, concept, decision. Each test asserts the exact field shape recorded in Step 1.

Run: `cargo nextest run -p temper-cli commands::resource::render_create_output_tests`
Expected: All 6 PASS.

- [ ] **Step 7: Run `cargo make check`**

Run: `cargo make check`
Expected: Clean. Note: `render_create_output` may emit a dead-code warning here because nothing calls it yet — this is expected and gets resolved in Task 5.

If clippy fails with `-D warnings` because of the dead-code warning, mark `render_create_output` and `render_create_output_to_string` with `#[expect(dead_code, reason = "wired in next commit, Task 5")]`. **This is the one explicit exception** to the "don't suppress dead-code" rule — it's a transient marker between commits within this task series, with a reason field pointing to the resolving commit. Remove the `#[expect]` in Task 5.

- [ ] **Step 8: Commit**

```bash
git add crates/temper-cli/src/commands/resource.rs
git commit -m "$(cat <<'EOF'
phase4-completion B5b-4: add render_create_output surface helper

Adds doctype-aware JSON output renderer for the collapsed Local-mode
create path. Preserves each doctype's existing JSON shape via a
DocType match. Not yet wired into commands::resource::create — that's
the next commit.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 5: Collapse `commands/resource.rs::create` Local-arm to uniform dispatch

**Files:**
- Modify: `crates/temper-cli/src/commands/resource.rs:144-205` (the Local-arm `match doc_type` block)

Replace the entire Local-arm `match doc_type { ... }` (lines 146-205) with a single uniform dispatch through `VaultBackend::create_resource`. The pre-match code (stdin read at line 144, slug resolution) is consolidated into the new flow.

The 6 doctype branches are gone. The 4 helper calls (`actions::task::create`, `commands::goal::create`, `commands::session::save`, `commands::research::save`) are no longer invoked. They remain defined; clippy will flag any that are now unreferenced — handled in Task 6.

- [ ] **Step 1: Re-read the current Local-arm body**

Read: `/Users/petetaylor/projects/tasker-systems/temper/crates/temper-cli/src/commands/resource.rs` lines 139-206.

Confirm:
- The match dispatch starts at line 146.
- The slug derivation logic (currently inside specific arms, e.g., concept gets no date prefix) needs to be lifted out.
- The body source (currently `stdin_content` at line 144) is consistent across arms.

- [ ] **Step 2: Write the new Local-arm body**

Replace lines 139-205 with:

```rust
    // Local-mode: existing vault-file create flow.
    // body_flag is intentionally unused in local mode (stdin piping handles body).
    let _ = body_flag;
    let _ = vault_state;

    let stdin_content = vault::read_stdin_if_piped();

    let doctype_enum = temper_core::frontmatter::DocType::from_str(doc_type)?;

    let slug_resolved = slug.map(String::from).unwrap_or_else(|| {
        let today = Local::now().format("%Y-%m-%d").to_string();
        let base_slug = vault::slugify(title);
        match doctype_enum {
            temper_core::frontmatter::DocType::Concept => base_slug,
            _ => format!("{today}-{base_slug}"),
        }
    });

    let body = stdin_content.unwrap_or_default();

    let cmd = temper_core::operations::commands::CreateResource {
        slug: slug_resolved,
        doctype: doc_type.to_string(),
        context: ctx.to_string(),
        title: title.to_string(),
        body: if body.is_empty() {
            None
        } else {
            Some(temper_core::types::BodyUpdate {
                content: body,
                content_hash: None,
            })
        },
        managed_meta: temper_core::types::ManagedMeta {
            mode: mode.map(String::from),
            effort: effort.map(String::from),
            goal: goal.map(String::from),
            ..temper_core::types::ManagedMeta::default()
        },
        open_meta: None,
        origin_uri: None,
        chunks_packed: None,
        content_hash: None,
        origin: temper_core::operations::commands::Surface::CliLocalVault,
    };

    let (runtime, backend_ctx) =
        crate::vault_backend::assemble_vault_backend(config, &ctx)?;
    let backend = crate::vault_backend::VaultBackend::new(backend_ctx);
    let output = runtime.block_on(backend.create_resource(cmd))?;

    render_create_output(&output, doc_type, format)
```

Notes:
- The `_ = body_flag` line preserves existing behavior of ignoring `body_flag` in Local mode (the agent should confirm this is still the intent; if `body_flag` should now be honored in Local mode, that's an out-of-scope change — keep as `_ = body_flag` for B5b).
- `temper_core::operations::commands::CreateResource` and friends may already be imported at the file top; reuse imports if present, otherwise add to the `use` block.
- Remove the `#[expect(dead_code, reason = "wired in next commit, Task 5")]` from `render_create_output` and `render_create_output_to_string` added in Task 4 Step 7 if it was added.

- [ ] **Step 3: Run the full create-test regression guard**

Run: `cargo nextest run -p temper-cli --features test-db`
Expected: All green. The existing `create_*` integration tests are the regression guard.

If any test fails, investigate per the rule: **fix the underlying code, don't soften assertions**. Common possible failures:
- **JSON shape mismatch** — recorded shape in Task 4 Step 1 was wrong; update `render_create_output` to match.
- **Session/research save-or-update test failure** — the test invokes `temper resource create --type session` for an existing slug and asserts silent update. Per the spec, this now hard-errors. The test needs reframing (move to use `temper session save` directly, or assert the new error). This is the documented behavior change.
- **Missing `temper-id` or `temper-provisional-id`** — VaultBackend's create path emits one of these; if a test reads the file and expects something specific, adjust the test to match the new shape (this would have already been settled by B5a, but verify).

For any save-or-update test that's failing in a way that reflects the spec'd behavior change, reframe the test in place (don't delete) — adjust its expected outcome to match the new hard-error semantics and add a comment pointing to the spec addendum.

- [ ] **Step 4: Run e2e tests**

Run: `cargo nextest run --manifest-path tests/e2e/Cargo.toml --features test-db,test-embed`
Expected: All green.

- [ ] **Step 5: Run `cargo make check`**

Run: `cargo make check`
Expected: Will likely emit `dead_code` warnings for unreferenced functions (e.g., `actions::task::create` if nothing else calls it). With `-D warnings` clippy may fail. **DO NOT** suppress these — the warnings are the punchlist for Task 6. If the commit must land green, see "Commit strategy" below.

**Commit strategy for green-CI requirement:**
The codebase enforces `-D warnings`. If Task 5 lands a dead-code warning, `cargo make check` fails and so does `git commit` (pre-commit hook runs it). Two acceptable resolutions:

(a) **Combine Task 5 and Task 6 into one commit.** Land the dispatch collapse and the dead-code cleanup together. Single commit boundary is clean. This is the recommended path.

(b) **Use a single-commit `#[expect(dead_code, reason = "cleanup follows in Task 6 commit")]` marker** on each affected function, then in Task 6 remove both the markers and the functions. Two commits, both clean.

Default to (a). If (a) is chosen, skip Task 6 as a separate task — its work happens inside Task 5 as Steps 6-8 below.

- [ ] **Step 6 (if combining with Task 6): Read dead-code warnings**

Run: `cargo make check 2>&1 | grep -A1 "dead_code\|never used\|never read"`
Expected: list of functions/types/imports flagged as unused.

Compile the punchlist. Likely candidates:
- `actions::task::create` (the wrapper that pre-B5b was called from `resource.rs:148`)
- Possibly portions of `commands::goal::create`, `commands::session::save`, `commands::research::save` — but only if they have no other callers. Note that `temper goal create`, `temper session save`, `temper session start`, `temper research save`, `temper research finish` likely keep these alive; verify with `grep -rn "session::save\|research::save\|goal::create" /Users/petetaylor/projects/tasker-systems/temper/crates/temper-cli/src/`.

- [ ] **Step 7 (if combining with Task 6): Delete dead code; update task tracker tasks**

For each function clippy flags:
1. Delete the function.
2. Delete its unit tests (also dead).
3. If it's the only function in a module/file, delete the file too.
4. Update any `mod foo;` declarations that referenced the deleted file.

If a function looks like it shouldn't be deleted (e.g., it's a public API that's just not currently exercised in this binary), confirm by grepping the whole workspace before deletion:
```bash
grep -rn "fn_name" /Users/petetaylor/projects/tasker-systems/temper/
```

If no callers exist anywhere in the workspace, delete it. If callers exist in other crates (e.g., tests/e2e), keep it.

- [ ] **Step 8 (if combining with Task 6): Verify clean**

Run: `cargo make check`
Expected: All clean — no warnings.

Run: `cargo nextest run -p temper-cli --features test-db`
Expected: All green.

Run: `cargo nextest run --manifest-path tests/e2e/Cargo.toml --features test-db,test-embed`
Expected: All green.

- [ ] **Step 9: Commit (combined B5b-5 + B5b-6)**

```bash
git add crates/temper-cli/src/commands/resource.rs crates/temper-cli/src/actions/task.rs crates/temper-cli/src/commands/goal.rs crates/temper-cli/src/commands/session.rs crates/temper-cli/src/commands/research.rs
# Add any additional touched files based on what was deleted in Step 7

git commit -m "$(cat <<'EOF'
phase4-completion B5b-5: collapse commands/resource.rs::create Local arm

Local-mode create now dispatches uniformly through VaultBackend for all
6 doctypes. The per-doctype match at lines 146-205 is gone. Output
shape preservation handled by render_create_output's DocType switch.

Documented behavior change: `temper resource create --type session/research`
now hard-errors-on-exists (matches concept/decision). The dedicated
`temper session save` and `temper research save` subcommands preserve
their save-or-update overload via their existing logic, untouched here.

Dead-code cleanup: removes functions whose only callers were the
collapsed match arms (clippy-flagged after dispatch removal).

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 6 (separate-commit path, if Task 5 chose option (b))

If Task 5 chose option (b) — dispatch collapse with `#[expect(dead_code)]` markers in its own commit — this task removes the markers and deletes the flagged code. Otherwise this task is absorbed into Task 5.

**Files:**
- Modify or delete: whichever files clippy flagged in Task 5 Step 6.

- [ ] **Step 1: Re-read clippy warnings (in case state shifted)**

Run: `cargo make check 2>&1 | grep -A1 "dead_code\|never used\|never read"`
Expected: same list as Task 5 Step 6.

- [ ] **Step 2: Delete flagged functions and their `#[expect(dead_code)]` markers**

Per Task 5 Step 7's guidance.

- [ ] **Step 3: Verify clean**

Run: `cargo make check && cargo nextest run -p temper-cli --features test-db && cargo nextest run --manifest-path tests/e2e/Cargo.toml --features test-db,test-embed`
Expected: All clean and green.

- [ ] **Step 4: Commit**

```bash
git commit -m "$(cat <<'EOF'
phase4-completion B5b-6: delete dead code surfaced by dispatch collapse

Removes functions whose only callers were the per-doctype branches of
commands/resource.rs::create's Local-mode match (collapsed in B5b-5).

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 7 (C1): Inline `build_managed_meta_for_create` at 3 callers; delete helper

**Files:**
- Modify: `crates/temper-cli/src/commands/resource.rs:76` (cloud-mode create caller)
- Modify: `crates/temper-cli/src/commands/research.rs:61` (research save caller)
- Modify: `crates/temper-cli/src/commands/session.rs` (session save caller — grep for line)
- Delete (or modify to remove function): `crates/temper-cli/src/actions/frontmatter.rs:9-43` (`NewResourceArgs` + `build_managed_meta_for_create`)

- [ ] **Step 1: Locate the session.rs caller**

Run: `grep -n "build_managed_meta_for_create\|NewResourceArgs" /Users/petetaylor/projects/tasker-systems/temper/crates/temper-cli/src/commands/session.rs`

Record the line number.

- [ ] **Step 2: Read each caller's actual input**

For each of the 3 call sites, read 15 lines around the call and record exactly which `NewResourceArgs` fields are populated with non-`None` values. The inline replacement reproduces only the populated fields.

- [ ] **Step 3: Inline at `commands/resource.rs:76`**

Replace lines 76-91 (the call to `build_managed_meta_for_create`) with the equivalent inline `ManagedMeta { ... }` literal. Example from spec addendum (for the cloud-mode create at line 76):

```rust
let managed_meta = temper_core::types::ManagedMeta {
    mode: mode.map(String::from),
    effort: effort.map(String::from),
    goal: goal.map(String::from),
    ..temper_core::types::ManagedMeta::default()
};
```

Adjust based on what Step 2 found at this specific call site.

- [ ] **Step 4: Inline at `commands/research.rs:61`**

Same pattern. The research caller likely passes mostly None (research has no mode/effort/goal). The inline collapses to:

```rust
let managed_meta = temper_core::types::ManagedMeta::default();
```

Or, if the helper currently sets a title field that's not in `ManagedMeta::default()`, preserve that:

```rust
let managed_meta = temper_core::types::ManagedMeta {
    title: Some(title.to_string()),
    ..temper_core::types::ManagedMeta::default()
};
```

Note: `ManagedMeta::title` may or may not exist as a struct field — verify by grepping `pub struct ManagedMeta` in `crates/temper-core/`.

- [ ] **Step 5: Inline at the session.rs caller**

Same pattern at the line recorded in Step 1.

- [ ] **Step 6: Delete `build_managed_meta_for_create` + `NewResourceArgs` + their tests**

Modify `crates/temper-cli/src/actions/frontmatter.rs`:
- Delete the `NewResourceArgs` struct (lines 9-25).
- Delete the `build_managed_meta_for_create` function (lines 27-43).
- Delete any unit tests for either.
- If those were the only items in the file, leave a `pub use` re-export or delete the module and update `mod frontmatter;` in the parent.

Verify no other callers exist:
```bash
grep -rn "build_managed_meta_for_create\|NewResourceArgs" /Users/petetaylor/projects/tasker-systems/temper/
```

Expected after deletion: zero matches.

- [ ] **Step 7: Run regression suite**

Run: `cargo nextest run -p temper-cli --features test-db,embed`
Expected: All green.

- [ ] **Step 8: Run e2e and check**

Run: `cargo nextest run --manifest-path tests/e2e/Cargo.toml --features test-db,test-embed`
Run: `cargo make check`
Expected: All green and clean.

- [ ] **Step 9: Commit**

```bash
git add crates/temper-cli/src/commands/resource.rs crates/temper-cli/src/commands/research.rs crates/temper-cli/src/commands/session.rs crates/temper-cli/src/actions/frontmatter.rs
git commit -m "$(cat <<'EOF'
phase4-completion C1: inline build_managed_meta_for_create at 3 callers; delete helper

Three callers (cloud-mode create at resource.rs:76, research.rs:61,
session.rs caller) now construct ManagedMeta inline. Helper and
NewResourceArgs struct deleted. Eliminates one indirection layer that
B5b's local-mode create path already bypassed.

Closes vault task: 2026-05-11-delete-actions-frontmatter-build-managed-meta-for-create-after-phase-4b

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 8 (C2): Migrate `resolve_resource_id` callers; delete function

**Files:**
- Modify: `crates/temper-cli/src/commands/resource.rs` (lines 778, 973, 1122, 1191 callers + the function definition at 910-944)
- Possibly modify: `crates/temper-cli/src/commands/task.rs` (lines 70, 100 per parent plan — verify)
- Possibly modify: `crates/temper-cli/src/commands/session.rs` (lines 367, 413 per parent plan — verify)

The parent plan listed 6 callers; the verification report identified 4 in resource.rs and didn't confirm the other 2. Verify count before proceeding.

- [ ] **Step 1: Verify the full caller list**

Run: `grep -rn "resolve_resource_id" /Users/petetaylor/projects/tasker-systems/temper/crates/temper-cli/src/`

Record every line that calls `resolve_resource_id`. The function definition itself appears as one match — exclude that.

- [ ] **Step 2: Read the function body and identify the return type's structure**

Read `crates/temper-cli/src/commands/resource.rs:910-944`. Confirm:
- Return type: `Result<ResourceId>`.
- Body: tries local manifest lookup if VaultState::Local, else calls `client.resources().resolve_by_uri(...)`.

The migration replaces each caller with the appropriate direct call:
- **Local-mode lookup**: read the manifest directly. If the caller needs just the id, use whatever the manifest's local-resolution helper returns. Check for an existing `manifest::find_by_slug` or similar.
- **Cloud-mode / fallback**: `client.resources().resolve_by_uri(&format!("temper://contexts/{ctx}/{doctype}/{slug}"))` — verify URI format by grepping `resolve_by_uri` usages elsewhere.

The migration may simplify some callers (e.g., cloud-only paths drop the local branch entirely) and complicate others (callers that previously got "id from wherever it works" now have to do the mode check themselves).

- [ ] **Step 3: For each caller, read 15 lines of context and plan the replacement**

For each line recorded in Step 1, read the surrounding code to understand:
1. What is the caller doing with the returned `ResourceId`?
2. Is the caller already in a context with VaultState awareness?
3. Is a `client` already in scope, or does the caller need to acquire one?

Group callers by replacement pattern. The two likely patterns:

**Pattern A (cloud-only caller, already has client):**
```rust
// Before:
let id = resolve_resource_id(config, &client, doc_type, slug, Some(&ctx), vault_state).await?;

// After:
let uri = format!("temper://contexts/{ctx}/{doc_type}/{slug}");
let id = client.resources().resolve_by_uri(&uri).await?;
```

**Pattern B (mode-aware caller, needs local-or-cloud):**
```rust
// Before:
let id = resolve_resource_id(config, &client, doc_type, slug, Some(&ctx), vault_state).await?;

// After:
let id = if matches!(vault_state, VaultState::Local) {
    crate::manifest_io::find_by_slug(config, &ctx, doc_type, slug)?
        .ok_or_else(|| TemperError::NotFound(format!("{doc_type}/{slug}")))?
} else {
    let uri = format!("temper://contexts/{ctx}/{doc_type}/{slug}");
    client.resources().resolve_by_uri(&uri).await?
};
```

If `manifest_io::find_by_slug` doesn't exist with that exact name, find the appropriate manifest lookup helper. If no helper exists, factor one out (and add this work to Task 8's scope or descope C2).

If C2 turns out to need a manifest helper extraction, **stop and descope** per the spec addendum's checkpoint policy: land B5b + C1, and defer C2 to a new session/task.

- [ ] **Step 4: Apply Pattern A replacements**

For each Pattern-A caller, modify in place. Run after each modification:
```bash
cargo nextest run -p temper-cli --features test-db,embed
```
Expected: incremental green.

- [ ] **Step 5: Apply Pattern B replacements**

Same as Step 4 for Pattern-B callers.

- [ ] **Step 6: Delete `resolve_resource_id`**

Modify `crates/temper-cli/src/commands/resource.rs:910-944`: delete the entire function.

Verify no remaining callers:
```bash
grep -rn "resolve_resource_id" /Users/petetaylor/projects/tasker-systems/temper/
```
Expected: zero matches (or one match in a comment that we now update).

- [ ] **Step 7: Run full regression**

Run: `cargo nextest run -p temper-cli --features test-db,embed`
Run: `cargo nextest run --manifest-path tests/e2e/Cargo.toml --features test-db,test-embed`
Run: `cargo make check`
Expected: All green and clean.

- [ ] **Step 8: Commit**

```bash
git add crates/temper-cli/src/commands/resource.rs crates/temper-cli/src/commands/task.rs crates/temper-cli/src/commands/session.rs
# Plus any other touched files.

git commit -m "$(cat <<'EOF'
phase4-completion C2: migrate resolve_resource_id callers; delete function

All N callers (verified in Step 1) now use direct
client.resources().resolve_by_uri() for cloud-mode or
manifest-based lookup for local-mode. resolve_resource_id deleted
from commands/resource.rs. Surface code is explicit about mode
selection instead of hiding it behind a wrapper.

Closes vault task: 2026-05-11-delete-commands-resource-rs-resolve-resource-id-after-phase-4b

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

Replace `N` in the commit message with the actual caller count from Step 1.

---

## Final Verification (after all tasks land)

Before declaring B5b + C1 + C2 done:

- [ ] `cargo make check` — clean
- [ ] `cargo nextest run -p temper-cli --features test-db,embed` — all green
- [ ] `cargo nextest run --manifest-path tests/e2e/Cargo.toml --features test-db,test-embed` — all green
- [ ] Branch state: 13 + N commits ahead of main (where N is the number of B5b/C1/C2 commits landed — between 4 and 8)
- [ ] Active vault task `2026-05-11-wave-1-phase-4b-extract-commands-resource-rs-local-mode-writes-through-vaultbackend` ready for stage = done (defer to Phase D's PR-open task per parent plan)

If any descope checkpoint was hit (per the spec addendum: stop after B5b or B5b + C1 if scope expands), reflect the actual scope landed in the session-end note and roll the remainder forward to a new task in the vault.

---

## Cleanup Backlog Tracking

Two related vault tasks close when this plan completes:

- `2026-05-11-delete-actions-frontmatter-build-managed-meta-for-create-after-phase-4b` — closed by Task 7 (C1).
- `2026-05-11-delete-commands-resource-rs-resolve-resource-id-after-phase-4b` — closed by Task 8 (C2).

If either task is descoped, leave the corresponding vault task in the backlog with an updated note.
