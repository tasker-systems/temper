# Sync Cycle Fix: Unified Hashing Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Eliminate the sync push/pull cycle by unifying hash computation into a single shared module in `temper-core` that both CLI and API call, with doc-type defaults applied before hashing on both sides.

**Architecture:** Create `temper_core::hash` as the single source of truth for body, managed, and open hash computation. Move field partitioning (`split_frontmatter_tiers`) and JSON canonicalization into core. Both CLI and API normalize their inputs (YAML frontmatter or JSON from DB) into `BTreeMap<String, serde_json::Value>`, then call the same core functions which apply defaults before hashing. Delete all duplicate implementations.

**Tech Stack:** Rust, sha2, hex, serde_json, serde_yaml (all already in temper-core's Cargo.toml)

---

## File Structure

| File | Action | Responsibility |
|------|--------|----------------|
| `crates/temper-core/src/hash.rs` | **Create** | All hash functions: `compute_body_hash`, `compute_managed_hash`, `compute_open_hash`, `canonicalize_json`, `hash_canonical_json`. Also `split_frontmatter_tiers` and field classification constants. |
| `crates/temper-core/src/lib.rs` | **Modify** | Add `pub mod hash;` |
| `crates/temper-core/src/schema.rs` | **Modify** | Remove `compute_frontmatter_hashes` and `hash_map`. Keep `SYSTEM_MANAGED_FIELDS` and validation functions. |
| `crates/temper-cli/src/actions/sync.rs` | **Modify** | Replace `split_frontmatter_tiers`, `SKIP_FROM_MANAGED`, and all `compute_frontmatter_hashes`/`compute_content_hash` calls with `temper_core::hash::*`. |
| `crates/temper-cli/src/actions/ingest.rs` | **Modify** | Remove `compute_content_hash`, replace calls with `temper_core::hash::compute_body_hash`. |
| `crates/temper-api/src/services/ingest_service.rs` | **Modify** | Remove `hash_json_value`, `canonicalize_json`, `strip_system_managed_fields`. Replace with `temper_core::hash::*`. |

---

### Task 1: Create `temper_core::hash` with Unified Hash Functions

**Files:**
- Create: `crates/temper-core/src/hash.rs`
- Modify: `crates/temper-core/src/lib.rs:7` (add module)

This task creates the single source of truth for all sync-related hashing. Every hash
function lives here. Both CLI and API will call these functions.

**Design decisions:**
- `compute_managed_hash` takes a `doc_type` and `&serde_json::Value` (the managed meta),
  clones it, applies `apply_doc_type_defaults`, canonicalizes, and hashes. This means
  both sides always hash the canonical form with defaults, even if the input is missing them.
- `compute_open_hash` canonicalizes and hashes the open meta (no defaults to apply).
- `compute_body_hash` hashes a `&str` body directly.
- `split_frontmatter_tiers` moves from CLI to core, taking `serde_yaml::Value` and
  `doc_type`, returning `(serde_json::Value, serde_json::Value)`.
- Field classification uses a single `IDENTITY_FIELDS` constant (fields skipped entirely)
  and `TIER1_SYSTEM_FIELDS` constant (fields stripped from managed meta before hashing
  because they're tracked structurally in the DB).

**Field partitioning rules (unified):**
- **Skipped entirely** (identity — tracked structurally): `temper-id`, `temper-provisional-id`
- **Stripped from managed meta** (tier-1 system — DB columns own these): `temper-context`, `temper-type`, `temper-created`, `temper-updated`, `temper-owner`, `temper-source`, `temper-legacy-id`
- **Managed tier**: `temper-*` (remaining after skips/strips), `title`, `slug`, doc-type schema properties
- **Open tier**: everything else

- [ ] **Step 1: Write failing tests for `compute_body_hash`**

Create `crates/temper-core/src/hash.rs` with tests at the bottom:

```rust
//! Unified content hashing for sync operations.
//!
//! This module is the **single source of truth** for all hash computations
//! used in the sync manifest. Both CLI (file-based) and API (DB-based) call
//! these functions to ensure identical hashes for identical content.

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn body_hash_deterministic() {
        let hash1 = compute_body_hash("# Hello\n\nWorld");
        let hash2 = compute_body_hash("# Hello\n\nWorld");
        assert_eq!(hash1, hash2);
        assert!(hash1.starts_with("sha256:"), "hash should be prefixed: {hash1}");
    }

    #[test]
    fn body_hash_differs_for_different_content() {
        let hash1 = compute_body_hash("# Hello");
        let hash2 = compute_body_hash("# Goodbye");
        assert_ne!(hash1, hash2);
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo nextest run -p temper-core test_body_hash`
Expected: Compilation error — `compute_body_hash` not defined.

- [ ] **Step 3: Implement `compute_body_hash`**

Add above the tests in `hash.rs`:

```rust
use sha2::{Digest, Sha256};

/// Compute SHA-256 hash of markdown body content.
///
/// Returns `"sha256:<lowercase_hex>"`.
pub fn compute_body_hash(body: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(body.as_bytes());
    format!("sha256:{}", hex::encode(hasher.finalize()))
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo nextest run -p temper-core test_body_hash`
Expected: PASS

- [ ] **Step 5: Write failing tests for `canonicalize_json` and `hash_canonical_json`**

Add to the test module:

```rust
    #[test]
    fn canonicalize_sorts_keys() {
        let input = json!({"z": 1, "a": 2, "m": 3});
        let canonical = canonicalize_json(&input);
        let s = serde_json::to_string(&canonical).unwrap();
        assert_eq!(s, r#"{"a":2,"m":3,"z":1}"#);
    }

    #[test]
    fn canonicalize_sorts_nested_keys() {
        let input = json!({"b": {"z": 1, "a": 2}, "a": 1});
        let canonical = canonicalize_json(&input);
        let s = serde_json::to_string(&canonical).unwrap();
        assert_eq!(s, r#"{"a":1,"b":{"a":2,"z":1}}"#);
    }

    #[test]
    fn hash_canonical_json_deterministic() {
        let v1 = json!({"z": 1, "a": 2});
        let v2 = json!({"a": 2, "z": 1});
        assert_eq!(hash_canonical_json(&v1), hash_canonical_json(&v2));
    }

    #[test]
    fn hash_canonical_json_empty_object() {
        let empty = json!({});
        let hash = hash_canonical_json(&empty);
        assert!(hash.starts_with("sha256:"));
    }
```

- [ ] **Step 6: Run tests to verify they fail**

Run: `cargo nextest run -p temper-core test_canonicalize`
Expected: Compilation error — functions not defined.

- [ ] **Step 7: Implement `canonicalize_json` and `hash_canonical_json`**

Add to `hash.rs` above the tests:

```rust
use std::collections::BTreeMap;

/// Recursively sort JSON object keys for deterministic serialization.
pub fn canonicalize_json(value: &serde_json::Value) -> serde_json::Value {
    match value {
        serde_json::Value::Object(map) => {
            let sorted: BTreeMap<String, serde_json::Value> = map
                .iter()
                .map(|(k, v)| (k.clone(), canonicalize_json(v)))
                .collect();
            serde_json::to_value(sorted).unwrap_or(serde_json::Value::Object(map.clone()))
        }
        serde_json::Value::Array(arr) => {
            serde_json::Value::Array(arr.iter().map(canonicalize_json).collect())
        }
        other => other.clone(),
    }
}

/// Hash a JSON value in canonical form. Returns `"sha256:<lowercase_hex>"`.
pub fn hash_canonical_json(value: &serde_json::Value) -> String {
    let canonical = canonicalize_json(value);
    let serialized = serde_json::to_string(&canonical).unwrap_or_else(|_| "{}".to_string());
    let mut hasher = Sha256::new();
    hasher.update(serialized.as_bytes());
    format!("sha256:{}", hex::encode(hasher.finalize()))
}
```

- [ ] **Step 8: Run tests to verify they pass**

Run: `cargo nextest run -p temper-core test_canonicalize hash_canonical`
Expected: PASS

- [ ] **Step 9: Write failing tests for `compute_managed_hash` (the key function)**

This is the function that applies doc-type defaults before hashing. Add to tests:

```rust
    #[test]
    fn managed_hash_applies_defaults_before_hashing() {
        // A task with no temper-stage should hash identically to one with temper-stage: backlog
        let without_default = json!({"title": "test"});
        let with_default = json!({"title": "test", "temper-stage": "backlog"});
        assert_eq!(
            compute_managed_hash("task", &without_default),
            compute_managed_hash("task", &with_default),
            "defaults should be applied before hashing"
        );
    }

    #[test]
    fn managed_hash_preserves_explicit_values() {
        // A task with explicit temper-stage: in-progress should NOT hash like backlog
        let explicit = json!({"title": "test", "temper-stage": "in-progress"});
        let default = json!({"title": "test"});
        assert_ne!(
            compute_managed_hash("task", &explicit),
            compute_managed_hash("task", &default),
            "explicit values should not be overwritten by defaults"
        );
    }

    #[test]
    fn managed_hash_strips_tier1_system_fields() {
        // tier-1 fields should not affect the hash
        let with_system = json!({"title": "test", "temper-created": "2026-01-01T00:00:00Z"});
        let without_system = json!({"title": "test"});
        assert_eq!(
            compute_managed_hash("task", &with_system),
            compute_managed_hash("task", &without_system),
            "tier-1 system fields should be stripped before hashing"
        );
    }

    #[test]
    fn managed_hash_deterministic_regardless_of_key_order() {
        let v1 = json!({"title": "test", "temper-stage": "backlog"});
        let v2 = json!({"temper-stage": "backlog", "title": "test"});
        assert_eq!(
            compute_managed_hash("task", &v1),
            compute_managed_hash("task", &v2),
        );
    }
```

- [ ] **Step 10: Run tests to verify they fail**

Run: `cargo nextest run -p temper-core managed_hash`
Expected: Compilation error — `compute_managed_hash` not defined.

- [ ] **Step 11: Implement `compute_managed_hash` and `compute_open_hash`**

Add constants and functions to `hash.rs`:

```rust
/// Fields skipped entirely from hashing — tracked structurally (manifest key / DB primary key).
pub const IDENTITY_FIELDS: &[&str] = &["temper-id", "temper-provisional-id"];

/// Tier-1 system fields stripped from managed meta before hashing.
/// These are owned by DB columns (kb_resources table), not by managed_meta JSONB.
pub const TIER1_SYSTEM_FIELDS: &[&str] = &[
    "temper-context",
    "temper-type",
    "temper-created",
    "temper-updated",
    "temper-owner",
    "temper-source",
    "temper-legacy-id",
];

/// Compute SHA-256 hash of managed metadata in canonical form.
///
/// **Applies doc-type defaults** before hashing, so a task missing `temper-stage`
/// hashes identically to one with `temper-stage: "backlog"`. This ensures CLI
/// and API always agree on the hash regardless of whether defaults have been
/// explicitly written.
///
/// Also strips tier-1 system fields (owned by DB columns, not managed_meta).
pub fn compute_managed_hash(doc_type: &str, managed_meta: &serde_json::Value) -> String {
    let mut meta = managed_meta.clone();

    // Strip tier-1 system fields
    if let Some(obj) = meta.as_object_mut() {
        for field in TIER1_SYSTEM_FIELDS {
            obj.remove(*field);
        }
    }

    // Apply doc-type defaults (backlog for tasks, active for goals, etc.)
    crate::defaults::apply_doc_type_defaults(doc_type, &mut meta);

    hash_canonical_json(&meta)
}

/// Compute SHA-256 hash of open (user-owned) metadata in canonical form.
pub fn compute_open_hash(open_meta: &serde_json::Value) -> String {
    hash_canonical_json(open_meta)
}
```

- [ ] **Step 12: Run tests to verify they pass**

Run: `cargo nextest run -p temper-core managed_hash open_hash`
Expected: PASS

- [ ] **Step 13: Write failing test for `split_frontmatter_tiers`**

```rust
    #[test]
    fn split_tiers_partitions_correctly() {
        let yaml: serde_yaml::Value = serde_yaml::from_str(
            r#"
temper-id: "019d0000-0000-0000-0000-000000000000"
temper-type: task
temper-context: temper
temper-stage: backlog
title: "Test task"
slug: "test-task"
date: "2026-04-11"
custom-field: hello
"#,
        )
        .unwrap();

        let (managed, open) = split_frontmatter_tiers(&yaml, "task");

        let managed_obj = managed.as_object().unwrap();
        let open_obj = open.as_object().unwrap();

        // Identity fields should be absent from both
        assert!(!managed_obj.contains_key("temper-id"));
        assert!(!open_obj.contains_key("temper-id"));

        // Tier-1 system fields should be absent from both
        assert!(!managed_obj.contains_key("temper-type"));
        assert!(!managed_obj.contains_key("temper-context"));

        // Managed tier should have temper-stage, title, slug
        assert!(managed_obj.contains_key("temper-stage"));
        assert!(managed_obj.contains_key("title"));
        assert!(managed_obj.contains_key("slug"));

        // Open tier should have custom-field
        assert!(open_obj.contains_key("custom-field"));

        // date is NOT a schema property for tasks — goes to open
        // (it IS a schema property for sessions — tested separately)
        assert!(open_obj.contains_key("date"));
    }

    #[test]
    fn split_tiers_routes_schema_properties_to_managed() {
        let yaml: serde_yaml::Value = serde_yaml::from_str(
            r#"
temper-id: "019d0000-0000-0000-0000-000000000000"
title: "Test session"
date: "2026-04-11"
custom: value
"#,
        )
        .unwrap();

        let (managed, open) = split_frontmatter_tiers(&yaml, "session");

        // date IS a schema property for sessions — should be in managed
        assert!(
            managed.as_object().unwrap().contains_key("date"),
            "schema-defined 'date' should route to managed for sessions"
        );
        assert!(open.as_object().unwrap().contains_key("custom"));
    }
```

- [ ] **Step 14: Run tests to verify they fail**

Run: `cargo nextest run -p temper-core split_tiers`
Expected: Compilation error — `split_frontmatter_tiers` not defined.

- [ ] **Step 15: Implement `split_frontmatter_tiers`**

Add to `hash.rs`:

```rust
use std::collections::HashSet;

/// Split parsed YAML frontmatter into managed and open tiers.
///
/// **Managed tier** receives: `temper-*` fields (minus identity and tier-1 system
/// fields), `title`, `slug`, and any properties defined in the doc-type schema.
///
/// **Open tier** receives everything else.
///
/// Identity fields (`temper-id`, `temper-provisional-id`) and tier-1 system fields
/// (`temper-context`, `temper-type`, `temper-created`, `temper-updated`,
/// `temper-owner`, `temper-source`, `temper-legacy-id`) are excluded from both tiers.
pub fn split_frontmatter_tiers(
    fm: &serde_yaml::Value,
    doc_type: &str,
) -> (serde_json::Value, serde_json::Value) {
    let Some(mapping) = fm.as_mapping() else {
        return (serde_json::json!({}), serde_json::json!({}));
    };

    let schema_keys: HashSet<String> = crate::schema::schema_value(doc_type)
        .ok()
        .and_then(|v| v.get("properties")?.as_object().cloned())
        .map(|props| props.keys().cloned().collect())
        .unwrap_or_default();

    let mut managed = serde_json::Map::new();
    let mut open = serde_json::Map::new();

    for (key, value) in mapping {
        let Some(key_str) = key.as_str() else {
            continue;
        };

        // Skip identity fields
        if IDENTITY_FIELDS.contains(&key_str) {
            continue;
        }

        // Skip tier-1 system fields
        if TIER1_SYSTEM_FIELDS.contains(&key_str) {
            continue;
        }

        let json_value = serde_json::to_value(value).unwrap_or(serde_json::Value::Null);

        if key_str.starts_with("temper-")
            || key_str == "title"
            || key_str == "slug"
            || schema_keys.contains(key_str)
        {
            managed.insert(key_str.to_string(), json_value);
        } else {
            open.insert(key_str.to_string(), json_value);
        }
    }

    (
        serde_json::Value::Object(managed),
        serde_json::Value::Object(open),
    )
}
```

- [ ] **Step 16: Run tests to verify they pass**

Run: `cargo nextest run -p temper-core split_tiers`
Expected: PASS

- [ ] **Step 17: Write the combined round-trip test**

This is the critical test that proves CLI and API produce the same hash:

```rust
    #[test]
    fn cli_and_api_path_produce_same_managed_hash() {
        // Simulate CLI path: YAML frontmatter → split_frontmatter_tiers → compute_managed_hash
        let yaml: serde_yaml::Value = serde_yaml::from_str(
            r#"
temper-id: "019d0000-0000-0000-0000-000000000000"
temper-type: task
temper-context: temper
temper-created: "2026-04-11T00:00:00Z"
temper-owner: "@me"
title: "My task"
temper-stage: backlog
"#,
        )
        .unwrap();
        let (cli_managed, _cli_open) = split_frontmatter_tiers(&yaml, "task");
        let cli_hash = compute_managed_hash("task", &cli_managed);

        // Simulate API path: JSON managed_meta (as stored in DB, without tier-1 fields)
        let api_managed = serde_json::json!({
            "title": "My task",
            "temper-stage": "backlog"
        });
        let api_hash = compute_managed_hash("task", &api_managed);

        assert_eq!(cli_hash, api_hash, "CLI and API must produce identical managed hashes");
    }

    #[test]
    fn cli_and_api_agree_when_defaults_absent_locally() {
        // CLI file is missing temper-stage (not yet synced)
        let yaml: serde_yaml::Value = serde_yaml::from_str(
            r#"
temper-id: "019d0000-0000-0000-0000-000000000000"
temper-type: task
temper-context: temper
title: "My task"
"#,
        )
        .unwrap();
        let (cli_managed, _) = split_frontmatter_tiers(&yaml, "task");
        let cli_hash = compute_managed_hash("task", &cli_managed);

        // API has applied defaults server-side
        let api_managed = serde_json::json!({
            "title": "My task",
            "temper-stage": "backlog"
        });
        let api_hash = compute_managed_hash("task", &api_managed);

        assert_eq!(
            cli_hash, api_hash,
            "hash must match even when CLI file lacks default fields"
        );
    }
```

- [ ] **Step 18: Run round-trip tests**

Run: `cargo nextest run -p temper-core cli_and_api`
Expected: PASS (these call already-implemented functions)

- [ ] **Step 19: Register the module**

Add to `crates/temper-core/src/lib.rs`:

```rust
pub mod hash;
```

- [ ] **Step 20: Run full temper-core tests**

Run: `cargo nextest run -p temper-core`
Expected: All pass

- [ ] **Step 21: Commit**

```bash
git add crates/temper-core/src/hash.rs crates/temper-core/src/lib.rs
git commit -m "feat(core): add unified hash module with defaults-aware managed hash

Single source of truth for body, managed, and open hash computation.
compute_managed_hash applies doc-type defaults before hashing so CLI
and API always agree regardless of which fields are present in input."
```

---

### Task 2: Wire CLI to Use Core Hash Functions

**Files:**
- Modify: `crates/temper-cli/src/actions/sync.rs:21-30` (remove `compute_content_hash` usage)
- Modify: `crates/temper-cli/src/actions/sync.rs:101-155` (update `rehash_manifest`)
- Modify: `crates/temper-cli/src/actions/sync.rs:222-277` (remove `SKIP_FROM_MANAGED` and `split_frontmatter_tiers`)
- Modify: `crates/temper-cli/src/actions/sync.rs:654-789` (update `push_resource`)
- Modify: `crates/temper-cli/src/actions/sync.rs:791-897` (update `pull_resource`)
- Modify: `crates/temper-cli/src/actions/sync.rs:1237-1453` (update `sync_reset`)
- Modify: `crates/temper-cli/src/actions/ingest.rs:19-30` (remove `compute_content_hash`)
- Modify: `crates/temper-core/src/schema.rs:239-476` (remove `compute_frontmatter_hashes` and `hash_map`)

This task replaces all CLI hash call sites with the unified core functions.

**Critical change:** `rehash_manifest` currently calls `compute_frontmatter_hashes` which
does NOT apply defaults and does NOT strip tier-1 fields. Switching to the core functions
fixes both issues. However, `rehash_manifest` currently does not know the `doc_type` for
each entry. We need to extract it from the manifest entry's `path` field (format:
`@owner/context/doc_type/slug.md`).

- [ ] **Step 1: Write failing test for doc_type extraction from manifest path**

In `crates/temper-core/src/hash.rs`, add a helper and test:

```rust
    #[test]
    fn doc_type_from_vault_path_valid() {
        assert_eq!(
            doc_type_from_vault_path("@me/temper/task/my-task.md"),
            Some("task")
        );
        assert_eq!(
            doc_type_from_vault_path("@me/general/session/2026-04-11-foo.md"),
            Some("session")
        );
    }

    #[test]
    fn doc_type_from_vault_path_invalid() {
        assert_eq!(doc_type_from_vault_path("bad-path.md"), None);
        assert_eq!(doc_type_from_vault_path(""), None);
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo nextest run -p temper-core doc_type_from_vault`
Expected: Compilation error.

- [ ] **Step 3: Implement `doc_type_from_vault_path`**

Add to `hash.rs`:

```rust
/// Extract the doc-type segment from a vault-relative path.
///
/// Vault paths follow the pattern `@owner/context/doc_type/slug.md`.
/// Returns `None` if the path doesn't have enough segments.
pub fn doc_type_from_vault_path(path: &str) -> Option<&str> {
    let parts: Vec<&str> = path.split('/').collect();
    if parts.len() >= 3 {
        Some(parts[parts.len() - 2])
    } else {
        None
    }
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo nextest run -p temper-core doc_type_from_vault`
Expected: PASS

- [ ] **Step 5: Replace `compute_content_hash` in `ingest.rs`**

In `crates/temper-cli/src/actions/ingest.rs`, remove the `compute_content_hash` function
(lines 19-30) entirely. Replace all calls throughout the CLI with
`temper_core::hash::compute_body_hash`.

Search for all call sites:
```bash
grep -rn "ingest::compute_content_hash\|crate::actions::ingest::compute_content_hash" crates/temper-cli/
```

At each call site, replace `ingest::compute_content_hash(...)` with
`temper_core::hash::compute_body_hash(...)`.

Also check `crates/temper-cli/src/commands/pull.rs` for any direct calls.

- [ ] **Step 6: Replace `SKIP_FROM_MANAGED` and `split_frontmatter_tiers` in `sync.rs`**

In `crates/temper-cli/src/actions/sync.rs`:

1. Remove `SKIP_FROM_MANAGED` constant (lines 222-227)
2. Remove `split_frontmatter_tiers` function (lines 236-277)
3. Replace all calls to `split_frontmatter_tiers(&fm, &doc_type)` with
   `temper_core::hash::split_frontmatter_tiers(&fm, &doc_type)`

Search for call sites:
```bash
grep -n "split_frontmatter_tiers" crates/temper-cli/src/actions/sync.rs
```

- [ ] **Step 7: Update `rehash_manifest` to use core hash functions**

In `crates/temper-cli/src/actions/sync.rs`, update the `rehash_manifest` function.

Current code (around lines 128-152):
```rust
let body = strip_frontmatter(&content);
let current_hash = ingest::compute_content_hash(body);
let (managed_hash, open_hash) = if let Some(fm) = crate::vault::parse_frontmatter(&content) {
    temper_core::schema::compute_frontmatter_hashes(&fm)
} else {
    (String::new(), String::new())
};
```

Replace with:
```rust
let body = strip_frontmatter(&content);
let current_hash = temper_core::hash::compute_body_hash(body);

let doc_type = temper_core::hash::doc_type_from_vault_path(&entry.path)
    .unwrap_or("unknown");

let (managed_hash, open_hash) = if let Some(fm) = crate::vault::parse_frontmatter(&content) {
    let (managed_meta, open_meta) =
        temper_core::hash::split_frontmatter_tiers(&fm, doc_type);
    (
        temper_core::hash::compute_managed_hash(doc_type, &managed_meta),
        temper_core::hash::compute_open_hash(&open_meta),
    )
} else {
    (
        temper_core::hash::compute_managed_hash(doc_type, &serde_json::json!({})),
        temper_core::hash::compute_open_hash(&serde_json::json!({})),
    )
};
```

- [ ] **Step 8: Update `push_resource` manifest hash recording**

In `push_resource` (around line 768), after a successful push the manifest records
the frontmatter hashes. Update to use core functions:

Current:
```rust
let (pushed_managed_hash, pushed_open_hash) = {
    let current = std::fs::read_to_string(&file_path)?;
    if let Some(fm) = crate::vault::parse_frontmatter(&current) {
        temper_core::schema::compute_frontmatter_hashes(&fm)
    } else {
        (String::new(), String::new())
    }
};
```

Replace with:
```rust
let (pushed_managed_hash, pushed_open_hash) = {
    let current = std::fs::read_to_string(&file_path)?;
    if let Some(fm) = crate::vault::parse_frontmatter(&current) {
        let (managed_meta, open_meta) =
            temper_core::hash::split_frontmatter_tiers(&fm, &doc_type);
        (
            temper_core::hash::compute_managed_hash(&doc_type, &managed_meta),
            temper_core::hash::compute_open_hash(&open_meta),
        )
    } else {
        (
            temper_core::hash::compute_managed_hash(&doc_type, &serde_json::json!({})),
            temper_core::hash::compute_open_hash(&serde_json::json!({})),
        )
    }
};
```

- [ ] **Step 9: Update `pull_resource` manifest hash recording**

In `pull_resource` (around line 856), update similarly:

Current:
```rust
let (managed_hash, open_hash) = if let Some(fm) = crate::vault::parse_frontmatter(&full_content) {
    temper_core::schema::compute_frontmatter_hashes(&fm)
} else {
    (String::new(), String::new())
};
```

Replace with:
```rust
let (managed_hash, open_hash) = if let Some(fm) = crate::vault::parse_frontmatter(&full_content) {
    let (managed_meta, open_meta) =
        temper_core::hash::split_frontmatter_tiers(&fm, &doc_type);
    (
        temper_core::hash::compute_managed_hash(&doc_type, &managed_meta),
        temper_core::hash::compute_open_hash(&open_meta),
    )
} else {
    (
        temper_core::hash::compute_managed_hash(&doc_type, &serde_json::json!({})),
        temper_core::hash::compute_open_hash(&serde_json::json!({})),
    )
};
```

Note: `doc_type` is already available in `pull_resource` from the `SyncPullItem`.

- [ ] **Step 10: Update `sync_reset` hash computation**

In `sync_reset` (around line 1300), same pattern. The doc_type is extracted from the
vault path during the walk. Apply the same replacement pattern.

- [ ] **Step 11: Update any remaining call sites**

Search for all remaining references:
```bash
grep -rn "compute_frontmatter_hashes\|compute_content_hash\|hash_json_value" crates/temper-cli/
```

Replace each one with the corresponding core function. Common sites:
- Conflict resolution paths in `sync.rs`
- Merge finalization
- Any other hash computation in the sync flow

- [ ] **Step 12: Remove `compute_frontmatter_hashes` and `hash_map` from `schema.rs`**

In `crates/temper-core/src/schema.rs`:
1. Remove `compute_frontmatter_hashes` (lines 239-265)
2. Remove `hash_map` (lines 469-476)

These are now dead code — all callers use `temper_core::hash::*`.

- [ ] **Step 13: Build and fix any compilation errors**

Run: `cargo make build`
Expected: Clean build. Fix any remaining references to removed functions.

- [ ] **Step 14: Run all unit tests**

Run: `cargo make test`
Expected: All pass.

- [ ] **Step 15: Commit**

```bash
git add crates/temper-cli/ crates/temper-core/src/schema.rs
git commit -m "refactor(cli): replace all hash calls with temper_core::hash

Remove compute_content_hash from ingest.rs, SKIP_FROM_MANAGED and
split_frontmatter_tiers from sync.rs, compute_frontmatter_hashes
and hash_map from schema.rs. All callers now use the unified core
hash module which applies doc-type defaults before hashing."
```

---

### Task 3: Wire API to Use Core Hash Functions

**Files:**
- Modify: `crates/temper-api/src/services/ingest_service.rs:91-153` (remove `strip_system_managed_fields`, `hash_json_value`, `canonicalize_json`)
- Modify: `crates/temper-api/src/services/ingest_service.rs` (all call sites of removed functions)

This task replaces API-side hash computation with the unified core functions.

**Critical change:** The API currently calls `hash_json_value(managed_meta)` which
does NOT apply defaults (defaults are applied separately in the ingest flow). After
this change, `compute_managed_hash(doc_type, managed_meta)` handles both stripping
and defaults internally.

- [ ] **Step 1: Replace `strip_system_managed_fields`**

In `crates/temper-api/src/services/ingest_service.rs`, the function
`strip_system_managed_fields` (lines 91-112) strips tier-1 fields from incoming
managed_meta. This logic is now handled inside `compute_managed_hash`. However,
we still need to strip these fields before **storing** the managed_meta in the DB
(they shouldn't be in the JSONB column).

Keep a simpler version that just strips fields for storage, OR use the core constants:

Replace lines 91-112:
```rust
fn strip_system_managed_fields(mut meta: serde_json::Value) -> serde_json::Value {
    if let Some(obj) = meta.as_object_mut() {
        for field in temper_core::hash::IDENTITY_FIELDS
            .iter()
            .chain(temper_core::hash::TIER1_SYSTEM_FIELDS.iter())
        {
            if obj.remove(*field).is_some() {
                tracing::warn!(
                    field = *field,
                    "stripped system field from input managed_meta"
                );
            }
        }
    }
    meta
}
```

- [ ] **Step 2: Replace `hash_json_value` calls with `compute_managed_hash`**

Search for all call sites:
```bash
grep -n "hash_json_value" crates/temper-api/src/services/ingest_service.rs
```

In `create_resource_with_manifest` (around line 318):
```rust
// Before:
let managed_hash = hash_json_value(params.managed_meta);
let open_hash = hash_json_value(params.open_meta);

// After:
let managed_hash = temper_core::hash::compute_managed_hash(params.doc_type, params.managed_meta);
let open_hash = temper_core::hash::compute_open_hash(params.open_meta);
```

Note: `params.doc_type` must be available in `CreateResourceParams`. Check the struct
definition and add `doc_type: &str` if not already present. The doc_type is known at
every call site (it's passed through the ingest flow).

Apply the same pattern at every `hash_json_value` call site in the file.

- [ ] **Step 3: Replace inline body hash computation**

In the `ingest` function (around line 426) and `update` function (around line 614),
the body hash is computed inline:

```rust
// Before:
let hash = {
    let mut hasher = Sha256::new();
    hasher.update(payload.content.as_bytes());
    hasher.finalize()
};
payload.content_hash = Some(format!("sha256:{:x}", hash));

// After:
payload.content_hash = Some(temper_core::hash::compute_body_hash(&payload.content));
```

- [ ] **Step 4: Remove dead code**

Remove `hash_json_value` and `canonicalize_json` functions (lines 127-153) from
`ingest_service.rs`. Also remove the `use sha2::{Digest, Sha256};` import if no
longer needed in this file.

- [ ] **Step 5: Verify `CreateResourceParams` has `doc_type`**

Read `CreateResourceParams` struct. If it doesn't have a `doc_type` field, check
what's available. The doc_type name is typically resolved earlier in the flow and
passed as a separate parameter. You may need to:
- Add `pub doc_type: &'a str` to `CreateResourceParams`
- Pass it through from the call site

Similarly check `update_resource_manifest` params.

- [ ] **Step 6: Build and fix compilation errors**

Run: `cargo make build`
Expected: Clean build. Fix any remaining import or call site issues.

- [ ] **Step 7: Run unit tests**

Run: `cargo make test`
Expected: All pass.

- [ ] **Step 8: Commit**

```bash
git add crates/temper-api/
git commit -m "refactor(api): replace hash_json_value with temper_core::hash

API now uses compute_managed_hash (with defaults) and compute_open_hash
from the unified core module. Removes canonicalize_json and hash_json_value
from ingest_service. strip_system_managed_fields now uses shared constants."
```

---

### Task 4: Integration Test — CLI and API Hash Agreement

**Files:**
- Create: `crates/temper-core/src/hash.rs` (add integration-style tests to existing test module)

This task adds comprehensive tests proving that every path through the system
produces the same hash for the same logical content.

- [ ] **Step 1: Add cross-path agreement tests**

Add to `crates/temper-core/src/hash.rs` test module:

```rust
    /// Simulates the full round-trip: file with frontmatter → split → hash (CLI path)
    /// vs. JSON from DB → hash (API path). Must agree for every doc type.
    #[test]
    fn round_trip_hash_agreement_all_doc_types() {
        let cases = vec![
            (
                "task",
                r#"
temper-id: "019d0000-0000-0000-0000-000000000000"
temper-type: task
temper-context: temper
temper-created: "2026-04-11T00:00:00Z"
temper-updated: "2026-04-11T00:00:00Z"
temper-owner: "@me"
title: "Test task"
slug: "test-task"
"#,
                // API-side managed_meta (after strip_system_managed_fields, before defaults)
                json!({"title": "Test task", "slug": "test-task"}),
            ),
            (
                "goal",
                r#"
temper-id: "019d0000-0000-0000-0000-000000000001"
temper-type: goal
temper-context: temper
title: "Ship v1"
slug: "ship-v1"
"#,
                json!({"title": "Ship v1", "slug": "ship-v1"}),
            ),
            (
                "session",
                r#"
temper-id: "019d0000-0000-0000-0000-000000000002"
temper-type: session
temper-context: temper
title: "My session"
slug: "my-session"
date: "2026-04-11"
"#,
                json!({"title": "My session", "slug": "my-session", "date": "2026-04-11"}),
            ),
        ];

        for (doc_type, yaml_str, api_managed) in cases {
            let yaml: serde_yaml::Value = serde_yaml::from_str(yaml_str).unwrap();
            let (cli_managed, _) = split_frontmatter_tiers(&yaml, doc_type);
            let cli_hash = compute_managed_hash(doc_type, &cli_managed);
            let api_hash = compute_managed_hash(doc_type, &api_managed);
            assert_eq!(
                cli_hash, api_hash,
                "hash mismatch for doc_type={doc_type}: CLI managed={cli_managed}, API managed={api_managed}"
            );
        }
    }

    /// Verify that a file missing defaults hashes the same as one with them.
    #[test]
    fn defaults_make_hashes_converge() {
        // Goal without explicit status
        let yaml_no_status: serde_yaml::Value = serde_yaml::from_str(
            r#"
temper-id: "019d0000-0000-0000-0000-000000000001"
temper-type: goal
title: "Ship v1"
"#,
        )
        .unwrap();

        // Goal with explicit status: active (the default)
        let yaml_with_status: serde_yaml::Value = serde_yaml::from_str(
            r#"
temper-id: "019d0000-0000-0000-0000-000000000001"
temper-type: goal
title: "Ship v1"
temper-status: active
"#,
        )
        .unwrap();

        let (m1, _) = split_frontmatter_tiers(&yaml_no_status, "goal");
        let (m2, _) = split_frontmatter_tiers(&yaml_with_status, "goal");

        assert_eq!(
            compute_managed_hash("goal", &m1),
            compute_managed_hash("goal", &m2),
            "missing default and explicit default must hash identically"
        );
    }
```

- [ ] **Step 2: Run new tests**

Run: `cargo nextest run -p temper-core round_trip_hash defaults_make_hashes`
Expected: PASS

- [ ] **Step 3: Commit**

```bash
git add crates/temper-core/src/hash.rs
git commit -m "test(core): add cross-path hash agreement tests

Proves CLI (YAML file → split → hash) and API (JSON from DB → hash)
produce identical results for all doc types, including when defaults
are absent from one side."
```

---

### Task 5: Full Verification and Cleanup

**Files:**
- All modified files from previous tasks

- [ ] **Step 1: Run `cargo make check`**

Run: `cargo make check`
Expected: No lint warnings, no format issues. Fix any that appear.

- [ ] **Step 2: Run `cargo make fix` if needed**

Run: `cargo make fix`
Expected: Auto-fixes applied.

- [ ] **Step 3: Run full test suite**

Run: `cargo make test`
Expected: All unit tests pass across all crates.

- [ ] **Step 4: Run integration tests (if DB is available)**

Run: `cargo make docker-up && cargo make test-db`
Expected: All integration tests pass.

- [ ] **Step 5: Verify no dead imports or unused code**

Run: `cargo make check`
Expected: No `unused import` or `dead_code` warnings from modified files.

- [ ] **Step 6: Search for any remaining hash divergence**

```bash
grep -rn "Sha256::new\|sha2::Digest\|hex::encode.*hasher" crates/temper-cli/ crates/temper-api/ crates/temper-core/src/schema.rs
```

The only remaining `Sha256` usage should be:
- `crates/temper-core/src/hash.rs` (the unified module)
- `crates/temper-ingest/src/chunk.rs` (per-chunk hash, unrelated to sync)
- `crates/temper-ingest/src/merge.rs` (conflict annotation, unrelated to sync)

If any sync-related hash computation remains outside `temper_core::hash`, fix it.

- [ ] **Step 7: Commit any cleanup**

```bash
git add -A
git commit -m "chore: cleanup dead imports and unused code from hash unification"
```

(Only if there are changes to commit.)
