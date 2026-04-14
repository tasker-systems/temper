# temper-core::frontmatter Module — Session 1 (Foundation) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build a new `temper-core::frontmatter` module end-to-end with full unit + integration test coverage, consumed by nothing in production code yet. Pure addition. Zero behavior change to any existing code path.

**Architecture:** Introduce an aggregate `Frontmatter` type in `crates/temper-core/src/frontmatter/` that owns parsing, alias normalization, tier splitting, canonical display ordering, projection to typed structs, mutation, serialization to disk, and hash computation (delegating to existing `hash::compute_managed_hash` / `compute_open_hash`). Organized across eight small focused files.

**Tech Stack:** Rust 2021, `serde_yaml` 0.9, `serde_json` 1, `jsonschema` 0.45 (already in `temper-core`), `cargo-nextest` for tests, inline `#[cfg(test)] mod tests` per file plus a dedicated integration test file with synthetic fixtures and golden outputs.

**Reference spec:** `docs/superpowers/specs/2026-04-13-temper-core-frontmatter-consolidation-design.md`. When this plan is silent on a design question, the spec is authoritative.

**Scope boundaries for Session 1:**
- **In scope:** new module, `DocType` enum, all files in the Module Layout, full unit + integration tests, golden outputs, manual `temper doctor` smoke check, one or more commits on `jct/frontmatter-consolidation`, PR (additive only).
- **Out of scope:** migrating any existing call site, deleting `hash::split_frontmatter_tiers`, touching `ResourceRelationships::tags`, deleting `EdgeType::TaggedWith`, any changes to `sync.rs` / `normalize.rs` production paths, `temper doctor --fix aliases` command. Those are sessions 2 and 3.
- **Key invariant to preserve:** for every existing vault file, `Frontmatter::hashes()` must produce byte-identical `(managed_hash, open_hash)` to the existing `hash::compute_frontmatter_hashes_from_yaml` path. A regression test anchors this per doctype.

---

## File Structure

### New files (all created in Session 1)

- **`crates/temper-core/src/frontmatter/mod.rs`** — module root; re-exports public API; high-level docs; declares submodules.
- **`crates/temper-core/src/frontmatter/document.rs`** — `DocType` enum + `Frontmatter` aggregate type + `TryFrom<&str>` + `parse_file` + `serialize` + `write_to` + `managed_json` / `open_json` / `hashes` + mutation methods (`set_relationships`, `set_managed_field`, `set_open_field`, `remove_field`) + `validate` + `tags` accessor + inline unit tests.
- **`crates/temper-core/src/frontmatter/parse.rs`** — text block split (`---` fences), YAML parse into `serde_yaml::Value`, `normalize_aliases` that rewrites hyphen-form known-open keys to canonical underscore form, inline unit tests.
- **`crates/temper-core/src/frontmatter/tiers.rs`** — `split_managed_open` function that explicitly routes fields using the known-open registry (not `$ref`-followed schema introspection), inline unit tests.
- **`crates/temper-core/src/frontmatter/canonical.rs`** — 5-tier display ordering algorithm for `serialize()`, deterministic under randomized input, inline unit tests.
- **`crates/temper-core/src/frontmatter/registry.rs`** — `KNOWN_OPEN_FIELDS` slice + `KnownOpenField` struct + `OpenFieldType` + `FieldCategory` + `lookup_by_alias_or_canonical` helper + inline unit tests.
- **`crates/temper-core/src/frontmatter/fields.rs`** — consolidated constants (`IDENTITY_FIELDS`, `TIER1_SYSTEM_FIELDS`, `SYSTEM_MANAGED_FIELDS`); **re-exported from old locations for Session 1 only** to avoid breaking imports (Session 3 moves these properly and deletes the re-exports). Inline unit tests asserting re-exports match.
- **`crates/temper-core/src/frontmatter/projections.rs`** — `From<&Frontmatter> for ResourceRelationships`, `TryFrom<&Frontmatter> for ManagedMeta`, `TryFrom<&Frontmatter> for ResourceFrontmatter` + inline unit tests.

### New test files

- **`crates/temper-core/tests/frontmatter_test.rs`** — integration test driver with shared helpers.
- **`crates/temper-core/tests/fixtures/frontmatter/*.md`** — one synthetic fixture per doctype (minimal + full + aliases variants), plus error-case fixtures.
- **`crates/temper-core/tests/fixtures/frontmatter/golden/*.canonical.md`** — expected canonical output for each round-trippable fixture.

### Files modified (minimal, in Session 1)

- **`crates/temper-core/src/lib.rs`** — add `pub mod frontmatter;` declaration.

No other production files are modified in Session 1. Sessions 2 and 3 will do the migrations.

---

## Commit strategy

Per-task commits for safety and rollback granularity. All commits land on `jct/frontmatter-consolidation`. At PR time, they may be squashed into one `feat: new temper-core::frontmatter module (not yet consumed)` commit or left as a clean history — reviewer's choice.

---

## Task 1: Module skeleton + `DocType` enum

**Files:**
- Create: `crates/temper-core/src/frontmatter/mod.rs`
- Create: `crates/temper-core/src/frontmatter/document.rs`
- Modify: `crates/temper-core/src/lib.rs`

**Why first:** every other file depends on `DocType` and on the module declaration existing.

- [ ] **Step 1: Verify `DocType` enum is not already present**

Run:
```bash
rg -n 'pub enum DocType|enum DocType' crates/temper-core/src/
```
Expected: no output.

- [ ] **Step 2: Add module declaration to `lib.rs`**

In `crates/temper-core/src/lib.rs`, add after `pub mod defaults;`:

```rust
pub mod frontmatter;
```

Alphabetical position: between `defaults` and `hash`.

- [ ] **Step 3: Create `frontmatter/mod.rs` skeleton**

Write `crates/temper-core/src/frontmatter/mod.rs`:

```rust
//! Authoritative frontmatter handling for temper vault files.
//!
//! This module is the single source of truth for parsing, validating,
//! mutating, tier-splitting, hashing, and writing YAML frontmatter in
//! vault markdown files. The central type is [`Frontmatter`], an
//! aggregate that owns the canonicalized YAML and exposes typed
//! projections via standard trait impls.
//!
//! Hash computation delegates unchanged to `crate::hash::compute_managed_hash`
//! and `crate::hash::compute_open_hash` from PR #40 — `Frontmatter::hashes()`
//! never introduces a new canonicalization algorithm for hashing. The
//! display-ordering algorithm in [`canonical`] is strictly for on-disk
//! writes and has zero effect on hash output.

pub mod canonical;
pub mod document;
pub mod fields;
pub mod parse;
pub mod projections;
pub mod registry;
pub mod tiers;

pub use document::{DocType, Frontmatter};
pub use registry::{
    FieldCategory, KnownOpenField, OpenFieldType, KNOWN_OPEN_FIELDS,
};
```

- [ ] **Step 4: Write failing test for `DocType::from_str` and `as_str`**

Create `crates/temper-core/src/frontmatter/document.rs` with the test first (no implementation yet):

```rust
//! `Frontmatter` aggregate type and `DocType` enum.

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn doc_type_round_trip_all_six() {
        for name in ["task", "goal", "session", "research", "decision", "concept"] {
            let dt = DocType::from_str(name).expect("valid doctype");
            assert_eq!(dt.as_str(), name);
        }
    }

    #[test]
    fn doc_type_rejects_unknown() {
        assert!(DocType::from_str("bogus").is_err());
        assert!(DocType::from_str("").is_err());
        assert!(DocType::from_str("Task").is_err()); // case-sensitive
    }
}
```

Then add the empty stub just enough for the file to compile (the test itself won't compile until the impl lands, which is the point):

```rust
// Stub only — Step 5 implements.
```

- [ ] **Step 5: Run test to verify it fails**

Run:
```bash
cargo nextest run -p temper-core frontmatter::document::tests::doc_type_round_trip_all_six 2>&1 | tail -20
```
Expected: compilation error "cannot find type `DocType`" or similar.

- [ ] **Step 6: Implement `DocType` enum**

Replace the stub in `document.rs` with:

```rust
//! `Frontmatter` aggregate type and `DocType` enum.

use crate::error::{Result, TemperError};

/// Typed vault doctype. All valid values are enumerated exhaustively —
/// unknown doctypes fail at parse, not at validation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DocType {
    Task,
    Goal,
    Session,
    Research,
    Decision,
    Concept,
}

impl DocType {
    /// Canonical string form as used in YAML frontmatter and vault paths.
    pub fn as_str(&self) -> &'static str {
        match self {
            DocType::Task => "task",
            DocType::Goal => "goal",
            DocType::Session => "session",
            DocType::Research => "research",
            DocType::Decision => "decision",
            DocType::Concept => "concept",
        }
    }

    /// Parse from canonical string form. Case-sensitive.
    pub fn from_str(s: &str) -> Result<Self> {
        match s {
            "task" => Ok(DocType::Task),
            "goal" => Ok(DocType::Goal),
            "session" => Ok(DocType::Session),
            "research" => Ok(DocType::Research),
            "decision" => Ok(DocType::Decision),
            "concept" => Ok(DocType::Concept),
            other => Err(TemperError::Config(format!(
                "unknown doctype '{other}'; expected one of: task, goal, session, research, decision, concept"
            ))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn doc_type_round_trip_all_six() {
        for name in ["task", "goal", "session", "research", "decision", "concept"] {
            let dt = DocType::from_str(name).expect("valid doctype");
            assert_eq!(dt.as_str(), name);
        }
    }

    #[test]
    fn doc_type_rejects_unknown() {
        assert!(DocType::from_str("bogus").is_err());
        assert!(DocType::from_str("").is_err());
        assert!(DocType::from_str("Task").is_err()); // case-sensitive
    }
}
```

- [ ] **Step 7: Run tests to verify they pass**

Run:
```bash
cargo nextest run -p temper-core frontmatter::document
```
Expected: both tests pass.

- [ ] **Step 8: Verify the rest of the module compiles (empty submodule files ok for now)**

Create empty stub files that satisfy the `mod.rs` declarations. Each file contains only a module-level doc comment to keep `cargo check` green until subsequent tasks fill them in:

```bash
# Create each stub with just a doc comment
```

Write `crates/temper-core/src/frontmatter/canonical.rs`:
```rust
//! Canonical 5-tier display ordering for `Frontmatter::serialize()`.
```

Write `crates/temper-core/src/frontmatter/fields.rs`:
```rust
//! Consolidated frontmatter field constants. Session 1 re-exports from
//! their existing locations; Session 3 moves them here properly.
```

Write `crates/temper-core/src/frontmatter/parse.rs`:
```rust
//! Text block splitting, YAML parsing, and alias normalization at the
//! parse boundary.
```

Write `crates/temper-core/src/frontmatter/projections.rs`:
```rust
//! Trait impls projecting `Frontmatter` to the typed structs in
//! `crate::types`: `ResourceRelationships`, `ManagedMeta`, `ResourceFrontmatter`.
```

Write `crates/temper-core/src/frontmatter/registry.rs`:
```rust
//! `KNOWN_OPEN_FIELDS` registry + alias lookups.
```

Write `crates/temper-core/src/frontmatter/tiers.rs`:
```rust
//! Managed / open tier splitting. Routes explicitly via the known-open
//! registry rather than relying on `$ref` not being followed.
```

- [ ] **Step 9: Verify the whole crate compiles**

Run:
```bash
cargo check -p temper-core
```
Expected: no errors. Warnings about unused modules are acceptable at this point.

- [ ] **Step 10: Commit**

```bash
git add crates/temper-core/src/frontmatter crates/temper-core/src/lib.rs
git commit -m "feat(frontmatter): module skeleton + DocType enum"
```

---

## Task 2: `fields.rs` — consolidated constants (Session-1 re-exports)

**Files:**
- Modify: `crates/temper-core/src/frontmatter/fields.rs`

**Why:** downstream files (`tiers`, `canonical`, `document`) need `IDENTITY_FIELDS`, `TIER1_SYSTEM_FIELDS`, `SYSTEM_MANAGED_FIELDS` as a single import site. Re-exporting from existing locations is cheap and keeps Session 1 strictly additive.

- [ ] **Step 1: Write failing test for re-exports**

Replace the stub content of `crates/temper-core/src/frontmatter/fields.rs` with:

```rust
//! Consolidated frontmatter field constants. Session 1 re-exports from
//! their existing locations; Session 3 moves them here properly.
//!
//! Downstream files in `crate::frontmatter` should import from this
//! module exclusively, so Session 3's move is a purely local edit.

pub use crate::hash::{IDENTITY_FIELDS, TIER1_SYSTEM_FIELDS};
pub use crate::schema::SYSTEM_MANAGED_FIELDS;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_fields_match_hash_module() {
        assert_eq!(IDENTITY_FIELDS, crate::hash::IDENTITY_FIELDS);
    }

    #[test]
    fn tier1_system_fields_match_hash_module() {
        assert_eq!(TIER1_SYSTEM_FIELDS, crate::hash::TIER1_SYSTEM_FIELDS);
    }

    #[test]
    fn system_managed_fields_match_schema_module() {
        assert_eq!(SYSTEM_MANAGED_FIELDS, crate::schema::SYSTEM_MANAGED_FIELDS);
    }

    #[test]
    fn identity_fields_contains_expected_keys() {
        assert!(IDENTITY_FIELDS.contains(&"temper-id"));
        assert!(IDENTITY_FIELDS.contains(&"temper-provisional-id"));
    }

    #[test]
    fn tier1_fields_contains_expected_keys() {
        for key in ["temper-context", "temper-type", "temper-created", "temper-updated", "temper-owner", "temper-source"] {
            assert!(TIER1_SYSTEM_FIELDS.contains(&key), "missing key {key}");
        }
    }
}
```

- [ ] **Step 2: Run tests to verify they pass immediately**

Because `fields.rs` is a thin re-export, the tests should pass without further implementation.

Run:
```bash
cargo nextest run -p temper-core frontmatter::fields
```
Expected: 5 tests pass.

- [ ] **Step 3: Commit**

```bash
git add crates/temper-core/src/frontmatter/fields.rs
git commit -m "feat(frontmatter): consolidated field-constant re-exports"
```

---

## Task 3: `registry.rs` — `KNOWN_OPEN_FIELDS`

**Files:**
- Modify: `crates/temper-core/src/frontmatter/registry.rs`

**Why:** `parse.rs` needs the alias map, `tiers.rs` needs the known-open set, `canonical.rs` needs the order. All three depend on `registry.rs`, so it must land before them.

- [ ] **Step 1: Write failing tests**

Replace `registry.rs` stub with tests first:

```rust
//! `KNOWN_OPEN_FIELDS` registry + alias lookups.

// Implementation below.

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_contains_ten_entries() {
        assert_eq!(KNOWN_OPEN_FIELDS.len(), 10);
    }

    #[test]
    fn every_canonical_is_unique() {
        let mut seen = std::collections::HashSet::new();
        for f in KNOWN_OPEN_FIELDS {
            assert!(seen.insert(f.canonical), "duplicate canonical: {}", f.canonical);
        }
    }

    #[test]
    fn lookup_by_canonical_resolves_each_entry() {
        for f in KNOWN_OPEN_FIELDS {
            let found = lookup(f.canonical).expect("canonical hits");
            assert_eq!(found.canonical, f.canonical);
        }
    }

    #[test]
    fn lookup_by_hyphen_alias_resolves_to_canonical() {
        let cases = [
            ("relates-to", "relates_to"),
            ("depends-on", "depends_on"),
            ("preceded-by", "preceded_by"),
            ("derived-from", "derived_from"),
        ];
        for (alias, expected) in cases {
            let found = lookup(alias).expect("alias hits");
            assert_eq!(found.canonical, expected);
        }
    }

    #[test]
    fn lookup_unknown_returns_none() {
        assert!(lookup("not_a_field").is_none());
        assert!(lookup("").is_none());
    }

    #[test]
    fn relationships_use_registry_order() {
        let rels: Vec<&'static str> = KNOWN_OPEN_FIELDS
            .iter()
            .filter(|f| matches!(f.category, FieldCategory::Relationship))
            .map(|f| f.canonical)
            .collect();
        assert_eq!(
            rels,
            vec![
                "relates_to", "depends_on", "extends", "references",
                "preceded_by", "derived_from", "parent",
            ]
        );
    }

    #[test]
    fn metadata_uses_registry_order() {
        let meta: Vec<&'static str> = KNOWN_OPEN_FIELDS
            .iter()
            .filter(|f| matches!(f.category, FieldCategory::Metadata))
            .map(|f| f.canonical)
            .collect();
        assert_eq!(meta, vec!["tags", "aliases", "date"]);
    }

    #[test]
    fn tags_is_metadata_not_relationship() {
        let tags = lookup("tags").expect("tags exists");
        assert!(matches!(tags.category, FieldCategory::Metadata));
        assert!(matches!(tags.field_type, OpenFieldType::Tags));
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run:
```bash
cargo nextest run -p temper-core frontmatter::registry 2>&1 | tail -30
```
Expected: compilation errors for `KNOWN_OPEN_FIELDS`, `KnownOpenField`, `lookup`, `FieldCategory`, `OpenFieldType`.

- [ ] **Step 3: Implement registry types and constant**

Replace `registry.rs` with the full implementation:

```rust
//! `KNOWN_OPEN_FIELDS` registry + alias lookups.
//!
//! The registry is the single source of truth for which open-meta
//! field names Temper recognizes, what their canonical form is, which
//! hyphen-form aliases map to them, what value type they hold, and
//! whether they contribute edges (relationships) or are Obsidian
//! metadata (tags/aliases/date).

/// Value-type discriminator for a known open field.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpenFieldType {
    /// A single string scalar (e.g. `parent`, `date`).
    String,
    /// A list of strings (e.g. `relates_to`, `depends_on`).
    StringList,
    /// A list of strings with Obsidian tag semantics — NOT resource refs.
    Tags,
}

/// Category driving edge extraction policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FieldCategory {
    /// Drives edge extraction in `edge_service`.
    Relationship,
    /// Obsidian-compatible universal, non-relational.
    Metadata,
}

/// One entry in the known-open-field registry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KnownOpenField {
    /// Canonical key as stored in normalized `Frontmatter::value`.
    pub canonical: &'static str,
    /// Alternate keys accepted at parse time, all normalized to `canonical`.
    pub aliases: &'static [&'static str],
    /// Shape of the value.
    pub field_type: OpenFieldType,
    /// Whether the field drives edges or is metadata.
    pub category: FieldCategory,
}

/// The authoritative list of open fields Temper knows about. Order is
/// load-bearing: `canonical::serialize` uses registry order to group
/// relationships before metadata in emitted YAML.
pub const KNOWN_OPEN_FIELDS: &[KnownOpenField] = &[
    // Relationships — drive edges in `edge_service`.
    KnownOpenField {
        canonical: "relates_to",
        aliases: &["relates-to"],
        field_type: OpenFieldType::StringList,
        category: FieldCategory::Relationship,
    },
    KnownOpenField {
        canonical: "depends_on",
        aliases: &["depends-on"],
        field_type: OpenFieldType::StringList,
        category: FieldCategory::Relationship,
    },
    KnownOpenField {
        canonical: "extends",
        aliases: &[],
        field_type: OpenFieldType::StringList,
        category: FieldCategory::Relationship,
    },
    KnownOpenField {
        canonical: "references",
        aliases: &[],
        field_type: OpenFieldType::StringList,
        category: FieldCategory::Relationship,
    },
    KnownOpenField {
        canonical: "preceded_by",
        aliases: &["preceded-by"],
        field_type: OpenFieldType::StringList,
        category: FieldCategory::Relationship,
    },
    KnownOpenField {
        canonical: "derived_from",
        aliases: &["derived-from"],
        field_type: OpenFieldType::StringList,
        category: FieldCategory::Relationship,
    },
    KnownOpenField {
        canonical: "parent",
        aliases: &[],
        field_type: OpenFieldType::String,
        category: FieldCategory::Relationship,
    },
    // Metadata — Obsidian-compatible, NOT resource refs.
    KnownOpenField {
        canonical: "tags",
        aliases: &[],
        field_type: OpenFieldType::Tags,
        category: FieldCategory::Metadata,
    },
    KnownOpenField {
        canonical: "aliases",
        aliases: &[],
        field_type: OpenFieldType::StringList,
        category: FieldCategory::Metadata,
    },
    KnownOpenField {
        canonical: "date",
        aliases: &[],
        field_type: OpenFieldType::String,
        category: FieldCategory::Metadata,
    },
];

/// Look up a known open field by either its canonical name or one of
/// its aliases. Returns `None` for unknown keys.
pub fn lookup(key: &str) -> Option<&'static KnownOpenField> {
    KNOWN_OPEN_FIELDS.iter().find(|f| {
        f.canonical == key || f.aliases.iter().any(|a| *a == key)
    })
}

#[cfg(test)]
mod tests {
    // (tests defined above — keep them here verbatim)
    use super::*;
    // ... (same tests from Step 1)
}
```

Paste the tests from Step 1 into the `#[cfg(test)] mod tests` block at the bottom of the file.

- [ ] **Step 4: Run tests to verify they pass**

Run:
```bash
cargo nextest run -p temper-core frontmatter::registry
```
Expected: 8 tests pass.

- [ ] **Step 5: Commit**

```bash
git add crates/temper-core/src/frontmatter/registry.rs
git commit -m "feat(frontmatter): KNOWN_OPEN_FIELDS registry with alias lookups"
```

---

## Task 4: `parse.rs` — text split, YAML parse, alias normalization

**Files:**
- Modify: `crates/temper-core/src/frontmatter/parse.rs`

**Why:** `document.rs`'s `TryFrom<&str> for Frontmatter` is implemented on top of these primitives. Parse happens first, alias normalization happens next, tier split (Task 5) consumes the normalized value.

**Design notes:**
- Text split reuses the same fence-detection logic as `normalize::split_frontmatter_block` but returns owned strings so the parser can borrow-check cleanly. (Session 2 will delete the original.)
- `normalize_aliases` rewrites any top-level mapping key that `registry::lookup(key)` recognizes as a non-canonical alias, replacing it with its canonical form. It operates on the root mapping only — aliases don't nest.
- `normalize_aliases` is idempotent.

- [ ] **Step 1: Write failing tests for `split_frontmatter_block`**

Start replacing `parse.rs`:

```rust
//! Text block splitting, YAML parsing, and alias normalization at the
//! parse boundary.
//!
//! The three functions in this module are the entry points for turning
//! a vault markdown file string into an alias-normalized
//! `serde_yaml::Value` plus its body. Consumers in [`crate::frontmatter::document`]
//! chain them together inside `TryFrom<&str> for Frontmatter`.

use crate::error::{Result, TemperError};
use crate::frontmatter::registry::{lookup, KnownOpenField};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_block_strips_opening_and_closing_fences() {
        let input = "---\na: 1\n---\nbody\n";
        let (yaml, body) = split_frontmatter_block(input).expect("split ok");
        assert_eq!(yaml, "a: 1\n");
        assert_eq!(body, "body\n");
    }

    #[test]
    fn split_block_handles_bom() {
        let input = "\u{feff}---\na: 1\n---\n";
        let (yaml, body) = split_frontmatter_block(input).expect("split ok");
        assert_eq!(yaml, "a: 1\n");
        assert_eq!(body, "");
    }

    #[test]
    fn split_block_rejects_missing_opening_fence() {
        let input = "no frontmatter here\n";
        assert!(split_frontmatter_block(input).is_err());
    }

    #[test]
    fn split_block_rejects_unterminated_block() {
        let input = "---\na: 1\n";
        assert!(split_frontmatter_block(input).is_err());
    }

    #[test]
    fn split_block_preserves_body_byte_for_byte() {
        let input = "---\nk: v\n---\nline1\nline2\n\nline4\n";
        let (_, body) = split_frontmatter_block(input).expect("ok");
        assert_eq!(body, "line1\nline2\n\nline4\n");
    }

    #[test]
    fn parse_yaml_succeeds_for_mapping() {
        let value = parse_yaml("a: 1\nb: [x, y]\n").expect("parse ok");
        assert!(value.as_mapping().is_some());
    }

    #[test]
    fn parse_yaml_errors_on_non_mapping_root() {
        assert!(parse_yaml("- just\n- a\n- list\n").is_err());
    }

    #[test]
    fn normalize_aliases_rewrites_hyphen_form_keys() {
        let mut v: serde_yaml::Value = serde_yaml::from_str(
            "relates-to: [a]\ndepends-on: [b]\nparent: c\n",
        ).unwrap();
        normalize_aliases(&mut v);
        let m = v.as_mapping().unwrap();
        assert!(m.contains_key(serde_yaml::Value::String("relates_to".into())));
        assert!(m.contains_key(serde_yaml::Value::String("depends_on".into())));
        assert!(!m.contains_key(serde_yaml::Value::String("relates-to".into())));
        assert!(!m.contains_key(serde_yaml::Value::String("depends-on".into())));
    }

    #[test]
    fn normalize_aliases_preserves_values() {
        let mut v: serde_yaml::Value = serde_yaml::from_str(
            "relates-to: [a, b, c]\n",
        ).unwrap();
        normalize_aliases(&mut v);
        let list = v.as_mapping().unwrap()
            .get(serde_yaml::Value::String("relates_to".into()))
            .unwrap()
            .as_sequence().unwrap();
        assert_eq!(list.len(), 3);
    }

    #[test]
    fn normalize_aliases_is_idempotent() {
        let mut v: serde_yaml::Value = serde_yaml::from_str(
            "relates-to: [a]\n",
        ).unwrap();
        normalize_aliases(&mut v);
        let before = v.clone();
        normalize_aliases(&mut v);
        assert_eq!(before, v);
    }

    #[test]
    fn normalize_aliases_ignores_unknown_hyphen_keys() {
        let mut v: serde_yaml::Value = serde_yaml::from_str(
            "my-custom-field: value\n",
        ).unwrap();
        let before = v.clone();
        normalize_aliases(&mut v);
        assert_eq!(before, v);
    }

    #[test]
    fn normalize_aliases_collision_prefers_canonical_form() {
        // If both alias and canonical are present (unlikely but possible
        // after hand edits), keep the canonical value and drop the alias.
        let mut v: serde_yaml::Value = serde_yaml::from_str(
            "relates_to: [canonical]\nrelates-to: [alias]\n",
        ).unwrap();
        normalize_aliases(&mut v);
        let m = v.as_mapping().unwrap();
        assert!(!m.contains_key(serde_yaml::Value::String("relates-to".into())));
        let list = m.get(serde_yaml::Value::String("relates_to".into()))
            .unwrap().as_sequence().unwrap();
        assert_eq!(list[0].as_str().unwrap(), "canonical");
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run:
```bash
cargo nextest run -p temper-core frontmatter::parse 2>&1 | tail -30
```
Expected: compilation errors for `split_frontmatter_block`, `parse_yaml`, `normalize_aliases`.

- [ ] **Step 3: Implement `split_frontmatter_block`**

Add to `parse.rs` (above the `mod tests` block):

```rust
/// Split a vault file into (yaml_frontmatter_text, body).
///
/// Requires the file to begin with `---` (optionally preceded by a UTF-8 BOM)
/// and contain a closing `---` on its own line. Body is returned byte-for-byte.
pub fn split_frontmatter_block(content: &str) -> Result<(String, String)> {
    let stripped = content.strip_prefix('\u{feff}').unwrap_or(content);

    let after_open = stripped
        .strip_prefix("---\n")
        .or_else(|| stripped.strip_prefix("---\r\n"))
        .ok_or_else(|| {
            TemperError::Config(
                "missing frontmatter block: file must begin with '---'".to_string(),
            )
        })?;

    let close_idx = find_closing_fence(after_open).ok_or_else(|| {
        TemperError::Config(
            "unterminated frontmatter block: missing closing '---'".to_string(),
        )
    })?;

    let yaml_text = after_open[..close_idx].to_string();
    let after_yaml = &after_open[close_idx..];

    let body = after_yaml
        .strip_prefix("---\n")
        .or_else(|| after_yaml.strip_prefix("---\r\n"))
        .or_else(|| after_yaml.strip_prefix("---"))
        .unwrap_or("")
        .to_string();

    Ok((yaml_text, body))
}

/// Locate the byte offset of the closing `---` fence inside `after_open`.
fn find_closing_fence(after_open: &str) -> Option<usize> {
    let mut search_from = 0;
    while let Some(rel) = after_open[search_from..].find("---") {
        let abs = search_from + rel;
        let at_line_start = abs == 0 || after_open.as_bytes()[abs - 1] == b'\n';
        let after = &after_open[abs + 3..];
        let at_line_end =
            after.is_empty() || after.starts_with('\n') || after.starts_with("\r\n");
        if at_line_start && at_line_end {
            return Some(abs);
        }
        search_from = abs + 3;
    }
    None
}
```

- [ ] **Step 4: Implement `parse_yaml`**

Add to `parse.rs`:

```rust
/// Parse a YAML text block into a `serde_yaml::Value`. The root must be
/// a mapping — anything else is rejected.
pub fn parse_yaml(text: &str) -> Result<serde_yaml::Value> {
    let value: serde_yaml::Value = serde_yaml::from_str(text)
        .map_err(|e| TemperError::Config(format!("failed to parse YAML frontmatter: {e}")))?;
    if value.as_mapping().is_none() {
        return Err(TemperError::Config(
            "frontmatter is not a YAML mapping".to_string(),
        ));
    }
    Ok(value)
}
```

- [ ] **Step 5: Implement `normalize_aliases`**

Add to `parse.rs`:

```rust
/// Rewrite known hyphen-form aliases to their canonical underscore form.
///
/// Operates in place on the top-level mapping. Non-mapping values and
/// unknown hyphen keys are left alone. If both the alias and the canonical
/// form are present (unlikely but possible after hand edits), the canonical
/// value wins and the alias is dropped.
pub fn normalize_aliases(value: &mut serde_yaml::Value) {
    let Some(mapping) = value.as_mapping_mut() else {
        return;
    };

    // Collect (alias_key, canonical_key) pairs first — we can't mutate
    // while iterating.
    let mut rewrites: Vec<(serde_yaml::Value, serde_yaml::Value)> = Vec::new();
    for (k, _) in mapping.iter() {
        let Some(k_str) = k.as_str() else {
            continue;
        };
        if let Some(entry) = alias_target(k_str) {
            if entry.canonical != k_str {
                rewrites.push((
                    k.clone(),
                    serde_yaml::Value::String(entry.canonical.to_string()),
                ));
            }
        }
    }

    for (alias_key, canonical_key) in rewrites {
        // If canonical is already present, drop the alias (canonical wins).
        if mapping.contains_key(&canonical_key) {
            mapping.remove(&alias_key);
            continue;
        }
        // Otherwise rename: remove old, insert new with old's value.
        if let Some(val) = mapping.remove(&alias_key) {
            mapping.insert(canonical_key, val);
        }
    }
}

/// Look up whether `key` matches any known open field (canonical or alias).
fn alias_target(key: &str) -> Option<&'static KnownOpenField> {
    lookup(key)
}
```

- [ ] **Step 6: Run tests to verify they pass**

Run:
```bash
cargo nextest run -p temper-core frontmatter::parse
```
Expected: all 12 tests pass.

- [ ] **Step 7: Commit**

```bash
git add crates/temper-core/src/frontmatter/parse.rs
git commit -m "feat(frontmatter): text split, YAML parse, alias normalization"
```

---

## Task 5: `tiers.rs` — explicit managed/open split

**Files:**
- Modify: `crates/temper-core/src/frontmatter/tiers.rs`

**Why:** Both `document.rs::managed_json / open_json` and `canonical.rs` (for ordering) need a routing primitive. The algorithm here explicitly references the known-open registry rather than depending on the `$ref`-not-followed quirk of `schema::schema_value`. This is where the "correct-by-accident → correct-by-design" fix lands.

**Routing rules (exactly):**
1. If the key is in `IDENTITY_FIELDS` or `TIER1_SYSTEM_FIELDS` → **skip** (neither tier).
2. Else if the key starts with `temper-` → **managed**.
3. Else if the key is `title` or `slug` → **managed**.
4. Else if the key appears in the **doc-type schema's own `properties`** (not base) → **managed**. (Example: `date` for session, `temper-stage` for task already handled by rule 2.)
5. Else if `registry::lookup(key)` is `Some` → **open** (known open field — relationships and metadata).
6. Else → **open** (unknown fields, preserved).

The crucial difference from the old algorithm: rule 5 makes known-open routing **explicit**, so the old accident of `schema_value` not following `$ref` is no longer load-bearing.

**Regression anchor:** step 8 asserts that for every existing vault fixture, this routing matches `hash::split_frontmatter_tiers` byte-for-byte. That guarantees we aren't silently changing hash inputs.

- [ ] **Step 1: Write failing tests**

Replace `tiers.rs`:

```rust
//! Managed / open tier splitting. Routes explicitly via the known-open
//! registry rather than relying on `$ref` not being followed.

use crate::frontmatter::document::DocType;
use crate::frontmatter::fields::{IDENTITY_FIELDS, TIER1_SYSTEM_FIELDS};
use crate::frontmatter::registry::lookup as registry_lookup;
use std::collections::HashSet;

// Implementation below.

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn yaml(s: &str) -> serde_yaml::Value {
        serde_yaml::from_str(s).unwrap()
    }

    #[test]
    fn identity_fields_are_stripped() {
        let v = yaml(r#"
temper-id: "019d8110-8ff3-70c2-85ae-57e04ed62885"
temper-provisional-id: "019d8110-8ff3-70c2-85ae-57e04ed62886"
title: Hello
slug: hello
"#);
        let (managed, open) = split_managed_open(&v, DocType::Task);
        assert!(managed.get("temper-id").is_none());
        assert!(managed.get("temper-provisional-id").is_none());
        assert!(open.get("temper-id").is_none());
    }

    #[test]
    fn tier1_system_fields_are_stripped() {
        let v = yaml(r#"
temper-type: task
temper-context: temper
temper-created: "2026-04-13T00:00:00Z"
temper-updated: "2026-04-13T00:00:00Z"
temper-owner: "@me"
temper-source: manual
title: T
slug: t
"#);
        let (managed, open) = split_managed_open(&v, DocType::Task);
        for f in ["temper-type", "temper-context", "temper-created", "temper-updated", "temper-owner", "temper-source"] {
            assert!(managed.get(f).is_none(), "{f} must not be in managed");
            assert!(open.get(f).is_none(), "{f} must not be in open");
        }
    }

    #[test]
    fn temper_prefixed_fields_go_to_managed() {
        let v = yaml(r#"
title: T
slug: t
temper-stage: in-progress
temper-mode: build
temper-effort: medium
"#);
        let (managed, _open) = split_managed_open(&v, DocType::Task);
        assert_eq!(managed["temper-stage"], json!("in-progress"));
        assert_eq!(managed["temper-mode"], json!("build"));
        assert_eq!(managed["temper-effort"], json!("medium"));
    }

    #[test]
    fn title_and_slug_go_to_managed() {
        let v = yaml(r#"
title: Hello
slug: hello
"#);
        let (managed, open) = split_managed_open(&v, DocType::Task);
        assert_eq!(managed["title"], json!("Hello"));
        assert_eq!(managed["slug"], json!("hello"));
        assert!(open.get("title").is_none());
        assert!(open.get("slug").is_none());
    }

    #[test]
    fn doc_type_specific_properties_go_to_managed() {
        // `date` is declared in session.schema.json (not base), so it
        // routes to managed for sessions.
        let v = yaml(r#"
title: My session
slug: my-session
date: "2026-04-13"
"#);
        let (managed, open) = split_managed_open(&v, DocType::Session);
        assert_eq!(managed["date"], json!("2026-04-13"));
        assert!(open.get("date").is_none());
    }

    #[test]
    fn known_open_relationship_fields_go_to_open() {
        let v = yaml(r#"
title: T
slug: t
relates_to: [a, b]
depends_on: [c]
parent: p
"#);
        let (managed, open) = split_managed_open(&v, DocType::Task);
        assert_eq!(open["relates_to"], json!(["a", "b"]));
        assert_eq!(open["depends_on"], json!(["c"]));
        assert_eq!(open["parent"], json!("p"));
        assert!(managed.get("relates_to").is_none());
    }

    #[test]
    fn known_open_metadata_fields_go_to_open() {
        let v = yaml(r#"
title: T
slug: t
tags: [auth, observability]
aliases: [alt]
"#);
        let (_managed, open) = split_managed_open(&v, DocType::Task);
        assert_eq!(open["tags"], json!(["auth", "observability"]));
        assert_eq!(open["aliases"], json!(["alt"]));
    }

    #[test]
    fn unknown_fields_go_to_open() {
        let v = yaml(r#"
title: T
slug: t
custom_field: 42
another: something
"#);
        let (_managed, open) = split_managed_open(&v, DocType::Task);
        assert_eq!(open["custom_field"], json!(42));
        assert_eq!(open["another"], json!("something"));
    }

    #[test]
    fn session_date_does_not_accidentally_collide_with_metadata_date() {
        // session.schema.json declares `date`, so it's managed for sessions.
        // `date` is also in the metadata registry. Doc-type schema wins:
        // session `date` must land in managed, not open.
        let v = yaml(r#"
title: S
slug: s
date: "2026-04-13"
"#);
        let (managed, open) = split_managed_open(&v, DocType::Session);
        assert_eq!(managed["date"], json!("2026-04-13"));
        assert!(open.get("date").is_none());
    }

    #[test]
    fn non_mapping_input_returns_empty_tiers() {
        let v: serde_yaml::Value = serde_yaml::from_str("- just\n- a list\n").unwrap();
        let (managed, open) = split_managed_open(&v, DocType::Task);
        assert_eq!(managed, json!({}));
        assert_eq!(open, json!({}));
    }

    // Regression anchor: for inputs that the old and new algorithms both
    // handle, routing must be byte-identical.
    #[test]
    fn matches_legacy_split_for_task_fixture() {
        let v = yaml(r#"
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
"#);
        let (new_managed, new_open) = split_managed_open(&v, DocType::Task);
        let (legacy_managed, legacy_open) = crate::hash::split_frontmatter_tiers(&v, "task");
        assert_eq!(new_managed, legacy_managed, "managed tier drift for task");
        assert_eq!(new_open, legacy_open, "open tier drift for task");
    }

    #[test]
    fn matches_legacy_split_for_session_fixture() {
        let v = yaml(r#"
temper-id: "019d8110-8ff3-70c2-85ae-57e04ed62885"
temper-type: session
temper-context: temper
temper-created: "2026-04-13T00:00:00Z"
title: S
slug: s
date: "2026-04-13"
relates_to: [a]
tags: [x]
"#);
        let (new_managed, new_open) = split_managed_open(&v, DocType::Session);
        let (legacy_managed, legacy_open) = crate::hash::split_frontmatter_tiers(&v, "session");
        assert_eq!(new_managed, legacy_managed, "managed tier drift for session");
        assert_eq!(new_open, legacy_open, "open tier drift for session");
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run:
```bash
cargo nextest run -p temper-core frontmatter::tiers 2>&1 | tail -30
```
Expected: compilation errors for `split_managed_open`.

- [ ] **Step 3: Implement `split_managed_open`**

Add to `tiers.rs` (above the test module):

```rust
/// Split a YAML frontmatter mapping into (managed_json, open_json) tiers.
///
/// Routing rules, applied in order:
/// 1. Keys in [`IDENTITY_FIELDS`] or [`TIER1_SYSTEM_FIELDS`] → dropped.
/// 2. Keys prefixed `temper-` → managed.
/// 3. Keys `title` / `slug` → managed.
/// 4. Keys in the doc-type schema's own `properties` (not base) → managed.
/// 5. Known open fields from the registry → open.
/// 6. Everything else → open (unknown fields preserved).
///
/// Rule 4 uses `crate::schema::schema_value` which returns the doc-type
/// schema's own `properties` object — it does NOT follow `$ref`. That is
/// deliberate: base-schema fields like `relates_to` must route to open
/// via rule 5 (the registry), not via rule 4.
pub fn split_managed_open(
    fm: &serde_yaml::Value,
    doc_type: DocType,
) -> (serde_json::Value, serde_json::Value) {
    let Some(mapping) = fm.as_mapping() else {
        return (serde_json::json!({}), serde_json::json!({}));
    };

    let skip: HashSet<&str> = IDENTITY_FIELDS
        .iter()
        .chain(TIER1_SYSTEM_FIELDS.iter())
        .copied()
        .collect();

    let schema_keys: HashSet<String> = crate::schema::schema_value(doc_type.as_str())
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
        if skip.contains(key_str) {
            continue;
        }
        let json_value = serde_json::to_value(value).unwrap_or(serde_json::Value::Null);

        let to_managed = key_str.starts_with("temper-")
            || key_str == "title"
            || key_str == "slug"
            || schema_keys.contains(key_str);

        if to_managed {
            managed.insert(key_str.to_string(), json_value);
        } else {
            // Rule 5 and rule 6 collapse to the same bucket — known open
            // fields and unknowns both land in `open`. The registry still
            // matters for `canonical::serialize`'s ordering.
            let _ = registry_lookup(key_str); // explicit for readers
            open.insert(key_str.to_string(), json_value);
        }
    }

    (
        serde_json::Value::Object(managed),
        serde_json::Value::Object(open),
    )
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run:
```bash
cargo nextest run -p temper-core frontmatter::tiers
```
Expected: 11 tests pass — including the two `matches_legacy_split_*` regression tests that compare against `crate::hash::split_frontmatter_tiers`.

- [ ] **Step 5: Commit**

```bash
git add crates/temper-core/src/frontmatter/tiers.rs
git commit -m "feat(frontmatter): explicit managed/open tier splitting"
```

---

## Task 6: `canonical.rs` — 5-tier display ordering

**Files:**
- Modify: `crates/temper-core/src/frontmatter/canonical.rs`

**Why:** `document.rs::serialize` uses this to write deterministic, diff-friendly YAML. Must be bit-deterministic under arbitrary input orderings.

**Algorithm (exact):**
1. **Identity fields** — emit in the fixed order declared by `IDENTITY_FIELDS` (only fields that are actually present).
2. **Tier-1 system fields** — emit in the fixed order declared by `TIER1_SYSTEM_FIELDS` (only if present).
3. **Managed fields** — emit `title`, `slug`, then all other keys that belong in the managed tier (rule-4 doc-type schema properties, plus any extra `temper-*` keys not in Tier-1), in the order the doc-type schema's `properties` object enumerates them. Any additional managed keys beyond schema declarations get alphabetized at the end of this tier.
4. **Known open fields** — emit in `KNOWN_OPEN_FIELDS` registry order (relationships first, then metadata), skipping any not present.
5. **Unknown open fields** — preserved in original input order. Alphabetize only when two unknown keys came from the same source ordering; otherwise, preserve.

The algorithm is **pure** — given an input `serde_yaml::Value`, it returns a new `serde_yaml::Value` containing the same key/value pairs reordered.

- [ ] **Step 1: Write failing tests**

Replace `canonical.rs`:

```rust
//! Canonical 5-tier display ordering for `Frontmatter::serialize()`.
//!
//! This is strictly a display concern. Hashing uses alphabetical
//! `BTreeMap` canonicalization in `crate::hash::canonicalize_json` — the
//! two algorithms are independent, and a test in `document.rs` locks
//! that independence in.

use crate::frontmatter::document::DocType;
use crate::frontmatter::fields::{IDENTITY_FIELDS, TIER1_SYSTEM_FIELDS};
use crate::frontmatter::registry::KNOWN_OPEN_FIELDS;

// Implementation below.

#[cfg(test)]
mod tests {
    use super::*;

    fn yaml(s: &str) -> serde_yaml::Value {
        serde_yaml::from_str(s).unwrap()
    }

    fn keys_of(v: &serde_yaml::Value) -> Vec<String> {
        v.as_mapping()
            .unwrap()
            .iter()
            .map(|(k, _)| k.as_str().unwrap().to_string())
            .collect()
    }

    #[test]
    fn identity_fields_come_first_in_fixed_order() {
        let v = yaml(r#"
title: T
slug: t
temper-provisional-id: "019d8110-8ff3-70c2-85ae-57e04ed62886"
temper-id: "019d8110-8ff3-70c2-85ae-57e04ed62885"
"#);
        let out = canonicalize(&v, DocType::Task);
        let ks = keys_of(&out);
        assert_eq!(ks[0], "temper-id");
        assert_eq!(ks[1], "temper-provisional-id");
    }

    #[test]
    fn tier1_system_fields_follow_identity_in_fixed_order() {
        let v = yaml(r#"
title: T
slug: t
temper-updated: "2026-04-13T00:00:00Z"
temper-context: temper
temper-id: "019d8110-8ff3-70c2-85ae-57e04ed62885"
temper-type: task
temper-created: "2026-04-12T00:00:00Z"
"#);
        let out = canonicalize(&v, DocType::Task);
        let ks = keys_of(&out);
        let start = ks.iter().position(|k| k == "temper-context").unwrap();
        // Fixed tier-1 order from TIER1_SYSTEM_FIELDS.
        let expected_order = [
            "temper-context", "temper-type", "temper-created", "temper-updated",
        ];
        // Every present tier-1 key preserves the TIER1_SYSTEM_FIELDS order.
        let mut prev_idx = usize::MAX;
        for key in expected_order {
            if let Some(pos) = ks.iter().position(|k| k == key) {
                if prev_idx != usize::MAX {
                    assert!(pos > prev_idx, "tier1 key {key} out of order");
                }
                prev_idx = pos;
            }
        }
        assert!(start < ks.iter().position(|k| k == "title").unwrap(),
            "tier1 must precede managed");
    }

    #[test]
    fn title_comes_before_slug_in_managed_tier() {
        let v = yaml(r#"
slug: t
title: T
temper-id: "019d8110-8ff3-70c2-85ae-57e04ed62885"
temper-type: task
"#);
        let out = canonicalize(&v, DocType::Task);
        let ks = keys_of(&out);
        let title_idx = ks.iter().position(|k| k == "title").unwrap();
        let slug_idx = ks.iter().position(|k| k == "slug").unwrap();
        assert!(title_idx < slug_idx);
    }

    #[test]
    fn doc_type_schema_properties_land_in_managed_in_schema_order() {
        // task.schema.json declares: temper-stage, temper-mode, temper-effort,
        // temper-goal, temper-seq, temper-branch, temper-pr, slug.
        let v = yaml(r#"
temper-pr: pr-url
temper-mode: build
temper-stage: in-progress
title: T
temper-id: "019d8110-8ff3-70c2-85ae-57e04ed62885"
temper-effort: small
"#);
        let out = canonicalize(&v, DocType::Task);
        let ks = keys_of(&out);
        // temper-stage must precede temper-mode must precede temper-effort must precede temper-pr.
        let stage = ks.iter().position(|k| k == "temper-stage").unwrap();
        let mode = ks.iter().position(|k| k == "temper-mode").unwrap();
        let effort = ks.iter().position(|k| k == "temper-effort").unwrap();
        let pr = ks.iter().position(|k| k == "temper-pr").unwrap();
        assert!(stage < mode && mode < effort && effort < pr);
    }

    #[test]
    fn known_open_fields_follow_in_registry_order() {
        let v = yaml(r#"
title: T
slug: t
temper-id: "019d8110-8ff3-70c2-85ae-57e04ed62885"
temper-type: task
tags: [x]
relates_to: [a]
depends_on: [b]
parent: p
"#);
        let out = canonicalize(&v, DocType::Task);
        let ks = keys_of(&out);
        let relates = ks.iter().position(|k| k == "relates_to").unwrap();
        let depends = ks.iter().position(|k| k == "depends_on").unwrap();
        let parent = ks.iter().position(|k| k == "parent").unwrap();
        let tags = ks.iter().position(|k| k == "tags").unwrap();
        // Registry order: relates_to < depends_on < ... < parent < tags (metadata).
        assert!(relates < depends);
        assert!(depends < parent);
        assert!(parent < tags);
    }

    #[test]
    fn unknown_fields_preserved_in_original_order() {
        let v = yaml(r#"
title: T
slug: t
temper-id: "019d8110-8ff3-70c2-85ae-57e04ed62885"
temper-type: task
zebra: 1
alpha: 2
mango: 3
"#);
        let out = canonicalize(&v, DocType::Task);
        let ks = keys_of(&out);
        let zebra = ks.iter().position(|k| k == "zebra").unwrap();
        let alpha = ks.iter().position(|k| k == "alpha").unwrap();
        let mango = ks.iter().position(|k| k == "mango").unwrap();
        assert!(zebra < alpha);
        assert!(alpha < mango);
    }

    #[test]
    fn canonicalize_is_idempotent() {
        let v = yaml(r#"
relates_to: [a]
title: T
slug: t
temper-id: "019d8110-8ff3-70c2-85ae-57e04ed62885"
temper-type: task
custom: 1
"#);
        let once = canonicalize(&v, DocType::Task);
        let twice = canonicalize(&once, DocType::Task);
        assert_eq!(once, twice);
    }

    #[test]
    fn canonicalize_is_deterministic_under_input_permutations() {
        let a = yaml(r#"
title: T
slug: t
temper-id: "019d8110-8ff3-70c2-85ae-57e04ed62885"
temper-type: task
relates_to: [x]
tags: [y]
"#);
        let b = yaml(r#"
tags: [y]
relates_to: [x]
temper-type: task
temper-id: "019d8110-8ff3-70c2-85ae-57e04ed62885"
slug: t
title: T
"#);
        assert_eq!(canonicalize(&a, DocType::Task), canonicalize(&b, DocType::Task));
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run:
```bash
cargo nextest run -p temper-core frontmatter::canonical 2>&1 | tail -30
```
Expected: compilation errors for `canonicalize`.

- [ ] **Step 3: Implement `canonicalize`**

Add to `canonical.rs` (above the tests):

```rust
/// Reorder a frontmatter mapping into canonical 5-tier display order.
///
/// The input is not mutated; the returned value is a new mapping with
/// the same keys and values in deterministic order.
pub fn canonicalize(
    fm: &serde_yaml::Value,
    doc_type: DocType,
) -> serde_yaml::Value {
    let Some(input) = fm.as_mapping() else {
        return fm.clone();
    };

    // Look up each key by string for cheap contains-checks.
    let contains = |key: &str| {
        input
            .iter()
            .any(|(k, _)| k.as_str().map(|s| s == key).unwrap_or(false))
    };
    let get = |key: &str| -> Option<serde_yaml::Value> {
        for (k, v) in input.iter() {
            if k.as_str().map(|s| s == key).unwrap_or(false) {
                return Some(v.clone());
            }
        }
        None
    };

    let mut out = serde_yaml::Mapping::new();
    let mut emitted: std::collections::HashSet<String> = std::collections::HashSet::new();

    let mut push = |out: &mut serde_yaml::Mapping,
                    emitted: &mut std::collections::HashSet<String>,
                    key: &str,
                    value: serde_yaml::Value| {
        out.insert(serde_yaml::Value::String(key.to_string()), value);
        emitted.insert(key.to_string());
    };

    // Tier 1a: identity fields (fixed order).
    for &field in IDENTITY_FIELDS {
        if contains(field) {
            if let Some(v) = get(field) {
                push(&mut out, &mut emitted, field, v);
            }
        }
    }

    // Tier 1b: tier-1 system fields (fixed order).
    for &field in TIER1_SYSTEM_FIELDS {
        if contains(field) {
            if let Some(v) = get(field) {
                push(&mut out, &mut emitted, field, v);
            }
        }
    }

    // Tier 2: managed fields — title, slug, then schema-declared order.
    for fixed in ["title", "slug"] {
        if contains(fixed) {
            if let Some(v) = get(fixed) {
                push(&mut out, &mut emitted, fixed, v);
            }
        }
    }
    let schema_order: Vec<String> = crate::schema::schema_value(doc_type.as_str())
        .ok()
        .and_then(|v| {
            v.get("properties").and_then(|p| p.as_object()).map(|obj| {
                obj.keys().cloned().collect::<Vec<_>>()
            })
        })
        .unwrap_or_default();
    for key in &schema_order {
        if key == "title" || key == "slug" {
            continue;
        }
        if !emitted.contains(key) && contains(key) {
            if let Some(v) = get(key) {
                push(&mut out, &mut emitted, key, v);
            }
        }
    }

    // Tier 2 (additional): any `temper-*` keys not yet emitted and not in
    // tier-1 system fields go here, alphabetically, as a safety net for
    // schema-declared fields we might not know about.
    let mut extra_temper: Vec<String> = input
        .iter()
        .filter_map(|(k, _)| k.as_str())
        .filter(|s| s.starts_with("temper-") && !emitted.contains(*s))
        .map(String::from)
        .collect();
    extra_temper.sort();
    for key in extra_temper {
        if let Some(v) = get(&key) {
            push(&mut out, &mut emitted, &key, v);
        }
    }

    // Tier 3: known open fields, registry order.
    for entry in KNOWN_OPEN_FIELDS {
        let name = entry.canonical;
        if !emitted.contains(name) && contains(name) {
            if let Some(v) = get(name) {
                push(&mut out, &mut emitted, name, v);
            }
        }
    }

    // Tier 4: unknown open fields in input order, preserving insertion order.
    for (k, v) in input.iter() {
        let Some(name) = k.as_str() else { continue };
        if !emitted.contains(name) {
            push(&mut out, &mut emitted, name, v.clone());
        }
    }

    serde_yaml::Value::Mapping(out)
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run:
```bash
cargo nextest run -p temper-core frontmatter::canonical
```
Expected: 8 tests pass.

- [ ] **Step 5: Commit**

```bash
git add crates/temper-core/src/frontmatter/canonical.rs
git commit -m "feat(frontmatter): 5-tier display ordering for serialize"
```

---

## Task 7: `document.rs` — `Frontmatter` aggregate type

**Files:**
- Modify: `crates/temper-core/src/frontmatter/document.rs`

**Why:** This is the public face of the module. Everything built in tasks 2–6 gets glued together here.

**Public API (exactly, per spec):**

```rust
pub struct Frontmatter { /* private fields */ }

impl Frontmatter {
    pub fn doc_type(&self) -> DocType;
    pub fn body(&self) -> &str;
    pub fn value(&self) -> &serde_yaml::Value;
    pub fn parse_file(path: &std::path::Path) -> Result<Self>;
    pub fn validate(&self) -> Result<Vec<crate::schema::ValidationIssue>>;
    pub fn managed_json(&self) -> serde_json::Value;
    pub fn open_json(&self) -> serde_json::Value;
    pub fn hashes(&self) -> (String, String);
    pub fn serialize(&self) -> Result<String>;
    pub fn write_to(&self, path: &std::path::Path) -> Result<()>;
    pub fn tags(&self) -> Vec<String>;

    // Mutation
    pub fn set_managed_field(&mut self, key: &str, value: serde_json::Value);
    pub fn set_open_field(&mut self, key: &str, value: serde_json::Value);
    pub fn set_relationships(&mut self, rels: &crate::types::graph::ResourceRelationships);
    pub fn remove_field(&mut self, key: &str);
}

impl TryFrom<&str> for Frontmatter {
    type Error = TemperError;
    fn try_from(content: &str) -> Result<Self>;
}
```

**Implementation discipline:**
- `TryFrom<&str>` calls `parse::split_frontmatter_block` → `parse::parse_yaml` → `parse::normalize_aliases`, then extracts `temper-type` from the mapping and calls `DocType::from_str` to set the typed doctype.
- `serialize` calls `canonical::canonicalize(&self.value, self.doc_type)` then `serde_yaml::to_string` + the `---\n<yaml>---\n<body>` envelope.
- `write_to` uses the same atomic-write pattern as `normalize.rs::write_atomic` (private helper in `document.rs`, not re-exported).
- `hashes` calls `tiers::split_managed_open` then `crate::hash::compute_managed_hash` / `compute_open_hash`. Never re-implements canonicalization for hashing.
- `set_relationships` mutates the YAML mapping: for each non-empty field in the input `ResourceRelationships`, insert/overwrite the corresponding canonical key; for each empty field, remove the corresponding canonical key if present. `tags` is NOT touched by `set_relationships` — tags are metadata, not a relationship (session 2 formalizes this by removing `tags` from `ResourceRelationships`; for session 1 we simply do not treat `tags` as a relationship here).
- `tags` reader returns the `tags` open-meta vector as `Vec<String>`, empty if absent or wrong type.

- [ ] **Step 1: Write failing tests (batch 1 — parse + accessors)**

Replace `document.rs` content with the `DocType` code from Task 1 plus a growing test module. Add the following tests under `#[cfg(test)] mod tests` (alongside the existing `doc_type_*` tests):

```rust
    const TASK_FIXTURE: &str = r#"---
temper-id: "019d8110-8ff3-70c2-85ae-57e04ed62885"
temper-type: task
temper-context: temper
temper-created: "2026-04-13T00:00:00Z"
title: My Task
slug: my-task
temper-stage: in-progress
temper-mode: build
temper-effort: small
relates_to: [other-task]
tags: [auth]
---
body content here
"#;

    #[test]
    fn try_from_str_parses_task_fixture() {
        let fm = Frontmatter::try_from(TASK_FIXTURE).expect("parse ok");
        assert_eq!(fm.doc_type(), DocType::Task);
        assert!(fm.body().starts_with("body content"));
    }

    #[test]
    fn try_from_str_fails_on_missing_temper_type() {
        let bad = "---\ntitle: T\nslug: t\n---\n";
        assert!(Frontmatter::try_from(bad).is_err());
    }

    #[test]
    fn try_from_str_fails_on_unknown_temper_type() {
        let bad = "---\ntemper-type: bogus\ntitle: T\nslug: t\n---\n";
        assert!(Frontmatter::try_from(bad).is_err());
    }

    #[test]
    fn try_from_str_normalizes_hyphen_aliases() {
        let input = r#"---
temper-id: "019d8110-8ff3-70c2-85ae-57e04ed62885"
temper-type: task
temper-context: temper
temper-created: "2026-04-13T00:00:00Z"
title: T
slug: t
relates-to: [a]
depends-on: [b]
---
"#;
        let fm = Frontmatter::try_from(input).expect("parse ok");
        let m = fm.value().as_mapping().unwrap();
        assert!(m.contains_key(serde_yaml::Value::String("relates_to".into())));
        assert!(m.contains_key(serde_yaml::Value::String("depends_on".into())));
        assert!(!m.contains_key(serde_yaml::Value::String("relates-to".into())));
    }
```

- [ ] **Step 2: Run tests — they fail**

Run:
```bash
cargo nextest run -p temper-core frontmatter::document 2>&1 | tail -20
```
Expected: compilation errors — `Frontmatter`, `try_from`, `body`, `value`, etc. do not exist yet.

- [ ] **Step 3: Implement the `Frontmatter` struct + `TryFrom<&str>` + simple accessors**

Add to `document.rs` (below `DocType` and above the tests):

```rust
use crate::frontmatter::parse::{normalize_aliases, parse_yaml, split_frontmatter_block};

/// Authoritative in-memory representation of a vault markdown file's
/// frontmatter block plus its body.
///
/// Invariants:
/// - `value` is alias-normalized (hyphen-form keys rewritten to canonical
///   underscore form) at construction time.
/// - `doc_type` is a typed enum — unknown doctypes are rejected at parse.
/// - `body` is preserved byte-for-byte; writes re-emit it unchanged.
#[derive(Debug, Clone)]
pub struct Frontmatter {
    doc_type: DocType,
    value: serde_yaml::Value,
    body: String,
}

impl Frontmatter {
    /// Typed doctype of this frontmatter.
    pub fn doc_type(&self) -> DocType {
        self.doc_type
    }

    /// The canonicalized frontmatter value (alias-normalized).
    pub fn value(&self) -> &serde_yaml::Value {
        &self.value
    }

    /// The markdown body preserved byte-for-byte.
    pub fn body(&self) -> &str {
        &self.body
    }
}

impl TryFrom<&str> for Frontmatter {
    type Error = TemperError;

    fn try_from(content: &str) -> Result<Self> {
        let (yaml_text, body) = split_frontmatter_block(content)?;
        let mut value = parse_yaml(&yaml_text)?;
        normalize_aliases(&mut value);

        let mapping = value
            .as_mapping()
            .ok_or_else(|| TemperError::Config("frontmatter is not a mapping".to_string()))?;
        let type_value = mapping
            .get(serde_yaml::Value::String("temper-type".into()))
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                TemperError::Config("frontmatter missing required `temper-type`".to_string())
            })?;
        let doc_type = DocType::from_str(type_value)?;

        Ok(Self {
            doc_type,
            value,
            body,
        })
    }
}
```

- [ ] **Step 4: Run the batch-1 tests**

Run:
```bash
cargo nextest run -p temper-core frontmatter::document
```
Expected: the four new tests pass alongside the pre-existing `doc_type_*` tests.

- [ ] **Step 5: Add batch-2 tests (tier views + hashes)**

Append to the test module:

```rust
    #[test]
    fn managed_json_contains_title_slug_and_temper_fields() {
        let fm = Frontmatter::try_from(TASK_FIXTURE).unwrap();
        let m = fm.managed_json();
        let obj = m.as_object().unwrap();
        assert_eq!(obj.get("title").and_then(|v| v.as_str()), Some("My Task"));
        assert_eq!(obj.get("slug").and_then(|v| v.as_str()), Some("my-task"));
        assert_eq!(
            obj.get("temper-stage").and_then(|v| v.as_str()),
            Some("in-progress")
        );
    }

    #[test]
    fn open_json_contains_relationships_and_tags() {
        let fm = Frontmatter::try_from(TASK_FIXTURE).unwrap();
        let o = fm.open_json();
        let obj = o.as_object().unwrap();
        assert!(obj.contains_key("relates_to"));
        assert!(obj.contains_key("tags"));
    }

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

    #[test]
    fn hashes_are_independent_of_input_key_ordering() {
        let a = Frontmatter::try_from(TASK_FIXTURE).unwrap();
        let permuted = r#"---
temper-type: task
temper-created: "2026-04-13T00:00:00Z"
title: My Task
temper-id: "019d8110-8ff3-70c2-85ae-57e04ed62885"
relates_to: [other-task]
temper-effort: small
temper-mode: build
temper-context: temper
temper-stage: in-progress
tags: [auth]
slug: my-task
---
body content here
"#;
        let b = Frontmatter::try_from(permuted).unwrap();
        assert_eq!(a.hashes(), b.hashes(),
            "hash must be stable under input reordering");
    }

    #[test]
    fn hashes_are_independent_of_alias_form() {
        let canonical_input = r#"---
temper-id: "019d8110-8ff3-70c2-85ae-57e04ed62885"
temper-type: task
temper-context: temper
temper-created: "2026-04-13T00:00:00Z"
title: T
slug: t
relates_to: [a]
depends_on: [b]
---
"#;
        let alias_input = r#"---
temper-id: "019d8110-8ff3-70c2-85ae-57e04ed62885"
temper-type: task
temper-context: temper
temper-created: "2026-04-13T00:00:00Z"
title: T
slug: t
relates-to: [a]
depends-on: [b]
---
"#;
        let a = Frontmatter::try_from(canonical_input).unwrap();
        let b = Frontmatter::try_from(alias_input).unwrap();
        assert_eq!(a.hashes(), b.hashes(),
            "alias-form and canonical-form must hash identically");
    }
```

- [ ] **Step 6: Run tests — they fail**

Run:
```bash
cargo nextest run -p temper-core frontmatter::document 2>&1 | tail -20
```
Expected: compilation errors for `managed_json`, `open_json`, `hashes`.

- [ ] **Step 7: Implement `managed_json`, `open_json`, `hashes`**

Add to `impl Frontmatter`:

```rust
    /// Managed-tier JSON projection of this frontmatter.
    pub fn managed_json(&self) -> serde_json::Value {
        let (managed, _) =
            crate::frontmatter::tiers::split_managed_open(&self.value, self.doc_type);
        managed
    }

    /// Open-tier JSON projection of this frontmatter.
    pub fn open_json(&self) -> serde_json::Value {
        let (_, open) =
            crate::frontmatter::tiers::split_managed_open(&self.value, self.doc_type);
        open
    }

    /// (managed_hash, open_hash) for this frontmatter.
    ///
    /// Delegates unchanged to `crate::hash::compute_managed_hash` /
    /// `compute_open_hash`. The display canonicalization in
    /// `crate::frontmatter::canonical` has zero effect on this output.
    pub fn hashes(&self) -> (String, String) {
        let managed = self.managed_json();
        let open = self.open_json();
        (
            crate::hash::compute_managed_hash(self.doc_type.as_str(), &managed),
            crate::hash::compute_open_hash(&open),
        )
    }
```

- [ ] **Step 8: Run tests — they pass**

```bash
cargo nextest run -p temper-core frontmatter::document
```
Expected: all new tests pass.

- [ ] **Step 9: Add batch-3 tests (serialize, write_to, parse_file, validate, tags)**

Append:

```rust
    #[test]
    fn serialize_emits_canonical_order() {
        let permuted = r#"---
slug: my-task
relates_to: [other]
temper-stage: in-progress
title: My Task
temper-type: task
temper-id: "019d8110-8ff3-70c2-85ae-57e04ed62885"
temper-context: temper
temper-created: "2026-04-13T00:00:00Z"
---
body
"#;
        let fm = Frontmatter::try_from(permuted).unwrap();
        let out = fm.serialize().unwrap();

        // Prefix contains identity, tier1, then managed in the right order.
        let yaml_part = out.split("---\n").nth(1).unwrap();
        let id_pos = yaml_part.find("temper-id:").unwrap();
        let type_pos = yaml_part.find("temper-type:").unwrap();
        let title_pos = yaml_part.find("title:").unwrap();
        let stage_pos = yaml_part.find("temper-stage:").unwrap();
        assert!(id_pos < type_pos);
        assert!(type_pos < title_pos);
        assert!(title_pos < stage_pos);

        // And the body is preserved.
        assert!(out.ends_with("body\n"));
    }

    #[test]
    fn serialize_is_idempotent_fixed_point() {
        let fm = Frontmatter::try_from(TASK_FIXTURE).unwrap();
        let once = fm.serialize().unwrap();
        let twice = Frontmatter::try_from(once.as_str()).unwrap().serialize().unwrap();
        assert_eq!(once, twice, "canonical form must be a fixed point");
    }

    #[test]
    fn parse_file_and_write_to_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("task.md");
        std::fs::write(&path, TASK_FIXTURE).unwrap();

        let fm = Frontmatter::parse_file(&path).unwrap();
        let other = dir.path().join("task2.md");
        fm.write_to(&other).unwrap();

        let round = Frontmatter::parse_file(&other).unwrap();
        assert_eq!(fm.hashes(), round.hashes());
        assert_eq!(fm.body(), round.body());
    }

    #[test]
    fn tags_accessor_reads_open_meta_tags() {
        let fm = Frontmatter::try_from(TASK_FIXTURE).unwrap();
        assert_eq!(fm.tags(), vec!["auth".to_string()]);
    }

    #[test]
    fn tags_accessor_returns_empty_when_absent() {
        let input = r#"---
temper-id: "019d8110-8ff3-70c2-85ae-57e04ed62885"
temper-type: task
temper-context: temper
temper-created: "2026-04-13T00:00:00Z"
title: T
slug: t
---
"#;
        let fm = Frontmatter::try_from(input).unwrap();
        assert_eq!(fm.tags(), Vec::<String>::new());
    }

    #[test]
    fn validate_returns_issues_for_missing_required() {
        // task.schema.json requires `temper-stage` and `slug`. Omit `slug`.
        let input = r#"---
temper-id: "019d8110-8ff3-70c2-85ae-57e04ed62885"
temper-type: task
temper-context: temper
temper-created: "2026-04-13T00:00:00Z"
title: T
temper-stage: in-progress
---
"#;
        let fm = Frontmatter::try_from(input).unwrap();
        let issues = fm.validate().unwrap();
        assert!(!issues.is_empty(), "expected validation issues, got none");
    }
```

- [ ] **Step 10: Run tests — they fail**

Expected: compilation errors for `serialize`, `write_to`, `parse_file`, `tags`, `validate`. And `tempfile` needs to be imported in tests.

- [ ] **Step 11: Implement `serialize`, `write_to`, `parse_file`, `validate`, `tags`**

Add these methods to `impl Frontmatter`:

```rust
    /// Serialize to the canonical on-disk form: `---\n<yaml>---\n<body>`.
    ///
    /// Display ordering is [`crate::frontmatter::canonical::canonicalize`].
    /// The body is re-emitted byte-for-byte.
    pub fn serialize(&self) -> Result<String> {
        let canonical = crate::frontmatter::canonical::canonicalize(&self.value, self.doc_type);
        let yaml_text = serde_yaml::to_string(&canonical).map_err(|e| {
            TemperError::Config(format!("failed to serialize frontmatter: {e}"))
        })?;
        let mut yaml_normalized = yaml_text.trim_end_matches('\n').to_string();
        yaml_normalized.push('\n');
        Ok(format!("---\n{yaml_normalized}---\n{body}", body = self.body))
    }

    /// Atomically write this frontmatter to `path` in canonical form.
    pub fn write_to(&self, path: &std::path::Path) -> Result<()> {
        let content = self.serialize()?;
        write_atomic(path, &content)
    }

    /// Parse a vault file from disk.
    pub fn parse_file(path: &std::path::Path) -> Result<Self> {
        let content = std::fs::read_to_string(path).map_err(|e| {
            TemperError::Config(format!("failed to read {}: {e}", path.display()))
        })?;
        Self::try_from(content.as_str())
    }

    /// Schema-validate this frontmatter against its doc-type schema.
    /// Accepts `temper-provisional-id` in place of `temper-id`.
    pub fn validate(&self) -> Result<Vec<crate::schema::ValidationIssue>> {
        crate::schema::validate_allowing_provisional(self.doc_type.as_str(), &self.value)
    }

    /// Return the `tags` open-meta vector, or an empty vec if absent.
    pub fn tags(&self) -> Vec<String> {
        let mapping = match self.value.as_mapping() {
            Some(m) => m,
            None => return Vec::new(),
        };
        mapping
            .get(serde_yaml::Value::String("tags".to_string()))
            .and_then(|v| v.as_sequence())
            .map(|seq| {
                seq.iter()
                    .filter_map(|e| e.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default()
    }
```

Add a private `write_atomic` helper at the module bottom (outside `impl`):

```rust
fn write_atomic(path: &std::path::Path, content: &str) -> Result<()> {
    let parent = path.parent().ok_or_else(|| {
        TemperError::Config(format!("path has no parent directory: {}", path.display()))
    })?;
    let file_name = path
        .file_name()
        .and_then(|s| s.to_str())
        .ok_or_else(|| TemperError::Config(format!("invalid file name: {}", path.display())))?;
    let tmp_path = parent.join(format!(".{file_name}.frontmatter.tmp"));

    std::fs::write(&tmp_path, content).map_err(|e| {
        TemperError::Config(format!("failed to write {}: {e}", tmp_path.display()))
    })?;
    std::fs::rename(&tmp_path, path).map_err(|e| {
        TemperError::Config(format!(
            "failed to rename {} -> {}: {e}",
            tmp_path.display(),
            path.display()
        ))
    })?;
    Ok(())
}
```

Also make sure the test module imports `tempfile`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    // tempfile is already a dev-dependency of temper-core per Cargo.toml
    // ...
}
```

- [ ] **Step 12: Run tests — they pass**

Run:
```bash
cargo nextest run -p temper-core frontmatter::document
```
Expected: all document tests pass.

- [ ] **Step 13: Add batch-4 tests (mutation methods)**

Append to the test module:

```rust
    #[test]
    fn set_managed_field_inserts_new_key() {
        let mut fm = Frontmatter::try_from(TASK_FIXTURE).unwrap();
        fm.set_managed_field("temper-seq", serde_json::json!(42));
        let m = fm.managed_json();
        assert_eq!(m["temper-seq"], serde_json::json!(42));
    }

    #[test]
    fn set_managed_field_overwrites_existing() {
        let mut fm = Frontmatter::try_from(TASK_FIXTURE).unwrap();
        fm.set_managed_field("temper-stage", serde_json::json!("done"));
        let m = fm.managed_json();
        assert_eq!(m["temper-stage"], serde_json::json!("done"));
    }

    #[test]
    fn set_open_field_inserts_at_top_level() {
        let mut fm = Frontmatter::try_from(TASK_FIXTURE).unwrap();
        fm.set_open_field("custom_key", serde_json::json!("v"));
        let o = fm.open_json();
        assert_eq!(o["custom_key"], serde_json::json!("v"));
    }

    #[test]
    fn remove_field_deletes_key() {
        let mut fm = Frontmatter::try_from(TASK_FIXTURE).unwrap();
        fm.remove_field("tags");
        let o = fm.open_json();
        assert!(o.get("tags").is_none());
    }

    #[test]
    fn set_relationships_replaces_canonical_relationship_keys() {
        use crate::types::graph::ResourceRelationships;

        let mut fm = Frontmatter::try_from(TASK_FIXTURE).unwrap();
        let mut rels = ResourceRelationships::default();
        rels.relates_to = vec!["new-one".to_string(), "new-two".to_string()];
        rels.depends_on = vec!["dep".to_string()];
        fm.set_relationships(&rels);

        let o = fm.open_json();
        assert_eq!(o["relates_to"], serde_json::json!(["new-one", "new-two"]));
        assert_eq!(o["depends_on"], serde_json::json!(["dep"]));
    }

    #[test]
    fn set_relationships_removes_empty_fields() {
        use crate::types::graph::ResourceRelationships;

        let mut fm = Frontmatter::try_from(TASK_FIXTURE).unwrap();
        // Input fixture has relates_to: [other-task]. Clear it via empty rels.
        let rels = ResourceRelationships::default();
        fm.set_relationships(&rels);

        let o = fm.open_json();
        assert!(o.get("relates_to").is_none(),
            "empty relates_to must remove the key");
    }

    #[test]
    fn set_relationships_does_not_touch_tags() {
        use crate::types::graph::ResourceRelationships;

        let mut fm = Frontmatter::try_from(TASK_FIXTURE).unwrap();
        let before_tags = fm.tags();
        let rels = ResourceRelationships::default();
        fm.set_relationships(&rels);
        assert_eq!(fm.tags(), before_tags,
            "tags are metadata, set_relationships must not touch them");
    }
```

- [ ] **Step 14: Run tests — they fail**

Expected: compilation errors for `set_managed_field`, `set_open_field`, `remove_field`, `set_relationships`.

- [ ] **Step 15: Implement mutation methods**

Add to `impl Frontmatter`:

```rust
    /// Insert or overwrite a managed-tier field at the top level.
    pub fn set_managed_field(&mut self, key: &str, value: serde_json::Value) {
        self.set_raw_field(key, value);
    }

    /// Insert or overwrite an open-tier field at the top level.
    pub fn set_open_field(&mut self, key: &str, value: serde_json::Value) {
        self.set_raw_field(key, value);
    }

    /// Remove a top-level field by canonical name.
    pub fn remove_field(&mut self, key: &str) {
        if let Some(mapping) = self.value.as_mapping_mut() {
            mapping.remove(serde_yaml::Value::String(key.to_string()));
        }
    }

    /// Replace canonical relationship keys from a typed
    /// [`crate::types::graph::ResourceRelationships`]. Empty fields
    /// result in key removal. **Does not touch `tags`** — tags are
    /// metadata, not a relationship.
    pub fn set_relationships(&mut self, rels: &crate::types::graph::ResourceRelationships) {
        use serde_json::json;

        let list_pairs: &[(&str, &[String])] = &[
            ("relates_to", &rels.relates_to),
            ("depends_on", &rels.depends_on),
            ("extends", &rels.extends),
            ("references", &rels.references),
            ("preceded_by", &rels.preceded_by),
            ("derived_from", &rels.derived_from),
        ];

        for (key, values) in list_pairs {
            if values.is_empty() {
                self.remove_field(key);
            } else {
                self.set_raw_field(key, json!(*values));
            }
        }

        match &rels.parent {
            Some(p) => self.set_raw_field("parent", json!(p)),
            None => self.remove_field("parent"),
        }
    }

    /// Shared implementation: insert or overwrite any top-level key.
    fn set_raw_field(&mut self, key: &str, value: serde_json::Value) {
        let yaml_value: serde_yaml::Value = serde_yaml::to_value(value).unwrap_or(serde_yaml::Value::Null);
        if let Some(mapping) = self.value.as_mapping_mut() {
            mapping.insert(serde_yaml::Value::String(key.to_string()), yaml_value);
        }
    }
```

- [ ] **Step 16: Run tests — they pass**

Run:
```bash
cargo nextest run -p temper-core frontmatter::document
```
Expected: all document tests (roughly 20) pass.

- [ ] **Step 17: Commit**

```bash
git add crates/temper-core/src/frontmatter/document.rs
git commit -m "feat(frontmatter): Frontmatter aggregate type with mutation + serialize + hashes"
```

---

## Task 8: `projections.rs` — `From` / `TryFrom` to typed structs

**Files:**
- Modify: `crates/temper-core/src/frontmatter/projections.rs`

**Why:** downstream consumers (tests and session 2 migrations) want ergonomic `ResourceRelationships::from(&fm)` and `ManagedMeta::try_from(&fm)` calls.

**Impls (exact):**
- `From<&Frontmatter> for ResourceRelationships` — projects `fm.open_json()` via `serde_json::from_value`. Infallible because every field is `#[serde(default)]`.
- `TryFrom<&Frontmatter> for ManagedMeta` — projects `fm.managed_json()` via `serde_json::from_value`. Fails only if types mismatch (e.g. someone manually wrote `temper-seq: "not a number"`).
- `TryFrom<&Frontmatter> for ResourceFrontmatter` — fails if required identity/title/context/created fields are missing.

**Session-1 caveat:** `ResourceRelationships` still has a `tags` field in Session 1 (removal is session 2). Our `From` impl must not assume `tags` is gone. Fine — serde will happily consume any `tags` array present in `open_json`.

- [ ] **Step 1: Write failing tests**

Replace `projections.rs`:

```rust
//! Trait impls projecting `Frontmatter` to the typed structs in
//! `crate::types`: `ResourceRelationships`, `ManagedMeta`, `ResourceFrontmatter`.

use crate::error::{Result, TemperError};
use crate::frontmatter::document::Frontmatter;
use crate::types::graph::ResourceRelationships;
use crate::types::managed_meta::ManagedMeta;
use crate::types::vault::ResourceFrontmatter;

// Implementation below.

#[cfg(test)]
mod tests {
    use super::*;

    const TASK_FIXTURE: &str = r#"---
temper-id: "019d8110-8ff3-70c2-85ae-57e04ed62885"
temper-type: task
temper-context: temper
temper-created: "2026-04-13T00:00:00Z"
title: My Task
slug: my-task
temper-stage: in-progress
temper-mode: build
temper-effort: small
temper-seq: 42
relates_to: [peer-a, peer-b]
depends_on: [dep-c]
parent: the-parent
tags: [auth, observability]
---
body
"#;

    #[test]
    fn projects_to_resource_relationships() {
        let fm = Frontmatter::try_from(TASK_FIXTURE).unwrap();
        let rels = ResourceRelationships::from(&fm);
        assert_eq!(rels.relates_to, vec!["peer-a", "peer-b"]);
        assert_eq!(rels.depends_on, vec!["dep-c"]);
        assert_eq!(rels.parent.as_deref(), Some("the-parent"));
        // tags still lives on the struct in session 1, so it should round-trip.
        assert_eq!(rels.tags, vec!["auth", "observability"]);
    }

    #[test]
    fn projects_to_managed_meta() {
        let fm = Frontmatter::try_from(TASK_FIXTURE).unwrap();
        let mm = ManagedMeta::try_from(&fm).unwrap();
        assert_eq!(mm.title.as_deref(), Some("My Task"));
        assert_eq!(mm.slug.as_deref(), Some("my-task"));
        assert_eq!(mm.stage.as_deref(), Some("in-progress"));
        assert_eq!(mm.mode.as_deref(), Some("build"));
        assert_eq!(mm.seq, Some(42));
    }

    #[test]
    fn projects_to_resource_frontmatter() {
        let fm = Frontmatter::try_from(TASK_FIXTURE).unwrap();
        let rf = ResourceFrontmatter::try_from(&fm).unwrap();
        assert_eq!(rf.title, "My Task");
        assert_eq!(rf.context, "temper");
        assert_eq!(rf.doc_type, "task");
    }

    #[test]
    fn projects_to_resource_frontmatter_fails_without_required_fields() {
        let input = r#"---
temper-id: "019d8110-8ff3-70c2-85ae-57e04ed62885"
temper-type: task
temper-context: temper
temper-created: "2026-04-13T00:00:00Z"
slug: t
---
"#;
        // Missing `title`, which ResourceFrontmatter requires.
        let fm = Frontmatter::try_from(input).unwrap();
        assert!(ResourceFrontmatter::try_from(&fm).is_err());
    }

    #[test]
    fn projection_of_empty_relationships_is_default() {
        let input = r#"---
temper-id: "019d8110-8ff3-70c2-85ae-57e04ed62885"
temper-type: task
temper-context: temper
temper-created: "2026-04-13T00:00:00Z"
title: T
slug: t
temper-stage: in-progress
---
"#;
        let fm = Frontmatter::try_from(input).unwrap();
        let rels = ResourceRelationships::from(&fm);
        assert!(rels.is_empty());
    }
}
```

- [ ] **Step 2: Run tests — they fail**

Run:
```bash
cargo nextest run -p temper-core frontmatter::projections 2>&1 | tail -20
```
Expected: compilation errors — the `From` / `TryFrom` impls don't exist.

- [ ] **Step 3: Implement the three projections**

Add above the test module in `projections.rs`:

```rust
impl From<&Frontmatter> for ResourceRelationships {
    /// Project a [`Frontmatter`] to a [`ResourceRelationships`] by
    /// deserializing the open-tier JSON. Infallible because every
    /// field on `ResourceRelationships` is `#[serde(default)]`.
    fn from(fm: &Frontmatter) -> Self {
        let open = fm.open_json();
        serde_json::from_value(open).unwrap_or_default()
    }
}

impl TryFrom<&Frontmatter> for ManagedMeta {
    type Error = TemperError;

    fn try_from(fm: &Frontmatter) -> Result<Self> {
        let managed = fm.managed_json();
        serde_json::from_value(managed).map_err(|e| {
            TemperError::Config(format!("failed to project to ManagedMeta: {e}"))
        })
    }
}

impl TryFrom<&Frontmatter> for ResourceFrontmatter {
    type Error = TemperError;

    fn try_from(fm: &Frontmatter) -> Result<Self> {
        // ResourceFrontmatter's serde mapping covers identity + top-level
        // fields but is only partially aligned with raw YAML key names.
        // We build it from the managed tier plus a few pulls from the
        // full value for context/created.
        let managed = fm.managed_json();
        let title = managed
            .get("title")
            .and_then(|v| v.as_str())
            .ok_or_else(|| TemperError::Config("ResourceFrontmatter requires `title`".to_string()))?
            .to_string();
        let doc_type = fm.doc_type().as_str().to_string();

        // Context and created live in the top-level value (tier-1 system fields
        // don't make it into the managed tier).
        let mapping = fm
            .value()
            .as_mapping()
            .ok_or_else(|| TemperError::Config("frontmatter is not a mapping".to_string()))?;
        let context = mapping
            .get(serde_yaml::Value::String("temper-context".into()))
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                TemperError::Config("ResourceFrontmatter requires `temper-context`".to_string())
            })?
            .to_string();
        let created_str = mapping
            .get(serde_yaml::Value::String("temper-created".into()))
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                TemperError::Config("ResourceFrontmatter requires `temper-created`".to_string())
            })?;
        let created = chrono::DateTime::parse_from_rfc3339(created_str)
            .map_err(|e| TemperError::Config(format!("invalid temper-created: {e}")))?
            .with_timezone(&chrono::Utc);
        let temper_id_str = mapping
            .get(serde_yaml::Value::String("temper-id".into()))
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                TemperError::Config("ResourceFrontmatter requires `temper-id`".to_string())
            })?;
        let temper_id = uuid::Uuid::parse_str(temper_id_str)
            .map_err(|e| TemperError::Config(format!("invalid temper-id uuid: {e}")))?;

        Ok(ResourceFrontmatter {
            temper_id,
            title,
            context,
            doc_type,
            ingestion_source: mapping
                .get(serde_yaml::Value::String("temper-source".into()))
                .and_then(|v| v.as_str())
                .map(String::from),
            created,
        })
    }
}
```

- [ ] **Step 4: Run tests — they pass**

Run:
```bash
cargo nextest run -p temper-core frontmatter::projections
```
Expected: 5 tests pass.

- [ ] **Step 5: Commit**

```bash
git add crates/temper-core/src/frontmatter/projections.rs
git commit -m "feat(frontmatter): projection impls to ResourceRelationships/ManagedMeta/ResourceFrontmatter"
```

---

## Task 9: Integration tests — fixtures + golden files + regression anchor

**Files:**
- Create: `crates/temper-core/tests/frontmatter_test.rs`
- Create: `crates/temper-core/tests/fixtures/frontmatter/task_minimal.md`
- Create: `crates/temper-core/tests/fixtures/frontmatter/task_full.md`
- Create: `crates/temper-core/tests/fixtures/frontmatter/task_with_aliases.md`
- Create: `crates/temper-core/tests/fixtures/frontmatter/goal_full.md`
- Create: `crates/temper-core/tests/fixtures/frontmatter/session_full.md`
- Create: `crates/temper-core/tests/fixtures/frontmatter/research_full.md`
- Create: `crates/temper-core/tests/fixtures/frontmatter/decision_full.md`
- Create: `crates/temper-core/tests/fixtures/frontmatter/concept_full.md`
- Create: `crates/temper-core/tests/fixtures/frontmatter/malformed_yaml.md`
- Create: `crates/temper-core/tests/fixtures/frontmatter/wrong_doc_type.md`
- Create: `crates/temper-core/tests/fixtures/frontmatter/missing_required.md`
- Create: `crates/temper-core/tests/fixtures/frontmatter/tags_as_strings.md`
- Create: `crates/temper-core/tests/fixtures/frontmatter/golden/task_minimal.canonical.md`
- Create: `crates/temper-core/tests/fixtures/frontmatter/golden/task_full.canonical.md`
- Create: `crates/temper-core/tests/fixtures/frontmatter/golden/task_with_aliases.canonical.md`
- Create: `crates/temper-core/tests/fixtures/frontmatter/golden/goal_full.canonical.md`
- Create: `crates/temper-core/tests/fixtures/frontmatter/golden/session_full.canonical.md`
- Create: `crates/temper-core/tests/fixtures/frontmatter/golden/research_full.canonical.md`
- Create: `crates/temper-core/tests/fixtures/frontmatter/golden/decision_full.canonical.md`
- Create: `crates/temper-core/tests/fixtures/frontmatter/golden/concept_full.canonical.md`

**Why:** The integration test suite enforces round-trip stability across every doctype, hash parity with the legacy path, alias/hash symmetry, and error handling. Golden files lock canonical output so future changes to `canonical.rs` are visible in the diff.

### Golden-file discipline

The golden files are generated by the integration test itself on a one-time run, then committed. This avoids hand-authoring canonical output (tedious, error-prone) while still treating goldens as review-able source artifacts. The test file has a `REGENERATE_GOLDENS` env var: when set to `1`, it writes the current serializer output into the golden files instead of asserting against them. The engineer runs it once with `REGENERATE_GOLDENS=1`, inspects the resulting files (sanity check), and then commits them.

- [ ] **Step 1: Create the input fixtures**

Write `crates/temper-core/tests/fixtures/frontmatter/task_minimal.md`:

```markdown
---
temper-id: "019d8110-8ff3-70c2-85ae-57e04ed62885"
temper-type: task
temper-context: temper
temper-created: "2026-04-13T00:00:00Z"
title: Minimal Task
slug: minimal-task
temper-stage: in-progress
---
minimal body
```

Write `crates/temper-core/tests/fixtures/frontmatter/task_full.md`:

```markdown
---
temper-id: "019d8110-8ff3-70c2-85ae-57e04ed62885"
temper-type: task
temper-context: temper
temper-created: "2026-04-13T00:00:00Z"
temper-updated: "2026-04-13T12:00:00Z"
temper-owner: "@me"
title: Full Task
slug: full-task
temper-stage: in-progress
temper-mode: build
temper-effort: small
temper-goal: some-goal
temper-seq: 7
temper-branch: jct/full-task
temper-pr: "https://github.com/example/repo/pull/1"
relates_to: [peer-a, peer-b]
depends_on: [dep-c]
extends:
  - ancestor
references: [ref-x]
preceded_by: [before-me]
derived_from: [origin-doc]
parent: the-parent
tags: [auth, observability]
aliases: [alt-name]
custom_open_field: extra-value
---
Full task body content.

Multiple paragraphs.
```

Write `crates/temper-core/tests/fixtures/frontmatter/task_with_aliases.md`:

```markdown
---
temper-id: "019d8110-8ff3-70c2-85ae-57e04ed62885"
temper-type: task
temper-context: temper
temper-created: "2026-04-13T00:00:00Z"
title: Aliased Task
slug: aliased-task
temper-stage: in-progress
relates-to: [a]
depends-on: [b]
preceded-by: [c]
derived-from: [d]
---
body
```

Write `crates/temper-core/tests/fixtures/frontmatter/goal_full.md`:

```markdown
---
temper-id: "019d8110-8ff3-70c2-85ae-57e04ed62886"
temper-type: goal
temper-context: temper
temper-created: "2026-04-13T00:00:00Z"
title: Full Goal
slug: full-goal
temper-status: active
temper-seq: 1
relates_to: [other-goal]
---
goal body
```

Write `crates/temper-core/tests/fixtures/frontmatter/session_full.md`:

```markdown
---
temper-id: "019d8110-8ff3-70c2-85ae-57e04ed62887"
temper-type: session
temper-context: temper
temper-created: "2026-04-13T00:00:00Z"
title: Full Session
slug: full-session
date: "2026-04-13"
relates_to: [task-a]
tags: [retro]
---
session body
```

Write `crates/temper-core/tests/fixtures/frontmatter/research_full.md`:

```markdown
---
temper-id: "019d8110-8ff3-70c2-85ae-57e04ed62888"
temper-type: research
temper-context: temper
temper-created: "2026-04-13T00:00:00Z"
title: Full Research
slug: full-research
date: "2026-04-13"
references: [external-paper]
---
research body
```

Write `crates/temper-core/tests/fixtures/frontmatter/decision_full.md`:

```markdown
---
temper-id: "019d8110-8ff3-70c2-85ae-57e04ed62889"
temper-type: decision
temper-context: temper
temper-created: "2026-04-13T00:00:00Z"
title: Full Decision
slug: full-decision
extends: [parent-decision]
---
decision body
```

Write `crates/temper-core/tests/fixtures/frontmatter/concept_full.md`:

```markdown
---
temper-id: "019d8110-8ff3-70c2-85ae-57e04ed6288a"
temper-type: concept
temper-context: temper
temper-created: "2026-04-13T00:00:00Z"
title: Full Concept
slug: full-concept
tags: [core]
---
concept body
```

Write `crates/temper-core/tests/fixtures/frontmatter/malformed_yaml.md`:

```markdown
---
temper-id: "019d8110-8ff3-70c2-85ae-57e04ed62885"
temper-type: task
title: "Unterminated string
---
body
```

Write `crates/temper-core/tests/fixtures/frontmatter/wrong_doc_type.md`:

```markdown
---
temper-id: "019d8110-8ff3-70c2-85ae-57e04ed62885"
temper-type: bogus-type
temper-context: temper
temper-created: "2026-04-13T00:00:00Z"
title: Wrong
slug: wrong
---
body
```

Write `crates/temper-core/tests/fixtures/frontmatter/missing_required.md`:

```markdown
---
temper-id: "019d8110-8ff3-70c2-85ae-57e04ed62885"
temper-type: task
temper-context: temper
temper-created: "2026-04-13T00:00:00Z"
slug: missing-title
temper-stage: in-progress
---
body
```

Write `crates/temper-core/tests/fixtures/frontmatter/tags_as_strings.md`:

```markdown
---
temper-id: "019d8110-8ff3-70c2-85ae-57e04ed62885"
temper-type: task
temper-context: temper
temper-created: "2026-04-13T00:00:00Z"
title: Tags Test
slug: tags-test
temper-stage: in-progress
tags: [auth, observability, not-a-resource]
---
body
```

- [ ] **Step 2: Create the integration test driver**

Write `crates/temper-core/tests/frontmatter_test.rs`:

```rust
//! Integration tests for `temper_core::frontmatter`.
//!
//! Covers parse + project + mutate + serialize + hash across every
//! doctype, plus alias/hash symmetry and error cases. Golden files
//! are committed; set `REGENERATE_GOLDENS=1` to overwrite them after
//! intentional serializer changes.

use std::fs;
use std::path::{Path, PathBuf};

use temper_core::frontmatter::{DocType, Frontmatter};
use temper_core::types::graph::ResourceRelationships;

fn fixtures_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/frontmatter")
}

fn load_fixture(name: &str) -> String {
    let path = fixtures_dir().join(name);
    fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("failed to read fixture {}: {e}", path.display()))
}

fn golden_path(stem: &str) -> PathBuf {
    fixtures_dir().join("golden").join(format!("{stem}.canonical.md"))
}

fn assert_golden_matches(stem: &str, actual: &str) {
    let path = golden_path(stem);
    if std::env::var("REGENERATE_GOLDENS").as_deref() == Ok("1") {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("create golden dir");
        }
        fs::write(&path, actual).expect("write golden");
        return;
    }
    let expected = fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("failed to read golden {}: {e} — run with REGENERATE_GOLDENS=1 to create it", path.display()));
    assert_eq!(actual, expected, "golden mismatch for {stem}");
}

/// All round-trippable fixtures — `(stem, doctype)`. Excludes the
/// error-case fixtures (malformed, wrong type, missing required) and
/// the alias fixture (tested separately to check normalization).
const ROUND_TRIP_CASES: &[(&str, DocType)] = &[
    ("task_minimal", DocType::Task),
    ("task_full", DocType::Task),
    ("task_with_aliases", DocType::Task),
    ("goal_full", DocType::Goal),
    ("session_full", DocType::Session),
    ("research_full", DocType::Research),
    ("decision_full", DocType::Decision),
    ("concept_full", DocType::Concept),
];

#[test]
fn every_fixture_parses_and_matches_its_golden() {
    for (stem, expected_doctype) in ROUND_TRIP_CASES {
        let content = load_fixture(&format!("{stem}.md"));
        let fm = Frontmatter::try_from(content.as_str())
            .unwrap_or_else(|e| panic!("parse failed for {stem}: {e}"));
        assert_eq!(fm.doc_type(), *expected_doctype, "doctype mismatch for {stem}");
        let serialized = fm.serialize()
            .unwrap_or_else(|e| panic!("serialize failed for {stem}: {e}"));
        assert_golden_matches(stem, &serialized);
    }
}

#[test]
fn golden_is_a_fixed_point_of_parse_serialize() {
    // Re-reading the golden and re-serializing must produce byte-identical
    // output. Locks the "canonical form is a fixed point" property.
    if std::env::var("REGENERATE_GOLDENS").as_deref() == Ok("1") {
        return; // skip during regeneration
    }
    for (stem, _) in ROUND_TRIP_CASES {
        let golden = fs::read_to_string(golden_path(stem))
            .unwrap_or_else(|e| panic!("read golden {stem}: {e}"));
        let fm = Frontmatter::try_from(golden.as_str())
            .unwrap_or_else(|e| panic!("re-parse golden {stem}: {e}"));
        let re_serialized = fm.serialize()
            .unwrap_or_else(|e| panic!("re-serialize golden {stem}: {e}"));
        assert_eq!(re_serialized, golden, "fixed-point failed for {stem}");
    }
}

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

#[test]
fn alias_form_hashes_match_canonical_form() {
    let alias = Frontmatter::try_from(load_fixture("task_with_aliases.md").as_str()).unwrap();
    // Construct a canonical-form equivalent.
    let canonical = r#"---
temper-id: "019d8110-8ff3-70c2-85ae-57e04ed62885"
temper-type: task
temper-context: temper
temper-created: "2026-04-13T00:00:00Z"
title: Aliased Task
slug: aliased-task
temper-stage: in-progress
relates_to: [a]
depends_on: [b]
preceded_by: [c]
derived_from: [d]
---
body
"#;
    let c = Frontmatter::try_from(canonical).unwrap();
    assert_eq!(alias.hashes(), c.hashes(),
        "alias-form and canonical-form must hash identically");
}

#[test]
fn display_ordering_has_zero_effect_on_hashes() {
    // Three permutations of the same task frontmatter → identical hashes.
    let a = r#"---
temper-id: "019d8110-8ff3-70c2-85ae-57e04ed62885"
temper-type: task
temper-context: temper
temper-created: "2026-04-13T00:00:00Z"
title: T
slug: t
temper-stage: in-progress
relates_to: [x]
---
"#;
    let b = r#"---
slug: t
title: T
temper-stage: in-progress
relates_to: [x]
temper-created: "2026-04-13T00:00:00Z"
temper-context: temper
temper-type: task
temper-id: "019d8110-8ff3-70c2-85ae-57e04ed62885"
---
"#;
    let c = r#"---
relates_to: [x]
temper-stage: in-progress
temper-created: "2026-04-13T00:00:00Z"
temper-id: "019d8110-8ff3-70c2-85ae-57e04ed62885"
title: T
temper-context: temper
slug: t
temper-type: task
---
"#;
    let h_a = Frontmatter::try_from(a).unwrap().hashes();
    let h_b = Frontmatter::try_from(b).unwrap().hashes();
    let h_c = Frontmatter::try_from(c).unwrap().hashes();
    assert_eq!(h_a, h_b);
    assert_eq!(h_b, h_c);
}

#[test]
fn mutate_then_write_round_trips_through_parse() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("task.md");

    let mut fm = Frontmatter::try_from(load_fixture("task_full.md").as_str()).unwrap();
    let mut new_rels = ResourceRelationships::default();
    new_rels.relates_to = vec!["brand-new".to_string()];
    fm.set_relationships(&new_rels);

    fm.write_to(&path).unwrap();
    let re = Frontmatter::parse_file(&path).unwrap();
    let re_rels = ResourceRelationships::from(&re);
    assert_eq!(re_rels.relates_to, vec!["brand-new"]);
    assert!(re_rels.depends_on.is_empty(),
        "depends_on should have been cleared by set_relationships");
}

#[test]
fn malformed_yaml_errors() {
    let content = load_fixture("malformed_yaml.md");
    assert!(Frontmatter::try_from(content.as_str()).is_err());
}

#[test]
fn wrong_doc_type_errors() {
    let content = load_fixture("wrong_doc_type.md");
    assert!(Frontmatter::try_from(content.as_str()).is_err());
}

#[test]
fn missing_required_parses_but_fails_validation() {
    // Parsing succeeds because the file structurally valid — validation
    // surfaces the missing `title` field.
    let content = load_fixture("missing_required.md");
    let fm = Frontmatter::try_from(content.as_str()).unwrap();
    let issues = fm.validate().unwrap();
    assert!(!issues.is_empty(), "missing required fields should produce issues");
}

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
    // tags itself lives on the struct for session 1 — verify it got there.
    assert_eq!(rels.tags, vec!["auth", "observability", "not-a-resource"]);

    // And the accessor returns the same thing.
    assert_eq!(fm.tags(), vec!["auth".to_string(), "observability".to_string(), "not-a-resource".to_string()]);
}
```

Note: `temper_core::frontmatter::parse` is a `pub` module, so the integration test can import its helpers for the hash-drift regression test.

- [ ] **Step 3: First run — regenerate goldens**

Run:
```bash
REGENERATE_GOLDENS=1 cargo nextest run -p temper-core --test frontmatter_test
```
Expected: all tests pass (the golden-matching tests skip or write goldens instead of asserting).

- [ ] **Step 4: Inspect the generated golden files**

Run:
```bash
ls crates/temper-core/tests/fixtures/frontmatter/golden/
```
Expected: eight `*.canonical.md` files corresponding to the `ROUND_TRIP_CASES` entries.

Eyeball each one briefly — the keys should appear in the canonical 5-tier order. If any look wrong, fix the serializer/ordering bug and regenerate. **Do not commit until you've confirmed the output is actually canonical.**

- [ ] **Step 5: Second run — goldens now assert**

Run:
```bash
cargo nextest run -p temper-core --test frontmatter_test
```
Expected: all integration tests pass, including `every_fixture_parses_and_matches_its_golden` and `golden_is_a_fixed_point_of_parse_serialize`.

- [ ] **Step 6: Commit fixtures, goldens, and integration tests**

```bash
git add crates/temper-core/tests/frontmatter_test.rs crates/temper-core/tests/fixtures/frontmatter
git commit -m "feat(frontmatter): integration tests with synthetic fixtures + goldens"
```

---

## Task 10: Full gates + manual smoke check + PR

**Files:** none modified in this task.

**Why:** Session-1 acceptance criteria require `cargo make check` clean, the full Rust suite green including integration tests, and a manual sanity run of `target/debug/temper doctor` against the real vault before PR.

- [ ] **Step 1: Run `cargo make check`**

Run:
```bash
cargo make check
```
Expected: fmt, clippy `-D warnings`, docs, machete, TS typecheck, biome all clean.

If clippy complains, fix it in place (no `#[allow]` unless truly justified). Most likely issues: unused imports in stub modules, missing doc comments on public types.

- [ ] **Step 2: Run the full Rust unit test suite**

Run:
```bash
cargo nextest run --workspace
```
Expected: all tests pass — including pre-existing tests in `hash.rs`, `normalize.rs`, `schema.rs`, and the new `frontmatter::*` modules.

- [ ] **Step 3: Run the DB-gated integration tests**

Run (requires Docker Postgres on port 5437):
```bash
cargo make docker-up && cargo make test-db
```
Expected: integration suite passes. Session 1 added no new DB-dependent tests, so this is a regression check.

- [ ] **Step 4: Run the e2e suite**

Run:
```bash
cargo nextest run -p temper-e2e --features test-db
```
Expected: all e2e tests pass unchanged. Session 1 did not touch sync paths, so Phase E2's full suite should remain green.

- [ ] **Step 5: Manual smoke check — `temper doctor` against the real vault**

Build the CLI and run `temper doctor` in read-only mode against the production vault:

```bash
cargo build -p temper-cli
./target/debug/temper doctor
```
Expected: identical output to what the current `main`-branch `temper doctor` produces against the same vault. The new module is unconsumed, so behavior should be unchanged.

If `temper doctor` reports any new diffs or errors, investigate — it likely means an unnoticed test module drop-in or a clippy auto-fix touched something it shouldn't have.

- [ ] **Step 6: Regenerate the SQL cache if any SQL touched**

Session 1 does not touch SQL. Skip this step unless `cargo sqlx prepare` would produce diffs, which it should not.

- [ ] **Step 7: Push and open the PR**

```bash
git push -u origin jct/frontmatter-consolidation
gh pr create --title "feat(frontmatter): temper-core::frontmatter module (session 1, additive)" \
  --body "$(cat <<'EOF'
## Summary

Session 1 of the frontmatter consolidation work (spec: `docs/superpowers/specs/2026-04-13-temper-core-frontmatter-consolidation-design.md`). Adds a new `temper-core::frontmatter` module with full unit and integration test coverage. Zero behavior change to existing code paths — nothing in production consumes the new module yet; that's sessions 2 and 3.

What landed:
- New `temper-core::frontmatter` module with `Frontmatter` aggregate type, `DocType` enum, `KNOWN_OPEN_FIELDS` registry, parse + tier-split + canonical display ordering + projections + mutation + serialize + write_to + hashes.
- Explicit tier-split routing via the registry (no longer relies on `$ref` not being followed).
- Unit tests inline per file; integration tests with synthetic fixtures and golden outputs for every doctype.
- Regression anchors: hashes match the legacy `hash::compute_frontmatter_hashes_from_yaml` path byte-for-byte per doctype; alias-form and canonical-form hash identically; display ordering has zero effect on hash output.

## Test plan

- [x] `cargo make check` clean (fmt, clippy, docs, machete, TS typecheck, biome)
- [x] `cargo nextest run --workspace` green
- [x] `cargo nextest run -p temper-e2e --features test-db` green
- [x] Manual `target/debug/temper doctor` against `/Users/petetaylor/projects/kb-vault` produces identical output to main
- [x] Golden files reviewed for canonical order across all six doctypes

## What's NOT in this PR

- Session 2 (migrate sync + normalize, fix tags phantom edges, delete `hash::split_frontmatter_tiers`)
- Session 3 (retire remaining APIs, add `temper doctor --fix aliases`)
EOF
)"
```
Expected: PR opens cleanly. CI should pass — fmt, clippy, TS checks, test-rust, test-typescript, Phase E2 e2e.

- [ ] **Step 8: Await CI green**

Wait for CI to report green on all jobs. If anything fails, fix in place and push a new commit.

---

## Self-review checklist (run before dispatch)

- **Spec coverage:** every item in the spec's "Session 1 — Foundation" subsection maps to a concrete task here (module creation ✓, `DocType` ✓, registry ✓, TryFrom + projections + mutation + serialize + write_to + hashes ✓, alias normalization ✓, unit tests per file ✓, integration tests + fixtures + goldens ✓, `cargo make check` + full suite + manual `temper doctor` ✓).
- **Hash-stability anchor:** Task 9's `hashes_are_byte_identical_to_legacy_path_per_doctype` and Task 5's `matches_legacy_split_*` together lock in that the new module produces bit-identical hashes for every doctype we have fixtures for. Alias/canonical parity is separately locked by `alias_form_hashes_match_canonical_form`. Display-ordering independence is locked by `display_ordering_has_zero_effect_on_hashes`.
- **Placeholder scan:** no "TODO", "implement later", "add error handling" — every step has concrete code or exact commands.
- **Type-name consistency:** `Frontmatter`, `DocType`, `KnownOpenField`, `OpenFieldType`, `FieldCategory`, `split_managed_open`, `canonicalize`, `normalize_aliases`, `split_frontmatter_block`, `parse_yaml` are used consistently across tasks.
- **Additive-only discipline:** no production call sites in `temper-cli` or `temper-api` are modified. No existing public APIs are deleted. Re-export pattern for `fields.rs` preserves all external imports of `IDENTITY_FIELDS` / `TIER1_SYSTEM_FIELDS` / `SYSTEM_MANAGED_FIELDS`.
- **Pre-existing test non-regression:** `schema_test.rs` (which calls `hash::split_frontmatter_tiers` directly) is untouched. The existing `hash.rs` and `normalize.rs` tests are untouched. Session 1 only adds; sessions 2 and 3 remove.

---

## Execution Handoff

**Plan complete and saved to `docs/superpowers/plans/2026-04-13-temper-core-frontmatter-module-session-1.md`. Two execution options:**

**1. Subagent-Driven (recommended)** — I dispatch a fresh subagent per task, review between tasks, fast iteration.

**2. Inline Execution** — Execute tasks in this session using executing-plans, batch execution with checkpoints.

**Which approach?**
