# Streaming, Resumable Multi-Block Ingestion — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ingest arbitrarily large documents through CLI + API with bounded memory and resume-from-last-segment, by segmenting a body into multiple ordered `kb_content_blocks` written one request at a time.

**Architecture:** A large body becomes an ordered run of blocks (`seq 0..N`). Block 0 lands via the existing create path (`resource_created`); blocks `1..N` land via a new idempotent `BlockAppend` write firing the **dormant** `block_created` event; a new `resource_finalized` event marks completion. In-progress/complete state is *derived from the event ledger* — no column on `kb_resources`. The orphaned `kb_ingestion_records` table is resurrected for per-resource source provenance + resume integrity. The SQL substrate is already multi-block (readback, merkle, search); the change is concentrated in Rust create/append wrappers, three small API endpoints, and a streaming client chunker.

**Tech Stack:** Rust (sqlx, axum, tokio), PostgreSQL 18 (local) / 17 (Neon), pgvector, ts-rs, cargo-make, cargo-nextest.

**Spec:** `docs/superpowers/specs/2026-07-08-streaming-resumable-ingestion-design.md`

## Global Constraints

- **Additive-only on `main`** — new migration files only; never edit an applied migration (breaks prod sqlx checksum). Applied-migration immutability is absolute.
- **Persistence layer discipline** — SQL/persistence CRUD lives in `temper-substrate` (`writes`/`readback`) and `temper-services/src/services/`; surfaces (handlers, CLI actions) dispatch through `DbBackend`/the `Backend` trait for writes, never inline `sqlx::query!()`.
- **Typed structs over inline JSON** — no `serde_json::json!()` for known-shape data; define a struct. Cross-runtime wire types live in `temper-core` with `ts-rs` derives.
- **Params structs** — >5 domain params → a params struct.
- **Auth before writes** — authorization checks precede any mutation on every surface handler.
- **sqlx macros** — production SQL uses `sqlx::query!`/`query_scalar!`/`query_as!`; after any SQL change run `cargo sqlx prepare --workspace -- --all-features`, then `cargo make prepare-services` and `cargo make prepare-api` for test-target queries; prune orphaned `.sqlx` entries.
- **All builds/clippy** use `--all-features`; lint is `-D warnings`; suppress with `#[expect(..., reason = "...")]` not `#[allow]`.
- **DATABASE_URL** for local dev: `postgresql://temper:temper@localhost:5437/temper_development`. Tests run against Docker Postgres (`cargo make docker-up`). New `#[sqlx::test]` modules need `#[cfg(all(test, feature = "test-db"))]`.
- **Run `cargo fmt` before every commit** — pre-commit gates on `fmt --check` (exit 105).
- **Block budget constant:** default `SEGMENT_BUDGET_BYTES = 262_144` (256 KiB of text), config-overridable. It is *also* the one-shot/segmented threshold: a body ≤ one budget is a single-block one-shot create (unchanged); a body > one budget is segmented.

---

## File Structure

**Beat 1 — persistence + events (temper-substrate, migrations)**
- Create: `migrations/20260708000009_streaming_ingest.sql` — seed `resource_finalized` event type; `block_append()` + `resource_finalize()` SQL functions.
- Modify: `crates/temper-substrate/src/payloads.rs` — extend `BlockCreated`, add `ResourceFinalized`.
- Modify: `crates/temper-substrate/src/events.rs` — `SeedAction::BlockAppend`, `EventKind::BlockCreated` wiring, fire arm.
- Modify: `crates/temper-substrate/src/writes.rs` — `append_block`, `finalize_ingest`, `upsert_ingestion_record`, params structs.
- Test: `crates/temper-substrate/tests/streaming_ingest_test.rs`.

**Beat 2 — API + client (temper-core, temper-services, temper-api, temper-client)**
- Modify: `crates/temper-core/src/types/ingest.rs` — segmented wire types.
- Modify: `crates/temper-workflow/src/operations/` — `Backend` trait append/finalize/list-blocks methods.
- Modify: `crates/temper-services/src/backend/db_backend.rs` — implement them.
- Create: `crates/temper-api/src/handlers/segments.rs` — append/finalize/list-blocks handlers; segmented begin on `handlers/ingest.rs`.
- Modify: `crates/temper-api/src/lib.rs` (or routes module) — route wiring.
- Modify: `crates/temper-client/src/ingest.rs` — client sub-methods.

**Beat 3 — streaming client (temper-ingest, temper-cli)**
- Create: `crates/temper-ingest/src/stream.rs` — `SegmentReader` + `chunk_markdown_with_prefix`.
- Modify: `crates/temper-ingest/src/chunk.rs` — factor the heading-stack scan to accept an initial breadcrumb.
- Modify: `crates/temper-cli/src/actions/ingest.rs` — segmented orchestration.
- Create: `crates/temper-cli/src/actions/ingest_manifest.rs` — `.temper/ingest/<id>.json` read/write + resume diff.

**Beat 4 — e2e (tests/e2e)**
- Create: `tests/e2e/tests/streaming_ingest_test.rs`.

---

## BEAT 1 — Persistence + Events

### Task 1.1: SQL migration — `resource_finalized` type, `block_append()`, `resource_finalize()`

**Files:**
- Create: `migrations/20260708000009_streaming_ingest.sql`
- Reference (read, do not edit): `migrations/20260624000002_canonical_functions.sql:619` (`_project_blocks`), `:765` (`_event_append`), `:957` (`block_mutate`), `migrations/20260624000003_canonical_seed.sql:31-57` (event-type seed rows), `migrations/20260629000001_cogmap_charter_set.sql:18` (the `(resource_id, seq) WHERE NOT is_folded` partial unique index).

**Interfaces:**
- Consumes: existing `_project_blocks(p_resource uuid, p_event uuid, p_manifests jsonb, p_content jsonb)`, `_event_append(...)`, `kb_resource_homes`, `kb_block_revisions.block_body_hash`.
- Produces (SQL, called by Beat-1 Rust): `block_append(p_payload jsonb, p_content jsonb, p_emitter uuid, p_metadata jsonb, p_invocation uuid) RETURNS uuid` (the appended `block_id`); `resource_finalize(p_payload jsonb, p_emitter uuid, p_metadata jsonb, p_invocation uuid) RETURNS uuid` (the `resource_finalized` event id).

`block_append` semantics: payload carries `{resource_id, block:{block_id, seq, chunks:[{chunk_id, chunk_index, content_hash}]}}` plus a content sidecar (same shape `_project_blocks` consumes). It (a) resolves the resource's home anchor exactly as `block_mutate` does; (b) is **idempotent** — if a non-folded block already exists at `(resource_id, seq)`, compare the incoming block merkle `sha256(concat chunk content_hashes)` against that block's latest `kb_block_revisions.block_body_hash`: equal → return the existing `block_id` with **no event fired**; unequal → `RAISE` (a determinism/changed-source conflict); (c) otherwise `_event_append('block_created', ...)` then `_project_blocks(resource, event, jsonb_build_array(block_manifest), content)` and return the new `block_id`.

`resource_finalize` semantics: payload carries `{resource_id, expected_blocks, expected_body_hash}`. It validates `(SELECT count(*) FROM kb_content_blocks WHERE resource_id=X AND NOT is_folded) = expected_blocks` and `(SELECT body_hash FROM kb_resources WHERE id=X) = expected_body_hash`, `RAISE` on either mismatch, else `_event_append('resource_finalized', emitter, anchor_tbl, anchor, payload, ...)` and return the event id. Projection-less (the ledger row is the whole effect).

- [ ] **Step 1: Write the failing test** (in Beat 1's test file, created here to drive the migration)

```rust
// crates/temper-substrate/tests/streaming_ingest_test.rs
#![cfg(all(test, feature = "artifact-tests"))]
//! Streaming/segmented ingest: append + finalize + idempotency.
use temper_substrate::MIGRATOR;

#[sqlx::test(migrator = "MIGRATOR")]
async fn resource_finalized_event_type_is_seeded(pool: sqlx::PgPool) {
    let name: Option<String> =
        sqlx::query_scalar("SELECT name FROM kb_event_types WHERE name = 'resource_finalized'")
            .fetch_optional(&pool)
            .await
            .unwrap();
    assert_eq!(name.as_deref(), Some("resource_finalized"));
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo make docker-up && cargo nextest run -p temper-substrate --features artifact-tests resource_finalized_event_type_is_seeded`
Expected: FAIL — no such row (migration not written yet).

- [ ] **Step 3: Write the migration**

```sql
-- migrations/20260708000009_streaming_ingest.sql
-- Streaming/segmented ingestion: activate the dormant `block_created` event per
-- appended segment, add a `resource_finalized` completion event, and provide the
-- idempotent block_append + validating resource_finalize functions. Additive:
-- block 0 still lands via resource_created; segments 1..N append here.

-- `block_created` is already seeded (canonical_seed.sql). Seed only the new type.
INSERT INTO kb_event_types (id, name, description)
VALUES (uuid_generate_v7(), 'resource_finalized',
        'A segmented ingest declared complete: all expected blocks present and body_hash matches.');

-- Append one new block at seq=N into an existing resource, firing block_created.
-- Idempotent on (resource_id, seq, block merkle): a re-append of the same segment
-- is a no-op returning the existing block id; a same-seq different-content append
-- raises. Anchor resolution + sidecar shape mirror block_mutate / _project_blocks.
CREATE FUNCTION block_append(p_payload jsonb, p_content jsonb, p_emitter uuid,
                             p_metadata jsonb DEFAULT '{}', p_invocation uuid DEFAULT NULL)
RETURNS uuid LANGUAGE plpgsql AS $$
DECLARE
    v_resource uuid := (p_payload->>'resource_id')::uuid;
    v_block_json jsonb := p_payload->'block';
    v_seq int := (v_block_json->>'seq')::int;
    v_incoming_hash text;
    v_existing_block uuid;
    v_existing_hash text;
    v_anchor_tbl text; v_anchor uuid;
    v_ev uuid;
BEGIN
    IF v_resource IS NULL OR v_block_json IS NULL THEN
        RAISE EXCEPTION 'block_append: payload missing resource_id or block';
    END IF;
    IF v_block_json->'chunks' IS NULL OR jsonb_array_length(v_block_json->'chunks') = 0 THEN
        RAISE EXCEPTION 'block_append: empty chunk set for resource % seq %', v_resource, v_seq;
    END IF;
    -- Incoming block merkle = sha256 over the ordered chunk content_hashes (same
    -- rule _project_blocks uses to derive block_body_hash).
    SELECT encode(sha256(convert_to(string_agg(c->>'content_hash', '' ORDER BY (c->>'chunk_index')::int), 'UTF8')), 'hex')
      INTO v_incoming_hash
      FROM jsonb_array_elements(v_block_json->'chunks') c;

    -- Idempotency: an already-landed non-folded block at this seq.
    SELECT b.id INTO v_existing_block
      FROM kb_content_blocks b
     WHERE b.resource_id = v_resource AND b.seq = v_seq AND NOT b.is_folded;
    IF v_existing_block IS NOT NULL THEN
        SELECT block_body_hash INTO v_existing_hash
          FROM kb_block_revisions WHERE block_id = v_existing_block
         ORDER BY created DESC LIMIT 1;
        IF v_existing_hash IS DISTINCT FROM v_incoming_hash THEN
            RAISE EXCEPTION 'block_append: seq % already present for resource % with different content (source changed?)', v_seq, v_resource;
        END IF;
        RETURN v_existing_block;  -- no-op: same segment re-appended
    END IF;

    SELECT anchor_table, anchor_id INTO v_anchor_tbl, v_anchor FROM kb_resource_homes
        WHERE resource_id = v_resource ORDER BY (anchor_table = 'kb_cogmaps') DESC LIMIT 1;
    IF v_anchor IS NULL THEN
        RAISE EXCEPTION 'block_append: resource % has no home to anchor the event', v_resource;
    END IF;

    v_ev := _event_append('block_created', p_emitter, v_anchor_tbl, v_anchor, p_payload,
                          p_metadata => p_metadata, p_invocation => p_invocation);
    PERFORM _project_blocks(v_resource, v_ev, jsonb_build_array(v_block_json), p_content);
    RETURN (v_block_json->>'block_id')::uuid;
END;
$$;

-- Declare a segmented ingest complete. Validates the landed set against the
-- caller's expectation, then records a projection-less resource_finalized event.
CREATE FUNCTION resource_finalize(p_payload jsonb, p_emitter uuid,
                                  p_metadata jsonb DEFAULT '{}', p_invocation uuid DEFAULT NULL)
RETURNS uuid LANGUAGE plpgsql AS $$
DECLARE
    v_resource uuid := (p_payload->>'resource_id')::uuid;
    v_expected_blocks int := (p_payload->>'expected_blocks')::int;
    v_expected_hash text := p_payload->>'expected_body_hash';
    v_actual_blocks int;
    v_actual_hash text;
    v_anchor_tbl text; v_anchor uuid;
BEGIN
    SELECT count(*) INTO v_actual_blocks FROM kb_content_blocks
        WHERE resource_id = v_resource AND NOT is_folded;
    IF v_actual_blocks <> v_expected_blocks THEN
        RAISE EXCEPTION 'resource_finalize: resource % has % live blocks, expected %',
            v_resource, v_actual_blocks, v_expected_blocks;
    END IF;
    SELECT body_hash INTO v_actual_hash FROM kb_resources WHERE id = v_resource;
    IF v_actual_hash IS DISTINCT FROM v_expected_hash THEN
        RAISE EXCEPTION 'resource_finalize: resource % body_hash % does not match expected %',
            v_resource, v_actual_hash, v_expected_hash;
    END IF;
    SELECT anchor_table, anchor_id INTO v_anchor_tbl, v_anchor FROM kb_resource_homes
        WHERE resource_id = v_resource ORDER BY (anchor_table = 'kb_cogmaps') DESC LIMIT 1;
    IF v_anchor IS NULL THEN
        RAISE EXCEPTION 'resource_finalize: resource % has no home', v_resource;
    END IF;
    RETURN _event_append('resource_finalized', p_emitter, v_anchor_tbl, v_anchor, p_payload,
                         p_metadata => p_metadata, p_invocation => p_invocation);
END;
$$;
```

Before writing, open `_event_append` (`canonical_functions.sql:765`) and confirm its exact parameter names (`p_metadata`, `p_invocation`) and signature; match them verbatim.

- [ ] **Step 4: Run to verify it passes**

Run: `cargo nextest run -p temper-substrate --features artifact-tests resource_finalized_event_type_is_seeded`
Expected: PASS.

- [ ] **Step 5: Regenerate sqlx cache and commit**

```bash
DATABASE_URL=postgresql://temper:temper@localhost:5437/temper_development cargo sqlx migrate run
cargo fmt
git add migrations/20260708000009_streaming_ingest.sql crates/temper-substrate/tests/streaming_ingest_test.rs
git commit -m "feat(ingest): block_append + resource_finalize SQL + resource_finalized event type"
```

---

### Task 1.2: Payloads — extend `BlockCreated`, add `ResourceFinalized`

**Files:**
- Modify: `crates/temper-substrate/src/payloads.rs` (read `BlockCreated` near line 554 and `BlockMutated` for the `ChunkManifest`/`BlockManifest` shapes first).

**Interfaces:**
- Consumes: existing `BlockManifest`, `ChunkManifest`, `content_sidecar`, `Incorporation` in `payloads.rs`.
- Produces: `payloads::BlockCreated { resource_id: ResourceId, block: BlockManifest }` (serializes to the `{resource_id, block:{block_id, seq, chunks:[...]}}` shape `block_append` reads); `payloads::ResourceFinalized { resource_id: ResourceId, expected_blocks: u32, expected_body_hash: String }`.

- [ ] **Step 1: Write the failing test**

```rust
// append to crates/temper-substrate/tests/streaming_ingest_test.rs
#[test]
fn block_created_payload_serializes_with_resource_and_block() {
    use temper_substrate::payloads::{BlockCreated, BlockManifest};
    // BlockManifest::from(&PreparedBlock) is the existing constructor; build a
    // minimal PreparedBlock via content::prepare_block_from_chunks (ONNX-free).
    let block = temper_substrate::content::prepare_block_from_chunks(
        3, None,
        vec![temper_substrate::content::IncomingChunk {
            chunk_index: 0, content_hash: "abc".into(), content: "hi".into(),
            embedding: vec![], header_path: String::new(), heading_depth: 0,
        }],
    );
    let p = BlockCreated {
        resource_id: temper_substrate::ids::ResourceId::from(uuid::Uuid::now_v7()),
        block: BlockManifest::from(&block),
    };
    let v = serde_json::to_value(&p).unwrap();
    assert!(v.get("resource_id").is_some());
    assert_eq!(v["block"]["seq"], 3);
    assert!(v["block"]["chunks"].is_array());
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo nextest run -p temper-substrate --features artifact-tests block_created_payload_serializes_with_resource_and_block`
Expected: FAIL — `BlockCreated` has no `resource_id`/`block` fields (it is the dormant stub).

- [ ] **Step 3: Implement**

In `payloads.rs`, replace the dormant `BlockCreated` stub with:

```rust
/// Payload for the (now fired) `block_created` event — one appended segment.
/// The projector (`block_append` → `_project_blocks`) reads `resource_id` + the
/// single-block manifest; the content sidecar carries the chunk prose/embeddings.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlockCreated {
    pub resource_id: ResourceId,
    pub block: BlockManifest,
}

/// Payload for `resource_finalized` — a segmented ingest declared complete.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceFinalized {
    pub resource_id: ResourceId,
    pub expected_blocks: u32,
    pub expected_body_hash: String,
}
```

If replay (`replay.rs`) references the old `BlockCreated` shape, update that reference to the new fields (grep `BlockCreated` across the crate first). Confirm `BlockManifest`/`ChunkManifest` carry `block_id`, `seq`, `chunks[].{chunk_id, chunk_index, content_hash}` — the shape `_project_blocks` consumes.

- [ ] **Step 4: Run to verify it passes**

Run: `cargo nextest run -p temper-substrate --features artifact-tests block_created_payload_serializes_with_resource_and_block`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
cargo fmt
git add crates/temper-substrate/src/payloads.rs crates/temper-substrate/tests/streaming_ingest_test.rs
git commit -m "feat(ingest): BlockCreated + ResourceFinalized event payloads"
```

---

### Task 1.3: `SeedAction::BlockAppend` + fire arm + `EventKind::BlockCreated`

**Files:**
- Modify: `crates/temper-substrate/src/events.rs` — add the enum variant (near `BlockMutate`, line 229), the `event_type()` arm (line 313), the fire arm (near line 734), and wire `EventKind::BlockCreated` (enum at line 37, `as_str` line 63, `from_str` line 92).

**Interfaces:**
- Consumes: `payloads::BlockCreated`, `payloads::content_sidecar` (or `content_sidecar_chunks`), `PreparedBlock`, the `block_append` SQL fn (Task 1.1).
- Produces: `SeedAction::BlockAppend { resource: ResourceId, block: &'a PreparedBlock, emitter: EntityId }`; fires `block_created`; returns `Fired::Block(BlockId)`.

- [ ] **Step 1: Write the failing test** (append to `streaming_ingest_test.rs`)

```rust
#[sqlx::test(migrator = "MIGRATOR")]
async fn append_block_lands_second_block_and_fires_block_created(pool: sqlx::PgPool) {
    // Seed a resource with block 0 via the ordinary create path, then append seq 1.
    let ctx = streaming_test_support::seed_resource_with_block0(&pool).await; // helper below
    let block1 = temper_substrate::content::prepare_block_from_chunks(
        1, None, streaming_test_support::one_chunk("second segment"));
    let block_id = temper_substrate::writes::append_block(
        &pool,
        temper_substrate::writes::AppendParams {
            resource: ctx.resource,
            block: &block1,
            sources: vec![],
            emitter: ctx.emitter,
        },
    ).await.unwrap();
    // A block_created event exists for this resource.
    let n: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM kb_events e JOIN kb_event_types t ON t.id=e.event_type_id \
         WHERE t.name='block_created' AND (e.payload->>'resource_id')::uuid = $1")
        .bind(ctx.resource.uuid()).fetch_one(&pool).await.unwrap();
    assert_eq!(n, 1);
    // The body now reassembles both segments in seq order.
    let body = temper_substrate::readback::body(&pool, ctx.principal, ctx.resource).await.unwrap();
    assert!(body.contains("second segment"));
    let _ = block_id;
}
```

Write a small `streaming_test_support` module in the test file: `seed_resource_with_block0` creates a context + resource via `writes::create_resource_with` (reuse the pattern from `crates/temper-substrate/tests/content_multichunk.rs`), and `one_chunk(text)` builds a `Vec<IncomingChunk>` with a sha256 content_hash. Read `content_multichunk.rs` for the exact create helper signatures.

- [ ] **Step 2: Run to verify it fails**

Run: `cargo nextest run -p temper-substrate --features artifact-tests append_block_lands_second_block_and_fires_block_created`
Expected: FAIL — `writes::append_block` / `AppendParams` do not exist (Task 1.4), and the `SeedAction::BlockAppend` arm is missing.

- [ ] **Step 3: Implement the enum + fire arm**

Add to `EventKind` (events.rs:37+): `BlockCreated` variant; `as_str` → `"block_created"`; `from_str` → `EventKind::BlockCreated`. Add to `SeedAction` (after `BlockMutate`):

```rust
    /// Append a NEW block at `block.seq` to an existing resource (segmented ingest).
    /// Unlike `BlockMutate` (revise-in-place), this creates a fresh block and fires
    /// the `block_created` event. Idempotent in SQL on (resource, seq, block merkle).
    BlockAppend {
        resource: ResourceId,
        block: &'a PreparedBlock,
        emitter: EntityId,
    },
```

`event_type()` arm: `SeedAction::BlockAppend { .. } => EventKind::BlockCreated,`.

Fire arm (model on the `BlockMutate` arm at events.rs:734 and `CharterSet` at :761 for the sidecar helper):

```rust
        SeedAction::BlockAppend {
            resource,
            block,
            emitter,
        } => {
            let payload = payloads::BlockCreated {
                resource_id: resource,
                block: payloads::BlockManifest::from(block),
            };
            let sidecar = serde_json::to_value(payloads::content_sidecar(std::slice::from_ref(block)))?;
            let id = sqlx::query_scalar!(
                "SELECT block_append($1,$2,$3,$4,$5)",
                serde_json::to_value(&payload)?,
                sidecar,
                emitter.uuid(),
                ctx_meta,
                ctx_inv,
            )
            .fetch_one(&mut *conn)
            .await?
            .context("block_append returned null")?;
            Ok(Fired::Block(BlockId::from(id)))
        }
```

Confirm `content_sidecar(&[PreparedBlock])` exists (used by `CharterSet`); if only `content_sidecar_chunks` exists, build the sidecar from `block.chunks` the way the `BlockMutate` arm does.

- [ ] **Step 4:** proceed to Task 1.4 (the test needs `append_block`); do not run yet.

- [ ] **Step 5: Commit** (after 1.4 compiles)

```bash
cargo fmt && git add crates/temper-substrate/src/events.rs
git commit -m "feat(ingest): SeedAction::BlockAppend fires the dormant block_created event"
```

---

### Task 1.4: `writes::append_block` + `AppendParams`

**Files:**
- Modify: `crates/temper-substrate/src/writes.rs` (model on `create_resource_impl:149`).

**Interfaces:**
- Consumes: `SeedAction::BlockAppend` (Task 1.3), `PreparedBlock`.
- Produces:
```rust
pub struct AppendParams<'a> {
    pub resource: ResourceId,
    pub block: &'a PreparedBlock,          // seq is authoritative (block.seq)
    pub sources: Vec<payloads::Incorporation>,
    pub emitter: EntityId,
}
pub async fn append_block(pool: &PgPool, p: AppendParams<'_>) -> Result<BlockId>;
pub async fn append_block_with(pool: &PgPool, p: AppendParams<'_>, ctx: EventContext) -> Result<BlockId>;
```

- [ ] **Step 1:** (test already written in Task 1.3).

- [ ] **Step 2: Implement**

```rust
/// Append one already-prepared block at `p.block.seq` to an existing resource — the
/// segmented-ingest write. `append_block` under the default (un-attributed) context.
pub async fn append_block(pool: &PgPool, p: AppendParams<'_>) -> Result<BlockId> {
    append_block_with(pool, p, EventContext::default()).await
}

pub async fn append_block_with(
    pool: &PgPool,
    p: AppendParams<'_>,
    ctx: EventContext,
) -> Result<BlockId> {
    // Carry resource-level sources onto the block manifest → kb_block_provenance.
    let mut block = p.block.clone();
    block.incorporated = p.sources;
    let mut tx = begin_scoped(pool).await?;
    let id = fire_with(
        &mut tx,
        SeedAction::BlockAppend {
            resource: p.resource,
            block: &block,
            emitter: p.emitter,
        },
        ctx,
    )
    .await?
    .block()?;
    tx.commit().await?;
    Ok(id)
}
```

Confirm `PreparedBlock: Clone`; if not, derive `Clone` on it (and `PreparedChunk`) in `content.rs` — they are plain data structs.

- [ ] **Step 3: Run the Task 1.3 test to verify it passes**

Run: `cargo nextest run -p temper-substrate --features artifact-tests append_block_lands_second_block_and_fires_block_created`
Expected: PASS.

- [ ] **Step 4: Add the idempotency test**

```rust
#[sqlx::test(migrator = "MIGRATOR")]
async fn append_block_is_idempotent_on_reappend(pool: sqlx::PgPool) {
    let ctx = streaming_test_support::seed_resource_with_block0(&pool).await;
    let block1 = temper_substrate::content::prepare_block_from_chunks(
        1, None, streaming_test_support::one_chunk("segment one"));
    let a = temper_substrate::writes::append_block(&pool,
        temper_substrate::writes::AppendParams { resource: ctx.resource, block: &block1, sources: vec![], emitter: ctx.emitter }).await.unwrap();
    // Re-append the SAME prepared block (same chunk content_hash → same merkle).
    let b = temper_substrate::writes::append_block(&pool,
        temper_substrate::writes::AppendParams { resource: ctx.resource, block: &block1, sources: vec![], emitter: ctx.emitter }).await.unwrap();
    assert_eq!(a, b, "re-append is a no-op returning the same block id");
    let live: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM kb_content_blocks WHERE resource_id=$1 AND seq=1 AND NOT is_folded")
        .bind(ctx.resource.uuid()).fetch_one(&pool).await.unwrap();
    assert_eq!(live, 1, "no duplicate block at seq 1");
}
```

Run: `cargo nextest run -p temper-substrate --features artifact-tests append_block_is_idempotent_on_reappend`
Expected: PASS (the SQL idempotency branch returns the existing block).

- [ ] **Step 5: Regenerate sqlx + commit**

```bash
cargo sqlx prepare --workspace -- --all-features
cargo fmt
git add crates/temper-substrate/src/writes.rs crates/temper-substrate/src/content.rs crates/temper-substrate/tests/streaming_ingest_test.rs .sqlx
git commit -m "feat(ingest): writes::append_block + idempotent re-append"
```

---

### Task 1.5: `writes::finalize_ingest` + `FinalizeParams`

**Files:**
- Modify: `crates/temper-substrate/src/writes.rs`.

**Interfaces:**
- Consumes: the `resource_finalize` SQL fn (Task 1.1), `payloads::ResourceFinalized`.
- Produces:
```rust
pub struct FinalizeParams {
    pub resource: ResourceId,
    pub expected_blocks: u32,
    pub expected_body_hash: String,
    pub emitter: EntityId,
}
pub async fn finalize_ingest(pool: &PgPool, p: FinalizeParams) -> Result<EventId>;
```

- [ ] **Step 1: Write the failing test**

```rust
#[sqlx::test(migrator = "MIGRATOR")]
async fn finalize_validates_block_count_and_hash(pool: sqlx::PgPool) {
    let ctx = streaming_test_support::seed_resource_with_block0(&pool).await;
    let block1 = temper_substrate::content::prepare_block_from_chunks(
        1, None, streaming_test_support::one_chunk("segment one"));
    temper_substrate::writes::append_block(&pool,
        temper_substrate::writes::AppendParams { resource: ctx.resource, block: &block1, sources: vec![], emitter: ctx.emitter }).await.unwrap();
    let actual_hash: String = sqlx::query_scalar("SELECT body_hash FROM kb_resources WHERE id=$1")
        .bind(ctx.resource.uuid()).fetch_one(&pool).await.unwrap();

    // Wrong count → error.
    let bad = temper_substrate::writes::finalize_ingest(&pool,
        temper_substrate::writes::FinalizeParams { resource: ctx.resource, expected_blocks: 5, expected_body_hash: actual_hash.clone(), emitter: ctx.emitter }).await;
    assert!(bad.is_err());

    // Correct count + hash → a resource_finalized event lands.
    temper_substrate::writes::finalize_ingest(&pool,
        temper_substrate::writes::FinalizeParams { resource: ctx.resource, expected_blocks: 2, expected_body_hash: actual_hash, emitter: ctx.emitter }).await.unwrap();
    let n: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM kb_events e JOIN kb_event_types t ON t.id=e.event_type_id \
         WHERE t.name='resource_finalized' AND (e.payload->>'resource_id')::uuid=$1")
        .bind(ctx.resource.uuid()).fetch_one(&pool).await.unwrap();
    assert_eq!(n, 1);
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo nextest run -p temper-substrate --features artifact-tests finalize_validates_block_count_and_hash`
Expected: FAIL — `finalize_ingest` does not exist.

- [ ] **Step 3: Implement**

```rust
/// Declare a segmented ingest complete: validate the landed block set + body_hash
/// against the caller's expectation and record a `resource_finalized` event.
pub async fn finalize_ingest(pool: &PgPool, p: FinalizeParams) -> Result<EventId> {
    let payload = crate::payloads::ResourceFinalized {
        resource_id: p.resource,
        expected_blocks: p.expected_blocks,
        expected_body_hash: p.expected_body_hash,
    };
    let ev = sqlx::query_scalar!(
        "SELECT resource_finalize($1,$2,$3,$4)",
        serde_json::to_value(&payload)?,
        p.emitter.uuid(),
        serde_json::json!({}),
        Option::<uuid::Uuid>::None,
    )
    .fetch_one(pool)
    .await?
    .context("resource_finalize returned null")?;
    Ok(EventId::from(ev))
}
```

- [ ] **Step 4: Run to verify it passes**

Run: `cargo nextest run -p temper-substrate --features artifact-tests finalize_validates_block_count_and_hash`
Expected: PASS.

- [ ] **Step 5: Regenerate sqlx + commit**

```bash
cargo sqlx prepare --workspace -- --all-features
cargo fmt && git add crates/temper-substrate/src/writes.rs .sqlx crates/temper-substrate/tests/streaming_ingest_test.rs
git commit -m "feat(ingest): writes::finalize_ingest with count+hash validation"
```

---

### Task 1.6: `writes::upsert_ingestion_record` — resurrect `kb_ingestion_records`

**Files:**
- Modify: `crates/temper-substrate/src/writes.rs`.

**Interfaces:**
- Produces:
```rust
pub struct IngestionRecord<'a> {
    pub resource: ResourceId,
    pub source_uri: &'a str,
    pub source_mimetype: Option<&'a str>,
    pub conversion_tool: &'a str,     // "passthrough" for raw markdown, "kreuzberg" for extraction
    pub conversion_version: &'a str,
    pub source_hash: Option<&'a str>, // sha256 of the source bytes, for resume integrity
}
pub async fn upsert_ingestion_record(pool: &PgPool, r: IngestionRecord<'_>) -> Result<()>;
```

- [ ] **Step 1: Write the failing test**

```rust
#[sqlx::test(migrator = "MIGRATOR")]
async fn ingestion_record_upserts_source_provenance(pool: sqlx::PgPool) {
    let ctx = streaming_test_support::seed_resource_with_block0(&pool).await;
    temper_substrate::writes::upsert_ingestion_record(&pool,
        temper_substrate::writes::IngestionRecord {
            resource: ctx.resource, source_uri: "vault://big.md", source_mimetype: Some("text/markdown"),
            conversion_tool: "passthrough", conversion_version: "1", source_hash: Some("deadbeef"),
        }).await.unwrap();
    let hash: Option<String> = sqlx::query_scalar(
        "SELECT source_hash FROM kb_ingestion_records WHERE resource_id=$1")
        .bind(ctx.resource.uuid()).fetch_optional(&pool).await.unwrap();
    assert_eq!(hash.as_deref(), Some("deadbeef"));
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo nextest run -p temper-substrate --features artifact-tests ingestion_record_upserts_source_provenance`
Expected: FAIL — function absent.

- [ ] **Step 3: Implement** (`fetched_at`/`converted_at` are `NOT NULL`; set both to `now()` server-side)

```rust
/// Upsert the per-resource source-provenance row (`kb_ingestion_records`, PK
/// resource_id) — its designed "ingestion idempotency" role, finally written. Holds
/// the source uri + hash the resume path checks the client's source against.
pub async fn upsert_ingestion_record(pool: &PgPool, r: IngestionRecord<'_>) -> Result<()> {
    sqlx::query!(
        "INSERT INTO kb_ingestion_records \
           (resource_id, source_uri, source_mimetype, conversion_tool, conversion_version, fetched_at, converted_at, source_hash) \
         VALUES ($1,$2,$3,$4,$5, now(), now(), $6) \
         ON CONFLICT (resource_id) DO UPDATE SET \
           source_uri = EXCLUDED.source_uri, source_mimetype = EXCLUDED.source_mimetype, \
           conversion_tool = EXCLUDED.conversion_tool, conversion_version = EXCLUDED.conversion_version, \
           converted_at = now(), source_hash = EXCLUDED.source_hash",
        r.resource.uuid(), r.source_uri, r.source_mimetype, r.conversion_tool,
        r.conversion_version, r.source_hash,
    )
    .execute(pool)
    .await?;
    Ok(())
}
```

- [ ] **Step 4: Run to verify it passes**

Run: `cargo nextest run -p temper-substrate --features artifact-tests ingestion_record_upserts_source_provenance`
Expected: PASS.

- [ ] **Step 5: Regenerate sqlx + commit; run the full Beat-1 suite**

```bash
cargo sqlx prepare --workspace -- --all-features && cargo fmt
cargo nextest run -p temper-substrate --features artifact-tests streaming_ingest
git add crates/temper-substrate/src/writes.rs .sqlx crates/temper-substrate/tests/streaming_ingest_test.rs
git commit -m "feat(ingest): upsert_ingestion_record resurrects kb_ingestion_records"
```

**Beat 1 gate:** all `streaming_ingest_test` tests pass; `cargo make check` is green.

---

## BEAT 2 — API + Client Transport

### Task 2.1: Segmented wire types (`temper-core`)

**Files:**
- Modify: `crates/temper-core/src/types/ingest.rs`.

**Interfaces:**
- Produces (all `#[derive(Debug, Clone, Serialize, Deserialize)]`, `#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]`, and `ts-rs` where the UI needs them):
```rust
/// One landed segment as begin/list report it (the resume unit).
pub struct SegmentInfo { pub seq: u32, pub content_hash: String }
/// Response to a segmented begin (block 0 landed).
pub struct SegmentedBeginResponse { pub resource_id: uuid::Uuid, pub correlation_id: uuid::Uuid, pub blocks: Vec<SegmentInfo> }
/// Append one segment to an in-progress resource.
pub struct AppendBlockPayload { pub seq: u32, pub content: String, pub content_hash: String, pub chunks_packed: String }
/// Response to append/list-blocks: the currently landed set.
pub struct BlocksResponse { pub blocks: Vec<SegmentInfo> }
/// Declare a segmented ingest complete.
pub struct FinalizePayload { pub expected_blocks: u32, pub expected_body_hash: String }
```
Add an optional `#[serde(default, skip_serializing_if = "Option::is_none")] pub segmented: Option<SegmentedBegin>` field to `IngestPayload`, where `SegmentedBegin { total_blocks_hint: Option<u32>, block_budget: u32, source_hash: Option<String> }` — presence tells the ingest handler to return a `SegmentedBeginResponse` instead of the one-shot response.

- [ ] **Step 1: Write the failing test** — round-trip serialize each new struct in `crates/temper-core/src/types/ingest.rs`'s `#[cfg(test)] mod tests`:

```rust
#[test]
fn append_payload_round_trips() {
    let p = super::AppendBlockPayload { seq: 2, content: "x".into(), content_hash: "h".into(), chunks_packed: "b64".into() };
    let j = serde_json::to_string(&p).unwrap();
    let back: super::AppendBlockPayload = serde_json::from_str(&j).unwrap();
    assert_eq!(back.seq, 2);
}
```

- [ ] **Step 2: Run** `cargo nextest run -p temper-core append_payload_round_trips` → FAIL (types absent).
- [ ] **Step 3:** add the structs above.
- [ ] **Step 4: Run** → PASS. Then regenerate TS types: `cargo make generate-ts-types` and commit any changed `.ts` files (regenerated codegen rides along).
- [ ] **Step 5: Commit**

```bash
cargo fmt && git add crates/temper-core/src/types/ingest.rs packages/temper-ui
git commit -m "feat(ingest): segmented begin/append/finalize wire types"
```

---

### Task 2.2: `Backend` trait + `DbBackend` append/finalize/list-blocks/begin

**Files:**
- Modify: `crates/temper-workflow/src/operations/` (the `Backend` trait — grep `trait Backend`), `crates/temper-services/src/backend/db_backend.rs` (impl; model on `create_resource` at `db_backend.rs:956`).

**Interfaces:**
- Produces on `Backend` (async trait methods): `append_block(resource, AppendBlockPayload) -> Result<BlocksResponse>`, `finalize_ingest(resource, FinalizePayload) -> Result<()>`, `list_blocks(resource) -> Result<BlocksResponse>`, and a `begin_segmented` path folded into the existing `create_resource` when `IngestPayload.segmented.is_some()`. Each unpacks `chunks_packed` to `Vec<IncomingChunk>` (reuse `unpack_incoming_chunks`, `db_backend.rs:1000`), calls `content::prepare_block_from_chunks(seq, None, chunks)`, then `writes::append_block` / `writes::finalize_ingest`, and (append/begin) `writes::upsert_ingestion_record`.

- Auth: `append_block`/`finalize_ingest`/`list_blocks` MUST call `can_modify_resource(profile_id, resource)` (the same gate the update path uses) **before** any write. Grep `can_modify_resource` for the exact signature.

- `list_blocks` reads landed segments service-direct (a read passthrough): `SELECT b.seq, r.block_body_hash FROM kb_content_blocks b JOIN LATERAL (SELECT block_body_hash FROM kb_block_revisions WHERE block_id=b.id ORDER BY created DESC LIMIT 1) r ON true WHERE b.resource_id=$1 AND NOT b.is_folded ORDER BY b.seq`. Note: the reported `content_hash` in `SegmentInfo` is the **block merkle** (`block_body_hash`) — the client re-derives the same value from its packed chunk hashes for the resume diff (documented in the type).

- [ ] **Step 1: Write the failing integration test** — `crates/temper-services/tests/segmented_backend_test.rs` (`#[cfg(all(test, feature = "test-db"))]`): begin (create block 0) → append seq 1 → `list_blocks` returns 2 → `finalize` succeeds; an append by a non-owning profile returns an auth error. Use the existing services test harness (grep an existing `#[sqlx::test]` in `crates/temper-services/tests/`).
- [ ] **Step 2: Run** `cargo nextest run -p temper-services --features test-db --test segmented_backend_test` → FAIL.
- [ ] **Step 3:** implement the trait methods + `DbBackend` impls per the interface above (build the `PreparedBlock`, auth-gate, dispatch to `writes`).
- [ ] **Step 4: Run** → PASS.
- [ ] **Step 5:** `cargo make prepare-services && cargo fmt`; commit code + `crates/temper-services/.sqlx`.

---

### Task 2.3: API handlers + routes

**Files:**
- Create: `crates/temper-api/src/handlers/segments.rs`; Modify: `crates/temper-api/src/handlers/ingest.rs:30` (segmented begin), the router module (grep `.route("/api/ingest"`).

**Interfaces:**
- Produces routes: `POST /api/resources/{id}/blocks` → `append_block_handler`; `POST /api/resources/{id}/finalize` → `finalize_handler`; `GET /api/resources/{id}/blocks` → `list_blocks_handler`. Each: `AuthUser` extractor → build `DbBackend::new(pool, profile_id)` → dispatch the Task 2.2 method → map errors via the existing `ApiError`. Thin handlers only (fundamentals: middleware → validation → business logic → response). The auth check lives in the backend method (Task 2.2), invoked before writes.
- `handlers/ingest.rs`: when `payload.segmented.is_some()`, return `Json(SegmentedBeginResponse{...})` from the create; else the existing one-shot response — no change to the small-body path.

- [ ] **Step 1: Write the failing test** — `crates/temper-api/tests/segments_handler_test.rs` (`#[cfg(all(test, feature = "test-db"))]`, model on `relationship_handler_test`): spin the router, begin+append+finalize over HTTP, assert `GET /blocks` reflects landed seqs; assert a cross-profile append → 403.
- [ ] **Step 2: Run** `cargo nextest run -p temper-api --features test-db --test segments_handler_test` → FAIL.
- [ ] **Step 3:** implement handlers + wire routes.
- [ ] **Step 4: Run** → PASS.
- [ ] **Step 5:** `cargo make prepare-api && cargo fmt`; commit code + `crates/temper-api/.sqlx`.

---

### Task 2.4: `temper-client` sub-methods

**Files:**
- Modify: `crates/temper-client/src/ingest.rs` (model on `IngestClient::create` at `ingest.rs:31`).

**Interfaces:**
- Produces: `IngestClient::begin_segmented(&IngestPayload) -> Result<SegmentedBeginResponse>` (POST `/api/ingest` with `segmented` set), `append_block(resource_id, &AppendBlockPayload) -> Result<BlocksResponse>` (POST `/api/resources/{id}/blocks`), `finalize(resource_id, &FinalizePayload) -> Result<()>`, `list_blocks(resource_id) -> Result<BlocksResponse>` (GET `/api/resources/{id}/blocks`). All bearer-authed via the existing `send_json` helper.

- [ ] **Step 1:** unit-test the URL/method construction (grep how `ingest.rs` tests build a client, or assert via a mock like existing client tests).
- [ ] **Step 2: Run** → FAIL.
- [ ] **Step 3:** implement the four methods.
- [ ] **Step 4: Run** → PASS.
- [ ] **Step 5:** `cargo fmt`; commit.

**Beat 2 gate:** `cargo make check` green; services + api segmented tests pass.

---

## BEAT 3 — Streaming Client + CLI Orchestration

### Task 3.1: `chunk_markdown_with_prefix` — carry the heading stack

**Files:**
- Modify: `crates/temper-ingest/src/chunk.rs` (read `chunk_markdown:317` and the heading-stack scan `collect_sections`).

**Interfaces:**
- Produces: `pub fn chunk_markdown_with_prefix(text: &str, initial_breadcrumb: &[String]) -> Vec<ChunkData>` — identical to `chunk_markdown` but seeds the breadcrumb stack from `initial_breadcrumb`, so a segment that begins mid-section carries its ancestor `header_path`. `chunk_markdown(text)` becomes `chunk_markdown_with_prefix(text, &[])`.

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn prefix_breadcrumb_prepends_ancestor_headings() {
    // A segment that starts under "A > B" but whose text only contains "## C".
    let out = super::chunk_markdown_with_prefix("## C\n\nbody", &["A".into(), "B".into()]);
    assert_eq!(out[0].header_path, "A > B > C");
}
#[test]
fn empty_prefix_equals_plain_chunk_markdown() {
    let text = "# H\n\npara one\n\n## H2\n\npara two";
    assert_eq!(super::chunk_markdown_with_prefix(text, &[]), super::chunk_markdown(text));
}
```

- [ ] **Step 2: Run** `cargo nextest run -p temper-ingest prefix_breadcrumb` → FAIL.
- [ ] **Step 3:** factor the internal scan to take an initial stack; `chunk_markdown` delegates with `&[]`. Confirm the `header_path` join separator matches existing (`" > "`).
- [ ] **Step 4: Run** both tests → PASS.
- [ ] **Step 5:** `cargo fmt`; commit.

---

### Task 3.2: `SegmentReader` — bounded streaming segmentation

**Files:**
- Create: `crates/temper-ingest/src/stream.rs`; export from `crates/temper-ingest/src/lib.rs`.

**Interfaces:**
- Produces:
```rust
pub struct Segment { pub seq: u32, pub text: String, pub initial_breadcrumb: Vec<String> }
/// Read `src` line-by-line, emitting segments of at most `budget` bytes, preferring
/// to cut at a heading boundary; carries the heading stack across segments so each
/// Segment.initial_breadcrumb seeds chunk_markdown_with_prefix. Peak memory = one segment.
pub fn segment_reader<R: std::io::BufRead>(src: R, budget: usize) -> impl Iterator<Item = std::io::Result<Segment>>;
```
- The budget default (`SEGMENT_BUDGET_BYTES = 262_144`) lives as a `pub const` here. A single segment never splits a line (lines are the atom; #316's `hard_split` already bounds pathological single lines at the chunk layer, which each segment's `chunk_markdown_with_prefix` still applies).

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn segments_are_bounded_and_seq_ordered() {
    let doc = "# A\n".to_string() + &"x\n".repeat(200_000); // ~400 KB
    let segs: Vec<_> = super::segment_reader(std::io::Cursor::new(doc), 262_144)
        .map(|r| r.unwrap()).collect();
    assert!(segs.len() >= 2, "large doc splits");
    for (i, s) in segs.iter().enumerate() {
        assert_eq!(s.seq as usize, i);
        assert!(s.text.len() <= 262_144 + 4096, "each segment near-budget");
    }
}
#[test]
fn small_doc_is_one_segment() {
    let segs: Vec<_> = super::segment_reader(std::io::Cursor::new("# H\n\nshort".to_string()), 262_144)
        .map(|r| r.unwrap()).collect();
    assert_eq!(segs.len(), 1);
    assert_eq!(segs[0].seq, 0);
}
```

- [ ] **Step 2: Run** `cargo nextest run -p temper-ingest segments_are_bounded` → FAIL.
- [ ] **Step 3:** implement the line-buffered reader: accumulate lines, tracking the running heading stack; flush a `Segment` when adding the next line would exceed `budget`, preferring to flush right before a heading line; carry the stack into the next `Segment.initial_breadcrumb`.
- [ ] **Step 4: Run** → PASS.
- [ ] **Step 5:** `cargo fmt`; commit.

---

### Task 3.3: `.temper/` manifest + resume diff

**Files:**
- Create: `crates/temper-cli/src/actions/ingest_manifest.rs`.

**Interfaces:**
- Produces:
```rust
#[derive(Serialize, Deserialize)]
pub struct IngestManifest {
    pub resource_id: uuid::Uuid,
    pub source_hash: String,
    pub block_budget: u32,
    pub correlation_id: uuid::Uuid,
    pub blocks: Vec<SegmentInfo>,   // seq + content_hash (block merkle) of each landed segment
    pub finalized: bool,
}
pub fn manifest_path(vault: &Path, resource_id: Uuid) -> PathBuf; // .temper/ingest/<id>.json
pub fn load(path: &Path) -> Result<Option<IngestManifest>>;
pub fn store(path: &Path, m: &IngestManifest) -> Result<()>;
/// Segments still to send = local segments whose (seq, hash) is not in `landed`.
pub fn resume_gap(local: &[SegmentInfo], landed: &[SegmentInfo]) -> Vec<u32>;
```

- [ ] **Step 1: Write the failing test** — `resume_gap` returns only missing seqs; a `source_hash` mismatch is surfaced to the caller (unit-test the diff and a load/store round-trip in a `tempfile::tempdir`).
- [ ] **Step 2: Run** → FAIL.
- [ ] **Step 3:** implement load/store (serde_json to `.temper/ingest/<id>.json`) + `resume_gap` (set difference on `(seq, content_hash)`).
- [ ] **Step 4: Run** → PASS.
- [ ] **Step 5:** `cargo fmt`; commit.

---

### Task 3.4: CLI orchestration — one-shot vs segmented + resume

**Files:**
- Modify: `crates/temper-cli/src/actions/ingest.rs` (the `compute_body_chunks:36` / `resolve_body` path) and `crates/temper-cli/src/cloud_backend/backend.rs` create dispatch.

**Interfaces:**
- Consumes: `segment_reader`, `chunk_markdown_with_prefix`, `embed_texts`, `pack_chunks`, `IngestClient::{begin_segmented, append_block, finalize, list_blocks}`, the manifest module.
- Behavior: the CLI stats the source; if `size <= SEGMENT_BUDGET_BYTES` → existing one-shot path (unchanged). Else: stream via `segment_reader`; for each segment, chunk (`chunk_markdown_with_prefix` with the segment's `initial_breadcrumb`) → `embed_texts` → `pack_chunks`; segment 0 → `begin_segmented`, segments 1..N → `append_block`; write the manifest after each landed segment; `finalize` at the end. On a re-run, `load` the manifest, `list_blocks`, compute `resume_gap`, and send only missing segments (re-deriving them deterministically from the source). A `source_hash` mismatch → clear the manifest and restart.
- Peak memory holds one segment's text + chunks + vectors, never the whole body.

- [ ] **Step 1: Write the failing test** — a CLI action unit test (grep existing `actions/ingest.rs` tests) asserting the size-threshold branch: a body ≤ budget selects one-shot; a body > budget selects segmented (inject a fake `IngestClient` or assert the chosen path via a seam). If the current action isn't unit-seamed, add the branch selection as a small pure function `fn ingest_mode(source_len: usize, budget: usize) -> IngestMode` and test that directly; the e2e (Beat 4) covers the wired path.
- [ ] **Step 2: Run** → FAIL.
- [ ] **Step 3:** implement the orchestration + `ingest_mode` seam.
- [ ] **Step 4: Run** → PASS.
- [ ] **Step 5:** rebuild the CLI binary (`cargo build -p temper-cli --bin temper`), `cargo fmt`, commit.

**Beat 3 gate:** `cargo make check` green; `temper-ingest` + CLI unit tests pass.

---

## BEAT 4 — End-to-End

### Task 4.1: e2e multi-segment round-trip + one-shot no-regression

**Files:**
- Create: `tests/e2e/tests/streaming_ingest_test.rs` (model on an existing e2e test in `tests/e2e/tests/`; use the shared harness in `tests/e2e/tests/common/`).

**Interfaces:**
- Consumes: the real Axum server + Postgres + the `temper` CLI binary (built fresh — nextest does NOT rebuild it; `cargo build -p temper-cli --bin temper` first).

- [ ] **Step 1: Write the test**
  - Generate a >1 MB markdown doc (multiple headings) in a tempdir.
  - `temper resource create --type reference --title Big --context @me/<ctx> --body @big.md` → drives the segmented path.
  - Assert `temper resource show <ref>` reassembles the full body (byte-equal to source after chunk round-trip, allowing the known chunk-boundary newline normalization).
  - Assert a `temper search` for a phrase from the last segment returns the resource (segments are FTS-searchable).
  - Assert a small body still creates in one shot (a follow-up create with a tiny body; verify via server logs/response shape that no `block_created` event fired — query `kb_events`).
- [ ] **Step 2: Run** `cargo build -p temper-cli --bin temper && cargo make test-e2e-embed` (segmented path computes real embeddings → needs the embed feature). Expected: FAIL first (harness/test wiring), then iterate to PASS.
- [ ] **Step 3–4:** fix wiring until green.
- [ ] **Step 5:** `cargo make prepare-e2e` if any macro SQL was added; `cargo fmt`; commit.

---

### Task 4.2: e2e interrupt → resume sends only the gap

**Files:**
- Modify: `tests/e2e/tests/streaming_ingest_test.rs`.

- [ ] **Step 1: Write the test**
  - Begin + append a subset of segments (drive the CLI to send seq 0..k via a fault seam, or call the client directly to land a partial set), then leave a `.temper/ingest/<id>.json` manifest.
  - Re-run the create against the same source; assert (via server event counts) that already-landed segments fire **no new** `block_created` events (idempotent no-op), only the missing seqs land, and `finalize` succeeds.
  - Assert a changed source (different `source_hash`) triggers a clean restart rather than a merge.
- [ ] **Step 2: Run** `cargo make test-e2e-embed` → iterate to PASS.
- [ ] **Step 5:** `cargo fmt`; commit.

**Beat 4 gate:** `cargo make test-e2e-embed` green; `cargo make check` green.

---

## Self-Review — spec coverage map

| Spec section | Task(s) |
|---|---|
| §2 segment/chunk decoupling | 3.1, 3.2 (chunker), 1.3–1.4 (block=segment) |
| §3a multi-block substrate (reuse) | 1.1 (`_project_blocks` reuse), verified pre-existing |
| §3b/§4 event-native completion (`block_created` fired, `resource_finalized`, ledger-derived state) | 1.1, 1.2, 1.3, 1.5 |
| §3c/§4 resurrect `kb_ingestion_records` | 1.6 |
| §5 budget-as-threshold | 3.2 (`SEGMENT_BUDGET_BYTES`), 3.4 (`ingest_mode`) |
| §6 streaming chunker + carried breadcrumb | 3.1, 3.2 |
| §7 three bounded endpoints + idempotent append | 2.1–2.4 |
| §8 `.temper/` manifest + resume | 3.3, 3.4, 4.2 |
| §7 async-embed composition | 2.2 (`prepare_block_from_chunks` verbatim; deferred path available via existing `create_resource_deferred`) |
| §10 tests | every task's TDD steps; 4.1–4.2 e2e |
| §11 one-shot no regression | 3.4 threshold, 4.1 small-body assertion |
| §12 MCP + reaper deferred | out of scope by design |

**Deferred (not in this plan, per spec §12):** MCP begin/append/finalize tools; the abandoned-ingest reaper sweep (the derived predicate exists; the sweep is later).
