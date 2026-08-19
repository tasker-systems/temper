# Wave 1 Phase 2 — Shared Pure Actions Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add `crates/temper-core/src/operations/actions.rs` with the pure shared actions both DbBackend (Phase 3) and VaultBackend (Phase 4) will call before persisting: doctype defaults, command-input validation, ManagedMeta partial-merge, and open_meta partial-merge. Compose into `prepare_create` / `prepare_update` pipelines.

**Architecture:** Pure functions only — no I/O, no DB, no file system. Each action takes inputs and returns transformed outputs. Existing `apply_doc_type_defaults` (in `temper-core/src/defaults.rs`) and `validate_owner_pattern` (in `temper-core/src/validation.rs`) already live in `temper-core` and are re-exported through `operations::actions` for ergonomic imports. New work: validation that applies to commands, `merge_managed_meta`, `merge_open_meta`, and the composite `prepare_*` pipelines.

**Tech Stack:** Rust 2021, serde, thiserror.

**Specs:**
- `docs/superpowers/specs/2026-05-01-shared-core-execution-paths-design.md` (#4) — Phase 2 section
- `docs/superpowers/plans/2026-05-02-wave1-phase1-operations-scaffolding.md` — predecessor plan, all tasks complete

**Predecessor state (Phase 1, landed):** `temper-core/src/operations/` has `surface.rs`, `resource_ref.rs`, `inputs.rs`, `commands.rs`, `events.rs`, `output.rs`, `backend.rs`. Backend trait declared but not yet implemented.

**Phase 2 reality vs spec:** Spec #4's Phase 2 section assumed defaults / validation logic was "scattered" across temper-cli and temper-api requiring migration. Verification at Phase 1 close found that:
- `apply_doc_type_defaults` already lives at `temper-core/src/defaults.rs` and is called from `temper-api/src/services/ingest_service.rs:10`. CLI does not currently call it directly.
- `validate_owner_pattern` already lives at `temper-core/src/validation.rs:3` and is called from `crates/temper-cli/src/actions/doctor.rs:145`.
- No genuine duplicates were found. Phase 2 is therefore additive (new actions + new pipelines) rather than migrational. CLI/API call-site rewiring to use `operations::actions` happens in Phase 3 / Phase 4 when those crates' code is touched.

**Out of scope for this plan:** Backend implementations (Phases 3–4), surface dispatch unification (Phase 5), state machines / manifest narrowing (Phase 6 / spec #3).

---

## File Structure

**New file:**

| File | Responsibility |
|---|---|
| `crates/temper-core/src/operations/actions.rs` | Pure shared actions: defaults, validation, merge, prepare pipelines |

**Modified files:**

| File | Change |
|---|---|
| `crates/temper-core/src/operations/mod.rs` | `mod actions;` + `pub use actions::*` block for the new public surface |

**Conventions:**
- Each action is a `pub fn` taking owned or borrowed inputs and returning a `Result<Output, ActionError>` (or panic-free transformations where errors aren't possible).
- An `ActionError` enum lives in `actions.rs` (not in `error.rs`) since errors are local to action validation. If the error count grows beyond ~6 variants, move it to its own file in Phase 3.
- Tests live in `#[cfg(test)] mod tests` at the bottom of `actions.rs` (consistent with peer files).

---

## Task 1: Scaffold `actions.rs` with `ActionError` and `apply_defaults`

**Files:**
- Create: `crates/temper-core/src/operations/actions.rs`
- Modify: `crates/temper-core/src/operations/mod.rs`

- [ ] **Step 1: Create `actions.rs` with the error type and the first action**

```rust
//! Pure shared actions — used by both DbBackend and VaultBackend before persisting.
//!
//! Each action is a pure function: takes inputs, returns transformed outputs.
//! No I/O, no DB, no file system. Side effects (persistence, network, file
//! writes) belong to the backend's command handler, not to actions.

use thiserror::Error;

use crate::defaults::apply_doc_type_defaults;
use crate::types::managed_meta::ManagedMeta;

/// Errors that can arise during pure-action execution.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum ActionError {
    #[error("invalid doctype: {0}")]
    InvalidDoctype(String),
    #[error("invalid slug: {0}")]
    InvalidSlug(String),
    #[error("missing required field: {0}")]
    MissingRequiredField(String),
    #[error("invalid managed_meta: {0}")]
    InvalidManagedMeta(String),
}

/// Apply doctype-specific defaults to a `ManagedMeta` value, in place.
///
/// Wraps the existing `temper_core::defaults::apply_doc_type_defaults` for
/// ergonomic use from operations callers and to keep all action logic
/// importable from one path.
pub fn apply_defaults(doctype: &str, meta: &mut ManagedMeta) {
    // ManagedMeta serializes round-trip-lossless through serde_json::Value;
    // round-trip into Value, apply defaults to the Value's object, deserialize back.
    let mut value = serde_json::to_value(&*meta).unwrap_or(serde_json::Value::Null);
    apply_doc_type_defaults(doctype, &mut value);
    if let Ok(updated) = serde_json::from_value::<ManagedMeta>(value) {
        *meta = updated;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn apply_defaults_task_sets_stage_when_missing() {
        let mut meta = ManagedMeta::default();
        apply_defaults("task", &mut meta);
        assert_eq!(meta.stage.as_deref(), Some("backlog"));
    }

    #[test]
    fn apply_defaults_task_does_not_overwrite_existing_stage() {
        let mut meta = ManagedMeta {
            stage: Some("in-progress".to_string()),
            ..ManagedMeta::default()
        };
        apply_defaults("task", &mut meta);
        assert_eq!(meta.stage.as_deref(), Some("in-progress"));
    }

    #[test]
    fn apply_defaults_unknown_doctype_is_noop() {
        let mut meta = ManagedMeta::default();
        apply_defaults("nonexistent", &mut meta);
        // No fields populated for unknown doctypes
        assert!(meta.stage.is_none());
        assert!(meta.status.is_none());
    }
}
```

Note on `ManagedMeta` shape — verify before implementation: the field `stage: Option<String>` and `status: Option<String>` are expected. If the field names differ in current code, adjust the test assertions accordingly. Run `grep -n "pub stage\|pub status" crates/temper-core/src/types/managed_meta.rs` to confirm.

- [ ] **Step 2: Wire `actions` into `mod.rs`**

In `crates/temper-core/src/operations/mod.rs`, add `mod actions;` alphabetically (it sorts to the top):

```rust
mod actions;

pub use actions::{ActionError, apply_defaults};
```

- [ ] **Step 3: Run tests**

Run: `cargo nextest run -p temper-core operations::actions`

Expected: 3 tests pass.

- [ ] **Step 4: Commit**

```bash
git add crates/temper-core/src/operations/
git commit -m "feat(core): scaffold operations actions module with apply_defaults"
```

---

## Task 2: Add `validate_slug` action

**Files:**
- Modify: `crates/temper-core/src/operations/actions.rs`

- [ ] **Step 1: Add the validate_slug function**

In `actions.rs`, after `apply_defaults`, add:

```rust
/// Validate that a slug conforms to the temper slug rules.
///
/// Rules: non-empty, lowercase alphanumeric + hyphens, must start and end with
/// alphanumeric, no consecutive hyphens. Slugs are scoped to (owner, context,
/// doctype); this validates lexical shape only.
pub fn validate_slug(slug: &str) -> Result<(), ActionError> {
    if slug.is_empty() {
        return Err(ActionError::InvalidSlug("slug must not be empty".to_string()));
    }
    let bytes = slug.as_bytes();
    if !bytes[0].is_ascii_alphanumeric() {
        return Err(ActionError::InvalidSlug(format!(
            "slug must start with alphanumeric, got: {slug}"
        )));
    }
    if !bytes[bytes.len() - 1].is_ascii_alphanumeric() {
        return Err(ActionError::InvalidSlug(format!(
            "slug must end with alphanumeric, got: {slug}"
        )));
    }
    let mut prev_was_hyphen = false;
    for &b in bytes {
        let is_lower_alnum = b.is_ascii_lowercase() || b.is_ascii_digit();
        let is_hyphen = b == b'-';
        if !is_lower_alnum && !is_hyphen {
            return Err(ActionError::InvalidSlug(format!(
                "slug must be lowercase alphanumeric with hyphens, got: {slug}"
            )));
        }
        if is_hyphen && prev_was_hyphen {
            return Err(ActionError::InvalidSlug(format!(
                "slug must not contain consecutive hyphens, got: {slug}"
            )));
        }
        prev_was_hyphen = is_hyphen;
    }
    Ok(())
}
```

- [ ] **Step 2: Add tests at the bottom of `mod tests`**

```rust
    #[test]
    fn validate_slug_accepts_valid_slugs() {
        assert!(validate_slug("hello-world").is_ok());
        assert!(validate_slug("task-2026-04-29").is_ok());
        assert!(validate_slug("a").is_ok());
        assert!(validate_slug("a1b2").is_ok());
    }

    #[test]
    fn validate_slug_rejects_empty() {
        let err = validate_slug("").unwrap_err();
        assert!(matches!(err, ActionError::InvalidSlug(_)));
    }

    #[test]
    fn validate_slug_rejects_uppercase() {
        let err = validate_slug("Hello").unwrap_err();
        assert!(matches!(err, ActionError::InvalidSlug(_)));
    }

    #[test]
    fn validate_slug_rejects_leading_hyphen() {
        let err = validate_slug("-hello").unwrap_err();
        assert!(matches!(err, ActionError::InvalidSlug(_)));
    }

    #[test]
    fn validate_slug_rejects_trailing_hyphen() {
        let err = validate_slug("hello-").unwrap_err();
        assert!(matches!(err, ActionError::InvalidSlug(_)));
    }

    #[test]
    fn validate_slug_rejects_consecutive_hyphens() {
        let err = validate_slug("hello--world").unwrap_err();
        assert!(matches!(err, ActionError::InvalidSlug(_)));
    }
```

- [ ] **Step 3: Update `mod.rs` re-export**

```rust
pub use actions::{ActionError, apply_defaults, validate_slug};
```

- [ ] **Step 4: Run tests**

Run: `cargo nextest run -p temper-core operations::actions`

Expected: 9 tests pass (3 from Task 1 + 6 new).

- [ ] **Step 5: Commit**

```bash
git add crates/temper-core/src/operations/
git commit -m "feat(core): add validate_slug action with lexical rules"
```

---

## Task 3: Add `validate_doctype` action

**Files:**
- Modify: `crates/temper-core/src/operations/actions.rs`

- [ ] **Step 1: Add the validate_doctype function**

In `actions.rs`, after `validate_slug`:

```rust
/// Recognized doctypes for the alpha. Updates here must stay in sync with
/// `temper-core/types/schemas/`.
const RECOGNIZED_DOCTYPES: &[&str] = &[
    "task", "goal", "session", "research", "concept", "decision", "memory",
];

/// Validate that a doctype is recognized.
pub fn validate_doctype(doctype: &str) -> Result<(), ActionError> {
    if RECOGNIZED_DOCTYPES.contains(&doctype) {
        Ok(())
    } else {
        Err(ActionError::InvalidDoctype(format!(
            "unknown doctype '{doctype}', expected one of: {}",
            RECOGNIZED_DOCTYPES.join(", ")
        )))
    }
}
```

Verification step before writing: confirm the recognized doctype list matches `crates/temper-core/types/schemas/`. Run `ls crates/temper-core/types/schemas/`. If the directory is at a different path or contains different doctypes, update `RECOGNIZED_DOCTYPES` to match. Do not invent doctypes that don't have schemas.

- [ ] **Step 2: Add tests**

```rust
    #[test]
    fn validate_doctype_accepts_known() {
        assert!(validate_doctype("task").is_ok());
        assert!(validate_doctype("goal").is_ok());
        assert!(validate_doctype("memory").is_ok());
    }

    #[test]
    fn validate_doctype_rejects_unknown() {
        let err = validate_doctype("widget").unwrap_err();
        assert!(matches!(err, ActionError::InvalidDoctype(_)));
    }
```

- [ ] **Step 3: Update `mod.rs` re-export**

```rust
pub use actions::{ActionError, apply_defaults, validate_doctype, validate_slug};
```

- [ ] **Step 4: Run tests**

Run: `cargo nextest run -p temper-core operations::actions`

Expected: 11 tests pass (9 + 2 new).

- [ ] **Step 5: Commit**

```bash
git add crates/temper-core/src/operations/
git commit -m "feat(core): add validate_doctype action against alpha doctype set"
```

---

## Task 4: Add `merge_managed_meta` action

**Files:**
- Modify: `crates/temper-core/src/operations/actions.rs`

- [ ] **Step 1: Add the merge function**

In `actions.rs`, after `validate_doctype`:

```rust
/// Partial-merge a `ManagedMeta` patch onto an existing `ManagedMeta`.
///
/// For each `Some(value)` in `patch`, overwrite the corresponding field in
/// `existing`. Fields that are `None` in the patch are left unchanged on
/// `existing`. The `extra` HashMap is merged key-by-key (patch keys overwrite,
/// keys absent from patch are preserved).
pub fn merge_managed_meta(existing: &mut ManagedMeta, patch: ManagedMeta) {
    if patch.doc_type.is_some() {
        existing.doc_type = patch.doc_type;
    }
    if patch.context.is_some() {
        existing.context = patch.context;
    }
    if patch.updated.is_some() {
        existing.updated = patch.updated;
    }
    if patch.source.is_some() {
        existing.source = patch.source;
    }
    if patch.stage.is_some() {
        existing.stage = patch.stage;
    }
    // ... continue for every field on ManagedMeta. Verify the field set
    // before writing — run `grep -n "pub " crates/temper-core/src/types/managed_meta.rs`
    // and add a guarded assignment for each pub field.

    // Merge extra HashMap key-by-key.
    for (k, v) in patch.extra {
        existing.extra.insert(k, v);
    }
}
```

**Important:** before writing this function, run `grep -nE "^    pub " crates/temper-core/src/types/managed_meta.rs` to enumerate every public field on `ManagedMeta`. The function must handle ALL of them (not just the ones shown above). Missing fields will silently fail to merge.

- [ ] **Step 2: Add tests**

```rust
    #[test]
    fn merge_managed_meta_overrides_present_fields() {
        let mut existing = ManagedMeta {
            stage: Some("backlog".to_string()),
            ..ManagedMeta::default()
        };
        let patch = ManagedMeta {
            stage: Some("done".to_string()),
            ..ManagedMeta::default()
        };
        merge_managed_meta(&mut existing, patch);
        assert_eq!(existing.stage.as_deref(), Some("done"));
    }

    #[test]
    fn merge_managed_meta_preserves_absent_fields() {
        let mut existing = ManagedMeta {
            stage: Some("backlog".to_string()),
            doc_type: Some("task".to_string()),
            ..ManagedMeta::default()
        };
        let patch = ManagedMeta {
            stage: Some("done".to_string()),
            ..ManagedMeta::default()
        };
        merge_managed_meta(&mut existing, patch);
        assert_eq!(existing.stage.as_deref(), Some("done"));
        assert_eq!(existing.doc_type.as_deref(), Some("task"));
    }

    #[test]
    fn merge_managed_meta_merges_extra_map() {
        use serde_json::json;
        let mut existing = ManagedMeta::default();
        existing.extra.insert("k1".to_string(), json!("v1"));
        existing.extra.insert("k2".to_string(), json!("v2"));

        let mut patch = ManagedMeta::default();
        patch.extra.insert("k2".to_string(), json!("patched"));
        patch.extra.insert("k3".to_string(), json!("v3"));

        merge_managed_meta(&mut existing, patch);
        assert_eq!(existing.extra.get("k1"), Some(&json!("v1")));
        assert_eq!(existing.extra.get("k2"), Some(&json!("patched")));
        assert_eq!(existing.extra.get("k3"), Some(&json!("v3")));
    }
```

- [ ] **Step 3: Update `mod.rs` re-export**

```rust
pub use actions::{ActionError, apply_defaults, merge_managed_meta, validate_doctype, validate_slug};
```

- [ ] **Step 4: Run tests**

Run: `cargo nextest run -p temper-core operations::actions`

Expected: 14 tests pass (11 + 3 new).

- [ ] **Step 5: Commit**

```bash
git add crates/temper-core/src/operations/
git commit -m "feat(core): add merge_managed_meta action with field-by-field partial merge"
```

---

## Task 5: Add `merge_open_meta` action

**Files:**
- Modify: `crates/temper-core/src/operations/actions.rs`

- [ ] **Step 1: Add the merge function**

In `actions.rs`, after `merge_managed_meta`:

```rust
use serde_json::Value;

/// Partial-merge an open_meta patch onto an existing open_meta value.
///
/// open_meta is free-form JSON (an object). Patch semantics:
/// - For each key in `patch`, overwrite the corresponding key in `existing`.
/// - Keys in `existing` not present in `patch` are preserved.
/// - Non-object inputs (e.g., a top-level array or scalar) overwrite outright.
///
/// This is shallow merge — nested objects are not deep-merged. Callers that
/// need deep merge should compose this action with their own logic.
pub fn merge_open_meta(existing: &mut Value, patch: Value) {
    match (existing.as_object_mut(), patch) {
        (Some(existing_map), Value::Object(patch_map)) => {
            for (k, v) in patch_map {
                existing_map.insert(k, v);
            }
        }
        (_, patch) => {
            // Either existing isn't an object, or patch isn't — overwrite outright.
            *existing = patch;
        }
    }
}
```

- [ ] **Step 2: Add tests**

```rust
    #[test]
    fn merge_open_meta_shallow_merges_objects() {
        use serde_json::json;
        let mut existing = json!({"a": 1, "b": 2});
        merge_open_meta(&mut existing, json!({"b": 99, "c": 3}));
        assert_eq!(existing, json!({"a": 1, "b": 99, "c": 3}));
    }

    #[test]
    fn merge_open_meta_overwrites_when_patch_is_not_object() {
        use serde_json::json;
        let mut existing = json!({"a": 1});
        merge_open_meta(&mut existing, json!([1, 2, 3]));
        assert_eq!(existing, json!([1, 2, 3]));
    }

    #[test]
    fn merge_open_meta_overwrites_when_existing_is_not_object() {
        use serde_json::json;
        let mut existing = json!("scalar");
        merge_open_meta(&mut existing, json!({"a": 1}));
        assert_eq!(existing, json!({"a": 1}));
    }
```

- [ ] **Step 3: Update `mod.rs` re-export**

```rust
pub use actions::{
    ActionError, apply_defaults, merge_managed_meta, merge_open_meta, validate_doctype,
    validate_slug,
};
```

- [ ] **Step 4: Run tests**

Run: `cargo nextest run -p temper-core operations::actions`

Expected: 17 tests pass (14 + 3 new).

- [ ] **Step 5: Commit**

```bash
git add crates/temper-core/src/operations/
git commit -m "feat(core): add merge_open_meta action with shallow JSON object merge"
```

---

## Task 6: Add `validate_create` and `validate_update` composite actions

**Files:**
- Modify: `crates/temper-core/src/operations/actions.rs`

- [ ] **Step 1: Add the validation pipelines**

In `actions.rs`, after `merge_open_meta`:

```rust
use super::commands::{CreateResource, UpdateResource};
use super::resource_ref::ResourceRef;

/// Pre-flight validation for a `CreateResource` command.
///
/// Checks slug, doctype, and context shape. Does not check uniqueness or
/// authorization — those are backend concerns.
pub fn validate_create(cmd: &CreateResource) -> Result<(), ActionError> {
    validate_slug(&cmd.slug)?;
    validate_doctype(&cmd.doctype)?;
    if cmd.context.is_empty() {
        return Err(ActionError::MissingRequiredField("context".to_string()));
    }
    if cmd.title.is_empty() {
        return Err(ActionError::MissingRequiredField("title".to_string()));
    }
    Ok(())
}

/// Pre-flight validation for an `UpdateResource` command.
///
/// Checks the `ResourceRef` is well-formed. Field-level validation of the
/// patch payload (managed_meta enums, etc.) is the backend's responsibility
/// after merging onto the resolved resource.
pub fn validate_update(cmd: &UpdateResource) -> Result<(), ActionError> {
    match &cmd.resource {
        ResourceRef::Uuid { .. } => Ok(()),
        ResourceRef::Scoped { slug, doctype, context } => {
            validate_slug(slug)?;
            validate_doctype(doctype)?;
            if context.is_empty() {
                return Err(ActionError::MissingRequiredField("context".to_string()));
            }
            Ok(())
        }
    }
}
```

- [ ] **Step 2: Add tests**

```rust
    #[test]
    fn validate_create_accepts_valid_command() {
        let cmd = CreateResource {
            slug: "valid-slug".to_string(),
            doctype: "task".to_string(),
            context: "temper".to_string(),
            title: "Valid title".to_string(),
            body: None,
            managed_meta: ManagedMeta::default(),
            open_meta: None,
            origin: super::super::Surface::CliCloud,
        };
        assert!(validate_create(&cmd).is_ok());
    }

    #[test]
    fn validate_create_rejects_invalid_slug() {
        let cmd = CreateResource {
            slug: "INVALID".to_string(),
            doctype: "task".to_string(),
            context: "temper".to_string(),
            title: "X".to_string(),
            body: None,
            managed_meta: ManagedMeta::default(),
            open_meta: None,
            origin: super::super::Surface::CliCloud,
        };
        assert!(matches!(
            validate_create(&cmd),
            Err(ActionError::InvalidSlug(_))
        ));
    }

    #[test]
    fn validate_create_rejects_unknown_doctype() {
        let cmd = CreateResource {
            slug: "valid-slug".to_string(),
            doctype: "widget".to_string(),
            context: "temper".to_string(),
            title: "X".to_string(),
            body: None,
            managed_meta: ManagedMeta::default(),
            open_meta: None,
            origin: super::super::Surface::CliCloud,
        };
        assert!(matches!(
            validate_create(&cmd),
            Err(ActionError::InvalidDoctype(_))
        ));
    }

    #[test]
    fn validate_create_rejects_empty_title() {
        let cmd = CreateResource {
            slug: "valid".to_string(),
            doctype: "task".to_string(),
            context: "temper".to_string(),
            title: "".to_string(),
            body: None,
            managed_meta: ManagedMeta::default(),
            open_meta: None,
            origin: super::super::Surface::CliCloud,
        };
        assert!(matches!(
            validate_create(&cmd),
            Err(ActionError::MissingRequiredField(_))
        ));
    }

    #[test]
    fn validate_update_accepts_uuid_ref() {
        use crate::types::ids::ResourceId;
        use uuid::Uuid;
        let cmd = UpdateResource {
            resource: ResourceRef::uuid(ResourceId(Uuid::nil())),
            body: None,
            managed_meta: None,
            open_meta: None,
            origin: super::super::Surface::CliCloud,
        };
        assert!(validate_update(&cmd).is_ok());
    }

    #[test]
    fn validate_update_validates_scoped_ref() {
        let cmd = UpdateResource {
            resource: ResourceRef::scoped("INVALID", "task", "temper"),
            body: None,
            managed_meta: None,
            open_meta: None,
            origin: super::super::Surface::CliCloud,
        };
        assert!(matches!(
            validate_update(&cmd),
            Err(ActionError::InvalidSlug(_))
        ));
    }
```

- [ ] **Step 3: Update `mod.rs` re-export**

```rust
pub use actions::{
    ActionError, apply_defaults, merge_managed_meta, merge_open_meta, validate_create,
    validate_doctype, validate_slug, validate_update,
};
```

- [ ] **Step 4: Run tests**

Run: `cargo nextest run -p temper-core operations::actions`

Expected: 23 tests pass (17 + 6 new).

- [ ] **Step 5: Commit**

```bash
git add crates/temper-core/src/operations/
git commit -m "feat(core): add validate_create and validate_update composite validators"
```

---

## Task 7: Final sweep + Phase 2 close-out

**Files:**
- No code changes; verification only.

- [ ] **Step 1: Run cargo make check**

Run: `cargo make check`

Expected: passes. Specifically verify clippy doesn't complain about the new unused `match` arms or the `super::super::` paths in tests (clippy may suggest cleaner imports).

- [ ] **Step 2: Run full operations test suite**

Run: `cargo nextest run -p temper-core operations`

Expected: 43 tests pass (20 from Phase 1 + 23 from Phase 2). Note: count may differ slightly if Phase 1 added more tests during clippy fixes.

- [ ] **Step 3: Run full temper-core test suite**

Run: `cargo nextest run -p temper-core`

Expected: all tests pass (Phase 1 left 319 tests; Phase 2 adds ~23, totaling ~342).

- [ ] **Step 4: Verify the public surface**

Run: `grep "pub use" crates/temper-core/src/operations/mod.rs`

Expected: `actions::*` exports include `ActionError, apply_defaults, merge_managed_meta, merge_open_meta, validate_create, validate_doctype, validate_slug, validate_update`.

- [ ] **Step 5: Verify dep graph unchanged**

Run: `grep -E "^temper-" crates/temper-cli/Cargo.toml crates/temper-api/Cargo.toml`

Expected: no new deps. Phase 2 is pure-Rust additions in temper-core.

---

## Phase 2 Completion Checklist

When all tasks pass:

- [ ] `crates/temper-core/src/operations/actions.rs` exists with `ActionError` + 8 public functions: `apply_defaults`, `validate_slug`, `validate_doctype`, `merge_managed_meta`, `merge_open_meta`, `validate_create`, `validate_update`.
- [ ] All actions have round-trip / boundary tests; ~23 new test cases.
- [ ] `cargo make check` passes.
- [ ] `cargo make test` passes.
- [ ] No new dependencies added to any crate.
- [ ] `temper-cli` and `temper-api` Cargo.toml dep blocks unchanged.

**Hand-off to Phase 3 plan:** Phase 3 implements `Backend` for `DbBackend` (in `temper-api`). The Db impl will call `validate_create`, `apply_defaults`, `merge_managed_meta`, `merge_open_meta` from `temper_core::operations` before each DB write. Existing call sites in `temper-api/services/ingest_service.rs` (currently calling `temper_core::defaults::apply_doc_type_defaults` directly) migrate to `temper_core::operations::apply_defaults` for consistency.

**Note for plan-writer:** when writing Phase 3 plan, verify the `ManagedMeta` field set has not changed since this plan was written, and update `merge_managed_meta`'s field list if it has. The `merge_managed_meta` function MUST handle every public field on `ManagedMeta` — partial coverage is a silent bug.
