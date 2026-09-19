//! Wire types for the admin / system-settings surface (Chunk 6).
//!
//! `UpdateSettingsRequest` is a partial-update payload: every `Some` field
//! overwrites that `kb_system_settings` column, every `None` leaves it
//! unchanged (COALESCE on the server). `access_mode` is retired as a control
//! (spec §14 / D18): standing now answers per-principal what a global mode
//! switch used to answer instance-wide, so the settings surface no longer
//! accepts it. Phase 2 has since dropped the column.
//!
//! `PromoteAdminRequest` mints a system admin by writing a principal-governance
//! grant and, if needed, approved standing. It also grants the target `owner`
//! on a team; a `None` `team_id` means "the configured gating team" (resolved
//! server-side so the gating slug never leaves the server). That team row is a
//! side effect — ownership of it confers nothing on its own.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Partial-update body for `PATCH /api/access/admin/settings`.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "admin.ts"))]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct UpdateSettingsRequest {
    /// Gating team slug recorded in instance settings. Ownership of it confers no authorization:
    /// `is_system_admin` reads the principal-governance grant. `None` leaves it unchanged.
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
        self.gating_team_slug.is_none()
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
    /// Target team for the side-effect `owner` row; `None` ⇒ the configured gating team. The
    /// governance grant is what mints the admin, not this membership.
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

    /// The card's invitation rows are field-pinned to exclude the redemption `token` (spec §6
    /// rule 1). Pinning it HERE at the type level means a future `token` field on any card
    /// component fails this test at the wire, not at a review.
    #[test]
    fn profile_card_serializes_no_token_anywhere() {
        let card = AdminProfileCard {
            profile_id: Uuid::now_v7(),
            handle: "alice".to_owned(),
            display_name: "Alice".to_owned(),
            standing: "denied".to_owned(),
            standing_updated: None,
            is_system_admin: false,
            auth_links: vec![AdminProfileAuthLink {
                auth_provider: "saml:okta".to_owned(),
                email: Some("alice@corp.example".to_owned()),
                email_verified: true,
                is_default: true,
                linked_at: chrono::Utc::now(),
            }],
            email: Some("alice@corp.example".to_owned()),
            provisioned_via: Some("saml:okta".to_owned()),
            teams: vec![AdminProfileTeamMembership {
                team_slug: "platform".to_owned(),
                role: "member".to_owned(),
            }],
            pending_invitations: vec![AdminProfileInvitation {
                id: Uuid::now_v7(),
                team_slug: "platform".to_owned(),
                role: "member".to_owned(),
                invited_by_profile_id: Uuid::now_v7(),
                created: chrono::Utc::now(),
                expires_at: chrono::Utc::now(),
            }],
            open_join_request: None,
            open_reconsideration: None,
            hints: vec!["temper admin access approve <uuid>".to_owned()],
        };
        let json = serde_json::to_string(&card).expect("serialize");
        assert!(
            !json.contains("token"),
            "the state card must never serialize a token-bearing field"
        );
        let back: AdminProfileCard = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back.handle, "alice");
        assert_eq!(back.pending_invitations.len(), 1);
    }

    #[test]
    fn matched_email_is_absent_from_json_when_unset() {
        let entry = AdminDirectoryEntry {
            profile_id: Uuid::now_v7(),
            handle: "bob".to_owned(),
            display_name: "Bob".to_owned(),
            standing: "denied".to_owned(),
            standing_updated: None,
            is_system_admin: false,
            email: None,
            provisioned_via: None,
            team_count: 0,
            has_pending_request: false,
            matched_email: None,
        };
        let json = serde_json::to_string(&entry).expect("serialize");
        assert!(!json.contains("matched_email"));
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
    /// The erasure act's opaque request reference. Mirrors
    /// `temper_substrate::payloads::RefRel::Request`.
    #[serde(rename = "request")]
    Request,
}

/// What an event points at. Mirrors `temper_substrate::payloads::AnchorTable`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum LedgerRefKind {
    #[serde(rename = "kb_contexts")]
    Contexts,
    #[serde(rename = "kb_cogmaps")]
    Cogmaps,
    /// A binary blob — an edge ENDPOINT (D3): a `relationship_asserted` event whose
    /// source/target is a blob points here, and the mirror-mapping test makes the two
    /// vocabularies grow together (a value one side can decode and the other cannot fails
    /// the whole page at compile time instead).
    #[serde(rename = "kb_blobs")]
    Blobs,
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
    /// An event — `derived_from`'s target. Mirrored here because both sides decode the SAME
    /// column: an admin event carrying a `kb_events` target that this enum could not decode would
    /// make `to_wire_page` fail the WHOLE page, not the one reference.
    #[serde(rename = "kb_events")]
    Events,
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

// ── the operator directory (admin-operator-directory spec §5/§6) ─────────────
//
// Read-only over existing tables: no new state, no write path, GET-only routes. The directory
// inventories HUMAN principals only — machines are inventoried by `admin machine list` and
// connection profiles carry no standing row at all. Every response shape below is a projection;
// none carries a capability, and the card's invitation rows are field-pinned to exclude the
// redemption `token` (a leaked token converts this read into a mutation capability).

/// Query parameters for `GET /api/access/admin/profiles` — one type, both directions:
/// temper-client (PR-2) serializes it into the query string, temper-api deserializes it back out.
///
/// `email` and `email_contains` answer different questions (exact identity resolution vs
/// display-adjacent search) with different response shapes (a state card vs a page), so they are
/// separate parameters with separate semantics — never aliases. The server refuses a request
/// naming both rather than picking one.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AdminProfilesListQuery {
    /// Filter by admission state: `denied|requested|approved|revoked|deactivated|needs-access|all`.
    /// Default `needs-access` — every non-approved state INCLUDING no standing row (the
    /// operator's work queue). Case-insensitive.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub standing: Option<String>,
    /// Case-insensitive LITERAL substring matched over the profile's verified auth-link emails.
    /// `%`, `_` and `\` have no wildcard meaning here. When set, each row carries `matched_email`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub email_contains: Option<String>,
    /// EXACT, case-insensitive match over verified auth-link emails. When present the route
    /// answers with the single matching profile's state card (not a page): the one place a
    /// human-controlled address becomes a target UUID. Zero matches → 404; more than one profile
    /// verified-owns the address → 404 whose body names the collision. Mutually exclusive with
    /// `email_contains`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
    /// Restrict to members of this team, by slug or UUID.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub team: Option<String>,
    /// Page size. Clamped server-side: default 50, capped at 200.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<i64>,
    /// Page offset. Floor 0, capped at 10000 (depth protection).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub offset: Option<i64>,
}

/// One row of the directory list — minimal on purpose; the state card is the deep read.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "admin.ts"))]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdminDirectoryEntry {
    pub profile_id: Uuid,
    pub handle: String,
    pub display_name: String,
    /// Admission state as rendered: `denied|requested|approved|revoked|deactivated`. A profile
    /// with no standing row renders `denied` (absence denies) rather than as an absent field.
    pub standing: String,
    /// When the standing row was last written; `None` exactly when no row exists.
    pub standing_updated: Option<chrono::DateTime<chrono::Utc>>,
    /// Reads the principal-governance grant and nothing else.
    pub is_system_admin: bool,
    /// The profile's default verified email (default link if verified, else earliest-verified),
    /// `None` when no verified link exists. An unverified email never renders here.
    pub email: Option<String>,
    /// Auth-link provider of the link [`Self::email`] came from (or of the earliest link when no
    /// verified link exists) — the "provisioned via Google / saml:idp-key" answer.
    pub provisioned_via: Option<String>,
    pub team_count: i64,
    /// The profile holds an open (`pending`) join request — the direct bridge to
    /// `admin requests review <id>`.
    pub has_pending_request: bool,
    /// Present only when the `email_contains` filter drove the match: the address that
    /// satisfied it, so "why was this person enumerated" is answerable from the row itself.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub matched_email: Option<String>,
}

/// A page of the directory. Carries `total` — the count of EVERY profile matching the filter,
/// not just this page — so a paging agent is never silently incomplete.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "admin.ts"))]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdminDirectoryListResponse {
    pub entries: Vec<AdminDirectoryEntry>,
    /// Total profiles matching the filter across all pages.
    pub total: i64,
}

/// One identity link on the card — the SAML-provenance answer. Rendered verbatim from
/// `kb_profile_auth_links`; `email_verified` is what "verified" means throughout the directory.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "admin.ts"))]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdminProfileAuthLink {
    pub auth_provider: String,
    /// The linked address, `None` for machine-style links (which a human card will not carry,
    /// but the projection stays total over the table).
    pub email: Option<String>,
    pub email_verified: bool,
    pub is_default: bool,
    pub linked_at: chrono::DateTime<chrono::Utc>,
}

/// A team membership on the card — including memberships held while denied, which is legal
/// today and confers nothing until Approve.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "admin.ts"))]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdminProfileTeamMembership {
    pub team_slug: String,
    pub role: String,
}

/// A pending team invitation attributed to the profile. FIELD-PINNED: there is deliberately no
/// `token` field — the token accepts the invitation, and leaking it here would hand every reader
/// of a terminal scrollback or agent transcript a mutation capability. An invitation whose
/// target email is verified-owned by two or more profiles is attributed to NEITHER card.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "admin.ts"))]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdminProfileInvitation {
    pub id: Uuid,
    pub team_slug: String,
    pub role: String,
    pub invited_by_profile_id: Uuid,
    pub created: chrono::DateTime<chrono::Utc>,
    pub expires_at: chrono::DateTime<chrono::Utc>,
}

/// The profile's open join request, if any — the direct bridge to
/// `admin requests review <id>`. Closed/rejected history is ledger territory and stays off v1.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "admin.ts"))]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdminOpenJoinRequest {
    pub id: Uuid,
    pub created: chrono::DateTime<chrono::Utc>,
    pub message: Option<String>,
}

/// The profile's open reconsideration request, if any — the bridge to
/// `admin reviews close <id>`.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "admin.ts"))]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdminOpenReviewRequest {
    pub id: Uuid,
    pub created: chrono::DateTime<chrono::Utc>,
}

/// The principal state card — one response composed from existing tables, the deep read behind
/// `GET /api/access/admin/profiles/{profile_id}` and `?email=`. Hints name EXISTING commands;
/// they are reads that print, not new mutation doors.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "admin.ts"))]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdminProfileCard {
    // identity
    pub profile_id: Uuid,
    pub handle: String,
    pub display_name: String,
    // admission — absence of a standing row renders `denied`
    pub standing: String,
    pub standing_updated: Option<chrono::DateTime<chrono::Utc>>,
    // governance
    pub is_system_admin: bool,
    // identity links — full list plus the §6 default-link fallback pair
    pub auth_links: Vec<AdminProfileAuthLink>,
    /// The default verified email (default link if it exists and is verified, else the
    /// earliest-verified link), `None` when no verified link exists.
    pub email: Option<String>,
    /// Provider of the link [`Self::email`] came from, else of the earliest link overall.
    pub provisioned_via: Option<String>,
    // memberships (including held-while-denied)
    pub teams: Vec<AdminProfileTeamMembership>,
    // pending invitations addressed to any of the profile's verified emails — token-free
    pub pending_invitations: Vec<AdminProfileInvitation>,
    // queue state — open items only
    pub open_join_request: Option<AdminOpenJoinRequest>,
    pub open_reconsideration: Option<AdminOpenReviewRequest>,
    /// The existing enablement commands, as copy-runnable text.
    pub hints: Vec<String>,
}

/// MCP input for the `admin_profiles_list` tool (PR-3). Mirrors [`AdminProfilesListQuery`]'s
/// filters with `standing` as a plain string; the server validates it either way.
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AdminProfilesListInput {
    /// `denied|requested|approved|revoked|deactivated|needs-access|all`. Default `needs-access`.
    pub standing: Option<String>,
    /// Literal case-insensitive substring over verified emails.
    pub email_contains: Option<String>,
    /// Members of this team, by slug or UUID.
    pub team: Option<String>,
    /// Page size. Clamped server-side: default 50, capped at 200.
    pub limit: Option<i64>,
    pub offset: Option<i64>,
}

/// MCP input for the `admin_profiles_show` tool (PR-3). Exactly one of the two.
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AdminProfilesShowInput {
    /// The profile to show, by UUID.
    pub profile_id: Option<Uuid>,
    /// The profile to show, by exact verified email. Ambiguous addresses are refused, not resolved.
    pub email: Option<String>,
}
