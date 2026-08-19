# Unify `temper add` and `temper import` Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Merge `temper add` and `temper import` into a single `temper add` command that always writes vault files.

**Architecture:** Replace the `Add` CLI variant with `Import`'s args, rebuild `add.rs` from `import_cmd.rs` logic + URL-to-vault handling, delete `import_cmd.rs` entirely.

**Tech Stack:** Rust, clap, tokio, temper-cli/temper-client/temper-ingest

**Spec:** `docs/superpowers/specs/2026-04-04-unify-temper-add-and-import-design.md`

---

### Task 1: Update CLI definition and router

Replace the `Commands::Add` variant with import's args and remove `Commands::Import`.

**Files:**
- Modify: `crates/temper-cli/src/cli.rs:105-151`
- Modify: `crates/temper-cli/src/main.rs:329-355`

- [ ] **Step 1: Update `Commands::Add` in cli.rs**

Replace lines 105-151 (both `Add` and `Import` variants) with a single `Add` variant that has import's full arg set plus URL support:

```rust
    /// Add a file, URL, or directory to the vault
    Add {
        /// File path, directory path, URL, or resource UUID (for promotion)
        path: String,
        /// Add all files in a directory
        #[arg(long)]
        dir: bool,
        /// Context name (required for file imports, unless --doc-type auto)
        #[arg(long)]
        context: Option<String>,
        /// Doc type — use "auto" to read from each file's YAML frontmatter
        #[arg(long, default_value = "resource")]
        doc_type: String,
        /// Output format
        #[arg(long, default_value = "text")]
        format: String,
        /// Override size guardrails
        #[arg(long)]
        force: bool,
        /// Preview what would be added without uploading
        #[arg(long)]
        dry_run: bool,
        /// Regex pattern to exclude files (matched against filename)
        #[arg(long)]
        ignore: Option<String>,
    },
```

Note: `--context` changes from `String` (required) to `Option<String>` (optional). `--dry-run` and `--ignore` are new.

- [ ] **Step 2: Update the router in main.rs**

Replace lines 329-355 (both `Add` and `Import` match arms) with a single arm:

```rust
        Commands::Add {
            path,
            dir,
            context,
            doc_type,
            format,
            force,
            dry_run,
            ignore,
        } => commands::add::run(
            &path,
            dir,
            context.as_deref(),
            &doc_type,
            &format,
            force,
            dry_run,
            ignore.as_deref(),
        ),
```

- [ ] **Step 3: Verify it compiles (expect errors in add.rs — that's fine)**

Run: `cargo check -p temper-cli 2>&1 | head -30`
Expected: Errors about `commands::add::run` signature mismatch and `commands::import_cmd` not found. This is expected — we fix these in the next tasks.

- [ ] **Step 4: Commit**

```bash
git add crates/temper-cli/src/cli.rs crates/temper-cli/src/main.rs
git commit -m "refactor(cli): unify Add/Import CLI variants into single Add command"
```

---

### Task 2: Rebuild add.rs from import_cmd.rs

This is the main task. Replace `add.rs` with `import_cmd.rs`'s logic, add URL-to-vault support, and update all messaging from "imported" to "added".

**Files:**
- Rewrite: `crates/temper-cli/src/commands/add.rs`
- Delete: `crates/temper-cli/src/commands/import_cmd.rs`
- Modify: `crates/temper-cli/src/commands/mod.rs:7`

**Important context for the implementer:**

- `import_cmd.rs` is 915 lines. `add.rs` is 626 lines. The new `add.rs` will be roughly the size of `import_cmd.rs` plus ~60 lines for URL handling.
- `import_cmd.rs` already calls `add::DirectoryConfig`, `add::collect_files`, and `add::preflight_check` — these directory utilities stay in `add.rs`.
- The old `add.rs` URL handling (`run_url` at lines 108-147) calls `ingest::ingest_url()` which returns `(resource, extracted_content)` but never writes a vault file. The new code must write a vault file after URL ingest.
- `import_cmd.rs` has a re-export line `pub use ingest::{build_frontmatter, build_vault_path};` — this was for backward compat but `pull.rs` imports from `ingest` directly, so it's not needed.

- [ ] **Step 1: Copy import_cmd.rs to add.rs**

Back up the old `add.rs` content mentally (we need `DirectoryConfig`, `collect_files`, `preflight_check`, and the URL handling pattern from `run_url`), then replace `add.rs` with `import_cmd.rs`'s content.

- [ ] **Step 2: Update the module doc comment**

Replace the module doc at the top:

```rust
//! `temper add` — add a file, URL, or directory to the vault.
//!
//! Four flows:
//! 1. **URL**: fetch content, extract to markdown, write vault file, upload.
//! 2. **Promotion**: given a resource UUID, fetch from cloud, write vault file,
//!    register in manifest.
//! 3. **Directory** (`--dir`): walk directory, apply filters, batch import all files.
//! 4. **Single file**: extract, write vault file with frontmatter, upload.
//!    Supports `--doc-type auto` to derive metadata from YAML frontmatter.
```

- [ ] **Step 3: Update imports**

The new `add.rs` needs these imports (merge of both files):

```rust
use std::path::{Path, PathBuf};

use uuid::Uuid;

use crate::actions::{ingest, runtime};
use crate::error::TemperError;
use crate::format::OutputFormat;
use crate::output;
```

Note: `Path` is needed for directory utilities (was in old `add.rs`). `Uuid` is needed for promotion (was in `import_cmd.rs`).

- [ ] **Step 4: Update `run()` entry point**

The `run()` function must match the new signature from the router and add URL detection before UUID detection:

```rust
#[expect(
    clippy::too_many_arguments,
    reason = "thin CLI entry point — arguments map 1:1 to clap flags"
)]
pub fn run(
    path: &str,
    dir: bool,
    context: Option<&str>,
    doc_type: &str,
    format: &str,
    force: bool,
    dry_run: bool,
    ignore: Option<&str>,
) -> crate::error::Result<()> {
    // Compile --ignore pattern up front so we fail fast on bad regex.
    let ignore_re = ignore
        .map(|pat| {
            regex::Regex::new(pat)
                .map_err(|e| TemperError::Config(format!("invalid --ignore pattern: {e}")))
        })
        .transpose()?;

    // URL detection — must come before UUID check (URLs aren't UUIDs).
    if path.starts_with("http://") || path.starts_with("https://") {
        let context = context.ok_or_else(|| {
            TemperError::Config("--context is required for URL imports".to_string())
        })?;
        if dry_run {
            output::plain(format!("dry-run: would add {path}"));
            return Ok(());
        }
        return run_url(path, context, doc_type, format);
    }

    // Check if path is a UUID -> promotion flow
    if let Ok(resource_id) = Uuid::parse_str(path) {
        if dry_run {
            output::plain(format!("dry-run: would promote resource {resource_id}"));
            return Ok(());
        }
        return promote_resource(resource_id, context, doc_type, format);
    }

    // File/directory: --context is required unless --doc-type auto
    let is_auto = doc_type == "auto";
    if !is_auto && context.is_none() {
        return Err(TemperError::Config(
            "--context is required for file imports (or use --doc-type auto)".to_string(),
        ));
    }

    if dir {
        return run_directory(
            path,
            context,
            doc_type,
            format,
            force,
            dry_run,
            ignore_re.as_ref(),
        );
    }

    run_single_file(path, context, doc_type, format, dry_run)
}
```

- [ ] **Step 5: Add URL-to-vault function**

Add this after the single-file functions. It fetches URL, uploads, then writes a vault file (the key new behavior):

```rust
// ---------------------------------------------------------------------------
// URL ingest (with vault file)
// ---------------------------------------------------------------------------

fn run_url(
    url: &str,
    context: &str,
    doc_type: &str,
    format: &str,
) -> crate::error::Result<()> {
    let fmt = OutputFormat::parse(format);

    if fmt == OutputFormat::Text {
        output::progress("  Fetching... ");
    }

    let (rt, client) = runtime::build_runtime_and_client()?;
    rt.block_on(runtime::ensure_profile(&client))?;

    let (resource, extracted_content) = rt.block_on(async {
        ingest::ingest_url(&client, url, context, doc_type, Some("added"))
            .await
            .map_err(|e| TemperError::Api(e.to_string()))
    })?;

    if fmt == OutputFormat::Text {
        output::plain(format!(
            "done ({} KB markdown)",
            extracted_content.len() / 1024
        ));
    }

    let vault_root = crate::config::resolve_vault(None)?;
    let slug = ingest::slug_from_title(&resource.title);
    let slug = ingest::dedup_vault_slug(&vault_root, context, doc_type, &slug);

    let vault_path = ingest::write_vault_file_and_register(
        &vault_root,
        context,
        doc_type,
        &slug,
        &resource,
        &extracted_content,
        Some(url),
        None,
    )?;

    emit_event(fmt, url, &resource, &vault_path);
    Ok(())
}
```

- [ ] **Step 6: Rename import-specific functions**

Throughout the file, rename for consistency:
- `run_single_import` → `run_single_file`
- `run_single_auto_import` → `run_single_auto_file`
- `run_directory_import` → `run_directory`
- `import_single_auto_file` → `add_single_auto_file`
- `emit_import_event` → `emit_event`

In `emit_event`, change the text output from `"Imported:"` to `"Added:"` and the JSON event name from `"import"` to `"add"`.

In `run_directory`'s summary output, change `"{added} imported"` to `"{added} added"`.

In `ingest_file` calls, change `Some("imported")` to `Some("added")` for the resource_mode parameter.

- [ ] **Step 7: Keep directory utilities in place**

The `DirectoryConfig` struct, `collect_files()`, and `preflight_check()` functions from old `add.rs` must remain in the new `add.rs` since `run_directory` calls them directly (no longer cross-module). Copy them from old `add.rs` exactly as-is — they're at lines 153-248 of the original file.

- [ ] **Step 8: Remove import_cmd.rs and update mod.rs**

Delete `crates/temper-cli/src/commands/import_cmd.rs`.

In `crates/temper-cli/src/commands/mod.rs`, remove line 7:
```rust
pub mod import_cmd;
```

- [ ] **Step 9: Update ingest.rs doc comment**

In `crates/temper-cli/src/actions/ingest.rs` line 4, change:
```rust
//! `commands::add`, `commands::import_cmd`, and `commands::pull`. Command
```
to:
```rust
//! `commands::add` and `commands::pull`. Command
```

- [ ] **Step 10: Verify compilation**

Run: `cargo check -p temper-cli`
Expected: Clean compilation with no errors.

- [ ] **Step 11: Commit**

```bash
git add -A crates/temper-cli/src/commands/add.rs crates/temper-cli/src/commands/mod.rs crates/temper-cli/src/actions/ingest.rs
git rm crates/temper-cli/src/commands/import_cmd.rs
git commit -m "refactor(cli): rebuild add.rs from import logic, add URL-to-vault, delete import_cmd"
```

---

### Task 3: Merge and update tests

The new `add.rs` needs tests from both old files, updated for the new unified signature.

**Files:**
- Modify: `crates/temper-cli/src/commands/add.rs` (test module)

- [ ] **Step 1: Write the unified test module**

Replace the test module at the bottom of `add.rs` with tests from both old files, updated for the new `run()` signature:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    // --- URL detection ---

    #[test]
    fn url_http_routes_to_url_handler() {
        let err = run(
            "http://example.com/doc.pdf",
            false,
            Some("work"),
            "note",
            "text",
            false,
            false,
            None,
        )
        .unwrap_err();
        assert!(
            !err.to_string().contains("not yet implemented"),
            "URL should be routed, not rejected: {err}"
        );
    }

    #[test]
    fn url_https_routes_to_url_handler() {
        let err = run(
            "https://example.com/paper.md",
            false,
            Some("work"),
            "note",
            "text",
            false,
            false,
            None,
        )
        .unwrap_err();
        assert!(
            !err.to_string().contains("not yet implemented"),
            "URL should be routed, not rejected: {err}"
        );
    }

    // --- UUID detection ---

    #[test]
    fn uuid_path_detected_as_uuid() {
        let uuid_str = "12345678-1234-1234-1234-123456789abc";
        assert!(
            uuid::Uuid::parse_str(uuid_str).is_ok(),
            "should parse as UUID: {uuid_str}"
        );
    }

    #[test]
    fn file_path_not_detected_as_uuid() {
        let file_path = "/home/user/documents/my-notes.pdf";
        assert!(
            uuid::Uuid::parse_str(file_path).is_err(),
            "file path should not parse as UUID: {file_path}"
        );
    }

    #[test]
    fn relative_file_path_not_detected_as_uuid() {
        let file_path = "notes/my-document.md";
        assert!(
            uuid::Uuid::parse_str(file_path).is_err(),
            "relative path should not parse as UUID: {file_path}"
        );
    }

    // --- run() integration ---

    #[test]
    fn run_with_uuid_path_without_vault_fails_gracefully() {
        let result = run(
            "12345678-1234-1234-1234-123456789abc",
            false,
            None,
            "resource",
            "text",
            false,
            false,
            None,
        );
        assert!(result.is_err());
    }

    #[test]
    fn run_file_without_context_returns_error() {
        let result = run(
            "/tmp/some-file.md",
            false,
            None,
            "resource",
            "text",
            false,
            false,
            None,
        );
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("--context is required"));
    }

    #[test]
    fn run_auto_without_context_does_not_require_context_upfront() {
        let result = run(
            "/tmp/nonexistent.md",
            false,
            None,
            "auto",
            "text",
            false,
            false,
            None,
        );
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            !err.contains("--context is required"),
            "auto mode should not require --context upfront: {err}"
        );
    }

    #[test]
    fn url_without_context_returns_error() {
        let result = run(
            "https://example.com/doc.pdf",
            false,
            None,
            "resource",
            "text",
            false,
            false,
            None,
        );
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("--context is required"));
    }

    // --- Nonexistent file ---

    #[test]
    fn nonexistent_file_returns_error() {
        let result = run(
            "/tmp/does-not-exist-xyz-12345.md",
            false,
            Some("work"),
            "note",
            "text",
            false,
            false,
            None,
        );
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("file not found"));
    }

    // --- Directory mode ---

    #[test]
    fn collect_files_respects_max_depth() {
        use std::fs;
        use tempfile::TempDir;

        let dir = TempDir::new().unwrap();
        let root = dir.path();

        fs::write(root.join("top.md"), "# Top").unwrap();

        let sub = root.join("sub");
        fs::create_dir(&sub).unwrap();
        fs::write(sub.join("inner.md"), "# Inner").unwrap();

        let deep = sub.join("deep");
        fs::create_dir(&deep).unwrap();
        fs::write(deep.join("deep.md"), "# Deep").unwrap();

        let deeper = deep.join("deeper");
        fs::create_dir(&deeper).unwrap();
        fs::write(deeper.join("too_deep.md"), "# Too Deep").unwrap();

        let config = DirectoryConfig {
            max_depth: 2,
            ..DirectoryConfig::default()
        };
        let files = collect_files(root, &config).unwrap();
        let names: Vec<_> = files
            .iter()
            .filter_map(|p| p.file_name().and_then(|n| n.to_str()))
            .collect();

        assert!(names.contains(&"top.md"), "top.md not found in {names:?}");
        assert!(names.contains(&"inner.md"), "inner.md not found in {names:?}");
        assert!(!names.contains(&"deep.md"), "deep.md should be excluded");
        assert!(!names.contains(&"too_deep.md"), "too_deep.md should be excluded");
    }

    #[test]
    fn collect_files_filters_by_extension() {
        use std::fs;
        use tempfile::TempDir;

        let dir = TempDir::new().unwrap();
        let root = dir.path();

        fs::write(root.join("doc.md"), "# Markdown").unwrap();
        fs::write(root.join("notes.txt"), "plain text").unwrap();
        fs::write(root.join("image.png"), "binary").unwrap();
        fs::write(root.join("data.csv"), "a,b,c").unwrap();
        fs::write(root.join("page.html"), "<html/>").unwrap();

        let config = DirectoryConfig::default();
        let files = collect_files(root, &config).unwrap();
        let names: Vec<_> = files
            .iter()
            .filter_map(|p| p.file_name().and_then(|n| n.to_str()))
            .collect();

        assert!(names.contains(&"doc.md"), "doc.md should be included");
        assert!(names.contains(&"notes.txt"), "notes.txt should be included");
        assert!(names.contains(&"page.html"), "page.html should be included");
        assert!(!names.contains(&"image.png"), "image.png should be excluded");
        assert!(!names.contains(&"data.csv"), "data.csv should be excluded");
    }

    #[test]
    fn preflight_check_rejects_oversized_directory() {
        use std::fs;
        use tempfile::TempDir;

        let dir = TempDir::new().unwrap();
        let root = dir.path();

        fs::write(root.join("big.md"), "# Big file with lots of content").unwrap();

        let config = DirectoryConfig {
            max_total_bytes: 1,
            ..DirectoryConfig::default()
        };

        let err = preflight_check(root, &config).unwrap_err();
        assert!(
            err.to_string().contains("exceeds limit"),
            "expected 'exceeds limit' in: {err}"
        );
    }

    #[test]
    fn preflight_check_accepts_within_limit() {
        use std::fs;
        use tempfile::TempDir;

        let dir = TempDir::new().unwrap();
        let root = dir.path();

        fs::write(root.join("small.md"), "# Small").unwrap();

        let config = DirectoryConfig::default();
        let files = preflight_check(root, &config).unwrap();
        let names: Vec<_> = files
            .iter()
            .filter_map(|p| p.file_name().and_then(|n| n.to_str()))
            .collect();
        assert!(names.contains(&"small.md"));
    }

    #[test]
    fn run_directory_errors_on_non_directory() {
        let err = run(
            "/tmp/not-a-real-directory-xyz-12345",
            true,
            Some("work"),
            "note",
            "text",
            false,
            false,
            None,
        )
        .unwrap_err();
        assert!(
            err.to_string().contains("not a directory")
                || err.to_string().contains("exceeds limit")
                || err.to_string().contains("No matching"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn dry_run_url_does_not_upload() {
        let result = run(
            "https://example.com/doc.pdf",
            false,
            Some("work"),
            "resource",
            "text",
            false,
            true,
            None,
        );
        // dry-run should succeed without network access
        assert!(result.is_ok());
    }

    #[test]
    fn dry_run_uuid_does_not_promote() {
        let result = run(
            "12345678-1234-1234-1234-123456789abc",
            false,
            None,
            "resource",
            "text",
            false,
            true,
            None,
        );
        assert!(result.is_ok());
    }
}
```

- [ ] **Step 2: Run the tests**

Run: `cargo nextest run -p temper-cli`
Expected: All tests pass. The URL/UUID tests that hit the network should fail with auth/network errors (not "not yet implemented"). The directory tests should pass cleanly.

- [ ] **Step 3: Commit**

```bash
git add crates/temper-cli/src/commands/add.rs
git commit -m "test(cli): unified test suite for temper add"
```

---

### Task 4: Update README and run full checks

**Files:**
- Modify: `README.md:88-89, 138-139`

- [ ] **Step 1: Update README quickstart**

At line 88-89, change:
```markdown
# Import your docs — temper extracts markdown and indexes it
temper import --context myapp --dir ~/projects/myapp/docs
```
to:
```markdown
# Add your docs — temper extracts markdown and indexes it
temper add --context myapp --dir ~/projects/myapp/docs
```

- [ ] **Step 2: Update README command table**

At lines 138-139, replace both rows:
```markdown
| `temper import <path>` | Import a file into the vault (managed, frontmatter, sync-ready) |
| `temper add <path>` | Add a file to the cloud (searchable, pullable, not vault-managed) |
```
with:
```markdown
| `temper add <path>` | Add a file, URL, or directory to the vault (managed, frontmatter, sync-ready) |
```

- [ ] **Step 3: Run full quality checks**

Run: `cargo make check`
Expected: All fmt, clippy, and lint checks pass.

- [ ] **Step 4: Run full test suite**

Run: `cargo make test`
Expected: All unit tests pass.

- [ ] **Step 5: Commit**

```bash
git add README.md
git commit -m "docs: update README for unified temper add command"
```
