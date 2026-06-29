//! Relationship tools — assert, retype, reweight, and fold graph edges.
//!
//! Each tool mirrors one HTTP endpoint from `temper-api/src/handlers/edges.rs`
//! and dispatches through `DbBackend` — the same write path the HTTP handlers
//! use. Both endpoints are decorated refs (a UUID or the `slug-<uuid>` form)
//! resolved via `parse_ref` into a `ResourceId`.

use rmcp::model::CallToolResult;
use schemars::JsonSchema;
use serde::Deserialize;
use uuid::Uuid;

use temper_api::backend::DbBackend;
use temper_core::error::TemperError;
use temper_core::types::authorship::ActInput;
use temper_core::types::graph::{EdgeKind, Polarity};
use temper_core::types::ids::{EdgeId, ProfileId};
use temper_core::types::relationship_requests::RelationshipAck;
use temper_workflow::operations::{
    AssertRelationship, Backend, FoldRelationship, RetypeRelationship, ReweightRelationship,
    Surface,
};

use crate::service::TemperMcpService;

// ── Input structs ──────────────────────────────────────────────────────────────

/// MCP input for assert_relationship.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct AssertRelationshipInput {
    /// Source resource ref: a UUID or the decorated `slug-<uuid>` form.
    pub source: String,
    /// Target resource ref: a UUID or the decorated `slug-<uuid>` form.
    pub target: String,
    /// Structural edge kind — one of `express`, `contains`, `leads_to`, `near`.
    pub edge_kind: EdgeKind,
    /// Edge direction sign — `forward` or `inverse`.
    pub polarity: Polarity,
    /// Human-readable relation label (e.g. `depends_on`, `parent_of`).
    pub label: String,
    /// Numeric edge weight (0.0–1.0 by convention; exact range is schema-defined).
    pub weight: f64,
    /// Per-act correlation (`invocation_id`) + discrete agent authorship. Flattened top-level
    /// keys; all optional. `confidence` required when any other authorship field is supplied.
    #[serde(flatten)]
    pub act: ActInput,
}

/// MCP input for retype_relationship.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct RetypeRelationshipInput {
    /// Relationship correlation ID returned by assert_relationship.
    pub edge_handle: Uuid,
    /// New structural edge kind.
    pub edge_kind: EdgeKind,
    /// New edge direction sign.
    pub polarity: Polarity,
}

/// MCP input for reweight_relationship.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct ReweightRelationshipInput {
    /// Relationship correlation ID returned by assert_relationship.
    pub edge_handle: Uuid,
    /// New edge weight.
    pub weight: f64,
}

/// MCP input for fold_relationship.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct FoldRelationshipInput {
    /// Relationship correlation ID returned by assert_relationship.
    pub edge_handle: Uuid,
    /// Optional human-readable reason for retracting the relationship.
    pub reason: Option<String>,
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
        TemperError::NotFound(_) => rmcp::ErrorData::invalid_params(
            format!("{action}: resource or relationship not found"),
            None,
        ),
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

pub async fn assert_relationship(
    svc: &TemperMcpService,
    input: AssertRelationshipInput,
) -> Result<CallToolResult, rmcp::ErrorData> {
    let profile = svc.require_profile().await?;
    let pool = &svc.api_state.pool;
    let profile_id = ProfileId::from(profile.id);

    let source = temper_workflow::operations::parse_ref(&input.source)
        .map_err(|e| rmcp::ErrorData::invalid_params(e.to_string(), None))?;
    let target = temper_workflow::operations::parse_ref(&input.target)
        .map_err(|e| rmcp::ErrorData::invalid_params(e.to_string(), None))?;

    let act = input
        .act
        .into_act_context()
        .map_err(|e| rmcp::ErrorData::invalid_params(e.to_string(), None))?;

    let cmd = AssertRelationship {
        source,
        target,
        edge_kind: input.edge_kind,
        polarity: input.polarity,
        label: input.label,
        weight: input.weight,
        act,
        origin: Surface::Mcp,
    };

    let backend = DbBackend::new(pool.clone(), profile_id);
    let out = backend
        .assert_relationship(cmd)
        .await
        .map_err(|e| map_err(e, "assert_relationship"))?;

    let ack = RelationshipAck {
        edge_handle: Uuid::from(out.value),
    };
    Ok(CallToolResult::success(vec![rmcp::model::Content::text(
        to_text(&ack),
    )]))
}

pub async fn retype_relationship(
    svc: &TemperMcpService,
    input: RetypeRelationshipInput,
) -> Result<CallToolResult, rmcp::ErrorData> {
    let profile = svc.require_profile().await?;
    let pool = &svc.api_state.pool;
    let profile_id = ProfileId::from(profile.id);

    let cmd = RetypeRelationship {
        edge_handle: EdgeId::from(input.edge_handle),
        edge_kind: input.edge_kind,
        polarity: input.polarity,
        origin: Surface::Mcp,
    };

    let backend = DbBackend::new(pool.clone(), profile_id);
    let out = backend
        .retype_relationship(cmd)
        .await
        .map_err(|e| map_err(e, "retype_relationship"))?;

    let ack = RelationshipAck {
        edge_handle: Uuid::from(out.value),
    };
    Ok(CallToolResult::success(vec![rmcp::model::Content::text(
        to_text(&ack),
    )]))
}

pub async fn reweight_relationship(
    svc: &TemperMcpService,
    input: ReweightRelationshipInput,
) -> Result<CallToolResult, rmcp::ErrorData> {
    let profile = svc.require_profile().await?;
    let pool = &svc.api_state.pool;
    let profile_id = ProfileId::from(profile.id);

    let cmd = ReweightRelationship {
        edge_handle: EdgeId::from(input.edge_handle),
        weight: input.weight,
        origin: Surface::Mcp,
    };

    let backend = DbBackend::new(pool.clone(), profile_id);
    let out = backend
        .reweight_relationship(cmd)
        .await
        .map_err(|e| map_err(e, "reweight_relationship"))?;

    let ack = RelationshipAck {
        edge_handle: Uuid::from(out.value),
    };
    Ok(CallToolResult::success(vec![rmcp::model::Content::text(
        to_text(&ack),
    )]))
}

pub async fn fold_relationship(
    svc: &TemperMcpService,
    input: FoldRelationshipInput,
) -> Result<CallToolResult, rmcp::ErrorData> {
    let profile = svc.require_profile().await?;
    let pool = &svc.api_state.pool;
    let profile_id = ProfileId::from(profile.id);

    let act = input
        .act
        .into_act_context()
        .map_err(|e| rmcp::ErrorData::invalid_params(e.to_string(), None))?;

    let cmd = FoldRelationship {
        edge_handle: EdgeId::from(input.edge_handle),
        reason: input.reason,
        act,
        origin: Surface::Mcp,
    };

    let backend = DbBackend::new(pool.clone(), profile_id);
    let out = backend
        .fold_relationship(cmd)
        .await
        .map_err(|e| map_err(e, "fold_relationship"))?;

    let ack = RelationshipAck {
        edge_handle: Uuid::from(out.value),
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
    fn assert_relationship_input_deserializes() {
        let json = serde_json::json!({
            "source": "foo-019e84ab-26ba-7560-9d34-c60d74a9fbe2",
            "target": "019e84ab-26ba-7560-9d34-c60d74a9fbe3",
            "edge_kind": "leads_to",
            "polarity": "inverse",
            "label": "depends_on",
            "weight": 1.0
        });
        let input: AssertRelationshipInput = serde_json::from_value(json).unwrap();
        assert_eq!(input.source, "foo-019e84ab-26ba-7560-9d34-c60d74a9fbe2");
        assert_eq!(input.target, "019e84ab-26ba-7560-9d34-c60d74a9fbe3");
        assert_eq!(input.edge_kind, EdgeKind::LeadsTo);
        assert_eq!(input.polarity, Polarity::Inverse);
        assert_eq!(input.label, "depends_on");
        assert_eq!(input.weight, 1.0);
    }

    #[test]
    fn assert_relationship_input_accepts_act_authorship_fields() {
        let json = serde_json::json!({
            "source": "019e84ab-26ba-7560-9d34-c60d74a9fbe2",
            "target": "019e84ab-26ba-7560-9d34-c60d74a9fbe3",
            "edge_kind": "leads_to",
            "polarity": "forward",
            "label": "depends_on",
            "weight": 1.0,
            "invocation_id": "019f0e28-1750-7490-919f-5e51c92c8391",
            "reasoning": "these two co-vary",
            "confidence": "confident",
        });
        let input: AssertRelationshipInput = serde_json::from_value(json).unwrap();
        assert_eq!(
            input.act.confidence,
            Some(temper_core::types::ConfidenceBand::Confident)
        );
        assert!(input.act.invocation_id.is_some());
        let ctx = input.act.into_act_context().expect("assembles");
        assert!(!ctx.is_empty());
    }

    #[test]
    fn fold_relationship_input_accepts_act_authorship_fields() {
        let id = Uuid::new_v4();
        let json = serde_json::json!({
            "edge_handle": id.to_string(),
            "reason": "superseded",
            "confidence": "tentative",
        });
        let input: FoldRelationshipInput = serde_json::from_value(json).unwrap();
        assert_eq!(input.edge_handle, id);
        assert_eq!(
            input.act.confidence,
            Some(temper_core::types::ConfidenceBand::Tentative)
        );
    }

    #[test]
    fn retype_relationship_input_deserializes() {
        let id = Uuid::new_v4();
        let json = serde_json::json!({
            "edge_handle": id.to_string(),
            "edge_kind": "near",
            "polarity": "forward"
        });
        let input: RetypeRelationshipInput = serde_json::from_value(json).unwrap();
        assert_eq!(input.edge_handle, id);
        assert_eq!(input.edge_kind, EdgeKind::Near);
        assert_eq!(input.polarity, Polarity::Forward);
    }

    #[test]
    fn reweight_relationship_input_deserializes() {
        let id = Uuid::new_v4();
        let json = serde_json::json!({
            "edge_handle": id.to_string(),
            "weight": 0.5
        });
        let input: ReweightRelationshipInput = serde_json::from_value(json).unwrap();
        assert_eq!(input.edge_handle, id);
        assert_eq!(input.weight, 0.5);
    }

    #[test]
    fn fold_relationship_input_deserializes_with_reason() {
        let id = Uuid::new_v4();
        let json = serde_json::json!({
            "edge_handle": id.to_string(),
            "reason": "no longer relevant"
        });
        let input: FoldRelationshipInput = serde_json::from_value(json).unwrap();
        assert_eq!(input.edge_handle, id);
        assert_eq!(input.reason, Some("no longer relevant".to_string()));
    }

    #[test]
    fn fold_relationship_input_deserializes_without_reason() {
        let id = Uuid::new_v4();
        let json = serde_json::json!({
            "edge_handle": id.to_string()
        });
        let input: FoldRelationshipInput = serde_json::from_value(json).unwrap();
        assert_eq!(input.edge_handle, id);
        assert_eq!(input.reason, None);
    }

    /// Generate the tool input schema the same way rmcp does at runtime
    /// (`SchemaSettings::draft2020_12`, ref-based generator). This is the exact
    /// path that surfaced the bug: scalar enums emitted as `$ref` into `$defs`
    /// reach the Anthropic tool-use layer with no type signal and come back as
    /// `null`. See `crate::tools::relationships` and the
    /// `review-mcp-assert-relationship-edge-issues` task.
    fn rmcp_schema_for<T: schemars::JsonSchema>() -> serde_json::Value {
        let generator = schemars::generate::SchemaSettings::draft2020_12().into_generator();
        serde_json::to_value(generator.into_root_schema_for::<T>()).unwrap()
    }

    /// A schema field must inline a string enum (`{"type":"string","enum":[…]}`)
    /// rather than reference it via `$ref` — otherwise the MCP client cannot
    /// see the allowed values and sends `null`.
    fn assert_inline_string_enum(field: &serde_json::Value, variants: &[&str]) {
        assert!(
            field.get("$ref").is_none(),
            "field must be inlined, not a $ref: {field}"
        );
        assert_eq!(
            field.get("type").and_then(|t| t.as_str()),
            Some("string"),
            "field must be a string enum: {field}"
        );
        let got: Vec<&str> = field
            .get("enum")
            .and_then(|e| e.as_array())
            .expect("field must carry inline enum variants")
            .iter()
            .map(|v| v.as_str().expect("enum variant is a string"))
            .collect();
        assert_eq!(got, variants, "inline enum variants must match: {field}");
    }

    #[test]
    fn assert_relationship_schema_inlines_edge_kind_and_polarity() {
        let schema = rmcp_schema_for::<AssertRelationshipInput>();
        assert!(
            schema.get("$defs").is_none(),
            "no $defs block should remain once enums are inlined: {schema}"
        );
        let props = &schema["properties"];
        assert_inline_string_enum(
            &props["edge_kind"],
            &["express", "contains", "leads_to", "near"],
        );
        assert_inline_string_enum(&props["polarity"], &["forward", "inverse"]);
    }

    #[test]
    fn retype_relationship_schema_inlines_edge_kind_and_polarity() {
        let schema = rmcp_schema_for::<RetypeRelationshipInput>();
        assert!(
            schema.get("$defs").is_none(),
            "no $defs block should remain once enums are inlined: {schema}"
        );
        let props = &schema["properties"];
        assert_inline_string_enum(
            &props["edge_kind"],
            &["express", "contains", "leads_to", "near"],
        );
        assert_inline_string_enum(&props["polarity"], &["forward", "inverse"]);
    }

    #[test]
    fn assert_relationship_input_edge_kind_variants() {
        for (kind_str, expected) in [
            ("express", EdgeKind::Express),
            ("contains", EdgeKind::Contains),
            ("leads_to", EdgeKind::LeadsTo),
            ("near", EdgeKind::Near),
        ] {
            let json = serde_json::json!({
                "source": "019e84ab-26ba-7560-9d34-c60d74a9fbe2",
                "target": "019e84ab-26ba-7560-9d34-c60d74a9fbe3",
                "edge_kind": kind_str,
                "polarity": "forward",
                "label": "test",
                "weight": 0.8
            });
            let input: AssertRelationshipInput = serde_json::from_value(json).unwrap();
            assert_eq!(
                input.edge_kind, expected,
                "edge_kind {kind_str} should deserialize"
            );
        }
    }
}
