//! `Frontmatter` aggregate type and `DocType` enum.

use crate::error::{Result, TemperError};
use crate::frontmatter::parse::{normalize_aliases, parse_yaml, split_frontmatter_block};

/// Typed vault doctype. All valid values are enumerated exhaustively —
/// unknown doctypes fail at parse, not at validation.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "doc_type.ts"))]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DocType {
    Task,
    Goal,
    Session,
    Research,
    Decision,
    Concept,
}

#[allow(clippy::should_implement_trait)]
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

    /// The embedded JSON Schema text for this doctype, as compiled into
    /// the binary via `include_str!`. Single source of truth for the
    /// `DocType → schema file` mapping.
    ///
    /// Matches on the enum variant, not a string — the compiler enforces
    /// exhaustiveness. Any new variant added to `DocType` forces this
    /// match to be updated.
    pub fn schema_json(&self) -> &'static str {
        match self {
            DocType::Task => include_str!("../../schemas/task.schema.json"),
            DocType::Goal => include_str!("../../schemas/goal.schema.json"),
            DocType::Session => include_str!("../../schemas/session.schema.json"),
            DocType::Research => include_str!("../../schemas/research.schema.json"),
            DocType::Decision => include_str!("../../schemas/decision.schema.json"),
            DocType::Concept => include_str!("../../schemas/concept.schema.json"),
        }
    }
}

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

    /// Mutable access to the canonicalized frontmatter value.
    ///
    /// Used by higher-level orchestrators (e.g. `normalize_file`) that
    /// need to inject doc-type defaults before writing back. Callers
    /// that mutate this value are responsible for maintaining the
    /// alias-normalized + mapping-typed invariant.
    pub fn value_mut(&mut self) -> &mut serde_yaml::Value {
        &mut self.value
    }

    /// The markdown body preserved byte-for-byte.
    pub fn body(&self) -> &str {
        &self.body
    }

    /// Managed-tier JSON projection of this frontmatter.
    pub fn managed_json(&self) -> serde_json::Value {
        let (managed, _) =
            crate::frontmatter::tiers::split_managed_open(&self.value, self.doc_type);
        managed
    }

    /// Open-tier JSON projection of this frontmatter.
    pub fn open_json(&self) -> serde_json::Value {
        let (_, open) = crate::frontmatter::tiers::split_managed_open(&self.value, self.doc_type);
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

    /// Serialize to the canonical on-disk form: `---\n<yaml>---\n<body>`.
    ///
    /// Display ordering is [`crate::frontmatter::canonical::canonicalize`].
    /// The body is re-emitted byte-for-byte.
    pub fn serialize(&self) -> Result<String> {
        let canonical = crate::frontmatter::canonical::canonicalize(&self.value, self.doc_type);
        let yaml_text = serde_yaml::to_string(&canonical)
            .map_err(|e| TemperError::Config(format!("failed to serialize frontmatter: {e}")))?;
        let mut yaml_normalized = yaml_text.trim_end_matches('\n').to_string();
        yaml_normalized.push('\n');
        Ok(format!(
            "---\n{yaml_normalized}---\n{body}",
            body = self.body
        ))
    }

    /// Atomically write this frontmatter to `path` in canonical form.
    pub fn write_to(&self, path: &std::path::Path) -> Result<()> {
        let content = self.serialize()?;
        write_atomic(path, &content)
    }

    /// Parse a vault file from disk.
    pub fn parse_file(path: &std::path::Path) -> Result<Self> {
        let content = std::fs::read_to_string(path)
            .map_err(|e| TemperError::Config(format!("failed to read {}: {e}", path.display())))?;
        Self::try_from(content.as_str())
    }

    /// Schema-validate this frontmatter against its doc-type schema.
    /// Accepts `temper-provisional-id` in place of `temper-id`.
    pub fn validate(&self) -> Result<Vec<crate::schema::ValidationIssue>> {
        crate::schema::validate_allowing_provisional(self.doc_type.as_str(), &self.value)
    }

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

    /// Shared implementation: insert or overwrite any top-level key.
    fn set_raw_field(&mut self, key: &str, value: serde_json::Value) {
        let yaml_value: serde_yaml::Value =
            serde_yaml::to_value(value).unwrap_or(serde_yaml::Value::Null);
        if let Some(mapping) = self.value.as_mapping_mut() {
            mapping.insert(serde_yaml::Value::String(key.to_string()), yaml_value);
        }
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
}

fn write_atomic(path: &std::path::Path, content: &str) -> Result<()> {
    let parent = path.parent().ok_or_else(|| {
        TemperError::Config(format!("path has no parent directory: {}", path.display()))
    })?;
    let file_name = path
        .file_name()
        .and_then(|s| s.to_str())
        .ok_or_else(|| TemperError::Config(format!("invalid file name: {}", path.display())))?;
    let tmp_path = parent.join(format!(".{file_name}.frontmatter.tmp"));

    std::fs::write(&tmp_path, content)
        .map_err(|e| TemperError::Config(format!("failed to write {}: {e}", tmp_path.display())))?;
    std::fs::rename(&tmp_path, path).map_err(|e| {
        TemperError::Config(format!(
            "failed to rename {} -> {}: {e}",
            tmp_path.display(),
            path.display()
        ))
    })?;
    Ok(())
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
relates_to: [other-task]
tags: [auth]
---
body content here
"#;

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
    fn task_fixture_hashes_are_golden() {
        // Regression anchor: TASK_FIXTURE produces stable managed + open
        // hashes. Goldens captured in session 2 task 10 when the legacy
        // compute_frontmatter_hashes_from_yaml API was deleted. If these
        // change, either the schema or canonicalization moved; investigate
        // before regenerating.
        let fm = Frontmatter::try_from(TASK_FIXTURE).unwrap();
        let (managed_hash, open_hash) = fm.hashes();
        assert_eq!(
            managed_hash, "sha256:40c2c784826e73014d5acfadc2914f73e4a3dce70185ce0a31d0bb1d28182b6c",
            "task fixture managed hash drift"
        );
        assert_eq!(
            open_hash, "sha256:b1c37240b0d306aab5aff2281173306122cf0bff36c6dec1dcc001b2df711061",
            "task fixture open hash drift"
        );
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
        assert_eq!(
            a.hashes(),
            b.hashes(),
            "hash must be stable under input reordering"
        );
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
        assert_eq!(
            a.hashes(),
            b.hashes(),
            "alias-form and canonical-form must hash identically"
        );
    }

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
        let twice = Frontmatter::try_from(once.as_str())
            .unwrap()
            .serialize()
            .unwrap();
        assert_eq!(once, twice, "canonical form must be a fixed point");
    }

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
        fm.set_managed_field(
            "temper-id",
            serde_json::json!("019d8110-8ff3-70c2-85ae-57e04ed62885"),
        );
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
        assert_eq!(
            original_value_keys, new_value_keys,
            "set_body must not touch frontmatter mapping"
        );
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
        let rels = ResourceRelationships {
            relates_to: vec!["new-one".to_string(), "new-two".to_string()],
            depends_on: vec!["dep".to_string()],
            ..ResourceRelationships::default()
        };
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
        assert!(
            o.get("relates_to").is_none(),
            "empty relates_to must remove the key"
        );
    }

    #[test]
    fn set_relationships_does_not_touch_tags() {
        use crate::types::graph::ResourceRelationships;

        let mut fm = Frontmatter::try_from(TASK_FIXTURE).unwrap();
        let before_tags = fm.tags();
        let rels = ResourceRelationships::default();
        fm.set_relationships(&rels);
        assert_eq!(
            fm.tags(),
            before_tags,
            "tags are metadata, set_relationships must not touch them"
        );
    }

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
}
