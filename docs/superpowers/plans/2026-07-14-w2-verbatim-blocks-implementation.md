# W2 — Verbatim block content + honest ingest completion: Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make the ingest path byte-exact (`sha256(PUT) == sha256(GET)`) and make an interrupted segmented ingest impossible to mistake for a complete document.

**Architecture:** The block *revision* gains authoritative raw bytes (`kb_block_content`); `kb_chunks`/`kb_chunk_content` are untouched and remain the derived retrieval index. Completion becomes a real projection (`ingest_state`) of the `resource_finalized` event that already fires but currently projects nothing.

**Spec:** [`docs/superpowers/specs/2026-07-14-w2-verbatim-blocks-resumable-ingest-design.md`](../specs/2026-07-14-w2-verbatim-blocks-resumable-ingest-design.md)

**Tech Stack:** Rust (sqlx, axum, rmcp), PostgreSQL 17 (Neon prod) / 18 (local+CI), plpgsql projectors.

## Global Constraints

- **Additive-only on `main`.** Auto-deploy of `main` must never break a running instance. Every migration is additive; every wire field is optional.
- **Shipped migrations are checksum-locked.** Never edit an applied migration — add a new one.
- **`uuid_generate_v7()`, never native `uuidv7()`.** The latter passes PG18 dev/CI and breaks Neon's PG17.
- **No Rust code INSERTs into `kb_content_blocks` / `kb_chunks` / `kb_block_revisions` / `kb_chunk_content`.** All writes go through plpgsql projectors (`_project_blocks`, `_project_block_mutated`) called by `resource_create` / `block_append` / `block_mutate`. Respect this — do not add a Rust INSERT.
- **Chunk prose and vectors never enter the ledger** (the CAS rule). Content travels in the transient **sidecar** (`p_content`), never in the event payload. Raw block bytes follow the same rule.
- **`kb_events` is append-only** — a trigger blocks UPDATE/DELETE.
- **After changing any SQL:** `cargo sqlx prepare --workspace -- --all-features`, then `cargo make prepare-services`, then `cargo make prepare-api`, then `cargo make prepare-e2e`. Stage the regenerated `.sqlx/` — the drift gate diffs against git, so a correctly-regenerated-but-unstaged cache still fails `cargo make check`.
- **Run `cargo make check` before every commit.**
- **Hash formats differ and this is a live footgun:** `temper_core::hash::compute_body_hash` returns `"sha256:<hex>"`; `temper_core::hash::sha256_hex` returns **bare** hex. The DB's `block_body_hash` / `body_hash` are **bare** hex. Never compare across the two forms.

---

## PR 1 — The segmenter must stop normalizing (prerequisite)

**Why first:** `segment_reader` reads via `src.lines()`, which **strips line endings**, and the CLI sends those segments as the block text. If we start storing block bytes *before* fixing this, early rows would be labelled `verbatim` while having already silently lost CRLF and any trailing newline. Byte-exactness must be true at the source before we attest to it.

**Files:**
- Modify: `crates/temper-ingest/src/stream.rs` (`segment_reader` / `SegmentReader`, ~:30-46; test at :193-215)
- Test: `crates/temper-ingest/src/stream.rs` (inline `mod tests`)

**Interfaces:**
- Produces: `Segment { seq, text, initial_breadcrumb }` where **`segments.map(.text).join("") == source`** exactly (was: `join("\n")`).

- [ ] **Step 1: Write the failing test** — replace `never_splits_a_line_and_reassembles_verbatim` with a byte-exact version covering the three cases `lines()` destroys.

```rust
#[test]
fn segments_reassemble_byte_exactly() {
    for doc in [
        "# T\n\nalpha\nbeta\n",                    // trailing newline
        "# T\n\nalpha\nbeta",                      // NO trailing newline
        "# T\r\n\r\nalpha\r\nbeta\r\n",            // CRLF
        "# T\n\nnaïve — ünïcode ✅\n",             // multibyte
    ] {
        let segs: Vec<_> = super::segment_reader(std::io::Cursor::new(doc), 16)
            .map(|r| r.unwrap())
            .collect();
        let rejoined: String = segs.iter().map(|s| s.text.as_str()).collect();
        assert_eq!(rejoined, doc, "segments must rejoin byte-exactly");
    }
}
```

- [ ] **Step 2: Run it and watch it fail**

Run: `cargo nextest run -p temper-ingest -E 'test(segments_reassemble_byte_exactly)'`
Expected: FAIL — CRLF comes back as LF and the trailing newline is missing.

- [ ] **Step 3: Make `SegmentReader` preserve bytes**

Stop using `BufRead::lines()`. Read with `read_line`, which **retains** the terminator, and push the line verbatim into the buffer. Keep the existing budget + heading-boundary logic (a heading is still detected on the trimmed line), but never mutate the bytes you emit.

- [ ] **Step 4: Run the whole crate suite** (the segmenter feeds chunking, so guard the neighbours)

Run: `cargo nextest run -p temper-ingest`
Expected: PASS.

- [ ] **Step 5: Guard the CLI's own invariant** — add to `crates/temper-cli/src/actions/ingest.rs` tests:

```rust
#[test]
fn plan_segments_partitions_the_source_exactly() {
    let body = "# Doc\r\n\r\n".to_string() + &"line of text\n".repeat(5_000);
    let plan = plan_segments(&body, 1024).unwrap();
    let rejoined: String = plan.segments.iter().map(|s| s.text.as_str()).collect();
    assert_eq!(rejoined, body, "the wire segments must reconstitute the source");
}
```

Run: `cargo nextest run -p temper-cli -E 'test(plan_segments_partitions_the_source_exactly)'`

- [ ] **Step 6: `cargo make check`, then commit**

```bash
cargo make check
git add crates/temper-ingest/src/stream.rs crates/temper-cli/src/actions/ingest.rs
git commit -m "fix(ingest): the segmenter must not normalize line endings

segment_reader read via src.lines(), which strips line terminators, so a CRLF
source silently became LF and a trailing newline vanished — and the CLI ships
those segments as the block text. Byte-exactness has to be true at the source
before anything downstream can attest to it."
```

---

## PR 2 — Store the bytes (schema + projectors + sidecar)

**Why this shape:** the raw block text must reach the **projector**, and it must not enter the ledger. It therefore travels in a sidecar, exactly as chunk prose does. Adding a **new trailing parameter with a DEFAULT** to the three mutation functions is skew-safe *and* yields the `body_storage` discriminator for free: an un-upgraded app sends nothing → no `kb_block_content` row → the resource is honestly `derived`.

**Files:**
- Create: `migrations/20260714000001_block_content_verbatim.sql`
- Modify: `crates/temper-substrate/src/content.rs` (`PreparedBlock`), `crates/temper-substrate/src/payloads.rs` (new `block_content_sidecar`), `crates/temper-substrate/src/events.rs` (the `ResourceCreate` / `BlockAppend` / `BlockMutate` fire arms), `crates/temper-substrate/src/replay.rs` (`PROJECTION_DUMPS` + sidecar re-supply)
- Test: `crates/temper-substrate/tests/block_content.rs` (new)

**Interfaces:**
- Produces: `payloads::block_content_sidecar(blocks: &[PreparedBlock]) -> HashMap<String, BlockContent>` keyed by **block `seq`** (as a string — JSONB object keys are strings), value `BlockContent { content: String, content_hash: String }` where `content_hash` is **bare** `sha256_hex(content.as_bytes())`.
- Produces: `PreparedBlock.raw_text: Option<String>` — the verbatim segment/body text this block was cut from. `None` ⇒ legacy/derived write.

- [ ] **Step 1: Write the migration**

```sql
-- Verbatim block content (W2). The block REVISION owns the authoritative bytes;
-- kb_chunks/kb_chunk_content are untouched and remain the derived retrieval index.
CREATE TABLE kb_block_content (
    block_revision_id uuid PRIMARY KEY
        REFERENCES kb_block_revisions(id) ON DELETE CASCADE,
    content      text NOT NULL,
    content_hash text NOT NULL   -- bare sha256 hex of content's raw bytes
);

-- The attribution anchor points at its current state.
ALTER TABLE kb_content_blocks
    ADD COLUMN current_revision_id uuid REFERENCES kb_block_revisions(id);

-- Honest default: anything written by an un-upgraded server stays 'derived'.
ALTER TABLE kb_resources
    ADD COLUMN body_storage text NOT NULL DEFAULT 'derived';
ALTER TABLE kb_resources
    ADD CONSTRAINT ck_kb_resources_body_storage
        CHECK (body_storage IN ('verbatim', 'derived'));
```

Then `CREATE OR REPLACE` the two projectors, taking a new trailing
`p_block_content jsonb DEFAULT '{}'`, and the three mutation functions
(`resource_create`, `block_append`, `block_mutate`) to thread it through.
In `_project_blocks`, capture the revision id and write the content:

```sql
INSERT INTO kb_block_revisions (block_id, block_body_hash, chunk_count, created)
    VALUES (v_block, v_block_hash, v_chunk_count, v_occurred)
    RETURNING id INTO v_revision;

UPDATE kb_content_blocks SET current_revision_id = v_revision WHERE id = v_block;

v_raw := p_block_content -> (v_block_json->>'seq');
IF v_raw IS NOT NULL THEN
    INSERT INTO kb_block_content (block_revision_id, content, content_hash)
        VALUES (v_revision, v_raw->>'content', v_raw->>'content_hash');
    UPDATE kb_resources SET body_storage = 'verbatim' WHERE id = p_resource;
END IF;
```

`_project_block_mutated` does the same on its own revision INSERT (keyed by the
mutated block's `seq`), advancing `current_revision_id` alongside `last_event_id`.

> A revision with **no** `kb_block_content` row is a `derived` block. This is the
> skew-safe path and needs no coordination.

- [ ] **Step 2: Apply and eyeball**

```bash
cargo make docker-up
sqlx migrate run --source migrations
psql "$DATABASE_URL" -c '\d kb_block_content'
```

- [ ] **Step 3: Write the failing test** — `crates/temper-substrate/tests/block_content.rs`

```rust
#![cfg(feature = "artifact-tests")]

#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn create_stores_verbatim_block_content(pool: PgPool) {
    let body = "# T\r\n\r\nalpha\nbeta\n";           // CRLF + trailing newline
    let id = create_resource_with_body(&pool, body).await;

    let (stored, hash): (String, String) = sqlx::query_as(
        "SELECT bc.content, bc.content_hash
           FROM kb_content_blocks b
           JOIN kb_block_content bc ON bc.block_revision_id = b.current_revision_id
          WHERE b.resource_id = $1",
    ).bind(id).fetch_one(&pool).await.unwrap();

    assert_eq!(stored, body, "block content must be stored byte-for-byte");
    assert_eq!(hash, temper_core::hash::sha256_hex(body.as_bytes()));

    let storage: String = sqlx::query_scalar("SELECT body_storage FROM kb_resources WHERE id = $1")
        .bind(id).fetch_one(&pool).await.unwrap();
    assert_eq!(storage, "verbatim");
}

#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn superseded_revisions_keep_their_own_bytes(pool: PgPool) {
    let id = create_resource_with_body(&pool, "v1 body\n").await;
    update_resource_body(&pool, id, "v2 body\n").await;

    let all: Vec<String> = sqlx::query_scalar(
        "SELECT bc.content FROM kb_block_content bc
           JOIN kb_block_revisions r ON r.id = bc.block_revision_id
           JOIN kb_content_blocks b  ON b.id = r.block_id
          WHERE b.resource_id = $1 ORDER BY r.created",
    ).bind(id).fetch_all(&pool).await.unwrap();

    assert_eq!(all, vec!["v1 body\n".to_string(), "v2 body\n".to_string()],
        "each revision keeps its own immutable bytes — this is what buys point-in-time content");
}
```

- [ ] **Step 4: Run it and watch it fail**

Run: `cargo make test-artifacts`
Expected: FAIL — nothing writes `kb_block_content` yet.

- [ ] **Step 5: Thread `raw_text` through Rust**

Add `raw_text: Option<String>` to `PreparedBlock` (`content.rs`); populate it in the `prepare_block*` helpers from the text they were given. Add `payloads::block_content_sidecar`. In `events.rs`, pass the new sidecar as the extra bound argument on the `ResourceCreate`, `BlockAppend`, and `BlockMutate` arms (`SELECT resource_create($1,…,$7)` etc.).

- [ ] **Step 6: Update replay**

Add `kb_block_content` to `PROJECTION_DUMPS` (`replay.rs:~40`) so block bytes are covered by the equivalence diff, and re-supply the block sidecar from storage in `snapshot()` exactly as chunk prose is. **Do not touch the existing `kb_chunk_content` CAS retention assertion** — it stays true.

- [ ] **Step 7: Run artifact tests + replay equivalence**

Run: `cargo make test-artifacts`
Expected: PASS, including the existing replay-equivalence tests.

- [ ] **Step 8: Regenerate sqlx caches, check, commit**

```bash
cargo sqlx prepare --workspace -- --all-features
cargo make prepare-services && cargo make prepare-api && cargo make prepare-e2e
cargo make check
git add migrations/ crates/temper-substrate/ .sqlx/ crates/*/.sqlx tests/e2e/.sqlx
git commit -m "feat(ingest): the block revision owns its authoritative bytes

kb_block_content(block_revision_id) stores the verbatim text; kb_chunks and
kb_chunk_content are untouched and remain the derived retrieval index. The bytes
reach the projector via a sidecar (never the ledger — the CAS rule), and the new
p_block_content parameter defaults to '{}', so an un-upgraded server writing no
block content simply yields an honest body_storage='derived' row."
```

---

## PR 3 — Byte-exact readback

**Files:**
- Modify: `crates/temper-substrate/src/readback/mod.rs` (`body`, :472)
- Modify: `crates/temper-services/src/backend/substrate_read.rs` (`get_content_select`, :265-279)
- Modify: `crates/temper-workflow/src/types/resource.rs` (surface `body_storage` on the detail row)
- Test: `tests/e2e/tests/byte_exact_roundtrip_test.rs` (new)

**Interfaces:**
- Consumes: `kb_block_content` + `kb_content_blocks.current_revision_id` (PR 2).
- Produces: `readback::body` returns the stored bytes for `verbatim` resources; `reconstruct_body` is called **only** for `derived` ones.

- [ ] **Step 1: Write the failing e2e test** — drive the real CLI path, since that is the production caller.

```rust
#![cfg(all(feature = "test-db", feature = "test-embed"))]

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn body_round_trips_byte_exactly(pool: PgPool) {
    for body in [
        "# T\n\nalpha\nbeta\n",
        "# T\n\nalpha\nbeta",                 // no trailing newline
        "# T\r\n\r\nalpha\r\nbeta\r\n",       // CRLF
        "# T\n\nnaïve — ünïcode ✅\n",
        &large_multi_block_body(),            // > SEGMENT_BUDGET_BYTES ⇒ segmented path
    ] {
        let id = cli_create(&harness, body).await;
        let got = cli_show_content(&harness, id).await;
        assert_eq!(
            sha256_hex(got.as_bytes()), sha256_hex(body.as_bytes()),
            "readback must be byte-identical to what was written"
        );
    }
}
```

Note: assert against the **API/`show` content**, not the projected vault file —
`normalize_body_for_vault` (`actions/ingest.rs:736`) prepends a `\n` for the
frontmatter separator, which is a projection concern, not a storage one.

- [ ] **Step 2: Run it and watch it fail**

Run: `cargo make test-e2e-embed`
Expected: FAIL — the CRLF and no-trailing-newline cases come back altered (+ blank lines at chunk boundaries).

- [ ] **Step 3: Branch `readback::body` on `body_storage`**

```rust
// verbatim → the stored bytes, concatenated with NO separator.
let verbatim: Option<String> = sqlx::query_scalar(
    "SELECT string_agg(bc.content, '' ORDER BY b.seq)
       FROM kb_content_blocks b
       JOIN kb_block_content  bc ON bc.block_revision_id = b.current_revision_id
      WHERE b.resource_id = $1 AND NOT b.is_folded",
).bind(new_id).fetch_optional(pool).await?.flatten();

if let Some(body) = verbatim {
    return Ok(body);
}
// derived (legacy) → the old lossy reconstruction, unchanged.
Ok(crate::content::reconstruct_body(&chunks))
```

- [ ] **Step 4: Run the e2e**

Run: `cargo make test-e2e-embed`
Expected: PASS on all five bodies.

- [ ] **Step 5: Guard the legacy path** — assert a `derived` row still reads back through `reconstruct_body` unchanged (a resource created before PR 2 must not regress).

- [ ] **Step 6: Regenerate caches, check, commit**

```bash
cargo sqlx prepare --workspace -- --all-features && cargo make prepare-services && cargo make prepare-api && cargo make prepare-e2e
cargo make check
git commit -am "feat(ingest): byte-exact readback for verbatim resources

readback::body now returns the stored bytes concatenated with NO separator.
reconstruct_body survives only to serve legacy 'derived' rows, and dies with them —
taking the \"\\n\\n\" join bug and the heading-duplication bug (019f4694) with it."
```

---

## PR 4 — `ingest_state`: the missing projection

**Key discovery:** `resource_finalize()` records a **projection-less** `resource_finalized` event (its own comment says so, `migrations/20260708000012_streaming_ingest.sql:84-85`). "Is this complete?" is currently answerable only by scanning `kb_events`. So `ingest_state` is not a new invention — it is **the missing projection of an event that already fires**, which also means it can be **correctly backfilled from the ledger** rather than guessed.

**Files:**
- Create: `migrations/20260714000002_ingest_state_projection.sql`
- Modify: `crates/temper-services/src/backend/substrate_read.rs` (`filtered_visible_page` WHERE, :138-150)
- Modify: the `unified_search` SQL (`corpus` CTE **and** `search_vector_candidates`)
- Modify: `crates/temper-workflow/src/types/resource.rs` (surface `ingest_state`)

- [ ] **Step 1: Migration — column, projector, and a truthful backfill**

```sql
ALTER TABLE kb_resources
    ADD COLUMN ingest_state text NOT NULL DEFAULT 'complete';
ALTER TABLE kb_resources
    ADD CONSTRAINT ck_kb_resources_ingest_state
        CHECK (ingest_state IN ('in_progress', 'complete'));

CREATE INDEX idx_kb_resources_incomplete
    ON kb_resources (owner_profile_id) WHERE ingest_state = 'in_progress';

-- Project the event that already fires.
CREATE FUNCTION _project_resource_finalized(p_event uuid, p_payload jsonb)
RETURNS void LANGUAGE plpgsql AS $$
BEGIN
    UPDATE kb_resources SET ingest_state = 'complete'
     WHERE id = (p_payload->>'resource_id')::uuid;
END; $$;
-- ...and CREATE OR REPLACE resource_finalize to PERFORM it after _event_append.

-- Backfill from the ledger — derivable, not guessed:
-- a resource that BEGAN segmented (>1 live block) but has no resource_finalized
-- event was never completed.
UPDATE kb_resources r SET ingest_state = 'in_progress'
 WHERE (SELECT count(*) FROM kb_content_blocks b
         WHERE b.resource_id = r.id AND NOT b.is_folded) > 1
   AND NOT EXISTS (
        SELECT 1 FROM kb_events e JOIN kb_event_types t ON t.id = e.event_type_id
         WHERE t.name = 'resource_finalized'
           AND (e.payload->>'resource_id')::uuid = r.id);
```

> The default is `complete` because a one-shot create **is** atomic and complete. Only
> a multi-block resource with no finalize event is genuinely a partial.

- [ ] **Step 2: Set `in_progress` at segmented begin** — `resource_create` must know it is a segmented begin. The payload already distinguishes this (`begin_segmented_ingest`, `db_backend.rs:2509`); thread a `segmented: true` flag into the create payload and have `_project_resource_created` set `ingest_state = 'in_progress'` when present.

- [ ] **Step 3: Write the failing test** — kill mid-ingest.

```rust
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn an_interrupted_ingest_is_not_a_document(pool: PgPool) {
    let id = begin_segmented(&h, &body).await;
    append_block(&h, id, 0).await;
    append_block(&h, id, 1).await;
    // ... and then we die. No finalize.

    assert!(!search_hits(&h, "distinctive phrase").await.contains(&id),
        "a partial must never surface as an authoritative search hit");
    assert!(!list_ids(&h).await.contains(&id), "a partial must not be listed");

    let detail = show(&h, id).await;               // but it IS addressable
    assert_eq!(detail.ingest_state, "in_progress");
}
```

- [ ] **Step 4: Run it and watch it fail** — `cargo make test-e2e-embed`. Expected: the partial is currently listed and searchable.

- [ ] **Step 5: Exclude partials from list** — add `AND r.ingest_state = 'complete'` to the WHERE in `filtered_visible_page` (`substrate_read.rs:138-150`). Both `list_select` and `list_meta_select` funnel through it, so one edit covers both. **Do not** put this in `resources_visible_to`: visibility is *authorization*, completeness is *content*, and conflating them changes who can see what.

- [ ] **Step 6: Exclude partials from search — in BOTH arms**

Add the predicate to the `corpus` CTE **and** to `search_vector_candidates`. The vector arm is deliberately scope-aware so a scoped corpus is not starved by a global top-k; a completeness filter applied only post-ANN would reintroduce exactly that starvation for an in-progress-heavy corpus.

- [ ] **Step 7: Run the suite, regenerate caches, check, commit**

```bash
cargo make test-e2e-embed && cargo make test-db
cargo sqlx prepare --workspace -- --all-features && cargo make prepare-services && cargo make prepare-api && cargo make prepare-e2e
cargo make check
git commit -am "feat(ingest): project resource_finalized into an ingest_state

resource_finalize has always recorded a projection-LESS event, so 'is this document
complete?' was answerable only by scanning kb_events — which is why a killed upload
stayed listed, searchable and status:ok at 93% of its content. Project it. Partials
are excluded from list and search (in the query builders, never in resources_visible_to
— visibility is authorization, completeness is content) while remaining addressable
via show. Backfilled from the ledger, not guessed."
```

---

## PR 5 — The raw-bytes integrity check at finalize

**The nuance that makes this additive:** today's `expected_body_hash` is **not** an integrity check — it is a *concurrency token*. `BlocksResponse.body_hash` is the server-computed **chunk merkle**, handed to the client precisely because a non-chunking caller (MCP) cannot derive it, and it is documented **"Opaque: echo it back verbatim, never parse it."** Repurposing it would break MCP. So we **add** a field.

A raw-bytes hash needs **no chunker**, so unlike the merkle, *every* client can compute it — MCP included. The integrity check is universal in a way the merkle never could be.

**Files:**
- Modify: `crates/temper-core/src/types/ingest.rs` (`FinalizePayload`)
- Create: `migrations/20260714000003_finalize_content_hash.sql`
- Modify: `crates/temper-substrate/src/writes.rs` (`FinalizeParams`, :1155-1179)
- Modify: `crates/temper-cli/src/actions/ingest.rs` (`run_segmented_create` finalize call)

- [ ] **Step 1: Add the optional wire field** (additive — an old client omits it)

```rust
pub struct FinalizePayload {
    pub expected_blocks: u32,
    /// The server-computed chunk merkle, echoed back verbatim. A CONCURRENCY token:
    /// "nothing changed between my last append and now". Never parse it.
    pub expected_body_hash: String,
    /// Bare sha256 hex of the FULL raw body the client uploaded — an INTEGRITY check
    /// over the bytes themselves. Needs no chunker, so every surface can supply it.
    /// `None` from a client that predates this field: the check is then skipped.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_content_hash: Option<String>,
}
```

- [ ] **Step 2: Verify it in `resource_finalize`** (new migration, `CREATE OR REPLACE`)

```sql
IF p_payload ? 'expected_content_hash' THEN
    SELECT encode(sha256(convert_to(
             coalesce(string_agg(bc.content, '' ORDER BY b.seq), ''), 'UTF8')), 'hex')
      INTO v_actual_content_hash
      FROM kb_content_blocks b
      JOIN kb_block_content  bc ON bc.block_revision_id = b.current_revision_id
     WHERE b.resource_id = v_resource AND NOT b.is_folded;

    IF v_actual_content_hash IS DISTINCT FROM (p_payload->>'expected_content_hash') THEN
        RAISE EXCEPTION 'resource_finalize: resource % stored bytes hash %, expected %',
            v_resource, v_actual_content_hash, p_payload->>'expected_content_hash';
    END IF;
END IF;
```

The resource stays `in_progress` on failure (the exception rolls the tx back and the
`_project_resource_finalized` never runs) — **still resumable, never silently done.**

- [ ] **Step 3: Write the failing test**

```rust
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn finalize_rejects_a_body_hash_mismatch_and_stays_resumable(pool: PgPool) {
    let id = begin_and_append_all(&h, &body).await;
    let err = finalize_with_content_hash(&h, id, "0".repeat(64)).await.unwrap_err();
    assert!(err.to_string().contains("stored bytes hash"));
    assert_eq!(show(&h, id).await.ingest_state, "in_progress");
    // and the honest finalize still works afterwards
    finalize_with_content_hash(&h, id, sha256_hex(body.as_bytes())).await.unwrap();
    assert_eq!(show(&h, id).await.ingest_state, "complete");
}
```

- [ ] **Step 4: Send it from the CLI** — in `run_segmented_create`, set
  `expected_content_hash: Some(sha256_hex(source_bytes))`. **Bare hex** — not
  `compute_body_hash`, which prefixes `"sha256:"`.

- [ ] **Step 5: Run, regenerate, check, commit**

```bash
cargo make test-e2e-embed
cargo sqlx prepare --workspace -- --all-features && cargo make prepare-services && cargo make prepare-api && cargo make prepare-e2e
cargo make openapi && git add openapi.json clients/
cargo make check
git commit -am "feat(ingest): verify the stored BYTES at finalize

expected_body_hash was never an integrity check — it is a concurrency token (the
server-computed chunk merkle, echoed back opaquely because a non-chunking caller
cannot derive it). Add expected_content_hash: a raw sha256 over the uploaded bytes,
which needs no chunker and so is available to EVERY surface, MCP included. A mismatch
now fails the finalize and leaves the resource in_progress and resumable."
```

---

## Not in scope (and why)

- **Idempotent append** — **already implemented.** `block_append` recomputes the incoming block merkle and compares it to the latest revision: identical ⇒ no-op `RETURN v_existing_block`; different ⇒ `RAISE EXCEPTION '… already present … with different content'` (`migrations/20260708000012_streaming_ingest.sql:51-69`). The only open question is whether that exception surfaces as a **409** rather than a 500 — worth a follow-up, not a task here.
- **Dropping `kb_chunk_content` / `reconstruct_body`** — cannot happen until no `derived` rows remain.
- **Semantic blocking** — out of scope; the `body = concat(blocks)` invariant keeps it cheap later.
- **`resource_finalize` carries no authorship or invocation** (`p_metadata` hardcoded `{}`, `p_invocation` `None`, and it never gained `p_correlation`). Pre-existing; noted, not fixed here.
