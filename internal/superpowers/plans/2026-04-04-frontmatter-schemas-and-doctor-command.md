# Frontmatter Schemas & Doctor Command Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Implement JSON Schema validation for temper frontmatter and the `temper doctor` / `temper doctor fix` commands.

**Architecture:** JSON Schema files in `crates/temper-core/schemas/` define the data contract per doctype. A new `schema` module in temper-core loads and validates frontmatter against these schemas. The `temper doctor` command in temper-cli walks the vault, validates each file, and reports issues. `temper doctor fix` applies auto-fixable transformations (legacy field renames, missing field backfills). The existing `normalize` command is preserved but deprecated in favor of doctor fix.

**Tech Stack:** `jsonschema` crate for validation, `serde_yaml` + `serde_json` for frontmatter parsing, `sha2` for hash computation, existing `clap` CLI patterns.

**Spec:** `docs/superpowers/specs/2026-04-04-frontmatter-schemas-and-obsidian-alignment-design.md`

---

## Architectural Constraints

### SRP: commands/ vs actions/ boundary

Follow the existing normalize pattern: `commands/*.rs` files are **thin wrappers** that call into
`actions/*.rs` for business logic and then format the returned data for display. No business logic,
no data construction, no hand-rolled JSON in command files. The command layer's job is:
1. Call the action function
2. Format the result for the user (text or JSON via `Serialize` derives)

### Frontmatter manipulation lives in vault.rs

`vault.rs` already owns `parse_frontmatter`, `set_frontmatter_field`, `slugify`. New frontmatter
operations (`rename_frontmatter_field`, `remove_frontmatter_field`, `insert_frontmatter_field`)
belong there too. `actions/doctor.rs` composes these, it doesn't own them.

### Testability

- **TDD strictly**: write the failing test, run it, implement, run it, commit.
- **Unit tests** in `crates/temper-cli/tests/` for action-level behavior.
- **Vault utility tests** in `crates/temper-cli/tests/vault_test.rs` for new frontmatter functions.
- **Schema tests** in `crates/temper-core/tests/schema_test.rs` for validation logic.
- **E2E tests** in `tests/e2e/tests/doctor_test.rs` using `E2eTestApp` infrastructure.
- Each function does one thing (SG-2). If it scans AND validates AND fixes — split it.

### Subagent Guidance

All subagents executing this plan MUST follow the 10 principles in
`~/.claude/skills/temper/subagent-guidance.md` — especially SG-1 (read sibling files before
writing), SG-2 (single responsibility), SG-4 (tests must actually run), and SG-6 (verify
before claiming done).

## File Structure

### New Files

| File | Responsibility |
|------|---------------|
| `crates/temper-core/schemas/base.schema.json` | Common required fields + optional universals |
| `crates/temper-core/schemas/task.schema.json` | Task-specific fields and constraints |
| `crates/temper-core/schemas/goal.schema.json` | Goal-specific fields |
| `crates/temper-core/schemas/session.schema.json` | Session-specific fields |
| `crates/temper-core/schemas/research.schema.json` | Research-specific fields |
| `crates/temper-core/schemas/decision.schema.json` | Decision-specific fields |
| `crates/temper-core/schemas/concept.schema.json` | Concept-specific fields |
| `crates/temper-core/src/schema.rs` | Schema loading, validation API, hash computation |
| `crates/temper-cli/src/commands/doctor.rs` | Thin CLI wrapper: call actions, format output |
| `crates/temper-cli/src/actions/doctor.rs` | Business logic: vault scan, validate, fix |
| `crates/temper-cli/tests/doctor_test.rs` | Unit tests for doctor scan and fix actions |
| `crates/temper-cli/tests/vault_test.rs` | (extend) Tests for new frontmatter manipulation fns |
| `crates/temper-core/tests/schema_test.rs` | Tests for schema validation |
| `tests/e2e/tests/doctor_test.rs` | E2E tests exercising doctor against real vault setup |

### Modified Files

| File | Change |
|------|--------|
| `crates/temper-core/Cargo.toml` | Add `jsonschema` dependency |
| `crates/temper-core/src/lib.rs` | Add `pub mod schema;` |
| `crates/temper-cli/src/vault.rs` | Add `rename_frontmatter_field`, `remove_frontmatter_field`, `insert_frontmatter_field` |
| `crates/temper-cli/src/cli.rs` | Add `Doctor` command variant |
| `crates/temper-cli/src/main.rs` | Add doctor dispatch arm |
| `crates/temper-cli/src/commands/mod.rs` | Add `pub mod doctor;` |
| `crates/temper-cli/src/actions/mod.rs` | Add `pub mod doctor;` |

---

## Task 1: JSON Schema Files

**Files:**
- Create: `crates/temper-core/schemas/base.schema.json`
- Create: `crates/temper-core/schemas/task.schema.json`
- Create: `crates/temper-core/schemas/goal.schema.json`
- Create: `crates/temper-core/schemas/session.schema.json`
- Create: `crates/temper-core/schemas/research.schema.json`
- Create: `crates/temper-core/schemas/decision.schema.json`
- Create: `crates/temper-core/schemas/concept.schema.json`

- [ ] **Step 1: Create the schemas directory**

```bash
mkdir -p crates/temper-core/schemas
```

- [ ] **Step 2: Write base.schema.json**

This defines the fields common to all doctypes. Relationship fields and universal fields are optional.

```json
{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "$id": "https://temperkb.io/schemas/base.schema.json",
  "type": "object",
  "properties": {
    "temper-id": {
      "type": "string",
      "pattern": "^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$",
      "description": "UUIDv7 unique identifier"
    },
    "temper-type": {
      "type": "string",
      "enum": ["task", "goal", "session", "research", "decision", "concept"],
      "description": "Document type"
    },
    "temper-context": {
      "type": "string",
      "pattern": "^[a-z0-9][a-z0-9-]*$",
      "description": "Context (project) scope"
    },
    "temper-created": {
      "type": "string",
      "description": "RFC3339 creation timestamp"
    },
    "temper-updated": {
      "type": "string",
      "description": "RFC3339 last update timestamp"
    },
    "temper-source": {
      "type": "string",
      "description": "Original ingestion source path or URL"
    },
    "temper-legacy-id": {
      "type": "string",
      "description": "Previous UUID from migration"
    },
    "title": {
      "type": "string",
      "minLength": 1,
      "description": "Display name"
    },
    "tags": {
      "type": "array",
      "items": { "type": "string" },
      "description": "Obsidian-compatible tags"
    },
    "aliases": {
      "type": "array",
      "items": { "type": "string" },
      "description": "Obsidian-compatible alternative names"
    },
    "relates_to": {
      "type": "array",
      "items": { "type": "string" },
      "description": "Related resources (UUIDs, slugs, or context/type/slug paths)"
    },
    "depends_on": {
      "type": "array",
      "items": { "type": "string" },
      "description": "Dependencies (UUIDs, slugs, or context/type/slug paths)"
    },
    "extends": {
      "oneOf": [
        { "type": "string" },
        { "type": "array", "items": { "type": "string" } }
      ],
      "description": "Resources this extends"
    },
    "references": {
      "type": "array",
      "items": { "type": "string" },
      "description": "Referenced resources or external URIs"
    },
    "preceded_by": {
      "oneOf": [
        { "type": "string" },
        { "type": "array", "items": { "type": "string" } }
      ],
      "description": "Resources that precede this in sequence"
    },
    "derived_from": {
      "oneOf": [
        { "type": "string" },
        { "type": "array", "items": { "type": "string" } }
      ],
      "description": "Source resources this was derived from"
    }
  },
  "required": ["temper-id", "temper-type", "temper-context", "temper-created", "title"],
  "additionalProperties": true
}
```

- [ ] **Step 3: Write task.schema.json**

```json
{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "$id": "https://temperkb.io/schemas/task.schema.json",
  "allOf": [
    { "$ref": "base.schema.json" }
  ],
  "properties": {
    "temper-type": { "const": "task" },
    "temper-stage": {
      "type": "string",
      "enum": ["backlog", "in-progress", "done", "cancelled"],
      "description": "Task workflow stage"
    },
    "temper-mode": {
      "type": ["string", "null"],
      "enum": ["plan", "build", null],
      "description": "Work type"
    },
    "temper-effort": {
      "type": ["string", "null"],
      "enum": ["small", "medium", "large", null],
      "description": "Work size estimate"
    },
    "temper-goal": {
      "type": ["string", "null"],
      "description": "Parent goal slug"
    },
    "temper-seq": {
      "type": "integer",
      "minimum": 0,
      "description": "Ordering within goal"
    },
    "temper-branch": {
      "type": ["string", "null"],
      "description": "Git branch name"
    },
    "temper-pr": {
      "type": ["string", "null"],
      "description": "Pull request URL or identifier"
    },
    "slug": {
      "type": "string",
      "pattern": "^[a-z0-9][a-z0-9-]*$",
      "description": "URL-safe identifier"
    }
  },
  "required": ["temper-stage", "slug"],
  "additionalProperties": true
}
```

- [ ] **Step 4: Write goal.schema.json**

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
    "slug": {
      "type": "string",
      "pattern": "^[a-z0-9][a-z0-9-]*$",
      "description": "URL-safe identifier"
    }
  },
  "required": ["slug"],
  "additionalProperties": true
}
```

- [ ] **Step 5: Write session.schema.json**

```json
{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "$id": "https://temperkb.io/schemas/session.schema.json",
  "allOf": [
    { "$ref": "base.schema.json" }
  ],
  "properties": {
    "temper-type": { "const": "session" },
    "date": {
      "type": "string",
      "pattern": "^[0-9]{4}-[0-9]{2}-[0-9]{2}$",
      "description": "Session date (YYYY-MM-DD)"
    }
  },
  "required": ["date"],
  "additionalProperties": true
}
```

- [ ] **Step 6: Write research.schema.json**

```json
{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "$id": "https://temperkb.io/schemas/research.schema.json",
  "allOf": [
    { "$ref": "base.schema.json" }
  ],
  "properties": {
    "temper-type": { "const": "research" },
    "slug": {
      "type": "string",
      "pattern": "^[a-z0-9][a-z0-9-]*$",
      "description": "URL-safe identifier"
    },
    "date": {
      "type": "string",
      "pattern": "^[0-9]{4}-[0-9]{2}-[0-9]{2}$",
      "description": "Research date (YYYY-MM-DD)"
    }
  },
  "required": ["slug", "date"],
  "additionalProperties": true
}
```

- [ ] **Step 7: Write decision.schema.json**

```json
{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "$id": "https://temperkb.io/schemas/decision.schema.json",
  "allOf": [
    { "$ref": "base.schema.json" }
  ],
  "properties": {
    "temper-type": { "const": "decision" },
    "slug": {
      "type": "string",
      "pattern": "^[a-z0-9][a-z0-9-]*$",
      "description": "URL-safe identifier"
    },
    "date": {
      "type": "string",
      "pattern": "^[0-9]{4}-[0-9]{2}-[0-9]{2}$",
      "description": "Decision date (YYYY-MM-DD)"
    }
  },
  "required": ["slug", "date"],
  "additionalProperties": true
}
```

- [ ] **Step 8: Write concept.schema.json**

```json
{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "$id": "https://temperkb.io/schemas/concept.schema.json",
  "allOf": [
    { "$ref": "base.schema.json" }
  ],
  "properties": {
    "temper-type": { "const": "concept" },
    "slug": {
      "type": "string",
      "pattern": "^[a-z0-9][a-z0-9-]*$",
      "description": "URL-safe identifier"
    }
  },
  "required": ["slug"],
  "additionalProperties": true
}
```

- [ ] **Step 9: Commit**

```bash
git add crates/temper-core/schemas/
git commit -m "feat: add JSON Schema definitions for all six temper doctypes"
```

---

## Task 2: Schema Validation Module in temper-core

**Files:**
- Modify: `crates/temper-core/Cargo.toml`
- Modify: `crates/temper-core/src/lib.rs`
- Create: `crates/temper-core/src/schema.rs`
- Create: `crates/temper-core/tests/schema_test.rs`

- [ ] **Step 1: Add jsonschema dependency to temper-core**

In `crates/temper-core/Cargo.toml`, add to `[dependencies]`:

```toml
jsonschema = "0.28"
```

Check the latest version before adding:

```bash
cargo search jsonschema --limit 1
```

- [ ] **Step 2: Write failing test for schema loading**

Create `crates/temper-core/tests/schema_test.rs`:

```rust
use temper_core::schema;

#[test]
fn test_load_schema_for_each_doctype() {
    let doctypes = ["task", "goal", "session", "research", "decision", "concept"];
    for dt in &doctypes {
        let result = schema::load_schema(dt);
        assert!(result.is_ok(), "Failed to load schema for {dt}: {:?}", result.err());
    }
}

#[test]
fn test_load_schema_unknown_doctype_fails() {
    let result = schema::load_schema("unknown");
    assert!(result.is_err());
}
```

- [ ] **Step 3: Run test to verify it fails**

```bash
cargo nextest run -p temper-core test_load_schema
```

Expected: compilation error — `schema` module doesn't exist yet.

- [ ] **Step 4: Add schema module to lib.rs**

In `crates/temper-core/src/lib.rs`, add:

```rust
pub mod schema;
```

- [ ] **Step 5: Write schema.rs with load_schema**

Create `crates/temper-core/src/schema.rs`:

```rust
use std::collections::BTreeMap;

use jsonschema::Validator;
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::error::CoreError;

// Embed schema files at compile time
const BASE_SCHEMA: &str = include_str!("../schemas/base.schema.json");
const TASK_SCHEMA: &str = include_str!("../schemas/task.schema.json");
const GOAL_SCHEMA: &str = include_str!("../schemas/goal.schema.json");
const SESSION_SCHEMA: &str = include_str!("../schemas/session.schema.json");
const RESEARCH_SCHEMA: &str = include_str!("../schemas/research.schema.json");
const DECISION_SCHEMA: &str = include_str!("../schemas/decision.schema.json");
const CONCEPT_SCHEMA: &str = include_str!("../schemas/concept.schema.json");

/// Load and compile the JSON Schema for a given doctype.
pub fn load_schema(doc_type: &str) -> Result<Validator, CoreError> {
    let (schema_str, base_str) = match doc_type {
        "task" => (TASK_SCHEMA, Some(BASE_SCHEMA)),
        "goal" => (GOAL_SCHEMA, Some(BASE_SCHEMA)),
        "session" => (SESSION_SCHEMA, Some(BASE_SCHEMA)),
        "research" => (RESEARCH_SCHEMA, Some(BASE_SCHEMA)),
        "decision" => (DECISION_SCHEMA, Some(BASE_SCHEMA)),
        "concept" => (CONCEPT_SCHEMA, Some(BASE_SCHEMA)),
        _ => {
            return Err(CoreError::Config(format!(
                "Unknown doc type: {doc_type}"
            )));
        }
    };

    let schema_value: Value = serde_json::from_str(schema_str)
        .map_err(|e| CoreError::Config(format!("Invalid schema JSON for {doc_type}: {e}")))?;

    // Build a retriever that resolves $ref to base.schema.json
    if let Some(base) = base_str {
        let base_value: Value = serde_json::from_str(base)
            .map_err(|e| CoreError::Config(format!("Invalid base schema JSON: {e}")))?;

        let validator = jsonschema::options()
            .with_document(
                "https://temperkb.io/schemas/base.schema.json".to_string(),
                base_value,
            )
            .build(&schema_value)
            .map_err(|e| CoreError::Config(format!("Schema compilation error for {doc_type}: {e}")))?;

        Ok(validator)
    } else {
        Validator::new(&schema_value)
            .map_err(|e| CoreError::Config(format!("Schema compilation error for {doc_type}: {e}")))
    }
}

/// A single validation issue found in a vault file.
#[derive(Debug, Clone)]
pub struct ValidationIssue {
    /// JSON Schema validation path (e.g., "/temper-stage")
    pub path: String,
    /// Human-readable description of the issue
    pub message: String,
    /// Whether this issue can be auto-fixed by doctor fix
    pub auto_fixable: bool,
}

/// Result of validating a single file's frontmatter.
#[derive(Debug, Clone)]
pub struct ValidationResult {
    /// Relative path within the vault
    pub file_path: String,
    /// Issues found (empty = valid)
    pub issues: Vec<ValidationIssue>,
}

impl ValidationResult {
    pub fn is_valid(&self) -> bool {
        self.issues.is_empty()
    }
}

/// Validate frontmatter (as serde_yaml::Value) against the schema for the given doctype.
///
/// The frontmatter is converted from YAML Value to JSON Value for jsonschema validation.
/// Returns a list of validation issues (empty if valid).
pub fn validate_frontmatter(
    doc_type: &str,
    frontmatter: &serde_yaml::Value,
) -> Result<Vec<ValidationIssue>, CoreError> {
    let validator = load_schema(doc_type)?;

    // Convert YAML value to JSON value for jsonschema
    let yaml_str = serde_yaml::to_string(frontmatter)
        .map_err(|e| CoreError::Config(format!("YAML serialization error: {e}")))?;
    let json_value: Value = serde_json::from_str(
        &serde_yaml::from_str::<Value>(&yaml_str)
            .and_then(|v| serde_json::to_string(&v).map_err(serde::de::Error::custom))
            .map_err(|e| CoreError::Config(format!("YAML to JSON conversion error: {e}")))?,
    )
    .map_err(|e| CoreError::Config(format!("JSON parse error: {e}")))?;

    let mut issues = Vec::new();

    for error in validator.iter_errors(&json_value) {
        issues.push(ValidationIssue {
            path: error.instance_path.to_string(),
            message: error.to_string(),
            auto_fixable: false,
        });
    }

    Ok(issues)
}

/// Check for legacy field names that should be migrated.
///
/// Returns issues with auto_fixable = true for fields that doctor fix can rename.
pub fn check_legacy_fields(frontmatter: &serde_yaml::Value) -> Vec<ValidationIssue> {
    let legacy_map: &[(&str, &str)] = &[
        ("id", "temper-id"),
        ("type", "temper-type"),
        ("doc_type", "temper-type"),
        ("context", "temper-context"),
        ("project", "temper-context"),
        ("ingestion_source", "temper-source"),
        ("created", "temper-created"),
        ("updated", "temper-updated"),
        ("stage", "temper-stage"),
        ("mode", "temper-mode"),
        ("effort", "temper-effort"),
        ("goal", "temper-goal"),
        ("seq", "temper-seq"),
        ("branch", "temper-branch"),
        ("pr", "temper-pr"),
        ("status", "temper-status"),
        ("legacy_id", "temper-legacy-id"),
    ];

    let mut issues = Vec::new();
    let mapping = match frontmatter.as_mapping() {
        Some(m) => m,
        None => return issues,
    };

    for (old_name, new_name) in legacy_map {
        if mapping.contains_key(&serde_yaml::Value::String(old_name.to_string())) {
            issues.push(ValidationIssue {
                path: format!("/{old_name}"),
                message: format!("Legacy field '{old_name}' should be renamed to '{new_name}'"),
                auto_fixable: true,
            });
        }
    }

    issues
}

/// Check for unrecognized temper-* fields (possible typos).
pub fn check_unknown_temper_fields(frontmatter: &serde_yaml::Value) -> Vec<ValidationIssue> {
    let known_temper_fields: &[&str] = &[
        "temper-id",
        "temper-type",
        "temper-context",
        "temper-created",
        "temper-updated",
        "temper-source",
        "temper-legacy-id",
        "temper-stage",
        "temper-mode",
        "temper-effort",
        "temper-goal",
        "temper-seq",
        "temper-branch",
        "temper-pr",
        "temper-status",
    ];

    let mut issues = Vec::new();
    let mapping = match frontmatter.as_mapping() {
        Some(m) => m,
        None => return issues,
    };

    for (key, _) in mapping {
        if let Some(key_str) = key.as_str() {
            if key_str.starts_with("temper-") && !known_temper_fields.contains(&key_str) {
                issues.push(ValidationIssue {
                    path: format!("/{key_str}"),
                    message: format!("Unrecognized temper field '{key_str}' — possible typo?"),
                    auto_fixable: false,
                });
            }
        }
    }

    issues
}

/// Compute the three-tier hashes for a vault file's frontmatter.
///
/// Returns (meta_hash, open_hash) where:
/// - meta_hash: SHA-256 of sorted temper-* fields as canonical YAML
/// - open_hash: SHA-256 of sorted non-temper-* frontmatter fields as canonical YAML
pub fn compute_frontmatter_hashes(frontmatter: &serde_yaml::Value) -> (String, String) {
    let mapping = match frontmatter.as_mapping() {
        Some(m) => m,
        None => return (empty_hash(), empty_hash()),
    };

    let mut temper_fields = BTreeMap::new();
    let mut open_fields = BTreeMap::new();

    for (key, value) in mapping {
        let key_str = match key.as_str() {
            Some(s) => s,
            None => continue,
        };

        if key_str.starts_with("temper-") {
            temper_fields.insert(key_str.to_string(), value.clone());
        } else {
            open_fields.insert(key_str.to_string(), value.clone());
        }
    }

    let meta_hash = hash_sorted_fields(&temper_fields);
    let open_hash = hash_sorted_fields(&open_fields);

    (meta_hash, open_hash)
}

fn hash_sorted_fields(fields: &BTreeMap<String, serde_yaml::Value>) -> String {
    if fields.is_empty() {
        return empty_hash();
    }

    // Serialize as canonical YAML (BTreeMap is already sorted)
    let yaml_str = serde_yaml::to_string(fields).unwrap_or_default();
    let mut hasher = Sha256::new();
    hasher.update(yaml_str.as_bytes());
    format!("sha256:{:x}", hasher.finalize())
}

fn empty_hash() -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"");
    format!("sha256:{:x}", hasher.finalize())
}
```

- [ ] **Step 6: Run tests to verify they pass**

```bash
cargo nextest run -p temper-core test_load_schema
```

Expected: both tests pass.

- [ ] **Step 7: Write validation tests**

Add to `crates/temper-core/tests/schema_test.rs`:

```rust
#[test]
fn test_validate_valid_task_frontmatter() {
    let yaml: serde_yaml::Value = serde_yaml::from_str(
        r#"
temper-id: "019d5616-8e3c-7432-9867-222b36e46ea1"
temper-type: task
temper-context: temper
temper-created: "2026-04-03T21:23:32.026022-04:00"
temper-stage: in-progress
title: "My task"
slug: my-task
"#,
    )
    .unwrap();

    let issues = schema::validate_frontmatter("task", &yaml).unwrap();
    assert!(issues.is_empty(), "Expected no issues, got: {issues:?}");
}

#[test]
fn test_validate_task_missing_required_field() {
    let yaml: serde_yaml::Value = serde_yaml::from_str(
        r#"
temper-id: "019d5616-8e3c-7432-9867-222b36e46ea1"
temper-type: task
temper-context: temper
temper-created: "2026-04-03T21:23:32.026022-04:00"
title: "My task"
slug: my-task
"#,
    )
    .unwrap();

    // Missing temper-stage (required for task)
    let issues = schema::validate_frontmatter("task", &yaml).unwrap();
    assert!(!issues.is_empty(), "Expected validation issues for missing temper-stage");
}

#[test]
fn test_validate_task_invalid_stage_enum() {
    let yaml: serde_yaml::Value = serde_yaml::from_str(
        r#"
temper-id: "019d5616-8e3c-7432-9867-222b36e46ea1"
temper-type: task
temper-context: temper
temper-created: "2026-04-03T21:23:32.026022-04:00"
temper-stage: active
title: "My task"
slug: my-task
"#,
    )
    .unwrap();

    let issues = schema::validate_frontmatter("task", &yaml).unwrap();
    assert!(
        issues.iter().any(|i| i.message.contains("active")),
        "Expected enum validation error for 'active', got: {issues:?}"
    );
}

#[test]
fn test_validate_valid_session_frontmatter() {
    let yaml: serde_yaml::Value = serde_yaml::from_str(
        r#"
temper-id: "019d5977-f476-7e41-b4aa-fc4bd2b24426"
temper-type: session
temper-context: temper
temper-created: "2026-04-04T12:00:00-04:00"
title: "My session"
date: "2026-04-04"
"#,
    )
    .unwrap();

    let issues = schema::validate_frontmatter("session", &yaml).unwrap();
    assert!(issues.is_empty(), "Expected no issues, got: {issues:?}");
}

#[test]
fn test_validate_additional_properties_preserved() {
    let yaml: serde_yaml::Value = serde_yaml::from_str(
        r#"
temper-id: "019d5616-8e3c-7432-9867-222b36e46ea1"
temper-type: goal
temper-context: temper
temper-created: "2026-04-03T21:23:32.026022-04:00"
title: "My goal"
slug: my-goal
cssclasses:
  - wide
my_custom_field: "hello"
"#,
    )
    .unwrap();

    let issues = schema::validate_frontmatter("goal", &yaml).unwrap();
    assert!(issues.is_empty(), "User fields should not cause validation errors: {issues:?}");
}

#[test]
fn test_check_legacy_fields_detects_old_names() {
    let yaml: serde_yaml::Value = serde_yaml::from_str(
        r#"
id: "019d5616-8e3c-7432-9867-222b36e46ea1"
type: task
context: temper
stage: backlog
title: "Old style task"
"#,
    )
    .unwrap();

    let issues = schema::check_legacy_fields(&yaml);
    assert!(issues.len() >= 3, "Expected at least 3 legacy field issues, got: {issues:?}");
    assert!(issues.iter().all(|i| i.auto_fixable));
}

#[test]
fn test_check_unknown_temper_fields() {
    let yaml: serde_yaml::Value = serde_yaml::from_str(
        r#"
temper-id: "019d5616-8e3c-7432-9867-222b36e46ea1"
temper-type: task
temper-context: temper
temper-stge: backlog
"#,
    )
    .unwrap();

    let issues = schema::check_unknown_temper_fields(&yaml);
    assert_eq!(issues.len(), 1);
    assert!(issues[0].message.contains("temper-stge"));
}

#[test]
fn test_compute_frontmatter_hashes_separates_tiers() {
    let yaml: serde_yaml::Value = serde_yaml::from_str(
        r#"
temper-id: "019d5616-8e3c-7432-9867-222b36e46ea1"
temper-type: task
temper-context: temper
title: "My task"
tags:
  - cli
"#,
    )
    .unwrap();

    let (meta_hash, open_hash) = schema::compute_frontmatter_hashes(&yaml);

    // Both should be non-empty sha256 hashes
    assert!(meta_hash.starts_with("sha256:"), "meta_hash: {meta_hash}");
    assert!(open_hash.starts_with("sha256:"), "open_hash: {open_hash}");

    // Changing a temper field should change meta_hash but not open_hash
    let yaml2: serde_yaml::Value = serde_yaml::from_str(
        r#"
temper-id: "019d5616-8e3c-7432-9867-222b36e46ea1"
temper-type: goal
temper-context: temper
title: "My task"
tags:
  - cli
"#,
    )
    .unwrap();

    let (meta_hash2, open_hash2) = schema::compute_frontmatter_hashes(&yaml2);
    assert_ne!(meta_hash, meta_hash2, "meta_hash should change when temper-type changes");
    assert_eq!(open_hash, open_hash2, "open_hash should NOT change when only temper fields change");
}
```

- [ ] **Step 8: Run all schema tests**

```bash
cargo nextest run -p temper-core schema_test
```

Expected: all tests pass.

- [ ] **Step 9: Commit**

```bash
git add crates/temper-core/Cargo.toml crates/temper-core/src/lib.rs crates/temper-core/src/schema.rs crates/temper-core/tests/schema_test.rs
git commit -m "feat: add schema validation module with load, validate, legacy check, and hash computation"
```

---

## Task 3: Doctor Command — CLI Wiring

**Files:**
- Modify: `crates/temper-cli/src/cli.rs`
- Modify: `crates/temper-cli/src/main.rs`
- Modify: `crates/temper-cli/src/commands/mod.rs`
- Create: `crates/temper-cli/src/commands/doctor.rs`

- [ ] **Step 1: Add Doctor to Commands enum**

In `crates/temper-cli/src/cli.rs`, add to the `Commands` enum (after the `Normalize` variant):

```rust
    /// Validate vault frontmatter and repair drift
    Doctor {
        #[command(subcommand)]
        action: Option<DoctorAction>,
        /// Filter by context
        #[arg(long)]
        context: Option<String>,
        /// Output format (text or json)
        #[arg(long, default_value = "text")]
        format: String,
    },
```

Add the subcommand enum at the end of the file:

```rust
#[derive(Subcommand)]
pub enum DoctorAction {
    /// Auto-fix issues (rename legacy fields, backfill missing fields)
    Fix {
        /// Preview fixes without writing (dry run)
        #[arg(long)]
        dry_run: bool,
    },
}
```

- [ ] **Step 2: Add doctor module to commands/mod.rs**

In `crates/temper-cli/src/commands/mod.rs`, add:

```rust
pub mod doctor;
```

- [ ] **Step 3: Create commands/doctor.rs stub**

Create `crates/temper-cli/src/commands/doctor.rs`:

```rust
use crate::config::Config;
use crate::error::Result;

/// Run doctor (validate only).
pub fn run(config: &Config, context: Option<&str>, format: &str) -> Result<()> {
    let _ = (config, context, format);
    crate::output::plain("temper doctor: not yet implemented");
    Ok(())
}

/// Run doctor fix (validate + auto-fix).
pub fn run_fix(config: &Config, context: Option<&str>, dry_run: bool) -> Result<()> {
    let _ = (config, context, dry_run);
    crate::output::plain("temper doctor fix: not yet implemented");
    Ok(())
}
```

- [ ] **Step 4: Add dispatch arm in main.rs**

In `crates/temper-cli/src/main.rs`, find the match on `cli.command` and add:

```rust
        Commands::Doctor {
            action,
            context,
            format,
        } => {
            let config = load_config(cli.vault.as_deref())?;
            match action {
                Some(cli::DoctorAction::Fix { dry_run }) => {
                    commands::doctor::run_fix(&config, context.as_deref(), dry_run)?;
                }
                None => {
                    commands::doctor::run(&config, context.as_deref(), &format)?;
                }
            }
        }
```

- [ ] **Step 5: Verify it compiles and runs**

```bash
cargo build -p temper-cli 2>&1 | tail -5
cargo run -p temper-cli -- doctor 2>&1
cargo run -p temper-cli -- doctor fix --dry-run 2>&1
```

Expected: compiles, prints stub messages.

- [ ] **Step 6: Commit**

```bash
git add crates/temper-cli/src/cli.rs crates/temper-cli/src/main.rs crates/temper-cli/src/commands/mod.rs crates/temper-cli/src/commands/doctor.rs
git commit -m "feat: add temper doctor / doctor fix CLI command skeleton"
```

---

## Task 4: Doctor Action — Vault Scan and Validation

**Files:**
- Modify: `crates/temper-cli/src/actions/mod.rs`
- Create: `crates/temper-cli/src/actions/doctor.rs`
- Modify: `crates/temper-cli/src/commands/doctor.rs`
- Create: `crates/temper-cli/tests/doctor_test.rs`

- [ ] **Step 1: Write failing test for doctor scan**

Create `crates/temper-cli/tests/doctor_test.rs`:

```rust
use std::fs;
use tempfile::TempDir;

fn test_config(dir: &TempDir) -> temper_cli::config::Config {
    let vault_root = dir.path().to_path_buf();
    let state_dir = dir.path().join(".temper");
    fs::create_dir_all(&state_dir).unwrap();
    fs::write(state_dir.join("manifest.json"), "{}\n").unwrap();
    fs::write(state_dir.join("events.jsonl"), "").unwrap();
    temper_cli::config::Config {
        vault_root,
        state_dir,
        contexts: vec!["temper".to_string()],
        ..Default::default()
    }
}

fn write_vault_file(dir: &TempDir, rel_path: &str, content: &str) {
    let path = dir.path().join(rel_path);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, content).unwrap();
}

#[test]
fn test_doctor_valid_task_no_issues() {
    let dir = TempDir::new().unwrap();
    let config = test_config(&dir);

    write_vault_file(
        &dir,
        "temper/task/my-task.md",
        r#"---
temper-id: "019d5616-8e3c-7432-9867-222b36e46ea1"
temper-type: task
temper-context: temper
temper-created: "2026-04-03T21:23:32.026022-04:00"
temper-stage: backlog
title: "My task"
slug: my-task
---

# My task
"#,
    );

    let report = temper_cli::actions::doctor::scan(&config, None).unwrap();
    assert_eq!(report.files_checked, 1);
    assert_eq!(report.total_issues, 0);
}

#[test]
fn test_doctor_detects_legacy_fields() {
    let dir = TempDir::new().unwrap();
    let config = test_config(&dir);

    write_vault_file(
        &dir,
        "temper/task/old-task.md",
        r#"---
id: "019d5616-8e3c-7432-9867-222b36e46ea1"
type: task
context: temper
stage: backlog
title: "Old task"
slug: old-task
---

# Old task
"#,
    );

    let report = temper_cli::actions::doctor::scan(&config, None).unwrap();
    assert_eq!(report.files_checked, 1);
    assert!(report.total_issues > 0, "Should detect legacy fields");
    assert!(report.auto_fixable > 0, "Legacy fields should be auto-fixable");
}

#[test]
fn test_doctor_detects_invalid_enum() {
    let dir = TempDir::new().unwrap();
    let config = test_config(&dir);

    write_vault_file(
        &dir,
        "temper/task/bad-stage.md",
        r#"---
temper-id: "019d5616-8e3c-7432-9867-222b36e46ea1"
temper-type: task
temper-context: temper
temper-created: "2026-04-03T21:23:32.026022-04:00"
temper-stage: active
title: "Bad stage"
slug: bad-stage
---

# Bad stage
"#,
    );

    let report = temper_cli::actions::doctor::scan(&config, None).unwrap();
    assert!(report.total_issues > 0, "Should detect invalid stage enum");
}

#[test]
fn test_doctor_valid_session_no_issues() {
    let dir = TempDir::new().unwrap();
    let config = test_config(&dir);

    write_vault_file(
        &dir,
        "temper/session/2026-04-04 — my-session.md",
        r#"---
temper-id: "019d5977-f476-7e41-b4aa-fc4bd2b24426"
temper-type: session
temper-context: temper
temper-created: "2026-04-04T12:00:00-04:00"
title: "My session"
date: "2026-04-04"
---

## Goal
"#,
    );

    let report = temper_cli::actions::doctor::scan(&config, None).unwrap();
    assert_eq!(report.files_checked, 1);
    assert_eq!(report.total_issues, 0);
}

#[test]
fn test_doctor_scans_multiple_doctypes() {
    let dir = TempDir::new().unwrap();
    let config = test_config(&dir);

    write_vault_file(
        &dir,
        "temper/task/a-task.md",
        r#"---
temper-id: "019d5616-8e3c-7432-9867-222b36e46ea1"
temper-type: task
temper-context: temper
temper-created: "2026-04-03T21:23:32-04:00"
temper-stage: backlog
title: "A task"
slug: a-task
---
"#,
    );

    write_vault_file(
        &dir,
        "temper/goal/a-goal.md",
        r#"---
temper-id: "019d5038-ce94-7661-8869-8711545e9678"
temper-type: goal
temper-context: temper
temper-created: "2026-04-02T22:03:13+00:00"
title: "A goal"
slug: a-goal
---
"#,
    );

    let report = temper_cli::actions::doctor::scan(&config, None).unwrap();
    assert_eq!(report.files_checked, 2);
}
```

- [ ] **Step 2: Run test to verify it fails**

```bash
cargo nextest run -p temper-cli test_doctor
```

Expected: compilation error — `actions::doctor` doesn't exist yet.

- [ ] **Step 3: Add doctor module to actions/mod.rs**

In `crates/temper-cli/src/actions/mod.rs`, add:

```rust
pub mod doctor;
```

- [ ] **Step 4: Write actions/doctor.rs — DoctorReport type and scan function**

Create `crates/temper-cli/src/actions/doctor.rs`:

```rust
use std::fs;
use std::path::Path;

use temper_core::schema;

use crate::config::Config;
use crate::error::Result;
use crate::vault;

/// Doc types to scan. Must match the schema file set.
const SCAN_DOC_TYPES: &[&str] = &["task", "goal", "session", "research", "decision", "concept"];

/// Summary report from a doctor scan.
#[derive(Debug, Clone)]
pub struct DoctorReport {
    pub files_checked: u32,
    pub total_issues: u32,
    pub auto_fixable: u32,
    pub file_results: Vec<schema::ValidationResult>,
}

/// Scan all vault files and validate frontmatter against schemas.
pub fn scan(config: &Config, context_filter: Option<&str>) -> Result<DoctorReport> {
    let mut report = DoctorReport {
        files_checked: 0,
        total_issues: 0,
        auto_fixable: 0,
        file_results: Vec::new(),
    };

    let contexts: Vec<String> = if let Some(ctx) = context_filter {
        vec![ctx.to_string()]
    } else {
        config.contexts.clone()
    };

    for ctx in &contexts {
        for doc_type in SCAN_DOC_TYPES {
            let dir = config.vault_root.join(ctx).join(doc_type);
            if !dir.is_dir() {
                continue;
            }
            scan_directory(&dir, doc_type, &mut report)?;
        }
    }

    // Also scan research directory (separate structure: research/<context>/*.md)
    let research_dir = config.vault_root.join("research");
    if research_dir.is_dir() {
        scan_research_dir(&research_dir, context_filter, &mut report)?;
    }

    Ok(report)
}

fn scan_research_dir(
    research_dir: &Path,
    context_filter: Option<&str>,
    report: &mut DoctorReport,
) -> Result<()> {
    let subdirs: Vec<_> = fs::read_dir(research_dir)?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .collect();

    for subdir in subdirs {
        if let Some(filter) = context_filter {
            let dir_name = subdir.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if dir_name != filter {
                continue;
            }
        }
        scan_directory(&subdir, "research", report)?;
    }

    Ok(())
}

fn scan_directory(dir: &Path, doc_type: &str, report: &mut DoctorReport) -> Result<()> {
    let md_files: Vec<_> = fs::read_dir(dir)?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|ext| ext == "md"))
        .collect();

    for file_path in md_files {
        let content = fs::read_to_string(&file_path)?;
        let rel_path = file_path.display().to_string();

        let fm = match vault::parse_frontmatter(&content) {
            Some(fm) => fm,
            None => {
                report.files_checked += 1;
                let result = schema::ValidationResult {
                    file_path: rel_path,
                    issues: vec![schema::ValidationIssue {
                        path: String::new(),
                        message: "No YAML frontmatter found".to_string(),
                        auto_fixable: false,
                    }],
                };
                report.total_issues += 1;
                report.file_results.push(result);
                continue;
            }
        };

        let mut issues = Vec::new();

        // 1. Check for legacy field names (auto-fixable)
        let legacy_issues = schema::check_legacy_fields(&fm);
        issues.extend(legacy_issues);

        // 2. Determine the effective doc type for schema validation
        let effective_type = detect_doc_type(&fm, doc_type);

        // 3. Validate against schema (if we can determine the type)
        if let Some(ref dt) = effective_type {
            match schema::validate_frontmatter(dt, &fm) {
                Ok(schema_issues) => issues.extend(schema_issues),
                Err(e) => {
                    issues.push(schema::ValidationIssue {
                        path: String::new(),
                        message: format!("Schema validation error: {e}"),
                        auto_fixable: false,
                    });
                }
            }
        }

        // 4. Check for unknown temper-* fields
        let unknown_issues = schema::check_unknown_temper_fields(&fm);
        issues.extend(unknown_issues);

        report.files_checked += 1;
        report.auto_fixable += issues.iter().filter(|i| i.auto_fixable).count() as u32;
        report.total_issues += issues.len() as u32;

        report.file_results.push(schema::ValidationResult {
            file_path: rel_path,
            issues,
        });
    }

    Ok(())
}

/// Detect the document type from frontmatter, falling back to directory-inferred type.
///
/// Checks both new (`temper-type`) and legacy (`type`, `doc_type`) field names.
fn detect_doc_type(fm: &serde_yaml::Value, dir_doc_type: &str) -> Option<String> {
    // Try temper-type first (new format)
    if let Some(dt) = fm.get("temper-type").and_then(|v| v.as_str()) {
        return Some(dt.to_string());
    }
    // Try legacy field names
    if let Some(dt) = fm.get("type").and_then(|v| v.as_str()) {
        return Some(dt.to_string());
    }
    if let Some(dt) = fm.get("doc_type").and_then(|v| v.as_str()) {
        return Some(dt.to_string());
    }
    // Fall back to directory-inferred type
    Some(dir_doc_type.to_string())
}
```

- [ ] **Step 5: Run tests**

```bash
cargo nextest run -p temper-cli test_doctor
```

Expected: all 5 doctor tests pass.

- [ ] **Step 6: Commit**

```bash
git add crates/temper-cli/src/actions/mod.rs crates/temper-cli/src/actions/doctor.rs crates/temper-cli/tests/doctor_test.rs
git commit -m "feat: add doctor scan action with schema validation, legacy check, and unknown field detection"
```

---

## Task 5: Vault Frontmatter Utilities — rename, remove, insert

New frontmatter manipulation functions belong in `vault.rs` alongside the existing
`parse_frontmatter` and `set_frontmatter_field`. Test-first in `vault_test.rs`.

**Files:**
- Modify: `crates/temper-cli/src/vault.rs`
- Modify: `crates/temper-cli/tests/vault_test.rs`

- [ ] **Step 1: Write failing tests for new vault functions**

Add to `crates/temper-cli/tests/vault_test.rs`:

```rust
#[test]
fn test_rename_frontmatter_field() {
    let content = "---\nid: \"abc-123\"\ntype: task\ntitle: \"Hello\"\n---\n\n# Hello\n";
    let result = temper_cli::vault::rename_frontmatter_field(content, "id", "temper-id");
    assert!(result.contains("temper-id: \"abc-123\""), "got:\n{result}");
    assert!(!result.contains("\nid:"), "old key should be gone");
    assert!(result.contains("# Hello"), "body preserved");
}

#[test]
fn test_rename_frontmatter_field_preserves_body() {
    let content = "---\nstage: backlog\n---\n\nSome body with stage: info here.\n";
    let result = temper_cli::vault::rename_frontmatter_field(content, "stage", "temper-stage");
    assert!(result.contains("temper-stage: backlog"));
    assert!(result.contains("stage: info here"), "body line with 'stage:' should not be renamed");
}

#[test]
fn test_remove_frontmatter_field() {
    let content = "---\nid: \"abc\"\ntype: task\ntitle: \"Hello\"\n---\n\n# Hello\n";
    let result = temper_cli::vault::remove_frontmatter_field(content, "type");
    assert!(!result.contains("\ntype:"), "field should be removed");
    assert!(result.contains("id: \"abc\""), "other fields preserved");
    assert!(result.contains("# Hello"), "body preserved");
}

#[test]
fn test_insert_frontmatter_field() {
    let content = "---\ntitle: \"Hello\"\n---\n\n# Hello\n";
    let result = temper_cli::vault::insert_frontmatter_field(content, "temper-id", "\"new-uuid\"");
    assert!(result.contains("temper-id: \"new-uuid\""));
    // New field should appear before existing fields (after opening ---)
    let id_pos = result.find("temper-id").unwrap();
    let title_pos = result.find("title:").unwrap();
    assert!(id_pos < title_pos, "inserted field should be at top of frontmatter");
}
```

- [ ] **Step 2: Run tests to verify they fail**

```bash
cargo nextest run -p temper-cli test_rename_frontmatter_field test_remove_frontmatter_field test_insert_frontmatter_field
```

Expected: compilation error — functions don't exist yet.

- [ ] **Step 3: Implement the three functions in vault.rs**

Add to `crates/temper-cli/src/vault.rs`:

```rust
/// Rename a frontmatter field key, preserving the value. Only operates within
/// the YAML frontmatter block (between the first pair of `---` delimiters).
pub fn rename_frontmatter_field(content: &str, old_key: &str, new_key: &str) -> String {
    let mut result = Vec::new();
    let mut in_frontmatter = false;
    let mut fm_count = 0;

    for line in content.lines() {
        if line.trim() == "---" {
            fm_count += 1;
            in_frontmatter = fm_count == 1;
            result.push(line.to_string());
            continue;
        }

        if in_frontmatter && line.starts_with(&format!("{old_key}:")) {
            let value_part = &line[old_key.len()..];
            result.push(format!("{new_key}{value_part}"));
        } else {
            result.push(line.to_string());
        }
    }

    result.join("\n") + if content.ends_with('\n') { "\n" } else { "" }
}

/// Remove a frontmatter field entirely. Only operates within the YAML
/// frontmatter block.
pub fn remove_frontmatter_field(content: &str, key: &str) -> String {
    let mut result = Vec::new();
    let mut in_frontmatter = false;
    let mut fm_count = 0;

    for line in content.lines() {
        if line.trim() == "---" {
            fm_count += 1;
            in_frontmatter = fm_count == 1;
            result.push(line.to_string());
            continue;
        }

        if in_frontmatter && line.starts_with(&format!("{key}:")) {
            continue;
        }

        result.push(line.to_string());
    }

    result.join("\n") + if content.ends_with('\n') { "\n" } else { "" }
}

/// Insert a new field at the top of frontmatter (after the opening `---`).
pub fn insert_frontmatter_field(content: &str, key: &str, value: &str) -> String {
    if let Some(pos) = content.find("---\n") {
        let insert_pos = pos + 4;
        let mut result = content[..insert_pos].to_string();
        result.push_str(&format!("{key}: {value}\n"));
        result.push_str(&content[insert_pos..]);
        result
    } else {
        content.to_string()
    }
}
```

- [ ] **Step 4: Run tests**

```bash
cargo nextest run -p temper-cli test_rename_frontmatter test_remove_frontmatter test_insert_frontmatter
```

Expected: all 4 tests pass.

- [ ] **Step 5: Commit**

```bash
git add crates/temper-cli/src/vault.rs crates/temper-cli/tests/vault_test.rs
git commit -m "feat: add rename, remove, insert frontmatter field utilities to vault.rs"
```

---

## Task 6: Doctor Command — Thin CLI Wrapper and Output

The command layer follows the normalize pattern: call actions, format returned data.
`DoctorReport` and `FixReport` derive `Serialize` so JSON output is just
`serde_json::to_string_pretty`. No hand-built JSON, no raw `println!`/`eprintln!`.

**All terminal output goes through `crate::output::*` functions** (which use `anstream` +
`anstyle` for auto-detecting terminal capabilities and graceful degradation). Read
`crates/temper-cli/src/output/mod.rs` and `crates/temper-cli/src/output/styles.rs` before
writing any output code. Available styled output functions:

- `output::success(msg)` — green ✓ prefix
- `output::error(msg)` — red ✗ prefix (stderr)
- `output::warning(msg)` — yellow ! prefix (stderr)
- `output::header(msg)` — bold
- `output::label(name, value)` — bold label + value
- `output::hint(msg)` — dimmed guidance
- `output::status_icon(bool, msg)` — ✓/✗ based on health
- `output::item(msg)` — bullet point
- `output::plain(msg)` — unstyled
- `output::dim(msg)` — muted secondary info

**indicatif** is available in Cargo.toml (machete-excluded, not yet used anywhere). The doctor
scan is a good candidate for a progress bar when scanning large vaults. If using it, follow
`indicatif` patterns that compose with `anstream` (both use the same terminal detection).
This is optional — don't block the core functionality on it, but wire it up if time allows.

**Files:**
- Modify: `crates/temper-cli/src/commands/doctor.rs`
- Modify: `crates/temper-core/src/schema.rs` (add Serialize derives)
- Modify: `crates/temper-cli/src/actions/doctor.rs` (add Serialize derives)

- [ ] **Step 1: Add Serialize derives to report types**

In `crates/temper-core/src/schema.rs`, add `Serialize` to `ValidationIssue` and `ValidationResult`:

```rust
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct ValidationIssue { ... }

#[derive(Debug, Clone, Serialize)]
pub struct ValidationResult { ... }
```

In `crates/temper-cli/src/actions/doctor.rs`, add `Serialize` to `DoctorReport`:

```rust
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct DoctorReport { ... }
```

- [ ] **Step 2: Replace the command stub with thin wrapper**

Replace `crates/temper-cli/src/commands/doctor.rs`. Note: ALL output goes through
`crate::output::*` — never raw `println!`. JSON output uses `output::plain` with
`serde_json::to_string_pretty`.

```rust
use crate::actions::doctor;
use crate::config::Config;
use crate::error::Result;
use crate::output;

/// Run doctor (validate only). Delegates to actions::doctor::scan for all logic.
pub fn run(config: &Config, context: Option<&str>, format: &str) -> Result<()> {
    let report = doctor::scan(config, context)?;

    if format == "json" {
        output::plain(serde_json::to_string_pretty(&report).unwrap_or_default());
        return Ok(());
    }

    if report.total_issues == 0 {
        output::success(format!("{} files checked — no issues found", report.files_checked));
        return Ok(());
    }

    print_issues(&report);
    print_summary(&report);

    Ok(())
}

/// Run doctor fix (validate + auto-fix). Delegates to actions::doctor for all logic.
pub fn run_fix(config: &Config, context: Option<&str>, dry_run: bool) -> Result<()> {
    let report = doctor::scan(config, context)?;

    if report.auto_fixable == 0 {
        if report.total_issues == 0 {
            output::success(format!("{} files checked — no issues found", report.files_checked));
        } else {
            output::warning(format!(
                "{} issues found but none are auto-fixable. Run `temper doctor` for details.",
                report.total_issues
            ));
        }
        return Ok(());
    }

    let fixed = doctor::fix(config, context, dry_run)?;

    if dry_run {
        output::dim(format!(
            "Dry run: would fix {} issues across {} files",
            fixed.fields_renamed + fixed.fields_backfilled, fixed.files_modified,
        ));
    } else {
        output::success(format!(
            "Fixed {} files: {} fields renamed, {} fields backfilled",
            fixed.files_modified, fixed.fields_renamed, fixed.fields_backfilled,
        ));
    }

    let remaining = report.total_issues - report.auto_fixable;
    if remaining > 0 {
        output::hint(format!(
            "{remaining} issues require manual attention. Run `temper doctor` for details."
        ));
    }

    Ok(())
}

fn print_issues(report: &doctor::DoctorReport) {
    for result in &report.file_results {
        if result.issues.is_empty() {
            continue;
        }
        output::header(&result.file_path);
        for issue in &result.issues {
            let fixable_tag = if issue.auto_fixable { " [auto-fixable]" } else { "" };
            let path_tag = if issue.path.is_empty() {
                String::new()
            } else {
                format!(" {}", issue.path)
            };
            if issue.auto_fixable {
                output::warning(format!("{path_tag}: {}{fixable_tag}", issue.message));
            } else {
                output::error(format!("{path_tag}: {}", issue.message));
            }
        }
        output::blank();
    }
}

fn print_summary(report: &doctor::DoctorReport) {
    output::label("Checked", report.files_checked);
    output::label("Issues", format!(
        "{} ({} auto-fixable, {} manual)",
        report.total_issues, report.auto_fixable,
        report.total_issues - report.auto_fixable,
    ));
    if report.auto_fixable > 0 {
        output::hint("Run `temper doctor fix` to auto-fix, or `temper doctor fix --dry-run` to preview.");
    }
}
```

- [ ] **Step 3: Verify compilation**

```bash
cargo build -p temper-cli 2>&1 | tail -5
```

Expected: compiles cleanly.

- [ ] **Step 4: Commit**

```bash
git add crates/temper-core/src/schema.rs crates/temper-cli/src/commands/doctor.rs crates/temper-cli/src/actions/doctor.rs
git commit -m "feat: thin doctor command wrapper with styled output and Serialize-derived JSON"
```

---

## Task 7: Doctor Fix — Auto-Fix Action Logic

This task adds the `fix` function to `actions/doctor.rs`. It composes the vault.rs
frontmatter utilities (from Task 5) — no frontmatter manipulation logic lives here.

**Files:**
- Modify: `crates/temper-cli/src/actions/doctor.rs`
- Modify: `crates/temper-cli/tests/doctor_test.rs`

- [ ] **Step 1: Write failing tests for doctor fix**

Add to `crates/temper-cli/tests/doctor_test.rs`:

```rust
#[test]
fn test_doctor_fix_renames_legacy_fields() {
    let dir = TempDir::new().unwrap();
    let config = test_config(&dir);

    write_vault_file(
        &dir,
        "temper/task/old-task.md",
        r#"---
id: "019d5616-8e3c-7432-9867-222b36e46ea1"
type: task
context: temper
stage: backlog
title: "Old task"
slug: old-task
created: "2026-04-03T21:23:32.026022-04:00"
---

# Old task

Some content here.
"#,
    );

    let result = temper_cli::actions::doctor::fix(&config, None, false).unwrap();
    assert!(result.fields_renamed > 0, "Should have renamed fields");
    assert_eq!(result.files_modified, 1);

    let content = fs::read_to_string(dir.path().join("temper/task/old-task.md")).unwrap();
    assert!(content.contains("temper-id:"), "got:\n{content}");
    assert!(content.contains("temper-type:"));
    assert!(content.contains("temper-context:"));
    assert!(content.contains("temper-stage:"));
    assert!(content.contains("temper-created:"));
    assert!(!content.contains("\nid:"), "bare 'id:' should be gone");
    assert!(!content.contains("\ntype:"));
    assert!(!content.contains("\ncontext:"));
    assert!(content.contains("Some content here."), "body preserved");
}

#[test]
fn test_doctor_fix_dry_run_does_not_modify() {
    let dir = TempDir::new().unwrap();
    let config = test_config(&dir);

    let original = r#"---
id: "019d5616-8e3c-7432-9867-222b36e46ea1"
type: task
context: temper
stage: backlog
title: "Old task"
slug: old-task
created: "2026-04-03T21:23:32.026022-04:00"
---

# Old task
"#;

    write_vault_file(&dir, "temper/task/old-task.md", original);

    let result = temper_cli::actions::doctor::fix(&config, None, true).unwrap();
    assert!(result.fields_renamed > 0);

    let content = fs::read_to_string(dir.path().join("temper/task/old-task.md")).unwrap();
    assert_eq!(content, original, "Dry run should not modify file");
}

#[test]
fn test_doctor_fix_backfills_temper_created_from_date() {
    let dir = TempDir::new().unwrap();
    let config = test_config(&dir);

    write_vault_file(
        &dir,
        "temper/session/2026-04-04 — my-session.md",
        r#"---
temper-id: "019d5977-f476-7e41-b4aa-fc4bd2b24426"
temper-type: session
temper-context: temper
title: "My session"
date: "2026-04-04"
---

## Goal
"#,
    );

    let result = temper_cli::actions::doctor::fix(&config, None, false).unwrap();
    assert!(result.fields_backfilled > 0);

    let content =
        fs::read_to_string(dir.path().join("temper/session/2026-04-04 — my-session.md")).unwrap();
    assert!(content.contains("temper-created:"), "got:\n{content}");
}
```

- [ ] **Step 2: Run tests to verify they fail**

```bash
cargo nextest run -p temper-cli test_doctor_fix
```

Expected: compilation error or test failure — `fix` function not yet implemented.

- [ ] **Step 3: Implement fix function in actions/doctor.rs**

Add to `crates/temper-cli/src/actions/doctor.rs`. Note: this composes `vault::rename_frontmatter_field`,
`vault::remove_frontmatter_field`, `vault::insert_frontmatter_field`, and
`vault::set_frontmatter_field` — it does NOT implement its own frontmatter manipulation.

```rust
use serde::Serialize;

/// Summary of fixes applied by doctor fix.
#[derive(Debug, Clone, Serialize)]
pub struct FixReport {
    pub files_modified: u32,
    pub fields_renamed: u32,
    pub fields_backfilled: u32,
}

/// The legacy field rename map: (old_name, new_name).
const LEGACY_FIELD_MAP: &[(&str, &str)] = &[
    ("id", "temper-id"),
    ("type", "temper-type"),
    ("doc_type", "temper-type"),
    ("context", "temper-context"),
    ("project", "temper-context"),
    ("ingestion_source", "temper-source"),
    ("created", "temper-created"),
    ("updated", "temper-updated"),
    ("stage", "temper-stage"),
    ("mode", "temper-mode"),
    ("effort", "temper-effort"),
    ("goal", "temper-goal"),
    ("seq", "temper-seq"),
    ("branch", "temper-branch"),
    ("pr", "temper-pr"),
    ("status", "temper-status"),
    ("legacy_id", "temper-legacy-id"),
];

/// Apply auto-fixable transformations to vault files.
pub fn fix(config: &Config, context_filter: Option<&str>, dry_run: bool) -> Result<FixReport> {
    let mut report = FixReport {
        files_modified: 0,
        fields_renamed: 0,
        fields_backfilled: 0,
    };

    let contexts: Vec<String> = if let Some(ctx) = context_filter {
        vec![ctx.to_string()]
    } else {
        config.contexts.clone()
    };

    for ctx in &contexts {
        for doc_type in SCAN_DOC_TYPES {
            let dir = config.vault_root.join(ctx).join(doc_type);
            if !dir.is_dir() {
                continue;
            }
            fix_directory(&dir, &mut report, dry_run)?;
        }
    }

    let research_dir = config.vault_root.join("research");
    if research_dir.is_dir() {
        fix_research_dir(&research_dir, context_filter, &mut report, dry_run)?;
    }

    Ok(report)
}

fn fix_research_dir(
    research_dir: &Path,
    context_filter: Option<&str>,
    report: &mut FixReport,
    dry_run: bool,
) -> Result<()> {
    let subdirs: Vec<_> = fs::read_dir(research_dir)?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .collect();

    for subdir in subdirs {
        if let Some(filter) = context_filter {
            let dir_name = subdir.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if dir_name != filter {
                continue;
            }
        }
        fix_directory(&subdir, report, dry_run)?;
    }

    Ok(())
}

fn fix_directory(dir: &Path, report: &mut FixReport, dry_run: bool) -> Result<()> {
    let md_files: Vec<_> = fs::read_dir(dir)?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|ext| ext == "md"))
        .collect();

    for file_path in md_files {
        fix_file(&file_path, report, dry_run)?;
    }

    Ok(())
}

/// Apply auto-fixes to a single vault file. Each fix is a discrete step that
/// re-parses frontmatter after the previous step to avoid stale state.
fn fix_file(file_path: &Path, report: &mut FixReport, dry_run: bool) -> Result<()> {
    let content = fs::read_to_string(file_path)?;
    let mut modified = content.clone();
    let mut file_changed = false;

    let fm = match vault::parse_frontmatter(&modified) {
        Some(fm) => fm,
        None => return Ok(()),
    };

    let mapping = match fm.as_mapping() {
        Some(m) => m,
        None => return Ok(()),
    };

    // Step 1: Rename legacy fields (uses vault::rename/remove_frontmatter_field)
    for (old_name, new_name) in LEGACY_FIELD_MAP {
        let has_old = mapping.contains_key(&serde_yaml::Value::String(old_name.to_string()));
        let has_new = mapping.contains_key(&serde_yaml::Value::String(new_name.to_string()));

        if has_old && !has_new {
            modified = vault::rename_frontmatter_field(&modified, old_name, new_name);
            report.fields_renamed += 1;
            file_changed = true;
        } else if has_old && has_new {
            modified = vault::remove_frontmatter_field(&modified, old_name);
            report.fields_renamed += 1;
            file_changed = true;
        }
    }

    // Step 2: Backfill temper-created from date field if missing
    if let Some(ref v) = vault::parse_frontmatter(&modified) {
        if v.get("temper-created").is_none() {
            if let Some(date_str) = v.get("date").and_then(|d| d.as_str()) {
                let created_value = format!("{date_str}T00:00:00Z");
                modified = vault::set_frontmatter_field(&modified, "temper-created", &created_value);
                report.fields_backfilled += 1;
                file_changed = true;
            }
        }
    }

    // Step 3: Backfill temper-id if missing (generate UUIDv7)
    if let Some(ref v) = vault::parse_frontmatter(&modified) {
        if v.get("temper-id").is_none() {
            let new_id = uuid::Uuid::now_v7();
            modified = vault::insert_frontmatter_field(&modified, "temper-id", &format!("\"{new_id}\""));
            report.fields_backfilled += 1;
            file_changed = true;
        }
    }

    if file_changed {
        report.files_modified += 1;
        if !dry_run {
            fs::write(file_path, &modified)?;
        }
    }

    Ok(())
}
```

- [ ] **Step 4: Run all doctor tests**

```bash
cargo nextest run -p temper-cli test_doctor
```

Expected: all 8 tests pass (5 scan + 3 fix).

- [ ] **Step 5: Run full quality check**

```bash
cargo make check
```

Expected: fmt, clippy, and all existing tests pass.

- [ ] **Step 6: Commit**

```bash
git add crates/temper-cli/src/actions/doctor.rs crates/temper-cli/tests/doctor_test.rs
git commit -m "feat: implement doctor fix — legacy field rename, backfill, dry-run support"
```

---

## Task 8: Deprecate Normalize in Favor of Doctor

**Files:**
- Modify: `crates/temper-cli/src/cli.rs`
- Modify: `crates/temper-cli/src/commands/normalize.rs`

- [ ] **Step 1: Add deprecation notice to normalize command**

In `crates/temper-cli/src/cli.rs`, update the Normalize variant doc comment:

```rust
    /// [Deprecated: use `temper doctor fix`] Normalize vault structure and repair drift
    Normalize {
```

- [ ] **Step 2: Add deprecation warning to normalize command output**

In `crates/temper-cli/src/commands/normalize.rs`, add at the top of `run()`:

```rust
    crate::output::warning("Note: `temper normalize` is deprecated. Use `temper doctor` and `temper doctor fix` instead.");
```

- [ ] **Step 3: Verify**

```bash
cargo build -p temper-cli 2>&1 | tail -3
```

Expected: compiles cleanly.

- [ ] **Step 4: Commit**

```bash
git add crates/temper-cli/src/cli.rs crates/temper-cli/src/commands/normalize.rs
git commit -m "chore: deprecate normalize command in favor of temper doctor / doctor fix"
```

---

## Task 9: E2E Tests for Doctor

Exercise the doctor command through the full CLI path using the `E2eTestApp` infrastructure
and vault fixtures. These tests verify the command works end-to-end, not just the action layer.

**Files:**
- Create: `tests/e2e/tests/doctor_test.rs`

- [ ] **Step 1: Write e2e test — doctor reports clean for valid vault**

Create `tests/e2e/tests/doctor_test.rs`:

```rust
#![cfg(feature = "test-db")]

mod common;

use std::fs;

/// temper doctor reports no issues for a vault with valid new-format files.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn doctor_clean_vault(pool: sqlx::PgPool) {
    let app = common::setup(pool).await;

    // Create a valid task file in new format
    let task_dir = app.cli_config.vault_root.join("temper").join("task");
    fs::create_dir_all(&task_dir).unwrap();
    fs::write(
        task_dir.join("valid-task.md"),
        r#"---
temper-id: "019d5616-8e3c-7432-9867-222b36e46ea1"
temper-type: task
temper-context: temper
temper-created: "2026-04-03T21:23:32-04:00"
temper-stage: backlog
title: "Valid task"
slug: valid-task
---

# Valid task
"#,
    )
    .unwrap();

    let report = temper_cli::actions::doctor::scan(&app.cli_config, None).unwrap();
    assert_eq!(report.files_checked, 1);
    assert_eq!(report.total_issues, 0);
}

/// temper doctor detects legacy fields and doctor fix renames them.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn doctor_fix_legacy_roundtrip(pool: sqlx::PgPool) {
    let app = common::setup(pool).await;

    let task_dir = app.cli_config.vault_root.join("temper").join("task");
    fs::create_dir_all(&task_dir).unwrap();
    let task_path = task_dir.join("legacy-task.md");
    fs::write(
        &task_path,
        r#"---
id: "019d5616-8e3c-7432-9867-222b36e46ea1"
type: task
context: temper
stage: backlog
title: "Legacy task"
slug: legacy-task
created: "2026-04-03T21:23:32-04:00"
---

# Legacy task
"#,
    )
    .unwrap();

    // Doctor should detect issues
    let report = temper_cli::actions::doctor::scan(&app.cli_config, None).unwrap();
    assert!(report.total_issues > 0, "Should detect legacy fields");
    assert!(report.auto_fixable > 0);

    // Fix should rename them
    let fix_report = temper_cli::actions::doctor::fix(&app.cli_config, None, false).unwrap();
    assert!(fix_report.fields_renamed > 0);

    // Doctor again should be clean (or at least fewer issues)
    let report_after = temper_cli::actions::doctor::scan(&app.cli_config, None).unwrap();
    assert!(
        report_after.total_issues < report.total_issues,
        "Issues should decrease after fix: before={}, after={}",
        report.total_issues, report_after.total_issues,
    );

    // Verify the file has new field names
    let content = fs::read_to_string(&task_path).unwrap();
    assert!(content.contains("temper-id:"));
    assert!(content.contains("temper-type:"));
    assert!(content.contains("temper-context:"));
    assert!(content.contains("temper-stage:"));
}
```

- [ ] **Step 2: Run e2e tests**

```bash
cargo nextest run -p temper-e2e --features test-db doctor
```

Expected: both e2e tests pass. If the e2e infra doesn't have `cli_config.contexts` set to
include `"temper"`, the scan won't find files — check the `common::setup` function and adjust
the config or create the context first.

- [ ] **Step 3: Commit**

```bash
git add tests/e2e/tests/doctor_test.rs
git commit -m "test: add e2e tests for doctor scan and fix roundtrip"
```

---

## Task 10: Final Integration — Full Test Suite and Clippy

**Files:**
- No new files — verification only

- [ ] **Step 1: Run the full test suite**

```bash
cargo make test
```

Expected: all tests pass (existing + new doctor + schema + vault utility tests).

- [ ] **Step 2: Run full quality checks**

```bash
cargo make check
```

Expected: fmt clean, clippy clean, no warnings.

- [ ] **Step 3: Test the CLI manually against a temp vault**

```bash
# Create a temp vault with mixed legacy/new files
tmpdir=$(mktemp -d)
mkdir -p "$tmpdir/temper/task" "$tmpdir/temper/goal" "$tmpdir/.temper"
echo '{}' > "$tmpdir/.temper/manifest.json"
echo '' > "$tmpdir/.temper/events.jsonl"

# Legacy task file
cat > "$tmpdir/temper/task/old-task.md" << 'EOF'
---
id: "019d5616-8e3c-7432-9867-222b36e46ea1"
type: task
context: temper
stage: backlog
title: "Legacy task"
slug: old-task
created: "2026-04-03T21:23:32-04:00"
---

# Legacy task
EOF

# New-format goal file
cat > "$tmpdir/temper/goal/my-goal.md" << 'EOF'
---
temper-id: "019d5038-ce94-7661-8869-8711545e9678"
temper-type: goal
temper-context: temper
temper-created: "2026-04-02T22:03:13+00:00"
title: "My goal"
slug: my-goal
---

# My goal
EOF

# Run doctor
cargo run -p temper-cli -- --vault "$tmpdir" doctor
# Run doctor fix dry-run
cargo run -p temper-cli -- --vault "$tmpdir" doctor fix --dry-run
# Run doctor fix for real
cargo run -p temper-cli -- --vault "$tmpdir" doctor fix
# Run doctor again — should show fewer issues
cargo run -p temper-cli -- --vault "$tmpdir" doctor

# Cleanup
rm -rf "$tmpdir"
```

Expected: doctor reports legacy fields, fix renames them, second doctor run shows clean or reduced issues.

- [ ] **Step 4: Commit any remaining fixes**

If any issues were found during manual testing, fix and commit.

```bash
git add -A
git commit -m "fix: address issues found during doctor integration testing"
```
