# Chunk Dedup + `kb_resource_revisions` Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make `replace_resource_chunks` dedup-aware (stop rewriting 40 chunks on a one-word edit), and add a `kb_resource_revisions` table so downstream subsystems (sessions, audits, point-in-time retrieval) can pin body versions.

**Architecture:** New `kb_resource_revisions` table. `kb_chunks` gains `first_revision_id NOT NULL` + `superseded_revision_id NULL` (both `ON DELETE RESTRICT`). `replace_resource_chunks(resource_id, audit_id, body_hash, chunks)` preserves chunks whose `(chunk_index, content_hash)` already matches current and only supersedes diffs. Audit linkage threads through Rust ingest service and the TS cloud workflow. Phase C adds a point-in-time reconstruction function and a retention sweep.

**Tech Stack:** PostgreSQL 18 (native `uuidv7()`), pgvector, sqlx compile-time macros, Rust (temper-api, temper-core), TypeScript (temper-cloud Vercel workflows), cargo-nextest.

**Spec:** [`docs/superpowers/specs/2026-04-20-chunk-dedup-and-revisions-design.md`](../specs/2026-04-20-chunk-dedup-and-revisions-design.md) — read this first for the why.

---

## File Structure

| Action | Path | Responsibility |
|--------|------|----------------|
| Create | `migrations/20260420000005_kb_resource_revisions.sql` | Schema: new table + chunk columns + indexes |
| Create | `migrations/20260420000006_chunk_dedup_functions.sql` | Rewrites `persist_resource_chunks` and `replace_resource_chunks` with new signatures + dedup logic |
| Create | `migrations/20260420000007_backfill_revisions.sql` | One-shot backfill from `kb_resource_audits` to populate historical revisions and chunk column values |
| Create | `migrations/20260420000008_first_revision_id_not_null.sql` | Post-backfill NOT NULL tightening |
| Create | `migrations/20260420000009_resource_chunks_at_revision.sql` | Phase C point-in-time function |
| Create | `migrations/20260420000010_sweep_orphaned_revisions.sql` | Phase C retention sweep |
| Modify | `crates/temper-core/src/types/ids.rs` | Add `RevisionId` newtype |
| Modify | `crates/temper-api/src/services/ingest_service.rs` | Update `persist_chunks` / `replace_chunks` helpers (new signatures); `update_resource_manifest` returns `AuditId` |
| Create | `crates/temper-api/tests/chunk_dedup_test.rs` | SQL-level dedup scenarios |
| Create | `crates/temper-api/tests/ingest_revision_test.rs` | End-to-end: create/update produce revisions linked to audits |
| Create | `crates/temper-api/tests/revision_retention_test.rs` | Phase C: `resource_chunks_at_revision` + `sweep_orphaned_revisions` |
| Modify | `api/workflows/process-upload.ts` | Workflow mints audit, passes revision params to `persist_resource_chunks` |
| Modify | `api/workflows/process-ingest.ts` | Same as process-upload |
| Modify | `api/upload.ts` | Thread `profile_id` into `processUpload` |
| Create | `packages/temper-cloud/src/processing/__tests__/store-revision.test.ts` | Integration: workflow produces audit + revision |

All migrations land on branch `claude/continue-analysis-migrations-Sbctd` alongside the C1–C6 perf work already committed.

---

### Task 1: Schema — `kb_resource_revisions` table + `kb_chunks` revision columns

**Files:**
- Create: `migrations/20260420000005_kb_resource_revisions.sql`
- Test: `crates/temper-api/tests/chunk_dedup_test.rs` (schema smoke test)

- [ ] **Step 1: Write the failing schema smoke test**

Create `crates/temper-api/tests/chunk_dedup_test.rs`:

```rust
#![cfg(feature = "test-db")]

use sqlx::PgPool;

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn schema_has_kb_resource_revisions_table(pool: PgPool) {
    let exists: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM information_schema.tables \
         WHERE table_name = 'kb_resource_revisions')",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(exists, "kb_resource_revisions table must exist");
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn kb_chunks_has_revision_columns(pool: PgPool) {
    let cols: Vec<String> = sqlx::query_scalar(
        "SELECT column_name FROM information_schema.columns \
         WHERE table_name = 'kb_chunks' \
           AND column_name IN ('first_revision_id', 'superseded_revision_id') \
         ORDER BY column_name",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(
        cols,
        vec!["first_revision_id".to_string(), "superseded_revision_id".to_string()]
    );
}
```

- [ ] **Step 2: Run the test to verify it fails**

```bash
cargo nextest run -p temper-api --features test-db \
  -E 'binary(chunk_dedup_test)' --no-capture
```

Expected: both tests FAIL — `kb_resource_revisions` does not exist yet; columns missing.

- [ ] **Step 3: Create the migration**

Create `migrations/20260420000005_kb_resource_revisions.sql`:

```sql
-- kb_resource_revisions: content-version anchor for kb_chunks.
--
-- A revision is produced for every chunk-producing action on a resource
-- (kb_resource_audits.action IN ('create', 'update_body')). Metadata-only
-- updates (action='update_meta') produce audits but NOT revisions — chunks
-- are not re-written for meta edits.
--
-- audit_id is ON DELETE SET NULL so revisions outlive their audits and the
-- retention sweep for kb_resource_audits does not cascade into chunk loss.

CREATE TABLE kb_resource_revisions (
    id          UUID PRIMARY KEY,
    resource_id UUID NOT NULL REFERENCES kb_resources(id) ON DELETE CASCADE,
    audit_id    UUID REFERENCES kb_resource_audits(id) ON DELETE SET NULL,
    body_hash   TEXT NOT NULL,
    chunk_count INT NOT NULL,
    created     TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX idx_resource_revisions_resource_created
    ON kb_resource_revisions(resource_id, created DESC);
CREATE INDEX idx_resource_revisions_audit
    ON kb_resource_revisions(audit_id);
CREATE INDEX idx_resource_revisions_body_hash
    ON kb_resource_revisions(body_hash);

-- kb_chunks revision linkage.
-- first_revision_id nullable for now; Task 8 tightens to NOT NULL after backfill.
-- Both columns ON DELETE RESTRICT — a revision referenced by any chunk
-- cannot be deleted (retention sweep must skip pinned revisions).

ALTER TABLE kb_chunks
    ADD COLUMN first_revision_id      UUID REFERENCES kb_resource_revisions(id) ON DELETE RESTRICT,
    ADD COLUMN superseded_revision_id UUID REFERENCES kb_resource_revisions(id) ON DELETE RESTRICT;

CREATE INDEX idx_kb_chunks_first_revision
    ON kb_chunks(first_revision_id);
CREATE INDEX idx_kb_chunks_superseded_revision
    ON kb_chunks(superseded_revision_id)
    WHERE superseded_revision_id IS NOT NULL;
```

- [ ] **Step 4: Regenerate the sqlx offline cache**

```bash
cargo sqlx prepare --workspace -- --all-features
```

Expected: updated `.sqlx/` JSON files checked in.

- [ ] **Step 5: Run the test to verify it passes**

```bash
cargo nextest run -p temper-api --features test-db \
  -E 'binary(chunk_dedup_test)' --no-capture
```

Expected: both schema tests PASS.

- [ ] **Step 6: Commit**

```bash
git add migrations/20260420000005_kb_resource_revisions.sql \
        crates/temper-api/tests/chunk_dedup_test.rs \
        .sqlx/
git commit -m "feat(db): add kb_resource_revisions table and chunk revision columns"
```

---

### Task 2: SQL tests for dedup scenarios (failing)

**Files:**
- Modify: `crates/temper-api/tests/chunk_dedup_test.rs`

- [ ] **Step 1: Add a fixture helper and the six dedup scenarios to the test file**

Append to `crates/temper-api/tests/chunk_dedup_test.rs`:

```rust
use serde_json::{json, Value};
use uuid::Uuid;

/// Insert a minimal `kb_resources` + `kb_resource_manifests` row for a test
/// fixture. Returns the resource id.
async fn seed_resource(pool: &PgPool) -> Uuid {
    let profile_id: Uuid = sqlx::query_scalar(
        "INSERT INTO kb_profiles (id, handle, created) \
         VALUES (gen_random_uuid(), 'test', now()) RETURNING id",
    )
    .fetch_one(pool).await.unwrap();

    let context_id: Uuid = sqlx::query_scalar(
        "INSERT INTO kb_contexts (id, owner_profile_id, name, created) \
         VALUES (gen_random_uuid(), $1, 'test-ctx', now()) RETURNING id",
    )
    .bind(profile_id)
    .fetch_one(pool).await.unwrap();

    let doc_type_id: Uuid = sqlx::query_scalar(
        "SELECT id FROM kb_doc_types WHERE name = 'note' LIMIT 1",
    )
    .fetch_one(pool).await.unwrap();

    let resource_id: Uuid = sqlx::query_scalar(
        r#"
        INSERT INTO kb_resources (
            id, kb_context_id, kb_doc_type_id, origin_uri, title, slug,
            originator_profile_id, owner_profile_id, created, updated
        ) VALUES (
            gen_random_uuid(), $1, $2, 'test://r', 'T', 't', $3, $3, now(), now()
        ) RETURNING id
        "#,
    )
    .bind(context_id).bind(doc_type_id).bind(profile_id)
    .fetch_one(pool).await.unwrap();

    resource_id
}

/// Build one chunk jsonb entry. The embedding is a 768-dim zero vector.
fn chunk(index: i32, content: &str, hash: &str) -> Value {
    let zeros: Vec<f32> = vec![0.0; 768];
    let emb_str = format!(
        "[{}]",
        zeros.iter().map(|v| v.to_string()).collect::<Vec<_>>().join(",")
    );
    json!({
        "chunk_index": index,
        "header_path": "",
        "heading_depth": 0,
        "content": content,
        "content_hash": hash,
        "embedding": emb_str,
    })
}

/// Insert a minimal kb_resource_audits row and return its id.
async fn seed_audit(pool: &PgPool, resource_id: Uuid, body_hash: &str) -> Uuid {
    sqlx::query_scalar(
        "INSERT INTO kb_resource_audits \
         (resource_id, event_id, profile_id, device_id, body_hash, managed_hash, open_hash, action) \
         VALUES ($1, gen_random_uuid(), \
             (SELECT originator_profile_id FROM kb_resources WHERE id = $1), \
             'test-device', $2, 'mh', 'oh', 'update_body') \
         RETURNING id",
    )
    .bind(resource_id)
    .bind(body_hash)
    .fetch_one(pool).await.unwrap()
}

/// Count current chunks for a resource.
async fn count_current(pool: &PgPool, resource_id: Uuid) -> i64 {
    sqlx::query_scalar(
        "SELECT COUNT(*) FROM kb_chunks WHERE resource_id = $1 AND is_current = true",
    )
    .bind(resource_id).fetch_one(pool).await.unwrap()
}

/// Count total chunks for a resource (current + superseded).
async fn count_total(pool: &PgPool, resource_id: Uuid) -> i64 {
    sqlx::query_scalar("SELECT COUNT(*) FROM kb_chunks WHERE resource_id = $1")
        .bind(resource_id).fetch_one(pool).await.unwrap()
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn persist_chunks_creates_revision(pool: PgPool) {
    let rid = seed_resource(&pool).await;
    let audit = seed_audit(&pool, rid, "body1").await;
    let chunks = json!([chunk(0, "alpha", "ha"), chunk(1, "beta", "hb")]);

    let rev: Uuid = sqlx::query_scalar(
        "SELECT persist_resource_chunks($1, $2, $3, $4)",
    )
    .bind(rid).bind(audit).bind("body1").bind(&chunks)
    .fetch_one(&pool).await.unwrap();

    let (chunk_count, body_hash, audit_id): (i32, String, Option<Uuid>) = sqlx::query_as(
        "SELECT chunk_count, body_hash, audit_id FROM kb_resource_revisions WHERE id = $1",
    )
    .bind(rev).fetch_one(&pool).await.unwrap();

    assert_eq!(chunk_count, 2);
    assert_eq!(body_hash, "body1");
    assert_eq!(audit_id, Some(audit));
    assert_eq!(count_current(&pool, rid).await, 2);
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn replace_chunks_preserves_unchanged_positions(pool: PgPool) {
    let rid = seed_resource(&pool).await;
    let a1 = seed_audit(&pool, rid, "body1").await;
    let chunks = json!([chunk(0, "alpha", "ha"), chunk(1, "beta", "hb"), chunk(2, "gamma", "hg")]);
    let r1: Uuid = sqlx::query_scalar("SELECT persist_resource_chunks($1, $2, $3, $4)")
        .bind(rid).bind(a1).bind("body1").bind(&chunks)
        .fetch_one(&pool).await.unwrap();

    let a2 = seed_audit(&pool, rid, "body1").await;
    let r2: Uuid = sqlx::query_scalar("SELECT replace_resource_chunks($1, $2, $3, $4)")
        .bind(rid).bind(a2).bind("body1").bind(&chunks)
        .fetch_one(&pool).await.unwrap();

    assert_ne!(r1, r2, "second call must create a distinct revision");
    assert_eq!(count_current(&pool, rid).await, 3);
    assert_eq!(count_total(&pool, rid).await, 3, "preserved chunks must not be duplicated");

    let first_revs: Vec<Uuid> = sqlx::query_scalar(
        "SELECT first_revision_id FROM kb_chunks WHERE resource_id = $1",
    )
    .bind(rid).fetch_all(&pool).await.unwrap();
    assert!(first_revs.iter().all(|r| *r == r1), "preserved chunks keep original first_revision_id");
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn replace_chunks_supersedes_changed_content(pool: PgPool) {
    let rid = seed_resource(&pool).await;
    let a1 = seed_audit(&pool, rid, "b1").await;
    let c_initial = json!([chunk(0, "alpha", "ha"), chunk(1, "beta", "hb"), chunk(2, "gamma", "hg")]);
    let r1: Uuid = sqlx::query_scalar("SELECT persist_resource_chunks($1, $2, $3, $4)")
        .bind(rid).bind(a1).bind("b1").bind(&c_initial)
        .fetch_one(&pool).await.unwrap();

    let a2 = seed_audit(&pool, rid, "b2").await;
    let c_updated = json!([chunk(0, "alpha", "ha"), chunk(1, "BETA!", "hb2"), chunk(2, "gamma", "hg")]);
    let r2: Uuid = sqlx::query_scalar("SELECT replace_resource_chunks($1, $2, $3, $4)")
        .bind(rid).bind(a2).bind("b2").bind(&c_updated)
        .fetch_one(&pool).await.unwrap();

    assert_eq!(count_current(&pool, rid).await, 3);
    assert_eq!(count_total(&pool, rid).await, 4, "one new chunk + one superseded");

    let superseded: (Uuid, Option<Uuid>) = sqlx::query_as(
        "SELECT first_revision_id, superseded_revision_id FROM kb_chunks \
         WHERE resource_id = $1 AND chunk_index = 1 AND is_current = false",
    )
    .bind(rid).fetch_one(&pool).await.unwrap();
    assert_eq!(superseded.0, r1);
    assert_eq!(superseded.1, Some(r2));

    let current_new: (Uuid, Option<Uuid>) = sqlx::query_as(
        "SELECT first_revision_id, superseded_revision_id FROM kb_chunks \
         WHERE resource_id = $1 AND chunk_index = 1 AND is_current = true",
    )
    .bind(rid).fetch_one(&pool).await.unwrap();
    assert_eq!(current_new.0, r2);
    assert_eq!(current_new.1, None);
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn replace_chunks_supersedes_removed_positions(pool: PgPool) {
    let rid = seed_resource(&pool).await;
    let a1 = seed_audit(&pool, rid, "b1").await;
    let c_four = json!([
        chunk(0, "a", "h0"), chunk(1, "b", "h1"),
        chunk(2, "c", "h2"), chunk(3, "d", "h3"),
    ]);
    sqlx::query_scalar::<_, Uuid>("SELECT persist_resource_chunks($1, $2, $3, $4)")
        .bind(rid).bind(a1).bind("b1").bind(&c_four)
        .fetch_one(&pool).await.unwrap();

    let a2 = seed_audit(&pool, rid, "b2").await;
    let c_three = json!([chunk(0, "a", "h0"), chunk(1, "b", "h1"), chunk(2, "c", "h2")]);
    let r2: Uuid = sqlx::query_scalar("SELECT replace_resource_chunks($1, $2, $3, $4)")
        .bind(rid).bind(a2).bind("b2").bind(&c_three)
        .fetch_one(&pool).await.unwrap();

    assert_eq!(count_current(&pool, rid).await, 3);

    let removed_supersede: Option<Uuid> = sqlx::query_scalar(
        "SELECT superseded_revision_id FROM kb_chunks \
         WHERE resource_id = $1 AND chunk_index = 3",
    )
    .bind(rid).fetch_one(&pool).await.unwrap();
    assert_eq!(removed_supersede, Some(r2));
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn replace_chunks_adds_new_positions(pool: PgPool) {
    let rid = seed_resource(&pool).await;
    let a1 = seed_audit(&pool, rid, "b1").await;
    let c_two = json!([chunk(0, "a", "h0"), chunk(1, "b", "h1")]);
    sqlx::query_scalar::<_, Uuid>("SELECT persist_resource_chunks($1, $2, $3, $4)")
        .bind(rid).bind(a1).bind("b1").bind(&c_two)
        .fetch_one(&pool).await.unwrap();

    let a2 = seed_audit(&pool, rid, "b2").await;
    let c_four = json!([
        chunk(0, "a", "h0"), chunk(1, "b", "h1"),
        chunk(2, "c", "h2"), chunk(3, "d", "h3"),
    ]);
    let r2: Uuid = sqlx::query_scalar("SELECT replace_resource_chunks($1, $2, $3, $4)")
        .bind(rid).bind(a2).bind("b2").bind(&c_four)
        .fetch_one(&pool).await.unwrap();

    assert_eq!(count_current(&pool, rid).await, 4);

    let new_positions: Vec<Uuid> = sqlx::query_scalar(
        "SELECT first_revision_id FROM kb_chunks \
         WHERE resource_id = $1 AND chunk_index IN (2, 3) AND is_current = true",
    )
    .bind(rid).fetch_all(&pool).await.unwrap();
    assert_eq!(new_positions.len(), 2);
    assert!(new_positions.iter().all(|r| *r == r2));
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn replace_chunks_empty_input_supersedes_all(pool: PgPool) {
    let rid = seed_resource(&pool).await;
    let a1 = seed_audit(&pool, rid, "b1").await;
    let c_three = json!([chunk(0, "a", "h0"), chunk(1, "b", "h1"), chunk(2, "c", "h2")]);
    sqlx::query_scalar::<_, Uuid>("SELECT persist_resource_chunks($1, $2, $3, $4)")
        .bind(rid).bind(a1).bind("b1").bind(&c_three)
        .fetch_one(&pool).await.unwrap();

    let a2 = seed_audit(&pool, rid, "").await;
    let r2: Uuid = sqlx::query_scalar("SELECT replace_resource_chunks($1, $2, $3, $4)")
        .bind(rid).bind(a2).bind("").bind(json!([]))
        .fetch_one(&pool).await.unwrap();

    assert_eq!(count_current(&pool, rid).await, 0);

    let chunk_count: i32 = sqlx::query_scalar(
        "SELECT chunk_count FROM kb_resource_revisions WHERE id = $1",
    )
    .bind(r2).fetch_one(&pool).await.unwrap();
    assert_eq!(chunk_count, 0);
}
```

- [ ] **Step 2: Run the new tests to verify they fail**

```bash
cargo nextest run -p temper-api --features test-db \
  -E 'binary(chunk_dedup_test)' --no-capture
```

Expected: all six new tests FAIL — `persist_resource_chunks` / `replace_resource_chunks` do not yet accept 4 arguments; signatures still take 2.

- [ ] **Step 3: Commit the failing tests**

```bash
git add crates/temper-api/tests/chunk_dedup_test.rs
git commit -m "test(db): add dedup scenarios for replace_resource_chunks"
```

---

### Task 3: Implement dedup-aware `persist_resource_chunks` and `replace_resource_chunks`

**Files:**
- Create: `migrations/20260420000006_chunk_dedup_functions.sql`

- [ ] **Step 1: Create the function migration**

Create `migrations/20260420000006_chunk_dedup_functions.sql`:

```sql
-- Add dedup-aware + revision-linked overloads of persist_resource_chunks
-- and replace_resource_chunks. The new 4-arg signatures coexist with the
-- pre-existing 2-arg forms (PostgreSQL allows function overloading by
-- argument list). This lets Rust `sqlx::query!` macros compile against
-- cached 2-arg metadata while Task 4/5 migrate callers to the new form;
-- Task 5 drops the legacy 2-arg variants as its final step.
--
-- New signatures: (resource_id, audit_id, body_hash, chunks). Return
-- changes from INT (chunk count) to UUID (the new revision id).
--
-- Spec: docs/superpowers/specs/2026-04-20-chunk-dedup-and-revisions-design.md

-- First-create path. No existing chunks; just insert all at version 1.
CREATE FUNCTION persist_resource_chunks(
    p_resource_id UUID,
    p_audit_id    UUID,
    p_body_hash   TEXT,
    p_chunks      JSONB
) RETURNS UUID
LANGUAGE plpgsql AS $$
DECLARE
    v_revision_id UUID;
BEGIN
    PERFORM set_config('temper.skip_search_rebuild', 'true', true);

    v_revision_id := uuidv7();
    INSERT INTO kb_resource_revisions (id, resource_id, audit_id, body_hash, chunk_count)
    VALUES (v_revision_id, p_resource_id, p_audit_id, p_body_hash, jsonb_array_length(p_chunks));

    WITH chunk_data AS (
        SELECT
            gen_random_uuid() AS chunk_id,
            p_resource_id AS resource_id,
            (elem->>'chunk_index')::INT AS chunk_index,
            COALESCE(elem->>'header_path', '') AS header_path,
            COALESCE((elem->>'heading_depth')::SMALLINT, 0) AS heading_depth,
            elem->>'content' AS content,
            elem->>'content_hash' AS content_hash,
            elem->>'embedding' AS embedding_str
        FROM jsonb_array_elements(p_chunks) AS elem
    ),
    inserted_chunks AS (
        INSERT INTO kb_chunks (
            id, resource_id, chunk_index, version, header_path, heading_depth,
            content_hash, embedding, is_current, first_revision_id
        )
        SELECT
            cd.chunk_id, cd.resource_id, cd.chunk_index, 1, cd.header_path, cd.heading_depth,
            cd.content_hash, cd.embedding_str::vector, true, v_revision_id
        FROM chunk_data cd
        RETURNING id
    )
    INSERT INTO kb_chunk_content (chunk_id, content)
    SELECT cd.chunk_id, cd.content FROM chunk_data cd;

    PERFORM rebuild_resource_search_vector(p_resource_id);
    PERFORM set_config('temper.skip_search_rebuild', '', true);

    RETURN v_revision_id;
END;
$$;

-- Update path. Dedup on (chunk_index, content_hash):
--   * Preserve rows that match input exactly — no write, first_revision_id stays.
--   * Supersede rows whose content_hash differs at same chunk_index, or whose
--     chunk_index is no longer present in input.
--   * Insert new rows for (chunk_index, content_hash) pairs not in current set.
CREATE FUNCTION replace_resource_chunks(
    p_resource_id UUID,
    p_audit_id    UUID,
    p_body_hash   TEXT,
    p_chunks      JSONB
) RETURNS UUID
LANGUAGE plpgsql AS $$
DECLARE
    v_revision_id UUID;
BEGIN
    PERFORM set_config('temper.skip_search_rebuild', 'true', true);

    v_revision_id := uuidv7();
    INSERT INTO kb_resource_revisions (id, resource_id, audit_id, body_hash, chunk_count)
    VALUES (v_revision_id, p_resource_id, p_audit_id, p_body_hash, jsonb_array_length(p_chunks));

    WITH incoming AS (
        SELECT
            (elem->>'chunk_index')::INT AS chunk_index,
            COALESCE(elem->>'header_path', '') AS header_path,
            COALESCE((elem->>'heading_depth')::SMALLINT, 0) AS heading_depth,
            elem->>'content' AS content,
            elem->>'content_hash' AS content_hash,
            elem->>'embedding' AS embedding_str
        FROM jsonb_array_elements(p_chunks) AS elem
    ),
    existing AS (
        SELECT id, chunk_index, content_hash
          FROM kb_chunks
         WHERE resource_id = p_resource_id
           AND is_current = true
    ),
    preserved AS (
        SELECT e.id
          FROM existing e
          JOIN incoming i
            ON i.chunk_index = e.chunk_index
           AND i.content_hash = e.content_hash
    ),
    to_supersede AS (
        SELECT e.id
          FROM existing e
         WHERE e.id NOT IN (SELECT id FROM preserved)
    ),
    superseded AS (
        UPDATE kb_chunks
           SET is_current = false,
               superseded_revision_id = v_revision_id
         WHERE id IN (SELECT id FROM to_supersede)
        RETURNING id
    ),
    to_insert AS (
        SELECT i.*
          FROM incoming i
         WHERE NOT EXISTS (
             SELECT 1 FROM existing e
              WHERE e.chunk_index = i.chunk_index
                AND e.content_hash = i.content_hash
         )
    ),
    inserted_chunks AS (
        INSERT INTO kb_chunks (
            id, resource_id, chunk_index, version, header_path, heading_depth,
            content_hash, embedding, is_current, first_revision_id
        )
        SELECT
            gen_random_uuid(), p_resource_id, ti.chunk_index,
            COALESCE((SELECT MAX(version) FROM kb_chunks
                       WHERE resource_id = p_resource_id
                         AND chunk_index = ti.chunk_index), 0) + 1,
            ti.header_path, ti.heading_depth,
            ti.content_hash, ti.embedding_str::vector, true, v_revision_id
          FROM to_insert ti
        RETURNING id, chunk_index
    )
    INSERT INTO kb_chunk_content (chunk_id, content)
    SELECT ic.id, ti.content
      FROM inserted_chunks ic
      JOIN to_insert ti ON ti.chunk_index = ic.chunk_index;

    PERFORM rebuild_resource_search_vector(p_resource_id);
    PERFORM set_config('temper.skip_search_rebuild', '', true);

    RETURN v_revision_id;
END;
$$;
```

- [ ] **Step 2: Regenerate sqlx cache**

```bash
cargo sqlx prepare --workspace -- --all-features
```

- [ ] **Step 3: Run the dedup tests to verify they pass**

```bash
cargo nextest run -p temper-api --features test-db \
  -E 'binary(chunk_dedup_test)' --no-capture
```

Expected: all eight tests (two schema + six dedup scenarios) PASS.

- [ ] **Step 4: Commit**

```bash
git add migrations/20260420000006_chunk_dedup_functions.sql .sqlx/
git commit -m "feat(db): dedup-aware replace_resource_chunks with revision linkage"
```

---

### Task 4: Add `RevisionId` newtype to `temper-core`

**Files:**
- Modify: `crates/temper-core/src/types/ids.rs`

- [ ] **Step 1: Add `RevisionId` using the existing `define_id!` macro**

The file `crates/temper-core/src/types/ids.rs` has a `define_id!` macro (lines 6-86) that emits all the boilerplate — derives, `sqlx::Type/Encode/Decode`, `From<Uuid>`, `Deref`, `Display`, and `::new() -> Uuid::now_v7()`. Add one invocation at the bottom of the file, matching the style of the existing `ResourceAuditId` block at lines 113-116:

```rust
define_id!(
    /// A `kb_resource_revisions.id` value. Always UUIDv7 (time-sortable).
    RevisionId
);
```

Do not add any other code — the macro generates everything needed.

- [ ] **Step 2: Regenerate TypeScript types**

```bash
cargo make generate-ts-types
```

Expected: new `RevisionId.ts` emitted under `packages/temper-ui/src/lib/types/` (or wherever ts-rs points).

- [ ] **Step 3: Run core unit tests**

```bash
cargo nextest run -p temper-core
```

Expected: PASS (RevisionId has no bespoke logic beyond the macro, so nothing new to break).

- [ ] **Step 4: Commit**

```bash
git add crates/temper-core/src/types/ids.rs packages/temper-ui/src/lib/types/
git commit -m "feat(core): add RevisionId newtype for kb_resource_revisions"
```

---

### Task 5: Thread audit_id + body_hash through Rust ingest service

**Files:**
- Modify: `crates/temper-api/src/services/ingest_service.rs`
- Create: `crates/temper-api/tests/ingest_revision_test.rs`

- [ ] **Step 1: Write the failing end-to-end test**

Create `crates/temper-api/tests/ingest_revision_test.rs`. This test exercises the Rust service layer end-to-end: it creates a resource + manifest + audit + chunks via `create_resource_with_manifest`, then asserts the revision row is linked to the produced audit and that all chunks reference the revision as their `first_revision_id`:

```rust
#![cfg(feature = "test-db")]

mod common;

use common::fixtures::{self, TEMPER_CONTEXT_ID, RESEARCH_DOC_TYPE_ID};
use serde_json::json;
use sqlx::PgPool;
use temper_api::services::ingest_service::{self, CreateResourceParams};
use temper_core::types::ids::{ContextId, ProfileId};
use temper_core::types::ingest::{PackedChunk, pack_chunks};
use uuid::Uuid;

fn make_packed_chunks() -> String {
    let chunks = vec![
        PackedChunk {
            chunk_index: 0,
            header_path: String::new(),
            heading_depth: 0,
            content: "hello".into(),
            content_hash: "h0".into(),
            embedding: vec![0.0; 768],
        },
        PackedChunk {
            chunk_index: 1,
            header_path: String::new(),
            heading_depth: 0,
            content: "world".into(),
            content_hash: "h1".into(),
            embedding: vec![0.0; 768],
        },
    ];
    pack_chunks(&chunks).expect("pack_chunks")
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn create_resource_links_revision_to_create_audit(pool: PgPool) {
    fixtures::clean_and_seed(&pool).await;
    let profile_uuid = fixtures::create_test_profile(&pool, "alice@test.local").await;
    let context_uuid = Uuid::parse_str(TEMPER_CONTEXT_ID).unwrap();
    let doc_type_uuid = Uuid::parse_str(RESEARCH_DOC_TYPE_ID).unwrap();

    let packed = make_packed_chunks();
    let empty_meta = json!({});

    let resource = ingest_service::create_resource_with_manifest(
        &pool,
        &CreateResourceParams {
            profile_id: ProfileId::from(profile_uuid),
            device_id: "dev",
            context_id: ContextId::from(context_uuid),
            doc_type_id: doc_type_uuid,
            doc_type_name: "research",
            title: "T",
            slug: Some("revision-test"),
            origin_uri: "test://revision-test",
            content_hash: "sha256:deadbeef",
            managed_meta: &empty_meta,
            open_meta: &empty_meta,
            chunks_packed: Some(&packed),
        },
    )
    .await
    .expect("create_resource_with_manifest");

    let (rev_id, rev_audit, rev_body_hash, rev_chunk_count): (Uuid, Option<Uuid>, String, i32) =
        sqlx::query_as(
            "SELECT id, audit_id, body_hash, chunk_count FROM kb_resource_revisions \
             WHERE resource_id = $1",
        )
        .bind(*resource.id)
        .fetch_one(&pool).await.unwrap();

    let create_audit_id: Uuid = sqlx::query_scalar(
        "SELECT id FROM kb_resource_audits \
         WHERE resource_id = $1 AND action = 'create'",
    )
    .bind(*resource.id)
    .fetch_one(&pool).await.unwrap();

    assert_eq!(rev_audit, Some(create_audit_id), "revision.audit_id = create audit.id");
    assert_eq!(rev_body_hash, "sha256:deadbeef");
    assert_eq!(rev_chunk_count, 2);
    assert_ne!(rev_id, Uuid::nil());

    let chunk_revs: Vec<Uuid> = sqlx::query_scalar(
        "SELECT first_revision_id FROM kb_chunks WHERE resource_id = $1",
    )
    .bind(*resource.id).fetch_all(&pool).await.unwrap();
    assert_eq!(chunk_revs.len(), 2);
    assert!(chunk_revs.iter().all(|r| *r == rev_id),
        "both chunks reference the newly-created revision");
}
```

- [ ] **Step 2: Run the test to verify it fails**

```bash
cargo nextest run -p temper-api --features "test-db ingest-pipeline" \
  -E 'binary(ingest_revision_test)' --no-capture
```

Expected: FAIL — no `kb_resource_revisions` row yet (Rust code still calls old function signature which no longer exists; compilation fails in `ingest_service.rs`).

- [ ] **Step 3: Update `persist_chunks` and `replace_chunks` helpers in `ingest_service.rs`**

Replace the function bodies at `crates/temper-api/src/services/ingest_service.rs:215-253`:

```rust
async fn persist_chunks(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    resource_id: ResourceId,
    audit_id: Uuid,
    body_hash: &str,
    chunks: &[PackedChunk],
) -> ApiResult<RevisionId> {
    let chunks_json = chunks_to_jsonb(chunks);

    let rev: Uuid = sqlx::query_scalar!(
        "SELECT persist_resource_chunks($1, $2, $3, $4)",
        *resource_id,
        audit_id,
        body_hash,
        chunks_json
    )
    .fetch_one(&mut **tx)
    .await?
    .expect("persist_resource_chunks returned NULL");

    Ok(RevisionId::from(rev))
}

async fn replace_chunks(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    resource_id: ResourceId,
    audit_id: Uuid,
    body_hash: &str,
    chunks: &[PackedChunk],
) -> ApiResult<RevisionId> {
    let chunks_json = chunks_to_jsonb(chunks);

    let rev: Uuid = sqlx::query_scalar!(
        "SELECT replace_resource_chunks($1, $2, $3, $4)",
        *resource_id,
        audit_id,
        body_hash,
        chunks_json
    )
    .fetch_one(&mut **tx)
    .await?
    .expect("replace_resource_chunks returned NULL");

    Ok(RevisionId::from(rev))
}
```

Add the import at the top of the file:

```rust
use temper_core::types::ids::RevisionId;
```

- [ ] **Step 4: Update the `insert_event_and_audit` call site in `create_resource_with_manifest` to capture audit_id**

`insert_event_and_audit` already returns `ApiResult<(EventId, ResourceAuditId)>` (see `ingest_service.rs:149`); no signature change needed. Replace the block at `ingest_service.rs:341-363`:

```rust
    let (_event_id, audit_id) = insert_event_and_audit(
        &mut tx,
        params.profile_id,
        params.device_id,
        params.context_id,
        resource_id,
        "resource_created",
        "create",
        params.content_hash,
        &managed_hash,
        &open_hash,
    )
    .await?;

    if let Some(packed) = params.chunks_packed {
        let chunks = unpack_chunks(packed)
            .map_err(|e| ApiError::BadRequest(format!("invalid chunks_packed: {e}")))?;
        if !chunks.is_empty() {
            persist_chunks(&mut tx, resource_id, *audit_id, params.content_hash, &chunks).await?;
        }
    }
```

`audit_id` is a `ResourceAuditId` newtype; `*audit_id` dereferences to the inner `Uuid` that `persist_chunks` expects as its `audit_id: Uuid` parameter.

- [ ] **Step 5: Update `update_resource_manifest` to return the audit id and `update` to use it**

In `ingest_service.rs`, change `update_resource_manifest`'s return type from `ApiResult<()>` to `ApiResult<ResourceAuditId>` and bubble up the audit id:

```rust
pub async fn update_resource_manifest(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    params: &UpdateManifestParams<'_>,
) -> ApiResult<ResourceAuditId> {
    // ... existing body unchanged until the insert_event_and_audit call at :563-575 ...

    let (_event_id, audit_id) = insert_event_and_audit(
        tx,
        params.profile_id,
        params.device_id,
        ContextId::from(base.kb_context_id),
        params.resource_id,
        "body_updated",
        "update_body",
        params.content_hash,
        &managed_hash,
        &open_hash,
    )
    .await?;

    Ok(audit_id)
}
```

And in the `update` function at `ingest_service.rs:668-687`, capture the returned audit_id and pass it to `replace_chunks`:

```rust
    let mut tx = pool.begin().await?;

    let audit_id = update_resource_manifest(
        &mut tx,
        &UpdateManifestParams {
            profile_id,
            device_id,
            resource_id,
            doc_type_name: &payload.doc_type_name,
            content_hash: payload.content_hash.as_deref().unwrap_or(""),
            managed_meta: &managed_meta,
            open_meta: &open_meta,
        },
    )
    .await?;

    replace_chunks(
        &mut tx,
        resource_id,
        *audit_id,
        payload.content_hash.as_deref().unwrap_or(""),
        &chunks,
    )
    .await?;

    tx.commit().await?;
```

Add `use temper_core::types::ids::ResourceAuditId;` to the file's imports if not already present.

- [ ] **Step 6: Confirm `meta_service.rs` still compiles**

`insert_event_and_audit` already returned `(EventId, ResourceAuditId)` before this plan, so `meta_service.rs:222` is unaffected — the existing `.await?;` drop remains valid. No change needed. Grep to confirm:

```bash
grep -n "insert_event_and_audit" crates/temper-api/src/services/meta_service.rs
```

Expected: one call site matching the existing signature. (Meta path stays audit-only; no revision work — `update_meta` audits never produce chunks or revisions.)

- [ ] **Step 7: Regenerate sqlx cache**

```bash
cargo sqlx prepare --workspace -- --all-features
```

- [ ] **Step 8: Run the new integration test and any existing ingest tests**

```bash
cargo nextest run -p temper-api --features "test-db ingest-pipeline" \
  -E 'binary(ingest_revision_test) + test(ingest) + test(update_resource_manifest)' \
  --no-capture
```

Expected: `create_resource_links_revision_to_audit` PASSES. Adjust the `body_hash` assertion if the observed hash differs from the placeholder.

Then run the broader test-db suite to catch unintended breakage:

```bash
cargo nextest run -p temper-api --features "test-db ingest-pipeline"
```

Expected: existing test-db tests still pass. If the `sync_test::meta_only_push_must_not_touch_chunks` assertion breaks, the meta path is incorrectly creating a revision — revisit Step 6.

- [ ] **Step 9: Commit**

```bash
git add crates/temper-api/src/services/ingest_service.rs \
        crates/temper-api/src/services/meta_service.rs \
        crates/temper-api/tests/ingest_revision_test.rs \
        .sqlx/
git commit -m "feat(api): thread audit_id through ingest_service for revision linkage"
```

- [ ] **Step 10: Drop the legacy 2-arg SQL function variants**

Now that no caller references `persist_resource_chunks(uuid, jsonb)` or `replace_resource_chunks(uuid, jsonb)`, drop them. Create `migrations/20260420000006a_drop_legacy_chunk_functions.sql`:

```sql
-- Drop the pre-dedup 2-arg variants of persist_resource_chunks and
-- replace_resource_chunks. The 4-arg revision-aware forms introduced in
-- 20260420000006 are now the only callers (Rust temper-api updated in
-- this commit's parent; TS cloud workflows updated in Task 6).
DROP FUNCTION IF EXISTS persist_resource_chunks(UUID, JSONB);
DROP FUNCTION IF EXISTS replace_resource_chunks(UUID, JSONB);
```

Regenerate sqlx cache and commit:

```bash
cargo sqlx prepare --workspace -- --all-features
git add migrations/20260420000006a_drop_legacy_chunk_functions.sql .sqlx/
git commit -m "chore(db): drop legacy 2-arg persist/replace_resource_chunks"
```

Run the full test-db suite to confirm nothing still depends on the old forms:

```bash
cargo nextest run -p temper-api --features "test-db ingest-pipeline"
```

Expected: all non-HF-TLS tests still PASS.

---

### Task 6: Update TS workflow to mint audit + pass revision params

**Files:**
- Modify: `api/workflows/process-upload.ts`
- Modify: `api/workflows/process-ingest.ts`
- Modify: `api/upload.ts`
- Create: `packages/temper-cloud/src/processing/__tests__/store-revision.test.ts`

- [ ] **Step 1: Write the failing integration test**

Create `packages/temper-cloud/src/processing/__tests__/store-revision.test.ts`:

```typescript
import { describe, it, expect, beforeEach } from "vitest";
import { getDb } from "../../db.js";
import { chunksToJsonb, type ChunkRow } from "../store.js";

describe("TS workflow chunk persistence with revision", () => {
  it("calls persist_resource_chunks with audit_id and body_hash", async () => {
    const db = getDb();

    const [{ id: profileId }] = await db<{ id: string }[]>`
      INSERT INTO kb_profiles (id, handle, created)
      VALUES (gen_random_uuid(), 'cloud-test', now())
      RETURNING id
    `;
    const [{ id: contextId }] = await db<{ id: string }[]>`
      INSERT INTO kb_contexts (id, owner_profile_id, name, created)
      VALUES (gen_random_uuid(), ${profileId}::uuid, 'ctx', now())
      RETURNING id
    `;
    const [{ id: docTypeId }] = await db<{ id: string }[]>`
      SELECT id FROM kb_doc_types WHERE name = 'note' LIMIT 1
    `;
    const [{ id: resourceId }] = await db<{ id: string }[]>`
      INSERT INTO kb_resources (id, kb_context_id, kb_doc_type_id, origin_uri, title, slug,
                                originator_profile_id, owner_profile_id, created, updated)
      VALUES (gen_random_uuid(), ${contextId}::uuid, ${docTypeId}::uuid, 'test://r', 'T', 't',
              ${profileId}::uuid, ${profileId}::uuid, now(), now())
      RETURNING id
    `;

    const { insertEventAndAudit } = await import("../../events.js");
    const { auditId } = await insertEventAndAudit(db, {
      profileId,
      deviceId: "vercel-cloud",
      contextId,
      resourceId,
      eventType: "body_updated",
      action: "update_body",
      bodyHash: "body-abc",
      managedHash: "mh",
      openHash: "oh",
    });

    const chunkRows: ChunkRow[] = [
      { id: "", resource_id: resourceId, chunk_index: 0, version: 0,
        header_path: "", content: "hi", content_hash: "h0",
        embedding: new Array(768).fill(0) },
    ];
    const chunksJson = JSON.stringify(chunksToJsonb(chunkRows));

    const [{ persist_resource_chunks: revId }] = await db<{ persist_resource_chunks: string }[]>`
      SELECT persist_resource_chunks(
        ${resourceId}::uuid, ${auditId}::uuid, ${"body-abc"}, ${chunksJson}::jsonb
      )
    `;
    expect(revId).toMatch(/^[0-9a-f-]{36}$/);

    const [rev] = await db<{ audit_id: string; body_hash: string; chunk_count: number }[]>`
      SELECT audit_id, body_hash, chunk_count FROM kb_resource_revisions WHERE id = ${revId}::uuid
    `;
    expect(rev.audit_id).toBe(auditId);
    expect(rev.body_hash).toBe("body-abc");
    expect(rev.chunk_count).toBe(1);
  });
});
```

- [ ] **Step 2: Run the test to verify it fails**

```bash
cd packages/temper-cloud && bun run test:integration -- store-revision
```

Expected: FAIL — `persist_resource_chunks` does not match the 4-arg call if the integration DB has not run the new migrations. If it passes here (because the migrations ARE applied), the test still catches regressions going forward. Continue.

- [ ] **Step 3: Update `api/workflows/process-upload.ts`**

Change the `processUpload` signature and `storeStep` to accept + use `profileId`:

```typescript
import { chunkFile } from "../../packages/temper-cloud/src/processing/chunk.js";
import { embedTexts } from "../../packages/temper-cloud/src/processing/embed.js";
import {
  chunksToJsonb,
  type ChunkRow,
} from "../../packages/temper-cloud/src/processing/store.js";
import { getDb } from "../../packages/temper-cloud/src/db.js";
import { insertEventAndAudit, DEVICE_ID_CLOUD } from "../../packages/temper-cloud/src/events.js";
import { createHash } from "node:crypto";
import { buildStatusUpdateQuery } from "../../packages/temper-cloud/src/upload.js";

export async function processUpload(
  blobFileId: string,
  blobUrl: string,
  resourceId: string,
  profileId: string,
  contextId: string,
) {
  "use workflow";

  const chunks = await chunkStep(blobUrl);
  const embeddings = await embedStep(chunks.map((c) => c.content));
  await storeStep(blobFileId, resourceId, profileId, contextId, chunks, embeddings);
}

// ... existing chunkStep, embedStep unchanged ...

async function storeStep(
  blobFileId: string,
  resourceId: string,
  profileId: string,
  contextId: string,
  chunks: Array<{ header_path: string; content: string; content_hash: string; chunk_index: number }>,
  embeddings: number[][],
): Promise<void> {
  "use step";

  const db = getDb();

  const body = chunks.map((c) => c.content).join("\n\n");
  const bodyHash = createHash("sha256").update(body).digest("hex");

  const { auditId } = await insertEventAndAudit(db, {
    profileId,
    deviceId: DEVICE_ID_CLOUD,
    contextId,
    resourceId,
    eventType: "body_updated",
    action: "update_body",
    bodyHash,
    managedHash: "",
    openHash: "",
  });

  if (chunks.length > 0) {
    const chunkRows: ChunkRow[] = chunks.map((chunk, i) => ({
      id: "",
      resource_id: resourceId,
      chunk_index: chunk.chunk_index,
      version: 0,
      header_path: chunk.header_path,
      content: chunk.content,
      content_hash: chunk.content_hash,
      embedding: embeddings[i],
    }));

    const chunksJson = JSON.stringify(chunksToJsonb(chunkRows));
    await db`
      SELECT persist_resource_chunks(
        ${resourceId}::uuid, ${auditId}::uuid, ${bodyHash}, ${chunksJson}::jsonb
      )
    `;
  }

  const statusQuery = buildStatusUpdateQuery(blobFileId, "processed", null);
  await db.query(statusQuery.sql, statusQuery.params);
}
```

- [ ] **Step 4: Update `api/upload.ts` to pass profile_id and context_id**

In `api/upload.ts`, the handler currently fetches `profileId` (line 62) and verifies resource visibility (line 70). Extend the visibility query to also return the context_id, and pass both into `processUpload`:

```typescript
  const visibleResources = await db<{ resource_id: string; kb_context_id: string }[]>`
    SELECT rv.resource_id, r.kb_context_id
      FROM resources_visible_to(${profileId}::uuid) rv
      JOIN kb_resources r ON r.id = rv.resource_id
     WHERE rv.resource_id = ${resourceId}::uuid
  `;
  if (visibleResources.length === 0) {
    return new Response(
      JSON.stringify({ error: "Resource not found or not accessible" }),
      { status: 404, headers: { "Content-Type": "application/json" } }
    );
  }
  const contextId = visibleResources[0].kb_context_id;

  // ... blob upload, insert blob_files (unchanged) ...

  try {
    await processUpload(blobFileId, blob.url, resourceId, profileId, contextId);
  } catch (err) {
    logger.error({ err }, "Failed to trigger processing workflow");
  }
```

- [ ] **Step 5: Update `api/workflows/process-ingest.ts`**

Replace the entire file contents with:

```typescript
import { chunkText } from "../../packages/temper-cloud/src/processing/chunk.js";
import { embedTexts } from "../../packages/temper-cloud/src/processing/embed.js";
import {
  chunksToJsonb,
  type ChunkRow,
} from "../../packages/temper-cloud/src/processing/store.js";
import { getDb } from "../../packages/temper-cloud/src/db.js";
import { insertEventAndAudit, DEVICE_ID_CLOUD } from "../../packages/temper-cloud/src/events.js";
import { createHash } from "node:crypto";

export async function processIngest(
  resourceId: string,
  markdown: string,
  profileId: string,
  contextId: string,
) {
  "use workflow";

  const chunks = await chunkStep(markdown);
  const embeddings = await embedStep(chunks.map((c) => c.content));
  await storeStep(resourceId, profileId, contextId, markdown, chunks, embeddings);
}

async function chunkStep(
  markdown: string,
): Promise<Array<{ header_path: string; content: string; content_hash: string; chunk_index: number }>> {
  "use step";
  return chunkText(markdown);
}

async function embedStep(texts: string[]): Promise<number[][]> {
  "use step";
  return embedTexts(texts);
}

async function storeStep(
  resourceId: string,
  profileId: string,
  contextId: string,
  bodyText: string,
  chunks: Array<{ header_path: string; content: string; content_hash: string; chunk_index: number }>,
  embeddings: number[][],
): Promise<void> {
  "use step";

  if (chunks.length === 0) return;

  const db = getDb();

  const bodyHash = `sha256:${createHash("sha256").update(bodyText).digest("hex")}`;

  const { auditId } = await insertEventAndAudit(db, {
    profileId,
    deviceId: DEVICE_ID_CLOUD,
    contextId,
    resourceId,
    eventType: "body_updated",
    action: "update_body",
    bodyHash,
    managedHash: "",
    openHash: "",
  });

  const chunkRows: ChunkRow[] = chunks.map((chunk, i) => ({
    id: "",
    resource_id: resourceId,
    chunk_index: chunk.chunk_index,
    version: 0,
    header_path: chunk.header_path,
    content: chunk.content,
    content_hash: chunk.content_hash,
    embedding: embeddings[i],
  }));

  const chunksJson = JSON.stringify(chunksToJsonb(chunkRows));
  await db`
    SELECT persist_resource_chunks(
      ${resourceId}::uuid, ${auditId}::uuid, ${bodyHash}, ${chunksJson}::jsonb
    )
  `;
}
```

`processIngest` is currently uncalled (verified with `rg processIngest`), so no call sites need updating. The signature mirrors `processUpload` so future wiring is trivial.

Then update `process-upload.ts` to also prefix the body hash with `sha256:` (to match `compute_body_hash` in Rust — see `crates/temper-core/src/hash.rs:22`). In Step 3 above, change:

```typescript
  const bodyHash = createHash("sha256").update(body).digest("hex");
```

to:

```typescript
  const bodyHash = `sha256:${createHash("sha256").update(body).digest("hex")}`;
```

- [ ] **Step 6: Run the Vitest integration test**

```bash
cd packages/temper-cloud && bun run test:integration -- store-revision
```

Expected: PASS.

- [ ] **Step 7: Run the TS typecheck + lint**

```bash
cd packages/temper-cloud && bun run typecheck && bun run check
```

Expected: no errors.

- [ ] **Step 8: Commit**

```bash
git add api/workflows/process-upload.ts \
        api/workflows/process-ingest.ts \
        api/upload.ts \
        packages/temper-cloud/src/processing/__tests__/store-revision.test.ts
git commit -m "feat(cloud): workflow mints audit and passes revision params"
```

---

### Task 7: Backfill migration for historical chunks

**Files:**
- Create: `migrations/20260420000007_backfill_revisions.sql`

- [ ] **Step 1: Write a failing test asserting backfill leaves no null `first_revision_id`**

Append to `crates/temper-api/tests/chunk_dedup_test.rs`:

```rust
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn backfill_assigns_first_revision_to_every_chunk(pool: PgPool) {
    // Any chunk existing after all migrations run must have first_revision_id set.
    // The sqlx::test harness applies migrations in order, including the backfill,
    // so seed-data chunks (inserted by earlier migrations) must be populated.
    let null_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM kb_chunks WHERE first_revision_id IS NULL",
    )
    .fetch_one(&pool).await.unwrap();
    assert_eq!(null_count, 0, "backfill must leave zero chunks with null first_revision_id");
}
```

- [ ] **Step 2: Run to verify it fails**

```bash
cargo nextest run -p temper-api --features test-db \
  -E 'binary(chunk_dedup_test) + test(backfill_assigns)' --no-capture
```

Expected: FAIL if seed data includes chunks (currently none — but the test still guards future seed additions). If the test passes with zero chunks in seed data, proceed; the test ensures forward invariants.

- [ ] **Step 3: Write the backfill migration**

Create `migrations/20260420000007_backfill_revisions.sql`:

```sql
-- Backfill kb_resource_revisions from existing kb_resource_audits (chunk-producing
-- actions only) and populate kb_chunks.first_revision_id / superseded_revision_id
-- from the audit timeline.
--
-- Strategy:
--   1. Synthesize one revision per (resource_id, audit) where action is chunk-
--      producing. Revision.created = audit.created so timeline lookups align.
--   2. For each chunk, pick the nearest-preceding revision (by created).
--   3. For non-current chunks, pick the earliest-following revision as the
--      superseder.
--   4. For any chunk still unassigned (resources with no chunk-producing audit
--      history — should be rare/zero pre-release), synthesize a revision from
--      the chunk's own created + content_hash.

BEGIN;

-- Step 1: synthesize revisions from chunk-producing audits.
INSERT INTO kb_resource_revisions (id, resource_id, audit_id, body_hash, chunk_count, created)
SELECT
    uuidv7(),
    a.resource_id,
    a.id,
    a.body_hash,
    0,            -- updated below
    a.created
  FROM kb_resource_audits a
 WHERE a.action IN ('create', 'update_body');

-- Step 2: chunks' first_revision_id = nearest-preceding revision.
UPDATE kb_chunks c
   SET first_revision_id = (
       SELECT r.id FROM kb_resource_revisions r
        WHERE r.resource_id = c.resource_id
          AND r.created <= c.created
        ORDER BY r.created DESC
        LIMIT 1
   );

-- Step 3: non-current chunks get earliest-following revision as superseder.
UPDATE kb_chunks c
   SET superseded_revision_id = (
       SELECT r.id FROM kb_resource_revisions r
        WHERE r.resource_id = c.resource_id
          AND r.created > c.created
        ORDER BY r.created ASC
        LIMIT 1
   )
 WHERE c.is_current = false;

-- Step 4: fallback — chunks with no preceding audit get a synthetic revision.
-- Batch by resource so we emit one revision per orphan-chunk cohort.
WITH orphans AS (
    SELECT DISTINCT resource_id, MIN(created) AS first_chunk_created,
           MIN(content_hash) AS sample_hash,
           COUNT(*) AS n
      FROM kb_chunks
     WHERE first_revision_id IS NULL
     GROUP BY resource_id
),
inserted AS (
    INSERT INTO kb_resource_revisions (id, resource_id, audit_id, body_hash, chunk_count, created)
    SELECT uuidv7(), o.resource_id, NULL, o.sample_hash, o.n, o.first_chunk_created
      FROM orphans o
    RETURNING id, resource_id
)
UPDATE kb_chunks c
   SET first_revision_id = i.id
  FROM inserted i
 WHERE c.resource_id = i.resource_id
   AND c.first_revision_id IS NULL;

-- Step 5: recompute chunk_count per revision now that all links exist.
UPDATE kb_resource_revisions r
   SET chunk_count = (
       SELECT COUNT(*) FROM kb_chunks c
        WHERE c.first_revision_id = r.id
   );

COMMIT;
```

- [ ] **Step 4: Regenerate sqlx cache**

```bash
cargo sqlx prepare --workspace -- --all-features
```

- [ ] **Step 5: Run the test suite**

```bash
cargo nextest run -p temper-api --features test-db \
  -E 'binary(chunk_dedup_test)' --no-capture
```

Expected: `backfill_assigns_first_revision_to_every_chunk` PASSES. All earlier dedup tests still PASS.

- [ ] **Step 6: Commit**

```bash
git add migrations/20260420000007_backfill_revisions.sql \
        crates/temper-api/tests/chunk_dedup_test.rs \
        .sqlx/
git commit -m "migrate(db): backfill kb_resource_revisions and chunk revision columns"
```

---

### Task 8: Tighten `kb_chunks.first_revision_id` to NOT NULL

**Files:**
- Create: `migrations/20260420000008_first_revision_id_not_null.sql`

- [ ] **Step 1: Write the failing constraint test**

Append to `crates/temper-api/tests/chunk_dedup_test.rs`:

```rust
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn kb_chunks_first_revision_id_is_not_null(pool: PgPool) {
    let is_nullable: String = sqlx::query_scalar(
        "SELECT is_nullable FROM information_schema.columns \
         WHERE table_name = 'kb_chunks' AND column_name = 'first_revision_id'",
    )
    .fetch_one(&pool).await.unwrap();
    assert_eq!(is_nullable, "NO");
}
```

- [ ] **Step 2: Run to verify it fails**

```bash
cargo nextest run -p temper-api --features test-db \
  -E 'binary(chunk_dedup_test) + test(kb_chunks_first_revision)' --no-capture
```

Expected: FAIL — column still nullable (`is_nullable = 'YES'`).

- [ ] **Step 3: Write the migration**

Create `migrations/20260420000008_first_revision_id_not_null.sql`:

```sql
-- Post-backfill: every chunk now has first_revision_id populated. Tighten the
-- column to NOT NULL so future inserts are forced to supply it.
ALTER TABLE kb_chunks
    ALTER COLUMN first_revision_id SET NOT NULL;
```

- [ ] **Step 4: Regenerate sqlx cache**

```bash
cargo sqlx prepare --workspace -- --all-features
```

- [ ] **Step 5: Run the test**

```bash
cargo nextest run -p temper-api --features test-db \
  -E 'binary(chunk_dedup_test)' --no-capture
```

Expected: all chunk_dedup_test tests PASS.

- [ ] **Step 6: Commit**

```bash
git add migrations/20260420000008_first_revision_id_not_null.sql .sqlx/
git commit -m "chore(db): set kb_chunks.first_revision_id NOT NULL after backfill"
```

---

### Task 9: `resource_chunks_at_revision` point-in-time reconstruction

**Files:**
- Create: `migrations/20260420000009_resource_chunks_at_revision.sql`
- Create: `crates/temper-api/tests/revision_retention_test.rs`

- [ ] **Step 1: Write the failing point-in-time tests**

Create `crates/temper-api/tests/revision_retention_test.rs`:

```rust
#![cfg(feature = "test-db")]

use serde_json::json;
use sqlx::PgPool;
use uuid::Uuid;

// Re-use helpers from chunk_dedup_test via direct copy to avoid cross-test-file coupling.
// (Small DRY violation; worth it to keep test binaries independent.)
async fn seed_resource(pool: &PgPool) -> Uuid {
    let profile_id: Uuid = sqlx::query_scalar(
        "INSERT INTO kb_profiles (id, handle, created) \
         VALUES (gen_random_uuid(), 'rev-test', now()) RETURNING id",
    ).fetch_one(pool).await.unwrap();
    let context_id: Uuid = sqlx::query_scalar(
        "INSERT INTO kb_contexts (id, owner_profile_id, name, created) \
         VALUES (gen_random_uuid(), $1, 'ctx', now()) RETURNING id",
    ).bind(profile_id).fetch_one(pool).await.unwrap();
    let doc_type_id: Uuid = sqlx::query_scalar(
        "SELECT id FROM kb_doc_types WHERE name = 'note' LIMIT 1",
    ).fetch_one(pool).await.unwrap();
    sqlx::query_scalar(
        "INSERT INTO kb_resources (id, kb_context_id, kb_doc_type_id, origin_uri, title, slug, \
             originator_profile_id, owner_profile_id, created, updated) \
         VALUES (gen_random_uuid(), $1, $2, 'rev://r', 'T', 't', $3, $3, now(), now()) RETURNING id",
    ).bind(context_id).bind(doc_type_id).bind(profile_id).fetch_one(pool).await.unwrap()
}

async fn seed_audit(pool: &PgPool, resource_id: Uuid, body_hash: &str) -> Uuid {
    sqlx::query_scalar(
        "INSERT INTO kb_resource_audits \
         (resource_id, event_id, profile_id, device_id, body_hash, managed_hash, open_hash, action) \
         VALUES ($1, gen_random_uuid(), \
             (SELECT originator_profile_id FROM kb_resources WHERE id = $1), \
             'test', $2, 'mh', 'oh', 'update_body') RETURNING id",
    ).bind(resource_id).bind(body_hash).fetch_one(pool).await.unwrap()
}

fn chunk(index: i32, content: &str, hash: &str) -> serde_json::Value {
    let zeros: Vec<f32> = vec![0.0; 768];
    let emb = format!("[{}]", zeros.iter().map(f32::to_string).collect::<Vec<_>>().join(","));
    json!({ "chunk_index": index, "header_path": "", "heading_depth": 0,
            "content": content, "content_hash": hash, "embedding": emb })
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn resource_chunks_at_revision_returns_original_state(pool: PgPool) {
    let rid = seed_resource(&pool).await;

    let a1 = seed_audit(&pool, rid, "b1").await;
    let r1: Uuid = sqlx::query_scalar("SELECT persist_resource_chunks($1, $2, $3, $4)")
        .bind(rid).bind(a1).bind("b1")
        .bind(json!([chunk(0, "ORIG-0", "o0"), chunk(1, "ORIG-1", "o1")]))
        .fetch_one(&pool).await.unwrap();

    let a2 = seed_audit(&pool, rid, "b2").await;
    let _r2: Uuid = sqlx::query_scalar("SELECT replace_resource_chunks($1, $2, $3, $4)")
        .bind(rid).bind(a2).bind("b2")
        .bind(json!([chunk(0, "ORIG-0", "o0"), chunk(1, "NEW-1", "n1")]))
        .fetch_one(&pool).await.unwrap();

    let rows: Vec<(i32, String)> = sqlx::query_as(
        "SELECT chunk_index, content FROM resource_chunks_at_revision($1, $2) ORDER BY chunk_index",
    ).bind(rid).bind(r1).fetch_all(&pool).await.unwrap();

    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0], (0, "ORIG-0".to_string()));
    assert_eq!(rows[1], (1, "ORIG-1".to_string()));
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn resource_chunks_at_revision_unknown_returns_empty(pool: PgPool) {
    let rid = seed_resource(&pool).await;
    let rows: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM resource_chunks_at_revision($1, gen_random_uuid())",
    ).bind(rid).fetch_one(&pool).await.unwrap();
    assert_eq!(rows, 0);
}
```

- [ ] **Step 2: Run to verify it fails**

```bash
cargo nextest run -p temper-api --features test-db \
  -E 'binary(revision_retention_test)' --no-capture
```

Expected: FAIL — `resource_chunks_at_revision` does not exist.

- [ ] **Step 3: Write the migration**

Create `migrations/20260420000009_resource_chunks_at_revision.sql`:

```sql
-- Point-in-time reconstruction: return the chunks as they existed at a given
-- revision. A chunk is live at revision R when:
--   * its first_revision_id was created at or before R, AND
--   * it was either never superseded, or superseded strictly after R.
CREATE OR REPLACE FUNCTION resource_chunks_at_revision(
    p_resource_id UUID,
    p_revision_id UUID
) RETURNS TABLE(
    id UUID, chunk_index INT, header_path TEXT, heading_depth SMALLINT,
    content TEXT, content_hash VARCHAR(64), embedding vector(768), version INT
)
LANGUAGE sql STABLE AS $$
    WITH target AS (
        SELECT created FROM kb_resource_revisions
         WHERE id = p_revision_id AND resource_id = p_resource_id
    )
    SELECT c.id, c.chunk_index, c.header_path, c.heading_depth,
           cc.content, c.content_hash, c.embedding, c.version
      FROM kb_chunks c
      JOIN kb_chunk_content cc ON cc.chunk_id = c.id
      JOIN kb_resource_revisions first_rev ON first_rev.id = c.first_revision_id
      LEFT JOIN kb_resource_revisions sup_rev ON sup_rev.id = c.superseded_revision_id
     WHERE c.resource_id = p_resource_id
       AND first_rev.created <= (SELECT created FROM target)
       AND (sup_rev.id IS NULL OR sup_rev.created > (SELECT created FROM target))
     ORDER BY c.chunk_index;
$$;
```

- [ ] **Step 4: Regenerate sqlx cache and run tests**

```bash
cargo sqlx prepare --workspace -- --all-features
cargo nextest run -p temper-api --features test-db \
  -E 'binary(revision_retention_test)' --no-capture
```

Expected: both tests PASS.

- [ ] **Step 5: Commit**

```bash
git add migrations/20260420000009_resource_chunks_at_revision.sql \
        crates/temper-api/tests/revision_retention_test.rs \
        .sqlx/
git commit -m "feat(db): resource_chunks_at_revision point-in-time function"
```

---

### Task 10: `sweep_orphaned_revisions` retention sweep

**Files:**
- Create: `migrations/20260420000010_sweep_orphaned_revisions.sql`
- Modify: `crates/temper-api/tests/revision_retention_test.rs`

- [ ] **Step 1: Add failing retention tests**

Append to `crates/temper-api/tests/revision_retention_test.rs`:

```rust
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn sweep_skips_revisions_with_live_chunks(pool: PgPool) {
    let rid = seed_resource(&pool).await;
    let a1 = seed_audit(&pool, rid, "b1").await;
    let r1: Uuid = sqlx::query_scalar("SELECT persist_resource_chunks($1, $2, $3, $4)")
        .bind(rid).bind(a1).bind("b1").bind(json!([chunk(0, "x", "hx")]))
        .fetch_one(&pool).await.unwrap();

    // Set created far in the past so it would be age-eligible.
    sqlx::query("UPDATE kb_resource_revisions SET created = now() - interval '120 days' WHERE id = $1")
        .bind(r1).execute(&pool).await.unwrap();

    let deleted: i32 = sqlx::query_scalar(
        "SELECT sweep_orphaned_revisions(0, 90)",
    ).fetch_one(&pool).await.unwrap();

    assert_eq!(deleted, 0, "revision referenced by live chunk (first_revision_id) must not be deleted");
    let still_there: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM kb_resource_revisions WHERE id = $1)",
    ).bind(r1).fetch_one(&pool).await.unwrap();
    assert!(still_there);
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn sweep_respects_keep_last_n(pool: PgPool) {
    let rid = seed_resource(&pool).await;

    // Synthesize 15 orphan revisions (no chunks, so referential pin does not apply).
    for i in 0..15 {
        sqlx::query(
            "INSERT INTO kb_resource_revisions (id, resource_id, audit_id, body_hash, chunk_count, created) \
             VALUES (uuidv7(), $1, NULL, $2, 0, now() - ($3::int || ' days')::interval)",
        )
        .bind(rid)
        .bind(format!("b{i}"))
        .bind(200 - i)  // older first
        .execute(&pool).await.unwrap();
    }

    let deleted: i32 = sqlx::query_scalar(
        "SELECT sweep_orphaned_revisions(10, 0)",
    ).fetch_one(&pool).await.unwrap();

    assert_eq!(deleted, 5, "should delete 5 of 15 when keep_last_n = 10");

    let remaining: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM kb_resource_revisions WHERE resource_id = $1",
    ).bind(rid).fetch_one(&pool).await.unwrap();
    assert_eq!(remaining, 10);
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn sweep_respects_age_ceiling(pool: PgPool) {
    let rid = seed_resource(&pool).await;
    for i in 0..5 {
        sqlx::query(
            "INSERT INTO kb_resource_revisions (id, resource_id, audit_id, body_hash, chunk_count, created) \
             VALUES (uuidv7(), $1, NULL, $2, 0, now() - ($3::int || ' days')::interval)",
        )
        .bind(rid)
        .bind(format!("b{i}"))
        .bind(i * 5)  // all younger than 90 days
        .execute(&pool).await.unwrap();
    }
    let deleted: i32 = sqlx::query_scalar(
        "SELECT sweep_orphaned_revisions(0, 90)",
    ).fetch_one(&pool).await.unwrap();
    assert_eq!(deleted, 0);
}
```

- [ ] **Step 2: Run to verify they fail**

```bash
cargo nextest run -p temper-api --features test-db \
  -E 'binary(revision_retention_test) + test(sweep_)' --no-capture
```

Expected: FAIL — `sweep_orphaned_revisions` does not exist.

- [ ] **Step 3: Write the migration**

Create `migrations/20260420000010_sweep_orphaned_revisions.sql`:

```sql
-- Retention sweep. Three dials, evaluated jointly:
--   1. Referential pin (enforced by FKs): a revision referenced by any
--      kb_chunks.first_revision_id or .superseded_revision_id cannot be
--      deleted. We pre-filter candidates to skip the DELETE attempt.
--   2. Per-resource keep-last-N: the N most recent revisions per resource
--      are pinned regardless of age.
--   3. Age ceiling: revisions younger than p_age_ceiling_days are pinned.
--
-- Returns the count of revisions deleted.
CREATE OR REPLACE FUNCTION sweep_orphaned_revisions(
    p_keep_last_n      INT DEFAULT 10,
    p_age_ceiling_days INT DEFAULT 90
) RETURNS INT
LANGUAGE plpgsql AS $$
DECLARE
    v_deleted INT;
BEGIN
    WITH ranked AS (
        SELECT r.id,
               r.resource_id,
               r.created,
               row_number() OVER (PARTITION BY r.resource_id ORDER BY r.created DESC) AS rn
          FROM kb_resource_revisions r
    ),
    candidates AS (
        SELECT r.id
          FROM ranked r
         WHERE r.rn > p_keep_last_n
           AND r.created < now() - (p_age_ceiling_days || ' days')::interval
           AND NOT EXISTS (SELECT 1 FROM kb_chunks c WHERE c.first_revision_id = r.id)
           AND NOT EXISTS (SELECT 1 FROM kb_chunks c WHERE c.superseded_revision_id = r.id)
    ),
    deleted AS (
        DELETE FROM kb_resource_revisions
         WHERE id IN (SELECT id FROM candidates)
        RETURNING id
    )
    SELECT COUNT(*)::INT INTO v_deleted FROM deleted;

    RETURN v_deleted;
END;
$$;
```

- [ ] **Step 4: Regenerate sqlx cache and run tests**

```bash
cargo sqlx prepare --workspace -- --all-features
cargo nextest run -p temper-api --features test-db \
  -E 'binary(revision_retention_test)' --no-capture
```

Expected: all retention tests PASS.

- [ ] **Step 5: Commit**

```bash
git add migrations/20260420000010_sweep_orphaned_revisions.sql \
        crates/temper-api/tests/revision_retention_test.rs \
        .sqlx/
git commit -m "feat(db): sweep_orphaned_revisions retention function"
```

---

### Task 11: Full-suite verification

**Files:** (no new files)

- [ ] **Step 1: Run the full quality gate**

```bash
cargo make check
```

Expected: Rust fmt + clippy + docs + machete clean; TS typecheck + biome clean.

- [ ] **Step 2: Run all Rust tests including integration**

```bash
cargo make test-db
```

Expected: all tests PASS except the two pre-existing HF TLS failures (`sync_run_push_body_round_trip`, `graph_build_then_sync_materializes_edges`). If additional tests fail, investigate — likely a meta/chunk invariant breach or a signature mismatch somewhere not covered by the targeted runs.

- [ ] **Step 3: Run TS tests**

```bash
cargo make ts-test
```

Expected: PASS.

- [ ] **Step 4: Verify `sync_test.rs:1527` meta-only invariant**

The existing assertion `meta-only push must not touch kb_chunks rows` should still hold. Extend coverage to the new revisions table with a before/after pair in the same test (find it in `tests/e2e/tests/sync_test.rs` — search for the string `"meta-only push must not touch kb_chunks rows"`):

Before the meta-only push, capture the revision count:

```rust
let revisions_before: i64 = sqlx::query_scalar(
    "SELECT COUNT(*) FROM kb_resource_revisions WHERE resource_id = $1",
)
.bind(uuid::Uuid::from(r1.id))
.fetch_one(&pool).await
.expect("fetch revisions before");
```

After the meta-only push (right after the existing `chunks_after` assertion):

```rust
let revisions_after: i64 = sqlx::query_scalar(
    "SELECT COUNT(*) FROM kb_resource_revisions WHERE resource_id = $1",
)
.bind(uuid::Uuid::from(r1.id))
.fetch_one(&pool).await
.expect("fetch revisions after");
assert_eq!(revisions_after, revisions_before,
    "meta-only push must not create a new kb_resource_revisions row");
```

Run the e2e test to confirm:

```bash
cargo nextest run -p temper-e2e --features test-db \
  -E 'test(sync)' --no-capture
```

Expected: PASS (modulo the two pre-existing HF TLS failures).

- [ ] **Step 5: Commit any final adjustments**

```bash
git status    # verify clean or only intentional tweaks pending
# if there are changes:
git add <paths>
git commit -m "test(e2e): assert meta-only push does not create revisions"
```

- [ ] **Step 6: Push the branch**

```bash
git push origin claude/continue-analysis-migrations-Sbctd
```

Expected: push succeeds; branch now contains C1–C6 + revisions Phase B+C.

---

## Rollout Notes

- All migrations on one branch. Local `cargo make docker-up && cargo make test-db` verifies them as a set.
- No production deploy target in this plan — the rollup PR is opened from this branch at completion time.
- The single breaking change to SQL function signatures (`persist_resource_chunks` / `replace_resource_chunks`) is acceptable pre-release; the plan assumes no external consumers of these functions beyond the repo.
- The `insert_event_and_audit` Rust helper changing its return type from `()` to `EventAuditIds` affects only `ingest_service.rs` and `meta_service.rs` — grep-verified during Task 5.
