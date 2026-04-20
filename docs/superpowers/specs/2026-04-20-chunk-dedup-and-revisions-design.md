# Chunk Dedup + `kb_resource_revisions` Design

**Date:** 2026-04-20
**Status:** Accepted
**Branch:** `claude/continue-analysis-migrations-Sbctd`
**Companion plan:** [`docs/superpowers/plans/2026-04-20-chunk-dedup-and-revisions.md`](../plans/2026-04-20-chunk-dedup-and-revisions.md)
**Related code-review:** [`docs/code-reviews/2026-04-20-graph-performance-audit.md`](../../code-reviews/2026-04-20-graph-performance-audit.md)

## Problem

Two bugs, one shared fix.

**(1) `replace_resource_chunks` bloats `kb_chunks` on every body update.** The current implementation (`migrations/20260411000001_add_heading_depth_to_chunks.sql:66`) unconditionally flips every `is_current=true` row to `false`, then re-inserts every chunk at `version = MAX(version) + 1`. Rewriting a 40-chunk document with a one-word edit in section 3 produces 40 new rows and demotes 40 old ones — 79 of those 80 writes encode no information. The HNSW index on `kb_chunks.embedding` re-builds at a cost proportional to churn, not to change.

**(2) There is no body-version anchor that other subsystems can reference.** Sessions, audits, and events can name a `resource_id` but cannot pin the specific body revision they were working against. This matters once the vault has been edited multiple times — a session note that says "I was looking at the onboarding guide" is underspecified by the time the guide has moved on four revisions.

The fix for (1) is content-hash-keyed dedup inside `replace_resource_chunks`. That dedup naturally produces a version boundary per body update, and once that boundary has an identity, (2) is a one-table addition. Phases B and C land them together because the chunk columns that make dedup auditable are the same columns that Phase C's point-in-time reconstruction reads.

## Scope

- **Phase A (skipped, per user direction)** — would have added cheap body-hash early-exit in `replace_resource_chunks`. Unnecessary pre-release; Phase B subsumes it with finer granularity.
- **Phase B** — `kb_resource_revisions` table, chunk columns (`first_revision_id`, `superseded_revision_id`), dedup-aware `replace_resource_chunks`, `persist_resource_chunks` revision linkage, audit thread-through, backfill of historical chunks.
- **Phase C** — `resource_chunks_at_revision(p_resource_id, p_revision_id)` SQL function, retention sweep (pin from referring rows, keep-last-N per resource, age ceiling).

## First Principles

### Revisions are not audits

The temptation is to reuse `kb_resource_audits` as the revision anchor. Resist it. The two answer different questions:

| | Audit | Revision |
|--|-------|----------|
| Question answered | "Who did what, when?" | "Which bytes were the chunks derived from?" |
| Produced by `update_meta`? | Yes — every managed-meta edit | No — managed-meta edits produce no chunks |
| Exists without chunks? | Yes (metadata-only audits) | No (definitionally tied to a chunk set) |
| Cardinality to a resource | 1-to-many, includes meta-only | 1-to-many, body-producing only |

A revision is a physical anchor: "this is the `(resource_id, body_hash)` pair that produced chunks X through Y." An audit is a causal record. They commonly co-occur — every `action IN ('create', 'update_body')` audit in the Rust path produces exactly one revision — but their identities are different and their lifecycles differ (an audit can be preserved after its revision is garbage-collected, and a revision can exist with `audit_id = NULL` if the audit was later deleted or if the revision was produced by an async workflow that minted its own audit separately).

### Dedup is keyed on `(chunk_index, content_hash)`

`content_hash = sha256_hex(trimmed content)` — computed in `crates/temper-ingest/src/chunk.rs:343`. It captures the chunk's body text but **not** `header_path` or `heading_depth`. A section-heading rename that leaves body text untouched will not invalidate dedup — the chunk stays on its original revision with the old `header_path`, and a search against the new heading text will not find it.

This is acceptable pre-release. The alternative (hashing header + body together) would invalidate dedup on every heading-depth bump upstream of a section, which happens whenever the author inserts a top-level heading. The `(chunk_index, content_hash)` key prioritizes dedup stability over header-change visibility; the tradeoff is documented here so the decision surfaces in the next review rather than becoming a footgun.

### Invariant preserved: meta-only pushes do not touch chunks

The e2e test at `tests/e2e/tests/sync_test.rs:1527` asserts:

```rust
assert_eq!(chunks_after, chunks_before, "meta-only push must not touch kb_chunks rows");
```

Phase B must preserve this. The meta path (`meta_service.rs:222`, action `update_meta`) never calls `persist_chunks`/`replace_chunks` and never creates a revision. Revisions only exist for chunk-producing actions.

## Schema

### `kb_resource_revisions`

```sql
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
```

- `id` is uuidv7, generated caller-side — preserves the project's time-sortable ID convention.
- `audit_id` nullable + `ON DELETE SET NULL` — revisions outlive their audits. A retention sweep of `kb_resource_audits` will not cascade into chunk loss.
- `body_hash` denormalized for fast "has this content been seen?" lookups during backfill and retention.
- `chunk_count` cached for retention policy (e.g. "keep revisions whose chunk_count > 0"). Does not drift because the revision is immutable after creation.

### `kb_chunks` column additions

```sql
ALTER TABLE kb_chunks
    ADD COLUMN first_revision_id      UUID REFERENCES kb_resource_revisions(id) ON DELETE RESTRICT,
    ADD COLUMN superseded_revision_id UUID REFERENCES kb_resource_revisions(id) ON DELETE RESTRICT;

CREATE INDEX idx_kb_chunks_first_revision
    ON kb_chunks(first_revision_id);
CREATE INDEX idx_kb_chunks_superseded_revision
    ON kb_chunks(superseded_revision_id)
    WHERE superseded_revision_id IS NOT NULL;
```

- `first_revision_id` — the revision at which this chunk's `(chunk_index, content_hash)` pair first appeared. For a chunk whose content survives unchanged across revisions R3 → R4 → R5, `first_revision_id = R3`.
- `superseded_revision_id` — the revision at which this chunk was replaced (or removed). `NULL` means the chunk is still current at head. When `is_current = false`, this column must be non-NULL.
- `ON DELETE RESTRICT` — a revision cannot be deleted while any chunk still references it. This is the referential pin the retention sweep must respect.

Once backfill (below) completes, we tighten:

```sql
ALTER TABLE kb_chunks ALTER COLUMN first_revision_id SET NOT NULL;
```

### Invariant set after Phase B lands

1. `kb_chunks.first_revision_id IS NOT NULL` for every row.
2. `kb_chunks.is_current = true ⇔ kb_chunks.superseded_revision_id IS NULL`.
3. `(resource_id, chunk_index, version)` remains UNIQUE (existing constraint).
4. For any `(resource_id, chunk_index)`, at most one row has `is_current = true` (existing; unchanged).
5. Every revision row referenced by any chunk has `chunk_count > 0`.

## Phase B — Implementation

### B1. Dedup-aware `replace_resource_chunks`

New signature:

```sql
CREATE OR REPLACE FUNCTION replace_resource_chunks(
    p_resource_id UUID,
    p_audit_id    UUID,        -- may be NULL for TS workflow path
    p_body_hash   TEXT,
    p_chunks      JSONB
) RETURNS UUID  -- revision_id
```

Returns the newly-created revision id rather than a count. The count is derivable from `jsonb_array_length(p_chunks)` (chunk_count) and from `kb_current_chunks` after the call; returning the revision id is what callers actually need for downstream writes.

Algorithm (all in one CTE chain, one pass over `kb_chunks`):

1. `new_revision` — INSERT a row into `kb_resource_revisions` with `gen_uuidv7()`, `p_resource_id`, `p_audit_id`, `p_body_hash`, `chunk_count = jsonb_array_length(p_chunks)`. RETURNING id AS `v_revision_id`.
2. `incoming` — materialize `jsonb_array_elements(p_chunks)` into `(chunk_index, content_hash, header_path, heading_depth, content, embedding)`.
3. `existing` — `SELECT c.id, c.chunk_index, c.content_hash FROM kb_chunks c WHERE c.resource_id = p_resource_id AND c.is_current = true`.
4. `preserve` — `existing` ⋈ `incoming` on `(chunk_index, content_hash)`. These rows stay unchanged: neither `is_current` nor `superseded_revision_id` moves. `first_revision_id` is not touched. This is the dedup payoff.
5. `supersede` — `existing LEFT JOIN incoming ON chunk_index` where either no match (removed position) or `content_hash` differs (replaced content). `UPDATE kb_chunks SET is_current = false, superseded_revision_id = v_revision_id WHERE id IN supersede.id`.
6. `insert_new` — `incoming LEFT JOIN existing ON (chunk_index, content_hash)` where no match **and** no preserved row covers this position. Insert new rows with `first_revision_id = v_revision_id`, `is_current = true`, `version = COALESCE((SELECT MAX(version) FROM kb_chunks WHERE resource_id = p_resource_id AND chunk_index = <this index>), 0) + 1`.
7. `insert_content` — parallel INSERT into `kb_chunk_content` for the newly-inserted chunk ids.
8. `rebuild_resource_search_vector(p_resource_id)`.
9. `RETURN v_revision_id`.

Notes:

- `PERFORM set_config('temper.skip_search_rebuild', 'true', true)` around the body, matching the current function's pattern.
- The function is still one round-trip from the caller. It produces multiple SQL statements internally, but all inside a single PL/pgSQL body.
- If `p_chunks = '[]'::jsonb`, the function still creates a revision (with `chunk_count = 0`) and supersedes every existing current chunk. This is the "body became empty" case — semantically meaningful, not a no-op.

### B2. `persist_resource_chunks` (create path)

New signature:

```sql
CREATE OR REPLACE FUNCTION persist_resource_chunks(
    p_resource_id UUID,
    p_audit_id    UUID,
    p_body_hash   TEXT,
    p_chunks      JSONB
) RETURNS UUID  -- revision_id
```

Create-path variant. No existing chunks to dedup against. Creates the revision, inserts all chunks with `first_revision_id = v_revision_id`, `version = 1`, `is_current = true`, `superseded_revision_id = NULL`. Returns `v_revision_id`.

### B3. Rust caller changes

Two services need thread-through:

**`crates/temper-api/src/services/ingest_service.rs::create_resource_with_manifest`** (around `:341-362`):

- Capture `audit_id` from `insert_event_and_audit` (already returns it — see `migrations/20260406000002_insert_event_and_audit.sql:24`).
- Pass `audit_id` and `content_hash` into `persist_chunks`.
- `persist_chunks` signature gains `audit_id: Uuid, body_hash: &str`; returns `Result<RevisionId, ApiError>` instead of the chunk count.

**`crates/temper-api/src/services/ingest_service.rs::update`** (around `:668-687`):

- `update_resource_manifest` must return the `audit_id` it produced (currently returns `()`). Callers unwrap the return.
- `replace_chunks` signature gains `audit_id: Uuid, body_hash: &str`; returns `Result<RevisionId, ApiError>`.

New type in `crates/temper-core/src/types/ingest.rs`:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
pub struct RevisionId(pub Uuid);
```

Follows the pattern of `ResourceId`, `ProfileId`, etc. Has `Deref<Target = Uuid>` and conversions.

### B4. TS workflow changes

`api/workflows/process-upload.ts:storeStep` (and the currently-unused `process-ingest.ts:storeStep`) currently call `persist_resource_chunks(resource_id, chunks_json)`. After Phase B:

1. Compute `body_hash = sha256(extracted_text)` inside `storeStep`.
2. Call `insert_event_and_audit` to produce an audit row (action `update_body`, event_type `body_updated`, device_id `cloud-workflow`). The workflow has a `profile_id` available because resource access was verified in `api/upload.ts:62-79` before the workflow was triggered; pass the profile_id via workflow parameters.
3. Call `persist_resource_chunks(resource_id, audit_id, body_hash, chunks_json)`.

This means `api/upload.ts` must pass `profile_id` to `processUpload` alongside `blob_file_id`, `blob_url`, `resource_id`. It already has `profileId` in scope at line 62.

We intentionally choose "workflow mints its own audit" over "invoker mints the audit and passes `audit_id`" because the body_hash is not known at invocation time — it's produced by the extraction step. Letting the workflow create the audit keeps each layer authoritative for the data it actually computes.

### B5. Backfill

One-shot migration statement. For each existing `kb_chunks` row, the backfilled `first_revision_id` must satisfy:

1. `first_revision.resource_id = chunk.resource_id`.
2. `first_revision.created <= chunk.created`.
3. Nearest-preceding: `first_revision.created = MAX(revision.created) WHERE created <= chunk.created`.

Since `kb_resource_revisions` is brand new, Step 1 of backfill synthesizes one revision per existing `kb_resource_audits` row where `action IN ('create', 'update_body')`, using the audit's `body_hash`, `created`, and `id`:

```sql
INSERT INTO kb_resource_revisions (id, resource_id, audit_id, body_hash, chunk_count, created)
SELECT gen_uuidv7(), a.resource_id, a.id, a.body_hash,
       (SELECT COUNT(*) FROM kb_chunks c
         WHERE c.resource_id = a.resource_id
           AND c.created <= a.created
           AND NOT EXISTS (SELECT 1 FROM kb_resource_audits a2
                            WHERE a2.resource_id = a.resource_id
                              AND a2.action IN ('create', 'update_body')
                              AND a2.created > a.created
                              AND a2.created <= c.created)),
       a.created
  FROM kb_resource_audits a
 WHERE a.action IN ('create', 'update_body');
```

Step 2 of backfill assigns `first_revision_id` per chunk:

```sql
UPDATE kb_chunks c
   SET first_revision_id = (
       SELECT r.id FROM kb_resource_revisions r
        WHERE r.resource_id = c.resource_id
          AND r.created <= c.created
        ORDER BY r.created DESC
        LIMIT 1
   );
```

Step 3 assigns `superseded_revision_id` for non-current chunks using the earliest subsequent revision:

```sql
UPDATE kb_chunks c
   SET superseded_revision_id = (
       SELECT r.id FROM kb_resource_revisions r
        WHERE r.resource_id = c.resource_id
          AND r.created > c.created
        ORDER BY r.created ASC
        LIMIT 1
   )
 WHERE c.is_current = false;
```

Step 4: defensively, any chunk still NULL on `first_revision_id` — because its resource has no chunk-producing audit history — synthesizes a placeholder revision from the chunk's own `created` and `content_hash`. This should be zero rows in the local dev DB but is cheap insurance.

Step 5: `ALTER TABLE kb_chunks ALTER COLUMN first_revision_id SET NOT NULL;`.

## Phase C — Point-in-Time + Retention

### C1. `resource_chunks_at_revision`

```sql
CREATE OR REPLACE FUNCTION resource_chunks_at_revision(
    p_resource_id UUID,
    p_revision_id UUID
) RETURNS TABLE(
    id UUID, chunk_index INT, header_path TEXT, heading_depth SMALLINT,
    content TEXT, content_hash VARCHAR(64), embedding vector(768), version INT
)
LANGUAGE sql STABLE AS $$
    WITH target AS (
        SELECT created FROM kb_resource_revisions WHERE id = p_revision_id AND resource_id = p_resource_id
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

"Give me the chunks as they existed at revision X." A chunk is live at revision R if it was created at or before R, and either was never superseded or was superseded after R.

### C2. Retention sweep

Three dials, evaluated together:

**Pin 1 — Referring rows.** Never delete a revision that is referenced by:

- `kb_chunks.first_revision_id` (enforced by `ON DELETE RESTRICT`)
- `kb_chunks.superseded_revision_id` (same)
- Future referrers: `kb_sessions.anchor_revision_id`, `kb_resource_audits.revision_id` if those columns are later added. The sweep logic reads `pg_depend` dynamically so new referrers are respected without code changes.

**Pin 2 — Keep last N per resource.** Default N = 10. Revisions ranked by `created DESC` per `resource_id`; the top N are pinned regardless of age.

**Pin 3 — Age ceiling.** Default 90 days. A revision older than the ceiling is eligible for sweep only if Pin 1 and Pin 2 also clear. A revision newer than the ceiling is pinned regardless of Pin 2.

Sweep candidate = `NOT pin1 AND NOT pin2 AND older_than_ceiling`. The sweep function:

```sql
CREATE OR REPLACE FUNCTION sweep_orphaned_revisions(
    p_keep_last_n INT DEFAULT 10,
    p_age_ceiling_days INT DEFAULT 90
) RETURNS INT  -- number of revisions deleted
```

It runs the candidate query, `DELETE FROM kb_resource_revisions WHERE id = ANY(...)`, returns the count.

**Cascade from sweep:** a deleted revision sets `kb_chunks.superseded_revision_id` callers and `kb_resource_audits.audit_id` to `NULL` (per `ON DELETE SET NULL`). It cannot delete a revision with a live `first_revision_id` referrer (per `ON DELETE RESTRICT`) — this is enforced, not defensive. In practice the only deletable revisions are those whose chunks have already rolled off (all chunks that had this revision as `first_revision_id` were later themselves superseded and then garbage-collected by an earlier sweep — but we don't garbage-collect chunks in Phase C; chunks are permanent for the life of their resource unless the resource is deleted).

Phase C ships the function but does not schedule it. Scheduling (Vercel cron or similar) is a follow-up.

## Rollout Order

One branch, commits in order:

1. `feat(db): add kb_resource_revisions table and chunk revision columns` (schema only; no function changes yet)
2. `feat(db): dedup-aware replace_resource_chunks with revision linkage` (B1 + B2 SQL)
3. `feat(api): thread audit_id through ingest_service persist/replace_chunks` (B3)
4. `feat(cloud): workflow mints audit + passes revision params to persist_resource_chunks` (B4)
5. `migrate(db): backfill kb_resource_revisions and kb_chunks revision columns` (B5)
6. `chore(db): set kb_chunks.first_revision_id NOT NULL` (post-backfill tightening)
7. `feat(db): resource_chunks_at_revision point-in-time reconstruction` (C1)
8. `feat(db): sweep_orphaned_revisions retention function` (C2)

Steps 1–6 must land together (the schema + backfill set is not independently deployable). Steps 7–8 are independent additions with no schema risk.

## Tests

Each test pairs with the commit that introduces the code it exercises.

### B1/B2 SQL tests (new, `crates/temper-api/tests/chunk_dedup_test.rs`)

- `replace_chunks_preserves_unchanged_positions` — 3 chunks in, same 3 in again, zero new rows in `kb_chunks`, one new row in `kb_resource_revisions` with `chunk_count = 3`.
- `replace_chunks_supersedes_changed_content` — chunk 1 body changes, chunks 0 and 2 unchanged. Expect: chunk 1 old row has `is_current = false, superseded_revision_id = new_rev`; new chunk 1 row has `first_revision_id = new_rev`. Chunks 0 and 2 untouched.
- `replace_chunks_supersedes_removed_positions` — input shrinks from 4 chunks to 3. Chunk at index 3 flips `is_current = false, superseded_revision_id = new_rev`.
- `replace_chunks_adds_new_positions` — input grows from 2 chunks to 4. Positions 2 and 3 get fresh rows with `first_revision_id = new_rev, version = 1`.
- `replace_chunks_empty_input_supersedes_all` — 3 chunks in, `[]` in. All 3 flip non-current; new revision has `chunk_count = 0`.
- `persist_chunks_creates_revision` — first-time insert for a new resource; revision created with correct `body_hash`, all chunks reference it as `first_revision_id`.

### B3/B4 service tests

- `crates/temper-api/tests/ingest_revision_test.rs::create_resource_links_revision_to_audit` — after `ingest`, the one `kb_resource_revisions` row has `audit_id` equal to the `kb_resource_audits.id` created in the same transaction.
- `crates/temper-api/tests/ingest_revision_test.rs::update_resource_creates_new_revision` — second `update` call produces a second revision, audit_id matches the update's audit.
- `packages/temper-cloud/src/__tests__/process-upload.test.ts::workflow_creates_audit_and_revision` (integration) — end-to-end upload produces exactly one new revision with action="update_body" audit.

### Invariant preservation

- `tests/e2e/tests/sync_test.rs:1527` — existing assertion unchanged. Meta-only push must still produce zero `kb_chunks` writes and zero `kb_resource_revisions` writes. Assert the latter explicitly.

### C1 point-in-time tests (new)

- `resource_chunks_at_revision_returns_original_at_r1` — after R1 → R2 → R3, calling with `p_revision_id = R1` returns the exact R1 chunk set (content + hashes), not current state.
- `resource_chunks_at_revision_respects_mid_supersede` — chunk at index 2 superseded at R2 not visible at R3, visible at R1.
- `resource_chunks_at_revision_unknown_returns_empty` — unknown revision id returns zero rows, no error.

### C2 retention tests (new)

- `sweep_keeps_pinned_by_first_revision_id` — a revision with live chunks cannot be deleted (foreign key violation path; assert sweep skips it).
- `sweep_respects_keep_last_n` — 15 revisions for one resource, `sweep(keep_last_n=10, age_ceiling_days=0)` deletes 5.
- `sweep_respects_age_ceiling` — 5 revisions all younger than ceiling → zero deleted.

## Alternatives Considered

**Reuse `kb_resource_audits` as the revision anchor.** Rejected above — audits include `update_meta`, which produces no chunks; conflating creates a cardinality mismatch and makes the retention policy weird ("delete meta audits but only if no chunks reference them").

**Store revisions inline on `kb_chunks` (no separate table).** Every `kb_chunks` row already has `created` and `content_hash` — couldn't we reconstruct "revision" as `DISTINCT body_hash ORDER BY created`? Rejected because the audit linkage is needed for cascade behavior, the retention sweep needs a dedicated row to delete, and `chunk_count` must be cached for retention lookups without scanning chunks.

**Per-chunk `audit_id` instead of per-revision.** Rejected — a revision in this model is a single logical write event that produces N chunks together. Tagging each chunk with its own audit_id loses the "these chunks are one cohort" relationship.

**Hash `(header_path, content)` together for dedup.** Rejected — makes dedup fragile to upstream heading renames that don't change body text. The tradeoff is noted in the First Principles section.

## Deferred

- Revision-history UI in `packages/temper-ui`. Not in scope. The data will be available; the UI surface is a separate design.
- `kb_sessions.anchor_revision_id` (sessions pinning a revision). Schema exists to support it; the wiring is a follow-up.
- Retention sweep scheduling (cron). Function ships in this phase; scheduler is follow-up.
- Cross-resource content-hash dedup (a chunk identical across two resources stored once). Possible under this model but out of scope.
