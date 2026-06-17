//! Read selector (WS6 chunk 4b) — routes the service-direct read paths to either the legacy
//! `public.*` services or the `temper_next.*` readback, per `AppState.backend_selection`. These reads
//! bypass the `Backend` trait by design (the 4a finding: reads are service-direct passthroughs; the
//! trait projections are lossy and don't cover meta/body/content).
//!
//! Covered in 4b: `list` / `get_content` (body) / `get_meta` / `search`. **`by_uri` is NOT covered** —
//! it resolves a resource by `slug` (`ResolveByUriParams.ident`), and slug is §7-dissolved in
//! `temper_next` (the addressing key does not exist there; `origin_uri` is the substrate key). It stays
//! on legacy under `next`; re-addressing the endpoint is a post-flip surface concern.
//!
//! The `Next` arms are feature-gated behind `next-backend`; without the feature they return the same
//! `NotImplemented` gate as `select_backend`. Reads are visibility-SCOPED to the caller's profile (WS2 —
//! the readbacks gate through `temper_next.resources_visible_to`, CONFORMing to production's scoped
//! reads; the auth'd profile id is preserved by synthesis, so it is the `temper_next` principal directly).

use sqlx::PgPool;
use uuid::Uuid;

use temper_core::types::ids::{ProfileId, ResourceId};

use crate::backend::selection::BackendSelection;
use crate::error::ApiResult;
use crate::services::resource_service::{self, ResourceListParams, ResourceListResponse};
use crate::services::{meta_service, search_service};
use temper_core::types::api::{SearchParams, UnifiedSearchResultRow};
use temper_core::types::managed_meta::ResourceMetaResponse;
use temper_core::types::resource::ContentResponse;

/// `list` — list visible resources.
pub async fn list_select(
    selection: BackendSelection,
    pool: &PgPool,
    profile_id: Uuid,
    params: ResourceListParams,
) -> ApiResult<ResourceListResponse> {
    match selection {
        BackendSelection::Legacy => resource_service::list_visible(pool, profile_id, params).await,
        BackendSelection::Next => next_impl::list(pool, profile_id).await,
    }
}

/// `get_content` — reconstructed markdown body.
pub async fn get_content_select(
    selection: BackendSelection,
    pool: &PgPool,
    profile_id: Uuid,
    resource_id: Uuid,
) -> ApiResult<ContentResponse> {
    match selection {
        BackendSelection::Legacy => {
            resource_service::get_content(pool, profile_id, resource_id).await
        }
        BackendSelection::Next => next_impl::get_content(pool, profile_id, resource_id).await,
    }
}

/// `get_meta` — managed/open frontmatter for one resource.
pub async fn get_meta_select(
    selection: BackendSelection,
    pool: &PgPool,
    profile_id: ProfileId,
    resource_id: ResourceId,
) -> ApiResult<ResourceMetaResponse> {
    match selection {
        BackendSelection::Legacy => meta_service::get_meta(pool, profile_id, resource_id).await,
        BackendSelection::Next => {
            next_impl::get_meta(pool, Uuid::from(profile_id), Uuid::from(resource_id)).await
        }
    }
}

/// `search` — unified FTS/vector search.
pub async fn search_select(
    selection: BackendSelection,
    pool: &PgPool,
    profile_id: Uuid,
    params: SearchParams,
) -> ApiResult<Vec<UnifiedSearchResultRow>> {
    match selection {
        BackendSelection::Legacy => search_service::search(pool, profile_id, params).await,
        BackendSelection::Next => next_impl::search(pool, profile_id, params).await,
    }
}

// ---------------------------------------------------------------------------
// Next arms — feature-gated. Without `next-backend`, each gates with the same
// NotImplemented as `select_backend`; with it, each maps `temper_next` readback.
// ---------------------------------------------------------------------------

#[cfg(not(feature = "next-backend"))]
mod next_impl {
    use super::*;
    use crate::error::ApiError;
    use temper_core::error::TemperError;

    fn gate<T>() -> ApiResult<T> {
        Err(ApiError::from(TemperError::NotImplemented(
            "next backend requires the `next-backend` build feature".into(),
        )))
    }
    pub(super) async fn list(_: &PgPool, _: Uuid) -> ApiResult<ResourceListResponse> {
        gate()
    }
    pub(super) async fn get_content(_: &PgPool, _: Uuid, _: Uuid) -> ApiResult<ContentResponse> {
        gate()
    }
    pub(super) async fn get_meta(_: &PgPool, _: Uuid, _: Uuid) -> ApiResult<ResourceMetaResponse> {
        gate()
    }
    pub(super) async fn search(
        _: &PgPool,
        _: Uuid,
        _: SearchParams,
    ) -> ApiResult<Vec<UnifiedSearchResultRow>> {
        gate()
    }
}

#[cfg(feature = "next-backend")]
mod next_impl {
    use super::*;
    use crate::backend::next_backend::{map_readback_err, reconstruct_resource_row};
    use crate::error::ApiError;
    use std::collections::HashMap;
    use temper_core::error::TemperError;
    use temper_core::types::managed_meta::ManagedMeta;
    use temper_core::types::resource::{ResourceFacets, ResourceRow};
    use temper_next::readback;

    fn api_err(e: impl std::fmt::Display) -> ApiError {
        ApiError::from(TemperError::Api(e.to_string()))
    }

    /// `list` over `temper_next`: reconstruct a full `ResourceRow` per resource VISIBLE to the principal
    /// (WS2 — `resources_visible_to`, CONFORMing to production's scoped list). No pagination; the asserted
    /// invariant is the visible row SET + projected fields, not order or page bounds. `total` = row count;
    /// `facets.doc_type` = the doctype histogram over the visible set.
    pub(super) async fn list(pool: &PgPool, principal: Uuid) -> ApiResult<ResourceListResponse> {
        // WS2: only resources visible to the principal. `resources_visible_to` returns synthesized
        // (`temper_next`) ids directly (profile ids are preserved by synthesis), so we filter the set
        // up front — a not-visible id never enters the loop, where `reconstruct_resource_row`'s gate
        // would otherwise error. The per-row gate inside re-checks harmlessly (defense in depth).
        let visible: Vec<Uuid> =
            sqlx::query_scalar("SELECT resource_id FROM temper_next.resources_visible_to($1)")
                .bind(principal)
                .fetch_all(pool)
                .await
                .map_err(api_err)?;
        let mut rows: Vec<ResourceRow> = Vec::with_capacity(visible.len());
        for new_id in visible {
            rows.push(reconstruct_resource_row(pool, principal, new_id).await?);
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

    /// `get_content` over `temper_next`: reconstruct the markdown body (§9 body floor). `managed_meta`
    /// / `open_meta` are left `None` — the body markdown is the floor; the meta tier is `get_meta`.
    pub(super) async fn get_content(
        pool: &PgPool,
        principal: Uuid,
        prod_id: Uuid,
    ) -> ApiResult<ContentResponse> {
        let new_id = resolve_new_id(pool, prod_id).await?;
        // `readback::body` gates visibility (WS2) and returns a typed `ReadbackError`; `map_readback_err`
        // splits not-visible → NotFound (404, leak-safe deny, never 403) from a genuine fault → Api (500).
        let markdown = readback::body(pool, principal, new_id)
            .await
            .map_err(|e| ApiError::from(map_readback_err(e)))?;
        Ok(ContentResponse {
            resource_id: ResourceId::from(new_id),
            markdown,
            managed_meta: None,
            open_meta: None,
        })
    }

    /// `get_meta` over `temper_next`: reconstruct the managed/open split (`readback::meta`, the §7
    /// inverse fate). `managed_hash`/`open_hash` are §7-dissolved (no manifest in `temper_next`) — they
    /// are emitted empty (non-invariant; the §9 floor does not assert them).
    pub(super) async fn get_meta(
        pool: &PgPool,
        principal: Uuid,
        prod_id: Uuid,
    ) -> ApiResult<ResourceMetaResponse> {
        let new_id = resolve_new_id(pool, prod_id).await?;
        // `readback::meta` gates visibility (WS2) and returns a typed `ReadbackError`; `map_readback_err`
        // splits not-visible → NotFound (404, leak-safe deny, never 403) from a genuine fault → Api (500).
        let rb = readback::meta(pool, principal, new_id)
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

    /// `search` over `temper_next`: vector when an embedding is supplied, else FTS over the text query
    /// (§9 search floor). The matching SET (origin_uri) is the invariant; scores are not (emitted 0.0).
    /// Each matched `origin_uri` is enriched to a `UnifiedSearchResultRow` via a full-row reconstruction
    /// (title + doctype). `slug` is §7-dissolved (emitted empty).
    pub(super) async fn search(
        pool: &PgPool,
        principal: Uuid,
        params: SearchParams,
    ) -> ApiResult<Vec<UnifiedSearchResultRow>> {
        // WS2: the search readbacks JOIN `resources_visible_to(principal)`, so the result set is
        // already visibility-scoped (a not-visible match never surfaces).
        let (origin_uris, origin) = if let Some(embedding) = params.embedding.as_ref() {
            (
                readback::vector_search(pool, principal, embedding)
                    .await
                    .map_err(api_err)?,
                "vector",
            )
        } else if let Some(query) = params.query.as_ref() {
            (
                readback::fts_search(pool, principal, query)
                    .await
                    .map_err(api_err)?,
                "fts",
            )
        } else {
            (Vec::new(), "fts")
        };

        let mut hits = Vec::with_capacity(origin_uris.len());
        for origin_uri in origin_uris {
            let Some(new_id) = readback::resource_id_by_origin_uri(pool, &origin_uri)
                .await
                .map_err(api_err)?
            else {
                continue;
            };
            let row = reconstruct_resource_row(pool, principal, new_id).await?;
            hits.push(UnifiedSearchResultRow {
                resource_id: new_id,
                title: row.title,
                slug: String::new(),
                kb_uri: origin_uri.clone(),
                origin_uri,
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

    /// Map a production resource id to its synthesized counterpart (by `origin_uri`, via the bimap).
    async fn resolve_new_id(pool: &PgPool, prod_id: Uuid) -> ApiResult<Uuid> {
        let ids = readback::ResolvedIds::load(pool).await.map_err(api_err)?;
        ids.to_new(prod_id).ok_or_else(|| {
            ApiError::from(TemperError::NotFound(format!(
                "resource {prod_id} not in temper_next"
            )))
        })
    }
}
