# Phase 3: Combined Vector + Graph Search

**Date:** 2026-04-11
**Status:** Design
**Depends on:** Phase 1 (schema + SQL functions), Phase 2 (edge extraction in ingest pipeline)

## Problem

Search currently returns results based on full-text and vector similarity only. Documents with structural relationships (extends, depends_on, references) are invisible to the search pipeline. A user searching for "deployment config" should also see the architecture doc that the top result depends_on — but today that connection is lost.

Phase 1 built the graph schema and traversal functions. Phase 2 wires edge extraction into the ingest pipeline. Phase 3 makes those edges useful at query time by composing graph traversal with the existing unified search.

## Design

### SearchParams Extension

Add four fields to `SearchParams` in `temper-core/src/types/api.rs`:

| Field | Type | Default | Purpose |
|-------|------|---------|---------|
| `seed_ids` | `Option<Vec<Uuid>>` | `None` | Explicit seed resources for graph expansion |
| `edge_types` | `Option<Vec<String>>` | `None` | Filter to specific edge types during expansion |
| `graph_depth` | `Option<i32>` | `None` (server default: 2) | Max hops for graph traversal |
| `graph_expand` | `bool` | `true` | Opt-out flag — set `false` for pure FTS/vector |

All fields use `#[serde(default)]` so existing callers don't break. `graph_expand` defaults to `true` via a serde default function, meaning all existing search calls automatically get graph-enhanced results once edges exist.

`vector_weight` and `graph_weight` are server-side constants (0.7 / 0.3 per R7), not exposed to callers. Can be promoted to params later if needed.

### SQL Function: `graph_search()`

New migration. Composes `unified_search()` as a CTE, then graph-expands from its results:

```
graph_search(profile_id, query, embedding, search_config, context, doc_type,
             fts_weight, vec_weight, seed_ids, edge_types, graph_depth,
             graph_weight, limit, offset)

Stage 1: Call unified_search() as a CTE → base_results
Stage 2: Collect seeds = base_results.resource_id UNION p_seed_ids
Stage 3: Call graph_traverse(profile_id, seeds, graph_depth, edge_types)
         → graph_hits (exclude depth=0 seeds)
Stage 4: Score graph hits: max(path_weight / (depth + 1)) per resource
Stage 5: FULL OUTER JOIN base_results with graph_hits
         combined_score = 0.7 * max(fts_score, vector_score) + 0.3 * graph_proximity
Stage 6: Return same columns as unified_search() + extended origin values
```

Return type matches `unified_search()` exactly — same `UnifiedSearchResultRow` on the Rust side. The `origin` column gains a `'graph'` value for resources found only via graph expansion.

When there are no edges in the graph, `graph_traverse()` returns zero rows, so the function degrades to `unified_search()` results with unchanged scores. Zero cost when the graph is empty.

### Validation Change

Current rule: "at least one of query or embedding." New rule: "at least one of query, embedding, or seed_ids." This enables pure graph traversal: pass `seed_ids` with no query/embedding to explore the graph from known starting points.

When `graph_expand: false`, fall through to `unified_search()` directly — skip the graph function entirely.

### Service Routing

In `search_service.rs`, the `search()` function routes:

```
if !graph_expand:
    → unified_search()           # existing path, no change
else:
    → graph_search()             # new path, composes unified + graph
```

Same return type, same handler, same API response shape. The caller doesn't know which path ran.

### Origin Values

| Value | Meaning |
|-------|---------|
| `fts` | Found via full-text search only |
| `vector` | Found via vector similarity only |
| `graph` | Found via graph expansion only |
| `both` | Found via multiple signals |

The `origin` field is debugging telemetry. Callers should rank by `combined_score`, not origin.

## Testing

### E2E Test (graph_search_test.rs)

Ingest a small document graph through the full pipeline:
- Doc A ("Deployment Config") with dummy embeddings
- Doc B ("Architecture Design") — A declares `extends: [architecture-design]`
- Doc C ("Data Model") — B declares `depends_on: [data-model]`

Search with a vector that matches Doc A. Verify:
1. With `graph_expand: true` (default): A, B, C all appear in results
2. With `graph_expand: false`: only A appears (no graph expansion)
3. With `seed_ids: [B.id]` and no query/embedding: B and C appear via pure traversal

### Integration Test (temper-api level)

Test `graph_search()` SQL function directly: insert resources, edges, dummy chunks, call the function, verify scoring and origin values.

### Unit Tests

- Validation accepts seed_ids as sole input
- `graph_expand` defaults to true in deserialization
- SearchParams serde round-trip with new fields

## Non-Goals

- Custom vector/graph weight tuning per request (server defaults for now)
- Graph visualization or graph-specific response fields (Phase 4 scope)
- CLI `--mode graph` flag (Phase 4 scope)
- Bidirectional graph expansion in SQL (forward-only via `graph_traverse()`, which is what Phase 1 built; bidirectional is a future enhancement)

## Risks

| Risk | Mitigation |
|------|-----------|
| Query plan instability from complex CTE nesting | `graph_search()` calls `unified_search()` as a subquery, not a nested CTE. Postgres can optimize each independently. Monitor with EXPLAIN ANALYZE. |
| Graph expansion returns too many results, diluting relevance | graph_weight is 0.3 vs 0.7 for vector/FTS — distant graph results score low. Default depth=2 limits expansion. |
| Performance regression on non-graph queries | When `graph_expand: false`, goes straight to `unified_search()`. When `true` but no edges exist, `graph_traverse()` returns empty — negligible overhead. |
