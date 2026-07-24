//! Citation-audit wire type (Set 5, spec
//! `docs/superpowers/specs/2026-07-23-set5-adversary-citation-audit-design.md` §4.1-4.2) — the
//! request body an auditor sends to record one signed verdict.
//!
//! **A citation is a `(block, source)` pair**, so an audit is addressed at that grain and not at
//! the resource's. The finding is the path segment of `POST /api/resources/{id}/citation-audits`,
//! and it is a *routing* address only: the server derives the authorization subject from
//! `block_id` and refuses a mismatch, so naming a finding here confers nothing
//! (`temper-services/src/authz/audit_gate.rs:65-77`).
//!
//! **The auditor's own confidence does not belong on this body.** It rides the act envelope
//! (`ActContext` → `kb_events.metadata`); only the signed `value` below is ever read by the
//! standing projection. That is spec §4.2's self-grading prohibition: an agent's confidence in its
//! own verdict must never move standing (`temper-substrate/src/writes.rs:538-541`).

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::types::provenance::ProvenanceSource;

/// Request body for `POST /api/resources/{id}/citation-audits`.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "citation_audit.ts"))]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
pub struct CitationAuditRequest {
    /// The audited citation's block (`kb_content_blocks.id`). The server resolves this to its
    /// owning finding — that resolved finding, never the path id, is what authorization is
    /// evaluated over.
    pub block_id: Uuid,
    /// The cited source being assessed. Only `Resource`-kind citations are auditable: standing
    /// reads only resource-kind bases, so the SQL entry refuses anything else at the write path
    /// rather than letting it land as a no-op the auditor could never detect
    /// (`migrations/20260723000010_citation_audits.sql:123-126`).
    pub source: ProvenanceSource,
    /// The signed verdict in `[-1.0, 1.0]` — how much defensibility this citation confers for the
    /// connection it makes, never a claim about what the source says (spec §3.3). Out-of-range is a
    /// 400; the ledger column carries the same bound as a CHECK
    /// (`migrations/20260723000010_citation_audits.sql:28`).
    pub value: f64,
    /// Optional free-text rationale, recorded on the ledger row.
    pub reason: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The source rides as the tagged `{kind, value}` shape the SQL entry reads nested
    /// (`citation_audit` does `p_payload #>> '{source,kind}'`), so a request that serializes it
    /// flat would be rejected server-side. Pins the wire shape, not just the round-trip.
    #[test]
    fn request_carries_the_tagged_source_shape() {
        let req = CitationAuditRequest {
            block_id: Uuid::now_v7(),
            source: ProvenanceSource::Resource(Uuid::nil()),
            value: -1.0,
            reason: None,
        };
        let v = serde_json::to_value(&req).unwrap();
        assert_eq!(v["source"]["kind"], "resource");
        let back: CitationAuditRequest = serde_json::from_value(v).unwrap();
        assert_eq!(back, req);
    }
}
