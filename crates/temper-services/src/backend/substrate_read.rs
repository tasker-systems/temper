//! Substrate read dispatcher — the service-direct read paths (list / show / get_content / get_meta /
//! search) over the one schema.
//!
//! These reads bypass the `Backend` trait by design (the trait projections are lossy and don't cover
//! meta/body/content); they resolve against `temper_substrate::readback`. The resource read paths
//! answer in [`ResourceView`] — the one shape a resource has — assembled from the batched
//! `readback::hit_identities` and then given whatever [`SectionSet`] the caller asked for by
//! `fill_sections`. Visibility is scoped to the caller's profile (WS2) — the readbacks gate
//! through `resources_visible_to`. SQL is unqualified against the one schema (the connection
//! carries the search_path).
//!
//! `list` filters (context_ref/doc_type_name/stage/status/tags/owner/`q`-title), sorts, and paginates the
//! visible set (`filtered_visible_page`), reconstructing only the page; `context_ref` is resolved
//! to a context UUID before filtering so bare names are rejected (spec Decision 1). Full-text/vector `q`
//! on the list endpoint is search's job (a named deferral) — list `q` is a trivial title `ILIKE`.
//! Most filters are SQL predicates; `doc_type_name`/`stage`/`status` are applied Rust-side because
//! each publishes a facet histogram that must be computed WITHOUT its own predicate (see
//! `ResourceFacets`) — a predicate applied in SQL is one the histogram can no longer see past.

use std::collections::HashMap;

use sqlx::{PgPool, Row};
use uuid::Uuid;

use crate::backend::db_backend::map_readback_err;
use crate::error::{ApiError, ApiResult};
use crate::services::context_service::resolve_context_ref;
use crate::services::resource_service::{ResourceListParams, ResourceListResponse};
use temper_core::context_ref::parse_context_ref;
use temper_core::error::TemperError;
use temper_core::types::api::{
    ExactArm, ExactHit, SearchParams, SearchReason, SearchResponse, SearchScope, SearchScopeInfo,
    WideArm, WideHit,
};
use temper_core::types::cognitive_maps::{
    CharterBlock, CogmapAnalyticsRow, CogmapRegionMetricsRow, CogmapRegionRow, CogmapRegulationRow,
    CogmapStaleness,
};
use temper_core::types::home::HomeAnchor;
use temper_core::types::ids::{CogmapId, ContextId, DataArtifactId, LensId, ProfileId, ResourceId};
use temper_core::types::invocation::{
    Disposition, InvocationActRow, InvocationSummary, InvocationView,
};
use temper_core::types::provenance::BlockProvenanceRow;
use temper_core::types::resource_view::{ResourceSection, ResourceView, SectionSet};
use temper_substrate::readback;
use temper_workflow::types::resource::{
    ContentResponse, ResourceFacets, ResourceSortField, SortOrder,
};

fn api_err(e: impl std::fmt::Display) -> ApiError {
    ApiError::from(TemperError::Api(e.to_string()))
}

/// Tag an internal search fault with the stage it came from (issue #427), so a generic surface error
/// names the failing stage (`search_exact` or `search_wide` — the two arms, which fail independently
/// — vs `enrichment`, per-row display assembly) instead of collapsing to one opaque "Error occurred
/// during tool execution" string. Both surfaces (`temper-api`, `temper-mcp`) format the resulting `ApiError`
/// message verbatim, so the stage rides through to the client. Caller-facing 400/404s from scope
/// resolution (a bad `context_ref`) return upstream and never pass through here — they are already
/// self-describing. The embed stage cannot reach here: it degrades to FTS + graph rather than erroring.
fn search_stage_err(stage: &str, e: impl std::fmt::Display) -> ApiError {
    api_err(format!("search stage={stage}: {e}"))
}

/// One page of the filtered, visible resource set: the page's substrate ids (already
/// sorted + paginated), the FILTERED total (before limit/offset), and the three facet
/// histograms, each computed WITHOUT its own predicate (see [`ResourceFacets`]).
///
/// `offset`/`limit` are the values this page was actually cut with, not the caller's raw
/// params — a negative offset is floored at 0 and a negative limit means "no limit". The
/// list envelope reports them, so they are returned rather than re-normalized there:
/// two derivations of "the effective page" would drift, and the reported one would be
/// the one that is not the truth.
struct VisiblePage {
    page_ids: Vec<Uuid>,
    total: i64,
    facets: ResourceFacets,
    offset: i64,
    limit: Option<i64>,
}

/// One scanned row of the visible set, decoded once for the filter + histogram passes below.
///
/// The three histograms and the kept set each walk the same rows looking at the same three
/// columns; `sqlx::Row::get` re-decodes (and re-allocates) a `String` on every call, so reading
/// them straight off the `PgRow` four times would decode the whole visible set four times. This is
/// the decode, held once.
struct ScannedRow {
    id: Uuid,
    doc_type: Option<String>,
    stage: Option<String>,
    status: Option<String>,
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

/// Receive-side half of the `--status` validation, as a 400.
///
/// The predicate is `temper_workflow::schema::validate_goal_status`, shared with the
/// CLI's send-side check — one definition, so the door an agent uses and the door a human
/// uses cannot disagree about which statuses exist. Only the error mapping is local.
fn validate_goal_status(value: &str) -> ApiResult<()> {
    temper_workflow::schema::validate_goal_status(value)
        .map_err(|e| ApiError::BadRequest(e.to_string()))
}

/// Resolve the visible set, apply the `ResourceListParams` filters (context_ref /
/// doc_type_name / stage / status / tags / owner / `q` title-match) + sort + pagination, and
/// return only the page's ids (so the caller reconstructs the page, not every visible
/// row — this also fixes the prior all-rows N+1).
///
/// **`doc_type_name`, `stage` and `status` are the three filters that are NOT SQL predicates.**
/// Each of them publishes a facet histogram, and a histogram is only useful to a caller choosing
/// what to select next if it counts the options it did NOT select — which means it must be
/// computed over the set filtered by the OTHER two. Filtering in SQL destroys exactly that
/// information: it was the doc_type predicate's presence in the WHERE clause that collapsed
/// `facets.doc_type` to the single key the caller had already chosen, leaving a browse UI unable
/// to show or reach any other kind. So the query returns the set narrowed by every filter that has
/// no histogram (`context_ref`, `owner`, `q`, `goal`, `tags`, `cogmap_ids`), carrying `doc_type`,
/// `stage` and `status` as columns, and the four passes below cut the three histograms and the
/// kept set out of it.
///
/// This costs **no extra statement** — the rows were already being fetched in full and paginated
/// Rust-side (see the `skip`/`take` at the end), so this reuses the incumbent shape rather than
/// introducing one. `list_page_query_count_test` is the guard that it stays one query per page: a
/// histogram computed by a second probe query would pass this function's own tests and fail that.
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
    // as `resolve_search_anchor` does for search. The CLI/MCP already guard it; this closes the raw-HTTP
    // gap where the combination silently composed to the empty set instead of a 400.
    if context_id.is_some() && cogmap_ids.is_some() {
        return Err(ApiError::BadRequest(
            "context_ref and cogmap scope are mutually exclusive".into(),
        ));
    }
    // Reject a `status` outside the goal enum HERE rather than only in the CLI, so every
    // door gets it: the CLI's own check is a friendlier early error, but MCP and raw HTTP
    // reach this function directly. Without it an unknown value silently matches nothing,
    // which is the same family as the defect this filter was added to fix — a filter that
    // reports a confident empty page for what is really a typo.
    if let Some(status) = params.status.as_deref() {
        validate_goal_status(status)?;
    }
    // Tag filter: CSV → lowercased, trimmed, deduped `text[]`. The case fold lives HERE and only
    // here, so the CLI, MCP and raw HTTP cannot disagree about whether `CI` and `ci` are one tag.
    // (Production carries six pairs that differ only by case — authn/AuthN, ci/CI, cli/CLI,
    // authz/AuthZ, vercel/Vercel, pr-482/PR-482 — so this is a live distinction, not a hypothetical.)
    //
    // Deduping matters for the containment predicate below only as hygiene, but trimming does not:
    // `--tag a --tag b` arrives as "a,b" and a caller writing "a, b" must not be asking for " b".
    //
    // An empty CSV (or one that trims away entirely) is `None` — no filter — rather than an empty
    // array. `text[] @> '{}'` is TRUE for every row, so binding the empty array would turn a
    // caller's "" into a silent full scan; treating it as absent is the same outcome, honestly named.
    let tag_filter: Option<Vec<String>> = match params.tags.as_deref() {
        Some(csv) => {
            let mut tags: Vec<String> = csv
                .split(',')
                .map(|t| t.trim().to_lowercase())
                .filter(|t| !t.is_empty())
                .collect();
            tags.sort();
            tags.dedup();
            (!tags.is_empty()).then_some(tags)
        }
        None => None,
    };
    // Doc-type filter: CSV → a trimmed, empty-dropped set. Same trim/empty handling as the tag
    // parser above, and for the same reason — a caller writing "task, goal" is not asking for
    // " goal", and a CSV that trims away entirely is no filter rather than a filter that matches
    // nothing (absent and empty must not mean two different things at one door).
    //
    // A SET rather than a list because the semantics are UNION, not containment: a resource has
    // exactly one doc type, so ANDing two names could only ever match zero rows. This is the one
    // multi-value filter here that widens with each addition; `tags` narrows, because a resource
    // has many tags. Membership is exact and case-SENSITIVE, unchanged from the single-value SQL
    // predicate this replaces — doc-type names are a closed lowercase vocabulary, and folding case
    // here would be a second, unshared normalization the write path does not perform.
    //
    // An unknown name is deliberately kept, not rejected: it simply matches nothing, exactly as
    // before. Validating the vocabulary is a separate change with its own compatibility question.
    let doc_type_filter: Option<std::collections::HashSet<String>> =
        match params.doc_type_name.as_deref() {
            Some(csv) => {
                let names: std::collections::HashSet<String> = csv
                    .split(',')
                    .map(str::trim)
                    .filter(|n| !n.is_empty())
                    .map(str::to_string)
                    .collect();
                (!names.is_empty()).then_some(names)
            }
            None => None,
        };
    let sort = params.sort.unwrap_or_default();
    let dir = match params.order.unwrap_or_default() {
        SortOrder::Asc => "ASC",
        SortOrder::Desc => "DESC",
    };

    // INNER JOIN dt (every resource carries exactly one `doc_type` property, as in
    // `readback::reconstruct`); LEFT JOIN the `kb_resource_workflow_props` pivot view
    // (migration 20260709000002) for the optional workflow keys used by filters/sort.
    //
    // `stage`/`status` are SELECTED but not filtered on: they are the two columns whose predicates
    // moved to Rust, and their histograms need the values for rows the filter would have removed.
    // They come from the pivot view's LEFT JOIN, so a resource carrying no `temper-stage` /
    // `temper-status` yields NULL — which the passes below skip rather than counting under a
    // synthetic key.
    let sql = format!(
        "SELECT r.id AS id, dt.property_value #>> '{{}}' AS doc_type_name,
                wp.stage AS stage, wp.status AS status
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
            -- search, by the same predicate carried inline in `search_exact` and `search_wide`)
            -- until `resource_finalize` says the last block landed. It stays addressable and
            -- readable via `show`, which reports `ingest_state`.
            --
            -- Deliberately HERE and not in `resources_visible_to`: visibility is an *authorization*
            -- predicate, completeness is a *content* predicate. Folding one into the other would
            -- quietly change who can see what.
            AND r.ingest_state = 'complete'
            AND ($2::uuid IS NULL OR c.id = $2)
            -- The doc_type, stage and status predicates are DELIBERATELY ABSENT. Each publishes a
            -- facet histogram, and a histogram narrowed by its own predicate has exactly one key —
            -- the one already chosen — so a browse UI could neither show nor reach the
            -- alternatives. They are applied in Rust below, where each histogram is cut from the
            -- set narrowed by the other two. Every predicate that remains here has no histogram,
            -- so filtering it early is free.
            AND ($3::uuid IS NULL OR h.owner_profile_id = $3)
            AND ($4::text IS NULL OR p.handle = $4)
            AND ($5::text IS NULL OR r.title ILIKE '%' || $5 || '%')
            AND ($6::uuid IS NULL OR EXISTS (
                  SELECT 1 FROM kb_edges ge
                   WHERE ge.source_table = 'kb_resources' AND ge.source_id = r.id
                     AND ge.target_table = 'kb_resources' AND ge.target_id = $6
                     AND ge.edge_kind = 'leads_to' AND ge.label = $7
                     AND NOT ge.is_folded))
            AND ($8::uuid[] IS NULL OR (h.anchor_table = 'kb_cogmaps' AND h.anchor_id = ANY($8)
                 AND cogmap_readable_by_profile($1, h.anchor_id)))
            -- Tag filter. A correlated subquery in the same idiom as the goal filter above,
            -- rather than a join: tags live in `kb_properties` under the single key `tags`, not as
            -- a column on the `kb_resource_workflow_props` pivot (which holds one scalar per key).
            -- So this cannot be a sixth pivot column the way `status` was.
            --
            -- `text[] @> text[]` is containment, which IS the AND semantics: the row matches when
            -- its tag set contains every requested tag. Both sides are lowercased — the bind is
            -- folded in Rust, the row's tags by `lower(...)` here — so the match is
            -- case-insensitive with one fold on each side and no `lower()` on the bind at query
            -- time.
            --
            -- A resource with no `tags` property is excluded because the `coalesce` makes it the
            -- EMPTY ARRAY and `'{{}}' @> $9` is false — NOT because it is NULL.
            -- `[corrected — 2026-08-15, found in review]` This said it *aggregates to NULL, and
            -- `NULL @> $9` is NULL, so it is correctly excluded*. `array_agg` over zero rows IS
            -- NULL, but the `coalesce` sits inside this expression and never lets that NULL out, so
            -- the stated mechanism is not the one operating. The outcome was right and the reason
            -- was wrong, which is the dangerous combination: a reader who dropped the `coalesce`
            -- *because NULL is what excludes* would make every tag filter NULL and match nothing.
            -- Resources carrying `tags: []` aggregate to the same empty array and are likewise
            -- excluded for any non-empty filter.
            --
            -- **The shape rule is NOT restated here.** `[converged — 2026-08-15]` This carried the
            -- third of three encodings of what a `tags` value IS — the one that was Rust rather
            -- than SQL, and therefore invisible to every SQL-side test. It read the table directly
            -- and normalized inline, splitting a bare string on whitespace; the selection act's
            -- fragment carried a second copy of the same expression and the FTS projection a third
            -- that disagreed with both. `kb_property_elements` (`20260815000030`) is now the one
            -- definition, and this reads it, so a shape convention cannot be lost or wrongly
            -- inherited here (design §6.3). The view's `NOT is_folded` is the liveness predicate
            -- this used to spell itself.
            AND ($9::text[] IS NULL OR (
                  SELECT coalesce(array_agg(DISTINCT lower(pe.element #>> '{{}}')), '{{}}')
                    FROM kb_property_elements pe
                   WHERE pe.owner_table = 'kb_resources' AND pe.owner_id = r.id
                     AND pe.property_key = 'tags'
                ) @> $9)
          ORDER BY {sort_col} {dir}, r.id ASC",
        sort_col = sort_column_sql(sort),
    );

    // The goal filter matches the live `advances`→goal edge minted by the create/update
    // projection — same `edge_kind`='leads_to' and label (`GOAL_EDGE_LABEL`). They must agree.
    //
    // The binds are POSITIONAL: this sequence is `$1..$9` in order, and nothing checks the
    // correspondence at compile time — a bind dropped or added out of order silently applies one
    // caller's filter as another's. Read it against the WHERE clause above, one for one:
    // $1 profile_id, $2 context_id, $3 owner_self, $4 owner_handle, $5 q, $6 goal,
    // $7 GOAL_EDGE_LABEL, $8 cogmap_ids, $9 tag_filter.
    let rows = sqlx::query(&sql)
        .bind(profile_id)
        .bind(context_id)
        .bind(owner_self)
        .bind(owner_handle)
        .bind(params.q.as_deref())
        .bind(params.goal)
        .bind(super::db_backend::GOAL_EDGE_LABEL)
        .bind(cogmap_ids)
        .bind(tag_filter.as_deref())
        .fetch_all(pool)
        .await
        .map_err(api_err)?;

    // Decode once (see [`ScannedRow`]); the four passes below all read the same three columns.
    let scanned: Vec<ScannedRow> = rows
        .iter()
        .map(|row| ScannedRow {
            id: row.get("id"),
            doc_type: row.get("doc_type_name"),
            stage: row.get("stage"),
            status: row.get("status"),
        })
        .collect();

    // The three moved predicates, as closures — so each pass below states which ones it applies,
    // and the one it omits is visible at the call site rather than buried in a condition. A row
    // whose column is NULL fails a present filter (it is not the requested stage) but is admitted
    // by an absent one, which is the same behaviour `wp.stage = $N` had against a LEFT JOIN.
    let doc_type_ok = |row: &ScannedRow| match &doc_type_filter {
        Some(wanted) => row
            .doc_type
            .as_deref()
            .is_some_and(|dt| wanted.contains(dt)),
        None => true,
    };
    let stage_ok = |row: &ScannedRow| match params.stage.as_deref() {
        Some(wanted) => row.stage.as_deref() == Some(wanted),
        None => true,
    };
    let status_ok = |row: &ScannedRow| match params.status.as_deref() {
        Some(wanted) => row.status.as_deref() == Some(wanted),
        None => true,
    };

    // Each histogram omits its OWN predicate and applies the other two, so it counts the options
    // the caller did not pick — the counts a browse UI needs to offer them. `total`, below, is the
    // fully-filtered count and remains the number that describes the page.
    let mut doc_type_facets: HashMap<String, i64> = HashMap::new();
    for row in scanned.iter().filter(|r| stage_ok(r) && status_ok(r)) {
        if let Some(doc_type) = &row.doc_type {
            *doc_type_facets.entry(doc_type.clone()).or_insert(0) += 1;
        }
    }
    // A NULL stage/status is SKIPPED rather than counted under a synthetic key: "carries no
    // temper-stage" is not a stage a caller can select, so a bucket for it would offer a filter
    // value that does not exist. Most doc types carry neither key, so this is the common row.
    let mut stage_facets: HashMap<String, i64> = HashMap::new();
    for row in scanned.iter().filter(|r| doc_type_ok(r) && status_ok(r)) {
        if let Some(stage) = &row.stage {
            *stage_facets.entry(stage.clone()).or_insert(0) += 1;
        }
    }
    let mut status_facets: HashMap<String, i64> = HashMap::new();
    for row in scanned.iter().filter(|r| doc_type_ok(r) && stage_ok(r)) {
        if let Some(status) = &row.status {
            *status_facets.entry(status.clone()).or_insert(0) += 1;
        }
    }
    let facets = ResourceFacets {
        doc_type: doc_type_facets,
        stage: stage_facets,
        status: status_facets,
    };

    // The kept set: every predicate, in the SQL's ORDER BY order (a filter preserves order, so
    // pagination below is unaffected by where the filtering happens).
    let all_ids: Vec<Uuid> = scanned
        .iter()
        .filter(|r| doc_type_ok(r) && stage_ok(r) && status_ok(r))
        .map(|r| r.id)
        .collect();
    let total = all_ids.len() as i64;

    let offset = params.offset.unwrap_or(0).max(0);
    let limit = params.limit.filter(|l| *l >= 0);
    let page_ids: Vec<Uuid> = match limit {
        Some(limit) => all_ids
            .into_iter()
            .skip(offset as usize)
            .take(limit as usize)
            .collect(),
        None => all_ids.into_iter().skip(offset as usize).collect(),
    };

    Ok(VisiblePage {
        page_ids,
        total,
        facets,
        offset,
        limit,
    })
}

/// One page of [`ResourceView`]s plus the paging state the page was actually cut with.
///
/// The list read path's own shape, not a wire type: it carries the paging state the page was cut
/// with, which `ResourceListResponse::new` then turns into `returned`/`truncated`. The envelope is
/// assembled *from* this rather than the other way round, so the derivation has one home.
#[derive(Debug)]
pub struct ResourceViewPage {
    /// This page's rows, in the order `filtered_visible_page`'s ORDER BY produced.
    pub views: Vec<ResourceView>,
    /// The FILTERED match count — every row the filters admit, before `limit`/`offset`.
    pub total: i64,
    /// The doc-type/stage/status histograms. **Not** over the same set as `total`: each excludes
    /// its own filter predicate (see [`ResourceFacets`]), so they describe what is reachable from
    /// here while `total` describes the page itself.
    pub facets: ResourceFacets,
    /// The effective page size applied, or `None` for an uncapped page.
    pub limit: Option<i64>,
    /// The offset this page starts at; `0` when the caller sent none.
    pub offset: i64,
}

/// Fill onto a page of assembled views the sections the caller asked for, and only those.
///
/// The **one** mapping from a section to the read that serves it, so `list` and `show` cannot
/// disagree about what `body` or `open-meta` means. It takes a SLICE, not one view, so the page is
/// the unit of work: `show` calls it through `std::slice::from_mut`, which is this same batch at
/// size one rather than a second code path.
///
/// **`open-meta` costs ONE statement for the whole page**, via `readback::meta_batch`. The
/// predecessor of this function ran `readback::meta` per view — itself two statements, an
/// `ensure_visible` plus the read — so a 13-row page asking for the open tier cost **28 statements,
/// measured**, exactly the N+1 `hit_identities` had already been written to end one section over.
/// It is **3** now, and `list_page_open_meta_query_count_test` is what holds it there.
///
/// **`body` is per-view, and that is now safe because only `show` can ask for it.** Reconstructing
/// markdown is a per-resource read with no batched form (`readback::body` is itself 2-3 statements:
/// `ensure_visible`, the verbatim `string_agg`, then the derived fallback), so this loop is an N+1
/// by construction. It was reachable from `list` over raw HTTP, MCP and temper-rb — where `--all`
/// sends no limit, making it unbounded — until `list_select` narrowed its accepted vocabulary to
/// `ResourceSection::LIST`. The remaining caller is `show_view_select`, whose slice is one view
/// long (MCP's `get_resource` is the live one), so "per-view" and "per-page" now coincide.
///
/// The arm is kept rather than moved into `show_view_select` because the section→read mapping is
/// this function's whole job, and splitting it would give `body` a second home for the sake of a
/// loop that runs once. What keeps it honest is the door, not the loop.
///
/// `content: None` and `open_meta: None` mean **not requested**, never "empty": an empty body is
/// `Some(String::new())` and an empty open tier is `Some({})`. Both distinctions survive the wire
/// (`ResourceView`'s `body_absent_is_distinguishable_from_body_empty`). A view whose id the batch
/// did not return still gets `Some({})` — it was already read through a visibility-gated identity
/// read, so "no property rows" is an EMPTY open tier, not a missing one.
///
/// [`ResourceSection::Edges`] is deliberately unhandled. Edges are fetched *alongside* a view, not
/// carried on it — [`ResourceView`] has no edges field — so there is nothing here to fill; the
/// surface that serves `--with edges` composes the edge read itself.
///
/// The managed tier is **not** a section and so is not filled here: it arrives already populated
/// from `hit_identities`' joined aggregate. That is what makes dropping the hoisted
/// `stage`/`mode`/`effort`/`seq` columns lossless.
async fn fill_sections(
    pool: &PgPool,
    profile_id: ProfileId,
    views: &mut [ResourceView],
    sections: &SectionSet,
) -> ApiResult<()> {
    if views.is_empty() {
        return Ok(());
    }
    if sections.contains(ResourceSection::OpenMeta) {
        let ids: Vec<ResourceId> = views.iter().map(|view| view.id).collect();
        let mut by_id = readback::meta_batch(pool, profile_id, &ids)
            .await
            .map_err(|e| ApiError::from(map_readback_err(e)))?;
        for view in views.iter_mut() {
            let open = by_id.remove(&view.id).map(|rb| rb.open).unwrap_or_default();
            view.open_meta = Some(serde_json::Value::Object(open));
        }
    }
    if sections.contains(ResourceSection::Body) {
        for view in views.iter_mut() {
            view.content = Some(
                readback::body(pool, profile_id, view.id)
                    .await
                    .map_err(|e| ApiError::from(map_readback_err(e)))?,
            );
        }
    }
    Ok(())
}

/// `list`, as [`ResourceView`]s — the resources VISIBLE to the principal (WS2 —
/// `resources_visible_to`), filtered + sorted + paginated per `ResourceListParams`, each carrying
/// the always-present managed tier plus whatever `sections` asks for.
///
/// **The page costs one statement per PAGE, not one per row.** `filtered_visible_page` cuts the
/// page in SQL; `readback::hit_identities` then reconstructs the whole page in ONE round-trip —
/// the same batched call search already used ("50 results meant 51 queries",
/// `temper-substrate/src/readback/mod.rs`). The predecessor of this function looped
/// `native_resource_row` per page id, which measured **27 statements for a 13-row page**; the doc
/// comment above it claimed "no all-rows N+1", which was true about *all rows* and silent about
/// the page. Do not reintroduce the loop — `list_page_is_one_batched_read_not_one_query_per_row`
/// measures it.
///
/// `total` = the FILTERED count (before limit/offset). The three `facets` histograms are over
/// DIFFERENT sets — each excludes its own filter predicate, so `facets.doc_type` counts kinds this
/// page holds none of (see [`ResourceFacets`]). Summing a histogram will not give `total`, and it
/// is not meant to.
pub async fn list_views_select(
    pool: &PgPool,
    profile_id: ProfileId,
    params: &ResourceListParams,
    sections: &SectionSet,
) -> ApiResult<ResourceViewPage> {
    let page = filtered_visible_page(pool, profile_id, params).await?;
    let ids: Vec<ResourceId> = page
        .page_ids
        .iter()
        .copied()
        .map(ResourceId::from)
        .collect();
    let mut by_id: HashMap<Uuid, ResourceView> = readback::hit_identities(pool, profile_id, &ids)
        .await
        .map_err(api_err)?
        .into_iter()
        .map(|view| (view.id.uuid(), view))
        .collect();

    // `hit_identities` answers `WHERE r.id = ANY($2)` — a SET, with no ORDER BY. The page's order
    // is the one `filtered_visible_page`'s ORDER BY produced, so it is restored by walking the page
    // ids rather than by trusting the readback's row order (which is the planner's to choose).
    let mut views = Vec::with_capacity(page.page_ids.len());
    for new_id in &page.page_ids {
        // An id absent from the batch means the row stopped being visible between the two
        // statements — a dropped row, not a fault. Same convention as the readback's own.
        if let Some(view) = by_id.remove(new_id) {
            views.push(view);
        }
    }
    // Sections are filled for the PAGE, not per row: `open-meta` is one statement for all of them.
    // Doing it inside the loop above is what made the open tier an N+1 (see `fill_sections`).
    fill_sections(pool, profile_id, &mut views, sections).await?;

    Ok(ResourceViewPage {
        views,
        total: page.total,
        facets: page.facets,
        limit: page.limit,
        offset: page.offset,
    })
}

/// `list` — the one list envelope, over [`list_views_select`].
///
/// There is no second list function. `?meta_only=true` used to route to `list_meta_select`, whose
/// rows were a different type in a different envelope; a caller now asks for *parts* of the one
/// shape through `params.sections`, and gets the same `ResourceListResponse` either way.
///
/// **The CSV is parsed here, not in the handler**, for the same reason the tag case-fold lives in
/// `filtered_visible_page`: the HTTP surface, the MCP tools and any other caller reach this
/// function directly, and a vocabulary parsed at one door is a vocabulary the other doors do not
/// have. An unknown section name is a `400` naming the whole valid set.
///
/// **The accepted set is `ResourceSection::LIST`, not `ResourceSection::ALL`** — `body` is refused
/// here rather than filled. It had been fillable: `fill_sections` handles it, and the CLI's own
/// `list_section_parser` has refused it since sections shipped, so the CLI was the *only* door
/// enforcing a rule the server did not. MCP, raw HTTP and temper-rb could ask a page of any size
/// (`--all` sends no limit) to reconstruct every body, at 2-3 statements per row against
/// `readback::body`'s per-resource read — the one remaining N+1 on the read path, and unbounded.
/// Refusing it at the door removes that reachability rather than batching it, because a page of
/// bodies is not a thing `list` should serve at any statement count: `list` orients, `show` reads.
/// `edges` is refused for a different reason — nothing on this door fills it (see
/// [`ResourceSection::LIST`] for both).
///
/// So `fill_sections`' body arm is now reachable only from `show_view_select`, where the slice is
/// one view long. That is asserted, not assumed —
/// `the_list_door_refuses_body_and_names_only_what_it_accepts`.
pub async fn list_select(
    pool: &PgPool,
    profile_id: ProfileId,
    params: ResourceListParams,
) -> ApiResult<ResourceListResponse> {
    let sections = match params.sections.as_deref() {
        Some(csv) => {
            SectionSet::parse_csv_accepting(csv, &ResourceSection::LIST).map_err(ApiError::from)?
        }
        None => SectionSet::default(),
    };
    let page = list_views_select(pool, profile_id, &params, &sections).await?;
    Ok(ResourceListResponse::new(
        page.views,
        page.total,
        page.facets,
        page.limit,
        page.offset,
    ))
}

// `show_select` (→ `ResourceRow`) and `show_detail_select` (→ `ResourceDetail`) are GONE, with the
// two `From<ResourceView>` narrowings that fed them. Their last consumer was temper-mcp, which
// answered in a `ResourceRow`-shaped `EnrichedResource`; it answers in `ResourceView` now, so both
// projections and both source types are retired. `show_view_select` is the one door.

/// `show`, as a [`ResourceView`] — one resource, the always-present managed tier, plus whatever
/// `sections` asks for.
///
/// Reads through the same batched `readback::hit_identities` the list path uses, at a batch of
/// one: a resource's identity does not depend on how it was reached, so a second per-resource
/// query would be a second definition of the same shape. Visibility is gated inside that readback
/// (WS2 — `resources_visible_to`); an id the principal cannot see comes back as no row, rendered
/// here as the same `NotFound` message `map_readback_err` produces, so this door and the write
/// path's readback cannot disagree about how a miss reads.
///
/// Deliberately **not** gated on `ingest_state`: an interrupted segmented ingest stays fully
/// addressable and readable via `show` (which reports the state) even though list and search
/// exclude it.
pub async fn show_view_select(
    pool: &PgPool,
    profile_id: ProfileId,
    id: ResourceId,
    sections: &SectionSet,
) -> ApiResult<ResourceView> {
    let mut view = readback::hit_identities(pool, profile_id, &[id])
        .await
        .map_err(api_err)?
        .into_iter()
        .next()
        .ok_or_else(|| ApiError::NotFound(format!("resource {id} not found")))?;
    fill_sections(pool, profile_id, std::slice::from_mut(&mut view), sections).await?;
    Ok(view)
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

/// `get_meta` — one resource with both metadata tiers, as a [`ResourceView`].
///
/// `GET /api/resources/{id}/meta` used to answer in a `ResourceMetaResponse` — `{id, managed_meta,
/// open_meta}` — whose whole design constraint was to be *a literal strict subset* of what `show`
/// returns. That constraint exists only while the two are different types. They are one type now,
/// so the endpoint answers in it: the subset relation became identity, which is the convergence
/// this arc is for.
///
/// Composed from [`show_view_select`] with the `open-meta` section asked for, so this door and
/// `show`'s cannot disagree about a resource's identity, its managed tier, or how a miss reads.
/// The body is not requested; `GET /api/resources/{id}/content` remains its door.
///
/// Carries no meta hashes: `managed_hash`/`open_hash` were §7-dissolved and had been emitted as
/// empty strings ever since. The real `body_hash` is on the view.
pub async fn get_meta_select(
    pool: &PgPool,
    profile_id: ProfileId,
    resource_id: ResourceId,
) -> ApiResult<ResourceView> {
    let sections: SectionSet = [ResourceSection::OpenMeta].into_iter().collect();
    show_view_select(pool, profile_id, resource_id, &sections).await
}

// `get_meta_batch_select` is GONE. It looped `get_meta_select` — itself three statements — once per
// id, which made the MCP list surface cost ~3n. Its one caller was temper-mcp's `enrich_resources`,
// and the meta a list row needs now arrives ON the row: MCP asks `list_select` for
// `sections=open-meta`, so the managed tier comes from `hit_identities` and the open tier from
// `readback::meta_batch`, both once for the whole page. A second batched meta door beside the
// section vocabulary would be a second answer to "what are this resource's tiers".

/// Surface A caps resolved once, before the SQL call (pure → unit-tested).
pub(crate) struct ClampedSearch {
    pub limit: i64,
}

/// limit → \[1,50\] (the documented API ceiling), default 10. The graph-depth clamp is gone with the
/// graph arm; the limit now bounds EACH arm independently rather than one merged list, so a caller
/// asking for 10 can receive up to 10 exact and 10 wide.
pub(crate) fn clamp_search_params(params: &SearchParams) -> ClampedSearch {
    ClampedSearch {
        limit: params.limit.unwrap_or(10).clamp(1, 50),
    }
}

/// Resolve the scope selectors into the single thing both arms take: an anchor, or nothing.
///
/// `context_ref` and `cogmap_id` name two different homes, so asking for both is incoherent and is a
/// `400`. Everything else that used to live here is gone with wayfind: there is no id-set to
/// assemble, because the arms scope by the anchor pair inside SQL rather than by a flattened pile of
/// resource ids handed down from Rust.
///
/// A multi-map scope is likewise gone. One anchor is one anchor; asking several maps at once is a
/// composition, which is `/api/query`'s job, not a comma in this parameter.
///
/// **Both anchor kinds are gated here before they become an anchor**: a `context_ref` by
/// `resolve_context_ref`, a cogmap uuid by `cogmap_readable_by_profile`. Both refuse with
/// `NotFound`. The cogmap arms' SQL carries its own readability conjunct as well — that is defence
/// in depth, not duplication, and the two are witnessed separately because they say different
/// things (this refuses; the SQL empties).
async fn resolve_search_anchor(
    pool: &PgPool,
    profile_id: ProfileId,
    params: &SearchParams,
) -> ApiResult<Option<HomeAnchor>> {
    let effective_cogmaps: Vec<uuid::Uuid> = match params.cogmap_ids.as_deref() {
        Some(ids) if !ids.is_empty() => ids.to_vec(),
        _ => params.cogmap_id.into_iter().collect(),
    };

    if params.context_ref.is_some() && !effective_cogmaps.is_empty() {
        return Err(ApiError::BadRequest(
            "context_ref and cogmap scope are mutually exclusive".into(),
        ));
    }
    if effective_cogmaps.len() > 1 {
        return Err(ApiError::BadRequest(
            "search scopes to a single anchor; ask several maps as a composition instead".into(),
        ));
    }

    if let Some(s) = params.context_ref.as_deref() {
        let cref = temper_core::context_ref::parse_context_ref(s)
            .map_err(|e| ApiError::BadRequest(e.to_string()))?;
        let id =
            *crate::services::context_service::resolve_context_ref(pool, profile_id, &cref).await?;
        return Ok(Some(HomeAnchor::Context(ContextId::from(id))));
    }
    if let [one] = effective_cogmaps.as_slice() {
        // Deny-as-absence, symmetric with the context branch above: `resolve_context_ref` gates a
        // `context_ref` before it can become an anchor, and a raw cogmap uuid arrives from the
        // caller with no gate of its own. Without this the only thing standing between an
        // unreadable map and a scoped search is the arm's own SQL — which is a real gate, but one
        // layer, and this parameter is the caller's unvalidated input.
        //
        // THE SQL GATE AND THIS ONE ARE NOT REDUNDANT, THEY DIFFER IN WHAT THEY SAY. The arm's
        // conjunct yields an EMPTY RESULT — indistinguishable from "the map is readable and holds
        // nothing you match". This refuses. Asking to search a map you cannot read is not a search
        // that found nothing, it is a request that cannot be honoured, and a caller that cannot
        // tell those apart will retry against a wider query forever.
        //
        // `cogmap_readable_by_profile` is called, never restated — the same predicate the arm's SQL
        // calls, so the two layers cannot disagree about who reads a map. `readable!`: sqlx types a
        // function-call column as nullable, but the function is `SELECT EXISTS (...) OR
        // profile_explicit_grant(...)` and both arms are total, so it can never be NULL (the
        // argument spelled out at `graph_service::cogmap_neighborhood_slice`'s identical gate).
        let readable: bool = sqlx::query_scalar!(
            r#"SELECT cogmap_readable_by_profile($1, $2) AS "readable!""#,
            profile_id.as_uuid(),
            *one,
        )
        .fetch_one(pool)
        .await?;
        if !readable {
            // Byte-identical to `graph_service`'s refusal for the same condition. NotFound, not
            // Forbidden: a refusal that distinguished "exists but you may not read it" from "no
            // such map" would make this parameter an existence oracle over every cogmap uuid.
            return Err(ApiError::NotFound(
                "cognitive map not found or not readable".to_string(),
            ));
        }
        return Ok(Some(HomeAnchor::Cogmap(CogmapId::from(*one))));
    }
    Ok(None)
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
/// partial results beat none, and the `degraded` signal is surfaced on the wide arm ([`WideArm`]).
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
    match embed_query_text(&query).await {
        QueryEmbed::Embedded(embedding) => {
            params.embedding = Some(embedding);
            false
        }
        QueryEmbed::Unavailable => true,
    }
}

/// What a server-side embed attempt came to. **Two outcomes, and neither is "the caller declined"**
/// — that question is answered before this is called, by whether a vector already arrived.
///
/// The distinction matters differently to each surface. `/api/search` collapses it into a
/// `degraded` boolean because its arms are fixed and an arm that could not run must say so in
/// place. `/api/query` cannot: an unavailable embedding is a per-stage REFUSAL there
/// ([`temper_core::types::query::RefusalReason::EmbeddingUnavailable`]), reported where a reader is
/// already looking. So this returns the outcome and lets each surface render it.
pub(crate) enum QueryEmbed {
    Embedded(Vec<f32>),
    /// The server tried and could not — an embed error, a task panic, or the budget expiring.
    ///
    /// **Never "there was nothing to embed".** An empty query is the caller's omission, decided
    /// statically by whoever calls this, long before here.
    Unavailable,
}

/// Embed one query string, off the async executor and under a wall-clock budget.
///
/// Extracted from [`embed_query_if_missing`] so `/api/query` reaches the SAME attempt rather than a
/// second one, and it has to be the same: the query is embedded with the same plain `embed_text`
/// path the corpus was ingested with (no BGE "represent this sentence…" query prefix), which is what
/// keeps it in the stored chunks' vector space so scores stay comparable. A second implementation
/// here would be a second answer to "which space is this vector in".
pub(crate) async fn embed_query_text(query: &str) -> QueryEmbed {
    let budget = query_embed_budget();
    let owned = query.to_string();
    let embed = tokio::task::spawn_blocking(move || temper_ingest::embed::embed_text(&owned));
    match tokio::time::timeout(budget, embed).await {
        Ok(Ok(Ok(embedding))) => QueryEmbed::Embedded(embedding),
        Ok(Ok(Err(e))) => {
            tracing::warn!(
                error = %e,
                "server-side query embedding failed; falling back to FTS + graph only"
            );
            QueryEmbed::Unavailable
        }
        Ok(Err(join_err)) => {
            tracing::warn!(
                error = %join_err,
                "server-side query embedding task panicked; falling back to FTS + graph only"
            );
            QueryEmbed::Unavailable
        }
        Err(_elapsed) => {
            tracing::warn!(
                budget_ms = budget.as_millis(),
                "server-side query embedding exceeded its budget; falling back to FTS + graph only"
            );
            QueryEmbed::Unavailable
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
/// [`SearchScopeInfo`] rather than a variant.
fn classify_scope(params: &SearchParams) -> SearchScope {
    if params.cogmap_id.is_some()
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

/// Build the agent-facing one-liner carried on an arm's `hint`. Pure — no DB —
/// so it is unit-testable. Returns `None` only when the result set is unremarkable (`Ok` reason and
/// no degraded signal); otherwise it explains the shape and suggests a concrete next step.
///
/// # Every emitted string must be ASCII
///
/// This text ships to the caller inside the `x-temper-search-diagnostics` **response header**, and
/// on the deployed platform a non-ASCII byte does not survive that trip: it arrives percent-encoded,
/// so an em dash reaches the user as a literal `%E2%80%94` in the middle of a sentence. Both Rust
/// sides are innocent — the handler writes raw UTF-8 with `HeaderValue::from_bytes` and the client
/// reads it back with `str::from_utf8` — and both carried comments asserting the round-trip was
/// therefore safe. It is safe against a bare Axum server, which is exactly what the e2e drives; the
/// encoding is introduced further out, at the serverless adapter boundary the tests never cross.
/// `[observed on prod — 2026-08-01]`, in a hint shipped by #360 and unnoticed until a later hint
/// started firing on the common path.
///
/// So: **plain ASCII punctuation only in these strings.** `every_emitted_hint_is_ascii` enumerates
/// the arms and fails on any non-ASCII byte, because "remember not to type an em dash" is not a
/// property anyone can hold. Prose in this function is a wire value, not commentary — the comments
/// around it are free to use whatever punctuation reads best.
fn search_hint(
    scope: SearchScope,
    reason: SearchReason,
    scope_size: Option<i64>,
    degraded: bool,
    offset: i64,
) -> Option<String> {
    // The narrowed-reach hint (issue #585) is GONE with wayfind. It reported `anchors_selected`
    // against `anchors_visible` — a pooled-competition signal that has no referent once a search
    // scopes to at most one anchor and each arm returns its own list. Its successor, if one is ever
    // wanted, belongs to `/api/query`'s per-stage disclosure and describes a composition rather than
    // a blend.
    //
    // ASCII ONLY still applies to every string emitted here, guarded by `every_emitted_hint_is_ascii`
    // — see that test for why the header's percent-encoding scar outlived the header itself.
    let mut parts: Vec<String> = Vec::new();
    match (scope, reason) {
        (SearchScope::Cogmap, SearchReason::OutOfScope) => parts.push(
            "the --cogmap scope resolved to zero visible resources; a different phrasing will \
             not help - check the map ref, or drop the scope to search everything you can see."
                .to_string(),
        ),
        // An empty page at a non-zero offset is NOT "nothing matched", and telling the caller to
        // rephrase is a wrong answer to the question they asked: they walked off the end of a walk
        // that may have been returning hits the whole way. The wire enum cannot tell these apart —
        // `SearchReason` has no past-the-end variant and adding one is a breaking widen — so the
        // distinction lives in the hint, which is where a caller reads what to DO next.
        (_, SearchReason::NoMatch) if offset > 0 => {
            parts.push(format!(
                "no results at offset {offset}: this is the end of this arm, not an empty corpus. \
                 Earlier pages may have matched. Rephrasing will not help; go back a page."
            ));
        }
        (_, SearchReason::NoMatch) => {
            let prefix = scope_size
                .map(|n| format!("{n} candidate resource(s) in scope; "))
                .unwrap_or_default();
            parts.push(format!(
                "{prefix}this arm matched nothing: try rephrasing or a broader scope."
            ));
        }
        _ => {}
    }
    if degraded {
        parts.push(
            "the query could not be embedded server-side, so this arm had no signal to run on; \
             its result is not an answer about the corpus. Send a precomputed embedding, or read \
             the exact arm."
                .to_string(),
        );
    }
    if parts.is_empty() {
        None
    } else {
        Some(parts.join(" "))
    }
}

/// Refuse an embedding that cannot have a cosine direction, BEFORE it reaches the database.
///
/// pgvector's `<=>` against a zero-magnitude vector is **NaN**. That NaN propagates through
/// `vec_norm`, and JSON has no NaN literal — so serde emits `null` for a field the shared hit rows
/// declare as `f32`, and the response cannot be deserialized by the very type the server used to
/// produce it. Measured against production on 2026-08-05, when the arms were still blended: a
/// 768-dimension zero vector returned `"vector_score":null,"combined_score":null`, and
/// `temper-client` failed with `invalid type: null, expected f32`. Those two fields are gone; the
/// NaN path is not — it now lands on `vec_norm`, which is why this guard stays.
///
/// So this is refused at the door rather than repaired downstream. Coercing NaN to 0.0 at
/// serialization would make a degenerate request look like a successful one that simply matched
/// nothing, which is a worse answer than a 400 naming the problem.
///
/// The predicate is the squared magnitude in `f64`, not an all-zeros comparison, so that non-finite
/// components are caught by the same test — they reach `<=>` and come back NaN just as a zero does.
///
/// **`f64` is not arbitrary: it is what pgvector itself uses.** Measured 2026-08-05 on pgvector
/// 0.8.2 — a 768-dimension vector of `1e-30` (every component non-zero, but every *square*
/// underflowing in `f32`) against an ordinary embedding returns `0`, not NaN, which means pgvector
/// accumulates the norm in double precision. Computing this predicate in `f32` would therefore
/// reject inputs the database handles perfectly well. An earlier draft asserted the opposite and
/// its test failed, which is how this came to be measured instead of assumed.
///
/// **Declared remainder:** this guards the *query* side only. A zero-magnitude **stored** chunk
/// embedding would produce the same NaN from the other direction, and nothing guards that — the
/// analogous "T7 NaN trap" guard exists in `wayfind_region_scores` for zero-vector region centroids
/// but has no counterpart for chunks. Measured on production 2026-08-05: **0 zero-norm current
/// chunks and 0 zero-norm live centroids**, so the case is not live today. It is unguarded, not
/// impossible.
fn reject_degenerate_embedding(embedding: Option<&[f32]>) -> ApiResult<()> {
    let Some(emb) = embedding else {
        return Ok(());
    };
    if emb.is_empty() {
        return Ok(()); // dimension mismatch is the database's error to give, not this one's
    }
    let norm_sq: f64 = emb.iter().map(|c| f64::from(*c) * f64::from(*c)).sum();
    if !norm_sq.is_finite() || norm_sq <= 0.0 {
        return Err(ApiError::BadRequest(
            "embedding has no direction (zero magnitude or non-finite components); \
             cosine distance against it is undefined"
                .to_string(),
        ));
    }
    Ok(())
}

/// `search` — the two arms of `/api/search`, run independently and returned unmerged.
///
/// Each arm is asked the same question of the same scope and answers with its own quantity. Nothing
/// here combines them: there is no weight, no sum, and no single ordered list they could be merged
/// into. Decision `019fd25a-ef4c-7473-b72e-265a7d36dd65`.
///
/// The arms are run CONCURRENTLY (`try_join`) rather than in sequence. They share no state and
/// neither feeds the other — which is precisely what "never combined" buys, and it is worth taking:
/// the wide arm's ANN and the exact arm's GIN scan are independent round-trips.
pub async fn search_select(
    pool: &PgPool,
    profile_id: ProfileId,
    mut params: SearchParams,
) -> ApiResult<SearchResponse> {
    reject_degenerate_embedding(params.embedding.as_deref())?;
    // `degraded` is the WIDE arm's property: a failed embed leaves the exact arm untouched and makes
    // the wide arm impossible. It is reported on that arm rather than on the response.
    let degraded = embed_query_if_missing(&mut params).await;
    let scope = classify_scope(&params);
    let clamped = clamp_search_params(&params);
    // Offset and limit apply PER ARM, never to a merged list — each arm is paginated through its own
    // order. A caller asking offset 10 gets each arm's second page, which is the only reading that
    // survives the arms being incommensurable: there is no combined sequence to be at position 10 of.
    let offset = params.offset.unwrap_or(0).max(0) as usize;
    let limit = clamped.limit as usize;
    let anchor = resolve_search_anchor(pool, profile_id, &params).await?;

    // ONE MORE ROW THAN THE PAGE. Ordering and paging are the SQL functions' now, so the arms
    // return at most `limit + 1` rows instead of their whole match set — measured at 1,883 rows for
    // a ten-row request against a 3,402-resource production corpus, every one of which used to be
    // hydrated into a full `ResourceView` before the page was sliced.
    //
    // The `+ 1` is the probe row. It never reaches the caller; it exists so an arm can tell "there
    // is more after this page" without a second counting statement. `saturating_add` because
    // `limit` is caller-supplied and an `i32::MAX` page must not wrap to a negative LIMIT.
    let fetch = i32::try_from(clamped.limit)
        .unwrap_or(i32::MAX)
        .saturating_add(1);
    let arm = readback::ArmQuery {
        principal: profile_id,
        // `None` is UNBOUNDED, not "bounded to nothing" — the wire field's own `None`/`Some(&[])`
        // distinction rides straight through: an empty set from the caller arrives as
        // `Some(&[])` (zero rows), never collapsed into `None` (unbounded).
        bound_ids: params.bound_ids.as_deref(),
        anchor,
        doc_type: params.doc_type.as_deref(),
        limit: Some(fetch),
        offset: i32::try_from(offset).unwrap_or(i32::MAX),
    };

    let (exact_hits, wide_hits) = tokio::try_join!(
        async {
            readback::search_exact(pool, params.query.as_deref(), arm)
                .await
                .map_err(|e| search_stage_err("search_exact", e))
        },
        async {
            readback::search_wide(pool, params.embedding.as_deref(), VECTOR_K, arm)
                .await
                .map_err(|e| search_stage_err("search_wide", e))
        },
    )?;

    // Drop the probe row before anything else sees it, so enrichment pays for the page only.
    let exact_hits: Vec<_> = exact_hits.into_iter().take(limit).collect();
    let wide_hits: Vec<_> = wide_hits.into_iter().take(limit).collect();

    // Enrichment is shared: both arms name resources, and a resource's identity does not depend on
    // which arm found it. One batched round-trip over the union, then each arm keeps its own order.
    let mut wanted: Vec<ResourceId> = exact_hits.iter().map(|h| h.resource_id).collect();
    wanted.extend(wide_hits.iter().map(|h| h.resource_id));
    wanted.sort_unstable_by_key(|r| r.uuid());
    wanted.dedup();
    let identities = enrich_hits(pool, profile_id, &wanted).await?;

    // NO RE-SORT HERE. Each arm arrives ordered by its own quantity with `resource_id` as the
    // tiebreak, decided by the SQL function that also applied the LIMIT. Re-deriving the order in
    // Rust would be a second implementation of the page boundary: if the two rules ever disagreed
    // on a tie, a row could appear on two pages or on none, and the Rust sort would silently win
    // over the one that actually chose the rows. `filter_map` preserves order.
    let exact: Vec<ExactHit> = exact_hits
        .into_iter()
        .filter_map(|h| {
            identities
                .get(&h.resource_id.uuid())
                .map(|i| i.clone().into_exact(h.fts_norm))
        })
        .collect();

    let wide: Vec<WideHit> = wide_hits
        .into_iter()
        .filter_map(|h| {
            identities
                .get(&h.resource_id.uuid())
                .map(|i| i.clone().into_wide(h.vec_norm))
        })
        .collect();

    let scope_size: Option<i64> = None;
    let exact_reason = arm_reason(exact.len());
    let wide_reason = arm_reason(wide.len());

    let offset_i64 = i64::try_from(offset).unwrap_or(i64::MAX);
    Ok(SearchResponse {
        exact: ExactArm {
            hint: search_hint(scope, exact_reason, scope_size, false, offset_i64),
            reason: exact_reason,
            hits: exact,
        },
        wide: WideArm {
            hint: search_hint(scope, wide_reason, scope_size, degraded, offset_i64),
            reason: wide_reason,
            hits: wide,
            degraded,
        },
        scope: SearchScopeInfo {
            kind: scope,
            size: scope_size,
        },
    })
}

/// How many nearest chunks the wide arm's UNSCOPED branch draws before visibility and completeness
/// are applied. Carried over from the retired `unified_search`'s `vector_k` — the one tuning constant
/// this phase keeps, because it governs ADMISSION (how much the index is asked for) rather than
/// ranking (how what came back is ordered). Nothing weighs it against anything.
///
/// **`hnsw.ef_search` is pinned on `query_find_wide` at or above this value** — the twin
/// `readback::search_wide` calls since `/api/search` gained a resource bound; proconfig binds to a
/// signature, so the pin on the incumbent `search_wide` no longer covers this path. Raising it past the pin
/// silently truncates the draw — the pin's whole reason for existing. See
/// `search_wide_pins_ef_search_at_or_above_the_k_it_is_asked_for`.
const VECTOR_K: i32 = 100;

/// Enrich both arms' hits in one batched round-trip, keyed by resource id.
async fn enrich_hits(
    pool: &PgPool,
    profile_id: ProfileId,
    ids: &[ResourceId],
) -> ApiResult<std::collections::HashMap<Uuid, ResourceView>> {
    let rows = readback::hit_identities(pool, profile_id, ids)
        .await
        .map_err(|e| search_stage_err("enrichment", e))?;
    Ok(rows.into_iter().map(|v| (v.id.uuid(), v)).collect())
}

/// Project a shared identity into an arm's row. Two near-identical bodies because the rows are two
/// types on purpose: each carries its own quantity, so neither can be built without naming which.
trait IntoArmHit {
    fn into_exact(self, fts_norm: f32) -> ExactHit;
    fn into_wide(self, vec_norm: f32) -> WideHit;
}

/// A wrap, and nothing else. The view goes onto the hit unchanged, so a hit and a list row describe
/// the same resource with the same bytes — the property this convergence exists to hold.
///
/// This used to flatten the view into eight inlined fields, collapsing `context_name`/`cogmap_name`
/// into one `context` slot along the way. That collapse existed only to fill the flat field; with no
/// flat field there is nothing to collapse into, and a caller that wants the display name calls
/// [`ResourceView::home_display`] — the one accessor for the mutual exclusion, which this was a
/// second, local copy of.
impl IntoArmHit for ResourceView {
    fn into_exact(self, fts_norm: f32) -> ExactHit {
        ExactHit {
            resource: self,
            fts_norm,
        }
    }

    fn into_wide(self, vec_norm: f32) -> WideHit {
        WideHit {
            resource: self,
            vec_norm,
        }
    }
}

/// An arm's disposition from its own hit count. `OutOfScope` is not reachable per-arm today: the
/// anchor pair is applied inside the SQL, so an unreadable or empty anchor yields zero rows that are
/// indistinguishable here from "nothing matched". Named rather than silently collapsed — restoring
/// the distinction needs the arm to report its admitted count, which is the task's open item.
fn arm_reason(hits: usize) -> SearchReason {
    if hits == 0 {
        SearchReason::NoMatch
    } else {
        SearchReason::Ok
    }
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

// ── Data artifact reads ───────────────────────────────────────────────────────────────────────

/// List data artifacts for a resource, fully hydrated (metadata + content). Visibility-gated
/// through `resources_visible_to` in the SQL.
pub async fn list_artifacts(
    pool: &PgPool,
    profile_id: ProfileId,
    resource_id: ResourceId,
    kind: Option<&str>,
    intent: Option<&str>,
    include_folded: bool,
) -> ApiResult<Vec<temper_core::types::data_artifact::ArtifactView>> {
    let intent = intent.map(parse_intent_str).transpose()?;
    let artifacts = readback::artifacts_for_resource(
        pool,
        profile_id,
        resource_id,
        kind,
        intent,
        include_folded,
    )
    .await
    .map_err(|e| ApiError::from(TemperError::Api(e.to_string())))?;

    Ok(artifacts
        .into_iter()
        .map(|a| temper_core::types::data_artifact::ArtifactView {
            artifact_id: a.artifact_id,
            resource_id: a.resource_id,
            kind_owner_table: match a.kind_owner {
                temper_substrate::payloads::KindOwner::Profile(_) => "kb_profiles".to_owned(),
                temper_substrate::payloads::KindOwner::Team(_) => "kb_teams".to_owned(),
            },
            kind_owner_id: match a.kind_owner {
                temper_substrate::payloads::KindOwner::Profile(id) => id,
                temper_substrate::payloads::KindOwner::Team(id) => id,
            },
            artifact_kind: a.artifact_kind,
            intent: match a.intent {
                temper_substrate::payloads::ArtifactIntent::Current => "current".to_owned(),
                temper_substrate::payloads::ArtifactIntent::Member => "member".to_owned(),
                temper_substrate::payloads::ArtifactIntent::Pinned => "pinned".to_owned(),
            },
            precedence: a.precedence,
            content_hash: a.content_hash,
            content_bytes: a.content_bytes,
            shape_state: match a.shape_state {
                temper_substrate::payloads::ShapeState::NeverDeclared => {
                    "never_declared".to_owned()
                }
            },
            is_folded: a.is_folded,
            created: a.created,
            content: a.content,
        })
        .collect())
}

/// Per-family artifact counts for a resource, without content hydration.
pub async fn artifact_counts(
    pool: &PgPool,
    profile_id: ProfileId,
    resource_id: ResourceId,
    include_folded: bool,
) -> ApiResult<Vec<temper_core::types::data_artifact::ArtifactCountRow>> {
    let counts =
        readback::artifact_counts_for_resource(pool, profile_id, resource_id, include_folded)
            .await
            .map_err(|e| ApiError::from(TemperError::Api(e.to_string())))?;

    Ok(counts
        .into_iter()
        .map(|c| temper_core::types::data_artifact::ArtifactCountRow {
            kind_owner_table: match c.kind_owner {
                temper_substrate::payloads::KindOwner::Profile(_) => "kb_profiles".to_owned(),
                temper_substrate::payloads::KindOwner::Team(_) => "kb_teams".to_owned(),
            },
            kind_owner_id: match c.kind_owner {
                temper_substrate::payloads::KindOwner::Profile(id) => id,
                temper_substrate::payloads::KindOwner::Team(id) => id,
            },
            artifact_kind: c.artifact_kind,
            count: c.count,
            total_bytes: c.total_bytes,
        })
        .collect())
}

/// Retrieve a single data artifact by ID. Returns `None` (→ 404) if the artifact does not
/// exist or the owning resource is not visible to the caller.
pub async fn get_artifact(
    pool: &PgPool,
    profile_id: ProfileId,
    artifact_id: DataArtifactId,
) -> ApiResult<Option<temper_core::types::data_artifact::ArtifactView>> {
    let artifact = readback::artifact_by_id(pool, profile_id, artifact_id)
        .await
        .map_err(|e| ApiError::from(TemperError::Api(e.to_string())))?;

    Ok(
        artifact.map(|a| temper_core::types::data_artifact::ArtifactView {
            artifact_id: a.artifact_id,
            resource_id: a.resource_id,
            kind_owner_table: match a.kind_owner {
                temper_substrate::payloads::KindOwner::Profile(_) => "kb_profiles".to_owned(),
                temper_substrate::payloads::KindOwner::Team(_) => "kb_teams".to_owned(),
            },
            kind_owner_id: match a.kind_owner {
                temper_substrate::payloads::KindOwner::Profile(id) => id,
                temper_substrate::payloads::KindOwner::Team(id) => id,
            },
            artifact_kind: a.artifact_kind,
            intent: match a.intent {
                temper_substrate::payloads::ArtifactIntent::Current => "current".to_owned(),
                temper_substrate::payloads::ArtifactIntent::Member => "member".to_owned(),
                temper_substrate::payloads::ArtifactIntent::Pinned => "pinned".to_owned(),
            },
            precedence: a.precedence,
            content_hash: a.content_hash,
            content_bytes: a.content_bytes,
            shape_state: match a.shape_state {
                temper_substrate::payloads::ShapeState::NeverDeclared => {
                    "never_declared".to_owned()
                }
            },
            is_folded: a.is_folded,
            created: a.created,
            content: a.content,
        }),
    )
}

pub fn parse_intent_str(s: &str) -> ApiResult<temper_substrate::payloads::ArtifactIntent> {
    match s {
        "current" => Ok(temper_substrate::payloads::ArtifactIntent::Current),
        "member" => Ok(temper_substrate::payloads::ArtifactIntent::Member),
        "pinned" => Ok(temper_substrate::payloads::ArtifactIntent::Pinned),
        other => Err(ApiError::from(TemperError::Api(format!(
            "unrecognized intent '{other}'; the vocabulary is current, member, pinned"
        )))),
    }
}

#[cfg(test)]
mod clamp_tests {
    use super::*;
    use temper_core::types::api::SearchParams;

    #[test]
    fn clamps_limit_to_the_documented_ceiling() {
        // The limit now bounds EACH arm independently, not one merged list.
        let p = SearchParams {
            limit: Some(999),
            ..SearchParams::default()
        };
        assert_eq!(clamp_search_params(&p).limit, 50, "limit capped at 50");
        assert_eq!(
            clamp_search_params(&SearchParams::default()).limit,
            10,
            "default limit 10"
        );
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
    }

    #[test]
    fn hint_out_of_scope_cogmap_mentions_cogmap() {
        let h = search_hint(
            SearchScope::Cogmap,
            SearchReason::OutOfScope,
            Some(0),
            false,
            0,
        )
        .expect("out-of-scope cogmap must hint");
        assert!(h.contains("cogmap"), "got: {h}");
    }

    #[test]
    fn hint_no_match_reports_scope_size_when_known() {
        let with = search_hint(
            SearchScope::Cogmap,
            SearchReason::NoMatch,
            Some(7),
            false,
            0,
        )
        .expect("no-match must hint");
        assert!(with.contains('7'), "should surface scope_size; got: {with}");
        assert!(
            with.contains("rephras"),
            "should suggest rephrasing; got: {with}"
        );

        let without = search_hint(SearchScope::Global, SearchReason::NoMatch, None, false, 0)
            .expect("no-match must hint even without scope_size");
        assert!(without.contains("matched"), "got: {without}");
    }

    /// An empty page at a non-zero offset must not be reported as an empty corpus.
    ///
    /// The old rule computed the arm's reason from the POST-pagination slice, so walking off the
    /// end of a result set produced `NoMatch` plus "try rephrasing" — advice that is wrong twice:
    /// the arm did match, and rephrasing would lose the results the caller already had. The reason
    /// is still `NoMatch` because the wire enum has no past-the-end variant, so the whole of the
    /// distinction is carried by the hint.
    #[test]
    fn hint_at_a_nonzero_offset_says_end_of_arm_not_empty_corpus() {
        let paged = search_hint(SearchScope::Global, SearchReason::NoMatch, None, false, 20)
            .expect("an empty page must still hint");
        assert!(
            paged.contains("offset 20"),
            "the hint must name the offset the caller actually asked for; got: {paged}"
        );
        assert!(
            !paged.contains("rephras"),
            "rephrasing is the one thing that cannot help a past-the-end page; got: {paged}"
        );

        // The positive control. Without it, a rule that dropped the rephrasing advice everywhere
        // would pass the assertion above while destroying the genuine no-match hint.
        let fresh = search_hint(SearchScope::Global, SearchReason::NoMatch, None, false, 0)
            .expect("a genuine no-match must hint");
        assert!(
            fresh.contains("rephras"),
            "at offset 0 an empty arm really did match nothing; got: {fresh}"
        );
    }

    #[test]
    fn hint_ok_is_silent_unless_degraded() {
        assert!(
            search_hint(SearchScope::Global, SearchReason::Ok, None, false, 0).is_none(),
            "an unremarkable Ok result needs no hint"
        );
        let degraded = search_hint(SearchScope::Global, SearchReason::Ok, None, true, 0)
            .expect("an arm that could not run must say so even with an Ok reason");
        assert!(
            degraded.contains("could not be embedded"),
            "got: {degraded}"
        );
    }

    #[test]
    fn hint_appends_degraded_to_a_reason() {
        let h = search_hint(
            SearchScope::Cogmap,
            SearchReason::OutOfScope,
            Some(0),
            true,
            0,
        )
        .expect("hint");
        assert!(h.contains("--cogmap scope"), "got: {h}");
        assert!(
            h.contains("could not be embedded"),
            "the could-not-run note must append to the reason; got: {h}"
        );
    }

    /// Every hint this function can emit is ASCII.
    ///
    /// The scar that produced this outlived its cause: hints used to ride the
    /// `x-temper-search-diagnostics` header, and the serverless adapter percent-encoded non-ASCII
    /// header bytes, so an em dash reached callers as `%E2%80%94`. Hints live in the body now and
    /// the encoding cannot happen — but nothing is gained by relaxing the rule, and a future
    /// header-borne field would reintroduce the hazard silently.
    #[test]
    fn every_emitted_hint_is_ascii() {
        let scopes = [
            SearchScope::Global,
            SearchScope::Context,
            SearchScope::Cogmap,
        ];
        let reasons = [
            SearchReason::Ok,
            SearchReason::NoMatch,
            SearchReason::OutOfScope,
        ];
        let mut emitted = 0usize;
        for scope in scopes {
            for reason in reasons {
                for degraded in [false, true] {
                    for size in [None, Some(0), Some(7)] {
                        // Offsets both sides of the past-the-end branch, so its string is
                        // enumerated too — an arm added later must not escape the ASCII rule.
                        for offset in [0i64, 20] {
                            let Some(h) = search_hint(scope, reason, size, degraded, offset) else {
                                continue;
                            };
                            emitted += 1;
                            assert!(h.is_ascii(), "hint carries non-ASCII: {h:?}");
                        }
                    }
                }
            }
        }
        assert!(
            emitted > 0,
            "the enumeration produced no hints at all, so it asserted nothing"
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

/// Pure-predicate tests for [`reject_degenerate_embedding`] — deliberately NOT behind `test-db`,
/// because the predicate needs no database and this is what puts it in the Unit CI job rather than
/// only the DB-backed one.
#[cfg(test)]
mod degenerate_embedding_tests {
    use super::reject_degenerate_embedding;

    #[test]
    fn a_zero_vector_is_refused() {
        // The exact production repro: `temper-client`'s integration test sends `vec![0.0; 768]`,
        // prod answers `"vector_score":null`, and the client cannot deserialize its own wire type.
        let err = reject_degenerate_embedding(Some(&vec![0.0_f32; 768])).unwrap_err();
        assert!(
            err.to_string().contains("no direction"),
            "the refusal must name the problem, got: {err}"
        );
    }

    #[test]
    fn non_finite_components_are_refused() {
        for bad in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            let mut v = vec![0.1_f32; 8];
            v[3] = bad;
            assert!(
                reject_degenerate_embedding(Some(&v)).is_err(),
                "{bad} must be refused: it reaches `<=>` and comes back NaN just as a zero does"
            );
        }
    }

    #[test]
    fn a_tiny_but_nonzero_embedding_is_admitted_because_pgvector_agrees() {
        // This test asserted the OPPOSITE first, on the reasoning that squares of `1e-30` underflow
        // in `f32` and so the magnitude is "really" zero. It failed, and measuring settled it:
        //
        //     SELECT '[1e-30, ...768]'::vector <=> '[0.05, ...768]'::vector  -->  0     (not NaN)
        //     SELECT '[1e-30, ...768]'::vector <=> '[1e-30, ...768]'::vector -->  NaN
        //
        // pgvector accumulates the norm in double, so a tiny query vector against a real embedding
        // is answered with a number and nothing breaks. Refusing it here would reject an input the
        // database handles — this guard exists to stop unparseable JSON, not to police oddity.
        //
        // The second line is the STORED-side case, which needs a tiny embedding in `kb_chunks`.
        // That is the declared remainder on `reject_degenerate_embedding`, measured at 0 rows on
        // production; this guard does not and cannot cover it.
        let v = vec![1.0e-30_f32; 768];
        assert_eq!(v.iter().filter(|c| **c == 0.0).count(), 0, "none are zero");
        assert!(
            reject_degenerate_embedding(Some(&v)).is_ok(),
            "must match pgvector's own double-precision arithmetic, not a stricter f32 story"
        );
    }

    #[test]
    fn an_ordinary_embedding_passes() {
        let mut v = vec![0.0_f32; 768];
        v[0] = 1.0;
        assert!(reject_degenerate_embedding(Some(&v)).is_ok());
        assert!(reject_degenerate_embedding(Some(&vec![0.05_f32; 768])).is_ok());
    }

    #[test]
    fn absent_and_empty_embeddings_are_not_this_functions_business() {
        // No embedding at all is an FTS-only search. An empty vector is a dimension mismatch, which
        // the database reports with a better message than this could — refusing it here would move
        // that error somewhere less informative.
        assert!(reject_degenerate_embedding(None).is_ok());
        assert!(reject_degenerate_embedding(Some(&[])).is_ok());
    }
}
