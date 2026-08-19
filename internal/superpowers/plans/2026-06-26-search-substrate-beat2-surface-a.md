# Search Beat 2 — Surface A (general search done right) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make `POST /api/search` blend and rank the substrate's three signals (lexical FTS, semantic vector, structural graph) into one ordered, scored result — replacing today's either/or, zero-score, HNSW-defeating path.

**Architecture:** All candidate-generation + fusion runs in Postgres as one aggregate SQL function (`unified_search`) composed from three standalone, separately-testable SQL functions used as CTE bases (`search_fts_candidates`, `search_vector_candidates`, `search_graph_expand`). The Rust `search_select` collapses to one readback call + per-row display enrichment. Weighted-sum fusion with fixed `[0,1]`-bounded sub-scores; graph expansion is self-seeded from the text/vector blend and scoped through `resources_visible_to`.

**Tech Stack:** Rust (sqlx runtime queries — the `::vector` cast forbids the compile-time macros), PostgreSQL 17/18 + pgvector (HNSW) + GIN tsvector, temper-substrate `artifact-tests` (`#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]` ephemeral `public`-schema DBs), ts-rs for the TS type.

Design spec: `docs/superpowers/specs/2026-06-26-search-substrate-beat2-surface-a-design.md`.

## Global Constraints

- **Additive-only-on-`main`.** New SQL functions + one struct field only — no destructive DDL, no edits to shipped migrations. The Beat 2 migration is brand-new and unshipped, so editing it *across tasks within this plan* is fine; once merged it is immutable.
- **One new migration file** for all four SQL functions: `migrations/20260626000002_search_beat2_surface_a.sql`. Tasks 1–3 and 5 each append one `CREATE FUNCTION` to it.
- **No new sqlx macro queries.** Every query touching these functions or `::vector` uses runtime `sqlx::query` / `sqlx::query_as` (the established pgvector exception; see the readback module note at `crates/temper-substrate/src/readback/mod.rs:16-21`). Therefore **no `.sqlx` cache regeneration is required** — confirm with offline `cargo make check` at the end (Task 7).
- **Visibility scoping is a correctness invariant, not a knob.** Every candidate function joins `resources_visible_to(p_principal)`. The graph traversal is `kb_resources`-only and `NOT is_folded` (Surface A excludes cogmaps by construction).
- **Tuning constants live in ONE place:** SQL literals inside `unified_search` (`w_fts=1.0`, `w_vec=1.0`, `w_graph=0.5`, `γ=0.5`, `vector_k=100`, `auto_seed_n=20`). Never exposed as API params. Retuning post-ship = a new `CREATE OR REPLACE` migration.
- **Params-struct rule:** the readback's `unified_search` takes a `UnifiedSearchQuery` struct (12 fields), never a 12-arg signature with `#[allow(too_many_arguments)]`.
- **TDD throughout:** failing test → run-and-see-fail → minimal impl → run-and-see-pass → commit.
- **Run before pushing:** `cargo make test-artifacts` (the Embed CI feature set; ONNX + Docker Postgres on `5437`). `DATABASE_URL=postgresql://temper:temper@localhost:5437/temper_development`.

## File Structure

| File | Responsibility | Change |
|------|----------------|--------|
| `migrations/20260626000002_search_beat2_surface_a.sql` | The 4 SQL functions (fts / vector / graph / aggregate) | Create (Task 1), append (Tasks 2,3,5) |
| `crates/temper-substrate/tests/search_surface_a.rs` | artifact-tests for all 4 functions + the blend | Create (Task 1), extend (Tasks 2,3,5) |
| `crates/temper-substrate/src/readback/mod.rs` | `ScoredHit`, `UnifiedSearchQuery`, `unified_search` readback | Modify (Task 5) |
| `crates/temper-core/src/types/api.rs:119` | `UnifiedSearchResultRow.graph_score` field | Modify (Task 4) |
| `crates/temper-cli/src/actions/search.rs:157` | construction site of `UnifiedSearchResultRow` | Modify (Task 4) |
| `crates/temper-api/src/backend/substrate_read.rs:284` | `search_select` rewrite + `clamp_search_params` | Modify (Task 6) |
| `crates/temper-core/types/search.ts` (generated) | regenerated TS type | Regenerate (Task 4) |

The `graph_traverse` function (`migrations/20260624000002_canonical_functions.sql:1308`) is the **pattern reference** for `search_graph_expand` — same `visible` CTE scoping and recursive `WITH RECURSIVE … LANGUAGE sql STABLE` shape — but do **not** reuse it directly: it is forward-direction only, carries no `weight`/decay/score, and has no `edge_types` filter.

---

### Task 1: Migration scaffold + `search_fts_candidates`

**Files:**
- Create: `migrations/20260626000002_search_beat2_surface_a.sql`
- Create: `crates/temper-substrate/tests/search_surface_a.rs`

**Interfaces:**
- Produces SQL: `search_fts_candidates(p_principal uuid, p_query text) RETURNS TABLE(resource_id uuid, fts_norm real)` — normalized lexical score in `[0,1)` via `ts_rank(…, 32)`, visibility-scoped, empty/NULL query → zero rows.

- [ ] **Step 1: Write the failing test** (`crates/temper-substrate/tests/search_surface_a.rs`)

```rust
#![cfg(feature = "artifact-tests")]
//! Search Beat 2 — Surface A candidate functions + the unified blend, on the substrate.
//! Isolated ephemeral DB via `MIGRATOR`.

mod common;

use temper_substrate::ids::{ContextId, EntityId, ProfileId};
use temper_substrate::scenario::bootseed;
use temper_substrate::writes;
use uuid::Uuid;

async fn system_actor(pool: &sqlx::PgPool) -> (ProfileId, EntityId) {
    let profile: Uuid = sqlx::query_scalar("SELECT id FROM kb_profiles WHERE handle='system'")
        .fetch_one(pool).await.unwrap();
    let entity: Uuid =
        sqlx::query_scalar("SELECT id FROM kb_entities WHERE profile_id=$1 AND name='system'")
            .bind(profile).fetch_one(pool).await.unwrap();
    (ProfileId::from(profile), EntityId::from(entity))
}

async fn ctx(pool: &sqlx::PgPool, owner: ProfileId, slug: &str) -> ContextId {
    ContextId::from(common::insert_context(pool, "kb_profiles", owner.uuid(), slug, slug).await.unwrap())
}

/// Create a body-only `concept` resource (no chunks needed for FTS — body is indexed by Beat 1).
async fn mk(pool: &sqlx::PgPool, home: ContextId, owner: ProfileId, emitter: EntityId,
            title: &str, body: &str, uri: &str) -> Uuid {
    writes::create_resource(pool, writes::CreateParams {
        title, origin_uri: uri, body, doc_type: "concept",
        home, owner, originator: owner, emitter, properties: &[], chunks: None,
    }).await.unwrap().uuid()
}

/// Rows from `search_fts_candidates`, as (id, fts_norm).
async fn fts_candidates(pool: &sqlx::PgPool, principal: Uuid, q: &str) -> Vec<(Uuid, f32)> {
    use sqlx::Row;
    sqlx::query("SELECT resource_id, fts_norm FROM search_fts_candidates($1, $2)")
        .bind(principal).bind(q).fetch_all(pool).await.unwrap()
        .iter().map(|r| (r.get::<Uuid, _>("resource_id"), r.get::<f32, _>("fts_norm"))).collect()
}

#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn fts_candidates_normalized_and_scoped(pool: sqlx::PgPool) {
    bootseed::seed_system(&pool).await.unwrap();
    let (owner, emitter) = system_actor(&pool).await;
    let home = ctx(&pool, owner, "fts").await;
    let hit = mk(&pool, home, owner, emitter, "Quenching furnace", "tempering steel hot", "temper://fts/1").await;
    let _miss = mk(&pool, home, owner, emitter, "Unrelated", "nothing relevant here", "temper://fts/2").await;

    let got = fts_candidates(&pool, owner.uuid(), "tempering").await;
    let ids: Vec<Uuid> = got.iter().map(|(id, _)| *id).collect();
    assert!(ids.contains(&hit), "matching resource is a candidate");
    assert!(!ids.contains(&_miss), "non-matching resource is absent");
    let score = got.iter().find(|(id, _)| *id == hit).unwrap().1;
    assert!(score > 0.0 && score < 1.0, "ts_rank|32 normalizes into [0,1): got {score}");

    // Empty query → zero rows (term-zero path).
    assert!(fts_candidates(&pool, owner.uuid(), "").await.is_empty(), "empty query yields no candidates");
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo nextest run -p temper-substrate --features artifact-tests --test search_surface_a fts_candidates_normalized_and_scoped`
Expected: FAIL — `function search_fts_candidates(uuid, text) does not exist`.

- [ ] **Step 3: Write minimal implementation** (`migrations/20260626000002_search_beat2_surface_a.sql`)

```sql
-- Search Beat 2 — Surface A: blend FTS + vector + graph on /api/search.
-- Four additive SQL functions composed by unified_search. Builds on Beat 1's stored tsvector
-- (20260626000001). Additive-only-on-main: new functions, no schema change.

-- ── Lexical candidates: Beat 1's GIN-indexed stored tsvector, normalized to [0,1) ──────────────
-- ts_rank(..., 32) applies the rank/(rank+1) normalization flag — a FIXED, batch-independent
-- transform, so a doc's score does not depend on what else matched (stable across queries/corpus).
CREATE FUNCTION search_fts_candidates(p_principal uuid, p_query text)
RETURNS TABLE (resource_id uuid, fts_norm real)
LANGUAGE sql STABLE AS $$
  SELECT r.id,
         (ts_rank(si.search_vector, plainto_tsquery('english', p_query), 32))::real
    FROM kb_resource_search_index si
    JOIN kb_resources r                       ON r.id = si.resource_id
    JOIN resources_visible_to(p_principal) v   ON v.resource_id = r.id
   WHERE p_query IS NOT NULL AND p_query <> ''
     AND r.is_active
     AND si.search_vector @@ plainto_tsquery('english', p_query);
$$;
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo nextest run -p temper-substrate --features artifact-tests --test search_surface_a fts_candidates_normalized_and_scoped`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add migrations/20260626000002_search_beat2_surface_a.sql crates/temper-substrate/tests/search_surface_a.rs
git commit -m "Search Beat 2: search_fts_candidates (normalized lexical candidates)"
```

---

### Task 2: `search_vector_candidates` (HNSW over-fetch-then-filter)

**Files:**
- Modify: `migrations/20260626000002_search_beat2_surface_a.sql` (append)
- Modify: `crates/temper-substrate/tests/search_surface_a.rs` (append)

**Interfaces:**
- Consumes: the test harness helpers from Task 1.
- Produces SQL: `search_vector_candidates(p_principal uuid, p_emb vector, p_k int) RETURNS TABLE(resource_id uuid, vec_norm real)` — best-chunk-per-resource cosine, `vec_norm = 1 − dist/2` in `[0,1]`, engages `idx_kb_chunks_embedding`, NULL embedding → zero rows.

- [ ] **Step 1: Write the failing test** (append to `search_surface_a.rs`)

```rust
use temper_substrate::content::{PreparedBlock, PreparedChunk};
use temper_substrate::events::{fire, SeedAction};
use temper_substrate::ids::{BlockId, ChunkId, ResourceId};
use temper_substrate::payloads::AnchorRef;

/// One block/chunk with a caller-chosen 768-d embedding (ONNX-free — structural).
fn block_with_embedding(content: &str, emb: Vec<f32>) -> PreparedBlock {
    PreparedBlock {
        block_id: BlockId::from(Uuid::now_v7()), seq: 0, role: None,
        chunks: vec![PreparedChunk {
            chunk_id: ChunkId::from(Uuid::now_v7()), chunk_index: 0,
            content_hash: format!("{:064x}", Uuid::now_v7().as_u128()),
            content: content.to_string(), embedding: emb, header_path: None, heading_depth: None,
        }],
    }
}

async fn mk_embedded(pool: &sqlx::PgPool, home: ContextId, owner: ProfileId, emitter: EntityId,
                     title: &str, uri: &str, emb: Vec<f32>) -> ResourceId {
    let blocks = vec![block_with_embedding(title, emb)];
    let mut tx = pool.begin().await.unwrap();
    let id = fire(&mut tx, SeedAction::ResourceCreate {
        title, origin_uri: uri, resource_id: None, home: AnchorRef::context(home),
        owner, originator: None, blocks: &blocks, doc_type: Some("concept"), emitter,
    }).await.unwrap().resource().unwrap();
    tx.commit().await.unwrap();
    id
}

/// pgvector text literal for binding a query embedding.
fn vlit(v: &[f32]) -> String {
    let mut s = String::from("[");
    for (i, x) in v.iter().enumerate() { if i > 0 { s.push(','); } s.push_str(&x.to_string()); }
    s.push(']'); s
}

fn unit(dim: usize) -> Vec<f32> { let mut e = vec![0.0_f32; 768]; e[dim] = 1.0; e }

async fn vector_candidates(pool: &sqlx::PgPool, principal: Uuid, emb: &[f32], k: i32) -> Vec<(Uuid, f32)> {
    use sqlx::Row;
    sqlx::query("SELECT resource_id, vec_norm FROM search_vector_candidates($1, $2::vector, $3)")
        .bind(principal).bind(vlit(emb)).bind(k).fetch_all(pool).await.unwrap()
        .iter().map(|r| (r.get::<Uuid, _>("resource_id"), r.get::<f32, _>("vec_norm"))).collect()
}

#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn vector_candidates_best_per_resource_normalized(pool: sqlx::PgPool) {
    bootseed::seed_system(&pool).await.unwrap();
    let (owner, emitter) = system_actor(&pool).await;
    let home = ctx(&pool, owner, "vec").await;
    let near = mk_embedded(&pool, home, owner, emitter, "near", "temper://vec/near", unit(0)).await;
    let far  = mk_embedded(&pool, home, owner, emitter, "far",  "temper://vec/far",  unit(1)).await;

    let got = vector_candidates(&pool, owner.uuid(), &unit(0), 100).await;
    let near_score = got.iter().find(|(id, _)| *id == near.uuid()).expect("near present").1;
    let far_score  = got.iter().find(|(id, _)| *id == far.uuid()).expect("far present").1;
    assert!((near_score - 1.0).abs() < 1e-4, "identical embedding ⇒ vec_norm≈1.0: got {near_score}");
    assert!(near_score > far_score, "nearer resource scores higher");
    assert!(far_score >= 0.0 && far_score <= 1.0, "vec_norm bounded [0,1]: got {far_score}");
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo nextest run -p temper-substrate --features artifact-tests --test search_surface_a vector_candidates_best_per_resource_normalized`
Expected: FAIL — `function search_vector_candidates(uuid, vector, integer) does not exist`.

- [ ] **Step 3: Write minimal implementation** (append to the migration)

```sql
-- ── Semantic candidates: HNSW over-fetch-then-filter. The inner `ann` CTE carries ONLY the
-- index's own predicate (is_current) + ORDER BY <=> LIMIT, so idx_kb_chunks_embedding engages.
-- Visibility/active filtering happens AFTER (applying it inside the ANN would force a seq-scan and
-- defeat the index — the exact bug in the legacy GROUP BY/MIN-over-a-join shape). Over-fetch (p_k»limit)
-- absorbs the post-ANN attrition. Best chunk per resource decides rank; vec_norm = 1 - dist/2 ∈ [0,1].
CREATE FUNCTION search_vector_candidates(p_principal uuid, p_emb vector, p_k int)
RETURNS TABLE (resource_id uuid, vec_norm real)
LANGUAGE sql STABLE AS $$
  WITH ann AS (
    SELECT c.resource_id, (c.embedding <=> p_emb) AS dist
      FROM kb_chunks c
     WHERE p_emb IS NOT NULL AND c.is_current
     ORDER BY c.embedding <=> p_emb
     LIMIT p_k
  )
  SELECT a.resource_id, (1.0 - MIN(a.dist) / 2.0)::real
    FROM ann a
    JOIN kb_resources r                       ON r.id = a.resource_id AND r.is_active
    JOIN resources_visible_to(p_principal) v   ON v.resource_id = a.resource_id
   GROUP BY a.resource_id;
$$;
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo nextest run -p temper-substrate --features artifact-tests --test search_surface_a vector_candidates_best_per_resource_normalized`
Expected: PASS.

- [ ] **Step 5: Add the HNSW-engagement regression-guard test** (append)

```rust
/// The vector CTE MUST use idx_kb_chunks_embedding (the whole point of the over-fetch shape).
/// EXPLAIN the inner ANN query and assert an Index Scan on the HNSW index — guards against silently
/// sliding back to a seq-scan blend.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn vector_ann_uses_hnsw_index(pool: sqlx::PgPool) {
    bootseed::seed_system(&pool).await.unwrap();
    let (owner, emitter) = system_actor(&pool).await;
    let home = ctx(&pool, owner, "ann").await;
    for i in 0..5 {
        mk_embedded(&pool, home, owner, emitter, &format!("e{i}"), &format!("temper://ann/{i}"), unit(i)).await;
    }
    // EXPLAIN the index-using shape directly (the function body's `ann` CTE).
    let plan: Vec<(String,)> = sqlx::query_as(
        "EXPLAIN SELECT c.resource_id FROM kb_chunks c WHERE c.is_current \
         ORDER BY c.embedding <=> $1::vector LIMIT 100")
        .bind(vlit(&unit(0))).fetch_all(&pool).await.unwrap();
    let text = plan.iter().map(|(l,)| l.as_str()).collect::<Vec<_>>().join("\n");
    assert!(text.contains("idx_kb_chunks_embedding"),
        "ANN candidate path must use the HNSW index; plan was:\n{text}");
}
```

> **Note:** HNSW is only chosen by the planner when the index exists and the cost model favors it. On a tiny seeded corpus Postgres may prefer a seq-scan. If this assertion is flaky on small N, set `SET LOCAL enable_seqscan = off` before the `EXPLAIN` in the test (the index must still be *usable* — that's what we're guarding), and document why.

- [ ] **Step 6: Run + commit**

Run: `cargo nextest run -p temper-substrate --features artifact-tests --test search_surface_a`
Expected: PASS (all vector tests).

```bash
git add migrations/20260626000002_search_beat2_surface_a.sql crates/temper-substrate/tests/search_surface_a.rs
git commit -m "Search Beat 2: search_vector_candidates (HNSW over-fetch-then-filter)"
```

---

### Task 3: `search_graph_expand` (scoped, weighted, bidirectional, max-over-paths)

**Files:**
- Modify: `migrations/20260626000002_search_beat2_surface_a.sql` (append)
- Modify: `crates/temper-substrate/tests/search_surface_a.rs` (append)

**Interfaces:**
- Consumes: harness helpers (Tasks 1–2) + an edge-assert helper (added here).
- Produces SQL: `search_graph_expand(p_principal uuid, p_seed_ids uuid[], p_depth int, p_edge_types text[], p_gamma double precision) RETURNS TABLE(resource_id uuid, graph_score real)` — seeds at hop 0 score 1.0; neighbors scored `MAX over paths of γ^hop · Π edge_weight`; bidirectional; `resources_visible_to`-scoped; `kb_resources`-only, `NOT is_folded`; `edge_types` filter (empty/NULL = all); depth-capped with a cycle guard.

- [ ] **Step 1: Write the failing test** (append)

```rust
use temper_substrate::affinity::EdgeKind;
use temper_substrate::events::EdgeHome;
use temper_substrate::payloads::EdgePolarity;

/// Assert one weighted edge src→tgt of `kind`, returning nothing.
async fn edge(pool: &sqlx::PgPool, src: ResourceId, tgt: ResourceId, home: ContextId,
              emitter: EntityId, kind: EdgeKind, weight: f64) {
    let mut tx = pool.begin().await.unwrap();
    fire(&mut tx, SeedAction::RelationshipAssert {
        src, tgt, kind, polarity: EdgePolarity::Forward, label: Some("rel"),
        weight, home: EdgeHome::Context(home), emitter,
    }).await.unwrap().relationship().unwrap();
    tx.commit().await.unwrap();
}

async fn graph_expand(pool: &sqlx::PgPool, principal: Uuid, seeds: &[Uuid], depth: i32,
                      edge_types: &[&str], gamma: f64) -> Vec<(Uuid, f32)> {
    use sqlx::Row;
    let et: Vec<String> = edge_types.iter().map(|s| s.to_string()).collect();
    sqlx::query("SELECT resource_id, graph_score FROM search_graph_expand($1, $2::uuid[], $3, $4::text[], $5)")
        .bind(principal).bind(seeds).bind(depth).bind(et).bind(gamma)
        .fetch_all(pool).await.unwrap()
        .iter().map(|r| (r.get::<Uuid, _>("resource_id"), r.get::<f32, _>("graph_score"))).collect()
}

#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn graph_expand_decay_and_max_over_paths(pool: sqlx::PgPool) {
    bootseed::seed_system(&pool).await.unwrap();
    let (owner, emitter) = system_actor(&pool).await;
    let home = ctx(&pool, owner, "g").await;
    // a — b — c  (a is the seed; b at hop 1, c at hop 2), all weight 1.0
    let a = mk_embedded(&pool, home, owner, emitter, "a", "temper://g/a", unit(0)).await;
    let b = mk_embedded(&pool, home, owner, emitter, "b", "temper://g/b", unit(1)).await;
    let c = mk_embedded(&pool, home, owner, emitter, "c", "temper://g/c", unit(2)).await;
    edge(&pool, a, b, home, emitter, EdgeKind::LeadsTo, 1.0).await;
    edge(&pool, b, c, home, emitter, EdgeKind::LeadsTo, 1.0).await;

    let got = graph_expand(&pool, owner.uuid(), &[a.uuid()], 2, &[], 0.5).await;
    let score = |id: Uuid| got.iter().find(|(g, _)| *g == id).map(|(_, s)| *s);
    assert_eq!(score(a.uuid()), Some(1.0), "seed scored 1.0 at hop 0");
    assert!((score(b.uuid()).unwrap() - 0.5).abs() < 1e-5, "hop1: γ^1·w = 0.5");
    assert!((score(c.uuid()).unwrap() - 0.25).abs() < 1e-5, "hop2: γ^2·w = 0.25 (bidirectional walk reached c)");
}

#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn graph_expand_filters_and_scope(pool: sqlx::PgPool) {
    bootseed::seed_system(&pool).await.unwrap();
    let (owner, emitter) = system_actor(&pool).await;
    let home = ctx(&pool, owner, "gf").await;
    let a = mk_embedded(&pool, home, owner, emitter, "a", "temper://gf/a", unit(0)).await;
    let b = mk_embedded(&pool, home, owner, emitter, "b", "temper://gf/b", unit(1)).await;
    edge(&pool, a, b, home, emitter, EdgeKind::LeadsTo, 1.0).await;

    // edge_types filter excludes the only edge ⇒ b unreached.
    let filtered = graph_expand(&pool, owner.uuid(), &[a.uuid()], 2, &["depends_on"], 0.5).await;
    assert!(filtered.iter().all(|(id, _)| *id != b.uuid()), "edge_types filter excludes non-matching kinds");

    // A second profile that cannot see these resources gets no neighbors (visibility scoping).
    let stranger = Uuid::now_v7();
    let unscoped = graph_expand(&pool, stranger, &[a.uuid()], 2, &[], 0.5).await;
    assert!(unscoped.is_empty(), "a principal who cannot see the seeds/neighbors gets nothing");
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo nextest run -p temper-substrate --features artifact-tests --test search_surface_a graph_expand`
Expected: FAIL — `function search_graph_expand(...) does not exist`.

- [ ] **Step 3: Write minimal implementation** (append to the migration)

```sql
-- ── Structural candidates: scoped, weighted, bidirectional multi-hop expansion from seeds.
-- Mirrors graph_traverse's `visible` CTE scoping (canonical_functions.sql:1308) but is purpose-built:
-- BIDIRECTIONAL (follow an edge from either endpoint), WEIGHTED (γ^hop · Π edge_weight), SCORED with
-- MAX-over-paths (hub-robust: best path wins), and edge_kind-filtered. Surface A scope: kb_resources
-- endpoints only, NOT is_folded, every endpoint joined through resources_visible_to. Seeds = hop 0,
-- score 1.0. A path array gives the cycle guard (and bounds termination alongside p_depth).
CREATE FUNCTION search_graph_expand(
  p_principal uuid, p_seed_ids uuid[], p_depth int, p_edge_types text[], p_gamma double precision)
RETURNS TABLE (resource_id uuid, graph_score real)
LANGUAGE sql STABLE AS $$
  WITH RECURSIVE visible AS (
    SELECT rv.resource_id AS id FROM resources_visible_to(p_principal) rv
  ),
  adj AS (   -- undirected adjacency over visible, unfolded, kb_resources edges (optional kind filter)
    SELECT e.source_id AS a, e.target_id AS b, e.weight
      FROM kb_edges e
     WHERE e.source_table = 'kb_resources' AND e.target_table = 'kb_resources'
       AND NOT e.is_folded
       AND (p_edge_types IS NULL OR array_length(p_edge_types, 1) IS NULL
            OR e.edge_kind::text = ANY(p_edge_types))
       AND e.source_id IN (SELECT id FROM visible)
       AND e.target_id IN (SELECT id FROM visible)
  ),
  walk AS (
    SELECT s.id AS node, 1.0::double precision AS score, 0 AS hop, ARRAY[s.id] AS path
      FROM unnest(p_seed_ids) AS s(id)
     WHERE s.id IN (SELECT id FROM visible)
    UNION ALL
    SELECT nb.node, w.score * p_gamma * nb.weight, w.hop + 1, w.path || nb.node
      FROM walk w
      JOIN LATERAL (
        SELECT adj.b AS node, adj.weight FROM adj WHERE adj.a = w.node
        UNION ALL
        SELECT adj.a AS node, adj.weight FROM adj WHERE adj.b = w.node
      ) nb ON true
     WHERE w.hop < p_depth
       AND NOT nb.node = ANY(w.path)
  )
  SELECT node, MAX(score)::real
    FROM walk
   GROUP BY node;
$$;
```

> **Risk note (the riskiest SQL in this plan):** the `JOIN LATERAL (... UNION ALL ...)` inside the recursive term is the part most likely to hit a Postgres restriction. If the planner rejects it, the equivalent fallback is two recursive branches without the LATERAL — one `JOIN adj ON adj.a = w.node` (→ `adj.b`) and one `JOIN adj ON adj.b = w.node` (→ `adj.a`), both `UNION ALL`-ed into `walk`. Keep iterating against the ephemeral DB until `graph_expand_decay_and_max_over_paths` passes; that test pins the exact decay arithmetic.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo nextest run -p temper-substrate --features artifact-tests --test search_surface_a graph_expand`
Expected: PASS (both graph tests).

- [ ] **Step 5: Commit**

```bash
git add migrations/20260626000002_search_beat2_surface_a.sql crates/temper-substrate/tests/search_surface_a.rs
git commit -m "Search Beat 2: search_graph_expand (scoped weighted bidirectional traversal)"
```

---

### Task 4: `UnifiedSearchResultRow.graph_score` field + ts-rs regen

**Files:**
- Modify: `crates/temper-core/src/types/api.rs:119-135` (add field)
- Modify: `crates/temper-cli/src/actions/search.rs:157` (construction site)
- Regenerate: `crates/temper-core/types/search.ts` (or the configured ts-rs output path)

**Interfaces:**
- Produces: `UnifiedSearchResultRow` gains `pub graph_score: f32` between `vector_score` and `combined_score`. Every constructor must set it.

- [ ] **Step 1: Add the field** (`crates/temper-core/src/types/api.rs`, in `struct UnifiedSearchResultRow`)

```rust
    pub fts_score: f32,
    pub vector_score: f32,
    /// Surface A (Beat 2) structural-proximity score: max-over-paths γ^hop·Π edge_weight, 0 when the
    /// candidate was reached only by FTS/vector. Exposed so the graph term is observable for tuning.
    pub graph_score: f32,
    pub combined_score: f32,
```

- [ ] **Step 2: Fix the CLI construction site** (`crates/temper-cli/src/actions/search.rs:157`)

Add `graph_score: 0.0,` to the test-fixture `UnifiedSearchResultRow { … }` literal (between `vector_score` and `combined_score`).

- [ ] **Step 3: Verify the workspace compiles**

Run: `cargo check --workspace --all-features`
Expected: PASS — no "missing field `graph_score`" errors remain (this surfaces every other constructor, if any).

- [ ] **Step 4: Regenerate the TS type**

Run: `cargo make generate-ts-types`
Expected: `search.ts`'s `UnifiedSearchResultRow` now includes `graph_score: number`.

- [ ] **Step 5: Commit**

```bash
git add crates/temper-core/src/types/api.rs crates/temper-cli/src/actions/search.rs crates/temper-core/types/
git commit -m "Search Beat 2: add graph_score to UnifiedSearchResultRow (+ ts-rs regen)"
```

---

### Task 5: `unified_search` SQL aggregate + readback wrapper

**Files:**
- Modify: `migrations/20260626000002_search_beat2_surface_a.sql` (append the aggregate)
- Modify: `crates/temper-substrate/src/readback/mod.rs` (add `ScoredHit`, `UnifiedSearchQuery`, `unified_search`)
- Modify: `crates/temper-substrate/tests/search_surface_a.rs` (append blend tests)

**Interfaces:**
- Consumes: the three candidate functions (Tasks 1–3).
- Produces SQL: `unified_search(p_principal uuid, p_query text, p_emb vector, p_seed_ids uuid[], p_depth int, p_edge_types text[], p_context_id uuid, p_doc_type text, p_graph_expand boolean, p_limit int, p_offset int) RETURNS TABLE(resource_id uuid, fts_score real, vector_score real, graph_score real, combined_score real)`. Owns all tuning constants.
- Produces Rust: `readback::ScoredHit { resource_id, fts_score, vector_score, graph_score, combined_score }`, `readback::UnifiedSearchQuery<'a>` (params struct), `readback::unified_search(pool, UnifiedSearchQuery) -> Result<Vec<ScoredHit>>`.

- [ ] **Step 1: Write the failing blend tests** (append to `search_surface_a.rs`)

```rust
use temper_substrate::readback::{self, UnifiedSearchQuery};

fn q<'a>(principal: Uuid) -> UnifiedSearchQuery<'a> {
    UnifiedSearchQuery {
        principal, query: None, embedding: None, seed_ids: &[], depth: 2,
        edge_types: &[], context_id: None, doc_type: None, graph_expand: true,
        limit: 10, offset: 0,
    }
}

#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn blend_term_zeroing_and_either_or_dissolved(pool: sqlx::PgPool) {
    bootseed::seed_system(&pool).await.unwrap();
    let (owner, emitter) = system_actor(&pool).await;
    let home = ctx(&pool, owner, "bl").await;
    let r = mk_embedded(&pool, home, owner, emitter, "tempering steel", "temper://bl/r", unit(0)).await;

    // Text-only: vector term is 0, fts term drives the score.
    let text_only = readback::unified_search(&pool, UnifiedSearchQuery {
        query: Some("tempering"), ..q(owner.uuid())
    }).await.unwrap();
    let hit = text_only.iter().find(|h| h.resource_id == r.uuid()).expect("found by text");
    assert!(hit.fts_score > 0.0 && hit.vector_score == 0.0, "text-only ⇒ vector term zero");

    // Vector-only: fts term is 0.
    let vec_only = readback::unified_search(&pool, UnifiedSearchQuery {
        embedding: Some(&unit(0)), ..q(owner.uuid())
    }).await.unwrap();
    let hit = vec_only.iter().find(|h| h.resource_id == r.uuid()).expect("found by vector");
    assert!(hit.vector_score > 0.0 && hit.fts_score == 0.0, "vector-only ⇒ fts term zero");
}

#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn blend_self_seeding_boosts_structural_neighbor(pool: sqlx::PgPool) {
    bootseed::seed_system(&pool).await.unwrap();
    let (owner, emitter) = system_actor(&pool).await;
    let home = ctx(&pool, owner, "ss").await;
    // `core` matches the query; `neighbor` does NOT match text but is edged to `core`.
    let core = mk_embedded(&pool, home, owner, emitter, "tempering furnace", "temper://ss/core", unit(0)).await;
    let neighbor = mk_embedded(&pool, home, owner, emitter, "unrelated wording", "temper://ss/nbr", unit(1)).await;
    edge(&pool, core, neighbor, home, emitter, EdgeKind::LeadsTo, 1.0).await;

    let on = readback::unified_search(&pool, UnifiedSearchQuery {
        query: Some("tempering"), graph_expand: true, ..q(owner.uuid())
    }).await.unwrap();
    assert!(on.iter().any(|h| h.resource_id == neighbor.uuid()),
        "graph recall-expansion pulls in the structurally-near non-text-matching neighbor");

    let off = readback::unified_search(&pool, UnifiedSearchQuery {
        query: Some("tempering"), graph_expand: false, ..q(owner.uuid())
    }).await.unwrap();
    assert!(off.iter().all(|h| h.resource_id != neighbor.uuid()),
        "graph_expand=false ⇒ pure FTS∪vector, neighbor absent");
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo nextest run -p temper-substrate --features artifact-tests --test search_surface_a blend_`
Expected: FAIL — `UnifiedSearchQuery` / `readback::unified_search` not found (won't compile).

- [ ] **Step 3a: Write the aggregate SQL function** (append to the migration)

```sql
-- ── The aggregate: compose the three candidate functions into one ranked, scored result.
-- TUNING CONSTANTS LIVE HERE (the one place): weights, γ, vector over-fetch k, auto-seed N. Self-seed:
-- seeds = explicit p_seed_ids ∪ top-N of the pre-graph FTS/vector blend. graph_expand=false ⇒ empty
-- seeds ⇒ graph term zero. Recall = FTS ∪ vector ∪ graph; missing signals COALESCE to 0.
CREATE FUNCTION unified_search(
  p_principal uuid, p_query text, p_emb vector, p_seed_ids uuid[], p_depth int,
  p_edge_types text[], p_context_id uuid, p_doc_type text, p_graph_expand boolean,
  p_limit int, p_offset int)
RETURNS TABLE (resource_id uuid, fts_score real, vector_score real, graph_score real, combined_score real)
LANGUAGE sql STABLE AS $$
  WITH
  k AS (SELECT 1.0::float8 AS w_fts, 1.0::float8 AS w_vec, 0.5::float8 AS w_graph,
               0.5::float8 AS gamma, 100 AS vector_k, 20 AS auto_seed_n),
  fts AS (SELECT * FROM search_fts_candidates(p_principal, p_query)),
  vec AS (SELECT * FROM search_vector_candidates(p_principal, p_emb, (SELECT vector_k FROM k))),
  blend0 AS (
    SELECT COALESCE(f.resource_id, v.resource_id) AS id,
           (SELECT w_fts FROM k) * COALESCE(f.fts_norm, 0)
         + (SELECT w_vec FROM k) * COALESCE(v.vec_norm, 0) AS s0
      FROM fts f FULL OUTER JOIN vec v ON f.resource_id = v.resource_id
  ),
  seeds AS (
    SELECT unnest(COALESCE(p_seed_ids, ARRAY[]::uuid[])) AS id
    UNION
    SELECT id FROM (SELECT id, s0 FROM blend0 ORDER BY s0 DESC LIMIT (SELECT auto_seed_n FROM k)) t
  ),
  graph AS (
    SELECT * FROM search_graph_expand(
      p_principal,
      CASE WHEN p_graph_expand THEN ARRAY(SELECT id FROM seeds) ELSE ARRAY[]::uuid[] END,
      p_depth, p_edge_types, (SELECT gamma FROM k))
  ),
  cand AS (SELECT id FROM blend0 UNION SELECT resource_id FROM graph),
  corpus AS (   -- context/doc_type candidate-corpus filter
    SELECT c.id FROM cand c
     WHERE (p_context_id IS NULL OR EXISTS (
             SELECT 1 FROM kb_resource_homes h
              WHERE h.resource_id = c.id AND h.anchor_table = 'kb_contexts' AND h.anchor_id = p_context_id))
       AND (p_doc_type IS NULL OR EXISTS (
             SELECT 1 FROM kb_properties p
              WHERE p.owner_table = 'kb_resources' AND p.owner_id = c.id
                AND p.property_key = 'doc_type' AND NOT p.is_folded
                AND p.property_value #>> '{}' = p_doc_type))
  ),
  scored AS (
    SELECT co.id,
           COALESCE(f.fts_norm, 0)::real    AS fts_score,
           COALESCE(v.vec_norm, 0)::real    AS vector_score,
           COALESCE(g.graph_score, 0)::real AS graph_score,
           ((SELECT w_fts FROM k)   * COALESCE(f.fts_norm, 0)
          + (SELECT w_vec FROM k)   * COALESCE(v.vec_norm, 0)
          + (SELECT w_graph FROM k) * COALESCE(g.graph_score, 0))::real AS combined_score
      FROM corpus co
      LEFT JOIN fts f   ON f.resource_id = co.id
      LEFT JOIN vec v   ON v.resource_id = co.id
      LEFT JOIN graph g ON g.resource_id = co.id
  )
  SELECT id, fts_score, vector_score, graph_score, combined_score
    FROM scored
   ORDER BY combined_score DESC, id
   LIMIT p_limit OFFSET p_offset;
$$;
```

- [ ] **Step 3b: Add the readback wrapper** (`crates/temper-substrate/src/readback/mod.rs`)

```rust
/// One scored hit from Surface A unified search (Beat 2). The scores are the real blended sub-scores —
/// the either/or path's 0.0 placeholders are gone.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct ScoredHit {
    pub resource_id: Uuid,
    pub fts_score: f32,
    pub vector_score: f32,
    pub graph_score: f32,
    pub combined_score: f32,
}

/// Request parameters for [`unified_search`] (params struct — 11 domain fields). Borrowed views; the
/// caller owns the underlying `SearchParams`. Empty `seed_ids`/`edge_types` ⇒ no explicit seeds / all
/// edge kinds. `None` `query`/`embedding` ⇒ that signal's term is zeroed in the blend.
#[derive(Debug, Clone)]
pub struct UnifiedSearchQuery<'a> {
    pub principal: Uuid,
    pub query: Option<&'a str>,
    pub embedding: Option<&'a [f32]>,
    pub seed_ids: &'a [Uuid],
    pub depth: i32,
    pub edge_types: &'a [String],
    pub context_id: Option<Uuid>,
    pub doc_type: Option<&'a str>,
    pub graph_expand: bool,
    pub limit: i64,
    pub offset: i64,
}

/// Surface A general search (Beat 2): one composed SQL statement (`unified_search`) blending FTS +
/// vector + graph into ranked, scored hits. Runtime `sqlx::query_as` — the `::vector` cast forbids the
/// compile-time macros (module note). All tuning constants live in the SQL function, not here.
pub async fn unified_search(pool: &PgPool, q: UnifiedSearchQuery<'_>) -> Result<Vec<ScoredHit>> {
    let emb_text = q.embedding.map(format_pgvector);
    let edge_types: Vec<String> = q.edge_types.to_vec();
    let hits = sqlx::query_as::<_, ScoredHit>(
        "SELECT resource_id, fts_score, vector_score, graph_score, combined_score
           FROM unified_search($1, $2, $3::vector, $4::uuid[], $5, $6::text[], $7, $8, $9, $10, $11)",
    )
    .bind(q.principal)
    .bind(q.query)
    .bind(emb_text)        // NULL when None → p_emb NULL → vector term zeroed
    .bind(q.seed_ids)
    .bind(q.depth)
    .bind(edge_types)
    .bind(q.context_id)
    .bind(q.doc_type)
    .bind(q.graph_expand)
    .bind(q.limit)
    .bind(q.offset)
    .fetch_all(pool)
    .await?;
    Ok(hits)
}
```

> `format_pgvector` already exists in this module (`mod.rs:837`). `ScoredHit`, `UnifiedSearchQuery`, and `unified_search` must be re-exported if the module gates items behind a `pub use` — match the existing visibility of `fts_search`/`vector_search` (they are `pub` in `readback`, surfaced as `readback::fts_search`).

- [ ] **Step 4: Run the blend tests to verify they pass**

Run: `cargo nextest run -p temper-substrate --features artifact-tests --test search_surface_a blend_`
Expected: PASS.

- [ ] **Step 5: Add filter + ordering tests** (append)

```rust
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn blend_context_and_doctype_filters(pool: sqlx::PgPool) {
    bootseed::seed_system(&pool).await.unwrap();
    let (owner, emitter) = system_actor(&pool).await;
    let home = ctx(&pool, owner, "flt").await;
    let r = mk(&pool, home, owner, emitter, "tempering one", "body tempering", "temper://flt/r").await;

    // doc_type filter that excludes 'concept' ⇒ no hits.
    let none = readback::unified_search(&pool, UnifiedSearchQuery {
        query: Some("tempering"), doc_type: Some("session"), ..q(owner.uuid())
    }).await.unwrap();
    assert!(none.iter().all(|h| h.resource_id != r.uuid()), "doc_type filter restricts the corpus");

    // doc_type='concept' keeps it.
    let some = readback::unified_search(&pool, UnifiedSearchQuery {
        query: Some("tempering"), doc_type: Some("concept"), ..q(owner.uuid())
    }).await.unwrap();
    assert!(some.iter().any(|h| h.resource_id == r.uuid()), "matching doc_type passes the filter");
}
```

- [ ] **Step 6: Run all artifact tests + commit**

Run: `cargo nextest run -p temper-substrate --features artifact-tests --test search_surface_a`
Expected: PASS (all Surface A tests).

```bash
git add migrations/20260626000002_search_beat2_surface_a.sql crates/temper-substrate/src/readback/mod.rs crates/temper-substrate/tests/search_surface_a.rs
git commit -m "Search Beat 2: unified_search aggregate + readback wrapper"
```

---

### Task 6: Rewrite `search_select` + `clamp_search_params`

**Files:**
- Modify: `crates/temper-api/src/backend/substrate_read.rs:284-326` (rewrite `search_select`, add `clamp_search_params`)

**Interfaces:**
- Consumes: `temper_substrate::readback::{unified_search, UnifiedSearchQuery}` (Task 5); `UnifiedSearchResultRow.graph_score` (Task 4); existing `native_resource_row` (returns `.title`, `.origin_uri`, `.context_name`, `.doc_type_name`).
- Produces: `/api/search` returns real blended scores; `SearchParams` graph fields + context/doc_type filters are honored; `graph_depth` clamped `[1,3]`, `limit` clamped `[1,50]`.

- [ ] **Step 1: Write the failing unit test for clamping** (in `substrate_read.rs`, under a `#[cfg(test)] mod tests`)

```rust
#[cfg(test)]
mod clamp_tests {
    use super::*;
    use temper_core::types::api::SearchParams;

    #[test]
    fn clamps_depth_and_limit_to_surface_a_caps() {
        let p = SearchParams { graph_depth: Some(10), limit: Some(999), ..SearchParams::default() };
        let c = clamp_search_params(&p);
        assert_eq!(c.depth, 3, "graph_depth capped at 3 for Surface A");
        assert_eq!(c.limit, 50, "limit capped at 50");

        let d = clamp_search_params(&SearchParams::default());
        assert_eq!(d.depth, 2, "default depth 2");
        assert_eq!(d.limit, 10, "default limit 10");
    }
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo nextest run -p temper-api --features test-db clamps_depth_and_limit_to_surface_a_caps`
Expected: FAIL — `clamp_search_params` not found.

- [ ] **Step 3: Implement the rewrite** (replace `search_select`, add the clamp helper + struct)

```rust
/// Surface A caps resolved once, before the SQL call (pure → unit-tested).
pub(crate) struct ClampedSearch {
    pub depth: i32,
    pub limit: i64,
}

/// graph_depth → [1,3] (deep traversal is a Surface-B concern; a 10-hop fan-out would threaten the DB);
/// limit → [1,50] (the documented API ceiling). Defaults: depth 2, limit 10.
pub(crate) fn clamp_search_params(params: &SearchParams) -> ClampedSearch {
    ClampedSearch {
        depth: params.graph_depth.unwrap_or(2).clamp(1, 3),
        limit: params.limit.unwrap_or(10).clamp(1, 50),
    }
}

/// `search` — Surface A general search (Beat 2): one composed `unified_search` readback blending FTS +
/// vector + graph into ranked, scored hits, then per-row display enrichment. Replaces the either/or,
/// zero-score path. Visibility is enforced inside every candidate function (`resources_visible_to`).
pub async fn search_select(
    pool: &PgPool,
    profile_id: Uuid,
    params: SearchParams,
) -> ApiResult<Vec<UnifiedSearchResultRow>> {
    let clamped = clamp_search_params(&params);
    let context_id = match params.context_name.as_deref() {
        Some(name) => resolve_context_id(pool, name).await?,   // see Step 4
        None => None,
    };

    let hits = readback::unified_search(
        pool,
        readback::UnifiedSearchQuery {
            principal: profile_id,
            query: params.query.as_deref(),
            embedding: params.embedding.as_deref(),
            seed_ids: params.seed_ids.as_deref().unwrap_or(&[]),
            depth: clamped.depth,
            edge_types: params.edge_types.as_deref().unwrap_or(&[]),
            context_id,
            doc_type: params.doc_type.as_deref(),
            graph_expand: params.graph_expand,
            limit: clamped.limit,
            offset: params.offset.unwrap_or(0),
        },
    )
    .await
    .map_err(api_err)?;

    let mut out = Vec::with_capacity(hits.len());
    for h in hits {
        // Per-row display enrichment (unchanged from the pre-Beat-2 path; the candidate set is ≤ limit).
        let row = native_resource_row(pool, profile_id, h.resource_id).await?;
        out.push(UnifiedSearchResultRow {
            resource_id: h.resource_id,
            title: row.title,
            slug: String::new(),
            kb_uri: row.origin_uri.clone(),
            origin_uri: row.origin_uri,
            context: Some(row.context_name),
            doc_type: row.doc_type_name,
            fts_score: h.fts_score,
            vector_score: h.vector_score,
            graph_score: h.graph_score,
            combined_score: h.combined_score,
            origin: "unified".to_string(),
        });
    }
    Ok(out)
}
```

- [ ] **Step 4: Wire `resolve_context_id`**

Reuse the existing context-name→id resolution that `list_resources`/`enriched_list` already use for the `context_name` filter (search `substrate_read.rs` / the readback for how list resolves a context name). If a reusable helper exists, call it; if the resolution is currently inlined in the list path, extract a small `async fn resolve_context_id(pool: &PgPool, name: &str) -> ApiResult<Option<Uuid>>` (returns `None` when the name doesn't resolve — an unknown context yields an empty corpus, not an error) and use it from both sites. Do not inline a second copy.

- [ ] **Step 5: Run the clamp test + compile**

Run: `cargo nextest run -p temper-api --features test-db clamps_depth_and_limit_to_surface_a_caps`
Expected: PASS.
Run: `cargo check --workspace --all-features`
Expected: PASS.

- [ ] **Step 6: Run the e2e search suite (regression gate)**

Run: `cargo make test-e2e` (covers `search_test.rs`, `fts_search_test.rs`, `graph_search_test.rs` through real Axum + Postgres)
Expected: PASS — existing search behavior preserved; results now carry non-zero scores.

> If any e2e search test asserted on the old `0.0` scores or the `origin: "fts"`/`"vector"` tag, update that assertion to the Beat 2 reality (real scores; `origin: "unified"`) — that is a correct behavior change, not a regression. Bundle the assertion update into this task's commit.

- [ ] **Step 7: Commit**

```bash
git add crates/temper-api/src/backend/substrate_read.rs tests/e2e/
git commit -m "Search Beat 2: search_select blends + ranks via unified_search; clamp + filters wired"
```

---

### Task 7: Full-suite verification + branch finalize

**Files:** none (verification only).

- [ ] **Step 1: Offline check (honest sqlx-cache + clippy probe)**

Run: `cargo make check`
Expected: PASS. (If it fails with `relation/function does not exist`, the local DB is behind — run `sqlx migrate run` against the dev DB, then re-run. No `.sqlx` regeneration is expected because no new macro queries were added; if `check` reports a stale cache entry, that's a signal a macro query crept in — investigate before regenerating.)

- [ ] **Step 2: Run the substrate artifact tests (the core gate)**

Run: `cargo make test-artifacts`
Expected: PASS — all `search_surface_a` tests + the existing Beat 1 `search_index` tests green.

- [ ] **Step 3: Run the full Rust + e2e suite**

Run: `cargo make test-e2e` and `cargo make test`
Expected: PASS.

- [ ] **Step 4: Confirm the migration applies cleanly from scratch**

Run: `cargo make db-reset` (or apply migrations onto a fresh DB) — confirms `20260626000002` applies on top of the canonical set with no ordering surprises.
Expected: all migrations apply; no error.

- [ ] **Step 5: Final commit (if Steps 1–4 produced any fixups)**

```bash
git add -A
git commit -m "Search Beat 2: verification fixups"
```

(Push / PR is a separate, user-gated step — do not push as part of plan execution.)

---

## Self-Review

**Spec coverage:**
- §3.1 composed-SQL one-statement aggregate → Task 5 (`unified_search`). ✓
- §3.2 `search_fts_candidates` (ts_rank|32) → Task 1. ✓
- §3.2 `search_vector_candidates` HNSW over-fetch → Task 2 (+ EXPLAIN guard). ✓
- §3.2 `search_graph_expand` scoped/weighted/bidirectional/max-over-paths → Task 3. ✓
- §3.3 weighted-sum fusion → Task 5 `scored` CTE. ✓
- §3.3/§3.4 weights as one-place SQL constants + defaults → Task 5 `k` CTE. ✓
- §3.5 `SearchParams` fields live + context/doc_type filters → Tasks 5 (SQL) + 6 (wiring). ✓
- §3.5 scores exposed + `graph_score` field → Task 4. ✓
- §3.5 `search_select` collapse + `origin="unified"` → Task 6. ✓
- §3.4 depth/limit clamps → Task 6 `clamp_search_params`. ✓
- §3.6 migration additive → Global Constraints + Task 7 Step 4. ✓
- §5 test plan (per-function, blend, EXPLAIN, filters) → Tasks 1–6 tests. ✓
- Spec open Q1 (display fold-in) resolved: keep per-row enrichment, fold-in deferred (Task 6 note). ✓
- Spec open Q2 (`origin`) resolved: `"unified"` (Task 6). ✓

**Placeholder scan:** No TBD/TODO. The two "find the existing helper" steps (Task 5 re-export visibility, Task 6 `resolve_context_id`) name the exact symbols to match and the fallback if absent — not open-ended.

**Type consistency:** `ScoredHit` fields (Task 5) ↔ `UnifiedSearchResultRow` mapping (Task 6) ↔ `graph_score` field (Task 4) all agree. `UnifiedSearchQuery` field names/types match the `unified_search` readback binds (Task 5) and the `search_select` construction (Task 6). SQL function signatures in the migration match every call site (test helpers + `unified_search` + the readback string).

## Execution Handoff

Per the standing preference (subagent-driven execution, review deferred to end of plan), this plan is ready for **subagent-driven-development**: one fresh subagent per task, with consolidated spec + code-quality review after Task 7 rather than per-task review chaining.
