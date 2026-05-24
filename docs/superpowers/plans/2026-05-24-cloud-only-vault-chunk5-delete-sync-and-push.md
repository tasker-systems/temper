# Cloud-only vault — Chunk 5: delete sync engine, `push`, sync_cmd subcommands, `research`, publish helper, dead e2e tests

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Delete the local-vault sync engine (`crates/temper-cli/src/actions/sync.rs`, ~4.4k lines) along with everything that consumes it from the *surface* side: the `Push` CLI subcommand, the `Sync::{Status,Refresh,Reset}` CLI subcommands (`Sync::Run` survives as the cloud-only error), the `commands::research` module (the surfaceless integration-test-only `research save` flow), the `publish_local_write_best_effort` runtime helper, and four e2e tests whose entire scope was the deleted machinery (`sync_test.rs`, `push_command_test.rs`, `pull_command_test.rs`, `locally_missing_recovery_test.rs`).

**Architecture:** Peel from the leaves inward, the same shape Chunk 4 used. First delete the leaves that have zero downstream impact (the four e2e tests). Then delete the `commands/research.rs` module + its integration test (this orphans `publish_local_write_best_effort`'s only caller). Then delete `publish_local_write_best_effort` itself (this orphans `actions::sync::publish_local_write`, but `sync.rs` is going away anyway). Then delete `commands/push.rs` + its enum variant + dispatch arm. Then gut `commands/sync_cmd.rs` to keep only the cloud-only `run` error (drops the three remaining `actions::sync` consumers in temper-cli). At that point `actions/sync.rs` has zero non-test callers and can die in one commit. Finally sweep pub-orphans surfaced by `cargo make check`, fix two dangling doc-comments, and run the four verification tiers + consolidated review.

**Tech Stack:** Rust 2024 edition, cargo-make, cargo-nextest, sqlx (compile-time queries — unchanged by this chunk).

---

## Plan gate — resolution

The task content lists, as Chunk 5 acceptance criteria, "`Manifest` and `manifest_io` symbols absent from `rg 'Manifest\b|manifest_io' --type rust`" and "`crates/temper-cli/src/manifest_io.rs` is gone" and "`crates/temper-core/src/types/manifest.rs` and `types/sync.rs` are gone." That is **not achievable in Chunk 5 without breaking the build**, because seven deferred consumers in Chunk-6/7 scope still import these symbols:

| Consumer | Scope | Reference |
|---|---|---|
| `crates/temper-cli/src/actions/search.rs` | Chunk 6 | `use temper_core::types::{Manifest, ResourceId};` |
| `crates/temper-cli/src/commands/search_cmd.rs` | Chunk 6 | `crate::manifest_io::load_manifest(&temper_dir, &device_id)?` |
| `crates/temper-cli/src/actions/doctor.rs` | Chunk 7 | `manifest_io::load_manifest`, `manifest_io::save_manifest`, `temper_core::types::Manifest` |
| `crates/temper-cli/src/actions/doctor_fix.rs` | Chunk 7 | `use temper_core::types::manifest::Manifest;` (plus test fixtures) |
| `crates/temper-cli/src/commands/doctor.rs` | Chunk 7 | format string mentions `Manifest` (cosmetic) |
| `crates/temper-cli/src/actions/ingest.rs` | Chunk 7 | `crate::manifest_io::load_manifest`, `crate::manifest_io::save_manifest` |
| `tests/e2e/tests/graph_build_e2e_test.rs` | Chunk 6 | `use temper_core::types::Manifest;` |

**Decision: Option 1 (Defer).** Chunk 5 deletes the *upstream* surfaces and the sync engine itself. `manifest_io.rs`, `temper-core::types::manifest`, and `temper-core::types::sync` stay live until their last consumers go in Chunks 6+7. The "`Manifest` symbol absent" acceptance criterion is moved to end-of-Chunk-7's checklist.

**This is the same pattern Chunk 4 used.** Chunk 4 was supposed to delete `manifest_io` per its spec literal but deferred because 25 files outside `vault_backend/` referenced it; the deferral is now structural across two more chunks. Chunk 5 acknowledges this and proceeds with the upstream cuts.

**False positives confirmed (not real `Manifest`/`manifest_io` consumers):**
- `crates/temper-cli/src/actions/index_build.rs::IndexManifest` — a `pub(crate)` HNSW sidecar struct, unrelated to sync manifest
- `crates/temper-cli/src/actions/graph_index/cluster.rs` — comment-only ref to `IndexManifest`
- `tests/e2e/tests/meta_test.rs:221` — comment-only ref to "Manifest"
- `crates/temper-cli/src/projection.rs:54` — doc-comment analogy to `manifest_io::save_manifest` (will dangle once manifest_io is gone in Chunk 7; trivial update in this chunk's Task 8)

**Surface decision: `commands/research.rs` is deprecated, not migrated** (per user). It has no CLI subcommand registration — `pub fn save` is callable only from the `crates/temper-cli/tests/research_test.rs` integration test. Module + integration test both die together. `ResearchTemplate` itself stays alive because `vault::get_template` (the `--show-template` preview helper) uses it.

---

## Cleanups bundled in this chunk

| Item | Why it travels with Chunk 5 |
|------|----------------------------|
| `commands::push` CLI command + `Commands::Push` enum variant + `main.rs` dispatch arm | Push is the only surface that targeted local-vault → remote-sync; cloud-only writes go through `temper resource create`/`update` directly |
| `Sync::{Status,Refresh,Reset}` CLI subcommands + `SyncAction` enum variants + `main.rs` dispatch arms | Only callable against a local manifest; cloud-only has no local manifest to inspect or rebuild |
| `commands::research` module + `crates/temper-cli/tests/research_test.rs` + stale doc-comment ref in `commands/resource.rs:66` | Surfaceless after the deprecation choice; `--type research` via `temper resource create` remains the supported entry |
| `actions::runtime::publish_local_write_best_effort` (and its 2 inline tests) | After `commands::research` is gone, has zero callers |
| Four doomed e2e tests (`sync_test.rs`, `push_command_test.rs`, `pull_command_test.rs`, `locally_missing_recovery_test.rs`) | Each is structurally dependent on the sync engine, the `Manifest` type, or both |
| Dangling doc-comments at `projection.rs:54`, `commands/resource.rs:66` | References to deleted code; trivial sweep |
| Pub-orphans surfaced by `cargo make check` after the deletions | Same symmetric-removal heuristic Chunk 4 used; verify both producers and consumers |

## Items explicitly NOT in this chunk (deferred)

- `crates/temper-cli/src/manifest_io.rs` and `pub mod manifest_io;` in `lib.rs` — wait for Chunk 7's last consumer
- `crates/temper-core/src/types/manifest.rs`, `types/sync.rs`, and the `pub use` lines in `types/mod.rs` — same
- `actions/search.rs`, `commands/search_cmd.rs`, `actions/index_build.rs`, `actions/graph_index/cluster.rs`, `tests/e2e/tests/graph_build_e2e_test.rs` — Chunk 6
- `actions/doctor.rs`, `actions/doctor_fix.rs`, `commands/doctor.rs`, `actions/ingest.rs` — Chunk 7
- `tests/e2e/tests/meta_test.rs` — surviving comment-only ref to "Manifest" gets cleaned up in Chunk 7's manifest-deletion task (no edit here)

## Branch

`jct/cloud-only-vault-pr-b` — **do not branch**. Chunks 3–8 accumulate on the same branch; the PR opens after Chunk 8.

## Execution discipline (carry forward from Chunks 3 + 4)

- Subagent-driven execution, fresh sonnet implementer per task; opus only for the final consolidated review (per `feedback_subagent_review_cadence`).
- Each task ends with `cargo make check` green and a commit, so the branch stays bisectable.
- Per-task verification is **tightened**: `cargo make check` + targeted `-p` nextest only. Full workspace + e2e tiers run once in the final consolidated task (this saved hours in Chunk 4 — Task 3 ran 57 min when it ran full e2e; subsequent tasks ran 6–13 min).
- **Pub-orphan sweep audit (symmetric removal):** when deleting a reader, audit the writer side; when deleting a writer, audit the consumer side. Chunk 4's `PROFILE_SLUG_CACHE` orphan-writer escape is the cautionary tale.
- **Cargo output redirection:** always `> /tmp/foo.log 2>&1`. Never `2>&1 | tail` (silently produces 0-byte files under the harness, per `feedback_cargo_output_redirection`).
- **Plan committed early** (per Chunk 4 carry-forward lesson #7): Task 0 below commits this plan before Task 1 starts, so each subsequent commit's context references it.

---

## Task 0: Commit this plan

Land the plan file before any code change so subsequent commits reference it. No code edit; one commit.

- [ ] **Step 1: Commit the plan file**

```bash
git add docs/superpowers/plans/2026-05-24-cloud-only-vault-chunk5-delete-sync-and-push.md
git commit -m "cloud-only(ch5): record the chunk 5 implementation plan"
```

---

## Task 1: Test triage

Inventory every test file whose code path is touched by this chunk's deletions. Produce explicit delete/keep/repoint verdicts in an empty commit so the analysis is bisectable.

- [ ] **Step 1: Inventory affected test files via grep**

```bash
# Test files in tests/e2e/ that reference deleting symbols
rg -l 'manifest_io|::sync::|sync_actions|Push(Kind|Action)|publish_local_write_best_effort|commands::research' \
  --type rust tests/e2e/tests/ 2>&1 > /tmp/ch5_e2e_triage.log

# Test files in crates/ (unit + integration) that reference deleting symbols
rg -l 'manifest_io|publish_local_write_best_effort|commands::research|commands::push|sync_cmd::(status|refresh|reset)' \
  --type rust crates/temper-cli/src/ crates/temper-cli/tests/ crates/temper-core/src/ 2>&1 > /tmp/ch5_unit_triage.log

cat /tmp/ch5_e2e_triage.log /tmp/ch5_unit_triage.log
```

- [ ] **Step 2: Produce a verdict table**

For each file the grep surfaces, decide:
- **Delete with parent** — file lives inside a deleted module; dies in its parent's task
- **Delete whole file** — entire test file's scope is the deleted machinery
- **Repoint** — file references a deleting symbol but the underlying test still has value; update to cloud-mode equivalent
- **Keep** — false positive (e.g. comment-only ref) or scope is unaffected
- **Defer to later chunk** — file's test is for sync/push/graph code that survives this chunk (none expected after Task 0; the deferred-chunk consumers don't have temper-cli-side e2e coverage in this chunk's scope)

**Expected verdicts (verify with the grep above; this is the working hypothesis):**

| File | Verdict | Lands in |
|---|---|---|
| `tests/e2e/tests/sync_test.rs` (~2,200 lines) | Delete whole file | Task 5 |
| `tests/e2e/tests/push_command_test.rs` (~512+ lines) | Delete whole file | Task 5 |
| `tests/e2e/tests/pull_command_test.rs` | Delete whole file | Task 5 |
| `tests/e2e/tests/locally_missing_recovery_test.rs` | Delete whole file | Task 5 |
| `tests/e2e/tests/meta_test.rs:221` (comment-only) | Keep (false positive; comment cleaned up in Chunk 7) | — |
| `tests/e2e/tests/graph_build_e2e_test.rs` | Keep (defers with Chunk 6) | — |
| `crates/temper-cli/tests/research_test.rs` (85 lines) | Delete whole file | Task 2 |
| `crates/temper-cli/src/actions/runtime.rs` (inline `tests` + `expiry_warning_tests`) | Repoint — delete only `publish_best_effort_returns_ok_none_when_no_token` test (lines ~258–287); keep the rest | Task 3 |
| `crates/temper-cli/src/commands/research.rs` (inline `inline_research_write_tests`) | Delete with parent | Task 2 |
| `crates/temper-cli/src/commands/sync_cmd.rs` (no inline tests; main.rs dispatch arms) | Delete with parent subcommands | Task 6 |

- [ ] **Step 3: Commit the inventory (empty)**

```bash
git commit --allow-empty -m "$(cat <<'EOF'
cloud-only(ch5): test-triage inventory for chunk 5

E2E whole-file deletions (Task 5):
  - tests/e2e/tests/sync_test.rs
  - tests/e2e/tests/push_command_test.rs
  - tests/e2e/tests/pull_command_test.rs
  - tests/e2e/tests/locally_missing_recovery_test.rs

Integration test deletions:
  - crates/temper-cli/tests/research_test.rs (Task 2)

Inline-test deletions:
  - crates/temper-cli/src/commands/research.rs::inline_research_write_tests
    (dies with parent module in Task 2)
  - crates/temper-cli/src/actions/runtime.rs::publish_best_effort_returns_ok_none_when_no_token
    (dies with publish_local_write_best_effort in Task 3); other tests in
    the same module survive (test_require_device_id_returns_error_when_not_logged_in,
    with_client_errors_when_temper_token_set_but_invalid, expiry_warning_tests)

Keep / false positives:
  - tests/e2e/tests/meta_test.rs:221 — comment-only ref to "Manifest"
  - tests/e2e/tests/graph_build_e2e_test.rs — defers with Chunk 6
EOF
)"
```

---

## Task 2: Delete `commands/research.rs` + integration test

`commands::research` has no CLI subcommand wiring — `pub fn save` is reachable only from `crates/temper-cli/tests/research_test.rs`. Per user decision, the surfaceless `research save` flow is deprecated; users invoke `temper resource create --type research --title ...` instead. Delete module + test together; this removes the only caller of `publish_local_write_best_effort` (which Task 3 then deletes).

**Files:**
- Delete: `crates/temper-cli/src/commands/research.rs` (266 lines including `inline_research_write_tests`)
- Delete: `crates/temper-cli/tests/research_test.rs` (85 lines)
- Modify: `crates/temper-cli/src/commands/mod.rs` (remove `pub mod research;` at line 14)
- Modify: `crates/temper-cli/src/commands/resource.rs` (update the stale doc-comment at line ~66 that says `Source: commands::research::save`)

- [ ] **Step 1: Verify no surface registration exists**

```bash
rg 'commands::research|Research \{|Research,|ResearchAction' crates/temper-cli/src/main.rs crates/temper-cli/src/cli.rs 2>&1
```

Expected: zero hits. If anything surfaces, STOP and report — the surface assumption is wrong.

- [ ] **Step 2: Verify the only callers of `commands::research`**

```bash
rg -n 'commands::research|research::save' --type rust 2>&1
```

Expected hits:
- `crates/temper-cli/src/commands/resource.rs:66` (doc-comment only — update in Step 5)
- `crates/temper-cli/tests/research_test.rs` (integration test — deleted in Step 4)

If any other file surfaces, STOP and report.

- [ ] **Step 3: Delete the module file**

```bash
git rm crates/temper-cli/src/commands/research.rs
```

- [ ] **Step 4: Delete the integration test**

```bash
git rm crates/temper-cli/tests/research_test.rs
```

- [ ] **Step 5: Remove the module declaration + update stale doc-comment**

Edit `crates/temper-cli/src/commands/mod.rs`: delete the line `pub mod research;` (line 14 today).

Edit `crates/temper-cli/src/commands/resource.rs:~66`: read the surrounding 10 lines first to understand context. The comment is part of a larger docstring describing JSON-output shape. Rewrite it so it no longer names `commands::research::save` — keep the JSON-shape description; just drop or generalize the source attribution (e.g. change `Source: commands::research::save, serde_json::json!() at line 84.` to `Source: research-doctype create path, serialized via serde_json::json!().` or simply omit the line).

- [ ] **Step 6: Run `cargo make check`**

```bash
cargo make check > /tmp/ch5_task2_check.log 2>&1; tail -50 /tmp/ch5_task2_check.log
```

Expected: 0 errors. **There may now be a dead-code warning for `publish_local_write_best_effort`** — that's expected and gets fixed in Task 3.

- [ ] **Step 7: Run targeted nextest**

```bash
cargo nextest run -p temper-cli > /tmp/ch5_task2_nextest.log 2>&1; tail -30 /tmp/ch5_task2_nextest.log
```

Expected: all pass. No `commands::research` or `research_test` test names should appear in the executed list.

- [ ] **Step 8: Commit**

```bash
git add -A
git commit -m "cloud-only(ch5): delete commands/research module and integration test"
```

---

## Task 3: Delete `actions::runtime::publish_local_write_best_effort`

After Task 2, this helper has zero callers. Delete the function and its `publish_best_effort_returns_ok_none_when_no_token` unit test. Keep the rest of `runtime.rs::tests` and `runtime.rs::expiry_warning_tests` modules intact — they cover other surviving runtime helpers.

**Files:**
- Modify: `crates/temper-cli/src/actions/runtime.rs` (delete `publish_local_write_best_effort` at lines ~180–229; delete the `publish_best_effort_returns_ok_none_when_no_token` test at lines ~258–287)

- [ ] **Step 1: Confirm zero callers**

```bash
rg -n 'publish_local_write_best_effort' --type rust 2>&1
```

Expected: only the definition + its own test. If any other file surfaces, STOP — Task 2 missed a caller.

- [ ] **Step 2: Delete the function and its docstring**

In `crates/temper-cli/src/actions/runtime.rs`, locate `pub fn publish_local_write_best_effort` (around line 197). Delete:
- The docstring block above the function (lines ~180–196, starting with `/// This is the single source of truth for the publish-tail policy...`)
- The function body (lines ~197–229)

Verify no other references in the file via:
```bash
rg 'publish_local_write_best_effort' crates/temper-cli/src/actions/runtime.rs
```
Should print nothing after the edit.

- [ ] **Step 3: Delete the orphaned test**

In the `#[cfg(test)] mod tests` block, locate `fn publish_best_effort_returns_ok_none_when_no_token` (around lines ~258–287). Delete that single test function. Keep the surrounding `test_require_device_id_returns_error_when_not_logged_in` and `with_client_errors_when_temper_token_set_but_invalid` tests.

- [ ] **Step 4: Run `cargo make check`**

```bash
cargo make check > /tmp/ch5_task3_check.log 2>&1; tail -50 /tmp/ch5_task3_check.log
```

Expected: 0 errors. **`actions::sync::publish_local_write` may now show a dead-code warning** — fine, the entire `actions::sync` module dies in Task 7.

- [ ] **Step 5: Run targeted nextest**

```bash
cargo nextest run -p temper-cli runtime > /tmp/ch5_task3_nextest.log 2>&1; tail -30 /tmp/ch5_task3_nextest.log
```

Expected: the two surviving tests run and pass; `publish_best_effort_returns_ok_none_when_no_token` is absent from the list.

- [ ] **Step 6: Commit**

```bash
git add crates/temper-cli/src/actions/runtime.rs
git commit -m "cloud-only(ch5): drop publish_local_write_best_effort (no callers post-Task 2)"
```

---

## Task 4: Delete `commands/push.rs` + CLI surface

Remove the entire `Push` command surface — the module, the `Commands::Push` enum variant, and the dispatch arm in `main.rs`. There is no separate `actions/push.rs` — `commands/push.rs` inlines its own logic that calls into `actions::sync::push_single` (which dies with `sync.rs` in Task 7) and `manifest_io`.

**Files:**
- Delete: `crates/temper-cli/src/commands/push.rs` (81 lines)
- Modify: `crates/temper-cli/src/commands/mod.rs` (remove `pub mod push;` at line 13)
- Modify: `crates/temper-cli/src/cli.rs` (remove the `Push { target }` enum variant at lines ~126–131, including its docstring)
- Modify: `crates/temper-cli/src/main.rs` (remove the `Commands::Push { target } => commands::push::run(&target),` dispatch arm at line 355)

- [ ] **Step 1: Verify the surface footprint**

```bash
rg -n 'commands::push|Commands::Push|Push \{|push::run' --type rust 2>&1
```

Expected hits should be exactly the four files above. If any other file constructs `Commands::Push` or calls `commands::push::run`, STOP and report.

- [ ] **Step 2: Delete the module**

```bash
git rm crates/temper-cli/src/commands/push.rs
```

- [ ] **Step 3: Remove the module declaration**

In `crates/temper-cli/src/commands/mod.rs`, delete the line `pub mod push;` (line 13 today).

- [ ] **Step 4: Remove the CLI enum variant**

In `crates/temper-cli/src/cli.rs`, delete the `Push { target: String }` variant block (around lines 126–131) and its `/// Push a single resource to the cloud...` docstring. Leave the variants above (`Pull { context: String }`) and below (`Sync { action: SyncAction }`) intact.

- [ ] **Step 5: Remove the dispatch arm**

In `crates/temper-cli/src/main.rs`, delete the single line:
```rust
Commands::Push { target } => commands::push::run(&target),
```
at line 355.

- [ ] **Step 6: Run `cargo make check`**

```bash
cargo make check > /tmp/ch5_task4_check.log 2>&1; tail -50 /tmp/ch5_task4_check.log
```

Expected: 0 errors. If a "no variant `Push`" or "missing match arm" error appears, the enum/dispatch removals are out of sync — re-verify both.

- [ ] **Step 7: Run targeted nextest**

```bash
cargo nextest run -p temper-cli > /tmp/ch5_task4_nextest.log 2>&1; tail -30 /tmp/ch5_task4_nextest.log
```

Expected: all pass.

- [ ] **Step 8: Commit**

```bash
git add -A
git commit -m "cloud-only(ch5): delete commands/push.rs and Push CLI surface"
```

---

## Task 5: Delete four doomed e2e test files

Whole-file deletions for tests whose entire scope is the sync engine + push command + the `Manifest` type's `Clean/LocallyMissing` lifecycle. These don't impact temper-cli's build; they're isolated to the e2e test crate.

**Files:**
- Delete: `tests/e2e/tests/sync_test.rs`
- Delete: `tests/e2e/tests/push_command_test.rs`
- Delete: `tests/e2e/tests/pull_command_test.rs`
- Delete: `tests/e2e/tests/locally_missing_recovery_test.rs`

- [ ] **Step 1: Confirm no other e2e tests import shared helpers from these files**

```bash
rg -n 'mod sync_test|mod push_command_test|mod pull_command_test|mod locally_missing_recovery_test|sync_test::|push_command_test::|pull_command_test::|locally_missing_recovery_test::' tests/e2e/ 2>&1
```

Expected: zero hits (these test files are independent siblings, not module roots). If anything surfaces, STOP and report.

- [ ] **Step 2: Delete the files**

```bash
git rm tests/e2e/tests/sync_test.rs \
       tests/e2e/tests/push_command_test.rs \
       tests/e2e/tests/pull_command_test.rs \
       tests/e2e/tests/locally_missing_recovery_test.rs
```

- [ ] **Step 3: Run `cargo make check`**

```bash
cargo make check > /tmp/ch5_task5_check.log 2>&1; tail -30 /tmp/ch5_task5_check.log
```

Expected: 0 errors.

- [ ] **Step 4: Run targeted e2e nextest with `test-db`**

```bash
cargo nextest run --manifest-path tests/e2e/Cargo.toml --features test-db > /tmp/ch5_task5_nextest.log 2>&1; tail -40 /tmp/ch5_task5_nextest.log
```

Expected: all surviving e2e tests pass. None of the deleted test names should appear. (Skipping the `test-embed` tier in per-task verification per the tightened-verification discipline.)

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "cloud-only(ch5): delete sync/push/pull/locally-missing e2e tests"
```

---

## Task 6: Gut `commands/sync_cmd.rs` — keep only the cloud-only `run` error

After Task 5, `Sync::{Run,Status,Refresh,Reset}` are still wired up. `Run` is the cloud-only error message that survives; `Status`/`Refresh`/`Reset` go away. This task drops the three subcommand handlers, the `SyncAction::{Status,Refresh,Reset}` enum variants, the `main.rs` dispatch arms, and the `warn_blocked_paths` helper (used only by the deleted subcommands).

**Files:**
- Modify: `crates/temper-cli/src/commands/sync_cmd.rs` (gut from 268 lines → ~25 lines: keep `run` + minimal imports)
- Modify: `crates/temper-cli/src/cli.rs` (drop the `SyncAction::{Status, Refresh, Reset}` variants — locate the `pub enum SyncAction` definition; keep `Run`)
- Modify: `crates/temper-cli/src/main.rs` (drop the three dispatch arms at lines 361–372; keep the `SyncAction::Run { context, format }` arm)

- [ ] **Step 1: Read the current `SyncAction` enum**

```bash
rg -n -A 5 'pub enum SyncAction' crates/temper-cli/src/cli.rs
```

Note the exact field shape for each variant (`Run { context: String, format: Option<String> }`, etc.) so the surviving `Run` variant + its dispatch arm stay intact.

- [ ] **Step 2: Rewrite `sync_cmd.rs` to keep only `run`**

Read `crates/temper-cli/src/commands/sync_cmd.rs` end-to-end first. The surviving content is the `pub fn run(_contexts: &[String], _format: &str) -> Result<()>` function (currently lines 41–50) — the cloud-only error stub added in Chunk 3.

Replace the entire file contents with:

```rust
//! `temper sync` — cloud-only mode keeps only the `run` subcommand as an
//! explanatory error. Use `temper resource create` / `temper resource update`
//! to write, and `temper pull <context>` to refresh the local projection.

use crate::error::Result;

/// `temper sync run` — removed. temper is cloud-only: there is no local
/// vault to reconcile.
pub fn run(_contexts: &[String], _format: &str) -> Result<()> {
    Err(crate::error::TemperError::Project(
        "temper is cloud-only — there is no local vault to sync. Use \
         `temper resource create` / `temper resource update` to write, \
         and `temper pull <context>` to refresh the local projection."
            .to_string(),
    ))
}
```

Match the existing `run` body's error type and exact wording (verify by reading the current `run` at lines 41–50 before overwriting).

This rewrite drops:
- `use crate::actions::progress::TerminalProgress;` and `use crate::actions::{runtime, sync as sync_actions};` imports (no longer needed)
- `use crate::format::OutputFormat;` and `use crate::output;` (only the deleted subcommands used them)
- The `warn_blocked_paths` helper (only the deleted subcommands called it)
- `pub fn status`, `pub fn refresh`, `pub fn reset` (deleted entirely)

- [ ] **Step 3: Drop `SyncAction::{Status,Refresh,Reset}` variants**

In `crates/temper-cli/src/cli.rs`, find the `pub enum SyncAction` definition. Delete the `Status`, `Refresh`, and `Reset` variants and their docstrings. Keep `Run { context: String, format: Option<String> }` (or whatever its exact fields are).

- [ ] **Step 4: Drop the three dispatch arms in `main.rs`**

In `crates/temper-cli/src/main.rs`, the current dispatch (lines 356–373):

```rust
Commands::Sync { action } => match action {
    SyncAction::Run { context, format } => {
        let format = temper_cli::format::resolve_format_str(format.as_deref());
        commands::sync_cmd::run(&context, format)
    }
    SyncAction::Status { context, format } => { ... }
    SyncAction::Refresh { format } => { ... }
    SyncAction::Reset { format } => { ... }
},
```

Reduce to:

```rust
Commands::Sync { action } => match action {
    SyncAction::Run { context, format } => {
        let format = temper_cli::format::resolve_format_str(format.as_deref());
        commands::sync_cmd::run(&context, format)
    }
},
```

If clippy complains about a single-variant `match` (it sometimes prefers `let SyncAction::Run { context, format } = action;`), accept whichever shape clippy prefers. The shape isn't load-bearing — the goal is the deletion.

- [ ] **Step 5: Run `cargo make check`**

```bash
cargo make check > /tmp/ch5_task6_check.log 2>&1; tail -60 /tmp/ch5_task6_check.log
```

Expected: 0 errors. **Dead-code warnings for `actions::sync` internals (huge wall of them) are expected and fine — `sync.rs` dies in Task 7.**

If real errors appear (not warnings), STOP and report. Likely cause: a stale `use` import or a missed `main.rs` arm.

- [ ] **Step 6: Run targeted nextest**

```bash
cargo nextest run -p temper-cli > /tmp/ch5_task6_nextest.log 2>&1; tail -30 /tmp/ch5_task6_nextest.log
```

Expected: all pass.

- [ ] **Step 7: Commit**

```bash
git add -A
git commit -m "cloud-only(ch5): gut commands/sync_cmd to keep only the cloud-only run error"
```

---

## Task 7: Delete `actions/sync.rs`

After Tasks 2, 3, 4, and 6, `actions::sync` has zero non-internal callers. Delete the file (4,369 lines — the bulk of the chunk) and the module declaration in `actions/mod.rs`.

**Files:**
- Delete: `crates/temper-cli/src/actions/sync.rs`
- Modify: `crates/temper-cli/src/actions/mod.rs` (remove `pub mod sync;` or `mod sync;` — verify current visibility)

- [ ] **Step 1: Confirm zero non-internal callers**

```bash
rg -n 'actions::sync|crate::actions::sync|sync_actions' --type rust 2>&1
```

Expected: only references inside `actions/sync.rs` itself. If any external file (in `crates/temper-cli/src/{commands,actions}/*.rs` other than `sync.rs`) imports from `actions::sync`, STOP — earlier tasks missed something.

Note: `actions/mod.rs` will reference `sync` via the `pub mod sync;` declaration; that's the only legitimate external ref pre-deletion.

- [ ] **Step 2: Delete the file**

```bash
git rm crates/temper-cli/src/actions/sync.rs
```

- [ ] **Step 3: Remove the module declaration**

In `crates/temper-cli/src/actions/mod.rs`, delete the line that declares the sync module:

```bash
rg -n 'sync' crates/temper-cli/src/actions/mod.rs
```

Delete every line the grep surfaces that's a `mod sync;` / `pub mod sync;` declaration or a `pub use sync::*;` re-export. Comment-only refs can also be cleaned up if present (use judgement).

- [ ] **Step 4: Run `cargo make check`**

```bash
cargo make check > /tmp/ch5_task7_check.log 2>&1; tail -80 /tmp/ch5_task7_check.log
```

Expected: 0 errors. **A wave of new dead-code warnings is likely** — items in `temper-cli` that were only used by `actions::sync` are now orphaned. Task 9 sweeps them.

If real errors appear (especially `unresolved import` from a `commands/*` or `actions/*` file other than `sync.rs`), STOP — Tasks 2/4/6 missed a consumer. Grep:
```bash
rg 'use crate::actions::sync' --type rust
```
Should print nothing.

- [ ] **Step 5: Run targeted nextest**

```bash
cargo nextest run -p temper-cli > /tmp/ch5_task7_nextest.log 2>&1; tail -30 /tmp/ch5_task7_nextest.log
```

Expected: all pass. The huge `sync_test` suite is already deleted (Task 5), so unit-test count drops noticeably — that's fine.

- [ ] **Step 6: Commit**

```bash
git add -A
git commit -m "cloud-only(ch5): delete actions/sync.rs (4.4k LOC sync engine)"
```

---

## Task 8: Trivial doc-comment sweeps

Two dangling doc-comments reference now-deleted code. Update them so nothing dangles.

**Files:**
- Modify: `crates/temper-cli/src/projection.rs:54` — the comment `// pattern used by `manifest_io::save_manifest`).` references a module still alive but inside Task 9's sweep target list; update to describe the pattern without naming an unrelated module
- Verify: `crates/temper-cli/src/commands/resource.rs:~66` — already swept in Task 2's Step 5; nothing more to do unless the grep surfaces a residual

- [ ] **Step 1: Update `projection.rs:54` doc-comment**

Read `crates/temper-cli/src/projection.rs:50–66` (the `write_cursor` function's docstring + body). Today it says:

```
/// Atomically write a context's cursor sidecar (temp file + rename, the
/// pattern used by `manifest_io::save_manifest`).
```

Rewrite the parenthetical so it stands on its own:

```
/// Atomically write a context's cursor sidecar using the standard
/// temp-file-plus-rename pattern.
```

Verify the rest of `projection.rs` has no remaining external dependency on `manifest_io` (should still be zero per Chunk 5's plan-gate audit):

```bash
rg -n 'manifest_io|Manifest' crates/temper-cli/src/projection.rs
```

Expected: zero hits after this edit.

- [ ] **Step 2: Spot-check `commands/resource.rs:66`**

```bash
rg -n 'commands::research|research::save' crates/temper-cli/src/commands/resource.rs
```

Expected: zero hits (Task 2 swept this). If anything surfaces, repeat Task 2's Step 5 for it.

- [ ] **Step 3: Run `cargo make check`**

```bash
cargo make check > /tmp/ch5_task8_check.log 2>&1; tail -30 /tmp/ch5_task8_check.log
```

Expected: 0 errors. (Dead-code warnings from Task 7 are still present; Task 9 sweeps them.)

- [ ] **Step 4: Commit**

```bash
git add crates/temper-cli/src/projection.rs
git commit -m "cloud-only(ch5): unhook projection.rs doc-comment from manifest_io"
```

---

## Task 9: Sweep pub-orphans surfaced by `cargo make check`

Tasks 2, 3, 4, 6, and 7 deleted a lot of code. Clippy under `-D warnings` will surface helpers and types that were only used by deleted callers. Walk the warnings, verify each candidate is truly unused (symmetric-removal: check producers AND consumers), delete, recompile.

**Likely sweep targets (verify each — these are starting points, not a fixed list):**
- `crates/temper-cli/src/actions/runtime.rs` — helpers that only `publish_local_write_best_effort` or `actions::sync::publish_local_write` consumed (e.g. `with_client` variants, token-store helpers, `expiry_warning_tests` fixtures if their only assertions were against deleted helpers)
- `crates/temper-cli/src/commands/sync_cmd.rs` — `warn_blocked_paths` was already deleted with the file rewrite in Task 6, but verify
- `crates/temper-cli/src/manifest_io.rs` — has fewer callers now; check for fully orphaned functions even though the module itself stays. Apply symmetric-removal: a `pub fn` whose only callers were in `actions/sync.rs` or `commands/sync_cmd.rs::{status,refresh,reset}` is now dead. Don't delete the module — only the individual orphaned functions.
- `crates/temper-core/src/types/manifest.rs`, `types/sync.rs` — same: types stay (deferred to Chunk 7) but individual fields/methods whose only callers were in the deleted scope are dead. **Be conservative here** — over-deletion in temper-core hurts the deferred consumers in Chunks 6/7.

**Files (candidates — verify each is unused before deleting):**
- Modify: `crates/temper-cli/src/actions/runtime.rs` (verify which helpers became dead)
- Modify: `crates/temper-cli/src/manifest_io.rs` (verify which `pub fn`s lost their last caller)
- Modify: `crates/temper-core/src/types/manifest.rs` and `types/sync.rs` (verify which `pub` items lost their last caller — be conservative)

- [ ] **Step 1: Run `cargo make check` and collect dead-code warnings**

```bash
cargo make check > /tmp/ch5_task9_check_raw.log 2>&1
rg 'dead.code|never.used|is never read|never.constructed' /tmp/ch5_task9_check_raw.log | sort -u > /tmp/ch5_task9_warnings.log
wc -l /tmp/ch5_task9_warnings.log
cat /tmp/ch5_task9_warnings.log
```

This produces the authoritative list. Use it as the deletion target.

- [ ] **Step 2: Symmetric-removal verification**

For each warning-surfaced item, run a final grep to confirm no surviving caller exists:

```bash
rg '\b<ITEM_NAME>\b' --type rust
```

For each, the only hits should be the definition itself (or peer items in the same dead chain). If a real caller exists (in `actions/{search,doctor,doctor_fix,ingest,index_build}`, `commands/{search_cmd,doctor}`, or `tests/e2e/tests/graph_build_e2e_test.rs`), **do not delete** — that means the item survives for the deferred chunks.

**Critical: symmetric audit.** For each `pub fn X` flagged dead:
- Check if `X` is a *reader* — what was the *writer* / producer that fed it? Is that now dead too?
- Check if `X` is a *writer* — what was the *consumer* that read its output? Is that now dead too?

Chunk 4's `lookup::find_resource` (reader) deletion left `set_cached_profile_slug` (writer) orphaned. Don't make the same miss.

- [ ] **Step 3: Delete the verified-dead items**

Walk each warning-surfaced item top-down through the candidate files. Delete the item plus any `use` lines that become unused.

**For `manifest_io.rs` specifically:** delete only individual `pub fn`s that lost their last caller; do NOT delete the module file itself (deferred to Chunk 7).

**For `temper-core/src/types/{manifest,sync}.rs` specifically:** be **conservative**. Only delete fields/methods that grep proves have zero remaining callers across the workspace (including the deferred Chunk-6/7 consumers). When in doubt, leave it — Chunk 7's deletion of the whole types module will scoop everything up.

- [ ] **Step 4: Re-run `cargo make check`**

```bash
cargo make check > /tmp/ch5_task9_check_final.log 2>&1; tail -40 /tmp/ch5_task9_check_final.log
```

Expected: 0 errors, 0 dead-code warnings. (Some pub-at-lib items may stay silent — that's the same trap Chunk 4 noted. Sample-check 2–3 items from the previous warning list and confirm they're either deleted or genuinely re-consumed.)

- [ ] **Step 5: Run targeted nextest for affected crates**

```bash
cargo nextest run -p temper-cli > /tmp/ch5_task9_temper_cli.log 2>&1; tail -20 /tmp/ch5_task9_temper_cli.log
cargo nextest run -p temper-core > /tmp/ch5_task9_temper_core.log 2>&1; tail -20 /tmp/ch5_task9_temper_core.log
```

Expected: all pass.

- [ ] **Step 6: Commit**

```bash
git add -A
git commit -m "cloud-only(ch5): sweep pub-orphans after sync/push deletion"
```

---

## Task 10: Full verification + consolidated review

Run all four verification tiers locally, then dispatch a fresh opus reviewer for consolidated review. Address findings inline; PR stays unopened (PR B accumulates Chunks 3–8).

**Files:** none modified except possible review-followup fixes.

- [ ] **Step 1: Tier 1 — `cargo make check`**

```bash
cargo make check > /tmp/ch5_task10_tier1.log 2>&1; tail -30 /tmp/ch5_task10_tier1.log
```

Expected: 0 errors, 0 warnings.

- [ ] **Step 2: Tier 2 — workspace unit + integration tests**

```bash
cargo nextest run --workspace > /tmp/ch5_task10_tier2.log 2>&1; tail -40 /tmp/ch5_task10_tier2.log
```

Expected: 100% pass.

- [ ] **Step 3: Tier 3 — e2e with `test-db`**

```bash
cargo make test-e2e > /tmp/ch5_task10_tier3.log 2>&1; tail -40 /tmp/ch5_task10_tier3.log
```

Expected: 100% pass. Per Chunks 3 + 4 session notes, the `access_gate_test` parallel e2e flake is environmental — if it fails, re-run serially:

```bash
cargo nextest run --manifest-path tests/e2e/Cargo.toml --features test-db -j 1 access_gate > /tmp/ch5_task10_tier3_serial.log 2>&1
```

- [ ] **Step 4: Tier 4 — e2e with `test-db,test-embed`**

```bash
cargo nextest run --manifest-path tests/e2e/Cargo.toml --features test-db,test-embed > /tmp/ch5_task10_tier4.log 2>&1; tail -40 /tmp/ch5_task10_tier4.log
```

Expected: 100% pass (modulo the known env flake).

- [ ] **Step 5: Verify the (adjusted) acceptance criteria**

```bash
# (1) actions/sync.rs is gone
test ! -f crates/temper-cli/src/actions/sync.rs && echo "OK: sync.rs gone" || echo "FAIL"

# (2) commands/push.rs is gone
test ! -f crates/temper-cli/src/commands/push.rs && echo "OK: push.rs gone" || echo "FAIL"

# (3) commands/research.rs is gone
test ! -f crates/temper-cli/src/commands/research.rs && echo "OK: research.rs gone" || echo "FAIL"

# (4) crates/temper-cli/tests/research_test.rs is gone
test ! -f crates/temper-cli/tests/research_test.rs && echo "OK: research_test gone" || echo "FAIL"

# (5) sync_cmd surfaces only `run` (the cloud-only error)
rg -n 'pub fn (status|refresh|reset)' crates/temper-cli/src/commands/sync_cmd.rs && echo "FAIL: status/refresh/reset still present" || echo "OK: sync_cmd gutted"

# (6) Push and SyncAction::{Status,Refresh,Reset} variants are gone
rg -n 'Commands::Push|SyncAction::(Status|Refresh|Reset)' crates/temper-cli/src/ && echo "FAIL" || echo "OK: surfaces removed"

# (7) publish_local_write_best_effort is gone
rg -n 'publish_local_write_best_effort' --type rust && echo "FAIL: still present" || echo "OK: helper gone"

# (8) The 4 doomed e2e tests are gone
for f in sync_test push_command_test pull_command_test locally_missing_recovery_test; do
  test ! -f "tests/e2e/tests/$f.rs" && echo "OK: $f.rs gone" || echo "FAIL: $f.rs still present"
done

# (9) projection.rs no longer mentions manifest_io anywhere
rg -n 'manifest_io' crates/temper-cli/src/projection.rs && echo "FAIL" || echo "OK: projection.rs clean"

# (10) Plan-gate question documented in this plan's preamble (manual: this file's "Plan gate — resolution" section)
echo "OK: plan-gate documented (see file's preamble)"
```

All ten should print OK.

**Note:** The original task content lists "`Manifest` and `manifest_io` symbols absent" as acceptance criteria — **those are explicitly deferred to Chunk 7** per the plan-gate resolution above and should NOT be checked here.

- [ ] **Step 6: Dispatch the consolidated opus review**

Dispatch a fresh opus subagent (general-purpose) with this prompt (substitute the actual predecessor-commit SHA — currently `35399b5` "cloud-only(ch4): record the chunk 4 implementation plan"):

```
You are reviewing the implementation of Chunk 5 of the cloud-only-vault
deprecation on the branch `jct/cloud-only-vault-pr-b`. Inspect the
commits added in this chunk (since the predecessor commit 35399b5
"cloud-only(ch4): record the chunk 4 implementation plan").

The plan is at:
  docs/superpowers/plans/2026-05-24-cloud-only-vault-chunk5-delete-sync-and-push.md

The plan resolves an explicit plan-gate question by deferring
`manifest_io.rs` and `temper-core::types::{manifest,sync}` deletion to
Chunk 7 — this is INTENDED. The seven Chunk-6/7 consumers
(`actions/search`, `commands/search_cmd`, `actions/doctor`,
`actions/doctor_fix`, `commands/doctor`, `actions/ingest`,
`tests/e2e/tests/graph_build_e2e_test`) keep importing those symbols
and must continue to compile. Do NOT flag the surviving
`manifest_io` / `Manifest` / `temper-core::types::{manifest,sync}`
symbols as misses.

Review for:
1. Correctness — was each task implemented as specified? Did the test
   triage's verdict table get honored (no false-positive deletions)?
2. Code quality — match against the project's CLAUDE.md rules: typed
   structs over inline JSON, service layer owns SQL (N/A here),
   params structs over too-many-args, no premature backward-compat
   shims, patterns match siblings.
3. Bisectability — did intermediate commits leave the build broken
   between tasks? (e.g. did Task 6's `sync_cmd` rewrite reference
   `sync::publish_local_write` after Task 3 deleted its only caller
   but before Task 7 deleted sync.rs?)
4. Pub-orphan sweep completeness (symmetric removal) — are there still
   dead pub items where one side (reader OR writer) got deleted but
   the other survived? Specifically check:
   - `actions/runtime.rs` after `publish_local_write_best_effort` died
   - `manifest_io.rs` after the sync engine + push + sync_cmd
     subcommands died
   - `temper-core/src/types/{manifest,sync}.rs` — should still have
     items used by deferred consumers; flag only over-deletions, not
     under-deletions (under-deletion is intentional per plan-gate).
5. Surface consistency — `temper push` and `temper sync status/refresh/reset`
   should produce helpful errors (or clap's "unknown subcommand"), not
   panics. `temper sync run` should still print the cloud-only message.
6. Did the doc-comment sweep (Task 8) leave any dangling references?
   Quick grep: `rg 'manifest_io|publish_local_write_best_effort|commands::research'`
   should only surface deferred-chunk consumers + this plan file itself.

Return READY / READY_WITH_FOLLOWUPS / NEEDS_CHANGES. List findings
by severity (critical / important / minor) with file:line refs.
```

- [ ] **Step 7: Address findings inline**

If the review returns READY_WITH_FOLLOWUPS or NEEDS_CHANGES, address critical/important findings in a single review-followup commit. Minor findings (docstring nits, naming) fold into the same commit. Per Chunks 3 + 4 precedent, this lands as one commit, not per-finding.

```bash
git add -A
git commit -m "cloud-only(ch5): review followups"
```

- [ ] **Step 8: Save session note**

Pipe the session summary via stdin:

```bash
cat <<'EOF' | temper resource create --type session --title "Cloud-only vault Chunk 5 landed (sync engine + push + research + 4 e2e tests deleted)" --context temper
## Goal
(describe goal here — Chunk 5 of cloud-only-vault deprecation: delete sync
engine, push command, sync_cmd subcommands, research module, publish helper,
and the four doomed e2e tests)

## What Happened
(describe execution and surprises — especially pub-orphan sweep findings
and any deferred-consumer compile issues that surfaced)

## Decisions
(describe key decisions — plan-gate Option 1 deferral of manifest_io to
Chunk 7; user-chosen Option C for research.rs deprecation; user-chosen
delete-not-repoint for pull_command_test)

## Connections
- Branch (no PR yet): jct/cloud-only-vault-pr-b
- Plan: docs/superpowers/plans/2026-05-24-cloud-only-vault-chunk5-delete-sync-and-push.md
- Predecessor session: 2026-05-23-cloud-only-vault-chunk-4-landed-vault-backend-deleted-variants-removed-6500-loc-net
- Spec: docs/superpowers/specs/2026-05-21-cloud-only-vault-deprecation-design.md

## Next Steps
- Chunk 6: HNSW + graph + search rework (drops actions::search, commands::search_cmd, actions::index_build, actions::graph_index/cluster, tests/e2e/tests/graph_build_e2e_test)
- Chunk 7: doctor + ingest rework (drops actions::doctor, actions::doctor_fix, commands::doctor, actions::ingest) — also FINALLY deletes manifest_io.rs and temper-core::types::{manifest,sync}
- Project memory: project_cloud_only_vault_direction (update with Chunk 5 done)
EOF
```

- [ ] **Step 9: Mark the task done**

```bash
temper resource update 2026-05-24-cloud-only-vault-chunk-5-delete-sync-engine-push-manifest-io-and-temper-core-manifest-sync-types --type task --context temper --stage done
```

---

## Self-Review

**Adjusted spec coverage (acceptance criteria the plan actually achieves):**

| Criterion | Plan coverage |
|---|---|
| `crates/temper-cli/src/actions/sync.rs` is gone | Task 7 |
| `crates/temper-cli/src/commands/sync_cmd.rs::{status,refresh,reset}` are gone; `run` survives or is inlined cleanly | Task 6 |
| `crates/temper-cli/src/commands/push.rs` is gone (and its `commands::push` registration in `lib.rs`/`main.rs`) | Task 4 |
| `actions::runtime::publish_local_write_best_effort` gone | Task 3 |
| `projection.rs` no longer depends on `manifest_io` | Task 8 (doc-comment update; the actual `manifest_io` module continues to exist for deferred consumers) |
| Plan-gate question (which `manifest_io` consumers move now vs Chunks 6+7) explicitly resolved | Preamble |
| All four verification tiers green | Task 10 Steps 1–4 |
| E2E tests `sync_test/push_command_test/pull_command_test/locally_missing_recovery_test` have explicit delete-or-repoint verdicts applied | Task 5 (all DELETE) |
| No PR opened | Implicit; PR B accumulates Chunks 3–8 |

**Acceptance criteria DEFERRED (and why):**

| Criterion | Where it lands instead | Reason |
|---|---|---|
| `crates/temper-cli/src/manifest_io.rs` is gone | Chunk 7 (after `actions/ingest`, `actions/doctor*`, `commands/search_cmd` rework) | Seven deferred consumers still import |
| `crates/temper-core/src/types/manifest.rs` and `types/sync.rs` are gone | Chunk 7 | Same |
| `Manifest` and `manifest_io` symbols absent from `rg 'Manifest\b|manifest_io' --type rust` | Chunk 7 | Same |

This deferral mirrors Chunk 4's `manifest_io` deferral and is documented in the plan-gate resolution above.

**Plan-gate consistency check:** Tasks 2, 3, 4, and 6 progressively orphan `actions::sync` from the outside in (research.rs → publish helper → push surface → sync_cmd subcommands). Task 7 deletes `sync.rs` only after every external caller is gone. Each task ends with `cargo make check` green (modulo dead-code warnings, which Task 9 sweeps). The branch stays bisectable.

**Type-consistency check:**
- `sync_cmd::run`'s signature `(_contexts: &[String], _format: &str) -> Result<()>` is preserved exactly (Task 6) so `main.rs`'s `SyncAction::Run` dispatch arm continues to compile.
- `actions::runtime`'s other public helpers (`require_device_id`, `with_client`, `load_cloud_config`, `resolve_token_store`) survive Task 3 — they're called from cloud-mode read paths and Chunks 6/7 consumers.
- Pub-orphan sweep (Task 9) is conservative for `temper-core` types: under-deletion is intentional; over-deletion would break deferred consumers.

**Placeholder scan:** No `TBD`, no "TODO", no "implement similar to Task N". Code blocks present for every behavior-changing step. Tasks 9 and 10 describe sweeps/verifications whose source-of-truth is `cargo make check` output and grep, not a fixed list.

**Notes for the implementer-subagent dispatch (sonnet recommended for Tasks 0–9; opus only for Task 10 Step 6):**

- Include `SG-1`, `SG-2`, `SG-5`, `SG-6`, `SG-10` from `subagent-guidance.md` verbatim in every dispatch prompt.
- Include the project fundamentals references on typed structs, service-layer SQL ownership, params structs, and "no premature backward-compat" (relevant because this chunk is pure deletion — no legacy shims).
- For Tasks 2–8 (deletion): emphasize "verify before deleting" — every removed item must be confirmed unused by grep first. Symmetric-removal audit on every `pub` item touched.
- Cargo output redirection discipline: `> /tmp/foo.log 2>&1`, never `2>&1 | tail`.
- Task 9's sweep instruction has a "be conservative" clause for `temper-core/src/types/{manifest,sync}.rs` — make sure the dispatched subagent reads it; over-deletion there breaks Chunk 6/7's consumers.
