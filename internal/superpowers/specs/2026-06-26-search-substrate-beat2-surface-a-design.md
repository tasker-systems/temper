# Search Substrate — Beat 2: Surface A, general search done right (design)

**Date:** 2026-06-26
**Arc:** Search followup — leverage the substrate (graph-nearness + cogmap-region salience)
**Beat:** 2 of 3 (Surface A). See the goal roadmap; builds directly on Beat 1's stored tsvector
(`docs/superpowers/specs/2026-06-26-search-substrate-beat1-stored-tsvector-design.md`).
**Mode:** build · **Effort:** large

---

## 1. Problem

Beat 1 fixed the FTS *index*; the *ranking model* is still the weakest expression of a rich substrate.
`POST /api/search` today is:

- **Either/or, never blended.** `search_select` runs `vector_search` when an embedding is supplied,
  *else* `fts_search` over the text query — never both, never combined
  (`crates/temper-api/src/backend/substrate_read.rs:290-306`).
- **Unranked.** Every row is emitted with `fts_score: 0.0, vector_score: 0.0, combined_score: 0.0`
  (`substrate_read.rs:319-321`); final order is whatever the readback `ORDER BY` produced.
- **Index-defeating on the vector side.** `vector_search` (`crates/temper-substrate/src/readback/mod.rs:866`)
  is `… JOIN kb_chunks … GROUP BY r.id ORDER BY MIN(c.embedding <=> $2::vector)`. The `GROUP BY/MIN`
  over a join forces a full per-chunk distance scan — `idx_kb_chunks_embedding` (the partial HNSW,
  `schema:579`) **cannot** engage.
- **Graph-blind.** `SearchParams` already declares `seed_ids` / `edge_types` / `graph_depth` /
  `graph_expand` (`crates/temper-core/src/types/api.rs:59-70`), and `kb_edges` is a weighted graph
  (`weight DOUBLE PRECISION DEFAULT 1.0`, `schema:637`) — but `search_select` ignores all of it.

This Beat makes `/api/search` **blend and rank** the substrate's three signals — lexical (Beat 1's GIN
tsvector), semantic (the HNSW chunk index, *used* this time), and structural (weighted graph
traversal) — into one ordered result. It is **Surface A only**: the general corpus visible via
`resources_visible_to`, **no cogmaps, no regions** (that is Beat 3 / Surface B).

## 2. Current-state ground truth (verified against the live tree, 2026-06-26)

- **Live path:** `POST /api/search` → `crates/temper-api/src/handlers/search.rs` →
  `substrate_read::search_select` (`substrate_read.rs:284`) → `temper_substrate::readback::fts_search`
  (`mod.rs:809`) **xor** `readback::vector_search` (`mod.rs:866`). Each readback returns a bare
  `Vec<Uuid>`; `search_select` reconstructs each to a full row via `native_resource_row` and emits
  zero scores.
- **FTS (post-Beat-1)** reads the stored `kb_resource_search_index` + `resources_visible_to($1)`,
  `@@ plainto_tsquery('english', $2)`, `ORDER BY ts_rank(...) DESC`, returns ids only — **discards the
  rank** (`mod.rs:809-830`).
- **Vector** is the HNSW-defeating `GROUP BY r.id / MIN(<=>)` shape, returns ids only (`mod.rs:866-887`).
- **Graph** (`readback::neighbors`, `mod.rs:925`) is **1-hop only, deliberately UNSCOPED** (no
  principal — a flagged access gap, its own doc-comment notes the leak-safe gate
  `edges_visible_to(principal)` is unbuilt), and has **no surface caller** — only a parity test reads it.
- **`search_select` ignores** `SearchParams.context_name`, `doc_type`, `seed_ids`, `edge_types`,
  `graph_depth`, `graph_expand` — it passes only `principal` + (`embedding` xor `query`) to the readbacks.
- **Substrate already present:** `kb_chunks.embedding vector(768)` + partial HNSW
  `idx_kb_chunks_embedding USING hnsw (embedding vector_cosine_ops) WHERE is_current` (`schema:579`);
  `kb_edges` weighted graph with `idx_kb_edges_source` / `idx_kb_edges_target` partial on `NOT is_folded`
  (`schema:648-649`); `kb_resource_search_index` GIN (Beat 1).
- **`UnifiedSearchResultRow`** (`api.rs:119`) already carries `fts_score` / `vector_score` /
  `combined_score` (all `f32`) and an `origin` tag — the score plumbing exists, it's just fed zeros.

## 3. Design

### 3.1 Shape — composed SQL, one aggregate statement

The entire candidate-generation + fusion runs **in Postgres**, as one aggregate statement assembled
from **composable SQL functions used as CTE bases**. Rationale: avoid round-trip chatter in an
already non-trivial mechanism; let Postgres plan and optimize the whole shape once; surface indexing
pressure on the real aggregate query (not hidden between many small Rust↔SQL calls); and keep each
signal a standalone, separately-testable, separately-optimizable SQL unit.

```
search_select (Rust): build params → ONE unified_search readback → map scored rows

unified_search aggregate query (one statement):
  WITH
    fts   AS (SELECT * FROM search_fts_candidates($princ, $query)),         -- (id, fts_norm)
    vec   AS (SELECT * FROM search_vector_candidates($princ, $emb, $k)),    -- (id, vec_norm)
    blend0 AS (SELECT id, w_fts·fts_norm + w_vec·vec_norm AS s0
                 FROM fts FULL OUTER JOIN vec USING (id)),                  -- pre-graph blend
    seeds AS (SELECT unnest($seed_ids)                                      -- explicit seeds
              UNION
              SELECT id FROM blend0 ORDER BY s0 DESC LIMIT $N),             -- auto-seeds (self-seed)
    graph AS (SELECT * FROM search_graph_expand($princ,                     -- (id, graph_score)
                 ARRAY(SELECT id FROM seeds), $depth, $edge_types)),
    cand  AS (fts FULL OUTER JOIN vec FULL OUTER JOIN graph USING (id)),    -- recall = ∪ of all three
    scored AS (SELECT id,
                      COALESCE(fts_norm,0)   AS fts_score,
                      COALESCE(vec_norm,0)   AS vector_score,
                      COALESCE(graph_score,0) AS graph_score,
                      w_fts·COALESCE(fts_norm,0)
                    + w_vec·COALESCE(vec_norm,0)
                    + w_graph·COALESCE(graph_score,0) AS combined
                 FROM cand)
  SELECT * FROM scored ORDER BY combined DESC LIMIT $limit OFFSET $offset
```

The self-seed dependency (graph traversal needs the text/vector blend's top-N) lives **inside the
statement**: the `seeds` CTE reads `blend0` and hands an `id[]` array to `search_graph_expand`. No Rust
round-trip. When `graph_expand = false`, the `seeds` + `graph` CTEs are skipped and `cand` is just
`fts FULL OUTER JOIN vec` (the graph term is `0`).

**Missing signals → term-zero, not branch.** No embedding ⇒ `vec` is empty ⇒ `vector_score = 0` for
every candidate; no query text ⇒ `fts` empty ⇒ `fts_score = 0`. The `FULL OUTER JOIN` + `COALESCE(…,0)`
dissolves today's either/or into a single blend that degrades gracefully to whichever signals are
present.

### 3.2 The three candidate functions

Each is a standalone SQL function (so it is unit-testable and the planner can be reasoned about per
signal). Names/signatures are the spec contract; exact PL/pgSQL vs SQL body is an implementation choice
(recursive traversal forces PL/pgSQL or a `LANGUAGE sql` with `WITH RECURSIVE`).

#### `search_fts_candidates(p_principal uuid, p_query text) → TABLE(resource_id uuid, fts_norm real)`

Reads Beat 1's stored vector. Exercises the **GIN** index (`idx_resource_search_vector`).

```sql
SELECT r.id,
       (ts_rank(si.search_vector, plainto_tsquery('english', p_query), 32))::real AS fts_norm
  FROM kb_resource_search_index si
  JOIN kb_resources r             ON r.id = si.resource_id
  JOIN resources_visible_to(p_principal) v ON v.resource_id = r.id
 WHERE r.is_active
   AND si.search_vector @@ plainto_tsquery('english', p_query)
```

- **`ts_rank(..., 32)`** — normalization flag `32` = `rank / (rank + 1)`, a **fixed, batch-independent**
  transform into `[0, 1)`. This is the key choice that makes weighted-sum stable: a document's
  `fts_norm` does not depend on what else matched the query (no min-max-over-candidate-set), so scores
  are comparable across queries and stable as the corpus grows.
- `'english'` is hardcoded to match Beat 1's storage recipe; the multilingual `search_config` column
  stays storage-only (Beat 1 §6) until a real need.
- Returns empty (zero rows) when `p_query IS NULL` — the aggregate's `COALESCE` then zeroes the term.

#### `search_vector_candidates(p_principal uuid, p_emb vector, p_k int) → TABLE(resource_id uuid, vec_norm real)`

The HNSW fix. **Over-fetch top-`p_k` chunks via a pure ANN order, *then* filter + dedup.**

```sql
WITH ann AS (                                  -- pure HNSW: no other predicate in this CTE
    SELECT c.resource_id, (c.embedding <=> p_emb) AS dist
      FROM kb_chunks c
     WHERE c.is_current                        -- matches the partial-index predicate
     ORDER BY c.embedding <=> p_emb            -- engages idx_kb_chunks_embedding
     LIMIT p_k
)
SELECT a.resource_id,
       (1.0 - MIN(a.dist) / 2.0)::real AS vec_norm   -- cosine_dist∈[0,2] → vec_norm∈[0,1]
  FROM ann a
  JOIN kb_resources r             ON r.id = a.resource_id AND r.is_active
  JOIN resources_visible_to(p_principal) v ON v.resource_id = a.resource_id
 GROUP BY a.resource_id
```

- The inner `ann` CTE is *only* `ORDER BY <=> LIMIT k` on `WHERE is_current` — exactly the partial HNSW
  index's predicate, nothing else — so the planner uses the index. Applying `resources_visible_to` /
  `is_active` *inside* that CTE would turn the ANN scan into a filtered seq-scan and defeat the index
  (the same class of mistake today's `GROUP BY/MIN` makes).
- **Over-fetch** (`p_k` default `100`, » the API `limit` of ≤50) absorbs the post-ANN attrition: chunks
  belonging to non-visible / inactive resources are dropped *after* the ANN, so we fetch generously to
  ensure enough visible resources survive. `MIN(dist)` per resource = best-chunk-decides-rank (the
  intent today's query had, now index-using).
- `vec_norm = 1 − dist/2` maps cosine distance `[0,2]` → `[0,1]` (identical ⇒ `1.0`, opposite ⇒ `0.0`).
- Returns empty when `p_emb IS NULL`.

> **Visibility-vs-HNSW caveat (documented, not silently accepted):** over-fetch is a heuristic, not a
> guarantee — a principal who can see only a sliver of a huge corpus could in theory have all `p_k`
> nearest chunks be invisible. `p_k=100` is comfortably sufficient for Temper's per-principal corpus
> sizes today; if multi-tenant scale ever makes this bite, the fix is a visibility-aware ANN
> (pre-filtered index or iterative widening), tracked as a future concern, not built now (YAGNI).

#### `search_graph_expand(p_principal uuid, p_seeds uuid[], p_depth int, p_edge_types text[]) → TABLE(resource_id uuid, graph_score real)`

Scoped, weighted, multi-hop recall-expansion. `WITH RECURSIVE` over `kb_edges`.

- **Surface A scope (non-negotiable):** `source_table = 'kb_resources' AND target_table = 'kb_resources'`
  (no cogmap endpoints — Surface A excludes cogmaps by construction), `NOT is_folded`, and **every
  traversed endpoint joined through `resources_visible_to(p_principal)`** so a non-visible neighbor can
  never leak into results. This closes the access gap the legacy unscoped `neighbors()` left open.
- **Traversal is symmetric** (an edge connects its endpoints regardless of direction): from a frontier
  node, follow edges where it is `source_id` (→ `target`) or `target_id` (→ `source`).
- **`p_edge_types`** filters `edge_kind::text = ANY(p_edge_types)` when non-empty; empty/NULL = all kinds.
- **Score = MAX-over-paths** `γ^hop × Π edge_weight(path)`, `γ = 0.5`. Seeds themselves are hop 0,
  `graph_score = 1.0`. A node reached by several paths keeps its single **best** path's score
  (hub-robust: a doc near one strong seed is not out-competed by a hub wired to many weak seeds). In the
  recursive CTE this is the running `path_score = parent_path_score × γ × edge_weight`, with a final
  `MAX(path_score) GROUP BY resource_id`.
- **`p_depth`** clamped server-side (§3.4); a cycle guard tracks visited ids along each path
  (`NOT id = ANY(path)` array accumulator) to terminate.

### 3.3 Fusion (in the aggregate `scored` CTE)

```
combined(d) = w_fts·fts_norm(d) + w_vec·vec_norm(d) + w_graph·graph_score(d)
```

All three sub-scores are `[0,1]`-bounded by construction, so the weights are directly interpretable.
**Weights are server-side named constants in ONE place** — NOT API parameters. (Per-query weight tuning
is YAGNI: a caller who genuinely needs to retune the blend, and knows what they're doing, can fork or
contribute a patch. Exposing the knob invites mis-tuning far more than it serves a real need.) The
constants live alongside the function/migration as a single tunable surface.

### 3.4 Defaults (tuning-provisional — to be calibrated on the real corpus)

| Knob | Default | Hard cap | Rationale |
|------|---------|----------|-----------|
| `w_fts` | `1.0` | — | lexical precision is the trusted baseline |
| `w_vec` | `1.0` | — | semantic recall co-equal with lexical |
| `w_graph` | `0.5` | — | structural proximity *amplifies*, never *dominates* |
| `γ` (hop decay) | `0.5` | — | a 2-hop neighbor is worth ¼ of adjacency |
| `p_depth` (graph) | `2` | `3` | edge fan-out is exponential; Surface A amplifies the core, it is not a deep walk |
| `p_k` (vector over-fetch) | `100` | — | absorbs post-ANN visibility/active attrition |
| `N` (auto-seeds) | `20` | — | top of the pre-graph text/vector blend |
| `limit` / `offset` | `10` / `0` | `50` | already documented on `SearchParams` |

`graph_depth` from the API is clamped to `[1, 3]`; the `SearchParams` doc-comment's "max 10" is reduced
to a hard cap of 3 for Surface A (deep traversal is a Surface-B / cogmap concern, and a 10-hop recursive
fan-out would threaten Neon).

### 3.5 Rust wiring

- **`SearchParams` fields go live** (`api.rs:40-71`): `seed_ids` (union with auto-seeds), `edge_types`
  (traversal filter), `graph_depth` (clamped), `graph_expand` (`false` ⇒ skip seeds+graph CTEs),
  `context_name` + `doc_type` (see below). `embedding`/`query` presence drives term-zeroing as in §3.1.
- **`context_name` + `doc_type` filters wired in the same pass.** Currently ignored by `search_select`;
  they become candidate-corpus predicates (resolved to ids server-side, applied in the base functions'
  visibility join, or as an outer filter on the aggregate). In-scope here — it is table-stakes for
  "general search done right," not a separate beat.
- **Scores exposed.** `UnifiedSearchResultRow.fts_score` / `vector_score` / `combined_score` stop being
  `0.0` and carry the real sub-scores. **Add `graph_score: real`** to the struct (regenerates the ts-rs
  `search.ts` type): the graph term must be observable to be tunable and debuggable. `origin` becomes
  `"unified"` (or is derived from which signals fired) rather than the either/or `"fts"`/`"vector"` tag.
- **`search_select` collapses** (`substrate_read.rs:284`) to: build the params struct → one
  `readback::unified_search(pool, principal, &params)` call returning scored rows → map to
  `UnifiedSearchResultRow` (still reconstructing display fields via `native_resource_row`, or folding the
  needed display columns into the aggregate's final SELECT to save the per-row reconstruction — an
  optimization the plan can weigh).
- **Readback `unified_search`** is a new runtime, schema-qualified `sqlx::query_as` (the established
  pgvector-`::vector`-cast exception — the `query!` macros can't bind `::vector`; the module already
  uses runtime queries for `vector_search`/`fts_search` for exactly this reason). The old single-signal
  `fts_search` / `vector_search` readbacks are **retained** (they are the §9 substrate parity floor that
  the artifact-tests assert against) but `search_select` no longer calls them. The unscoped 1-hop
  `neighbors()` is **superseded** by `search_graph_expand` (and may be retired once no test references
  it — the plan confirms).

### 3.6 Migration

One additive migration (additive-only-on-`main` compliant — new functions + a struct field, no
destructive DDL): the three candidate functions + `search_graph_expand` + the weight constants surface.
No new tables (the substrate — GIN, HNSW, `kb_edges` — already exists). The Rust read swap ships in the
same change. `CREATE OR REPLACE FUNCTION` for any helper that already exists; we never edit a shipped
migration in place (Beat 1 §4.2 rule).

## 4. Decisions

1. **Weighted-sum fusion, not RRF.** Graph proximity maps naturally onto an **additive boost**, not an
   artificial "ranked list" RRF would force; magnitude is preserved (a strong-on-all-signals doc beats a
   rank-1-on-each-but-weak doc). The usual knock on weighted-sum — normalizing an unbounded `ts_rank` —
   is removed by the `ts_rank(…, 32)` flag's fixed `[0,1)` transform.
2. **All-SQL, decomposed into CTE-base functions.** No Rust↔SQL round-trip chatter; Postgres plans the
   whole shape once; indexing pressure surfaces on the real aggregate; each signal stays a standalone
   testable/optimizable unit.
3. **Self-seed from the text/vector blend.** Graph contributes on **every** query (amplifies the
   lexical/semantic core with structurally-near docs), giving `graph_expand: true` real teeth; explicit
   `seed_ids` augment.
4. **Max-over-paths γ^hop·Πweight, hub-robust.** A doc near one strong seed is not out-competed by a hub
   wired to many weak seeds.
5. **HNSW over-fetch-then-filter.** The ANN CTE carries only the index's own predicate; visibility/active
   filtering happens after, with generous `p_k` to cover attrition — the established way to keep an ANN
   index engaged under row-level scoping.
6. **Recall expansion, not boost-only.** `cand = fts ∪ vec ∪ graph` — a graph neighbor absent from the
   text/vector hits still enters the result with its graph-only score (the goal's "recall-EXPANSION +
   boost").
7. **Scope + cogmap exclusion are correctness invariants, not knobs.** Graph traversal is
   `resources_visible_to`-scoped and `kb_resources`-only; this is Surface A by definition and closes the
   legacy unscoped-`neighbors()` access gap.

## 5. Test plan

Substrate `artifact-tests` (`#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]`, ephemeral
`public`-schema DBs) — the surface Beat 1 used. Plus Rust unit coverage for the param-clamping logic.

**Per-function:**
- `search_fts_candidates` — returns `[0,1)` normalized ranks; respects visibility (a non-visible
  matching resource is absent); empty query ⇒ zero rows.
- `search_vector_candidates` — **`EXPLAIN` asserts `idx_kb_chunks_embedding` is used** (the regression
  guard against sliding back to a seq-scan blend); dedups to best-distance-per-resource; over-fetch
  survives a visibility filter that drops near chunks; `vec_norm` arithmetic (`dist=0 ⇒ 1.0`).
- `search_graph_expand` — hop decay (`γ^hop`), edge-weight product, **MAX-over-paths** (a node on two
  paths keeps the better score), `edge_types` filter, `NOT is_folded` exclusion, depth cap, cycle
  termination, **and visibility scoping** (a non-visible neighbor never leaks); seeds = hop 0 score 1.0.

**Blend / aggregate:**
- Missing-signal term-zeroing: text-only (vec=0), vector-only (fts=0), both present, neither (empty).
- `graph_expand = false` ⇒ result equals pure `fts ∪ vec` blend (seeds/graph CTEs skipped).
- Self-seeding: a doc structurally adjacent to a top text/vector hit ranks **above** an equal-text/vector
  doc with no graph connection.
- Explicit `seed_ids` augment the auto-seeds (a hand-passed anchor pulls its neighbors in).
- `context_name` / `doc_type` filters restrict the candidate corpus.
- A tolerant ranking-quality assertion: a known query over the seeded corpus yields the expected top-k
  set/order under default weights (kept tolerant — weights are provisional).

**sqlx cache:** new functions / changed macro queries → regenerate the workspace `.sqlx`
(`cargo sqlx prepare --workspace -- --all-features`) plus any per-crate test-target caches
(`cargo make prepare-*`). Run `cargo make test-artifacts` (the Embed CI feature set) locally before
pushing.

## 6. Out of scope

### Rejected (load-bearing — resist scope creep back in)
- **Weights as API parameters** (§3.3). Per-query weight tuning is YAGNI and an invitation to mis-tune;
  weights are server-side constants. A real retuning need = a fork or a patch.
- **Cogmaps / regions in Surface A.** Region-salience scoping is Surface B (Beat 3); Surface A's graph
  traversal is `kb_resources`-only by construction.
- **RRF / learning-to-rank / weight auto-tuning.** We chose interpretable weighted-sum with hand-set,
  hand-tunable constants. Adaptive ranking is a much later concern.
- **Snippet / highlight generation.** That is a presentation concern, not ranking;
  `UnifiedSearchResultRow` stays snippet-less this beat (the field isn't even on the struct).
- **A visibility-aware ANN index.** Over-fetch is sufficient at Temper's per-principal corpus sizes;
  pre-filtered/iterative-widening ANN is a scale concern tracked, not built (§3.2 caveat).

### Deferred (in scope for a later Beat)
- **Surface B — cogmap wayfinding** (Beat 3): region-salience-first scoping over cogmap-homed
  participants. Specced in this arc; built in the WS7 cognitive-map agent-invocation arc.
- **Per-resource multilingual `search_config`** (Beat 1 left it storage-only).
- **Retiring the §9 single-signal parity-floor readbacks** (`fts_search`/`vector_search`/`neighbors`)
  once nothing references them — kept here because the substrate parity tests assert against them.

## 7. Open questions (resolve during implementation)

- **Display-column reconstruction vs fold-into-aggregate.** Keep `native_resource_row` per-result-row
  reconstruction (simple, proven) or pull title/uri/context/doc_type into the aggregate's final SELECT
  (one query, no N+1)? Measure; the plan decides. Leaning fold-in for the N+1 win, but only if it doesn't
  bloat the aggregate.
- **`origin` semantics.** Derive `"fts"`/`"vector"`/`"graph"`/`"unified"` from which signals fired for a
  given row, or just emit `"unified"` always? Minor; pick the more useful-for-debugging option when
  wiring.
- **Exact `p_k` / `N` / `γ` calibration** — provisional defaults here; calibrate against the real corpus
  during/after implementation (the goal's standing tuning task).
