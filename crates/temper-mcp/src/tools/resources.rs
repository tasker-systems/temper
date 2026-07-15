//! Resource tools — unified CRUD with name-based resolution and optional content.

use rmcp::model::CallToolResult;
use schemars::JsonSchema;
use serde::Deserialize;
use uuid::Uuid;

use temper_core::context_ref::parse_context_ref;
use temper_core::error::TemperError;
use temper_core::types::authorship::ActInput;
use temper_core::types::cognitive_maps::{GrantCapabilityRequest, RevokeCapabilityRequest};
use temper_core::types::home::HomeAnchor;
use temper_core::types::ids::{ProfileId, ResourceId};
use temper_core::types::workflow_job::EmbeddingStatus;
use temper_services::backend::{substrate_read, DbBackend};
use temper_services::error::ApiError;
use temper_services::services::access_service;
use temper_services::services::context_service::resolve_context_ref;
use temper_workflow::operations::{Backend, BodyUpdate, CreateResource, Surface};
use temper_workflow::types::managed_meta::ManagedMeta;

use crate::service::TemperMcpService;

/// Schemars `schema_with` for every `open_meta` input field.
///
/// `open_meta` is a free-form JSON object, held as `serde_json::Value` at
/// runtime. The default `JsonSchema` impl for `Value` advertises **no type** at
/// all, and some MCP clients, seeing no `type`, serialize the object as a JSON
/// **string** (`"{}"`) instead of an object — which the server then rejects with
/// `open_meta: "{}" is not of type "object"`. Advertising `type: object` fixes
/// the encoding at the client while `additionalProperties: true` keeps the tier
/// genuinely free-form (any key is allowed and stored).
///
/// We reuse the canonical recognized-conventions schema — the same
/// `open_meta.schema.json` that `describe_open_meta` serves — so the recognized
/// keys (`tags`, `keywords`, `descriptor`, `date`, …) are advertised as hints
/// from a single source of truth, with no risk of drift. `$schema`/`$id` are
/// stripped: they identify a standalone schema document, not an inlined
/// subschema, and would only confuse a client's schema resolution. A hardcoded
/// `type: object` fallback guarantees the load-bearing part of the fix even if
/// the canonical schema ever failed to parse.
fn open_meta_input_schema(_generator: &mut schemars::SchemaGenerator) -> schemars::Schema {
    fn recognized_conventions() -> Option<schemars::Schema> {
        let mut value = temper_workflow::schema::open_meta_schema_value().ok()?;
        if let Some(obj) = value.as_object_mut() {
            obj.remove("$schema");
            obj.remove("$id");
        }
        schemars::Schema::try_from(value).ok()
    }

    recognized_conventions().unwrap_or_else(|| {
        schemars::json_schema!({
            "type": "object",
            "description": "Open (caller-defined) frontmatter as a JSON object. Free-form: any key is allowed and stored.",
            "additionalProperties": true,
        })
    })
}

// ── Input structs ──────────────────────────────────────────────────

/// MCP input for create_resource.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct CreateResourceInput {
    /// Context ref (UUID or `@owner/slug`), resolved server-side.
    /// Bare names (no `@` prefix, not a UUID) are rejected. Mutually exclusive
    /// with `cogmap`; supply exactly one home.
    #[serde(default)]
    pub context_ref: Option<String>,
    /// Cognitive-map ref (UUID or decorated `slug-<uuid>`) to home the resource
    /// in. Mutually exclusive with `context_ref`; supply exactly one home.
    #[serde(default)]
    pub cogmap: Option<String>,
    /// Human-readable doc type name (e.g. "task", "session", "research").
    pub doc_type_name: String,
    /// Resource title.
    pub title: String,
    /// Optional markdown content body. Processed through the ingest
    /// pipeline (chunk + embed) synchronously on create.
    pub content: Option<String>,
    /// Block-provenance sources this body was distilled from: resource refs (UUID or decorated) or
    /// external http/https URLs. Each becomes a provenance record on the created resource's body
    /// block (list position → accretion `seq`). Requires `content` — with no body block there is
    /// nothing to attribute.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sources: Option<Vec<String>>,
    /// Optional goal link: a ref (UUID or decorated `slug-<uuid>`) of the goal this resource
    /// advances. Projects a live `advances`→goal edge on create. Relationship-fated, not
    /// metadata — first-class here, never a `managed_meta` key.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub goal: Option<String>,
    /// Optional origin URI. Defaults to `mcp://agent/{uuid}`. An external (http/https) origin URI
    /// with `content` but no explicit `sources` seeds a Remote block-provenance record pointing at
    /// it (issue #352), so a resource distilled from a URL is citation-grade by default.
    pub origin_uri: Option<String>,
    /// Optional owner (defaults to @me). Reserved for future team scoping.
    pub owner: Option<String>,
    /// Managed workflow/provenance frontmatter — a **closed, temper-owned
    /// vocabulary** of optional `temper-*` keys: stage/mode/effort/status/seq/
    /// branch/pr/llm-model/llm-run/provenance. Identity (`title`), type
    /// (`doc_type_name`), and home (`context_ref`/`cogmap`) are first-class
    /// fields on this input, not metadata. An unknown key is rejected;
    /// caller-defined ("bring-your-own") fields belong in `open_meta`.
    #[serde(default)]
    pub managed_meta: Option<ManagedMeta>,
    /// Open (caller-defined) frontmatter as a free-form JSON **object**. Any key
    /// is allowed and stored; recognized keys (`tags`, `keywords`, `descriptor`,
    /// `date`, …) are advertised for shape/ranking but not required.
    #[serde(default)]
    #[schemars(schema_with = "open_meta_input_schema")]
    pub open_meta: Option<serde_json::Value>,
    /// Per-act correlation (`invocation_id`) + discrete agent authorship
    /// (`reasoning`/`confidence`/`rationale`/`persona`/`model`). Flattened as top-level keys;
    /// all optional. `confidence` is required when any other authorship field is supplied.
    #[serde(flatten)]
    pub act: ActInput,
}

/// MCP input for get_resource.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct GetResourceInput {
    /// Resource ref: a UUID or the decorated `slug-<uuid>` form.
    pub id: String,
    /// If true, includes the full reconstituted markdown content.
    pub include_content: Option<bool>,
    /// Subselect top-level response keys. Anchor key `id` is always
    /// preserved. Nested paths (containing `.`) rejected with a hint
    /// pointing at `jq` — MCP callers should perform deeper projection
    /// at their own end. When None or empty, no filtering is applied.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fields: Option<Vec<String>>,
}

/// MCP input for get_block_provenance.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct GetBlockProvenanceInput {
    /// The resource whose per-block provenance to read (UUID).
    pub resource: Uuid,
}

/// MCP input for resource_lineage (Ledger L2).
#[derive(Debug, Deserialize, JsonSchema)]
pub struct ResourceLineageInput {
    /// Resource ref: a UUID or the decorated `slug-<uuid>` form.
    pub id: String,
    /// Max hop distance to walk from the seed (default 16, clamped to 1..=64).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub depth: Option<i32>,
}

/// MCP input for annotate_resource (issue #355) — attach provenance sources without a body revise.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct AnnotateResourceInput {
    /// The resource to annotate (UUID).
    pub id: Uuid,
    /// Sources to attach to the addressed block: resource refs (UUID or decorated) or external
    /// http/https URLs. A URL may carry a span-locator fragment (e.g. `…/doc.md#L120-L180`), which is
    /// preserved verbatim and surfaced by `get_block_provenance`. Position → accretion `seq`. Required
    /// and non-empty — an annotate with nothing to attribute is an error.
    pub sources: Vec<String>,
    /// Which content block to annotate (a block UUID). Omit to address the resource's sole non-folded
    /// body block (the default); required for a resource with more than one block. The block must
    /// belong to the resource and be non-folded.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_block: Option<Uuid>,
    /// Per-act correlation (`invocation_id`) + discrete agent authorship. Flattened top-level keys;
    /// all optional. `confidence` required when any other authorship field is supplied.
    #[serde(flatten)]
    pub act: ActInput,
}

/// MCP input for list_resources.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct ListResourcesInput {
    /// Filter by context ref (UUID or @owner/slug). Bare context names are rejected.
    pub context_ref: Option<String>,
    /// Filter by doc type name (e.g. "task", "research").
    pub doc_type_name: Option<String>,
    /// Filter by goal: a ref (UUID or decorated `slug-<uuid>`) of a goal resource. Returns only
    /// resources linked to it via a live `advances`→goal edge (task-only in practice).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub goal: Option<String>,
    /// Max results (default 50, max 200).
    pub limit: Option<i64>,
    /// Pagination offset.
    pub offset: Option<i64>,
    /// Subselect top-level response keys for each row. Anchor key `id`
    /// is always preserved per row. Nested paths (containing `.`) are
    /// rejected with a hint pointing at `jq` — MCP callers should
    /// perform deeper projection at their own end. When None or empty,
    /// no filtering is applied.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fields: Option<Vec<String>>,
}

/// MCP input for update_resource.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct UpdateResourceInput {
    /// UUID of the resource to update.
    pub id: Uuid,
    /// New title.
    pub title: Option<String>,
    /// Set (or replace) the resource's goal: a ref (UUID or decorated `slug-<uuid>`) of the goal
    /// this resource advances. Folds any existing `advances`→goal edge and asserts the new one.
    /// Mutually exclusive with `clear_goal`. Omit to leave the goal edge untouched.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub goal: Option<String>,
    /// Clear the resource's goal: when `true`, folds the current `advances`→goal edge, leaving it
    /// goal-less. The tri-state complement to `goal` (absent = untouched, `goal` = set/replace,
    /// `clear_goal` = retract). Mutually exclusive with `goal`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub clear_goal: Option<bool>,
    /// New markdown content. Replaces existing content and triggers
    /// re-processing.
    pub content: Option<String>,
    /// Block-provenance sources this body was distilled from: resource refs (UUID or decorated) or
    /// external http/https URLs. Each becomes a provenance record on the resource's body block (list
    /// position → accretion `seq`). Requires `content` — with no body update there is nothing to
    /// attribute.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sources: Option<Vec<String>>,
    /// Which content block the body revise + `sources` target (a block UUID). Omit to address the
    /// resource's sole body block (the default); required to revise a resource that has more than one
    /// block. The block must belong to the resource and be non-folded. Requires `content`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_block: Option<Uuid>,
    /// Managed workflow/provenance frontmatter — a **closed, temper-owned
    /// vocabulary** of optional `temper-*` keys: stage/mode/effort/status/seq/
    /// branch/pr/llm-model/llm-run/provenance. Identity (`title`), type
    /// (`doc_type_name`), and home (`context_ref`/`cogmap`) are first-class
    /// fields on this input, not metadata. An unknown key is rejected;
    /// caller-defined ("bring-your-own") fields belong in `open_meta`.
    #[serde(default)]
    pub managed_meta: Option<ManagedMeta>,
    /// Open (caller-defined) frontmatter as a free-form JSON **object**. Any key
    /// is allowed and stored; recognized keys (`tags`, `keywords`, `descriptor`,
    /// `date`, …) are advertised for shape/ranking but not required.
    #[serde(default)]
    #[schemars(schema_with = "open_meta_input_schema")]
    pub open_meta: Option<serde_json::Value>,
    /// Per-act correlation (`invocation_id`) + discrete agent authorship. Flattened top-level
    /// keys; all optional. `confidence` required when any other authorship field is supplied.
    #[serde(flatten)]
    pub act: ActInput,
}

/// MCP input for update_resource_meta.
///
/// Use when the caller wants to change only a resource's frontmatter
/// (managed_meta / open_meta) without re-chunking or re-embedding the
/// body. This is the MCP peer of `PUT /api/resources/{id}/meta`.
///
/// `managed_meta` is a **closed, temper-owned vocabulary** — exactly the
/// optional `temper-*` Property keys (stage/mode/effort/status/seq/branch/pr/
/// llm-model/llm-run/provenance). Unknown keys are rejected; this path is
/// Property-only — identity (`title`/`slug`), type, and home are NOT accepted
/// here (change them via `update_resource`). `open_meta` stays a free-form JSON
/// object by design — the open tier accepts any key, and is advertised to
/// clients as `type: object` with `additionalProperties: true` (via the
/// `open_meta_input_schema` helper).
#[derive(Debug, Deserialize, JsonSchema)]
pub struct UpdateResourceMetaInput {
    /// UUID of the resource to update.
    pub id: Uuid,
    /// New managed (temper-*) frontmatter — a **closed, temper-owned
    /// vocabulary**. Only the typed temper-* keys are accepted; an unknown key
    /// is rejected. Caller-defined fields belong in `open_meta`.
    pub managed_meta: ManagedMeta,
    /// New open (caller-defined) frontmatter as a free-form JSON **object**. Any
    /// key is allowed and stored; recognized keys (`tags`, `keywords`,
    /// `descriptor`, `date`, …) are advertised for shape/ranking but not required.
    #[schemars(schema_with = "open_meta_input_schema")]
    pub open_meta: serde_json::Value,
    /// Per-act correlation (`invocation_id`) + discrete agent authorship. Flattened top-level
    /// keys; all optional. `confidence` required when any other authorship field is supplied.
    #[serde(flatten)]
    pub act: ActInput,
}

/// MCP input for delete_resource.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct DeleteResourceInput {
    /// UUID of the resource to delete.
    pub id: Uuid,
    /// Per-act correlation (`invocation_id`) + discrete agent authorship. Flattened top-level
    /// keys; all optional. `confidence` required when any other authorship field is supplied.
    #[serde(flatten)]
    pub act: ActInput,
}

// ── Response types ─────────────────────────────────────────────────

/// Status of a create_resource operation.
#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CreateStatus {
    Created,
    Existing,
}

/// Typed response for create_resource.
#[derive(Debug, serde::Serialize)]
pub struct CreateResourceResponse {
    pub resource: EnrichedResource,
    pub status: CreateStatus,
}

/// Typed response for delete_resource.
#[derive(Debug, serde::Serialize)]
pub struct DeleteResourceResponse {
    pub deleted: bool,
    pub id: Uuid,
}

/// Typed response for update_resource_meta.
#[derive(Debug, serde::Serialize)]
pub struct UpdateResourceMetaResponse {
    pub updated: bool,
    pub id: Uuid,
}

// ── Response enrichment ────────────────────────────────────────────

/// Enriched resource response with human-readable names.
///
/// `managed_meta` and `open_meta` always carry the resource's
/// frontmatter — every enrichment path populates them. The
/// `skip_serializing_if` covers the genuine no-manifest case (a
/// resource created via POST without a body trio has no manifest row
/// yet), and keeps the wire shape stable for those resources.
#[derive(Debug, serde::Serialize)]
pub struct EnrichedResource {
    pub id: Uuid,
    pub title: String,
    pub slug: Option<String>,
    pub context_name: String,
    pub doc_type_name: String,
    pub owner: String,
    pub origin_uri: String,
    /// Decorated, self-resolving identifier: `sluggify(title)-<uuid>`.
    pub r#ref: String,
    pub is_active: bool,
    pub created: chrono::DateTime<chrono::Utc>,
    pub updated: chrono::DateTime<chrono::Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub managed_meta: Option<ManagedMeta>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub open_meta: Option<serde_json::Value>,
    /// Derived embedding-readiness (issue #299, Phase 4): `ready` once the resource's vector is
    /// searchable, `pending` while an async embed is in flight, `failed` when it needs re-driving.
    /// FTS is always immediate; this tracks only the eventually-consistent vector under async embed.
    pub embedding_status: EmbeddingStatus,
}

/// Assemble an [`EnrichedResource`] from a row plus its already-fetched
/// meta. Pure assembly — `context_name`/`doc_type_name` are read off the
/// row (both schemas' full-row reads populate them via the browse view /
/// readback reconstruction), so there is no per-row context/doc_type DB
/// round-trip. Meta is a required, explicit input, so every caller decides
/// where it comes from (a batch query for lists, `get_content`'s response
/// for the content path).
fn build_enriched(
    row: &temper_workflow::types::resource::ResourceRow,
    managed_meta: Option<ManagedMeta>,
    open_meta: Option<serde_json::Value>,
    embedding_status: EmbeddingStatus,
) -> EnrichedResource {
    EnrichedResource {
        id: row.id.into(),
        title: row.title.clone(),
        slug: None,
        context_name: row
            .home_display()
            .map(str::to_owned)
            .unwrap_or_else(|| "—".to_string()),
        doc_type_name: row.doc_type_name.clone(),
        owner: "@me".to_string(),
        origin_uri: row.origin_uri.clone(),
        r#ref: temper_workflow::operations::decorated_ref(&row.title, row.id),
        is_active: row.is_active,
        created: row.created,
        updated: row.updated,
        managed_meta,
        open_meta,
        embedding_status,
    }
}

/// Enrich a batch of resource rows, each with its `managed_meta` /
/// `open_meta`. The meta tier is fetched through
/// [`substrate_read::get_meta_batch_select`] (flag-gated): the Legacy arm
/// is a single `get_meta_batch` query, so the list surface is not N+1 on
/// meta; the Next arm projects the substrate per id. Rows are pre-scoped
/// to the caller (the rows came from a visibility-scoped query), so the
/// Legacy batch fetch skips a redundant per-row visibility check.
pub async fn enrich_resources(
    pool: &sqlx::PgPool,
    profile_id: Uuid,
    rows: &[temper_workflow::types::resource::ResourceRow],
) -> Result<Vec<EnrichedResource>, rmcp::ErrorData> {
    let ids: Vec<ResourceId> = rows.iter().map(|row| row.id).collect();
    let mut meta = substrate_read::get_meta_batch_select(pool, ProfileId::from(profile_id), &ids)
        .await
        .map_err(|e| rmcp::ErrorData::internal_error(format!("Failed to get meta: {e}"), None))?;

    // One batch read of derived embedding-readiness alongside the meta fetch (design §8) — keeps the
    // list/enrich path off an N+1. Absent ids (shouldn't happen) default to `ready`.
    let raw_ids: Vec<Uuid> = ids.iter().map(|id| Uuid::from(*id)).collect();
    let statuses = temper_services::services::embed_service::embedding_status_batch(pool, &raw_ids)
        .await
        .map_err(|e| {
            rmcp::ErrorData::internal_error(format!("Failed to get embedding status: {e}"), None)
        })?;

    let mut enriched = Vec::with_capacity(rows.len());
    for row in rows {
        let (managed_meta, open_meta) = meta
            .remove(&row.id)
            .map(|m| (m.managed_meta, m.open_meta))
            .unwrap_or((None, None));
        let embedding_status = statuses
            .get(&Uuid::from(row.id))
            .copied()
            .unwrap_or(EmbeddingStatus::Ready);
        enriched.push(build_enriched(
            row,
            managed_meta,
            open_meta,
            embedding_status,
        ));
    }
    Ok(enriched)
}

/// Enrich a single resource row, including its frontmatter. Thin
/// single-row wrapper over [`enrich_resources`].
pub async fn enrich_resource(
    pool: &sqlx::PgPool,
    profile_id: Uuid,
    row: &temper_workflow::types::resource::ResourceRow,
) -> Result<EnrichedResource, rmcp::ErrorData> {
    Ok(
        enrich_resources(pool, profile_id, std::slice::from_ref(row))
            .await?
            .pop()
            .expect("enrich_resources returns one row per input row"),
    )
}

// ── Helpers ────────────────────────────────────────────────────────

fn to_text<T: serde::Serialize>(value: &T) -> String {
    serde_json::to_string_pretty(value).unwrap_or_else(|_| "{}".to_string())
}

/// Build the command's `BodyUpdate` from optional content + optional resource-id sources,
/// mapping each source id to `ProvenanceSource::Resource` (list position → accretion seq
/// downstream). Shared by create and update. Guards the parse-don't-validate invariant:
/// sources without a body block have nothing to attribute, so that combination is an
/// `invalid_params` error rather than a silent drop.
fn provenance_body(
    content: Option<String>,
    sources: Option<Vec<String>>,
    content_block: Option<Uuid>,
) -> Result<Option<BodyUpdate>, rmcp::ErrorData> {
    match content {
        Some(content) if !content.is_empty() => {
            let mut body = BodyUpdate::new(content);
            // Classify each source (http/https URL → Remote, else ref → Resource) with the same shared
            // resolver the CLI uses; an unparseable value is a hard error, never a silent drop.
            body.sources = sources
                .unwrap_or_default()
                .iter()
                .map(|s| temper_workflow::operations::resolve_provenance_source(s))
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| {
                    rmcp::ErrorData::invalid_params(format!("invalid sources value: {e}"), None)
                })?;
            body.content_block = content_block;
            Ok(Some(body))
        }
        _ => {
            if sources.is_some_and(|s| !s.is_empty()) {
                return Err(rmcp::ErrorData::invalid_params(
                    "sources supplied without content — there is no body block to attribute"
                        .to_owned(),
                    None,
                ));
            }
            if content_block.is_some() {
                return Err(rmcp::ErrorData::invalid_params(
                    "content_block supplied without content — there is no body revise to address"
                        .to_owned(),
                    None,
                ));
            }
            Ok(None)
        }
    }
}

// ── Tool handlers ──────────────────────────────────────────────────

/// Build the shared `CreateResource` command from an MCP create input: validate the owner format,
/// resolve the home anchor (running the cogmap producer gate before any write), derive the slug from
/// the title, default `origin_uri`, assemble the act context, and resolve the optional goal ref.
///
/// Shared by [`create_resource`] and `tools::ingest::ingest_begin` so the two cannot drift — a
/// segmented begin creates a resource by exactly the same rules as a one-shot create.
pub(crate) async fn build_create_command(
    svc: &TemperMcpService,
    profile_id: ProfileId,
    input: CreateResourceInput,
) -> Result<CreateResource, rmcp::ErrorData> {
    let pool = &svc.api_state.pool;

    // Validate owner format if provided (stub for R11)
    if let Some(ref owner) = input.owner {
        if !owner.starts_with('@') && !owner.starts_with('+') {
            return Err(rmcp::ErrorData::invalid_params(
                "owner must start with @ (profile) or + (team)".to_string(),
                None,
            ));
        }
    }

    // Resolve the home anchor — exactly one of a cognitive map or a context.
    // Symmetric with the HTTP ingest handler: the cogmap branch runs the
    // producer write gate (auth before writes) before homing in the map.
    let home = match (input.cogmap.as_deref(), input.context_ref.as_deref()) {
        (Some(_), Some(_)) => {
            return Err(rmcp::ErrorData::invalid_params(
                "context_ref and cogmap are mutually exclusive; supply exactly one home"
                    .to_string(),
                None,
            ));
        }
        (None, None) => {
            return Err(rmcp::ErrorData::invalid_params(
                "no home specified — supply exactly one of context_ref or cogmap".to_string(),
                None,
            ));
        }
        (Some(cogmap_ref), None) => {
            // Trailing-UUID-only resolution (no server lookup).
            let map = temper_workflow::operations::parse_ref(cogmap_ref)
                .map_err(|e| {
                    rmcp::ErrorData::invalid_params(format!("invalid cogmap ref: {e}"), None)
                })?
                .0;
            let cogmap = temper_core::types::ids::CogmapId::from(map);
            // Auth before writes: producer gate (service seam → an explicit `can_write` grant on the
            // map; `cogmap_authorable_by_profile`, NOT membership — membership confers read only, per
            // the Q-A flip). Shares the one `cogmap_service::authorable_by_profile` seam with the HTTP
            // handler; no inline SQL on the surface. This is a fast-fail pre-check — `DbBackend::
            // create_resource` re-enforces the same predicate on the shared write path (F1).
            let ok = temper_services::services::cogmap_service::authorable_by_profile(
                pool, profile_id, cogmap,
            )
            .await
            .map_err(|e| rmcp::ErrorData::internal_error(e.to_string(), None))?;
            if !ok {
                return Err(rmcp::ErrorData::invalid_params(
                    "not authorized to author in this cognitive map".to_string(),
                    None,
                ));
            }
            HomeAnchor::Cogmap(cogmap)
        }
        (None, Some(context_ref)) => {
            // Parse + resolve the context ref (UUID or @owner/slug). Bare names are rejected.
            let cref = parse_context_ref(context_ref).map_err(|e| {
                rmcp::ErrorData::invalid_params(format!("invalid context_ref: {e}"), None)
            })?;
            let context = resolve_context_ref(pool, profile_id, &cref)
                .await
                .map_err(|e| {
                    rmcp::ErrorData::invalid_params(format!("context not found: {e}"), None)
                })?;
            HomeAnchor::Context(context)
        }
    };

    // Slug is §7-dissolved (never stored; addressing is trailing-UUID-only), so it is NOT a
    // caller input — always derived from the title via the one canonical slugifier, whose
    // output is validate_slug-conformant (ASCII, runs collapsed). (issue #307 Bug 2)
    let slug = temper_workflow::operations::sluggify(&input.title);

    let origin_uri = input
        .origin_uri
        .unwrap_or_else(|| format!("mcp://agent/{}", Uuid::new_v4()));

    let content = input.content.unwrap_or_default();

    // Identity travels first-class on the cmd (title/slug below); managed_meta is
    // Property-only. The caller-supplied managed_meta passes through untouched —
    // the DbBackend validation pipeline injects identity into the validation
    // document from the typed title/slug.
    let managed_meta = input.managed_meta.unwrap_or_default();

    // Create always writes a single new body block; per-block addressing is an update-only concern.
    let body = provenance_body(Some(content), input.sources, None)?;

    // Assemble the per-act correlation + authorship from the flattened discrete fields. The
    // shared assembler enforces "confidence required iff authorship supplied"; map its
    // BadRequest to invalid_params.
    let act = input
        .act
        .into_act_context()
        .map_err(|e| rmcp::ErrorData::invalid_params(e.to_string(), None))?;

    // Resolve the optional goal ref client-side (trailing-UUID-only, like `edge assert`); the
    // backend projects the live `advances`→goal edge after create.
    let goal = input
        .goal
        .as_deref()
        .map(temper_workflow::operations::parse_ref)
        .transpose()
        .map_err(|e| rmcp::ErrorData::invalid_params(e.to_string(), None))?;

    let cmd = CreateResource {
        slug,
        doctype: input.doc_type_name,
        home,
        title: input.title,
        body,
        managed_meta,
        open_meta: input.open_meta,
        goal,
        origin_uri: Some(origin_uri),
        chunks_packed: None,
        content_hash: None,
        act,
        origin: Surface::Mcp,
    };

    Ok(cmd)
}

pub async fn create_resource(
    svc: &TemperMcpService,
    input: CreateResourceInput,
) -> Result<CallToolResult, rmcp::ErrorData> {
    let profile = svc.require_profile().await?;
    let pool = &svc.api_state.pool;
    let profile_id = ProfileId::from(profile.id);

    let cmd = build_create_command(svc, profile_id, input).await?;

    let backend = DbBackend::new(pool.clone(), profile_id);
    let out = backend.create_resource(cmd).await.map_err(|e| match e {
        TemperError::NotFound(_) => rmcp::ErrorData::invalid_params(
            "Context or doc_type not found. Use create_context / list_doc_types to verify."
                .to_string(),
            None,
        ),
        TemperError::BadRequest(msg) => rmcp::ErrorData::invalid_params(msg, None),
        other => {
            rmcp::ErrorData::internal_error(format!("Failed to create resource: {other}"), None)
        }
    })?;
    let resource = out.value;

    let enriched = enrich_resource(pool, profile.id, &resource).await?;
    let response = CreateResourceResponse {
        resource: enriched,
        status: CreateStatus::Created,
    };
    Ok(CallToolResult::success(vec![rmcp::model::Content::text(
        to_text(&response),
    )]))
}

/// Map a `ProjectionError` to an `rmcp::ErrorData` invalid-params response.
///
/// Centralises the error-boundary translation so both `get_resource` and
/// `list_resources` can call `.map_err(map_projection_err)?` without
/// duplicating the match arms.
fn map_projection_err(e: temper_core::projection::ProjectionError) -> rmcp::ErrorData {
    use temper_core::projection::ProjectionError;
    match e {
        ProjectionError::DottedPath { hint } => rmcp::ErrorData::invalid_params(
            format!("fields supports top-level keys only; use jq for nested projection: {hint}"),
            None,
        ),
        ProjectionError::EmptyField => {
            rmcp::ErrorData::invalid_params("fields contained an empty entry".to_string(), None)
        }
    }
}

// WS6 Spec B: `get_resource` routes the base read through `substrate_read` (the single backend
// post-collapse). The row comes from
// `show_select`, meta from `get_meta_select`, and body (when requested) from `get_content_select` —
// uniform across backends. Sourcing meta via `get_meta_select` (not the legacy "`get_content` returns
// meta" coupling) is what lets the Next path work: its `get_content` returns `None` meta. The §9 read
// floor (row + managed/open) is exactly what `build_enriched` assembles; relationship enrichment is a
// separate, post-floor concern not layered here. The MCP `search` tool is likewise routed (see search.rs).
pub async fn get_resource(
    svc: &TemperMcpService,
    input: GetResourceInput,
) -> Result<CallToolResult, rmcp::ErrorData> {
    let profile = svc.require_profile().await?;
    let pool = &svc.api_state.pool;

    let id = temper_workflow::operations::parse_ref(&input.id)
        .map_err(|e| rmcp::ErrorData::invalid_params(e.to_string(), None))?;

    // `show_detail_select` is the one place that composes the row + meta readbacks; it is
    // what `GET /api/resources/{id}` returns too, so both surfaces read the same shape.
    let detail = substrate_read::show_detail_select(pool, ProfileId::from(profile.id), id)
        .await
        .map_err(|e| {
            rmcp::ErrorData::internal_error(format!("Failed to get resource: {e}"), None)
        })?;
    let row = detail.row;

    let body_markdown = if input.include_content.unwrap_or(false) {
        let content = substrate_read::get_content_select(pool, ProfileId::from(profile.id), row.id)
            .await
            .map_err(|e| {
                rmcp::ErrorData::internal_error(format!("Failed to get content: {e}"), None)
            })?;
        Some(content.markdown)
    } else {
        None
    };

    let embedding_status = temper_services::services::embed_service::embedding_status_batch(
        pool,
        &[Uuid::from(row.id)],
    )
    .await
    .map_err(|e| {
        rmcp::ErrorData::internal_error(format!("Failed to get embedding status: {e}"), None)
    })?
    .get(&Uuid::from(row.id))
    .copied()
    .unwrap_or(EmbeddingStatus::Ready);

    let enriched = build_enriched(
        &row,
        detail.managed_meta,
        detail.open_meta,
        embedding_status,
    );

    let enriched_value = serde_json::to_value(&enriched)
        .map_err(|e| rmcp::ErrorData::internal_error(format!("Failed to serialize: {e}"), None))?;

    let filtered = if let Some(fields) = input.fields.as_deref() {
        temper_core::projection::apply_top_level_filter(enriched_value, fields, "id")
            .map_err(map_projection_err)?
    } else {
        enriched_value
    };

    let mut parts = vec![rmcp::model::Content::text(
        serde_json::to_string_pretty(&filtered).unwrap_or_else(|_| "{}".to_string()),
    )];
    if let Some(markdown) = body_markdown {
        parts.push(rmcp::model::Content::text(markdown));
    }
    Ok(CallToolResult::success(parts))
}

/// Itemized per-block provenance for a resource. Service-direct read (reads bypass the Backend
/// trait); the access gate lives in the SQL function — an unreadable resource yields an empty list.
pub async fn get_block_provenance(
    svc: &TemperMcpService,
    input: GetBlockProvenanceInput,
) -> Result<CallToolResult, rmcp::ErrorData> {
    let profile = svc.require_profile().await?;
    let pool = &svc.api_state.pool;

    let rows = substrate_read::resource_block_provenance_select(
        pool,
        ProfileId::from(profile.id),
        input.resource,
    )
    .await
    .map_err(|e| {
        rmcp::ErrorData::internal_error(format!("Failed to read provenance: {e}"), None)
    })?;

    Ok(CallToolResult::success(vec![rmcp::model::Content::text(
        to_text(&rows),
    )]))
}

/// Ledger L2 — a resource's bidirectional `derived_from` lineage: what it derives
/// from (ancestors) and what derives from it (descendants), access-gated.
/// Service-direct read; the walk + gate live in the `resource_lineage` SQL
/// function. An unreadable/absent seed is a not-found error (not an empty leak).
pub async fn resource_lineage(
    svc: &TemperMcpService,
    input: ResourceLineageInput,
) -> Result<CallToolResult, rmcp::ErrorData> {
    let profile = svc.require_profile().await?;
    let pool = &svc.api_state.pool;

    let id = temper_workflow::operations::parse_ref(&input.id)
        .map_err(|e| rmcp::ErrorData::invalid_params(e.to_string(), None))?;
    let depth = input.depth.unwrap_or(16).clamp(1, 64);

    let lineage = temper_services::services::lineage_service::resource_lineage(
        pool,
        profile.id,
        Uuid::from(id),
        depth,
    )
    .await
    .map_err(|e| match e {
        temper_services::error::ApiError::NotFound => {
            rmcp::ErrorData::invalid_params("resource not found or not readable".to_string(), None)
        }
        other => rmcp::ErrorData::internal_error(format!("lineage read failed: {other}"), None),
    })?;

    Ok(CallToolResult::success(vec![rmcp::model::Content::text(
        to_text(&lineage),
    )]))
}

pub async fn list_resources(
    svc: &TemperMcpService,
    input: ListResourcesInput,
) -> Result<CallToolResult, rmcp::ErrorData> {
    let profile = svc.require_profile().await?;
    let pool = &svc.api_state.pool;

    // Resolve the optional goal filter ref client-side (trailing-UUID-only, like the write path).
    let goal = input
        .goal
        .as_deref()
        .map(temper_workflow::operations::parse_ref)
        .transpose()
        .map_err(|e| rmcp::ErrorData::invalid_params(e.to_string(), None))?;

    // Build list params — context_ref is resolved server-side by filtered_visible_page;
    // bare context names are rejected there (spec Decision 1).
    let params = temper_workflow::types::resource::ResourceListParams {
        context_ref: input.context_ref.clone(),
        doc_type_name: input.doc_type_name.clone(),
        goal: goal.map(uuid::Uuid::from),
        limit: input.limit.or(Some(50)).map(|l| l.min(200)),
        offset: input.offset,
        ..Default::default()
    };
    let list_result = substrate_read::list_select(pool, ProfileId::from(profile.id), params)
        .await
        .map_err(|e| match e {
            // A bare context name or invalid ref is rejected with BadRequest (spec Decision 1).
            // An unresolvable ref (not visible / not found) yields NotFound.
            // Both are caller errors → invalid_params (400-class).
            temper_services::error::ApiError::BadRequest(msg) => {
                rmcp::ErrorData::invalid_params(msg, None)
            }
            temper_services::error::ApiError::NotFound => rmcp::ErrorData::invalid_params(
                format!(
                    "unknown filter: context_ref {:?} not found or not visible",
                    input.context_ref
                ),
                None,
            ),
            other => {
                rmcp::ErrorData::internal_error(format!("Failed to list resources: {other}"), None)
            }
        })?;

    let enriched = enrich_resources(pool, profile.id, &list_result.rows).await?;

    let array_value = serde_json::to_value(&enriched)
        .map_err(|e| rmcp::ErrorData::internal_error(format!("Failed to serialize: {e}"), None))?;

    let filtered = if let Some(fields) = input.fields.as_deref() {
        temper_core::projection::apply_top_level_filter(array_value, fields, "id")
            .map_err(map_projection_err)?
    } else {
        array_value
    };

    Ok(CallToolResult::success(vec![rmcp::model::Content::text(
        serde_json::to_string_pretty(&filtered).unwrap_or_else(|_| "[]".to_string()),
    )]))
}

pub async fn update_resource(
    svc: &TemperMcpService,
    input: UpdateResourceInput,
) -> Result<CallToolResult, rmcp::ErrorData> {
    let profile = svc.require_profile().await?;
    let pool = &svc.api_state.pool;
    let profile_id = ProfileId::from(profile.id);
    let resource_id = ResourceId::from(input.id);

    // Identity (title) travels first-class on the cmd; managed_meta is Property-only. The
    // caller-supplied managed_meta passes through untouched — the DbBackend validation pipeline
    // injects identity into the validation document from the effective title (cmd.title / current
    // row). Slug is §7-dissolved and not a caller input; the backend derives it. (issue #307)
    let managed_meta = input.managed_meta.unwrap_or_default();

    let act = input
        .act
        .into_act_context()
        .map_err(|e| rmcp::ErrorData::invalid_params(e.to_string(), None))?;
    let body = provenance_body(input.content, input.sources, input.content_block)?;
    // Goal patch is tri-state: `goal` (set/replace, ref resolved client-side) wins over
    // `clear_goal` (retract); absent leaves the goal edge untouched.
    let goal = match (input.goal.as_deref(), input.clear_goal) {
        (Some(r), _) => Some(temper_workflow::operations::GoalPatch::Set(
            temper_workflow::operations::parse_ref(r)
                .map_err(|e| rmcp::ErrorData::invalid_params(e.to_string(), None))?,
        )),
        (None, Some(true)) => Some(temper_workflow::operations::GoalPatch::Clear),
        _ => None,
    };
    let cmd = temper_workflow::operations::UpdateResource {
        resource: resource_id,
        title: input.title.clone(),
        slug: None,
        body,
        managed_meta: Some(managed_meta),
        open_meta: input.open_meta,
        goal,
        move_to: None,
        context_ref: None,
        act,
        origin: Surface::Mcp,
    };

    let backend = DbBackend::new(pool.clone(), profile_id);
    backend.update_resource(cmd).await.map_err(|e| match e {
        TemperError::Forbidden => rmcp::ErrorData::invalid_params(
            "Resource not found or not modifiable".to_string(),
            None,
        ),
        TemperError::NotFound(msg) => {
            rmcp::ErrorData::invalid_params(format!("Resource not found: {msg}"), None)
        }
        TemperError::BadRequest(msg) => rmcp::ErrorData::invalid_params(msg, None),
        other => {
            rmcp::ErrorData::internal_error(format!("Failed to update resource: {other}"), None)
        }
    })?;

    // Return enriched current state
    let row = substrate_read::show_select(
        pool,
        ProfileId::from(profile.id),
        ResourceId::from(input.id),
    )
    .await
    .map_err(|e| rmcp::ErrorData::internal_error(format!("Failed to get resource: {e}"), None))?;

    let enriched = enrich_resource(pool, profile.id, &row).await?;
    Ok(CallToolResult::success(vec![rmcp::model::Content::text(
        to_text(&enriched),
    )]))
}

pub async fn annotate_resource(
    svc: &TemperMcpService,
    input: AnnotateResourceInput,
) -> Result<CallToolResult, rmcp::ErrorData> {
    let profile = svc.require_profile().await?;
    let pool = &svc.api_state.pool;
    let profile_id = ProfileId::from(profile.id);
    let resource_id = ResourceId::from(input.id);

    let act = input
        .act
        .into_act_context()
        .map_err(|e| rmcp::ErrorData::invalid_params(e.to_string(), None))?;
    // Classify each source (http/https URL → Remote, else ref → Resource) with the shared resolver;
    // an unparseable value is a hard error, never a silent drop.
    let sources = input
        .sources
        .iter()
        .map(|s| temper_workflow::operations::resolve_provenance_source(s))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| {
            rmcp::ErrorData::invalid_params(format!("invalid sources value: {e}"), None)
        })?;
    let cmd = temper_workflow::operations::AnnotateResource {
        resource: resource_id,
        sources,
        content_block: input.content_block,
        act,
        origin: Surface::Mcp,
    };

    let backend = DbBackend::new(pool.clone(), profile_id);
    backend.annotate_resource(cmd).await.map_err(|e| match e {
        TemperError::Forbidden => rmcp::ErrorData::invalid_params(
            "Resource not found or not modifiable".to_string(),
            None,
        ),
        TemperError::NotFound(msg) => {
            rmcp::ErrorData::invalid_params(format!("Resource not found: {msg}"), None)
        }
        TemperError::BadRequest(msg) => rmcp::ErrorData::invalid_params(msg, None),
        other => {
            rmcp::ErrorData::internal_error(format!("Failed to annotate resource: {other}"), None)
        }
    })?;

    // Return the itemized provenance so the caller sees the rows it just recorded (the read that
    // proves the annotate landed), rather than re-fetching the unchanged resource body.
    let rows = substrate_read::resource_block_provenance_select(pool, profile_id, input.id)
        .await
        .map_err(|e| {
            rmcp::ErrorData::internal_error(format!("Failed to read provenance: {e}"), None)
        })?;
    Ok(CallToolResult::success(vec![rmcp::model::Content::text(
        to_text(&rows),
    )]))
}

pub async fn update_resource_meta(
    svc: &TemperMcpService,
    input: UpdateResourceMetaInput,
) -> Result<CallToolResult, rmcp::ErrorData> {
    let profile = svc.require_profile().await?;
    let pool = &svc.api_state.pool;
    let profile_id = ProfileId::from(profile.id);
    let resource_id = ResourceId::from(input.id);

    // Dispatch through the unified DbBackend write path. The translator's
    // meta-only branch runs resource_service::update with body=None, which
    // merges managed_meta / open_meta into the manifest, cascades identity
    // fields (doc_type / context), recomputes managed_hash / open_hash
    // server-side (Phase 5: caller-supplied hashes are no longer trusted),
    // emits the update_meta audit, and reconciles edges.
    let act = input
        .act
        .into_act_context()
        .map_err(|e| rmcp::ErrorData::invalid_params(e.to_string(), None))?;
    let cmd = temper_workflow::operations::UpdateResource {
        resource: resource_id,
        // Meta-only path is Property-only (Fork 2): identity changes go through
        // the full update_resource path, never here.
        title: None,
        slug: None,
        body: None,
        managed_meta: Some(input.managed_meta),
        open_meta: Some(input.open_meta),
        // Meta-only path is Property-only (Fork 2); goal links travel via update_resource.
        goal: None,
        move_to: None,
        context_ref: None,
        act,
        origin: Surface::Mcp,
    };

    let backend = DbBackend::new(pool.clone(), profile_id);
    backend.update_resource(cmd).await.map_err(|e| match e {
        TemperError::Forbidden => rmcp::ErrorData::invalid_params(
            "Resource not found or not modifiable".to_string(),
            None,
        ),
        TemperError::NotFound(msg) => {
            rmcp::ErrorData::invalid_params(format!("Resource not found: {msg}"), None)
        }
        TemperError::BadRequest(msg) => rmcp::ErrorData::invalid_params(msg, None),
        other => rmcp::ErrorData::internal_error(
            format!("Failed to update resource meta: {other}"),
            None,
        ),
    })?;

    let response = UpdateResourceMetaResponse {
        updated: true,
        id: input.id,
    };
    Ok(CallToolResult::success(vec![rmcp::model::Content::text(
        to_text(&response),
    )]))
}

pub async fn delete_resource(
    svc: &TemperMcpService,
    input: DeleteResourceInput,
) -> Result<CallToolResult, rmcp::ErrorData> {
    let profile = svc.require_profile().await?;
    let pool = &svc.api_state.pool;
    let profile_id = ProfileId::from(profile.id);

    let act = input
        .act
        .into_act_context()
        .map_err(|e| rmcp::ErrorData::invalid_params(e.to_string(), None))?;
    let cmd = temper_workflow::operations::DeleteResource {
        resource: ResourceId::from(input.id),
        // CLI-side concern; DbBackend ignores per spec (force=true is only
        // relevant when a CLI surface presents a confirmation prompt).
        force: false,
        act,
        origin: Surface::Mcp,
    };

    let backend = DbBackend::new(pool.clone(), profile_id);
    backend.delete_resource(cmd).await.map_err(|e| match e {
        TemperError::Forbidden => rmcp::ErrorData::invalid_params(
            "Resource not found or not modifiable".to_string(),
            None,
        ),
        TemperError::NotFound(msg) => {
            rmcp::ErrorData::invalid_params(format!("Resource not found: {msg}"), None)
        }
        other => {
            rmcp::ErrorData::internal_error(format!("Failed to delete resource: {other}"), None)
        }
    })?;

    let response = DeleteResourceResponse {
        deleted: true,
        id: input.id,
    };
    Ok(CallToolResult::success(vec![rmcp::model::Content::text(
        to_text(&response),
    )]))
}

// ── resource_grant / resource_revoke (service-direct) ──────────────
//
// Per-resource capability grants over `kb_access_grants`, mirroring the cogmap grant tool.
// Service-direct (admin events): gated by `is_system_admin OR can(...,'grant',...)` — which,
// via the owner-grant seam, includes the resource owner. NOT routed through DbBackend.

/// MCP input for resource_grant. `resource` is a ref; exactly one of `to_profile`/`to_team`
/// (raw UUID) names the principal. At least one capability must be set (read implied by write/grant).
#[derive(Debug, Deserialize, JsonSchema)]
pub struct ResourceGrantInput {
    /// The resource, by ref (UUID or `slug-<uuid>`).
    pub resource: String,
    /// Grant to this profile (UUID). Mutually exclusive with `to_team`.
    #[serde(default)]
    pub to_profile: Option<Uuid>,
    /// Grant to this team (UUID). Mutually exclusive with `to_profile`.
    #[serde(default)]
    pub to_team: Option<Uuid>,
    #[serde(default)]
    pub read: bool,
    #[serde(default)]
    pub write: bool,
    #[serde(default)]
    pub grant: bool,
}

/// MCP input for resource_revoke. `resource` is a ref; exactly one of `from_profile`/`from_team`.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct ResourceRevokeInput {
    pub resource: String,
    #[serde(default)]
    pub from_profile: Option<Uuid>,
    #[serde(default)]
    pub from_team: Option<Uuid>,
}

/// Resolve exactly one of (profile, team) into `(principal_table, principal_id)`.
fn resolve_grant_principal(
    profile: Option<Uuid>,
    team: Option<Uuid>,
) -> Result<(String, Uuid), rmcp::ErrorData> {
    match (profile, team) {
        (Some(p), None) => Ok(("kb_profiles".to_string(), p)),
        (None, Some(t)) => Ok(("kb_teams".to_string(), t)),
        (Some(_), Some(_)) => Err(rmcp::ErrorData::invalid_params(
            "supply exactly one principal, not both a profile and a team".to_string(),
            None,
        )),
        (None, None) => Err(rmcp::ErrorData::invalid_params(
            "no principal — supply exactly one of a profile or a team".to_string(),
            None,
        )),
    }
}

fn map_grant_error(context: &str, err: ApiError) -> rmcp::ErrorData {
    match err {
        ApiError::Forbidden => rmcp::ErrorData::invalid_params(
            format!("{context}: caller may not administer grants on this resource"),
            None,
        ),
        other => rmcp::ErrorData::internal_error(format!("{context} failed: {other}"), None),
    }
}

/// Grant a capability on a resource. SERVICE-DIRECT, gated by `is_system_admin OR can_grant OR
/// owner`. `read` forced on when `write`/`grant` is set.
pub async fn resource_grant(
    svc: &TemperMcpService,
    input: ResourceGrantInput,
) -> Result<CallToolResult, rmcp::ErrorData> {
    let profile = svc.require_profile().await?;
    let resource_id = uuid::Uuid::from(
        temper_workflow::operations::parse_ref(&input.resource)
            .map_err(|e| rmcp::ErrorData::invalid_params(format!("bad resource ref: {e}"), None))?,
    );
    let (principal_table, principal_id) = resolve_grant_principal(input.to_profile, input.to_team)?;
    if !(input.read || input.write || input.grant) {
        return Err(rmcp::ErrorData::invalid_params(
            "no capability selected — set at least one of read/write/grant".to_string(),
            None,
        ));
    }
    let req = GrantCapabilityRequest {
        subject_table: "kb_resources".to_string(),
        subject_id: resource_id,
        principal_table,
        principal_id,
        can_read: input.read || input.write || input.grant,
        can_write: input.write,
        can_delete: false,
        can_grant: input.grant,
    };
    let outcome =
        access_service::grant_capability(&svc.api_state.pool, ProfileId::from(profile.id), &req)
            .await
            .map_err(|e| map_grant_error("resource_grant", e))?;
    let text = serde_json::to_string_pretty(&outcome).unwrap_or_else(|_| "{}".to_string());
    Ok(CallToolResult::success(vec![rmcp::model::Content::text(
        text,
    )]))
}

/// Revoke a capability grant on a resource. SERVICE-DIRECT, admin/can_grant/owner-gated. No-op safe.
pub async fn resource_revoke(
    svc: &TemperMcpService,
    input: ResourceRevokeInput,
) -> Result<CallToolResult, rmcp::ErrorData> {
    let profile = svc.require_profile().await?;
    let resource_id = uuid::Uuid::from(
        temper_workflow::operations::parse_ref(&input.resource)
            .map_err(|e| rmcp::ErrorData::invalid_params(format!("bad resource ref: {e}"), None))?,
    );
    let (principal_table, principal_id) =
        resolve_grant_principal(input.from_profile, input.from_team)?;
    let req = RevokeCapabilityRequest {
        subject_table: "kb_resources".to_string(),
        subject_id: resource_id,
        principal_table,
        principal_id,
    };
    let outcome =
        access_service::revoke_capability(&svc.api_state.pool, ProfileId::from(profile.id), &req)
            .await
            .map_err(|e| map_grant_error("resource_revoke", e))?;
    let text = serde_json::to_string_pretty(&outcome).unwrap_or_else(|_| "{}".to_string());
    Ok(CallToolResult::success(vec![rmcp::model::Content::text(
        text,
    )]))
}

#[cfg(test)]
mod grant_tests {
    use super::*;

    #[test]
    fn resource_grant_input_deserializes() {
        let id = Uuid::now_v7();
        let raw = serde_json::json!({ "resource": "r", "to_team": id.to_string(), "write": true });
        let input: ResourceGrantInput = serde_json::from_value(raw).unwrap();
        assert_eq!(input.to_team, Some(id));
        assert!(input.write);
        assert!(!input.grant);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Gap 1 regression: `managed_meta` is a typed `ManagedMeta`, so an MCP
    /// client passing a real JSON object (not a string-encoded one)
    /// deserializes straight into the typed shape.
    #[test]
    fn create_resource_input_accepts_object_valued_managed_meta() {
        let raw = serde_json::json!({
            "context_ref": "@me/demo",
            "doc_type_name": "task",
            "title": "Demo Task",
            "managed_meta": { "temper-stage": "backlog", "temper-mode": "build" },
        });
        let input: CreateResourceInput =
            serde_json::from_value(raw).expect("object-valued managed_meta must deserialize");
        let managed = input.managed_meta.expect("managed_meta present");
        assert_eq!(managed.stage.as_deref(), Some("backlog"));
        assert_eq!(managed.mode.as_deref(), Some("build"));
    }

    /// `managed_meta` is a closed vocabulary: an unknown key under it must be
    /// rejected at input parse, not silently absorbed. Proves the closed
    /// `ManagedMeta` type reaches the MCP tool boundary.
    #[test]
    fn create_input_rejects_unknown_managed_key() {
        let raw = serde_json::json!({
            "context_ref": "@me/temper",
            "doc_type_name": "task",
            "title": "T",
            "managed_meta": { "my-tag": "x" },
        });
        let err = serde_json::from_value::<CreateResourceInput>(raw).unwrap_err();
        assert!(
            err.to_string().contains("my-tag"),
            "unknown managed key must be rejected at input parse, got: {err}"
        );
    }

    #[test]
    fn update_resource_input_accepts_object_valued_managed_meta() {
        let raw = serde_json::json!({
            "id": "00000000-0000-0000-0000-000000000000",
            "managed_meta": { "temper-stage": "done" },
        });
        let input: UpdateResourceInput =
            serde_json::from_value(raw).expect("object-valued managed_meta must deserialize");
        assert_eq!(
            input
                .managed_meta
                .expect("managed_meta present")
                .stage
                .as_deref(),
            Some("done"),
        );
    }

    /// The non-authored MCP write inputs (update / delete) accept the same flattened act fields, so
    /// an agent can correlate + author an update/delete the same way it does a create.
    #[test]
    fn update_resource_input_accepts_act_authorship_fields() {
        let raw = serde_json::json!({
            "id": "00000000-0000-0000-0000-000000000000",
            "open_meta": { "reviewed_by": "qa" },
            "invocation_id": "019f0e28-1750-7490-919f-5e51c92c8391",
            "reasoning": "applying review outcome",
            "confidence": "confident",
        });
        let input: UpdateResourceInput =
            serde_json::from_value(raw).expect("flattened act fields must deserialize");
        assert!(input.act.invocation_id.is_some());
        assert_eq!(
            input.act.confidence,
            Some(temper_core::types::ConfidenceBand::Confident)
        );
        assert!(!input.act.into_act_context().expect("assembles").is_empty());
    }

    #[test]
    fn delete_resource_input_accepts_act_authorship_fields() {
        let raw = serde_json::json!({
            "id": "00000000-0000-0000-0000-000000000000",
            "reasoning": "tombstoning the duplicate",
            "confidence": "tentative",
        });
        let input: DeleteResourceInput =
            serde_json::from_value(raw).expect("flattened act fields must deserialize");
        assert_eq!(
            input.act.reasoning.as_deref(),
            Some("tombstoning the duplicate")
        );
        assert!(!input.act.into_act_context().expect("assembles").is_empty());
    }

    /// Chunk B: the flattened [`ActInput`] discrete fields deserialize as top-level keys on the
    /// MCP input (invocation_id + the authorship fields), and assemble into an `ActContext`.
    #[test]
    fn create_resource_input_accepts_act_authorship_fields() {
        let raw = serde_json::json!({
            "context_ref": "@me/demo",
            "doc_type_name": "task",
            "title": "Demo Task",
            "invocation_id": "019f0e28-1750-7490-919f-5e51c92c8391",
            "reasoning": "seeding the demo corpus",
            "confidence": "probable",
            "persona": "steward",
        });
        let input: CreateResourceInput =
            serde_json::from_value(raw).expect("flattened act fields must deserialize");
        assert_eq!(
            input.act.confidence,
            Some(temper_core::types::ConfidenceBand::Probable)
        );
        assert_eq!(
            input.act.reasoning.as_deref(),
            Some("seeding the demo corpus")
        );
        assert_eq!(input.act.persona.as_deref(), Some("steward"));
        assert!(input.act.invocation_id.is_some(), "invocation_id present");
        // And it assembles into a non-empty ActContext.
        let ctx = input.act.into_act_context().expect("assembles");
        assert!(!ctx.is_empty());
    }

    /// Chunk B: the flattened authorship/correlation fields must inline as a string enum
    /// (`confidence`) and a string-uuid (`invocation_id`) in the generated schema — a `$ref` into
    /// `$defs` reaches the Anthropic tool-use layer with no type signal and comes back as `null`
    /// (the same bug fixed for EdgeKind/Polarity). Generated via the exact rmcp runtime path.
    #[test]
    fn create_resource_input_schema_inlines_act_fields() {
        let generator = schemars::generate::SchemaSettings::draft2020_12().into_generator();
        let schema = serde_json::to_value(generator.into_root_schema_for::<CreateResourceInput>())
            .expect("schema serializes");

        // confidence: inline string enum (the trailing `null` is the field's Option-ness).
        let confidence = &schema["properties"]["confidence"];
        assert!(
            confidence.get("$ref").is_none(),
            "confidence must inline, not $ref: {confidence}"
        );
        let variants: Vec<&str> = confidence
            .get("enum")
            .and_then(|e| e.as_array())
            .expect("confidence carries inline enum variants")
            .iter()
            .filter_map(|v| v.as_str())
            .collect();
        assert_eq!(variants, ["tentative", "probable", "confident"]);

        // invocation_id: inline string-uuid, not a $ref into $defs.
        let invocation = &schema["properties"]["invocation_id"];
        assert!(
            invocation.get("$ref").is_none(),
            "invocation_id must inline, not $ref: {invocation}"
        );
        assert_eq!(
            invocation.get("format").and_then(|f| f.as_str()),
            Some("uuid"),
            "invocation_id inlines as a uuid-format string: {invocation}"
        );

        // correlation_id: same contract. A caller-minted act-grain thread is useless if the tool
        // layer sees `null` where a uuid should be.
        let correlation = &schema["properties"]["correlation_id"];
        assert!(
            correlation.get("$ref").is_none(),
            "correlation_id must inline, not $ref: {correlation}"
        );
        assert_eq!(
            correlation.get("format").and_then(|f| f.as_str()),
            Some("uuid"),
            "correlation_id inlines as a uuid-format string: {correlation}"
        );
        assert!(
            !schema.to_string().contains("CorrelationId"),
            "CorrelationId must not survive as a named $defs entry: {schema}"
        );
    }

    /// Gap 1: the generated JsonSchema must describe `managed_meta` as the
    /// concrete `ManagedMeta` object rather than free-form JSON — that
    /// concreteness is what stops MCP clients from string-encoding the field.
    #[test]
    fn create_resource_input_managed_meta_schema_is_concrete() {
        let schema = schemars::schema_for!(CreateResourceInput);
        let json = serde_json::to_string(&schema).expect("schema serializes");
        assert!(
            json.contains("ManagedMeta"),
            "managed_meta should reference the typed ManagedMeta schema: {json}"
        );
    }
}

#[cfg(test)]
mod build_enriched_tests {
    use super::*;

    fn sample_row() -> temper_workflow::types::resource::ResourceRow {
        use temper_core::types::ids::{ContextId, ProfileId, ResourceId};
        use temper_workflow::types::resource::ResourceRow;
        let nil = uuid::Uuid::nil();
        ResourceRow {
            id: ResourceId::from(uuid::Uuid::now_v7()),
            kb_context_id: Some(ContextId::from(nil)),
            origin_uri: "temper://fixture/task-doc".to_string(),
            title: "Wire the widget".to_string(),
            originator_profile_id: ProfileId::from(nil),
            owner_profile_id: ProfileId::from(nil),
            is_active: true,
            created: chrono::Utc::now(),
            updated: chrono::Utc::now(),
            context_name: Some("temper".to_string()),
            doc_type_name: "task".to_string(),
            owner_handle: "@me".to_string(),
            context_slug: Some("temper".to_string()),
            context_owner_ref: Some("@me".to_string()),
            cogmap_id: None,
            cogmap_name: None,
            stage: Some("in-progress".to_string()),
            seq: None,
            mode: None,
            effort: None,
            body_hash: None,
            ingest_state: Some(temper_workflow::types::IngestState::Complete),
        }
    }

    #[test]
    fn build_enriched_uses_row_names_and_decorated_ref() {
        let row = sample_row();
        let e = build_enriched(&row, None, None, EmbeddingStatus::Ready);
        assert_eq!(e.context_name, "temper");
        assert_eq!(e.embedding_status, EmbeddingStatus::Ready);
        assert_eq!(e.doc_type_name, "task");
        assert_eq!(
            e.r#ref,
            temper_workflow::operations::decorated_ref(&row.title, row.id)
        );
    }
}

#[cfg(test)]
mod fields_projection_tests {
    use super::*;

    #[test]
    fn get_resource_input_is_ref_only() {
        let raw = serde_json::json!({ "id": "my-task-019e84ab-26ba-7560-9d34-c60d74a9fbe2" });
        let input: GetResourceInput = serde_json::from_value(raw).unwrap();
        assert_eq!(input.id, "my-task-019e84ab-26ba-7560-9d34-c60d74a9fbe2");
    }

    #[test]
    fn get_resource_input_accepts_fields() {
        // Compile-time check that GetResourceInput carries the field.
        let _input = GetResourceInput {
            id: "x".to_string(),
            include_content: Some(false),
            fields: Some(vec!["managed_meta".to_string()]),
        };
    }

    #[test]
    fn enriched_resource_filtered_by_fields_preserves_id_and_managed_meta() {
        // Stub an EnrichedResource value
        let value = serde_json::json!({
            "id": "11111111-1111-1111-1111-111111111111",
            "title": "Test",
            "slug": "test",
            "context_name": "temper",
            "doc_type_name": "task",
            "owner": "@me",
            "origin_uri": "",
            "is_active": true,
            "created": "2026-05-27T00:00:00Z",
            "updated": "2026-05-27T00:00:00Z",
            "managed_meta": {"stage": "in-progress"},
            "open_meta": {"tags": []}
        });
        let filtered = temper_core::projection::apply_top_level_filter(
            value,
            &["managed_meta".to_string()],
            "id",
        )
        .expect("filter");
        assert!(filtered.get("id").is_some(), "anchor id missing");
        assert!(
            filtered.get("managed_meta").is_some(),
            "managed_meta missing"
        );
        assert!(filtered.get("title").is_none(), "title should be dropped");
        assert!(
            filtered.get("open_meta").is_none(),
            "open_meta should be dropped"
        );
    }

    #[test]
    fn list_resources_input_accepts_fields() {
        // Compile-time check that ListResourcesInput grows the fields field.
        let _input = ListResourcesInput {
            context_ref: None,
            doc_type_name: None,
            goal: None,
            limit: None,
            offset: None,
            fields: Some(vec!["managed_meta".to_string()]),
        };
    }

    #[test]
    fn enriched_resource_carries_decorated_ref() {
        let id = uuid::Uuid::parse_str("019e84ab-26ba-7560-9d34-c60d74a9fbe2").unwrap();
        let got = temper_workflow::operations::decorated_ref(
            "My Task",
            temper_core::types::ids::ResourceId(id),
        );
        assert_eq!(got, "my-task-019e84ab-26ba-7560-9d34-c60d74a9fbe2");
    }

    #[test]
    fn enriched_resource_array_filtered_by_fields() {
        let value = serde_json::json!([
            {
                "id": "11111111-1111-1111-1111-111111111111",
                "title": "A",
                "managed_meta": {"stage": "done"}
            },
            {
                "id": "22222222-2222-2222-2222-222222222222",
                "title": "B",
                "managed_meta": {"stage": "in-progress"}
            }
        ]);
        let filtered = temper_core::projection::apply_top_level_filter(
            value,
            &["managed_meta".to_string()],
            "id",
        )
        .expect("filter");
        let arr = filtered.as_array().expect("array");
        assert_eq!(arr.len(), 2);
        for row in arr {
            assert!(row.get("id").is_some());
            assert!(row.get("managed_meta").is_some());
            assert!(row.get("title").is_none());
        }
    }
}
