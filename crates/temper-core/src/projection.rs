//! Top-level key projection over JSON values.
//!
//! Used by the CLI action layer (`--fields`) and MCP tool handlers
//! (`fields` parameter) to subselect top-level keys from an API
//! response while always preserving a designated anchor key
//! (e.g. `id` or `resource_id`).
//!
//! Nested-path projection is intentionally rejected — the boundary is
//! "we do not own a query language." Callers needing nested projection
//! pipe the unfiltered output to `jq`.

use serde_json::{Map, Value};
use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ProjectionError {
    /// A field name contained `.`, indicating nested-path projection
    /// which is intentionally unsupported. The `hint` carries the
    /// `jq` invocation the caller should run instead.
    #[error("--fields supports top-level keys only; use jq for nested projection: {hint}")]
    DottedPath { hint: String },
    /// A field name was empty or whitespace-only.
    #[error("empty field name in --fields")]
    EmptyField,
}

/// Filter the top-level keys of a JSON value.
///
/// - If `fields` is empty, the value is returned unchanged.
/// - For an object: returns a new object containing `anchor` (always,
///   when present in the input) plus any `fields` entries that exist
///   as top-level keys. Unknown keys are silently dropped.
/// - For an array of objects: applies the filter to each element.
/// - For other shapes (scalars, mixed arrays): returns the input
///   unchanged.
///
/// Validates all field names before applying. Returns `DottedPath` if
/// any contains `.`, or `EmptyField` if any is empty/whitespace.
pub fn apply_top_level_filter(
    value: Value,
    fields: &[String],
    anchor: &str,
) -> Result<Value, ProjectionError> {
    if fields.is_empty() {
        return Ok(value);
    }

    for raw in fields {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            return Err(ProjectionError::EmptyField);
        }
        if trimmed.contains('.') {
            return Err(ProjectionError::DottedPath {
                hint: format!("pipe the unfiltered output to `jq '.{trimmed}'`"),
            });
        }
    }

    match value {
        Value::Object(map) => Ok(Value::Object(filter_object(map, fields, anchor))),
        Value::Array(items) => {
            let filtered: Vec<Value> = items
                .into_iter()
                .map(|item| match item {
                    Value::Object(m) => Value::Object(filter_object(m, fields, anchor)),
                    other => other,
                })
                .collect();
            Ok(Value::Array(filtered))
        }
        other => Ok(other),
    }
}

fn filter_object(
    mut map: Map<String, Value>,
    fields: &[String],
    anchor: &str,
) -> Map<String, Value> {
    let mut out = Map::new();
    if let Some(v) = map.remove(anchor) {
        out.insert(anchor.to_string(), v);
    }
    for raw in fields {
        let f = raw.trim();
        if f == anchor {
            continue;
        }
        if let Some(v) = map.remove(f) {
            out.insert(f.to_string(), v);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn empty_fields_returns_input_unchanged() {
        let input = json!({"id": "x", "managed_meta": {"stage": "in-progress"}});
        let out = apply_top_level_filter(input.clone(), &[], "id").unwrap();
        assert_eq!(out, input);
    }

    #[test]
    fn object_preserves_anchor_plus_requested_keys() {
        let input = json!({
            "id": "abc",
            "managed_meta": {"stage": "done"},
            "open_meta": {"tags": ["x"]},
            "managed_hash": "sha256:..."
        });
        let out = apply_top_level_filter(input, &["managed_meta".to_string()], "id").unwrap();
        assert_eq!(out, json!({"id": "abc", "managed_meta": {"stage": "done"}}));
    }

    #[test]
    fn object_drops_unknown_keys_silently() {
        let input = json!({"id": "abc", "managed_meta": {}});
        let out = apply_top_level_filter(
            input,
            &["managed_meta".to_string(), "nonexistent".to_string()],
            "id",
        )
        .unwrap();
        assert_eq!(out, json!({"id": "abc", "managed_meta": {}}));
    }

    #[test]
    fn explicit_anchor_in_fields_is_harmless() {
        let input = json!({"id": "abc", "managed_meta": {}});
        let out =
            apply_top_level_filter(input, &["id".to_string(), "managed_meta".to_string()], "id")
                .unwrap();
        assert_eq!(out, json!({"id": "abc", "managed_meta": {}}));
    }

    #[test]
    fn array_of_objects_filters_each_element() {
        let input = json!([
            {"id": "a", "managed_meta": {}, "open_meta": null},
            {"id": "b", "managed_meta": {}, "open_meta": null}
        ]);
        let out = apply_top_level_filter(input, &["managed_meta".to_string()], "id").unwrap();
        assert_eq!(
            out,
            json!([
                {"id": "a", "managed_meta": {}},
                {"id": "b", "managed_meta": {}}
            ])
        );
    }

    #[test]
    fn dotted_path_returns_error_with_jq_hint() {
        let input = json!({"id": "x"});
        let err =
            apply_top_level_filter(input, &["managed_meta.stage".to_string()], "id").unwrap_err();
        match err {
            ProjectionError::DottedPath { hint } => {
                assert!(hint.contains("jq"), "hint must mention jq: {hint}");
                assert!(
                    hint.contains("managed_meta.stage"),
                    "hint must echo the path: {hint}"
                );
            }
            other => panic!("expected DottedPath, got {other:?}"),
        }
    }

    #[test]
    fn empty_field_name_returns_error() {
        let input = json!({"id": "x"});
        let err = apply_top_level_filter(input.clone(), &["".to_string()], "id").unwrap_err();
        assert_eq!(err, ProjectionError::EmptyField);

        let err = apply_top_level_filter(input, &["   ".to_string()], "id").unwrap_err();
        assert_eq!(err, ProjectionError::EmptyField);
    }

    #[test]
    fn scalar_value_returns_input_unchanged() {
        let input = json!("hello");
        let out = apply_top_level_filter(input.clone(), &["anything".to_string()], "id").unwrap();
        assert_eq!(out, input);
    }
}
