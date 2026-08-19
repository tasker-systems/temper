# Cloud-Only Vault Chunk 3 — `CloudBackend` Unconditional, Remove `VaultState` Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make the temper CLI cloud-only: `backend_select` returns `CloudBackend` unconditionally, the `VaultState` mode switch is removed, every `match VaultState` collapses to its cloud branch, and `create`/`update`/`delete` rewrite (or remove) the affected local projection file on success.

**Architecture:** This is Chunk 3 of the 8-chunk cloud-only-vault deprecation (spec: `docs/superpowers/specs/2026-05-21-cloud-only-vault-deprecation-design.md`), the first chunk of PR B. It is the breaking mode flip. The redesign keeps `CloudBackend`, the `Backend` trait, and `temper-client` — the cloud write path is already built. `vault_backend/` is **not** deleted here: it is `pub`-exported from `temper-cli`'s lib, so it survives Chunk 3 as compiled-but-unreferenced code that triggers no dead-code warning; Chunk 4 deletes it. `Surface::CliLocalVault` therefore also stays (vault_backend still constructs it). The projection write-on-success is surface-side, mirroring Chunk 2's `temper resource show`, enabled by a `build_backend` that now also hands back the API client.

**Tech Stack:** Rust, tokio, `temper-client` (HTTP), clap, cargo-make + cargo-nextest. clippy runs with `-D warnings` (dead-code is a build error). Quality gates this session: rust-analyzer LSP, superpowers TDD, `cargo make check`, greptile review.

---

## Background: why these specific changes

`VaultState` (`crates/temper-core/src/types/config.rs`) is an enum with `Local` / `Cloud`, resolved from the `TEMPER_VAULT_STATE` env var. It is matched in ~20 places across `temper-cli`. Chunk 3 removes the enum entirely; every match collapses to its `Cloud` arm.

Two facts shape the task ordering:

1. **`VaultState` can only be deleted from `temper-core` once zero references remain.** Every consumer is rewritten first (Tasks 1–14); the enum is removed last (Task 15). Collapsing a `match VaultState::from_env() { Cloud => X, Local => Y }` to just `X` compiles fine *while the enum still exists* — you simply stop referencing it.

2. **`-D warnings` means every task must leave the build green.** A `pub` item of `temper-cli`'s lib crate is part of the public API and is *not* dead-code-flagged when it loses its callers — so `pub` modules (`vault_backend/`, `lookup`, `manifest_io`, `actions::show_cache`, the `pub fn`s `scan_rows`/`render_list`) survive Chunk 3 untouched and are swept in Chunks 4–6. But a `pub(crate)` or private item *is* flagged. So each task that orphans a `pub(crate)`/private helper **must delete that helper in the same task** — there is no trailing sweep for those.

The `pub(crate)`/private helpers that become orphaned during Chunk 3, and the task that deletes each:

| Helper | Visibility | Orphaned by | Deleted in |
|---|---|---|---|
| `surface_for_state` | `pub(crate)` fn, `commands/resource.rs` | its 4 callers rewired | Task 3 |
| `has_vault_file_event` | `pub(crate)` fn, `commands/resource.rs` | create/update/session-save rewired | Task 12 |
| `vault_file_path_from_events` | `pub(crate)` fn, `commands/resource.rs` | create/update rewired | Task 12 |
| `find_or_compute_local_path` | private fn, `commands/resource.rs` | `show_generic` Local branch removed | Task 10 |
| `resolve_id_local_first` | `pub(crate)` fn, `commands/resource.rs` | all 3 `show` Local branches removed | Task 12 |
| `show_via_api_fallback` | `pub(crate)` fn, `commands/resource.rs` | all 3 `show` Local branches removed | Task 12 |

`scan_rows`, `render_list`, `RenderListParams`, `ListFilters`, the local `ResourceRow` struct, `parse_row`, `sort_rows`, `filter_rows`, `lookup::find_resource`, `manifest_io::*`, `actions::show_cache::*` are all either `pub` or still referenced by the surviving `vault_backend/` — **leave them all alone**. They are removed in later chunks.

---

## File Structure

Files modified in this chunk:

- `crates/temper-cli/src/backend_select.rs` — collapse to unconditional `CloudBackend`; `build_backend` returns the client.
- `crates/temper-cli/src/projection.rs` — add `remove_resource_file`.
- `crates/temper-cli/src/commands/resource.rs` — collapse `require_context`, `surface_for_state`, `create`, `list`, `delete`, `update`, `show_generic`; projection write/remove tails.
- `crates/temper-cli/src/commands/task.rs` — collapse `show`; projection refresh.
- `crates/temper-cli/src/commands/session.rs` — collapse `session_exists`, `show`, `save`.
- `crates/temper-cli/src/commands/auth.rs` — re-key `export_token` guard.
- `crates/temper-cli/src/actions/runtime.rs` — re-key `resolve_token_store`.
- `crates/temper-cli/src/commands/sync_cmd.rs` — `run` becomes a cloud-only error.
- `crates/temper-core/src/types/config.rs` — delete `VaultState`, `from_env`, `is_cloud`, `TEMPER_VAULT_STATE_ENV`, tests.
- `crates/temper-core/src/types/mod.rs` — drop the `VaultState` re-export.
- `tests/e2e/tests/` — new projection-write e2e tests.

`vault_backend/`, `lookup.rs`, `manifest_io.rs`, `actions/show_cache.rs`, the `Surface` enum, the `DomainEvent::VaultFile*` variants are **not** touched.

---

## Project code-quality rules (apply to every task)

These are inherited from `crates/temper-cli/.claude` guidance and `CLAUDE.md`. Every implementer subagent must follow them:

- **Typed structs over inline JSON** — never `serde_json::json!()` for data with a known shape; define a struct. (Pre-existing `json!` in `render_create_output_to_string`/`update` is out of scope — do not widen it, do not refactor it.)
- **Service/backend layering** — writes dispatch through the `Backend` trait; reads stay surface-direct. The projection write is a *derivative refresh*, not the authoritative write — it is surface-side, exactly like Chunk 2's `show`.
- **Params structs at >5 args**; `#[expect(clippy::too_many_arguments)]` is a smell. (The existing `#[expect]` on `create` is pre-existing — leave it.)
- **Auth before writes.**
- **`#[expect(lint, reason = "...")]`** over `#[allow]`.
- All public types implement `Debug`.
- Match the surrounding code's comment density and idiom.

---

## Task 1: Collapse `backend_select` to unconditional `CloudBackend`

**Files:**
- Modify: `crates/temper-cli/src/backend_select.rs`

The current `build_backend` matches `VaultState::from_env()` and constructs `VaultBackend` (Local) or `CloudBackend` (Cloud). Drop the match and the Local arm. `VaultState` is *not* removed from `temper-core` yet — this task simply stops `backend_select` referencing it.

- [ ] **Step 1: Rewrite `build_backend` and its doc comment**

Replace lines 1–47 of `backend_select.rs` (the module doc comment through the end of `build_backend`) with:

```rust
//! Backend selection — the single helper surfaces use to acquire a
//! `Box<dyn Backend>`.
//!
//! temper is cloud-only: every surface dispatches writes through
//! `CloudBackend`. Surfaces never instantiate `CloudBackend` directly;
//! they always go through this helper.
//!
//! See `docs/superpowers/specs/2026-05-21-cloud-only-vault-deprecation-design.md`.

use tokio::runtime::Runtime;

use temper_core::operations::Backend;

use crate::config::Config;
use crate::error::Result;

/// Build a tokio runtime + `Box<dyn Backend>` for a CLI invocation.
///
/// Always returns `CloudBackend` via `assemble_cloud_backend`, which
/// errors if no token resolves — temper is cloud-only and has no offline
/// write path. In no-embed builds, `CloudBackend`'s methods return
/// `BadRequest`.
///
/// **Why bundle the runtime:** `assemble_cloud_backend` constructs a
/// runtime, then builds the client on it. Returning both as a tuple
/// gives surfaces one `block_on` handle without constructing a second
/// runtime by accident.
pub fn build_backend(config: &Config, ctx: &str) -> Result<(Runtime, Box<dyn Backend>)> {
    let (runtime, backend_ctx) = crate::cloud_backend::assemble_cloud_backend(config, ctx)?;
    let backend: Box<dyn Backend> = Box::new(crate::cloud_backend::CloudBackend::new(backend_ctx));
    Ok((runtime, backend))
}
```

(The return type stays a 2-tuple in this task; Task 2 widens it.)

- [ ] **Step 2: Replace the test**

The existing test `build_backend_local_mode_succeeds_when_state_is_local` (lines 64–90) tested local mode and is now invalid. Replace the entire `#[cfg(test)] mod tests { ... }` block with:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn make_config(vault_root: &std::path::Path) -> Config {
        Config {
            vault_root: vault_root.to_path_buf(),
            state_dir: vault_root.join(".temper"),
            contexts: vec!["temper".to_string()],
            subscriptions: vec![],
            skill_output: vault_root.join("skills"),
            profile_slug: None,
        }
    }

    #[test]
    fn build_backend_errors_without_a_token() {
        // temper is cloud-only — a write backend requires a resolved
        // token. With no token, `build_backend` must fail fast with a
        // clear `temper auth login` directive (before any network call).
        let temp = tempfile::tempdir().unwrap();
        let config = make_config(temp.path());
        let auth_path = temp.path().join("auth.json");
        let nonexistent_config = temp.path().join("no-such-config.toml");

        temp_env::with_vars(
            [
                ("TEMPER_TOKEN", None::<&str>),
                ("TEMPER_AUTH_PATH", Some(auth_path.to_str().unwrap())),
                (
                    "TEMPER_GLOBAL_CONFIG",
                    Some(nonexistent_config.to_str().unwrap()),
                ),
            ],
            || {
                let err = build_backend(&config, "temper")
                    .expect_err("no token must error");
                assert!(
                    format!("{err:?}").contains("temper auth login"),
                    "expected auth-login directive, got: {err:?}"
                );
            },
        );
    }
}
```

- [ ] **Step 3: Verify**

Run: `cargo nextest run -p temper-cli build_backend`
Expected: PASS (`build_backend_errors_without_a_token`).

Run: `cargo make check`
Expected: green. (`VaultState` still exists in `temper-core`; `backend_select` no longer references it — that is fine.)

- [ ] **Step 4: Commit**

```bash
git add crates/temper-cli/src/backend_select.rs
git commit -m "cloud-only(ch3): backend_select returns CloudBackend unconditionally"
```

---

## Task 2: `build_backend` returns the API client

**Files:**
- Modify: `crates/temper-cli/src/backend_select.rs`
- Modify: every `build_backend` call site (compiler-enumerated; known sites: `commands/resource.rs` `create`/`update`/`delete`, plus any in `commands/session.rs`)

Surfaces need an `Arc<TemperClient>` to do the post-write projection refresh (Tasks 5–7). `assemble_cloud_backend` already builds the client into `CloudBackendCtx`; thread a clone of it out.

- [ ] **Step 1: Widen the return type**

In `backend_select.rs`, change the `use` block and `build_backend` to:

```rust
use std::sync::Arc;

use tokio::runtime::Runtime;

use temper_client::TemperClient;
use temper_core::operations::Backend;

use crate::config::Config;
use crate::error::Result;
```

```rust
pub fn build_backend(
    config: &Config,
    ctx: &str,
) -> Result<(Runtime, Box<dyn Backend>, Arc<TemperClient>)> {
    let (runtime, backend_ctx) = crate::cloud_backend::assemble_cloud_backend(config, ctx)?;
    // Clone the `Arc` out before `CloudBackend::new` consumes the ctx —
    // surfaces use it for the post-write projection refresh.
    let client = Arc::clone(&backend_ctx.client);
    let backend: Box<dyn Backend> = Box::new(crate::cloud_backend::CloudBackend::new(backend_ctx));
    Ok((runtime, backend, client))
}
```

Update the doc comment's first line to mention the returned client: append a sentence — `The returned `Arc<TemperClient>` is the same client the backend dispatches through; surfaces use it for the post-write projection refresh.`

- [ ] **Step 2: Fix every call site**

Run `rg -n 'build_backend\(' crates/temper-cli/src` to enumerate. Each site currently binds `let (runtime, backend) = ...`. Update each to `let (runtime, backend, client) = ...`. Where the client is not used yet in that function, bind it `_client` for now (Tasks 5–7 will start using it in `create`/`update`/`delete`; bind those three as `client`).

- [ ] **Step 3: Verify**

Run: `cargo make check`
Expected: green.

Run: `cargo nextest run -p temper-cli`
Expected: green.

- [ ] **Step 4: Commit**

```bash
git add crates/temper-cli/src
git commit -m "cloud-only(ch3): build_backend hands back the API client for projection refresh"
```

---

## Task 3: Remove `surface_for_state`; callers use `Surface::CliCloud`

**Files:**
- Modify: `crates/temper-cli/src/commands/resource.rs` (delete `surface_for_state`, lines 36–43; rewire callers at lines 272, 785, 1467)
- Modify: `crates/temper-cli/src/commands/session.rs` (the `save` call site)

`surface_for_state()` maps `VaultState` to a `Surface`. Cloud is the only mode; every caller wants `Surface::CliCloud` directly.

- [ ] **Step 1: Delete `surface_for_state`**

In `commands/resource.rs`, delete the entire function (lines 36–43):

```rust
/// Map current `VaultState` to the appropriate `Surface` origin for cmd construction.
pub(crate) fn surface_for_state() -> temper_core::operations::Surface {
    use temper_core::types::config::VaultState;
    match VaultState::from_env() {
        VaultState::Local => temper_core::operations::Surface::CliLocalVault,
        VaultState::Cloud => temper_core::operations::Surface::CliCloud,
    }
}
```

- [ ] **Step 2: Rewire callers**

Run `rg -n 'surface_for_state\(\)' crates/temper-cli/src`. Replace every `origin: surface_for_state(),` with:

```rust
        origin: temper_core::operations::Surface::CliCloud,
```

Known sites: `commands/resource.rs` in `create` (~line 272), `delete` (~line 785), `update` (~line 1467); `commands/session.rs` in `save`.

- [ ] **Step 3: Verify**

Run: `rg -n 'surface_for_state' crates/temper-cli/src`
Expected: no matches.

Run: `cargo make check`
Expected: green.

- [ ] **Step 4: Commit**

```bash
git add crates/temper-cli/src
git commit -m "cloud-only(ch3): drop surface_for_state — origin is always CliCloud"
```

---

## Task 4: Add `projection::remove_resource_file`

**Files:**
- Modify: `crates/temper-cli/src/projection.rs`

`temper resource delete`, on success, must remove the resource's projection file. `projection.rs` already owns projection file IO (`write_resource_file_from_parts`, `prune_context`); add the single-file removal counterpart so `delete`'s surface does not inline `std::fs` against a vault path.

- [ ] **Step 1: Write the failing test**

In `projection.rs`, inside `#[cfg(test)] mod tests`, add:

```rust
    #[test]
    fn remove_resource_file_deletes_the_canonical_file() {
        let dir = tempfile::TempDir::new().unwrap();
        let root = dir.path();
        let task_dir = root.join("@me/myctx/task");
        std::fs::create_dir_all(&task_dir).unwrap();
        let file = task_dir.join("doomed.md");
        std::fs::write(&file, "body").unwrap();

        remove_resource_file(root, "@me", "myctx", "task", "doomed").unwrap();

        assert!(!file.exists(), "projection file removed");
    }

    #[test]
    fn remove_resource_file_is_ok_when_file_absent() {
        let dir = tempfile::TempDir::new().unwrap();
        // Never-written file: removal is a silent no-op, not an error.
        remove_resource_file(dir.path(), "@me", "myctx", "task", "ghost").unwrap();
    }
```

- [ ] **Step 2: Run the test, verify it fails**

Run: `cargo nextest run -p temper-cli -E 'test(remove_resource_file)'`
Expected: FAIL — `cannot find function remove_resource_file`.

- [ ] **Step 3: Implement `remove_resource_file`**

In `projection.rs`, add this function after `write_resource_file` (after line 277):

```rust
/// Remove a resource's projection file at its canonical vault path.
///
/// A best-effort counterpart to [`write_resource_file_from_parts`], used
/// by `temper resource delete` after a successful server-side delete. An
/// already-absent file is a silent success — the projection is
/// derivative, so "the file is gone" is the desired end state either way.
pub fn remove_resource_file(
    vault_root: &Path,
    owner: &str,
    context: &str,
    doc_type: &str,
    slug: &str,
) -> Result<()> {
    let path = Vault::new(vault_root).doc_file(owner, context, doc_type, slug);
    match std::fs::remove_file(&path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(TemperError::Config(format!(
            "projection remove {}: {e}",
            path.display()
        ))),
    }
}
```

- [ ] **Step 4: Run the tests, verify they pass**

Run: `cargo nextest run -p temper-cli -E 'test(remove_resource_file)'`
Expected: PASS (both tests).

Run: `cargo make check`
Expected: green.

- [ ] **Step 5: Commit**

```bash
git add crates/temper-cli/src/projection.rs
git commit -m "cloud-only(ch3): add projection::remove_resource_file"
```

---

## Task 5: `create` writes the projection file on success

**Files:**
- Modify: `crates/temper-cli/src/commands/resource.rs` (`create`, lines 205–310)
- Test: `tests/e2e/tests/cloud_writes_test.rs` (or a sibling e2e test file)

On a successful `create`, the CLI must materialize the new resource's projection file so the local copy reflects server state immediately. This is surface-side, using the client from `build_backend` and `projection::write_resource_file` (the fetch-then-write helper, which already exists). The mode-implicit discovery-event gate (`has_vault_file_event`) is also removed — cloud is the only mode.

- [ ] **Step 1: Write the failing e2e test**

In the e2e crate, following the harness pattern in `tests/e2e/tests/cloud_writes_test.rs` (real Axum + Postgres), add a test:

```rust
#[tokio::test]
async fn create_writes_canonical_projection_file() {
    // Harness: spawn server, authenticate, point the CLI's vault_root at
    // a tempdir. Run `temper resource create --type task --title "..."`
    // through the CLI code path used by the other tests in this file.
    //
    // Assert: after a successful create, the projection file exists at
    // <vault_root>/@me/<context>/task/<slug>.md and its body round-trips.
}
```

Implement it concretely against the existing harness (mirror an existing `cloud_writes_test.rs` test for setup; the new assertion is `assert!(projection_path.exists())` plus a frontmatter `temper-slug` check).

- [ ] **Step 2: Run the test, verify it fails**

Run: `cargo nextest run --manifest-path tests/e2e/Cargo.toml --features test-db -E 'test(create_writes_canonical_projection_file)'`
Expected: FAIL — projection file does not exist (create does not write it yet).

- [ ] **Step 3: Add the projection-write tail to `create`**

In `commands/resource.rs::create`, the dispatch block currently reads (lines ~285–307):

```rust
    // Acquire backend (mode picked via VaultState::from_env) and dispatch.
    let (runtime, backend) = crate::backend_select::build_backend(config, &ctx)?;
    let output = runtime.block_on(backend.create_resource(cmd))?;

    // Discovery event (local mode only — gated on VaultFileWritten presence).
    // Concept and Decision were never emitted pre-Phase 5; preserve that parity.
    if !matches!(
        doctype_enum,
        temper_core::frontmatter::DocType::Concept | temper_core::frontmatter::DocType::Decision
    ) && has_vault_file_event(&output.events)
    {
        let rel_path = vault_file_path_from_events(&output.events).unwrap_or_default();
        let event = Event::ResourceCreate {
            ts: Local::now().to_rfc3339(),
            doc_type: doc_type.to_string(),
            title: title.to_string(),
            path: rel_path,
            context: ctx.to_string(),
        };
        if let Err(e) = discovery::append_event(&config.state_dir, &event) {
            tracing::warn!("Failed to append discovery event: {e}");
        }
    }

    render_create_output(&output, doc_type, format)
```

Replace it with:

```rust
    // Acquire the cloud backend + client and dispatch the create.
    let (runtime, backend, client) = crate::backend_select::build_backend(config, &ctx)?;
    let output = runtime.block_on(backend.create_resource(cmd))?;

    // Projection refresh: write the new resource to its canonical
    // projection path so the local copy reflects server state at once.
    // Best-effort — a projection write failure must not fail the create.
    let projection_path = match runtime.block_on(crate::projection::write_resource_file(
        &client,
        &config.vault_root,
        &output.value,
    )) {
        Ok(path) => Some(path),
        Err(e) => {
            output::warning(format!("could not write projection file: {e}"));
            None
        }
    };

    // Discovery event for non-Concept/Decision doctypes (Concept and
    // Decision were never emitted pre-Phase 5; preserve that parity).
    if !matches!(
        doctype_enum,
        temper_core::frontmatter::DocType::Concept | temper_core::frontmatter::DocType::Decision
    ) {
        let rel_path = projection_path
            .as_deref()
            .and_then(|p| p.strip_prefix(&config.vault_root).ok())
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_default();
        let event = Event::ResourceCreate {
            ts: Local::now().to_rfc3339(),
            doc_type: doc_type.to_string(),
            title: title.to_string(),
            path: rel_path,
            context: ctx.to_string(),
        };
        if let Err(e) = discovery::append_event(&config.state_dir, &event) {
            tracing::warn!("Failed to append discovery event: {e}");
        }
    }

    render_create_output(&output, doc_type, format, projection_path.as_deref())
}
```

Then update `render_create_output` and `render_create_output_to_string` to take a `projection_path: Option<&std::path::Path>` parameter *instead of* deriving the path from `vault_file_path_from_events(&output.events)`. Concretely:

- Change `render_create_output`'s signature to add `projection_path: Option<&std::path::Path>` and forward it to `render_create_output_to_string`.
- In `render_create_output_to_string`, change the signature the same way, and replace the line
  `let vault_path = vault_file_path_from_events(&output.events);`
  with
  ```rust
  let vault_path: Option<String> = projection_path
      .map(|p| p.to_string_lossy().into_owned());
  ```
  The rest of `render_create_output_to_string` (the per-doctype `json!` blocks that read `vault_path.as_deref().unwrap_or("")`) is unchanged. Now the JSON `path` field carries the real projection path the create just wrote — the path is valid on disk, so agents chaining `... --format json | jq -r .path` get a real file.

Note: do not delete `has_vault_file_event` / `vault_file_path_from_events` here — `update` and `session::save` still call them. Task 12 deletes both.

- [ ] **Step 4: Run the test, verify it passes**

Run: `cargo nextest run --manifest-path tests/e2e/Cargo.toml --features test-db -E 'test(create_writes_canonical_projection_file)'`
Expected: PASS.

Run: `cargo make check`
Expected: green.

- [ ] **Step 5: Commit**

```bash
git add crates/temper-cli/src/commands/resource.rs tests/e2e
git commit -m "cloud-only(ch3): create writes the projection file on success"
```

---

## Task 6: `update` writes the projection file; collapse to cloud-only rendering

**Files:**
- Modify: `crates/temper-cli/src/commands/resource.rs` (`update`, lines 1427–1525)
- Test: `tests/e2e/tests/cloud_writes_test.rs`

`update`'s rendering is currently mode-implicit: a `VaultFileWritten` event (local) renders `"Updated: {path}"`; its absence (cloud) renders `{temper-slug, content_hash}` JSON. Cloud is the only mode now — render the JSON unconditionally and add the projection-write tail.

- [ ] **Step 1: Write the failing e2e test**

In the e2e crate, add (mirroring the harness in `cloud_writes_test.rs`):

```rust
#[tokio::test]
async fn update_rewrites_projection_file_on_success() {
    // Harness: create a resource, then `temper resource update` it with a
    // new body. Assert the projection file on disk now matches the
    // returned server row (new body present).
}
```

- [ ] **Step 2: Run the test, verify it fails**

Run: `cargo nextest run --manifest-path tests/e2e/Cargo.toml --features test-db -E 'test(update_rewrites_projection_file_on_success)'`
Expected: FAIL — projection file is absent or stale.

- [ ] **Step 3: Rewrite `update`'s dispatch + render block**

In `commands/resource.rs::update`, replace the block from line 1470 (`// 5. Acquire backend + dispatch.`) through the end of the function (line 1525, the closing `}` of `update`) with:

```rust
    // 5. Acquire the cloud backend + client and dispatch the update.
    let (runtime, backend, client) = crate::backend_select::build_backend(config, &ctx)?;
    let output = runtime.block_on(backend.update_resource(cmd))?;

    // 6. Projection refresh: rewrite the affected projection file from
    //    the returned server row. Best-effort — a projection write
    //    failure must not fail the update.
    if let Err(e) = runtime.block_on(crate::projection::write_resource_file(
        &client,
        &config.vault_root,
        &output.value,
    )) {
        output::warning(format!("could not rewrite projection file: {e}"));
    }

    // 7. Emit the agent-facing {temper-slug, content_hash} JSON to stdout
    //    — the show-edit-cat workflow contract (per CLAUDE.md).
    let slug_display = output
        .value
        .slug
        .clone()
        .unwrap_or_else(|| output.value.id.to_string());
    let hash_display = output.value.body_hash.as_deref().unwrap_or("").to_string();
    println!(
        "{}",
        serde_json::json!({
            "temper-slug": slug_display,
            "content_hash": hash_display,
        })
    );

    Ok(())
}
```

This deletes the `has_vault_file_event`-gated branch (the `Event::ResourceUpdate` discovery event lived only in that local-mode branch — it is dropped; discovery telemetry for updates was never emitted in cloud mode, so this preserves cloud parity). The `serde_json::json!` here is pre-existing inline JSON kept verbatim — not in scope to restructure.

- [ ] **Step 4: Run the test, verify it passes**

Run: `cargo nextest run --manifest-path tests/e2e/Cargo.toml --features test-db -E 'test(update_rewrites_projection_file_on_success)'`
Expected: PASS.

Run: `cargo make check`
Expected: green.

- [ ] **Step 5: Commit**

```bash
git add crates/temper-cli/src/commands/resource.rs tests/e2e
git commit -m "cloud-only(ch3): update rewrites the projection file; cloud-only rendering"
```

---

## Task 7: `delete` removes the projection file; drop the local-mode prompt gate

**Files:**
- Modify: `crates/temper-cli/src/commands/resource.rs` (`delete`, lines 742–823)
- Test: `tests/e2e/tests/cloud_writes_test.rs`

`delete` currently has a local-mode-only `[y/N]` confirmation gate and an event loop that handles `VaultFileRemoved` / `VaultManifestUpdated` (local-only events). Cloud mode never prompted; `CloudBackend::delete_resource` emits only `RemoteSynced`. Collapse the gate away and add the projection-remove tail.

- [ ] **Step 1: Write the failing e2e test**

In the e2e crate, add:

```rust
#[tokio::test]
async fn delete_removes_the_projection_file() {
    // Harness: create a resource (projection file now exists), then
    // `temper resource delete --force`. Assert the projection file is gone.
}
```

- [ ] **Step 2: Run the test, verify it fails**

Run: `cargo nextest run --manifest-path tests/e2e/Cargo.toml --features test-db -E 'test(delete_removes_the_projection_file)'`
Expected: FAIL — projection file still on disk after delete.

- [ ] **Step 3: Rewrite `delete`**

Replace the whole `delete` function (lines 732–823) with:

```rust
/// Delete a resource.
///
/// temper is cloud-only: the server-side soft-delete is the operation;
/// the projection file is removed afterward as a best-effort tail. The
/// API failure surfaces as an error before any local mutation.
///
/// `force` is accepted for CLI-surface compatibility but is not consulted
/// — a cloud delete is non-interactive (there is no local-file removal to
/// confirm; the projection file is derivative).
pub fn delete(
    config: &Config,
    doc_type: &str,
    slug: &str,
    context: Option<&str>,
    force: bool,
) -> Result<()> {
    use temper_core::operations::{DeleteResource, DomainEvent, ResourceRef};

    let _ = temper_core::frontmatter::DocType::from_str(doc_type)?;

    let ctx = require_context(context)?;
    let owner = config.owner_for_context(&ctx);

    let cmd = DeleteResource {
        resource: ResourceRef::scoped(&owner, &ctx, doc_type, slug),
        force,
        origin: temper_core::operations::Surface::CliCloud,
    };

    let (runtime, backend, _client) = crate::backend_select::build_backend(config, &ctx)?;
    let output = runtime.block_on(backend.delete_resource(cmd))?;

    // Projection refresh: remove the resource's projection file. Best-effort
    // — a removal failure must not fail the (already-committed) delete.
    if let Err(e) =
        crate::projection::remove_resource_file(&config.vault_root, &owner, &ctx, doc_type, slug)
    {
        output::warning(format!("could not remove projection file: {e}"));
    }

    // CloudBackend emits exactly one `RemoteSynced` event on success.
    for event in &output.events {
        if let DomainEvent::RemoteSynced { .. } = event {
            self::output::success(format!("Deleted {doc_type}/{slug}"));
        }
    }

    Ok(())
}
```

Notes:
- This task changes `require_context` to a one-argument call (`require_context(context)?`). **Task 9 makes that signature change.** To keep this task's commit green, either (a) sequence Task 9 before Task 7, or (b) in this task call the current two-arg form `require_context(config, context)?` and let Task 9 update it. **Pick (a): do Task 9 before Task 7.** The task list below is already in dependency order — see "Execution order" at the end.
- `force` is now unused-as-logic but kept in the signature (the clap layer passes it). Bind it into the `DeleteResource` cmd as shown so it is not an unused parameter.
- The `VaultFileRemoved` / `VaultManifestUpdated` match arms are dropped — `CloudBackend` never emits them.

- [ ] **Step 4: Run the test, verify it passes**

Run: `cargo nextest run --manifest-path tests/e2e/Cargo.toml --features test-db -E 'test(delete_removes_the_projection_file)'`
Expected: PASS.

Run: `cargo make check`
Expected: green.

- [ ] **Step 5: Commit**

```bash
git add crates/temper-cli/src/commands/resource.rs tests/e2e
git commit -m "cloud-only(ch3): delete removes the projection file; drop local-mode prompt"
```

---

## Task 8: `list` — drop the local-scan fallback

**Files:**
- Modify: `crates/temper-cli/src/commands/resource.rs` (`list`, lines 629–730)

`list` currently falls back to a local vault scan (`render_list`) when the server is unreachable *in local mode*. Cloud is the only mode — server errors surface as-is. `render_list` / `scan_rows` etc. are `pub` and still used by the surviving `vault_backend/`; **do not delete them** — just stop `list` calling the fallback.

- [ ] **Step 1: Rewrite the body of `list`**

In `commands/resource.rs::list`, the function starts at line 630 with `use crate::actions::runtime;` and `use temper_core::types::config::VaultState;`. Replace from that `use` block through line 721 (the `let body = match server_rows { ... };` block) with:

```rust
    use crate::actions::runtime;

    // Hints for filters that only apply to certain types (unchanged).
    if params.stage.is_some() && params.doc_type != "task" {
        output::hint(format!(
            "--stage filter is only meaningful for tasks; ignored for {}.",
            params.doc_type
        ));
    }
    if params.goal.is_some() && params.doc_type != "task" {
        output::hint(format!(
            "--goal filter is only meaningful for tasks; ignored for {}.",
            params.doc_type
        ));
    }
    if params.status.is_some() && params.doc_type != "goal" {
        output::hint(format!(
            "--status filter is only meaningful for goals; ignored for {}.",
            params.doc_type
        ));
    }

    if let Some(s) = params.stage {
        if params.doc_type == "task" {
            vault::validate_stage(s)?;
        }
    }

    let format = OutputFormat::parse(params.format);
    let doc_type = params.doc_type.to_string();
    let context = params.context.map(ToString::to_string);
    let limit = params.limit.unwrap_or(20);
    let state_dir = config.state_dir.clone();

    // Cloud-only list: a non-blocking staleness pre-flight, then the
    // server query. Any error (network, auth, 4xx/5xx) surfaces as-is —
    // there is no local-scan fallback.
    let rows = runtime::with_client(move |client| {
        Box::pin(async move {
            if let Some(ctx) = context.as_deref() {
                crate::projection::warn_if_context_stale(client, &state_dir, ctx).await;
            }
            fetch_list_rows(client, &doc_type, context.as_deref(), limit).await
        })
    })?;

    let body = render_server_rows(params.doc_type, &rows, format)?;
```

The remaining tail of `list` (the `if body.trim().is_empty()` hint + `output::plain`) is unchanged.

- [ ] **Step 2: Verify**

Run: `cargo make check`
Expected: green. (`render_list`, `RenderListParams`, `ListFilters`, `scan_rows` are still `pub` / still used by `vault_backend/` — no dead-code warning.)

Run: `cargo nextest run -p temper-cli`
Expected: green.

- [ ] **Step 3: Commit**

```bash
git add crates/temper-cli/src/commands/resource.rs
git commit -m "cloud-only(ch3): list is cloud-only — drop the local-scan fallback"
```

---

## Task 9: `require_context` — trust the supplied name

**Files:**
- Modify: `crates/temper-cli/src/commands/resource.rs` (`require_context`, lines 20–34; call sites in `create`, `update`)

`require_context` currently does a vault-filesystem fallback in local mode; in cloud mode it trusts the name. Collapse to the cloud behavior and drop the now-unused `config` parameter.

- [ ] **Step 1: Rewrite `require_context`**

Replace lines 15–34 (the doc comment and function) with:

```rust
/// Require a context, returning an error if none specified.
///
/// temper is cloud-only: there are no context directories on disk to
/// check, so a supplied name is trusted directly.
fn require_context(context: Option<&str>) -> Result<String> {
    match context {
        Some(ctx) => Ok(ctx.to_string()),
        None => Err(TemperError::Project(
            "no context specified — use --context <name>".into(),
        )),
    }
}
```

- [ ] **Step 2: Update call sites**

Run `rg -n 'require_context\(' crates/temper-cli/src`. Each call is `require_context(config, context)` — change to `require_context(context)`. Known sites: `create` (~line 223), `update` (~line 1453). (Task 7's `delete` already uses the one-arg form.)

- [ ] **Step 3: Handle `resolve_context_with_fallback`**

`require_context` was the caller of `super::resolve_context_with_fallback`. Run `rg -n 'resolve_context_with_fallback' crates/temper-cli/src`. If it now has zero non-test callers AND it is `pub(crate)`/private, delete it (and a focused unit test for it, if any). If it is `pub` or still has another caller, leave it.

- [ ] **Step 4: Verify**

Run: `cargo make check`
Expected: green.

- [ ] **Step 5: Commit**

```bash
git add crates/temper-cli/src/commands/resource.rs crates/temper-cli/src/commands/mod.rs
git commit -m "cloud-only(ch3): require_context trusts the supplied name"
```

---

## Task 10: `show_generic` — collapse to the cloud branch

**Files:**
- Modify: `crates/temper-cli/src/commands/resource.rs` (`show_generic`, lines 1043–1194)

`show_generic` matches `VaultState`: the `Cloud` arm resolves + fetches + does a best-effort projection refresh; the `Local` arm runs the three-tier `show_cache` ladder. Keep the `Cloud` arm only.

- [ ] **Step 1: Rewrite `show_generic`**

Replace the whole function (lines 1043–1194) with:

```rust
/// Show a generic resource (goal, research, concept, decision).
///
/// Cloud-only: resolves the id via `resolve_by_uri`, fetches content,
/// renders it, and writes the canonical projection file (per-resource
/// refresh — best-effort).
fn show_generic(
    config: &Config,
    doc_type: &str,
    slug: &str,
    context: Option<&str>,
    format: &str,
) -> Result<()> {
    use crate::actions::runtime;

    let doc_type_s = doc_type.to_string();
    let slug_s = slug.to_string();
    let context_owned = context.map(str::to_string);
    let format_s = format.to_string();

    let config_clone = config.clone();
    let doc_type_inner = doc_type_s.clone();
    let slug_inner = slug_s.clone();
    let ctx_inner = context_owned.clone();

    let body = runtime::with_client(|client| {
        Box::pin(async move {
            let ctx = ctx_inner
                .as_deref()
                .ok_or_else(|| {
                    TemperError::Project("no context specified — use --context <name>".into())
                })?
                .to_string();
            let owner = config_clone.owner_for_context(&ctx);
            let row = client
                .resources()
                .resolve_by_uri(&owner, &ctx, &doc_type_inner, &slug_inner)
                .await
                .map_err(crate::actions::runtime::client_err_to_temper)?;
            let resp = client
                .resources()
                .content(*row.id.as_uuid())
                .await
                .map_err(crate::actions::runtime::client_err_to_temper)?;

            // Per-resource projection refresh: write the fetched resource
            // to its canonical projection path. Best-effort — a write
            // failure must not stop `show` from displaying.
            if let Err(e) = crate::projection::write_resource_file_from_parts(
                &config_clone.vault_root,
                &row,
                &resp,
            ) {
                crate::output::warning(format!(
                    "could not refresh projection file for '{slug_inner}': {e}"
                ));
            }

            Ok(resp.markdown)
        })
    })?;

    let ctx = context_owned.unwrap_or_default();
    render_generic_output(&doc_type_s, &slug_s, &ctx, config, None, body, &format_s)
}
```

- [ ] **Step 2: Delete `find_or_compute_local_path`**

`find_or_compute_local_path` was called only by `show_generic`'s now-removed `Local` branch. Run `rg -n 'find_or_compute_local_path' crates/temper-cli/src` — if zero non-test callers remain, delete the function (and any focused test of it). If `cargo make check` reports it still has a caller, that caller was missed — investigate before deleting.

Do **not** delete `show_via_api_fallback` or `resolve_id_local_first` here — `task::show` and `session::show` still call them. Task 12 deletes both.

- [ ] **Step 3: Verify**

Run: `cargo make check`
Expected: green.

Run: `cargo nextest run -p temper-cli`
Expected: green.

- [ ] **Step 4: Commit**

```bash
git add crates/temper-cli/src/commands/resource.rs
git commit -m "cloud-only(ch3): show_generic is cloud-only"
```

---

## Task 11: `task::show` — collapse to cloud + projection refresh

**Files:**
- Modify: `crates/temper-cli/src/commands/task.rs` (`show`, lines 18–123)

`task::show` matches `VaultState`. Keep the `Cloud` arm, and add the per-resource projection refresh that `show_generic`'s cloud arm already does (Chunk 2 wired the refresh into `show_generic` only — `task::show` is a separate function and should match).

- [ ] **Step 1: Rewrite `task::show`**

Replace the whole `show` function (lines 10–123) with:

```rust
/// Show a single task's content.
///
/// Cloud-only: resolves the task id via `resolve_by_uri`, fetches content,
/// prints it, and writes the canonical projection file (per-resource
/// refresh — best-effort).
pub fn show(
    config: &Config,
    slug_or_suffix: &str,
    context: Option<&str>,
    _format: &str,
) -> Result<()> {
    use crate::actions::runtime;

    let context_s = context.map(str::to_string);
    let slug_s = slug_or_suffix.to_string();
    let config_clone = config.clone();

    let body = runtime::with_client(|client| {
        Box::pin(async move {
            let ctx = context_s.as_deref().ok_or_else(|| {
                TemperError::Project("no context specified — use --context <name>".into())
            })?;
            let owner = config_clone.owner_for_context(ctx);
            let row = client
                .resources()
                .resolve_by_uri(&owner, ctx, "task", &slug_s)
                .await
                .map_err(crate::actions::runtime::client_err_to_temper)?;
            let resp = client
                .resources()
                .content(*row.id.as_uuid())
                .await
                .map_err(crate::actions::runtime::client_err_to_temper)?;

            // Per-resource projection refresh — best-effort.
            if let Err(e) = crate::projection::write_resource_file_from_parts(
                &config_clone.vault_root,
                &row,
                &resp,
            ) {
                crate::output::warning(format!(
                    "could not refresh projection file for '{slug_s}': {e}"
                ));
            }

            Ok(resp.markdown)
        })
    })?;

    print!("{body}");
    Ok(())
}
```

Notes:
- The `format` parameter is now unused (the old `Local` arm emitted a `TaskInfo` JSON struct for `--format json`; the cloud arm always printed markdown — preserve the cloud behavior). Bind it `_format` to keep the clap-layer call site unchanged. Leave a one-line comment: `// `format` is unused — cloud `show` always prints the markdown body.`
- The unused `pub use ... TaskInfo` / `Vault` imports at the top of `task.rs` may now be dead. If `cargo make check` flags an unused import, remove it. Keep `find_task` / `load_tasks` / `next_seq` re-exports if `cargo make check` does not flag them (they are `pub use` — likely still used by other modules).

- [ ] **Step 2: Verify**

Run: `cargo make check`
Expected: green.

Run: `cargo nextest run -p temper-cli`
Expected: green.

- [ ] **Step 3: Commit**

```bash
git add crates/temper-cli/src/commands/task.rs
git commit -m "cloud-only(ch3): task show is cloud-only with projection refresh"
```

---

## Task 12: `session.rs` — collapse `session_exists`, `show`, `save`; delete orphaned helpers

**Files:**
- Modify: `crates/temper-cli/src/commands/session.rs` (`session_exists` lines 178–205, `show` lines 286–492, `save`'s `VaultState` match)
- Modify: `crates/temper-cli/src/commands/resource.rs` (delete `has_vault_file_event`, `vault_file_path_from_events`, `resolve_id_local_first`, `show_via_api_fallback`)

This task removes the last `VaultState` references in the command layer and deletes the helpers that those branches kept alive.

- [ ] **Step 1: Collapse `session_exists`**

Replace `session_exists` (lines 173–205) with:

```rust
/// Check whether a session for the current day already exists.
///
/// Cloud-only: queries the API via `resolve_by_uri`. Any error (404 or
/// network) is treated as "doesn't exist".
fn session_exists(_config: &Config, context: &str, owner: &str, slug: &str) -> Result<bool> {
    let owner = owner.to_string();
    let context = context.to_string();
    let slug = slug.to_string();
    match crate::actions::runtime::with_client(|client| {
        Box::pin(async move {
            client
                .resources()
                .resolve_by_uri(&owner, &context, "session", &slug)
                .await
                .map_err(crate::actions::runtime::client_err_to_temper)
        })
    }) {
        Ok(_) => Ok(true),
        Err(_) => Ok(false),
    }
}
```

(`config` becomes unused — bind `_config` to keep call sites unchanged.)

- [ ] **Step 2: Collapse `session::show`**

`session::show` (lines 286–492) matches `VaultState`. Replace the whole function body from the `let vault_state = VaultState::from_env();` line through the closing brace of the `match` with the `Cloud`-arm logic only, keeping the `SessionShow` struct definition and the `runtime`/`Duration` imports trimmed to what remains. The resulting function:

```rust
pub fn show(
    config: &Config,
    slug_or_suffix: &str,
    context: Option<&str>,
    format: &str,
) -> Result<()> {
    use crate::actions::runtime;

    #[derive(Serialize)]
    struct SessionShow {
        date: String,
        context: String,
        title: String,
        path: String,
        content: String,
    }

    let ctx_s = context.map(str::to_string);
    let slug_s = slug_or_suffix.to_string();
    let config_clone = config.clone();

    let body = runtime::with_client(|client| {
        Box::pin(async move {
            let ctx = ctx_s.as_deref().ok_or_else(|| {
                crate::error::TemperError::Project(
                    "no context specified — use --context <name>".into(),
                )
            })?;
            let owner = config_clone.owner_for_context(ctx);
            let row = client
                .resources()
                .resolve_by_uri(&owner, ctx, "session", &slug_s)
                .await
                .map_err(crate::actions::runtime::client_err_to_temper)?;
            let resp = client
                .resources()
                .content(*row.id.as_uuid())
                .await
                .map_err(crate::actions::runtime::client_err_to_temper)?;

            // Per-resource projection refresh — best-effort.
            if let Err(e) = crate::projection::write_resource_file_from_parts(
                &config_clone.vault_root,
                &row,
                &resp,
            ) {
                crate::output::warning(format!(
                    "could not refresh projection file for '{slug_s}': {e}"
                ));
            }

            Ok(resp.markdown)
        })
    })?;

    if format == "json" {
        let ctx = context.unwrap_or("");
        let info = SessionShow {
            date: String::new(),
            context: ctx.to_string(),
            title: slug_or_suffix.to_string(),
            path: String::new(),
            content: body,
        };
        let json = serde_json::to_string_pretty(&info).unwrap_or_default();
        println!("{json}");
        return Ok(());
    }

    print!("{body}");
    Ok(())
}
```

This also adds the per-resource projection refresh to the cloud path (the old cloud arm did not write it — bring it in line with `show_generic`/`task::show`).

The session-`show` unit tests (`session.rs` `mod tests`, lines 533+) are local-vault fixture tests (`write_session` writes files to disk; `show` reads them). After this collapse `show` no longer reads disk — these tests now exercise the cloud path with no server and will fail. **Delete the local-fixture `show` tests** (`show_exact_slug_match`, `show_partial_slug_match`, `show_not_found_falls_back_to_api`, `show_returns_most_recent_when_multiple_match`, `show_scans_all_contexts_when_none_specified`, `show_wrong_context_returns_error`) and their helpers (`write_session`, `test_vault`, `isolate_env`, `SessionEntry`-only-test usages) if those helpers become unused. Cloud-path `session::show` is covered by the e2e suite. If a helper (`parse_date_from_file`, `extract_date_from_stem`, `SessionEntry`) is still used by non-test code, leave it; if it is now orphaned and `pub(crate)`/private, delete it (compiler-driven — `cargo make check` names them).

- [ ] **Step 3: Collapse the `VaultState` match in `session::save`**

`save` matches `VaultState` (the `Local` arm at ~line 308, `Cloud` at ~line 447) and uses `has_vault_file_event`. Read `save` in full, then collapse the match to the `Cloud` arm and remove the `has_vault_file_event(&output.events)` check (cloud is the only mode — render the cloud shape unconditionally, exactly as `create`/`update` were collapsed). Keep the `save` JSON/output shape that the cloud arm produced. Remove the `use temper_core::types::VaultState;` import.

- [ ] **Step 4: Delete the now-orphaned `pub(crate)` helpers**

In `commands/resource.rs`, delete:
- `has_vault_file_event` (lines 45–51)
- `vault_file_path_from_events` (lines 53–63)
- `resolve_id_local_first`
- `show_via_api_fallback`

For each: run `rg -n '<name>' crates/temper-cli/src` first to confirm zero remaining non-test callers. If `cargo make check` reports a remaining caller, investigate — do not force the deletion.

- [ ] **Step 5: Verify**

Run: `rg -n 'VaultState' crates/temper-cli/src`
Expected: no matches (all command-layer references are now gone; only `temper-core` still defines the enum, removed in Task 15).

Run: `cargo make check`
Expected: green.

Run: `cargo nextest run -p temper-cli`
Expected: green.

- [ ] **Step 6: Commit**

```bash
git add crates/temper-cli/src/commands/session.rs crates/temper-cli/src/commands/resource.rs
git commit -m "cloud-only(ch3): session commands cloud-only; drop orphaned local-mode helpers"
```

---

## Task 13: Re-key the token-store axis off `TEMPER_TOKEN` presence

**Files:**
- Modify: `crates/temper-cli/src/actions/runtime.rs` (`resolve_token_store`, lines 79–105)
- Modify: `crates/temper-cli/src/commands/auth.rs` (`export_token`, lines 189–219)

`VaultState` conflated two axes: *where the vault lives* (removed) and *where the auth token comes from* — disk (`DiskTokenStore`, a developer laptop after `temper auth login`) vs env (`MemoryTokenStore`, an ephemeral cloud agent fed `TEMPER_TOKEN`). The token-store axis is real and must survive; re-key it off `TEMPER_TOKEN` presence.

- [ ] **Step 1: Rewrite `resolve_token_store`**

Replace `resolve_token_store` (lines 79–105) with:

```rust
/// Resolve the active [`TokenStore`] for this process.
///
/// A cloud agent session is handed its token via the `TEMPER_TOKEN` env
/// var — when that is set, use a [`MemoryTokenStore`]. Otherwise this is
/// an interactive developer machine: read the token from disk via
/// [`DiskTokenStore`], honoring the `TEMPER_AUTH_PATH` / `auth.path`
/// precedence so tests can isolate from `~/.config/temper/auth.json`.
fn resolve_token_store(config: &TemperConfig) -> Result<Arc<dyn TokenStore>> {
    if std::env::var("TEMPER_TOKEN").ok().filter(|v| !v.is_empty()).is_some() {
        let mem = MemoryTokenStore::from_env_required()
            .map_err(|e| TemperError::Config(e.to_string()))?;
        // The env-supplied AT is refresh-less by design (see
        // `stored_auth_from_env`). Warn early when it is near expiry so
        // the user has time to re-export.
        if let Ok(Some(stored)) = mem.load() {
            if let Some(msg) =
                token_expiry_warning(&stored, chrono::Utc::now(), chrono::Duration::hours(1))
            {
                eprintln!("{msg}");
            }
        }
        Ok(Arc::new(mem))
    } else {
        Ok(Arc::new(DiskTokenStore::at(auth_path(config))))
    }
}
```

- [ ] **Step 2: Rewrite the `export_token` guard**

`export_token` reads from the on-disk grant; it must refuse when there is no disk grant — i.e. when running as a cloud agent (`TEMPER_TOKEN` set). Replace the guard (lines 190–200) — the current `use temper_core::types::VaultState;` and the `if matches!(VaultState::from_env(), VaultState::Cloud) { ... }` block — with:

```rust
    // `export-token` reads from the on-disk `DiskTokenStore` grant. A
    // cloud agent session (`TEMPER_TOKEN` set) has no disk grant to
    // export — refuse with a directive to run this on the laptop.
    if std::env::var("TEMPER_TOKEN").ok().filter(|v| !v.is_empty()).is_some() {
        return Err(crate::error::TemperError::Config(
            "temper auth export-token reads the on-disk grant — this \
             session was handed its token via TEMPER_TOKEN and has \
             nothing to export. Run this on your laptop, paste the token \
             into the cloud session's secrets, and the agent reads \
             TEMPER_TOKEN."
                .into(),
        ));
    }
```

(Drop the `use temper_core::types::VaultState;` line.)

- [ ] **Step 3: Verify**

Run: `cargo make check`
Expected: green.

Run: `cargo nextest run -p temper-cli`
Expected: green.

- [ ] **Step 4: Commit**

```bash
git add crates/temper-cli/src/actions/runtime.rs crates/temper-cli/src/commands/auth.rs
git commit -m "cloud-only(ch3): token store keyed off TEMPER_TOKEN presence, not VaultState"
```

---

## Task 14: `temper sync run` — a cloud-only error

**Files:**
- Modify: `crates/temper-cli/src/commands/sync_cmd.rs` (`run`, lines 41–186)

`sync run`'s only `VaultState` reference is a guard that errors in cloud mode. Cloud is the only mode now — `run` permanently errors. (The whole `sync` command is removed in Chunk 5; this task only severs the `VaultState` dependency.)

- [ ] **Step 1: Rewrite `run`**

Replace `run` (lines 41–186) with:

```rust
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

- [ ] **Step 2: Delete `warn_blocked_paths` if orphaned**

`warn_blocked_paths` (lines 18–39) was called only by the old `run` body. Run `rg -n 'warn_blocked_paths' crates/temper-cli/src`. If `status`/`refresh`/`reset` do not call it and no other caller remains, delete it. Then run `cargo make check` — if it reports any now-unused imports at the top of `sync_cmd.rs` (e.g. `TerminalProgress`, `sync as sync_actions`, `runtime`), remove only the ones it flags; the ones still used by `status`/`refresh`/`reset` stay.

- [ ] **Step 3: Verify**

Run: `cargo make check`
Expected: green.

- [ ] **Step 4: Commit**

```bash
git add crates/temper-cli/src/commands/sync_cmd.rs
git commit -m "cloud-only(ch3): temper sync run is a cloud-only error"
```

---

## Task 15: Remove `VaultState` from `temper-core`

**Files:**
- Modify: `crates/temper-core/src/types/config.rs` (delete `VaultState`, `from_env`, `is_cloud`, `TEMPER_VAULT_STATE_ENV`, tests)
- Modify: `crates/temper-core/src/types/mod.rs` (drop the `VaultState` re-export)

All consumers are rewritten. Remove the enum itself.

- [ ] **Step 1: Confirm zero references**

Run: `rg -n 'VaultState|TEMPER_VAULT_STATE' --type rust`
Expected: matches **only** in `crates/temper-core/src/types/config.rs` (the definition + tests) and `crates/temper-core/src/types/mod.rs` (the re-export). If anything else appears (outside `#[cfg(test)]` env-scrubbing lists, which are harmless string literals), stop and rewrite that consumer before proceeding.

- [ ] **Step 2: Delete the definition**

In `config.rs`, delete:
- The `TEMPER_VAULT_STATE_ENV` const and its doc comment (lines 4–6).
- The `VaultState` enum, its doc comment, and the entire `impl VaultState { ... }` block (`from_env`, `is_cloud`) — lines 16–60.
- Every `VaultState`-related test in the `#[cfg(test)] mod tests` block (the `assert_eq!(VaultState::default(), ...)` test, the serde tests, and all `from_env` tests — lines ~565–630). Leave the non-`VaultState` tests in that module intact.

- [ ] **Step 3: Drop the re-export**

In `crates/temper-core/src/types/mod.rs`, remove `VaultState` from the `pub use config::{... VaultState, ...}` re-export list (line ~51). Leave the other re-exported names.

- [ ] **Step 4: Verify**

Run: `rg -n 'VaultState|TEMPER_VAULT_STATE' --type rust`
Expected: no matches anywhere (env-scrubbing test literals like `"TEMPER_VAULT_STATE"` in `remove_var` lists are acceptable to leave — they are harmless; remove them too if trivially co-located, otherwise leave for the Chunk-3 test sweep).

Run: `cargo make check`
Expected: green.

Run: `cargo nextest run --workspace`
Expected: green.

- [ ] **Step 5: Commit**

```bash
git add crates/temper-core/src/types
git commit -m "cloud-only(ch3): remove the VaultState enum and TEMPER_VAULT_STATE"
```

---

## Task 16: Workspace + e2e verification

**Files:** none (verification only)

Per `feedback_workspace_test_surfaces_pipeline_bugs`: `cargo nextest run --workspace` activates feature-unified code paths that per-crate runs miss. The embed-gated e2e tier is CI's only ONNX-equipped job.

- [ ] **Step 1: Full workspace check**

Run: `cargo make check`
Expected: green (clippy `-D warnings`, fmt, docs, machete).

- [ ] **Step 2: Full workspace test**

Run: `cargo nextest run --workspace`
Expected: green.

- [ ] **Step 3: e2e suite (test-db)**

Run: `cargo make test-e2e`
Expected: green — including the three new projection-write tests from Tasks 5–7.

- [ ] **Step 4: e2e embed-gated tier**

Run: `cargo nextest run --manifest-path tests/e2e/Cargo.toml --features test-db,test-embed`
Expected: green (matches CI's "Embed & MCP Round-Trip Tests" job).

- [ ] **Step 5: Confirm no stale references**

Run: `rg -n 'VaultState|TEMPER_VAULT_STATE|surface_for_state' --type rust`
Expected: no matches.

If every step is green, Chunk 3 is complete. Do **not** open a PR — PR B accumulates Chunks 3–8 on the `jct/cloud-only-vault-pr-b` branch; the PR opens after Chunk 8.

---

## Execution order

The tasks are dependency-ordered as numbered, with one exception already noted: **Task 9 (`require_context`) must run before Task 7 (`delete`)**, because Task 7's `delete` rewrite calls the one-argument `require_context`. Recommended sequence:

1, 2, 3, **T-TM** (test migration — see below), 4, 8, 9, 5, 6, 7, 10, 11, 12, 13, 14, 15, 16.

(Tasks 5/6/7 — create/update/delete — come after 9 so the one-arg `require_context` exists; 8 and 9 are independent and can precede them. Task 15 is strictly last before verification.)

Each task ends with a green `cargo make check` and a commit, so the branch is bisectable throughout.

## Task T-TM (inserted after Task 3): Delete local-mode CLI integration tests

**Why:** Task 1's auth-required `build_backend` breaks ~29 CLI integration tests in `crates/temper-cli/tests/` whose shared `common::create_goal`/`create_task` helpers create resources through the now-cloud-only command path (no server in unit tests). The spec anticipated this ("~95% of tests exercise local mode… test migration is folded into each chunk"). This task does the bulk deletion up front so the subsequent code tasks run against a green suite.

**Files to delete (8 files; each justified):**

| File | Disposition | Replacement / Justification |
|---|---|---|
| `crates/temper-cli/tests/actions_goal_test.rs` | DELETE | Local-vault scan helpers (`load_goals`/`find_goal`/`next_seq`); cloud equivalent is `client.resources().list()`, covered by `tests/e2e/tests/resource_crud_test.rs`. |
| `crates/temper-cli/tests/actions_task_test.rs` | DELETE | Same justification for tasks. |
| `crates/temper-cli/tests/discovery_test.rs` | DELETE | Local `.temper/events.jsonl` telemetry; tests build resources via `create_goal`. Discovery's role in cloud-only is doctor/status territory (Chunk 7). |
| `crates/temper-cli/tests/resource_create_discovery_test.rs` | DELETE | Telemetry side-effect of create; underlying create behavior covered by `tests/e2e/tests/cloud_writes_test.rs` + Task 5's new `create_writes_canonical_projection_file` e2e. |
| `crates/temper-cli/tests/resource_body_update_test.rs` | DELETE | All tests are `local_mode_update_*` — local-mode update behavior is being removed. Cloud-mode update covered by Task 6's new `update_rewrites_projection_file_on_success` e2e. |
| `crates/temper-cli/tests/session_test.rs` | DELETE | Local-mode session save/list/show. Session save is cloud-ified in Task 12; e2e coverage at `cloud_writes_test.rs`. |
| `crates/temper-cli/tests/session_task_test.rs` | DELETE | Session→task linking writes through the vault file; needs cloud redesign — flag as follow-on. |
| `crates/temper-cli/tests/warmup_test.rs` | DELETE | `temper warmup` is a local-scan UX command; needs cloud-ification — flag as follow-on. |

**Steps:**

- [ ] **Step 1:** `git rm` each of the 8 files above.
- [ ] **Step 2:** If a now-unused helper in `crates/temper-cli/tests/common/mod.rs` (e.g. `create_goal`/`create_task`/`init_isolated_auth`) becomes orphaned, leave it for now — other CLI test files (`research_test`, `resource_delete_test`, `context_fallback_test`, etc.) may still use it. Only delete a helper if `cargo nextest run -p temper-cli` reports it dead (`#[allow(dead_code)]` suppresses the warning today, so most likely nothing breaks).
- [ ] **Step 3:** Create vault follow-on tasks via `temper resource create --type task --context temper --mode build --effort medium`:
  - `cloud-mode-session-task-linking` — short body: "Re-implement session-save's `link_session_to_task` for cloud-only (currently rewrites the task vault file directly). Likely needs an API affordance to append `sessions` to a task's frontmatter list, plus optional stage update. Out of scope for Chunk 3 (CLI integration tests deleted in Chunk 3 left this behavior uncovered)."
  - `cloud-ified-temper-warmup` — short body: "`temper warmup` currently scans the local vault for in-progress tasks. Re-implement against `client.resources().list({stage: in-progress})`. Out of scope for Chunk 3."
- [ ] **Step 4: Verify.** Run `cargo nextest run -p temper-cli` — expect a green suite (the 29 failures are gone; no NEW failures introduced — if any appear, investigate before committing).
- [ ] **Step 5: Verify** `cargo make check` — expect green.
- [ ] **Step 6: Commit.**

```bash
git add -A
git commit -m "cloud-only(ch3): delete local-mode CLI integration tests (covered at e2e or behavior removed)"
```

**Out-of-scope test files** (left untouched in this task; later chunks/tasks will handle them):
- `graph_build_test.rs`, `graph_index_integration.rs` — Chunk 6 removes graph-build.
- `init_test`, `doctor_test`, `doctor_fix_integration_test`, `status_test` — Chunk 7 reworks these commands.
- `check_test`, `config_test`, `ids_test`, `skill_test`, `vault_test` — pure helpers; unaffected.
- `research_test`, `resource_delete_test`, `context_fallback_test` — may break under later Chunk 3 tasks; address inline if they do.

---

## Self-Review

**Spec coverage** (against `2026-05-21-cloud-only-vault-deprecation-design.md`, Chunk 3 = "`backend_select` returns `CloudBackend` unconditionally. Remove `VaultState`, `from_env`, `TEMPER_VAULT_STATE`, `surface_for_state`, `Surface::CliLocalVault`. `create`/`update`/`delete` rewrite the affected projection file on success."):

- `backend_select` unconditional `CloudBackend` → Task 1. ✓
- Remove `VaultState` / `from_env` / `TEMPER_VAULT_STATE` → Task 15 (consumers rewired in Tasks 1–14). ✓
- Remove `surface_for_state` → Task 3. ✓
- Remove `Surface::CliLocalVault` → **deliberately deferred to Chunk 4.** `vault_backend/` survives Chunk 3 (it is `pub`-exported, so no dead-code warning) and still constructs `Surface::CliLocalVault`; removing the variant now would break the compile. This is a spec-decomposition adjustment, called out in the task's vault entry and here. The variant is removed in Chunk 4 alongside `vault_backend/`.
- `create`/`update`/`delete` rewrite the projection file on success → Tasks 5, 6, 7. ✓
- Validation "full CRUD works cloud-only; no `TEMPER_VAULT_STATE` references remain" → Task 16. ✓

**Placeholder scan:** The compiler-driven deletion steps (Tasks 9, 10, 12, 14 — "if `rg` shows zero callers, delete; if `cargo make check` reports a caller, investigate") are *not* vague placeholders — they are bounded instructions with an exact verification gate, the correct technique for a removal-heavy refactor where transitively-dead private helpers cannot be enumerated without compiling. Every step that *writes new code* (Tasks 1, 2, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15) contains the complete code.

**Type consistency:** `build_backend` returns `(Runtime, Box<dyn Backend>, Arc<TemperClient>)` after Task 2; Tasks 5/6/7 all destructure `(runtime, backend, client)` / `(runtime, backend, _client)` consistently. `projection::write_resource_file(client, vault_root, &row)` takes `&temper_core::types::resource::ResourceRow`, which is exactly the type of `CommandOutput::value` from `create_resource`/`update_resource` (`wire_resource_to_resource_row` produces it). `projection::remove_resource_file(vault_root, owner, context, doc_type, slug)` (Task 4) matches its `delete` call site (Task 7). `require_context(context)` one-arg signature (Task 9) matches all three call sites.

**Decisions worth a reviewer's attention:**
- *Projection write is surface-side, not inside `CloudBackend`.* `CloudBackend` stays a pure API-translation layer; the projection refresh is a derivative read-model update, handled by the surface exactly as Chunk 2's `show` does. This keeps the "writes go through the backend, the projection is derivative" split clean and avoids putting file IO in `CloudBackend`. `build_backend` returning the client is the small enabling change.
- *Token-store axis re-keyed off `TEMPER_TOKEN`* (Task 13) — `VaultState` was overloaded; the disk-vs-env token decision is independent of the (now-removed) vault-location decision and must survive.
