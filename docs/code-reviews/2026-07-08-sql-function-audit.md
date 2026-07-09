# SQL Function Audit — 2026-07-08

Audit of all **129 live PostgreSQL functions** (definitions dumped from the migrated dev database — migrations are append-only, so the live definition is authoritative) across five dimensions:

| Rule | Dimension |
|------|-----------|
| SQLA-1 | Indexing & performance |
| SQLA-2 | Design consistency |
| SQLA-3 | Correctness for purpose |
| SQLA-4 | RBAC & visibility |
| SQLA-5 | PG17/PG18 portability |

Method: 7 domain-unit auditors (access-rbac, event-projections, mutation-commands, graph-atlas, search-wayfind, cogmap-analytics, steward-jobs-teams) → adversarial per-unit verification → aggregation. Every function was examined and is accounted for as either a finding or an explicit clean bill. **15 findings survived verification; 114 functions came back clean. Zero SQLA-5 (portability) findings.**

## Summary

Of Temper's 129 live PostgreSQL functions, 114 came back clean and 15 confirmed findings survived adversarial verification — a healthy surface whose problems cluster in two patterns rather than being scattered rot. The dominant pattern is SQLA-1 (9 findings): missing or unusable indexes on the append-only spine tables (kb_events x3, kb_edges, kb_resource_homes, kb_workflow_jobs) plus three structural cost issues (per-row visibility recompute in edges_visible_to, an unclamped recursion, a sibling-divergent degree predicate) — systemic across 4 units and largely fixable with one mechanical index-pack migration plus two targeted function rewrites. The second systemic class is SQLA-3 lifecycle-flag asymmetry (3 findings): readers that ignore is_active or is_folded where sibling functions gate on them, producing a soft-deleted-content leak through resources_readable_by consumers and superseded-charter prose leaking into FTS. One-offs round it out: a dead function pair (team_viewable_by/team_child_zones), vis_team definitional drift (under-shows, no breach), and one unbounded-depth defensive gap. On the RBAC/visibility dimension the model is fundamentally sound: exactly one confirmed SQLA-4 finding in the entire audit — graph_subgraph_nodes' edge_count aggregates edges without an edges_visible_to gate (count-only disclosure, rendered edges are properly gated) — while the core access functions (resources_visible_to, can, endpoint_readable_by_profile, the cogmap/context scope family) are all clean, so six of seven units sign off clean and only graph-atlas is flagged. All fixes ship as new additive migrations (CREATE OR REPLACE FUNCTION / CREATE INDEX), PG17/18-portable, broken into 9 independently reviewable PR chunks below.

## RBAC / visibility sign-off

| Unit | Status | Note |
|------|--------|------|
| access-rbac | **clean** | No SQLA-4 findings. Core visibility/authz functions (resources_visible_to, can, endpoint_readable_by_profile, cogmap/context scope family, 19 fns examined) enforce correctly. Unit carries SQLA-1/2/3 perf and lifecycle findings, but no over-show of protected data through the RBAC rules themselves — the soft-delete content leak is classified SQLA-3 lifecycle asymmetry, tracked in chunk 2. |
| event-projections | **clean** | No SQLA-4 findings across 27 projection/trigger functions. The _rebuild_resource_search_vector folded-block leak is an SQLA-3 consistency bug (stale content in FTS), not a visibility-boundary breach. |
| mutation-commands | **clean** | All 19 mutation commands clean — auth-before-write pattern holds; no SQLA-4 findings. |
| graph-atlas | **flagged** | graph_subgraph_nodes: edge_count subquery (functions-def.sql:1898-1900) counts every non-folded resource-resource edge with NO edges_visible_to gate, unlike siblings graph_atlas_nodes_visible/_cogmap — leaks the count of edges to invisible resources. Count-only disclosure (rendered edges ARE gated), severity low, but it is the audit's one confirmed RBAC finding. Fix is chunk 1. |
| search-wayfind | **clean** | All 4 functions clean; unified_search and wayfind scope through the visibility functions correctly. |
| cogmap-analytics | **clean** | No SQLA-4 findings across 12 functions; cogmap_staleness finding is a pure index-usability issue (SQLA-1). |
| steward-jobs-teams | **clean** | No SQLA-4 findings across 13 functions; the two findings here (steward_ingest_delta, workflow_job_redrive_resource) are missing-index perf gaps. |

## Findings by rule

### SQLA-1 — 9 findings (systemic)

Systemic — spans 9 functions across 4 units (access-rbac, graph-atlas, cogmap-analytics, steward-jobs-teams). Two sub-patterns: (a) missing/unusable indexes on append-only spine tables — kb_events payload-key scans (element_trail_edge/node), kb_events producing-anchor scan (steward_ingest_delta), kb_edges folded-inclusive home-anchor scan (cogmap_staleness), kb_resource_homes owner columns (resources_visible_to), kb_workflow_jobs status='dead' (workflow_job_redrive_resource) — 6 findings fixable mostly by one CREATE INDEX migration; (b) structural cost — per-row resources_visible_to recompute in edges_visible_to (hidden N+1 into graph_atlas), unclamped recursion depth in graph_traverse, index-defeating missing table predicate in graph_cogmap_orphan_nodes.

### SQLA-2 — 2 findings (one-off)

Two one-offs, both in access-rbac: team_viewable_by (+ helper team_child_zones) is confirmed dead code that also embeds the team_descendants soft-delete bug if ever revived; vis_team has drifted from resources_in_team_scope by omitting the D3a container read-grant branch — under-shows only (no leak), and the narrowing may be intentional, so verify intent before changing.

### SQLA-3 — 3 findings (systemic)

Systemic as a class — lifecycle-flag asymmetry recurs across 3 functions in 2 units: readers ignore a soft-state flag that sibling functions gate on. is_active: resources_readable_by feeds three content surfaces (cogmap_regulation, resource_blocks, resource_block_provenance) that serve soft-deleted resources' content while graph_atlas_nodes filters them; team_descendants omits the is_active gates team_ancestors applies (latent — consumers are dead). is_folded: _rebuild_resource_search_vector includes folded blocks in the FTS vector while _recompute_resource_body_hash and every vector/read gate exclude them, so superseded charter prose stays searchable and body_hash diverges.

### SQLA-4 — 1 finding (one-off)

One confirmed finding in 129 functions: graph_subgraph_nodes' ungated edge_count (degree includes edges to invisible resources — count-only disclosure). Every other read surface gates through resources_visible_to/edges_visible_to correctly. The RBAC model itself is sound.

## Confirmed findings

### `cogmap_staleness` — SQLA-1 (medium) · `partial-index-excludes-folded`

**Unit:** cogmap-analytics · **File:** `migrations/20260624000002_canonical_functions.sql` (span 542-544) · **Effort:** S

**Evidence:** touch CTE edges leg: SELECT ev.occurred_at FROM kb_edges e JOIN kb_events ev ON ev.id = e.last_event_id WHERE e.home_anchor_table = 'kb_cogmaps' AND e.home_anchor_id = p_cogmap -- no is_folded restriction; only index is partial idx_kb_edges_home ... WHERE (NOT is_folded)

**Why it matters:** kb_edges is the largest-growing table; cogmap_staleness runs on every cogmap_analytics call. Live EXPLAIN proves the current predicate cannot use the only home-anchor index (partial on NOT is_folded), forcing a seq scan that grows with the corpus.

**Suggested fix:** Either add a non-partial index on kb_edges (home_anchor_table, home_anchor_id) [optionally INCLUDE last_event_id] to serve the folded-inclusive scan, OR add AND NOT e.is_folded to the edges leg (and AND NOT reg.is_folded to the regions leg) so the existing partial index applies -- decide based on whether folds should count as a touch.

**Verification note:** Confirmed in functions-def.sql (edges leg has no NOT is_folded) and live EXPLAIN: current query Seq Scans kb_edges; adding AND NOT e.is_folded switches the plan to Index Scan using idx_kb_edges_home. indexes.txt has no non-partial home-anchor index, so the WHERE-(NOT is_folded) partial cannot serve the folded-inclusive predicate. Real index gap on the schema's largest-growing table, invoked per cogmap_analytics read; medium/effort-S stand.

### `edges_visible_to` — SQLA-1 (medium) · `per-row-visibility-recompute`

**Unit:** access-rbac · **File:** `migrations/20260624000002_canonical_functions.sql` (span 305) · **Effort:** M

**Why it matters:** Feeds graph_atlas degree laterals JOINed row-by-row; hidden quadratic in edges × resources_visible_to cost.

**Suggested fix:** Materialize resources_visible_to(p_profile) once (CTE) and semi-join both endpoints + anchor set-based.

**Verification note:** Confirmed structurally (canonical:305): kb_edges scanned with three per-row scalar predicates; endpoint_readable_by_profile('kb_resources',...) runs `IN (SELECT resource_id FROM resources_visible_to(p_profile))`, invoked for both source and target on every surviving edge. STABLE scalar funcs in a WHERE filter are re-evaluated per row (no auto-memoize), so cost ≈ edges × cost(resources_visible_to). Genuine N+1; fine on tiny dev data, degrades with graph size.

### `element_trail_edge` — SQLA-1 (medium) · `event-payload-scan`

**Unit:** graph-atlas · **File:** `migrations/20260706000002_element_trail_payload_actor.sql` (span 7-22) · **Effort:** ?

**Why it matters:** Full sequential scan of the unbounded kb_events log on every edge-trail lookup.

**Suggested fix:** Add an expression index on ((payload->>'edge_id')::uuid) or route through the indexed references column.

**Verification note:** Confirmed live (event_service.rs:88 + .sqlx cache). functions-def.sql:1291 joins kb_events on (payload->>'edge_id')::uuid; indexes.txt lists only correlation/emitter/invocation/references(gin)/type/pkey on kb_events — none serves the payload edge_id expression, so an append-only log is seq-scanned per trail read. references gin (jsonb_path_ops) does not serve a ->>' expression predicate.

### `element_trail_node` — SQLA-1 (medium) · `event-payload-scan`

**Unit:** graph-atlas · **File:** `migrations/20260706000002_element_trail_payload_actor.sql` (span 25-50) · **Effort:** ?

**Why it matters:** Three unindexed full scans of the unbounded event log per node-trail lookup.

**Suggested fix:** Add expression indexes for the payload keys or migrate the lookups to the indexed references column.

**Verification note:** Confirmed live (event_service.rs:102). functions-def.sql:1309-1319 UNIONs three kb_events scans keyed on payload->>'resource_id', payload->'owner'->>'id', and a block_id join; indexes.txt has no kb_events index on any payload key — three seq scans of the append-only log per node-trail read. Same class as element_trail_edge, tripled.

### `resources_visible_to` — SQLA-1 (medium) · `missing-owner-home-index`

**Unit:** access-rbac · **File:** `migrations/20260703000001_team_metadata_soft_delete.sql` (span 84) · **Effort:** S

**Why it matters:** Hottest access fn; opening branch full-scans kb_resource_homes (one row/resource) on every check.

**Suggested fix:** Add btree indexes on kb_resource_homes(owner_profile_id) and (originator_profile_id); the OR resolves to a BitmapOr.

**Verification note:** Confirmed: indexes.txt lists only pkey(id), resource_id_key(resource_id), anchor(anchor_table,anchor_id) on kb_resource_homes — nothing on owner_profile_id/originator_profile_id; first UNION branch (migration line 84) has no resource_id restriction so it full-scans on every visibility check by the hottest fn in the model. Real index gap.

### `steward_ingest_delta` — SQLA-1 (medium) · `missing-anchor-index`

**Unit:** steward-jobs-teams · **File:** `migrations/20260701000005_steward_ingest_watermark.sql` (span 62-63) · **Effort:** S

**Evidence:** WHERE e.producing_anchor_table='kb_contexts' AND e.producing_anchor_id IN (SELECT id FROM team_ctx) AND (p_watermark IS NULL OR e.id > p_watermark). indexes.txt kb_events lists only pkey(id), emitter(emitter_entity_id,occurred_at), invocation_id, correlation_id, event_type_id, references(gin) — none on producing_anchor_id/table.

**Why it matters:** kb_events is the unbounded append-only event spine; the anchor lookup seq-scans it, and steward_drift_sweep calls this once per candidate cogmap via CROSS JOIN LATERAL, giving O(candidates x |kb_events|).

**Suggested fix:** Add btree on (producing_anchor_table, producing_anchor_id, id) (or a partial index on producing_anchor_id WHERE producing_anchor_table='kb_contexts') so both the anchor equality and the id> watermark range are index-served.

**Verification note:** Confirmed: live EXPLAIN shows Seq Scan on kb_events with Filter on producing_anchor_table/producing_anchor_id; indexes.txt has no covering index. Unbounded table + per-cogmap LATERAL call makes medium severity correct.

### `_rebuild_resource_search_vector` — SQLA-3 (medium) · `folded-block-leaks-into-fts`

**Unit:** event-projections · **File:** `migrations/20260626000001_fts_search_index.sql` (span 27-30) · **Effort:** S

**Evidence:** Line 545-548: FROM kb_chunks c JOIN kb_chunk_content cc ON cc.chunk_id=c.id WHERE c.resource_id=p_resource AND c.is_current — missing NOT b.is_folded join present in _recompute_resource_body_hash (574) and all read gates.

**Why it matters:** cogmap_charter_set re-set on an existing telos folds old blocks (line 177) but never sets their chunks is_current=false; _project_blocks creates new block IDs so old is_current chunks survive, so the rebuilt search_vector includes both stale and new charter prose, diverging from body_hash which excludes folded blocks and letting FTS match superseded content.

**Suggested fix:** Add the live-block join: FROM kb_chunks c JOIN kb_content_blocks b ON b.id=c.block_id AND NOT b.is_folded JOIN kb_chunk_content cc ON cc.chunk_id=c.id WHERE c.resource_id=p_resource AND c.is_current, mirroring _recompute_resource_body_hash and the vector gates.

**Verification note:** Confirmed in functions-def.sql (545-548) and migration (27-30): body agg gates only c.is_current, no NOT b.is_folded, while sibling _recompute_resource_body_hash (574) and every vector/read gate (966,1042,1412,1458,1499,1906) require NOT b.is_folded; charter supersede folds blocks (177) but only writer of is_current=false is block-scoped (91) and _project_blocks inserts fresh block IDs, so re-set leaks superseded charter prose into the telos search_vector and diverges from body_hash — real bug, medium is right.

### `resources_readable_by` — SQLA-3 (medium) · `soft-delete-content-leak`

**Unit:** access-rbac · **File:** `migrations/20260624000002_canonical_functions.sql` (span 244) · **Effort:** M

**Why it matters:** Content surfaces trusting resources_readable_by omit the is_active guard that graph surfaces apply.

**Suggested fix:** Centralize by filtering kb_resources.is_active in resources_visible_to/resources_readable_by, or add AND r.is_active to the three content surfaces.

**Verification note:** Confirmed: _project_resource_deleted only flips kb_resources.is_active=false (leaves homes/blocks/grants intact), so resources_visible_to/resources_readable_by still return the id. cogmap_regulation (JOIN kb_resources r, no r.is_active), resource_blocks, and resource_block_provenance gate solely on resources_readable_by and never check is_active, while graph_atlas_nodes DOES filter r.is_active (line 1416) — so a soft-deleted resource's title/body/provenance is still served by the content surfaces. Real asymmetry/content leak.

### `graph_cogmap_orphan_nodes` — SQLA-1 (low) · `missing-edge-table-filter`

**Unit:** graph-atlas · **File:** `migrations/20260704000005_orphan_anchor_label.sql` (span 42-70) · **Effort:** ?

**Why it matters:** Degree subquery omits the edge-table predicate, defeating the partial source/target indexes and diverging from atlas siblings.

**Suggested fix:** Add AND e.source_table='kb_resources' AND e.target_table='kb_resources' to the degree subquery.

**Verification note:** Confirmed at functions-def.sql:1549-1553 the degree LATERAL filters only (e.source_id=r.id OR e.target_id=r.id) with NO source_table/target_table='kb_resources' predicate; sibling graph_atlas_nodes_visible (1513) includes it. Partial indexes idx_kb_edges_source/target lead with (source_table,source_id) per indexes.txt:39-40, so the missing table predicate blocks index probing on PG17. Valid index-usability + consistency nit; low.

### `graph_traverse` — SQLA-1 (low) · `unbounded-recursion`

**Unit:** graph-atlas · **File:** `migrations/20260624000002_canonical_functions.sql` (span 1308-1340) · **Effort:** ?

**Why it matters:** Recursive CTE trusts p_depth directly; no internal clamp like every sibling.

**Suggested fix:** Clamp with w.depth < LEAST(p_depth, 10).

**Verification note:** Confirmed canonical migration line 1323 bounds only on `w.depth < p_depth` with no LEAST clamp, unlike scoped siblings (LEAST(p_depth,10)). Live caller aggregator_subgraph clamps depth to MAX_DEPTH=10 (graph_service.rs:28,124) and the handler hardcodes depth=2, so impact is defensive-only today; genuine but low internal-DoS gap on the public function.

### `workflow_job_redrive_resource` — SQLA-1 (low) · `missing-status-index`

**Unit:** steward-jobs-teams · **File:** `migrations/20260708000001_workflow_job_redrive.sql` (span 26-31) · **Effort:** S

**Evidence:** SELECT DISTINCT j.resource_id WHERE persona=$1 AND dispatch_type=$2 AND resource_id IS NOT NULL AND status='dead'. indexes.txt kb_workflow_jobs partials cover only status IN pending/waiting_for_retry (claimable) and pending/in_progress/waiting_for_retry (in_flight); none match status='dead'.

**Why it matters:** Terminal 'dead'/'done' jobs accumulate indefinitely; redrive seq-scans the whole table. Infrequent operator recovery path so bounded in practice, but the only queue predicate with no usable index.

**Suggested fix:** Add partial index CREATE INDEX idx_workflow_jobs_dead ON kb_workflow_jobs (persona, dispatch_type, resource_id) WHERE status='dead' AND resource_id IS NOT NULL, mirroring existing partial indexes.

**Verification note:** Confirmed: live EXPLAIN shows Seq Scan on kb_workflow_jobs filtering persona/dispatch_type/status='dead'; no partial index covers status='dead'. Genuine gap; low severity is accurate given the infrequent recovery-path usage.

### `team_viewable_by` — SQLA-2 (low) · `dead-function`

**Unit:** access-rbac · **File:** `migrations/20260703130000_graph_atlas_chunk_b_reads.sql` (span 25) · **Effort:** S

**Why it matters:** Dead code; also embeds the team_descendants soft-delete gap, so reviving it would ship a bug.

**Suggested fix:** Drop team_viewable_by (+ team_child_zones, team_descendants if confirmed unused) or wire into the intended team-graph read surface.

**Verification note:** Confirmed dead: no reference in crates/packages/tests, no SQL-to-SQL caller in functions-def.sql; only occurrences are its own CREATE (this migration) plus design docs (specs/plans). Its helper team_child_zones is likewise uncalled outside its own migration+docs. Real dead code in the access core.

**Resolution (2026-07-08, chunk 9 — deferred to Atlas Beat E):** Deadness re-confirmed (zero Rust/TS/test callers; `team_descendants`' only SQL callers are the other two dead functions). The trio was born in the Atlas R1 migration (`20260703000002_team_graph_scope_reads.sql`) for descendant-zone enumeration, but the shipped Atlas Home evolved past that design (`graph_home_contexts`/`graph_home_cogmaps`; team scoping flows through `resources_in_team_scope`, which is alive with seven atlas SQL callers). The Beat 2a spec still names a `TeamZoneMark`, unimplemented in temper-ui — so a future beat may yet consume zone enumeration. Decision: retain the trio for now; the Graph Atlas goal carries a Beat E review item to either wire them (fixing `team_descendants`' missing is_active gating first — revive-as-is is prohibited) or drop all three.

### `vis_team` — SQLA-2 (low) · `team-visibility-definition-drift`

**Unit:** access-rbac · **File:** `migrations/20260701000003_access_grants_store_migration.sql` (span 141-153) · **Effort:** M

**Why it matters:** 'Team visibility' for the cogmap axis (vis_team) has drifted from resources_in_team_scope; under-show, not a breach.

**Suggested fix:** If cogmaps should inherit container-granted team resources, add the D3a container-grant branch to vis_team to match resources_in_team_scope; otherwise document the deliberate narrowing.

**Verification note:** Confirmed: vis_team has only two ancestor-expanded branches (team-anchored resource read-grants; resources homed in contexts SHARED to the team). It omits the D3a explicit container read-grant branch that BOTH resources_visible_to and resources_in_team_scope honor for teams (and also omits their cogmap-join and team-owned-context branches). So a resource made team-readable via a container read-grant is invisible to a cogmap bound to that team. Genuine definitional drift, but it UNDER-shows (no leak) and the shared-reach narrowing may be intentional — verify intent before changing. Low.

**Resolution (2026-07-08, chunk 7 — deliberate, documented, no code change):** The narrowing is the locked Q-B leak-safety decision of the generalized access-capability arc, not drift. vis_team feeds `resources_accessible_to_cogmap` — the Cogmap principal axis, which is deliberately INTERSECTION/least-privilege (vs the Profile axis's UNION-up). Q-B (design doc `docs/superpowers/specs/2026-06-30-generalized-access-capability-model-design.md` §3.7/§4 step 4) states that explicit grants are Profile-axis only: profile-principal grants and context/cogmap **subjects** (the D3a container grants) never enter the producer intersection. The D5 migration (`20260701000003_access_grants_store_migration.sql`, comment at the vis_team re-emit) marks this filter load-bearing. The comparison baseline was the wrong axis: resources_in_team_scope is a Profile-axis read (atlas team-scope filtering), so it SHOULD honor D3a; vis_team should not. No alignment migration will be shipped.

### `team_descendants` — SQLA-3 (low) · `soft-delete-team-asymmetry`

**Unit:** access-rbac · **File:** `migrations/20260703000002_team_graph_scope_reads.sql` (span 17-24) · **Effort:** S

**Why it matters:** If revived, a soft-deleted sub-team's members would still confer view/enter on the parent — a soft-delete leak.

**Suggested fix:** Join kb_teams and filter is_active at each recursion level and gate the root to mirror team_ancestors, or retire alongside team_viewable_by.

**Verification note:** Confirmed asymmetry: team_descendants' recursive CTE never joins kb_teams/filters is_active, whereas team_ancestors gates the root (`t.is_active`) and every hop (`JOIN kb_teams pt ... AND pt.is_active`). So soft-deleted teams appear as descendants. Real, but latent — both consumers (team_viewable_by, team_child_zones) are dead code, so no reachable leak today; hence low.

### `graph_subgraph_nodes` — SQLA-4 (low) · `unscoped-degree-count`

**Unit:** graph-atlas · **File:** `migrations/20260626000003_graph_subgraph_nodes_by_id.sql` (span 43-45) · **Effort:** ?

**Why it matters:** edge_count counts every non-folded resource-resource edge touching the node with no visibility gate, unlike the honest-degree siblings.

**Suggested fix:** Join edges_visible_to(p_profile) into the edge_count subquery to match graph_atlas_nodes_visible.

**Verification note:** Confirmed at functions-def.sql:1898-1900 the edge_count subquery has NO edges_visible_to gate while siblings graph_atlas_nodes_visible/_cogmap (1509-1515) gate degree through edges_visible_to; leaks a count of edges to invisible resources / privately-anchored edges (rendered edges in fetch_subgraph_edges ARE gated, so the count is dishonest). Real but count-only disclosure, so severity is low rather than medium.

## Work breakdown (PR-sized chunks)

All fixes ship as **new additive migrations** (CREATE OR REPLACE FUNCTION / CREATE INDEX — never edits to old migrations), portable across PG17 (Neon prod) and PG18 (dev/CI), with the sqlx-prepare ritual where function bodies change.

### 1. Gate graph_subgraph_nodes edge_count through edges_visible_to (the audit's one RBAC finding)

**Priority:** high · **Effort:** S · **Rules:** SQLA-4 · **Functions:** `graph_subgraph_nodes`

The only confirmed visibility finding: edge_count counts every non-folded resource-resource edge touching a node with no visibility gate, leaking the existence-count of edges to resources the profile cannot see, while sibling atlas functions compute honest degree. Small, surgical, and it closes the RBAC dimension of the audit — ship it first and alone so the fix is independently reviewable as the security beat.

**PR scope:** One new additive migration with CREATE OR REPLACE FUNCTION graph_subgraph_nodes, joining edges_visible_to(p_profile) into the edge_count subquery to mirror graph_atlas_nodes_visible (functions-def.sql:1509-1515). Function-body-only change (no Rust query text changes) but run the sqlx ritual per CLAUDE.md anyway; add/extend a test-db integration test asserting subgraph degree matches gated edge fetch for a profile with partial visibility. Portable PG17/18.

### 2. Close the soft-delete content leak: is_active enforcement in the readable/visible path

**Priority:** high · **Effort:** M · **Rules:** SQLA-3 · **Functions:** `resources_visible_to`, `resources_readable_by`, `cogmap_regulation`, `resource_blocks`, `resource_block_provenance`

_project_resource_deleted only flips kb_resources.is_active, so resources_readable_by still returns deleted ids and three content surfaces (cogmap_regulation, resource_blocks, resource_block_provenance) serve a soft-deleted resource's title/body/provenance while graph_atlas_nodes correctly filters it. Deleted content remaining readable is the worst user-visible asymmetry in the audit. Prefer centralizing the is_active filter inside resources_visible_to/resources_readable_by (one fix covers all current and future consumers) over patching three surfaces; audit consumers first to confirm none legitimately needs deleted rows.

**PR scope:** One additive migration CREATE OR REPLACE on the chosen layer (visibility fns preferred; else the three content surfaces). Needs subsystem context: enumerate every caller of resources_visible_to/resources_readable_by and verify none depends on seeing inactive rows (e.g. redrive/admin paths) before centralizing. Add an e2e/test-db case: delete resource, assert blocks/provenance/regulation no longer return it. Run sqlx prepare ritual.

### 3. Fix _rebuild_resource_search_vector folded-block leak (+ one-shot search_vector re-rebuild)

**Priority:** high · **Effort:** S · **Rules:** SQLA-3 · **Functions:** `_rebuild_resource_search_vector`

The FTS body aggregation gates only c.is_current, missing the NOT b.is_folded join every sibling gate applies (_recompute_resource_body_hash, all vector/read gates). Charter re-set folds old blocks but their chunks survive as is_current, so superseded charter prose stays FTS-matchable and search_vector diverges from body_hash. Mechanical one-function fix with a known-correct template to copy.

**PR scope:** One additive migration: CREATE OR REPLACE mirroring _recompute_resource_body_hash's join (kb_content_blocks b ON b.id=c.block_id AND NOT b.is_folded), plus a backfill UPDATE in the same migration invoking the rebuild for resources with folded blocks so existing stale vectors are corrected (data fix rides the same PR — it is this bug's story). Test: charter set, supersede, assert FTS no longer matches the superseded prose. Portable PG17/18; sqlx ritual.

### 4. Index pack: five missing indexes on append-only/anchor tables

**Priority:** high · **Effort:** M · **Rules:** SQLA-1 · **Functions:** `resources_visible_to`, `element_trail_edge`, `element_trail_node`, `steward_ingest_delta`, `workflow_job_redrive_resource`

Five confirmed index gaps with purely mechanical fixes and zero function-body changes, grouped by pattern into one CREATE INDEX-only migration: kb_resource_homes(owner_profile_id) + (originator_profile_id) for the hottest access fn; kb_events expression indexes on ((payload->>'edge_id')::uuid), ((payload->>'resource_id')::uuid), ((payload->'owner'->>'id')::uuid) for the element trails; kb_events(producing_anchor_table, producing_anchor_id, id) for steward_ingest_delta's per-cogmap LATERAL; partial kb_workflow_jobs(persona, dispatch_type, resource_id) WHERE status='dead'. Two of these (steward + trails) are live EXPLAIN-confirmed seq scans of the unbounded event log.

**PR scope:** One additive CREATE INDEX-only migration — no CREATE OR REPLACE, no .sqlx cache impact (query shapes unchanged), trivially additive-only-on-main. Verify each with before/after EXPLAIN against dev DB; keep expressions IMMUTABLE-safe for PG17 (uuid cast of ->> is fine). Note in the migration comment that the element_trail expression indexes are the tactical fix; migrating trails to the indexed references column is a possible future refactor, deliberately out of scope here. Neon prod: plain CREATE INDEX per additive-migration runbook.

### 5. cogmap_staleness: decide fold-touch semantics, then index or predicate fix

**Priority:** medium · **Effort:** S · **Rules:** SQLA-1 · **Functions:** `cogmap_staleness`

The edges leg seq-scans kb_edges (largest-growing table, invoked per cogmap_analytics read) because the folded-inclusive predicate cannot use the partial idx_kb_edges_home (WHERE NOT is_folded); live EXPLAIN confirms adding NOT e.is_folded flips to an index scan. Kept separate from the index pack because it requires a product decision first: should a fold count as a staleness 'touch'? If yes, add a non-partial (home_anchor_table, home_anchor_id) INCLUDE (last_event_id) index; if no, add NOT is_folded to the edges leg (and NOT reg.is_folded to regions) and ride the existing partial index.

**PR scope:** One additive migration once the semantics call is made (either CREATE INDEX only, or CREATE OR REPLACE cogmap_staleness). Record the decision in the migration comment. EXPLAIN before/after; if the predicate route, verify cogmap_analytics output on a map with folded edges matches intent. Sqlx ritual if function replaced.

### 6. Set-based rewrite of edges_visible_to (kill the hidden N+1 into graph_atlas)

**Priority:** medium · **Effort:** M · **Rules:** SQLA-1 · **Functions:** `edges_visible_to`

endpoint_readable_by_profile runs the full resources_visible_to subselect per surviving edge, for BOTH endpoints — cost is edges x cost(resources_visible_to), feeding graph_atlas degree laterals. Fine on dev data, quadratic-degrading with graph size. Fix: materialize resources_visible_to(p_profile) once in a CTE and semi-join source, target, and anchor set-based. Needs care (semantics of anchor-readability and non-resource endpoints must be preserved exactly), hence its own PR rather than bundling with mechanical fixes.

**PR scope:** One additive CREATE OR REPLACE migration. Regression guard: assert result-set equality old-vs-new across the e2e visibility fixtures (partial-visibility profiles, team-anchored and cogmap-anchored edges, non-kb_resources endpoints) before/after; EXPLAIN to confirm single visible-set materialization. Same signature, so callers (graph_atlas_*, fetch_subgraph_edges) are untouched. Sqlx ritual.

### 7. Resolve vis_team definitional drift vs resources_in_team_scope (decision-first)

**Priority:** medium · **Effort:** M · **Rules:** SQLA-2 · **Functions:** `vis_team`

vis_team (the cogmap axis's notion of team visibility) omits the D3a explicit container read-grant branch that both resources_visible_to and resources_in_team_scope honor, so a resource made team-readable via container grant is invisible to a cogmap bound to that team. Under-show only — no breach — and the narrowing may be intentional, so this PR starts with an intent decision against the D1-D5 access-arc design, then either adds the container-grant branch to match resources_in_team_scope or documents the deliberate narrowing in the function comment and access docs.

**Disposition:** Resolved decision-first as DELIBERATE (locked Q-B leak-safety: explicit container grants are Profile-axis only and never enter the cogmap producer intersection). Documented in the finding's resolution note above; no migration. See D5 migration comments + design doc §3.7.

**PR scope:** If aligning: one additive CREATE OR REPLACE migration adding the container-grant branch, plus a test-db case (container read-grant to team → resource visible via team-bound cogmap scope). If documenting: comment-bearing CREATE OR REPLACE (or docs-only if no SQL change). Either way single-function scope; check the closed generalized-access-capability arc notes for original intent before choosing. Sqlx ritual if replaced.

### 8. Graph hardening pair: clamp graph_traverse depth + fix orphan-nodes degree predicate

**Priority:** low · **Effort:** S · **Rules:** SQLA-1 · **Functions:** `graph_traverse`, `graph_cogmap_orphan_nodes`

Two low-severity consistency-with-siblings fixes in graph-atlas, bundled as one align-with-the-pattern PR: graph_traverse trusts p_depth with no LEAST(p_depth,10) clamp (defensive only — the live caller clamps at MAX_DEPTH=10 — but every scoped sibling clamps internally); graph_cogmap_orphan_nodes' degree lateral omits the source_table/target_table='kb_resources' predicate, defeating the partial edge indexes on PG17 and diverging from graph_atlas_nodes_visible.

**PR scope:** One additive migration with two CREATE OR REPLACE statements — both are one-line predicate changes copying the exact sibling pattern (LEAST clamp from the scoped traverse; table predicate from graph_atlas_nodes_visible line 1513). No behavior change for current callers; EXPLAIN on orphan degree to confirm index probe on PG17-equivalent plans. Sqlx ritual.

### 9. Retire dead team-graph read helpers: team_viewable_by, team_child_zones, team_descendants

**Priority:** low · **Effort:** S · **Rules:** SQLA-2, SQLA-3 · **Functions:** `team_viewable_by`, `team_child_zones`, `team_descendants`

team_viewable_by and helper team_child_zones have zero callers in crates/packages/tests or SQL — confirmed dead — and team_viewable_by embeds team_descendants' missing is_active gating, so reviving it as-is would ship a soft-delete leak (deleted sub-team members conferring view/enter on the parent). Per the no-premature-backward-compat convention, DROP all three in one migration; if a team-graph read surface is genuinely planned, instead fix team_descendants to mirror team_ancestors' is_active gates (root + every hop) and wire the surface — but decide, don't leave armed dead code in the access core.

**PR scope:** One additive migration with DROP FUNCTION IF EXISTS for the trio (drop is additive-safe here: zero callers verified; re-grep crates/, packages/, tests/, and functions-def.sql for references immediately before shipping). If the wire-in path is chosen instead, that becomes a feature PR with the is_active fix included — do not ship the fix without a caller. No .sqlx impact for the drop path.

**Disposition:** Deferred — drop-or-wire moved to the Graph Atlas goal as a Beat E review item (the trio originated in Atlas R1 and a future zone-rendering beat may consume it; see the team_viewable_by resolution note above). No migration this chunk; the is_active caveat travels with the review item.

## Clean functions by unit

**access-rbac** (19): `anchor_readable_by_profile`, `can`, `cogmap_authorable_by_profile`, `cogmap_readable_by_profile`, `cogmap_scope_ids`, `cogmap_visible_maps`, `cogmaps_share_a_team`, `context_authorable_by_profile`, `context_visible_to`, `derived_access_profile`, `endpoint_readable_by_profile`, `has_system_access`, `is_system_admin`, `profile_effective_teams`, `profile_explicit_grant`, `resources_accessible_to_cogmap`, `resources_in_cogmap_scope`, `resources_in_team_scope`, `team_ancestors`

**event-projections** (27): `_event_append`, `_insert_block_provenance`, `_insert_chunk`, `_project_block_mutated`, `_project_blocks`, `_project_charter_set`, `_project_cogmap_seeded`, `_project_delegated_launch`, `_project_invocation_closed`, `_project_lens_created`, `_project_property_asserted`, `_project_property_set`, `_project_region_materialized`, `_project_relationship_asserted`, `_project_relationship_folded`, `_project_relationship_retyped`, `_project_relationship_reweighted`, `_project_resource_created`, `_project_resource_deleted`, `_project_resource_reassigned`, `_project_resource_rehomed`, `_project_resource_updated`, `_recompute_resource_body_hash`, `_upsert_remote_source`, `kb_events_append_only`, `normalize_remote_uri`, `uuid_generate_v7`

**mutation-commands** (19): `block_body_text`, `block_mutate`, `cogmap_charter_set`, `cogmap_genesis`, `invocation_close`, `invocation_open`, `lens_create`, `property_set`, `region_materialize`, `relationship_assert`, `relationship_fold`, `relationship_retype`, `relationship_reweight`, `resource_body_text`, `resource_create`, `resource_delete`, `resource_reassign`, `resource_rehome`, `resource_update`

**graph-atlas** (7): `graph_atlas_nodes_cogmap`, `graph_atlas_nodes_visible`, `graph_cogmap_territories`, `graph_home_cogmaps`, `graph_home_contexts`, `graph_region_composition_edges`, `graph_traverse_cogmap_scoped`

**search-wayfind** (4): `search_fts_candidates`, `search_graph_expand`, `unified_search`, `wayfind_scope_ids`

**cogmap-analytics** (12): `cogmap_analytics`, `cogmap_region_centrality`, `cogmap_region_content_cohesion`, `cogmap_region_internal_tension`, `cogmap_region_metrics`, `cogmap_region_reference_standing`, `cogmap_region_telos_alignment`, `cogmap_regulation`, `cogmap_shape`, `cogmap_telos`, `resource_block_provenance`, `resource_blocks`

**steward-jobs-teams** (13): `backfill_auto_join_team`, `ensure_auto_join_memberships`, `steward_candidate_cogmaps`, `steward_drift_sweep`, `sync_personal_team`, `sync_system_membership`, `workflow_job_claim`, `workflow_job_claim_resource`, `workflow_job_complete`, `workflow_job_complete_resource`, `workflow_job_enqueue`, `workflow_job_enqueue_resource`, `workflow_job_reap`

---

*Generated by the `sql-function-audit` workflow (adapted from `code-quality-audit`): 7 audit + 7 verify + 1 aggregate agents; findings adversarially verified against live definitions, `pg_indexes`, and Rust/TS call sites before inclusion.*
