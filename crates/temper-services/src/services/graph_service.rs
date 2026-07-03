//! Graph subgraph service — returns aggregator-centric subgraphs for the
//! knowledge-graph UI.
//!
//! A "subgraph" is a depth-2 BFS from aggregator seeds (concepts today, any
//! aggregator doc type tomorrow). Composes with `graph_traverse()` for the
//! actual traversal so we inherit visibility scoping, cycle detection, and
//! edge-type filtering.

use sqlx::PgPool;
use uuid::Uuid;

use crate::error::{ApiError, ApiResult};
use temper_core::types::graph::{EdgeKind, Polarity};
use temper_core::types::graph_atlas::{
    AtlasEdge, AtlasNode, AtlasSubgraph, NodeHome, SliceRequest,
};
use temper_core::types::graph_territory::{
    Bridge, OrphanNode, Territory, TerritoryKind, TerritoryOverview,
};
use temper_core::types::ids::{ProfileId, ResourceId};
use temper_workflow::frontmatter::document::DocType;
use temper_workflow::types::graph::{is_aggregator, GraphEdge, GraphNode, SubgraphResponse};

/// Hard upper bound on traversal depth. Recursive-CTE cost grows superlinearly
/// with depth; 10 hops covers any imaginable UI traversal. Clamped silently.
const MAX_DEPTH: u32 = 10;

/// Max characters of body text to keep in a peek-panel excerpt. The UI
/// re-flows at ~60 chars per line and we render three lines of parchment
/// serif, so 280 is a generous fit without crowding the metadata block.
const EXCERPT_MAX_CHARS: usize = 280;

/// Derive a peek-panel excerpt from the first body chunk of a resource.
///
/// Takes the first paragraph (text up to the first blank line), then trims
/// to `EXCERPT_MAX_CHARS`. Truncation prefers the last whitespace within the
/// final 10% of the budget and suffixes `…`; shorter paragraphs are returned
/// whole. Returns `None` when the input is empty or whitespace-only.
///
/// Pure, so the unit tests below cover the paragraph / truncation edges that
/// the integration test can't reach cleanly.
fn compute_excerpt(body: &str) -> Option<String> {
    let trimmed = body.trim();
    if trimmed.is_empty() {
        return None;
    }
    let first_paragraph = trimmed
        .split("\n\n")
        .map(str::trim)
        .find(|p| !p.is_empty())?;
    // Collapse intra-paragraph newlines so a soft-wrapped markdown paragraph
    // renders as one flowing sentence in the peek.
    let collapsed: String = first_paragraph
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    if collapsed.chars().count() <= EXCERPT_MAX_CHARS {
        return Some(collapsed);
    }
    // Byte index at the EXCERPT_MAX_CHARS-th character boundary (safe cut).
    let end_byte = collapsed
        .char_indices()
        .nth(EXCERPT_MAX_CHARS)
        .map(|(i, _)| i)
        .unwrap_or(collapsed.len());
    let slice = &collapsed[..end_byte];
    // Prefer to backtrack to the last whitespace in the final 10% of the
    // window so we don't sever mid-word.
    let fallback_char = EXCERPT_MAX_CHARS.saturating_sub(EXCERPT_MAX_CHARS / 10);
    let fallback_byte = slice
        .char_indices()
        .nth(fallback_char)
        .map(|(i, _)| i)
        .unwrap_or(0);
    let cut = slice[fallback_byte..]
        .rfind(' ')
        .map(|off| fallback_byte + off)
        .unwrap_or(slice.len());
    Some(format!("{}…", slice[..cut].trim_end()))
}

/// Parameters for `aggregator_subgraph`.
///
/// Factored into a struct so future filter additions (doc-type excludes,
/// edge-type filters) drop in without refactoring every call site.
#[derive(Debug, Clone)]
pub struct AggregatorSubgraphParams<'a> {
    pub caller_profile_id: Uuid,
    /// Resolved context ID. Callers must resolve the caller-supplied context ref
    /// (via `parse_context_ref` + `resolve_context_ref`) before constructing this
    /// struct — the service layer does not perform name or ref resolution.
    pub context_id: Uuid,
    pub aggregator_types: &'a [DocType],
    pub depth: u32,
}

/// Return a subgraph anchored on the given aggregator doc types within a
/// context, expanded by BFS to `depth` hops.
///
/// The caller's visibility is enforced by `graph_traverse()`'s internal
/// `resources_visible_to` join — cross-owner resources are never returned.
///
/// **Sessions are not nodes.** Per the R11 visual language (sessions are
/// annotations, not graph participants), session-typed resources are filtered
/// out of the returned node set. Each remaining node carries a
/// `session_count` equal to the number of sessions that share an edge with
/// it. Edges whose endpoint is a session are likewise dropped.
///
/// Implementation uses two round-trips:
///
/// 1. `graph_subgraph_nodes(...)` SQL function — seeds + traversed ID set
///    with edge_count, session_count, first_chunk, and stage_raw aggregated
///    via CTEs in a single planned query. Sessions and inactive resources
///    are excluded in the function.
/// 2. Edge rows where both endpoints are in the resolved (non-session) ID set.
pub async fn aggregator_subgraph(
    pool: &PgPool,
    params: AggregatorSubgraphParams<'_>,
) -> ApiResult<SubgraphResponse> {
    // SAFETY CLAMP: unvalidated callers can't DoS Postgres with a runaway
    // recursive CTE. v1 hands us `2`, but the guard is cheap insurance.
    let depth = params.depth.min(MAX_DEPTH);

    // DocType → lowercase name for the kb_doc_types.name match.
    let aggregator_names: Vec<String> = params
        .aggregator_types
        .iter()
        .map(|dt| dt.as_str().to_string())
        .collect();

    let (nodes, node_ids) = fetch_subgraph_nodes(pool, &params, &aggregator_names, depth).await?;

    if node_ids.is_empty() {
        return Ok(SubgraphResponse {
            nodes,
            edges: vec![],
        });
    }

    let edges = fetch_subgraph_edges(pool, &node_ids).await?;

    Ok(SubgraphResponse { nodes, edges })
}

/// Query 1: nodes via the packaged `graph_subgraph_nodes` SQL function. The
/// function does the seed + BFS + edge/session aggregation in a single planned
/// query, avoiding the N*4 correlated subqueries of the prior inline form.
///
/// Returns the projected [`GraphNode`]s alongside their raw ids (the input to
/// the edge query). An empty result yields `(vec![], vec![])`.
async fn fetch_subgraph_nodes(
    pool: &PgPool,
    params: &AggregatorSubgraphParams<'_>,
    aggregator_names: &[String],
    depth: u32,
) -> ApiResult<(Vec<GraphNode>, Vec<Uuid>)> {
    let node_records = sqlx::query!(
        r#"
        SELECT
            resource_id   AS "id!: Uuid",
            slug          AS "slug!",
            title         AS "title!",
            doc_type      AS "doc_type!",
            edge_count    AS "edge_count!: i32",
            session_count AS "session_count!: i32",
            first_chunk   AS "first_chunk: String",
            stage_raw     AS "stage_raw: String"
          FROM graph_subgraph_nodes($1, $2, $3::text[], $4::int)
        "#,
        params.caller_profile_id,
        params.context_id,
        aggregator_names,
        depth as i32,
    )
    .fetch_all(pool)
    .await?;

    let mut node_ids: Vec<Uuid> = Vec::with_capacity(node_records.len());
    let mut nodes: Vec<GraphNode> = Vec::with_capacity(node_records.len());
    for rec in node_records {
        // DocType::from_str returns TemperError on unknown name; map to
        // ApiError::Internal since an unrecognised doctype is a data-integrity issue.
        let doc_type = DocType::from_str(&rec.doc_type)
            .map_err(|e| ApiError::Internal(format!("unexpected doc_type in db: {e}")))?;
        node_ids.push(rec.id);
        let excerpt = rec.first_chunk.as_deref().and_then(compute_excerpt);
        // Stage is task-only. Ignore the managed_meta value on any other
        // doctype even if it happens to carry a `temper-stage` key.
        let stage = if matches!(doc_type, DocType::Task) {
            rec.stage_raw.filter(|s| !s.trim().is_empty())
        } else {
            None
        };
        nodes.push(GraphNode {
            id: ResourceId::from(rec.id),
            slug: rec.slug,
            title: rec.title,
            aggregator: is_aggregator(doc_type),
            doc_type,
            edge_count: rec.edge_count,
            session_count: rec.session_count,
            excerpt,
            stage,
        });
    }

    Ok((nodes, node_ids))
}

/// Query 2: edge rows — both endpoints must be in the resolved set. Because
/// `node_ids` only contains active resources (query 1), no dangling edges can
/// appear here.
async fn fetch_subgraph_edges(pool: &PgPool, node_ids: &[Uuid]) -> ApiResult<Vec<GraphEdge>> {
    let edge_records = sqlx::query!(
        r#"
        SELECT source_id AS "source!: Uuid", target_id AS "target!: Uuid",
               edge_kind AS "edge_kind!: EdgeKind", polarity AS "polarity!: Polarity",
               label AS "label: String"
          FROM kb_edges
         WHERE source_table = 'kb_resources' AND target_table = 'kb_resources'
           AND source_id = ANY($1::uuid[]) AND target_id = ANY($1::uuid[])
           AND NOT is_folded
        "#,
        node_ids,
    )
    .fetch_all(pool)
    .await?;

    let edges: Vec<GraphEdge> = edge_records
        .into_iter()
        .map(|rec| GraphEdge {
            source: rec.source,
            target: rec.target,
            edge_kind: rec.edge_kind,
            polarity: rec.polarity,
            label: rec.label.unwrap_or_default(),
        })
        .collect();

    Ok(edges)
}

/// R4 — team-scoped parameterized neighborhood slice. Service-direct read.
///
/// Composes `graph_traverse_scoped` (team-clamped, edge-kind-filtered BFS) with
/// `graph_atlas_nodes` (node projection over the same team scope) to build the
/// induced Atlas subgraph. Deny-as-absence (404) when the profile cannot view
/// the team (not a member of it or a descendant) — mirrors `team_service::graph_scope`.
pub async fn neighborhood_slice(
    pool: &PgPool,
    profile_id: ProfileId,
    team_id: Uuid,
    req: SliceRequest,
) -> ApiResult<AtlasSubgraph> {
    if req.seeds.is_empty() {
        return Err(ApiError::BadRequest("seeds must be non-empty".into()));
    }
    // Deny-as-absence: profile must read the team (member of it or a descendant).
    let viewable: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM team_descendants($1) d \
         JOIN kb_team_members tm ON tm.team_id = d.team_id AND tm.profile_id = $2)",
    )
    .bind(team_id)
    .bind(profile_id.as_uuid())
    .fetch_one(pool)
    .await?;
    if !viewable {
        return Err(ApiError::NotFound);
    }

    let depth = req.depth.min(MAX_DEPTH) as i32;

    // Walk: returns the edges of the induced subgraph. EdgeKind/Polarity decode
    // natively via their `sqlx::Type` derive (same mechanism as fetch_subgraph_edges
    // above), so req.edge_kinds binds directly as an `edge_kind[]` array param —
    // no `::text` cast round-trip.
    let walked = sqlx::query_as::<_, (Uuid, Uuid, EdgeKind, Polarity, Option<String>, f64, i32)>(
        "SELECT source_id, target_id, edge_kind, polarity, label, weight, depth \
         FROM graph_traverse_scoped($1, $2, $3, $4, $5)",
    )
    .bind(profile_id.as_uuid())
    .bind(team_id)
    .bind(&req.seeds)
    .bind(depth)
    .bind(&req.edge_kinds)
    .fetch_all(pool)
    .await?;

    let edges: Vec<AtlasEdge> = walked
        .iter()
        .map(
            |(source, target, edge_kind, polarity, label, weight, _depth)| AtlasEdge {
                source: *source,
                target: *target,
                edge_kind: *edge_kind,
                polarity: *polarity,
                label: label.clone(),
                weight: *weight,
            },
        )
        .collect();

    // Node id set = seeds ∪ all walked endpoints.
    let mut node_ids: Vec<Uuid> = req.seeds.clone();
    for (s, t, ..) in &walked {
        node_ids.push(*s);
        node_ids.push(*t);
    }

    let nodes: Vec<AtlasNode> = sqlx::query_as::<_, (Uuid, String, Option<String>, String, i32)>(
        "SELECT id, title, doc_type, home, degree FROM graph_atlas_nodes($1, $2, $3)",
    )
    .bind(profile_id.as_uuid())
    .bind(team_id)
    .bind(&node_ids)
    .fetch_all(pool)
    .await?
    .into_iter()
    .map(|(id, title, doc_type, home, degree)| AtlasNode {
        id,
        title,
        doc_type,
        home: if home == "cogmap" {
            NodeHome::Cogmap
        } else {
            NodeHome::Context
        },
        degree,
        salience: None, // neighborhood-tier salience deferred (no per-node source yet)
    })
    .collect();

    Ok(AtlasSubgraph { nodes, edges })
}

/// R2 — Tier-0 territory overview for a team scope. Service-direct read.
pub async fn territory_overview(
    pool: &PgPool,
    profile_id: ProfileId,
    team_id: Uuid,
    lens_id: Option<Uuid>,
) -> ApiResult<TerritoryOverview> {
    let viewable: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM team_descendants($1) d \
         JOIN kb_team_members tm ON tm.team_id = d.team_id AND tm.profile_id = $2)",
    )
    .bind(team_id)
    .bind(profile_id.as_uuid())
    .fetch_one(pool)
    .await?;
    if !viewable {
        return Err(ApiError::NotFound);
    }

    // Default lens = the global 'telos-default' (cogmap_id IS NULL).
    let lens: Uuid = match lens_id {
        Some(l) => l,
        None => sqlx::query_scalar(
            "SELECT id FROM kb_cogmap_lenses WHERE name = 'telos-default' AND cogmap_id IS NULL LIMIT 1",
        )
        .fetch_one(pool)
        .await?,
    };

    let mut territories: Vec<Territory> = Vec::new();

    let regions = sqlx::query_as::<_, (Uuid, Uuid, Option<String>, i32, f64)>(
        "SELECT region_id, cogmap_id, label, member_count, salience FROM graph_region_territories($1, $2, $3)",
    )
    .bind(profile_id.as_uuid())
    .bind(team_id)
    .bind(lens)
    .fetch_all(pool)
    .await?;
    for (region_id, cogmap_id, label, member_count, salience) in regions {
        territories.push(Territory {
            id: region_id,
            kind: TerritoryKind::Region,
            label,
            member_count,
            salience: Some(salience),
            anchor_id: cogmap_id,
        });
    }

    let contexts = sqlx::query_as::<_, (Uuid, String, i32)>(
        "SELECT context_id, label, member_count FROM graph_context_territories($1, $2)",
    )
    .bind(profile_id.as_uuid())
    .bind(team_id)
    .fetch_all(pool)
    .await?;
    for (context_id, label, member_count) in contexts {
        territories.push(Territory {
            id: context_id,
            kind: TerritoryKind::Context,
            label: Some(label),
            member_count,
            salience: None,
            anchor_id: context_id,
        });
    }

    const ORPHAN_LIMIT: usize = 50;
    let orphan_nodes: Vec<OrphanNode> =
        sqlx::query_as::<_, (Uuid, String, Option<String>, i32, Uuid)>(
            "SELECT id, title, doc_type, degree, anchor_id FROM graph_orphan_salient_nodes($1, $2)",
        )
        .bind(profile_id.as_uuid())
        .bind(team_id)
        .fetch_all(pool)
        .await?
        .into_iter()
        .take(ORPHAN_LIMIT)
        .map(|(id, title, doc_type, degree, anchor_id)| OrphanNode {
            id,
            title,
            doc_type,
            degree,
            anchor_id,
        })
        .collect();

    let bridges: Vec<Bridge> = sqlx::query_as::<_, (Uuid, Uuid, i32)>(
        "SELECT source_territory, target_territory, edge_count FROM graph_territory_bridges($1, $2)",
    )
    .bind(profile_id.as_uuid())
    .bind(team_id)
    .fetch_all(pool)
    .await?
    .into_iter()
    .map(|(source_territory, target_territory, edge_count)| Bridge {
        source_territory,
        target_territory,
        edge_count,
    })
    .collect();

    Ok(TerritoryOverview {
        territories,
        orphan_nodes,
        bridges,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn max_depth_constant_is_ten() {
        assert_eq!(MAX_DEPTH, 10);
    }

    #[test]
    fn depth_within_limit_passes_through() {
        // Compile-check: clamp is `params.depth.min(MAX_DEPTH)`.
        // We unit-test the clamp arithmetic; integration tests cover end-to-end.
        assert_eq!(5u32.min(MAX_DEPTH), 5);
    }

    #[test]
    fn depth_over_limit_clamps_to_max() {
        assert_eq!(100u32.min(MAX_DEPTH), 10);
        // Exercise the clamp with a runtime value at the numeric ceiling —
        // a literal u32::MAX trips clippy::unnecessary_min_or_max because the
        // result is statically knowable, but a black_box value preserves the
        // branch coverage we actually want.
        assert_eq!(std::hint::black_box(u32::MAX).min(MAX_DEPTH), MAX_DEPTH);
    }

    // ── compute_excerpt ─────────────────────────────────────────────────

    #[test]
    fn compute_excerpt_returns_none_for_empty_or_whitespace() {
        assert_eq!(compute_excerpt(""), None);
        assert_eq!(compute_excerpt("   \n\n  \t\n"), None);
    }

    #[test]
    fn compute_excerpt_returns_short_paragraph_whole() {
        let body = "Idempotency keys let retries be safe.";
        assert_eq!(
            compute_excerpt(body),
            Some("Idempotency keys let retries be safe.".to_string()),
        );
    }

    #[test]
    fn compute_excerpt_stops_at_first_blank_line() {
        let body = "First paragraph lives here.\n\nSecond paragraph is ignored.";
        assert_eq!(
            compute_excerpt(body),
            Some("First paragraph lives here.".to_string()),
        );
    }

    #[test]
    fn compute_excerpt_collapses_soft_wraps() {
        // Single paragraph with internal newlines collapses to one line — the
        // peek UI handles its own re-flow, so we normalise whitespace.
        let body = "A paragraph soft-wrapped\nacross multiple\nlines.";
        assert_eq!(
            compute_excerpt(body),
            Some("A paragraph soft-wrapped across multiple lines.".to_string()),
        );
    }

    #[test]
    fn compute_excerpt_skips_leading_blank_paragraphs() {
        let body = "\n\n\nActual opener.\n\nTrailing content.";
        assert_eq!(compute_excerpt(body), Some("Actual opener.".to_string()),);
    }

    #[test]
    fn compute_excerpt_truncates_past_max_chars_on_word_boundary() {
        // Build a paragraph well over EXCERPT_MAX_CHARS of ASCII words.
        let long: String = "lorem ipsum dolor sit amet ".repeat(20);
        let excerpt = compute_excerpt(&long).expect("excerpt");
        assert!(excerpt.ends_with('…'), "trailing ellipsis: {excerpt}");
        assert!(
            excerpt.chars().count() <= EXCERPT_MAX_CHARS + 1,
            "length bounded: {} chars",
            excerpt.chars().count()
        );
        // Cut must land on a word boundary: the original paragraph is space-
        // delimited words, and trimming the ellipsis should leave a complete
        // word run that appears verbatim in the source.
        let kept = excerpt.trim_end_matches('…').trim_end();
        assert!(
            long.starts_with(kept),
            "kept prefix must be a prefix of the source, got {kept:?}",
        );
        assert!(
            long[kept.len()..].starts_with(' '),
            "cut must land on a whitespace boundary in the source, byte after kept = {:?}",
            long[kept.len()..].chars().next(),
        );
    }

    #[test]
    fn compute_excerpt_handles_utf8_char_boundaries() {
        // Multi-byte chars must not panic the slice math. Build a paragraph
        // wider than the budget using 3-byte UTF-8 characters.
        let long: String = "漢字 ".repeat(400);
        let excerpt = compute_excerpt(&long).expect("excerpt");
        assert!(excerpt.ends_with('…'));
        assert!(excerpt.chars().count() <= EXCERPT_MAX_CHARS + 1);
    }
}
