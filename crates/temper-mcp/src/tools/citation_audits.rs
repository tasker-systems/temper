//! Citation-audit tool — record an auditor's signed defensibility verdict on one citation.
//!
//! Execution crosses the DEPLOYED API over the wire (beat G3c, the network door — the
//! fourth family to cross): the handler drives a per-request temper-client HTTP relay
//! built from the request's `Parts` and never touches the pool, the `DbBackend`, or a
//! service function directly, forwarding to the **block-addressed audit route**
//! `POST /api/citation-audits` (`temper-api/src/handlers/citation_audits.rs::record_for_block`,
//! merged PR #961) on the caller's own bearer. `BlockCitationAuditRequest` is the tool
//! input 1:1 — the MCP input's `act: ActInput` maps STRAIGHT THROUGH into the request's
//! `act`, never defaulted, never dropped: an audit is an authored, per-act write, and the
//! write keeps the authorship the caller supplied. `RecordCitationAudit` carries no finding
//! id and none may be added: the server derives the authorization subject from `block_id`
//! (`temper-services/src/authz/audit_gate.rs:65-77`), so this tool does not authorize — it
//! forwards, like every other network-door tool, and the API's gate is the ONE gate.
//!
//! # The one-string-by-construction property survives the wire
//!
//! Every finding-shaped refusal is one string by construction (`authz::audit_gate`'s
//! `FINDING_REFUSAL`) precisely so a prober cannot tell "no such finding" from "exists but
//! not yours" from "yours, so you may not grade it". Over the wire those arrive as 404s
//! whose bodies carry the server's constant — and this tool renders **its own fixed
//! sentence** keyed on the 404 status, byte-identical to the direct binding's arm, NEVER
//! the server's 404 message: one string in, one string out, no election between causes.
//! The status-keying carries one residual risk the in-process direct binding lacked: a
//! ROUTING 404 (a wrong path, or the fallback handler) would render as this same
//! finding-refusal sentence — which is exactly what the dead-base-URL probe class in the
//! parity suite catches on every suite run.
//!
//! # Declared parity deltas
//!
//! None new: the fixed 404 sentence is the tool's own (kept byte-exact), the 400 face
//! passes the server's sentence through bare exactly as the direct pass-through did, and
//! the family-wide deltas (the caller-actionable 409 face, the dropped NotFound prefix)
//! are declared in `relationships.rs`/`facets.rs` — the audit's 404 arm is exempt from
//! the prefix delta because it never carried the server's message at all.

use rmcp::model::CallToolResult;
use schemars::JsonSchema;
use serde::Deserialize;
use uuid::Uuid;

use temper_client::error::ClientError;
use temper_core::types::authorship::ActInput;
use temper_core::types::citation_audit::BlockCitationAuditRequest;
use temper_core::types::provenance::ProvenanceSource;

use crate::service::{api_error_cause, AcrossAuth, TemperMcpService};

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

/// Map an audit-path error onto an MCP error, arm-for-arm with the direct binding.
///
/// **Deliberately does NOT carry the server's 404 message, unlike its NotFound siblings.**
/// Every finding-shaped refusal is one string by construction (`authz::audit_gate`'s
/// `FINDING_REFUSAL`) precisely so a prober cannot tell "no such finding" from "exists but
/// not yours" from "yours, so you may not grade it". This arm enumerates all three without
/// electing between them, which is the same guarantee stated locally — keyed on the 404
/// status, never on the body. Making it message-transparent is safe only for as long as
/// everything upstream is that constant — a coupling nothing here can enforce, so it stays
/// independent.
fn map_err(e: ClientError, action: &str) -> rmcp::ErrorData {
    match e {
        ClientError::NotFound { .. } => rmcp::ErrorData::invalid_params(
            format!("{action}: finding not found, unreadable, or self-authored"),
            None,
        ),
        ClientError::Server {
            status: 400,
            message,
        } => rmcp::ErrorData::invalid_params(api_error_cause(&message).to_string(), None),
        // The door's 409 is caller-actionable — the direct catch-all rendered it
        // internal_error (the family's declared delta).
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
            format!("{action}: cannot audit this citation"),
            None,
        ),
        other => rmcp::ErrorData::internal_error(format!("{action}: {other}"), None),
    }
}

// ── Tool handlers ──────────────────────────────────────────────────────────────

/// Record an auditor's signed defensibility verdict on one `(block, source)` citation.
///
/// Forwards to the block-addressed route `POST /api/citation-audits` through the network
/// door; the act rides the request verbatim.
pub async fn record_citation_audit(
    svc: &TemperMcpService,
    parts: &http::request::Parts,
    input: RecordCitationAuditInput,
) -> Result<CallToolResult, rmcp::ErrorData> {
    let request = BlockCitationAuditRequest {
        block_id: input.block_id,
        source: input.source,
        value: input.value,
        reason: input.reason,
        // Straight through: never defaulted, never dropped.
        act: input.act,
    };

    let client = svc.relay_client(parts)?;
    let audit_id = client
        .resources()
        .record_citation_audit_for_block(&request)
        .await
        .across_auth(|e| map_err(e, "record_citation_audit"))?;

    // Matches the HTTP handler's response shape exactly: a bare audit id
    // (`temper-api/src/handlers/citation_audits.rs` returns `Json<Uuid>`), not a wrapping ack
    // struct — there is no shared `CitationAuditAck` type, and inventing one here would be a
    // second spelling of a response shape the HTTP surface already settled.
    Ok(CallToolResult::success(vec![
        rmcp::model::ContentBlock::text(to_text(&audit_id)),
    ]))
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
