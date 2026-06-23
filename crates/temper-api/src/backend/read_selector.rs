//! Substrate read dispatcher — the service-direct read paths (list / show / get_content / get_meta /
//! search + the MCP enrichment list/meta-batch) over the one schema.
//!
//! These reads bypass the `Backend` trait by design (the trait projections are lossy and don't cover
//! meta/body/content); they resolve against `temper_next::readback`, reconstructing the
//! production-shaped types at the §9 floor. Visibility is scoped to the caller's profile (WS2) — the
//! readbacks gate through `resources_visible_to`. SQL is unqualified against the one schema (the
//! connection carries the search_path).
//!
//! `list`/`list_meta` are unfiltered at the §9 floor (the context/doctype/pagination filters on
//! `ResourceListParams` are not yet applied to the substrate list — a named post-collapse follow-up);
//! the enrichment path (`list_enriched`/MCP) DOES filter by name in SQL via `readback::enriched_list`.

use std::collections::HashMap;

use sqlx::PgPool;
use uuid::Uuid;

use crate::backend::db_backend::{map_readback_err, reconstruct_resource_row};
use crate::error::{ApiError, ApiResult};
use crate::services::resource_service::{ResourceListParams, ResourceListResponse};
use temper_core::error::TemperError;
use temper_core::types::api::{SearchParams, UnifiedSearchResultRow};
use temper_core::types::ids::{ContextId, DocTypeId, ProfileId, ResourceId};
use temper_core::types::managed_meta::{
    ManagedMeta, ResourceMetaListResponse, ResourceMetaResponse,
};
use temper_core::types::resource::{ContentResponse, ResourceFacets, ResourceRow};
use temper_next::readback;

fn api_err(e: impl std::fmt::Display) -> ApiError {
    ApiError::from(TemperError::Api(e.to_string()))
}

/// `list` — every resource VISIBLE to the principal (WS2 — `resources_visible_to`), reconstructed to a
/// full `ResourceRow`. No pagination; the asserted invariant is the visible row SET + projected fields
/// (the `_params` filters are a post-collapse follow-up). `total` = row count; `facets.doc_type` = the
/// doctype histogram over the visible set.
pub async fn list_select(
    pool: &PgPool,
    profile_id: Uuid,
    _params: ResourceListParams,
) -> ApiResult<ResourceListResponse> {
    // Only resources visible to the principal. `resources_visible_to` returns substrate ids directly
    // (profile ids preserved by synthesis), so we filter the set up front — a not-visible id never
    // enters the loop, where `reconstruct_resource_row`'s gate would otherwise error.
    let visible: Vec<Uuid> = sqlx::query_scalar("SELECT resource_id FROM resources_visible_to($1)")
        .bind(profile_id)
        .fetch_all(pool)
        .await
        .map_err(api_err)?;
    let mut rows: Vec<ResourceRow> = Vec::with_capacity(visible.len());
    for new_id in visible {
        rows.push(reconstruct_resource_row(pool, profile_id, new_id).await?);
    }
    let mut doc_type: HashMap<String, i64> = HashMap::new();
    for r in &rows {
        *doc_type.entry(r.doc_type_name.clone()).or_insert(0) += 1;
    }
    let total = rows.len() as i64;
    Ok(ResourceListResponse {
        rows,
        total,
        facets: ResourceFacets { doc_type },
    })
}

/// `show` — full resource row by id (§9 invariant floor) via the shared `reconstruct_resource_row`. The
/// inbound id IS the substrate id. Visibility is gated inside `reconstruct_resource_row` (WS2); the typed
/// `ReadbackError` is split by `map_readback_err` (not-visible → NotFound/404, fault → Api/500).
pub async fn show_select(pool: &PgPool, profile_id: Uuid, id: Uuid) -> ApiResult<ResourceRow> {
    reconstruct_resource_row(pool, profile_id, id)
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

/// `list_meta` — the `?meta_only=true` projection. Reuses the WS2-scoped `readback::enriched_list`
/// visible set (no context/doctype filter), mapping each row to a `ResourceMetaResponse`.
pub async fn list_meta_select(
    pool: &PgPool,
    profile_id: Uuid,
    _params: ResourceListParams,
) -> ApiResult<ResourceMetaListResponse> {
    let rows = readback::enriched_list(pool, profile_id, None, None)
        .await
        .map_err(api_err)?;
    let mut doc_type: HashMap<String, i64> = HashMap::new();
    let mut out = Vec::with_capacity(rows.len());
    for r in rows {
        *doc_type.entry(r.doc_type.clone()).or_insert(0) += 1;
        let managed: ManagedMeta =
            serde_json::from_value(serde_json::Value::Object(r.managed)).map_err(api_err)?;
        out.push(ResourceMetaResponse {
            resource_id: ResourceId::from(r.new_id),
            managed_meta: Some(managed),
            open_meta: Some(serde_json::Value::Object(r.open)),
            managed_hash: String::new(),
            open_hash: String::new(),
        });
    }
    let total = out.len() as i64;
    Ok(ResourceMetaListResponse {
        rows: out,
        total,
        facets: ResourceFacets { doc_type },
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
        let row = reconstruct_resource_row(pool, profile_id, new_id).await?;
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
/// types so the MCP consumer needs no feature gate. `slug`/timestamps are §9 non-invariants (None/now()).
pub async fn list_enriched_select(
    pool: &PgPool,
    profile_id: Uuid,
    context_name: Option<&str>,
    doc_type: Option<&str>,
) -> ApiResult<Vec<(ResourceRow, Option<ManagedMeta>, Option<serde_json::Value>)>> {
    let rows = readback::enriched_list(pool, profile_id, context_name, doc_type)
        .await
        .map_err(api_err)?;
    let now = chrono::Utc::now();
    let mut out = Vec::with_capacity(rows.len());
    for r in rows {
        let row = ResourceRow {
            id: ResourceId::from(r.new_id),
            kb_context_id: ContextId::from(Uuid::nil()), // re-minted; unused by build_enriched
            kb_doc_type_id: DocTypeId::from(Uuid::nil()), // re-minted; name is authoritative
            origin_uri: r.origin_uri,
            title: r.title,
            slug: None, // §7-dissolved
            originator_profile_id: ProfileId::from(Uuid::nil()),
            owner_profile_id: ProfileId::from(Uuid::nil()),
            is_active: r.is_active,
            created: now, // synthesis-collapsed (non-invariant)
            updated: now,
            context_name: r.context_name,
            doc_type_name: r.doc_type,
            owner_handle: "@me".to_string(),
            stage: r.stage,
            seq: None,
            mode: r.mode,
            effort: r.effort,
            body_hash: None,
            managed_hash: None,
            open_hash: None,
        };
        // Propagate a genuine deser failure (don't swallow to None — a malformed managed shape is a fault).
        let managed: Option<ManagedMeta> =
            Some(serde_json::from_value(serde_json::Value::Object(r.managed)).map_err(api_err)?);
        let open = Some(serde_json::Value::Object(r.open));
        out.push((row, managed, open));
    }
    Ok(out)
}
