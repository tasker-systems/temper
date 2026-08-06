//! Resource API types — shared between temper-api and temper-client.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use uuid::Uuid;

use super::managed_meta::ManagedMeta;
use temper_core::types::ids::{ContextId, ProfileId, ResourceId};
use temper_core::types::resource_view::ResourceView;

/// The two body-state enums moved to [`temper_core::types::resource`] — both are fields of
/// `ResourceView`, which temper-substrate reads back, and substrate sits *below* this crate.
/// Re-exported here so every `temper_workflow::types::resource::{BodyStorage, IngestState}`
/// call site resolves unchanged.
pub use temper_core::types::resource::{BodyStorage, IngestState};

/// Row type for resource listings — includes joined display fields
/// and managed_meta projections from `vault_resources_browse` view.
///
/// **Off the wire since Task 7** — every read and write surface answers in
/// [`ResourceView`]. What still reads this shape is temper-mcp's `EnrichedResource`
/// (`build_enriched`/`enrich_resources`); **Task 9** retires both, and this type with them.
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
    /// What guarantee does this body carry on read? `verbatim` — the body reads back **byte-for-byte**
    /// from the stored raw block bytes (`kb_block_content`, coverage-verified: every live block carries
    /// its source bytes). `derived` — the body is **reconstructed** from chunks (a lossy transform:
    /// CRLF→LF, trimmed, headings re-synthesized), which is the legacy path and any resource with only
    /// partial verbatim coverage. A *surfaced signal* derived from coverage, never an asserted flag.
    ///
    /// `Option` purely for **version skew** — the column is `NOT NULL` server-side (defaults `derived`),
    /// so a current server always sends it; `None` means the server predates W2 PR 3.
    pub body_storage: Option<BodyStorage>,
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
/// **Off the wire since Task 7** — `GET /api/resources/{id}` answers in [`ResourceView`].
/// What still reads this shape is temper-mcp's `get_resource`, via
/// `substrate_read::show_detail_select`; **Task 9** retires both.
///
/// No `ts_rs::TS` derive: ts-rs cannot codegen a `#[serde(flatten)]` field (see the
/// `act` field on `ResourceUpdateRequest`, `ts(skip)`-ped for the same reason) — which is
/// the constraint `ResourceView` was built to satisfy structurally.
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

// ─── Transitional narrowings between `ResourceView` and the incumbent shapes ───────────
//
// Task 7 moved the **wire** to `ResourceView` — `/api/resources`, `/api/ingest`, `/meta`, the
// temper-client and the CLI all speak it now, and the third impl (`From<ResourceRow> for
// ResourceView`, which `CloudBackend` needed to widen a row the client no longer returns) went
// with it.
//
// **The two below are Task 9's.** What holds them up is temper-mcp, which still answers in
// `EnrichedResource` — a `ResourceRow`-shaped projection built by `build_enriched` — and reads
// `substrate_read::show_detail_select`. Both narrow a view the read paths already produce; when
// the MCP tools answer in `ResourceView`, both impls and `ResourceRow`/`ResourceDetail`
// themselves have no remaining consumer.

/// Narrow a [`ResourceView`] back onto the incumbent [`ResourceRow`].
///
/// The four workflow columns come back out of `managed_meta`, which is exactly the
/// losslessness claim `ResourceView` makes: they did not go away, they went home.
///
/// **Transitional — deleted by Task 9**, with temper-mcp's `EnrichedResource`.
impl From<ResourceView> for ResourceRow {
    fn from(view: ResourceView) -> Self {
        Self {
            id: view.id,
            kb_context_id: view.kb_context_id,
            origin_uri: view.origin_uri,
            title: view.title,
            originator_profile_id: view.originator_profile_id,
            owner_profile_id: view.owner_profile_id,
            is_active: view.is_active,
            created: view.created,
            updated: view.updated,
            context_name: view.context_name,
            doc_type_name: view.doc_type_name,
            owner_handle: view.owner_handle,
            context_slug: view.context_slug,
            context_owner_ref: view.context_owner_ref,
            cogmap_id: view.cogmap_id,
            cogmap_name: view.cogmap_name,
            stage: view.managed_meta.stage,
            seq: view.managed_meta.seq,
            mode: view.managed_meta.mode,
            effort: view.managed_meta.effort,
            body_hash: view.body_hash,
            ingest_state: view.ingest_state,
            body_storage: view.body_storage,
        }
    }
}

/// Narrow a [`ResourceView`] onto the incumbent [`ResourceDetail`] (row + both meta tiers).
///
/// `managed_meta` is `Some` unconditionally because on the view it is not an `Option` at
/// all; `open_meta` rides through as the caller's section request left it.
///
/// **Transitional — deleted by Task 9**, with temper-mcp's `get_resource`.
impl From<ResourceView> for ResourceDetail {
    fn from(mut view: ResourceView) -> Self {
        let managed_meta = view.managed_meta.clone();
        let open_meta = view.open_meta.take();
        Self {
            row: view.into(),
            managed_meta: Some(managed_meta),
            open_meta,
        }
    }
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
    /// Goal-status filter (goal only): matches `temper-status` via the
    /// `kb_resource_workflow_props` pivot, exactly as `stage` does. Validated against the
    /// goal schema's enum surface-side, so an unknown value is an error rather than a
    /// silent full scan — which is what this field's absence used to produce.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    /// Goal filter (task only): the resolved goal `ResourceId` (as UUID). Returns
    /// only resources joined to this goal via a live `advances`→goal edge. The CLI/MCP
    /// resolve the caller's `--goal <ref>` to this UUID (trailing-UUID-only) before the
    /// query. `None` = no goal filter.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub goal: Option<Uuid>,
    /// Tag filter: a comma-separated list of tags. Returns only resources whose `open_meta.tags`
    /// contains **every** tag listed (AND, so each added tag narrows — the same direction every
    /// other filter here composes in). Matching is exact per tag and **case-insensitive**: the
    /// fold happens receive-side in `filtered_visible_page`, so the CLI, MCP and raw HTTP cannot
    /// disagree about it.
    ///
    /// A CSV string rather than a `Vec` for the same reason as `cogmap_ids`: the list endpoint is a
    /// GET whose params ride the query string (serde_urlencoded, which does not encode sequences).
    /// No tag in the corpus contains a comma (verified against production, 580 distinct tags), and
    /// a tag containing one cannot be expressed through this transport — which is a constraint on
    /// the tag vocabulary, not a silent truncation: the split is on `,` and each piece is trimmed.
    ///
    /// Unlike `stage` (task-only) and `status` (goal-only), tags are **not** doc-type-scoped — 14
    /// doc types carry them in production. So there is deliberately no mismatched-filter refusal
    /// for this field; pairing it with any `doc_type_name`, or with none, is meaningful.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tags: Option<String>,
    /// Cognitive-map scope: a comma-separated list of cogmap UUIDs. Returns only resources homed in
    /// one of these maps (`anchor_table = 'kb_cogmaps'`), intersected with the caller's visible set.
    /// A CSV string rather than a `Vec` because the list endpoint is a GET whose params ride the
    /// query string (serde_urlencoded, which does not encode sequences); the CLI/MCP resolve each
    /// `--cogmap <ref>` (trailing-UUID-only) and join here. `None`/empty = no cogmap filter.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cogmap_ids: Option<String>,
    pub sort: Option<ResourceSortField>,
    pub order: Option<SortOrder>,
    #[cfg_attr(feature = "typescript", ts(type = "number | null"))]
    pub limit: Option<i64>,
    #[cfg_attr(feature = "typescript", ts(type = "number | null"))]
    pub offset: Option<i64>,
    /// Optional sections to fill on each returned [`ResourceView`]: a comma-separated list of
    /// [`temper_core::types::resource_view::ResourceSection`] names (`open-meta`, `body`,
    /// `edges`). `None`/empty asks for none, which is the default list row.
    ///
    /// Replaces `meta_only`, which named a *response type* rather than a part: it selected a
    /// second envelope (`ResourceMetaListResponse`) whose rows were a different shape. There is
    /// one response shape now, so what the caller varies is which parts of it are filled.
    ///
    /// A CSV string rather than a `Vec` for the same reason as `tags` and `cogmap_ids` above —
    /// the list endpoint is a GET whose params ride the query string, and serde_urlencoded does
    /// not encode sequences. Parsed by `SectionSet::parse_csv`, which refuses an unknown name
    /// naming the whole valid set.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sections: Option<String>,
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
///
/// The envelope carries its own paging state, so a caller can tell a whole set from a
/// page of one without knowing what it asked for. `returned` and `truncated` used to be
/// injected render-time by the CLI alone (`temper-cli/src/commands/resource.rs`), so MCP
/// and raw-HTTP callers never received them — while the shipped agent skill instructs
/// every agent that "every list response carries `total`, `returned`, and `truncated`".
/// Same defect class as the `ref` the CLI was likewise the only surface to emit.
///
/// Build through [`ResourceListResponse::new`], which derives `returned` and `truncated`
/// from the page rather than trusting a caller to keep them consistent with `rows`.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "resource.ts"))]
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
pub struct ResourceListResponse {
    /// One [`ResourceView`] per row — **the same shape `show` answers in**, so a single-row
    /// list and a `show` of that resource serialize identically when neither asks for the body.
    /// There is no second list envelope: `?sections=` varies which parts of this shape are
    /// filled, never which type comes back.
    pub rows: Vec<ResourceView>,
    /// The FILTERED match count — every row the filters admit, before `limit`/`offset`.
    pub total: i64,
    pub facets: ResourceFacets,
    /// This page's row count. Always `rows.len()`; carried explicitly so the count
    /// survives a projection that drops or summarizes the rows.
    pub returned: i64,
    /// Are there matching rows beyond this page? `offset + returned < total`.
    ///
    /// Deliberately not `total > returned`, which is true on the last page of a walk
    /// (total 25, offset 20, returned 5) where nothing is in fact hidden. `true` here is
    /// what tells a caller it may not conclude a resource is absent from what it sees.
    pub truncated: bool,
    /// The effective page size the server applied, or `None` for an uncapped page
    /// (`--all`). An echo of what the caller asked for, not a clamp — no server-side
    /// clamp exists.
    pub limit: Option<i64>,
    /// The offset this page starts at; `0` when the caller sent none.
    pub offset: i64,
}

impl ResourceListResponse {
    /// Assemble a page and derive its paging state from it.
    ///
    /// The one place `returned` and `truncated` are computed. `returned` comes from the
    /// page itself rather than from a parameter, so it cannot disagree with `rows`.
    #[must_use]
    pub fn new(
        rows: Vec<ResourceView>,
        total: i64,
        facets: ResourceFacets,
        limit: Option<i64>,
        offset: i64,
    ) -> Self {
        let returned = rows.len() as i64;
        Self {
            rows,
            total,
            facets,
            returned,
            truncated: offset + returned < total,
            limit,
            offset,
        }
    }
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
    /// Owner-scoped create idempotency key (issue #581). A client-minted opaque token (a UUIDv7 by
    /// convention): a retried create carrying the same key converges on the already-committed
    /// resource instead of minting a duplicate, deduped on `(owner, key)`. `None` = an ordinary,
    /// non-idempotent create. The server mints the resource id; the key never supplies one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub idempotency_key: Option<uuid::Uuid>,
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
    /// Additive open_meta patch: each key's list is UNIONED with the value already
    /// stored, preserving existing order and appending only what is new.
    ///
    /// Exists because `open_meta` merges at the KEY level, so sending `{"tags":["x"]}`
    /// replaces the whole list — which made `--tags`, a flag whose help said *"Add tag"*,
    /// silently destroy every tag it did not name. Both semantics are legitimate, so they
    /// get separate channels rather than one channel with a mode: `open_meta` still
    /// replaces (and `{"tags":[]}` still clears), `open_meta_add` accumulates.
    ///
    /// Values must be arrays — a scalar here is a 400, not a silent overwrite. When a key
    /// appears in BOTH, the replace lands first and the union applies on top of it, so
    /// `open_meta {"tags":[]}` + `open_meta_add {"tags":["x"]}` reads as "clear, then add".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub open_meta_add: Option<serde_json::Value>,
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

    pub(super) fn sample_resource_row() -> ResourceRow {
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
            body_storage: Some(BodyStorage::Derived),
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
            open_meta_add: None,
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
            open_meta_add: None,
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

/// The list envelope's paging state — the three quantities an agent is instructed to
/// read (`total`, `returned`, `truncated`) and the two that say what page it is
/// looking at (`limit`, `offset`).
#[cfg(test)]
mod list_paging_tests {
    use super::*;

    /// A page of `n` rows. Only the count matters here — `returned` is `rows.len()`
    /// and nothing in the derivation reads a row's fields.
    fn page_of(n: usize) -> Vec<ResourceView> {
        let view = ResourceView {
            id: ResourceId::from(Uuid::nil()),
            r#ref: String::new(),
            title: "A Node".to_string(),
            origin_uri: String::new(),
            kb_context_id: None,
            context_name: None,
            context_slug: None,
            context_owner_ref: None,
            context_ref: None,
            cogmap_id: None,
            cogmap_name: None,
            doc_type_name: "concept".to_string(),
            owner_handle: "someone".to_string(),
            owner_profile_id: ProfileId::from(Uuid::nil()),
            originator_profile_id: ProfileId::from(Uuid::nil()),
            is_active: true,
            created: DateTime::<Utc>::from_timestamp(0, 0).expect("epoch"),
            updated: DateTime::<Utc>::from_timestamp(0, 0).expect("epoch"),
            body_hash: None,
            ingest_state: Some(IngestState::Complete),
            body_storage: Some(BodyStorage::Derived),
            managed_meta: ManagedMeta::default(),
            open_meta: None,
            content: None,
        };
        vec![view; n]
    }

    /// The default page of a large set hides rows, and says so.
    #[test]
    fn truncated_is_true_when_a_page_hides_rows() {
        let page =
            ResourceListResponse::new(page_of(20), 100, ResourceFacets::default(), Some(20), 0);

        assert_eq!(page.returned, 20, "`returned` is this page's row count");
        assert!(
            page.truncated,
            "80 of 100 matching rows are beyond this page"
        );
    }

    /// The off-by-one case: the last page of a walk shows the tail of the set, so
    /// nothing is hidden even though `total` still exceeds `returned`. This is what
    /// makes the derivation `offset + returned < total` rather than `total > returned`.
    #[test]
    fn truncated_is_false_on_the_last_page() {
        let page =
            ResourceListResponse::new(page_of(5), 25, ResourceFacets::default(), Some(20), 20);

        assert_eq!(page.offset, 20, "the page starts where the caller asked");
        assert!(
            !page.truncated,
            "rows 21-25 of 25 are the tail of the set — nothing is beyond this page"
        );
    }

    /// `--all` sends no limit and receives the whole filtered set, so there is nothing
    /// to warn about.
    #[test]
    fn all_returns_untruncated() {
        let page = ResourceListResponse::new(page_of(7), 7, ResourceFacets::default(), None, 0);

        assert_eq!(
            page.limit, None,
            "`--all` is an absent limit, not a large one"
        );
        assert_eq!(page.returned, page.total, "the whole set came back");
        assert!(!page.truncated);
    }
}
