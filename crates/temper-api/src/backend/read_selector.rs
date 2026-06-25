//! Substrate read dispatcher — the service-direct read paths (list / show / get_content / get_meta /
//! search + the MCP enrichment list/meta-batch) over the one schema.
//!
//! These reads bypass the `Backend` trait by design (the trait projections are lossy and don't cover
//! meta/body/content); they resolve against `temper_next::readback`, producing native `ResourceRow`s
//! (real timestamps, name-only doc type, no fabricated fields) via `native_resource_row`. Visibility
//! is scoped to the caller's profile (WS2) — the readbacks gate through `resources_visible_to`. SQL
//! is unqualified against the one schema (the connection carries the search_path).
//!
//! `list`/`list_meta` filter (context_name/doc_type_name/stage/owner/`q`-title), sort, and paginate the
//! visible set in SQL (`filtered_visible_page`), reconstructing only the page; the enrichment path
//! (`list_enriched`/MCP) filters by name in SQL via `readback::enriched_list`. Full-text/vector `q` on
//! the list endpoint is search's job (a named deferral) — list `q` is a trivial title `ILIKE`.

use std::collections::HashMap;

use sqlx::{PgPool, Row};
use uuid::Uuid;

use crate::backend::db_backend::{map_readback_err, native_resource_row};
use crate::error::{ApiError, ApiResult};
use crate::services::resource_service::{ResourceListParams, ResourceListResponse};
use temper_core::error::TemperError;
use temper_core::types::api::{SearchParams, UnifiedSearchResultRow};
use temper_core::types::ids::{ContextId, ProfileId, ResourceId};
use temper_core::types::managed_meta::{
    ManagedMeta, ResourceMetaListResponse, ResourceMetaResponse,
};
use temper_core::types::resource::{
    ContentResponse, ResourceFacets, ResourceRow, ResourceSortField, SortOrder,
};
use temper_next::readback;

fn api_err(e: impl std::fmt::Display) -> ApiError {
    ApiError::from(TemperError::Api(e.to_string()))
}

/// One page of the filtered, visible resource set: the page's substrate ids (already
/// sorted + paginated), the FILTERED total (before limit/offset), and the doc_type
/// histogram over the filtered set (`ResourceFacets` = "current filter set").
struct VisiblePage {
    page_ids: Vec<Uuid>,
    total: i64,
    facets: HashMap<String, i64>,
}

/// The ORDER BY column expression for a sort field. Enum-controlled (no caller string
/// reaches SQL) so it is injection-safe to interpolate. Columns ground against the
/// substrate: `kb_resources` (updated/created/title), `kb_contexts.name`, and the
/// `kb_properties` workflow keys (`temper-stage`/`temper-seq`/`doc_type`).
fn sort_column_sql(field: ResourceSortField) -> &'static str {
    match field {
        ResourceSortField::Updated => "r.updated",
        ResourceSortField::Created => "r.created",
        ResourceSortField::Title => "r.title",
        ResourceSortField::Stage => "st.property_value #>> '{}'",
        ResourceSortField::Seq => "(sq.property_value #>> '{}')::bigint",
        ResourceSortField::ContextName => "c.name",
        ResourceSortField::DocTypeName => "dt.property_value #>> '{}'",
    }
}

/// Resolve the visible set, apply the `ResourceListParams` filters (context_name /
/// doc_type_name / stage / owner / `q` title-match) + sort + pagination IN SQL, and
/// return only the page's ids (so the caller reconstructs the page, not every visible
/// row — this also fixes the prior all-rows N+1).
///
/// `owner`: `@me` resolves to the caller's profile; any other value matches the owner
/// profile's `handle` (per `graph.rs`'s handle convention). `q` is a trivial title
/// `ILIKE` (full text/vector `q` is search's job — a named deferral). Dynamic ORDER BY
/// is built from the enum; the WHERE binds Option params via the `($N IS NULL OR …)`
/// idiom, so this is the documented runtime-`query` exception (dynamic ORDER clause),
/// not a static macro.
async fn filtered_visible_page(
    pool: &PgPool,
    profile_id: Uuid,
    params: &ResourceListParams,
) -> ApiResult<VisiblePage> {
    let owner_self: Option<Uuid> = match params.owner.as_deref() {
        Some("@me") => Some(profile_id),
        _ => None,
    };
    let owner_handle: Option<&str> = match params.owner.as_deref() {
        Some(h) if h != "@me" => Some(h),
        _ => None,
    };
    let sort = params.sort.unwrap_or_default();
    let dir = match params.order.unwrap_or_default() {
        SortOrder::Asc => "ASC",
        SortOrder::Desc => "DESC",
    };

    // INNER JOIN dt (every resource carries exactly one `doc_type` property, as in
    // `readback::reconstruct`); LEFT JOIN the optional workflow keys used by filters/sort.
    let sql = format!(
        "SELECT r.id AS id, dt.property_value #>> '{{}}' AS doc_type_name
           FROM kb_resources r
           JOIN resources_visible_to($1) v ON v.resource_id = r.id
           JOIN kb_resource_homes h ON h.resource_id = r.id
           JOIN kb_contexts c
             ON c.id = h.anchor_id AND h.anchor_table = 'kb_contexts'
           JOIN kb_profiles p ON p.id = h.owner_profile_id
           JOIN kb_properties dt
             ON dt.owner_table = 'kb_resources' AND dt.owner_id = r.id
            AND dt.property_key = 'doc_type' AND NOT dt.is_folded
           LEFT JOIN kb_properties st
             ON st.owner_table = 'kb_resources' AND st.owner_id = r.id
            AND st.property_key = 'temper-stage' AND NOT st.is_folded
           LEFT JOIN kb_properties sq
             ON sq.owner_table = 'kb_resources' AND sq.owner_id = r.id
            AND sq.property_key = 'temper-seq' AND NOT sq.is_folded
          WHERE r.is_active
            AND ($2::text IS NULL OR c.name = $2)
            AND ($3::text IS NULL OR dt.property_value #>> '{{}}' = $3)
            AND ($4::text IS NULL OR st.property_value #>> '{{}}' = $4)
            AND ($5::uuid IS NULL OR h.owner_profile_id = $5)
            AND ($6::text IS NULL OR p.handle = $6)
            AND ($7::text IS NULL OR r.title ILIKE '%' || $7 || '%')
          ORDER BY {sort_col} {dir}, r.id ASC",
        sort_col = sort_column_sql(sort),
    );

    let rows = sqlx::query(&sql)
        .bind(profile_id)
        .bind(params.context_name.as_deref())
        .bind(params.doc_type_name.as_deref())
        .bind(params.stage.as_deref())
        .bind(owner_self)
        .bind(owner_handle)
        .bind(params.q.as_deref())
        .fetch_all(pool)
        .await
        .map_err(api_err)?;

    let total = rows.len() as i64;
    let mut facets: HashMap<String, i64> = HashMap::new();
    let mut all_ids: Vec<Uuid> = Vec::with_capacity(rows.len());
    for row in &rows {
        let id: Uuid = row.get("id");
        if let Some(dt) = row.get::<Option<String>, _>("doc_type_name") {
            *facets.entry(dt).or_insert(0) += 1;
        }
        all_ids.push(id);
    }

    let offset = params.offset.unwrap_or(0).max(0) as usize;
    let page_ids: Vec<Uuid> = match params.limit {
        Some(limit) if limit >= 0 => all_ids
            .into_iter()
            .skip(offset)
            .take(limit as usize)
            .collect(),
        _ => all_ids.into_iter().skip(offset).collect(),
    };

    Ok(VisiblePage {
        page_ids,
        total,
        facets,
    })
}

/// `list` — the resources VISIBLE to the principal (WS2 — `resources_visible_to`), filtered + sorted +
/// paginated per `ResourceListParams`, each reconstructed to a full `ResourceRow`. The filter/sort/page
/// happen in SQL (`filtered_visible_page`); only the page's ids are reconstructed (no all-rows N+1).
/// `total` = the FILTERED count (before limit/offset); `facets.doc_type` = the doctype histogram over the
/// filtered set.
pub async fn list_select(
    pool: &PgPool,
    profile_id: Uuid,
    params: ResourceListParams,
) -> ApiResult<ResourceListResponse> {
    let page = filtered_visible_page(pool, profile_id, &params).await?;
    let mut rows: Vec<ResourceRow> = Vec::with_capacity(page.page_ids.len());
    for new_id in page.page_ids {
        rows.push(native_resource_row(pool, profile_id, new_id).await?);
    }
    Ok(ResourceListResponse {
        rows,
        total: page.total,
        facets: ResourceFacets {
            doc_type: page.facets,
        },
    })
}

/// `show` — full resource row by id (§9 invariant floor) via the shared `reconstruct_resource_row`. The
/// inbound id IS the substrate id. Visibility is gated inside `reconstruct_resource_row` (WS2); the typed
/// `ReadbackError` is split by `map_readback_err` (not-visible → NotFound/404, fault → Api/500).
pub async fn show_select(pool: &PgPool, profile_id: Uuid, id: Uuid) -> ApiResult<ResourceRow> {
    native_resource_row(pool, profile_id, id)
        .await
        .map_err(ApiError::from)
}

/// `get_content` — reconstructed markdown body (§9 body floor). `managed_meta`/`open_meta` are `None`
/// (the meta tier is `get_meta`).
pub async fn get_content_select(
    pool: &PgPool,
    profile_id: Uuid,
    resource_id: Uuid,
) -> ApiResult<ContentResponse> {
    let markdown = readback::body(pool, profile_id, resource_id)
        .await
        .map_err(|e| ApiError::from(map_readback_err(e)))?;
    Ok(ContentResponse {
        resource_id: ResourceId::from(resource_id),
        markdown,
        managed_meta: None,
        open_meta: None,
    })
}

/// `get_meta` — managed/open frontmatter for one resource (`readback::meta`, the §7 inverse fate).
/// `managed_hash`/`open_hash` are §7-dissolved (emitted empty; §9 non-invariants).
pub async fn get_meta_select(
    pool: &PgPool,
    profile_id: ProfileId,
    resource_id: ResourceId,
) -> ApiResult<ResourceMetaResponse> {
    let new_id = Uuid::from(resource_id);
    let rb = readback::meta(pool, Uuid::from(profile_id), new_id)
        .await
        .map_err(|e| ApiError::from(map_readback_err(e)))?;
    let managed: ManagedMeta =
        serde_json::from_value(serde_json::Value::Object(rb.managed)).map_err(api_err)?;
    Ok(ResourceMetaResponse {
        resource_id: ResourceId::from(new_id),
        managed_meta: Some(managed),
        open_meta: Some(serde_json::Value::Object(rb.open)),
        managed_hash: String::new(),
        open_hash: String::new(),
    })
}

/// `list_meta` — the `?meta_only=true` projection. Same WS2-scoped, filtered + sorted + paginated set as
/// `list` (`filtered_visible_page`); each page id maps to a `ResourceMetaResponse` via `get_meta_select`
/// (the §7 meta tier). `total`/`facets` mirror `list` (the FILTERED set).
pub async fn list_meta_select(
    pool: &PgPool,
    profile_id: Uuid,
    params: ResourceListParams,
) -> ApiResult<ResourceMetaListResponse> {
    let page = filtered_visible_page(pool, profile_id, &params).await?;
    let mut out = Vec::with_capacity(page.page_ids.len());
    for new_id in page.page_ids {
        out.push(
            get_meta_select(pool, ProfileId::from(profile_id), ResourceId::from(new_id)).await?,
        );
    }
    Ok(ResourceMetaListResponse {
        rows: out,
        total: page.total,
        facets: ResourceFacets {
            doc_type: page.facets,
        },
    })
}

/// `get_meta_batch` — the batched meta tier for many ids (the MCP `enrich_resources` path). Loops
/// `get_meta` per id (each WS2-gated); a not-visible id is OMITTED from the map (parity with the prior
/// batch's "absent = no meta"), while a genuine fault propagates.
pub async fn get_meta_batch_select(
    pool: &PgPool,
    profile_id: Uuid,
    ids: &[ResourceId],
) -> ApiResult<HashMap<ResourceId, ResourceMetaResponse>> {
    let mut map = HashMap::with_capacity(ids.len());
    for id in ids {
        match get_meta_select(pool, ProfileId::from(profile_id), *id).await {
            Ok(resp) => {
                map.insert(*id, resp);
            }
            // A not-visible id is simply absent from the map; a genuine fault still propagates.
            Err(ApiError::NotFound) => {}
            Err(e) => return Err(e),
        }
    }
    Ok(map)
}

/// `search` — vector when an embedding is supplied, else FTS over the text query (§9 search floor). The
/// matching SET is the invariant; scores are not (emitted 0.0). Each match reconstructs to a full row.
pub async fn search_select(
    pool: &PgPool,
    profile_id: Uuid,
    params: SearchParams,
) -> ApiResult<Vec<UnifiedSearchResultRow>> {
    // The search readbacks JOIN `resources_visible_to(principal)`, so the result set is already scoped.
    let (ids, origin) = if let Some(embedding) = params.embedding.as_ref() {
        (
            readback::vector_search(pool, profile_id, embedding)
                .await
                .map_err(api_err)?,
            "vector",
        )
    } else if let Some(query) = params.query.as_ref() {
        (
            readback::fts_search(pool, profile_id, query)
                .await
                .map_err(api_err)?,
            "fts",
        )
    } else {
        (Vec::new(), "fts")
    };

    let mut hits = Vec::with_capacity(ids.len());
    for new_id in ids {
        let row = native_resource_row(pool, profile_id, new_id).await?;
        hits.push(UnifiedSearchResultRow {
            resource_id: new_id,
            title: row.title,
            slug: String::new(),
            kb_uri: row.origin_uri.clone(),
            origin_uri: row.origin_uri,
            context: Some(row.context_name),
            doc_type: row.doc_type_name,
            fts_score: 0.0,
            vector_score: 0.0,
            combined_score: 0.0,
            origin: origin.to_string(),
        });
    }
    Ok(hits)
}

/// `list_resources` enrichment — full rows + their managed/open meta, filtered by `context_name` +
/// `doc_type` in SQL via `readback::enriched_list` (WS2-scoped). Returns always-compiled temper-core
/// types so the MCP consumer needs no feature gate. Native rows: real timestamps (event-sourced
/// from `kb_events.occurred_at`), name-only doc type, no fabricated fields.
pub async fn list_enriched_select(
    pool: &PgPool,
    profile_id: Uuid,
    context_name: Option<&str>,
    doc_type: Option<&str>,
) -> ApiResult<Vec<(ResourceRow, Option<ManagedMeta>, Option<serde_json::Value>)>> {
    let rows = readback::enriched_list(pool, profile_id, context_name, doc_type)
        .await
        .map_err(api_err)?;
    let mut out = Vec::with_capacity(rows.len());
    for r in rows {
        let row = ResourceRow {
            id: ResourceId::from(r.new_id),
            kb_context_id: ContextId::from(Uuid::nil()), // re-minted; unused by build_enriched
            origin_uri: r.origin_uri,
            title: r.title,
            originator_profile_id: ProfileId::from(Uuid::nil()),
            owner_profile_id: ProfileId::from(Uuid::nil()),
            is_active: r.is_active,
            created: r.created,
            updated: r.updated,
            context_name: r.context_name,
            doc_type_name: r.doc_type,
            owner_handle: "@me".to_string(),
            stage: r.stage,
            seq: None,
            mode: r.mode,
            effort: r.effort,
            body_hash: None,
        };
        // Propagate a genuine deser failure (don't swallow to None — a malformed managed shape is a fault).
        let managed: Option<ManagedMeta> =
            Some(serde_json::from_value(serde_json::Value::Object(r.managed)).map_err(api_err)?);
        let open = Some(serde_json::Value::Object(r.open));
        out.push((row, managed, open));
    }
    Ok(out)
}
