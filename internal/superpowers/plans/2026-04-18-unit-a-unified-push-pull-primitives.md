# Unit A: Unified Push/Pull Primitives — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Factor the one-entry sync orchestration out of `actions/sync.rs` into `push_one_resource` / `pull_one_resource` primitives that accept `Option<&mut Manifest>`, expose `temper push <id|path>` and `temper pull <id>` as first-class CLI commands, and route the sync engine's body-tier push/pull through the same primitives — preserving all hashing, schema, and ownership invariants.

**Architecture:** Two new primitives live in `crates/temper-cli/src/actions/sync.rs` (the existing sync module, so helpers are shared cheaply). They replicate the current body-push / body-pull logic, but take `Option<&mut Manifest>`: `Some` updates the manifest entry as today; `None` performs a manifest-less write (snapshot to CWD for pull; POST-and-rewrite-frontmatter for push with provisional ids). Meta-only push/pull stays inside the sync engine — those tiers only exist because the sync diff surfaces them, and `temper push <path>` always sends body+meta together. `sync_orchestration`'s body branches delegate to the primitives; meta-only branches untouched. `commands/pull.rs` reduces to a thin wrapper; new `commands/push.rs` accepts `<id|path>` and routes through `push_one_resource`.

**Tech Stack:** Rust workspace, tokio async, reqwest via temper-client, cargo-nextest, sqlx against Docker Postgres for e2e. clap v4 for CLI arg parsing.

**Design reference:** `docs/superpowers/specs/2026-04-18-cloud-mode-and-portable-memory-design.md` (Unit A section).

**Branch:** `jct/temper-cloud-mode-portable-memory` (already checked out; working branch for all of Units A/B/C).

---

## Key Invariants (preserved from existing behavior)

1. **Hash invariants:** `body_hash`, `managed_hash`, `open_hash`, `remote_*_hash` recomputed post-write via `Frontmatter::parse_file(...).hashes()` for meta, `compute_body_hash(strip_frontmatter(...))` for body. Pre-write hashes are captured inside `ingest::build_ingest_payload`.
2. **Provisional → canonical rewrite:** If a POST returns a different UUID than the frontmatter's `temper-provisional-id`, the local file is rewritten to swap `temper-provisional-id: "<local>"` for `temper-id: "<server>"` (string replace, both quoted and unquoted forms), and the manifest entry (if present) is remapped from the local id to the server id with `provisional: false`.
3. **Ownership preflight:** `preflight_ownership_check` runs before any upload inside `sync_orchestration`. Not applicable to `temper push <path>` (no manifest to cross-check); add the check inline only when `manifest.is_some()` and the file resolves to a tracked entry.
4. **Schema validation:** Happens inside `ingest::build_ingest_payload` via `temper_core::normalize::validate`. Always applied. (Not a new code path — `build_ingest_payload` already does it.)
5. **Meta-only tier routing:** Unchanged. `push_resource_meta_only` / `pull_resource_meta_only` stay as-is inside the sync engine.

---

## File Structure

**Modified:**
- `crates/temper-cli/src/actions/sync.rs` — add `PushTarget`, `PushResult`, `PullResult`, `PullBranch`, `push_one_resource`, `pull_one_resource`. Slim `push_resource_body` and `pull_resource_body` to thin adapters over the primitives. Keep meta-only paths untouched.
- `crates/temper-cli/src/commands/pull.rs` — reduce `run()` to a thin wrapper that loads manifest (if present) and calls `pull_one_resource`.
- `crates/temper-cli/src/commands/mod.rs` — add `pub mod push;`.
- `crates/temper-cli/src/cli.rs` — add `Commands::Push { target: String }` variant.
- `crates/temper-cli/src/main.rs` — add `Commands::Push { target } => commands::push::run(&target)` match arm.

**Created:**
- `crates/temper-cli/src/commands/push.rs` — new `pub fn run(target: &str) -> Result<()>` that UUID-parses first, falls back to treating the arg as a path, loads manifest if present, calls `push_one_resource`.
- `tests/e2e/tests/push_command_test.rs` — e2e test for `temper push <path>` round-trip.
- `tests/e2e/tests/pull_command_test.rs` — e2e test for `temper pull <id>` both branches (manifest-tracked and snapshot-to-CWD).

---

## Task 1: Add primitive types + signatures (no behavior change)

**Purpose:** Land the type surface first so later tasks have stable targets. No runtime behavior yet — functions stubbed with `unimplemented!()` except where the compiler forces otherwise.

**Files:**
- Modify: `crates/temper-cli/src/actions/sync.rs:17-21` (add types near the other type re-exports, near top of file)

- [ ] **Step 1.1: Add the new types + function signatures to `actions/sync.rs`**

Insert directly after the existing `pub struct OwnershipMismatch { ... }` block (around line 34). Use the exact types — they're used unchanged in later tasks.

```rust
// ---------------------------------------------------------------------------
// Single-resource push/pull primitives
//
// These are the factored-out, per-resource orchestration that sync_orchestration
// batches over its diff sets. They also power `temper push <id|path>` and
// `temper pull <id>` as first-class commands.
//
// The `manifest: Option<&mut Manifest>` parameter is the mode switch:
// - Some(...) — local-vault mode, updates the manifest entry in place
// - None     — cloud mode / raw push, no manifest side effects
// ---------------------------------------------------------------------------

/// What a single push targets. `Path` reads frontmatter to locate the id;
/// `Id` requires a manifest to resolve the on-disk path.
#[derive(Debug)]
pub enum PushTarget<'a> {
    Path(&'a std::path::Path),
    Id(ResourceId),
}

/// Per-resource push outcome.
#[derive(Debug, Clone)]
pub struct PushResult {
    pub resource_id: ResourceId,
    pub path: std::path::PathBuf,
    pub kind: PushKind,
}

/// Which pull branch ran.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PullBranch {
    /// Wrote to the manifest-resolved vault path and updated the entry.
    ManifestTracked,
    /// Wrote to CWD as `{id}.md` (no manifest, or id not in manifest).
    Snapshot,
}

/// Per-resource pull outcome.
#[derive(Debug, Clone)]
pub struct PullResult {
    pub resource_id: ResourceId,
    pub path: std::path::PathBuf,
    pub branch: PullBranch,
}
```

- [ ] **Step 1.2: Compile**

Run: `cargo check -p temper-cli`
Expected: clean build. No new functions yet — just the types.

- [ ] **Step 1.3: Commit**

```bash
git add crates/temper-cli/src/actions/sync.rs
git commit -m "feat(cli): add PushTarget/PushResult/PullResult primitive types

Unit A scaffolding: types only, no behavior change. Later tasks add the
push_one_resource / pull_one_resource functions and wire them through
sync_orchestration + new commands.

Refs: docs/superpowers/specs/2026-04-18-cloud-mode-and-portable-memory-design.md"
```

---

## Task 2: Factor `pull_one_resource` out of the existing pull paths

**Purpose:** The current `commands/pull.rs::run` already bifurcates on manifest presence (the very shape the primitive needs). Move that logic into `actions/sync.rs::pull_one_resource` with `Option<&mut Manifest>`, preserving both branches exactly. `commands/pull.rs` becomes a thin wrapper. `pull_resource_body` inside `sync_orchestration` delegates here too — that's how the sync engine body-pull unifies.

The existing `sync_orchestration` pull-body uses `SyncPullItem` which carries `uri`, `resource_id`, and `content_hash`. `pull_one_resource` takes just `resource_id`; it re-derives `ctx`/`doc_type` from the server's `ResourceRow.context` + `doc_type` fields (available on every resource response) instead of parsing the `kb://` URI. This is a semantic improvement: the server is the authoritative source of those fields after a pull.

**Files:**
- Modify: `crates/temper-cli/src/actions/sync.rs` — add `pull_one_resource` (~line 1060, next to existing `pull_resource`)
- Modify: `crates/temper-cli/src/commands/pull.rs` — reduce to wrapper

- [ ] **Step 2.1: Write e2e test for the snapshot (manifest=None) branch first**

**File:** `tests/e2e/tests/pull_command_test.rs` (create)

Follow the pattern in `tests/e2e/tests/sync_test.rs:1-30` for setup. Test specifically that `pull_one_resource(client, vault_root, id, None)` writes `{id}.md` to `vault_root` (CWD for the test).

```rust
//! E2E: temper-cli `pull` command primitive.

use temper_cli::actions::sync::{pull_one_resource, PullBranch};

mod common;
use common::{e2e_client, seed_resource};

#[tokio::test]
async fn pull_one_resource_without_manifest_writes_snapshot_to_vault_root() {
    let ctx = e2e_test_context("pull-snapshot").await;
    let client = e2e_client(&ctx).await;

    // Seed a resource server-side.
    let resource = seed_resource(&ctx, "session", "pull-snapshot-test").await;

    let tmp = tempfile::tempdir().expect("tmpdir");
    let result = pull_one_resource(&client, tmp.path(), resource.id.into(), None)
        .await
        .expect("pull_one_resource");

    assert_eq!(result.branch, PullBranch::Snapshot);
    let expected_path = tmp.path().join(format!("{}.md", resource.id));
    assert_eq!(result.path, expected_path);
    assert!(expected_path.exists(), "snapshot file must exist");
    let content = std::fs::read_to_string(&expected_path).unwrap();
    assert!(content.contains(&resource.title), "body must include title from seed");
}
```

Also add a manifest-tracked variant:

```rust
#[tokio::test]
async fn pull_one_resource_with_manifest_writes_to_vault_path_and_updates_entry() {
    let ctx = e2e_test_context("pull-tracked").await;
    let client = e2e_client(&ctx).await;
    let resource = seed_resource(&ctx, "session", "pull-tracked-test").await;

    let tmp = tempfile::tempdir().unwrap();
    let temper_dir = tmp.path().join(".temper");
    std::fs::create_dir_all(&temper_dir).unwrap();
    let mut manifest = temper_core::types::Manifest::new("test-device".to_string());
    // Seed the manifest entry so ManifestTracked branch fires.
    let rel_path = format!("{}/test/session/pull-tracked-test.md", ctx.owner());
    manifest.entries.insert(
        resource.id.into(),
        temper_core::types::ManifestEntry {
            path: rel_path.clone(),
            body_hash: String::new(),
            remote_body_hash: String::new(),
            managed_hash: String::new(),
            open_hash: String::new(),
            remote_managed_hash: String::new(),
            remote_open_hash: String::new(),
            synced_at: chrono::Utc::now(),
            state: temper_core::types::ManifestEntryState::Clean,
            mtime_secs: None,
            last_audit_id: None,
            provisional: false,
        },
    );
    // Pre-create a stub file at the expected path so the ManifestTracked
    // branch doesn't fall back to the ADDED shape.
    let abs = tmp.path().join(&rel_path);
    std::fs::create_dir_all(abs.parent().unwrap()).unwrap();
    std::fs::write(&abs, "---\ntemper-id: 00000000-0000-0000-0000-000000000000\n---\n").unwrap();

    let result = pull_one_resource(&client, tmp.path(), resource.id.into(), Some(&mut manifest))
        .await
        .expect("pull_one_resource");

    assert_eq!(result.branch, PullBranch::ManifestTracked);
    assert_eq!(result.path, abs);
    let entry = manifest.entries.get(&resource.id.into()).unwrap();
    assert!(!entry.body_hash.is_empty(), "manifest body_hash must be populated post-pull");
    assert_eq!(entry.state, temper_core::types::ManifestEntryState::Clean);
}
```

If `common::` module doesn't yet expose the helpers used, reuse the same test-harness patterns already in `tests/e2e/tests/sync_test.rs`. Inspect that file first and adapt naming accordingly.

- [ ] **Step 2.2: Run the e2e test to verify failure**

Run: `cargo nextest run -p temper-e2e --features test-db pull_one_resource -E 'test(pull_one_resource)'`
Expected: FAIL — `pull_one_resource` is not defined yet.

- [ ] **Step 2.3: Implement `pull_one_resource` in `actions/sync.rs`**

Insert right below the existing `pull_resource` function (around line 1069 in `crates/temper-cli/src/actions/sync.rs`). The implementation is a direct adaptation of `commands/pull.rs::run` (lines 12-95) — with `client` passed in (not taken from `runtime::with_client`), manifest as `Option<&mut Manifest>`, id typed as `ResourceId`, and returning `PullResult` instead of calling `output::success`.

```rust
/// Pull a single resource from the server.
///
/// With `Some(manifest)` and a tracked entry, writes to the manifest-resolved
/// vault path and updates the entry (hashes, state=Clean, synced_at). With
/// `None` or an untracked id, writes a snapshot as `{id}.md` under
/// `vault_root` — the manifest-less branch preserves the ADDED behavior from
/// `commands/pull.rs`.
pub async fn pull_one_resource(
    client: &temper_client::TemperClient,
    vault_root: &Path,
    resource_id: ResourceId,
    manifest: Option<&mut Manifest>,
) -> Result<PullResult> {
    let id = Uuid::from(resource_id);

    let resource = client
        .resources()
        .get(id)
        .await
        .map_err(crate::commands::client_err)?;
    let content_response = client
        .resources()
        .content(id)
        .await
        .map_err(crate::commands::client_err)?;

    // Manifest-tracked branch: only when we have a manifest AND the id is in it.
    if let Some(manifest) = manifest {
        if let Some(entry) = manifest.entries.get_mut(&resource_id) {
            let vault_path = vault_root.join(&entry.path);
            if let Some(parent) = vault_path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            let (ctx, dtype) = match Vault::parse_rel(&entry.path) {
                Some(parsed) => (parsed.context.to_string(), parsed.doc_type.to_string()),
                None => ("default".to_string(), "resource".to_string()),
            };
            let managed_value = content_response
                .managed_meta
                .as_ref()
                .map(|m| serde_json::to_value(m).unwrap_or(serde_json::Value::Null));
            let fm = ingest::build_frontmatter_from_resource(
                &resource,
                &ctx,
                &dtype,
                ingest::normalize_body_for_vault(&content_response.markdown),
                managed_value.as_ref(),
                content_response.open_meta.as_ref(),
            )?;
            fm.write_to(&vault_path).map_err(|e| {
                TemperError::Vault(format!("pull write {}: {e}", vault_path.display()))
            })?;

            let content_hash = temper_core::hash::compute_body_hash(fm.body());
            entry.body_hash = content_hash.clone();
            entry.remote_body_hash = content_hash;
            entry.synced_at = chrono::Utc::now();
            entry.state = ManifestEntryState::Clean;

            return Ok(PullResult {
                resource_id,
                path: vault_path,
                branch: PullBranch::ManifestTracked,
            });
        }
    }

    // Snapshot branch: no manifest, or id not tracked. Write to vault_root as
    // {id}.md. This matches the ADDED branch in commands/pull.rs today.
    let filename = format!("{id}.md");
    let snapshot_path = vault_root.join(&filename);
    std::fs::write(&snapshot_path, &content_response.markdown)?;
    Ok(PullResult {
        resource_id,
        path: snapshot_path,
        branch: PullBranch::Snapshot,
    })
}
```

- [ ] **Step 2.4: Run the e2e tests to verify they pass**

Run: `cargo nextest run -p temper-e2e --features test-db -E 'test(pull_one_resource)'`
Expected: PASS (2 tests).

- [ ] **Step 2.5: Reduce `commands/pull.rs::run` to a thin wrapper**

**File:** `crates/temper-cli/src/commands/pull.rs` — replace entire contents:

```rust
//! `temper pull` — refresh a vault file from the cloud.

use uuid::Uuid;

use crate::actions::runtime;
use crate::actions::sync::{pull_one_resource, PullBranch};
use crate::error::TemperError;
use crate::output;
use temper_core::types::ResourceId;

pub fn run(resource_id: &str) -> crate::error::Result<()> {
    let id = Uuid::parse_str(resource_id)
        .map_err(|e| TemperError::NotFound(format!("Invalid UUID: {e}")))?;
    let resource_id_typed = ResourceId::from(id);

    runtime::with_client(|client| {
        Box::pin(async move {
            let vault_root = crate::config::resolve_vault(None)?;
            let temper_dir = vault_root.join(".temper");
            let device_id =
                crate::config::load_device_id().unwrap_or_else(|| "unknown".to_string());

            // Try to load a manifest; if missing, fall through to snapshot mode.
            let (mut manifest_opt, persist) = match crate::manifest_io::load_manifest(&temper_dir, &device_id) {
                Ok(m) => (Some(m), true),
                Err(_) => (None, false),
            };

            let result = pull_one_resource(
                client,
                &vault_root,
                resource_id_typed,
                manifest_opt.as_mut(),
            )
            .await?;

            // Fetch title for the user-facing message (cheap — single GET we already did, but pull_one_resource doesn't return it; re-fetch from server).
            let resource = client
                .resources()
                .get(id)
                .await
                .map_err(crate::commands::client_err)?;

            match result.branch {
                PullBranch::ManifestTracked => {
                    if persist {
                        if let Some(m) = &manifest_opt {
                            crate::manifest_io::save_manifest(&temper_dir, m)?;
                        }
                    }
                    output::success(format!(
                        "Pulled: \"{}\" -> {}",
                        resource.title,
                        result.path.display()
                    ));
                }
                PullBranch::Snapshot => {
                    output::success(format!(
                        "Pulled: \"{}\" -> {}",
                        resource.title,
                        result.path.display()
                    ));
                }
            }
            Ok(())
        })
    })
}
```

**Note:** The extra GET for `resource.title` is not free — but `pull_one_resource` intentionally does not return `ResourceRow` to keep its signature minimal. If the extra round-trip is a concern during code review, an optional `PullResult.title: String` field can be added later. For this task, correctness + thin-wrapper shape matter more.

- [ ] **Step 2.6: Run existing sync tests to confirm no regression**

Run: `cargo nextest run -p temper-e2e --features test-db sync`
Expected: all existing sync_test tests PASS.

- [ ] **Step 2.7: Run the existing pull integration test (if one exists) + unit tests**

Run: `cargo nextest run -p temper-cli`
Expected: PASS.

- [ ] **Step 2.8: Commit**

```bash
git add crates/temper-cli/src/actions/sync.rs crates/temper-cli/src/commands/pull.rs tests/e2e/tests/pull_command_test.rs
git commit -m "feat(cli): factor pull_one_resource primitive with Option<&mut Manifest>

commands/pull.rs reduces to a thin wrapper. pull_one_resource handles both
manifest-tracked and snapshot-to-CWD branches behind a single API. E2E
tests cover both.

Next: route sync_orchestration's body-pull through the primitive."
```

---

## Task 3: Route `sync_orchestration`'s body-pull through the primitive

**Purpose:** Unify the body-pull path. `pull_resource_body` is currently called only from `pull_resource` when `item.kind == Body`. Replace the body-arm of `pull_resource` with a call to `pull_one_resource`. The meta-only arm is untouched. Existing `pull_resource_body` can be removed since its behavior is subsumed.

**Files:**
- Modify: `crates/temper-cli/src/actions/sync.rs:1059-1069` (the `pull_resource` dispatcher) and lines 1308-1431 (`pull_resource_body` — delete).

- [ ] **Step 3.1: Replace `pull_resource` body-arm**

In `crates/temper-cli/src/actions/sync.rs`, find:

```rust
async fn pull_resource(
    client: &temper_client::TemperClient,
    manifest: &mut Manifest,
    vault_root: &Path,
    item: &SyncPullItem,
) -> Result<()> {
    match item.kind {
        SyncItemKind::Body => pull_resource_body(client, manifest, vault_root, item).await,
        SyncItemKind::MetaOnly => pull_resource_meta_only(client, manifest, vault_root, item).await,
    }
}
```

Replace with:

```rust
async fn pull_resource(
    client: &temper_client::TemperClient,
    manifest: &mut Manifest,
    vault_root: &Path,
    item: &SyncPullItem,
) -> Result<()> {
    match item.kind {
        SyncItemKind::Body => {
            // Delegate to the unified primitive. The primitive writes the
            // file, updates the entry's body_hash/remote_body_hash/state/
            // synced_at. One gap: pull_one_resource does not reconcile
            // remote_body_hash with the server-computed hash from the diff
            // (item.content_hash) — but the primitive recomputes it from
            // the file it just wrote, which is identical to what
            // pull_resource_body did historically (line 1416 in the old
            // code: `remote_body_hash: item.content_hash.clone()`). The
            // server's content_hash is a redundancy check; post-write
            // recompute is the authoritative value.
            pull_one_resource(client, vault_root, item.resource_id, Some(manifest))
                .await
                .map(|_| ())
        }
        SyncItemKind::MetaOnly => {
            pull_resource_meta_only(client, manifest, vault_root, item).await
        }
    }
}
```

- [ ] **Step 3.2: Delete now-unused `pull_resource_body`**

Find `async fn pull_resource_body(` (around line 1308) and delete the entire function body through its closing brace (around line 1431). Also delete the helper `write_pulled_file` (lines 1437-1469) — it was only called from inside `pull_resource_body`. Confirm no other callers with grep before deleting `write_pulled_file`.

Run: `grep -n 'write_pulled_file' crates/temper-cli/src/actions/sync.rs` (or use your Grep tool).
Expected: zero matches after deletion.

- [ ] **Step 3.3: Delete or update any imports/uses that `pull_resource_body` needed but `pull_resource` doesn't**

Specifically: `parse_kb_uri` may become unused in `pull_resource` flow (was used in `pull_resource_body:1326`). It may still be used elsewhere — do not remove globally. Just ensure `cargo check` passes.

- [ ] **Step 3.4: Verify compile + full sync test suite passes unchanged**

Run: `cargo nextest run -p temper-e2e --features test-db sync`
Expected: all PASS — the sync-body behavior is preserved because `pull_one_resource`'s manifest-tracked branch does the same writes, and the meta-only branch is untouched.

⚠️ **Semantic difference to verify:** The previous `pull_resource_body` had a slug-dedup path for "manifest entry exists but file missing" (lines 1355-1368). `pull_one_resource` does not replicate that — it always writes to the manifest-resolved path. This is a deliberate simplification: the dedup only helped in a pathological state (manifest says X but file is gone), and the manifest path is authoritative. If a sync test fails because of this, the fix is to align the test with the simpler behavior; if no test exercises that edge case, the simplification stands.

- [ ] **Step 3.5: Commit**

```bash
git add crates/temper-cli/src/actions/sync.rs
git commit -m "refactor(cli): route sync body-pull through pull_one_resource

pull_resource's Body arm now delegates to the unified primitive; the
inlined pull_resource_body and its write_pulled_file helper are removed.
MetaOnly arm untouched — it's a sync-only concept.

Behavior difference: the 'manifest entry exists but file missing' case
no longer slug-dedupes — always writes to the manifest-resolved path.
This is the authoritative source."
```

---

## Task 4: Factor `push_one_resource` out of `push_resource_body`

**Purpose:** New function in `actions/sync.rs` that handles the body-push flow with `Option<&mut Manifest>`. Accepts either a `PushTarget::Path(&Path)` (read frontmatter for the id) or `PushTarget::Id(ResourceId)` (requires manifest to resolve path).

**Files:**
- Modify: `crates/temper-cli/src/actions/sync.rs` — add `push_one_resource` directly above `push_resource_body` (around line 920).

- [ ] **Step 4.1: Write e2e test for `push_one_resource` with manifest=None**

**File:** `tests/e2e/tests/push_command_test.rs` (create)

```rust
//! E2E: temper-cli `push` command primitive.

use temper_cli::actions::sync::{push_one_resource, PushTarget};
use temper_core::types::PushKind;

mod common;
use common::{e2e_client, e2e_test_context};

#[tokio::test]
async fn push_one_resource_with_path_no_manifest_creates_and_rewrites_provisional() {
    let ctx = e2e_test_context("push-no-manifest").await;
    let client = e2e_client(&ctx).await;

    let tmp = tempfile::tempdir().unwrap();
    let vault_root = tmp.path();

    // Write a file with a provisional id. Pattern lifted from the templates
    // a normal `temper resource create` would produce.
    let provisional_id = uuid::Uuid::now_v7();
    let rel_path = format!("{}/test/session/push-no-manifest-seed.md", ctx.owner());
    let abs = vault_root.join(&rel_path);
    std::fs::create_dir_all(abs.parent().unwrap()).unwrap();
    let content = format!(
        "---\n\
         temper-provisional-id: \"{provisional_id}\"\n\
         temper-context: test\n\
         temper-type: session\n\
         temper-created: 2026-04-18\n\
         temper-owner: '@me'\n\
         title: Push No Manifest Seed\n\
         slug: push-no-manifest-seed\n\
         date: 2026-04-18\n\
         ---\n\
         Body content.\n"
    );
    std::fs::write(&abs, &content).unwrap();

    let result = push_one_resource(&client, vault_root, PushTarget::Path(&abs), None)
        .await
        .expect("push_one_resource");

    assert_eq!(result.kind, PushKind::New);
    assert_ne!(uuid::Uuid::from(result.resource_id), provisional_id,
        "server must assign a canonical id different from provisional");

    // Verify the file was rewritten in place: temper-provisional-id gone,
    // temper-id present with the server id.
    let updated = std::fs::read_to_string(&abs).unwrap();
    assert!(!updated.contains("temper-provisional-id"),
        "provisional id must be replaced");
    assert!(updated.contains(&format!("temper-id: \"{}\"", result.resource_id.as_uuid())),
        "canonical temper-id must be present: {updated}");

    // Verify server has the resource.
    let got = client.resources().get(result.resource_id.into()).await.unwrap();
    assert_eq!(got.title, "Push No Manifest Seed");
}

#[tokio::test]
async fn push_one_resource_with_manifest_updates_entry_state() {
    let ctx = e2e_test_context("push-with-manifest").await;
    let client = e2e_client(&ctx).await;
    let tmp = tempfile::tempdir().unwrap();
    let vault_root = tmp.path();

    let provisional_id = uuid::Uuid::now_v7();
    let rel_path = format!("{}/test/session/push-with-manifest-seed.md", ctx.owner());
    let abs = vault_root.join(&rel_path);
    std::fs::create_dir_all(abs.parent().unwrap()).unwrap();
    let content = format!(
        "---\n\
         temper-provisional-id: \"{provisional_id}\"\n\
         temper-context: test\n\
         temper-type: session\n\
         temper-created: 2026-04-18\n\
         temper-owner: '@me'\n\
         title: Push With Manifest Seed\n\
         slug: push-with-manifest-seed\n\
         date: 2026-04-18\n\
         ---\n\
         Body content.\n"
    );
    std::fs::write(&abs, &content).unwrap();

    let mut manifest = temper_core::types::Manifest::new("test-device".into());
    manifest.entries.insert(
        temper_core::types::ResourceId::from(provisional_id),
        temper_core::types::ManifestEntry {
            path: rel_path.clone(),
            body_hash: String::new(),
            remote_body_hash: String::new(),
            managed_hash: String::new(),
            open_hash: String::new(),
            remote_managed_hash: String::new(),
            remote_open_hash: String::new(),
            synced_at: chrono::Utc::now(),
            state: temper_core::types::ManifestEntryState::Dirty,
            mtime_secs: None,
            last_audit_id: None,
            provisional: true,
        },
    );

    let result = push_one_resource(&client, vault_root, PushTarget::Path(&abs), Some(&mut manifest))
        .await
        .expect("push_one_resource");

    // Manifest entry remapped to server id.
    assert!(manifest.entries.get(&result.resource_id).is_some(),
        "canonical id must be in manifest after remap");
    assert!(manifest.entries.get(&temper_core::types::ResourceId::from(provisional_id)).is_none(),
        "provisional entry must be removed from manifest");
    let entry = manifest.entries.get(&result.resource_id).unwrap();
    assert_eq!(entry.state, temper_core::types::ManifestEntryState::Clean);
    assert!(!entry.provisional);
    assert!(!entry.body_hash.is_empty());
}
```

- [ ] **Step 4.2: Run tests to verify failure**

Run: `cargo nextest run -p temper-e2e --features test-db -E 'test(push_one_resource)'`
Expected: FAIL — `push_one_resource` is not defined yet.

- [ ] **Step 4.3: Implement `push_one_resource`**

Insert in `crates/temper-cli/src/actions/sync.rs` directly above `async fn push_resource_body` (around line 920). This function subsumes almost all of `push_resource_body`'s logic; the existing function becomes a two-line adapter in Task 5.

```rust
/// Push a single resource's body+meta to the server.
///
/// With a `Path` target, reads the file, extracts `temper-id` or
/// `temper-provisional-id` from frontmatter, and either PUTs (canonical id)
/// or POSTs (provisional or missing id) + rewrites frontmatter to the
/// canonical id. With an `Id` target, requires a manifest entry to resolve
/// the on-disk path.
///
/// When `manifest.is_some()` and the resolved id is tracked, updates the
/// entry's hashes/state/synced_at/mtime. Remaps the entry key from
/// provisional → canonical on POST-create where the ids differ.
pub async fn push_one_resource(
    client: &temper_client::TemperClient,
    vault_root: &Path,
    target: PushTarget<'_>,
    mut manifest: Option<&mut Manifest>,
) -> Result<PushResult> {
    // Step 1: resolve (file_path, entry_id, is_provisional) from the target.
    let (file_path, entry_id, is_provisional) = match target {
        PushTarget::Path(path) => {
            let abs: std::path::PathBuf = if path.is_absolute() {
                path.to_path_buf()
            } else {
                vault_root.join(path)
            };
            if !abs.exists() {
                return Err(TemperError::NotFound(format!(
                    "file not found: {}",
                    abs.display()
                )));
            }
            let fm = Frontmatter::parse_file(&abs).map_err(|e| {
                TemperError::Config(format!(
                    "push requires parseable frontmatter at {}: {e}",
                    abs.display()
                ))
            })?;
            // Accept temper-id (canonical) or temper-provisional-id.
            // Frontmatter::doc_id() returns whichever is present per
            // the accepting logic in frontmatter/document.rs:169.
            let id = fm.doc_id().ok_or_else(|| {
                TemperError::Config(format!(
                    "push requires temper-id or temper-provisional-id in frontmatter at {}",
                    abs.display()
                ))
            })?;
            let is_prov = fm.is_provisional();
            (abs, ResourceId::from(id), is_prov)
        }
        PushTarget::Id(id) => {
            let manifest_ref = manifest.as_ref().ok_or_else(|| {
                TemperError::Config(
                    "push by id requires a manifest; pass a path for manifest-less push".into(),
                )
            })?;
            let entry = manifest_ref.entries.get(&id).ok_or_else(|| {
                TemperError::NotFound(format!("manifest entry not found: {id}"))
            })?;
            let abs = vault_root.join(&entry.path);
            if !abs.exists() {
                return Err(TemperError::NotFound(format!(
                    "vault file not found: {}",
                    abs.display()
                )));
            }
            (abs, id, entry.provisional)
        }
    };

    // Step 2: read, strip frontmatter, build payload.
    let content = std::fs::read_to_string(&file_path)?;
    let body = strip_frontmatter(&content);
    let (context, doc_type) = match file_path.strip_prefix(vault_root).ok().and_then(Vault::parse_rel) {
        Some(parsed) => (parsed.context.to_string(), parsed.doc_type.to_string()),
        None => {
            // Fall back to the frontmatter values for out-of-vault paths
            // (e.g. cloud-mode working dirs where the path doesn't follow
            // the `{owner}/{context}/{doc_type}/` convention).
            let fm = Frontmatter::try_from(content.as_str()).map_err(|e| {
                TemperError::Config(format!(
                    "push: could not parse frontmatter for context/doc_type: {e}"
                ))
            })?;
            (
                fm.context().unwrap_or("default").to_string(),
                fm.doc_type().as_str().to_string(),
            )
        }
    };

    let (managed_meta, open_meta) = match Frontmatter::try_from(content.as_str()) {
        Ok(fm) => (Some(fm.managed_json()), Some(fm.open_json())),
        Err(_) => (None, None),
    };
    let title = ingest::title_from_path(&file_path);
    let mut payload = ingest::build_ingest_payload(body, &title, &context, &doc_type, None)?;
    payload.managed_meta = managed_meta;
    payload.open_meta = open_meta;

    // Step 3: POST (new/provisional) or PUT (existing canonical).
    let is_existing_canonical = !is_provisional;
    let resource = if is_existing_canonical {
        client
            .ingest()
            .update(Uuid::from(entry_id), &payload)
            .await
            .map_err(crate::commands::client_err)?
    } else {
        client
            .ingest()
            .create(&payload)
            .await
            .map_err(crate::commands::client_err)?
    };
    let server_id = ResourceId::from(resource.id);
    let push_kind = if is_existing_canonical { PushKind::Modified } else { PushKind::New };

    // Step 4: handle provisional → canonical rewrite if server assigned a new id.
    if server_id != entry_id || is_provisional {
        tracing::info!(
            %entry_id,
            %server_id,
            is_provisional,
            "remapping resource: local ID → server ID"
        );
        // Rewrite local file: provisional → canonical.
        let file_content = std::fs::read_to_string(&file_path)?;
        let updated = file_content
            .replace(
                &format!("temper-provisional-id: \"{}\"", Uuid::from(entry_id)),
                &format!("temper-id: \"{}\"", Uuid::from(server_id)),
            )
            .replace(
                &format!("temper-provisional-id: {}", Uuid::from(entry_id)),
                &format!("temper-id: {}", Uuid::from(server_id)),
            );
        let updated = if updated != file_content {
            updated
        } else {
            // Fallback: file already had temper-id with local UUID.
            file_content.replace(&Uuid::from(entry_id).to_string(), &Uuid::from(server_id).to_string())
        };
        if updated != file_content {
            std::fs::write(&file_path, &updated)?;
        } else {
            tracing::warn!(
                %entry_id,
                "provisional id not found in file content — frontmatter not updated"
            );
        }

        // Remap manifest entry if present.
        if let Some(m) = manifest.as_deref_mut() {
            if let Some(mut entry) = m.entries.remove(&entry_id) {
                entry.provisional = false;
                m.entries.insert(server_id, entry);
            }
        }
    }

    // Step 5: recompute hashes from the just-written file + update manifest.
    let (pushed_managed_hash, pushed_open_hash) = Frontmatter::parse_file(&file_path)
        .map_err(|e| {
            TemperError::Vault(format!(
                "push_one_resource post-write hash compute {}: {e}",
                file_path.display()
            ))
        })?
        .hashes();

    if let Some(m) = manifest.as_deref_mut() {
        if let Some(e) = m.entries.get_mut(&server_id) {
            e.remote_body_hash = payload.content_hash.clone().unwrap_or_default();
            e.remote_managed_hash = pushed_managed_hash;
            e.remote_open_hash = pushed_open_hash;
            e.state = ManifestEntryState::Clean;
            e.synced_at = chrono::Utc::now();
            e.mtime_secs = file_mtime_secs(&file_path).ok();
        }
    }

    Ok(PushResult {
        resource_id: server_id,
        path: file_path,
        kind: push_kind,
    })
}
```

⚠️ **Verify the helpers exist before writing this code:**
- `Frontmatter::doc_id()` → returns `Option<Uuid>` for either `temper-id` or `temper-provisional-id`. Check `crates/temper-core/src/frontmatter/document.rs` around line 169. If it's named differently (e.g. `id()`, `temper_id()`), use whichever accessor already accepts both forms. Grep: `grep -n 'fn.*id\|provisional' crates/temper-core/src/frontmatter/document.rs`
- `Frontmatter::is_provisional()` → returns `bool` for whether the frontmatter carries a provisional id. If missing, substitute: `fm.doc_id_is_provisional()` or compute inline by checking `content.contains("temper-provisional-id:")`.
- `Frontmatter::context()` → returns `Option<&str>`. If missing, read the raw frontmatter yaml mapping.

If any helper is missing, **do not** invent a new API. Either add a minimal accessor to `frontmatter/document.rs` (if the underlying data is already there), or substitute with string/yaml inspection inline. Keep the changes minimal. Whichever you pick, grep for the closest existing pattern first (follows SG-1: Follow Existing Patterns).

- [ ] **Step 4.4: Run tests to verify pass**

Run: `cargo nextest run -p temper-e2e --features test-db -E 'test(push_one_resource)'`
Expected: PASS (2 tests).

- [ ] **Step 4.5: Commit**

```bash
git add crates/temper-cli/src/actions/sync.rs tests/e2e/tests/push_command_test.rs
git commit -m "feat(cli): add push_one_resource primitive with Option<&mut Manifest>

Handles PushTarget::Path (read frontmatter for id) and PushTarget::Id
(manifest lookup). POST-or-PUT routing, provisional→canonical rewrite,
manifest entry remap all preserved from push_resource_body.

Next: route sync_orchestration's body-push through this primitive."
```

---

## Task 5: Route `sync_orchestration`'s body-push through `push_one_resource`

**Purpose:** Delegate `push_resource_body` to the primitive. The sync engine's `push_resource` dispatcher keeps its Body/MetaOnly split; the body arm calls `push_one_resource` with the manifest-resolved path.

**Files:**
- Modify: `crates/temper-cli/src/actions/sync.rs` — replace `push_resource_body` body.

- [ ] **Step 5.1: Replace `push_resource_body` with a thin adapter**

Find `async fn push_resource_body(` (around line 920). Replace the entire function (up to its closing brace around line 1057) with:

```rust
async fn push_resource_body(
    client: &temper_client::TemperClient,
    manifest: &mut Manifest,
    vault_root: &Path,
    item: &SyncPushItem,
) -> Result<()> {
    // Resolve the id the same way the primitive used to: prefer the
    // server-assigned id if present, else extract from the kb:// URI.
    let entry_id = match item.resource_id {
        Some(id) => id,
        None => extract_resource_id(&item.uri)?,
    };
    push_one_resource(client, vault_root, PushTarget::Id(entry_id), Some(manifest))
        .await
        .map(|_| ())
}
```

- [ ] **Step 5.2: Full sync test sweep**

Run: `cargo nextest run -p temper-e2e --features test-db sync`
Expected: all sync tests PASS — body-push behavior unchanged because `push_one_resource` is byte-for-byte equivalent to the old `push_resource_body` when called with `PushTarget::Id(entry_id)` + `Some(manifest)`.

- [ ] **Step 5.3: Run the full Rust test suite to catch any unrelated regression**

Run: `cargo make test-db` (requires Docker Postgres running: `cargo make docker-up`)
Expected: PASS.

- [ ] **Step 5.4: Commit**

```bash
git add crates/temper-cli/src/actions/sync.rs
git commit -m "refactor(cli): route sync body-push through push_one_resource

push_resource_body becomes a two-line adapter that calls the primitive
with PushTarget::Id + Some(manifest). MetaOnly arm untouched. All
existing sync tests pass unchanged."
```

---

## Task 6: Add `temper push` CLI command

**Purpose:** Expose the primitive as a first-class CLI command. Accepts `<target>` as UUID-or-path — UUID parse first, fall back to treating as a filesystem path.

**Files:**
- Create: `crates/temper-cli/src/commands/push.rs`
- Modify: `crates/temper-cli/src/commands/mod.rs` — add `pub mod push;`
- Modify: `crates/temper-cli/src/cli.rs` — add `Push` variant near the existing `Pull` variant
- Modify: `crates/temper-cli/src/main.rs` — add match arm

- [ ] **Step 6.1: Create `commands/push.rs`**

```rust
//! `temper push` — upload a single resource to the cloud.

use std::path::Path;

use uuid::Uuid;

use crate::actions::runtime;
use crate::actions::sync::{push_one_resource, PushTarget};
use crate::error::TemperError;
use crate::output;
use temper_core::types::ResourceId;

/// Accept either a UUID (requires manifest) or a filesystem path.
pub fn run(target: &str) -> crate::error::Result<()> {
    let target_owned = target.to_string();

    runtime::with_client(|client| {
        Box::pin(async move {
            let vault_root = crate::config::resolve_vault(None)?;
            let temper_dir = vault_root.join(".temper");
            let device_id =
                crate::config::load_device_id().unwrap_or_else(|| "unknown".to_string());

            // Try to load a manifest; if absent, proceed manifest-less.
            let (mut manifest_opt, persist) =
                match crate::manifest_io::load_manifest(&temper_dir, &device_id) {
                    Ok(m) => (Some(m), true),
                    Err(_) => (None, false),
                };

            // UUID first — if it parses, treat as an id target (needs manifest).
            // Else resolve as a path: CWD-relative first, then vault-root-relative.
            let result = if let Ok(uuid) = Uuid::parse_str(&target_owned) {
                push_one_resource(
                    client,
                    &vault_root,
                    PushTarget::Id(ResourceId::from(uuid)),
                    manifest_opt.as_mut(),
                )
                .await?
            } else {
                let cwd_path = std::env::current_dir()?.join(&target_owned);
                let resolved: std::path::PathBuf = if cwd_path.exists() {
                    cwd_path
                } else {
                    let vr = vault_root.join(&target_owned);
                    if !vr.exists() {
                        return Err(TemperError::NotFound(format!(
                            "push target not found: {target_owned}"
                        )));
                    }
                    vr
                };
                push_one_resource(
                    client,
                    &vault_root,
                    PushTarget::Path(&resolved),
                    manifest_opt.as_mut(),
                )
                .await?
            };

            if persist {
                if let Some(m) = &manifest_opt {
                    crate::manifest_io::save_manifest(&temper_dir, m)?;
                }
            }
            output::success(format!(
                "Pushed: {} -> {}",
                result.path.display(),
                result.resource_id
            ));
            Ok(())
        })
    })
}
```

- [ ] **Step 6.2: Register the module**

**File:** `crates/temper-cli/src/commands/mod.rs`

Find the module declarations (lines 1-23) and add `pub mod push;` alphabetically (after `pub mod pull;` on line 12 — insert as line 13):

```rust
pub mod pull;
pub mod push;
```

- [ ] **Step 6.3: Add the `Push` CLI variant**

**File:** `crates/temper-cli/src/cli.rs`

Find the `Pull { resource_id: String }` variant (around line 120-124) and add a `Push` variant directly after it:

```rust
    /// Pull a resource from the cloud
    Pull {
        /// Resource UUID
        resource_id: String,
    },

    /// Push a single resource to the cloud. Target can be a UUID (requires
    /// manifest) or a filesystem path. Always sends body + meta together.
    Push {
        /// Resource UUID or path to a vault file
        target: String,
    },
```

- [ ] **Step 6.4: Add the main.rs dispatch arm**

**File:** `crates/temper-cli/src/main.rs`

Find `Commands::Pull { resource_id } => commands::pull::run(&resource_id)` (line 337) and add below it:

```rust
        Commands::Pull { resource_id } => commands::pull::run(&resource_id),
        Commands::Push { target } => commands::push::run(&target),
```

- [ ] **Step 6.5: Verify compile**

Run: `cargo check -p temper-cli`
Expected: clean build.

- [ ] **Step 6.6: Run the CLI end-to-end via an e2e test**

Extend `tests/e2e/tests/push_command_test.rs` with a test that invokes the CLI binary via `assert_cmd` (check `tests/e2e/tests/common/` for existing CLI-invocation helpers — there's typically an `e2e_cli_cmd(ctx)` builder).

```rust
#[tokio::test]
async fn temper_push_cli_path_argument_round_trips() {
    let ctx = e2e_test_context("push-cli-path").await;
    let tmp = tempfile::tempdir().unwrap();

    let provisional_id = uuid::Uuid::now_v7();
    let file_path = tmp.path().join("push-cli-seed.md");
    std::fs::write(&file_path, format!(
        "---\ntemper-provisional-id: \"{provisional_id}\"\ntemper-context: test\ntemper-type: session\ntemper-created: 2026-04-18\ntemper-owner: '@me'\ntitle: Push CLI Seed\nslug: push-cli-seed\ndate: 2026-04-18\n---\nBody.\n",
    )).unwrap();

    // Invoke `temper push <path>` with the e2e CLI harness (auth configured
    // for ctx; vault_root points at tmp). Adapt to the repo's existing
    // pattern in tests/e2e/tests/common/.
    let output = common::e2e_cli_cmd(&ctx)
        .arg("push")
        .arg(&file_path)
        .output()
        .await
        .expect("cli");
    assert!(output.status.success(), "stderr: {}", String::from_utf8_lossy(&output.stderr));

    // Verify the file was rewritten.
    let updated = std::fs::read_to_string(&file_path).unwrap();
    assert!(!updated.contains("temper-provisional-id"));
    assert!(updated.contains("temper-id:"));
}
```

⚠️ **If the e2e CLI harness doesn't exist / differs:** skip this specific test (the programmatic `push_one_resource` tests from Task 4 already cover the functionality). Document the gap at the bottom of `push_command_test.rs` as a `// TODO: CLI invocation test once e2e CLI harness lands`.

Run: `cargo nextest run -p temper-e2e --features test-db -E 'test(temper_push_cli)'`
Expected: PASS (if the test is included).

- [ ] **Step 6.7: Commit**

```bash
git add crates/temper-cli/src/commands/push.rs crates/temper-cli/src/commands/mod.rs crates/temper-cli/src/cli.rs crates/temper-cli/src/main.rs tests/e2e/tests/push_command_test.rs
git commit -m "feat(cli): add temper push <id|path> command

UUID-first target resolution; falls back to path (CWD then vault-root-
relative). Manifest loaded if present, passed Some(&mut) to the
primitive so entries are tracked in local mode. Manifest-less calls
work for cloud mode (Unit B.2 dispatch target)."
```

---

## Task 7: Final verification sweep

**Purpose:** Confirm nothing regressed, check the full quality gate.

- [ ] **Step 7.1: Run all Rust unit tests**

Run: `cargo make test`
Expected: PASS (fast — no DB).

- [ ] **Step 7.2: Run the full Rust integration suite**

Precondition: `cargo make docker-up` (Docker Postgres must be running on 5437).
Run: `cargo make test-db`
Expected: all PASS. Pay attention to the sync_test, push_command_test, and pull_command_test suites specifically.

- [ ] **Step 7.3: Run `cargo make check`**

Run: `cargo make check`
Expected: clean — fmt, clippy with `-D warnings`, docs, machete, biome all pass.

- [ ] **Step 7.4: Smoke-test the new commands manually (optional but recommended)**

```bash
# In one terminal
cargo make docker-up
cargo make run  # starts local API

# In another, against a test vault with a pending file
cargo run -p temper-cli -- push tests/fixtures/some-session.md
cargo run -p temper-cli -- pull <the-id-from-above>
```

Expected: both commands succeed, file has canonical temper-id, pull round-trips cleanly.

- [ ] **Step 7.5: Review acceptance criteria against implementation**

Walk through each acceptance criterion from the task definition:

1. ✅ `temper push <path>` round-trips identically to sync-run-push — Task 4 primitive is the exact same logic as old `push_resource_body`; Task 5 routes sync through it.
2. ✅ `temper pull <id>` with manifest writes to manifest path + updates entry — Task 2 primitive's `ManifestTracked` branch.
3. ✅ `temper pull <id>` without manifest writes snapshot to CWD — Task 2 primitive's `Snapshot` branch preserves ADDED behavior.
4. ✅ `sync run` unchanged — Tasks 3 and 5 route the body tiers through primitives that replicate previous behavior; meta-only untouched; e2e sync_test passes.
5. ✅ Primitives take `Option<&mut Manifest>` — Task 1 type signatures; Tasks 2/4 implementations.

- [ ] **Step 7.6: Final commit if any checkpoint-derived fixups**

If anything was adjusted during verification, commit as:

```bash
git commit -m "chore(cli): verification fixups for Unit A primitives"
```

---

## Execution Notes

### Parallelism
- Tasks 2 and 4 (add `pull_one_resource`, add `push_one_resource`) touch different regions of `actions/sync.rs` and are independently implementable. They could be dispatched in parallel if using subagent-driven development. Tasks 3 and 5 must come after their respective primitives but are also independent of each other.

### Subagent guidance (from skill)
Every subagent dispatched for this plan MUST receive:
- The full subagent-guidance principles (SG-1 through SG-13) from the `temper` skill's `subagent-guidance.md`
- Project fundamentals: typed structs over inline JSON, shared types at boundaries, service layer owns SQL, params structs, auth before writes, profile scoping, pino logging (TS), SQL macros for compile-time check
- TDD discipline: write test, run and see it fail, implement minimal, run and see it pass, commit
- Verification-before-completion: run the exact verification command before claiming done
- `cargo make check` must pass before any "done" claim

### Notes specific to this plan
- The existing `push_resource_body` and `pull_resource_body` have nuanced behavior around provisional-id rewrite and slug dedup. The primitives preserve provisional-id rewrite exactly; they simplify slug dedup (pull_one_resource always writes to the manifest-resolved path, dropping the "manifest exists, file missing → dedup slug" fallback). If any existing test exercises that fallback, either the test is stale and should be updated, or the fallback is load-bearing and we restore it — resolve case-by-case, don't suppress failures.
- `Frontmatter::doc_id()` / `is_provisional()` / `context()` accessors are assumed; verify against `crates/temper-core/src/frontmatter/document.rs` before writing Task 4 code. If missing, add minimal accessors rather than re-implementing inline at each callsite (SG-3: No Logic Duplication).
- The meta-only push/pull paths are intentionally untouched — they are a sync-diff-engine optimization, not a user-invokable command. Future `temper push --meta-only` would be additive and is out of scope.
