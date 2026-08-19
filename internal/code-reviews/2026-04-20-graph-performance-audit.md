# Code Review: Knowledge-Graph Performance & Maintainability Audit

**Branch:** `claude/continue-analysis-migrations-Sbctd`
**Scope:** Graph traversal, aggregator subgraph, edge resolution, and search composition introduced in the R7 knowledge-graph migrations (`20260411000002_knowledge_graph_edges.sql`, `20260411000003_graph_search.sql`) and the services that call them.
**Date:** 2026-04-20
**Status:** Pre-alpha — behavior-breaking changes are acceptable; single rollup PR.

---

## Summary

The R7 knowledge-graph MVP is visibility-correct in the hot paths and composes cleanly through `resources_visible_to()`, but the first call site for the graph-panel UI (`aggregator_subgraph`) embeds four correlated subqueries per returned row, and the edge-reconciliation path runs a 1–2 round-trip resolver once per declaration. Both patterns scale poorly past toy vaults. This audit documents six concrete changes, ordered by value/risk, to be rolled up into a single perf/maintainability PR before release.

---

## Visibility Correctness

All hot paths call `resources_visible_to()` at every boundary:

- `graph_traverse` base + recursive (`migrations/20260411000002_knowledge_graph_edges.sql:91,117`)
- `graph_neighbors` both branches (`migrations/20260411000002_knowledge_graph_edges.sql:157,173`)
- `graph_resource_edges` both branches (`migrations/20260411000002_knowledge_graph_edges.sql:210,229`)
- `aggregator_subgraph` seed CTE (`crates/temper-api/src/services/graph_service.rs:136`), expansion inherits from `graph_traverse`
- `edge_service::resolve_target` all three branches (`crates/temper-api/src/services/edge_service.rs:36,50,68`)
- `fts_search` / `unified_search` / `graph_search` all compose via the function
- `vault_resources_browse` + `resources_visible_to($1)` in `crates/temper-api/src/services/resource_service.rs:230,238,243`

### Watch items (no correctness fix required today)

1. **`session_count` scope** — `crates/temper-api/src/services/graph_service.rs:166-176`. The correlated subquery counts sessions connected to each node regardless of visibility. Intentional per the comment at line 93-97 ("sessions are annotations, not participants"), but in a multi-profile future this leaks an integer count of invisible work. Revisit when multi-profile teams land.
2. **`reconcile_edges` auth** — `crates/temper-api/src/services/edge_service.rs:427-447`. Reads `kb_resource_edges` filtered only by `source/target = $1`. Safe today because the surrounding flow authorizes the caller against the resource, but this reader is not visibility-scoped. Flag for the day `kb_resource_edges` holds cross-owner edges.
3. **`graph_search` score weighting** — `migrations/20260411000003_graph_search.sql:83-84`. Uses `GREATEST(fts, vector)` and ignores `p_fts_weight` / `p_vec_weight` in the non-graph term. `unified_search` respects the weights; `graph_search` does not. Treated as a bug below (C4).

---

## Performance Findings (Ranked)

### F1. `aggregator_subgraph` Query 1: four correlated subqueries per row

`crates/temper-api/src/services/graph_service.rs:162-191`. Each returned node triggers four extra table accesses:

- `edge_count`: full scan of `kb_resource_edges` for `source OR target`
- `session_count`: joins `kb_resource_edges → kb_resources → kb_doc_types` for `source OR target`
- `first_chunk`: scans `kb_current_chunks` (a view over `kb_chunks ⋈ kb_chunk_content`)
- `stage_raw`: per-row lookup on `kb_resource_manifests`

At depth 2 with ~100 nodes that's ~400 extra subplans. Rewrite as CTE aggregations + LEFT JOINs and package the whole thing as a SQL function `graph_subgraph(p_profile_id, p_context_name, p_aggregator_types, p_depth)` so the planner caches the plan and the Rust side becomes `SELECT … FROM graph_subgraph(…)`.

### F2. N+1 in edge resolution and upsert

- `resolve_declarations` calls `resolve_target` per declaration (`crates/temper-api/src/services/edge_service.rs:101-148`)
- `resolve_target` fires 1–2 queries (same-ctx then cross-ctx) (`crates/temper-api/src/services/edge_service.rs:47-76`)
- `upsert_edges` loops one `INSERT` at a time (`crates/temper-api/src/services/edge_service.rs:181-223`)
- `defer_edges` loops one `INSERT` at a time (`crates/temper-api/src/services/edge_service.rs:193-223`)

For a resource with 10 relationships: up to 20 resolution round-trips and 10–20 upsert round-trips per create/update. Fold each into one batch query using `UNNEST`.

### F3. `resolve_target` two-step same-ctx vs cross-ctx

Even without batching, one query using `ORDER BY (r.kb_context_id = $3) DESC LIMIT 2` resolves the same-ctx-wins + ambiguity check in a single round-trip.

### F4. `graph_traverse` materializes the full visibility set

`migrations/20260411000002_knowledge_graph_edges.sql:89-92`. The `visible` CTE materializes every resource the profile can see before the recursion even starts. For a vault with 10k resources, that's 10k rows persisted at the base step for what might be a 50-row traversal. `resources_visible_to` already accepts `p_resource_ids[]`; `graph_traverse`'s base step can pass `p_seed_ids` to pre-filter, a one-line change with no correctness risk. The recursive step can't use it (discovered IDs are unknown), so it keeps the `'{}'` form.

### F5. `list_resource_edges` fires a redundant pre-check

`crates/temper-api/src/services/edge_service.rs:559-560`. Calls `get_visible` first, then `graph_resource_edges` which visibility-checks peers. The outer check is of the resource itself (not peers), so it isn't redundant — but folding an `EXISTS(SELECT 1 FROM resources_visible_to(...) WHERE resource_id = p_resource_id)` into `graph_resource_edges` and returning an empty set saves a round-trip on the 404 path.

---

## Index Gaps

One small migration adds the following:

| Index | Why | Cost |
|---|---|---|
| `idx_kb_resources_ctx_doctype_active` on `(kb_context_id, kb_doc_type_id) WHERE is_active` | Aggregator seed filters on all three; today uses `idx_kb_resources_context` and re-checks `is_active` | Small, writes rare |
| `idx_kb_resource_edges_provenance_src` on `(source_resource_id)` where `metadata->>'provenance' = 'frontmatter'` | `reconcile_edges` filters this on every update (`edge_service.rs:431, 443`) | Tiny |
| `idx_deferred_edges_ctx_ref` on `(target_context_id, target_ref)` | Batched deferred-edge resolution; current `idx_deferred_edges_target_ref` is enough for point lookups but will matter once a create triggers a multi-slug resolve | Tiny |
| `idx_kb_chunks_resource_index` on `(resource_id, chunk_index) WHERE is_current` | `first_chunk` does `ORDER BY chunk_index LIMIT 1` per node; currently sorts via `idx_chunks_resource` | Small; paid once per ingest |

A covering index on edges (`… INCLUDE (edge_type, target_resource_id)`) was considered and deliberately skipped — the existing compound `idx_edges_source_type` already covers the traversal access pattern.

---

## Changes (In Order of Value / Risk)

| # | Change | Type | Blast radius |
|---|---|---|---|
| C1 | Wrap `aggregator_subgraph` as SQL function `graph_subgraph(...)` with CTE aggregation | perf + maintainability | Internal only — Rust call site is the single consumer |
| C2 | Batch `resolve_declarations` / `upsert_edges` / `defer_edges` | perf | Internal only — service surface unchanged |
| C3 | Add four indexes in one migration | perf | DDL only |
| C4 | Fix `graph_search` score weighting — apply `p_fts_weight` / `p_vec_weight` consistently with `unified_search` | **breaking** for any caller passing non-default weights | Search results re-ranked; currently only temper-ui and the CLI consume this. Acceptable in pre-alpha per project owner. |
| C5 | Pass `p_seed_ids` to `resources_visible_to` in `graph_traverse` base step | perf | One-line DDL change; recursive step unchanged |
| C6 | Fold `list_resource_edges` pre-check into `graph_resource_edges` | perf (404 path only) | Internal only |

### Out of scope for this PR

- Watch items W1 (session_count visibility) and W2 (reconcile_edges scope) — defer until multi-profile / cross-team edges land.
- UI-side changes in `packages/temper-ui` are expected to be none (wire format unchanged; C4 changes scores, not the schema). Spot-checked during implementation.

---

## Validation Plan

Each commit:

1. Update `.sqlx/` cache via `cargo sqlx prepare --workspace -- --all-features`
2. `cargo make check` (fmt, clippy, machete, typecheck, biome)
3. `cargo make test-db` (integration tests against real Postgres)
4. Spot check: `temper graph build` + UI graph panel for behavior regressions on C1/C4

Single rollup PR at the end, referencing this document by commit SHA.
