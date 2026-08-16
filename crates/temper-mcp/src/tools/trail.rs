//! Element-trail tool — the MCP door onto the append-only event ledger.
//!
//! One read tool calling the same service-direct read path the API handler and the CLI both call
//! (`event_service::element_trail`). The visibility gate lives INSIDE the SQL functions
//! (`element_trail_edge` / `element_trail_node`), not here — this tool resolves a ref to a UUID and
//! dispatches, exactly like `steward_ingest_delta` and the search tool. An unreadable or
//! nonexistent element yields an empty trail, never an error: the gate is leak-safe by design.
//!
//! # Why this is a read tool, not a write
//!
//! The trail is append-only history. It cannot be written through this surface — the read/write
//! separability clause (goal `01a00b14-f32f`) says a read door does not mix with any write door,
//! and the trail is the canonical read. Adding `trail` closes the gap the declaration goal's
//! audit (`019fa618`) named: "The only `trail` occurrence under crates/temper-mcp/src/tools/ is
//! the phrase trailing-UUID-only — a grep false positive."
//!
//! # Ref shape — one ref, two kinds
//!
//! `kind` selects the trail function (`node` → `element_trail_node`, `edge` → `element_trail_edge`).
//! `element` is a ref resolved trailing-UUID-only (`parse_ref`): a resource ref for a node, an edge
//! UUID for an edge. The slug half of a decorated ref is parsed off and ignored, matching every
//! other ref on every surface.

use rmcp::model::CallToolResult;
use schemars::JsonSchema;
use serde::Deserialize;

use temper_core::error::TemperError;
use temper_core::types::element_trail::{ElementKind, EventTrail};
use temper_core::types::ids::ProfileId;
use temper_services::services::event_service;

use crate::service::TemperMcpService;

/// MCP input for `element_trail`: a kind (node | edge) and a ref.
///
/// `kind` reuses the shared `ElementKind` wire type, which already carries `#[cfg_attr(feature =
/// "mcp", derive(schemars::JsonSchema))]` under the `mcp` feature temper-mcp enables on
/// temper-core. The schema an agent reads IS the contract — no restatement here.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct ElementTrailInput {
    /// Which element's trail to read: `node` (a resource) or `edge` (a relationship).
    pub kind: ElementKind,

    /// The element ref: a resource ref (UUID or decorated `slug-<uuid>`) for a node, or the
    /// edge's UUID for an edge. Resolved trailing-UUID-only — the slug half is parsed off and
    /// ignored, so a stale slug half is harmless.
    pub element: String,
}

fn to_text<T: serde::Serialize>(value: &T) -> String {
    serde_json::to_string_pretty(value).unwrap_or_else(|_| "{}".to_string())
}

fn map_err(e: TemperError, action: &str) -> rmcp::ErrorData {
    match e {
        TemperError::BadRequest(msg) => rmcp::ErrorData::invalid_params(msg, None),
        other => rmcp::ErrorData::internal_error(format!("{action}: {other}"), None),
    }
}

/// Read an element's event trail — the append-only history of events that produced and mutated a
/// single node (resource) or edge (relationship).
///
/// Service-direct onto `event_service::element_trail`, the same path the API handler
/// (`GET /api/graph/elements/{kind}/{id}/trail`) and the CLI (`temper trail <kind> <ref>`) call.
/// Visibility is gated inside the SQL functions: nodes via `resources_visible_to`, edges via the
/// `edges_visible_to` triple (home readable AND both endpoints readable). An unreadable or
/// nonexistent element returns an empty trail, never an error.
pub async fn element_trail(
    svc: &TemperMcpService,
    input: ElementTrailInput,
) -> Result<CallToolResult, rmcp::ErrorData> {
    let profile = svc.require_profile().await?;

    let element_id = temper_workflow::operations::parse_ref(&input.element)
        .map_err(|e| rmcp::ErrorData::invalid_params(format!("bad element ref: {e}"), None))?
        .0;

    let trail: EventTrail = event_service::element_trail(
        &svc.api_state.pool,
        ProfileId::from(profile.id),
        input.kind,
        element_id,
    )
    .await
    .map_err(|e| map_err(TemperError::from(e), "element_trail"))?;

    Ok(CallToolResult::success(vec![rmcp::model::Content::text(
        to_text(&trail),
    )]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn element_trail_input_deserializes_node() {
        let json = serde_json::json!({
            "kind": "node",
            "element": "019f0e28-1750-7490-919f-5e51c92c8391"
        });
        let input: ElementTrailInput = serde_json::from_value(json).unwrap();
        assert_eq!(input.kind, ElementKind::Node);
        assert_eq!(
            input.element,
            "019f0e28-1750-7490-919f-5e51c92c8391".to_string()
        );
    }

    #[test]
    fn element_trail_input_deserializes_edge_decorated_ref() {
        // A decorated ref carries a slug half that parse_ref ignores — the tool accepts it verbatim.
        let json = serde_json::json!({
            "kind": "edge",
            "element": "supports-claim-019f0e28-1750-7490-919f-5e51c92c8391"
        });
        let input: ElementTrailInput = serde_json::from_value(json).unwrap();
        assert_eq!(input.kind, ElementKind::Edge);
        assert!(input.element.contains("019f0e28"));
    }

    /// A bad ref is rejected at parse time, never reaching the service. The parse error is mapped
    /// to `invalid_params` directly (the `map_err` path is for service errors; a ref error is an
    /// input error and is rendered as such at the parse call site, not run through `map_err`).
    #[test]
    fn a_bad_ref_renders_as_invalid_params() {
        // parse_ref on a non-UUID, non-decorated string fails before any network call.
        let bad = "not-a-ref";
        let err = temper_workflow::operations::parse_ref(bad).expect_err("garbage is not a ref");
        let mapped = rmcp::ErrorData::invalid_params(format!("bad element ref: {err}"), None);
        assert_eq!(mapped.code, rmcp::model::ErrorCode::INVALID_PARAMS);
    }

    /// Generate the tool input schema the same way rmcp does at runtime. Asserts `ElementKind`
    /// inlines as an enum (not a `$ref` the model layer cannot resolve) — the same property the
    /// citation-audit schema guard holds for `ProvenanceSource`.
    fn rmcp_schema_for<T: schemars::JsonSchema>() -> serde_json::Value {
        let generator = schemars::generate::SchemaSettings::draft2020_12().into_generator();
        serde_json::to_value(generator.into_root_schema_for::<T>()).unwrap()
    }

    #[test]
    fn element_trail_schema_inlines_element_kind() {
        let schema = rmcp_schema_for::<ElementTrailInput>();
        assert!(
            schema.get("$defs").is_none(),
            "no $defs block should remain once ElementKind is inlined: {schema}"
        );
        let kind_field = &schema["properties"]["kind"];
        assert!(
            kind_field.get("$ref").is_none(),
            "kind field must be inlined, not a $ref: {kind_field}"
        );
        // The inlined enum should surface its variants directly.
        let variants = kind_field
            .get("enum")
            .or_else(|| {
                kind_field
                    .get("oneOf")
                    .and_then(|o| o.as_array().and_then(|a| a.first()))
                    .and_then(|f| f.get("enum"))
            })
            .expect("inlined kind field must carry its variants");
        assert!(
            variants.as_array().is_some_and(|a| !a.is_empty()),
            "inlined kind field must carry at least one variant: {kind_field}"
        );
    }

    #[test]
    fn element_trail_schema_requires_both_fields() {
        let schema = rmcp_schema_for::<ElementTrailInput>();
        let required = schema["required"].as_array().expect("required is an array");
        let names: Vec<&str> = required.iter().filter_map(|v| v.as_str()).collect();
        assert!(names.contains(&"kind"), "kind must be required: {names:?}");
        assert!(
            names.contains(&"element"),
            "element must be required: {names:?}"
        );
    }
}
