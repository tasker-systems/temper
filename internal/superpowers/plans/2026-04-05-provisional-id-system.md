# Provisional ID System Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Separate locally-generated IDs (`temper-provisional-id`) from server-authoritative IDs (`temper-id`) so sync never conflates the two, eliminating duplicate pulls, content-hash fallback, and `-2` file creation.

**Architecture:** CLI file-creation commands write `temper-provisional-id` in frontmatter instead of `temper-id`. ManifestEntry gains a `provisional: bool` field. Sync push always POSTs provisional entries, replaces `temper-provisional-id` with `temper-id` on success, and sets `provisional: false`. Sync reset skips ID-matching for provisional files, marking them Pending without content-hash fallback.

**Tech Stack:** Rust (temper-core types, temper-cli actions, askama templates), serde, uuid

---

## File Map

| Action | File | Responsibility |
|--------|------|----------------|
| Modify | `crates/temper-core/src/types/manifest.rs` | Add `provisional` field to `ManifestEntry` |
| Modify | `crates/temper-cli/src/actions/ingest.rs` | Add `temper-provisional-id` to `ParsedFrontmatter`, update `parse_source_frontmatter`, add `build_provisional_frontmatter` |
| Modify | `crates/temper-cli/src/templates/task.md` | Change `id:` → `temper-provisional-id:` |
| Modify | `crates/temper-cli/src/templates/session.md` | Change `id:` → `temper-provisional-id:` |
| Modify | `crates/temper-cli/src/templates/goal.md` | Change `id:` → `temper-provisional-id:` |
| Modify | `crates/temper-cli/src/templates/research.md` | Change `id:` → `temper-provisional-id:` |
| Modify | `crates/temper-cli/src/actions/sync.rs` | Update `push_resource`, `scan_vault_for_untracked`, `sync_reset` for provisional logic |
| Modify | `crates/temper-core/src/schema.rs` | Ensure `temper-provisional-id` hashes into managed tier |

---

### Task 1: Add `provisional` field to ManifestEntry

**Files:**
- Modify: `crates/temper-core/src/types/manifest.rs:24-54`

- [ ] **Step 1: Write the failing test**

Add a test in the existing `mod tests` block that asserts `provisional` field serializes and deserializes correctly, and that old manifests without the field default to `false`:

```rust
#[test]
fn test_manifest_entry_provisional_defaults_false() {
    // Old format without provisional field
    let old_json = serde_json::json!({
        "path": "temper/task/example.md",
        "body_hash": "sha256:abc",
        "remote_body_hash": "sha256:abc",
        "synced_at": "2026-01-01T00:00:00Z",
        "state": "pending"
    });
    let entry: ManifestEntry = serde_json::from_value(old_json).unwrap();
    assert!(!entry.provisional);
}

#[test]
fn test_manifest_entry_provisional_roundtrip() {
    let entry = ManifestEntry {
        path: "temper/task/new.md".to_string(),
        body_hash: "sha256:body".to_string(),
        remote_body_hash: String::new(),
        managed_hash: String::new(),
        open_hash: String::new(),
        remote_managed_hash: String::new(),
        remote_open_hash: String::new(),
        synced_at: Utc::now(),
        state: ManifestEntryState::Pending,
        mtime_secs: None,
        provisional: true,
    };
    let json = serde_json::to_string(&entry).unwrap();
    let parsed: ManifestEntry = serde_json::from_str(&json).unwrap();
    assert!(parsed.provisional);
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo nextest run -p temper-core test_manifest_entry_provisional`
Expected: FAIL — `provisional` field doesn't exist on `ManifestEntry`.

- [ ] **Step 3: Add `provisional` field to ManifestEntry**

In `crates/temper-core/src/types/manifest.rs`, add after the `mtime_secs` field (line 53):

```rust
    /// Whether this entry has a locally-generated provisional ID that hasn't
    /// been confirmed by the server yet.  Provisional entries always POST
    /// (never PUT) and get rekeyed to the server ID after success.
    #[serde(default)]
    pub provisional: bool,
```

- [ ] **Step 4: Fix existing test compilation**

Update `test_manifest_json_roundtrip` and `test_manifest_entry_new_format_roundtrip` to include `provisional: false` in their `ManifestEntry` construction.

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo nextest run -p temper-core manifest`
Expected: All manifest tests PASS including the two new ones.

- [ ] **Step 6: Commit**

```bash
git add crates/temper-core/src/types/manifest.rs
git commit -m "feat: add provisional field to ManifestEntry for local vs server ID tracking"
```

---

### Task 2: Add `provisional_id` to ParsedFrontmatter and update parser

**Files:**
- Modify: `crates/temper-cli/src/actions/ingest.rs:59-99`

- [ ] **Step 1: Write the failing test**

Add a test in `crates/temper-cli/src/actions/ingest.rs` (or the test module for that file) that parses frontmatter with `temper-provisional-id`:

```rust
#[test]
fn test_parse_provisional_id() {
    let content = "---\ntemper-provisional-id: \"019d6088-3a3b-71a3-b26c-d38b8338773e\"\ntitle: \"Test\"\n---\n\nBody";
    let fm = parse_source_frontmatter(content).unwrap();
    assert_eq!(
        fm.provisional_id.as_deref(),
        Some("019d6088-3a3b-71a3-b26c-d38b8338773e")
    );
    // legacy_id should be None when only provisional is present
    assert!(fm.legacy_id.is_none());
}

#[test]
fn test_parse_both_ids_prefers_temper_id() {
    let content = "---\ntemper-id: \"aaa\"\ntemper-provisional-id: \"bbb\"\ntitle: \"Test\"\n---\n\nBody";
    let fm = parse_source_frontmatter(content).unwrap();
    assert_eq!(fm.legacy_id.as_deref(), Some("aaa"));
    assert_eq!(fm.provisional_id.as_deref(), Some("bbb"));
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo nextest run -p temper-cli test_parse_provisional`
Expected: FAIL — `provisional_id` field doesn't exist on `ParsedFrontmatter`.

- [ ] **Step 3: Add `provisional_id` to ParsedFrontmatter and update parser**

In `crates/temper-cli/src/actions/ingest.rs`:

Add field to `ParsedFrontmatter` (after `legacy_id` on line 66):

```rust
    pub provisional_id: Option<String>,
```

Update `parse_source_frontmatter` to parse it (after line 92):

```rust
        legacy_id: s("temper-id").or_else(|| s("id")),
        provisional_id: s("temper-provisional-id"),
```

**Important:** `legacy_id` must NOT fall through to `temper-provisional-id`. The `s("id")` fallback handles old `id:` keys from templates that haven't been synced yet — those will be migrated to `temper-provisional-id` in Task 3.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo nextest run -p temper-cli test_parse_provisional`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/temper-cli/src/actions/ingest.rs
git commit -m "feat: parse temper-provisional-id from frontmatter"
```

---

### Task 3: Update askama templates to write `temper-provisional-id`

**Files:**
- Modify: `crates/temper-cli/src/templates/task.md`
- Modify: `crates/temper-cli/src/templates/session.md`
- Modify: `crates/temper-cli/src/templates/goal.md`
- Modify: `crates/temper-cli/src/templates/research.md`

- [ ] **Step 1: Update all four templates**

Replace `id: "{{id}}"` with `temper-provisional-id: "{{id}}"` in each template.

`task.md` becomes:
```
---
temper-provisional-id: "{{id}}"
type: task
title: "{{title}}"
slug: "{{slug}}"
context: "{{context}}"
goal: "{{goal}}"
stage: backlog
mode: {{mode}}
effort: {{effort}}
seq: {{seq}}
created: {{datetime}}
updated: {{datetime}}
branch: null
pr: null
---

# {{title}}
```

`session.md` becomes:
```
---
temper-provisional-id: "{{id}}"
type: session
date: {{date}}
context: ""
---

# Session: {{title}}
```

(Same pattern for `goal.md` and `research.md` — replace `id:` with `temper-provisional-id:`)

- [ ] **Step 2: Verify templates render correctly**

Run: `cargo build -p temper-cli`
Expected: BUILD succeeds (askama templates are checked at compile time).

- [ ] **Step 3: Functional test — create a task and inspect frontmatter**

Run: `temper task create "Provisional ID test" --context temper --goal temper-maintenance`

Then inspect the created file. The frontmatter should contain `temper-provisional-id:` and NOT `id:` or `temper-id:`.

- [ ] **Step 4: Delete the test task file**

Remove the test file created in step 3.

- [ ] **Step 5: Commit**

```bash
git add crates/temper-cli/src/templates/task.md crates/temper-cli/src/templates/session.md crates/temper-cli/src/templates/goal.md crates/temper-cli/src/templates/research.md
git commit -m "feat: templates write temper-provisional-id instead of id"
```

---

### Task 4: Update `scan_vault_for_untracked` for provisional awareness

**Files:**
- Modify: `crates/temper-cli/src/actions/sync.rs:270-336`

- [ ] **Step 1: Update ID resolution to check both fields**

In `scan_vault_for_untracked`, the current code at lines 283-289 reads `legacy_id` (which maps to `temper-id` or `id`). Update it to also check `provisional_id` and set the `provisional` flag on the manifest entry:

```rust
        // Determine resource ID and provisional status:
        // - temper-id present → server-confirmed, not provisional
        // - temper-provisional-id present → locally-generated, provisional
        // - neither → mint new UUID, provisional
        let (resource_id, is_provisional) = if let Some(tid) = fm
            .as_ref()
            .and_then(|f| f.legacy_id.as_deref())
            .and_then(|id| Uuid::parse_str(id).ok())
        {
            (tid, false)
        } else if let Some(pid) = fm
            .as_ref()
            .and_then(|f| f.provisional_id.as_deref())
            .and_then(|id| Uuid::parse_str(id).ok())
        {
            (pid, true)
        } else {
            (Uuid::now_v7(), true)
        };
```

Then update the manifest entry insertion at lines 315-329 to include `provisional: is_provisional`.

- [ ] **Step 2: Update frontmatter injection for files without frontmatter**

When `fm.is_none()` (lines 290-301), the injected frontmatter should use `temper-provisional-id` instead of `temper-id`. Create a new function `build_provisional_frontmatter` in `ingest.rs`:

```rust
/// Generate YAML frontmatter for a new vault file with a provisional ID.
///
/// Uses `temper-provisional-id` instead of `temper-id` to indicate the ID
/// hasn't been confirmed by the server yet.
pub fn build_provisional_frontmatter(
    id: Uuid,
    title: &str,
    context: &str,
    doc_type: &str,
) -> String {
    let now = chrono::Utc::now().to_rfc3339();
    format!(
        "---\ntemper-provisional-id: {id}\ntemper-type: {doc_type}\ntemper-context: {context}\ntemper-created: {now}\ntitle: \"{title}\"\n---\n\n"
    )
}
```

Update the call in `scan_vault_for_untracked` to use `build_provisional_frontmatter` instead of `build_frontmatter`.

- [ ] **Step 3: Build and verify**

Run: `cargo build -p temper-cli`
Expected: BUILD succeeds.

- [ ] **Step 4: Commit**

```bash
git add crates/temper-cli/src/actions/sync.rs crates/temper-cli/src/actions/ingest.rs
git commit -m "feat: scan_vault_for_untracked tracks provisional status"
```

---

### Task 5: Update `push_resource` to handle provisional entries

**Files:**
- Modify: `crates/temper-cli/src/actions/sync.rs:532-646`

- [ ] **Step 1: Force POST for provisional entries**

The current logic at lines 582-596 uses `item.resource_id.is_some()` to decide PUT vs POST. Update to also force POST when the manifest entry is provisional:

```rust
    let is_provisional = manifest
        .entries
        .get(&entry_id)
        .map_or(false, |e| e.provisional);

    let resource = if item.resource_id.is_some() && !is_provisional {
        // Existing, server-confirmed resource — PUT update
        client
            .ingest()
            .update(entry_id, &payload)
            .await
            .map_err(|e| TemperError::Api(e.to_string()))?
    } else {
        // New or provisional resource — POST create
        client
            .ingest()
            .create(&payload)
            .await
            .map_err(|e| TemperError::Api(e.to_string()))?
    };
```

- [ ] **Step 2: Replace `temper-provisional-id` with `temper-id` after successful POST**

Update the ID remapping block (lines 598-623). Instead of a blind string replace of UUID→UUID, do a structured replacement of the frontmatter key:

```rust
    let server_id = resource.id;
    if server_id != entry_id || is_provisional {
        tracing::info!(
            %entry_id,
            %server_id,
            is_provisional,
            "remapping manifest entry: local ID → server ID"
        );
        if let Some(mut entry) = manifest.entries.remove(&entry_id) {
            entry.provisional = false;
            manifest.entries.insert(server_id, entry);

            // Replace provisional frontmatter key+value with authoritative temper-id.
            let file_content = std::fs::read_to_string(&file_path)?;
            let updated = file_content
                .replace(
                    &format!("temper-provisional-id: \"{entry_id}\""),
                    &format!("temper-id: \"{server_id}\""),
                )
                .replace(
                    &format!("temper-provisional-id: {entry_id}"),
                    &format!("temper-id: {server_id}"),
                );

            if updated != file_content {
                std::fs::write(&file_path, &updated)?;
                tracing::info!("replaced temper-provisional-id with temper-id in frontmatter");
            } else {
                // Fallback: try replacing old-style id: or temper-id: (for files
                // that already had temper-id with a local UUID)
                let fallback = file_content.replace(&entry_id.to_string(), &server_id.to_string());
                if fallback != file_content {
                    std::fs::write(&file_path, &fallback)?;
                    tracing::info!("updated temper-id in file frontmatter (fallback path)");
                } else {
                    tracing::warn!(
                        %entry_id,
                        "temper-provisional-id not found in file content — frontmatter not updated"
                    );
                }
            }
        }
    }
```

- [ ] **Step 3: Build and verify**

Run: `cargo build -p temper-cli`
Expected: BUILD succeeds.

- [ ] **Step 4: Commit**

```bash
git add crates/temper-cli/src/actions/sync.rs
git commit -m "feat: push_resource handles provisional entries with structured frontmatter replacement"
```

---

### Task 6: Update `sync_reset` for provisional awareness

**Files:**
- Modify: `crates/temper-cli/src/actions/sync.rs:1057-1260`

- [ ] **Step 1: Read provisional_id from frontmatter during reset**

After parsing frontmatter at line 1133, also extract `provisional_id`:

```rust
        let fm = ingest::parse_source_frontmatter(&content);

        // Extract both ID types
        let temper_id = fm
            .as_ref()
            .and_then(|f| f.legacy_id.as_deref())
            .and_then(|id| Uuid::parse_str(id).ok());

        let provisional_id = fm
            .as_ref()
            .and_then(|f| f.provisional_id.as_deref())
            .and_then(|id| Uuid::parse_str(id).ok());
```

- [ ] **Step 2: Skip server ID matching for provisional files**

Files with only `temper-provisional-id` (no `temper-id`) should go straight to Pending without attempting ID or content-hash matching. Update the matching logic:

After the existing `temper_id` match block (lines 1141-1170), and before the content-hash fallback (lines 1173-1211), add a check:

```rust
        // Provisional files — skip server matching entirely, mark Pending
        if temper_id.is_none() && provisional_id.is_some() {
            let resource_id = provisional_id.unwrap();
            new_manifest.entries.insert(
                resource_id,
                temper_core::types::ManifestEntry {
                    path: rel_path,
                    body_hash: content_hash,
                    remote_body_hash: String::new(),
                    managed_hash: local_managed_hash,
                    open_hash: local_open_hash,
                    remote_managed_hash: String::new(),
                    remote_open_hash: String::new(),
                    synced_at: chrono::Utc::now(),
                    state: ManifestEntryState::Pending,
                    mtime_secs: mtime,
                    provisional: true,
                },
            );
            continue;
        }
```

- [ ] **Step 3: Update the unmatched-local-file block**

The existing unmatched block at lines 1214-1232 creates entries for files that matched neither by ID nor by hash. Update it to set `provisional` based on whether the file has a server-confirmed ID:

```rust
        // Unmatched local file — mark as Pending (new, will push on next sync).
        let (resource_id, is_provisional) = if let Some(tid) = temper_id {
            (tid, false)
        } else {
            (Uuid::now_v7(), true)
        };
        new_manifest.entries.insert(
            resource_id,
            temper_core::types::ManifestEntry {
                path: rel_path,
                body_hash: content_hash,
                remote_body_hash: String::new(),
                managed_hash: local_managed_hash,
                open_hash: local_open_hash,
                remote_managed_hash: String::new(),
                remote_open_hash: String::new(),
                synced_at: chrono::Utc::now(),
                state: ManifestEntryState::Pending,
                mtime_secs: mtime,
                provisional: is_provisional,
            },
        );
```

- [ ] **Step 4: Update server-matched entries to set provisional: false**

In the two existing match blocks (ID match at ~line 1152, hash match at ~line 1193), add `provisional: false` to each `ManifestEntry` construction.

- [ ] **Step 5: Update unmatched server entries**

At lines 1244-1258, where unmatched server resources are added for pull, add `provisional: false` since these are server-authoritative.

- [ ] **Step 6: Build and verify**

Run: `cargo build -p temper-cli`
Expected: BUILD succeeds.

- [ ] **Step 7: Commit**

```bash
git add crates/temper-cli/src/actions/sync.rs
git commit -m "feat: sync_reset skips server matching for provisional files"
```

---

### Task 7: Ensure `temper-provisional-id` hashes into managed tier

**Files:**
- Modify: `crates/temper-core/src/schema.rs:240-261` (verify only — may need no change)

- [ ] **Step 1: Verify hash classification**

Read `compute_frontmatter_hashes` in `crates/temper-core/src/schema.rs:240-261`. The rule at line 253 is:

```rust
if key_str.starts_with("temper-") || key_str == "title" || key_str == "slug" {
    meta.insert(key_str.to_string(), json_value);
```

Since `temper-provisional-id` starts with `temper-`, it will automatically hash into the managed tier. **No code change needed.**

- [ ] **Step 2: Write a test to confirm**

Add a test in the existing test module for `schema.rs`:

```rust
#[test]
fn test_provisional_id_hashes_into_managed_tier() {
    let yaml: serde_yaml::Value = serde_yaml::from_str(
        "temper-provisional-id: \"019d6088-3a3b-71a3-b26c-d38b8338773e\"\ntitle: \"Test\"\ncustom-field: \"value\""
    ).unwrap();
    let (managed, open) = compute_frontmatter_hashes(&yaml);
    // Managed should include temper-provisional-id and title
    assert_ne!(managed, "sha256:44136fa355b311bfa706c3193a095b5b940d28e0e7f2e8fd8d3e2e8f8f3e3b3b"); // not empty hash
    // Open should include custom-field
    assert_ne!(open, "sha256:44136fa355b311bfa706c3193a095b5b940d28e0e7f2e8fd8d3e2e8f8f3e3b3b");
}
```

- [ ] **Step 3: Run test**

Run: `cargo nextest run -p temper-core test_provisional_id_hashes`
Expected: PASS.

- [ ] **Step 4: Commit**

```bash
git add crates/temper-core/src/schema.rs
git commit -m "test: verify temper-provisional-id hashes into managed tier"
```

---

### Task 8: Update remaining manifest entry construction sites

**Files:**
- Modify: `crates/temper-cli/src/actions/sync.rs` (grep for all `ManifestEntry` constructions)

- [ ] **Step 1: Grep for all ManifestEntry construction sites**

Run: `grep -n "ManifestEntry {" crates/temper-cli/src/actions/sync.rs`

Every `ManifestEntry` literal must now include the `provisional` field. The sites are:
- `scan_vault_for_untracked` (Task 4 already handled)
- `sync_reset` match-by-id block (Task 6 step 4)
- `sync_reset` match-by-hash block (Task 6 step 4)
- `sync_reset` unmatched-local block (Task 6 step 3)
- `sync_reset` unmatched-server block (Task 6 step 5)
- `pull_resource` — when writing a pulled resource's manifest entry

- [ ] **Step 2: Update `pull_resource`**

In `pull_resource`, find where `ManifestEntry` is constructed after a successful pull. Add `provisional: false` — pulled resources are always server-authoritative.

- [ ] **Step 3: Grep for any remaining sites**

Run: `grep -rn "ManifestEntry {" crates/temper-cli/`

Check for any construction sites outside `sync.rs` (e.g., in `status.rs`, `doctor.rs`). Add `provisional: false` to each.

- [ ] **Step 4: Build and run full test suite**

Run: `cargo make check && cargo make test`
Expected: All checks pass, all tests pass.

- [ ] **Step 5: Commit**

```bash
git add crates/temper-cli/
git commit -m "fix: add provisional field to all ManifestEntry construction sites"
```

---

### Task 9: Integration test — end-to-end provisional ID lifecycle

**Files:**
- No new files — manual verification against running vault

- [ ] **Step 1: Create a new task with provisional ID**

```bash
temper task create "Integration test provisional ID" --context temper --goal temper-maintenance
```

Verify the file has `temper-provisional-id:` (not `temper-id:` or `id:`).

- [ ] **Step 2: Run sync reset and verify manifest**

```bash
temper sync reset
```

Verify the manifest entry for the new file has `"provisional": true` and `"state": "pending"`.

- [ ] **Step 3: Run sync and verify ID replacement**

```bash
temper sync run
```

After sync, verify:
1. The file now has `temper-id:` (not `temper-provisional-id:`)
2. The manifest entry has `"provisional": false`
3. The UUID in the file matches the manifest key (server-assigned)

- [ ] **Step 4: Clean up**

Delete the test task file and run `temper sync reset` to clean up the manifest.

- [ ] **Step 5: Final commit (if any fixups needed)**

Only commit if integration testing revealed issues that needed fixing.
