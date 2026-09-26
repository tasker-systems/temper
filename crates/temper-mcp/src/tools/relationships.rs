//! Relationship tools — assert, retype, reweight, and fold graph edges.
//!
//! Execution crosses the DEPLOYED API over the wire (beat G3c, the network door — the
//! fourth family to cross): every verb drives a per-request temper-client HTTP relay
//! built from the request's `Parts` and never touches the pool, the `DbBackend`, or a
//! service function directly, forwarding to the same `/api/relationships` routes the CLI
//! calls on the caller's own bearer. The input's `ActInput` maps STRAIGHT THROUGH into
//! each request's `act` — never defaulted, never dropped — and the `origin: Surface::Mcp`
//! stamp the direct binding put on each command is now the relay's planted carrier at
//! the door. The verbs' ref arguments (source/target/edge handles) are resolved
//! MCP-local via `parse_ref` before the forward.
//!
//! # Declared parity deltas (the direct faces pinned by `ledger_graph_parity_test.rs`,
//! re-derived at the door and flipped there deliberately)
//!
//! - **Closed-invocation 409**: the direct `map_err` had no `Conflict` arm, so the act
//!   gate's non-open refusal fell to the `internal_error` catch-all. The wire's 409 is
//!   caller-actionable — the run the correlation claim names is closed, which no retry
//!   changes and no input edit repairs except naming an open run — so it renders
//!   `invalid_params` with the server's own sentence, the `contexts.rs::map_api_error`
//!   Conflict arm's precedent (`a taken slug is caller-fixable, not an internal error`).
//!   The suite pinned `internal_error` pre-swap and pins the door's face now.
//! - **NotFound prefixes**: the direct map prefixed every not-found with `{action}: `.
//!   Over the wire, `ClientError::NotFound` carries the server's own sentence, and the
//!   door does not re-apply prefixes the direct tool applied — the kind (`invalid_params`)
//!   and the gate are identical, per the resources family's precedent (`resources.rs`'s
//!   context-refusal arm). Only the prefix drops; every sentence the server speaks is
//!   carried through verbatim.
//!
//! Every other arm maps arm-for-arm: `ForbiddenDetail` speaks the gate's own sentence
//! under INVALID_REQUEST prefixed `{action}: ` (the direct face byte-for-byte), the terse
//! `Forbidden` arm keeps the tool's fixed "cannot modify this resource" sentence, and the
//! 400 face passes the server's sentence through bare — the direct binding's pass-through.

use rmcp::model::CallToolResult;
use schemars::JsonSchema;
use serde::Deserialize;
use uuid::Uuid;

use temper_client::error::ClientError;
use temper_core::types::authorship::ActInput;
use temper_core::types::graph::{EdgeKind, Polarity};
use temper_core::types::relationship_requests::{
    AssertRelationshipRequest, FoldRelationshipRequest, RelationshipTarget,
    RetypeRelationshipRequest, ReweightRelationshipRequest,
};

use crate::service::{api_error_cause, AcrossAuth, TemperMcpService};

// ── Input structs ──────────────────────────────────────────────────────────────

/// MCP input for assert_relationship.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct AssertRelationshipInput {
    /// Source resource ref: a UUID or the decorated `slug-<uuid>` form.
    pub source: String,
    /// The target's id: a resource UUID or the decorated `slug-<uuid>` form when
    /// `target_table` is `resource`; a bare blob UUID when it is `blob`.
    pub target: String,
    /// What `target` addresses. Defaults to `resource`; `blob` points the edge
    /// at a binary blob the caller can READ (its source doc / evidence), with
    /// the edge homed in the source resource's home.
    #[serde(default)]
    pub target_table: RelationshipTarget,
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
    /// Per-act correlation (`invocation_id`) + discrete agent authorship. Flattened top-level
    /// keys; all optional. `confidence` required when any other authorship field is supplied.
    #[serde(flatten)]
    pub act: ActInput,
}

/// MCP input for reweight_relationship.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct ReweightRelationshipInput {
    /// Relationship correlation ID returned by assert_relationship.
    pub edge_handle: Uuid,
    /// New edge weight.
    pub weight: f64,
    /// Per-act correlation (`invocation_id`) + discrete agent authorship. Flattened top-level
    /// keys; all optional. `confidence` required when any other authorship field is supplied.
    #[serde(flatten)]
    pub act: ActInput,
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

/// Map a relationship-path error onto an MCP error, arm-for-arm with the direct binding.
///
/// The two declared parity deltas (409, NotFound prefix) and the arm-for-arm
/// discipline are argued in the module doc.
fn map_err(e: ClientError, action: &str) -> rmcp::ErrorData {
    match e {
        // The server's own sentence, un-prefixed: the direct binding's tool wrapped
        // each not-found with "{action}: ", a prefix the door does not re-apply —
        // the kind (invalid_params) and the gate are identical.
        ClientError::NotFound { message } => rmcp::ErrorData::invalid_params(message, None),
        ClientError::Server {
            status: 400,
            message,
        } => rmcp::ErrorData::invalid_params(api_error_cause(&message).to_string(), None),
        // The door's 409 is caller-actionable — the direct catch-all rendered it
        // internal_error (the declared delta).
        ClientError::Conflict { message } => {
            rmcp::ErrorData::invalid_params(api_error_cause(&message).to_string(), None)
        }
        // A refusal that named the capability it withheld — carry the gate's own sentence. The
        // terse arm below stays exactly as it was, for the caller who may not even read the subject.
        ClientError::ForbiddenDetail { message } => rmcp::ErrorData::new(
            rmcp::model::ErrorCode::INVALID_REQUEST,
            format!("{action}: {message}"),
            None,
        ),
        ClientError::Forbidden => rmcp::ErrorData::new(
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
    parts: &http::request::Parts,
    input: AssertRelationshipInput,
) -> Result<CallToolResult, rmcp::ErrorData> {
    // MCP-local input shaping: the ref parses never touch the wire.
    let source = temper_workflow::operations::parse_ref(&input.source)
        .map_err(|e| rmcp::ErrorData::invalid_params(e.to_string(), None))?;
    let target = temper_workflow::operations::parse_ref(&input.target)
        .map_err(|e| rmcp::ErrorData::invalid_params(e.to_string(), None))?;

    let request = AssertRelationshipRequest {
        source,
        target,
        target_table: input.target_table,
        edge_kind: input.edge_kind,
        polarity: input.polarity,
        label: input.label,
        weight: input.weight,
        // Straight through: never defaulted, never dropped.
        act: input.act,
    };

    let client = svc.relay_client(parts)?;
    let ack = client
        .relationships()
        .assert(&request)
        .await
        .across_auth(|e| map_err(e, "assert_relationship"))?;

    Ok(CallToolResult::success(vec![
        rmcp::model::ContentBlock::text(to_text(&ack)),
    ]))
}

pub async fn retype_relationship(
    svc: &TemperMcpService,
    parts: &http::request::Parts,
    input: RetypeRelationshipInput,
) -> Result<CallToolResult, rmcp::ErrorData> {
    let request = RetypeRelationshipRequest {
        edge_kind: input.edge_kind,
        polarity: input.polarity,
        act: input.act,
    };

    let client = svc.relay_client(parts)?;
    let ack = client
        .relationships()
        .retype(input.edge_handle, &request)
        .await
        .across_auth(|e| map_err(e, "retype_relationship"))?;

    Ok(CallToolResult::success(vec![
        rmcp::model::ContentBlock::text(to_text(&ack)),
    ]))
}

pub async fn reweight_relationship(
    svc: &TemperMcpService,
    parts: &http::request::Parts,
    input: ReweightRelationshipInput,
) -> Result<CallToolResult, rmcp::ErrorData> {
    let request = ReweightRelationshipRequest {
        weight: input.weight,
        act: input.act,
    };

    let client = svc.relay_client(parts)?;
    let ack = client
        .relationships()
        .reweight(input.edge_handle, &request)
        .await
        .across_auth(|e| map_err(e, "reweight_relationship"))?;

    Ok(CallToolResult::success(vec![
        rmcp::model::ContentBlock::text(to_text(&ack)),
    ]))
}

pub async fn fold_relationship(
    svc: &TemperMcpService,
    parts: &http::request::Parts,
    input: FoldRelationshipInput,
) -> Result<CallToolResult, rmcp::ErrorData> {
    let request = FoldRelationshipRequest {
        reason: input.reason,
        act: input.act,
    };

    let client = svc.relay_client(parts)?;
    let ack = client
        .relationships()
        .fold(input.edge_handle, &request)
        .await
        .across_auth(|e| map_err(e, "fold_relationship"))?;

    Ok(CallToolResult::success(vec![
        rmcp::model::ContentBlock::text(to_text(&ack)),
    ]))
}

// ── Consolidated tool (4→1) ─────────────────────────────────────────────────────

/// The relationship action to perform.
#[derive(Debug, Deserialize, JsonSchema)]
#[schemars(inline)]
#[serde(rename_all = "snake_case")]
pub enum RelationshipAction {
    /// Assert a new directed relationship from source to target.
    Assert,
    /// Change the edge_kind and polarity of an existing relationship.
    Retype,
    /// Update the weight of an existing relationship.
    Reweight,
    /// Retract (fold) an existing relationship, marking it inactive.
    Fold,
}

/// Consolidated relationship tool — one write tool with an `action` discriminator.
///
/// Collapses `assert_relationship`, `retype_relationship`, `reweight_relationship`,
/// and `fold_relationship` into a single MCP tool. The `action` field selects which
/// operation to perform; only the fields relevant to that action are required.
///
/// **Action → required fields:**
/// - `assert`: `source`, `target`, `edge_kind`, `polarity`, `label`, `weight`
/// - `retype`: `edge_handle`, `edge_kind`, `polarity`
/// - `reweight`: `edge_handle`, `weight`
/// - `fold`: `edge_handle` (`reason` optional)
#[derive(Debug, Deserialize, JsonSchema)]
pub struct RelationshipInput {
    /// Which relationship action to perform.
    pub action: RelationshipAction,
    /// Source resource ref (UUID or `slug-<uuid>`). Required for `assert`; ignored otherwise.
    #[serde(default)]
    pub source: Option<String>,
    /// Target ref: a resource (UUID or `slug-<uuid>`), or a bare blob UUID when
    /// `target_table` is `blob`. Required for `assert`; ignored otherwise.
    #[serde(default)]
    pub target: Option<String>,
    /// What `target` addresses. Defaults to `resource`; `blob` points the edge
    /// at a binary blob the caller can READ (its source doc / evidence), with
    /// the edge homed in the source resource's home. Used only with `assert`.
    #[serde(default)]
    pub target_table: Option<RelationshipTarget>,
    /// The edge handle (from `assert`'s response). Required for `retype`, `reweight`, `fold`; ignored for `assert`.
    #[serde(default)]
    pub edge_handle: Option<Uuid>,
    /// Structural edge kind — `express`, `contains`, `leads_to`, `near`. Required for `assert` and `retype`.
    #[serde(default)]
    pub edge_kind: Option<EdgeKind>,
    /// Edge direction sign — `forward` or `inverse`. Required for `assert` and `retype`.
    #[serde(default)]
    pub polarity: Option<Polarity>,
    /// Human-readable relation label (e.g. `depends_on`). Required for `assert`.
    #[serde(default)]
    pub label: Option<String>,
    /// Numeric edge weight (0.0–1.0). Required for `assert` and `reweight`.
    #[serde(default)]
    pub weight: Option<f64>,
    /// Optional human-readable reason for folding. Used only with `fold`.
    #[serde(default)]
    pub reason: Option<String>,
    /// Per-act correlation (`invocation_id`) + discrete agent authorship. Flattened top-level
    /// keys; all optional. `confidence` required when any other authorship field is supplied.
    #[serde(flatten)]
    pub act: ActInput,
}

/// Dispatch the consolidated relationship tool.
pub async fn relationship(
    svc: &TemperMcpService,
    parts: &http::request::Parts,
    input: RelationshipInput,
) -> Result<CallToolResult, rmcp::ErrorData> {
    match input.action {
        RelationshipAction::Assert => {
            let source = input.source.ok_or_else(|| {
                rmcp::ErrorData::invalid_params("assert requires `source`".to_string(), None)
            })?;
            let target = input.target.ok_or_else(|| {
                rmcp::ErrorData::invalid_params("assert requires `target`".to_string(), None)
            })?;
            let edge_kind = input.edge_kind.ok_or_else(|| {
                rmcp::ErrorData::invalid_params("assert requires `edge_kind`".to_string(), None)
            })?;
            let polarity = input.polarity.ok_or_else(|| {
                rmcp::ErrorData::invalid_params("assert requires `polarity`".to_string(), None)
            })?;
            let label = input.label.ok_or_else(|| {
                rmcp::ErrorData::invalid_params("assert requires `label`".to_string(), None)
            })?;
            let weight = input.weight.ok_or_else(|| {
                rmcp::ErrorData::invalid_params("assert requires `weight`".to_string(), None)
            })?;
            assert_relationship(
                svc,
                parts,
                AssertRelationshipInput {
                    source,
                    target,
                    target_table: input.target_table.unwrap_or_default(),
                    edge_kind,
                    polarity,
                    label,
                    weight,
                    act: input.act,
                },
            )
            .await
        }
        RelationshipAction::Retype => {
            let edge_handle = input.edge_handle.ok_or_else(|| {
                rmcp::ErrorData::invalid_params("retype requires `edge_handle`".to_string(), None)
            })?;
            let edge_kind = input.edge_kind.ok_or_else(|| {
                rmcp::ErrorData::invalid_params("retype requires `edge_kind`".to_string(), None)
            })?;
            let polarity = input.polarity.ok_or_else(|| {
                rmcp::ErrorData::invalid_params("retype requires `polarity`".to_string(), None)
            })?;
            retype_relationship(
                svc,
                parts,
                RetypeRelationshipInput {
                    edge_handle,
                    edge_kind,
                    polarity,
                    act: input.act,
                },
            )
            .await
        }
        RelationshipAction::Reweight => {
            let edge_handle = input.edge_handle.ok_or_else(|| {
                rmcp::ErrorData::invalid_params("reweight requires `edge_handle`".to_string(), None)
            })?;
            let weight = input.weight.ok_or_else(|| {
                rmcp::ErrorData::invalid_params("reweight requires `weight`".to_string(), None)
            })?;
            reweight_relationship(
                svc,
                parts,
                ReweightRelationshipInput {
                    edge_handle,
                    weight,
                    act: input.act,
                },
            )
            .await
        }
        RelationshipAction::Fold => {
            let edge_handle = input.edge_handle.ok_or_else(|| {
                rmcp::ErrorData::invalid_params("fold requires `edge_handle`".to_string(), None)
            })?;
            fold_relationship(
                svc,
                parts,
                FoldRelationshipInput {
                    edge_handle,
                    reason: input.reason,
                    act: input.act,
                },
            )
            .await
        }
    }
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
    fn retype_relationship_input_accepts_act_authorship_fields() {
        let id = Uuid::new_v4();
        let json = serde_json::json!({
            "edge_handle": id.to_string(),
            "edge_kind": "near",
            "polarity": "forward",
            "invocation_id": "019f0e28-1750-7490-919f-5e51c92c8391",
            "reasoning": "kind was mislabelled",
            "confidence": "probable",
        });
        let input: RetypeRelationshipInput = serde_json::from_value(json).unwrap();
        assert!(input.act.invocation_id.is_some());
        assert_eq!(
            input.act.confidence,
            Some(temper_core::types::ConfidenceBand::Probable)
        );
        assert!(!input.act.into_act_context().expect("assembles").is_empty());
    }

    #[test]
    fn reweight_relationship_input_accepts_act_authorship_fields() {
        let id = Uuid::new_v4();
        let json = serde_json::json!({
            "edge_handle": id.to_string(),
            "weight": 0.5,
            "confidence": "tentative",
        });
        let input: ReweightRelationshipInput = serde_json::from_value(json).unwrap();
        assert_eq!(
            input.act.confidence,
            Some(temper_core::types::ConfidenceBand::Tentative)
        );
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
