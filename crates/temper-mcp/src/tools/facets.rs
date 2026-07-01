//! Facet tool — set (upsert) a typed property on a resource.
//!
//! Mirrors the HTTP endpoint `POST /api/facets` (`temper-api/src/handlers/facets.rs`)
//! and dispatches through `DbBackend` — the same write path the HTTP handler
//! uses. The resource is a decorated ref (a UUID or the `slug-<uuid>` form)
//! resolved via `parse_ref` into a `ResourceId`.

use rmcp::model::CallToolResult;
use schemars::JsonSchema;
use serde::Deserialize;

use temper_core::error::TemperError;
use temper_core::types::authorship::ActInput;
use temper_core::types::facet_requests::FacetAck;
use temper_core::types::ids::ProfileId;
use temper_services::backend::DbBackend;
use temper_workflow::operations::{Backend, SetFacet, Surface};
use uuid::Uuid;

use crate::service::TemperMcpService;

// ── Input structs ──────────────────────────────────────────────────────────────

/// MCP input for facet_set.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct FacetSetInput {
    /// Resource ref: a UUID or the decorated `slug-<uuid>` form.
    pub resource: String,
    /// The facet's typed value payload.
    pub values: serde_json::Value,
    /// Facet salience/confidence weight (0.0-1.0 by convention). Defaults to 1.0.
    pub weight: Option<f64>,
    /// Per-act correlation (`invocation_id`) + discrete agent authorship. Flattened top-level
    /// keys; all optional. `confidence` required when any other authorship field is supplied.
    #[serde(flatten)]
    pub act: ActInput,
}

// ── Helpers ────────────────────────────────────────────────────────────────────

fn to_text<T: serde::Serialize>(value: &T) -> String {
    serde_json::to_string_pretty(value).unwrap_or_else(|_| "{}".to_string())
}

fn map_err(e: TemperError, action: &str) -> rmcp::ErrorData {
    match e {
        TemperError::NotFound(_) => {
            rmcp::ErrorData::invalid_params(format!("{action}: resource not found"), None)
        }
        TemperError::BadRequest(msg) => rmcp::ErrorData::invalid_params(msg, None),
        TemperError::Forbidden => rmcp::ErrorData::new(
            rmcp::model::ErrorCode::INVALID_REQUEST,
            format!("{action}: cannot modify this resource"),
            None,
        ),
        other => rmcp::ErrorData::internal_error(format!("{action}: {other}"), None),
    }
}

// ── Tool handlers ──────────────────────────────────────────────────────────────

pub async fn facet_set(
    svc: &TemperMcpService,
    input: FacetSetInput,
) -> Result<CallToolResult, rmcp::ErrorData> {
    let profile = svc.require_profile().await?;
    let pool = &svc.api_state.pool;
    let profile_id = ProfileId::from(profile.id);

    let resource = temper_workflow::operations::parse_ref(&input.resource)
        .map_err(|e| rmcp::ErrorData::invalid_params(e.to_string(), None))?;

    let act = input
        .act
        .into_act_context()
        .map_err(|e| rmcp::ErrorData::invalid_params(e.to_string(), None))?;

    let cmd = SetFacet {
        resource,
        values: input.values,
        weight: input.weight.unwrap_or(1.0),
        act,
        origin: Surface::Mcp,
    };

    let backend = DbBackend::new(pool.clone(), profile_id);
    let out = backend
        .set_facet(cmd)
        .await
        .map_err(|e| map_err(e, "facet_set"))?;

    let ack = FacetAck {
        property_id: Uuid::from(out.value),
    };
    Ok(CallToolResult::success(vec![rmcp::model::Content::text(
        to_text(&ack),
    )]))
}

// ── Tests ──────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn facet_set_input_deserializes_without_act() {
        let json = serde_json::json!({
            "resource": "019e84ab-26ba-7560-9d34-c60d74a9fbe2",
            "values": {"summary": "example"},
            "weight": 0.5
        });
        let input: FacetSetInput = serde_json::from_value(json).unwrap();
        assert_eq!(input.resource, "019e84ab-26ba-7560-9d34-c60d74a9fbe2");
        assert_eq!(input.values, serde_json::json!({"summary": "example"}));
        assert_eq!(input.weight, Some(0.5));
        assert!(input.act.into_act_context().expect("assembles").is_empty());
    }

    #[test]
    fn facet_set_input_deserializes_with_act_authorship_fields() {
        let json = serde_json::json!({
            "resource": "foo-019e84ab-26ba-7560-9d34-c60d74a9fbe2",
            "values": {"summary": "example"},
            "invocation_id": "019f0e28-1750-7490-919f-5e51c92c8391",
            "reasoning": "derived from ingest",
            "confidence": "confident",
        });
        let input: FacetSetInput = serde_json::from_value(json).unwrap();
        assert_eq!(input.resource, "foo-019e84ab-26ba-7560-9d34-c60d74a9fbe2");
        assert_eq!(input.weight, None);
        assert_eq!(
            input.act.confidence,
            Some(temper_core::types::ConfidenceBand::Confident)
        );
        assert!(input.act.invocation_id.is_some());
        let ctx = input.act.into_act_context().expect("assembles");
        assert!(!ctx.is_empty());
    }
}
