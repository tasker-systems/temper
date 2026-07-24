//! Citation-audit tool — record an auditor's signed defensibility verdict on one citation.
//!
//! Mirrors the HTTP endpoint `POST /api/resources/{id}/citation-audits`
//! (`temper-api/src/handlers/citation_audits.rs`) and dispatches through `DbBackend` — the same
//! write path the HTTP handler uses. `RecordCitationAudit` carries no finding id: the server
//! derives the authorization subject from `block_id` server-side
//! (`temper-services/src/authz/audit_gate.rs:65-77`), so this tool does not authorize — it
//! dispatches, like every other MCP tool, and the backend command is the ONE gate.

use rmcp::model::CallToolResult;
use schemars::JsonSchema;
use serde::Deserialize;
use uuid::Uuid;

use temper_core::error::TemperError;
use temper_core::types::authorship::ActInput;
use temper_core::types::ids::{BlockId, ProfileId};
use temper_core::types::provenance::ProvenanceSource;
use temper_services::backend::DbBackend;
use temper_workflow::operations::{Backend, RecordCitationAudit, Surface};

use crate::service::TemperMcpService;

// ── Input structs ──────────────────────────────────────────────────────────────

/// MCP input for record_citation_audit.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct RecordCitationAuditInput {
    /// The audited citation's block (`kb_content_blocks.id`). The server resolves this to its
    /// owning finding — that resolved finding, never a finding named by the caller, is what
    /// authorization is evaluated over.
    pub block_id: Uuid,
    /// The cited source being assessed. Only `Resource`-kind sources are auditable; anything
    /// else is refused at the write path.
    pub source: ProvenanceSource,
    /// The signed defensibility verdict in `[-1.0, 1.0]`: how much this source supports the
    /// specific connection the citation claims — never whether the underlying claim is true, and
    /// never what the source itself says. A strongly negative value expresses "this source does
    /// not carry the connection made here" without asserting what the source does say. Positive
    /// values reinforce the citation; the auditor did not author it, so assessing it in either
    /// direction is the adversarial relation, not self-grading. Out-of-range values are refused
    /// (the ledger column carries the same `[-1.0, 1.0]` CHECK).
    pub value: f64,
    /// Optional free-text rationale, recorded on the ledger row.
    pub reason: Option<String>,
    /// Per-act correlation (`invocation_id`) + discrete agent authorship. Flattened top-level
    /// keys; all optional. `confidence` required when any other authorship field is supplied.
    /// The auditor's own confidence in its verdict belongs here, never in `value` — only the
    /// signed verdict ever moves standing.
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
            format!("{action}: finding not found, unreadable, or self-authored"),
            None,
        ),
        TemperError::BadRequest(msg) => rmcp::ErrorData::invalid_params(msg, None),
        TemperError::Forbidden => rmcp::ErrorData::new(
            rmcp::model::ErrorCode::INVALID_REQUEST,
            format!("{action}: cannot audit this citation"),
            None,
        ),
        other => rmcp::ErrorData::internal_error(format!("{action}: {other}"), None),
    }
}

// ── Tool handlers ──────────────────────────────────────────────────────────────

/// Record an auditor's signed defensibility verdict on one `(block, source)` citation.
///
/// CLI/HTTP equivalent: `POST /api/resources/{id}/citation-audits`.
pub async fn record_citation_audit(
    svc: &TemperMcpService,
    input: RecordCitationAuditInput,
) -> Result<CallToolResult, rmcp::ErrorData> {
    let profile = svc.require_profile().await?;
    let pool = &svc.api_state.pool;
    let profile_id = ProfileId::from(profile.id);

    let act = input
        .act
        .into_act_context()
        .map_err(|e| rmcp::ErrorData::invalid_params(e.to_string(), None))?;

    let cmd = RecordCitationAudit {
        block: BlockId::from(input.block_id),
        source: input.source,
        value: input.value,
        reason: input.reason,
        act,
        origin: Surface::Mcp,
    };

    let backend = DbBackend::new(pool.clone(), profile_id);
    let out = backend
        .record_citation_audit(cmd)
        .await
        .map_err(|e| map_err(e, "record_citation_audit"))?;

    // Matches the HTTP handler's response shape exactly: a bare audit id
    // (`temper-api/src/handlers/citation_audits.rs` returns `Json<Uuid>`), not a wrapping ack
    // struct — there is no shared `CitationAuditAck` type, and inventing one here would be a
    // second spelling of a response shape the HTTP surface already settled.
    Ok(CallToolResult::success(vec![rmcp::model::Content::text(
        to_text(&out.value),
    )]))
}

// ── Tests ──────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn record_citation_audit_input_deserializes() {
        let json = serde_json::json!({
            "block_id": "019e84ab-26ba-7560-9d34-c60d74a9fbe2",
            "source": {"kind": "resource", "value": "019e84ab-26ba-7560-9d34-c60d74a9fbe3"},
            "value": -0.8,
            "reason": "source does not support the causal claim"
        });
        let input: RecordCitationAuditInput = serde_json::from_value(json).unwrap();
        assert_eq!(
            input.block_id,
            Uuid::parse_str("019e84ab-26ba-7560-9d34-c60d74a9fbe2").unwrap()
        );
        assert_eq!(
            input.source,
            ProvenanceSource::Resource(
                Uuid::parse_str("019e84ab-26ba-7560-9d34-c60d74a9fbe3").unwrap()
            )
        );
        assert_eq!(input.value, -0.8);
        assert_eq!(
            input.reason,
            Some("source does not support the causal claim".to_string())
        );
    }

    #[test]
    fn record_citation_audit_input_deserializes_without_reason() {
        let json = serde_json::json!({
            "block_id": "019e84ab-26ba-7560-9d34-c60d74a9fbe2",
            "source": {"kind": "remote", "value": "https://example.com/doc"},
            "value": 0.5
        });
        let input: RecordCitationAuditInput = serde_json::from_value(json).unwrap();
        assert_eq!(
            input.source,
            ProvenanceSource::Remote("https://example.com/doc".into())
        );
        assert_eq!(input.reason, None);
    }

    #[test]
    fn record_citation_audit_input_accepts_act_authorship_fields() {
        let json = serde_json::json!({
            "block_id": "019e84ab-26ba-7560-9d34-c60d74a9fbe2",
            "source": {"kind": "event", "value": "019e84ab-26ba-7560-9d34-c60d74a9fbe3"},
            "value": 1.0,
            "invocation_id": "019f0e28-1750-7490-919f-5e51c92c8391",
            "reasoning": "cross-checked against the cited resource directly",
            "confidence": "confident",
        });
        let input: RecordCitationAuditInput = serde_json::from_value(json).unwrap();
        assert_eq!(
            input.act.confidence,
            Some(temper_core::types::ConfidenceBand::Confident)
        );
        assert!(input.act.invocation_id.is_some());
        let ctx = input.act.into_act_context().expect("assembles");
        assert!(!ctx.is_empty());
    }

    /// Generate the tool input schema the same way rmcp does at runtime
    /// (`SchemaSettings::draft2020_12`, ref-based generator). This is the exact path that
    /// surfaced the original bug: scalar/tagged enums emitted as `$ref` into `$defs` reach the
    /// Anthropic tool-use layer with no type signal and come back as `null`. See
    /// `crate::tools::relationships` for the fix this mirrors.
    fn rmcp_schema_for<T: schemars::JsonSchema>() -> serde_json::Value {
        let generator = schemars::generate::SchemaSettings::draft2020_12().into_generator();
        serde_json::to_value(generator.into_root_schema_for::<T>()).unwrap()
    }

    #[test]
    fn record_citation_audit_schema_inlines_provenance_source() {
        let schema = rmcp_schema_for::<RecordCitationAuditInput>();
        assert!(
            schema.get("$defs").is_none(),
            "no $defs block should remain once ProvenanceSource is inlined: {schema}"
        );
        let source_field = &schema["properties"]["source"];
        assert!(
            source_field.get("$ref").is_none(),
            "source field must be inlined, not a $ref: {source_field}"
        );
        // Tagged {kind, value} enum: each variant shape must be visible directly (oneOf/anyOf of
        // object schemas), not hidden behind a reference the model cannot resolve.
        let variants = source_field
            .get("oneOf")
            .or_else(|| source_field.get("anyOf"))
            .expect("source field must inline its variant shapes")
            .as_array()
            .expect("variants form an array");
        assert!(
            !variants.is_empty(),
            "inlined source field must carry variant shapes: {source_field}"
        );
    }
}
