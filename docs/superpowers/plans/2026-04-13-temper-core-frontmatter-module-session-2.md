# Temper-Core Frontmatter Consolidation — Session 2 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Migrate every sync-sensitive and normalize-sensitive caller of the legacy `hash::split_frontmatter_tiers` / `hash::compute_frontmatter_hashes_from_yaml` / `normalize::split_frontmatter_block` APIs to the new `temper_core::frontmatter::Frontmatter` module landed in Session 1, delete the legacy APIs, and fix the `ResourceRelationships::tags` phantom-edge bug as a drive-by.

**Architecture:** This is a code-mechanics session, not a design session. The `Frontmatter` aggregate already exists and its hash output is proven byte-identical to the legacy path (Session 1 regression anchors). Every migration replaces scattered YAML+schema parsing with a single `Frontmatter::try_from(&str)` or `Frontmatter::parse_file(&Path)` call, pulls tier-split JSON via `fm.managed_json()` / `fm.open_json()`, and pulls (managed_hash, open_hash) via `fm.hashes()`. `normalize_file` becomes a thin orchestrator: `parse_file` → mutate `value_mut()` via `apply_doc_type_defaults_yaml` → `write_to`. Once every caller has migrated, the legacy public APIs are deleted, the three in-module regression anchors (which currently compare against those APIs) are rewritten to use committed golden hash constants, and the `tags` / `TaggedWith` bug fix is bundled with TypeScript regeneration.

**Tech Stack:** Rust (`temper-core`, `temper-cli`, `temper-api`, `temper-e2e`), `serde_yaml`, `serde_json`, existing `hash::compute_managed_hash` / `compute_open_hash` primitives, TypeScript regeneration via `cargo make generate-ts-types`.

---

## Branch + Entry State

- **Branch:** `jct/frontmatter-consolidation` (already checked out, 11 commits ahead of `origin/main`, clean tree at `abe4eb0`)
- **PR:** draft #43 — Session 2 commits land on this branch, PR stays draft until Session 3 ships.
- **Session 1 outputs (in-tree):**
  - `crates/temper-core/src/frontmatter/{mod,parse,tiers,canonical,registry,fields,projections,document}.rs` — new module, 284 tests green
  - `crates/temper-core/tests/frontmatter_test.rs` + `tests/fixtures/frontmatter/*.md` + goldens
  - `DocType::schema_json()` inherent method on `crate::frontmatter::DocType` (from PR #43 feedback commit `abe4eb0`)
- **Session 1 legacy-comparison anchors still present in the new module:**
  - `src/frontmatter/tiers.rs:269` `matches_legacy_split_for_task_fixture` (calls `crate::hash::split_frontmatter_tiers`)
  - `src/frontmatter/tiers.rs:290` `matches_legacy_split_for_session_fixture` (same)
  - `src/frontmatter/document.rs:379-393` `hashes_match_legacy_path_byte_for_byte` (calls `crate::hash::compute_frontmatter_hashes_from_yaml`)
  - `tests/frontmatter_test.rs:97-116` `hashes_are_byte_identical_to_legacy_path_per_doctype` (same)
  - These are deliberate — they prove Session 1 matches legacy byte-for-byte. Task 10 rewrites them to golden-hash form so Task 11 can delete the legacy APIs.

## Known SG-13 Rule (Active for This Session)

From `~/.claude/skills/temper/subagent-guidance.md`: **No stringly-typed matches over bounded sets.** Every subagent dispatched for this plan will receive SG-13 in its prompt. The specific application here: `Frontmatter::try_from` already returns a typed `DocType`, so migrated call sites should prefer `fm.doc_type()` (enum) over re-parsing the string, and any new helpers that take a doctype should accept `DocType`, not `&str`.

## Legacy API Surface to Delete (Task 11)

These three functions and one private helper get removed after all callers migrate:

```rust
// crates/temper-core/src/hash.rs
pub fn split_frontmatter_tiers(
    frontmatter: &serde_yaml::Value,
    doc_type: &str,
) -> (serde_json::Value, serde_json::Value) { /* ~60 lines */ }

pub fn compute_frontmatter_hashes_from_yaml(
    frontmatter: Option<&serde_yaml::Value>,
    doc_type: &str,
) -> (String, String) { /* ~20 lines */ }

// crates/temper-core/src/normalize.rs
fn split_frontmatter_block<'a>(content: &'a str, path: &Path)
    -> Result<(&'a str, &'a str)> { /* ~40 lines (private) */ }
```

The supporting primitives `hash::compute_managed_hash`, `hash::compute_open_hash`, `hash::compute_body_hash`, `hash::canonicalize_json`, `hash::doc_type_from_vault_path` stay — `Frontmatter::hashes()` delegates to them unchanged, and e2e seed code still uses `compute_managed_hash` + `compute_open_hash` directly for tests that construct JSON server-side.

## Migration Pattern Library

Every migration boils down to one of four patterns. Later tasks reference these by name.

### Pattern P1 — Replace `compute_frontmatter_hashes_from_yaml` with `Frontmatter::hashes()`

**Old:**
```rust
let (managed_hash, open_hash) = temper_core::hash::compute_frontmatter_hashes_from_yaml(
    crate::vault::parse_frontmatter(&content).as_ref(),
    doc_type,
);
```

**New:**
```rust
let (managed_hash, open_hash) = empty_hashes_fallback(
    temper_core::frontmatter::Frontmatter::try_from(content.as_str()),
    doc_type,
);
```

Where `empty_hashes_fallback` is a small free helper added to `sync.rs` (Task 3, before the first call site that needs it):

```rust
/// Behavior-preserving wrapper that returns the `Frontmatter::hashes()`
/// result on success, or hashes of empty `{}` JSON on parse failure.
/// Matches the legacy `compute_frontmatter_hashes_from_yaml(None, ..)`
/// behavior of silently treating files-without-frontmatter as empty.
fn empty_hashes_fallback(
    parsed: temper_core::error::Result<temper_core::frontmatter::Frontmatter>,
    doc_type: &str,
) -> (String, String) {
    match parsed {
        Ok(fm) => fm.hashes(),
        Err(_) => (
            temper_core::hash::compute_managed_hash(doc_type, &serde_json::json!({})),
            temper_core::hash::compute_open_hash(&serde_json::json!({})),
        ),
    }
}
```

Rationale: the legacy `compute_frontmatter_hashes_from_yaml` silently treats `None` (parse failure, missing frontmatter) as `(empty managed, empty open)`. For a sync-sensitive session we preserve that exactly. A later cleanup session can decide whether to surface the error.

### Pattern P2 — Replace `split_frontmatter_tiers` with `Frontmatter::{managed_json, open_json}`

**Old:**
```rust
let (managed_meta_json, open_meta) = temper_core::hash::split_frontmatter_tiers(fm, doc_type);
let (managed_hash, open_hash) =
    temper_core::hash::compute_frontmatter_hashes_from_yaml(Some(fm), doc_type);
```

**New:**
```rust
let fm_parsed = temper_core::frontmatter::Frontmatter::try_from(content.as_str())?;
let managed_meta_json = fm_parsed.managed_json();
let open_meta = fm_parsed.open_json();
let (managed_hash, open_hash) = fm_parsed.hashes();
```

Where `content: &str` is the in-memory file text. If the surrounding function takes `fm: &serde_yaml::Value` instead of raw text, change the signature to take `Frontmatter` (or the raw text) and migrate the caller in the same task.

### Pattern P3 — Replace `normalize::split_frontmatter_block` with `Frontmatter::parse_file` / `Frontmatter::try_from`

**Old (test-only after Task 2):**
```rust
let (yaml_text, body) = split_frontmatter_block(&content, &path)?;
```

**New:**
```rust
let fm = Frontmatter::try_from(content.as_str())?;
let body = fm.body();  // byte-identical
```

### Pattern P4 — `normalize_file` orchestration

**Old shape (lines 89-160 of `normalize.rs`):**
```rust
let original = std::fs::read_to_string(path)?;
let (yaml_text, body) = split_frontmatter_block(&original, path)?;
let original_value: serde_yaml::Value = serde_yaml::from_str(yaml_text)?;
let original_mapping = original_value.as_mapping()?.clone();
let mut normalized_mapping = original_mapping.clone();
apply_doc_type_defaults_yaml(doc_type, &mut normalized_mapping);
let normalized_value = serde_yaml::Value::Mapping(normalized_mapping.clone());
let issues = validate_allowing_provisional(doc_type, &normalized_value)?;
// ... hash + compose_file + write_atomic ...
```

**New shape:**
```rust
let mut fm = Frontmatter::parse_file(path)?;
let body_hash = compute_body_hash(fm.body());

// Snapshot pre-defaults hashes in case we end up not rewriting.
let pre_defaults_hashes = fm.hashes();

// Mutate the value in place to apply doc-type defaults.
if let Some(mapping) = fm.value_mut().as_mapping_mut() {
    apply_doc_type_defaults_yaml(doc_type, mapping);
}

let issues = fm.validate()?;
if !issues.is_empty() {
    // Non-conformant: hashes describe pre-defaults on-disk reality.
    let (managed_hash, open_hash) = pre_defaults_hashes;
    return Ok(NormalizeOutcome { changed: false, body_hash, managed_hash, open_hash, issues });
}

// Compare canonical-serialized output to on-disk text. If they differ, write.
let new_content = fm.serialize()?;
let original = std::fs::read_to_string(path)?;
let changed = new_content != original;
if changed && write {
    fm.write_to(path)?;
}
let (managed_hash, open_hash) = fm.hashes();
Ok(NormalizeOutcome { changed, body_hash, managed_hash, open_hash, issues: Vec::new() })
```

**Behavior delta to acknowledge:** the old path preserved original YAML key order when rewriting; the new path emits in canonical display order (identity → tier-1 → managed → known open → unknown open). For files that are already in canonical order, output is byte-identical (Session 1 proved this via the idempotent-fixed-point test at `document.rs:486`). For files that are out of order, `normalize_file` now rewrites them into canonical order — this is a deliberate part of the consolidation and is gated by the real-vault dry-run verification in Task 14.

## Shared Verification Gates

Every task that touches production code must end with:

```bash
cargo nextest run -p temper-core -p temper-cli 2>&1 | tail -30
```

Every task that touches sync.rs must additionally end with:

```bash
cargo nextest run -p temper-cli --features test-db 2>&1 | tail -30
cargo nextest run -p temper-e2e --features test-db 2>&1 | tail -60
```

The final task (14) runs the full matrix:

```bash
cargo make check
cargo nextest run --workspace --features test-db
cargo nextest run -p temper-e2e --features test-db
./target/debug/temper doctor  # real vault byte-diff against main
./target/debug/temper sync run --dry-run  # real vault, assert no unexpected changes
```

---

## Task List

### Task 1: Add `Frontmatter::value_mut()` accessor (prep for Task 2)

**Files:**
- Modify: `crates/temper-core/src/frontmatter/document.rs:81-230` (add method after `value()` at line 88)
- Test: inline `#[cfg(test)] mod tests` at the bottom of `document.rs`

**Context for implementer:** `normalize_file` needs to mutate the YAML value in place to apply doc-type defaults (e.g. inserting `temper-stage: backlog`). The existing `Frontmatter` type exposes `value(&self) -> &serde_yaml::Value` (line 88) but no mutable counterpart. Task 2 needs `value_mut()` to call `.as_mapping_mut()` and pass to `apply_doc_type_defaults_yaml`. This is a pure additive change — zero risk.

- [ ] **Step 1: Write the failing test**

Add to the tests module in `crates/temper-core/src/frontmatter/document.rs`:

```rust
#[test]
fn value_mut_allows_in_place_mutation() {
    let mut fm = Frontmatter::try_from(TASK_FIXTURE).unwrap();
    if let Some(mapping) = fm.value_mut().as_mapping_mut() {
        mapping.insert(
            serde_yaml::Value::String("injected".into()),
            serde_yaml::Value::String("value".into()),
        );
    }
    // Mutation visible through the immutable accessor.
    let m = fm.value().as_mapping().unwrap();
    assert_eq!(
        m.get(serde_yaml::Value::String("injected".into()))
            .and_then(|v| v.as_str()),
        Some("value")
    );
}
```

- [ ] **Step 2: Run test to verify it fails**

```bash
cargo nextest run -p temper-core value_mut_allows_in_place_mutation 2>&1 | tail -20
```

Expected: compile error — `value_mut` not defined on `Frontmatter`.

- [ ] **Step 3: Add the `value_mut` method**

In `crates/temper-core/src/frontmatter/document.rs`, insert after the `value()` method (currently at line 88-90):

```rust
/// Mutable access to the canonicalized frontmatter value.
///
/// Used by higher-level orchestrators (e.g. `normalize_file`) that
/// need to inject doc-type defaults before writing back. Callers
/// that mutate this value are responsible for maintaining the
/// alias-normalized + mapping-typed invariant.
pub fn value_mut(&mut self) -> &mut serde_yaml::Value {
    &mut self.value
}
```

- [ ] **Step 4: Run test to verify it passes**

```bash
cargo nextest run -p temper-core value_mut_allows_in_place_mutation 2>&1 | tail -20
```

Expected: PASS.

- [ ] **Step 5: Run the full `temper-core` suite to confirm no regressions**

```bash
cargo nextest run -p temper-core 2>&1 | tail -15
```

Expected: all tests pass (should be 284 + 1 = 285 as of Session 1).

- [ ] **Step 6: Commit**

```bash
git add crates/temper-core/src/frontmatter/document.rs
git commit -m "$(cat <<'EOF'
feat(frontmatter): add Frontmatter::value_mut() for normalize_file migration

Needed for session 2's normalize_file migration to apply doc-type
defaults in place via serde_yaml::Mapping::insert. Pure additive
change; no existing caller touched.
EOF
)"
```

---

### Task 2: Migrate `normalize_file` to `Frontmatter` pipeline + retire `normalize::split_frontmatter_block`

**Files:**
- Modify: `crates/temper-core/src/normalize.rs:1-160` (rewrite `normalize_impl`, delete `split_frontmatter_block` private helper, delete `compose_file` private helper, remove legacy imports)
- Modify: `crates/temper-core/src/normalize.rs:600-680` (update 3 tests that call `split_frontmatter_block` or `compute_frontmatter_hashes_from_yaml` directly)

**Context for implementer:** This task is the single highest-risk production migration in the session. `normalize_file` is called by every `temper sync` command path (`rehash_manifest`, `apply_pull_meta_only`, `doctor`). Its invariants: (1) idempotent — second call is a no-op, (2) preserves body byte-for-byte, (3) only rewrites when defaults were inserted or validation passed on mutated state. Session 1 proved `Frontmatter::serialize` is a fixed point and that its hashes match `compute_frontmatter_hashes_from_yaml` byte-for-byte per doctype. This task leverages those invariants to produce an equivalent implementation with ~70% less code.

**Behavior change to acknowledge:** `normalize_file` now emits canonical display order on rewrite. For files already in canonical order (which the real vault is, per Session 1's byte-identical doctor verification), output is unchanged. The Task 14 real-vault dry-run is the verification gate.

- [ ] **Step 1: Run the existing normalize tests as a baseline**

```bash
cargo nextest run -p temper-core normalize 2>&1 | tail -20
```

Expected: all 12 existing normalize tests pass (per `normalize.rs:341-692`). Record their names — Step 5 confirms they still pass after rewrite.

- [ ] **Step 2: Rewrite the imports and `normalize_impl` body**

Replace the entire contents of `crates/temper-core/src/normalize.rs` lines 1-160 (everything up to and including `fn normalize_impl`). The new top of the file:

```rust
//! File normalization primitive — schema validate, apply defaults to YAML
//! frontmatter, rewrite the file if changed, and recompute three hashes.
//!
//! Used by every `temper sync` subcommand and by `temper doctor` to enforce
//! the invariant that on-disk file state matches the normalized form its
//! doc-type schema declares.
//!
//! This module is now a thin orchestrator over [`crate::frontmatter::Frontmatter`].
//! All YAML parsing, tier splitting, hashing, canonical serialization, and
//! atomic write logic lives in that module; `normalize_file` adds exactly one
//! responsibility on top: apply doc-type-specific defaults to the parsed
//! value before validating and writing back.

use crate::error::{Result, TemperError};
use crate::frontmatter::Frontmatter;
use crate::hash::compute_body_hash;
use crate::schema::ValidationIssue;
use std::path::Path;

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// Result of normalizing a single file.
#[derive(Debug, Clone)]
pub struct NormalizeOutcome {
    /// True if the file was rewritten to disk (defaults materialized or
    /// frontmatter canonicalized). False if the file was already canonical
    /// or if `issues` contained a non-auto-fixable error.
    pub changed: bool,

    /// SHA-256 hash of the markdown body.
    pub body_hash: String,

    /// Managed-tier hash, computed on the final state.
    pub managed_hash: String,

    /// Open-tier hash, computed on the final state.
    pub open_hash: String,

    /// Schema violations. Empty means conformant.
    pub issues: Vec<ValidationIssue>,
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Read a vault file, validate its frontmatter against the doc-type schema,
/// materialize any missing doc-type defaults, and return the three hashes
/// computed on the final state.
///
/// If the file has non-auto-fixable validation issues, it is NOT rewritten
/// and `issues` is non-empty; hashes in that case describe the on-disk
/// (pre-defaults) state so the caller can update its manifest.
///
/// # Errors
/// Returns [`TemperError::Config`] if the file is missing, has no
/// frontmatter block, or contains invalid YAML (propagated from
/// [`Frontmatter::parse_file`]).
pub fn normalize_file(path: &Path, doc_type: &str) -> Result<NormalizeOutcome> {
    normalize_impl(path, doc_type, true)
}

/// Dry-run variant — runs the same validation and default-materialization
/// logic but never writes to disk. `changed` indicates what a real
/// [`normalize_file`] call WOULD do.
pub fn normalize_file_inspect(path: &Path, doc_type: &str) -> Result<NormalizeOutcome> {
    normalize_impl(path, doc_type, false)
}

// ---------------------------------------------------------------------------
// Internal: shared implementation
// ---------------------------------------------------------------------------

fn normalize_impl(path: &Path, doc_type: &str, write: bool) -> Result<NormalizeOutcome> {
    // Parse the file through the authoritative frontmatter module.
    // Alias normalization and YAML parsing happen inside `parse_file`.
    let mut fm = Frontmatter::parse_file(path)?;

    // Sanity check: the filesystem-inferred doc_type should agree with
    // what the frontmatter declares. Surface a mismatch rather than
    // silently trusting one over the other.
    if fm.doc_type().as_str() != doc_type {
        return Err(TemperError::Config(format!(
            "doc_type mismatch for {}: frontmatter says '{}', caller says '{}'",
            path.display(),
            fm.doc_type().as_str(),
            doc_type
        )));
    }

    let body_hash = compute_body_hash(fm.body());

    // Snapshot hashes BEFORE applying defaults. We need these if validation
    // fails, so the caller's manifest can record on-disk reality.
    let pre_defaults_hashes = fm.hashes();

    // Apply doc-type-specific defaults by mutating the YAML mapping in place.
    // The `Frontmatter` type's alias-normalized + mapping invariant still
    // holds because we only insert canonical keys.
    if let Some(mapping) = fm.value_mut().as_mapping_mut() {
        apply_doc_type_defaults_yaml(doc_type, mapping);
    }

    // Validate the post-defaults state.
    let issues = fm.validate()?;

    if !issues.is_empty() {
        // Non-conformant: don't rewrite. Hashes describe pre-defaults state.
        let (managed_hash, open_hash) = pre_defaults_hashes;
        return Ok(NormalizeOutcome {
            changed: false,
            body_hash,
            managed_hash,
            open_hash,
            issues,
        });
    }

    // Compare canonical-serialized output against the on-disk text.
    // If they differ, write atomically via `Frontmatter::write_to`.
    let new_content = fm.serialize()?;
    let original = std::fs::read_to_string(path)
        .map_err(|e| TemperError::Config(format!("failed to read {}: {e}", path.display())))?;
    let changed = new_content != original;

    if changed && write {
        fm.write_to(path)?;
    }

    let (managed_hash, open_hash) = fm.hashes();

    Ok(NormalizeOutcome {
        changed,
        body_hash,
        managed_hash,
        open_hash,
        issues: Vec::new(),
    })
}
```

Delete the now-unused `split_frontmatter_block`, `find_closing_fence`, `compose_file`, and `write_atomic` private helpers (lines 244-337). They have no callers outside this file (they were already private). `apply_doc_type_defaults_yaml`, `default_keys_for`, and `is_missing_default` stay — they're the normalize module's actual responsibility.

- [ ] **Step 3: Update tests that reference deleted private helpers**

Test at `normalize.rs:604` (`normalize_preserves_body_content_exactly`) calls `split_frontmatter_block(&on_disk, &path)` to extract the body for comparison. Replace with:

```rust
    let on_disk = read_file(&path);
    let fm_on_disk = Frontmatter::try_from(on_disk.as_str()).expect("parse normalized file");
    assert_eq!(
        fm_on_disk.body(),
        body,
        "body should be byte-identical after normalize"
    );
```

Test at `normalize.rs:650` (`normalize_hash_matches_direct_hash_helper`) calls `compute_frontmatter_hashes_from_yaml(Some(&value), "task")` to build expected hashes. Replace with the new module's equivalent:

```rust
    #[test]
    fn normalize_hash_matches_frontmatter_hashes_helper() {
        let dir = tempdir().unwrap();
        let canonical_text = r#"---
temper-id: "019d8110-8ff3-70c2-85ae-57e04ed6288b"
temper-type: task
temper-context: temper
temper-created: "2026-04-12T00:00:00Z"
title: Test
slug: test
temper-stage: backlog
---
hello body
"#;
        let path = write_file(dir.path(), "task.md", canonical_text);

        let outcome = normalize_file(&path, "task").expect("normalize ok");
        assert!(!outcome.changed);

        // Compare against a fresh Frontmatter parse of the same canonical text.
        let fm = Frontmatter::try_from(canonical_text).expect("parse ok");
        let (direct_managed, direct_open) = fm.hashes();
        let direct_body = compute_body_hash(fm.body());

        assert_eq!(outcome.body_hash, direct_body);
        assert_eq!(outcome.managed_hash, direct_managed);
        assert_eq!(outcome.open_hash, direct_open);
    }
```

(Rename the test so the old name `normalize_hash_matches_direct_hash_helper` doesn't linger with misleading contents.)

Also delete the test's previous `compose_file` call — the new test constructs the canonical form as a string literal directly.

Test at `normalize.rs:400` (`normalize_task_already_canonical_is_noop`) uses `compose_file(&value, "body content\n")`. Since `compose_file` is deleted, replace with an inline string literal in canonical form:

```rust
    #[test]
    fn normalize_task_already_canonical_is_noop() {
        let dir = tempdir().unwrap();
        // Inline canonical form matches the order emitted by
        // Frontmatter::serialize — identity → tier1 → managed in schema order.
        let canonical = r#"---
temper-id: "019d8110-8ff3-70c2-85ae-57e04ed62885"
temper-type: task
temper-context: temper
temper-created: "2026-04-12T00:00:00Z"
title: Test
slug: test
temper-stage: in-progress
---
body content
"#;
        let path = write_file(dir.path(), "task.md", canonical);
        let before = read_file(&path);

        let outcome = normalize_file(&path, "task").expect("normalize ok");
        assert!(!outcome.changed, "canonical file should not be rewritten");
        assert!(outcome.issues.is_empty());
        let after = read_file(&path);
        assert_eq!(before, after, "file content should be byte-identical");
        assert!(outcome.managed_hash.starts_with("sha256:"));
        assert!(outcome.open_hash.starts_with("sha256:"));
        assert!(outcome.body_hash.starts_with("sha256:"));
    }
```

Also add the needed import at the top of the tests module:

```rust
    use crate::frontmatter::Frontmatter;
```

(Or `use super::*; use crate::frontmatter::Frontmatter;` depending on current test module style.)

- [ ] **Step 4: Run all normalize tests and verify they pass**

```bash
cargo nextest run -p temper-core normalize 2>&1 | tail -30
```

Expected: all 12 existing tests pass (with the rename for the hash-helper test). If any test fails, the failure points at a behavior regression — investigate before continuing. Key tests to watch:

- `normalize_task_missing_stage_rewrites_with_default`
- `normalize_task_already_canonical_is_noop`
- `normalize_task_invalid_enum_does_not_rewrite`
- `normalize_provisional_task_validates_clean`
- `normalize_goal_missing_status_rewrites_with_default`
- `normalize_session_missing_date_rewrites_with_default`
- `normalize_preserves_key_order`
- `normalize_preserves_body_content_exactly`
- `normalize_file_inspect_does_not_write`
- `normalize_file_missing_frontmatter_errors`
- `normalize_hash_matches_frontmatter_hashes_helper` (renamed from `_direct_hash_helper`)
- `apply_doc_type_defaults_yaml_no_overwrite`

- [ ] **Step 5: Run `cargo make check` to catch clippy / doc / machete issues**

```bash
cargo make check 2>&1 | tail -30
```

Expected: clean. If clippy flags the unused imports from the old path (`compute_frontmatter_hashes_from_yaml`, `validate_allowing_provisional`, the old `split_frontmatter_block` local — it shouldn't, those are gone), remove them.

- [ ] **Step 6: Full `temper-core` + `temper-cli` sync test suite**

```bash
cargo nextest run -p temper-core -p temper-cli 2>&1 | tail -30
```

Expected: all green. `temper-cli` still calls the legacy `compute_frontmatter_hashes_from_yaml` — those call sites are the next tasks — but nothing it does should break from the normalize rewrite.

- [ ] **Step 7: Commit**

```bash
git add crates/temper-core/src/normalize.rs
git commit -m "$(cat <<'EOF'
refactor(normalize): rewrite normalize_file to use Frontmatter pipeline

normalize_file now parses via Frontmatter::parse_file, applies defaults
through value_mut(), validates, and writes back via write_to. Deletes
the private split_frontmatter_block, find_closing_fence, compose_file,
and write_atomic helpers — all logic now lives in temper-core::frontmatter.

Tests that referenced the deleted helpers are rewritten to use
Frontmatter::try_from. Test rename: normalize_hash_matches_direct_hash_helper
→ normalize_hash_matches_frontmatter_hashes_helper.

Note: normalize_file now emits canonical display order on rewrite (was:
original YAML key order). Files already in canonical form are byte-
identical through this change; verified in task 14 via real-vault dry-run.
EOF
)"
```

---

### Task 3: Migrate sync.rs push-side callers (`build_meta_update_payload`, `push_resource_body`) + add `empty_hashes_fallback` helper

**Files:**
- Modify: `crates/temper-cli/src/actions/sync.rs` — add helper near line 280, migrate `build_meta_update_payload` (lines 796-813), migrate `push_resource_body` (lines 882-1016)

**Context for implementer:** These are the two sync.rs functions that push local state up to the server. `build_meta_update_payload` takes pre-parsed YAML and is called by `push_resource_meta_only` at line 861. `push_resource_body` handles full-body pushes and has two hash call sites (line 918 uses `split_frontmatter_tiers`, line 999 uses `compute_frontmatter_hashes_from_yaml` for the post-push "what did we send" hashes). Both functions need to change in the same commit because `build_meta_update_payload`'s signature changes.

- [ ] **Step 1: Add the `empty_hashes_fallback` helper near line 280 of sync.rs**

Insert after `fn strip_frontmatter` (currently at line 401) but before `extract_frontmatter_block`. Or more logically, insert right after `file_mtime_secs` at line 349:

```rust
/// Behavior-preserving wrapper: returns `Frontmatter::hashes()` on
/// successful parse, or the hashes of empty `{}` JSON on parse failure.
///
/// Matches the legacy `compute_frontmatter_hashes_from_yaml(None, ..)`
/// semantics of silently treating files-without-frontmatter as empty.
/// Sync callers depend on this silent-swallow behavior; surfacing the
/// error is a separate cleanup.
fn empty_hashes_fallback(
    parsed: temper_core::error::Result<temper_core::frontmatter::Frontmatter>,
    doc_type: &str,
) -> (String, String) {
    match parsed {
        Ok(fm) => fm.hashes(),
        Err(_) => (
            temper_core::hash::compute_managed_hash(doc_type, &serde_json::json!({})),
            temper_core::hash::compute_open_hash(&serde_json::json!({})),
        ),
    }
}
```

Also add the import at the top of the file if not already present:

```rust
use temper_core::frontmatter::Frontmatter;
```

- [ ] **Step 2: Migrate `build_meta_update_payload` (lines 796-813)**

Current signature and body:

```rust
fn build_meta_update_payload(
    fm: &serde_yaml::Value,
    doc_type: &str,
    resource_id: Uuid,
) -> MetaUpdatePayload {
    let (managed_meta_json, open_meta) = temper_core::hash::split_frontmatter_tiers(fm, doc_type);
    let (managed_hash, open_hash) =
        temper_core::hash::compute_frontmatter_hashes_from_yaml(Some(fm), doc_type);
    let managed_meta: temper_core::types::managed_meta::ManagedMeta =
        serde_json::from_value(managed_meta_json).unwrap_or_default();
    MetaUpdatePayload {
        resource_id: ResourceId::from(resource_id),
        managed_meta,
        open_meta,
        managed_hash,
        open_hash,
    }
}
```

Replace with a version that takes `&Frontmatter` instead of `&serde_yaml::Value`:

```rust
/// Build a meta-only update payload from a parsed Frontmatter.
///
/// Splits frontmatter into managed/open tiers, computes their hashes, and
/// returns a typed `MetaUpdatePayload` ready to send to the server. The
/// managed tier round-trips through `ManagedMeta`'s `extra` flatten bucket
/// so the pre-deserialized JSON hash stays stable.
fn build_meta_update_payload(
    fm: &Frontmatter,
    resource_id: Uuid,
) -> MetaUpdatePayload {
    let managed_meta_json = fm.managed_json();
    let open_meta = fm.open_json();
    let (managed_hash, open_hash) = fm.hashes();
    let managed_meta: temper_core::types::managed_meta::ManagedMeta =
        serde_json::from_value(managed_meta_json).unwrap_or_default();
    MetaUpdatePayload {
        resource_id: ResourceId::from(resource_id),
        managed_meta,
        open_meta,
        managed_hash,
        open_hash,
    }
}
```

The `doc_type` parameter is gone — `Frontmatter` carries the doctype internally.

- [ ] **Step 3: Update the caller in `push_resource_meta_only` (around line 861)**

Current code at lines 854-861:

```rust
    let fm = crate::vault::parse_frontmatter(&content).ok_or_else(|| {
        TemperError::Config(format!(
            "meta-only push requires frontmatter: {}",
            file_path.display()
        ))
    })?;

    let payload = build_meta_update_payload(&fm, &doc_type, entry_id.into());
```

Replace with:

```rust
    let fm = Frontmatter::try_from(content.as_str()).map_err(|e| {
        TemperError::Config(format!(
            "meta-only push requires parseable frontmatter at {}: {e}",
            file_path.display()
        ))
    })?;

    // Sanity check: the manifest-derived doc_type should agree with the
    // parsed frontmatter. Mismatch here means the manifest path is out
    // of sync with file contents — refuse the push rather than corrupt
    // the server's tier routing.
    if fm.doc_type().as_str() != doc_type {
        return Err(TemperError::Config(format!(
            "meta-only push: manifest path says doc_type '{}' but file frontmatter says '{}': {}",
            doc_type,
            fm.doc_type().as_str(),
            file_path.display()
        )));
    }

    let payload = build_meta_update_payload(&fm, entry_id.into());
```

Also update the comment at lines 841-844 since `split_frontmatter_tiers` is no longer the name:

```rust
    // Unlike the body push path, we cannot fall back to a default doc_type
    // here — `Frontmatter::managed_json` uses the parsed doctype to decide
    // which fields are managed vs open, and a doc_type mismatch would
    // misclassify fields and corrupt the server-side meta state.
```

- [ ] **Step 4: Migrate `push_resource_body` (lines 882-1016)**

Two call sites: line 918 (`split_frontmatter_tiers` on parse) and line 999 (`compute_frontmatter_hashes_from_yaml` post-push).

At line 916-922, replace:

```rust
    // Parse frontmatter and split into managed/open tiers
    let (managed_meta, open_meta) = if let Some(fm) = crate::vault::parse_frontmatter(&content) {
        let (m, o) = temper_core::hash::split_frontmatter_tiers(&fm, &doc_type);
        (Some(m), Some(o))
    } else {
        (None, None)
    };
```

with:

```rust
    // Parse frontmatter and split into managed/open tiers.
    let (managed_meta, open_meta) = match Frontmatter::try_from(content.as_str()) {
        Ok(fm) => (Some(fm.managed_json()), Some(fm.open_json())),
        Err(_) => (None, None),
    };
```

At line 996-1003, replace:

```rust
    // Compute frontmatter hashes so we can record them as the remote values
    let (pushed_managed_hash, pushed_open_hash) = {
        let current = std::fs::read_to_string(&file_path)?;
        temper_core::hash::compute_frontmatter_hashes_from_yaml(
            crate::vault::parse_frontmatter(&current).as_ref(),
            &doc_type,
        )
    };
```

with:

```rust
    // Compute frontmatter hashes so we can record them as the remote values.
    let (pushed_managed_hash, pushed_open_hash) = {
        let current = std::fs::read_to_string(&file_path)?;
        empty_hashes_fallback(Frontmatter::try_from(current.as_str()), &doc_type)
    };
```

- [ ] **Step 5: Run targeted sync tests**

```bash
cargo nextest run -p temper-cli push_meta_only_payload_roundtrip push_resource 2>&1 | tail -30
```

Expected: the existing `push_meta_only_payload_roundtrip` test FAILS on its signature check because it still calls `build_meta_update_payload(&fm, "task", id.into())` with the old `&serde_yaml::Value` + `doc_type` args. That's fine — Task 7 migrates sync.rs tests. For this task, the fix is to make it compile by:

1. Keep the test compiling but update the call-site: change `&fm` to `&Frontmatter::try_from(fm_text.as_str()).unwrap()` and drop the `"task"` arg.
2. Leave the assertions comparing against legacy APIs untouched for now (Task 7 rewrites those).

Apply the minimal compile-fix to `push_meta_only_payload_roundtrip` at line 3156-3204:

Change line 3173 from:

```rust
        let payload = build_meta_update_payload(&fm, "task", id.into());
```

to:

```rust
        let fm_parsed = Frontmatter::try_from(fm_text.as_str()).expect("parse fm");
        let payload = build_meta_update_payload(&fm_parsed, id.into());
```

Leave the rest of the test unchanged for this task — the legacy comparisons at lines 3177-3186 stay; Task 7 rewrites them.

- [ ] **Step 6: Run the sync test suite**

```bash
cargo nextest run -p temper-cli sync 2>&1 | tail -40
```

Expected: all sync tests pass. If `push_meta_only_payload_roundtrip` still fails, the minimal compile-fix in Step 5 was incomplete — re-check.

- [ ] **Step 7: Run `cargo make check`**

```bash
cargo make check 2>&1 | tail -20
```

Expected: clean.

- [ ] **Step 8: Commit**

```bash
git add crates/temper-cli/src/actions/sync.rs
git commit -m "$(cat <<'EOF'
refactor(sync): migrate push-side callers to Frontmatter module

- build_meta_update_payload now takes &Frontmatter instead of
  (&serde_yaml::Value, &str). doc_type is pulled from the parsed
  frontmatter.
- push_resource_body uses Frontmatter::{managed_json, open_json}
  for tier split and empty_hashes_fallback for post-push hashes.
- push_resource_meta_only now validates that the manifest-derived
  doc_type agrees with the parsed frontmatter; mismatch errors out.

Adds empty_hashes_fallback helper to preserve the legacy
compute_frontmatter_hashes_from_yaml(None, ..) behavior of
silently hashing empty objects for unparseable frontmatter.
EOF
)"
```

---

### Task 4: Migrate sync.rs rehash + scan callers (`rehash_manifest`, `scan_vault_for_untracked`)

**Files:**
- Modify: `crates/temper-cli/src/actions/sync.rs:287-338` (`rehash_manifest`)
- Modify: `crates/temper-cli/src/actions/sync.rs:446-566` (`scan_vault_for_untracked`)

**Context for implementer:** `rehash_manifest` is the core hash-refresh loop called at the start of every `sync_orchestration`. `scan_vault_for_untracked` walks the vault for new files and inserts them into the manifest with computed hashes. Both currently call `compute_frontmatter_hashes_from_yaml` with `parse_frontmatter(&content).as_ref()` — i.e. legacy `(None, doc_type)` behavior for unparseable files. Use the `empty_hashes_fallback` helper from Task 3.

- [ ] **Step 1: Migrate `rehash_manifest` hash computation (line 319)**

Current code at lines 316-322:

```rust
        // Compute frontmatter tier hashes
        let doc_type =
            temper_core::hash::doc_type_from_vault_path(&entry.path).unwrap_or("unknown");
        let (managed_hash, open_hash) = temper_core::hash::compute_frontmatter_hashes_from_yaml(
            crate::vault::parse_frontmatter(&content).as_ref(),
            doc_type,
        );
```

Replace with:

```rust
        // Compute frontmatter tier hashes via the authoritative module.
        let doc_type =
            temper_core::hash::doc_type_from_vault_path(&entry.path).unwrap_or("unknown");
        let (managed_hash, open_hash) =
            empty_hashes_fallback(Frontmatter::try_from(content.as_str()), doc_type);
```

- [ ] **Step 2: Migrate `scan_vault_for_untracked` hash computation (line 538)**

Current code at lines 533-541:

```rust
        let full_content = std::fs::read_to_string(path)?;
        let body = strip_frontmatter(&full_content);
        let content_hash = temper_core::hash::compute_body_hash(body);
        let mtime = file_mtime_secs(path).ok();

        let (managed_hash, open_hash) = temper_core::hash::compute_frontmatter_hashes_from_yaml(
            crate::vault::parse_frontmatter(&full_content).as_ref(),
            &doc_type,
        );
```

Replace with:

```rust
        let full_content = std::fs::read_to_string(path)?;
        let body = strip_frontmatter(&full_content);
        let content_hash = temper_core::hash::compute_body_hash(body);
        let mtime = file_mtime_secs(path).ok();

        let (managed_hash, open_hash) =
            empty_hashes_fallback(Frontmatter::try_from(full_content.as_str()), &doc_type);
```

- [ ] **Step 3: Run rehash + scan tests**

```bash
cargo nextest run -p temper-cli rehash scan 2>&1 | tail -40
```

Expected: all rehash/scan unit tests pass. Key tests: `rehash_detects_local_modification`, `rehash_marks_deleted_files`, `rehash_skips_unchanged_files_with_complete_hashes`, `rehash_backfills_empty_managed_open_hashes`, `rehash_detects_frontmatter_changes`, `rehash_detects_body_change_with_frontmatter`, `rehash_skips_file_when_mtime_matches_and_hashes_complete`, `rehash_backfills_when_mtime_matches_but_hashes_empty`, `rehash_processes_file_when_mtime_is_none`.

Note: `rehash_detects_frontmatter_changes` at line 2237 is a test that itself calls `split_frontmatter_tiers` to compute expected hashes. That test will still compile (legacy API not yet deleted) and still pass (legacy and new produce equal hashes). Task 7 rewrites it to use Frontmatter directly.

- [ ] **Step 4: Run the full sync suite**

```bash
cargo nextest run -p temper-cli sync 2>&1 | tail -40
```

Expected: all green. If `push_meta_only_payload_roundtrip` now has a compile error, that's fine — check that Task 3's Step 5 fix is still in place.

- [ ] **Step 5: `cargo make check`**

```bash
cargo make check 2>&1 | tail -20
```

Expected: clean.

- [ ] **Step 6: Commit**

```bash
git add crates/temper-cli/src/actions/sync.rs
git commit -m "$(cat <<'EOF'
refactor(sync): migrate rehash_manifest + scan to Frontmatter module

Both now use empty_hashes_fallback(Frontmatter::try_from(..), doc_type)
in place of compute_frontmatter_hashes_from_yaml(parse_frontmatter(..), ..).
Behavior-preserving — unparseable files still produce empty-object hashes.
EOF
)"
```

---

### Task 5: Migrate sync.rs pull-side callers (`apply_pull_meta_only`, `pull_resource_body`)

**Files:**
- Modify: `crates/temper-cli/src/actions/sync.rs:1121-1165` (`apply_pull_meta_only`)
- Modify: `crates/temper-cli/src/actions/sync.rs:1249-1361` (`pull_resource_body`)

- [ ] **Step 1: Migrate `apply_pull_meta_only` (line 1151)**

Current code at lines 1150-1154:

```rust
    let final_content = std::fs::read_to_string(file_path)?;
    let (managed_hash, open_hash) = temper_core::hash::compute_frontmatter_hashes_from_yaml(
        crate::vault::parse_frontmatter(&final_content).as_ref(),
        doc_type,
    );
```

Replace with:

```rust
    let final_content = std::fs::read_to_string(file_path)?;
    let (managed_hash, open_hash) =
        empty_hashes_fallback(Frontmatter::try_from(final_content.as_str()), doc_type);
```

- [ ] **Step 2: Migrate `pull_resource_body` (line 1335)**

Current code at lines 1334-1338:

```rust
    // Compute frontmatter tier hashes from the written file
    let (managed_hash, open_hash) = temper_core::hash::compute_frontmatter_hashes_from_yaml(
        crate::vault::parse_frontmatter(&full_content).as_ref(),
        &doc_type,
    );
```

Replace with:

```rust
    // Compute frontmatter tier hashes from the written file.
    let (managed_hash, open_hash) =
        empty_hashes_fallback(Frontmatter::try_from(full_content.as_str()), &doc_type);
```

- [ ] **Step 3: Run pull tests**

```bash
cargo nextest run -p temper-cli pull 2>&1 | tail -30
```

Expected: all pull tests pass. Key tests: `pull_meta_only_rebuild_preserves_body`, and any pull body / meta-only integration tests.

- [ ] **Step 4: Full sync suite**

```bash
cargo nextest run -p temper-cli sync 2>&1 | tail -30
```

Expected: green.

- [ ] **Step 5: Commit**

```bash
git add crates/temper-cli/src/actions/sync.rs
git commit -m "refactor(sync): migrate pull-side callers to Frontmatter module"
```

---

### Task 6: Migrate sync.rs merge + reset callers (`merge_and_push_resource`, reset path)

**Files:**
- Modify: `crates/temper-cli/src/actions/sync.rs:1416-1500` (`merge_and_push_resource`)
- Modify: `crates/temper-cli/src/actions/sync.rs:1750-1810` (reset path, around line 1779)

- [ ] **Step 1: Migrate `merge_and_push_resource` hash computation (line 1481)**

Current code at lines 1479-1484:

```rust
    // 8. Compute frontmatter hashes from the merged file
    let (pushed_managed_hash, pushed_open_hash) =
        temper_core::hash::compute_frontmatter_hashes_from_yaml(
            crate::vault::parse_frontmatter(&new_file_content).as_ref(),
            &doc_type,
        );
```

Replace with:

```rust
    // 8. Compute frontmatter hashes from the merged file.
    let (pushed_managed_hash, pushed_open_hash) =
        empty_hashes_fallback(Frontmatter::try_from(new_file_content.as_str()), &doc_type);
```

- [ ] **Step 2: Migrate the reset path (line 1779)**

Current code at lines 1776-1783:

```rust
        // Compute local frontmatter tier hashes
        let reset_doc_type =
            temper_core::hash::doc_type_from_vault_path(&rel_path).unwrap_or("unknown");
        let (local_managed_hash, local_open_hash) =
            temper_core::hash::compute_frontmatter_hashes_from_yaml(
                crate::vault::parse_frontmatter(&content).as_ref(),
                reset_doc_type,
            );
```

Replace with:

```rust
        // Compute local frontmatter tier hashes.
        let reset_doc_type =
            temper_core::hash::doc_type_from_vault_path(&rel_path).unwrap_or("unknown");
        let (local_managed_hash, local_open_hash) =
            empty_hashes_fallback(Frontmatter::try_from(content.as_str()), reset_doc_type);
```

- [ ] **Step 3: Run merge + reset tests**

```bash
cargo nextest run -p temper-cli merge reset 2>&1 | tail -30
```

Expected: all merge and reset tests pass.

- [ ] **Step 4: Confirm no remaining `compute_frontmatter_hashes_from_yaml` or `split_frontmatter_tiers` calls in sync.rs production code**

```bash
cargo nextest run -p temper-cli --features test-db 2>&1 | tail -30
```

And grep to verify:

```bash
```

(Skipped — use the Grep tool to confirm zero hits in sync.rs outside the `#[cfg(test)]` block. Task 7 handles the three test-only sites.)

- [ ] **Step 5: Commit**

```bash
git add crates/temper-cli/src/actions/sync.rs
git commit -m "refactor(sync): migrate merge + reset paths to Frontmatter module"
```

---

### Task 7: Migrate sync.rs unit tests (lines 2249, 3178, 3186)

**Files:**
- Modify: `crates/temper-cli/src/actions/sync.rs:2236-2294` (`rehash_detects_frontmatter_changes`)
- Modify: `crates/temper-cli/src/actions/sync.rs:3156-3204` (`push_meta_only_payload_roundtrip`)

**Context for implementer:** These two tests currently call the legacy `split_frontmatter_tiers` / `compute_frontmatter_hashes_from_yaml` APIs directly to build expected values. With those APIs scheduled for deletion in Task 11, this task rewrites them to use `Frontmatter::try_from` + `fm.managed_json()` + `fm.open_json()` + `fm.hashes()`. The assertions stay behaviorally identical.

- [ ] **Step 1: Migrate `rehash_detects_frontmatter_changes` (lines 2246-2252)**

Current code:

```rust
        // Compute hashes for v1
        let body_hash = temper_core::hash::compute_body_hash(strip_frontmatter(file_v1));
        let fm_v1 = crate::vault::parse_frontmatter(file_v1).unwrap();
        let (managed_meta_v1, open_meta_v1) =
            temper_core::hash::split_frontmatter_tiers(&fm_v1, "task");
        let managed_hash_v1 = temper_core::hash::compute_managed_hash("task", &managed_meta_v1);
        let open_hash_v1 = temper_core::hash::compute_open_hash(&open_meta_v1);
```

Replace with:

```rust
        // Compute hashes for v1 via the authoritative frontmatter module.
        let body_hash = temper_core::hash::compute_body_hash(strip_frontmatter(file_v1));
        let fm_v1 = Frontmatter::try_from(file_v1).expect("parse v1");
        let (managed_hash_v1, open_hash_v1) = fm_v1.hashes();
```

Also remove any now-unused imports at the top of the tests module (e.g., `use temper_core::hash::{compute_managed_hash, compute_open_hash};` if present).

Note: the test fixture `file_v1` doesn't carry `temper-type` frontmatter, so `Frontmatter::try_from` would fail with "missing required temper-type". Verify by inspecting the fixture at line 2242:

```
"---\ntitle: Old Title\ncreated: 2026-01-01\n---\n\n# My Document\n\nSome content here.\n"
```

Indeed, no `temper-type`. Two options:
1. **Add `temper-type: task` to the fixture.** Preserves the test's intent (detect frontmatter-only changes) and matches what real vault files contain.
2. **Keep the legacy code.** Not an option because Task 11 deletes the legacy API.

Go with option 1. Update the fixtures:

```rust
        let file_v1 = "---\ntemper-type: task\ntitle: Old Title\ncreated: 2026-01-01\n---\n\n# My Document\n\nSome content here.\n";
        let file_v2 = "---\ntemper-type: task\ntitle: New Title\ncreated: 2026-04-03\n---\n\n# My Document\n\nSome content here.\n";
```

The test still validates the same invariant: frontmatter-only changes (title, created) flip `ManifestEntryState::LocalModified`.

- [ ] **Step 2: Migrate `push_meta_only_payload_roundtrip` (lines 3156-3204)**

Current code at lines 3171-3186:

```rust
        let fm = crate::vault::parse_frontmatter(&fm_text).expect("parse fm");

        let payload = build_meta_update_payload(&fm, "task", id.into());

        // Direct comparison against the hashing helper — same input must
        // produce identical hashes.
        let (expected_managed, expected_open) =
            temper_core::hash::compute_frontmatter_hashes_from_yaml(Some(&fm), "task");
        assert_eq!(payload.managed_hash, expected_managed);
        assert_eq!(payload.open_hash, expected_open);

        // Direct comparison against split_frontmatter_tiers. Round-trip
        // the managed side through the typed ManagedMeta via the flatten
        // extras bucket so the hash stays stable.
        let (expected_managed_meta_json, expected_open_meta) =
            temper_core::hash::split_frontmatter_tiers(&fm, "task");
```

(Note: the Task 3 compile-fix already replaced the `&fm, "task"` call with `&fm_parsed`. If that change is still in place, the failing compile was on the *later* expected-value lines, not the `build_meta_update_payload` call itself.)

Replace lines 3171-3190 with:

```rust
        let fm = Frontmatter::try_from(fm_text.as_str()).expect("parse fm");

        let payload = build_meta_update_payload(&fm, id.into());

        // Direct comparison against the parsed Frontmatter's hashes —
        // same input must produce identical (managed_hash, open_hash).
        let (expected_managed, expected_open) = fm.hashes();
        assert_eq!(payload.managed_hash, expected_managed);
        assert_eq!(payload.open_hash, expected_open);

        // Direct comparison against the Frontmatter projections. Round-trip
        // the managed side through the typed ManagedMeta via the flatten
        // extras bucket so the hash stays stable.
        let expected_managed_meta_json = fm.managed_json();
        let expected_open_meta = fm.open_json();
```

The assertions at lines 3187-3203 that use `expected_managed_meta_json` and `expected_open_meta` stay unchanged — they compare payload struct fields to the locally-computed expected values.

- [ ] **Step 3: Run the two modified tests**

```bash
cargo nextest run -p temper-cli rehash_detects_frontmatter_changes push_meta_only_payload_roundtrip 2>&1 | tail -30
```

Expected: both pass.

- [ ] **Step 4: Full sync unit suite**

```bash
cargo nextest run -p temper-cli sync 2>&1 | tail -30
```

Expected: all green.

- [ ] **Step 5: Commit**

```bash
git add crates/temper-cli/src/actions/sync.rs
git commit -m "refactor(sync): migrate sync unit tests to Frontmatter module"
```

---

### Task 8: Migrate `tests/e2e/tests/sync_test.rs` seed helper

**Files:**
- Modify: `tests/e2e/tests/sync_test.rs:1260-1300` (the seed helper that calls legacy APIs at lines 1265 + 1269)

**Context for implementer:** This is an e2e seed helper that simulates a file landing in the vault with server-side ingest already having computed server hashes. It needs hashes from both the local file AND a `managed_meta_split` value to feed into a seed `MetaUpdatePayload`. The new pattern: parse once via `Frontmatter::try_from`, pull `managed_json()` + `open_json()` + `hashes()` from a single call.

- [ ] **Step 1: Migrate the e2e seed helper**

Current code at lines 1260-1299:

```rust
    // Compute hashes the same way `rehash_manifest` / `pull_resource_body` do.
    let body = temper_cli::actions::sync::strip_frontmatter(&vault_content);
    let local_body_hash = temper_core::hash::compute_body_hash(body);
    let fm_yaml = temper_cli::vault::parse_frontmatter(&vault_content);
    let (managed_meta_split, open_meta_split) = match fm_yaml.as_ref() {
        Some(fm) => temper_core::hash::split_frontmatter_tiers(fm, doc_type),
        None => (serde_json::json!({}), serde_json::json!({})),
    };
    let (managed_hash, open_hash) =
        temper_core::hash::compute_frontmatter_hashes_from_yaml(fm_yaml.as_ref(), doc_type);
```

Replace with:

```rust
    // Compute hashes via the authoritative frontmatter module.
    let body = temper_cli::actions::sync::strip_frontmatter(&vault_content);
    let local_body_hash = temper_core::hash::compute_body_hash(body);
    let (managed_meta_split, open_meta_split, managed_hash, open_hash) =
        match temper_core::frontmatter::Frontmatter::try_from(vault_content.as_str()) {
            Ok(fm) => {
                let managed = fm.managed_json();
                let open = fm.open_json();
                let (mh, oh) = fm.hashes();
                (managed, open, mh, oh)
            }
            Err(_) => (
                serde_json::json!({}),
                serde_json::json!({}),
                temper_core::hash::compute_managed_hash(doc_type, &serde_json::json!({})),
                temper_core::hash::compute_open_hash(&serde_json::json!({})),
            ),
        };
```

Also update the comment at lines 1287-1290:

```rust
    // `managed_meta_split` is a JSON Value from Frontmatter::managed_json;
    // deserialize into the typed `ManagedMeta` via the flatten extras
    // bucket so the hash stays stable through the round-trip.
```

- [ ] **Step 2: Run e2e sync tests**

```bash
cargo nextest run -p temper-e2e --features test-db sync 2>&1 | tail -40
```

Expected: all e2e sync tests pass. The Phase E2 sync suite (98+ tests) is the regression anchor here.

- [ ] **Step 3: Commit**

```bash
git add tests/e2e/tests/sync_test.rs
git commit -m "refactor(e2e): migrate sync seed helper to Frontmatter module"
```

---

### Task 9: Migrate `schema_test.rs` (lines 264, 268, 314, 317)

**Files:**
- Modify: `crates/temper-core/tests/schema_test.rs:264-323` (two test functions that use `hash::split_frontmatter_tiers`)

**Context for implementer:** These tests verify that identical open fields produce identical open hashes across doc types and that open hash changes when open fields change. Both pre-date the Frontmatter module and call `hash::split_frontmatter_tiers` directly. The test fixtures need `temper-context` and `temper-created` to parse via `Frontmatter::try_from` (which requires them), but since these tests are asserting on hashes (not struct shape), the fixtures can be padded minimally.

- [ ] **Step 1: Read the current two tests**

The two tests are `test_hash_tiers_open_hash_equal_for_same_fields` (around line 240-293) and `test_hash_tiers_open_hash_changes_with_open_fields` (around 295-324). Both construct fixtures via the local `yaml(s)` helper and call `hash::split_frontmatter_tiers` + `compute_managed_hash` + `compute_open_hash`.

- [ ] **Step 2: Replace `test_hash_tiers_open_hash_equal_for_same_fields` hash computation**

Current code at lines 264-270:

```rust
    let (managed1, open1_meta) = hash::split_frontmatter_tiers(&fm1, "task");
    let meta1 = hash::compute_managed_hash("task", &managed1);
    let open1 = hash::compute_open_hash(&open1_meta);

    let (managed2, open2_meta) = hash::split_frontmatter_tiers(&fm2, "goal");
    let meta2 = hash::compute_managed_hash("goal", &managed2);
    let open2 = hash::compute_open_hash(&open2_meta);
```

Replace with fixtures re-created as strings + parsed via Frontmatter:

```rust
    let fm1 = Frontmatter::try_from(
        r#"---
temper-id: "01930000-0000-7000-8000-000000000040"
temper-type: task
temper-context: my-project
temper-created: "2024-01-01T00:00:00Z"
title: "My Task"
open-field: "hello"
---
"#,
    )
    .expect("parse task fixture");
    let fm2 = Frontmatter::try_from(
        r#"---
temper-id: "01930000-0000-7000-8000-000000000041"
temper-type: goal
temper-context: my-project
temper-created: "2024-01-01T00:00:00Z"
title: "My Task"
open-field: "hello"
---
"#,
    )
    .expect("parse goal fixture");

    let (meta1, open1) = fm1.hashes();
    let (meta2, open2) = fm2.hashes();
```

The original test uses the `yaml()` helper + named `fm1`, `fm2` serde_yaml values. With the rewrite those YAML values are gone — delete the two `let fm1 = yaml(...)` / `let fm2 = yaml(...)` blocks above (currently at lines 242-262). Keep the function signature and the assertions at lines 272-292 (`assert_ne!(meta1, meta2)`, `assert_eq!(open1, open2)`, sha256 prefix checks).

Add the import at the top of `schema_test.rs`:

```rust
use temper_core::frontmatter::Frontmatter;
```

- [ ] **Step 3: Replace `test_hash_tiers_open_hash_changes_with_open_fields` hash computation**

Current code at lines 295-324:

```rust
#[test]
fn test_hash_tiers_open_hash_changes_with_open_fields() {
    let fm1 = yaml(
        r#"
temper-id: "01930000-0000-7000-8000-000000000050"
temper-type: task
title: "My Task"
custom: "hello"
"#,
    );
    let fm2 = yaml(
        r#"
temper-id: "01930000-0000-7000-8000-000000000050"
temper-type: task
title: "My Task"
custom: "world"
"#,
    );

    let (_, open1_meta) = hash::split_frontmatter_tiers(&fm1, "task");
    let open1 = hash::compute_open_hash(&open1_meta);

    let (_, open2_meta) = hash::split_frontmatter_tiers(&fm2, "task");
    let open2 = hash::compute_open_hash(&open2_meta);

    assert_ne!(
        open1, open2,
        "open_hash should change when open fields change"
    );
}
```

Replace with:

```rust
#[test]
fn test_hash_tiers_open_hash_changes_with_open_fields() {
    let fm1 = Frontmatter::try_from(
        r#"---
temper-id: "01930000-0000-7000-8000-000000000050"
temper-type: task
temper-context: my-project
temper-created: "2024-01-01T00:00:00Z"
title: "My Task"
custom: "hello"
---
"#,
    )
    .expect("parse fm1");
    let fm2 = Frontmatter::try_from(
        r#"---
temper-id: "01930000-0000-7000-8000-000000000050"
temper-type: task
temper-context: my-project
temper-created: "2024-01-01T00:00:00Z"
title: "My Task"
custom: "world"
---
"#,
    )
    .expect("parse fm2");

    let (_, open1) = fm1.hashes();
    let (_, open2) = fm2.hashes();

    assert_ne!(
        open1, open2,
        "open_hash should change when open fields change"
    );
}
```

- [ ] **Step 4: Run schema tests**

```bash
cargo nextest run -p temper-core schema 2>&1 | tail -30
```

Expected: all schema tests pass, including the two rewritten ones.

- [ ] **Step 5: Commit**

```bash
git add crates/temper-core/tests/schema_test.rs
git commit -m "refactor(schema-test): migrate hash tier tests to Frontmatter module"
```

---

### Task 10: Rewrite legacy regression anchors in the new frontmatter module to golden-hash form

**Files:**
- Modify: `crates/temper-core/src/frontmatter/tiers.rs:237-297` (two `matches_legacy_split_*` tests)
- Modify: `crates/temper-core/src/frontmatter/document.rs:378-393` (`hashes_match_legacy_path_byte_for_byte`)
- Modify: `crates/temper-core/tests/frontmatter_test.rs:97-116` (`hashes_are_byte_identical_to_legacy_path_per_doctype`)

**Context for implementer:** These three test sites are the Session 1 regression anchors that prove `Frontmatter::hashes()` produces byte-identical output to the legacy `hash::compute_frontmatter_hashes_from_yaml` path. They each call the legacy API and `assert_eq!` against the new path. Task 11 will delete the legacy API; this task has to rewrite these anchors into a form that survives the deletion while preserving their regression value. The chosen approach: capture the known-good hash strings as const `&str` golden values, then assert the `Frontmatter::hashes()` result matches. This turns the "new matches legacy" invariant into a "new matches this exact committed hash" invariant — equally strong, just anchored differently.

**Step 0: Capture the golden hashes from a clean run.** Before editing any test, run the existing anchor tests and capture the actual hash values they produce. Do this with a small debug printout or by running `cargo test` with a panic injection. Simpler: construct the goldens by running the parsed fixtures through `Frontmatter::hashes()` in an ad-hoc main-ish program, OR look at the hashes at test runtime via `dbg!()`. The cleanest approach is to run the existing passing tests once with a `dbg!(fm.hashes())` injection, read the values from test output, then remove the dbg and paste them in as constants.

The plan gives you concrete constants for three of the fixtures below. For the remaining fixtures in the integration test, use the same capture-and-paste approach.

- [ ] **Step 1: Capture golden hashes for the tiers.rs inline fixtures**

Temporarily add a `dbg!` to each of the two tests at lines 268-269 and 289-290:

```rust
        let (new_managed, new_open) = split_managed_open(&v, DocType::Task);
        dbg!(&new_managed, &new_open);
        let combined_managed = crate::hash::compute_managed_hash("task", &new_managed);
        let combined_open = crate::hash::compute_open_hash(&new_open);
        dbg!(&combined_managed, &combined_open);
```

Run just those tests with the output captured:

```bash
cargo nextest run -p temper-core --no-capture matches_legacy_split_for_task_fixture matches_legacy_split_for_session_fixture 2>&1 | grep -E 'combined_|managed|open'
```

Record the printed `combined_managed` and `combined_open` strings. Revert the `dbg!` lines before committing.

- [ ] **Step 2: Rewrite `tiers.rs` regression anchors as golden-hash tests**

Replace the two tests at lines 237-297 with:

```rust
    // Regression anchor: the tier-split output produces stable hashes
    // for a known task fixture. The golden hashes below were captured
    // at session 2 and must not change without a schema change or a
    // deliberate canonicalization update.
    #[test]
    fn task_fixture_produces_stable_hashes() {
        let v = yaml(
            r#"
temper-id: "019d8110-8ff3-70c2-85ae-57e04ed62885"
temper-type: task
temper-context: temper
temper-created: "2026-04-13T00:00:00Z"
temper-updated: "2026-04-13T00:00:00Z"
title: T
slug: t
temper-stage: in-progress
temper-mode: build
temper-effort: small
temper-seq: 1
relates_to: [a]
depends_on: [b]
tags: [auth]
custom: ok
"#,
        );
        let (managed, open) = split_managed_open(&v, DocType::Task);
        let managed_hash = crate::hash::compute_managed_hash("task", &managed);
        let open_hash = crate::hash::compute_open_hash(&open);

        // Golden hashes captured from session 2 task 10. If these change,
        // either the schema or canonicalization algorithm moved; investigate
        // before regenerating.
        assert_eq!(
            managed_hash,
            "sha256:REPLACE_WITH_TASK_MANAGED_HASH",
            "task fixture managed hash drift"
        );
        assert_eq!(
            open_hash,
            "sha256:REPLACE_WITH_TASK_OPEN_HASH",
            "task fixture open hash drift"
        );
    }

    #[test]
    fn session_fixture_produces_stable_hashes() {
        let v = yaml(
            r#"
temper-id: "019d8110-8ff3-70c2-85ae-57e04ed62885"
temper-type: session
temper-context: temper
temper-created: "2026-04-13T00:00:00Z"
title: S
slug: s
date: "2026-04-13"
relates_to: [a]
tags: [x]
"#,
        );
        let (managed, open) = split_managed_open(&v, DocType::Session);
        let managed_hash = crate::hash::compute_managed_hash("session", &managed);
        let open_hash = crate::hash::compute_open_hash(&open);

        assert_eq!(
            managed_hash,
            "sha256:REPLACE_WITH_SESSION_MANAGED_HASH",
            "session fixture managed hash drift"
        );
        assert_eq!(
            open_hash,
            "sha256:REPLACE_WITH_SESSION_OPEN_HASH",
            "session fixture open hash drift"
        );
    }
```

Replace each `REPLACE_WITH_*` placeholder with the actual hash string captured in Step 1.

- [ ] **Step 3: Capture the document.rs golden hashes**

The current test at line 379-393 uses `TASK_FIXTURE` from line 286. Inject a `dbg!(&new_managed, &new_open)` after line 381 (before the legacy-path block), run:

```bash
cargo nextest run -p temper-core --no-capture hashes_match_legacy_path 2>&1 | grep -E '"sha256:'
```

Record the two values. Revert the dbg.

- [ ] **Step 4: Rewrite `document.rs::hashes_match_legacy_path_byte_for_byte`**

Replace the current test at lines 378-393:

```rust
    #[test]
    fn hashes_match_legacy_path_byte_for_byte() {
        let fm = Frontmatter::try_from(TASK_FIXTURE).unwrap();
        let (new_managed, new_open) = fm.hashes();

        // Legacy path: parse the same YAML independently and run through
        // `compute_frontmatter_hashes_from_yaml`.
        let (yaml_text, _) = split_frontmatter_block(TASK_FIXTURE).unwrap();
        let mut legacy_value = parse_yaml(&yaml_text).unwrap();
        normalize_aliases(&mut legacy_value);
        let (legacy_managed, legacy_open) =
            crate::hash::compute_frontmatter_hashes_from_yaml(Some(&legacy_value), "task");

        assert_eq!(new_managed, legacy_managed);
        assert_eq!(new_open, legacy_open);
    }
```

with:

```rust
    #[test]
    fn task_fixture_hashes_are_golden() {
        // Regression anchor: the task fixture at TASK_FIXTURE produces
        // stable managed + open hashes. Goldens captured in session 2
        // task 10 when the legacy compute_frontmatter_hashes_from_yaml
        // API was deleted. If these change, either the schema or
        // canonicalization moved; investigate before regenerating.
        let fm = Frontmatter::try_from(TASK_FIXTURE).unwrap();
        let (managed_hash, open_hash) = fm.hashes();
        assert_eq!(
            managed_hash,
            "sha256:REPLACE_WITH_TASK_FIXTURE_MANAGED_HASH",
            "task fixture managed hash drift"
        );
        assert_eq!(
            open_hash,
            "sha256:REPLACE_WITH_TASK_FIXTURE_OPEN_HASH",
            "task fixture open hash drift"
        );
    }
```

Replace the placeholders with captured values. The imports at the top of the file (`use crate::frontmatter::parse::{normalize_aliases, parse_yaml, split_frontmatter_block};`) can stay — `split_frontmatter_block` and friends are still used by the current `try_from` impl.

- [ ] **Step 5: Rewrite `tests/frontmatter_test.rs::hashes_are_byte_identical_to_legacy_path_per_doctype`**

This test iterates over all 8 fixtures in `ROUND_TRIP_CASES` and compares new-path vs legacy-path hashes per doctype. With the legacy API gone, rewrite as a golden-hash table.

First, capture the 8 pairs of hashes. Add a temporary `dbg!(stem, new_managed, new_open)` to the loop body and run:

```bash
REGENERATE_GOLDENS=1 cargo nextest run -p temper-core --no-capture hashes_are_byte_identical 2>&1 | grep -E '"sha256:'
```

Record each `(stem, managed_hash, open_hash)` tuple.

Then replace the test at lines 97-116:

```rust
#[test]
fn hashes_are_byte_identical_to_legacy_path_per_doctype() {
    use temper_core::frontmatter::parse::{normalize_aliases, parse_yaml, split_frontmatter_block};
    use temper_core::hash::compute_frontmatter_hashes_from_yaml;

    for (stem, dt) in ROUND_TRIP_CASES {
        let content = load_fixture(&format!("{stem}.md"));
        let fm = Frontmatter::try_from(content.as_str()).unwrap();
        let (new_managed, new_open) = fm.hashes();

        let (yaml_text, _body) = split_frontmatter_block(&content).unwrap();
        let mut legacy_value = parse_yaml(&yaml_text).unwrap();
        normalize_aliases(&mut legacy_value);
        let (legacy_managed, legacy_open) =
            compute_frontmatter_hashes_from_yaml(Some(&legacy_value), dt.as_str());

        assert_eq!(new_managed, legacy_managed, "managed hash drift for {stem}");
        assert_eq!(new_open, legacy_open, "open hash drift for {stem}");
    }
}
```

with:

```rust
/// Golden per-fixture (managed_hash, open_hash) pairs captured in
/// session 2 task 10. These anchor hash stability across future
/// refactors. To regenerate after an intentional schema or
/// canonicalization change, set REGENERATE_FRONTMATTER_HASH_GOLDENS=1
/// and paste the dbg! output back here.
const FIXTURE_HASH_GOLDENS: &[(&str, &str, &str)] = &[
    // (stem, managed_hash, open_hash)
    ("task_minimal", "sha256:REPLACE", "sha256:REPLACE"),
    ("task_full", "sha256:REPLACE", "sha256:REPLACE"),
    ("task_with_aliases", "sha256:REPLACE", "sha256:REPLACE"),
    ("goal_full", "sha256:REPLACE", "sha256:REPLACE"),
    ("session_full", "sha256:REPLACE", "sha256:REPLACE"),
    ("research_full", "sha256:REPLACE", "sha256:REPLACE"),
    ("decision_full", "sha256:REPLACE", "sha256:REPLACE"),
    ("concept_full", "sha256:REPLACE", "sha256:REPLACE"),
];

#[test]
fn fixture_hashes_match_goldens() {
    for (stem, expected_managed, expected_open) in FIXTURE_HASH_GOLDENS {
        let content = load_fixture(&format!("{stem}.md"));
        let fm = Frontmatter::try_from(content.as_str()).unwrap();
        let (managed_hash, open_hash) = fm.hashes();
        assert_eq!(
            managed_hash, *expected_managed,
            "managed hash drift for {stem}"
        );
        assert_eq!(open_hash, *expected_open, "open hash drift for {stem}");
    }
}
```

Replace each `"sha256:REPLACE"` with the captured value. All 16 replacements are required.

- [ ] **Step 6: Run all three rewritten tests**

```bash
cargo nextest run -p temper-core task_fixture_produces_stable session_fixture_produces_stable task_fixture_hashes_are_golden fixture_hashes_match_goldens 2>&1 | tail -30
```

Expected: all pass. If any fail, the captured hash strings were copied wrong — re-run the capture step.

- [ ] **Step 7: Run the full `temper-core` suite**

```bash
cargo nextest run -p temper-core 2>&1 | tail -20
```

Expected: all green. Legacy-API callers in `tiers.rs`, `document.rs`, and `frontmatter_test.rs` are now rewritten, but the legacy API still exists — Task 11 deletes it.

- [ ] **Step 8: Commit**

```bash
git add crates/temper-core/src/frontmatter/tiers.rs crates/temper-core/src/frontmatter/document.rs crates/temper-core/tests/frontmatter_test.rs
git commit -m "$(cat <<'EOF'
test(frontmatter): rewrite legacy regression anchors as golden-hash tests

The three tier-split / hash regression anchors that previously asserted
new-path matches legacy-path are rewritten to assert against committed
sha256: golden constants captured at this commit. This preserves the
regression value (hash drift is still caught loudly) while decoupling
from the legacy hash::compute_frontmatter_hashes_from_yaml API that
task 11 is about to delete.
EOF
)"
```

---

### Task 11: Delete legacy APIs (`hash::split_frontmatter_tiers`, `hash::compute_frontmatter_hashes_from_yaml`)

**Files:**
- Modify: `crates/temper-core/src/hash.rs` — delete `pub fn split_frontmatter_tiers` (currently at line 117), delete `pub fn compute_frontmatter_hashes_from_yaml` (currently at line 176), delete the 7 test sites in `hash.rs` that call these functions (lines 328, 355, 394, 412, 490, 532, 537, 548)

**Context for implementer:** At this point every caller in `temper-core`, `temper-cli`, `temper-api`, `temper-e2e`, and the `tests/frontmatter_test.rs` integration file has migrated to `Frontmatter::*`. The legacy public APIs and their tests are the last things holding them alive. This task deletes them outright.

- [ ] **Step 1: Verify zero remaining callers via grep**

Use the Grep tool to confirm no production or test code outside `hash.rs` itself references the three legacy symbols. Expected output: only `crates/temper-core/src/hash.rs` and the spec / plan docs in `docs/superpowers/` appear.

```
(Grep tool query: "split_frontmatter_tiers|compute_frontmatter_hashes_from_yaml|normalize::split_frontmatter_block" with glob "**/*.rs" — expect zero hits outside hash.rs)
```

If any `.rs` file other than `hash.rs` still references these symbols, STOP and revisit whatever task was supposed to migrate it. Do not proceed with the deletion until the grep is clean.

- [ ] **Step 2: Delete `split_frontmatter_tiers` from `hash.rs`**

Remove lines 117-175 of `crates/temper-core/src/hash.rs` (the entire `pub fn split_frontmatter_tiers(...)` function). Also delete the doc comment above it.

- [ ] **Step 3: Delete `compute_frontmatter_hashes_from_yaml` from `hash.rs`**

Remove lines 176-194 (or whichever range the function body occupies) of `hash.rs`. Also delete the doc comment above it.

- [ ] **Step 4: Delete the `hash.rs` tests that called those functions**

Find the 7 test sites (lines 328, 355, 394, 412, 490, 532-548 based on earlier grep). Delete the tests that reference `split_frontmatter_tiers` or `compute_frontmatter_hashes_from_yaml`:

- `tests_split_frontmatter_tiers_task` (or similar name at 328)
- `tests_split_frontmatter_tiers_session` (355)
- Any `tests_compute_frontmatter_hashes_*` tests that directly test the wrapper (394, 412, 490, 532, 537, 548)

Use the Read tool to identify each test's full range, then delete. Keep tests that only exercise `compute_managed_hash` / `compute_open_hash` / `canonicalize_json` directly — those primitives are not being deleted.

- [ ] **Step 5: Run the full `temper-core` suite**

```bash
cargo nextest run -p temper-core 2>&1 | tail -30
```

Expected: all green. Test count drops by 7 (the deleted hash.rs tests) but everything else passes.

- [ ] **Step 6: Run `cargo make check` to catch any leftover imports / dead code**

```bash
cargo make check 2>&1 | tail -30
```

Expected: clean. If clippy / machete flag dead imports in any file that used to pull `use temper_core::hash::{split_frontmatter_tiers, compute_frontmatter_hashes_from_yaml};`, remove them. Candidates: `sync.rs` (may still have a stray import from Task 3-6), `normalize.rs` (should be clean from Task 2), test files.

- [ ] **Step 7: Run the full workspace suite with DB**

```bash
cargo nextest run --workspace --features test-db 2>&1 | tail -30
```

Expected: all green.

- [ ] **Step 8: Commit**

```bash
git add -A
git commit -m "$(cat <<'EOF'
refactor(hash): delete split_frontmatter_tiers + compute_frontmatter_hashes_from_yaml

Every caller has migrated to temper_core::frontmatter::Frontmatter
(sessions 1 + 2 tasks 1-10). Deletes the legacy public API functions,
their tests, and any leftover dead imports. Frontmatter::managed_json,
Frontmatter::open_json, and Frontmatter::hashes() are the single
authoritative replacements.
EOF
)"
```

---

### Task 12: Verify zero `tagged_with` rows + remove `ResourceRelationships::tags` field + delete `EdgeType::TaggedWith`

**Files:**
- Modify: `crates/temper-core/src/types/graph.rs` — remove `tags` field (line 97), remove `tags.is_empty()` from `is_empty` (line 113), remove `(&self.tags, EdgeType::TaggedWith)` from `to_edge_declarations` (line 131), remove `EdgeType::TaggedWith` variant (line 26), remove `Self::TaggedWith => ...` from `Display` (line 39), remove the test assertion at line 343
- Modify: `crates/temper-core/src/frontmatter/projections.rs:108-122` — update `TASK_FIXTURE` and `projects_to_resource_relationships` test that asserts `rels.tags`
- Modify: `crates/temper-core/tests/frontmatter_test.rs:246-265` — update `tags_as_strings_do_not_become_parent_of_relationships` to not assert `rels.tags`

**Context for implementer:** `ResourceRelationships::tags` is a `Vec<String>` field that currently gets mapped to `EdgeType::TaggedWith` in `to_edge_declarations`, producing phantom edges from plain-string tags like `"auth"` that parse as slugs. The fix is structural: delete the field. `Frontmatter::tags()` already exists as the typed accessor for reading tags out of open_meta (added in Session 1). Because `ResourceRelationships` derives `#[serde(default)]` on every field, removing the `tags` field is serde-compatible — existing JSON with a `"tags": [...]` key will simply be ignored during deserialization.

The `EdgeType::TaggedWith` Rust variant can be removed even though the Postgres `edge_type` enum in migration `20260411000002_knowledge_graph_edges.sql:23` still contains `'tagged_with'` — the DB will accept that value but nothing writes it. Verification gate: zero `kb_resource_edges` rows currently use `tagged_with`.

- [ ] **Step 1: Verify zero `tagged_with` rows in the dev database**

```bash
psql "postgresql://temper:temper@localhost:5437/temper_development" -c "SELECT COUNT(*) FROM kb_resource_edges WHERE edge_type = 'tagged_with';" 2>&1
```

Expected: `count = 0`. If non-zero, STOP — there's real data using the variant. Investigate before deleting.

- [ ] **Step 2: Remove `tags` field from `ResourceRelationships`**

In `crates/temper-core/src/types/graph.rs`, lines 96-97:

```rust
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
```

Delete both lines.

- [ ] **Step 3: Remove `tags.is_empty()` from `ResourceRelationships::is_empty`**

Line 113 of `graph.rs`:

```rust
            && self.tags.is_empty()
```

Delete this line. The `is_empty` impl now sums the remaining seven fields.

- [ ] **Step 4: Remove `(&self.tags, EdgeType::TaggedWith)` from `to_edge_declarations`**

Line 131 of `graph.rs`:

```rust
            (&self.tags, EdgeType::TaggedWith),
```

Delete this entry from the `field_mappings` slice. The slice now has 6 entries (RelatesTo, Extends, DependsOn, References, PrecededBy, DerivedFrom) plus parent handled separately.

- [ ] **Step 5: Remove `EdgeType::TaggedWith` variant**

Line 26 of `graph.rs`:

```rust
    TaggedWith,
```

Delete this variant from the enum.

- [ ] **Step 6: Remove `Self::TaggedWith => ...` from `Display` impl**

Line 39 of `graph.rs`:

```rust
            Self::TaggedWith => write!(f, "tagged_with"),
```

Delete this arm.

- [ ] **Step 7: Remove the test assertion at line 343**

Line 343 in `graph.rs`'s test module:

```rust
        assert_eq!(EdgeType::TaggedWith.to_string(), "tagged_with");
```

Delete this line from the `edge_type_display` test.

- [ ] **Step 8: Update `projections.rs` test fixture**

The `TASK_FIXTURE` constant at `crates/temper-core/src/frontmatter/projections.rs:94-111` contains `tags: [auth, observability]` on line 108. The `projects_to_resource_relationships` test at line 121 asserts:

```rust
        // tags still lives on the struct in session 1, so it should round-trip.
        assert_eq!(rels.tags, vec!["auth", "observability"]);
```

Delete that assertion and the preceding comment. The fixture can keep `tags: [auth, observability]` — it's still valid open-meta YAML, and Session 1's `fm.tags()` accessor still reads it. The test just won't project tags through `ResourceRelationships` anymore.

- [ ] **Step 9: Update `frontmatter_test.rs::tags_as_strings_do_not_become_parent_of_relationships`**

At `crates/temper-core/tests/frontmatter_test.rs:233-266`, the test asserts `rels.tags == vec!["auth", "observability", "not-a-resource"]` at lines 247-256. That assertion is now a compile error.

Delete the `assert_eq!(rels.tags, vec![...])` block at lines 247-256. Keep the `fm.tags()` check at lines 258-265 — that's the canonical accessor and still works.

Updated test body:

```rust
#[test]
fn tags_as_strings_do_not_become_parent_of_relationships() {
    // `tags` is metadata — it must not project into any of the
    // `ResourceRelationships` edge-producing fields.
    let content = load_fixture("tags_as_strings.md");
    let fm = Frontmatter::try_from(content.as_str()).unwrap();
    let rels = ResourceRelationships::from(&fm);
    assert!(rels.relates_to.is_empty());
    assert!(rels.depends_on.is_empty());
    assert!(rels.extends.is_empty());
    assert!(rels.references.is_empty());
    assert!(rels.preceded_by.is_empty());
    assert!(rels.derived_from.is_empty());
    assert!(rels.parent.is_none());

    // The typed accessor on Frontmatter still exposes tags as metadata.
    assert_eq!(
        fm.tags(),
        vec![
            "auth".to_string(),
            "observability".to_string(),
            "not-a-resource".to_string()
        ]
    );
}
```

- [ ] **Step 10: Run `temper-core` tests**

```bash
cargo nextest run -p temper-core 2>&1 | tail -30
```

Expected: all green. The `to_edge_declarations_extracts_all_types` test at `graph.rs:278-318` currently asserts on 5 edges from a fixture that doesn't use tags, so it should still pass.

- [ ] **Step 11: Run `temper-api` tests (edge_service)**

```bash
cargo nextest run -p temper-api --features test-db edge 2>&1 | tail -40
```

Expected: all edge service tests pass. `edge_service::extract_declarations_from_open_meta` deserializes `ResourceRelationships` from JSON — with `#[serde(default)]` on every field, removing `tags` from the struct is backward-compatible at the JSON level (existing `tags` keys in open_meta JSON are simply ignored during deserialization, which is what we want).

- [ ] **Step 12: Full workspace test run**

```bash
cargo nextest run --workspace --features test-db 2>&1 | tail -30
```

Expected: all green.

- [ ] **Step 13: Commit**

```bash
git add crates/temper-core/src/types/graph.rs crates/temper-core/src/frontmatter/projections.rs crates/temper-core/tests/frontmatter_test.rs
git commit -m "$(cat <<'EOF'
fix(graph): remove ResourceRelationships::tags phantom-edge bug

Plain-string tags like "auth" or "observability" were being parsed as
slugs by TargetRef::parse and producing TaggedWith edges to nonexistent
resources. Tags are an Obsidian-compatible metadata vector, not a
resource relationship type.

Changes:
- Delete ResourceRelationships::tags field (backward-compatible at
  the JSON level because every field uses #[serde(default)])
- Delete EdgeType::TaggedWith variant + Display arm + test
- Update projections.rs and frontmatter_test.rs to drop rels.tags
  assertions (Frontmatter::tags() accessor still provides the read path)

Verified zero kb_resource_edges rows use 'tagged_with' in the dev
database before deletion. The Postgres edge_type enum still contains
the value for forward compatibility; removing it requires a separate
migration and is not worth the churn for a pre-alpha project.
EOF
)"
```

---

### Task 13: Regenerate TypeScript bindings + verify temper-ui typecheck

**Files:**
- Regenerate: `packages/temper-ui/src/lib/types/generated/graph.ts` and any other ts-rs output files
- Verify: `packages/temper-ui` typecheck

**Context for implementer:** `ResourceRelationships` and `EdgeType` both derive `ts_rs::TS`. After Task 12 deletes the `tags` field and `TaggedWith` variant, the TypeScript output in `graph.ts` must be regenerated. The project uses `cargo make generate-ts-types` for this.

- [ ] **Step 1: Regenerate TypeScript types**

```bash
cargo make generate-ts-types 2>&1 | tail -20
```

Expected: completes without errors.

- [ ] **Step 2: Verify the generated `graph.ts` no longer contains `tagged_with` or `tags` on ResourceRelationships**

```
(Grep tool query: "tagged_with" in packages/temper-ui/src/lib/types/generated/graph.ts — expect zero hits)
```

And:

```
(Grep tool query: "tags" in packages/temper-ui/src/lib/types/generated/graph.ts — expect zero hits on ResourceRelationships; the EdgeType string union should no longer include "tagged_with")
```

- [ ] **Step 3: Run temper-ui typecheck**

```bash
cd packages/temper-ui && bun run check 2>&1 | tail -30
```

Expected: clean. If the UI references `relationships.tags` or `"tagged_with"` anywhere, the check will flag it. Grep for those patterns pre-emptively:

```
(Grep tool query: "relationships\.tags|tagged_with" in packages/temper-ui/src — expect zero hits; Session 2 context gathering confirmed this earlier)
```

- [ ] **Step 4: Run TypeScript package tests**

```bash
cargo make ts-test 2>&1 | tail -30
```

Expected: all green.

- [ ] **Step 5: Commit**

```bash
cd /Users/petetaylor/projects/tasker-systems/temper
git add packages/temper-ui/src/lib/types/generated
git commit -m "chore(ts-types): regenerate after removing tags/TaggedWith"
```

---

### Task 14: Final verification — full matrix + real-vault dry-run + flip PR back to draft-but-ready

**Files:**
- No file changes. Verification only.

**Context for implementer:** Session 1's load-bearing verification was a byte-diff of `target/debug/temper doctor` against main across the real vault. Session 2 changes `normalize_file`'s write path, so this check is doubly important — if any real vault file produces different output under the new pipeline, that's a deliberate behavior change we need to see before committing Session 2.

- [ ] **Step 1: Full `cargo make check`**

```bash
cargo make check 2>&1 | tail -30
```

Expected: clean — fmt, clippy, docs, machete, TS typecheck, biome.

- [ ] **Step 2: Full Rust workspace unit suite**

```bash
cargo nextest run --workspace 2>&1 | tail -20
```

Expected: all green.

- [ ] **Step 3: Full Rust workspace DB suite**

```bash
cargo make docker-up 2>&1 | tail -10
cargo nextest run --workspace --features test-db 2>&1 | tail -30
```

Expected: all green.

- [ ] **Step 4: Full e2e suite**

```bash
cargo nextest run -p temper-e2e --features test-db 2>&1 | tail -40
```

Expected: all green.

- [ ] **Step 5: Real-vault `temper doctor` byte-diff against main**

```bash
cd /Users/petetaylor/projects/tasker-systems/temper
cargo build --release -p temper-cli 2>&1 | tail -5
./target/release/temper doctor --vault /Users/petetaylor/projects/kb-vault > /tmp/doctor_session2.txt 2>&1
git stash
git checkout main
cargo build --release -p temper-cli 2>&1 | tail -5
./target/release/temper doctor --vault /Users/petetaylor/projects/kb-vault > /tmp/doctor_main.txt 2>&1
git checkout jct/frontmatter-consolidation
git stash pop 2>/dev/null || true
diff -q /tmp/doctor_session2.txt /tmp/doctor_main.txt && echo "BYTE-IDENTICAL" || echo "DRIFT"
```

Expected: `BYTE-IDENTICAL`. If `DRIFT`, use `diff -u /tmp/doctor_main.txt /tmp/doctor_session2.txt | head -100` to inspect. Investigate every line of drift before continuing — a drift here means some real vault file is now being rewritten when it wasn't before, and we need to understand why. Acceptable drifts: none.

(If the stash/checkout/build dance is too fiddly, alternative: build release before the session started and save that binary, then just build new and diff.)

- [ ] **Step 6: Real-vault `temper sync run --dry-run`**

```bash
./target/release/temper sync run --dry-run --vault /Users/petetaylor/projects/kb-vault 2>&1 | tail -40
```

Expected: no unexpected content changes. If the output shows files wanting to be rewritten that weren't wanting rewrite pre-Session-2, investigate.

- [ ] **Step 7: `cargo sqlx prepare --check`**

```bash
SQLX_OFFLINE=false cargo sqlx prepare --workspace --check 2>&1 | tail -10
```

Expected: no drift. Session 2 touches zero SQL, so the `.sqlx/` cache should be untouched.

- [ ] **Step 8: Push the branch (if not already pushed)**

```bash
git push origin jct/frontmatter-consolidation 2>&1 | tail -5
```

- [ ] **Step 9: Confirm PR #43 is still in draft state**

```bash
gh pr view 43 --json state,isDraft 2>&1
```

Expected: `"isDraft": true`. Session 2 commits are now on the branch; PR flips back to ready when Session 3 also lands.

---

## Self-Review

**1. Spec coverage:**

| Spec section | Covered by task(s) |
|--------------|---------------------|
| Migrate `normalize::normalize_file` as orchestrator | Task 2 |
| Migrate `sync.rs` 4 `split_frontmatter_tiers` call sites | Tasks 3 (build_meta_update_payload @ 801; push_resource_body @ 918), 7 (tests @ 2249, 3186) |
| Migrate `sync.rs` ad-hoc YAML reading (`rehash_manifest`) | Task 4 |
| Migrate remaining 10+ `compute_frontmatter_hashes_from_yaml` call sites | Tasks 3 (push), 4 (rehash+scan), 5 (pull), 6 (merge+reset), 7 (tests), 8 (e2e), 9 (schema_test) |
| Delete `hash::split_frontmatter_tiers` | Task 11 |
| Delete `hash::compute_frontmatter_hashes_from_yaml` | Task 11 |
| Delete `normalize::split_frontmatter_block` | Task 2 |
| Remove `tags` field from `ResourceRelationships` | Task 12 |
| Delete `EdgeType::TaggedWith` variant | Task 12 |
| Regenerate TypeScript bindings | Task 13 |
| Verify `temper-ui` doesn't reference `relationships.tags` | Task 13 (already verified clean during context gathering; grep enforces it) |
| Add `Frontmatter::tags()` accessor | Already landed in Session 1 — nothing to do |
| Real-vault dry-run verification | Task 14 |
| Full e2e suite green | Task 14 |
| `cargo make check` clean | Tasks 2, 4, 11, 14 |

Coverage check: every Session 2 bullet from the spec's Migration Plan section maps to at least one task step. ✓

**2. Placeholder scan:**

- No "TODO" / "TBD" / "implement later" in task steps.
- Real placeholders: Task 10 has `REPLACE_WITH_*_HASH` and `"sha256:REPLACE"` constants in the test code — those are filled in at execution time via a capture step (Task 10 Step 1 and Step 3). They are NOT plan failures; they are explicit "capture runtime value, paste here" instructions. Each capture step is concrete (`dbg!`, run test, read stdout, paste).
- No "add appropriate error handling" / "handle edge cases" / "similar to Task N".

**3. Type consistency:**

- `Frontmatter::try_from(&str)` — used consistently across all migration tasks.
- `Frontmatter::parse_file(&Path)` — used in Task 2 (`normalize_file`).
- `Frontmatter::managed_json()` / `Frontmatter::open_json()` / `Frontmatter::hashes()` — used consistently.
- `Frontmatter::value_mut()` — added in Task 1, consumed in Task 2. Signature consistent.
- `empty_hashes_fallback(parsed, doc_type)` — defined in Task 3 Step 1, used in Tasks 3/4/5/6 by the same signature.
- `DocType::as_str()` — used in Task 2 (`fm.doc_type().as_str()`).

All cross-task type references are consistent. ✓

## Execution Handoff

Plan complete and saved to `docs/superpowers/plans/2026-04-13-temper-core-frontmatter-module-session-2.md`.

**Recommended execution:** Subagent-Driven (matches Session 1's pattern — fresh general-purpose subagent per task, two-stage spec + code review between tasks, per-task commits on `jct/frontmatter-consolidation`). Session 2 is the highest-risk session in this multi-session task; the two-stage review cadence is worth the overhead.

Fresh implementer-subagent prompts MUST include:
- SG-13 (no stringly-typed match over bounded sets) — inject from `~/.claude/skills/temper/subagent-guidance.md`
- Pattern P1–P4 definitions from this plan's "Migration Pattern Library" section — inject verbatim so the subagent doesn't have to re-read
- The specific task's Files + Steps + verification gates
- Explicit instruction: commit only after Step "Commit" passes; do not batch multiple tasks in one commit

Parent-agent review cadence:
1. After each task completes, run the spec-compliance reviewer on the diff
2. Then run superpowers:code-reviewer for quality / smell / style
3. Flag any DONE_WITH_CONCERNS to the user before moving to the next task

Session 2 ends when Task 14's real-vault dry-run reports `BYTE-IDENTICAL`, all gate suites are green, and commit `refactor(hash): delete split_frontmatter_tiers...` + the tags-fix commit are pushed. Then we save a session note via `temper resource create --type session`, update the task description with next-session context for Session 3, and wait for user review before starting Session 3 planning.
