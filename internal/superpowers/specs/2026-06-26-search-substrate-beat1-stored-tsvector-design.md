# Search Substrate — Beat 1: Stored tsvector + GIN index (design)

**Date:** 2026-06-26
**Arc:** Search followup — leverage the substrate (graph-nearness + cogmap-region salience)
**Beat:** 1 of 3 (foundation). See the goal roadmap for Beats 2 (Surface A general search) and 3 (Surface B cogmap wayfinding).
**Mode:** build · **Effort:** medium

---

## 1. Problem

FTS over the knowledge base is **unindexed at scale**. The production search path builds the
full-text tsvector **inline at query time**, over every visible resource, by `string_agg`-ing all
of a resource's current chunk content and running `to_tsvector` per row. There is no stored
`tsvector` column and no GIN index anywhere in the canonical schema. As the corpus grows this is an
O(corpus) sequential scan per query.

This Beat fixes only that regression: it adds a **stored, maintained, GIN-indexed** full-text vector.
It deliberately does **not** change the search *ranking model* (still either/or FTS-xor-vector, scores
still passthrough) — that is Beat 2's job. Beat 1 is the indexing foundation Beat 2 builds the blend on.

## 2. Current-state ground truth (verified against the live tree, 2026-06-26)

- **Live search path:** `POST /api/search` → `crates/temper-api/src/handlers/search.rs` →
  `crate::backend::substrate_read::search_select` (`crates/temper-api/src/backend/substrate_read.rs:284`)
  → `temper_substrate::readback::fts_search` / `readback::vector_search`
  (`crates/temper-substrate/src/readback/mod.rs:684` / `:746`).
- **FTS is inline + unindexed.** `readback::fts_search` (mod.rs:684) builds, per query:
  ```sql
  setweight(to_tsvector('english', r.title), 'A')
    || setweight(to_tsvector('english', COALESCE(string_agg(cc.content,' '), '')), 'B')
  ```
  over `kb_resources ⨝ resources_visible_to($1) ⨝ kb_chunks(is_current) ⨝ kb_chunk_content`,
  `GROUP BY r.id, r.title`, filtered by `search_vector @@ plainto_tsquery('english', $2)`,
  ordered `ts_rank(...) DESC`. No stored vector, no index.
- **Weighting is already title@A / body@B** (slug was dissolved by WS6 §7 — title-only @A).
- **Scores are discarded.** `search_select` returns the matched id set, then emits
  `fts_score: 0.0, vector_score: 0.0, combined_score: 0.0` for every row (substrate_read.rs:319-321).
  Order is whatever the readback `ORDER BY` produced.
- **Maintenance idiom is event-sourced projection, NOT triggers.** The canonical schema uses triggers
  only for invariant enforcement (`kb_events_append_only`, schema:504) and membership sync
  (`trg_sync_*`, functions:79/111). All read-model state is materialized by `_project_*` functions:
  - `_project_blocks` (functions:619) — content/chunks; ends with
    `_recompute_resource_body_hash(p_resource, v_occurred)` (functions:660).
  - `_project_block_mutated` (functions:914) — content edits.
  - `_project_resource_updated` (functions:1079) — title/meta updates.
  - `_project_resource_created` (functions:723) — initial create (projects title, then blocks).
  - Helper `block_body_text(p_block)` (functions:348) aggregates a block's chunk content.
- **Schema substrate that already exists:** `kb_chunks.embedding vector(768)` with a partial HNSW index
  `idx_kb_chunks_embedding USING hnsw (embedding vector_cosine_ops) WHERE is_current` (schema:579);
  `kb_chunk_content(chunk_id, content)` (schema:583). (HNSW under-use is a **Beat 2** concern.)

## 3. Legacy inspiration (git `5d5e852`, dropped by the WS6 collapse)

The deleted `migrations/20260405000001_fts_search_index.sql` is the blueprint. What we **carry forward**:
- A stored `kb_resource_search_index(resource_id PK, search_vector tsvector, search_config)`.
- A GIN index `USING GIN (search_vector) WITH (fastupdate = off)` (ingest-batch-then-search write
  pattern → trade slower writes for no pending-list merge on read).
- A `rebuild_resource_search_vector(resource_id)` that upserts the weighted vector (title@A + body@B).
- A backfill that populates every active resource.

What we **change** (the legacy migration targets the pre-collapse schema):
- **No triggers.** Legacy used three gated triggers (`temper.skip_search_rebuild`) to fight O(n²) during
  batch chunk writes. The canonical projection functions already process a resource's full block/chunk
  set in one call, so a single rebuild at the end of projection is naturally O(1)-per-write. The rebuild
  belongs in the `_project_*` functions, beside `_recompute_resource_body_hash` — not in triggers.
- **Title-only @A.** Legacy weighted `title || slug` @A; slug is gone (§7). Title@A, body@B.
- **Body source** = `string_agg(current-chunk content)` (the raw FTS body, exactly as the inline read
  aggregates today) — NOT the heading-prefixed assembled markdown from `block_body_text`/`reconstruct_body`
  (that is the `get_content` body, wrong for FTS).

## 4. Design

### 4.1 Storage — new table

```sql
CREATE TABLE kb_resource_search_index (
    resource_id    UUID PRIMARY KEY REFERENCES kb_resources(id) ON DELETE CASCADE,
    search_vector  tsvector NOT NULL,
    search_config  VARCHAR(64) NOT NULL DEFAULT 'english',
    updated        TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX idx_resource_search_vector
    ON kb_resource_search_index USING GIN (search_vector) WITH (fastupdate = off);
```
Dedicated table (not a column on `kb_resources`): the body is an aggregate across other tables, so a
`GENERATED ALWAYS` column is impossible; and keeping the tsvector off the hot `kb_resources` row keeps
metadata reads narrow. `ON DELETE CASCADE` ties the index row's lifetime to the resource.

### 4.2 Maintenance — a projection helper, wired into the `_project_*` choke points

```sql
CREATE FUNCTION _rebuild_resource_search_vector(p_resource uuid)
RETURNS void LANGUAGE plpgsql AS $$
DECLARE
    v_config varchar(64);
    v_title  text;
    v_body   text;
BEGIN
    SELECT COALESCE((SELECT search_config FROM kb_resource_search_index WHERE resource_id = p_resource),
                    'english')
      INTO v_config;
    SELECT title INTO v_title FROM kb_resources WHERE id = p_resource;
    IF v_title IS NULL THEN RETURN; END IF;   -- resource gone; nothing to index
    SELECT COALESCE(string_agg(cc.content, ' '), '')
      INTO v_body
      FROM kb_chunks c
      JOIN kb_chunk_content cc ON cc.chunk_id = c.id
     WHERE c.resource_id = p_resource AND c.is_current;
    INSERT INTO kb_resource_search_index (resource_id, search_vector, search_config, updated)
    VALUES (p_resource,
            setweight(to_tsvector(v_config::regconfig, COALESCE(v_title,'')), 'A')
              || setweight(to_tsvector(v_config::regconfig, v_body), 'B'),
            v_config, now())
    ON CONFLICT (resource_id) DO UPDATE
        SET search_vector = EXCLUDED.search_vector, updated = now();
END;
$$;
```

Call sites (idempotent upsert, so re-entrancy is safe):
- **`_project_blocks`** — add `PERFORM _rebuild_resource_search_vector(p_resource);` immediately after the
  existing `_recompute_resource_body_hash(p_resource, v_occurred)` at functions:660 (covers create + any
  full block projection). Body + title both exist by this point.
- **`_project_block_mutated`** (functions:914) — same call after its content projection (covers edits).
- **`_project_resource_updated`** (functions:1079) — call **only when the title changed** (a title-only
  update mutates no chunks, so `_project_blocks` won't fire). Guard on the title delta the function
  already computes; if it does not currently diff the title, rebuild unconditionally there (cheap — one
  upsert per resource-update event).

> These three functions are recreated by editing the **canonical functions migration's content is
> immutable once shipped** — so the change ships as a **new additive migration** that
> `CREATE OR REPLACE`s the three projection functions with the added `PERFORM` line, plus the new table,
> index, helper, and backfill. We never edit `20260624000002_canonical_functions.sql` in place.

### 4.3 Backfill

```sql
INSERT INTO kb_resource_search_index (resource_id, search_vector, search_config, updated)
SELECT r.id,
       setweight(to_tsvector('english', COALESCE(r.title,'')), 'A')
         || setweight(to_tsvector('english', COALESCE(b.body,'')), 'B'),
       'english', now()
FROM kb_resources r
LEFT JOIN LATERAL (
    SELECT string_agg(cc.content, ' ') AS body
      FROM kb_chunks c JOIN kb_chunk_content cc ON cc.chunk_id = c.id
     WHERE c.resource_id = r.id AND c.is_current
) b ON true
WHERE r.is_active
ON CONFLICT (resource_id) DO UPDATE
    SET search_vector = EXCLUDED.search_vector, updated = now();
```
Idempotent (upsert), safe to re-run. Ships in the **same migration** as the table so the read swap never
observes an empty index.

### 4.4 Read-path swap

Replace the inline-tsvector build in `temper_substrate::readback::fts_search` (mod.rs:684) with a read of
the stored index — same shape, same visibility join, same ordering:

```sql
SELECT r.id
  FROM kb_resource_search_index si
  JOIN kb_resources r            ON r.id = si.resource_id
  JOIN resources_visible_to($1) v ON v.resource_id = r.id
 WHERE r.is_active
   AND si.search_vector @@ plainto_tsquery('english', $2)
 ORDER BY ts_rank(si.search_vector, plainto_tsquery('english', $2)) DESC
```
The matched **set** and order are preserved (the stored vector is byte-identical to what the inline build
produced — same title@A/body@B/'english' recipe), so this is a behavior-preserving swap, not a ranking
change. Remains runtime `sqlx::query` (the readback module convention; see its module note).

### 4.5 Scores

**Out of scope for Beat 1.** `search_select` keeps emitting `0.0` scores. Threading the real `ts_rank`
through `UnifiedSearchResultRow.fts_score` is part of Beat 2's ranking work (it is meaningless without the
blend). Beat 1 changes *where the vector comes from*, nothing the caller observes beyond latency.

## 5. Decisions

1. **Projection-function maintenance, not triggers.** Matches the canonical event-sourcing idiom; the
   batch O(n²) problem that forced legacy's trigger-gating doesn't exist when the rebuild rides the
   already-batched projection call.
2. **Dedicated index table, not a `kb_resources` column.** Aggregate body precludes a generated column;
   keeps the hot row narrow.
3. **Behavior-preserving read swap.** Beat 1 must not change result sets or order — it's an indexing
   change. This keeps it independently shippable and trivially testable (set-equality against the old
   inline query on the real corpus).
4. **`fastupdate = off`** on the GIN index — Temper's write pattern is ingest-batch-then-search.
5. **Single migration** (table + index + helper + 3 `CREATE OR REPLACE` projections + backfill) so the
   index is populated and maintained atomically before any code reads it. Additive-only-on-`main`
   compliant (new table/index/function + `CREATE OR REPLACE` + insert-only backfill — no destructive DDL).

## 6. Out of scope

### Rejected (load-bearing — resist scope creep back in)
- **Triggers** (§5.1). The canonical schema reserves triggers for invariants/membership; read-model
  projection goes through `_project_*`. Re-introducing search triggers would fork the maintenance model.
- **A `search_config` per-resource override surface.** The column exists (default `'english'`) for future
  multilingual support, but no CLI/API path sets it in Beat 1. It is storage-only until a real need.
- **Changing the FTS recipe** (e.g. adding facet text, header paths, or weighting tiers). Beat 1
  reproduces today's recipe exactly so the swap is provably behavior-preserving.

### Deferred (in scope for a later Beat)
- **Real `ts_rank` scores through the API** → Beat 2 (ranking blend).
- **HNSW-using vector query** (the `GROUP BY/MIN` defeats `idx_kb_chunks_embedding`) → Beat 2.
- **Graph-expansion + region-salience** → Beats 2 (Surface A) / 3 (Surface B).

## 7. Test plan

- **Parity test (the core gate):** on a seeded corpus, assert the stored-index `fts_search` returns the
  **same id set** (and, for title/body terms, the same order) as the pre-swap inline query for a battery
  of queries. This is the substrate `artifact-tests` surface (`#[sqlx::test(migrator = ...)]`, ephemeral
  DB) — extend the existing readback/search floor test.
- **Maintenance tests:** create a resource → index row exists with expected lexemes; mutate a block's
  content → vector reflects new body; update title only → vector reflects new title; fold/rotate a chunk
  (`is_current` flip via the mutation path) → body reflects current chunks only.
- **Backfill test:** pre-insert resources, run the migration, assert every active resource has an index
  row matching the recipe.
- **sqlx cache:** new/changed macro queries → regenerate workspace + per-crate caches
  (`cargo sqlx prepare --workspace -- --all-features`, plus `cargo make prepare-*` for any test-target
  queries). Run `cargo make test-artifacts` (the Embed CI job's feature set) locally before pushing.

## 8. Open questions

- **Does `_project_resource_updated` already diff the title?** If yes, guard the rebuild on that delta; if
  no, rebuild unconditionally there (cheap). Confirm when implementing (read functions:1079-1093).
- **Is there a resource *delete/deactivate* projection** that should leave a stale index row? `ON DELETE
  CASCADE` handles hard deletes; soft-delete (`is_active=false`) leaves the row, but the read filters
  `r.is_active`, so a stale row is harmless. Confirm no other reader trusts the index unfiltered.
