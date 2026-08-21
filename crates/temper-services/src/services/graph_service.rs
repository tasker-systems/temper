//! Graph service — Atlas reads for the knowledge-graph UI: the cogmap-scoped
//! neighborhood slice, the cogmap panorama, the Beat-D region composition slice,
//! and the membership home. Each composes an SQL read (visibility-scoped in the
//! function) and projects it into the Atlas wire shapes.

use sqlx::PgPool;
use uuid::Uuid;

use crate::error::{ApiError, ApiResult};
use temper_core::types::graph::{EdgeKind, Polarity};
use temper_core::types::graph_atlas::{
    AtlasEdge, AtlasNode, AtlasSubgraph, NodeHome, SliceRequest,
};
use temper_core::types::graph_home::{AtlasHome, HomeCogmap, HomeContext};
use temper_core::types::graph_territory::{
    OrphanNode, Territory, TerritoryKind, TerritoryOverview,
};
use temper_core::types::ids::ProfileId;

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

/// A2 — cogmap-scoped R4 neighborhood slice. Composes `graph_traverse_cogmap_scoped`
/// (cogmap-clamped, edge-kind-filtered BFS) with `graph_atlas_nodes_cogmap` (node
/// projection over the same cogmap scope) to build the induced Atlas subgraph.
/// Deny-as-absence (404) when the profile cannot read the cogmap — mirrors
/// `cogmap_panorama`.
pub async fn cogmap_neighborhood_slice(
    pool: &PgPool,
    profile_id: ProfileId,
    cogmap_id: Uuid,
    req: SliceRequest,
) -> ApiResult<AtlasSubgraph> {
    if req.seeds.is_empty() {
        return Err(ApiError::BadRequest("seeds must be non-empty".into()));
    }
    // Deny-as-absence: profile must read the cogmap.
    // `readable!`: sqlx types a function-call column as nullable, but `cogmap_readable_by_profile`
    // is `SELECT EXISTS (...) OR profile_explicit_grant(...)` and `profile_explicit_grant` is itself
    // a bare `SELECT EXISTS (...)` — both arms are total, so the OR can never evaluate to NULL.
    let readable: bool = sqlx::query_scalar!(
        r#"SELECT cogmap_readable_by_profile($1, $2) AS "readable!""#,
        profile_id.as_uuid(),
        cogmap_id,
    )
    .fetch_one(pool)
    .await?;
    if !readable {
        return Err(ApiError::NotFound(
            "cognitive map not found or not readable".to_string(),
        ));
    }

    let depth = req.depth.min(MAX_DEPTH) as i32;

    // Walk: returns the edges of the induced subgraph. EdgeKind/Polarity decode
    // natively via their `sqlx::Type` derive, so req.edge_kinds binds directly as an
    // `edge_kind[]` array param — no `::text` cast round-trip.
    let walked = sqlx::query!(
        r#"SELECT id AS "id!", source_id AS "source_id!", target_id AS "target_id!",
                  edge_kind AS "edge_kind!: EdgeKind", polarity AS "polarity!: Polarity",
                  label, weight AS "weight!"
             FROM graph_traverse_cogmap_scoped($1, $2, $3, $4, $5)"#,
        profile_id.as_uuid(),
        cogmap_id,
        &req.seeds,
        depth,
        &req.edge_kinds as &[EdgeKind],
    )
    .fetch_all(pool)
    .await?;

    let edges: Vec<AtlasEdge> = walked
        .iter()
        .map(|w| AtlasEdge {
            id: w.id,
            source: w.source_id,
            target: w.target_id,
            edge_kind: w.edge_kind,
            polarity: w.polarity,
            label: w.label.clone(),
            weight: w.weight,
        })
        .collect();

    // Node id set = seeds ∪ all walked endpoints.
    let mut node_ids: Vec<Uuid> = req.seeds.clone();
    for w in &walked {
        node_ids.push(w.source_id);
        node_ids.push(w.target_id);
    }

    // Set-returning-function columns all read as nullable, so the non-null ones need an
    // override. In `\sf graph_atlas_nodes_cogmap`: `id`/`title` come from
    // `JOIN kb_resources r ON r.id = ids.id AND r.is_active` and both columns are NOT NULL;
    // `degree` is `COALESCE(deg.degree, 0)`; `home` is a total `CASE … ELSE 'context' END`
    // over `bool_or(...)` in an ungrouped-aggregate LATERAL, which always yields exactly one
    // row and takes the ELSE when `bool_or` is NULL over an empty set. `doc_type` (LEFT JOIN
    // `doc`) and `first_chunk` (a scalar subquery) stay genuinely optional.
    let nodes: Vec<AtlasNode> = sqlx::query!(
        r#"SELECT id AS "id!", title AS "title!", doc_type, home AS "home!",
                  degree AS "degree!", first_chunk
             FROM graph_atlas_nodes_cogmap($1, $2, $3)"#,
        profile_id.as_uuid(),
        cogmap_id,
        &node_ids,
    )
    .fetch_all(pool)
    .await?
    .into_iter()
    .map(|n| AtlasNode {
        id: n.id,
        title: n.title,
        doc_type: n.doc_type,
        home: if n.home == "cogmap" {
            NodeHome::Cogmap
        } else {
            NodeHome::Context
        },
        degree: n.degree,
        salience: None, // neighborhood-tier salience deferred (no per-node source yet)
        excerpt: n.first_chunk.as_deref().and_then(compute_excerpt),
        // graph_atlas_nodes_cogmap does not return a stage column (only the
        // graph_atlas_nodes_visible read was widened for it, spec D8), so None here.
        stage: None,
    })
    .collect();

    Ok(AtlasSubgraph { nodes, edges })
}

/// Cogmap-scoped panorama (enter-a-cogmap). Deny-as-absence via
/// cogmap_readable_by_profile. Returns the R2 TerritoryOverview shape so the
/// frontend renders it with the shipped TierPanorama.
pub async fn cogmap_panorama(
    pool: &PgPool,
    profile_id: ProfileId,
    cogmap_id: Uuid,
    lens_id: Option<Uuid>,
) -> ApiResult<TerritoryOverview> {
    // `readable!` for the reason given on `cogmap_neighborhood_slice`'s identical gate.
    let readable: bool = sqlx::query_scalar!(
        r#"SELECT cogmap_readable_by_profile($1, $2) AS "readable!""#,
        profile_id.as_uuid(),
        cogmap_id,
    )
    .fetch_one(pool)
    .await?;
    if !readable {
        return Err(ApiError::NotFound(
            "cognitive map not found or not readable".to_string(),
        ));
    }

    // Default lens (D2): the lens with the most live regions for THIS cogmap;
    // fall back to the global telos-default if the cogmap has no materialized region.
    let lens: Uuid = match lens_id {
        Some(l) => l,
        None => {
            // `lens!` is the ONE override here that is not provably non-null in SQL: the COALESCE
            // still yields NULL if the global `telos-default` lens row is missing. It is declared
            // non-null because the binding is `let lens: Uuid`, so the runtime version demanded a
            // non-null value already — the annotation states the existing contract rather than
            // widening it, and a missing default lens surfaces the same decode error either way.
            sqlx::query_scalar!(
                r#"SELECT COALESCE(
                 (SELECT lens_id FROM kb_cogmap_regions
                   WHERE cogmap_id = $1 AND NOT is_folded
                   GROUP BY lens_id ORDER BY count(*) DESC LIMIT 1),
                 (SELECT id FROM kb_cogmap_lenses
                   WHERE name = 'telos-default' AND cogmap_id IS NULL LIMIT 1)) AS "lens!""#,
                cogmap_id,
            )
            .fetch_one(pool)
            .await?
        }
    };

    // `region_id!`/`salience!`: `kb_cogmap_regions.id` is the PK and `.salience` is NOT NULL,
    // both reached straight off `FROM kb_cogmap_regions reg`. `cogmap_id!` is the interesting
    // one — that COLUMN is nullable, but the function body's `WHERE reg.cogmap_id = p_cogmap`
    // can never be true for a NULL, so the projected value is non-null by the predicate rather
    // than by the constraint. `member_count!` is `count(*)::int` in a CROSS JOIN LATERAL
    // (ungrouped aggregate ⇒ always one row, 0 over an empty set). `label` is
    // `COALESCE(reg.label, seen.rep_title)` where the fallback is `(array_agg(...))[1]` — NULL
    // when no member survives visibility — and `coherence` is the nullable
    // `reg.content_cohesion`; both stay optional.
    let territories: Vec<Territory> = sqlx::query!(
        r#"SELECT region_id AS "region_id!", cogmap_id AS "cogmap_id!", label,
                  member_count AS "member_count!", salience AS "salience!", coherence
             FROM graph_cogmap_territories($1, $2, $3)"#,
        profile_id.as_uuid(),
        cogmap_id,
        lens,
    )
    .fetch_all(pool)
    .await?
    .into_iter()
    .map(|t| Territory {
        id: t.region_id,
        kind: TerritoryKind::Region,
        label: t.label,
        member_count: t.member_count,
        salience: Some(t.salience),
        coherence: t.coherence,
        anchor_id: t.cogmap_id,
    })
    .collect();

    const ORPHAN_LIMIT: usize = 50;
    // `id!`/`title!`: both NOT NULL on `kb_resources`, reached through
    // `JOIN kb_resources r ON r.id = homed.resource_id AND r.is_active`. `degree!`: unlike its
    // sibling reads this one is NOT wrapped in a COALESCE, but the LATERAL is an ungrouped
    // `count(*)::int`, so it yields one row and 0 rather than no row. `anchor_id!`: the body
    // projects the `p_cogmap` PARAMETER verbatim, and we bind it from a non-null `Uuid`.
    // `doc_type` (LEFT JOIN `doc`) and `anchor_label` (a scalar subquery over `kb_cogmaps`)
    // stay optional.
    let orphan_nodes: Vec<OrphanNode> = sqlx::query!(
        r#"SELECT id AS "id!", title AS "title!", doc_type, degree AS "degree!",
                  anchor_id AS "anchor_id!", anchor_label
             FROM graph_cogmap_orphan_nodes($1, $2)"#,
        profile_id.as_uuid(),
        cogmap_id,
    )
    .fetch_all(pool)
    .await?
    .into_iter()
    .take(ORPHAN_LIMIT)
    .map(|o| OrphanNode {
        id: o.id,
        title: o.title,
        doc_type: o.doc_type,
        degree: o.degree,
        anchor_id: o.anchor_id,
        anchor_label: o.anchor_label,
    })
    .collect();

    // A single cogmap panorama has no cross-cogmap bridges.
    Ok(TerritoryOverview {
        territories,
        orphan_nodes,
        bridges: Vec::new(),
    })
}

/// Hydrate a resource-id set into [`AtlasNode`]s via `graph_atlas_nodes_visible`.
///
/// Visibility-gated **deny-as-absence**: the SQL joins `resources_visible_to`, so any id
/// the profile cannot see (or that is not `is_active`) simply drops out of the result —
/// its existence never leaks. Maps `home`/`first_chunk`/`stage` into the wire shape.
///
/// Shared by [`region_composition_slice`] (Beat D) and
/// [`crate::services::context_graph_service::context_composition`] (Beat E) so the node
/// projection cannot drift between the two composition reads — a bug that copy-paste
/// across the service boundary would eventually produce (SG-3).
pub(crate) async fn hydrate_atlas_nodes_visible(
    pool: &PgPool,
    profile_id: ProfileId,
    node_ids: &[Uuid],
) -> ApiResult<Vec<AtlasNode>> {
    // Same override reasoning as `graph_atlas_nodes_cogmap` above — `\sf
    // graph_atlas_nodes_visible` is the same projection with a `stage` column added:
    // `id`/`title` are NOT NULL through `JOIN kb_resources r … AND r.is_active`, `degree` is
    // `COALESCE(deg.degree, 0)`, and `home` is a total `CASE … ELSE 'context' END` in an
    // ungrouped-aggregate LATERAL. `doc_type`, `first_chunk` and `stage` are genuinely
    // optional (two LEFT JOINs onto `kb_properties` and a scalar subquery).
    Ok(sqlx::query!(
        r#"SELECT id AS "id!", title AS "title!", doc_type, home AS "home!",
                  degree AS "degree!", first_chunk, stage
             FROM graph_atlas_nodes_visible($1, $2)"#,
        profile_id.as_uuid(),
        node_ids,
    )
    .fetch_all(pool)
    .await?
    .into_iter()
    .map(|n| AtlasNode {
        id: n.id,
        title: n.title,
        doc_type: n.doc_type,
        home: if n.home == "cogmap" {
            NodeHome::Cogmap
        } else {
            NodeHome::Context
        },
        degree: n.degree,
        salience: None,
        excerpt: n.first_chunk.as_deref().and_then(compute_excerpt),
        stage: n.stage,
    })
    .collect())
}

/// Beat D — region → resources COMPOSITION drill. Given one or more regions
/// (a shift-selected union), returns the two-axis force-graph: the regions'
/// facets (knowledge axis) plus the context-homed resources they link to (the
/// builder axis), with all edges among that set. Unlike `cogmap_neighborhood_slice`
/// this is NOT fenced to cogmap scope — `graph_region_composition_edges` follows
/// visible edges out to context-homed resources. Deny-as-absence entry gate:
/// every region must exist, be unfolded, and sit in a cogmap the caller can read.
pub async fn region_composition_slice(
    pool: &PgPool,
    profile_id: ProfileId,
    region_ids: &[Uuid],
    depth: i32,
) -> ApiResult<AtlasSubgraph> {
    if region_ids.is_empty() {
        return Err(ApiError::BadRequest("region_ids must be non-empty".into()));
    }

    // Bound the union so the central idea-cluster stays legible (spec §6).
    const MAX_UNION_REGIONS: usize = 6;
    const NODE_CAP: usize = 120;
    // Dedup first: a caller repeating a region id must not trip the entry-gate
    // count check (distinct matched rows vs len), and the union bound counts
    // distinct regions.
    let mut regions: Vec<Uuid> = region_ids.to_vec();
    regions.sort_unstable();
    regions.dedup();
    if regions.len() > MAX_UNION_REGIONS {
        tracing::warn!(
            requested = regions.len(),
            cap = MAX_UNION_REGIONS,
            "region composition union clamped"
        );
        regions.truncate(MAX_UNION_REGIONS);
    }

    // Entry gate (deny-as-absence): every requested region must exist, be
    // unfolded, and be cogmap-readable by the caller. Selecting the count adds
    // no visibility surface — it is strictly less sensitive than the members
    // the composition returns below.
    // `readable!`: `count(*)` returns 0 over an empty set — never NULL.
    let readable: i64 = sqlx::query_scalar!(
        r#"SELECT count(*) AS "readable!" FROM kb_cogmap_regions reg
         WHERE reg.id = ANY($1) AND NOT reg.is_folded
           AND cogmap_readable_by_profile($2, reg.cogmap_id)"#,
        &regions,
        profile_id.as_uuid(),
    )
    .fetch_one(pool)
    .await?;
    if (readable as usize) < regions.len() {
        return Err(ApiError::NotFound(
            "region not found or not readable".to_string(),
        ));
    }

    let depth = depth.clamp(1, 3);

    // Edges of the induced cross-home subgraph. EdgeKind/Polarity decode natively via their
    // `sqlx::Type` derive (`type_name = "edge_kind"` / `"edge_polarity"`), so the columns need
    // only a type override — no `::text` cast round-trip. Every column but `label` is NOT NULL
    // on `kb_edges` and the body's final SELECT reads them straight off `FROM kb_edges e` with
    // inner joins only, so the set-returning function's blanket nullability is the only reason
    // an override is needed at all; `kb_edges.label` is genuinely nullable.
    let walked = sqlx::query!(
        r#"SELECT id AS "id!", source_id AS "source_id!", target_id AS "target_id!",
                  edge_kind AS "edge_kind!: EdgeKind", polarity AS "polarity!: Polarity",
                  label, weight AS "weight!"
             FROM graph_region_composition_edges($1, $2, $3)"#,
        profile_id.as_uuid(),
        &regions,
        depth,
    )
    .fetch_all(pool)
    .await?;

    // Node id set: region members (seeds) FIRST so NODE_CAP never drops a facet in
    // favour of a neighbor, then the walked endpoints. Seeds also ensure an
    // isolated facet with no edges still renders.
    let seeds: Vec<Uuid> = sqlx::query_scalar!(
        "SELECT DISTINCT member_id FROM kb_cogmap_region_members WHERE region_id = ANY($1)",
        &regions,
    )
    .fetch_all(pool)
    .await?;
    let mut seen: std::collections::HashSet<Uuid> = std::collections::HashSet::new();
    let mut node_ids: Vec<Uuid> = Vec::new();
    for id in seeds
        .into_iter()
        .chain(walked.iter().flat_map(|w| [w.source_id, w.target_id]))
    {
        if seen.insert(id) {
            node_ids.push(id);
        }
    }
    if node_ids.len() > NODE_CAP {
        tracing::warn!(
            nodes = node_ids.len(),
            cap = NODE_CAP,
            "region composition node set clamped (seeds kept, neighbors dropped)"
        );
        node_ids.truncate(NODE_CAP);
    }

    let nodes = hydrate_atlas_nodes_visible(pool, profile_id, &node_ids).await?;

    // Keep only edges whose BOTH endpoints made the final (capped + visibility- and
    // is_active-gated) node set, so the wire payload never references a node the
    // client can't place — no dangling edge into a dropped node.
    let present: std::collections::HashSet<Uuid> = nodes.iter().map(|n| n.id).collect();
    let edges: Vec<AtlasEdge> = walked
        .into_iter()
        .filter(|w| present.contains(&w.source_id) && present.contains(&w.target_id))
        .map(|w| AtlasEdge {
            id: w.id,
            source: w.source_id,
            target: w.target_id,
            edge_kind: w.edge_kind,
            polarity: w.polarity,
            label: w.label,
            weight: w.weight,
        })
        .collect();

    Ok(AtlasSubgraph { nodes, edges })
}

/// Atlas Home — the you→teams→cogmaps membership graph with count hints.
/// No entry gate: the read is inherently self-scoped (member teams +
/// cogmap_visible_maps), so it returns exactly what the caller may see.
pub async fn atlas_home(pool: &PgPool, profile_id: ProfileId) -> ApiResult<AtlasHome> {
    // build lens — the contexts the profile can build in (personal + team), each
    // sized + owner-scoped. Visibility-gated inside graph_home_contexts.
    // From `\sf graph_home_contexts`: `id`/`name`/`slug` are NOT NULL on `kb_contexts`, reached
    // through `JOIN kb_contexts c ON c.id = cand.context_id`. `owner_ref` is a TOTAL `CASE` —
    // every arm is a literal, a `IS NOT NULL`-guarded concat, or `COALESCE(…, 'shared')` — so
    // it cannot be NULL. `resource_count` is `(SELECT count(*) …)::int`, a scalar aggregate
    // subquery that always returns a row. `last_active_at` is `(SELECT max(rr.updated) …)`,
    // which IS NULL for a context with no visible active resource — it stays optional.
    let build: Vec<HomeContext> = sqlx::query!(
        r#"SELECT context_id AS "context_id!", name AS "name!", slug AS "slug!",
                  owner_ref AS "owner_ref!", resource_count AS "resource_count!",
                  last_active_at
             FROM graph_home_contexts($1)"#,
        profile_id.as_uuid(),
    )
    .fetch_all(pool)
    .await?
    .into_iter()
    .map(|c| HomeContext {
        id: c.context_id,
        name: c.name,
        slug: c.slug,
        owner_ref: c.owner_ref,
        resource_count: c.resource_count,
        last_active_at: c.last_active_at,
    })
    .collect();

    // research lens — the cogmaps the profile can reach, with a derived held-by scope.
    // From `\sf graph_home_cogmaps`: `id`/`name` are NOT NULL on `kb_cogmaps`, reached through
    // `JOIN kb_cogmaps c ON c.id = v.cogmap_id`. `owner_ref` is `COALESCE('+' || min(mt.slug),
    // 'temper')` and `team_ids` is `COALESCE(array_agg(…) FILTER (…), '{}')` — both COALESCE
    // onto a non-null literal, which is exactly what makes the empty case `'{}'` rather than
    // NULL. `region_count`/`facet_count` are `(SELECT count(*) …)::int` scalar subqueries.
    let research: Vec<HomeCogmap> = sqlx::query!(
        r#"SELECT cogmap_id AS "cogmap_id!", name AS "name!", owner_ref AS "owner_ref!",
                  team_ids AS "team_ids!", region_count AS "region_count!",
                  facet_count AS "facet_count!"
             FROM graph_home_cogmaps($1)"#,
        profile_id.as_uuid(),
    )
    .fetch_all(pool)
    .await?
    .into_iter()
    .map(|m| HomeCogmap {
        id: m.cogmap_id,
        name: m.name,
        owner_ref: m.owner_ref,
        team_ids: m.team_ids,
        region_count: m.region_count,
        facet_count: m.facet_count,
    })
    .collect();

    Ok(AtlasHome { build, research })
}

/// Default number of marks the entry read draws. **This is not the ruled K.**
///
/// Chunk A's job was to measure the degree distribution, not to choose from it: the spec exists
/// because asserting this number instead of measuring it is the error that produced the 244-of-250
/// draw in the first place. K and rung 2's threshold are ruled in chunk C from A's report
/// (spec §10.1). This constant is a *parameter's* default so the handler has something to answer
/// with before C runs, and C is expected to replace it with a measured value.
///
/// The measurement it will be replaced from, taken on production `[2026-08-21]` over a
/// 3574-resource / 4385-edge corpus, ranking by corpus degree and drawing the induced subgraph:
///
/// | K | cut degree | induced edges | unconnected | % |
/// |---|---|---|---|---|
/// | 50 | ≥16 | 56 | 18 | 36.0% |
/// | 130 | ≥11 | 275 | 26 | 20.0% |
/// | 250 | ≥8 | 594 | 51 | 20.4% |
/// | 549 | ≥5 | 1441 | 61 | 11.1% |
///
/// Two things in that table are worth carrying forward. The unconnected band gets **worse as K
/// shrinks**, not better. And spec §10.1's claim that degree ordering leaves every drawn node
/// connected "by construction" is **false** — at K=50, 18 of 50 nodes with corpus degree ≥ 16 have
/// no edge inside the drawing, because a hub's neighbours are typically leaves and leaves miss the
/// cut. 130 is used here only because it matches the node count of the answered state the reader
/// did not complain about, at a materially better band (20% against that state's 35%).
const ENTRY_DEFAULT_K: i32 = 130;

/// Hard ceiling on the entry read, independent of what the caller asks for.
///
/// Follows `region_composition_slice`'s `NODE_CAP` precedent rather than inventing a policy: a
/// bound the caller cannot raise is what stops one query from becoming the hairball this chunk was
/// written to prevent. Clamped loudly — the drop is logged, never silent.
const ENTRY_MAX_K: i32 = 600;

/// The entry read (spec §5.1) — **a place, and no question at all.**
///
/// Returns the K most-connected resources the caller can see, plus **every edge among them**, as an
/// `AtlasSubgraph`. No seeds required: this is the door for a reader who has supplied nothing.
///
/// The rule that makes it work, and the whole point of the chunk:
///
/// > **Rank by corpus degree; return the induced subgraph over the top-K.**
///
/// Ranking and drawing are then decided by the *same* criterion, so every returned edge has both
/// endpoints in the returned node set **by construction**. The defect this replaces did the
/// opposite — it walked from all 3561 visible resources while drawing 200 rows chosen by
/// `updated DESC`, two sets picked by unrelated criteria, so 244 of 250 marks came out unconnected.
///
/// Degree-zero resources are ranked and may be drawn. That is deliberate: a read must not make
/// presentation decisions, and the fallback rung is the client's to choose (spec §6).
///
/// Note the node payload carries `AtlasNode.degree` = **corpus** degree, because A reuses the one
/// incumbent node shape (spec §5.1). Spec §5.3's ruling that only the derived degree reaches the
/// screen is a claim about the *screen*, not the wire, and it holds only while the client keeps
/// recomputing degree over the drawn edge set.
pub async fn entry_orientation_slice(
    pool: &PgPool,
    profile_id: ProfileId,
    k: Option<i32>,
) -> ApiResult<AtlasSubgraph> {
    let requested = k.unwrap_or(ENTRY_DEFAULT_K);
    if requested <= 0 {
        return Err(ApiError::BadRequest("k must be positive".into()));
    }
    let k = if requested > ENTRY_MAX_K {
        tracing::warn!(requested, cap = ENTRY_MAX_K, "entry read k clamped");
        ENTRY_MAX_K
    } else {
        requested
    };

    // Rank by CORPUS degree. The degree predicate lives in `edges_visible_to`, which the ranking
    // function joins rather than restates — the same set the incumbent hydration counts, so the
    // ranking cannot drift from the number the node carries.
    //
    // `resource_id!`/`degree!`: both are NOT NULL in the body (`v.id` comes through an inner join
    // onto the visibility set, `degree` is a `count(*)::int`); the overrides exist only because a
    // set-returning function types every column as nullable.
    let ranked = sqlx::query!(
        r#"SELECT resource_id AS "resource_id!", degree AS "degree!"
             FROM graph_visible_degree_ranking($1, $2)"#,
        profile_id.as_uuid(),
        k,
    )
    .fetch_all(pool)
    .await?;

    let node_ids: Vec<Uuid> = ranked.iter().map(|r| r.resource_id).collect();
    if node_ids.is_empty() {
        // Rung 3 (spec §6): nothing readable. An empty subgraph, not an error — the caller
        // distinguishes "no corpus" from "no structure", and both are declarations it must make.
        return Ok(AtlasSubgraph {
            nodes: vec![],
            edges: vec![],
        });
    }

    // Depth 0 is load-bearing, not a default: it makes this the INDUCED subgraph over exactly the
    // ranked ids, with no outward expansion. Any depth above 0 would reintroduce endpoints that
    // are not drawn — which is precisely the bug. Measured on production at K=130: depth 0 returns
    // 275 edges, depth 1 returns 2672.
    //
    // Same override reasoning as `region_composition_slice`: every column but `label` is NOT NULL
    // on `kb_edges` and the body's final SELECT reads them off `FROM kb_edges e` through inner
    // joins only. EdgeKind/Polarity decode natively via their `sqlx::Type` derive.
    let walked = sqlx::query!(
        r#"SELECT id AS "id!", source_id AS "source_id!", target_id AS "target_id!",
                  edge_kind AS "edge_kind!: EdgeKind", polarity AS "polarity!: Polarity",
                  label, weight AS "weight!"
             FROM graph_induced_edges($1, $2, 0)"#,
        profile_id.as_uuid(),
        &node_ids,
    )
    .fetch_all(pool)
    .await?;

    // `graph_atlas_nodes_visible` carries NO `ORDER BY` — it is a hydration projection, and the row
    // order it happens to return is a query-plan artifact. Re-imposing the ranking here is what
    // makes "most-connected first" a property of the RESPONSE rather than of the id list that
    // produced it. Found by a bite probe: reversing the ranking direction in SQL left two order
    // assertions still passing, because the order they were reading was never the ranking's.
    let rank_of: std::collections::HashMap<Uuid, usize> = node_ids
        .iter()
        .enumerate()
        .map(|(i, id)| (*id, i))
        .collect();
    let mut nodes = hydrate_atlas_nodes_visible(pool, profile_id, &node_ids).await?;
    nodes.sort_by_key(|n| rank_of.get(&n.id).copied().unwrap_or(usize::MAX));

    // Keep only edges whose BOTH endpoints survived hydration, so the wire payload never dangles an
    // edge into a node the client cannot place. Depth 0 already guarantees both endpoints were in
    // the ranked set; this covers the narrower case of a node lost between the two reads.
    let present: std::collections::HashSet<Uuid> = nodes.iter().map(|n| n.id).collect();
    let edges: Vec<AtlasEdge> = walked
        .into_iter()
        .filter(|w| present.contains(&w.source_id) && present.contains(&w.target_id))
        .map(|w| AtlasEdge {
            id: w.id,
            source: w.source_id,
            target: w.target_id,
            edge_kind: w.edge_kind,
            polarity: w.polarity,
            label: w.label,
            weight: w.weight,
        })
        .collect();

    Ok(AtlasSubgraph { nodes, edges })
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
