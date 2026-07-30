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
use temper_core::types::facet_requests::{EdgeFacetsResponse, FacetAck, ResourceFacetsResponse};
use temper_core::types::ids::{EdgeId, ProfileId};
use temper_core::types::property_owner::PropertyOwner;
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
        TemperError::NotFound(msg) => {
            rmcp::ErrorData::invalid_params(format!("{action}: {msg}"), None)
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

/// Set a facet (typed property) on a resource — the steward's facet act.
///
/// CLI equivalent: `temper resource facet <ref> --values '<json>'`.
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
        owner: PropertyOwner::resource(resource),
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
        property_ids: out.value.into_iter().map(Uuid::from).collect(),
    };
    Ok(CallToolResult::success(vec![rmcp::model::Content::text(
        to_text(&ack),
    )]))
}

/// MCP input for `edge_facet_set`.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct EdgeFacetSetInput {
    /// The relationship's edge handle (the UUID `edge_assert` returned).
    pub edge_handle: Uuid,
    /// The facet's typed value payload.
    pub values: serde_json::Value,
    /// Facet salience/confidence weight (0.0-1.0 by convention). Defaults to 1.0.
    pub weight: Option<f64>,
    /// Per-act correlation (`invocation_id`) + discrete agent authorship. Flattened top-level
    /// keys; all optional. `confidence` required when any other authorship field is supplied.
    #[serde(flatten)]
    pub act: ActInput,
}

/// MCP input for `edge_facets` (read).
#[derive(Debug, Deserialize, JsonSchema)]
pub struct EdgeFacetsInput {
    /// The relationship's edge handle.
    pub edge_handle: Uuid,
}

/// MCP input for `resource_facets` (read).
#[derive(Debug, Deserialize, JsonSchema)]
pub struct ResourceFacetsInput {
    /// Resource ref: a UUID or the decorated `slug-<uuid>` form.
    pub resource: String,
}

/// Set a facet whose owner is an **edge** — a qualifier on a relationship rather than on a thing.
///
/// Mirrors `POST /api/relationships/{edge_handle}/facets`. The edge is addressed by its handle, not
/// a resource ref, and the write authorizes through the edge's own mutability clauses (its source
/// resource plus container-write on its home) rather than through `can_modify_resource`.
///
/// CLI equivalent: `temper edge facet <edge-handle> --values '<json>'`.
pub async fn edge_facet_set(
    svc: &TemperMcpService,
    input: EdgeFacetSetInput,
) -> Result<CallToolResult, rmcp::ErrorData> {
    let profile = svc.require_profile().await?;
    let pool = &svc.api_state.pool;
    let profile_id = ProfileId::from(profile.id);

    let act = input
        .act
        .into_act_context()
        .map_err(|e| rmcp::ErrorData::invalid_params(e.to_string(), None))?;

    let cmd = SetFacet {
        owner: PropertyOwner::edge(EdgeId::from(input.edge_handle)),
        values: input.values,
        weight: input.weight.unwrap_or(1.0),
        act,
        origin: Surface::Mcp,
    };

    let backend = DbBackend::new(pool.clone(), profile_id);
    let out = backend
        .set_facet(cmd)
        .await
        .map_err(|e| map_err(e, "edge_facet_set"))?;

    let ack = FacetAck {
        property_ids: out.value.into_iter().map(Uuid::from).collect(),
    };
    Ok(CallToolResult::success(vec![rmcp::model::Content::text(
        to_text(&ack),
    )]))
}

/// Read the live facets of one edge. Service-direct, like every other read.
pub async fn edge_facets(
    svc: &TemperMcpService,
    input: EdgeFacetsInput,
) -> Result<CallToolResult, rmcp::ErrorData> {
    let profile = svc.require_profile().await?;
    let facets = temper_services::services::edge_service::list_edge_facets(
        &svc.api_state.pool,
        profile.id,
        input.edge_handle,
    )
    .await
    .map_err(|e| map_err(TemperError::from(e), "edge_facets"))?;

    let out = EdgeFacetsResponse {
        edge_handle: input.edge_handle,
        facets,
    };
    Ok(CallToolResult::success(vec![rmcp::model::Content::text(
        to_text(&out),
    )]))
}

/// Read the live facets of one resource. Service-direct, like every other read.
///
/// The faithful view: one entry per live row, each with its weight and its author. `get_resource`
/// carries a facet inside `open_meta` collapsed to a single newest-wins value with the weight
/// discarded, so it cannot answer *"did my assert land"* when a key was asserted more than once.
///
/// CLI equivalent: `temper resource facets <ref>`.
pub async fn resource_facets(
    svc: &TemperMcpService,
    input: ResourceFacetsInput,
) -> Result<CallToolResult, rmcp::ErrorData> {
    let profile = svc.require_profile().await?;
    let resource = temper_workflow::operations::parse_ref(&input.resource)
        .map_err(|e| map_err(e, "resource_facets"))?;

    let facets = temper_services::services::facet_service::list_resource_facets(
        &svc.api_state.pool,
        ProfileId::from(profile.id),
        resource,
    )
    .await
    .map_err(|e| map_err(TemperError::from(e), "resource_facets"))?;

    let out = ResourceFacetsResponse {
        resource: Uuid::from(resource),
        facets,
    };
    Ok(CallToolResult::success(vec![rmcp::model::Content::text(
        to_text(&out),
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
