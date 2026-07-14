//! Resource API types — shared between temper-api and temper-client.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use uuid::Uuid;

use super::managed_meta::ManagedMeta;
use temper_core::types::ids::{ContextId, ProfileId, ResourceId};

/// Row type for resource listings — includes joined display fields
/// and managed_meta projections from `vault_resources_browse` view.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "resource.ts"))]
#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
pub struct ResourceRow {
    pub id: ResourceId,
    /// Home context — `Some` for a context-homed resource, `None` when the
    /// resource is homed in a cognitive map (Surface B). Mutually exclusive
    /// with the `cogmap_*` fields below.
    pub kb_context_id: Option<ContextId>,
    pub origin_uri: String,
    pub title: String,
    pub originator_profile_id: ProfileId,
    pub owner_profile_id: ProfileId,
    pub is_active: bool,
    pub created: DateTime<Utc>,
    pub updated: DateTime<Utc>,
    // Joined display fields — `context_*` present for a context home,
    // `cogmap_*` for a cogmap home.
    pub context_name: Option<String>,
    pub doc_type_name: String,
    pub owner_handle: String,
    /// Slug of the home context (the natural-key half of `@owner/slug`).
    /// `None` for a cogmap-homed resource.
    pub context_slug: Option<String>,
    /// Already-sigil'd owner: `@<handle>` for profiles, `+<team-slug>` for teams.
    /// Together with `context_slug`, forms the full decorated context ref `{context_owner_ref}/{context_slug}`.
    /// `None` for a cogmap-homed resource.
    pub context_owner_ref: Option<String>,
    /// Set when the resource is homed in a cognitive map (Surface B).
    /// Mutually exclusive with the `context_*` fields.
    pub cogmap_id: Option<Uuid>,
    /// Display name of the home cognitive map. `Some` iff `cogmap_id` is `Some`.
    pub cogmap_name: Option<String>,
    // Managed meta projections
    pub stage: Option<String>,
    #[cfg_attr(feature = "typescript", ts(type = "number | null"))]
    pub seq: Option<i64>,
    pub mode: Option<String>,
    pub effort: Option<String>,
    /// SHA-256 hash of the resource body content, from `kb_resource_manifests`.
    /// `None` when no manifest row exists (resource created via POST without a
    /// body trio, or the manifest join returned NULL).
    pub body_hash: Option<String>,
    /// Is the whole body here? A projection of the ingest lifecycle held in `kb_events` — written only
    /// by the `resource_created` / `resource_finalized` projectors, never mutated directly. `complete`
    /// for every ordinary (atomic) create; `in_progress` for a segmented ingest that has begun but not
    /// yet been finalized (remaining blocks not landed), which is **excluded from list and search** and
    /// readable only by `show`.
    ///
    /// Orthogonal to `embedding_status` (`pending`/`ready`), which asks a different question: *are the
    /// vectors ready?* This one asks *are the bytes all here?*
    ///
    /// `Option` purely for **version skew** — the column is `NOT NULL` server-side, so a current server
    /// always sends it; `None` means the server predates W2 PR 1. Do not read `None` as "incomplete".
    pub ingest_state: Option<IngestState>,
}

/// A resource's ingest-completion state — a **projection** of the append-only `kb_events` ledger
/// (`resource_created` → `block_created`… → `resource_finalized`), not an independently-mutated flag.
/// The ledger is the state machine; this is its materialized current-state view, kept as a column so
/// list/search can filter it with a cheap read instead of scanning events.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "resource.ts"))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum IngestState {
    /// A segmented ingest has begun but not been finalized — the body is incomplete. Hidden from
    /// list/search, still resumable and readable via `show`.
    InProgress,
    /// The whole body is present: every atomic create, and every finalized segmented ingest.
    Complete,
}

impl IngestState {
    /// The canonical wire/DB string (matches the `ck_kb_resources_ingest_state` CHECK values).
    pub fn as_str(self) -> &'static str {
        match self {
            IngestState::InProgress => "in_progress",
            IngestState::Complete => "complete",
        }
    }

    /// Parse the DB/wire string. The `ck_kb_resources_ingest_state` CHECK constrains the column to
    /// these two values, so an unrecognized string is a schema/version violation, not ordinary input —
    /// returned as `None` for the caller to handle rather than silently coerced.
    pub fn from_wire(s: &str) -> Option<Self> {
        match s {
            "in_progress" => Some(IngestState::InProgress),
            "complete" => Some(IngestState::Complete),
            _ => None,
        }
    }
}

impl ResourceRow {
    /// The display name of this resource's home — its context name, or its cognitive-map
    /// name when cogmap-homed. `None` only if neither is set (should not occur). The single
    /// accessor for the `context_* | cogmap_*` mutual exclusion: surfaces apply their own
    /// placeholder for the `None` case rather than re-deriving the fallback chain.
    pub fn home_display(&self) -> Option<&str> {
        self.context_name.as_deref().or(self.cogmap_name.as_deref())
    }
}

/// The single-resource read projection: a [`ResourceRow`] plus both metadata tiers.
///
/// `show` used to return a bare `ResourceRow`, which carries only the flat managed
/// projections (`stage`/`seq`/`mode`/`effort`) — so the "full" view silently omitted
/// both `managed_meta` and `open_meta`, and a script reading `open_meta` from it got
/// `None`. `list` keeps returning `ResourceRow`, so a 200-row listing pays nothing for
/// the tiers.
///
/// The two meta fields carry serde attributes identical to
/// [`super::managed_meta::ResourceMetaResponse`]'s, so the cheap `--meta-only`
/// projection is a literal strict subset of this shape.
///
/// No `ts_rs::TS` derive: ts-rs cannot codegen a `#[serde(flatten)]` field (see the
/// `act` field on `ResourceUpdateRequest`, `ts(skip)`-ped for the same reason). The
/// SvelteKit UI keeps typing `GET /api/resources/{id}` as `ResourceRow` — this shape is
/// a structural superset of it, so the extra keys are simply ignored there.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
pub struct ResourceDetail {
    #[serde(flatten)]
    pub row: ResourceRow,
    /// Typed managed (`temper-*`) frontmatter. `None` only if the manifest row predates
    /// meta population.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub managed_meta: Option<ManagedMeta>,
    /// Open (user-defined) frontmatter — the free-form tier, intentionally untyped.
    /// `None` only if the manifest row predates meta population.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub open_meta: Option<serde_json::Value>,
}

/// Sort field for resource listing.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "resource.ts"))]
#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum ResourceSortField {
    #[default]
    Updated,
    Created,
    Title,
    Stage,
    Seq,
    ContextName,
    DocTypeName,
}

/// Sort direction.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "resource.ts"))]
#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum SortOrder {
    #[default]
    Desc,
    Asc,
}

/// Query parameters for listing visible resources.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "resource.ts"))]
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[cfg_attr(feature = "web-api", derive(utoipa::IntoParams))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
pub struct ResourceListParams {
    pub kb_doc_type_id: Option<Uuid>,
    /// Context filter: UUID string or `@owner/slug` decorated ref.
    /// Bare context names are rejected server-side (spec Decision 1).
    pub context_ref: Option<String>,
    pub doc_type_name: Option<String>,
    pub owner: Option<String>,
    pub q: Option<String>,
    pub stage: Option<String>,
    /// Goal filter (task only): the resolved goal `ResourceId` (as UUID). Returns
    /// only resources joined to this goal via a live `advances`→goal edge. The CLI/MCP
    /// resolve the caller's `--goal <ref>` to this UUID (trailing-UUID-only) before the
    /// query. `None` = no goal filter.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub goal: Option<Uuid>,
    pub sort: Option<ResourceSortField>,
    pub order: Option<SortOrder>,
    #[cfg_attr(feature = "typescript", ts(type = "number | null"))]
    pub limit: Option<i64>,
    #[cfg_attr(feature = "typescript", ts(type = "number | null"))]
    pub offset: Option<i64>,
    /// When true, the list endpoint returns `ResourceMetaListResponse`
    /// (`Vec<ResourceDetail>` rows — full row + both meta tiers) instead of
    /// `ResourceListResponse` (`Vec<ResourceRow>` rows). Default: false.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub meta_only: Option<bool>,
}

/// Aggregated doc-type facet counts for the current filter set.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "resource.ts"))]
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
pub struct ResourceFacets {
    pub doc_type: std::collections::HashMap<String, i64>,
}

/// Paginated response for resource list endpoints, with doc-type facets.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "resource.ts"))]
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
pub struct ResourceListResponse {
    pub rows: Vec<ResourceRow>,
    pub total: i64,
    pub facets: ResourceFacets,
}

/// Request body for creating a resource.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
pub struct ResourceCreateRequest {
    /// Context addressed by UUID. The API wraps this as `ContextRef::Id` and resolves it through
    /// `resolve_context_ref` (visibility-gated). Stays UUID-keyed intentionally (spec §7 decision):
    /// the create path is server-to-server / MCP and always has the UUID at hand; a `context_ref`
    /// string here would add blast-radius with no practical benefit (deferred).
    pub kb_context_id: Uuid,
    /// Doc-type name (the substrate stores doc-type as a property name; the
    /// backend create path passes it straight through to `CreateResource`).
    pub doc_type: String,
    pub origin_uri: String,
    pub title: String,
    /// Per-act correlation (`invocation_id`) + discrete agent authorship for the create act.
    /// Flattened as top-level keys; all optional (empty when nothing is supplied).
    #[serde(default, flatten)]
    pub act: temper_core::types::authorship::ActInput,
}

/// Request body for annotating a resource's block with provenance sources (issue #355) —
/// `POST /api/resources/{id}/provenance`. The annotate-only write: attach sources WITHOUT a body
/// revise (no re-chunk/re-embed). Carries no content — that is the whole point.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "resource.ts"))]
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
pub struct ResourceAnnotateRequest {
    /// Sources to attach to the addressed block — resource refs, external URLs, or event ids
    /// (`{kind,value}`-tagged). Position → accretion `seq`. Must be non-empty.
    ///
    /// `ts(skip)`-ped for the same reason as `ResourceUpdateRequest.sources`: `ProvenanceSource` is a
    /// tagged enum the SvelteKit UI never sends (this is a CLI/agent write path).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[cfg_attr(feature = "typescript", ts(skip))]
    pub sources: Vec<temper_core::types::provenance::ProvenanceSource>,
    /// Which content block to annotate. `None` → the resource's sole non-folded body block; `Some(id)`
    /// addresses that block explicitly (must belong to the resource and be non-folded).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "typescript", ts(skip))]
    pub content_block: Option<uuid::Uuid>,
    /// Per-act correlation (`invocation_id`) + discrete agent authorship for the annotate act.
    /// Flattened as top-level keys; all optional.
    ///
    /// `ts(skip)`-ped: ts-rs cannot codegen a `#[serde(flatten)]` field, and the UI never sends
    /// authorship (precedent: `ResourceUpdateRequest.act`).
    #[serde(default, flatten)]
    #[cfg_attr(feature = "typescript", ts(skip))]
    pub act: temper_core::types::authorship::ActInput,
}

/// Request body for updating a resource.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "resource.ts"))]
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
pub struct ResourceUpdateRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// Partial managed_meta — only fields with `Some` apply.
    /// Untouched fields preserve their stored value. There is no in-band
    /// signal for "clear this field"; field-clearing is reserved for a
    /// future PUT endpoint.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub managed_meta: Option<ManagedMeta>,
    /// Partial open_meta — incoming keys win; absent keys preserved.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub open_meta: Option<serde_json::Value>,
    /// New body markdown. Required iff `content_hash` and `chunks_packed`
    /// are also `Some` (all-or-nothing trio).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    /// SHA-256 hash of `content`. Required iff `content` is `Some`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_hash: Option<String>,
    /// Pre-computed chunks (base64-encoded MessagePack). Required iff
    /// `content` is `Some`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chunks_packed: Option<String>,
    /// Context move ref: a bare UUID or `@owner/slug` decorated form.
    /// Bare names (no `@`/`+` prefix, not a UUID) are rejected 400 by the
    /// server (Decision 1). When present the server resolves the ref to a
    /// `ContextId` (visibility-gated to the principal) and re-homes the
    /// resource. Forwarded verbatim from the CLI `--context-to` flag.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_to: Option<String>,
    /// Type-move: convert the resource to a new doc-type. Forwarded from the CLI
    /// `--type-to` flag; the server rewrites the authoritative `doc_type` via
    /// `MoveSpec.type_to`. First-class since Phase 2 (type is no longer carried
    /// as a `temper-type` key inside `managed_meta`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub type_to: Option<String>,
    /// Goal-set: the resolved goal `ResourceId` (as UUID) to link this resource to.
    /// When present the server folds any existing `advances`→goal edge and asserts a
    /// new one. Mutually exclusive with `clear_goal` (the CLI rejects both together).
    /// Forwarded from the CLI `--goal <ref>` flag (resolved client-side).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub goal: Option<Uuid>,
    /// Goal-clear: when `Some(true)`, the server folds the resource's current
    /// `advances`→goal edge, leaving it goal-less. The tri-state complement to `goal`
    /// (absent = untouched, `goal` = set/replace, `clear_goal` = retract). Forwarded
    /// from the CLI `--clear-goal` flag.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub clear_goal: Option<bool>,
    /// Block-provenance sources this body was distilled from — recorded against the
    /// resource's body block, position → accretion `seq`. Resource refs only in T7b;
    /// URL/`remote` sources are T7c.
    ///
    /// `ts(skip)`-ped: `ProvenanceSource` is a `{kind,value}`-tagged enum with no ts-rs
    /// `export_to`, and the SvelteKit UI never sends provenance (this is a CLI/agent
    /// write path, exactly like `act` below). Skipping keeps the generated TS honest —
    /// the UI cannot set provenance — and avoids emitting a dangling `ProvenanceSource`
    /// import. (Precedent: the `act` field below, likewise `ts(skip)`-ped.)
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[cfg_attr(feature = "typescript", ts(skip))]
    pub sources: Vec<temper_core::types::provenance::ProvenanceSource>,
    /// Which content block the body revise + `sources` target. `None` → the resource's sole
    /// non-folded body block (today's default); `Some(id)` addresses that block explicitly (must
    /// belong to the resource and be non-folded). Also the escape hatch for a multi-block resource.
    ///
    /// `ts(skip)`-ped for the same reason as `sources`/`act`: per-block addressing is a CLI/agent
    /// write path the SvelteKit UI never exercises, so the generated TS omits it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "typescript", ts(skip))]
    pub content_block: Option<uuid::Uuid>,
    /// Per-act correlation (`invocation_id`) + discrete agent authorship for the update act.
    /// Flattened as top-level keys; all optional (empty when nothing is supplied).
    ///
    /// `ts(skip)`-ped: ts-rs cannot codegen a `#[serde(flatten)]` field, and the SvelteKit UI
    /// never sends authorship — so the generated TypeScript type omits it (precedent: the
    /// `sources`/`act` fields on `ResourceCreateRequest`, likewise `ts(skip)`-ped).
    #[serde(default, flatten)]
    #[cfg_attr(feature = "typescript", ts(skip))]
    pub act: temper_core::types::authorship::ActInput,
}

/// Chunk used to reconstitute markdown content.
#[derive(Debug, Clone, Serialize, FromRow)]
pub struct ContentChunk {
    pub chunk_index: i32,
    pub header_path: String,
    pub heading_depth: i16,
    pub content: String,
}

/// Response body for resource content.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "resource.ts"))]
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
pub struct ContentResponse {
    pub resource_id: ResourceId,
    pub markdown: String,
    /// Typed server-side managed_meta from kb_resource_manifests — the closed
    /// Property vocabulary. Only the named `temper-*` keys are represented;
    /// there is no catch-all. Used by CLI sync pull to reconstruct complete
    /// frontmatter (identity/type/home come from the resource row, not here).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub managed_meta: Option<ManagedMeta>,
    /// Server-side open_meta from kb_resource_manifests. Intentionally
    /// untyped — open_meta is the free-form tier. Typed extraction of
    /// relationship fields lives in `ResourceRelationships` (see
    /// `types::graph`), which parses this value on demand and ignores
    /// anything it doesn't recognize.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub open_meta: Option<serde_json::Value>,
}

/// Response body for resource deletion.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
pub struct DeleteResponse {
    pub deleted: bool,
}

#[cfg(test)]
mod resource_detail_tests {
    use super::*;

    fn sample_resource_row() -> ResourceRow {
        ResourceRow {
            id: ResourceId::from(Uuid::nil()),
            kb_context_id: None,
            origin_uri: String::new(),
            title: "A Node".to_string(),
            originator_profile_id: ProfileId::from(Uuid::nil()),
            owner_profile_id: ProfileId::from(Uuid::nil()),
            is_active: true,
            created: DateTime::<Utc>::from_timestamp(0, 0).expect("epoch"),
            updated: DateTime::<Utc>::from_timestamp(0, 0).expect("epoch"),
            context_name: None,
            doc_type_name: "concept".to_string(),
            owner_handle: "someone".to_string(),
            context_slug: None,
            context_owner_ref: None,
            cogmap_id: None,
            cogmap_name: None,
            stage: None,
            seq: None,
            mode: None,
            effort: None,
            body_hash: None,
            ingest_state: Some(IngestState::Complete),
        }
    }

    #[test]
    fn resource_detail_flattens_row_and_carries_both_meta_tiers() {
        let detail = ResourceDetail {
            row: sample_resource_row(),
            managed_meta: Some(ManagedMeta {
                mode: Some("build".to_string()),
                ..ManagedMeta::default()
            }),
            open_meta: Some(serde_json::json!({ "custom": "value" })),
        };

        let v = serde_json::to_value(&detail).expect("serialize");

        // ResourceRow's fields are flattened to the top level, not nested under `row`.
        assert!(v.get("row").is_none(), "row must be flattened: {v}");
        assert!(v.get("id").is_some(), "flattened id: {v}");
        assert_eq!(v["title"], "A Node");
        assert_eq!(v["managed_meta"]["temper-mode"], "build");
        assert_eq!(v["open_meta"]["custom"], "value");
    }

    #[test]
    fn resource_detail_omits_absent_meta_tiers() {
        let detail = ResourceDetail {
            row: sample_resource_row(),
            managed_meta: None,
            open_meta: None,
        };
        let v = serde_json::to_value(&detail).expect("serialize");
        assert!(v.get("managed_meta").is_none(), "{v}");
        assert!(v.get("open_meta").is_none(), "{v}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `ContentResponse` with both managed_meta and open_meta populated
    /// must roundtrip through serde cleanly, preserving both tiers.
    #[test]
    fn content_response_roundtrips_with_both_meta_tiers() {
        let resource_id = ResourceId::from(Uuid::nil());
        let managed_meta = ManagedMeta {
            stage: Some("draft".to_string()),
            ..Default::default()
        };
        let open_meta = serde_json::json!({
            "tags": ["one", "two"],
            "related": ["res_999"],
        });
        let original = ContentResponse {
            resource_id,
            markdown: "# Hello".to_string(),
            managed_meta: Some(managed_meta.clone()),
            open_meta: Some(open_meta.clone()),
        };

        let json = serde_json::to_value(&original).expect("serialize");
        let roundtrip: ContentResponse = serde_json::from_value(json.clone()).expect("deserialize");

        assert_eq!(roundtrip.markdown, "# Hello");
        assert_eq!(roundtrip.managed_meta, Some(managed_meta));
        assert_eq!(roundtrip.open_meta, Some(open_meta));
    }

    /// `ContentResponse` with `open_meta: None` must omit the field
    /// entirely from the serialized JSON (not emit `"open_meta": null`),
    /// matching the `skip_serializing_if = "Option::is_none"` contract
    /// used by `managed_meta`. This preserves wire compatibility with
    /// older clients that don't know about `open_meta`.
    #[test]
    fn content_response_omits_open_meta_when_none() {
        let resource_id = ResourceId::from(Uuid::nil());
        let original = ContentResponse {
            resource_id,
            markdown: "body".to_string(),
            managed_meta: None,
            open_meta: None,
        };

        let json = serde_json::to_value(&original).expect("serialize");
        let obj = json.as_object().expect("object");

        assert!(
            !obj.contains_key("open_meta"),
            "open_meta should be omitted when None, got: {json}"
        );
        assert!(
            !obj.contains_key("managed_meta"),
            "managed_meta should be omitted when None, got: {json}"
        );
    }

    /// Old clients (no `open_meta` field in their request) must still
    /// deserialize a `ContentResponse` from servers that omit it.
    /// This is the `#[serde(default)]` contract.
    #[test]
    fn content_response_deserializes_without_open_meta() {
        let json = serde_json::json!({
            "resource_id": Uuid::nil(),
            "markdown": "hi",
        });

        let parsed: ContentResponse = serde_json::from_value(json).expect("deserialize");
        assert_eq!(parsed.markdown, "hi");
        assert!(parsed.managed_meta.is_none());
        assert!(parsed.open_meta.is_none());
    }

    #[test]
    fn resource_update_request_serde_round_trips_with_all_fields() {
        use serde_json::json;
        let req = ResourceUpdateRequest {
            title: Some("New Title".to_string()),
            managed_meta: Some(ManagedMeta {
                stage: Some("done".to_string()),
                ..Default::default()
            }),
            open_meta: Some(json!({"tags": ["rust"]})),
            content: Some("# Body\n".to_string()),
            content_hash: Some("sha256:abc".to_string()),
            chunks_packed: Some("base64-blob".to_string()),
            context_to: Some("@me/knowledge".to_string()),
            type_to: Some("goal".to_string()),
            goal: None,
            clear_goal: None,
            act: Default::default(),
            sources: Vec::new(),
            content_block: Some(uuid::Uuid::nil()),
        };
        let serialized = serde_json::to_string(&req).unwrap();
        let parsed: ResourceUpdateRequest = serde_json::from_str(&serialized).unwrap();
        assert_eq!(parsed.title.as_deref(), Some("New Title"));
        assert_eq!(parsed.type_to.as_deref(), Some("goal"));
        assert_eq!(parsed.content_block, Some(uuid::Uuid::nil()));
        assert_eq!(
            parsed
                .managed_meta
                .as_ref()
                .and_then(|m| m.stage.as_deref()),
            Some("done")
        );
        assert_eq!(parsed.open_meta, Some(json!({"tags": ["rust"]})));
        assert_eq!(parsed.content.as_deref(), Some("# Body\n"));
        assert_eq!(parsed.content_hash.as_deref(), Some("sha256:abc"));
        assert_eq!(parsed.chunks_packed.as_deref(), Some("base64-blob"));
        assert_eq!(parsed.context_to.as_deref(), Some("@me/knowledge"));
    }

    #[test]
    fn resource_update_request_omits_none_fields_on_serialize() {
        let req = ResourceUpdateRequest {
            title: None,
            managed_meta: Some(ManagedMeta {
                stage: Some("done".to_string()),
                ..Default::default()
            }),
            open_meta: None,
            content: None,
            content_hash: None,
            chunks_packed: None,
            context_to: None,
            type_to: None,
            goal: None,
            clear_goal: None,
            act: Default::default(),
            sources: Vec::new(),
            content_block: None,
        };
        let serialized = serde_json::to_string(&req).unwrap();
        assert!(!serialized.contains("\"title\""));
        assert!(!serialized.contains("\"content\""));
        assert!(!serialized.contains("\"context_to\""));
        assert!(!serialized.contains("\"content_block\""));
        assert!(serialized.contains("\"managed_meta\""));
    }
}
