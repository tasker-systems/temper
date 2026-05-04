# Schema-Driven Managed-Meta — Phase 1: Schema Contract Foundation Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

> **Execution status (updated mid-flight, 2026-05-04):**
> - Tasks 1, 2, 3 — landed cleanly (commits `e10dc9a`, `6b92071`, `1dd48b2`).
> - Task 4 — landed in commit `add5b63` but over-scoped: the same commit also did **Task 7 (base.schema.json title rename)** and the **title-half of spec Phase 3 (canonical.rs)**. All test fixtures (input + golden + hash table) and the `ResourceFrontmatter::try_from` consumer were migrated forward to the canonical `temper-title` form. **Skip Task 7 when you reach it** — it's already done. Phase 3's slug-half remains for the separate Phase 3 plan.
> - Tasks 5, 6, 8-13, 14 — still to do.

**Goal:** Land the schema-and-types contract for the temper-prefix alignment. Renames `title` → `temper-title` and `slug` → `temper-slug` in the typed `ManagedMeta` struct, all 7 JSON schemas, the field-set constants, and the parse-boundary alias normalizer; drops `date` from managed-tier schemas (it becomes open_meta in Phase 2 of this plan via the doctor pass and DB migration, planned separately). After this plan lands, the contract is stated; the consumers (canonical-form rendering, server stripping, DB migration, doctor fix) get aligned in subsequent phase plans.

**Architecture:** Pure type-and-schema work. No DB migrations, no server-side SQL changes, no template edits. The transition window is bridged by extending the existing parse-boundary alias normalizer (`crates/temper-core/src/frontmatter/parse.rs::normalize_aliases`) so files emitted under the old contract keep parsing correctly. Test coverage is unit-level only — DB-backed and e2e regressions belong to subsequent phases that touch persistence.

**Tech Stack:** Rust 2021, serde, serde_yaml, JSON Schema 2020-12.

**Specs:**
- `docs/superpowers/specs/2026-05-03-schema-driven-managed-meta-design.md` — combines spec Phases 1 + 2 (the spec flagged these as likely consolidation candidates).
- Upstream backbone: `docs/superpowers/specs/2026-05-01-shared-core-execution-paths-design.md`.

**Predecessor state:** `crates/temper-core/src/types/managed_meta.rs::ManagedMeta` has `title` and `slug` fields with no `serde(rename)` (lines 91-96). All 7 JSON schemas in `crates/temper-core/schemas/` declare bare `title` (in base) and bare `slug` (in task/goal/research/decision/concept). `LEGACY_FIELDS` in `crates/temper-core/src/frontmatter/fields.rs` already has 14 bare→temper-prefix entries — title and slug are missing. `SYSTEM_MANAGED_FIELDS` has bare `slug` at line 86 (pre-existing inconsistency this plan resolves).

**Out of scope for this plan:**
- `crates/temper-core/src/frontmatter/canonical.rs` (lines 61-66 hardcode bare `title`/`slug`) — Phase 3 of the spec, separate plan.
- Askama templates in `crates/temper-cli/templates/` — Phase 4 of the spec, separate plan.
- Server-side stripping logic in `crates/temper-api/src/services/ingest_service.rs` — Phase 5 of the spec, blocked on locating the title/slug column-extraction SQL site (spec open question 3).
- DB migration to rewrite existing rows' managed_meta JSONB — Phase 6, blocked on Phase 5.
- `temper doctor fix` rewrite of legacy vault files — Phase 9.
- Re-enabling tier-2 in show_cache — Phase 8.

This plan ends with: the type contract is correct; existing vault files still parse; new file emissions use canonical keys; the test suite is green.

---

## File Structure

**Modified files:**

| File | Change |
|---|---|
| `crates/temper-core/src/types/managed_meta.rs` | Add `serde(rename = "temper-title"/"temper-slug")` to title/slug fields; update existing tests for new keys |
| `crates/temper-core/src/frontmatter/fields.rs` | Add `temper-title`/`temper-slug` to `KNOWN_TEMPER_FIELDS`; add `(title, temper-title)` and `(slug, temper-slug)` to `LEGACY_FIELDS`; replace bare `slug` with `temper-slug` in `SYSTEM_MANAGED_FIELDS`; update tests |
| `crates/temper-core/src/frontmatter/parse.rs` | Extend `normalize_aliases` to also rewrite managed-tier legacy keys via `LEGACY_FIELDS` (currently only rewrites open-field aliases via `KnownOpenField` lookup) |
| `crates/temper-core/schemas/base.schema.json` | Rename `title` → `temper-title` in `properties` and `required` |
| `crates/temper-core/schemas/task.schema.json` | Rename `slug` → `temper-slug` in `properties` and `required` |
| `crates/temper-core/schemas/goal.schema.json` | Rename `slug` → `temper-slug` in `properties` and `required` |
| `crates/temper-core/schemas/research.schema.json` | Rename `slug` → `temper-slug`; drop `date` from properties + required |
| `crates/temper-core/schemas/decision.schema.json` | Rename `slug` → `temper-slug`; drop `date` from properties + required |
| `crates/temper-core/schemas/concept.schema.json` | Rename `slug` → `temper-slug`; drop `date` from properties + required |
| `crates/temper-core/schemas/session.schema.json` | Drop `date` from properties + required |

**No new files.** This plan is entirely additive-or-modification within existing files.

**Conventions:**
- Tests live in `#[cfg(test)] mod tests` at the bottom of each `.rs` file (mirrors peer files).
- JSON schemas keep their existing trailing newline and 2-space indentation.
- Each task that touches a `.rs` file ends with a `cargo nextest run -p temper-core` step (per-crate, fast); the final task runs full-workspace `cargo make test` and `cargo make check`.

---

## Task 1: Add `(title, temper-title)` and `(slug, temper-slug)` to `LEGACY_FIELDS`

**Files:**
- Modify: `crates/temper-core/src/frontmatter/fields.rs`

- [ ] **Step 1: Write the failing test**

Append to the existing `tests` module at the bottom of `crates/temper-core/src/frontmatter/fields.rs` (insert before the closing `}` of `mod tests`):

```rust
#[test]
fn legacy_fields_map_title_and_slug_to_temper_prefix() {
    assert!(LEGACY_FIELDS.contains(&("title", "temper-title")));
    assert!(LEGACY_FIELDS.contains(&("slug", "temper-slug")));
}
```

- [ ] **Step 2: Run test to verify it fails**

```
cargo nextest run -p temper-core legacy_fields_map_title_and_slug_to_temper_prefix
```
Expected: FAIL — assertion fails because `LEGACY_FIELDS` does not yet contain those tuples.

- [ ] **Step 3: Add the entries to `LEGACY_FIELDS`**

In `crates/temper-core/src/frontmatter/fields.rs`, locate `pub static LEGACY_FIELDS: &[(&str, &str)] = &[` (around line 57). Append two entries before the closing `];`. Final form:

```rust
pub static LEGACY_FIELDS: &[(&str, &str)] = &[
    ("id", "temper-id"),
    ("type", "temper-type"),
    ("doc_type", "temper-type"),
    ("context", "temper-context"),
    ("project", "temper-context"),
    ("created", "temper-created"),
    ("updated", "temper-updated"),
    ("source", "temper-source"),
    ("stage", "temper-stage"),
    ("status", "temper-status"),
    ("mode", "temper-mode"),
    ("effort", "temper-effort"),
    ("goal", "temper-goal"),
    ("branch", "temper-branch"),
    ("pr", "temper-pr"),
    ("title", "temper-title"),
    ("slug", "temper-slug"),
];
```

- [ ] **Step 4: Run test to verify it passes**

```
cargo nextest run -p temper-core legacy_fields_map_title_and_slug_to_temper_prefix
```
Expected: PASS.

- [ ] **Step 5: Commit**

```
git add crates/temper-core/src/frontmatter/fields.rs
git commit -m "$(cat <<'EOF'
feat(core): add title/slug legacy aliases for temper-prefix rename

LEGACY_FIELDS now maps bare `title` and `slug` to `temper-title` and
`temper-slug`. Used by the parse-boundary alias normalizer to keep
existing vault files parsing during the transition window.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 2: Add `temper-title` and `temper-slug` to `KNOWN_TEMPER_FIELDS`

**Files:**
- Modify: `crates/temper-core/src/frontmatter/fields.rs`

- [ ] **Step 1: Write the failing test**

Append to the `tests` module at the bottom of `fields.rs`:

```rust
#[test]
fn known_temper_fields_includes_temper_title_and_temper_slug() {
    assert!(
        KNOWN_TEMPER_FIELDS.contains(&"temper-title"),
        "missing temper-title"
    );
    assert!(
        KNOWN_TEMPER_FIELDS.contains(&"temper-slug"),
        "missing temper-slug"
    );
}
```

- [ ] **Step 2: Run test to verify it fails**

```
cargo nextest run -p temper-core known_temper_fields_includes_temper_title_and_temper_slug
```
Expected: FAIL — assertion fails on first contains check.

- [ ] **Step 3: Add the entries to `KNOWN_TEMPER_FIELDS`**

In `crates/temper-core/src/frontmatter/fields.rs`, locate `pub static KNOWN_TEMPER_FIELDS: &[&str] = &[` (around line 29). Add the two entries in the "// task" or "// goal" group of the static — insert after `"temper-pr"` and before the `// goal` comment:

```rust
pub static KNOWN_TEMPER_FIELDS: &[&str] = &[
    "temper-id",
    "temper-provisional-id",
    "temper-type",
    "temper-context",
    "temper-created",
    "temper-updated",
    "temper-owner",
    "temper-source",
    // managed-tier identity (post-rename)
    "temper-title",
    "temper-slug",
    // task
    "temper-stage",
    "temper-mode",
    "temper-effort",
    "temper-goal",
    "temper-seq",
    "temper-branch",
    "temper-pr",
    // goal
    "temper-status",
    // session, research, decision, concept have no extra temper-* beyond base
    // LLM-assist managed fields
    "temper-provenance",
    "temper-llm-model",
    "temper-llm-run",
];
```

- [ ] **Step 4: Run test to verify it passes**

```
cargo nextest run -p temper-core known_temper_fields_includes_temper_title_and_temper_slug
```
Expected: PASS.

- [ ] **Step 5: Commit**

```
git add crates/temper-core/src/frontmatter/fields.rs
git commit -m "$(cat <<'EOF'
feat(core): add temper-title and temper-slug to KNOWN_TEMPER_FIELDS

KNOWN_TEMPER_FIELDS is the source of truth for temper-* field-name
typo detection in doctor scans. Adding the two managed-tier identity
fields ahead of the ManagedMeta serde rename in Task 4.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 3: Replace bare `slug` with `temper-slug` in `SYSTEM_MANAGED_FIELDS`

**Files:**
- Modify: `crates/temper-core/src/frontmatter/fields.rs`

Note: `SYSTEM_MANAGED_FIELDS` currently has `"slug"` at line 86 (a pre-existing inconsistency — every other entry is `temper-*` prefixed). This task corrects it.

- [ ] **Step 1: Write the failing test**

Append to the `tests` module at the bottom of `fields.rs`:

```rust
#[test]
fn system_managed_fields_uses_temper_slug_not_bare_slug() {
    assert!(
        SYSTEM_MANAGED_FIELDS.contains(&"temper-slug"),
        "expected temper-slug in SYSTEM_MANAGED_FIELDS"
    );
    assert!(
        !SYSTEM_MANAGED_FIELDS.contains(&"slug"),
        "bare `slug` should have been renamed to `temper-slug`"
    );
}
```

- [ ] **Step 2: Run test to verify it fails**

```
cargo nextest run -p temper-core system_managed_fields_uses_temper_slug_not_bare_slug
```
Expected: FAIL — first assertion fails because `temper-slug` is absent and `slug` is present.

- [ ] **Step 3: Update `SYSTEM_MANAGED_FIELDS`**

In `crates/temper-core/src/frontmatter/fields.rs`, locate `pub static SYSTEM_MANAGED_FIELDS: &[&str] = &[` (around line 76). Replace `"slug"` with `"temper-slug"`:

```rust
pub static SYSTEM_MANAGED_FIELDS: &[&str] = &[
    "temper-id",
    "temper-provisional-id",
    "temper-type",
    "temper-context",
    "temper-owner",
    "temper-created",
    "temper-updated",
    "temper-source",
    "temper-legacy-id",
    "temper-slug",
];
```

- [ ] **Step 4: Run test to verify it passes**

```
cargo nextest run -p temper-core system_managed_fields_uses_temper_slug_not_bare_slug
```
Expected: PASS.

- [ ] **Step 5: Verify no other consumers were depending on the bare form**

```
grep -rn 'SYSTEM_MANAGED_FIELDS' crates/ tests/
```
Expected: read each hit. If any consumer is comparing field names to literal `"slug"` (not via the constant), flag it as a code-review note before committing — do NOT modify call sites in this task. If a call site is broken by this change, the breakage will surface in Step 6.

- [ ] **Step 6: Run the full temper-core test suite**

```
cargo nextest run -p temper-core
```
Expected: ALL PASS. If anything fails, the failure points to a hidden dependency on the bare `"slug"` form — investigate and either fix it inline (if trivially a literal-string consumer that should use the constant) or report BLOCKED with the failing test name.

- [ ] **Step 7: Commit**

```
git add crates/temper-core/src/frontmatter/fields.rs
git commit -m "$(cat <<'EOF'
fix(core): rename bare slug to temper-slug in SYSTEM_MANAGED_FIELDS

SYSTEM_MANAGED_FIELDS is the gating set for "fields the user cannot
update via CLI". Every other entry was temper-* prefixed; bare `slug`
was a pre-existing inconsistency. Aligns with the temper-prefix
contract before the ManagedMeta serde rename.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 4: Add `serde(rename = "temper-title")` to `ManagedMeta.title`

**Files:**
- Modify: `crates/temper-core/src/types/managed_meta.rs`

- [ ] **Step 1: Write the failing test**

Append to the `tests` module at the bottom of `crates/temper-core/src/types/managed_meta.rs` (before the closing `}` of `mod tests`):

```rust
#[test]
fn managed_meta_serializes_title_as_temper_title_key() {
    let meta = ManagedMeta {
        title: Some("Improve sync".to_string()),
        ..Default::default()
    };
    let json = serde_json::to_string(&meta).unwrap();
    assert!(
        json.contains("\"temper-title\""),
        "expected temper-title key, got: {json}"
    );
    assert!(
        !json.contains("\"title\":"),
        "bare title key must not appear, got: {json}"
    );
}

#[test]
fn managed_meta_deserializes_temper_title_into_title_field() {
    let json = r#"{"temper-title":"Improve sync"}"#;
    let parsed: ManagedMeta = serde_json::from_str(json).unwrap();
    assert_eq!(parsed.title.as_deref(), Some("Improve sync"));
}
```

- [ ] **Step 2: Run tests to verify they fail**

```
cargo nextest run -p temper-core managed_meta_serializes_title_as_temper_title_key
cargo nextest run -p temper-core managed_meta_deserializes_temper_title_into_title_field
```
Expected: FAIL — first test fails because serialized JSON contains `"title"` not `"temper-title"`. Second fails because `temper-title` is unrecognized and lands in `extra` rather than the typed `title` field.

- [ ] **Step 3: Add the serde rename**

In `crates/temper-core/src/types/managed_meta.rs`, locate `/// Human-readable title (identity transport, no rename)` (around line 90). Replace the field block:

```rust
    /// Human-readable title. Renamed to `temper-title` per the
    /// temper-prefix contract for managed-tier keys.
    #[serde(rename = "temper-title", skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
```

- [ ] **Step 4: Run tests to verify they pass**

```
cargo nextest run -p temper-core managed_meta_serializes_title_as_temper_title_key
cargo nextest run -p temper-core managed_meta_deserializes_temper_title_into_title_field
```
Expected: PASS for both.

- [ ] **Step 5: Update existing `managed_meta_yaml_roundtrip` test**

The existing test at the bottom of `managed_meta.rs` (search for `fn managed_meta_yaml_roundtrip`) asserts `yaml.contains("title:")`. With the rename, the emitted YAML will say `temper-title:`. Update the assertion:

```rust
        // title and slug are renamed (post-temper-prefix alignment)
        assert!(yaml.contains("temper-title:"), "missing temper-title key");
        assert!(yaml.contains("temper-slug:"), "missing temper-slug key");
```

(Note: the `temper-slug` assertion is forward-looking; Task 5 makes it pass. After Task 4 the test will fail on the second assertion; that's expected because the two renames pair up. Either reorder so this is updated after Task 5, or accept the temporary failure across tasks 4-5 as part of the consolidation.)

**Decision:** keep test updates batched per-task. After Task 4, comment-out the `temper-slug:` assertion line; after Task 5, restore it. Or: leave the existing `slug:` assertion alone in this task, then update it in Task 5 alongside the slug rename. **Take the second option** — it keeps each task green standalone:

In this task (Task 4 only), update only the title assertion:

```rust
        // title is renamed to temper-title; slug update follows in Task 5
        assert!(yaml.contains("temper-title:"), "missing temper-title key");
        assert!(yaml.contains("slug:"), "missing slug key (renamed in Task 5)");
```

- [ ] **Step 6: Run the full managed_meta test module**

```
cargo nextest run -p temper-core --test-threads 1 -E 'package(temper-core) and binary(types::managed_meta)'
```

If the binary filter fails, fall back to:

```
cargo nextest run -p temper-core managed_meta
```

Expected: ALL PASS.

- [ ] **Step 7: Commit**

```
git add crates/temper-core/src/types/managed_meta.rs
git commit -m "$(cat <<'EOF'
feat(core): rename ManagedMeta.title to serialize as temper-title

Adds serde(rename = "temper-title") to ManagedMeta.title. Existing
yaml roundtrip test updated to assert the new key. Rust field name
stays `title` for ergonomic access; only the wire format changes.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 5: Add `serde(rename = "temper-slug")` to `ManagedMeta.slug`

**Files:**
- Modify: `crates/temper-core/src/types/managed_meta.rs`

- [ ] **Step 1: Write the failing test**

Append to the `tests` module at the bottom of `managed_meta.rs`:

```rust
#[test]
fn managed_meta_serializes_slug_as_temper_slug_key() {
    let meta = ManagedMeta {
        slug: Some("improve-sync".to_string()),
        ..Default::default()
    };
    let json = serde_json::to_string(&meta).unwrap();
    assert!(
        json.contains("\"temper-slug\""),
        "expected temper-slug key, got: {json}"
    );
    assert!(
        !json.contains("\"slug\":"),
        "bare slug key must not appear, got: {json}"
    );
}

#[test]
fn managed_meta_deserializes_temper_slug_into_slug_field() {
    let json = r#"{"temper-slug":"improve-sync"}"#;
    let parsed: ManagedMeta = serde_json::from_str(json).unwrap();
    assert_eq!(parsed.slug.as_deref(), Some("improve-sync"));
}
```

- [ ] **Step 2: Run tests to verify they fail**

```
cargo nextest run -p temper-core managed_meta_serializes_slug_as_temper_slug_key
cargo nextest run -p temper-core managed_meta_deserializes_temper_slug_into_slug_field
```
Expected: FAIL.

- [ ] **Step 3: Add the serde rename**

In `crates/temper-core/src/types/managed_meta.rs`, locate `/// URL-safe slug (identity transport, no rename)` (around line 94). Replace the field block:

```rust
    /// URL-safe slug. Renamed to `temper-slug` per the temper-prefix
    /// contract for managed-tier keys.
    #[serde(rename = "temper-slug", skip_serializing_if = "Option::is_none")]
    pub slug: Option<String>,
```

- [ ] **Step 4: Run tests to verify they pass**

```
cargo nextest run -p temper-core managed_meta_serializes_slug_as_temper_slug_key
cargo nextest run -p temper-core managed_meta_deserializes_temper_slug_into_slug_field
```
Expected: PASS.

- [ ] **Step 5: Restore the `temper-slug` assertion in `managed_meta_yaml_roundtrip`**

In the existing `managed_meta_yaml_roundtrip` test (the one updated in Task 4), update the slug-related lines:

```rust
        // title and slug are renamed (post-temper-prefix alignment)
        assert!(yaml.contains("temper-title:"), "missing temper-title key");
        assert!(yaml.contains("temper-slug:"), "missing temper-slug key");
```

Remove the `(renamed in Task 5)` comment.

- [ ] **Step 6: Run the full temper-core test suite**

```
cargo nextest run -p temper-core
```
Expected: ALL PASS.

- [ ] **Step 7: Commit**

```
git add crates/temper-core/src/types/managed_meta.rs
git commit -m "$(cat <<'EOF'
feat(core): rename ManagedMeta.slug to serialize as temper-slug

Adds serde(rename = "temper-slug") to ManagedMeta.slug. Together
with Task 4's temper-title rename, the typed ManagedMeta now emits
canonical temper-* keys for every managed-tier field. Rust field
names stay bare; only the wire format changes.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 6: Extend `normalize_aliases` to rewrite managed-tier legacy keys

**Files:**
- Modify: `crates/temper-core/src/frontmatter/parse.rs`

The current `normalize_aliases` (in `parse.rs`) only rewrites known open-field hyphen forms (e.g., `relates-to → relates_to`) via `KnownOpenField::lookup`. After Tasks 4-5, vault files written under the old contract have bare `title:` and `slug:` keys; the alias normalizer needs to rewrite those too so existing files keep deserializing into the typed `ManagedMeta`.

- [ ] **Step 1: Write the failing test**

Append to the `tests` module at the bottom of `crates/temper-core/src/frontmatter/parse.rs`:

```rust
#[test]
fn normalize_aliases_rewrites_managed_tier_legacy_keys() {
    let mut v: serde_yaml::Value =
        serde_yaml::from_str("title: Hello\nslug: hello\n").unwrap();
    normalize_aliases(&mut v);
    let m = v.as_mapping().unwrap();
    assert!(
        m.contains_key(serde_yaml::Value::String("temper-title".into())),
        "expected temper-title after normalization"
    );
    assert!(
        m.contains_key(serde_yaml::Value::String("temper-slug".into())),
        "expected temper-slug after normalization"
    );
    assert!(
        !m.contains_key(serde_yaml::Value::String("title".into())),
        "bare title should be removed"
    );
    assert!(
        !m.contains_key(serde_yaml::Value::String("slug".into())),
        "bare slug should be removed"
    );
}

#[test]
fn normalize_aliases_managed_tier_canonical_wins_on_collision() {
    // If both bare and temper-prefixed forms are present, temper- wins
    // and bare is dropped. Mirrors the open-field collision behavior.
    let mut v: serde_yaml::Value = serde_yaml::from_str(
        "title: legacy-value\ntemper-title: canonical-value\n",
    )
    .unwrap();
    normalize_aliases(&mut v);
    let m = v.as_mapping().unwrap();
    let title_val = m
        .get(serde_yaml::Value::String("temper-title".into()))
        .unwrap()
        .as_str()
        .unwrap();
    assert_eq!(title_val, "canonical-value");
    assert!(!m.contains_key(serde_yaml::Value::String("title".into())));
}
```

- [ ] **Step 2: Run tests to verify they fail**

```
cargo nextest run -p temper-core normalize_aliases_rewrites_managed_tier_legacy_keys
cargo nextest run -p temper-core normalize_aliases_managed_tier_canonical_wins_on_collision
```
Expected: FAIL — current `normalize_aliases` only knows about open-field aliases, so bare `title`/`slug` pass through unchanged.

- [ ] **Step 3: Extend `normalize_aliases` to consult `LEGACY_FIELDS`**

In `crates/temper-core/src/frontmatter/parse.rs`, modify the `normalize_aliases` function. Current form (around line 97):

```rust
pub fn normalize_aliases(value: &mut serde_yaml::Value) {
    let Some(mapping) = value.as_mapping_mut() else {
        return;
    };

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
        if let Some(val) = mapping.remove(&alias_key) {
            mapping.insert(canonical_key, val);
        }
    }
}
```

Replace with the extended version that consults both `KnownOpenField` and `LEGACY_FIELDS`:

```rust
pub fn normalize_aliases(value: &mut serde_yaml::Value) {
    let Some(mapping) = value.as_mapping_mut() else {
        return;
    };

    // Collect (alias_key, canonical_key) pairs first — we can't mutate
    // while iterating. Sources:
    //   1. Open-field hyphen-form aliases (e.g. relates-to → relates_to),
    //      via the open-field registry.
    //   2. Managed-tier legacy bare-key aliases (e.g. title → temper-title),
    //      via the LEGACY_FIELDS table in frontmatter::fields.
    let mut rewrites: Vec<(serde_yaml::Value, serde_yaml::Value)> = Vec::new();
    for (k, _) in mapping.iter() {
        let Some(k_str) = k.as_str() else {
            continue;
        };

        // Open-field aliases first.
        if let Some(entry) = alias_target(k_str) {
            if entry.canonical != k_str {
                rewrites.push((
                    k.clone(),
                    serde_yaml::Value::String(entry.canonical.to_string()),
                ));
                continue;
            }
        }

        // Managed-tier legacy aliases.
        if let Some(canonical) = managed_legacy_target(k_str) {
            rewrites.push((
                k.clone(),
                serde_yaml::Value::String(canonical.to_string()),
            ));
        }
    }

    for (alias_key, canonical_key) in rewrites {
        if mapping.contains_key(&canonical_key) {
            mapping.remove(&alias_key);
            continue;
        }
        if let Some(val) = mapping.remove(&alias_key) {
            mapping.insert(canonical_key, val);
        }
    }
}
```

Then add the new helper directly below the existing `alias_target` function (around line 132):

```rust
/// Look up whether `key` is a managed-tier legacy alias.
///
/// Returns the canonical `temper-*` form if `key` matches an entry in
/// `LEGACY_FIELDS`, otherwise None.
fn managed_legacy_target(key: &str) -> Option<&'static str> {
    crate::frontmatter::fields::LEGACY_FIELDS
        .iter()
        .find(|(legacy, _)| *legacy == key)
        .map(|(_, canonical)| *canonical)
}
```

- [ ] **Step 4: Run tests to verify they pass**

```
cargo nextest run -p temper-core normalize_aliases_rewrites_managed_tier_legacy_keys
cargo nextest run -p temper-core normalize_aliases_managed_tier_canonical_wins_on_collision
```
Expected: PASS for both.

- [ ] **Step 5: Run the full parse.rs test module to verify no regressions**

```
cargo nextest run -p temper-core normalize_aliases
```
Expected: ALL PASS — including the existing `normalize_aliases_rewrites_hyphen_form_keys`, `_preserves_values`, `_is_idempotent`, `_ignores_unknown_hyphen_keys`, `_collision_prefers_canonical_form` (open-field cases must continue to work).

- [ ] **Step 6: Commit**

```
git add crates/temper-core/src/frontmatter/parse.rs
git commit -m "$(cat <<'EOF'
feat(core): extend normalize_aliases to managed-tier legacy keys

normalize_aliases now consults LEGACY_FIELDS in addition to the
open-field registry. Vault files written before the temper-prefix
rename — with bare `title:` and `slug:` keys — keep parsing into
the typed ManagedMeta during the transition window.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 7: Update `base.schema.json` — rename `title` to `temper-title`

**Files:**
- Modify: `crates/temper-core/schemas/base.schema.json`

- [ ] **Step 1: Read the current schema state**

```
cat crates/temper-core/schemas/base.schema.json
```

Verify: `properties` block contains `"title": {...}` and `required` array contains `"title"`. Other temper-prefixed keys are already correct.

- [ ] **Step 2: Edit the schema**

Open `crates/temper-core/schemas/base.schema.json`. Make two edits:

(a) In the `properties` block, find:
```json
    "title": {
      "type": "string",
      "minLength": 1,
      "description": "Display name"
    },
```
Replace the key `"title"` with `"temper-title"`. Result:
```json
    "temper-title": {
      "type": "string",
      "minLength": 1,
      "description": "Display name"
    },
```

(b) In the `required` array (around line 107), find:
```json
  "required": ["temper-id", "temper-type", "temper-context", "temper-created", "title"],
```
Replace `"title"` with `"temper-title"`. Result:
```json
  "required": ["temper-id", "temper-type", "temper-context", "temper-created", "temper-title"],
```

- [ ] **Step 3: Validate schema is still valid JSON**

```
python3 -c "import json; json.load(open('crates/temper-core/schemas/base.schema.json'))"
```
Expected: no output (valid JSON). If python3 is unavailable, substitute `cat crates/temper-core/schemas/base.schema.json | jq .` and confirm no parse error.

- [ ] **Step 4: Run the temper-core test suite to catch consumers**

```
cargo nextest run -p temper-core
```
Expected: ALL PASS. Schema-validation tests in temper-core consume the schemas via include_str! at compile time; if any test asserts the old schema shape, it fails here. Read failures carefully — fix any test that asserts on `title` in the schema by updating to `temper-title`.

- [ ] **Step 5: Commit**

```
git add crates/temper-core/schemas/base.schema.json
git commit -m "$(cat <<'EOF'
feat(schemas): rename title to temper-title in base schema

Aligns base.schema.json with the temper-prefix contract for managed-
tier keys. Other temper-* properties were already prefixed; title was
the last bare managed-tier key in the base schema.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 8: Update `task.schema.json` — rename `slug` to `temper-slug`

**Files:**
- Modify: `crates/temper-core/schemas/task.schema.json`

- [ ] **Step 1: Edit the schema**

Open `crates/temper-core/schemas/task.schema.json`. Make two edits:

(a) In `properties`, find the `"slug": {...}` block (around line 41):
```json
    "slug": {
      "type": "string",
      "pattern": "^[a-z0-9][a-z0-9-]*$",
      "description": "URL-safe identifier"
    }
```
Rename the key to `"temper-slug"`:
```json
    "temper-slug": {
      "type": "string",
      "pattern": "^[a-z0-9][a-z0-9-]*$",
      "description": "URL-safe identifier"
    }
```

(b) In `required` (around line 47), find:
```json
  "required": ["temper-stage", "slug"],
```
Replace `"slug"` with `"temper-slug"`:
```json
  "required": ["temper-stage", "temper-slug"],
```

- [ ] **Step 2: Validate JSON**

```
python3 -c "import json; json.load(open('crates/temper-core/schemas/task.schema.json'))"
```
Expected: no output.

- [ ] **Step 3: Run temper-core tests**

```
cargo nextest run -p temper-core
```
Expected: ALL PASS.

- [ ] **Step 4: Commit**

```
git add crates/temper-core/schemas/task.schema.json
git commit -m "$(cat <<'EOF'
feat(schemas): rename slug to temper-slug in task schema

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 9: Update `goal.schema.json` — rename `slug` to `temper-slug`

**Files:**
- Modify: `crates/temper-core/schemas/goal.schema.json`

- [ ] **Step 1: Edit the schema**

Open `crates/temper-core/schemas/goal.schema.json`. Two edits:

(a) In `properties`, the `"slug": {...}` block (around line 19) — rename key to `"temper-slug"`.

(b) In `required` (around line 25), replace `"slug"` with `"temper-slug"`.

Final form:
```json
{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "$id": "https://temperkb.io/schemas/goal.schema.json",
  "allOf": [
    { "$ref": "base.schema.json" }
  ],
  "properties": {
    "temper-type": { "const": "goal" },
    "temper-status": {
      "type": "string",
      "enum": ["active", "completed", "paused", "cancelled"],
      "description": "Goal lifecycle status"
    },
    "temper-seq": {
      "type": "integer",
      "minimum": 0,
      "description": "Ordering within context"
    },
    "temper-slug": {
      "type": "string",
      "pattern": "^[a-z0-9][a-z0-9-]*$",
      "description": "URL-safe identifier"
    }
  },
  "required": ["temper-slug"],
  "additionalProperties": true
}
```

- [ ] **Step 2: Validate + run tests**

```
python3 -c "import json; json.load(open('crates/temper-core/schemas/goal.schema.json'))"
cargo nextest run -p temper-core
```
Expected: JSON valid; tests PASS.

- [ ] **Step 3: Commit**

```
git add crates/temper-core/schemas/goal.schema.json
git commit -m "$(cat <<'EOF'
feat(schemas): rename slug to temper-slug in goal schema

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 10: Update `research.schema.json` — rename `slug` and drop `date`

**Files:**
- Modify: `crates/temper-core/schemas/research.schema.json`

`date` moves to open-tier per the spec. The schema drops it from both `properties` and `required`. Open-tier fields are tolerated by `additionalProperties: true`.

- [ ] **Step 1: Replace the schema content**

Open `crates/temper-core/schemas/research.schema.json`. Replace the entire file with the post-rename form:

```json
{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "$id": "https://temperkb.io/schemas/research.schema.json",
  "allOf": [
    { "$ref": "base.schema.json" }
  ],
  "properties": {
    "temper-type": { "const": "research" },
    "temper-slug": {
      "type": "string",
      "pattern": "^[a-z0-9][a-z0-9-]*$",
      "description": "URL-safe identifier"
    }
  },
  "required": ["temper-slug"],
  "additionalProperties": true
}
```

- [ ] **Step 2: Validate + run tests**

```
python3 -c "import json; json.load(open('crates/temper-core/schemas/research.schema.json'))"
cargo nextest run -p temper-core
```
Expected: JSON valid; tests PASS. If a temper-core test asserts `date` is required for research, it fails — update the test to drop the assertion (the field is now open-tier, not managed; date stays in YAML but isn't schema-required).

- [ ] **Step 3: Commit**

```
git add crates/temper-core/schemas/research.schema.json
git commit -m "$(cat <<'EOF'
feat(schemas): rename slug→temper-slug, drop date from research

date moves to open-tier per the temper-prefix design (managed-tier =
temper-managed lifecycle fields only; date is user content). Existing
vault files keep their date field — additionalProperties tolerates it.
The DB migration to relocate date in stored JSONB lands in a later
phase.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 11: Update `decision.schema.json` — rename `slug` and drop `date`

**Files:**
- Modify: `crates/temper-core/schemas/decision.schema.json`

- [ ] **Step 1: Replace the schema content**

Open `crates/temper-core/schemas/decision.schema.json`. Replace the entire file:

```json
{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "$id": "https://temperkb.io/schemas/decision.schema.json",
  "allOf": [
    { "$ref": "base.schema.json" }
  ],
  "properties": {
    "temper-type": { "const": "decision" },
    "temper-slug": {
      "type": "string",
      "pattern": "^[a-z0-9][a-z0-9-]*$",
      "description": "URL-safe identifier"
    }
  },
  "required": ["temper-slug"],
  "additionalProperties": true
}
```

- [ ] **Step 2: Validate + run tests**

```
python3 -c "import json; json.load(open('crates/temper-core/schemas/decision.schema.json'))"
cargo nextest run -p temper-core
```
Expected: JSON valid; tests PASS.

- [ ] **Step 3: Commit**

```
git add crates/temper-core/schemas/decision.schema.json
git commit -m "$(cat <<'EOF'
feat(schemas): rename slug→temper-slug, drop date from decision

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 12: Update `concept.schema.json` — rename `slug` and drop `date`

**Files:**
- Modify: `crates/temper-core/schemas/concept.schema.json`

- [ ] **Step 1: Replace the schema content**

Open `crates/temper-core/schemas/concept.schema.json`. Replace the entire file:

```json
{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "$id": "https://temperkb.io/schemas/concept.schema.json",
  "allOf": [
    { "$ref": "base.schema.json" }
  ],
  "properties": {
    "temper-type": { "const": "concept" },
    "temper-slug": {
      "type": "string",
      "pattern": "^[a-z0-9][a-z0-9-]*$",
      "description": "URL-safe identifier"
    }
  },
  "required": ["temper-slug"],
  "additionalProperties": true
}
```

- [ ] **Step 2: Validate + run tests**

```
python3 -c "import json; json.load(open('crates/temper-core/schemas/concept.schema.json'))"
cargo nextest run -p temper-core
```
Expected: JSON valid; tests PASS.

- [ ] **Step 3: Commit**

```
git add crates/temper-core/schemas/concept.schema.json
git commit -m "$(cat <<'EOF'
feat(schemas): rename slug→temper-slug, drop date from concept

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 13: Update `session.schema.json` — drop `date`

**Files:**
- Modify: `crates/temper-core/schemas/session.schema.json`

Sessions have no `slug` to rename (they're identified by date+title in the URL, not a separate slug). This task only drops `date` from the schema; the field continues to live in vault files as open-tier metadata, tolerated by `additionalProperties: true`.

- [ ] **Step 1: Replace the schema content**

Open `crates/temper-core/schemas/session.schema.json`. Replace the entire file:

```json
{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "$id": "https://temperkb.io/schemas/session.schema.json",
  "allOf": [
    { "$ref": "base.schema.json" }
  ],
  "properties": {
    "temper-type": { "const": "session" }
  },
  "required": [],
  "additionalProperties": true
}
```

Note: empty `required` array kept explicit rather than removed — preserves the field's presence as a signaling shape (downstream code reads it).

- [ ] **Step 2: Validate + run tests**

```
python3 -c "import json; json.load(open('crates/temper-core/schemas/session.schema.json'))"
cargo nextest run -p temper-core
```
Expected: JSON valid; tests PASS. If a temper-core schema-validation test asserts a session document must have `date`, it fails here — update the test to drop the assertion or remove the test entirely if its sole purpose was the date check (per `feedback_no_premature_backward_compat.md`, prefer deletion to retention as dead code).

- [ ] **Step 3: Commit**

```
git add crates/temper-core/schemas/session.schema.json
git commit -m "$(cat <<'EOF'
feat(schemas): drop date from session schema

date is user-content, not temper-managed lifecycle. Sessions
continue to write date into YAML (additionalProperties: true);
the DB migration to relocate date in stored JSONB lands in a
later phase.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 14: Cross-crate verification

Verify that the schema/types contract change does not break any consumer in temper-cli, temper-api, temper-mcp, temper-client, or temper-cloud (TS).

**Files:**
- No modifications. Verification only.

- [ ] **Step 1: Workspace-wide unit tests**

```
cargo make test
```
Expected: ALL PASS. Failures in non-temper-core crates indicate consumers that depended on the old contract — record each failing test name and crate, then triage one of:
  - Trivial: a literal string `"title"` or `"slug"` in test fixture YAML; update the fixture.
  - Architectural: a code path that compares against bare `title`/`slug`; this is real drift the rename surfaced. Stop and report BLOCKED with the failing test name and the offending file:line.

- [ ] **Step 2: DB-backed integration tests**

```
cargo make docker-up
cargo make test-db
```
Expected: ALL PASS. Same triage as Step 1 for any failures. The integration tests against the real DB schema may surface server-side stripping or column-extraction bugs; if so, they are spec Phase 5 work and reporting BLOCKED is correct (do NOT silently soften).

- [ ] **Step 3: Lint + format**

```
cargo make check
```
Expected: ALL PASS.

- [ ] **Step 4: Regenerate sqlx offline cache (if any SQL changed transitively)**

This phase shouldn't change SQL, but defensive verification:

```
cargo sqlx prepare --workspace -- --all-features
git status -- .sqlx/
```
Expected: `git status` reports no changes to `.sqlx/`. If the cache regenerates with diff, that means the typed `ManagedMeta` field rename surfaced in a `query!` macro — investigate. (Most likely a non-issue because the JSONB column type is `jsonb` and the rename is at the serde layer, not the SQL layer.)

- [ ] **Step 5: TypeScript regeneration (ts-rs)**

```
cargo make generate-ts-types
git status -- packages/temper-ui/src/lib/types/
```
Expected: regenerated files contain `temper-title` and `temper-slug` keys (visible as `"temper-title"?: string` etc. in the generated `managed_meta.ts`). Commit the regenerated files.

If the TS types drift from what consumers in `packages/temper-ui/` expect — i.e., the SvelteKit code references `meta.title` directly assuming it's keyed as `title` rather than `temper-title` — that is in-scope ad-hoc cleanup for this task: update the TS consumers to read the new keys. This is a TS-side rename; no behavior change.

```
cd packages/temper-ui && bun run check
```
Expected: PASS. Failures point to TS consumers that need the rename.

- [ ] **Step 6: Validation-agent-pass checklist (per spec)**

Run each step from the spec's "Validation-Agent-Pass Checklist" section, capture the output, and confirm:

```
# 1. Already done in Step 3.
cargo make check

# 2. Already done in Step 1.
cargo make test

# 3. Already done in Step 2.
cargo make test-db

# 4. e2e — required.
cargo make test-e2e

# 5. Grep for production code emitting bare title/slug into managed_meta.
grep -rn '"title":\|"slug":\|set_managed_field("title"\|set_managed_field("slug"' \
  crates/temper-core/src/ crates/temper-api/src/services/ crates/temper-cli/src/actions/
# Expected: zero hits in production write paths. Hits in tests, fixtures, or
# the alias-handling code (LEGACY_FIELDS lookups, normalize_aliases) are
# expected and fine.

# 6. Grep for production code writing date into managed_meta.
grep -rn 'set_managed.*"date"\|"date".*managed' crates/temper-core/src/ crates/temper-api/src/services/
# Expected: zero hits. (The doctor-fix and ingest paths in temper-cli still
# read `date` for filename inference — those are NOT writing into managed
# and are out of scope here. Phase 7 of the spec retires them.)

# 7. Diff review — read every file changed in this plan and confirm:
#    - NO "for now" comments
#    - NO "until X reconciled" comments
#    - NO new TODOs without ticket links
git diff main..HEAD -- crates/temper-core/ | grep -i 'for now\|until.*reconciled\|TODO' \
  | grep -v 'tests::\|#\[test\]'
# Expected: zero hits.
```

If any expected-zero check has hits in production code, report BLOCKED. Test fixtures and alias-handling code are exempt.

- [ ] **Step 7: Commit any TS regeneration**

```
git add packages/temper-ui/src/lib/types/ packages/temper-ui/src/  # if TS consumer updates were needed
git commit -m "$(cat <<'EOF'
chore(types): regenerate TS types after temper-title/temper-slug rename

ts-rs regenerated managed_meta.ts to reflect the serde renames
landed in this phase. SvelteKit consumers updated to read the
new keys.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

If no TS files changed, skip this step.

---

## Self-Review Checklist (run before reporting plan complete)

- [ ] Each spec acceptance criterion that lands in this phase is covered:
  - "All seven JSON schemas use temper- prefix for every managed-tier key" → Tasks 7-13.
  - "ManagedMeta has serde renames matching schemas exactly" → Tasks 4-5.
  - "TIER1_SYSTEM_FIELDS and KNOWN_TEMPER_FIELDS updated" → Tasks 2 (KNOWN), 3 (SYSTEM_MANAGED). TIER1_SYSTEM_FIELDS does not need updating because temper-title/temper-slug are NOT tier-1 (they participate in managed_hash by design).
  - "Alias normalization at parse time during the transition window" → Tasks 1, 6.
- [ ] Out-of-scope items remain out-of-scope (server-side stripping, DB migration, doctor fix, canonical.rs, templates, tier-2 re-enable).
- [ ] No placeholders, no TODOs, no `// for now`. Every step has exact code or exact commands.
- [ ] Every test has a clear pass/fail expectation.
- [ ] Tasks are ordered so each commit leaves the workspace green: alias-table additions before they're consulted (Task 1 before Task 6), KNOWN_TEMPER_FIELDS update before its consumers, ManagedMeta serde renames paired (Tasks 4 then 5 with the test-update bridge in Step 5 of Task 4).

## What's Next After This Plan Lands

Three follow-up plans are needed to complete the spec's 9-phase migration:

- **Phase 3: Canonical-form display + hash** — touch `crates/temper-core/src/frontmatter/canonical.rs` (lines 61-66 hardcode bare `title`/`slug`). Decide between explicit pre-list rename and merging into `schema_property_order` with `allOf` traversal. Spec open question 2.
- **Phase 4: CLI write paths + askama templates** — emit canonical keys from `crates/temper-cli/templates/*.md` and `actions/frontmatter::build_managed_meta_for_create`. `[subagent-OK]` once Phase 3 is green.
- **Phases 5-9** — server-side stripping cleanup, DB migration, read-side cleanup, tier-2 re-enable, vault doctor fix. Phase 5 is blocked on locating the title/slug column-extraction SQL site (spec open question 3); the others depend on Phase 5 + 6 outcomes.

Each gets its own plan document in `docs/superpowers/plans/`, written when its open questions resolve and the predecessor plans have landed.
