# Cloud-only vault — Chunk 4: delete `vault_backend/`, `Surface::CliLocalVault`, `DomainEvent::VaultFile*`

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Delete the `vault_backend/` module tree (kept alive through Chunk 3 by `pub` visibility), drop the `Surface::CliLocalVault` variant and the three `DomainEvent::VaultFile*` event variants it constructed, and clean up the pub-orphans that fall out. Keep the workspace bisectable: every task ends with `cargo make check` green and a commit.

**Architecture:** Peel from the leaves inward. First detach the two non-`vault_backend/` callers that still consume internals (`commands/research.rs` imports `per_doctype`; `commands/resource.rs::show_edges` uses `manifest_io::load_manifest`). Then delete the directory. Then remove the now-unreferenced enum variants in `temper-core` (`Surface::CliLocalVault` and `DomainEvent::VaultFile{Written,Removed,ManifestUpdated}`). Finally sweep the pub-orphans that `cargo make check` surfaces.

**Tech Stack:** Rust 2024 edition, cargo-make, cargo-nextest, sqlx (compile-time queries — unchanged by this chunk), askama (template engine — keep).

---

## Plan gate — resolution

The spec's literal Chunk 4 scope says "remove `manifest_io` and the `temper-core` manifest/sync module" so that "`VaultBackend` and `Manifest` symbols are gone." That conflicts with Chunk 5's "delete sync engine" scope: 25 files outside `vault_backend/` reference `manifest_io` / `Manifest`, spread across `actions/sync.rs`, `commands/sync_cmd.rs`, `commands/push.rs`, `actions/index_build.rs`, `actions/doctor.rs`, `actions/ingest.rs`, `commands/search_cmd.rs`, the `temper-core` type modules (`types/manifest.rs`, `types/sync.rs`), and six e2e tests (`sync_test.rs`, `push_command_test.rs`, `pull_command_test.rs`, `locally_missing_recovery_test.rs`, `graph_build_e2e_test.rs`, `meta_test.rs`).

**Decision: Option 1 (Defer).** Chunk 4 = `vault_backend/` + `Surface::CliLocalVault` + `DomainEvent::VaultFile*` only. `manifest_io`, `temper-core/src/types/{manifest,sync}.rs`, and the `Manifest` symbol stay live; they get deleted with the sync engine in Chunk 5.

**Spec deviation acknowledged.** The `Manifest`-symbols-gone acceptance criterion is moved to Chunk 5's checklist. This plan documents that move so the consolidated reviewer doesn't flag it as a miss.

**Two consumers force this resolution:**

1. `commands/research.rs:9` imports `crate::vault_backend::per_doctype::{self, DoctypeFields, WriteArgs}` for the local-mode research-save path. Task 2 inlines the research-specific write so `per_doctype.rs` dies with `vault_backend/`. (We choose inline-into-research over relocate-out-of-vault_backend because research is the only non-vault_backend caller; relocating would carry along Task/Goal/Session/Concept/Decision dispatch for no consumer.)

2. `commands::resource::show_edges` resolves the resource id via `manifest_io::load_manifest`. The manifest is never populated in cloud-only mode, so this path is silently broken today. Task 3 switches to `client.resources().resolve_by_uri(...)` — same pattern `show` already uses (`resource.rs:827`).

---

## Cleanups bundled in this chunk

| Item | Why it travels with Chunk 4 |
|------|----------------------------|
| `Surface::CliLocalVault` variant | `vault_backend/` was its only production constructor (test fixtures in `temper-core` are co-deletable) |
| `DomainEvent::VaultFileWritten` / `VaultFileRemoved` / `VaultManifestUpdated` | Only `vault_backend/vault_backend.rs` and a test-only helper in `resource.rs` construct them |
| `commands::resource::show_edges` manifest dep | Silently broken in cloud-only today; Task 3 fixes it before the manifest types eventually move in Chunk 5 |
| `lookup::find_resource` + `FindableResource` + `ResolvedResource` | Only `show_edges` consumes them; orphaned after Task 3. `lookup::cached_profile_slug` / `set_cached_profile_slug` STAY (used by `actions/runtime.rs`) |
| `commands::resource` pub helpers: `scan_rows`, `parse_row`, `sort_rows`, `filter_rows`, `ResourceRow` local helpers, `render_list`, `RenderListParams`, `ListFilters` | Only `vault_backend/` consumed them. Verify with `cargo make check` after Task 5; delete whatever it surfaces. |
| `actions::runtime::with_arc_client` | Pre-existing dead code on `origin/main` (no callers) |
| `commands::resource::tests::output_with_vault_file` helper | Test-only constructor for `DomainEvent::VaultFileWritten`; tests that use it convert to `CommandOutput::new(row)` (the cloud-mode shape) |
| Doc-comment refs to `TEMPER_VAULT_STATE=cloud` in `operations/surface.rs:12,14` | Re-worded once `CliLocalVault` is gone |

## Items explicitly NOT in this chunk

- `manifest_io.rs`, `temper-core/src/types/{manifest,sync}.rs`, `Manifest` type — stay for Chunk 5
- `actions/sync.rs`, `commands/sync_cmd.rs::{status,refresh,reset}` — stay for Chunk 5
- `commands/push.rs`, `actions/push.rs` if present — stay for Chunk 5
- `projection.rs` — used by 6 cloud-mode read paths; stays (it consumes `manifest_io`, both die in Chunk 5)
- `temper-cli/tests/*.rs` integration tests — Chunk 3's T-TM swept these; only the dead `vault_backend/` test files (`ctx_tests.rs`, `tests.rs`) get deleted in this chunk
- E2E tests that exercise sync/push/graph commands — they still compile because their target commands survive; defer per-chunk

## Branch

`jct/cloud-only-vault-pr-b` — **do not branch**. This chunk continues the same branch that hosts Chunks 3–8; the PR opens after Chunk 8.

## Execution discipline (carry forward from Chunk 3)

- Subagent-driven execution, one fresh sonnet implementer per task; opus only for the final consolidated review.
- Each task ends with `cargo make check` green and a commit, so the branch stays bisectable.
- TDD where new behavior is added (Tasks 2 + 3); for deletion-only tasks (5, 6, 7, 8), the "test" is `cargo make check` plus relevant `cargo nextest`.

---

## Task 1: Test triage

Inventory every test file under `crates/temper-cli/src/`, `crates/temper-cli/tests/`, `crates/temper-core/src/`, and `tests/e2e/tests/` whose code path is touched by this chunk's deletions. Produce an explicit delete/keep/repoint verdict.

**Files:**
- Inspect: `crates/temper-cli/src/vault_backend/tests.rs`, `crates/temper-cli/src/vault_backend/ctx_tests.rs` (deleted with their parent module — no triage decision needed; named here for completeness)
- Inspect: `crates/temper-cli/src/commands/research.rs` inline tests if any
- Inspect: `crates/temper-cli/src/commands/resource.rs::tests` (uses `output_with_vault_file` helper — flag for Task 7)
- Inspect: `crates/temper-cli/src/lookup.rs` inline tests
- Inspect: `crates/temper-core/src/operations/{surface,events,actions,commands}.rs` inline tests
- Inspect: `crates/temper-cli/tests/*.rs` — any that reference `vault_backend`, `Surface::CliLocalVault`, `DomainEvent::VaultFile*`, `per_doctype`
- Inspect: `tests/e2e/tests/*.rs` — same grep

- [ ] **Step 1: Inventory affected test files via grep**

```bash
# Test files that import or reference deleting-symbols
rg -l 'vault_backend|Surface::CliLocalVault|DomainEvent::VaultFile|per_doctype' \
  --type rust crates/temper-cli/tests/ tests/e2e/tests/ 2>/dev/null
rg -l 'vault_backend|Surface::CliLocalVault|DomainEvent::VaultFile|per_doctype' \
  --type rust crates/temper-cli/src/ crates/temper-core/src/ 2>/dev/null | grep -v vault_backend/
```

- [ ] **Step 2: Produce a verdict table**

For each file the grep surfaces, decide:
- **Delete with parent** — file lives inside `vault_backend/`, dies in Task 5
- **Repoint** — file references a deleting symbol but the underlying test still has value; update to use cloud-mode equivalent
- **Keep** — false positive (e.g. mentions `Manifest` but uses the surviving `manifest_io`)
- **Defer to later chunk** — file's test is for sync/push/graph code that survives this chunk

Write the verdict table inline into the commit message for this task (no file produced; the table is ephemeral planning context).

- [ ] **Step 3: Commit the inventory**

No code change in this task. Make an empty commit recording the verdict so future bisects and reviewers can see the analysis:

```bash
git commit --allow-empty -m "$(cat <<'EOF'
cloud-only(ch4): test-triage inventory for chunk 4

Files referencing deleting symbols outside vault_backend/:
  - crates/temper-cli/src/commands/research.rs       -> Task 2 (refactor)
  - crates/temper-cli/src/commands/resource.rs       -> Task 3 + Task 7 (show_edges fix, helper drop)
  - crates/temper-core/src/operations/commands.rs    -> Task 6 (test fixtures)
  - crates/temper-core/src/operations/actions.rs     -> Task 6 (test fixtures)
  - crates/temper-core/src/operations/surface.rs     -> Task 6 (variant + doc comments)
  - crates/temper-core/src/operations/events.rs      -> Task 7 (variant + doc)

CLI integration tests (crates/temper-cli/tests/): none reference deleting
symbols directly (Chunk 3's T-TM already swept the local-mode tests).

E2E tests: none reference vault_backend/, Surface::CliLocalVault, or
DomainEvent::VaultFile* directly. Tests that mention 'Manifest' or
'manifest_io' (sync_test, push_command_test, pull_command_test,
locally_missing_recovery_test, graph_build_e2e_test, meta_test) all
exercise sync/push/graph paths that survive this chunk — deferred to
Chunk 5+.

vault_backend/{tests.rs, ctx_tests.rs} delete with the parent in Task 5.
EOF
)"
```

---

## Task 2: Inline `per_doctype::write_research` into `commands/research.rs`

Cuts the cloud-mode CLI's only non-`vault_backend/` dependency on the `per_doctype` module. After this task, `per_doctype.rs` has zero external callers and dies cleanly with the directory in Task 5.

**Files:**
- Modify: `crates/temper-cli/src/commands/research.rs`
- Test: `crates/temper-cli/src/commands/research.rs` (inline `#[cfg(test)]` module)

- [ ] **Step 1: Read `vault_backend/per_doctype.rs::write_research` (lines 622–760)**

Note the exact behavior: render `ResearchTemplate { title, slug, id, date, context }`, write to `vault.doc_file(owner, context, "research", slug)`, hard-error if the file already exists (`TemperError::Conflict`), then parse the frontmatter and overlay any body. Return a `WriteResult { resource_id: ResourceId, abs_path: PathBuf, rel_path: String }`.

Read it in full before writing the inlined version. Match its signature exactly except: inline only the research path (no doctype switch); accept the same args `research::save` already has in scope (`config: &Config`, `title: &str`, `context_name: &str`, `slug: &str`, `body: &str`); return the same `(resource_id: Uuid, abs_path: PathBuf, rel_path: String)` triple.

- [ ] **Step 2: Write the failing test**

Append to `commands/research.rs` (or its existing test module if one exists):

```rust
#[cfg(test)]
mod inline_research_write_tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn inline_write_research_creates_file_with_correct_frontmatter() {
        let tmp = TempDir::new().unwrap();
        let vault_root = tmp.path().to_path_buf();
        let config = Config {
            vault_root: vault_root.clone(),
            state_dir: tmp.path().join(".temper-state"),
            ..Config::default()
        };
        let result = write_research_inline(
            &config,
            "Sample Title",
            "temper",
            "2026-05-23-sample-title",
            "body text",
        )
        .expect("write must succeed");

        assert!(result.abs_path.exists(), "file must exist");
        let parsed = temper_core::frontmatter::Frontmatter::parse_file(&result.abs_path)
            .expect("must parse");
        let mm = parsed.managed_meta();
        assert_eq!(mm.get("temper-title").and_then(|v| v.as_str()), Some("Sample Title"));
        assert_eq!(mm.get("temper-slug").and_then(|v| v.as_str()), Some("2026-05-23-sample-title"));
        assert_eq!(parsed.body().trim(), "body text");
    }

    #[test]
    fn inline_write_research_errors_on_existing_slug() {
        let tmp = TempDir::new().unwrap();
        let config = Config {
            vault_root: tmp.path().to_path_buf(),
            state_dir: tmp.path().join(".temper-state"),
            ..Config::default()
        };
        write_research_inline(&config, "T", "temper", "2026-05-23-t", "")
            .expect("first write ok");
        let err = write_research_inline(&config, "T", "temper", "2026-05-23-t", "")
            .expect_err("second write must error");
        assert!(matches!(err, TemperError::Conflict(_)), "got {err:?}");
    }
}
```

If `Config::default()` is not available, mirror the pattern used in `vault_backend/per_doctype.rs::tests::make_config()` (lines ~745–765) — copy that helper into the new test module.

- [ ] **Step 3: Run test to verify it fails**

Run: `cargo nextest run -p temper-cli inline_research_write_tests`
Expected: FAIL — `write_research_inline` not defined.

- [ ] **Step 4: Implement the inline write helper**

Replace the contents of `crates/temper-cli/src/commands/research.rs` so it no longer imports `vault_backend::per_doctype`. The new file's outline:

```rust
use chrono::Local;
use std::path::PathBuf;
use temper_core::error::{Result as CoreResult, TemperError};
use temper_core::frontmatter::Frontmatter;
use temper_core::types::ids::ResourceId;
use temper_core::vault::Vault;
use uuid::Uuid;

use askama::Template;

use crate::config::Config;
use crate::discovery::{self, Event};
use crate::error::Result;
use crate::output;
use crate::templates::ResearchTemplate;
use crate::vault;

/// Result of an inline research-doctype write.
struct InlineWriteResult {
    resource_id: Uuid,
    abs_path: PathBuf,
    rel_path: String,
}

/// Render the `ResearchTemplate` and write a research doctype file under
/// `<vault_root>/<owner>/<context>/research/<slug>.md`. Errors if the file
/// already exists (callers handle the save-or-update overload at the surface).
///
/// This is the inlined replacement for `vault_backend::per_doctype::write_research`,
/// kept narrow to the research doctype because that was the only non-vault_backend
/// caller of `per_doctype`. Cloud-only mode treats the on-disk file as projection;
/// `publish_local_write_best_effort` (called by `save`) pushes it server-side.
fn write_research_inline(
    config: &Config,
    title: &str,
    context: &str,
    slug: &str,
    body: &str,
) -> CoreResult<InlineWriteResult> {
    let vault_layout = Vault::new(&config.vault_root);
    let owner = config.owner_for_context(context);
    let abs_path = vault_layout.doc_file(&owner, context, "research", slug);

    if abs_path.exists() {
        return Err(TemperError::Conflict(format!(
            "research note '{slug}' already exists at {}",
            abs_path.display()
        )));
    }

    if let Some(parent) = abs_path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let id = ResourceId::new();
    let date = Local::now().format("%Y-%m-%d").to_string();

    let tmpl = ResearchTemplate {
        id: id.to_string(),
        title: title.to_string(),
        slug: slug.to_string(),
        context: context.to_string(),
        date,
    };
    let rendered = tmpl
        .render()
        .map_err(|e| TemperError::Internal(format!("render ResearchTemplate: {e}")))?;
    std::fs::write(&abs_path, rendered)?;

    if !body.is_empty() {
        let mut fm = Frontmatter::parse_file(&abs_path)?;
        fm.set_body(body.to_string());
        fm.write_to(&abs_path)?;
    }

    let rel_path = abs_path
        .strip_prefix(&config.vault_root)
        .unwrap_or(&abs_path)
        .display()
        .to_string();

    Ok(InlineWriteResult {
        resource_id: Uuid::from(id),
        abs_path,
        rel_path,
    })
}

/// Create or update today's research note.
///
/// Two paths:
/// - If a research file with this slug already exists on disk, this is the
///   save-or-update overload: when `stdin_content` is `Some(_)`, the body is
///   replaced in place; otherwise the call is a no-op.
/// - If the file does not exist, delegate the bare file-write to
///   `write_research_inline` (which hard-errors on existing slug — the
///   pre-check above keeps that branch unreachable). The wrapper retains
///   publish-as-tail-action, discovery emission, and output.
pub fn save(
    config: &Config,
    title: &str,
    context: Option<&str>,
    stdin_content: Option<&str>,
    format: &str,
) -> Result<()> {
    let today = Local::now().format("%Y-%m-%d").to_string();
    let context_name = context.unwrap_or("general");
    let slug = format!("{today}-{}", vault::slugify(title));
    let vault_layout = Vault::new(&config.vault_root);
    let owner = config.owner_for_context(context_name);
    let note_path = vault_layout.doc_file(&owner, context_name, "research", &slug);

    if note_path.exists() {
        if let Some(body) = stdin_content {
            let mut fm = Frontmatter::parse_file(&note_path)?;
            fm.set_body(body.to_string());
            fm.write_to(&note_path)?;
            crate::actions::runtime::publish_local_write_best_effort(
                &config.vault_root,
                &note_path,
            )?;
            let relative = note_path
                .strip_prefix(&config.vault_root)
                .unwrap_or(&note_path);
            output::success(format!("Updated: {}", relative.display()));
        }
        return Ok(());
    }

    let body = stdin_content.unwrap_or("");
    let result = write_research_inline(config, title, context_name, &slug, body)?;
    crate::actions::runtime::publish_local_write_best_effort(&config.vault_root, &result.abs_path)?;

    let relative_str = result.rel_path.clone();
    let id = result.resource_id.to_string();

    if format == "json" {
        let json = serde_json::json!({
            "title": title,
            "project": context_name,
            "path": relative_str,
            "date": today,
            "id": id,
            "slug": slug,
        });
        println!(
            "{}",
            serde_json::to_string_pretty(&json).unwrap_or_default()
        );
    } else {
        output::success(format!("Created: {relative_str}"));
    }

    let ts = Local::now().to_rfc3339();
    let event = Event::ResourceCreate {
        ts,
        doc_type: "research".to_string(),
        title: title.to_string(),
        path: relative_str.to_string(),
        context: context_name.to_string(),
    };
    if let Err(e) = discovery::append_event(&config.state_dir, &event) {
        tracing::warn!("Failed to append discovery event: {e}");
    }

    Ok(())
}
```

Note: if `temper_core::error::TemperError::Internal` does not exist (it might be named differently), use the variant `vault_backend/per_doctype.rs::write_research` itself uses for askama errors — check that file's error mapping and match it.

If `ResearchTemplate`'s field names differ from what's shown above (the existing `per_doctype.rs::write_research` is the source of truth), use whatever names it uses — match exactly.

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo nextest run -p temper-cli inline_research_write_tests`
Expected: 2 PASS

- [ ] **Step 6: Verify the whole crate compiles**

Run: `cargo make check`
Expected: 0 errors, 0 new warnings. (Existing pub-orphan warnings from `vault_backend/`'s decoupling were already silent in Chunk 3 because of `pub`-at-lib visibility — that hasn't changed yet.)

- [ ] **Step 7: Commit**

```bash
git add crates/temper-cli/src/commands/research.rs
git commit -m "cloud-only(ch4): inline write_research into commands/research.rs"
```

---

## Task 3: Fix `commands::resource::show_edges` to use server-side id resolution

Replace the manifest scan with `client.resources().resolve_by_uri(...)`, mirroring the pattern `commands::resource::show` already uses (`resource.rs:794–830`).

**Files:**
- Modify: `crates/temper-cli/src/commands/resource.rs` (the `show_edges` function, around lines 855–895)

- [ ] **Step 1: Read the current `show_edges` and the surrounding cloud-mode pattern**

Read `crates/temper-cli/src/commands/resource.rs:820–900` to see how `show` resolves `(owner, context, doc_type, slug) -> resource_id` via `client.resources().resolve_by_uri(...)`. `show_edges` must follow the same pattern.

`show_edges` is called from the `show` command's `--edges` branch. The caller already knows `(slug, doc_type, context)` (or can derive them from the args). Inspect the call site to see what's in scope.

- [ ] **Step 2: Write the failing e2e test**

There may already be coverage in `tests/e2e/tests/`. Run:

```bash
rg 'show_edges|--edges' tests/e2e/tests/ -l
```

If no e2e test currently exercises `--edges` in cloud mode, add one to `tests/e2e/tests/edges_test.rs` (or the closest existing edges/resource test). Follow the pattern of `tests/e2e/tests/resource_show_test.rs` or similar — use the `common::TestEnv` harness, create a resource via API, then run `temper resource show <slug> --type <doctype> --edges --context <ctx>` and assert the command succeeds (not just that it prints "(none)" — the manifest-scan path used to error with "sync first" in cloud mode).

If e2e coverage already exists, this step is "verify it currently fails or already passes":
```bash
cargo nextest run --manifest-path tests/e2e/Cargo.toml --features test-db <edges_test_name>
```
Expected if it exists: the test should fail today because the manifest is never populated in cloud-only mode. If it already passes (perhaps because the test sync-then-shows), document why in the commit.

- [ ] **Step 3: Replace the `show_edges` implementation**

Find the function `fn show_edges(slug: &str, format: &str) -> Result<()>` in `crates/temper-cli/src/commands/resource.rs` (around line 858).

Replace its body so that, instead of `manifest_io::load_manifest` + entry-iteration, it:
1. Determines `(owner, context, doc_type)` for the slug — these must already be available at the call site; pass them in if needed (update the signature to `fn show_edges(owner: &str, context: &str, doc_type: &str, slug: &str, format: &str) -> Result<()>`)
2. Calls `runtime::with_client(|client| Box::pin(async move { client.resources().resolve_by_uri(&owner, &context, &doc_type, &slug).await }))` to get the `ResourceId`
3. Continues with the existing `client.resources().edges(resource_id)` call

Match the exact `resolve_by_uri` call form used by `show` at `resource.rs:827`. Update the call site (the `show` command's `--edges` branch) to pass the additional args.

Remove the `use crate::manifest_io;` import if `show_edges` was its only user in this file (verify with `rg 'manifest_io' crates/temper-cli/src/commands/resource.rs`).

- [ ] **Step 4: Run the e2e test**

Run: `cargo nextest run --manifest-path tests/e2e/Cargo.toml --features test-db <edges_test_name>`
Expected: PASS

- [ ] **Step 5: Run `cargo make check`**

Expected: 0 errors. `manifest_io` may now be reported as a dead pub item by clippy — that's expected and stays (it's a Chunk 5 deletion); we don't remove the `pub mod manifest_io` declaration here.

- [ ] **Step 6: Commit**

```bash
git add crates/temper-cli/src/commands/resource.rs tests/e2e/tests/<edges_test_file>.rs
git commit -m "cloud-only(ch4): show_edges resolves via server-side resolve_by_uri"
```

---

## Task 4: Update the `cloud_backend/ctx.rs` doc-comment

`crates/temper-cli/src/cloud_backend/ctx.rs:17` has a doc-comment that refers to `crate::vault_backend::VaultBackendCtx`. After Task 5, that path won't exist — the comment becomes a broken reference. Update it now so the deletion in Task 5 doesn't leave a dangling note.

**Files:**
- Modify: `crates/temper-cli/src/cloud_backend/ctx.rs` (line 17)

- [ ] **Step 1: Read the existing doc**

```bash
sed -n '14,22p' crates/temper-cli/src/cloud_backend/ctx.rs
```

- [ ] **Step 2: Rewrite the comment to describe `CloudBackendCtx` standalone**

Change `/// Mirrors [`crate::vault_backend::VaultBackendCtx`]'s shape.` to a self-contained description, e.g.:
`/// Per-request context for `CloudBackend` operations — carries the runtime, client, profile, and surface so handlers can dispatch without re-resolving config or auth.`

Use whatever phrasing matches the actual fields on `CloudBackendCtx`. Verify by reading the struct definition right below the comment.

- [ ] **Step 3: Run `cargo make check`**

Expected: 0 errors, no warnings introduced.

- [ ] **Step 4: Commit**

```bash
git add crates/temper-cli/src/cloud_backend/ctx.rs
git commit -m "cloud-only(ch4): cloud_backend/ctx.rs doc no longer refers to vault_backend"
```

---

## Task 5: Delete `crates/temper-cli/src/vault_backend/` directory

The big deletion. After Tasks 2–4, nothing outside `vault_backend/` references its internals. Remove the directory and the `pub mod` declaration in `lib.rs`. Expect compile errors from the now-orphaned `Surface::CliLocalVault` and `DomainEvent::VaultFile*` variant *constructions* inside the deleted code — but those are inside the deleted files, so they go with it. The variant *definitions* in `temper-core` survive this task (Tasks 6 and 7 remove them).

**Files:**
- Delete: `crates/temper-cli/src/vault_backend/` (entire directory: `mod.rs`, `vault_backend.rs`, `translators.rs`, `per_doctype.rs`, `tests.rs`, `ctx_tests.rs`)
- Modify: `crates/temper-cli/src/lib.rs` (remove `pub mod vault_backend;`)

- [ ] **Step 1: Delete the directory**

```bash
git rm -r crates/temper-cli/src/vault_backend/
```

- [ ] **Step 2: Remove the module declaration**

In `crates/temper-cli/src/lib.rs`, find the line `pub mod vault_backend;` (or `mod vault_backend;` — check current visibility) and delete it. Also delete any `use crate::vault_backend::*;` re-exports if present.

```bash
rg 'vault_backend' crates/temper-cli/src/lib.rs
```

Delete every line that the grep surfaces.

- [ ] **Step 3: Run `cargo make check`**

Expected: 0 errors. There may be **dead-code warnings** for:
- `Surface::CliLocalVault` (still defined in `temper-core`, no longer constructed in `temper-cli`)
- `DomainEvent::VaultFileWritten` / `VaultFileRemoved` / `VaultManifestUpdated` (same)
- Various pub helpers in `commands/resource.rs` and `lookup.rs` (Task 8 sweeps these)

The `temper-core` variant warnings only fire if the `#[cfg(test)]` fixtures don't construct them. Tasks 6 + 7 remove the variants and the fixtures together; if `cargo make check` is *currently* green because tests still construct them, that's fine — leave them for Task 6/7.

If `cargo make check` produces real errors (not warnings), STOP and report. Likely cause: a non-`vault_backend/` file still imports from `vault_backend::` — Tasks 2–4 missed one. Grep:
```bash
rg 'use.*vault_backend' --type rust crates/
```

- [ ] **Step 4: Run the full Rust test suite**

Run: `cargo nextest run --workspace`
Expected: Tests pass (or fail only on pre-known flakes — none expected from this deletion).

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "cloud-only(ch4): delete vault_backend/ module tree"
```

---

## Task 6: Remove `Surface::CliLocalVault` variant

After Task 5, only test fixtures in `temper-core` still construct this variant. Remove the variant, update those fixtures, and reword the doc comments that mention `TEMPER_VAULT_STATE=cloud` (the env var was deleted in Chunk 3).

**Files:**
- Modify: `crates/temper-core/src/operations/surface.rs` (remove variant, update doc comments, update inline `#[cfg(test)]` if present)
- Modify: `crates/temper-core/src/operations/commands.rs` (line ~125: test fixture)
- Modify: `crates/temper-core/src/operations/actions.rs` (lines ~1022, 1050, 1078, 1100: test fixtures)

- [ ] **Step 1: Read every current reference**

```bash
rg 'CliLocalVault' --type rust -n
```

Expected after Task 5: 6–8 references, all in `temper-core`.

- [ ] **Step 2: Remove the variant**

In `crates/temper-core/src/operations/surface.rs`, find the `pub enum Surface` definition (around line 10–20) and delete the `CliLocalVault,` line. Update any doc comments on the enum to drop references to it. Specifically, rewrite the lines that mention `TEMPER_VAULT_STATE=cloud` (currently at `surface.rs:12,14`) — the env var no longer exists; describe the remaining variants on their own terms.

- [ ] **Step 3: Update the inline `#[cfg(test)]` block in surface.rs**

`surface.rs:28,35` (a serde round-trip test) constructs `Surface::CliLocalVault`. Either:
- Replace with the next-most-relevant variant (likely `CloudApi` or whatever the remaining variants are — pick the one used elsewhere in the test module), OR
- Delete the assertions that specifically test `CliLocalVault` round-tripping if removing the variant makes them tautological.

Read the test, decide, edit. If unsure, default to replacing with whatever variant the other assertions in the same test use.

- [ ] **Step 4: Update `temper-core/src/operations/commands.rs:125`**

The line is inside a `#[cfg(test)]` block. Replace `Surface::CliLocalVault` with another variant; check what other tests in the same module use as the default — match them.

- [ ] **Step 5: Update `temper-core/src/operations/actions.rs:1022, 1050, 1078, 1100`**

Same pattern — these are all in `#[cfg(test)]` blocks. Replace with the same alternative variant used in Step 4 so the test fixtures stay internally consistent.

- [ ] **Step 6: Verify the env-var refs are also cleaned**

In `crates/temper-core/src/operations/events.rs:4`, a comment mentions `CliLocalVault`. Update it to drop the reference (the comment describes backend-qualified events generally — phrase without naming a removed variant).

- [ ] **Step 7: Run `cargo make check`**

Expected: 0 errors, 0 new warnings. The dead-code warning on `Surface::CliLocalVault` is now gone.

- [ ] **Step 8: Run the `temper-core` tests**

Run: `cargo nextest run -p temper-core`
Expected: all pass.

- [ ] **Step 9: Commit**

```bash
git add crates/temper-core/src/operations/
git commit -m "cloud-only(ch4): remove Surface::CliLocalVault variant"
```

---

## Task 7: Remove `DomainEvent::VaultFile{Written,Removed}` and `VaultManifestUpdated`

After Task 5, only the test-only helper `output_with_vault_file` in `commands/resource.rs::tests` still constructs these variants outside `temper-core` itself. Delete the helper, repoint the tests that called it to `CommandOutput::new(row)` (the cloud-mode shape), then delete the variants and their doc.

**Files:**
- Modify: `crates/temper-core/src/operations/events.rs` (remove three variants)
- Modify: `crates/temper-cli/src/commands/resource.rs` (delete `output_with_vault_file` helper, repoint its callers)

- [ ] **Step 1: Locate every remaining reference**

```bash
rg 'VaultFileWritten|VaultFileRemoved|VaultManifestUpdated' --type rust -n
```

Expected after Task 5: references in `temper-core/src/operations/events.rs` (definitions + module doc) and in `crates/temper-cli/src/commands/resource.rs:1812–1825` (the test helper).

- [ ] **Step 2: Delete the test-only helper and repoint callers**

In `crates/temper-cli/src/commands/resource.rs`, find:

```rust
fn output_with_vault_file(row: ResourceRow, rel_path: &str) -> CommandOutput<ResourceRow> {
    use temper_core::operations::DomainEvent;
    CommandOutput {
        value: row,
        events: vec![DomainEvent::VaultFileWritten {
            path: rel_path.to_string(),
        }],
    }
}
```

Delete the function. Then find every caller (`rg 'output_with_vault_file' crates/temper-cli/src/commands/resource.rs`) and replace each call with `CommandOutput::new(row)`.

The tests that called `output_with_vault_file` were asserting the legacy local-mode JSON shape (the `path` field populated from a `VaultFileWritten` event). In cloud-only mode the projection path is supplied explicitly via the `path` parameter of `render_create_output_to_string`, so the events-derived path is no longer needed. If a test specifically asserts the path field, ensure the explicit `Some(path)` argument is passed to `render_create_output_to_string` — most callers already do this.

If a test is now duplicating an existing cloud-mode test verbatim, delete the duplicate.

- [ ] **Step 3: Run `temper-cli` tests**

Run: `cargo nextest run -p temper-cli --features test-db`
Expected: all pass.

- [ ] **Step 4: Remove the variants from `events.rs`**

In `crates/temper-core/src/operations/events.rs`:
- Delete the three variants: `VaultFileWritten { path: String }` (line ~33), `VaultManifestUpdated { path: String }` (line ~35), `VaultFileRemoved { path: String }` (line ~37). Match the exact lines via the grep in Step 1.
- Update the module-level doc comment (lines 1–10) to drop the example `VaultFileWritten` reference. Reword along the lines of: "Events are backend-qualified: `DbResourceCreated` / `DbResourceUpdated` / `RemoteSynced` / `PushDeferred`."
- Update the inline `#[cfg(test)]` block at `events.rs:72` (constructs `DomainEvent::VaultFileWritten`) — either delete the test if it was specifically testing the removed variant, or repoint it to a surviving variant like `DomainEvent::DbResourceCreated`. Read the test to decide.

- [ ] **Step 5: Run `cargo make check`**

Expected: 0 errors, 0 new warnings.

- [ ] **Step 6: Run the full Rust workspace test suite**

Run: `cargo nextest run --workspace`
Expected: all pass.

- [ ] **Step 7: Commit**

```bash
git add crates/temper-core/src/operations/events.rs crates/temper-cli/src/commands/resource.rs
git commit -m "cloud-only(ch4): remove DomainEvent::VaultFile* variants"
```

---

## Task 8: Sweep pub-orphans surfaced by `cargo make check`

With `vault_backend/` and the variants gone, helpers that were `pub` solely for it become dead. Clippy under `-D warnings` will surface them. Delete one at a time, recompile per item, commit at the end of the sweep.

**Files (candidates — verify each is unused before deleting):**
- Modify: `crates/temper-cli/src/commands/resource.rs` (`scan_rows`, `parse_row`, `sort_rows`, `filter_rows`, `ResourceRow` local helpers, `render_list`, `RenderListParams`, `ListFilters`)
- Modify: `crates/temper-cli/src/lookup.rs` (`FindableResource`, `ResolvedResource`, `find_resource` — `cached_profile_slug` / `set_cached_profile_slug` **stay**)
- Modify: `crates/temper-cli/src/actions/runtime.rs` (`with_arc_client` at line 144)

- [ ] **Step 1: Run `cargo make check` and collect dead-code warnings**

```bash
cargo make check 2>&1 | rg 'dead.code|never.used|is never read' | sort -u
```

This produces the authoritative list. Use it as the deletion target.

- [ ] **Step 2: Verify each candidate is truly unused**

For each item the warning surfaces, run a final grep to confirm no surviving caller exists:

```bash
rg '\bscan_rows\b' --type rust
rg '\bparse_row\b' --type rust
rg '\bsort_rows\b' --type rust
rg '\bfilter_rows\b' --type rust
rg '\brender_list\b' --type rust
rg '\bRenderListParams\b' --type rust
rg '\bListFilters\b' --type rust
rg '\bfind_resource\b' --type rust
rg '\bFindableResource\b' --type rust
rg '\bResolvedResource\b' --type rust
rg '\bwith_arc_client\b' --type rust
```

For each, the only hits should be the definition itself (or peer items in the same dead chain). If a real caller exists, do not delete — note it and skip.

- [ ] **Step 3: Delete the dead items**

Walk each warning-surfaced item top-down through the candidate files. Delete the item plus any `use` lines that become unused. For `lookup.rs`, keep `cached_profile_slug` and `set_cached_profile_slug` — verify via `rg 'cached_profile_slug' --type rust` (should show callers in `actions/runtime.rs`).

For `commands/resource.rs`, the `ResourceRow` type — there are two: a `pub struct ResourceRow` at module scope used by cloud-mode `list` (KEEP) and possibly an inner helper struct (DELETE if unused). Read carefully before deleting.

- [ ] **Step 4: Run `cargo make check`**

Expected: 0 errors, 0 warnings.

- [ ] **Step 5: Run the full Rust test suite**

Run: `cargo nextest run --workspace`
Expected: all pass.

- [ ] **Step 6: Commit**

```bash
git add -A
git commit -m "cloud-only(ch4): sweep pub-orphans (lookup::find_resource, render_list, with_arc_client, etc.)"
```

---

## Task 9: Full verification + consolidated review

Run all four verification tiers locally, then dispatch a fresh opus reviewer subagent for consolidated review. Address any findings inline; PR stays unopened (PR B accumulates Chunks 3–8).

**Files:** none modified in this task except possibly review-followup fixes.

- [ ] **Step 1: Tier 1 — `cargo make check`**

Run: `cargo make check`
Expected: 0 errors, 0 warnings.

- [ ] **Step 2: Tier 2 — workspace unit + integration tests**

Run: `cargo nextest run --workspace`
Expected: 100% pass.

- [ ] **Step 3: Tier 3 — e2e with `test-db`**

Run: `cargo make test-e2e`
Expected: 100% pass.

- [ ] **Step 4: Tier 4 — e2e with `test-db,test-embed`**

Run: `cargo nextest run --manifest-path tests/e2e/Cargo.toml --features test-db,test-embed`
Expected: 100% pass. Note: per Chunk 3's session note, the embed tier has known environmental flakes under concurrent runs; serialize if needed.

- [ ] **Step 5: Verify the acceptance criteria**

```bash
# (1) vault_backend/ is gone entirely
test ! -d crates/temper-cli/src/vault_backend/ && echo "OK: directory gone" || echo "FAIL"

# (2) Surface::CliLocalVault is gone
rg 'CliLocalVault' --type rust && echo "FAIL: still referenced" || echo "OK: variant gone"

# (3) DomainEvent::VaultFile* are gone
rg 'VaultFileWritten|VaultFileRemoved|VaultManifestUpdated' --type rust && echo "FAIL" || echo "OK"

# (4) show_edges no longer depends on manifest_io
rg 'manifest_io' crates/temper-cli/src/commands/resource.rs && echo "FAIL" || echo "OK"

# (5) Plan-gate question is documented in the plan's preamble: ✓ (this file's "Plan gate — resolution" section)
```

All five should print OK.

- [ ] **Step 6: Dispatch the consolidated opus review**

Dispatch a fresh opus subagent (general-purpose) with this prompt:

```
You are reviewing the implementation of Chunk 4 of the cloud-only-vault
deprecation on the branch `jct/cloud-only-vault-pr-b`. Inspect the
commits added in this chunk (since the predecessor session, which ended
at commit 322cdeb "cloud-only(ch3): review followups").

The plan is at:
  docs/superpowers/plans/2026-05-23-cloud-only-vault-chunk4-delete-vault-backend.md

The plan resolves an explicit plan-gate question by deferring `manifest_io`
deletion to Chunk 5 — this is INTENDED. Do not flag the surviving
`manifest_io` / `Manifest` / `temper-core::types::{manifest,sync}` symbols
as misses.

Review for:
1. Correctness — was each task implemented as specified? Does the test
   coverage validate the new behavior?
2. Code quality — match against the project's CLAUDE.md rules: typed
   structs over inline JSON, service layer owns SQL (N/A here),
   params structs over too-many-args, no premature backward-compat shims,
   patterns match siblings.
3. Plan-gate consistency — did Tasks 2 + 3 (research.rs inline,
   show_edges fix) keep the workspace bisectable, or did intermediate
   commits leave the build broken?
4. Pub-orphan sweep completeness — are there still dead pub items in
   temper-cli that `cargo make check` is silent on only because of the
   pub-at-lib trick? Specifically check `lookup.rs`, `commands/resource.rs`,
   `actions/runtime.rs`, and `projection.rs` for items that no caller
   uses.
5. Test repointing — did Task 7 leave any test asserting the absence
   of `path` populated, where the old test asserted its presence?
   That's a sign of a missed migration.

Return READY / READY_WITH_FOLLOWUPS / NEEDS_CHANGES. List findings
by severity (critical / important / minor) with file:line refs.
```

- [ ] **Step 7: Address findings inline**

If the review returns READY_WITH_FOLLOWUPS or NEEDS_CHANGES, address critical/important findings in a final review-followup commit. Minor findings (docstring nits, naming) can also fold into the same commit. Per Chunk 3 precedent, this lands as a single commit, not per-finding.

```bash
git add -A
git commit -m "cloud-only(ch4): review followups"
```

- [ ] **Step 8: Save session note**

Pipe the session summary via stdin:

```bash
cat <<'EOF' | temper resource create --type session --title "Cloud-only vault Chunk 4 landed (vault_backend/ deleted, variants removed)" --context temper
## Goal
(describe goal here)

## What Happened
(describe execution and surprises)

## Decisions
(describe key decisions, especially plan-gate resolution)

## Connections
- Branch (no PR yet): jct/cloud-only-vault-pr-b
- Plan: docs/superpowers/plans/2026-05-23-cloud-only-vault-chunk4-delete-vault-backend.md
- Predecessor session: 2026-05-23-cloud-only-vault-chunk-3-landed-on-pr-b-branch-16-plan-tasks-2-inserted
- Spec: docs/superpowers/specs/2026-05-21-cloud-only-vault-deprecation-design.md

## Next Steps
- Chunk 5: delete sync engine + push + manifest_io + temper-core::types::{manifest,sync}
- Project memory: project_cloud_only_vault_direction
EOF
```

- [ ] **Step 9: Mark the task done**

```bash
temper resource update 2026-05-23-cloud-only-vault-chunk-4-delete-vaultbackend-manifest-and-surface-clilocalvault --type task --context temper --stage done
```

---

## Self-Review

**Spec coverage:**

| Spec acceptance criterion | Plan coverage |
|--------------------------|---------------|
| `vault_backend/` gone entirely | Task 5 |
| `Surface::CliLocalVault` gone | Task 6 |
| `DomainEvent::VaultFile*` gone (or kept with justification) | Task 7 |
| `show_edges` no longer manifest-dependent | Task 3 |
| Plan-gate question explicitly resolved | Preamble |
| All 4 verification tiers green | Task 9 (Steps 1–4) |
| No PR opened | Implicit; PR B accumulates Chunks 3–8 |
| `manifest_io` gone (spec literal) | **Deferred to Chunk 5** — documented above |
| `Manifest` symbol gone (spec literal) | **Deferred to Chunk 5** — documented above |
| `temper-core` manifest/sync module gone | **Deferred to Chunk 5** — documented above |

**Plan-gate consistency check:** Tasks 2 and 3 cut the only two non-`vault_backend/` callers of the deleting machinery before Task 5 deletes the directory. Tasks 6 + 7 remove the temper-core variant definitions after their only constructors are gone. Each task ends with `cargo make check` green.

**Type-consistency check:**
- `write_research_inline` (Task 2) returns `InlineWriteResult { resource_id: Uuid, abs_path: PathBuf, rel_path: String }` — matches the shape `per_doctype::WriteResult` already exposes, just inlined.
- `show_edges` signature changes to `(owner, context, doc_type, slug, format)` (Task 3) — caller updated at the same time; no other callers exist.
- `lookup::cached_profile_slug` and `set_cached_profile_slug` survive Task 8 — referenced explicitly in the task description.

**Placeholder scan:** No `TBD`, no "TODO", no "implement similar to Task N". Code blocks present for every behavior-changing step. Task 8 intentionally describes a *sweep* rather than listing exact item-by-item edits because the source-of-truth is `cargo make check`'s output — listed candidate items are starting points to verify, not a fixed list.

**Notes for the implementer subagent dispatch (sonnet recommended for Tasks 1–8; opus only for Task 9 Step 6):**

- Include `SG-1`, `SG-2`, `SG-5`, `SG-6`, `SG-10` from `subagent-guidance.md` verbatim in every dispatch prompt.
- Include the project fundamentals references on typed structs, service-layer SQL ownership, params structs, and "no premature backward-compat" (relevant because this chunk is pure deletion — no legacy shims).
- For Tasks 2–3 (TDD): include the red→green→refactor cadence explicitly.
- For Tasks 5–8 (deletion): emphasize "verify before deleting" — every removed item must be confirmed unused by grep first.
