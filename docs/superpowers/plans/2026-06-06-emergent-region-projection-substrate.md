# Emergent Region Projection — Plan 1: Substrate Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.
>
> **GROUNDING DISCIPLINE (inject + obey):** `~/.claude/skills/temper/guidance/implementation-grounding.md`. In particular: **GD-1** every "grounded in disk" claim carries a quoted excerpt or command output; **GD-2** grounding is *executable* here — `psql -f` the artifact and run the verdict queries, do not narrate; **GD-3** each task is tagged CONFORM/EXTEND/AMEND with citations; **GD-5** if a step can't be grounded, report BLOCKED, don't fabricate.

**Goal:** Extend the `temper_next` schema-artifact so `kb_cogmap_regions` can hold a *computed, decomposed* region projection (salience parts, content-cohesion, tension), introduce the immutable `kb_cogmap_lenses` perspective table, let edges carry facets, and add the SQL readout functions — leaving the cluster *membership* computation to Plan 2.

**Architecture:** Pure DDL + SQL functions against the destination artifact (`schema-artifact/{01_schema,02_functions,03_seed}.sql`). The readout functions are deterministic SQL aggregates over `kb_cogmap_region_members` + `kb_chunks` (embeddings) + `kb_edges` + `kb_block_provenance`; the region table becomes the memoization of the §1 projection. No Rust in this plan. Tested by `psql` verdict queries against a tiny in-file fixture, matching the artifact's `04_scenarios.sql` idiom.

**Tech Stack:** PostgreSQL 18 + pgvector (cosine via `<=>`, `avg(vector)` aggregate), `psql`. DB: `postgresql://temper:temper@localhost:5437/temper_development`, schema `temper_next` (set via `search_path`). Spec: [`docs/superpowers/specs/2026-06-06-emergent-region-projection-design.md`](../specs/2026-06-06-emergent-region-projection-design.md) (§3, §4, §2c, §6).

**Verified baseline (GD-2, executed 2026-06-06):** artifact loads clean (`01→03`); `kb_cogmap_regions` has exactly `{id, cogmap_id, centroid, salience, label, member_count, asserted_by_event_id, last_event_id, is_folded, created}`; `kb_cogmap_lenses` does **not** exist (`to_regclass` → NULL); `kb_properties` CHECK = `('kb_resources','kb_cogmaps')`. No `boundary_kind` exists — §3C is a no-op on disk (nothing to drop; do not introduce it).

---

## File Structure

| File | Responsibility | This plan |
|------|----------------|-----------|
| `schema-artifact/01_schema.sql` | tables, enums, indexes | **Modify** — add columns to `kb_cogmap_regions`; add `kb_cogmap_lenses` + `lens_id` FK; widen `kb_properties` CHECK |
| `schema-artifact/02_functions.sql` | access + projection functions | **Modify** — add 6 readout functions; extend `cogmap_shape` |
| `schema-artifact/03_seed.sql` | one worked scenario | **Modify** — seed the telos-default lens; point the existing hand-seeded region at it + hand-fill new columns (replaced wholesale in Plan 3) |
| `schema-artifact/04a_plan1_fixture.sql` | **Create** — isolated fixture + verdict queries proving each readout, kept separate from the main scenario suite |

> **Load order unchanged this plan:** `01 → 02 → 03`. `04a_plan1_fixture.sql` is run after `03` to verify. (Plan 3 introduces the harness step + supersedes the hand-seed.)

> **Re-grounding note for the implementer (GD-1):** `01_schema.sql` begins with `DROP SCHEMA IF EXISTS temper_next CASCADE`, so the whole artifact re-materializes from scratch every load — there is no migration; you edit the destination DDL directly and reload. Confirm with `head -5 schema-artifact/01_schema.sql` before starting.

---

## Task 1: Add the computed-readout columns to `kb_cogmap_regions`

**Tag:** EXTEND `kb_cogmap_regions` (spec §3A/§3B authorize: salience becomes *computed/memoized + decomposed*; character readouts added). CONFORM to the existing assert/fold column conventions (`DOUBLE PRECISION`, nullable derived values).

**Files:**
- Modify: `schema-artifact/01_schema.sql` (the `CREATE TABLE kb_cogmap_regions` block — verified at the columns above)

- [ ] **Step 1: Write the failing verdict (a column-presence probe)**

Create `schema-artifact/04a_plan1_fixture.sql` with this header + first check:

```sql
-- Plan-1 substrate verification. Run after 01→03. search_path pinned like 04_scenarios.sql.
SET search_path = temper_next, public;
\echo '== T1: readout columns present =='
SELECT string_agg(column_name, ',' ORDER BY column_name) AS got
FROM information_schema.columns
WHERE table_schema='temper_next' AND table_name='kb_cogmap_regions'
  AND column_name IN ('telos_alignment','reference_standing','centrality','content_cohesion','internal_tension');
-- EXPECT: centrality,content_cohesion,internal_tension,reference_standing,telos_alignment
```

- [ ] **Step 2: Run it to verify it fails**

Run:
```bash
DB="postgresql://temper:temper@localhost:5437/temper_development"
psql "$DB" -q -f schema-artifact/04a_plan1_fixture.sql 2>&1 | sed -n '/T1/,/EXPECT/p'
```
Expected: `got` is empty/null (columns absent) — the verdict does **not** match EXPECT.

- [ ] **Step 3: Add the columns**

In `schema-artifact/01_schema.sql`, inside the `CREATE TABLE kb_cogmap_regions (...)` definition, change the `salience` comment and add five columns immediately after `salience`:

```sql
    salience             DOUBLE PRECISION NOT NULL,   -- computed blend, memoized (was agent-assigned; spec §3A)
    telos_alignment      DOUBLE PRECISION,            -- cosine(centroid, telos_resource.embedding)  [salience part]
    reference_standing   DOUBLE PRECISION,            -- aggregate reinforce_count over members        [salience part]
    centrality           DOUBLE PRECISION,            -- internal declared-affinity density × size      [salience part]
    content_cohesion     DOUBLE PRECISION,            -- mean member-to-centroid cosine (surface↔relational, §2c)
    internal_tension     DOUBLE PRECISION,            -- over oppositional-labeled declared edges among members
```

- [ ] **Step 4: Reload the artifact and re-run the verdict**

Run:
```bash
DB="postgresql://temper:temper@localhost:5437/temper_development"
for f in 01_schema 02_functions 03_seed; do psql "$DB" -q -v ON_ERROR_STOP=1 -f schema-artifact/$f.sql; done
psql "$DB" -q -f schema-artifact/04a_plan1_fixture.sql 2>&1 | sed -n '/T1/,/EXPECT/p'
```
Expected: `got = centrality,content_cohesion,internal_tension,reference_standing,telos_alignment`.

- [ ] **Step 5: Commit**

```bash
git add schema-artifact/01_schema.sql schema-artifact/04a_plan1_fixture.sql
git commit -m "feat(artifact): kb_cogmap_regions gains computed salience-decomposition + character readouts"
```

---

## Task 2: Introduce `kb_cogmap_lenses` (immutable) + `lens_id` on regions

**Tag:** EXTEND (spec §3B — the plurality seam; the lens is declared, stored, immutable data; `lens_id` pins reproducibility). NEW table. CONFORM to the artifact's `uuid_generate_v7()` + `asserted_by_event_id` conventions.

**Files:**
- Modify: `schema-artifact/01_schema.sql` (add table after `kb_cogmap_region_members`; add `lens_id` column to `kb_cogmap_regions`)
- Modify: `schema-artifact/03_seed.sql` (seed one telos-default lens; set the hand-seeded region's `lens_id`)
- Modify: `schema-artifact/04a_plan1_fixture.sql` (verdict)

- [ ] **Step 1: Write the failing verdict**

Append to `04a_plan1_fixture.sql`:

```sql
\echo '== T2: telos-default lens exists and the seeded region points at it =='
SELECT l.name AS lens_name, l.selection_kind,
       (r.lens_id = l.id) AS region_linked
FROM kb_cogmap_lenses l
JOIN kb_cogmap_regions r ON r.lens_id = l.id
WHERE l.name = 'telos-default';
-- EXPECT: telos-default | homed | t
```

- [ ] **Step 2: Run to verify it fails**

Run:
```bash
psql "$DB" -q -f schema-artifact/04a_plan1_fixture.sql 2>&1 | sed -n '/T2/,/EXPECT/p'
```
Expected: error `relation "kb_cogmap_lenses" does not exist` (confirmed absent in baseline).

- [ ] **Step 3a: Add the lens table + `lens_id` column**

In `01_schema.sql`, after the `kb_cogmap_region_members` block, add:

```sql
-- Region lenses (spec §3B): a lens IS a declared, stored, IMMUTABLE projection-class
-- instance. Editing = assert a new row; a region's lens_id pins the exact weight-vector
-- it was computed under (the reproducibility anchor). Plurality = more rows; same function.
CREATE TABLE kb_cogmap_lenses (
    id                   UUID PRIMARY KEY DEFAULT uuid_generate_v7(),
    cogmap_id            UUID REFERENCES kb_cogmaps(id),  -- NULL = global default; non-null = map-specific
    name                 TEXT NOT NULL,
    selection_kind       TEXT NOT NULL DEFAULT 'homed',   -- 'homed' (this plan); 'team_visible' later
    w_express            DOUBLE PRECISION NOT NULL,
    w_contains           DOUBLE PRECISION NOT NULL,
    w_leads_to           DOUBLE PRECISION NOT NULL,
    w_near               DOUBLE PRECISION NOT NULL,
    w_prop               DOUBLE PRECISION NOT NULL,
    s_telos              DOUBLE PRECISION NOT NULL,
    s_ref                DOUBLE PRECISION NOT NULL,
    s_central            DOUBLE PRECISION NOT NULL,
    resolution           DOUBLE PRECISION NOT NULL,
    asserted_by_event_id UUID NOT NULL REFERENCES kb_events(id),
    created              TIMESTAMPTZ NOT NULL DEFAULT now()
);
```

Then add `lens_id` to the `kb_cogmap_regions` table definition (after `cogmap_id`), as `NOT NULL` with the FK:

```sql
    lens_id              UUID NOT NULL REFERENCES kb_cogmap_lenses(id),  -- the perspective that produced this region (§3B)
```

> **Ordering note (GD-1, CONFORM):** `kb_cogmap_lenses` must be declared *before* `kb_cogmap_regions` for the FK to resolve at load. Confirm the current order with `grep -n 'CREATE TABLE kb_cogmap_region' schema-artifact/01_schema.sql` — if `kb_cogmap_regions` is defined earlier, move the `kb_cogmap_lenses` block above it.

- [ ] **Step 3b: Seed the telos-default lens and link the hand-seeded region**

In `03_seed.sql`, the region is asserted under event `ev_region` (`region_materialized`) at the block verified near `03_seed.sql:219-227`. Add a lens declaration + value vector **before** the `INSERT INTO kb_cogmap_regions` and set `lens_id` on that insert. Declare a `v_lens uuid;` in the `DECLARE` block, then:

```sql
    -- telos-default lens (spec §5c). Concrete starting defaults; tunable (spec OQ-2).
    INSERT INTO kb_cogmap_lenses
        (cogmap_id, name, selection_kind, w_express, w_contains, w_leads_to, w_near,
         w_prop, s_telos, s_ref, s_central, resolution, asserted_by_event_id)
    VALUES (c_onboarding, 'telos-default', 'homed', 1.0, 1.0, 0.6, 0.3,
            0.4, 0.5, 0.3, 0.2, 0.5, ev_region)
    RETURNING id INTO v_lens;
```

Then add `lens_id` (value `v_lens`) and the five new readout columns (hand-valued for now — replaced in Plan 3) to the existing `INSERT INTO kb_cogmap_regions (...) VALUES (...)`:

```sql
    INSERT INTO kb_cogmap_regions
        (cogmap_id, lens_id, centroid, salience, telos_alignment, reference_standing,
         centrality, content_cohesion, internal_tension, label, member_count,
         asserted_by_event_id, last_event_id)
    VALUES (c_onboarding, v_lens, v_centroid, 0.9, 0.9, 1.0, 0.8, 0.7, 0.0,
            'first-week confidence', 1, ev_region, ev_region)
    RETURNING id INTO ev_region_id;  -- keep whatever RETURNING target the current insert uses
```

> **GD-1:** open `03_seed.sql` around the region insert first and preserve its existing `RETURNING ... INTO ...` target and column list ordering — graft the new columns onto the real statement, do not retype it from memory.

- [ ] **Step 4: Reload + re-run the verdict**

Run:
```bash
for f in 01_schema 02_functions 03_seed; do psql "$DB" -q -v ON_ERROR_STOP=1 -f schema-artifact/$f.sql; done
psql "$DB" -q -f schema-artifact/04a_plan1_fixture.sql 2>&1 | sed -n '/T2/,/EXPECT/p'
```
Expected: `telos-default | homed | t`.

- [ ] **Step 5: Commit**

```bash
git add schema-artifact/01_schema.sql schema-artifact/03_seed.sql schema-artifact/04a_plan1_fixture.sql
git commit -m "feat(artifact): kb_cogmap_lenses (immutable) + lens_id on regions; seed telos-default lens"
```

---

## Task 3: Let edges carry facets — widen `kb_properties.owner_table`

**Tag:** AMEND `kb_properties` CHECK (spec §4a). CONFORM: widening a CHECK is additive — existing rows stay valid; verified baseline CHECK = `('kb_resources','kb_cogmaps')`.

**Files:**
- Modify: `schema-artifact/01_schema.sql` (the `kb_properties.owner_table` CHECK)
- Modify: `schema-artifact/04a_plan1_fixture.sql` (verdict)

- [ ] **Step 1: Write the failing verdict**

Append to `04a_plan1_fixture.sql`:

```sql
\echo '== T3: kb_properties accepts an edge owner =='
SELECT pg_get_constraintdef(oid) LIKE '%kb_edges%' AS edges_allowed
FROM pg_constraint
WHERE conrelid='temper_next.kb_properties'::regclass AND contype='c'
  AND pg_get_constraintdef(oid) LIKE '%owner_table%';
-- EXPECT: t
```

- [ ] **Step 2: Run to verify it fails**

Run: `psql "$DB" -q -f schema-artifact/04a_plan1_fixture.sql 2>&1 | sed -n '/T3/,/EXPECT/p'`
Expected: `edges_allowed = f`.

- [ ] **Step 3: Widen the CHECK**

In `01_schema.sql`, in the `CREATE TABLE kb_properties` block, change:
```sql
    owner_table  VARCHAR(64) NOT NULL CHECK (owner_table IN ('kb_resources', 'kb_cogmaps')),
```
to:
```sql
    owner_table  VARCHAR(64) NOT NULL CHECK (owner_table IN ('kb_resources', 'kb_cogmaps', 'kb_edges')),  -- §4a: edges carry facets
```

- [ ] **Step 4: Reload + re-run**

Run:
```bash
for f in 01_schema 02_functions 03_seed; do psql "$DB" -q -v ON_ERROR_STOP=1 -f schema-artifact/$f.sql; done
psql "$DB" -q -f schema-artifact/04a_plan1_fixture.sql 2>&1 | sed -n '/T3/,/EXPECT/p'
```
Expected: `edges_allowed = t`.

- [ ] **Step 5: Commit**

```bash
git add schema-artifact/01_schema.sql schema-artifact/04a_plan1_fixture.sql
git commit -m "feat(artifact): kb_properties.owner_table accepts kb_edges (edge facets, §4a)"
```

---

## Task 4: Readout function — `cogmap_region_content_cohesion`

**Tag:** EXTEND `02_functions.sql` (spec §2c/§6 — readouts live in SQL). NEW function. CONFORM to the file's `LANGUAGE sql STABLE` style (verified pattern at `cogmap_shape`, `02_functions.sql` ~line 316).

This is the headline readout (surface↔relational). It reads members from `kb_cogmap_region_members`, their current chunk embeddings from `kb_chunks`, pools per-concept-then-mean for the centroid, and returns mean member-to-centroid cosine. pgvector cosine **similarity** = `1 - (a <=> b)` (the `<=>` cosine-distance operator; the artifact's HNSW uses `vector_cosine_ops`, `01_schema.sql:323`).

**Files:**
- Modify: `schema-artifact/02_functions.sql` (add function near the shape-surface section ~line 311)
- Modify: `schema-artifact/04a_plan1_fixture.sql` (fixture + verdict)

- [ ] **Step 1: Write the failing verdict over a deterministic fixture**

Append to `04a_plan1_fixture.sql` — a self-contained fixture with two members whose embeddings have a *known* cohesion, then the assertion. Use simple constructed vectors so the expected value is hand-computable:

```sql
\echo '== T4: content_cohesion = mean member-to-centroid cosine =='
DO $fx$
DECLARE
  r_a uuid; r_b uuid; b_a uuid; b_b uuid; reg uuid;
  ev uuid; et uuid; ent uuid;
  -- two unit-ish vectors: e1=[1,0,0,...], e2=[0,1,0,...] in 768-dim
  v1 vector := ('[1,' || array_to_string(array_fill(0::float8, ARRAY[767]), ',') || ']')::vector;
  v2 vector := ('[0,1,' || array_to_string(array_fill(0::float8, ARRAY[766]), ',') || ']')::vector;
BEGIN
  SELECT id INTO et FROM kb_event_types WHERE name='region_materialized';
  SELECT emitter_entity_id INTO ent FROM kb_events LIMIT 1;   -- reuse any seeded entity
  INSERT INTO kb_events (event_type_id, emitter_entity_id, producing_anchor_table, producing_anchor_id)
    VALUES (et, ent, 'kb_cogmaps', (SELECT id FROM kb_cogmaps LIMIT 1)) RETURNING id INTO ev;
  INSERT INTO kb_resources (title, origin_uri) VALUES ('fx: A','temper://fx/a') RETURNING id INTO r_a;
  INSERT INTO kb_resources (title, origin_uri) VALUES ('fx: B','temper://fx/b') RETURNING id INTO r_b;
  INSERT INTO kb_content_blocks (resource_id, seq, genesis_event_id, last_event_id)
    VALUES (r_a,0,ev,ev) RETURNING id INTO b_a;
  INSERT INTO kb_content_blocks (resource_id, seq, genesis_event_id, last_event_id)
    VALUES (r_b,0,ev,ev) RETURNING id INTO b_b;
  INSERT INTO kb_chunks (block_id, resource_id, chunk_index, content_hash, embedding, is_current)
    VALUES (b_a, r_a, 0, 'h-a', v1, true);
  INSERT INTO kb_chunks (block_id, resource_id, chunk_index, content_hash, embedding, is_current)
    VALUES (b_b, r_b, 0, 'h-b', v2, true);
  INSERT INTO kb_cogmap_regions
    (cogmap_id, lens_id, centroid, salience, label, member_count, asserted_by_event_id, last_event_id)
    VALUES ((SELECT id FROM kb_cogmaps LIMIT 1), (SELECT id FROM kb_cogmap_lenses LIMIT 1),
            v1, 0.0, 'fx', 2, ev, ev) RETURNING id INTO reg;
  INSERT INTO kb_cogmap_region_members (region_id, member_table, member_id)
    VALUES (reg,'kb_resources',r_a),(reg,'kb_resources',r_b);
  -- centroid of e1,e2 = [0.5,0.5,0,...]; cos(e1,centroid)=cos(e2,centroid)=0.7071; mean=0.7071
  RAISE NOTICE 'content_cohesion=% (EXPECT ~0.7071)', round(cogmap_region_content_cohesion(reg)::numeric, 4);
END $fx$;
```

- [ ] **Step 2: Run to verify it fails**

Run: `psql "$DB" -q -f schema-artifact/04a_plan1_fixture.sql 2>&1 | grep -iE 'content_cohesion|does not exist'`
Expected: `function cogmap_region_content_cohesion(uuid) does not exist`.

- [ ] **Step 3: Implement the function**

In `02_functions.sql`, add (near the `COGMAP SHAPE SURFACE` section, ~line 311):

```sql
-- Content cohesion (spec §2c): mean member-to-centroid cosine. A DOWNSTREAM readout over a
-- formed region (cosine never enters FORMATION — that is Plan 2's declared-only affinity).
-- Per-concept pooling: each member resource's current chunk embeddings are mean-pooled to one
-- vector first (pool-per-concept-then-mean, map-regions OQ-1); the region centroid is the mean
-- of those; cohesion is the mean cosine of each member-vector to the centroid.
CREATE FUNCTION cogmap_region_content_cohesion(p_region uuid)
RETURNS double precision LANGUAGE sql STABLE AS $$
    WITH member_vec AS (   -- one pooled vector per member resource
        SELECT m.member_id, avg(ch.embedding) AS v
        FROM kb_cogmap_region_members m
        JOIN kb_chunks ch ON ch.resource_id = m.member_id AND ch.is_current
        WHERE m.region_id = p_region AND m.member_table = 'kb_resources'
        GROUP BY m.member_id
    ),
    ctr AS (SELECT avg(v) AS c FROM member_vec)
    SELECT avg(1 - (mv.v <=> ctr.c)) FROM member_vec mv, ctr;
$$;
```

- [ ] **Step 4: Reload + re-run**

Run:
```bash
for f in 01_schema 02_functions 03_seed; do psql "$DB" -q -v ON_ERROR_STOP=1 -f schema-artifact/$f.sql; done
psql "$DB" -q -f schema-artifact/04a_plan1_fixture.sql 2>&1 | grep -i content_cohesion
```
Expected: `content_cohesion=0.7071 (EXPECT ~0.7071)`.

- [ ] **Step 5: Commit**

```bash
git add schema-artifact/02_functions.sql schema-artifact/04a_plan1_fixture.sql
git commit -m "feat(artifact): cogmap_region_content_cohesion readout (mean member-to-centroid cosine)"
```

---

## Task 5: Readout function — `cogmap_region_telos_alignment`

**Tag:** EXTEND `02_functions.sql` (spec §2c — the first salience component). NEW function. CONFORM to `LANGUAGE sql STABLE`.

`telos_alignment` = `cosine(region.centroid, telos_resource.embedding)`. The telos resource is `kb_cogmaps.telos_resource_id` (verified `01_schema.sql:180`); its embedding is the pooled mean of its current chunks.

**Files:**
- Modify: `schema-artifact/02_functions.sql`
- Modify: `schema-artifact/04a_plan1_fixture.sql`

- [ ] **Step 1: Write the failing verdict**

Append to `04a_plan1_fixture.sql` — assert the function exists and returns a value in `[-1,1]` for the real seeded region (the onboarding region from `03_seed.sql`). (Exact value depends on seeded telos chunks; bound-check is the deterministic assertion here.)

```sql
\echo '== T5: telos_alignment is a valid cosine for the seeded region =='
SELECT (a IS NOT NULL AND a BETWEEN -1.0 AND 1.0) AS ok, round(a::numeric,4) AS telos_alignment
FROM (
  SELECT cogmap_region_telos_alignment(r.id, r.cogmap_id) AS a
  FROM kb_cogmap_regions r
  JOIN kb_cogmaps c ON c.id = r.cogmap_id
  WHERE c.name = 'onboarding-cogmap'   -- the cogmap_genesis-seeded map (03_seed.sql)
  LIMIT 1
) t;
-- EXPECT: ok = t   (note: NULL is acceptable IF the seeded telos has no current chunks; see Step 3 guard)
```

> **GD-1:** confirm the seeded cogmap's name with `psql "$DB" -tAc "SELECT name FROM temper_next.kb_cogmaps;"` and use the real name — do not assume `'onboarding-cogmap'` if the seed differs.

- [ ] **Step 2: Run to verify it fails**

Run: `psql "$DB" -q -f schema-artifact/04a_plan1_fixture.sql 2>&1 | grep -iE 'telos_alignment|does not exist'`
Expected: `function cogmap_region_telos_alignment(uuid, uuid) does not exist`.

- [ ] **Step 3: Implement**

In `02_functions.sql`:

```sql
-- Telos alignment (spec §2c, salience part): cosine of the region centroid to the cogmap's
-- telos-resource embedding (kb_cogmaps.telos_resource_id). "Importance under the map's telos,"
-- literal because the telos IS a resource with chunks. NULL iff the telos has no current chunks.
CREATE FUNCTION cogmap_region_telos_alignment(p_region uuid, p_cogmap uuid)
RETURNS double precision LANGUAGE sql STABLE AS $$
    WITH telos AS (
        SELECT avg(ch.embedding) AS v
        FROM kb_cogmaps c
        JOIN kb_chunks ch ON ch.resource_id = c.telos_resource_id AND ch.is_current
        WHERE c.id = p_cogmap
    ),
    reg AS (SELECT centroid AS v FROM kb_cogmap_regions WHERE id = p_region)
    SELECT 1 - (reg.v <=> telos.v) FROM reg, telos WHERE telos.v IS NOT NULL;
$$;
```

- [ ] **Step 4: Reload + re-run**

Run:
```bash
for f in 01_schema 02_functions 03_seed; do psql "$DB" -q -v ON_ERROR_STOP=1 -f schema-artifact/$f.sql; done
psql "$DB" -q -f schema-artifact/04a_plan1_fixture.sql 2>&1 | sed -n '/T5/,/EXPECT/p'
```
Expected: `ok = t` (or NULL with the documented guard if the seeded telos has no chunks — note which, per GD-1).

- [ ] **Step 5: Commit**

```bash
git add schema-artifact/02_functions.sql schema-artifact/04a_plan1_fixture.sql
git commit -m "feat(artifact): cogmap_region_telos_alignment readout (centroid↔telos cosine)"
```

---

## Task 6: Readout functions — `reference_standing`, `centrality`, `internal_tension`

**Tag:** EXTEND `02_functions.sql` (spec §2c — the remaining salience parts + the tension character readout). NEW functions. CONFORM to `LANGUAGE sql STABLE`.

Grounded shapes:
- **`reference_standing`** = count of `kb_block_provenance` accretions over the member resources' blocks (verified table at `01_schema.sql`; page-04 "reinforce_count is a `count()` over `kb_block_provenance`, never stored"). Exclude `is_corrected`.
- **`centrality`** = declared-affinity density × size among members: sum of declared `kb_edges` weights *internal* to the member set, times member count. (Edge weight/kind verified on `kb_edges`, `01_schema.sql:377,389`.) Lens-weighting of edge kinds is applied by Plan 2 at materialization; this readout reports the raw internal declared mass so the harness can scale it.
- **`internal_tension`** = count/weight of declared edges among members whose `label` or attached `{stance:opposed}` facet marks opposition. Per spec §2a the *literal* `contradicts` is a convention, not a reserved word — so this readout matches a **caller-supplied** label set, defaulting to `'contradicts'`, never hardcoding semantics into formation.

**Files:**
- Modify: `schema-artifact/02_functions.sql`
- Modify: `schema-artifact/04a_plan1_fixture.sql`

- [ ] **Step 1: Write the failing verdicts**

Append to `04a_plan1_fixture.sql`, extending the T4 fixture region `reg` with a declared edge between its two members (so centrality/tension have something to read):

```sql
\echo '== T6: reference_standing / centrality / internal_tension exist and compute =='
DO $fx6$
DECLARE r_a uuid; r_b uuid; reg uuid; ev uuid;
BEGIN
  SELECT id INTO r_a FROM kb_resources WHERE origin_uri='temper://fx/a';
  SELECT id INTO r_b FROM kb_resources WHERE origin_uri='temper://fx/b';
  SELECT id INTO reg FROM kb_cogmap_regions WHERE label='fx';
  SELECT id INTO ev FROM kb_events ORDER BY occurred_at DESC LIMIT 1;
  -- a declared leads_to edge A->B, weight 0.8, homed in the fixture cogmap
  INSERT INTO kb_edges (source_table, source_id, target_table, target_id, edge_kind, label, weight,
                        home_anchor_table, home_anchor_id, asserted_by_event_id, last_event_id)
    VALUES ('kb_resources', r_a, 'kb_resources', r_b, 'leads_to', 'depends_on', 0.8,
            'kb_cogmaps', (SELECT id FROM kb_cogmaps LIMIT 1), ev, ev);
  RAISE NOTICE 'reference_standing=% centrality=% tension=%',
    cogmap_region_reference_standing(reg),
    round(cogmap_region_centrality(reg)::numeric,4),
    cogmap_region_internal_tension(reg, ARRAY['contradicts']);
END $fx6$;
-- EXPECT: reference_standing=0 centrality=1.6000 tension=0   (2 members × 0.8 internal weight; no opposed edge)
```

> **GD-1:** confirm the real `kb_edges` column list with `awk '/CREATE TABLE kb_edges/,/\\);/' schema-artifact/01_schema.sql` before writing the INSERT — graft onto the real columns (the access-spec edge-home is `home_anchor_table/home_anchor_id`, verified `01_schema.sql:365` note).

- [ ] **Step 2: Run to verify it fails**

Run: `psql "$DB" -q -f schema-artifact/04a_plan1_fixture.sql 2>&1 | grep -iE 'reference_standing|does not exist'`
Expected: `function cogmap_region_reference_standing(uuid) does not exist`.

- [ ] **Step 3: Implement the three functions**

In `02_functions.sql`:

```sql
-- Reference standing (spec §2c): summed reinforce_count over the member resources' blocks
-- (a count() over kb_block_provenance, page-04 — derived, never stored). is_corrected excluded.
CREATE FUNCTION cogmap_region_reference_standing(p_region uuid)
RETURNS double precision LANGUAGE sql STABLE AS $$
    SELECT coalesce(count(p.*), 0)::double precision
    FROM kb_cogmap_region_members m
    JOIN kb_content_blocks b ON b.resource_id = m.member_id AND NOT b.is_folded
    JOIN kb_block_provenance p ON p.block_id = b.id AND NOT p.is_corrected
    WHERE m.region_id = p_region AND m.member_table = 'kb_resources';
$$;

-- Centrality (spec §2c): internal declared-affinity mass × size. Sum of declared edge weights
-- BOTH of whose endpoints are members of the region, times member_count. Raw (un-lens-weighted);
-- Plan 2 scales by the lens at materialization. Cosine never enters.
CREATE FUNCTION cogmap_region_centrality(p_region uuid)
RETURNS double precision LANGUAGE sql STABLE AS $$
    WITH mem AS (
        SELECT member_id FROM kb_cogmap_region_members
        WHERE region_id = p_region AND member_table = 'kb_resources'
    ),
    internal AS (
        SELECT coalesce(sum(e.weight), 0) AS mass
        FROM kb_edges e
        WHERE e.source_table='kb_resources' AND e.target_table='kb_resources'
          AND e.source_id IN (SELECT member_id FROM mem)
          AND e.target_id IN (SELECT member_id FROM mem)
          AND NOT e.is_folded
    )
    SELECT internal.mass * (SELECT count(*) FROM mem) FROM internal;
$$;

-- Internal tension (spec §2a/§2c): declared opposition among members — a FEATURE of the region,
-- never a fracture. Matches a caller-supplied label set (default {'contradicts'}); semantics are
-- NOT reserved at the kernel — the caller (lens) decides what counts as opposed.
CREATE FUNCTION cogmap_region_internal_tension(p_region uuid, p_opposed_labels text[])
RETURNS double precision LANGUAGE sql STABLE AS $$
    WITH mem AS (
        SELECT member_id FROM kb_cogmap_region_members
        WHERE region_id = p_region AND member_table = 'kb_resources'
    )
    SELECT coalesce(sum(e.weight), 0)::double precision
    FROM kb_edges e
    WHERE e.source_table='kb_resources' AND e.target_table='kb_resources'
      AND e.source_id IN (SELECT member_id FROM mem)
      AND e.target_id IN (SELECT member_id FROM mem)
      AND NOT e.is_folded
      AND e.label = ANY(p_opposed_labels);
$$;
```

- [ ] **Step 4: Reload + re-run**

Run:
```bash
for f in 01_schema 02_functions 03_seed; do psql "$DB" -q -v ON_ERROR_STOP=1 -f schema-artifact/$f.sql; done
psql "$DB" -q -f schema-artifact/04a_plan1_fixture.sql 2>&1 | grep -iE 'reference_standing='
```
Expected: `reference_standing=0 centrality=1.6000 tension=0`.

- [ ] **Step 5: Commit**

```bash
git add schema-artifact/02_functions.sql schema-artifact/04a_plan1_fixture.sql
git commit -m "feat(artifact): reference_standing / centrality / internal_tension readouts"
```

---

## Task 7: Extend `cogmap_shape` to surface the new signals + a lens selector

**Tag:** AMEND `cogmap_shape` (spec §3D). CONFORM to the existing principal gate — do **not** change the access logic, only the projection list + an optional lens filter. Verified current signature returns `(region_id, salience, label, member_count)` (`02_functions.sql` ~line 316).

**Files:**
- Modify: `schema-artifact/02_functions.sql` (the `cogmap_shape` function)
- Modify: `schema-artifact/04a_plan1_fixture.sql`

- [ ] **Step 1: Write the failing verdict**

Append to `04a_plan1_fixture.sql`:

```sql
\echo '== T7: cogmap_shape returns lens_id + content_cohesion =='
SELECT count(*) FILTER (WHERE lens_id IS NOT NULL) AS rows_with_lens,
       bool_and(content_cohesion IS NOT NULL OR member_count >= 0) AS shape_ok
FROM cogmap_shape(
       (SELECT id FROM kb_cogmaps WHERE name='onboarding-cogmap'),
       'cogmap',
       (SELECT id FROM kb_cogmaps WHERE name='onboarding-cogmap'));
-- EXPECT: rows_with_lens >= 1, shape_ok = t
```

- [ ] **Step 2: Run to verify it fails**

Run: `psql "$DB" -q -f schema-artifact/04a_plan1_fixture.sql 2>&1 | sed -n '/T7/,/EXPECT/p'`
Expected: error — `column "lens_id" does not exist` (the current `RETURNS TABLE` lacks it).

- [ ] **Step 3: Amend the function**

In `02_functions.sql`, replace the current `cogmap_shape` definition (verified shape above) with the extended one — same access gate, new projection + optional `p_lens`:

```sql
CREATE OR REPLACE FUNCTION cogmap_shape(
    p_cogmap uuid, p_principal_kind text, p_principal_id uuid, p_lens uuid DEFAULT NULL)
RETURNS TABLE(region_id uuid, lens_id uuid, salience double precision,
              content_cohesion double precision, label text, member_count int)
LANGUAGE sql STABLE AS $$
    SELECT reg.id, reg.lens_id, reg.salience, reg.content_cohesion, reg.label, reg.member_count
    FROM kb_cogmap_regions reg
    WHERE reg.cogmap_id = p_cogmap
      AND NOT reg.is_folded
      AND (p_lens IS NULL OR reg.lens_id = p_lens)   -- default = all lenses; Plan 3 may default to telos-default
      AND (
        (p_principal_kind = 'profile' AND cogmap_readable_by_profile(p_principal_id, p_cogmap))
        OR (p_principal_kind = 'cogmap' AND p_principal_id = p_cogmap)
      );
$$;
```

> **GD-3 (CONFORM):** the two-branch principal gate is load-bearing access logic — copy it verbatim from the current function (`cogmap_readable_by_profile` / `p_principal_id = p_cogmap`); do not re-derive it.

- [ ] **Step 4: Reload + re-run**

Run:
```bash
for f in 01_schema 02_functions 03_seed; do psql "$DB" -q -v ON_ERROR_STOP=1 -f schema-artifact/$f.sql; done
psql "$DB" -q -f schema-artifact/04a_plan1_fixture.sql 2>&1 | sed -n '/T7/,/EXPECT/p'
```
Expected: `rows_with_lens >= 1`, `shape_ok = t`.

- [ ] **Step 5: Full re-run of the existing scenario suite (regression guard)**

The original `04_scenarios.sql` S6 still references `cogmap_shape`; confirm nothing regressed:

```bash
psql "$DB" -q -f schema-artifact/04_scenarios.sql 2>&1 | grep -iE 'error|fatal' || echo "04_scenarios: clean"
```
Expected: `04_scenarios: clean` (S6's surface read still works against the amended function — extra return columns don't break the existing `SELECT`s; if any S6 query used `SELECT *` positionally, update it and note the gap).

- [ ] **Step 6: Commit**

```bash
git add schema-artifact/02_functions.sql schema-artifact/04a_plan1_fixture.sql
git commit -m "feat(artifact): cogmap_shape surfaces lens_id + content_cohesion, optional lens filter (§3D)"
```

---

## Self-Review (run before handoff)

**1. Spec coverage (§3 / §4 / §2c / §6 readouts):**
- §3A salience computed + decomposed → T1 ✓ · §3B lens table + lens_id → T2 ✓ · §3C boundary_kind dropped → no-op confirmed (baseline has none; not introduced) ✓ · §3D surface read extension → T7 ✓ · §4a owner_table += kb_edges → T3 ✓ · §2c readouts (centroid pooling, content_cohesion, telos_alignment, reference_standing, centrality, internal_tension) → T4/T5/T6 ✓.
- §2b clustering, §5 full falsification seed/suite, §6 harness → **Plans 2 & 3** (out of scope here, by design).
- §3E amend-and-scar of map-regions §5 is a *spec-doc* change already landed in the spec; no artifact task.

**2. Placeholder scan:** every step has runnable SQL + exact commands + expected output. The lens default vector (T2) and `resolution` are concrete tunable values (spec OQ-2 flags them plan-level), not placeholders.

**3. Type/name consistency:** function names used across tasks — `cogmap_region_content_cohesion`, `cogmap_region_telos_alignment(p_region, p_cogmap)`, `cogmap_region_reference_standing`, `cogmap_region_centrality`, `cogmap_region_internal_tension(p_region, p_opposed_labels)`, `cogmap_shape(...,p_lens)` — are consistent within the plan and match the spec §2c/§3D names. Column names match T1.

**4. Grounding (GD):** every task carries a CONFORM/EXTEND/AMEND tag; every disk claim cites a verified line or a `psql` baseline check; the riskiest re-typings (seed region insert T2, kb_edges INSERT T6, principal gate T7) carry explicit GD-1 "open the file first, graft onto the real statement" guards.

---

## Execution Handoff

**Plan 1 complete and saved to `docs/superpowers/plans/2026-06-06-emergent-region-projection-substrate.md`.** It produces working, testable software on its own: the artifact loads clean and every readout returns a verified value over its fixture. Plans 2 (`temper-next` clustering harness) and 3 (enriched seed + S6a–h falsification suite) build on it and will be written as their own artifacts.

Two execution options:
1. **Subagent-Driven (recommended)** — fresh subagent per task, two-stage review between tasks (the grounding discipline is in the header; the controller enforces GD-1/GD-2 on each returned task).
2. **Inline Execution** — execute tasks in this session with checkpoints.

Which approach — and do you want me to write Plans 2 & 3 now, or execute Plan 1 first?
