//! Wire types for the admin / system-settings surface (Chunk 6).
//!
//! `UpdateSettingsRequest` is a partial-update payload: every `Some` field
//! overwrites that `kb_system_settings` column, every `None` leaves it
//! unchanged (COALESCE on the server). `access_mode` is a raw string validated
//! server-side against `{open, invite_only}` — mirrors how `SystemSettings`
//! keeps `access_mode` as `String` rather than a sqlx-decoded enum.
//!
//! `PromoteAdminRequest` grants the target profile `owner` on a team. A `None`
//! `team_id` means "the configured gating team" (resolved server-side so the
//! gating slug never leaves the server). System-admin ≡ owner of the gating
//! team, so the default case mints a second system admin.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Partial-update body for `PATCH /api/access/admin/settings`.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "admin.ts"))]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct UpdateSettingsRequest {
    /// `"open"` or `"invite_only"`. Validated server-side.
    pub access_mode: Option<String>,
    /// Slug of the team that gates the instance in `invite_only` mode.
    pub gating_team_slug: Option<String>,
    /// Human-facing instance name.
    pub instance_name: Option<String>,
    /// Terms-of-service version label.
    pub terms_version: Option<String>,
    /// URI of the terms-of-service resource.
    pub terms_resource_uri: Option<String>,
}

impl UpdateSettingsRequest {
    /// True when no field is set — the caller wants a read, not a write.
    pub fn is_empty(&self) -> bool {
        self.access_mode.is_none()
            && self.gating_team_slug.is_none()
            && self.instance_name.is_none()
            && self.terms_version.is_none()
            && self.terms_resource_uri.is_none()
    }
}

/// Body for `POST /api/access/admin/promote`.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "admin.ts"))]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PromoteAdminRequest {
    /// Profile to promote (grant `owner` on the target team).
    pub profile_id: Uuid,
    /// Target team; `None` ⇒ the configured gating team (mints a system admin).
    pub team_id: Option<Uuid>,
}

/// Body for `POST /api/access/admin/demote`.
///
/// The governance twin of [`PromoteAdminRequest`]: it revokes the system-admin grant. Not
/// team-scoped — governance is keyed on the profile alone, so it carries no team.
///
/// Leaner derives than its sibling on purpose: this is an operator-only endpoint, excluded from the
/// OpenAPI contract (no `#[utoipa::path]`), fronted by no MCP tool, and consumed by no UI — so it
/// carries only the wire derives, not the `typescript`/`web-api`/`mcp` set that would generate
/// surface nothing consumes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DemoteAdminRequest {
    /// Profile to demote (revoke its system-admin governance grant).
    pub profile_id: Uuid,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn update_settings_is_empty_detects_no_fields() {
        assert!(UpdateSettingsRequest::default().is_empty());
        let one = UpdateSettingsRequest {
            instance_name: Some("Acme".to_owned()),
            ..Default::default()
        };
        assert!(!one.is_empty());
    }

    #[test]
    fn promote_request_roundtrips_through_json() {
        let req = PromoteAdminRequest {
            profile_id: Uuid::nil(),
            team_id: None,
        };
        let json = serde_json::to_string(&req).expect("serialize");
        let back: PromoteAdminRequest = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back.profile_id, req.profile_id);
        assert!(back.team_id.is_none());
    }
}

// ── re-embed trigger (operator-only) ──────────────────────────────────────────
//
// Deliberately NOT in the OpenAPI contract: the handler is mounted with a plain `.route()`, like the
// rest of `/api/*/admin/*`. It is an operator action, not part of the product surface.

/// Body for `POST /api/embed/admin/reembed`.
///
/// Exactly one scope: a single resource, a whole context, or everything. Three granularities because a
/// re-embed is a thing you try on **one**, then a **few**, then **all** — in that order. A trigger that
/// only offers "all" is one nobody dares pull.
///
/// Nothing is *marked* dirty. Staleness is derived — a chunk is stale when it has no vector, or when
/// its `embedded_with` is not the model the server embeds with — so this only ever enqueues work for
/// chunks that genuinely need it, and it is safe to re-run at any time.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ReembedRequest {
    /// Re-embed just this resource.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resource_id: Option<Uuid>,
    /// Re-embed every stale resource homed in this context.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_id: Option<Uuid>,
    /// Re-embed everything stale. Must be set explicitly — an empty body is a no-op, not "all".
    #[serde(default)]
    pub all: bool,
    /// Max resources to enqueue this call. Bounds blast radius: run it repeatedly to walk the index.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<i32>,
    /// Report what is stale without enqueuing anything. The safe first move.
    #[serde(default)]
    pub dry_run: bool,
}

/// Result of a re-embed trigger — and, on `dry_run`, just the survey.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReembedSummary {
    /// Resources in scope still holding stale chunks.
    pub stale_resources: u64,
    /// Stale chunks in scope. Divide by the drain's per-tick throughput to estimate the drain time.
    pub stale_chunks: u64,
    /// Resources actually enqueued by this call. Empty on `dry_run`, and empty for any resource that
    /// already had a live job (re-running never double-queues).
    pub enqueued: Vec<Uuid>,
}

// ---------------------------------------------------------------------------
// The admin ledger's read surface (admin-event-sink Task 6).
//
// These carry the same wire shape as `temper_substrate::payloads::{EventRef, RefRel, RefTarget,
// AnchorTable}`. **That is not accidental duplication awaiting a cleanup — do not "fix" it by
// merging them.**
//
// They are a different TYPE because they are a different THING. Substrate's vocabulary describes
// events in general: anything the system records, anchored anywhere. These describe ledger
// entries — admin acts, deliberately bounded by reach and by event type
// (`ADMIN_EVENT_TYPES`), firewalled from cognition by their NULL anchor, and readable only
// through a gate that dispatches per act family. An admin-ledger reference is not a general
// event reference that happens to look alike; it is a narrower claim with narrower rules.
//
// Keeping them distinct is a security posture as much as a modelling one. Types are not a hard
// boundary, but they express INTENT, and the compiler enforces the intent for free: a general
// event ref cannot be passed where a ledger ref is expected without someone writing a conversion
// and thereby saying so. Collapsing them would delete that declaration and let admin-ledger data
// flow into cognition paths — and vice versa — with nothing to notice.
//
// There is a practical constraint pointing the same way: temper-core is the dependency LEAF, and
// temper-client (which deserializes this) cannot take a temper-substrate dependency — substrate
// pulls `temper-ingest(embed)` non-optionally, so it would link ort/ONNX into the HTTP client.
// Relocating the substrate types instead would restale the `kb_event_types.payload_schema`
// fixtures (`grant_created.v1.schema.json` and friends) that the boot-seed stamps into the
// registry.
//
// What the split must NOT become is silent drift in the wire form itself, since both sides read
// and write the same `kb_events."references"` column. That is closed by test, not by hope:
// `temper-api/tests/admin_ledger_wire_parity_test.rs` pins every variant in both directions and
// stops compiling if either side gains one.
// ---------------------------------------------------------------------------

/// Why an event points at a thing. Mirrors `temper_substrate::payloads::RefRel`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum LedgerRefRel {
    #[serde(rename = "supersedes")]
    Supersedes,
    #[serde(rename = "derived_from")]
    DerivedFrom,
    #[serde(rename = "touches")]
    Touches,
    /// What the act was performed ON.
    #[serde(rename = "subject")]
    Subject,
    /// WHO the act was performed FOR.
    #[serde(rename = "principal")]
    Principal,
}

/// What an event points at. Mirrors `temper_substrate::payloads::AnchorTable`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum LedgerRefKind {
    #[serde(rename = "kb_contexts")]
    Contexts,
    #[serde(rename = "kb_cogmaps")]
    Cogmaps,
    #[serde(rename = "kb_resources")]
    Resources,
    #[serde(rename = "kb_edges")]
    Edges,
    #[serde(rename = "kb_content_blocks")]
    ContentBlocks,
    #[serde(rename = "kb_teams")]
    Teams,
    #[serde(rename = "kb_profiles")]
    Profiles,
    #[serde(rename = "kb_connections")]
    Connections,
    #[serde(rename = "kb_machine_clients")]
    MachineClients,
}

/// One typed pointer out of a ledger event. Mirrors `temper_substrate::payloads::RefTarget`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct LedgerRefTarget {
    pub kind: LedgerRefKind,
    pub id: Uuid,
}

/// Mirrors `temper_substrate::payloads::EventRef`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct LedgerRef {
    pub rel: LedgerRefRel,
    pub target: LedgerRefTarget,
}

/// One act on the admin ledger.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdminLedgerEntry {
    pub event_id: Uuid,
    pub event_type: String,
    pub actor_profile_id: Uuid,
    pub actor_handle: String,
    pub occurred_at: chrono::DateTime<chrono::Utc>,
    pub payload: serde_json::Value,
    pub references: Vec<LedgerRef>,
    pub correlation_id: Option<Uuid>,
}

/// A page of the ledger, always carrying the epoch.
///
/// The epoch is what stops an empty `entries` from lying. Admin history *begins* at the epoch —
/// acts before it genuinely happened, but no writer recorded them — so an empty list with an
/// epoch reads as "nothing since T", never "nothing ever". There is deliberately no standalone
/// epoch endpoint: `ledger_epoch` takes no caller and has no gate, so the only safe place to
/// surface it is inside a response the service has already authorized.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdminLedgerResponse {
    pub entries: Vec<AdminLedgerEntry>,
    pub epoch: Option<chrono::DateTime<chrono::Utc>>,
}

/// Query for `GET /api/admin/ledger` — **exactly one axis**, never both.
///
/// One type for both directions: temper-client serializes it into the query string, temper-api
/// deserializes it back out. A second copy on the server side is how a client learns to send a
/// parameter the server stopped reading.
///
/// The two axes answer different questions and gate differently — subject reads are gated per act
/// family against that subject, actor reads are self-gating — so the server refuses a request
/// naming both rather than picking one.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AdminLedgerQuery {
    /// Subject axis: `<kind>:<uuid>`, e.g. `kb_resources:0199c3f1-...`.
    ///
    /// Carried as ONE string, split only server-side. Splitting it into a kind half and an id half
    /// on the wire would put the grammar in every client — and a parser written twice is two
    /// grammars. `admin_ledger_service::parse_subject_spec` is the only place this is understood.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subject: Option<String>,
    /// Actor axis: whose acts to read. Reading your own is always allowed; reading another's is
    /// an audit, and audits are admin-only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actor: Option<Uuid>,
    /// Page size. Clamped server-side rather than rejected.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub offset: Option<i64>,
}

/// MCP input for the `admin_ledger` tool.
///
/// Mirrors the CLI's two flags rather than [`AdminLedgerQuery`]'s split subject fields: an agent
/// passes `subject: "kb_resources:<uuid>"` as one string, exactly as a human types it. The
/// server-side split is the same code path either way.
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AdminLedgerInput {
    /// Subject axis: `<kind>:<uuid>`, e.g. `kb_resources:0199c3f1-...`. Asks what was done TO a
    /// thing. Mutually exclusive with `actor`.
    pub subject: Option<String>,
    /// Actor axis: a profile UUID. Asks what a principal DID. Your own acts are always readable;
    /// reading another's is an audit and requires admin.
    pub actor: Option<String>,
    /// Page size. Clamped server-side to 200.
    pub limit: Option<i64>,
    pub offset: Option<i64>,
}
