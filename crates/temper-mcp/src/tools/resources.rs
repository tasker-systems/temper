//! Resource tools — unified CRUD with name-based resolution and optional content.

use rmcp::model::CallToolResult;
use schemars::JsonSchema;
use serde::Deserialize;
use uuid::Uuid;

use temper_api::backend::{substrate_read, DbBackend};
use temper_api::services::context_service::resolve_context_ref;
use temper_core::context_ref::parse_context_ref;
use temper_core::error::TemperError;
use temper_core::types::authorship::ActInput;
use temper_core::types::home::HomeAnchor;
use temper_core::types::ids::{ProfileId, ResourceId};
use temper_workflow::operations::{Backend, BodyUpdate, CreateResource, Surface};
use temper_workflow::types::managed_meta::ManagedMeta;

use crate::service::TemperMcpService;

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
    /// Optional URL-friendly slug.
    pub slug: Option<String>,
    /// Optional origin URI. Defaults to `mcp://agent/{uuid}`.
    pub origin_uri: Option<String>,
    /// Optional owner (defaults to @me). Reserved for future team scoping.
    pub owner: Option<String>,
    /// Managed (temper-*) frontmatter. Typed: the schema covers every key
    /// temper governs and extras round-trip through `ManagedMeta::extra`.
    /// A concrete object schema (rather than free-form JSON) keeps MCP
    /// clients from string-encoding nested objects.
    #[serde(default)]
    pub managed_meta: Option<ManagedMeta>,
    /// Open frontmatter (user-owned fields) as JSON.
    #[serde(default)]
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

/// MCP input for list_resources.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct ListResourcesInput {
    /// Filter by context ref (UUID or @owner/slug). Bare context names are rejected.
    pub context_ref: Option<String>,
    /// Filter by doc type name (e.g. "task", "research").
    pub doc_type_name: Option<String>,
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
    /// New slug.
    pub slug: Option<String>,
    /// New markdown content. Replaces existing content and triggers
    /// re-processing.
    pub content: Option<String>,
    /// Managed (temper-*) frontmatter. Typed: the schema covers every key
    /// temper governs and extras round-trip through `ManagedMeta::extra`.
    /// A concrete object schema (rather than free-form JSON) keeps MCP
    /// clients from string-encoding nested objects.
    #[serde(default)]
    pub managed_meta: Option<ManagedMeta>,
    /// Open frontmatter (user-owned fields) as JSON.
    #[serde(default)]
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
/// `managed_meta` is typed: agents get a schema-validated shape for
/// the fields temper knows about, and the `extra` flatten bucket on
/// `ManagedMeta` accepts any additional keys (doc-type-schema fields
/// like `date`, plus forward-compat unknowns) without dropping them.
/// `open_meta` stays a free-form JSON value by design — the open tier
/// is intentionally untyped.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct UpdateResourceMetaInput {
    /// UUID of the resource to update.
    pub id: Uuid,
    /// New managed (temper-*) frontmatter. Typed fields cover every
    /// key temper governs; extras round-trip through `ManagedMeta::extra`.
    pub managed_meta: ManagedMeta,
    /// New open (user-defined) frontmatter as JSON.
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
) -> EnrichedResource {
    EnrichedResource {
        id: row.id.into(),
        title: row.title.clone(),
        slug: None,
        context_name: row
            .context_name
            .clone()
            .or_else(|| row.cogmap_name.clone())
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

    let mut enriched = Vec::with_capacity(rows.len());
    for row in rows {
        let (managed_meta, open_meta) = meta
            .remove(&row.id)
            .map(|m| (m.managed_meta, m.open_meta))
            .unwrap_or((None, None));
        enriched.push(build_enriched(row, managed_meta, open_meta));
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

// ── Tool handlers ──────────────────────────────────────────────────

pub async fn create_resource(
    svc: &TemperMcpService,
    input: CreateResourceInput,
) -> Result<CallToolResult, rmcp::ErrorData> {
    let profile = svc.require_profile().await?;
    let pool = &svc.api_state.pool;
    let profile_id = ProfileId::from(profile.id);

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
            // Auth before writes: producer gate (seam → team-cogmap membership).
            let ok: bool = sqlx::query_scalar!(
                "SELECT cogmap_authorable_by_profile($1, $2)",
                *profile_id,
                map
            )
            .fetch_one(pool)
            .await
            .map_err(|e| rmcp::ErrorData::internal_error(e.to_string(), None))?
            .unwrap_or(false);
            if !ok {
                return Err(rmcp::ErrorData::invalid_params(
                    "not authorized to author in this cognitive map".to_string(),
                    None,
                ));
            }
            HomeAnchor::Cogmap(temper_core::types::ids::CogmapId::from(map))
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

    // Build slug from title if not provided
    let slug = input.slug.unwrap_or_else(|| {
        input
            .title
            .to_lowercase()
            .replace(|c: char| !c.is_alphanumeric() && c != '-', "-")
            .trim_matches('-')
            .to_owned()
    });

    let origin_uri = input
        .origin_uri
        .unwrap_or_else(|| format!("mcp://agent/{}", Uuid::new_v4()));

    let content = input.content.unwrap_or_default();

    // Inject canonical temper-title / temper-slug into managed_meta JSONB so
    // the local canonical form matches what the server will hash. Symmetric
    // with the CLI send-side wiring in build_ingest_payload (Phase 5 Task 3).
    let mut managed_meta_value = serde_json::to_value(input.managed_meta.unwrap_or_default())
        .map_err(|e| {
            rmcp::ErrorData::internal_error(format!("managed_meta serialize: {e}"), None)
        })?;
    temper_workflow::operations::ensure_managed_identity_keys(
        &mut managed_meta_value,
        &input.title,
        Some(&slug),
    );

    // Dispatch through DbBackend so MCP shares the unified write path with
    // HTTP. The send-side ensure_managed_identity_keys above ran on the
    // JSONB form; deserialize back to the typed ManagedMeta the cmd carries
    // (extras bucket preserves unknown keys; serde renames re-emit canonical
    // temper-* keys on round-trip).
    let managed_meta: ManagedMeta = serde_json::from_value(managed_meta_value)
        .map_err(|e| rmcp::ErrorData::invalid_params(format!("invalid managed_meta: {e}"), None))?;

    let body = if content.is_empty() {
        None
    } else {
        Some(BodyUpdate::new(content))
    };

    // Assemble the per-act correlation + authorship from the flattened discrete fields. The
    // shared assembler enforces "confidence required iff authorship supplied"; map its
    // BadRequest to invalid_params.
    let act = input
        .act
        .into_act_context()
        .map_err(|e| rmcp::ErrorData::invalid_params(e.to_string(), None))?;

    let cmd = CreateResource {
        slug,
        doctype: input.doc_type_name,
        home,
        title: input.title,
        body,
        managed_meta,
        open_meta: input.open_meta,
        origin_uri: Some(origin_uri),
        chunks_packed: None,
        content_hash: None,
        act,
        origin: Surface::Mcp,
    };

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

    let row = substrate_read::show_select(pool, ProfileId::from(profile.id), id)
        .await
        .map_err(|e| {
            rmcp::ErrorData::internal_error(format!("Failed to get resource: {e}"), None)
        })?;

    let meta = substrate_read::get_meta_select(pool, ProfileId::from(profile.id), row.id)
        .await
        .map_err(|e| rmcp::ErrorData::internal_error(format!("Failed to get meta: {e}"), None))?;

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

    let enriched = build_enriched(&row, meta.managed_meta, meta.open_meta);

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

pub async fn list_resources(
    svc: &TemperMcpService,
    input: ListResourcesInput,
) -> Result<CallToolResult, rmcp::ErrorData> {
    let profile = svc.require_profile().await?;
    let pool = &svc.api_state.pool;

    // Build list params — context_ref is resolved server-side by filtered_visible_page;
    // bare context names are rejected there (spec Decision 1).
    let params = temper_workflow::types::resource::ResourceListParams {
        context_ref: input.context_ref.clone(),
        doc_type_name: input.doc_type_name.clone(),
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
            temper_api::error::ApiError::BadRequest(msg) => {
                rmcp::ErrorData::invalid_params(msg, None)
            }
            temper_api::error::ApiError::NotFound => rmcp::ErrorData::invalid_params(
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

    // Send-side canonical-key injection (Phase 5 symmetric defense). When the
    // caller is also touching title or slug, fetch existing.title for the
    // canonical-key fill so the wire payload's temper-title / temper-slug
    // match what the receive-side will write. Pure meta-only updates skip
    // the fetch — the backend update's receive-side ensure call fills
    // canonical keys from the stored title/slug for us.
    let mut managed_meta_value = serde_json::to_value(input.managed_meta.unwrap_or_default())
        .map_err(|e| {
            rmcp::ErrorData::internal_error(format!("managed_meta serialize: {e}"), None)
        })?;
    if input.title.is_some() || input.slug.is_some() || input.content.is_some() {
        let existing = substrate_read::show_select(
            pool,
            ProfileId::from(profile.id),
            ResourceId::from(input.id),
        )
        .await
        .map_err(|e| {
            rmcp::ErrorData::internal_error(format!("Failed to get resource: {e}"), None)
        })?;
        let title = input.title.clone().unwrap_or(existing.title);
        // slug is §7-dissolved from ResourceRow; derive from effective title when the
        // caller hasn't supplied one explicitly.
        let slug = input
            .slug
            .clone()
            .unwrap_or_else(|| temper_workflow::operations::sluggify(&title));
        temper_workflow::operations::ensure_managed_identity_keys(
            &mut managed_meta_value,
            &title,
            Some(slug.as_str()),
        );
    }

    // Build the typed cmd. Mirror title/slug onto ManagedMeta so the
    // translator's manifest-merge path picks them up from cmd.managed_meta
    // alongside any caller-supplied managed_meta keys.
    let mut managed_meta: ManagedMeta = serde_json::from_value(managed_meta_value)
        .map_err(|e| rmcp::ErrorData::invalid_params(format!("invalid managed_meta: {e}"), None))?;
    if input.title.is_some() {
        managed_meta.title = input.title.clone();
    }
    if input.slug.is_some() {
        managed_meta.slug = input.slug.clone();
    }

    let act = input
        .act
        .into_act_context()
        .map_err(|e| rmcp::ErrorData::invalid_params(e.to_string(), None))?;
    let cmd = temper_workflow::operations::UpdateResource {
        resource: resource_id,
        body: input.content.map(BodyUpdate::new),
        managed_meta: Some(managed_meta),
        open_meta: input.open_meta,
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
        body: None,
        managed_meta: Some(input.managed_meta),
        open_meta: Some(input.open_meta),
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
        }
    }

    #[test]
    fn build_enriched_uses_row_names_and_decorated_ref() {
        let row = sample_row();
        let e = build_enriched(&row, None, None);
        assert_eq!(e.context_name, "temper");
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
