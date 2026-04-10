//! Doc type tools — list and describe document types.

use std::collections::BTreeMap;

use rmcp::model::CallToolResult;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::service::TemperMcpService;

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// Summary of a document type returned by `list_doc_types`.
#[derive(Debug, Clone, Serialize)]
pub struct DocTypeSummary {
    pub id: Uuid,
    pub name: String,
    pub has_schema: bool,
    pub required_fields: Vec<String>,
}

/// Full description of a document type returned by `describe_doc_type`.
#[derive(Debug, Clone, Serialize)]
pub struct DescribeDocTypeResponse {
    pub name: String,
    pub schema: serde_json::Value,
    pub required_fields: Vec<String>,
    pub enum_fields: BTreeMap<String, Vec<String>>,
    pub example_managed_meta: serde_json::Value,
}

/// MCP input for `describe_doc_type`.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct DescribeDocTypeInput {
    /// The document type name (e.g. "task", "goal", "session").
    pub name: String,
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Tier-1 and tier-2 fields that are system-managed — excluded from
/// `example_managed_meta` because agents never supply them.
const SYSTEM_FIELDS: &[&str] = &[
    "slug",
    "title",
    "temper-context",
    "temper-type",
    "temper-id",
    "temper-created",
    "temper-updated",
    "temper-owner",
];

/// Build a [`DocTypeSummary`] from a database row and schema metadata.
pub fn build_doc_type_summary(id: Uuid, name: &str) -> DocTypeSummary {
    let (has_schema, required_fields) = match temper_core::schema::required_fields(name) {
        Ok(fields) => (true, fields),
        Err(_) => (false, Vec::new()),
    };

    DocTypeSummary {
        id,
        name: name.to_string(),
        has_schema,
        required_fields,
    }
}

/// Build a [`DescribeDocTypeResponse`] from the embedded schema.
pub fn describe_doc_type_impl(name: &str) -> Result<DescribeDocTypeResponse, rmcp::ErrorData> {
    let schema = temper_core::schema::schema_value(name).map_err(|e| {
        rmcp::ErrorData::new(
            rmcp::model::ErrorCode::INVALID_PARAMS,
            format!("Unknown doc type '{name}': {e}"),
            None,
        )
    })?;

    let required_fields = temper_core::schema::required_fields(name).unwrap_or_default();
    let enum_fields = temper_core::schema::enum_fields(name).unwrap_or_default();

    // Build example_managed_meta from required tier-3 fields (exclude system fields).
    let mut example = serde_json::Map::new();
    if let Some(props) = schema.get("properties").and_then(|p| p.as_object()) {
        for field in &required_fields {
            if SYSTEM_FIELDS.contains(&field.as_str()) {
                continue;
            }
            if let Some(prop) = props.get(field) {
                // Use first enum value if available, otherwise a placeholder.
                let value = if let Some(enum_vals) = prop.get("enum").and_then(|e| e.as_array()) {
                    enum_vals
                        .iter()
                        .find(|v| v.is_string())
                        .cloned()
                        .unwrap_or(serde_json::Value::String("<value>".to_string()))
                } else if prop.get("type").and_then(|t| t.as_str()) == Some("integer") {
                    serde_json::Value::Number(0.into())
                } else {
                    serde_json::Value::String("<value>".to_string())
                };
                example.insert(field.clone(), value);
            }
        }
    }

    Ok(DescribeDocTypeResponse {
        name: name.to_string(),
        schema,
        required_fields,
        enum_fields,
        example_managed_meta: serde_json::Value::Object(example),
    })
}

// ---------------------------------------------------------------------------
// Tool implementations
// ---------------------------------------------------------------------------

pub async fn list_doc_types(svc: &TemperMcpService) -> Result<CallToolResult, rmcp::ErrorData> {
    let _profile = svc.require_profile().await?;

    let rows = temper_api::services::doc_type_service::list_all(&svc.api_state.pool)
        .await
        .map_err(|e| {
            rmcp::ErrorData::internal_error(format!("Failed to list doc types: {e}"), None)
        })?;

    let summaries: Vec<DocTypeSummary> = rows
        .iter()
        .map(|row| build_doc_type_summary(row.id, &row.name))
        .collect();

    let text = serde_json::to_string_pretty(&summaries).unwrap_or_else(|_| "[]".to_string());
    Ok(CallToolResult::success(vec![rmcp::model::Content::text(
        text,
    )]))
}

pub async fn describe_doc_type(
    svc: &TemperMcpService,
    input: DescribeDocTypeInput,
) -> Result<CallToolResult, rmcp::ErrorData> {
    let _profile = svc.require_profile().await?;

    let response = describe_doc_type_impl(&input.name)?;

    let text = serde_json::to_string_pretty(&response).unwrap_or_else(|_| "{}".to_string());
    Ok(CallToolResult::success(vec![rmcp::model::Content::text(
        text,
    )]))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn doc_type_summary_includes_required_fields_for_task() {
        let summary = build_doc_type_summary(Uuid::nil(), "task");
        assert!(summary.has_schema);
        assert!(
            summary
                .required_fields
                .contains(&"temper-stage".to_string()),
            "task required_fields should include temper-stage, got: {:?}",
            summary.required_fields
        );
        assert!(
            summary.required_fields.contains(&"slug".to_string()),
            "task required_fields should include slug, got: {:?}",
            summary.required_fields
        );
    }

    #[test]
    fn doc_type_summary_unknown_type_has_no_schema() {
        let summary = build_doc_type_summary(Uuid::nil(), "widget");
        assert!(!summary.has_schema);
        assert!(summary.required_fields.is_empty());
    }

    #[test]
    fn describe_doc_type_task_returns_schema_and_example() {
        let response = describe_doc_type_impl("task").expect("task should be a known doc type");
        assert_eq!(response.name, "task");

        // required_fields should include temper-stage
        assert!(
            response
                .required_fields
                .contains(&"temper-stage".to_string()),
            "required_fields should contain temper-stage: {:?}",
            response.required_fields
        );

        // enum_fields should have temper-stage with "backlog" as a value
        let stage_enums = response
            .enum_fields
            .get("temper-stage")
            .expect("enum_fields should contain temper-stage");
        assert!(
            stage_enums.contains(&"backlog".to_string()),
            "temper-stage enum should include backlog: {:?}",
            stage_enums
        );

        // example_managed_meta should include temper-stage but not system fields
        let example = response.example_managed_meta.as_object().unwrap();
        assert!(
            example.contains_key("temper-stage"),
            "example should contain temper-stage"
        );
        assert!(
            !example.contains_key("temper-id"),
            "example should not contain system field temper-id"
        );
        assert!(
            !example.contains_key("slug"),
            "example should not contain system field slug"
        );
    }

    #[test]
    fn describe_doc_type_unknown_type_errors() {
        let result = describe_doc_type_impl("widget");
        assert!(result.is_err(), "unknown doc type should return an error");
    }
}
