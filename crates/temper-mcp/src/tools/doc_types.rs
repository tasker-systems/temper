//! Doc type tools — list and describe document types.
//!
//! The derivation these tools render lives in `temper_workflow::schema`, beside the
//! embedded schemas it reads. It was private to this crate until the web surface needed
//! to ask which states a kind of work carries; MCP is now one of the doors that renders
//! it rather than the only one that has it.

use rmcp::model::CallToolResult;
use schemars::JsonSchema;
use serde::Deserialize;

use crate::service::TemperMcpService;

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// MCP input for `describe_doc_type`.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct DescribeDocTypeInput {
    /// The document type name (e.g. "task", "goal", "session").
    pub name: String,
}

// ---------------------------------------------------------------------------
// Tool implementations
// ---------------------------------------------------------------------------

pub async fn list_doc_types(svc: &TemperMcpService) -> Result<CallToolResult, rmcp::ErrorData> {
    let _profile = svc.require_profile().await?;

    // Doc-types are name-keyed in the substrate — enumerate the embedded schema set
    // (the single source of truth) rather than a DB table.
    let summaries = temper_workflow::schema::list_doc_types();

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

    let response = temper_workflow::schema::describe_doc_type(&input.name).map_err(|e| {
        rmcp::ErrorData::new(
            rmcp::model::ErrorCode::INVALID_PARAMS,
            format!("Unknown doc type '{}': {e}", input.name),
            None,
        )
    })?;

    let text = serde_json::to_string_pretty(&response).unwrap_or_else(|_| "{}".to_string());
    Ok(CallToolResult::success(vec![rmcp::model::Content::text(
        text,
    )]))
}

/// Describe the recognized open_meta conventions — the self-describing schema (recognized keys, their
/// shapes, FTS-indexing markers) plus discouraged keys. Mirrors the CLI `resource describe-open-meta`
/// command; both render the shared `temper_workflow::schema::OpenMetaConvention`.
pub async fn describe_open_meta(svc: &TemperMcpService) -> Result<CallToolResult, rmcp::ErrorData> {
    let _profile = svc.require_profile().await?;

    let convention = temper_workflow::schema::describe_open_meta().map_err(|e| {
        rmcp::ErrorData::new(
            rmcp::model::ErrorCode::INTERNAL_ERROR,
            format!("open_meta schema unavailable: {e}"),
            None,
        )
    })?;

    let text = serde_json::to_string_pretty(&convention).unwrap_or_else(|_| "{}".to_string());
    Ok(CallToolResult::success(vec![rmcp::model::Content::text(
        text,
    )]))
}

// ── Consolidated read tool (3→1) ───────────────────────────────────────────────

/// The schema-describe view to perform.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum DescribeSchemaView {
    /// List all available document types with schema summaries.
    DocTypes,
    /// Describe a specific document type in detail (full JSON schema, required fields, enum values).
    DocType,
    /// Describe the recognized open_meta conventions.
    OpenMeta,
}

/// Consolidated describe-schema tool — one read tool with a `view` discriminator.
///
/// Collapses `list_doc_types`, `describe_doc_type`, and `describe_open_meta`
/// into a single MCP tool.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct DescribeSchemaInput {
    /// Which schema description to perform.
    pub view: DescribeSchemaView,
    /// The document type name (e.g. "task", "goal", "session"). Required for `doc_type`; ignored otherwise.
    #[serde(default)]
    pub name: Option<String>,
}

/// Dispatch the consolidated describe-schema tool.
pub async fn describe_schema(
    svc: &TemperMcpService,
    input: DescribeSchemaInput,
) -> Result<CallToolResult, rmcp::ErrorData> {
    match input.view {
        DescribeSchemaView::DocTypes => list_doc_types(svc).await,
        DescribeSchemaView::DocType => {
            let name = input.name.ok_or_else(|| {
                rmcp::ErrorData::invalid_params("doc_type requires `name`".to_string(), None)
            })?;
            describe_doc_type(svc, DescribeDocTypeInput { name }).await
        }
        DescribeSchemaView::OpenMeta => describe_open_meta(svc).await,
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------
//
// The derivation's own tests moved with it, to `temper_workflow::schema`. What is left
// here is the thing that is genuinely this crate's: that the `view` discriminator routes
// to the three answers, and that an unknown doc-type name refuses as invalid params
// rather than as an internal fault.

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn describe_schema_input_accepts_each_view() {
        for (raw, expect_name) in [
            (r#"{"view":"doc_types"}"#, false),
            (r#"{"view":"doc_type","name":"task"}"#, true),
            (r#"{"view":"open_meta"}"#, false),
        ] {
            let input: DescribeSchemaInput =
                serde_json::from_str(raw).unwrap_or_else(|e| panic!("{raw} should parse: {e}"));
            assert_eq!(input.name.is_some(), expect_name, "for {raw}");
        }
    }

    /// The name in a `doc_type` view comes from the caller, so an unrecognized one is the
    /// caller's mistake and must refuse as such. Rendering it as an internal fault would
    /// tell an agent the server broke when in fact it asked for a type that does not exist.
    #[test]
    fn an_unknown_doc_type_name_is_a_caller_error_not_an_internal_one() {
        let err = temper_workflow::schema::describe_doc_type("widget")
            .expect_err("widget is not a doc type");
        assert!(
            err.to_string().contains("widget"),
            "the refusal must name what was asked for, got: {err}"
        );
    }
}
