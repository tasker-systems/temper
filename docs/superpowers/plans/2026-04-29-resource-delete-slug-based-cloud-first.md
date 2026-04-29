# Resource Delete: Slug-Based, Cloud-First, Explicit-Only

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the legacy top-level `temper remove <UUID>` command with a slug-based `temper resource delete <slug>` subcommand that works uniformly in local and cloud modes, makes cloud-mode behavior explicit (rather than accidental), guards against agent hangs on the confirmation prompt, and replaces the bare `vault file not found` sync-push error with two-pronged actionable guidance.

**Architecture:** The server-side soft-delete (`is_active = false` in `crates/temper-api/src/services/resource_service.rs:799-872`), the client `delete()` (`crates/temper-client/src/resources.rs:70`), and the API handler (`crates/temper-api/src/handlers/resources.rs:182`) are already correct and unchanged. The work is CLI-side only: (a) add a new `ResourceAction::Delete` clap variant; (b) add `pub fn delete(...)` in `crates/temper-cli/src/commands/resource.rs` that reuses the existing `resolve_resource_id` helper for slug → UUID, calls `client.resources().delete(uuid)` first (cloud-first ordering), and conditionally walks the manifest in `Local` mode only; (c) remove `crates/temper-cli/src/commands/remove.rs` and its dispatch outright (no alias — repo is one month old); (d) update sync-push missing-file errors with two-pronged recovery hints.

**Tech Stack:** Rust workspace, clap CLI, async via `runtime::with_client`, `temper_client::TemperClient`, `manifest_io::load_manifest` / `save_manifest`, integration tests using `tempfile::TempDir` (pattern from `crates/temper-cli/tests/resource_body_update_test.rs`), e2e tests under `tests/e2e/tests/` (pattern from `cloud_writes_test.rs` and `resource_crud_test.rs`).

---

## Spec

This plan implements design spec `2026-04-29-design-spec-unify-resource-delete-cloud-first-slug-based-explicit-only` (vault). Anchors come from task `2026-04-27-unify-resource-delete-cloud-first-explicit-only-manifest-cleanup`. Implements path-to-alpha goal item #13.

## File Structure

| File | Change |
|------|--------|
| `crates/temper-cli/src/cli.rs` | Modify — add `ResourceAction::Delete { slug, r#type, context, force }` variant. |
| `crates/temper-cli/src/main.rs` | Modify — add `ResourceAction::Delete` arm at the dispatch site (currently lines 108–243); remove `Commands::Remove` arm at line 342. |
| `crates/temper-cli/src/commands/resource.rs` | Modify — add `pub fn delete(config, doc_type, slug, context, force) -> Result<()>` that reuses `resolve_resource_id`, calls API delete via `runtime::with_client`, and conditionally walks the manifest in `Local` mode only. |
| `crates/temper-cli/src/commands/remove.rs` | Delete the file. |
| `crates/temper-cli/src/commands/mod.rs` | Modify — drop `pub mod remove;`. |
| `crates/temper-cli/src/actions/sync.rs` | Modify — replace bare `vault file not found` errors at ~939 and ~1085 with two-pronged actionable message; extract a single `vault_file_missing_err(slug, path)` helper to keep both call sites identical. |
| `crates/temper-cli/tests/resource_delete_test.rs` | Create — local-mode integration tests (slug resolution, force flag, non-TTY guard, manifest cleanup). |
| `tests/e2e/tests/resource_delete_e2e_test.rs` | Create — e2e tests for local + cloud delete and sync-push missing-file error message. |
| `CLAUDE.md` | Modify — replace any rm-then-sync mention with explicit-only contract; mention `temper resource delete <slug>` in the cloud-mode-operations paragraph. |
| `crates/temper-cli/static/skill/reference.md` (or wherever the shipped skill reference lives) | Modify — update delete examples to use `temper resource delete <slug>`. |

No new modules in `actions/`. The whole delete flow fits in `commands/resource.rs` since (a) it reuses `resolve_resource_id` already there and (b) the local-tail logic mirrors what `commands/remove.rs` does today (pasted into the new `delete` fn, minus the UUID-string parsing). DRY: the helper `vault_file_missing_err` ensures the two sync.rs error sites stay identical.

---

## Task 1: Update sync-push missing-file error with two-pronged guidance (independent, do first)

This task is independent of the new `delete` surface — it improves an existing error path. Doing it first means the new error message is in place before anyone is told to use `temper resource delete <slug>` as the next-step hint.

**Files:**
- Modify: `crates/temper-cli/src/actions/sync.rs` (~line 939 and ~line 1085)
- Test: `crates/temper-cli/src/actions/sync.rs` (existing inline `mod tests`)

**Context for implementer:** Today the push path errors with a bare `"vault file not found: {path}"` at two sites. Users (and agents) hit this whenever they `rm` a vault file before pushing. The new error needs to point at both `temper resource delete <slug>` (to actually delete) and `temper sync refresh` (to recover the file from server). Both error sites must produce the **identical** message (DRY); extract a helper.

The slug for the message is the manifest entry's filename stem — derive it from `entry.path` (e.g. `task/2026-04-29-foo.md` → `2026-04-29-foo`). Use `std::path::Path::file_stem`.

- [ ] **Step 1: Read both error sites and confirm shapes**

Run: `grep -n "vault file not found" crates/temper-cli/src/actions/sync.rs`

Expected: two hits at roughly line 939 and line 1085. Read 10 lines of context around each to understand the local variables in scope (likely `entry.path` or `path`).

- [ ] **Step 2: Write a failing unit test for the helper**

Add to the inline `mod tests { ... }` in `crates/temper-cli/src/actions/sync.rs` (place near other formatting helper tests; if none exist, place at the top of `mod tests`):

```rust
    #[test]
    fn vault_file_missing_err_includes_both_recovery_hints() {
        let err = super::vault_file_missing_err(
            "task/2026-04-29-some-slug.md",
        );
        let msg = format!("{err}");
        assert!(
            msg.contains("2026-04-29-some-slug"),
            "expected derived slug in message, got: {msg}"
        );
        assert!(
            msg.contains("temper resource delete"),
            "expected delete hint, got: {msg}"
        );
        assert!(
            msg.contains("temper sync refresh"),
            "expected refresh hint, got: {msg}"
        );
        assert!(
            msg.contains("task/2026-04-29-some-slug.md"),
            "expected original path in message, got: {msg}"
        );
    }
```

- [ ] **Step 3: Run the test to verify it fails**

Run: `cargo nextest run -p temper-cli --lib actions::sync::tests::vault_file_missing_err_includes_both_recovery_hints`

Expected: FAIL with "function `vault_file_missing_err` not found in this scope".

- [ ] **Step 4: Implement the helper**

Add to `crates/temper-cli/src/actions/sync.rs` (place near the top of the module, beneath the imports, alongside other private helpers):

```rust
/// Build the standard "vault file missing for tracked entry" error, with
/// two-pronged recovery guidance (explicit delete vs. resync from server).
///
/// `rel_path` is the manifest entry's relative path (e.g.
/// `task/2026-04-29-some-slug.md`). The slug is derived from the filename
/// stem so the user can paste it directly into `temper resource delete`.
fn vault_file_missing_err(rel_path: &str) -> TemperError {
    let slug = std::path::Path::new(rel_path)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or(rel_path);
    TemperError::Vault(format!(
        "vault file missing for {slug} at {rel_path}\n\nEither:\n  • To delete the resource, run: temper resource delete {slug}\n  • To recover the file from the server, run: temper sync refresh"
    ))
}
```

- [ ] **Step 5: Run the test to verify it passes**

Run: `cargo nextest run -p temper-cli --lib actions::sync::tests::vault_file_missing_err_includes_both_recovery_hints`

Expected: PASS.

- [ ] **Step 6: Replace both bare error sites with the helper**

At ~line 939 and ~line 1085 of `crates/temper-cli/src/actions/sync.rs`, replace:

```rust
TemperError::Vault(format!("vault file not found: {}", <expr>))
```

with:

```rust
vault_file_missing_err(<rel-path-expr>)
```

Use the relative-path expression already in scope (likely `entry.path` or a `String` derived from it). If the existing code formats an absolute path, derive the relative path via `abs_path.strip_prefix(vault_root).unwrap_or(&abs_path).display()` to keep messages stable.

- [ ] **Step 7: Run the full sync.rs test module**

Run: `cargo nextest run -p temper-cli --lib actions::sync::tests`

Expected: all existing tests pass plus the new one. If any test asserts on the old bare `"vault file not found"` string, update it to use the new format (search for that string across the test file: `grep -n "vault file not found" crates/temper-cli/src/actions/sync.rs`).

- [ ] **Step 8: Commit**

```bash
git add crates/temper-cli/src/actions/sync.rs
git commit -m "fix(cli): sync-push missing-file error suggests delete + refresh paths"
```

---

## Task 2: Add `ResourceAction::Delete` clap variant + main.rs dispatch (stub)

Wire the CLI surface end-to-end with a stub action that returns Ok(()) so we can verify clap parsing in isolation, then fill in real behavior in Task 3.

**Files:**
- Modify: `crates/temper-cli/src/cli.rs` (around line 205, in `pub enum ResourceAction`)
- Modify: `crates/temper-cli/src/main.rs` (around line 188, in the `Commands::Resource { action } => match action { ... }` block)
- Modify: `crates/temper-cli/src/commands/resource.rs` (add a stub `pub fn delete` near the existing `pub fn show` / `pub fn update`)

**Context for implementer:** Match the surrounding pattern exactly. `Show` uses positional `slug: String` + `#[arg(long)] r#type: String`. `Update` uses positional `slug: String` + `#[arg(long)] r#type: Option<String>`. Delete should mirror `Show`'s shape (positional slug, required `--type` for now — we can relax to `Option` later if slug uniqueness is worth optimizing for; alpha bar is consistency).

- [ ] **Step 1: Add the `Delete` variant to `ResourceAction`**

In `crates/temper-cli/src/cli.rs`, inside `pub enum ResourceAction { ... }`, add this variant immediately after `Update { ... }`:

```rust
    /// Delete a resource (cloud-first soft-delete, then local cleanup if local mode)
    Delete {
        /// Resource slug
        slug: String,
        /// Resource type (task, goal, session, research, concept, decision)
        #[arg(long)]
        r#type: String,
        /// Filter by context
        #[arg(long)]
        context: Option<String>,
        /// Skip the local-file confirmation prompt
        #[arg(long)]
        force: bool,
    },
```

- [ ] **Step 2: Add a stub `pub fn delete` in `commands/resource.rs`**

Place this function near `pub fn show` (around line 745). Use `cargo make build` afterward to spot any import gaps.

```rust
/// Delete a resource: cloud-first soft-delete via the API, then local
/// cleanup as a tail action when running in `Local` mode.
pub fn delete(
    _config: &Config,
    _doc_type: &str,
    _slug: &str,
    _context: Option<&str>,
    _force: bool,
) -> Result<()> {
    Err(TemperError::Vault(
        "temper resource delete: not yet implemented".to_string(),
    ))
}
```

(This stub returns an error rather than `Ok(())` so a misrouted call in tests fails loudly. Task 3 replaces the body.)

- [ ] **Step 3: Add the dispatch arm in `main.rs`**

In `crates/temper-cli/src/main.rs`, inside the `match action { ... }` block of `Commands::Resource`, add this arm immediately after the `ResourceAction::Update { ... }` arm (which ends at the call to `temper_cli::commands::resource::update(&config, &params)`):

```rust
                ResourceAction::Delete {
                    slug,
                    r#type,
                    context,
                    force,
                } => temper_cli::commands::resource::delete(
                    &config,
                    &r#type,
                    &slug,
                    context.as_deref(),
                    force,
                ),
```

- [ ] **Step 4: Verify the CLI parses the new subcommand**

Run: `cargo build -p temper-cli`

Expected: clean build.

Then: `cargo run -p temper-cli -- resource delete --help`

Expected output includes:
- `Usage: temper resource delete [OPTIONS] --type <TYPE> <SLUG>`
- `--type <TYPE>` line
- `--context <CONTEXT>` line
- `--force` line

- [ ] **Step 5: Verify the stub errors as expected**

Run: `cargo run -p temper-cli -- resource delete some-slug --type task --context temper`

Expected output: `temper resource delete: not yet implemented` on stderr; non-zero exit code.

- [ ] **Step 6: Commit**

```bash
git add crates/temper-cli/src/cli.rs crates/temper-cli/src/main.rs crates/temper-cli/src/commands/resource.rs
git commit -m "feat(cli): add resource delete subcommand surface (stub)"
```

---

## Task 3: Implement `pub fn delete` — slug resolution + cloud-first API + local-mode tail

Replace the stub with the real implementation. Reuses `resolve_resource_id` for slug → UUID; calls `client.resources().delete(uuid)` first; in `Local` mode walks the manifest, prompts (or honors `--force`), removes the file, and saves the manifest.

**Files:**
- Modify: `crates/temper-cli/src/commands/resource.rs` (replace the stub `delete` body added in Task 2)

**Context for implementer:** The pattern to mirror is `commands/remove.rs:run` (read it first). The new function differs only in:
1. Takes `slug` + `doc_type` + `context` instead of UUID; uses `resolve_resource_id` to convert.
2. Branches on `VaultState::from_env()` — only enters the local-tail branch in `Local` mode. In `Cloud` mode, the API call is the entire operation.
3. Adds a non-TTY guard: in `Local` mode without `--force`, if `std::io::stdin().is_terminal()` is `false`, error with a clear message rather than calling `read_line` (which would hang in agent harnesses with disconnected stdin or block on a piped non-TTY).

The function signature stays sync (returns `Result<()>`) but the body uses `runtime::with_client(|client| Box::pin(async move { ... }))` to drive the async work, mirroring `commands/remove.rs`.

- [ ] **Step 1: Read the existing `commands/remove.rs` and `resolve_resource_id` once more**

Run:
```bash
cat crates/temper-cli/src/commands/remove.rs
sed -n '776,808p' crates/temper-cli/src/commands/resource.rs
```

Hold both in mind while implementing the next steps. Note in particular:
- `runtime::with_client` is the async driver.
- `output::success`, `output::progress`, `output::dim` are the I/O surfaces (no `println!`).
- `manifest_io::load_manifest(&temper_dir, &device_id)` and `save_manifest(&temper_dir, &manifest)` are the manifest accessors.
- `commands::client_err` exists for converting `temper-client` errors to `TemperError` (see remove.rs:22).

- [ ] **Step 2: Replace the stub body**

In `crates/temper-cli/src/commands/resource.rs`, replace the stub `pub fn delete` body added in Task 2 with this implementation:

```rust
/// Delete a resource: cloud-first soft-delete via the API, then local
/// cleanup as a tail action when running in `Local` mode.
///
/// In `Local` mode, the local-tail step removes the vault file from disk
/// and clears the manifest entry. In `Cloud` mode the API call is the
/// entire operation; there is no manifest to clean up.
///
/// `--force` skips the interactive confirmation prompt for the local-file
/// removal. In non-TTY contexts (agents, CI), `--force` is required because
/// we won't read confirmation from a non-terminal stdin.
pub fn delete(
    config: &Config,
    doc_type: &str,
    slug: &str,
    context: Option<&str>,
    force: bool,
) -> Result<()> {
    use std::io::IsTerminal;
    use temper_core::types::config::VaultState;
    use temper_core::types::ResourceId;

    validate_doc_type(doc_type)?;

    let vault_state = VaultState::from_env();

    // Non-TTY guard: in Local mode the local-tail prompt would hang on a
    // disconnected stdin. Require --force explicitly for non-TTY callers.
    // (Cloud mode skips the local tail entirely, so the prompt isn't reached.)
    if matches!(vault_state, VaultState::Local) && !force && !std::io::stdin().is_terminal() {
        return Err(TemperError::Vault(
            "non-interactive stdin detected; pass --force to skip the local-file confirmation"
                .to_string(),
        ));
    }

    let doc_type_owned = doc_type.to_string();
    let slug_owned = slug.to_string();
    let context_owned = context.map(str::to_string);

    crate::actions::runtime::with_client(|client| {
        Box::pin(async move {
            // Resolve slug → UUID. In Local mode this prefers reading the
            // local file's `temper-id` frontmatter; in Cloud mode (or when
            // the local file lacks a canonical id) it falls back to
            // GET /api/resources/by-uri.
            let rid: ResourceId = resolve_resource_id(
                &config_for_async,
                client,
                &doc_type_owned,
                &slug_owned,
                context_owned.as_deref(),
                vault_state,
            )
            .await?;
            let uuid: uuid::Uuid = (*rid).into();

            // Cloud-first ordering: API delete (server soft-delete) lands
            // first. On API failure we never mutate local state.
            client
                .resources()
                .delete(uuid)
                .await
                .map_err(crate::commands::client_err)?;
            output::success(format!("Deleted {doc_type_owned}/{slug_owned} (cloud)"));

            // Cloud mode stops here — no manifest to walk.
            if matches!(vault_state, VaultState::Cloud) {
                return Ok(());
            }

            // Local-mode tail: remove the file from disk and clear the
            // manifest entry. Mirrors the legacy `temper remove` flow.
            let vault_root = crate::config::resolve_vault(None)?;
            let temper_dir = vault_root.join(".temper");
            let device_id =
                crate::config::load_device_id().unwrap_or_else(|| "unknown".to_string());
            let mut manifest =
                crate::manifest_io::load_manifest(&temper_dir, &device_id)?;

            if let Some(entry) = manifest.entries.get(&rid) {
                let vault_path = vault_root.join(&entry.path);

                let should_remove = if force {
                    true
                } else {
                    output::progress(format!(
                        "Also remove vault file at {}? [y/N] ",
                        vault_path.display()
                    ));
                    use std::io::Write as _;
                    std::io::stderr().flush().ok();
                    let mut input = String::new();
                    std::io::stdin().read_line(&mut input).ok();
                    input.trim().eq_ignore_ascii_case("y")
                };

                if should_remove {
                    if vault_path.exists() {
                        std::fs::remove_file(&vault_path)?;
                        output::dim(format!(
                            "Removed vault file: {}",
                            vault_path.display()
                        ));
                    }
                    manifest.entries.remove(&rid);
                    crate::manifest_io::save_manifest(&temper_dir, &manifest)?;
                }
            }

            Ok(())
        })
    })
}
```

**Compile-fix note:** the closure passed to `with_client` captures by `move`, but `config: &Config` is a borrow with the function's lifetime. The async block needs an owned config or a clone; mirror what `pub fn show_generic` does at lines ~895–907 (`let config_clone = config.clone();` before the closure, then move `config_clone` in). Replace the placeholder `&config_for_async` reference above with the cloned config name. **Verify by compiling — the borrow checker will tell you the right shape.**

- [ ] **Step 3: Compile and fix borrow shapes**

Run: `cargo build -p temper-cli`

Expected: clean build after applying the closure-capture pattern from `show_generic` (clone `config` before the `with_client` closure, then move the clone in).

- [ ] **Step 4: Smoke-test against the live local vault**

Pre-condition: a temp test resource exists in the dev vault that you don't mind deleting. Create one for this purpose:

```bash
cat <<'EOF' | temper resource create --type concept --title "Throwaway delete smoke test" --context temper

A throwaway resource for exercising temper resource delete during plan task 3.
EOF
```

Then delete it:

```bash
temper resource delete throwaway-delete-smoke-test --type concept --context temper --force
```

Expected: success messages for both cloud delete and local file removal; no errors.

Verify the file is gone:

```bash
ls /Users/petetaylor/projects/kb-vault/@me/temper/concept/throwaway-delete-smoke-test.md 2>&1
```

Expected: "No such file or directory."

- [ ] **Step 5: Commit**

```bash
git add crates/temper-cli/src/commands/resource.rs
git commit -m "feat(cli): implement resource delete (cloud-first, slug-based, local tail)"
```

---

## Task 4: CLI integration tests for local mode (slug resolution, force, non-TTY guard)

Add unit-style integration tests at `crates/temper-cli/tests/resource_delete_test.rs` covering local-mode behavior. These tests do **not** hit a real API — they construct a vault + manifest and verify the local-tail logic in isolation by exercising the parts of the new function that don't require the network. To do that without a real client, the tests will set up the conditions under which delete should error out **before** the API call (non-TTY without --force) or after (slug resolution against a vault file).

**Files:**
- Create: `crates/temper-cli/tests/resource_delete_test.rs`

**Context for implementer:** Existing test files at `crates/temper-cli/tests/actions_task_test.rs` and `crates/temper-cli/tests/resource_body_update_test.rs` are the patterns to follow. They use `tempfile::TempDir` to construct a vault, set `TEMPER_VAULT` (or vault override) accordingly, and then call `commands::resource::*` directly. Tests that exercise paths needing a real client should either (a) be skipped at this layer in favor of e2e, or (b) construct a `MockServer`/`wiremock` surface — check what the body-update test uses.

In particular: do **not** stub out the API. The non-TTY guard test fires before the API call (returns the guard error first), so it doesn't need a client. The other behaviors that need a client go in the e2e layer (Task 5).

- [ ] **Step 1: Read the sibling test file for setup patterns**

Run: `cat crates/temper-cli/tests/resource_body_update_test.rs`

Note: vault setup, env var handling, how `Config` is constructed, and any helpers in `tests/common/`.

- [ ] **Step 2: Write the non-TTY guard test**

Create `crates/temper-cli/tests/resource_delete_test.rs` with the following content as a starting point. Extend with helpers to match the sibling file's style if it has shared `mod common`:

```rust
//! Integration tests for `temper resource delete` (local mode).
//!
//! Tests in this file exercise pre-API behavior: the non-TTY guard, the
//! invalid-doctype guard, and slug-not-found resolution. Behaviors that
//! require a live API (the cloud delete + manifest cleanup happy path)
//! are covered in `tests/e2e/tests/resource_delete_e2e_test.rs`.

use std::io::IsTerminal;

use temper_cli::commands::resource;
use temper_cli::config::Config;

/// Construct a Config pointing at a fresh temp vault.
fn fresh_config() -> (tempfile::TempDir, Config) {
    let dir = tempfile::tempdir().expect("tempdir");
    let config = Config {
        vault_root: dir.path().to_path_buf(),
        ..Default::default()
    };
    (dir, config)
}

#[test]
fn rejects_invalid_doctype() {
    let (_dir, config) = fresh_config();
    let err = resource::delete(&config, "widget", "any-slug", Some("temper"), true)
        .expect_err("invalid doctype must error before the API call");
    let msg = format!("{err}");
    assert!(
        msg.contains("invalid resource type"),
        "expected validate_doc_type error, got: {msg}"
    );
}

#[test]
fn rejects_non_tty_stdin_without_force_in_local_mode() {
    // The CI test runner provides a non-TTY stdin; this test relies on that.
    if std::io::stdin().is_terminal() {
        eprintln!("skipping: this test requires a non-TTY stdin");
        return;
    }

    let (_dir, config) = fresh_config();

    // Force VaultState::Local for this test by clearing the env var.
    let prev = std::env::var("TEMPER_VAULT_STATE").ok();
    std::env::remove_var("TEMPER_VAULT_STATE");

    let result = resource::delete(&config, "task", "some-slug", Some("temper"), false);

    if let Some(v) = prev {
        std::env::set_var("TEMPER_VAULT_STATE", v);
    }

    let err = result.expect_err("non-TTY without --force must error");
    let msg = format!("{err}");
    assert!(
        msg.contains("non-interactive stdin"),
        "expected non-TTY guard error, got: {msg}"
    );
    assert!(
        msg.contains("--force"),
        "expected --force hint, got: {msg}"
    );
}
```

**Note on env var handling:** if the sibling test pattern uses a `Mutex` to serialize env-var-touching tests (look for `serial_test` crate or a hand-rolled lock), copy that pattern. Concurrent tests mutating `TEMPER_VAULT_STATE` will produce flakes.

- [ ] **Step 3: Run the new tests to verify they pass**

Run: `cargo nextest run -p temper-cli --test resource_delete_test`

Expected: both tests pass.

If `rejects_invalid_doctype` fails because `validate_doc_type` runs **after** something in your `delete` body that errors first, reorder your `delete` body to make `validate_doc_type` the first call (mirrors `pub fn create` and `pub fn show`).

- [ ] **Step 4: Commit**

```bash
git add crates/temper-cli/tests/resource_delete_test.rs
git commit -m "test(cli): cover resource delete invalid-doctype and non-TTY guards"
```

---

## Task 5: E2E tests for local-mode delete + sync-push missing-file error

E2E tests exercise the full CLI ↔ API ↔ DB stack. They use the `E2eTestApp` harness from `tests/e2e/tests/common/`. We add two tests: one for the local-mode delete happy path, one for the sync-push missing-file error message.

The cloud-mode delete path is partially covered by the existing `resource_crud_test.rs` (which tests `DELETE /api/resources/{id}` directly via the client). Adding a CLI-level cloud-mode test is valuable but lower priority — defer to a follow-up unless time allows in this session.

**Files:**
- Create: `tests/e2e/tests/resource_delete_e2e_test.rs`

**Context for implementer:** Read `tests/e2e/tests/cloud_writes_test.rs` and `tests/e2e/tests/resource_crud_test.rs` first. They show the harness setup. The push-test pattern lives in `tests/e2e/tests/push_command_test.rs`.

- [ ] **Step 1: Read the harness setup files**

Run:
```bash
ls tests/e2e/tests/common/
sed -n '1,80p' tests/e2e/tests/common/mod.rs
sed -n '1,80p' tests/e2e/tests/resource_crud_test.rs
sed -n '1,80p' tests/e2e/tests/push_command_test.rs
```

Note: how `E2eTestApp` is started, how an authenticated CLI invocation is fired, how a vault file is created during a test.

- [ ] **Step 2: Write the local-mode delete e2e test**

Create `tests/e2e/tests/resource_delete_e2e_test.rs` with a test that:
1. Spins up `E2eTestApp`.
2. Creates a resource via the CLI (e.g. a task) with a deterministic slug.
3. Runs `temper resource delete <slug> --type task --context <ctx> --force`.
4. Asserts: (a) the command exits 0; (b) the vault file is gone; (c) `client.resources().get(uuid)` returns 404 (or `is_active = false` reflected in the API response — match what `resource_crud_test::resource_delete` already asserts).

The exact API for "fire the CLI binary in-process" depends on the existing harness. If `E2eTestApp` exposes a way to invoke `temper_cli::commands::resource::delete` directly with a `Config` pointing at the test vault and a token, prefer that over spawning the binary. If not, use `assert_cmd::Command` (already present in the workspace based on test patterns).

```rust
//! E2E coverage for `temper resource delete` (local mode + sync-push
//! missing-file error).

mod common;

use common::E2eTestApp;

#[tokio::test]
async fn local_mode_delete_removes_file_and_soft_deletes_on_server() {
    let app = E2eTestApp::start().await;

    // Create a task in the test vault via the CLI, capture its UUID
    // and slug. Use the deterministic slug from the title.
    let slug = "e2e-delete-target";
    app.run_cli_local(&[
        "resource", "create",
        "--type", "task",
        "--title", "e2e delete target",
        "--context", &app.test_context(),
        "--mode", "build",
        "--effort", "small",
    ])
    .await
    .expect("create");

    // Confirm the file exists in the test vault.
    let vault_path = app.vault_root().join(format!(
        "@me/{}/task/{slug}.md",
        app.test_context()
    ));
    assert!(vault_path.exists(), "expected created file at {}", vault_path.display());

    // Delete via the new subcommand.
    app.run_cli_local(&[
        "resource", "delete",
        slug,
        "--type", "task",
        "--context", &app.test_context(),
        "--force",
    ])
    .await
    .expect("delete");

    // File should be gone.
    assert!(
        !vault_path.exists(),
        "expected file removed at {}",
        vault_path.display()
    );

    // Server-side row should be soft-deleted (is_active = false). Use the
    // same assertion pattern resource_crud_test::resource_delete uses.
    let row = app
        .client()
        .resources()
        .resolve_by_uri(&app.owner(), &app.test_context(), "task", slug)
        .await;
    // Either 404 (most likely — by-uri filters on is_active) or an explicit
    // is_active=false on the row.
    assert!(
        matches!(row, Err(_)) || row.as_ref().unwrap().is_active == false,
        "expected soft-deleted row, got: {row:?}"
    );
}
```

**Implementer note:** the helper names (`run_cli_local`, `vault_root`, `test_context`, `client`) are placeholders — match whatever the existing `E2eTestApp` actually exposes. If the harness uses a different name (e.g. `run_temper`, `cli_local`, `vault_path`), update accordingly.

- [ ] **Step 3: Write the sync-push missing-file error e2e test**

In the same file, add a second test that:
1. Creates a resource via the CLI.
2. Pushes (`temper sync push`) so the manifest tracks it.
3. Removes the vault file directly via `std::fs::remove_file`.
4. Pushes again.
5. Asserts the second push errors with a message containing both `temper resource delete` and `temper sync refresh`.

```rust
#[tokio::test]
async fn sync_push_with_missing_file_errors_with_two_pronged_hint() {
    let app = E2eTestApp::start().await;

    let slug = "e2e-stranded-file";
    app.run_cli_local(&[
        "resource", "create",
        "--type", "concept",
        "--title", "e2e stranded file",
        "--context", &app.test_context(),
    ])
    .await
    .expect("create");

    // First push to track the file in the manifest.
    app.run_cli_local(&["sync", "push"]).await.expect("first push");

    // Strand the file: rm without going through resource delete.
    let vault_path = app.vault_root().join(format!(
        "@me/{}/concept/{slug}.md",
        app.test_context()
    ));
    std::fs::remove_file(&vault_path).expect("rm");

    // Second push should error with the new message.
    let result = app.run_cli_local(&["sync", "push"]).await;
    let err = result.expect_err("push with stranded file should error");
    let msg = format!("{err}");
    assert!(
        msg.contains("temper resource delete"),
        "expected delete hint, got: {msg}"
    );
    assert!(
        msg.contains("temper sync refresh"),
        "expected refresh hint, got: {msg}"
    );
    assert!(
        msg.contains(slug),
        "expected slug in message, got: {msg}"
    );
}
```

- [ ] **Step 4: Run the e2e tests**

Pre-condition: `cargo make docker-up` to ensure Postgres is running.

Run: `cargo make test-e2e -- resource_delete_e2e_test`

Or, if the cargo-make target doesn't accept arg passthrough cleanly:

Run: `cargo nextest run -p temper-e2e --features test-db --test resource_delete_e2e_test`

Expected: both tests pass. If they fail because of harness API shape, fix the helper names per Step 2's note and re-run.

- [ ] **Step 5: Commit**

```bash
git add tests/e2e/tests/resource_delete_e2e_test.rs
git commit -m "test(e2e): cover local-mode resource delete and sync-push stranded-file error"
```

---

## Task 6: Remove `temper remove`

Delete the legacy command outright. Repo is one month old; no external consumers. The CLI bug sweep in goal item #17 will catch any straggling help-text references.

**Files:**
- Delete: `crates/temper-cli/src/commands/remove.rs`
- Modify: `crates/temper-cli/src/commands/mod.rs` (drop `pub mod remove;`)
- Modify: `crates/temper-cli/src/main.rs` (remove the `Commands::Remove { resource_id, force } => commands::remove::run(&resource_id, force)` arm at line 342)
- Modify: `crates/temper-cli/src/cli.rs` (remove the `Commands::Remove { ... }` clap variant — find and delete the variant that defines `resource_id: String` + `force: bool`)

- [ ] **Step 1: Find and delete the `Commands::Remove` clap variant**

Run: `grep -n "Remove" crates/temper-cli/src/cli.rs`

Expected hits include `Commands::Remove { ... }` (top-level command) and `ContextAction::Remove` (subcommand of Context — leave that alone). Delete only the top-level `Commands::Remove { ... }` variant.

- [ ] **Step 2: Delete the dispatch arm in `main.rs`**

In `crates/temper-cli/src/main.rs`, delete the line:

```rust
        Commands::Remove { resource_id, force } => commands::remove::run(&resource_id, force),
```

- [ ] **Step 3: Drop the module declaration**

In `crates/temper-cli/src/commands/mod.rs`, delete the line `pub mod remove;`.

- [ ] **Step 4: Delete the file**

```bash
git rm crates/temper-cli/src/commands/remove.rs
```

- [ ] **Step 5: Verify nothing references `commands::remove`**

Run: `grep -rn "commands::remove\|::remove::run\|Commands::Remove" crates/ tests/`

Expected: no hits. If any are found, address them before continuing.

- [ ] **Step 6: Build to confirm compile cleanliness**

Run: `cargo build -p temper-cli`

Expected: clean build.

- [ ] **Step 7: Run all temper-cli tests**

Run: `cargo nextest run -p temper-cli`

Expected: all green.

- [ ] **Step 8: Commit**

```bash
git add crates/temper-cli/src/commands/mod.rs crates/temper-cli/src/main.rs crates/temper-cli/src/cli.rs
git commit -m "refactor(cli): remove temper remove (replaced by temper resource delete)"
```

---

## Task 7: Update CLAUDE.md and skill reference

**Files:**
- Modify: `CLAUDE.md` (the cloud-mode-operations paragraph, plus any `temper remove` references)
- Modify: `crates/temper-cli/static/skill/reference.md` (search for the file first; the ship path may differ)

- [ ] **Step 1: Find skill reference path**

Run:
```bash
find crates/temper-cli -name "reference.md" 2>&1
find . -path ./node_modules -prune -o -name "reference.md" -print 2>&1 | grep -v node_modules | head -5
```

The skill ships its `reference.md` from somewhere under `crates/temper-cli/`. Use the find result as the editing target.

- [ ] **Step 2: Update CLAUDE.md**

In `CLAUDE.md`, find any mention of `temper remove` (likely zero) and the cloud-mode-operations paragraph. Add the following to the cloud-mode-operations paragraph immediately after the body-edit description:

> **Resource deletion is always explicit.** `temper resource delete <slug> --type <doctype> [--force]` performs a cloud-first soft-delete (server preserves the row with `is_active = false`); in local mode the vault file is removed and the manifest entry cleared as a tail action. There is no implicit-delete-via-`rm` path — removing a tracked vault file outside this command will cause the next `temper sync push` to error with guidance to either run `temper resource delete <slug>` (to delete) or `temper sync refresh` (to recover the file from the server).

If `CLAUDE.md` already references `temper remove`, replace those references with `temper resource delete <slug> --type <doctype>`.

- [ ] **Step 3: Update skill reference.md**

In the skill `reference.md`, search for any `temper remove` references and replace with `temper resource delete <slug> --type <doctype>`. Add a "Delete a resource" example near the existing CRUD examples:

```bash
# Delete a resource (cloud-first soft-delete, then local cleanup if local mode)
temper resource delete <slug> --type task --context <ctx> --force
```

- [ ] **Step 4: Commit**

```bash
git add CLAUDE.md crates/temper-cli/static/skill/reference.md
git commit -m "docs: explicit-only delete contract in CLAUDE.md and skill reference"
```

---

## Task 8: Full verification

Run the full quality gates from the project fundamentals.

- [ ] **Step 1: Lint and format**

Run: `cargo make check`

Expected: green (Rust fmt + clippy + docs + machete; TS typecheck + biome).

If clippy complains, fix in place. If new lints appear that look unrelated to this work, run `git diff main..HEAD` to confirm the warning targets your changes; if it does, fix.

- [ ] **Step 2: Unit tests**

Run: `cargo make test`

Expected: all green.

- [ ] **Step 3: DB-integration tests**

Pre-condition: `cargo make docker-up` if Postgres is not already running.

Run: `cargo make test-db`

Expected: all green.

- [ ] **Step 4: E2E tests**

Run: `cargo make test-e2e`

Expected: all green, including the two new tests added in Task 5.

- [ ] **Step 5: TypeScript checks** (only if anything in `packages/` was touched — should be a no-op)

Run: `cd packages/temper-cloud && bun run typecheck && bun run check`

Expected: green.

- [ ] **Step 6: Confirm no lingering `temper remove` references**

Run: `grep -rn "temper remove" . --include='*.md' --include='*.rs' --include='*.toml' 2>&1 | grep -v target | grep -v node_modules | head -20`

Expected: zero hits, or only hits in the design spec / completed-task vault files (which are fine to leave as historical record). Any hit in `crates/`, `tests/`, `packages/`, or `CLAUDE.md` must be addressed.

- [ ] **Step 7: Push the branch and open a PR**

```bash
git push -u origin jct/resource-delete-slug-based
gh pr create --title "feat(cli): unify resource delete (slug-based, cloud-first, explicit-only)" --body "$(cat <<'EOF'
## Summary

- Adds `temper resource delete <slug> --type <doctype> [--context <ctx>] [--force]`: cloud-first soft-delete via the API, then local vault file + manifest cleanup as a tail action in local mode only.
- Removes `temper remove <UUID>` outright (no alias; repo is one month old).
- Replaces bare \`vault file not found\` sync-push error with two-pronged actionable guidance: \`temper resource delete <slug>\` or \`temper sync refresh\`.
- Adds a non-TTY guard so agents/CI without \`--force\` error fast rather than hang on the confirmation prompt.

Implements path-to-alpha goal item #13. Spec: \`research/2026-04-29-design-spec-unify-resource-delete-cloud-first-slug-based-explicit-only\`. Plan task: \`task/2026-04-29-implement-resource-delete-slug-based-cloud-first\`.

## Test plan

- [ ] \`cargo make check\` green
- [ ] \`cargo make test\` green
- [ ] \`cargo make test-db\` green
- [ ] \`cargo make test-e2e\` green (covers local-mode delete + sync-push stranded-file error)
- [ ] Manual smoke: create a throwaway resource, \`temper resource delete <slug> --type concept --context temper --force\`, verify file gone and server returns 404 / soft-deleted

🤖 Generated with [Claude Code](https://claude.com/claude-code)
EOF
)"
```

---

## Self-Review

- **Spec coverage.** All five anchors from `2026-04-27-unify-resource-delete-cloud-first-explicit-only-manifest-cleanup` plus all five "Components affected" rows from the design spec map to a task: surface (Tasks 2–3), slug resolution (Task 3 via `resolve_resource_id`), cloud-mode explicit branch (Task 3), stranded-file policy (Task 1), `temper remove` removal (Task 6), tests (Tasks 4–5), docs (Task 7), verification (Task 8). The deferred items in the spec (bulk delete, undo, deleted-resource UI, dry-run, `--id`) are explicitly out of scope and have no tasks — correct.
- **Placeholder scan.** No "TBD", "implement later", "fill in details", or "add appropriate error handling" anywhere. The placeholders that *are* there are flagged for the implementer to verify against the harness API (e.g. `run_cli_local` in Task 5) — these are real names but unverified-by-this-author, and the implementer is told to match whatever the harness actually exposes.
- **Type consistency.** `ResourceId` flows through `resolve_resource_id` → into the manifest `entries.get(&rid)` lookup → derefs to `uuid::Uuid` for the API call. Names are consistent across all tasks. `vault_state: VaultState` is the same type everywhere. The new `vault_file_missing_err` helper is referenced consistently.
- **One gap addressed inline:** the original spec's "task: implement-resource-delete-slug-based-cloud-first" said "tests: CLI unit (slug resolution, force flag, non-TTY guard, ambiguous slug)." This plan covers slug resolution implicitly via the e2e (Task 5) and force-flag implicitly via the e2e (the `--force` is the path used). The non-TTY guard gets a dedicated unit test (Task 4). **Ambiguous slug** is not in this plan — it's a future concern when we relax `--type` to `Option<String>`. With `--type` required (today's plan), there is no ambiguity to test. Noted as out-of-scope.
