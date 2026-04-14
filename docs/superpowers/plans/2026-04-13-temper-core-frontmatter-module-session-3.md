# temper-core Frontmatter Consolidation — Session 3 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Finish the frontmatter consolidation by retiring every remaining caller of the `temper-cli/src/vault.rs` ad-hoc YAML helpers (`parse_frontmatter`, `set_frontmatter_field`, `insert_frontmatter_field`, `rename_frontmatter_field`, `remove_frontmatter_field`, `replace_body`), migrating every `temper-cli` read/write call site to the new `temper_core::frontmatter::Frontmatter` aggregate, moving the consolidated frontmatter-field constants to `frontmatter::fields` proper, adopting `DocType::schema_json()` in the three stringly-typed `schema.rs` sites, wiring `KNOWN_OPEN_FIELDS` validation into `temper-api`'s `meta_service.rs`, and folding alias canonicalization into `temper doctor fix` as a final pass — with the same byte-identical real-vault verification gate Session 2 used as its acceptance signal.

**Architecture:** The work decomposes into six sequential phases (A–F) with a short shared pattern library. Phase A flips constant ownership and adopts `DocType::schema_json()` (low risk, additive). Phase B migrates the two write-heavy modules (`doctor_fix.rs` apply_plan + `ingest.rs` build_frontmatter) — these are the highest-risk migrations because they change on-disk byte representation. Phase C migrates ~12 read-only `parse_frontmatter` call sites mechanically. Phase D migrates ~21 in-place mutation call sites that go through `vault::set_frontmatter_field`/`replace_body` (commands/research, commands/session, commands/resource, actions/task, actions/goal). Phase E retires the ad-hoc helpers, adds the `doctor fix` canonicalization pass, and wires `KNOWN_OPEN_FIELDS` into `meta_service.rs`. Phase F is the final verification gate. Sessions 1 + 2's testing discipline holds: per-task `cargo make check` + per-task commit + real-vault `temper doctor` byte-diff against `main` as the load-bearing acceptance gate.

**Tech Stack:** Rust 1.x workspace, `cargo nextest`, `cargo-make`, `serde_yaml`, the `Frontmatter` aggregate at `crates/temper-core/src/frontmatter/`, the `Vault` layout helpers at `crates/temper-core/src/vault.rs`, and the legacy ad-hoc helpers at `crates/temper-cli/src/vault.rs` that this session retires.

---

## Plan-Reality Grounding (2026-04-14)

This section locks in the controller's verified observations of the codebase at branch `jct/frontmatter-consolidation` HEAD `17574ef`. Every file path + line number below has been grep-confirmed. **Implementer subagents must re-grep before editing** — `git pull` between sessions can shift line numbers, but file/function identities are stable.

### State of the `Frontmatter` aggregate (`crates/temper-core/src/frontmatter/`)

Public API surface as it exists today:

```rust
// document.rs
impl Frontmatter {
    pub fn try_from(content: &str) -> Result<Self>;             // implemented as TryFrom<&str>
    pub fn parse_file(path: &Path) -> Result<Self>;
    pub fn doc_type(&self) -> DocType;
    pub fn value(&self) -> &serde_yaml::Value;
    pub fn value_mut(&mut self) -> &mut serde_yaml::Value;
    pub fn body(&self) -> &str;
    pub fn managed_json(&self) -> serde_json::Value;
    pub fn open_json(&self) -> serde_json::Value;
    pub fn hashes(&self) -> (String, String);
    pub fn validate(&self) -> Result<Vec<crate::schema::ValidationIssue>>;
    pub fn serialize(&self) -> Result<String>;
    pub fn write_to(&self, path: &Path) -> Result<()>;
    pub fn set_managed_field(&mut self, key: &str, value: serde_json::Value);
    pub fn set_open_field(&mut self, key: &str, value: serde_json::Value);
    pub fn remove_field(&mut self, key: &str);
    pub fn set_relationships(&mut self, rels: &ResourceRelationships);
    pub fn tags(&self) -> Vec<String>;
}

// document.rs — DocType enum
impl DocType {
    pub fn as_str(&self) -> &'static str;
    pub fn from_str(s: &str) -> Result<Self>;
    pub fn schema_json(&self) -> &'static str;   // static schema text per variant
}

// frontmatter::fields (Session 1 re-exports — Session 3 flips ownership)
pub use crate::hash::{IDENTITY_FIELDS, TIER1_SYSTEM_FIELDS};
pub use crate::schema::SYSTEM_MANAGED_FIELDS;
```

**Missing API for write-from-scratch:** `Frontmatter::new(doc_type, body)` does not exist. `ingest.rs::build_frontmatter` constructs the YAML by `format!`-ing a string from individual fields, then concatenating `body`. **Task 1 adds this constructor** so Phase B's `ingest.rs` migration has a clean target.

**Missing API for canonicalization-only writes:** `Frontmatter::write_to` always re-serializes via `serialize()`. There is no "write only if canonical-form bytes differ from on-disk" helper. **Task 16 adds it** as a small private helper in `actions::doctor::fix` (not a new `Frontmatter` method — it's a doctor-pass concern).

### State of `temper-cli/src/vault.rs` — the retirement targets

```rust
// crates/temper-cli/src/vault.rs
pub fn parse_frontmatter(content: &str) -> Option<serde_yaml::Value>;     // line 201
pub fn set_frontmatter_field(content: &str, key: &str, value: &str) -> String;  // line 245
pub fn rename_frontmatter_field(content: &str, old_key: &str, new_key: &str) -> String;  // line 262
pub fn remove_frontmatter_field(content: &str, key: &str) -> String;      // line 292
pub fn insert_frontmatter_field(content: &str, key: &str, value: &str) -> String;  // line 322
pub fn replace_body(existing: &str, new_body: &str) -> String;            // line 354
```

These six are the consolidation targets. **Session 3 deletes all six** once every caller migrates. (Note: `crates/temper-core/src/vault.rs` is the unrelated `Vault` layout-rules module — do NOT confuse the two. It owns `Vault::new`, `parse_rel`, `doc_file`, `rel_path`, etc., and stays exactly as-is.)

### Production call sites — `vault::parse_frontmatter` (read paths, 12 sites)

| File | Line | Function | What it does with `fm` | Migration target |
|---|---|---|---|---|
| `commands/session.rs` | 158 | `cmd_open` | reads `temper-id` for session lookup | `Frontmatter::try_from` + `value().get("temper-id")` |
| `commands/session.rs` | 342 | `handle_link_task` | extracts fields for task linking | `Frontmatter::try_from` + `value()` |
| `commands/resource.rs` | 341 | `cmd_edit` | checks for missing `temper-context` | `Frontmatter::try_from` + `value()` |
| `commands/resource.rs` | 549 | `cmd_open` | extract doc_type + context | `fm.doc_type()` + `value()` |
| `actions/doctor.rs` | 159 | `scan_file` | issue collection | `Frontmatter::try_from(&content)` + `fm.value()` |
| `actions/doctor.rs` | 413 | `collect_fixes_for_file` | dedup pre-pass parse check | `Frontmatter::try_from` `is_ok` |
| `actions/doctor.rs` | 420 | `collect_fixes_for_file` | post-dedup re-parse | `Frontmatter::try_from` |
| `actions/ingest.rs` | 62 | `parse_source_frontmatter` | extracts metadata from source files | `Frontmatter::try_from` + `value()` |
| `actions/task.rs` | 53 | `cmd_update_status` | field presence check | `Frontmatter::try_from` |
| `actions/goal.rs` | 47 | `cmd_update_status` | field presence check | `Frontmatter::try_from` |
| `actions/goal.rs` | 214 | (other status helper) | field presence check | `Frontmatter::try_from` |
| `actions/sync.rs` | 62 | `do_sync_pull` | extract fields for manifest sync | `Frontmatter::try_from` |
| `actions/vault.rs` | 12 | `read_document` | reads `temper-type` and `title`, returns `VaultDocument` | `Frontmatter::try_from` (read-only) |

Note: `actions/vault.rs::read_document` is **read-only** despite an earlier mis-classification as a write path — it just builds a `VaultDocument` struct for ingest discovery. No `fs::write` follows it.

### Production call sites — `vault::set_frontmatter_field` / `replace_body` / `insert_frontmatter_field` (write paths, ~21 sites)

| File | Line | Function | Mutation pattern |
|---|---|---|---|
| `commands/research.rs` | 31 | `cmd_create` | `replace_body(&content, &new_body)` |
| `commands/research.rs` | 53 | `cmd_edit` | `set_frontmatter_field` chained |
| `commands/research.rs` | 56 | `cmd_edit` | `replace_body` after field set |
| `commands/session.rs` | 67 | `cmd_create` | `replace_body` |
| `commands/session.rs` | 89 | `cmd_create` | `set_frontmatter_field` |
| `commands/session.rs` | 93 | `cmd_create` | `set_frontmatter_field` |
| `commands/session.rs` | 210 | `handle_link_task` | `set_frontmatter_field("temper-branch", ...)` |
| `commands/session.rs` | 218 | `handle_link_task` | `set_frontmatter_field("temper-stage", ...)` |
| `commands/resource.rs` | 170 | `cmd_edit` | `set_frontmatter_field` (one of 5 call sites in this file) |
| `commands/resource.rs` | 817 | `cmd_set_meta` | `set_frontmatter_field` |
| `commands/resource.rs` | 850 | `cmd_set_meta` | `set_frontmatter_field` |
| `commands/resource.rs` | 867 | `cmd_set_meta` | `set_frontmatter_field` |
| `commands/resource.rs` | 873 | `cmd_set_meta` | `set_frontmatter_field` |
| `actions/goal.rs` | 180 | `cmd_update_status` | `set_frontmatter_field("temper-stage", ...)` |
| `actions/task.rs` | 259, 272, 275, 279, 283, 287, 323–329 | `cmd_update_*` family | `set_frontmatter_field` chained 7+ times across the file |
| `actions/doctor_fix.rs` | 838 | `apply_plan` `RenameField` branch | `parse_frontmatter` + `rename_frontmatter_field`/`remove_frontmatter_field` + `fs::write` |
| `actions/doctor_fix.rs` | 867 | `apply_plan` `SetField` branch | `insert_frontmatter_field` + `fs::write` |
| `actions/doctor_fix.rs` | 893 | `apply_plan` `SetOwnerField` branch | `insert_frontmatter_field` + `fs::write` |

### Production call sites — `ingest.rs` ad-hoc YAML construction

| File | Line | Function | Pattern |
|---|---|---|---|
| `actions/ingest.rs` | 406–428 | `build_frontmatter` | `format!`-driven YAML construction from individual fields |
| `actions/ingest.rs` | 451–474 | `emit_meta_tier` | private helper that walks a JSON map and string-formats it as YAML |
| `actions/ingest.rs` | 483–520 | `build_frontmatter_from_resource` | combines `build_frontmatter` + `emit_meta_tier(managed)` + `emit_meta_tier(open)` |
| `actions/ingest.rs` | 525–563 | `json_value_to_yaml` | private recursive JSON → YAML stringifier feeding `emit_meta_tier` |
| `actions/ingest.rs` | 566–570 | `yaml_escape_string` | private double-quoted-scalar escaper |
| `actions/ingest.rs` | 598–607 | `write_vault_file_and_register` | calls `build_frontmatter` then `format!("{frontmatter}{content}")` then `fs::write` |

**All six of these helpers go away** when Phase B migrates `write_vault_file_and_register` to `Frontmatter::new` + `set_managed_field` + `set_open_field` + `write_to`. The `serde_yaml` serializer inside `Frontmatter::serialize()` handles every case `json_value_to_yaml`/`yaml_escape_string` was hand-rolling.

### Production call sites — `temper-core::schema.rs` constants and stringly-typed matches

```rust
// crates/temper-core/src/schema.rs

// Lines 57–77 — moves to frontmatter::fields in Task 3
static KNOWN_TEMPER_FIELDS: &[&str] = &[
    "temper-id", "temper-provisional-id", "temper-type", "temper-context",
    "temper-created", "temper-updated", "temper-owner", "temper-source",
    "temper-stage", "temper-mode", "temper-effort", "temper-goal", "temper-seq",
    "temper-branch", "temper-pr", "temper-status",
];

// Lines 81–97 — moves to frontmatter::fields in Task 3
static LEGACY_FIELDS: &[(&str, &str)] = &[
    ("id", "temper-id"), ("type", "temper-type"), ("doc_type", "temper-type"),
    ("context", "temper-context"), ("project", "temper-context"),
    /* ... 12 total mappings, gather verbatim from current source ... */
];

// Lines 276–287 — moves to frontmatter::fields in Task 3
pub static SYSTEM_MANAGED_FIELDS: &[&str] = &[
    "temper-id", "temper-provisional-id", "temper-type", "temper-context",
    "temper-owner", "temper-created", "temper-updated", "temper-source",
    "temper-legacy-id", "slug",
];

// Lines 111–124 — Task 4: replace match with DocType::schema_json()
pub fn load_schema(doc_type: &str) -> Result<Validator> {
    let doc_schema_str = match doc_type {
        "task" => TASK_SCHEMA, /* ... */
    };
    /* ... */
}

// Lines 291–299 — Task 5
pub fn updatable_fields(doc_type: &str) -> Result<Vec<(String, serde_json::Value)>> {
    let schema_str = match doc_type { /* ... */ };
    /* ... */
}

// Lines 402–415 — Task 6
pub fn schema_value(doc_type: &str) -> Result<serde_json::Value> {
    let raw = match doc_type { /* ... */ };
    /* ... */
}
```

### Production call sites — `validate_frontmatter` / `validate_allowing_provisional` outside `schema.rs`

- `crates/temper-core/src/frontmatter/document.rs:166` — `Frontmatter::validate()` delegates to `validate_allowing_provisional`. Stays.
- `crates/temper-api/src/services/ingest_service.rs:760` — server-side validation calls `validate_frontmatter(params.doc_type, &yaml_value)`. Out of scope for migration (server path), but keeps `validate_frontmatter` in the public API.

**Conclusion:** `validate_frontmatter` and `validate_allowing_provisional` cannot be deleted in Session 3 — `ingest_service.rs` is a live caller. Leave both as-is. Update the spec's "Future Work" section if it implies otherwise. A separate Phase B follow-up could thin them, but Session 3 doesn't touch them.

### State of `crates/temper-cli/src/commands/doctor.rs` (the `temper doctor` CLI)

Two entry points, no subcommands beyond `fix`:

```rust
pub fn run(config: &Config, context: Option<&str>, format: &str) -> Result<()>;
pub fn run_fix(config: &Config, context: Option<&str>, dry_run: bool) -> Result<()>;
```

The `--fix` is actually a subcommand: invocation is `temper doctor fix [--dry-run] [--context <ctx>]`. Confirmed via `commands/doctor.rs:107` hint text (`"Run \`temper doctor fix\`"`).

`run_fix` calls `actions::doctor::fix(config, context, dry_run)` and prints a one-line summary built from `ApplyReport` fields (`fields_renamed`, `fields_set`, `owner_backfilled`, `files_renamed`, `files_relocated`, `manifest_updated`, `manifest_removed`).

**Task 16** adds a `canonicalized: u32` field to `ApplyReport`, runs the canonicalization pass at the end of `actions::doctor::fix` (after `apply_plan` + manifest reconciliation), and updates `commands/doctor.rs::run_fix` to include the count in its dim/success line.

### State of `temper-api/src/services/meta_service.rs`

Currently imports:

```rust
use temper_core::types::ids::{ContextId, ProfileId, ResourceId};
use temper_core::types::managed_meta::{ManagedMeta, MetaUpdatePayload, ResourceMetaResponse};
```

No import of `KNOWN_OPEN_FIELDS` or any local hardcoded open-field list. The `update_meta` function (lines 60–165) trusts that `MetaUpdatePayload`'s `open_meta: serde_json::Value` is well-formed — there is no key validation today.

**Task 17** adds an import of `temper_core::frontmatter::KNOWN_OPEN_FIELDS` and walks the `open_meta` keys before the UPDATE statement, returning a 400-class error for unknown open-meta keys (after normalizing aliases — same semantics as `Frontmatter::try_from`'s alias normalization). Server-side validation is purely additive: existing well-formed clients continue to work.

### Verification gates before any code change

Each task's "verification" step is one of:
- `cargo nextest run --workspace` — fast workspace tests (no DB)
- `cargo nextest run --workspace --features test-db` — full Rust suite (postgres required)
- `cargo make check` — fmt, clippy -D warnings, docs, machete, TS typecheck, biome
- `cd packages/temper-ui && bun run check` — svelte-check 0/0
- `cargo make ts-test` — vitest
- `cargo sqlx prepare --workspace --check` — SQL cache drift (Session 3 touches zero SQL — should always pass clean)
- **Real-vault byte-diff:** the load-bearing acceptance gate. After Phase B and Phase D, the byte-diff against `main` for `/Users/petetaylor/projects/kb-vault` must be empty for every `.md` file. Method: build `target/debug/temper` from this branch, `cp -r kb-vault /tmp/kb-vault-session3-pre`, run `temper doctor` on the original, then run `git stash && cargo build && temper doctor` from `main`, then `diff -r` the two trees. (Detail in Task 18.)

---

## Migration Pattern Library

Every migration in this session follows one of the six patterns below. Implementer subagents should reference these by name in their commit messages so reviewers can spot pattern drift.

### Pattern P1 — Read-only parse (`vault::parse_frontmatter` → `Frontmatter::try_from`)

Use when a call site reads a YAML field and never writes back.

**Before:**
```rust
let content = fs::read_to_string(path)?;
let fm = match crate::vault::parse_frontmatter(&content) {
    Some(fm) => fm,
    None => return Err(...),  // or some fallback
};
let temper_id = fm.get("temper-id").and_then(|v| v.as_str())?;
```

**After:**
```rust
let content = fs::read_to_string(path)?;
let fm = temper_core::frontmatter::Frontmatter::try_from(content.as_str())?;
let temper_id = fm
    .value()
    .get("temper-id")
    .and_then(|v| v.as_str())
    .ok_or_else(|| /* same error */)?;
```

**Notes:**
- `Frontmatter::try_from` is **stricter** than `vault::parse_frontmatter`: it requires a parseable YAML mapping AND a present `temper-type` AND a known doctype. `vault::parse_frontmatter` returns `Some(value)` for any well-formed YAML. A handful of read sites currently use `parse_frontmatter` on files where `temper-type` may be absent (e.g. for legacy detection). For those sites, fall back to `Frontmatter::try_from(content).ok()` and proceed only if `Some`.
- For sites that just want to know "does this file have a parseable frontmatter," use `Frontmatter::try_from(content).is_ok()`.
- For sites that want `doc_type`, use `fm.doc_type().as_str()` rather than `fm.value().get("temper-type")`.

### Pattern P2 — In-place field mutation (`set_frontmatter_field` → `Frontmatter::set_managed_field` + `write_to`)

Use when a call site reads a file, mutates one or more YAML fields, and writes it back.

**Before:**
```rust
let content = fs::read_to_string(path)?;
let updated = crate::vault::set_frontmatter_field(&content, "temper-stage", "in-progress");
fs::write(path, updated)?;
```

**After:**
```rust
let mut fm = temper_core::frontmatter::Frontmatter::parse_file(path)?;
fm.set_managed_field("temper-stage", serde_json::json!("in-progress"));
fm.write_to(path)?;
```

**Notes:**
- For multiple consecutive field sets, do them all on the same `Frontmatter` instance and call `write_to` once at the end. This avoids parse + write round-trips.
- `set_managed_field` and `set_open_field` are currently identical implementations (both call `set_raw_field`). Pick whichever is most semantically accurate at the call site — if the field is a managed-tier field (per `SYSTEM_MANAGED_FIELDS` or schema), use `set_managed_field`; if it's an open-tier field (relationships, tags, custom keys), use `set_open_field`. The split exists for documentation, not behavior.
- `Frontmatter::write_to` writes atomically (tmp file + rename) so partial writes are not a concern.

### Pattern P3 — Body replacement (`replace_body` → `Frontmatter` mutate-body)

`Frontmatter` does not currently expose a setter for the body. **Task 1 adds `Frontmatter::set_body(&mut self, body: String)`** to fill this gap. After Task 1 lands:

**Before:**
```rust
let updated = crate::vault::replace_body(&content, &new_body);
fs::write(path, updated)?;
```

**After:**
```rust
let mut fm = temper_core::frontmatter::Frontmatter::parse_file(path)?;
fm.set_body(new_body);
fm.write_to(path)?;
```

### Pattern P4 — Build from scratch (`ingest::build_frontmatter` → `Frontmatter::new`)

`Frontmatter::new(doc_type, body)` does not currently exist. **Task 1 adds it.** Constructor returns a `Frontmatter` with an empty mapping containing only the `temper-type` key set to `doc_type.as_str()`. Callers populate the rest via `set_managed_field` / `set_open_field` and `write_to` produces fully canonical YAML.

**Before:**
```rust
let frontmatter = build_frontmatter(resource.id, &resource.title, context, doc_type, ...);
let vault_content = format!("{frontmatter}{content}");
fs::write(&vault_path, &vault_content)?;
```

**After:**
```rust
let mut fm = temper_core::frontmatter::Frontmatter::new(
    DocType::from_str(doc_type)?,
    content.to_string(),
);
fm.set_managed_field("temper-id", json!(resource.id.to_string()));
fm.set_managed_field("temper-context", json!(context));
fm.set_managed_field("temper-created", json!(now.to_rfc3339()));
fm.set_managed_field("title", json!(resource.title));
if let Some(s) = ingestion_source {
    fm.set_managed_field("temper-source", json!(s));
}
for (k, v) in extra_fields.unwrap_or(&[]) {
    fm.set_managed_field(k, json!(v));
}
fm.write_to(&vault_path)?;
```

### Pattern P5 — Doctor fix applicator field branches (`apply_plan` → `Frontmatter` mutate)

The three field-mutation branches of `actions::doctor_fix::apply_plan` (RenameField, SetField, SetOwnerField) all follow Pattern P2 with one wrinkle: RenameField also has to handle the case where the new key already exists (in which case the old key is removed without renaming).

**Before (RenameField branch):**
```rust
let content = fs::read_to_string(path)?;
let updated = if let Some(fm) = crate::vault::parse_frontmatter(&content) {
    let new_exists = fm.get(new_key.as_str()).is_some();
    if new_exists {
        crate::vault::remove_frontmatter_field(&content, old_key)
    } else {
        crate::vault::rename_frontmatter_field(&content, old_key, new_key)
    }
} else {
    crate::vault::rename_frontmatter_field(&content, old_key, new_key)
};
fs::write(path, updated)?;
```

**After:**
```rust
let mut fm = match temper_core::frontmatter::Frontmatter::parse_file(path) {
    Ok(fm) => fm,
    Err(_) => {
        // File has no parseable frontmatter — skip (matches old fallback semantics:
        // the old code would still call rename_frontmatter_field, which is a no-op
        // on files with no frontmatter block).
        report.fields_renamed += 1;
        continue;
    }
};
let value_mut = fm.value_mut().as_mapping_mut().expect("Frontmatter invariant: value is mapping");
let old_yaml_key = serde_yaml::Value::String(old_key.clone());
let new_yaml_key = serde_yaml::Value::String(new_key.clone());
let new_exists = value_mut.contains_key(&new_yaml_key);
if new_exists {
    value_mut.remove(&old_yaml_key);
} else if let Some(old_value) = value_mut.remove(&old_yaml_key) {
    value_mut.insert(new_yaml_key, old_value);
}
fm.write_to(path)?;
report.fields_renamed += 1;
```

**SetField and SetOwnerField branches** are simpler — they're pure Pattern P2.

### Pattern P6 — Doctor canonicalization pass (new in Task 16)

After `apply_plan` runs, walk the vault again and re-save every parseable file in canonical form. Files already in canonical form produce byte-identical output and skip the write. The pass increments `report.canonicalized` for each file actually rewritten.

```rust
fn canonicalize_pass(
    config: &Config,
    context_filter: Option<&str>,
    dry_run: bool,
    report: &mut ApplyReport,
) -> Result<()> {
    let vault_layout = Vault::new(&config.vault_root);
    let contexts: Vec<String> = if let Some(ctx) = context_filter {
        vec![ctx.to_string()]
    } else {
        config.contexts.clone()
    };
    for doc_type in ENTITY_DOC_TYPES {
        for ctx in &contexts {
            let owner = config.owner_for_context(ctx);
            let dir = vault_layout.doc_type_dir(&owner, ctx, doc_type);
            if !dir.is_dir() { continue; }
            for entry in fs::read_dir(&dir)? {
                let path = entry?.path();
                if path.extension().is_none_or(|e| e != "md") { continue; }
                canonicalize_one(&path, dry_run, report)?;
            }
        }
    }
    Ok(())
}

fn canonicalize_one(path: &Path, dry_run: bool, report: &mut ApplyReport) -> Result<()> {
    let original = fs::read_to_string(path)?;
    let fm = match temper_core::frontmatter::Frontmatter::try_from(original.as_str()) {
        Ok(fm) => fm,
        Err(_) => return Ok(()), // skip silently — schema/parse failures aren't this pass's job
    };
    let canonical = fm.serialize()?;
    if canonical != original {
        if !dry_run {
            fm.write_to(path)?;
        }
        report.canonicalized += 1;
    }
    Ok(())
}
```

---

## Subagent Guidance (SG-1 through SG-13)

These are the same standing rules used in Session 2. **Every implementer prompt must include the SG entries that apply to its task** (verbatim, not summarized).

### SG-1: Plan/reality verification before any edit

Before editing any file the plan claims has function `X` at line `N`, **re-grep** to confirm `X` still exists and is at approximately `N`. Line numbers shift between sessions; function identities don't. If the function is renamed, gone, or moved to a different file, **STOP and report the gap** — do not invent a fix. The controller is responsible for resolving plan/reality drift, not the implementer.

### SG-2: Per-task commits

One task = one commit (or two if a code review fix needs splitting). Commit message format: `{type}({scope}): {what}` with optional body explaining `why`. Use `refactor` for migration-only commits, `feat` for new APIs (Task 1, Task 16, Task 17), `chore` for constant moves, `test` for test-only commits.

### SG-3: Verification before claiming success

Per `superpowers:verification-before-completion`. After every code change:
1. Run the smallest test scope that exercises the change (`cargo nextest run -p <crate> <test_name>`).
2. Then run `cargo make check` (fmt + clippy + docs + machete + TS + biome).
3. Only after both pass do you write the "completed" line in the task tracking.

Never claim a task is done because "the code looks right." Run the tests.

### SG-4: No drive-by refactors

If you notice unrelated code that looks improvable, **leave it alone**. Note it in the task's commit body as a "spotted, deferred" item. Drive-by refactors break bisect and inflate review surface.

### SG-5: Preserve doc comments

When rewriting a function body, copy the original doc comment forward verbatim. If the function's contract changes (new params, new error semantics, new return type), update the doc comment to match. **Never silently drop a doc comment.** Code review on Session 2 caught this once; it shouldn't happen again.

### SG-6: Use the in-scope Result alias

`temper-cli` uses `crate::error::Result` as its alias. `temper-core` uses `crate::error::Result` (or just `Result` after `use crate::error::Result`). Don't mix `temper_core::error::Result` and `crate::error::Result` in the same file — pick the in-scope alias and stay consistent. Session 2 had to fix this exactly once.

### SG-7: Typed structs over inline JSON

Per CLAUDE.md: never use `serde_json::json!()` for data that has a known struct shape. **Exception**: `Frontmatter::set_managed_field` and `set_open_field` take `serde_json::Value`, so single-field sets via `json!("...")` are appropriate at call sites. Multi-field sets that mirror an existing struct should use `serde_json::to_value(&typed_struct)?` instead of constructing the JSON literal by hand.

### SG-8: No premature backwards compat

Per `feedback_no_premature_backward_compat`: this project is one month old. When migrating a call site, **delete** the old helper as soon as the last caller is gone. Do not leave deprecated re-exports, `#[deprecated]` attributes, or "kept for compat" comments. The whole point of Session 3 is to retire helpers — leaving stubs defeats it.

### SG-9: SG-13 — Match on enums, not strings

Per `feedback_no_stringly_typed_match`: when the match domain is a bounded set you own, match on the enum directly. The Phase A schema.rs cleanup (Tasks 4, 5, 6) is exactly this pattern: replace `match doc_type { "task" => TASK_SCHEMA, ... }` with `DocType::from_str(doc_type)?.schema_json()`. Don't introduce a new stringly-typed match anywhere else in the session.

### SG-10: Real-vault verification is the load-bearing gate

Tests can pass while sync output drifts. The only way to prove a migration preserves real-vault behavior is to byte-diff the on-disk vault before and after. **Phase B and Phase D end with this check** (Task 18). If the byte-diff is non-empty, treat it as a failure regardless of test results.

### SG-11: Deletions go in the same commit as the last migration

When migrating the last caller of a soon-to-be-deleted helper, delete the helper in the same commit. Reviewer sees both halves of the change at once and bisect can find the exact commit that retired the helper. Task 14's deletions piggyback on Phase D's last migration commits.

### SG-12: Comment references in docstrings

Several files have inline doc comments that reference the old helpers by name (e.g. `// Uses split_frontmatter_tiers under the hood`). When the named helper is gone, **update the comment** to reference the new path (`Frontmatter::managed_json` etc.) — don't leave a dangling reference. `grep -rn "split_frontmatter_tiers\|set_frontmatter_field\|insert_frontmatter_field"` should return zero hits in production code (excluding `docs/` and `tests/fixtures/`) at the end of the session.

### SG-13: Test fixtures are the bridge

Some tests still hand-craft YAML strings to feed the legacy helpers. When migrating their production target, **rewrite the test to feed `Frontmatter::try_from`** with the same YAML string. The assertions can keep their original shape — `Frontmatter::value().get("key")` is the lookup path. Do not delete tests just because their helper is going away; rewrite them.

---

## Tasks

### Task 1: Add `Frontmatter::new(doc_type, body)` and `Frontmatter::set_body(body)` constructors

**Files:**
- Modify: `crates/temper-core/src/frontmatter/document.rs` (add two methods to `impl Frontmatter`)
- Modify: `crates/temper-core/src/frontmatter/document.rs` (add tests in `#[cfg(test)] mod tests`)

**Why first:** Phase B (Tasks 8, 9) needs `Frontmatter::new` to migrate `ingest::build_frontmatter`. Phase D's `replace_body` migration needs `set_body`. Both APIs are tiny additive surface — landing them first unblocks every subsequent task.

- [ ] **Step 1: Re-grep `crates/temper-core/src/frontmatter/document.rs` for the `impl Frontmatter` block start and end** (per SG-1)

Run: `grep -n "^impl Frontmatter" crates/temper-core/src/frontmatter/document.rs`
Expected: line 81 (or thereabouts).

- [ ] **Step 2: Write the failing tests for `Frontmatter::new` and `Frontmatter::set_body`**

Add to `mod tests` near the existing `parse_file_and_write_to_round_trip` test:

```rust
#[test]
fn new_constructor_creates_minimal_frontmatter() {
    let fm = Frontmatter::new(DocType::Task, "body content\n".to_string());
    assert_eq!(fm.doc_type(), DocType::Task);
    assert_eq!(fm.body(), "body content\n");
    // The mapping has at least temper-type set, so try_from on the serialized
    // form would round-trip.
    let serialized = fm.serialize().expect("serialize ok");
    let parsed = Frontmatter::try_from(serialized.as_str()).expect("round-trip ok");
    assert_eq!(parsed.doc_type(), DocType::Task);
    assert_eq!(parsed.body(), "body content\n");
}

#[test]
fn new_constructor_allows_subsequent_field_population() {
    let mut fm = Frontmatter::new(DocType::Goal, String::new());
    fm.set_managed_field("title", serde_json::json!("Ship the thing"));
    fm.set_managed_field("slug", serde_json::json!("ship-the-thing"));
    fm.set_managed_field("temper-id", serde_json::json!("019d8110-8ff3-70c2-85ae-57e04ed62885"));
    fm.set_managed_field("temper-context", serde_json::json!("temper"));
    fm.set_managed_field("temper-created", serde_json::json!("2026-04-14T00:00:00Z"));
    let serialized = fm.serialize().expect("serialize ok");
    let parsed = Frontmatter::try_from(serialized.as_str()).expect("round-trip ok");
    assert_eq!(parsed.doc_type(), DocType::Goal);
    assert_eq!(
        parsed.value().get("title").and_then(|v| v.as_str()),
        Some("Ship the thing"),
    );
    assert_eq!(
        parsed.value().get("slug").and_then(|v| v.as_str()),
        Some("ship-the-thing"),
    );
}

#[test]
fn set_body_replaces_body_preserving_frontmatter() {
    let mut fm = Frontmatter::try_from(TASK_FIXTURE).unwrap();
    let original_value_keys: Vec<String> = fm
        .value()
        .as_mapping()
        .unwrap()
        .keys()
        .filter_map(|k| k.as_str().map(String::from))
        .collect();
    fm.set_body("brand new body\n".to_string());
    assert_eq!(fm.body(), "brand new body\n");
    let new_value_keys: Vec<String> = fm
        .value()
        .as_mapping()
        .unwrap()
        .keys()
        .filter_map(|k| k.as_str().map(String::from))
        .collect();
    assert_eq!(original_value_keys, new_value_keys, "set_body must not touch frontmatter mapping");
}
```

- [ ] **Step 3: Run the failing tests to confirm they fail with "no method `new`"/"no method `set_body`"**

Run: `cargo nextest run -p temper-core frontmatter::document::tests::new_constructor`
Expected: compile error — `no method named 'new' found for struct 'Frontmatter'`

- [ ] **Step 4: Implement `Frontmatter::new` and `Frontmatter::set_body`**

Add to `impl Frontmatter` in `crates/temper-core/src/frontmatter/document.rs` (before the existing `set_raw_field` private helper):

```rust
/// Construct a new `Frontmatter` with only `temper-type` set. Body is the
/// literal markdown body to emit after the frontmatter block. Callers
/// populate additional managed/open fields via [`Self::set_managed_field`]
/// and [`Self::set_open_field`] before writing.
///
/// The resulting mapping is alias-normalized by construction (it has only
/// one key, the canonical `temper-type`). Subsequent inserts are the
/// caller's responsibility — pass canonical keys.
pub fn new(doc_type: DocType, body: String) -> Self {
    let mut mapping = serde_yaml::Mapping::new();
    mapping.insert(
        serde_yaml::Value::String("temper-type".to_string()),
        serde_yaml::Value::String(doc_type.as_str().to_string()),
    );
    Self {
        doc_type,
        value: serde_yaml::Value::Mapping(mapping),
        body,
    }
}

/// Replace the body. Frontmatter mapping is untouched.
pub fn set_body(&mut self, body: String) {
    self.body = body;
}
```

- [ ] **Step 5: Run the new tests + the full `frontmatter::document` test suite**

Run: `cargo nextest run -p temper-core frontmatter::document`
Expected: all green, including the three new tests.

- [ ] **Step 6: Run `cargo make check`**

Run: `cargo make check`
Expected: clean.

- [ ] **Step 7: Commit**

```bash
git add crates/temper-core/src/frontmatter/document.rs
git commit -m "feat(frontmatter): add Frontmatter::new and set_body constructors

Adds two small additive APIs needed by Session 3 Phase B + Phase D
migrations:

- Frontmatter::new(doc_type, body) — write-from-scratch constructor
  with empty mapping (only temper-type set). Used by ingest::build_frontmatter
  migration.
- Frontmatter::set_body(body) — replace body in place. Used by
  vault::replace_body migration in commands/research.rs and commands/session.rs."
```

---

### Task 2: Move `IDENTITY_FIELDS` and `TIER1_SYSTEM_FIELDS` from `hash.rs` to `frontmatter::fields`

**Files:**
- Modify: `crates/temper-core/src/hash.rs:16-26` (delete the const definitions, add `use crate::frontmatter::fields::{IDENTITY_FIELDS, TIER1_SYSTEM_FIELDS};` at the top of the function bodies that need them — currently only `compute_managed_hash` at hash.rs:89)
- Modify: `crates/temper-core/src/frontmatter/fields.rs` (replace the `pub use crate::hash::...` re-exports with the actual const definitions)
- Modify: `crates/temper-core/src/frontmatter/fields.rs` (delete the `identity_fields_match_hash_module` and `tier1_system_fields_match_hash_module` tests — they no longer have anything to compare against once the source of truth is local)

**Why:** `frontmatter::fields` is the natural home; Session 1 only re-exported because `hash.rs` was a stable owner. Session 3 is the right time to flip ownership so future readers find these constants where they expect them.

- [ ] **Step 1: Re-grep all consumers of `IDENTITY_FIELDS` and `TIER1_SYSTEM_FIELDS`**

Run:
```bash
grep -rn "IDENTITY_FIELDS\|TIER1_SYSTEM_FIELDS" crates/temper-core/src crates/temper-cli/src crates/temper-api/src
```
Expected (production sites only — exclude tests):
- `crates/temper-core/src/hash.rs:89` (in `compute_managed_hash`)
- `crates/temper-core/src/frontmatter/canonical.rs:9, 45, 53` (already imports from `frontmatter::fields`)
- `crates/temper-core/src/frontmatter/tiers.rs:5, 29, 31` (already imports from `frontmatter::fields`)
- `crates/temper-api/src/services/ingest_service.rs:97, 103, 105` (currently `use temper_core::hash::{IDENTITY_FIELDS, TIER1_SYSTEM_FIELDS};`)

The two existing `frontmatter::fields` imports are already pointing at the right module — they just go through a re-export. After this task, the `pub use` becomes a real definition and they keep working unchanged.

- [ ] **Step 2: Move the const definitions**

In `crates/temper-core/src/frontmatter/fields.rs`, replace:

```rust
pub use crate::hash::{IDENTITY_FIELDS, TIER1_SYSTEM_FIELDS};
pub use crate::schema::SYSTEM_MANAGED_FIELDS;
```

With:

```rust
/// Identity fields — keys that uniquely identify a vault file across systems.
/// Always rendered first in the canonical display order; never appear in
/// either tier's hash input.
pub const IDENTITY_FIELDS: &[&str] = &["temper-id", "temper-provisional-id"];

/// Tier-1 system fields — `temper-*` keys managed by sync, written by the
/// server at create/update time, and stripped from `managed_meta` before
/// hashing (they live as columns on the resource row).
pub const TIER1_SYSTEM_FIELDS: &[&str] = &[
    "temper-context",
    "temper-type",
    "temper-created",
    "temper-updated",
    "temper-owner",
    "temper-source",
    "temper-legacy-id",
];

pub use crate::schema::SYSTEM_MANAGED_FIELDS; // moved in Task 3
```

- [ ] **Step 3: Update `hash.rs` to import from the new home**

In `crates/temper-core/src/hash.rs`, delete the existing definitions:

```rust
pub const IDENTITY_FIELDS: &[&str] = &["temper-id", "temper-provisional-id"];

pub const TIER1_SYSTEM_FIELDS: &[&str] = &[
    "temper-context",
    "temper-type",
    "temper-created",
    "temper-updated",
    "temper-owner",
    "temper-source",
    "temper-legacy-id",
];
```

Add at the top of the file (alongside other `use` statements):

```rust
use crate::frontmatter::fields::TIER1_SYSTEM_FIELDS;
```

(`IDENTITY_FIELDS` is not used inside `hash.rs` itself per the grounding — only `TIER1_SYSTEM_FIELDS` is needed at line 89's `compute_managed_hash`. Confirm with `grep -n "IDENTITY_FIELDS" crates/temper-core/src/hash.rs` before editing — if it's actually used somewhere I missed, import it too.)

- [ ] **Step 4: Update `temper-api/src/services/ingest_service.rs` imports**

Find the current import (around line 97):

```rust
use temper_core::hash::{IDENTITY_FIELDS, TIER1_SYSTEM_FIELDS};
```

Replace with:

```rust
use temper_core::frontmatter::fields::{IDENTITY_FIELDS, TIER1_SYSTEM_FIELDS};
```

- [ ] **Step 5: Delete the now-meaningless cross-module assertion tests**

In `crates/temper-core/src/frontmatter/fields.rs::tests`, delete:

```rust
#[test]
fn identity_fields_match_hash_module() { ... }

#[test]
fn tier1_system_fields_match_hash_module() { ... }
```

Keep `system_managed_fields_match_schema_module` (Task 3 deletes it after the SYSTEM_MANAGED_FIELDS move). Keep `identity_fields_contains_expected_keys` and `tier1_fields_contains_expected_keys` — they're meaningful smoke tests.

- [ ] **Step 6: Run the workspace test suite**

Run: `cargo nextest run --workspace`
Expected: all green. Look specifically at `compute_managed_hash` tests in `hash.rs` and the canonical/tiers tests in `frontmatter::*` — they should all still pass because the constants are byte-identical, just relocated.

- [ ] **Step 7: Run `cargo make check`**

Run: `cargo make check`
Expected: clean.

- [ ] **Step 8: Commit**

```bash
git add crates/temper-core/src/hash.rs crates/temper-core/src/frontmatter/fields.rs crates/temper-api/src/services/ingest_service.rs
git commit -m "chore(frontmatter): move IDENTITY_FIELDS and TIER1_SYSTEM_FIELDS to frontmatter::fields

Session 1 left these as re-exports from hash.rs to keep the additive
phase strictly non-breaking. Session 3 flips ownership so the constants
live where readers expect them — alongside the rest of the frontmatter
module's field metadata.

hash.rs and ingest_service.rs both update their imports to point at
frontmatter::fields. Byte-identical const arrays; no behavior change."
```

---

### Task 3: Move `SYSTEM_MANAGED_FIELDS`, `KNOWN_TEMPER_FIELDS`, `LEGACY_FIELDS` from `schema.rs` to `frontmatter::fields`

**Files:**
- Modify: `crates/temper-core/src/schema.rs:57-77` (delete `KNOWN_TEMPER_FIELDS`)
- Modify: `crates/temper-core/src/schema.rs:81-97` (delete `LEGACY_FIELDS`)
- Modify: `crates/temper-core/src/schema.rs:276-287` (delete `SYSTEM_MANAGED_FIELDS`)
- Modify: `crates/temper-core/src/schema.rs:230, 251, 309, 586` (the four call sites within `schema.rs` that read these constants — update to import from `frontmatter::fields`)
- Modify: `crates/temper-core/src/frontmatter/fields.rs` (add the three const definitions; delete `pub use crate::schema::SYSTEM_MANAGED_FIELDS`; delete the `system_managed_fields_match_schema_module` test)
- Modify: any `temper-cli` / `temper-api` site that imports these constants directly (likely none, but `grep` to confirm)

- [ ] **Step 1: Re-grep all references**

```bash
grep -rn "KNOWN_TEMPER_FIELDS\|LEGACY_FIELDS\|SYSTEM_MANAGED_FIELDS" crates/temper-core/src crates/temper-cli/src crates/temper-api/src
```

Expected production sites:
- `crates/temper-core/src/schema.rs` — definition + 4 internal references
- `crates/temper-core/src/frontmatter/fields.rs` — 1 re-export + tests
- Any others surfaced by the grep — note them down.

- [ ] **Step 2: Move the const definitions**

In `crates/temper-core/src/frontmatter/fields.rs`, the file should now look like (after Tasks 2 and 3 combined):

```rust
//! Consolidated frontmatter field constants. Single source of truth for
//! every place in the codebase that needs to know "is X an identity field?"
//! / "is X a tier-1 system field?" / "is X a system-managed managed-tier field?"
//! / "is X a known temper-* field?" / "is X a legacy-form alias?".
//!
//! Owned here in Session 3 (Session 1 re-exported these from hash.rs and
//! schema.rs to keep the additive phase strictly non-breaking).

/// Identity fields — keys that uniquely identify a vault file across systems.
pub const IDENTITY_FIELDS: &[&str] = &["temper-id", "temper-provisional-id"];

/// Tier-1 system fields — `temper-*` keys managed by sync.
pub const TIER1_SYSTEM_FIELDS: &[&str] = &[
    "temper-context",
    "temper-type",
    "temper-created",
    "temper-updated",
    "temper-owner",
    "temper-source",
    "temper-legacy-id",
];

/// Known canonical `temper-*` keys — anything matching `temper-*` not in
/// this set is flagged by `check_unknown_temper_fields` as a possible typo.
pub static KNOWN_TEMPER_FIELDS: &[&str] = &[
    "temper-id",
    "temper-provisional-id",
    "temper-type",
    "temper-context",
    "temper-created",
    "temper-updated",
    "temper-owner",
    "temper-source",
    "temper-stage",
    "temper-mode",
    "temper-effort",
    "temper-goal",
    "temper-seq",
    "temper-branch",
    "temper-pr",
    "temper-status",
];

/// Legacy → canonical aliases. `check_legacy_fields` flags any key in the
/// left column for replacement with the right column's key. NOT the same
/// as the open-meta hyphen aliases (those live in `frontmatter::registry`).
pub static LEGACY_FIELDS: &[(&str, &str)] = &[
    ("id", "temper-id"),
    ("type", "temper-type"),
    ("doc_type", "temper-type"),
    ("context", "temper-context"),
    ("project", "temper-context"),
    // ... copy all 12 entries verbatim from the current schema.rs:81-97 ...
];

/// Managed-tier `temper-*` and `slug` keys that the server writes
/// authoritatively. Stripped from `managed_meta` updates per
/// `meta_service` — clients cannot mutate these.
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
    "slug",
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_fields_contains_expected_keys() {
        assert!(IDENTITY_FIELDS.contains(&"temper-id"));
        assert!(IDENTITY_FIELDS.contains(&"temper-provisional-id"));
    }

    #[test]
    fn tier1_fields_contains_expected_keys() {
        for key in [
            "temper-context",
            "temper-type",
            "temper-created",
            "temper-updated",
            "temper-owner",
            "temper-source",
        ] {
            assert!(TIER1_SYSTEM_FIELDS.contains(&key), "missing key {key}");
        }
    }

    #[test]
    fn known_temper_fields_includes_lifecycle_keys() {
        for key in ["temper-stage", "temper-mode", "temper-effort", "temper-goal"] {
            assert!(KNOWN_TEMPER_FIELDS.contains(&key), "missing key {key}");
        }
    }

    #[test]
    fn legacy_fields_map_id_and_type() {
        assert!(LEGACY_FIELDS.contains(&("id", "temper-id")));
        assert!(LEGACY_FIELDS.contains(&("type", "temper-type")));
    }

    #[test]
    fn system_managed_fields_includes_temper_owner() {
        assert!(SYSTEM_MANAGED_FIELDS.contains(&"temper-owner"));
    }
}
```

**Important — copy `LEGACY_FIELDS` verbatim from current `schema.rs:81-97`.** Don't paraphrase. The exact 12 entries must round-trip byte-identically.

- [ ] **Step 3: Update `schema.rs` to import from the new home**

Delete the three const definitions at lines 57-77 (`KNOWN_TEMPER_FIELDS`), 81-97 (`LEGACY_FIELDS`), and 276-287 (`SYSTEM_MANAGED_FIELDS`).

Add at the top of `schema.rs` (alongside other `use` statements):

```rust
use crate::frontmatter::fields::{
    KNOWN_TEMPER_FIELDS, LEGACY_FIELDS, SYSTEM_MANAGED_FIELDS,
};
```

Update the four internal references (`schema.rs:230, 251, 309, 586`) — they should "just work" because the names are unchanged. Re-run them mentally to confirm: `for (legacy, replacement) in LEGACY_FIELDS` (line 230), `let known: HashSet<&str> = KNOWN_TEMPER_FIELDS.iter().copied().collect()` (line 251), `if !SYSTEM_MANAGED_FIELDS.contains(&key.as_str())` (line 309), `for system_field in SYSTEM_MANAGED_FIELDS` (line 586). All identifiers stay the same.

- [ ] **Step 4: Run the workspace test suite**

Run: `cargo nextest run --workspace`
Expected: all green. Pay attention to `schema_test.rs`, `check_legacy_fields`, `check_unknown_temper_fields`, and the `meta_service::strip_system_managed_fields` tests.

- [ ] **Step 5: `cargo make check`**

Run: `cargo make check`
Expected: clean.

- [ ] **Step 6: Commit**

```bash
git add crates/temper-core/src/schema.rs crates/temper-core/src/frontmatter/fields.rs
git commit -m "chore(frontmatter): move KNOWN_TEMPER_FIELDS/LEGACY_FIELDS/SYSTEM_MANAGED_FIELDS to frontmatter::fields

Completes the frontmatter field-metadata consolidation started in Task 2.
schema.rs imports the constants back from frontmatter::fields, keeping
all four internal call sites byte-identical.

Adds basic smoke tests for the moved constants (presence checks for
the keys most likely to silently disappear in a future refactor)."
```

---

### Task 4: Adopt `DocType::schema_json()` in `schema::load_schema`

**Files:**
- Modify: `crates/temper-core/src/schema.rs:111-124` (replace stringly-typed match with `DocType::schema_json()`)

**Why:** Per `feedback_no_stringly_typed_match` (SG-9). The bounded set of doctypes is owned by `DocType`; matching on `&str` is the wrong abstraction. `DocType::schema_json()` already encapsulates the `include_str!` mapping and is exhaustively matched on the enum.

- [ ] **Step 1: Re-grep `load_schema` to confirm signature and current shape**

Run: `grep -n "fn load_schema" crates/temper-core/src/schema.rs`
Read the function body (~15 lines).

- [ ] **Step 2: Rewrite `load_schema`**

```rust
pub fn load_schema(doc_type: &str) -> Result<Validator> {
    use crate::frontmatter::DocType;

    let dt = DocType::from_str(doc_type)?;
    let doc_schema_str = dt.schema_json();

    // ... rest of the function (compile + return) stays unchanged
}
```

The constant `TASK_SCHEMA`, `GOAL_SCHEMA`, etc. `pub static` items in `schema.rs` should now be **dead code** — they're no longer referenced because `DocType::schema_json()` owns the `include_str!` mapping. Verify with `grep -n "TASK_SCHEMA\|GOAL_SCHEMA\|SESSION_SCHEMA\|RESEARCH_SCHEMA\|DECISION_SCHEMA\|CONCEPT_SCHEMA" crates/temper-core/src/schema.rs` — if there are no other references, delete the `pub static` declarations. If `updatable_fields` and `schema_value` (Tasks 5, 6) still reference them, leave the constants until those tasks land — Tasks 5 and 6 will delete them then.

- [ ] **Step 3: Run the schema tests**

Run: `cargo nextest run -p temper-core schema`
Expected: all green.

- [ ] **Step 4: `cargo make check`**

Run: `cargo make check`
Expected: clean.

- [ ] **Step 5: Commit**

```bash
git add crates/temper-core/src/schema.rs
git commit -m "refactor(schema): adopt DocType::schema_json() in load_schema

Replaces a stringly-typed match doc_type.as_str() with the exhaustive
enum dispatch on DocType. Per the SG-13 cleanup follow-through from
PR #43 review."
```

---

### Task 5: Adopt `DocType::schema_json()` in `schema::updatable_fields`

**Files:**
- Modify: `crates/temper-core/src/schema.rs:291-299` (replace stringly-typed match with `DocType::schema_json()`)

- [ ] **Step 1: Re-grep `updatable_fields` to confirm shape**

Run: `grep -n "fn updatable_fields" crates/temper-core/src/schema.rs`

- [ ] **Step 2: Rewrite the schema-text lookup**

```rust
pub fn updatable_fields(doc_type: &str) -> Result<Vec<(String, serde_json::Value)>> {
    use crate::frontmatter::DocType;

    let dt = DocType::from_str(doc_type)?;
    let schema_str = dt.schema_json();

    // ... rest stays unchanged
}
```

- [ ] **Step 3: Run the schema tests**

Run: `cargo nextest run -p temper-core schema`
Expected: all green.

- [ ] **Step 4: `cargo make check`**

Run: `cargo make check`
Expected: clean.

- [ ] **Step 5: Commit**

```bash
git add crates/temper-core/src/schema.rs
git commit -m "refactor(schema): adopt DocType::schema_json() in updatable_fields"
```

---

### Task 6: Adopt `DocType::schema_json()` in `schema::schema_value` and delete dead schema constants

**Files:**
- Modify: `crates/temper-core/src/schema.rs:402-415` (replace stringly-typed match with `DocType::schema_json()`)
- Modify: `crates/temper-core/src/schema.rs` (delete the now-unused `pub static {TASK,GOAL,SESSION,RESEARCH,DECISION,CONCEPT}_SCHEMA: &str = include_str!(...)` definitions if Tasks 4 and 5 left them stranded)

- [ ] **Step 1: Re-grep `schema_value` to confirm shape**

Run: `grep -n "fn schema_value" crates/temper-core/src/schema.rs`

- [ ] **Step 2: Rewrite the schema-text lookup**

```rust
pub fn schema_value(doc_type: &str) -> Result<serde_json::Value> {
    use crate::frontmatter::DocType;

    let dt = DocType::from_str(doc_type)?;
    let raw = dt.schema_json();

    // ... rest stays unchanged
}
```

- [ ] **Step 3: Delete unused `pub static *_SCHEMA` constants if any remain**

Run:
```bash
grep -n "static TASK_SCHEMA\|static GOAL_SCHEMA\|static SESSION_SCHEMA\|static RESEARCH_SCHEMA\|static DECISION_SCHEMA\|static CONCEPT_SCHEMA" crates/temper-core/src/schema.rs
```

If matches exist, run a second grep to confirm no remaining usages:
```bash
grep -n "TASK_SCHEMA\|GOAL_SCHEMA\|SESSION_SCHEMA\|RESEARCH_SCHEMA\|DECISION_SCHEMA\|CONCEPT_SCHEMA" crates/temper-core/src/schema.rs
```

If the only matches are the definitions themselves, delete them. If anything else references them (e.g. a test that loads the schema text directly), leave the constants and note the spot for a follow-up.

- [ ] **Step 4: Run the schema tests + workspace tests**

Run: `cargo nextest run --workspace`
Expected: all green.

- [ ] **Step 5: `cargo make check`**

Run: `cargo make check`
Expected: clean. `cargo machete` should specifically find no unused includes.

- [ ] **Step 6: Commit**

```bash
git add crates/temper-core/src/schema.rs
git commit -m "refactor(schema): adopt DocType::schema_json() in schema_value; drop dead schema constants

Completes the SG-13 cleanup of the three stringly-typed schema lookup sites
in schema.rs. The pub static *_SCHEMA constants are now redundant — the
DocType::schema_json() variant owns the include_str! mapping as the single
source of truth."
```

---

### Task 7: Migrate `actions/doctor_fix.rs::apply_plan` field-mutation branches to `Frontmatter`

**Files:**
- Modify: `crates/temper-cli/src/actions/doctor_fix.rs:824-901` (the three field-mutation match arms in `apply_plan`)
- Modify: `crates/temper-cli/src/actions/doctor_fix.rs` (delete the `needs_quoting` helper at lines 770-772 — no longer needed because `Frontmatter::serialize` handles all YAML escaping internally)

**Why first in Phase B:** the doctor_fix applicator is the most-tested write path in the CLI (the e2e doctor suite exercises it heavily). Migrating it first proves the Pattern P5 approach against the strongest test signal before touching the less-tested ingest paths.

- [ ] **Step 1: Re-grep `apply_plan` to confirm the three arms are at the expected lines**

Run: `grep -n "FixAction::RenameField\|FixAction::SetField\|FixAction::SetOwnerField" crates/temper-cli/src/actions/doctor_fix.rs`
Expected: 3 match-arm lines around 826, 852, 877 (all inside `apply_plan`).

- [ ] **Step 2: Rewrite the `RenameField` arm**

Replace the existing arm body (lines ~830-849) with:

```rust
FixAction::RenameField {
    path,
    old_key,
    new_key,
} => {
    if !dry_run {
        match temper_core::frontmatter::Frontmatter::parse_file(path) {
            Ok(mut fm) => {
                let mapping = fm
                    .value_mut()
                    .as_mapping_mut()
                    .expect("Frontmatter invariant: value is a mapping");
                let old_yaml_key = serde_yaml::Value::String(old_key.clone());
                let new_yaml_key = serde_yaml::Value::String(new_key.clone());
                let new_exists = mapping.contains_key(&new_yaml_key);
                if new_exists {
                    // The new key is already present — old key wins as a duplicate
                    // and gets removed without copying its value over.
                    mapping.remove(&old_yaml_key);
                } else if let Some(old_value) = mapping.remove(&old_yaml_key) {
                    mapping.insert(new_yaml_key, old_value);
                }
                fm.write_to(path).map_err(|e| {
                    crate::error::TemperError::Vault(format!(
                        "RenameField write {}: {e}",
                        path.display()
                    ))
                })?;
            }
            Err(_) => {
                // File has no parseable frontmatter — silently skip, matching
                // the historical fallback where rename_frontmatter_field was a
                // no-op on un-parseable frontmatter blocks.
            }
        }
    }
    report.fields_renamed += 1;
}
```

- [ ] **Step 3: Rewrite the `SetField` arm**

```rust
FixAction::SetField {
    path, key, value, ..
} => {
    if !dry_run {
        let mut fm = temper_core::frontmatter::Frontmatter::parse_file(path).map_err(|e| {
            crate::error::TemperError::Vault(format!(
                "SetField parse {}: {e}",
                path.display()
            ))
        })?;
        // Heuristic: if the value parses as a plain YAML scalar (number, bool,
        // null, or quoted string), use it as-is; otherwise wrap as a string.
        // Frontmatter::set_managed_field takes serde_json::Value, so use that
        // as the wire format.
        fm.set_managed_field(key, serde_json::Value::String(value.clone()));
        fm.write_to(path).map_err(|e| {
            crate::error::TemperError::Vault(format!(
                "SetField write {}: {e}",
                path.display()
            ))
        })?;
    }
    report.fields_set += 1;
}
```

**Note on the heuristic:** the old `needs_quoting` helper inspected the value for spaces, quotes, and `#` characters and applied YAML escape rules. `Frontmatter::set_managed_field(key, json!(value))` followed by `serialize()` lets `serde_yaml` decide the right scalar form — single-quoted, double-quoted, or unquoted — based on its own rules. This is strictly more correct than the hand-rolled `needs_quoting` because it handles every YAML edge case, not just the three the old helper checked. Verify after the migration with the doctor e2e suite that the resulting YAML is valid for every fixture.

- [ ] **Step 4: Rewrite the `SetOwnerField` arm**

```rust
FixAction::SetOwnerField { path, value } => {
    if !dry_run {
        let mut fm = temper_core::frontmatter::Frontmatter::parse_file(path).map_err(|e| {
            crate::error::TemperError::Vault(format!(
                "SetOwnerField parse {}: {e}",
                path.display()
            ))
        })?;
        fm.set_managed_field("temper-owner", serde_json::Value::String(value.clone()));
        fm.write_to(path).map_err(|e| {
            crate::error::TemperError::Vault(format!(
                "SetOwnerField write {}: {e}",
                path.display()
            ))
        })?;
    }
    report.owner_backfilled += 1;
}
```

- [ ] **Step 5: Delete `needs_quoting` and any unused vault helper imports**

Search the top of `doctor_fix.rs` for `use crate::vault::*` and remove any imports of `parse_frontmatter`, `insert_frontmatter_field`, `rename_frontmatter_field`, `remove_frontmatter_field`, `set_frontmatter_field`. Delete the local `needs_quoting` helper at the top of the file.

- [ ] **Step 6: Run the doctor unit tests + e2e suite**

Run:
```bash
cargo nextest run -p temper-cli doctor_fix
cargo nextest run -p temper-cli doctor
cargo nextest run --workspace --features test-db doctor
```
Expected: all green. The doctor tests are the load-bearing signal here.

- [ ] **Step 7: `cargo make check`**

Run: `cargo make check`
Expected: clean.

- [ ] **Step 8: Commit**

```bash
git add crates/temper-cli/src/actions/doctor_fix.rs
git commit -m "refactor(doctor): migrate doctor_fix::apply_plan to Frontmatter aggregate

Pattern P5: the three field-mutation arms (RenameField, SetField,
SetOwnerField) now go through Frontmatter::parse_file → mutate →
write_to. The old vault::{rename,insert,remove}_frontmatter_field
helpers are no longer called from doctor_fix; Task 14 deletes them
along with the rest of vault.rs's ad-hoc helpers.

Drops the local needs_quoting heuristic — serde_yaml's serializer
handles every YAML escape case correctly, replacing the hand-rolled
three-character check."
```

---

### Task 8: Migrate `ingest::build_frontmatter` and `build_frontmatter_from_resource` to `Frontmatter::new` (Pattern P4)

**Files:**
- Modify: `crates/temper-cli/src/actions/ingest.rs:406-428` (`build_frontmatter`)
- Modify: `crates/temper-cli/src/actions/ingest.rs:483-520` (`build_frontmatter_from_resource`)
- Modify: `crates/temper-cli/src/actions/ingest.rs:430-446` (delete `SKIP_IDENTITY_FIELDS` const — the new path doesn't need a duplicate-key defense because `Frontmatter::set_managed_field` is idempotent on key collisions)
- Modify: `crates/temper-cli/src/actions/ingest.rs:448-474` (delete `emit_meta_tier`)
- Modify: `crates/temper-cli/src/actions/ingest.rs:525-563` (delete `json_value_to_yaml`)
- Modify: `crates/temper-cli/src/actions/ingest.rs:566-570` (delete `yaml_escape_string`)
- Modify: `crates/temper-cli/src/actions/ingest.rs:582-650` (`write_vault_file_and_register` — call sites consume the new API)

**Why grouped:** every helper in lines 430-570 exists solely to support `build_frontmatter` and `build_frontmatter_from_resource`. Migrating both functions to `Frontmatter` deletes the entire helper subgraph in one move. This is the largest single LOC delta in the session.

- [ ] **Step 1: Re-grep all internal call sites within ingest.rs**

```bash
grep -n "build_frontmatter\|emit_meta_tier\|json_value_to_yaml\|yaml_escape_string\|SKIP_IDENTITY_FIELDS" crates/temper-cli/src/actions/ingest.rs
```

Note every line. The migration must update every call site or the helpers can't be deleted.

- [ ] **Step 2: Rewrite `build_frontmatter` to return a populated `Frontmatter`**

Replace the existing function (signature change — it now returns `Result<Frontmatter>` instead of `String`):

```rust
/// Construct a fresh `Frontmatter` for a vault file. Caller can mutate
/// further or write to disk via `Frontmatter::write_to`.
///
/// `extra_fields` allows callers to inject additional managed-tier
/// key-value pairs (e.g. `temper-stage`, `temper-mode`) without bloating
/// the signature.
pub fn build_frontmatter(
    id: impl std::fmt::Display,
    title: &str,
    context: &str,
    doc_type: &str,
    body: String,
    ingestion_source: Option<&str>,
    extra_fields: Option<&[(&str, &str)]>,
) -> Result<temper_core::frontmatter::Frontmatter> {
    use temper_core::frontmatter::{DocType, Frontmatter};

    let dt = DocType::from_str(doc_type)?;
    let now = chrono::Utc::now().to_rfc3339();
    let mut fm = Frontmatter::new(dt, body);
    fm.set_managed_field("temper-id", serde_json::Value::String(id.to_string()));
    fm.set_managed_field("temper-context", serde_json::Value::String(context.to_string()));
    fm.set_managed_field("temper-created", serde_json::Value::String(now));
    fm.set_managed_field("title", serde_json::Value::String(title.to_string()));
    if let Some(source) = ingestion_source {
        fm.set_managed_field("temper-source", serde_json::Value::String(source.to_string()));
    }
    if let Some(fields) = extra_fields {
        for (key, value) in fields {
            fm.set_managed_field(key, serde_json::Value::String(value.to_string()));
        }
    }
    Ok(fm)
}
```

- [ ] **Step 3: Rewrite `build_frontmatter_from_resource` similarly**

```rust
pub fn build_frontmatter_from_resource(
    resource: &temper_core::types::ResourceRow,
    context: &str,
    doc_type: &str,
    body: String,
    managed_meta: Option<&serde_json::Value>,
    open_meta: Option<&serde_json::Value>,
) -> Result<temper_core::frontmatter::Frontmatter> {
    use temper_core::frontmatter::{DocType, Frontmatter};

    let dt = DocType::from_str(doc_type)?;
    let mut fm = Frontmatter::new(dt, body);
    fm.set_managed_field("temper-id", serde_json::Value::String(resource.id.to_string()));
    fm.set_managed_field("temper-context", serde_json::Value::String(context.to_string()));
    fm.set_managed_field("temper-created", serde_json::Value::String(resource.created.to_rfc3339()));
    fm.set_managed_field("title", serde_json::Value::String(resource.title.clone()));
    if let Some(slug) = &resource.slug {
        fm.set_managed_field("slug", serde_json::Value::String(slug.clone()));
    }
    if !resource.owner_handle.is_empty() {
        fm.set_managed_field(
            "temper-owner",
            serde_json::Value::String(resource.owner_handle.clone()),
        );
    }
    if let Some(obj) = managed_meta.and_then(|m| m.as_object()) {
        for (k, v) in obj {
            // System fields are already set above; skip them as defense-in-depth
            // against double-application.
            if temper_core::frontmatter::fields::SYSTEM_MANAGED_FIELDS.contains(&k.as_str()) {
                continue;
            }
            fm.set_managed_field(k, v.clone());
        }
    }
    if let Some(obj) = open_meta.and_then(|m| m.as_object()) {
        for (k, v) in obj {
            fm.set_open_field(k, v.clone());
        }
    }
    Ok(fm)
}
```

- [ ] **Step 4: Update `write_vault_file_and_register` to call the new API**

Find the current invocation (around line 598-607) and replace:

```rust
let fm = build_frontmatter(
    resource.id,
    &resource.title,
    context,
    doc_type,
    content.to_string(),
    ingestion_source,
    extra_fields,
)?;
fm.write_to(&vault_path).map_err(|e| {
    crate::error::TemperError::Vault(format!("write {}: {e}", vault_path.display()))
})?;
```

(No more `format!("{frontmatter}{content}")` — `Frontmatter::write_to` handles it.)

Find any callers of `build_frontmatter_from_resource` (likely in `actions/sync.rs`'s pull path) and update similarly.

- [ ] **Step 5: Delete `SKIP_IDENTITY_FIELDS`, `emit_meta_tier`, `json_value_to_yaml`, `yaml_escape_string`**

These four helpers are now unreferenced. Delete them. Run `grep -n "emit_meta_tier\|json_value_to_yaml\|yaml_escape_string\|SKIP_IDENTITY_FIELDS" crates/temper-cli/src/actions/ingest.rs` after the deletion to confirm zero hits.

- [ ] **Step 6: Run the ingest tests + workspace suite**

```bash
cargo nextest run -p temper-cli ingest
cargo nextest run --workspace
```
Expected: all green. Some ingest unit tests may construct `build_frontmatter`/`build_frontmatter_from_resource` outputs directly and assert on YAML text — they need updating to match canonical output (probably easier to reparse via `Frontmatter::try_from` and assert on `value()`).

- [ ] **Step 7: `cargo make check`**

Run: `cargo make check`
Expected: clean. `cargo machete` should not flag anything new.

- [ ] **Step 8: Commit**

```bash
git add crates/temper-cli/src/actions/ingest.rs
git commit -m "refactor(ingest): migrate build_frontmatter to Frontmatter::new aggregate

Pattern P4: build_frontmatter and build_frontmatter_from_resource now
return a populated Frontmatter that callers write via Frontmatter::write_to.
The hand-rolled YAML emitters (emit_meta_tier, json_value_to_yaml,
yaml_escape_string) and the SKIP_IDENTITY_FIELDS defense-in-depth const
all go away — serde_yaml's serializer inside Frontmatter::serialize()
handles every escape case correctly.

Net delta: ~140 LOC removed, ingest.rs no longer hand-rolls YAML."
```

---

### Task 9: Phase B real-vault byte-diff verification gate

**Files:** None modified — this is a verification-only step.

**Why:** Phase B has now changed two on-disk byte representations: doctor fix's field mutations now produce canonical YAML, and ingest's freshly-written files use `Frontmatter::serialize`. Before moving to Phase C, prove against the real vault that nothing drifted.

- [ ] **Step 1: Build the current branch binary**

Run: `cargo build --bin temper`
Expected: clean build.

- [ ] **Step 2: Capture the current vault state**

```bash
cp -r /Users/petetaylor/projects/kb-vault /tmp/kb-vault-session3-phase-b-pre
```

- [ ] **Step 3: Run `temper doctor` on the real vault (no fix)**

```bash
target/debug/temper doctor 2>&1 | tee /tmp/session3-phase-b-doctor-output.txt
```
Expected: same issue count as on `main` (or fewer; never more).

- [ ] **Step 4: Run `temper doctor fix --dry-run` on the real vault**

```bash
target/debug/temper doctor fix --dry-run 2>&1 | tee /tmp/session3-phase-b-fix-dry.txt
```
Expected: report any fixes that would be applied. For a clean vault, this should be 0.

- [ ] **Step 5: Run `temper doctor fix` (no dry-run) on the real vault**

```bash
target/debug/temper doctor fix 2>&1 | tee /tmp/session3-phase-b-fix-real.txt
```
Expected: same counts as the dry-run.

- [ ] **Step 6: Byte-diff the post-fix vault against the pre-fix copy**

```bash
diff -r /tmp/kb-vault-session3-phase-b-pre /Users/petetaylor/projects/kb-vault > /tmp/session3-phase-b-diff.txt
wc -l /tmp/session3-phase-b-diff.txt
```
Expected: empty diff (zero lines). If non-empty, **stop and investigate** — the migration introduced unexpected drift.

- [ ] **Step 7: Restore the pre-fix vault if any drift surfaced (paranoia)**

If diff is empty: `rm -rf /tmp/kb-vault-session3-phase-b-pre` and proceed to Phase C.
If non-empty: `rm -rf /Users/petetaylor/projects/kb-vault && cp -r /tmp/kb-vault-session3-phase-b-pre /Users/petetaylor/projects/kb-vault` and report the drift in the controller's task tracking.

- [ ] **Step 8: No commit** — this is a verification step. Move to Task 10.

---

### Task 10: Migrate read-only `parse_frontmatter` call sites in `commands/{session,resource}.rs` (Pattern P1)

**Files:**
- Modify: `crates/temper-cli/src/commands/session.rs:158` (`cmd_open`)
- Modify: `crates/temper-cli/src/commands/session.rs:342` (`handle_link_task`)
- Modify: `crates/temper-cli/src/commands/resource.rs:341` (`cmd_edit`)
- Modify: `crates/temper-cli/src/commands/resource.rs:549` (`cmd_open`)

- [ ] **Step 1: Re-grep each site to confirm line numbers**

```bash
grep -n "parse_frontmatter" crates/temper-cli/src/commands/session.rs crates/temper-cli/src/commands/resource.rs
```

- [ ] **Step 2: Migrate each site using Pattern P1**

For each call site, replace:
```rust
let fm = match crate::vault::parse_frontmatter(&content) {
    Some(fm) => fm,
    None => /* error */,
};
let value = fm.get("temper-id").and_then(|v| v.as_str())?;
```

With:
```rust
let fm = temper_core::frontmatter::Frontmatter::try_from(content.as_str())?;
let value = fm
    .value()
    .get("temper-id")
    .and_then(|v| v.as_str())
    .ok_or_else(|| /* same error */)?;
```

Adapt the error mapping at each site to match the existing pattern (some sites use `?`, some return early, some have custom error types).

- [ ] **Step 3: Run the targeted tests**

```bash
cargo nextest run -p temper-cli session
cargo nextest run -p temper-cli resource
```
Expected: all green.

- [ ] **Step 4: `cargo make check`**

Run: `cargo make check`
Expected: clean.

- [ ] **Step 5: Commit**

```bash
git add crates/temper-cli/src/commands/session.rs crates/temper-cli/src/commands/resource.rs
git commit -m "refactor(commands): migrate read-only parse_frontmatter sites in session.rs and resource.rs

Pattern P1. Four call sites in commands/{session,resource}.rs that read
fields from frontmatter without writing back; all migrated to
Frontmatter::try_from + value().get(...)."
```

---

### Task 11: Migrate read-only `parse_frontmatter` call sites in `actions/{doctor,ingest,task,goal,sync,vault}.rs` (Pattern P1)

**Files:**
- Modify: `crates/temper-cli/src/actions/doctor.rs:159` (`scan_file`) — already imports through `temper_core::vault::Vault` for layout, but uses `crate::vault::parse_frontmatter` for content read
- Modify: `crates/temper-cli/src/actions/doctor.rs:413, 420` (`collect_fixes_for_file`)
- Modify: `crates/temper-cli/src/actions/ingest.rs:62` (`parse_source_frontmatter`)
- Modify: `crates/temper-cli/src/actions/task.rs:53` (`cmd_update_status`)
- Modify: `crates/temper-cli/src/actions/goal.rs:47, 214`
- Modify: `crates/temper-cli/src/actions/sync.rs:62` (`do_sync_pull`)
- Modify: `crates/temper-cli/src/actions/vault.rs:12` (`read_document`)

**Note on `actions/doctor.rs:159`:** the `scan_file` function passes the parsed YAML to `check_legacy_fields`, `check_unknown_temper_fields`, and `extract_temper_owner`. After migration, those still take `&serde_yaml::Value` — pass `fm.value()` instead of the old `fm`. **However** — `Frontmatter::try_from` is stricter than `parse_frontmatter`: it requires a present `temper-type` AND a known doctype. For files that fail this stricter check, `scan_file` should still emit a "missing temper-type" or "unknown temper-type" issue rather than aborting. Wrap the parse:

```rust
let fm = match temper_core::frontmatter::Frontmatter::try_from(content.as_str()) {
    Ok(fm) => fm,
    Err(e) => {
        issues.push(ValidationIssue {
            path: String::new(),
            message: format!("frontmatter parse failed: {e}"),
            auto_fixable: false,
        });
        return Ok(ValidationResult { file_path: file_path_str, issues });
    }
};
// pass fm.value() to the existing schema checks
```

The existing fallback that reports "No YAML frontmatter found" should now report the more specific parse error from `Frontmatter::try_from` (which is strictly more informative). Verify the doctor unit tests that assert specific issue messages still pass; if any assert on the old "No YAML frontmatter found" string, update them to match the new message.

- [ ] **Step 1: Re-grep all sites**

```bash
grep -n "parse_frontmatter" crates/temper-cli/src/actions/
```

- [ ] **Step 2: Migrate each site using Pattern P1**

Walk each file in order: `doctor.rs`, `ingest.rs`, `task.rs`, `goal.rs`, `sync.rs`, `vault.rs`. For each call site, apply Pattern P1 with the file-specific error/fallback semantics noted above.

- [ ] **Step 3: Run targeted tests for each modified file**

```bash
cargo nextest run -p temper-cli doctor
cargo nextest run -p temper-cli ingest
cargo nextest run -p temper-cli task
cargo nextest run -p temper-cli goal
cargo nextest run -p temper-cli sync
cargo nextest run --workspace
```
Expected: all green.

- [ ] **Step 4: `cargo make check`**

Run: `cargo make check`
Expected: clean.

- [ ] **Step 5: Commit**

```bash
git add crates/temper-cli/src/actions/
git commit -m "refactor(actions): migrate read-only parse_frontmatter sites to Frontmatter::try_from

Pattern P1. Eight call sites across doctor.rs, ingest.rs, task.rs, goal.rs,
sync.rs, and vault.rs (read_document). The doctor.rs scan_file path now
returns a more specific parse-error message via Frontmatter::try_from
instead of the old generic 'No YAML frontmatter found' fallback."
```

---

### Task 12: Migrate `commands/research.rs` write paths (Pattern P2 + P3)

**Files:**
- Modify: `crates/temper-cli/src/commands/research.rs:31` (`cmd_create` — `replace_body`)
- Modify: `crates/temper-cli/src/commands/research.rs:53, 56` (`cmd_edit` — `set_frontmatter_field` + `replace_body`)

- [ ] **Step 1: Re-read `cmd_create` and `cmd_edit` to understand the current flow**

Run:
```bash
grep -n "replace_body\|set_frontmatter_field" crates/temper-cli/src/commands/research.rs
```

Read the surrounding 30 lines for each match.

- [ ] **Step 2: Migrate `cmd_create`**

Original (sketch):
```rust
let content = fs::read_to_string(path)?;
let updated = vault::replace_body(&content, &new_body);
fs::write(path, updated)?;
```

After:
```rust
let mut fm = temper_core::frontmatter::Frontmatter::parse_file(path)?;
fm.set_body(new_body);
fm.write_to(path)?;
```

- [ ] **Step 3: Migrate `cmd_edit`**

If the original chains `set_frontmatter_field` and `replace_body`:

```rust
let content = fs::read_to_string(path)?;
let with_field = vault::set_frontmatter_field(&content, "key", "value");
let updated = vault::replace_body(&with_field, &new_body);
fs::write(path, updated)?;
```

After:
```rust
let mut fm = temper_core::frontmatter::Frontmatter::parse_file(path)?;
fm.set_managed_field("key", serde_json::Value::String("value".to_string()));
fm.set_body(new_body);
fm.write_to(path)?;
```

(One parse + one write instead of two `fs::write` calls.)

- [ ] **Step 4: Run research command tests**

Run: `cargo nextest run -p temper-cli research`
Expected: all green.

- [ ] **Step 5: `cargo make check`**

Run: `cargo make check`
Expected: clean.

- [ ] **Step 6: Commit**

```bash
git add crates/temper-cli/src/commands/research.rs
git commit -m "refactor(research): migrate cmd_create and cmd_edit write paths to Frontmatter

Patterns P2 + P3. Three call sites (cmd_create's body replacement and
cmd_edit's chained field-set + body replacement) now go through
Frontmatter::parse_file → set_managed_field/set_body → write_to."
```

---

### Task 13: Migrate `commands/session.rs` write paths (Pattern P2 + P3)

**Files:**
- Modify: `crates/temper-cli/src/commands/session.rs:67` (`cmd_create` — `replace_body`)
- Modify: `crates/temper-cli/src/commands/session.rs:89, 93` (`cmd_create` — chained `set_frontmatter_field`)
- Modify: `crates/temper-cli/src/commands/session.rs:210, 218` (`handle_link_task` — `set_frontmatter_field` for `temper-branch` + `temper-stage`)

- [ ] **Step 1: Re-grep and read context**

```bash
grep -n "replace_body\|set_frontmatter_field" crates/temper-cli/src/commands/session.rs
```

- [ ] **Step 2: Migrate `cmd_create`**

The pattern is "create the file, then mutate it twice." Refactor to a single parse + multi-set + single write per file write phase. If the file is being created from scratch, that's a Pattern P4 case (use `Frontmatter::new`); if an existing file is being mutated, Pattern P2 + P3.

- [ ] **Step 3: Migrate `handle_link_task`**

```rust
let mut fm = temper_core::frontmatter::Frontmatter::parse_file(session_path)?;
fm.set_managed_field("temper-branch", serde_json::Value::String(branch.to_string()));
fm.set_managed_field("temper-stage", serde_json::Value::String("in-progress".to_string()));
fm.write_to(session_path)?;
```

- [ ] **Step 4: Run session command tests**

```bash
cargo nextest run -p temper-cli session
```
Expected: all green.

- [ ] **Step 5: `cargo make check`**

Run: `cargo make check`
Expected: clean.

- [ ] **Step 6: Commit**

```bash
git add crates/temper-cli/src/commands/session.rs
git commit -m "refactor(session): migrate cmd_create and handle_link_task to Frontmatter

Patterns P2/P3/P4. Five call sites in session.rs now chain through a single
Frontmatter aggregate per file write rather than re-reading and re-writing
the file once per field mutation."
```

---

### Task 14: Migrate `commands/resource.rs` write paths (Pattern P2)

**Files:**
- Modify: `crates/temper-cli/src/commands/resource.rs:170, 817, 850, 867, 873` (5 `set_frontmatter_field` call sites in `cmd_edit` and `cmd_set_meta`)

- [ ] **Step 1: Re-grep and read context for each site**

```bash
grep -n "set_frontmatter_field" crates/temper-cli/src/commands/resource.rs
```

- [ ] **Step 2: Migrate each site**

Some of these may chain — e.g. `cmd_set_meta` likely mutates multiple fields per invocation. Refactor each function to parse once, mutate all fields, write once.

- [ ] **Step 3: Run resource command tests**

```bash
cargo nextest run -p temper-cli resource
```
Expected: all green.

- [ ] **Step 4: `cargo make check`**

Run: `cargo make check`
Expected: clean.

- [ ] **Step 5: Commit**

```bash
git add crates/temper-cli/src/commands/resource.rs
git commit -m "refactor(resource): migrate cmd_edit and cmd_set_meta to Frontmatter

Pattern P2. Five set_frontmatter_field call sites consolidated into
parse-once + mutate-multiple + write-once flows."
```

---

### Task 15: Migrate `actions/{task,goal}.rs` write paths (Pattern P2)

**Files:**
- Modify: `crates/temper-cli/src/actions/task.rs:259, 272, 275, 279, 283, 287, 323-329` (7+ `set_frontmatter_field` call sites in `cmd_update_*`)
- Modify: `crates/temper-cli/src/actions/goal.rs:180` (`cmd_update_status`)

**Why grouped:** these are the highest-density write paths in the CLI. `actions/task.rs::cmd_update_status` and friends each mutate 3-7 frontmatter fields per invocation; the current code re-reads and re-writes the file once per field. Migrating them to single-pass `Frontmatter` operations is both a correctness win (eliminates intermediate states on disk) and a perf win.

- [ ] **Step 1: Re-grep and read each function**

```bash
grep -n "set_frontmatter_field" crates/temper-cli/src/actions/task.rs crates/temper-cli/src/actions/goal.rs
```

- [ ] **Step 2: Migrate each `cmd_update_*` function in actions/task.rs**

For each function, hoist the file read out of any field-set loop, mutate the `Frontmatter` instance N times, write once at the end. Carefully preserve any conditional logic that decides whether to mutate a given field.

- [ ] **Step 3: Migrate `actions/goal.rs::cmd_update_status`**

```rust
let mut fm = temper_core::frontmatter::Frontmatter::parse_file(path)?;
fm.set_managed_field("temper-stage", serde_json::Value::String(new_stage.to_string()));
fm.write_to(path)?;
```

- [ ] **Step 4: Run task/goal tests**

```bash
cargo nextest run -p temper-cli task
cargo nextest run -p temper-cli goal
```
Expected: all green.

- [ ] **Step 5: `cargo make check`**

Run: `cargo make check`
Expected: clean.

- [ ] **Step 6: Commit**

```bash
git add crates/temper-cli/src/actions/task.rs crates/temper-cli/src/actions/goal.rs
git commit -m "refactor(task,goal): migrate cmd_update_* functions to Frontmatter

Pattern P2. Eight+ set_frontmatter_field call sites across task.rs and
goal.rs consolidated. Each cmd_update_* function now reads + writes the
file exactly once instead of N times for N field mutations."
```

---

### Task 16: Add canonicalization pass to `actions::doctor::fix` and update `ApplyReport` (Pattern P6)

**Files:**
- Modify: `crates/temper-cli/src/actions/doctor_fix.rs` (add `canonicalized: u32` field to `ApplyReport`)
- Modify: `crates/temper-cli/src/actions/doctor.rs::fix` (add the canonicalization pass after `apply_plan` + manifest reconciliation)
- Modify: `crates/temper-cli/src/commands/doctor.rs::run_fix` (include the count in the dim/success line)
- Modify: `crates/temper-cli/src/actions/doctor.rs` (add the `canonicalize_pass` and `canonicalize_one` private functions per Pattern P6)

- [ ] **Step 1: Re-grep `ApplyReport` to find its current shape**

```bash
grep -n "struct ApplyReport\|impl ApplyReport" crates/temper-cli/src/actions/doctor_fix.rs
```

- [ ] **Step 2: Add the `canonicalized` field**

Add to the existing struct (preserve all current fields):

```rust
#[derive(Debug, Default)]
pub struct ApplyReport {
    pub fields_renamed: u32,
    pub fields_set: u32,
    pub owner_backfilled: u32,
    pub files_renamed: u32,
    pub files_relocated: u32,
    pub manifest_updated: u32,
    pub manifest_removed: u32,
    pub canonicalized: u32, // <-- new
}
```

- [ ] **Step 3: Add the canonicalization pass to `actions::doctor::fix`**

Insert the `canonicalize_pass` call after the existing `apply_manifest_actions` + `normalize_all_entries` block, before the final `Ok(report)`. Implement the pass per Pattern P6:

```rust
// After apply_plan, manifest reconciliation, and normalize_all_entries:
canonicalize_pass(config, context_filter, dry_run, &mut report)?;

Ok(report)
```

Add the two private helper functions (`canonicalize_pass` and `canonicalize_one`) at the bottom of `actions/doctor.rs`, following the exact code in Pattern P6.

- [ ] **Step 4: Update `commands/doctor.rs::run_fix` to include the count**

In the `format!` strings around lines 43-56, add `report.canonicalized` to both the dry-run and the success outputs:

Dry-run:
```rust
output::dim(format!(
    "Dry run: would apply {total} fixes ({} field renames, {} fields set, {} file renames, {} relocations, {} manifest updates, {} manifest removals, {} owner backfills, {} canonicalized)",
    report.fields_renamed,
    report.fields_set,
    report.files_renamed,
    report.files_relocated,
    report.manifest_updated,
    report.manifest_removed,
    report.owner_backfilled,
    report.canonicalized,
));
```

Success:
```rust
output::success(format!(
    "Fixed: {} field renames, {} fields set, {} file renames, {} relocations, {} owner backfills, {} canonicalized",
    report.fields_renamed,
    report.fields_set,
    report.files_renamed,
    report.files_relocated,
    report.owner_backfilled,
    report.canonicalized,
));
```

Update the `total` calculation to include `report.canonicalized` so the count tally is right.

- [ ] **Step 5: Write an integration test for the canonicalization pass**

Add a test fixture under `crates/temper-cli/tests/` (or extend an existing doctor test) that:
1. Creates a temp vault with one file containing hyphen-form keys (`relates-to: [foo]`)
2. Runs `temper doctor fix` against it
3. Verifies the file's bytes after-fix are canonical (`relates_to: [foo]` in canonical key order)
4. Runs `temper doctor fix` a second time
5. Verifies `report.canonicalized == 0` on the second run (idempotency)

```rust
#[test]
fn doctor_fix_canonicalizes_hyphen_aliases() {
    let dir = tempfile::tempdir().unwrap();
    let vault_root = dir.path().to_path_buf();
    let task_dir = vault_root.join("@me/temper/task");
    std::fs::create_dir_all(&task_dir).unwrap();
    let file_path = task_dir.join("test.md");
    std::fs::write(
        &file_path,
        r#"---
temper-id: "019d8110-8ff3-70c2-85ae-57e04ed62885"
temper-type: task
temper-context: temper
temper-created: "2026-04-14T00:00:00Z"
title: T
slug: t
relates-to: [foo]
---
body
"#,
    )
    .unwrap();

    let config = /* construct minimal Config pointing at vault_root, context "temper" */;
    let report = crate::actions::doctor::fix(&config, Some("temper"), false).unwrap();
    assert!(report.canonicalized >= 1, "first run should canonicalize");

    let after = std::fs::read_to_string(&file_path).unwrap();
    assert!(after.contains("relates_to: [foo]") || after.contains("relates_to:\n- foo"),
            "hyphen alias must be canonicalized: {after}");
    assert!(!after.contains("relates-to:"), "hyphen-form must be gone");

    let second_report = crate::actions::doctor::fix(&config, Some("temper"), false).unwrap();
    assert_eq!(second_report.canonicalized, 0, "second run must be a no-op");
}
```

- [ ] **Step 6: Run the doctor tests**

```bash
cargo nextest run -p temper-cli doctor
```
Expected: all green, including the new canonicalization integration test.

- [ ] **Step 7: `cargo make check`**

Run: `cargo make check`
Expected: clean.

- [ ] **Step 8: Commit**

```bash
git add crates/temper-cli/src/actions/doctor.rs crates/temper-cli/src/actions/doctor_fix.rs crates/temper-cli/src/commands/doctor.rs
git commit -m "feat(doctor): fold alias canonicalization into doctor fix

Pattern P6. After apply_plan and manifest reconciliation, doctor fix now
walks every vault file and re-saves any whose canonical-form bytes differ
from the on-disk text. Files already in canonical form are skipped (the
parse + serialize is a fixed point for canonical input — Session 1 proved
this).

Folded into doctor fix rather than a separate --fix aliases flag because
users don't need to know about types-of-fix; canonical YAML is the
authoritative on-disk form regardless.

Adds report.canonicalized to ApplyReport with a count in the success/dim
output. Includes an integration test that creates a hyphen-form fixture,
runs doctor fix, verifies canonicalization, and asserts idempotency on a
second run."
```

---

### Task 17: Wire `KNOWN_OPEN_FIELDS` into `temper-api/src/services/meta_service.rs` — **DEFERRED TO SESSION 4**

**STATUS: DEFERRED.** Per controller decision at Session 3 dispatch time, this task moves to Session 4 to keep Session 3 focused on the `temper-cli` frontmatter consolidation. The spec requirement (open-meta key validation server-side) is unchanged — it just happens in the next session, after Session 3 lands and clears the way.

The detailed steps below are preserved for the Session 4 plan to copy verbatim. **Do not execute Task 17 in Session 3.** Skip directly from Task 16 to Task 18.

**Files (when executed in Session 4):**
- Modify: `crates/temper-api/src/services/meta_service.rs` (add `KNOWN_OPEN_FIELDS` import + open-meta key validation in `update_meta`)

**Why:** Per spec scope (Section "Migrations" in Session 3 of the consolidation spec). This is purely additive server-side validation — well-formed clients are unaffected. Catches typo-d open-meta keys at the API boundary instead of letting them silently land in jsonb.

- [ ] **Step 1: Re-read `meta_service.rs::update_meta`**

```bash
grep -n "fn update_meta" crates/temper-api/src/services/meta_service.rs
```

Read the function body and locate the spot just before the SQL UPDATE.

- [ ] **Step 2: Add the import**

```rust
use temper_core::frontmatter::registry::{KNOWN_OPEN_FIELDS, KnownOpenField};
```

- [ ] **Step 3: Add the validation helper**

Below the existing imports / near the top of `meta_service.rs`:

```rust
/// Validate every key in `open_meta` is either a known canonical open field
/// or a known alias. Returns the offending key on first miss.
fn validate_open_meta_keys(open_meta: &serde_json::Value) -> Result<(), String> {
    let Some(obj) = open_meta.as_object() else {
        return Ok(()); // not an object → nothing to validate
    };
    let known: std::collections::HashSet<&str> = KNOWN_OPEN_FIELDS
        .iter()
        .flat_map(|f: &KnownOpenField| {
            std::iter::once(f.canonical).chain(f.aliases.iter().copied())
        })
        .collect();
    for key in obj.keys() {
        if !known.contains(key.as_str()) {
            return Err(key.clone());
        }
    }
    Ok(())
}
```

- [ ] **Step 4: Call the validator in `update_meta` before the UPDATE**

```rust
// Inside update_meta, before the SQL UPDATE:
if let Err(bad_key) = validate_open_meta_keys(&payload.open_meta) {
    return Err(ApiError::BadRequest(format!(
        "unknown open_meta key '{bad_key}'; expected one of: relates_to, depends_on, extends, references, preceded_by, derived_from, parent, tags, aliases, date"
    )));
}
```

(Adapt `ApiError::BadRequest` to whatever the actual error variant is in this crate — re-grep if needed.)

- [ ] **Step 5: Run the temper-api tests**

```bash
cargo nextest run -p temper-api --features test-db meta_service
```
Expected: all green.

- [ ] **Step 6: `cargo make check`**

Run: `cargo make check`
Expected: clean.

- [ ] **Step 7: Commit**

```bash
git add crates/temper-api/src/services/meta_service.rs
git commit -m "feat(meta): validate open_meta keys against KNOWN_OPEN_FIELDS

Server-side validation: meta_service::update_meta now rejects open_meta
payloads with unknown keys at the API boundary, returning a 400 with the
offending key name. Catches typos in CLI/MCP clients before they land in
jsonb storage.

Purely additive — well-formed clients are unaffected. Aliases (relates-to
etc.) are accepted alongside canonical forms (relates_to) since the
KNOWN_OPEN_FIELDS registry is the single source of truth for both."
```

---

### Task 18: Delete `temper-cli/src/vault.rs` ad-hoc helpers

**Files:**
- Modify: `crates/temper-cli/src/vault.rs` (delete `parse_frontmatter`, `set_frontmatter_field`, `rename_frontmatter_field`, `remove_frontmatter_field`, `insert_frontmatter_field`, `replace_body` — keep any other unrelated helpers)

**Prerequisite:** every caller of every one of these six helpers must already be migrated. Verify with grep before deleting.

- [ ] **Step 1: Grep-verify zero remaining production callers**

```bash
grep -rn "vault::parse_frontmatter\|vault::set_frontmatter_field\|vault::rename_frontmatter_field\|vault::remove_frontmatter_field\|vault::insert_frontmatter_field\|vault::replace_body" crates/temper-cli/src crates/temper-core/src crates/temper-api/src
```

Expected: **zero** matches in production code (excluding the definitions themselves and any references in `tests/` or `docs/`). If the grep returns a hit, **stop** — the migration is incomplete. Find the missed call site, file an additional task to migrate it, then return to this task.

Tests in `crates/temper-cli/src/vault.rs::tests` that exercise these helpers can be deleted along with the helpers — Frontmatter has its own coverage for parsing / mutation / serialization.

- [ ] **Step 2: Delete the six functions**

Open `crates/temper-cli/src/vault.rs` and delete the function definitions at lines ~201, ~245, ~262, ~292, ~322, ~354 along with their inline tests.

If any helpers depend on each other (e.g. `remove_frontmatter_field` calling `parse_frontmatter`), the chain falls naturally because nothing outside the file references them.

- [ ] **Step 3: Run the workspace test suite**

```bash
cargo nextest run --workspace --features test-db
```
Expected: all green.

- [ ] **Step 4: `cargo make check`**

Run: `cargo make check`
Expected: clean. `cargo machete` should not flag anything new.

- [ ] **Step 5: Commit**

```bash
git add crates/temper-cli/src/vault.rs
git commit -m "refactor(vault): delete ad-hoc YAML helpers retired by Session 3

Removes parse_frontmatter, set_frontmatter_field, rename_frontmatter_field,
remove_frontmatter_field, insert_frontmatter_field, and replace_body from
temper-cli/src/vault.rs. Every production caller has been migrated to
Frontmatter::parse_file / Frontmatter::write_to / set_managed_field /
set_open_field / set_body in earlier Session 3 tasks.

Closes the consolidation: temper_core::frontmatter::Frontmatter is now the
sole API for vault file parse/mutate/write."
```

---

### Task 19: Final verification + grep guards + real-vault byte-diff against main

**Files:** None modified — verification only.

- [ ] **Step 1: Workspace tests**

```bash
cargo nextest run --workspace
```
Expected: 757+/757+ pass.

- [ ] **Step 2: Test-db tests**

```bash
cargo make docker-up
cargo nextest run --workspace --features test-db
```
Expected: 910+/910+ pass.

- [ ] **Step 3: TypeScript checks**

```bash
cd packages/temper-ui && bun run check
cd ../.. && cargo make ts-test
```
Expected: 0 errors, 0 warnings; vitest 31/31.

- [ ] **Step 4: SQL cache check**

```bash
cargo sqlx prepare --workspace --check
```
Expected: no drift (Session 3 touches zero SQL).

- [ ] **Step 5: `cargo make check`**

Run: `cargo make check`
Expected: clean.

- [ ] **Step 6: Grep guards**

```bash
# Should return zero hits in production code:
grep -rn "split_frontmatter_tiers\|split_frontmatter_block\|compute_frontmatter_hashes_from_yaml" crates/temper-cli/src crates/temper-core/src crates/temper-api/src

# Should return zero hits:
grep -rn "vault::parse_frontmatter\|vault::set_frontmatter_field\|vault::insert_frontmatter_field\|vault::rename_frontmatter_field\|vault::remove_frontmatter_field\|vault::replace_body" crates/temper-cli/src crates/temper-core/src crates/temper-api/src

# Constants should not live in schema.rs / hash.rs anymore:
grep -n "static KNOWN_TEMPER_FIELDS\|static LEGACY_FIELDS\|pub static SYSTEM_MANAGED_FIELDS\|pub const IDENTITY_FIELDS\|pub const TIER1_SYSTEM_FIELDS" crates/temper-core/src/schema.rs crates/temper-core/src/hash.rs
```

Expected: zero hits for all three.

- [ ] **Step 7: Real-vault byte-diff against main**

```bash
# Snapshot the current real vault
cp -r /Users/petetaylor/projects/kb-vault /tmp/kb-vault-session3-final-pre

# Build session 3 binary and run doctor (no fix, just walk + report)
cargo build --bin temper
target/debug/temper doctor 2>&1 | tee /tmp/session3-final-doctor.txt

# Run doctor fix in dry-run mode
target/debug/temper doctor fix --dry-run 2>&1 | tee /tmp/session3-final-fix-dry.txt

# Then run real fix and capture state
target/debug/temper doctor fix 2>&1 | tee /tmp/session3-final-fix.txt

# Diff against the pre-state
diff -r /tmp/kb-vault-session3-final-pre /Users/petetaylor/projects/kb-vault > /tmp/session3-final-diff.txt
wc -l /tmp/session3-final-diff.txt
```

Expected:
- `wc -l /tmp/session3-final-diff.txt` → 0 (empty diff)
- `temper doctor` output: same issue count as on `main`
- `report.canonicalized` in the fix output: 0 (vault is already canonical per Session 2's verification)

If `canonicalized > 0`, **investigate** — Session 2 proved the vault is canonical, so any new canonicalization indicates either:
1. A field has gained an alias-form key in the vault since Session 2 (low risk; just rerun fix)
2. The canonical display ordering has drifted between Session 2 and Session 3 (high risk; bisect)

- [ ] **Step 8: Real-vault byte-diff against main (full comparison)**

```bash
# Stash session 3 work
git stash --include-untracked

# Switch to main and snapshot the doctor output
git checkout main
cargo build --bin temper
target/debug/temper doctor 2>&1 > /tmp/session3-main-doctor.txt

# Compare doctor outputs
diff /tmp/session3-final-doctor.txt /tmp/session3-main-doctor.txt
```

Expected: identical issue counts (any difference indicates Session 3 changed validation semantics).

```bash
# Return to session 3
git checkout jct/frontmatter-consolidation
git stash pop
```

- [ ] **Step 9: No commit** — verification only. Move to session save.

---

## Self-Review

Spec coverage check — every Session 3 spec requirement maps to a task:

| Spec requirement | Task |
|---|---|
| `doctor_fix.rs` YAML write path → `Frontmatter::write_to` | Task 7 |
| `doctor.rs`, `ingest.rs`, `vault.rs` parse paths → `Frontmatter::parse_file` / `try_from` | Tasks 11 (and 12, 13, 14, 15 cover commands/) |
| `temper-api/src/services/meta_service.rs` imports `KNOWN_OPEN_FIELDS` | Task 17 — **deferred to Session 4** |
| Move `KNOWN_TEMPER_FIELDS`, `LEGACY_FIELDS`, `SYSTEM_MANAGED_FIELDS` to `frontmatter::fields` | Task 3 |
| Adopt `DocType::schema_json()` in `schema::{load_schema, updatable_fields, schema_value}` | Tasks 4, 5, 6 |
| Move `IDENTITY_FIELDS`, `TIER1_SYSTEM_FIELDS` from `hash.rs` to `frontmatter::fields` | Task 2 |
| Delete `schema::validate_frontmatter` if no callers remain | Out of scope — `ingest_service.rs` still calls it; documented in plan-reality grounding |
| `temper doctor --fix aliases` command | **Reinterpreted** per user direction — Task 16 folds canonicalization into `doctor fix` instead of a separate `--fix aliases` value |
| Real-vault byte-diff verification | Tasks 9 (Phase B gate) and 19 (final gate) |
| `cargo make check` clean | Every task |
| Full nextest workspace + test-db + e2e green | Task 19 |
| Grep guards | Task 19 |

Type/API consistency check:
- `Frontmatter::new(doc_type: DocType, body: String) -> Self` defined in Task 1, used in Task 8 — signature consistent.
- `Frontmatter::set_body(&mut self, body: String)` defined in Task 1, used in Tasks 12, 13 — signature consistent.
- `Frontmatter::value_mut() -> &mut serde_yaml::Value` already exists per Session 1; used in Task 7's RenameField rewrite.
- `ApplyReport::canonicalized` added in Task 16; consumed in `commands/doctor.rs::run_fix` in the same task.

Placeholder scan: zero "TBD", "TODO", "fill in", or "see Task N for details" — every code block is concrete. Where a task specifies "adapt the error mapping at each site to match the existing pattern," it's because the call sites have heterogeneous error types and the implementer must read each one — but the migration template is given verbatim per Pattern P1.

## Notes for the Controller

- **Session size:** 18 active tasks (Task 17 deferred to Session 4 at dispatch time). Session 2 was 14 tasks + 17 commits. Reasonable energy budget.
- **Task 17 deferred:** The KNOWN_OPEN_FIELDS server-side validation moves to Session 4 to keep Session 3 focused on temper-cli consolidation. Skip directly from Task 16 to Task 18.
- **Subagent dispatch pattern:** Tasks 1, 7, 8, 16 are genuinely new code and warrant fresh-subagent dispatch with two-stage review (per Session 2's pattern for Tasks 1-3). Tasks 2-6 and 10-15 are mechanical migrations and can be controller-direct (per Session 2's pattern for Tasks 4-14). Task 18 is a deletion and Task 19 is verification — both controller-direct.
- **Per-task commits:** Hold the line. Even mechanical tasks get their own commit. Session 2's 17-commit cadence kept bisect useful and made it trivial to roll forward fixture fixes when a test broke ahead of schedule.
- **Real-vault byte-diff is the only acceptance signal that matters.** Tests can pass while sync drifts. Run the diff at Task 9 (Phase B gate) and Task 19 (final gate).
