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
use temper_core::types::api::{
    SearchDiagnostics, SearchParams, SearchReason, SearchResponse, SearchScope,
    UnifiedSearchResultRow,
};
use temper_core::types::cognitive_maps::{
    CharterBlock, CogmapAnalyticsRow, CogmapRegionMetricsRow, CogmapRegionRow, CogmapRegulationRow,
    CogmapStaleness,
};
use temper_core::types::home::HomeAnchor;
use temper_core::types::ids::{CogmapId, ContextId, LensId, ProfileId, ResourceId};
use temper_core::types::invocation::{
    Disposition, InvocationActRow, InvocationSummary, InvocationView,
};
use temper_core::types::provenance::BlockProvenanceRow;
use temper_substrate::readback;
use temper_workflow::types::managed_meta::{
    ManagedMeta, ResourceMetaListResponse, ResourceMetaResponse,
};
use temper_workflow::types::resource::{
    ContentResponse, ResourceDetail, ResourceFacets, ResourceRow, ResourceSortField, SortOrder,
};

fn api_err(e: impl std::fmt::Display) -> ApiError {
    ApiError::from(TemperError::Api(e.to_string()))
}

/// Tag an internal search fault with the stage it came from (issue #427), so a generic surface error
/// names the failing stage (`unified_search` — the blended FTS/vector/graph query — vs `enrichment` —
/// per-row display assembly) instead of collapsing to one opaque "Error occurred during tool
/// execution" string. Both surfaces (`temper-api`, `temper-mcp`) format the resulting `ApiError`
/// message verbatim, so the stage rides through to the client. Caller-facing 400/404s from scope
/// resolution (a bad `context_ref`) return upstream and never pass through here — they are already
/// self-describing. The embed stage cannot reach here: it degrades to FTS + graph rather than erroring.
fn search_stage_err(stage: &str, e: impl std::fmt::Display) -> ApiError {
    api_err(format!("search stage={stage}: {e}"))
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
    // Cognitive-map scope: a CSV of cogmap UUIDs (the list GET can't carry a Vec over the query
    // string). Parse to a `uuid[]`; the WHERE below filters on the resource's home anchor. A single
    // home per resource means this composes with `context_ref` only to the empty set (no resource is
    // homed in both), which the CLI guards against — here it is simply an honest empty result.
    let cogmap_ids: Option<Vec<Uuid>> = match params.cogmap_ids.as_deref() {
        Some(csv) if !csv.trim().is_empty() => Some(
            csv.split(',')
                .map(|s| {
                    Uuid::parse_str(s.trim()).map_err(|e| {
                        ApiError::BadRequest(format!("invalid cogmap id {:?}: {e}", s.trim()))
                    })
                })
                .collect::<ApiResult<Vec<_>>>()?,
        ),
        _ => None,
    };
    // `context_ref` and a cogmap scope name two different homes — reject the pair server-side, exactly
    // as `resolve_search_scope` does for search. The CLI/MCP already guard it; this closes the raw-HTTP
    // gap where the combination silently composed to the empty set instead of a 400.
    if context_id.is_some() && cogmap_ids.is_some() {
        return Err(ApiError::BadRequest(
            "context_ref and cogmap scope are mutually exclusive".into(),
        ));
    }
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
            -- An interrupted segmented ingest is NOT a document. It is excluded from list (and from
            -- search, in `unified_search`'s corpus CTE) until `resource_finalize` says the last block
            -- landed. It stays addressable and readable via `show`, which reports `ingest_state`.
            --
            -- Deliberately HERE and not in `resources_visible_to`: visibility is an *authorization*
            -- predicate, completeness is a *content* predicate. Folding one into the other would
            -- quietly change who can see what.
            AND r.ingest_state = 'complete'
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
            AND ($10::uuid[] IS NULL OR (h.anchor_table = 'kb_cogmaps' AND h.anchor_id = ANY($10)
                 AND cogmap_readable_by_profile($1, h.anchor_id)))
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
        .bind(cogmap_ids)
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

/// `show_detail` — one resource with both metadata tiers.
///
/// Composes the two existing readbacks rather than introducing a joined query: that keeps
/// this free of a new `sqlx::query!` macro (and therefore of the `.sqlx` cache regeneration
/// ritual). Two round-trips for a single resource is not an N+1.
///
/// Visibility is gated by `native_resource_row` (WS2); `get_meta_select` re-gates through
/// `readback::meta`, so an unreadable resource 404s before either tier is assembled.
///
/// This is the composition `temper-mcp`'s `get_resource` performed inline.
pub async fn show_detail_select(
    pool: &PgPool,
    profile_id: ProfileId,
    id: ResourceId,
) -> ApiResult<ResourceDetail> {
    let row = native_resource_row(pool, profile_id, id)
        .await
        .map_err(ApiError::from)?;
    let meta = get_meta_select(pool, profile_id, id).await?;

    Ok(ResourceDetail {
        row,
        managed_meta: meta.managed_meta,
        open_meta: meta.open_meta,
    })
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
///
/// Carries no meta hashes: `managed_hash`/`open_hash` were §7-dissolved and had been
/// emitted as empty strings ever since, so the fields were removed rather than kept as two
/// permanently-meaningless keys. The real `body_hash` lives on `ResourceRow`.
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
        id: ResourceId::from(new_id),
        managed_meta: Some(managed),
        open_meta: Some(serde_json::Value::Object(rb.open)),
    })
}

/// `list_meta` — the `?meta_only=true` projection. Same WS2-scoped, filtered + sorted + paginated set as
/// `list` (`filtered_visible_page`); each page id maps to a full [`ResourceDetail`] via `show_detail_select`
/// (row + both meta tiers — the whole per-resource view minus the body). `total`/`facets` mirror `list`
/// (the FILTERED set).
pub async fn list_meta_select(
    pool: &PgPool,
    profile_id: ProfileId,
    params: ResourceListParams,
) -> ApiResult<ResourceMetaListResponse> {
    let page = filtered_visible_page(pool, profile_id, &params).await?;
    let mut out = Vec::with_capacity(page.page_ids.len());
    for new_id in page.page_ids {
        out.push(show_detail_select(pool, profile_id, ResourceId::from(new_id)).await?);
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

/// Resolve the scope selectors (§6) into the corpus bounds for `unified_search`: an optional context id
/// (Surface A `context_ref`) and an optional explicit scope-id set (Surface B — `cogmap_id` single-map,
/// or the `wayfind` region-salience funnel). An empty scope-id set is the deny case — it yields zero
/// rows downstream via `c.id = ANY('{}')`, never an error ("no view from nowhere", spec §5/§7).
///
/// `context_ref` and `cogmap_id` remain mutually exclusive — they name two different homes, and asking
/// for both is incoherent. But **`wayfind` now composes with either** (spec §3.7): since wayfind pools
/// regions over both anchor kinds, `--context X --wayfind` is no longer a contradiction, it means
/// *"wayfind within this context"* — the anchor scopes the region pool. That composition is the whole
/// point of T7, so the old three-way exclusion is gone.
async fn resolve_search_scope(
    pool: &PgPool,
    profile_id: ProfileId,
    params: &SearchParams,
) -> ApiResult<(Option<uuid::Uuid>, Option<Vec<Uuid>>)> {
    // Effective cogmap set: the plural `cogmap_ids` wins when present; otherwise a scalar `cogmap_id`
    // is a one-element set. Empty ⇒ no cogmap scope. Reconciling the two wire fields ONCE, here, means
    // every downstream check (exclusion, wayfind anchor, scope resolution) sees a single shape.
    let effective_cogmaps: Vec<Uuid> = match params.cogmap_ids.as_deref() {
        Some(ids) if !ids.is_empty() => ids.to_vec(),
        _ => params.cogmap_id.into_iter().collect(),
    };

    if params.context_ref.is_some() && !effective_cogmaps.is_empty() {
        return Err(ApiError::BadRequest(
            "context_ref and cogmap scope are mutually exclusive".into(),
        ));
    }
    // Wayfind anchors on a SINGLE home, so a multi-map wayfind has no anchor to pool regions within.
    // The CLI rejects it pre-flight; reject it here too so a raw caller gets a 400 rather than a
    // silent global wayfind with the cogmap set dropped.
    if params.wayfind && effective_cogmaps.len() > 1 {
        return Err(ApiError::BadRequest(
            "wayfind anchors on a single map; supply at most one cogmap with wayfind".into(),
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

    // `wayfind` runs the region-salience funnel over every visible anchor of both kinds; the cogmap
    // set alone is the (single- OR multi-)map scope. Both visibility-gate inside the SQL; an empty
    // result is deny → zero rows, never an error.
    let scope_ids: Option<Vec<Uuid>> = if params.wayfind {
        // A named anchor scopes the region pool to itself ("wayfind within this context/cogmap").
        // Wayfind's anchor is a single home, so only a one-element cogmap set anchors it; a multi-map
        // wayfind is rejected client-side, and a >1 set here (defensive) pools every visible anchor.
        // The context/cogmap exclusion above guarantees at most one kind is set.
        let single_cogmap = match effective_cogmaps.as_slice() {
            [one] => Some(*one),
            _ => None,
        };
        let anchor = match (context_id, single_cogmap) {
            (Some(ctx), _) => Some(HomeAnchor::Context(ContextId::from(ctx))),
            (_, Some(map)) => Some(HomeAnchor::Cogmap(CogmapId::from(map))),
            (None, None) => None,
        };
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
                    anchor,
                },
            )
            .await
            .map_err(api_err)?,
        )
    } else if !effective_cogmaps.is_empty() {
        // Union of each map's homed, visible participants — one round-trip. Each map is independently
        // map-read gated inside `cogmap_scope_ids`, so an unreadable map in the set adds nothing.
        Some(
            sqlx::query_scalar!(
                "SELECT cogmap_scope_ids_multi($1, $2)",
                profile_id.uuid(),
                &effective_cogmaps
            )
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

/// The wall-clock budget for a single server-side query embed, in milliseconds. Overridable via
/// `TEMPER_QUERY_EMBED_BUDGET_MS` for targets whose serverless function deadline differs from the
/// 8s default (issue #427). A non-numeric or zero value falls back to the default rather than
/// disabling the guard — an unbounded embed is exactly the failure mode this protects against.
const DEFAULT_QUERY_EMBED_BUDGET_MS: u64 = 8_000;

fn query_embed_budget() -> std::time::Duration {
    let ms = std::env::var("TEMPER_QUERY_EMBED_BUDGET_MS")
        .ok()
        .and_then(|s| s.trim().parse::<u64>().ok())
        .filter(|&ms| ms > 0)
        .unwrap_or(DEFAULT_QUERY_EMBED_BUDGET_MS);
    std::time::Duration::from_millis(ms)
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
/// **The embed runs off the async executor, under a wall-clock budget (issue #427).** ONNX inference
/// is CPU-bound and a cold serverless instance also pays a one-time model load (ORT init + writing
/// the bundled runtime to `/tmp` + reading the quantized model); running it inline on the request's
/// executor thread both blocks that thread and can hold the whole invocation past the function's
/// deadline, at which point the platform kills the request and the client sees a generic "execution
/// error" with no results. So we hand it to `spawn_blocking` and race it against `query_embed_budget()`.
///
/// Failure mode is fallback-with-warn: on embed error, task panic, OR budget timeout we log and
/// proceed with FTS + graph only rather than turning a soft degradation into a killed request —
/// partial results beat none, and the `degraded` signal is surfaced in [`SearchDiagnostics`] (#360).
///
/// Returns `true` when a query needed server-side embedding but it *did not land* (error / panic /
/// timeout — the "degraded" case); `false` when the vector signal is intact — the query already
/// carried an embedding, there was nothing to embed, or the embed succeeded within budget.
async fn embed_query_if_missing(params: &mut SearchParams) -> bool {
    if params.embedding.is_some() {
        return false;
    }
    let query = match params.query.as_deref().map(str::trim) {
        Some(q) if !q.is_empty() => q.to_string(),
        _ => return false,
    };
    let budget = query_embed_budget();
    let embed = tokio::task::spawn_blocking(move || temper_ingest::embed::embed_text(&query));
    match tokio::time::timeout(budget, embed).await {
        Ok(Ok(Ok(embedding))) => {
            params.embedding = Some(embedding);
            false
        }
        Ok(Ok(Err(e))) => {
            tracing::warn!(
                error = %e,
                "server-side query embedding failed; falling back to FTS + graph only"
            );
            true
        }
        Ok(Err(join_err)) => {
            tracing::warn!(
                error = %join_err,
                "server-side query embedding task panicked; falling back to FTS + graph only"
            );
            true
        }
        Err(_elapsed) => {
            tracing::warn!(
                budget_ms = budget.as_millis(),
                "server-side query embedding exceeded its budget; falling back to FTS + graph only"
            );
            true
        }
    }
}

/// Classify the scope selector of a search request (issue #360). `Global` is the no-selector default.
///
/// `wayfind` wins the precedence deliberately: since T7 it composes with `context_ref`/`cogmap_id`
/// (spec §3.7), and when it is set it is the funnel that *produced* the corpus — which is what this
/// field exists to report. The anchor merely scoped that funnel.
///
/// **[`SearchScope`] is deliberately NOT widened** to carry the anchor. It is a wire enum — it rides
/// the `x-temper-search-diagnostics` header, `openapi.json`, the generated TS types, and the temper-rb
/// gem, which **`raise`s on an enum value it does not know** (`search_scope.rb:39`). A new variant is
/// therefore a hard-fail break for an older client, and it would buy nothing: a caller already knows
/// which anchor it asked for, and diagnostics exist to explain the *result shape*, not to echo the
/// request back. If the anchor is ever genuinely needed here, add an *optional* field to
/// [`SearchDiagnostics`] rather than a variant.
fn classify_scope(params: &SearchParams) -> SearchScope {
    if params.wayfind {
        SearchScope::Wayfind
    } else if params.cogmap_id.is_some()
        || params
            .cogmap_ids
            .as_deref()
            .is_some_and(|ids| !ids.is_empty())
    {
        // Single- or multi-map — both are `Cogmap` scope (the enum is a frozen wire contract; the
        // plural does not earn a new variant, only a wider corpus).
        SearchScope::Cogmap
    } else if params.context_ref.is_some() {
        SearchScope::Context
    } else {
        SearchScope::Global
    }
}

/// Build the agent-facing one-liner for a search's [`SearchDiagnostics`] (issue #360). Pure — no DB —
/// so it is unit-testable. Returns `None` only when the result set is unremarkable (`Ok` reason and
/// no degraded signal); otherwise it explains the shape and suggests a concrete next step.
fn search_hint(
    scope: SearchScope,
    reason: SearchReason,
    scope_size: Option<i64>,
    degraded: bool,
) -> Option<String> {
    // The `WAYFIND_UNREACHABLE` hint that used to live here is GONE, because it stopped being true
    // (spec §3.7). It told agents that "wayfind only reaches cogmap-distilled content — if what you
    // want is context-homed (in no cogmap), it is unreachable here regardless of phrasing." As of T7
    // wayfind pools regions over BOTH anchor kinds, so context-homed content is reachable, and a
    // `NoMatch` under wayfind now means what it means everywhere else: rephrase or widen. Leaving the
    // hint in would have actively taught agents to stop asking for the thing that now works.
    let mut parts: Vec<String> = Vec::new();
    match (scope, reason) {
        (SearchScope::Wayfind, SearchReason::OutOfScope) => parts.push(
            "wayfind scope is empty: 0 candidate resources across the anchors you can see. Drop \
             `--wayfind` for an unscoped search."
                .to_string(),
        ),
        (SearchScope::Cogmap, SearchReason::OutOfScope) => parts.push(
            "this cogmap admits 0 resources you can see — check the cogmap ref, or try \
             `--context <ref>`."
                .to_string(),
        ),
        (_, SearchReason::NoMatch) => {
            let prefix = scope_size
                .map(|n| format!("{n} candidate resource(s) in scope; "))
                .unwrap_or_default();
            parts.push(format!(
                "{prefix}nothing matched the query — try rephrasing or a broader scope."
            ));
        }
        // `Ok` (any scope) and the impossible `OutOfScope` for Global/Context need no reason hint.
        _ => {}
    }
    if degraded {
        parts.push(
            "vector ranking was unavailable (server-side embedding failed); results are FTS + \
             graph only."
                .to_string(),
        );
    }
    if parts.is_empty() {
        None
    } else {
        Some(parts.join(" "))
    }
}

/// `search` — Surface A general search (Beat 2): one composed `unified_search` readback blending FTS +
/// vector + graph into ranked, scored hits, then per-row display enrichment. Replaces the either/or,
/// zero-score path. Visibility is enforced inside every candidate function (`resources_visible_to`).
pub async fn search_select(
    pool: &PgPool,
    profile_id: ProfileId,
    mut params: SearchParams,
) -> ApiResult<SearchResponse> {
    let degraded = embed_query_if_missing(&mut params).await;
    let scope = classify_scope(&params);
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
            seed_only: params.seed_only,
        },
    )
    .await
    .map_err(|e| search_stage_err("unified_search", e))?;

    let mut out = Vec::with_capacity(hits.len());
    for h in hits {
        // Enrich every hit through the cogmap-aware `native_resource_row` (Task F:
        // `readback::resource_row` LEFT-JOINs kb_contexts AND kb_cogmaps and is visibility-gated).
        // Context-homed hits carry `context_*`; cogmap-homed hits carry `cogmap_*`. `home_display`
        // surfaces whichever home is set, so a `--cogmap` hit renders the map name rather than null.
        let row = native_resource_row(pool, profile_id, h.resource_id)
            .await
            .map_err(|e| search_stage_err("enrichment", e))?;
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

    // Scope-stage diagnostics (issue #360): let an agent distinguish "rephrase the query" from
    // "this scope can never see that content." `scope_size` is only cheaply knowable for the
    // bounded id-set selectors (wayfind/cogmap); an *empty* set there is the structurally-out-of-
    // scope case, never a rephrase problem.
    let scope_size: Option<i64> = match scope {
        SearchScope::Wayfind | SearchScope::Cogmap => {
            scope_ids.as_ref().map(|ids| ids.len() as i64)
        }
        SearchScope::Global | SearchScope::Context => None,
    };
    let matched = out.len() as i64;
    let reason =
        if matches!(scope, SearchScope::Wayfind | SearchScope::Cogmap) && scope_size == Some(0) {
            SearchReason::OutOfScope
        } else if matched == 0 {
            SearchReason::NoMatch
        } else {
            SearchReason::Ok
        };
    let hint = search_hint(scope, reason, scope_size, degraded);

    Ok(SearchResponse {
        results: out,
        // Always populated server-side; `None` is reserved for the client's old-server degrade path.
        diagnostics: Some(SearchDiagnostics {
            scope,
            scope_size,
            matched,
            reason,
            degraded,
            hint,
        }),
    })
}

/// `anchor_shape` — the surface-tier read of an anchor's materialized regions, for a context OR a
/// cogmap (spec §3.7, T8). Service-direct (reads bypass the Backend trait). The access gate lives in
/// the SQL function: a principal who cannot read the anchor gets an empty vec, never an error — and
/// for a context that gate is `context_readable_by_profile`, so a context read-grant grants this read
/// by construction rather than by a second hand-rolled check. Maps the substrate row to the wire type.
pub async fn anchor_shape_select(
    pool: &PgPool,
    profile_id: ProfileId,
    anchor: HomeAnchor,
    lens_id: Option<uuid::Uuid>,
) -> ApiResult<Vec<CogmapRegionRow>> {
    let rows = readback::anchor_shape(pool, anchor, profile_id, lens_id.map(LensId::from))
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

/// `anchor_region_metrics` — the per-region analytics tier, for either anchor kind (T8).
/// Service-direct; gate is in the SQL (deny → empty). Maps the substrate row to the wire type.
pub async fn anchor_region_metrics_select(
    pool: &PgPool,
    profile_id: ProfileId,
    anchor: HomeAnchor,
    lens_id: Option<uuid::Uuid>,
) -> ApiResult<Vec<CogmapRegionMetricsRow>> {
    let rows = readback::anchor_region_metrics(pool, anchor, profile_id, lens_id.map(LensId::from))
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
    // There is no `disposition` column: `invocation_close` writes the disposition into
    // `status` (`outcome` holds only the caller's opaque payload, despite what the comment
    // on `kb_invocations.outcome` in the canonical schema migration claims). Derive it here,
    // before `status` is moved below. `open` is the absence of a disposition, not an error;
    // any other unparseable value means the DB CHECK's invariant broke, so it propagates as
    // an error rather than silently degrading to `None`.
    let disposition = match row.status.as_str() {
        "open" => None,
        terminal => Some(Disposition::try_from(terminal).map_err(api_err)?),
    };
    Ok(Some(InvocationView {
        id: row.id,
        status: row.status,
        disposition,
        trigger_kind: row.trigger_kind,
        originating_cogmap_id: row.originating_cogmap_id,
        parent_cogmap_id: row.parent_cogmap_id,
        scoped_entity_id: row.scoped_entity_id,
        telos_resource_id: row.telos_resource_id,
        outcome: row.outcome,
        opened_at: row.opened_at,
        closed_at: row.closed_at,
        correlation_id: row.correlation_id,
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
            correlation_id: r.correlation_id,
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
    #[tokio::test]
    async fn embed_query_noop_when_embedding_already_present() {
        let precomputed = vec![0.5_f32; 768];
        let mut p = SearchParams {
            query: Some("kubernetes deploys".into()),
            embedding: Some(precomputed.clone()),
            ..SearchParams::default()
        };
        let degraded = embed_query_if_missing(&mut p).await;
        assert!(
            !degraded,
            "a precomputed embedding is not a degraded signal"
        );
        assert_eq!(
            p.embedding.as_deref(),
            Some(precomputed.as_slice()),
            "a precomputed embedding (CLI path) must pass through untouched"
        );
    }

    #[tokio::test]
    async fn embed_query_noop_when_query_is_none_or_blank() {
        for q in [None, Some(String::new()), Some("   \t\n".into())] {
            let mut p = SearchParams {
                query: q.clone(),
                embedding: None,
                ..SearchParams::default()
            };
            let degraded = embed_query_if_missing(&mut p).await;
            assert!(
                !degraded && p.embedding.is_none(),
                "empty/whitespace/absent query must not embed and is not degraded (query={q:?})"
            );
        }
    }

    #[test]
    fn classify_scope_picks_the_active_selector() {
        use temper_core::types::api::SearchScope;
        let global = SearchParams::default();
        assert_eq!(classify_scope(&global), SearchScope::Global);

        let ctx = SearchParams {
            context_ref: Some("@me/temper".into()),
            ..SearchParams::default()
        };
        assert_eq!(classify_scope(&ctx), SearchScope::Context);

        let cog = SearchParams {
            cogmap_id: Some(uuid::Uuid::now_v7()),
            ..SearchParams::default()
        };
        assert_eq!(classify_scope(&cog), SearchScope::Cogmap);

        let way = SearchParams {
            wayfind: true,
            ..SearchParams::default()
        };
        assert_eq!(classify_scope(&way), SearchScope::Wayfind);
    }

    #[test]
    fn hint_out_of_scope_wayfind_suggests_dropping_wayfind() {
        // T7: the useful escape hatch changed. An empty wayfind scope means zero candidates across
        // EVERY visible anchor — contexts now included — so steering at `--context` (as this hint used
        // to) is advice that cannot help: those regions were already pooled. Dropping `--wayfind` for
        // an unscoped search is the move that can actually widen the corpus.
        let h = search_hint(
            SearchScope::Wayfind,
            SearchReason::OutOfScope,
            Some(0),
            false,
        )
        .expect("out-of-scope wayfind must hint");
        assert!(h.contains("wayfind scope is empty"), "got: {h}");
        assert!(
            h.contains("--wayfind"),
            "must offer dropping --wayfind; got: {h}"
        );
    }

    #[test]
    fn hint_out_of_scope_cogmap_mentions_cogmap() {
        let h = search_hint(
            SearchScope::Cogmap,
            SearchReason::OutOfScope,
            Some(0),
            false,
        )
        .expect("out-of-scope cogmap must hint");
        assert!(h.contains("cogmap"), "got: {h}");
    }

    #[test]
    fn hint_no_match_reports_scope_size_when_known() {
        let with = search_hint(SearchScope::Cogmap, SearchReason::NoMatch, Some(7), false)
            .expect("no-match must hint");
        assert!(with.contains('7'), "should surface scope_size; got: {with}");
        assert!(
            with.contains("rephras"),
            "should suggest rephrasing; got: {with}"
        );

        let without = search_hint(SearchScope::Global, SearchReason::NoMatch, None, false)
            .expect("no-match must hint even without scope_size");
        assert!(without.contains("matched"), "got: {without}");
    }

    #[test]
    fn hint_wayfind_no_match_no_longer_claims_context_content_is_unreachable() {
        // T7 INVERTS this test. It used to require the hint to say that context-homed content is
        // "unreachable" via wayfind and to steer the caller to `--context`. Wayfind now pools context
        // regions too (spec §3.7), so that guidance is false — and false guidance is worse than none:
        // it would teach an agent to stop asking for the thing that now works. A wayfind `NoMatch` is
        // now an ordinary no-match, and falls through to the generic rephrase-or-widen hint.
        let h = search_hint(SearchScope::Wayfind, SearchReason::NoMatch, Some(3), false)
            .expect("wayfind no-match must still hint");
        assert!(h.contains('3'), "should surface scope_size; got: {h}");
        assert!(
            !h.contains("unreachable"),
            "the WAYFIND_UNREACHABLE claim must be gone — context-homed content is reachable now; \
             got: {h}"
        );
    }

    #[test]
    fn hint_ok_is_silent_unless_degraded() {
        assert!(
            search_hint(SearchScope::Global, SearchReason::Ok, None, false).is_none(),
            "an unremarkable Ok result needs no hint"
        );
        let degraded = search_hint(SearchScope::Global, SearchReason::Ok, None, true)
            .expect("a degraded Ok result must still warn");
        assert!(degraded.contains("vector ranking"), "got: {degraded}");
    }

    #[test]
    fn hint_appends_degraded_to_a_reason() {
        let h = search_hint(
            SearchScope::Wayfind,
            SearchReason::OutOfScope,
            Some(0),
            true,
        )
        .expect("hint");
        assert!(h.contains("wayfind scope is empty"), "got: {h}");
        assert!(
            h.contains("vector ranking"),
            "degraded note must append; got: {h}"
        );
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
