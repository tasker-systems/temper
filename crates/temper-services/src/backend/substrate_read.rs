//! Substrate read dispatcher — the service-direct read paths (list / show / get_content / get_meta /
//! search + the MCP enrichment list/meta-batch) over the one schema.
//!
//! These reads bypass the `Backend` trait by design (the trait projections are lossy and don't cover
//! meta/body/content); they resolve against `temper_substrate::readback`, producing native `ResourceRow`s
//! (real timestamps, name-only doc type, no fabricated fields) via `native_resource_row`. Visibility
//! is scoped to the caller's profile (WS2) — the readbacks gate through `resources_visible_to`. SQL
//! is unqualified against the one schema (the connection carries the search_path).
//!
//! `list`/`list_meta` filter (context_ref/doc_type_name/stage/owner/`q`-title), sort, and paginate the
//! visible set in SQL (`filtered_visible_page`), reconstructing only the page; `context_ref` is resolved
//! to a context UUID before filtering so bare names are rejected (spec Decision 1). Full-text/vector `q`
//! on the list endpoint is search's job (a named deferral) — list `q` is a trivial title `ILIKE`.

use std::collections::HashMap;

use sqlx::{PgPool, Row};
use uuid::Uuid;

use crate::backend::db_backend::{map_readback_err, native_resource_row};
use crate::error::{ApiError, ApiResult};
use crate::services::context_service::resolve_context_ref;
use crate::services::resource_service::{ResourceListParams, ResourceListResponse};
use temper_core::context_ref::parse_context_ref;
use temper_core::error::TemperError;
use temper_core::types::api::{SearchParams, UnifiedSearchResultRow};
use temper_core::types::cognitive_maps::{
    CharterBlock, CogmapAnalyticsRow, CogmapRegionMetricsRow, CogmapRegionRow, CogmapRegulationRow,
    CogmapStaleness,
};
use temper_core::types::ids::{CogmapId, ContextId, LensId, ProfileId, ResourceId};
use temper_core::types::invocation::{InvocationActRow, InvocationSummary, InvocationView};
use temper_core::types::provenance::BlockProvenanceRow;
use temper_substrate::readback;
use temper_workflow::types::managed_meta::{
    ManagedMeta, ResourceMetaListResponse, ResourceMetaResponse,
};
use temper_workflow::types::resource::{
    ContentResponse, ResourceFacets, ResourceRow, ResourceSortField, SortOrder,
};

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
/// substrate: `kb_resources` (updated/created/title), `kb_contexts.name`, the
/// `kb_resource_workflow_props` pivot view (`stage`/`seq`), and the `doc_type` property.
fn sort_column_sql(field: ResourceSortField) -> &'static str {
    match field {
        ResourceSortField::Updated => "r.updated",
        ResourceSortField::Created => "r.created",
        ResourceSortField::Title => "r.title",
        ResourceSortField::Stage => "wp.stage",
        ResourceSortField::Seq => "wp.seq::bigint",
        ResourceSortField::ContextName => "c.name",
        ResourceSortField::DocTypeName => "dt.property_value #>> '{}'",
    }
}

/// Resolve the visible set, apply the `ResourceListParams` filters (context_ref /
/// doc_type_name / stage / owner / `q` title-match) + sort + pagination IN SQL, and
/// return only the page's ids (so the caller reconstructs the page, not every visible
/// row — this also fixes the prior all-rows N+1).
///
/// `context_ref` is a UUID string or `@owner/slug` decorated ref. It is resolved to a
/// context UUID before the SQL runs — bare names are rejected with `BadRequest` (spec
/// Decision 1). Filter is then `c.id = $2` (UUID), eliminating the prior name-ambiguity
/// bug where two contexts sharing a name both matched.
///
/// `owner`: `@me` resolves to the caller's profile; any other value matches the owner
/// profile's `handle` (per `graph.rs`'s handle convention). `q` is a trivial title
/// `ILIKE` (full text/vector `q` is search's job — a named deferral). Dynamic ORDER BY
/// is built from the enum; the WHERE binds Option params via the `($N IS NULL OR …)`
/// idiom, so this is the documented runtime-`query` exception (dynamic ORDER clause),
/// not a static macro.
async fn filtered_visible_page(
    pool: &PgPool,
    profile_id: ProfileId,
    params: &ResourceListParams,
) -> ApiResult<VisiblePage> {
    // Resolve context_ref → UUID before building SQL. A bare name is
    // rejected by `parse_context_ref` (spec Decision 1); an @owner/slug
    // ref is resolved via `resolve_context_ref` (visibility-gated).
    let context_id: Option<Uuid> = match params.context_ref.as_deref() {
        Some(s) => {
            let cref = parse_context_ref(s).map_err(|e| ApiError::BadRequest(e.to_string()))?;
            Some(*resolve_context_ref(pool, profile_id, &cref).await?)
        }
        None => None,
    };

    let owner_self: Option<Uuid> = match params.owner.as_deref() {
        Some("@me") => Some(*profile_id),
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
    // `readback::reconstruct`); LEFT JOIN the `kb_resource_workflow_props` pivot view
    // (migration 20260709000002) for the optional workflow keys used by filters/sort.
    let sql = format!(
        "SELECT r.id AS id, dt.property_value #>> '{{}}' AS doc_type_name
           FROM kb_resources r
           JOIN resources_visible_to($1) v ON v.resource_id = r.id
           JOIN kb_resource_homes h ON h.resource_id = r.id
           LEFT JOIN kb_contexts c
             ON c.id = h.anchor_id AND h.anchor_table = 'kb_contexts'
           JOIN kb_profiles p ON p.id = h.owner_profile_id
           JOIN kb_properties dt
             ON dt.owner_table = 'kb_resources' AND dt.owner_id = r.id
            AND dt.property_key = 'doc_type' AND NOT dt.is_folded
           LEFT JOIN kb_resource_workflow_props wp ON wp.resource_id = r.id
          WHERE r.is_active
            AND ($2::uuid IS NULL OR c.id = $2)
            AND ($3::text IS NULL OR dt.property_value #>> '{{}}' = $3)
            AND ($4::text IS NULL OR wp.stage = $4)
            AND ($5::uuid IS NULL OR h.owner_profile_id = $5)
            AND ($6::text IS NULL OR p.handle = $6)
            AND ($7::text IS NULL OR r.title ILIKE '%' || $7 || '%')
            AND ($8::uuid IS NULL OR EXISTS (
                  SELECT 1 FROM kb_edges ge
                   WHERE ge.source_table = 'kb_resources' AND ge.source_id = r.id
                     AND ge.target_table = 'kb_resources' AND ge.target_id = $8
                     AND ge.edge_kind = 'leads_to' AND ge.label = $9
                     AND NOT ge.is_folded))
          ORDER BY {sort_col} {dir}, r.id ASC",
        sort_col = sort_column_sql(sort),
    );

    // The goal filter matches the live `advances`→goal edge minted by the create/update
    // projection — same `edge_kind`='leads_to' and label (`GOAL_EDGE_LABEL`). They must agree.
    let rows = sqlx::query(&sql)
        .bind(profile_id)
        .bind(context_id)
        .bind(params.doc_type_name.as_deref())
        .bind(params.stage.as_deref())
        .bind(owner_self)
        .bind(owner_handle)
        .bind(params.q.as_deref())
        .bind(params.goal)
        .bind(super::db_backend::GOAL_EDGE_LABEL)
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
    profile_id: ProfileId,
    params: ResourceListParams,
) -> ApiResult<ResourceListResponse> {
    let page = filtered_visible_page(pool, profile_id, &params).await?;
    let mut rows: Vec<ResourceRow> = Vec::with_capacity(page.page_ids.len());
    for new_id in page.page_ids {
        rows.push(native_resource_row(pool, profile_id, ResourceId::from(new_id)).await?);
    }
    Ok(ResourceListResponse {
        rows,
        total: page.total,
        facets: ResourceFacets {
            doc_type: page.facets,
        },
    })
}

/// `show` — full native resource row by id via `native_resource_row`. The inbound id IS the substrate id.
/// Visibility is gated inside `native_resource_row` (WS2); the typed `ReadbackError` is split by
/// `map_readback_err` (not-visible → NotFound/404, fault → Api/500).
pub async fn show_select(
    pool: &PgPool,
    profile_id: ProfileId,
    id: ResourceId,
) -> ApiResult<ResourceRow> {
    native_resource_row(pool, profile_id, id)
        .await
        .map_err(ApiError::from)
}

/// `get_content` — native markdown body for the resource. `managed_meta`/`open_meta` are `None`
/// (the meta tier is `get_meta`).
pub async fn get_content_select(
    pool: &PgPool,
    profile_id: ProfileId,
    resource_id: ResourceId,
) -> ApiResult<ContentResponse> {
    let markdown = readback::body(pool, profile_id, resource_id)
        .await
        .map_err(|e| ApiError::from(map_readback_err(e)))?;
    Ok(ContentResponse {
        resource_id,
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
    let rb = readback::meta(pool, profile_id, resource_id)
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
    profile_id: ProfileId,
    params: ResourceListParams,
) -> ApiResult<ResourceMetaListResponse> {
    let page = filtered_visible_page(pool, profile_id, &params).await?;
    let mut out = Vec::with_capacity(page.page_ids.len());
    for new_id in page.page_ids {
        out.push(get_meta_select(pool, profile_id, ResourceId::from(new_id)).await?);
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
    profile_id: ProfileId,
    ids: &[ResourceId],
) -> ApiResult<HashMap<ResourceId, ResourceMetaResponse>> {
    let mut map = HashMap::with_capacity(ids.len());
    for id in ids {
        match get_meta_select(pool, profile_id, *id).await {
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

/// Surface A caps resolved once, before the SQL call (pure → unit-tested).
pub(crate) struct ClampedSearch {
    pub depth: i32,
    pub limit: i64,
}

/// graph_depth → \[1,3\] (deep traversal is a Surface-B concern; a 10-hop fan-out would threaten the DB);
/// limit → \[1,50\] (the documented API ceiling). Defaults: depth 2, limit 10.
pub(crate) fn clamp_search_params(params: &SearchParams) -> ClampedSearch {
    ClampedSearch {
        depth: params.graph_depth.unwrap_or(2).clamp(1, 3),
        limit: params.limit.unwrap_or(10).clamp(1, 50),
    }
}

/// Resolve the one active scope selector (§6) into the corpus bounds for `unified_search`: an optional
/// context id (Surface A `context_ref`) and an optional explicit scope-id set (Surface B — `cogmap_id`
/// single-map, or `wayfind` region-salience funnel). At most one of `{context_ref, cogmap_id, wayfind}`
/// may be set; more than one is a `BadRequest`. An empty scope-id set is the deny case — it yields zero
/// rows downstream via `c.id = ANY('{}')`, never an error ("no view from nowhere", spec §5/§7).
async fn resolve_search_scope(
    pool: &PgPool,
    profile_id: ProfileId,
    params: &SearchParams,
) -> ApiResult<(Option<uuid::Uuid>, Option<Vec<Uuid>>)> {
    let scope_selectors = [
        params.context_ref.is_some(),
        params.cogmap_id.is_some(),
        params.wayfind,
    ]
    .into_iter()
    .filter(|&set| set)
    .count();
    if scope_selectors > 1 {
        return Err(ApiError::BadRequest(
            "context_ref, cogmap_id, and wayfind are mutually exclusive".into(),
        ));
    }

    // Resolve context_ref → context UUID. A bare name is rejected by `parse_context_ref` (spec
    // Decision 1); an @owner/slug or UUID ref resolves via `resolve_context_ref` (visibility-gated).
    let context_id: Option<uuid::Uuid> = match params.context_ref.as_deref() {
        Some(s) => {
            let cref = temper_core::context_ref::parse_context_ref(s)
                .map_err(|e| ApiError::BadRequest(e.to_string()))?;
            Some(
                *crate::services::context_service::resolve_context_ref(pool, profile_id, &cref)
                    .await?,
            )
        }
        None => None,
    };

    // `wayfind` runs the region-salience funnel (Task A); `cogmap_id` the single-map scope. Both
    // visibility-gate inside the SQL; an empty result is deny → zero rows, never an error.
    let scope_ids: Option<Vec<Uuid>> = if params.wayfind {
        Some(
            readback::wayfind_scope_ids(
                pool,
                readback::WayfindScopeQuery {
                    principal: profile_id,
                    lens_id: params.lens_id.map(LensId::from),
                    // The query embedding feeds BOTH region selection here and the blend inside
                    // `unified_search` — intentionally the same signal.
                    embedding: params.embedding.as_deref(),
                    // Saturate the i64→i32 narrowing so a huge N can't wrap negative; the SQL `k`
                    // CTE then clamps into [1, max_n].
                    regions: params.regions.map(|n| n.clamp(0, i32::MAX as i64) as i32),
                },
            )
            .await
            .map_err(api_err)?,
        )
    } else if let Some(map) = params.cogmap_id {
        Some(
            sqlx::query_scalar!("SELECT cogmap_scope_ids($1, $2)", profile_id.uuid(), map)
                .fetch_all(pool)
                .await
                .map_err(api_err)?
                .into_iter()
                .flatten()
                .collect(),
        )
    } else {
        None
    };

    Ok((context_id, scope_ids))
}

/// Embed a text-only query server-side so the vector arm contributes for callers that can't run the
/// ONNX model themselves (MCP clients, raw HTTP, agent workers). The CLI precomputes the vector and
/// sends it in `SearchParams.embedding`, so this is a no-op for that path — as is a query that is
/// empty/whitespace or already carries an embedding (issue #297).
///
/// The query is embedded with the SAME plain `embed_text` path the corpus was ingested with
/// (`prepare_markdown → embed_texts`, no BGE "represent this sentence…" query prefix), keeping it in
/// the stored chunks' vector space so scores stay comparable to the CLI's. Do not introduce a
/// query-side prefix here without re-embedding the corpus.
///
/// Failure mode is fallback-with-warn: if the model errors (e.g. unavailable on an unexpected target),
/// we log and proceed with FTS + graph only rather than turning a soft degradation into a hard 500 —
/// partial results beat none, and this preserves the pre-#297 behavior on embed failure.
fn embed_query_if_missing(params: &mut SearchParams) {
    if params.embedding.is_some() {
        return;
    }
    let query = match params.query.as_deref().map(str::trim) {
        Some(q) if !q.is_empty() => q.to_string(),
        _ => return,
    };
    match temper_ingest::embed::embed_text(&query) {
        Ok(embedding) => params.embedding = Some(embedding),
        Err(e) => tracing::warn!(
            error = %e,
            "server-side query embedding failed; falling back to FTS + graph only"
        ),
    }
}

/// `search` — Surface A general search (Beat 2): one composed `unified_search` readback blending FTS +
/// vector + graph into ranked, scored hits, then per-row display enrichment. Replaces the either/or,
/// zero-score path. Visibility is enforced inside every candidate function (`resources_visible_to`).
pub async fn search_select(
    pool: &PgPool,
    profile_id: ProfileId,
    mut params: SearchParams,
) -> ApiResult<Vec<UnifiedSearchResultRow>> {
    embed_query_if_missing(&mut params);
    let clamped = clamp_search_params(&params);
    let (context_id, scope_ids) = resolve_search_scope(pool, profile_id, &params).await?;

    // The wire `seed_ids` arrive as bare uuids; lift to the typed `&[ResourceId]` the query takes.
    let seed_ids: Vec<ResourceId> = params
        .seed_ids
        .as_deref()
        .unwrap_or(&[])
        .iter()
        .copied()
        .map(ResourceId::from)
        .collect();
    let hits = readback::unified_search(
        pool,
        readback::UnifiedSearchQuery {
            principal: profile_id,
            query: params.query.as_deref(),
            embedding: params.embedding.as_deref(),
            seed_ids: &seed_ids,
            depth: clamped.depth,
            edge_types: params.edge_types.as_deref().unwrap_or(&[]),
            context_id: context_id.map(ContextId::from),
            doc_type: params.doc_type.as_deref(),
            graph_expand: params.graph_expand,
            limit: clamped.limit,
            offset: params.offset.unwrap_or(0),
            scope_ids: scope_ids.as_deref(),
        },
    )
    .await
    .map_err(api_err)?;

    let mut out = Vec::with_capacity(hits.len());
    for h in hits {
        // Enrich every hit through the cogmap-aware `native_resource_row` (Task F:
        // `readback::resource_row` LEFT-JOINs kb_contexts AND kb_cogmaps and is visibility-gated).
        // Context-homed hits carry `context_*`; cogmap-homed hits carry `cogmap_*`. `home_display`
        // surfaces whichever home is set, so a `--cogmap` hit renders the map name rather than null.
        let row = native_resource_row(pool, profile_id, h.resource_id).await?;
        let context = row.home_display().map(str::to_owned);
        out.push(UnifiedSearchResultRow {
            resource_id: h.resource_id.uuid(),
            title: row.title,
            slug: String::new(),
            kb_uri: row.origin_uri.clone(),
            origin_uri: row.origin_uri,
            context,
            doc_type: row.doc_type_name,
            fts_score: h.fts_score,
            vector_score: h.vector_score,
            graph_score: h.graph_score,
            combined_score: h.combined_score,
            origin: "unified".to_string(),
            context_slug: row.context_slug,
            context_owner_ref: row.context_owner_ref,
        });
    }
    Ok(out)
}

/// `cogmap_shape` — the surface-tier read of a cognitive map's materialized regions. Service-direct
/// (reads bypass the Backend trait). The access gate lives in the SQL function: a principal who cannot
/// read the map gets an empty vec, never an error. Maps the substrate-local row to the wire type.
pub async fn cogmap_shape_select(
    pool: &PgPool,
    profile_id: ProfileId,
    cogmap_id: uuid::Uuid,
    lens_id: Option<uuid::Uuid>,
) -> ApiResult<Vec<CogmapRegionRow>> {
    let rows = readback::cogmap_shape(
        pool,
        CogmapId::from(cogmap_id),
        profile_id,
        lens_id.map(LensId::from),
    )
    .await
    .map_err(api_err)?;
    Ok(rows
        .into_iter()
        .map(|r| CogmapRegionRow {
            region_id: r.region_id,
            lens_id: r.lens_id,
            salience: r.salience,
            content_cohesion: r.content_cohesion,
            label: r.label,
            member_count: r.member_count,
        })
        .collect())
}

/// `cogmap_region_metrics` — the per-region analytics tier. Service-direct; gate is in the SQL
/// (deny → empty). Maps the substrate-local row to the wire type.
pub async fn cogmap_region_metrics_select(
    pool: &PgPool,
    profile_id: ProfileId,
    cogmap_id: uuid::Uuid,
    lens_id: Option<uuid::Uuid>,
) -> ApiResult<Vec<CogmapRegionMetricsRow>> {
    let rows = readback::cogmap_region_metrics(
        pool,
        CogmapId::from(cogmap_id),
        profile_id,
        lens_id.map(LensId::from),
    )
    .await
    .map_err(api_err)?;
    Ok(rows
        .into_iter()
        .map(|r| CogmapRegionMetricsRow {
            region_id: r.region_id,
            lens_id: r.lens_id,
            centrality: r.centrality,
            content_cohesion: r.content_cohesion,
            internal_tension: r.internal_tension,
            reference_standing: r.reference_standing,
            telos_alignment: r.telos_alignment,
        })
        .collect())
}

/// `cogmap_analytics` — the map-level analytics picture. Service-direct; gate is in the SQL
/// (deny → `None`, surfaced as 404 by the handler). Maps the substrate-local row to the wire type.
pub async fn cogmap_analytics_select(
    pool: &PgPool,
    profile_id: ProfileId,
    cogmap_id: uuid::Uuid,
) -> ApiResult<Option<CogmapAnalyticsRow>> {
    let got = readback::cogmap_analytics(pool, CogmapId::from(cogmap_id), profile_id)
        .await
        .map_err(api_err)?;
    Ok(got.map(|a| CogmapAnalyticsRow {
        telos_resource_id: a.telos_resource_id,
        staleness: CogmapStaleness {
            materialized_at: a.staleness.materialized_at,
            latest_touch: a.staleness.latest_touch,
            is_stale: a.staleness.is_stale,
        },
        regulation: a
            .regulation
            .into_iter()
            .map(|r| CogmapRegulationRow {
                resource_id: r.resource_id,
                title: r.title,
                body_text: r.body_text,
                edge_label: r.edge_label,
            })
            .collect(),
    }))
}

/// `cogmap_charter_select` (T1 Sequence C) — the telos/charter-block read: composes `cogmap_telos`
/// (resolve the map to its charter resource) with the generic `resource_blocks` projection, unfiltered
/// by role, so a caller gets the statement + questions + framing in seq order. Service-direct (reads
/// bypass the Backend trait); the access gate lives IN the SQL (`resources_readable_by('profile', …)`
/// composes `resources_visible_to`) — a principal who cannot read the charter resource gets an empty
/// vec, never an error. `role`/`body_text`/`seq` are forced non-null (`role!`/`body!`/`seq!`): a
/// non-folded charter block always carries exactly one `block_role` and an assembled body (design
/// invariant, `canonical_functions.sql`'s `resource_blocks` comment), so the sqlx-inferred nullability
/// (driven by the function's declared `RETURNS TABLE`, not the actual data) would otherwise force
/// `Option<String>` here for no reason.
pub async fn cogmap_charter_select(
    pool: &PgPool,
    profile_id: ProfileId,
    cogmap_id: uuid::Uuid,
) -> ApiResult<Vec<CharterBlock>> {
    sqlx::query_as!(
        CharterBlock,
        r#"SELECT seq AS "seq!", role AS "role!", body_text AS "body!"
             FROM resource_blocks(cogmap_telos($1), 'profile', $2, NULL)
            ORDER BY seq"#,
        cogmap_id,
        profile_id.uuid(),
    )
    .fetch_all(pool)
    .await
    .map_err(api_err)
}

/// `resource_block_provenance` — the itemized per-block provenance read for one resource. Service-direct
/// (reads bypass the Backend trait). The access gate lives IN the SQL function
/// (`resources_readable_by('profile', …)` composes `resources_visible_to`): a principal who cannot read
/// the resource gets an empty vec, never an error. Rows arrive ordered by `(block_seq, accretion_seq)`.
/// The function's `RETURNS TABLE` declares every column nullable, but each maps a `NOT NULL` table
/// column (or an always-present block field), so we force non-null (`col!`) to avoid needless `Option`s.
pub async fn resource_block_provenance_select(
    pool: &PgPool,
    profile_id: ProfileId,
    resource_id: uuid::Uuid,
) -> ApiResult<Vec<BlockProvenanceRow>> {
    sqlx::query_as!(
        BlockProvenanceRow,
        r#"SELECT block_id            AS "block_id!",
                  block_seq           AS "block_seq!",
                  source_kind         AS "source_kind!",
                  source_id           AS "source_id!",
                  source_uri,
                  accretion_seq       AS "accretion_seq!",
                  contributed_by_event_id AS "contributed_by_event_id!",
                  created             AS "created!"
             FROM resource_block_provenance($1, 'profile', $2)"#,
        resource_id,
        profile_id.uuid(),
    )
    .fetch_all(pool)
    .await
    .map_err(api_err)
}

/// `invocation_show` — the show projection of one invocation envelope plus its acts. Service-direct
/// (reads bypass the Backend trait). The access gate lives in the readback SQL: a principal who cannot
/// read the originating cogmap gets `None`, never an error. Maps the substrate-local row to the wire
/// type.
pub async fn invocation_show_select(
    pool: &PgPool,
    profile_id: ProfileId,
    invocation_id: uuid::Uuid,
) -> ApiResult<Option<InvocationView>> {
    let Some(row) = readback::invocation_show(pool, invocation_id, profile_id)
        .await
        .map_err(api_err)?
    else {
        return Ok(None);
    };
    Ok(Some(InvocationView {
        id: row.id,
        status: row.status,
        trigger_kind: row.trigger_kind,
        originating_cogmap_id: row.originating_cogmap_id,
        parent_cogmap_id: row.parent_cogmap_id,
        scoped_entity_id: row.scoped_entity_id,
        telos_resource_id: row.telos_resource_id,
        outcome: row.outcome,
        opened_at: row.opened_at,
        closed_at: row.closed_at,
        acts: row
            .acts
            .into_iter()
            .map(|a| InvocationActRow {
                event_id: a.event_id,
                event_kind: a.event_kind,
                emitter_entity_id: a.emitter_entity_id,
                occurred_at: a.occurred_at,
                invocation_id: a.invocation_id,
                // Decode the raw kb_events.metadata into typed authorship; `{}` (an unauthored act)
                // fails to deserialize (confidence is required) → None.
                authorship: serde_json::from_value(a.metadata).ok(),
            })
            .collect(),
    }))
}

/// `invocation_list` — the list projection of invocation envelopes, each gated by the principal's read
/// access to its originating cogmap. Service-direct. Optionally narrowed by originating `cogmap` and/or
/// `status`. Maps the substrate-local rows to the wire type.
pub async fn invocation_list_select(
    pool: &PgPool,
    profile_id: ProfileId,
    cogmap: Option<uuid::Uuid>,
    status: Option<String>,
) -> ApiResult<Vec<InvocationSummary>> {
    let rows = readback::invocation_list(pool, profile_id, cogmap, status)
        .await
        .map_err(api_err)?;
    Ok(rows
        .into_iter()
        .map(|r| InvocationSummary {
            id: r.id,
            status: r.status,
            trigger_kind: r.trigger_kind,
            originating_cogmap_id: r.originating_cogmap_id,
            opened_at: r.opened_at,
            closed_at: r.closed_at,
        })
        .collect())
}

#[cfg(test)]
mod clamp_tests {
    use super::*;
    use temper_core::types::api::SearchParams;

    #[test]
    fn clamps_depth_and_limit_to_surface_a_caps() {
        let p = SearchParams {
            graph_depth: Some(10),
            limit: Some(999),
            ..SearchParams::default()
        };
        let c = clamp_search_params(&p);
        assert_eq!(c.depth, 3, "graph_depth capped at 3 for Surface A");
        assert_eq!(c.limit, 50, "limit capped at 50");

        let d = clamp_search_params(&SearchParams::default());
        assert_eq!(d.depth, 2, "default depth 2");
        assert_eq!(d.limit, 10, "default limit 10");
    }

    // `embed_query_if_missing` guards — the no-op cases never touch the ONNX model, so they run in the
    // plain (non-embed) test job. The positive case (text → 768-dim vector) is proven end-to-end in the
    // `test-embed` e2e regression `server_embeds_text_only_query_surfaces_semantic_only_hit`.
    #[test]
    fn embed_query_noop_when_embedding_already_present() {
        let precomputed = vec![0.5_f32; 768];
        let mut p = SearchParams {
            query: Some("kubernetes deploys".into()),
            embedding: Some(precomputed.clone()),
            ..SearchParams::default()
        };
        embed_query_if_missing(&mut p);
        assert_eq!(
            p.embedding.as_deref(),
            Some(precomputed.as_slice()),
            "a precomputed embedding (CLI path) must pass through untouched"
        );
    }

    #[test]
    fn embed_query_noop_when_query_is_none_or_blank() {
        for q in [None, Some(String::new()), Some("   \t\n".into())] {
            let mut p = SearchParams {
                query: q.clone(),
                embedding: None,
                ..SearchParams::default()
            };
            embed_query_if_missing(&mut p);
            assert!(
                p.embedding.is_none(),
                "empty/whitespace/absent query must not trigger embedding (query={q:?})"
            );
        }
    }
}

// T1 Sequence C, Task C1: `cogmap_charter_select`. Genesis-es a cogmap with a real charter (statement
// + 2 questions + framing) through the scenario loader — the same load path
// `charter_yaml_roundtrip.rs` proves byte-exact — then asserts the composed read returns every block
// in seq order, and that a profile with no ownership/grant on the telos resource is denied (empty,
// not an error): the access gate is IN the SQL (`resources_readable_by` → `resources_visible_to`).
#[cfg(all(test, feature = "test-db"))]
mod charter_tests {
    use super::*;
    use temper_substrate::scenario::model::Seed;
    use temper_substrate::scenario::{bootseed, loader};

    const CHARTER_SEED_YAML: &str = r#"
name: charter-select-test
cogmap:
  telos:
    title: "Charter select test"
    statement: "Read the telos charter through the composed read."
    questions:
      - question: "Does the composed read preserve seq order?"
        context: "Statement, then questions, then framing, in that order."
      - question: "Does a bare question round-trip verbatim?"
    framing:
      - "Framing block one."
  owner: alice
  emitter: "charter-agent#1"
world:
  profiles: [{ handle: alice, display_name: Alice, system_access: approved }]
  entities: [{ name: "charter-agent#1", profile: alice }]
resources: []
uses_lenses: [telos-default]
"#;

    #[sqlx::test(migrations = "../../migrations")]
    async fn returns_blocks_in_seq_order_and_gates_by_readability(pool: PgPool) {
        bootseed::seed_system(&pool).await.expect("seed_system");

        let seed: Seed = serde_yaml::from_str(CHARTER_SEED_YAML).expect("parse seed yaml");
        let loaded = loader::load_seed(&pool, &seed).await.expect("load_seed");
        let owner = ProfileId::from(loaded.owner);

        let blocks = cogmap_charter_select(&pool, owner, loaded.cogmap)
            .await
            .expect("readable charter select");
        assert_eq!(
            blocks.len(),
            4,
            "statement + 2 questions + 1 framing: {blocks:?}"
        );
        assert_eq!(blocks[0].role, "statement");
        assert_eq!(
            blocks[0].body,
            "Read the telos charter through the composed read."
        );
        assert_eq!(blocks[1].role, "question");
        assert_eq!(
            blocks[1].body,
            "Does the composed read preserve seq order?\n\nStatement, then questions, then framing, in that order."
        );
        assert_eq!(blocks[2].role, "question");
        assert_eq!(blocks[2].body, "Does a bare question round-trip verbatim?");
        assert_eq!(blocks[3].role, "framing");
        assert_eq!(blocks[3].body, "Framing block one.");
        assert!(
            blocks.windows(2).all(|w| w[0].seq < w[1].seq),
            "blocks come back in strictly increasing seq order: {blocks:?}"
        );

        // A profile with no ownership/grant on the telos resource is denied by the in-SQL gate.
        let outsider: uuid::Uuid = sqlx::query_scalar(
            "INSERT INTO kb_profiles (handle, display_name, email) VALUES ($1, $1, $1) RETURNING id",
        )
        .bind("outsider@example.com")
        .fetch_one(&pool)
        .await
        .expect("insert outsider profile");

        let denied = cogmap_charter_select(&pool, ProfileId::from(outsider), loaded.cogmap)
            .await
            .expect("gate denial is empty, not an error");
        assert!(
            denied.is_empty(),
            "non-owner must see no charter blocks: {denied:?}"
        );
    }
}
