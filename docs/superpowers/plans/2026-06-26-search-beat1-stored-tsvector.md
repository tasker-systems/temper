# Search Beat 1 — Stored tsvector + GIN index Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the inline, unindexed query-time FTS build with a stored, GIN-indexed, projection-maintained tsvector — behavior-preserving (same result set/order).

**Architecture:** A new `kb_resource_search_index` table holds a stored `tsvector` (title@A + body@B), maintained by a `_rebuild_resource_search_vector` helper called from the canonical event-sourced projection functions (`_project_blocks`, `_project_block_mutated`, `_project_resource_updated`) — NOT triggers. A backfill populates existing rows in the same migration. Then `temper_substrate::readback::fts_search` is swapped to read the stored index instead of building the vector inline.

**Tech Stack:** PostgreSQL 17/18 + pgvector, sqlx migrations, Rust (`temper-substrate`), `#[sqlx::test(migrator="temper_substrate::MIGRATOR")]` ephemeral-DB artifact tests (feature `artifact-tests`).

## Global Constraints

- **Additive-only on `main`:** new table/index/function + `CREATE OR REPLACE` + insert-only backfill. No destructive DDL.
- **Never edit `migrations/20260624000002_canonical_functions.sql` in place** — the shipped canonical functions are immutable; recreate the three projection functions via `CREATE OR REPLACE` in the NEW migration.
- **Behavior-preserving:** the matched result SET (and order for title/body terms) must be identical before/after the read swap. The FTS recipe stays exactly `setweight(to_tsvector('english', title),'A') || setweight(to_tsvector('english', body),'B')`, body = `string_agg(current-chunk content, ' ')`, config `'english'`.
- **Readback module convention:** `readback::fts_search` uses runtime `sqlx::query` (NEVER the `query!` macros) — see the module-level note in `crates/temper-substrate/src/readback/mod.rs`.
- **Test queries:** use runtime `sqlx::query`/`query_scalar` for fixture/index lookups (trivial test-fixture lookups are allowed runtime per CLAUDE.md), so no `sqlx::query!` cache churn this beat.
- **Migration filename:** `migrations/20260626000001_fts_search_index.sql` (next after `20260625000001`).

---

### Task 1: Migration — stored index table, maintenance helper, projection wiring, backfill

**Files:**
- Create: `migrations/20260626000001_fts_search_index.sql`
- Test: `crates/temper-substrate/tests/search_index.rs` (new)

**Interfaces:**
- Produces (SQL, callable from later tasks/tests):
  - Table `kb_resource_search_index(resource_id uuid PK, search_vector tsvector, search_config varchar(64), updated timestamptz)`.
  - Index `idx_resource_search_vector` — `GIN (search_vector) WITH (fastupdate=off)`.
  - Function `_rebuild_resource_search_vector(p_resource uuid) RETURNS void`.
  - `_project_blocks`, `_project_block_mutated`, `_project_resource_updated` recreated, each now calling the helper.
- Consumes: existing `kb_resources`, `kb_chunks`, `kb_chunk_content`, `_recompute_resource_body_hash`, `temper_substrate::MIGRATOR` (auto-discovers the new file).

- [ ] **Step 1: Write the failing maintenance + backfill tests**

Create `crates/temper-substrate/tests/search_index.rs`:

```rust
#![cfg(feature = "artifact-tests")]
//! Search Beat 1 — the stored `kb_resource_search_index` is populated and maintained by the
//! event-sourced projection functions (create / block-edit / title-only update), and backfilled.
//! Isolated ephemeral DB via `MIGRATOR`.

mod common;

use temper_substrate::scenario::bootseed;
use temper_substrate::{readback, writes};
use uuid::Uuid;

/// The boot-seeded canonical `system` profile + entity.
async fn system_actor(pool: &sqlx::PgPool) -> (temper_substrate::ids::ProfileId, temper_substrate::ids::EntityId) {
    let profile: Uuid = sqlx::query_scalar("SELECT id FROM kb_profiles WHERE handle='system'")
        .fetch_one(pool).await.unwrap();
    let entity: Uuid = sqlx::query_scalar("SELECT id FROM kb_entities WHERE profile_id=$1 AND name='system'")
        .bind(profile).fetch_one(pool).await.unwrap();
    (temper_substrate::ids::ProfileId::from(profile), temper_substrate::ids::EntityId::from(entity))
}

async fn ctx(pool: &sqlx::PgPool, owner: temper_substrate::ids::ProfileId, slug: &str) -> temper_substrate::ids::ContextId {
    temper_substrate::ids::ContextId::from(
        common::insert_context(pool, "kb_profiles", owner.uuid(), slug, slug).await.unwrap())
}

/// Does the stored vector match a query term? (`@@ plainto_tsquery`).
async fn index_matches(pool: &sqlx::PgPool, resource: Uuid, term: &str) -> bool {
    sqlx::query_scalar::<_, bool>(
        "SELECT COALESCE((SELECT search_vector @@ plainto_tsquery('english', $2)
           FROM kb_resource_search_index WHERE resource_id = $1), false)")
        .bind(resource).bind(term).fetch_one(pool).await.unwrap()
}

#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn create_populates_index_with_title_and_body(pool: sqlx::PgPool) {
    bootseed::seed_system(&pool).await.unwrap();
    let (owner, emitter) = system_actor(&pool).await;
    let home = ctx(&pool, owner, "idx").await;
    let r = writes::create_resource(&pool, writes::CreateParams {
        title: "Salamander architecture",
        origin_uri: "temper://idx/r",
        body: "the quenching pipeline tempers steel",
        doc_type: "concept",
        home, owner, originator: owner, emitter,
        properties: &[], chunks: None,
    }).await.unwrap();

    assert!(index_matches(&pool, r.uuid(), "salamander").await, "title term indexed (weight A)");
    assert!(index_matches(&pool, r.uuid(), "quenching").await, "body term indexed (weight B)");
    assert!(!index_matches(&pool, r.uuid(), "unrelated").await, "non-present term does not match");
}

#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn body_edit_updates_index(pool: sqlx::PgPool) {
    bootseed::seed_system(&pool).await.unwrap();
    let (owner, emitter) = system_actor(&pool).await;
    let home = ctx(&pool, owner, "idx").await;
    let r = writes::create_resource(&pool, writes::CreateParams {
        title: "Doc", origin_uri: "temper://idx/r", body: "original lexeme here",
        doc_type: "concept", home, owner, originator: owner, emitter,
        properties: &[], chunks: None,
    }).await.unwrap();
    assert!(index_matches(&pool, r.uuid(), "original").await, "pre-edit body term present");

    writes::update_resource(&pool, writes::UpdateParams {
        resource: r, body: Some("revised distinctive wording"), title: None,
        origin_uri: None, properties: &[], chunks: None, rehome_to: None, emitter,
    }).await.unwrap();

    assert!(index_matches(&pool, r.uuid(), "distinctive").await, "new body term indexed after edit");
    assert!(!index_matches(&pool, r.uuid(), "original").await, "superseded body term gone from index");
}

#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn title_only_update_updates_index(pool: sqlx::PgPool) {
    bootseed::seed_system(&pool).await.unwrap();
    let (owner, emitter) = system_actor(&pool).await;
    let home = ctx(&pool, owner, "idx").await;
    let r = writes::create_resource(&pool, writes::CreateParams {
        title: "Aardvark", origin_uri: "temper://idx/r", body: "stable body",
        doc_type: "concept", home, owner, originator: owner, emitter,
        properties: &[], chunks: None,
    }).await.unwrap();

    writes::update_resource(&pool, writes::UpdateParams {
        resource: r, body: None, title: Some("Pangolin"),
        origin_uri: None, properties: &[], chunks: None, rehome_to: None, emitter,
    }).await.unwrap();

    assert!(index_matches(&pool, r.uuid(), "pangolin").await, "new title term indexed (title-only update)");
    assert!(!index_matches(&pool, r.uuid(), "aardvark").await, "old title term gone");
    assert!(index_matches(&pool, r.uuid(), "stable").await, "body unchanged still indexed");
}

#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn backfill_covers_preexisting_rows(pool: sqlx::PgPool) {
    // The migration's backfill is upsert + idempotent; re-running it must (a) cover every active
    // resource and (b) leave already-maintained rows correct. We assert coverage: every active
    // resource has an index row, and a known term matches.
    bootseed::seed_system(&pool).await.unwrap();
    let (owner, emitter) = system_actor(&pool).await;
    let home = ctx(&pool, owner, "idx").await;
    let r = writes::create_resource(&pool, writes::CreateParams {
        title: "Backfillable", origin_uri: "temper://idx/r", body: "corpus content word",
        doc_type: "concept", home, owner, originator: owner, emitter,
        properties: &[], chunks: None,
    }).await.unwrap();

    let missing: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM kb_resources r
          WHERE r.is_active
            AND NOT EXISTS (SELECT 1 FROM kb_resource_search_index si WHERE si.resource_id = r.id)")
        .fetch_one(&pool).await.unwrap();
    assert_eq!(missing, 0, "every active resource has an index row");
    assert!(index_matches(&pool, r.uuid(), "corpus").await, "backfilled/maintained term matches");
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo make test-artifacts 2>&1 | tail -40` (or `DATABASE_URL=postgresql://temper:temper@localhost:5437/temper_development cargo nextest run -p temper-substrate --features artifact-tests --test search_index`)
Expected: FAIL — `relation "kb_resource_search_index" does not exist` (table/migration not yet created).

- [ ] **Step 3: Write the migration**

Create `migrations/20260626000001_fts_search_index.sql`. (The three `CREATE OR REPLACE` bodies are the current bodies verbatim with one added `PERFORM` line each — do not alter anything else.)

```sql
-- =============================================================================
-- Search Beat 1: stored full-text index (kb_resource_search_index) + GIN.
-- Maintained by the canonical _project_* projection functions (NOT triggers).
-- Behavior-preserving: title@A + body@B, body = string_agg current-chunk content,
-- config 'english' — the exact recipe readback::fts_search built inline.
-- Additive-only-on-main: new table/index/function + CREATE OR REPLACE + backfill.
-- =============================================================================

CREATE TABLE kb_resource_search_index (
    resource_id    UUID PRIMARY KEY REFERENCES kb_resources(id) ON DELETE CASCADE,
    search_vector  tsvector NOT NULL,
    search_config  VARCHAR(64) NOT NULL DEFAULT 'english',
    updated        TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX idx_resource_search_vector
    ON kb_resource_search_index USING GIN (search_vector) WITH (fastupdate = off);

-- Rebuild a resource's stored vector: title@A + body@B. Idempotent upsert.
CREATE FUNCTION _rebuild_resource_search_vector(p_resource uuid)
RETURNS void LANGUAGE plpgsql AS $$
DECLARE v_config varchar(64); v_title text; v_body text;
BEGIN
    SELECT COALESCE((SELECT search_config FROM kb_resource_search_index WHERE resource_id = p_resource),
                    'english') INTO v_config;
    SELECT title INTO v_title FROM kb_resources WHERE id = p_resource;
    IF v_title IS NULL THEN RETURN; END IF;
    SELECT COALESCE(string_agg(cc.content, ' '), '')
      INTO v_body
      FROM kb_chunks c JOIN kb_chunk_content cc ON cc.chunk_id = c.id
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

-- ── _project_blocks: + rebuild after body-hash recompute ─────────────────────
CREATE OR REPLACE FUNCTION _project_blocks(p_resource uuid, p_event uuid, p_manifests jsonb, p_content jsonb)
RETURNS void LANGUAGE plpgsql AS $$
DECLARE
    v_block uuid; v_chunk uuid;
    v_block_json jsonb; v_chunk_json jsonb; v_side jsonb;
    v_block_hash text; v_chunk_hashes text; v_chunk_count int;
    v_occurred timestamptz := (SELECT occurred_at FROM kb_events WHERE id = p_event);
BEGIN
    FOR v_block_json IN SELECT jsonb_array_elements(p_manifests) LOOP
        v_block := (v_block_json->>'block_id')::uuid;
        INSERT INTO kb_content_blocks (id, resource_id, seq, genesis_event_id, last_event_id, created)
            VALUES (v_block, p_resource, (v_block_json->>'seq')::int, p_event, p_event, v_occurred);
        IF v_block_json ? 'role' AND jsonb_typeof(v_block_json->'role') = 'string' THEN
            INSERT INTO kb_properties (owner_table, owner_id, property_key, property_value,
                                       asserted_by_event_id, last_event_id, created)
            VALUES ('kb_content_blocks', v_block, 'block_role', v_block_json->'role',
                    p_event, p_event, v_occurred);
        END IF;
        v_chunk_hashes := '';
        v_chunk_count := 0;
        FOR v_chunk_json IN SELECT jsonb_array_elements(v_block_json->'chunks') LOOP
            v_chunk := (v_chunk_json->>'chunk_id')::uuid;
            v_side := p_content->(v_chunk_json->>'chunk_id');
            IF v_side IS NULL THEN
                RAISE EXCEPTION '_project_blocks: content sidecar missing chunk %', v_chunk;
            END IF;
            PERFORM _insert_chunk(v_chunk, v_block, p_resource, (v_chunk_json->>'chunk_index')::int,
                                  1, v_chunk_json->>'content_hash', v_side->'embedding', true,
                                  v_side->>'content', v_side->>'header_path',
                                  NULLIF(v_side->>'heading_depth','')::smallint, v_occurred);
            v_chunk_hashes := v_chunk_hashes || (v_chunk_json->>'content_hash');
            v_chunk_count := v_chunk_count + 1;
        END LOOP;
        v_block_hash := encode(sha256(convert_to(v_chunk_hashes, 'UTF8')), 'hex');
        INSERT INTO kb_block_revisions (block_id, block_body_hash, chunk_count, created)
            VALUES (v_block, v_block_hash, v_chunk_count, v_occurred);
    END LOOP;
    PERFORM _recompute_resource_body_hash(p_resource, v_occurred);
    PERFORM _rebuild_resource_search_vector(p_resource);   -- ← Beat 1
END;
$$;

-- ── _project_block_mutated: + rebuild after body-hash recompute ──────────────
CREATE OR REPLACE FUNCTION _project_block_mutated(p_event uuid, p_payload jsonb, p_content jsonb)
RETURNS uuid LANGUAGE plpgsql AS $$
DECLARE v_occurred timestamptz := (SELECT occurred_at FROM kb_events WHERE id = p_event);
        v_block    uuid := (p_payload->>'block_id')::uuid;
        v_resource uuid;
        v_next_ver int;
        v_chunk_json jsonb; v_chunk uuid; v_side jsonb;
        v_chunk_hashes text := ''; v_chunk_count int := 0; v_block_hash text;
BEGIN
    SELECT resource_id INTO v_resource FROM kb_content_blocks WHERE id = v_block;
    IF v_resource IS NULL THEN
        RAISE EXCEPTION '_project_block_mutated: block % not found', v_block;
    END IF;
    UPDATE kb_chunks SET is_current = false WHERE block_id = v_block AND is_current;
    SELECT coalesce(max(version), 0) + 1 INTO v_next_ver FROM kb_chunks WHERE block_id = v_block;
    FOR v_chunk_json IN SELECT jsonb_array_elements(p_payload->'chunks') LOOP
        v_chunk := (v_chunk_json->>'chunk_id')::uuid;
        v_side  := p_content->(v_chunk_json->>'chunk_id');
        IF v_side IS NULL THEN
            RAISE EXCEPTION '_project_block_mutated: content sidecar missing chunk %', v_chunk;
        END IF;
        PERFORM _insert_chunk(v_chunk, v_block, v_resource, (v_chunk_json->>'chunk_index')::int,
                              v_next_ver, v_chunk_json->>'content_hash', v_side->'embedding', true,
                              v_side->>'content', v_side->>'header_path',
                              NULLIF(v_side->>'heading_depth','')::smallint, v_occurred);
        v_chunk_hashes := v_chunk_hashes || (v_chunk_json->>'content_hash');
        v_chunk_count := v_chunk_count + 1;
    END LOOP;
    v_block_hash := encode(sha256(convert_to(v_chunk_hashes, 'UTF8')), 'hex');
    INSERT INTO kb_block_revisions (block_id, block_body_hash, chunk_count, created)
        VALUES (v_block, v_block_hash, v_chunk_count, v_occurred);
    UPDATE kb_content_blocks SET last_event_id = p_event WHERE id = v_block;
    PERFORM _recompute_resource_body_hash(v_resource, v_occurred);
    PERFORM _rebuild_resource_search_vector(v_resource);   -- ← Beat 1
    RETURN v_block;
END;
$$;

-- ── _project_resource_updated: + rebuild when the title key is present ────────
CREATE OR REPLACE FUNCTION _project_resource_updated(p_event uuid, p_payload jsonb)
RETURNS uuid LANGUAGE plpgsql AS $$
DECLARE v_resource uuid := (p_payload->>'resource_id')::uuid;
BEGIN
    UPDATE kb_resources SET
        title      = COALESCE(p_payload->>'title', title),
        origin_uri = COALESCE(p_payload->>'origin_uri', origin_uri),
        updated    = (SELECT occurred_at FROM kb_events WHERE id = p_event)
        WHERE id = v_resource;
    IF NOT FOUND THEN RAISE EXCEPTION 'resource_update: resource % not found', v_resource; END IF;
    IF p_payload ? 'title' THEN                            -- ← Beat 1 (origin_uri is not in the FTS vector)
        PERFORM _rebuild_resource_search_vector(v_resource);
    END IF;
    RETURN v_resource;
END;
$$;

-- ── Backfill every active resource (idempotent upsert) ───────────────────────
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

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo make test-artifacts 2>&1 | tail -40`
Expected: PASS — `create_populates_index_with_title_and_body`, `body_edit_updates_index`, `title_only_update_updates_index`, `backfill_covers_preexisting_rows`.

- [ ] **Step 5: Verify offline check is clean (no sqlx cache drift)**

Run: `cargo make check 2>&1 | tail -20`
Expected: clippy + fmt + sqlx-offline all green (no new `query!` macros were added, so no cache regen needed). If it reports a missing cache entry, regenerate: `cargo sqlx prepare --workspace -- --all-features`.

- [ ] **Step 6: Commit**

```bash
git add migrations/20260626000001_fts_search_index.sql crates/temper-substrate/tests/search_index.rs
git commit -m "feat(search): stored tsvector index maintained by projection functions (Beat 1/1)

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

### Task 2: Read-path swap — `fts_search` reads the stored index

**Files:**
- Modify: `crates/temper-substrate/src/readback/mod.rs` (the `fts_search` fn, currently ~684-710)
- Test: `crates/temper-substrate/tests/search_index.rs` (add a parity test)

**Interfaces:**
- Consumes: `kb_resource_search_index` (Task 1), `resources_visible_to`.
- Produces: `fts_search` with unchanged signature `pub async fn fts_search(pool: &PgPool, principal: Uuid, query: &str) -> Result<Vec<Uuid>>` — same matched set/order, now read from the stored vector.

- [ ] **Step 1: Write the failing parity test**

Add to `crates/temper-substrate/tests/search_index.rs`:

```rust
/// The stored-index `fts_search` returns the SAME id set as the legacy inline build for title/body
/// terms. The inline query below is the pre-swap recipe verbatim (the behavior-preservation gate).
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn fts_search_parity_with_inline_recipe(pool: sqlx::PgPool) {
    bootseed::seed_system(&pool).await.unwrap();
    let (owner, emitter) = system_actor(&pool).await;
    let home = ctx(&pool, owner, "p").await;
    for (t, b, u) in [
        ("Quenching furnace", "tempering steel at temperature", "temper://p/1"),
        ("Annealing notes",   "slow cooling relieves stress",    "temper://p/2"),
        ("Unrelated doc",     "nothing about metal here",         "temper://p/3"),
    ] {
        writes::create_resource(&pool, writes::CreateParams {
            title: t, origin_uri: u, body: b, doc_type: "concept",
            home, owner, originator: owner, emitter, properties: &[], chunks: None,
        }).await.unwrap();
    }

    // Legacy inline recipe (pre-swap), inlined here as the parity oracle.
    async fn inline_fts(pool: &sqlx::PgPool, principal: Uuid, q: &str) -> Vec<Uuid> {
        let rows = sqlx::query(
            "WITH doc AS (
               SELECT r.id,
                      setweight(to_tsvector('english', r.title), 'A') ||
                      setweight(to_tsvector('english', COALESCE(string_agg(cc.content, ' '), '')), 'B')
                        AS search_vector
                 FROM kb_resources r
                 JOIN resources_visible_to($1) v ON v.resource_id = r.id
                 LEFT JOIN kb_chunks c ON c.resource_id = r.id AND c.is_current
                 LEFT JOIN kb_chunk_content cc ON cc.chunk_id = c.id
                GROUP BY r.id, r.title)
             SELECT id FROM doc
              WHERE search_vector @@ plainto_tsquery('english', $2)
              ORDER BY ts_rank(search_vector, plainto_tsquery('english', $2)) DESC")
            .bind(principal).bind(q).fetch_all(pool).await.unwrap();
        rows.iter().map(|r| sqlx::Row::get::<Uuid, _>(r, "id")).collect()
    }

    for q in ["tempering", "cooling", "metal", "quenching steel", "furnace"] {
        let mut want = inline_fts(&pool, owner.uuid(), q).await;
        let mut got = readback::fts_search(&pool, owner.uuid(), q).await.unwrap();
        want.sort(); got.sort();
        assert_eq!(got, want, "stored-index fts_search set parity for query {q:?}");
    }
}
```

- [ ] **Step 2: Run the parity test to verify it fails**

Run: `DATABASE_URL=postgresql://temper:temper@localhost:5437/temper_development cargo nextest run -p temper-substrate --features artifact-tests --test search_index fts_search_parity_with_inline_recipe`
Expected: FAIL — current `fts_search` builds inline (so it actually still matches the oracle by accident on *set*); the test is here to **lock** parity across the swap. If it passes pre-swap that is fine (the inline impl == the oracle) — it must STILL pass post-swap. Run it now to confirm green-on-inline, then proceed; it becomes the regression gate for Step 3.

> Note: this is the one case where the test legitimately passes before the implementation change — the change is mechanism-preserving-behavior. The test's job is to fail loudly if the swap drifts the set.

- [ ] **Step 3: Swap `fts_search` to read the stored index**

In `crates/temper-substrate/src/readback/mod.rs`, replace the body of `fts_search` (keep the doc-comment, update its first line to note it now reads the stored index) with:

```rust
pub async fn fts_search(pool: &PgPool, principal: Uuid, query: &str) -> Result<Vec<Uuid>> {
    let rows = sqlx::query(
        "SELECT r.id
           FROM kb_resource_search_index si
           JOIN kb_resources r             ON r.id = si.resource_id
           JOIN resources_visible_to($1) v ON v.resource_id = r.id
          WHERE r.is_active
            AND si.search_vector @@ plainto_tsquery('english', $2)
          ORDER BY ts_rank(si.search_vector, plainto_tsquery('english', $2)) DESC",
    )
    .bind(principal)
    .bind(query)
    .fetch_all(pool)
    .await?;

    Ok(rows.iter().map(|r| r.get::<Uuid, _>("id")).collect())
}
```

- [ ] **Step 4: Run the full search-index suite to verify it passes**

Run: `cargo make test-artifacts 2>&1 | tail -40`
Expected: PASS — all of `search_index.rs` including `fts_search_parity_with_inline_recipe`, and the existing `write_path_mutations.rs` `fts_search` usage stays green.

- [ ] **Step 5: Verify the broader floor + offline check**

Run: `cargo make check 2>&1 | tail -20`
Expected: green. (No `query!` macro added; `fts_search` is runtime `sqlx::query`.)

- [ ] **Step 6: Commit**

```bash
git add crates/temper-substrate/src/readback/mod.rs crates/temper-substrate/tests/search_index.rs
git commit -m "feat(search): read FTS from the stored index (behavior-preserving swap) (Beat 1/1)

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Self-Review

**Spec coverage** (against `2026-06-26-search-substrate-beat1-stored-tsvector-design.md`):
- §4.1 storage table + GIN → Task 1 Step 3 ✓
- §4.2 helper + 3 projection wirings (with `p_payload ? 'title'` guard) → Task 1 Step 3 ✓
- §4.3 backfill → Task 1 Step 3 ✓
- §4.4 read swap → Task 2 Step 3 ✓
- §4.5 scores out of scope → not touched (search_select unchanged) ✓
- §7 tests: parity (Task 2), maintenance create/edit/title (Task 1), backfill (Task 1) ✓
- §8 OQ-1 (does `_project_resource_updated` diff title?) → resolved: it COALESCEs, no diff → guard on `p_payload ? 'title'` ✓
- §8 OQ-2 (stale soft-deleted index rows) → read filters `r.is_active`; harmless; documented in Task 2 query (`WHERE r.is_active`) ✓

**Placeholder scan:** none — every step has complete SQL/Rust.

**Type consistency:** `_rebuild_resource_search_vector(uuid)`, `kb_resource_search_index`, `fts_search(pool, principal, query) -> Result<Vec<Uuid>>`, `writes::{CreateParams,UpdateParams,create_resource,update_resource}`, `readback::fts_search` — consistent across tasks. `writes`/`readback` API shapes copied from `write_path_mutations.rs:546-586`.

## Notes for the executor
- After this lands, **reinstall the CLI is NOT needed** (no temper-cli change). The change is substrate-only.
- This is local commits only on `jct/search-beat1-stored-tsvector`; do not push/PR without asking.
- If `_recompute_resource_body_hash`'s signature differs from `(uuid, timestamptz)` in the live file, match the call exactly — copy the existing call line, don't retype.
