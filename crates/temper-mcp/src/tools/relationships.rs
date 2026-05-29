//! Relationship tools — assert, retype, reweight, and fold graph edges.
//!
//! Each tool mirrors one HTTP endpoint from `temper-api/src/handlers/edges.rs`
//! and dispatches through `DbBackend` — the same write path the HTTP handlers
//! use. The source resource is specified via four flat fields that map to a
//! `ResourceRef::Scoped` at dispatch time. `ResourceRef` has no `JsonSchema`
//! derive, so it cannot be exposed directly in an MCP input struct.

use rmcp::model::CallToolResult;
use schemars::JsonSchema;
use serde::Deserialize;
use uuid::Uuid;

use temper_api::backend::DbBackend;
use temper_core::error::TemperError;
use temper_core::operations::{
    AssertRelationship, FoldRelationship, ResourceRef, RetypeRelationship, ReweightRelationship,
    Surface,
};
use temper_core::types::graph::{EdgeKind, Polarity};
use temper_core::types::ids::ProfileId;
use temper_core::types::relationship_requests::RelationshipAck;

use crate::service::TemperMcpService;

// ── Input structs ──────────────────────────────────────────────────────────────

/// MCP input for assert_relationship.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct AssertRelationshipInput {
    /// Owner of the source resource (e.g. `@me` or `+team-acme`).
    pub source_owner: String,
    /// Context name of the source resource (e.g. `temper`).
    pub source_context: String,
    /// Doc-type name of the source resource (e.g. `task`, `research`).
    pub source_doctype: String,
    /// Slug of the source resource.
    pub source_slug: String,
    /// Slug of the target resource (resolved within the source's context).
    pub target_slug: String,
    /// Structural edge kind — one of `express`, `contains`, `leads_to`, `near`.
    pub edge_kind: EdgeKind,
    /// Edge direction sign — `forward` or `inverse`.
    pub polarity: Polarity,
    /// Human-readable relation label (e.g. `depends_on`, `parent_of`).
    pub label: String,
    /// Numeric edge weight (0.0–1.0 by convention; exact range is schema-defined).
    pub weight: f64,
}

/// MCP input for retype_relationship.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct RetypeRelationshipInput {
    /// Relationship correlation ID returned by assert_relationship.
    pub correlation_id: Uuid,
    /// New structural edge kind.
    pub edge_kind: EdgeKind,
    /// New edge direction sign.
    pub polarity: Polarity,
}

/// MCP input for reweight_relationship.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct ReweightRelationshipInput {
    /// Relationship correlation ID returned by assert_relationship.
    pub correlation_id: Uuid,
    /// New edge weight.
    pub weight: f64,
}

/// MCP input for fold_relationship.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct FoldRelationshipInput {
    /// Relationship correlation ID returned by assert_relationship.
    pub correlation_id: Uuid,
    /// Optional human-readable reason for retracting the relationship.
    pub reason: Option<String>,
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

    let source = ResourceRef::scoped(
        input.source_owner,
        input.source_context,
        input.source_doctype,
        input.source_slug,
    );

    let cmd = AssertRelationship {
        source,
        target_slug: input.target_slug,
        edge_kind: input.edge_kind,
        polarity: input.polarity,
        label: input.label,
        weight: input.weight,
        origin: Surface::Mcp,
    };

    let backend = DbBackend::new(pool.clone(), profile_id, "mcp".to_string(), Surface::Mcp);
    let out = backend
        .assert_relationship(cmd)
        .await
        .map_err(|e| map_err(e, "assert_relationship"))?;

    let ack = RelationshipAck {
        correlation_id: out.value,
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
        correlation_id: input.correlation_id,
        edge_kind: input.edge_kind,
        polarity: input.polarity,
        origin: Surface::Mcp,
    };

    let backend = DbBackend::new(pool.clone(), profile_id, "mcp".to_string(), Surface::Mcp);
    let out = backend
        .retype_relationship(cmd)
        .await
        .map_err(|e| map_err(e, "retype_relationship"))?;

    let ack = RelationshipAck {
        correlation_id: out.value,
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
        correlation_id: input.correlation_id,
        weight: input.weight,
        origin: Surface::Mcp,
    };

    let backend = DbBackend::new(pool.clone(), profile_id, "mcp".to_string(), Surface::Mcp);
    let out = backend
        .reweight_relationship(cmd)
        .await
        .map_err(|e| map_err(e, "reweight_relationship"))?;

    let ack = RelationshipAck {
        correlation_id: out.value,
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

    let cmd = FoldRelationship {
        correlation_id: input.correlation_id,
        reason: input.reason,
        origin: Surface::Mcp,
    };

    let backend = DbBackend::new(pool.clone(), profile_id, "mcp".to_string(), Surface::Mcp);
    let out = backend
        .fold_relationship(cmd)
        .await
        .map_err(|e| map_err(e, "fold_relationship"))?;

    let ack = RelationshipAck {
        correlation_id: out.value,
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
            "source_owner": "@me",
            "source_context": "temper",
            "source_doctype": "task",
            "source_slug": "foo",
            "target_slug": "bar",
            "edge_kind": "leads_to",
            "polarity": "inverse",
            "label": "depends_on",
            "weight": 1.0
        });
        let input: AssertRelationshipInput = serde_json::from_value(json).unwrap();
        assert_eq!(input.source_slug, "foo");
        assert_eq!(input.source_owner, "@me");
        assert_eq!(input.source_context, "temper");
        assert_eq!(input.source_doctype, "task");
        assert_eq!(input.target_slug, "bar");
        assert_eq!(input.edge_kind, EdgeKind::LeadsTo);
        assert_eq!(input.polarity, Polarity::Inverse);
        assert_eq!(input.label, "depends_on");
        assert_eq!(input.weight, 1.0);
    }

    #[test]
    fn retype_relationship_input_deserializes() {
        let id = Uuid::new_v4();
        let json = serde_json::json!({
            "correlation_id": id.to_string(),
            "edge_kind": "near",
            "polarity": "forward"
        });
        let input: RetypeRelationshipInput = serde_json::from_value(json).unwrap();
        assert_eq!(input.correlation_id, id);
        assert_eq!(input.edge_kind, EdgeKind::Near);
        assert_eq!(input.polarity, Polarity::Forward);
    }

    #[test]
    fn reweight_relationship_input_deserializes() {
        let id = Uuid::new_v4();
        let json = serde_json::json!({
            "correlation_id": id.to_string(),
            "weight": 0.5
        });
        let input: ReweightRelationshipInput = serde_json::from_value(json).unwrap();
        assert_eq!(input.correlation_id, id);
        assert_eq!(input.weight, 0.5);
    }

    #[test]
    fn fold_relationship_input_deserializes_with_reason() {
        let id = Uuid::new_v4();
        let json = serde_json::json!({
            "correlation_id": id.to_string(),
            "reason": "no longer relevant"
        });
        let input: FoldRelationshipInput = serde_json::from_value(json).unwrap();
        assert_eq!(input.correlation_id, id);
        assert_eq!(input.reason, Some("no longer relevant".to_string()));
    }

    #[test]
    fn fold_relationship_input_deserializes_without_reason() {
        let id = Uuid::new_v4();
        let json = serde_json::json!({
            "correlation_id": id.to_string()
        });
        let input: FoldRelationshipInput = serde_json::from_value(json).unwrap();
        assert_eq!(input.correlation_id, id);
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
                "source_owner": "@me",
                "source_context": "ctx",
                "source_doctype": "task",
                "source_slug": "s",
                "target_slug": "t",
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
