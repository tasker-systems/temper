//! Citation-audit wire types (Set 5, spec
//! `internal/superpowers/specs/2026-07-23-set5-adversary-citation-audit-design.md` §4.1-4.2, §5.2) —
//! the request body an auditor sends to record one signed verdict, and the attributed trail read
//! back off the ledger.
//!
//! **Both shapes live here, not in `standing.rs`.** `standing.rs` holds `StandingShape`, whose own
//! doc states its subject: the *aggregate* shape of a finding's evidence, fixed-width and
//! shape-primary. [`CitationAuditRow`] is not a component of that shape — it is one row of the
//! ledger that shape summarizes, at the same `(block, source)` grain as
//! [`CitationAuditRequest`] right beside it. Grouping by grain keeps the request and the row a
//! caller gets back for it in one file; grouping by endpoint would split them.
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

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use uuid::Uuid;

use crate::types::ids::{BlockId, ProfileId, ResourceId};
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
    /// (`migrations/20260724000110_citation_audits.sql:123-126`).
    pub source: ProvenanceSource,
    /// The signed verdict in `[-1.0, 1.0]` — how much defensibility this citation confers for the
    /// connection it makes, never a claim about what the source says (spec §3.3). Out-of-range is a
    /// 400; the ledger column carries the same bound as a CHECK
    /// (`migrations/20260724000110_citation_audits.sql:28`).
    pub value: f64,
    /// Optional free-text rationale, recorded on the ledger row.
    pub reason: Option<String>,
}

/// One audit of one citation, with its auditor named — an element of the response to
/// `GET /api/resources/{id}/citation-audits` (SQL `resource_citation_audit_trail`,
/// `migrations/20260724000220`).
///
/// **This is the attribution `StandingShape` collapses away.** `audit_coverage` and
/// `citation_quality` are aggregates over exactly these rows, so a finding pushed to `disputed` by
/// one auditor and a finding pushed there by three read identically on the shape and differently
/// here. Spec §5.2: identity travels on the emitting event
/// (`kb_events.emitter_entity_id → kb_entities.profile_id`), which Beat 1 materialized as
/// `kb_citation_audits.audited_by_profile_id`; this row carries that profile plus the two
/// human-readable columns of `kb_profiles` (`handle`, `display_name` — both `NOT NULL`), so a
/// caller never has to make a second round trip to name a voter.
///
/// All fields are non-nullable except `reason`, which is nullable on the ledger. The access gate is
/// INSIDE the SQL, so an unreadable finding produces no rows at all rather than a redacted one.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "citation_audit.ts"))]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, FromRow)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
pub struct CitationAuditRow {
    /// `kb_citation_audits.id` — the same id `POST /api/resources/{id}/citation-audits` returns
    /// (`handlers::citation_audits::record` → `Json<Uuid>`), which is why it is a bare `Uuid` here
    /// and not a newtype. A **masked surrogate**: the replay-stable identity of an audit is its
    /// `audited_by_event_id`, so this addresses the row in this response, not across a replay.
    pub audit_id: Uuid,
    /// The audited citation's block. Carried alongside `source_id` because a citation IS the
    /// `(block, source)` pair — one source cited by two of the finding's blocks is two citations,
    /// separately auditable.
    pub block_id: BlockId,
    /// The audited citation's cited source. Always resource-kind — only resource-kind citations are
    /// auditable (the `citation_audit` write path rejects every other kind), so no `source_kind`
    /// discriminator is carried.
    pub source_id: ResourceId,
    /// The auditor's signed verdict in `[-1.0, 1.0]`: how much defensibility this citation confers
    /// for the connection it makes (spec §3.3), never a claim about what the source says.
    pub value: f64,
    /// The auditor's free-text rationale, if it gave one.
    pub reason: Option<String>,
    /// When the audit was emitted, stamped from the owning event's `occurred_at` (never `now()`).
    /// Load-bearing, not decoration: `resource_citation_quality` weights each audit by
    /// `pow(0.5, age/30d)`, so this is the row's weight in the number the shape read returned.
    pub created: DateTime<Utc>,
    /// The auditor's profile. **Not the citer** — the self-audit denial arm exists precisely so the
    /// two cannot be the same profile for a given citation.
    pub auditor_profile_id: ProfileId,
    /// The auditor's unique `kb_profiles.handle`.
    pub auditor_handle: String,
    /// The auditor's `kb_profiles.display_name`.
    pub auditor_display_name: String,
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

    /// The trail row names its auditor on the wire under `auditor_*` keys. Pins the KEY NAMES, not
    /// just the round-trip: a UI that renders "who audited this" reads them by name, and the whole
    /// point of the read is that two auditors are distinguishable — which they are not if the
    /// identity fields are absent or renamed.
    #[test]
    fn trail_row_names_its_auditor_on_the_wire() {
        let auditor = Uuid::now_v7();
        let row = CitationAuditRow {
            audit_id: Uuid::now_v7(),
            block_id: BlockId::from(Uuid::now_v7()),
            source_id: ResourceId::from(Uuid::now_v7()),
            value: -1.0,
            reason: Some("cited paragraph does not support the claim".to_string()),
            created: Utc::now(),
            auditor_profile_id: ProfileId::from(auditor),
            auditor_handle: "adversary".to_string(),
            auditor_display_name: "The Adversary".to_string(),
        };
        let v = serde_json::to_value(&row).unwrap();
        assert_eq!(v["auditor_profile_id"], auditor.to_string());
        assert_eq!(v["auditor_handle"], "adversary");
        assert_eq!(v["auditor_display_name"], "The Adversary");
        let back: CitationAuditRow = serde_json::from_value(v).unwrap();
        assert_eq!(back, row);
    }
}
