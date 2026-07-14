# W2 — Verbatim block content + honest ingest completion: Implementation Plan (v2)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make the ingest path byte-exact (`sha256(PUT) == sha256(GET)`) and make an interrupted segmented ingest impossible to mistake for a complete document.

**Architecture:** The block *revision* gains authoritative raw bytes (`kb_block_content`); `kb_chunks`/`kb_chunk_content` are untouched and remain the derived retrieval index. Completion becomes a real projection (`ingest_state`) of the `resource_finalized` event that already fires but currently projects nothing.

**Spec:** [`../specs/2026-07-14-w2-verbatim-blocks-resumable-ingest-design.md`](../specs/2026-07-14-w2-verbatim-blocks-resumable-ingest-design.md)
**Field guide (read this first):** temper research doc *"How temper's content write path actually works"* — `019f6280-f57f-7cb0-9289-1d40cc6ceb58`.

> **v2.** v1 was rewritten after an adversarial review (5 surface adversaries + independent refutation)
> found it would have shipped a feature that was **inert on the production path while its own tests
> passed green**, plus a backfill that would have **hidden every cognitive map's charter** from list
> and search. The corrections are load-bearing; the ✗ notes below say what v1 got wrong and why.

## Global Constraints

- **Additive-only on `main`.** `main` auto-deploys. A running instance must survive **both** skew
  directions: old app + new DB, and new app + old DB.
- **✗ NEVER change a SQL mutation-function signature.** `CREATE OR REPLACE FUNCTION` **cannot add a
  parameter** — it mints a *second overload*, leaves the old function callable, and can make existing
  calls ambiguous. A signature change is also a **write outage** across deploy skew. New content
  travels by **reshaping the `jsonb` sidecar**, whose shape is ours on both ends.
- **No Rust code INSERTs into `kb_content_blocks` / `kb_chunks` / `kb_block_revisions` /
  `kb_chunk_content`.** All writes go through plpgsql projectors (`_project_blocks`,
  `_project_block_mutated`).
- **The CAS rule:** chunk prose and vectors never enter the ledger. Content travels in the transient
  sidecar (`p_content`), never the event payload. Raw block bytes obey the same rule.
- **`body_hash` is a two-level chunk merkle and MUST NOT be redefined.** It is consumed by the
  create-dedup precheck, the telos charter diff, and finalize. (The spec's "`body_hash = sha256(concat)`"
  line is wrong — see the spec amendment in PR 5.)
- **Hash-format footgun:** `compute_body_hash` → `"sha256:<hex>"`; `sha256_hex` → **bare** hex. The DB
  stores **bare** hex. Never compare across forms.
- **Shipped migrations are checksum-locked.** Never edit one; add a new one. `uuid_generate_v7()`, never
  native `uuidv7()`.
- **Re-derive replaced functions from the LIVE body**, not the canonical one — `_project_blocks`'s live
  definition is in `20260713000040_chunk_embedding_provenance.sql`, not `20260624000002`.
- **After any SQL change:** `cargo sqlx prepare --workspace -- --all-features`, then
  `cargo make prepare-services`, `cargo make prepare-api`, `cargo make prepare-e2e`. **Stage** the
  regenerated `.sqlx/` — the drift gate diffs against git.
- **Run `cargo make check` before every commit.**

---

## PR 1 — The segmenter must stop normalizing (prerequisite)

**Why first:** `segment_reader` reads via `src.lines()`, which **strips line endings**, and the CLI ships
those segments as the block text. Storing bytes before fixing this would label rows `verbatim` while CRLF
and trailing newlines had *already* been lost.

**Files:** `crates/temper-ingest/src/stream.rs`; `crates/temper-cli/src/actions/ingest_manifest.rs`

- [ ] **Step 1: Failing test** — replace `never_splits_a_line_and_reassembles_verbatim`:

```rust
#[test]
fn segments_reassemble_byte_exactly() {
    for doc in [
        "# T\n\nalpha\nbeta\n",              // trailing newline
        "# T\n\nalpha\nbeta",                // NO trailing newline
        "# T\r\n\r\nalpha\r\nbeta\r\n",      // CRLF
        "# T\n\nnaïve — ünïcode ✅\n",       // multibyte
    ] {
        let segs: Vec<_> = super::segment_reader(std::io::Cursor::new(doc), 16)
            .map(|r| r.unwrap()).collect();
        let rejoined: String = segs.iter().map(|s| s.text.as_str()).collect();
        assert_eq!(rejoined, doc, "segments must rejoin byte-exactly");
    }
}
```

- [ ] **Step 2: Run it, watch it fail.** `cargo nextest run -p temper-ingest -E 'test(segments_reassemble_byte_exactly)'`

- [ ] **Step 3: Preserve bytes.** Replace `BufRead::lines()` with `read_line`, which **retains** the
  terminator. Keep the budget + heading-boundary logic (heading detection still uses the *trimmed* line),
  but never mutate the bytes emitted. **Budget accounting now counts the terminator** — that is correct
  and intended.

- [ ] **Step 4: Invalidate stale resume manifests.**

> **✗ v1 missed this.** A `.temper/` manifest is keyed on `(source_hash, block_budget)`. After PR 1 a
> CRLF source produces *different segment bytes*, so a pre-PR-1 manifest would still **match** on that
> key and its recorded per-block `content_hash`es would disagree with the newly-cut segments — a resume
> that silently mixes old and new bytes.

Add a `segmenter_version: u32` to the manifest identity (`find_resumable` must require equality). Bump it
to `2`. A pre-PR-1 manifest now simply never matches, and the CLI begins a fresh session — which is the
correct, safe behaviour.

- [ ] **Step 5: Guard the CLI's invariant.** In `crates/temper-cli/src/actions/ingest.rs` tests
  (`#[cfg(feature = "embed")]`, since `plan_segments` is feature-gated):

```rust
#[test]
fn plan_segments_partitions_the_source_exactly() {
    let body = "# Doc\r\n\r\n".to_string() + &"line of text\n".repeat(5_000);
    let plan = plan_segments(&body, 1024).unwrap();
    let rejoined: String = plan.segments.iter().map(|s| s.text.as_str()).collect();
    assert_eq!(rejoined, body);
}
```

- [ ] **Step 6:** `cargo nextest run -p temper-ingest && cargo nextest run -p temper-cli`, then
  `cargo make check`, then commit.

---

## PR 2 — Store the bytes

### The two corrections that define this PR

> **✗ v1 error A — the feature would have been inert.** v1 said "add `raw_text` to `PreparedBlock` and
> populate it in the `prepare_block*` helpers." But **`prepare_block_from_chunks(seq, role, chunks)`
> receives no prose**, and that is the arm **every CLI write** lands on (the CLI always sends
> `chunks_packed`): `writes.rs:161` (create), `writes.rs:336` (update), `writes.rs:595`, and
> `db_backend.rs:2562` (segmented append). Only the *server-embed* arms (`prepare_block`,
> `prepare_block_deferred` — the MCP path) ever see prose. So every CLI-created resource would have
> stayed `derived`, the lossy readback would have persisted, and the substrate test would have passed
> **via the prose arm**, attesting to a guarantee production never provides.
>
> **Fix:** the bytes are already in scope at every call site (`CreateParams.body`, `UpdateParams.body`,
> `AppendBlockPayload.content`). Set `block.raw_text` **right after the match**, exactly as
> `block.incorporated = p.sources` already does (`writes.rs:166`, `writes.rs:345`).

> **✗ v1 error B — a signature change is a write outage.** v1 added `p_block_content jsonb DEFAULT '{}'`
> to `resource_create` / `block_append` / `block_mutate`. `CREATE OR REPLACE` **cannot add a parameter**:
> it creates an overload, leaves the old function live, and makes old-arity calls ambiguous. And a new
> app calling a 7-arg function against a not-yet-migrated DB is a **total write outage** — `main`
> auto-deploys.
>
> **Fix:** **change no signature.** Carry block bytes inside the existing `p_content` sidecar by
> reshaping it, and make the projectors accept **both** shapes.

### The sidecar reshape (skew-safe, no signature change)

Today: `p_content` is a flat map `{ "<chunk-uuid>": {content, embedding, …} }`.
New: `{ "chunks": { "<chunk-uuid>": {...} }, "blocks": { "<block-key>": {content, content_hash} } }`.

The projectors detect the shape: **if `p_content ? 'chunks'` → new shape, else → legacy flat map.**
That is the whole skew story. Old app + new DB → legacy shape → no block bytes → resource is honestly
`derived`. New app + old DB → old projector reads the flat map it expects… **which the new shape breaks.**
So the new app must keep emitting the **legacy flat shape until the migration has landed** —
therefore:

**PR 2 splits into two deploys.** `2a` = migration only (projectors tolerant of both shapes; nothing
writes the new shape yet). `2b` = the Rust change that starts emitting it. Land 2a, let it deploy, then
land 2b. This is the only ordering that is safe in both skew directions.

- **`<block-key>`** is the block's **`seq`** on the create/append paths, and the block's **id** on the
  mutate path.
  > **✗ v1 error C.** v1 keyed the mutate sidecar by `seq`. But `SeedAction::BlockMutate`
  > (`events.rs:260`) carries **no `PreparedBlock` and no seq**, and the update path builds
  > `prepare_block_from_chunks(0, …)` — **seq is hardcoded 0** regardless of which block is revised
  > (`writes.rs:336`). Blocks are addressed by **id** there. So the mutate sidecar must be keyed by
  > block id, and the raw text must ride on the `SeedAction::BlockMutate` variant (add
  > `raw: Option<&str>`), not on `PreparedBlock`.

### Files
- Create: `migrations/20260714000001_block_content_verbatim.sql` (PR 2a)
- Modify (PR 2b): `crates/temper-substrate/src/{content.rs,payloads.rs,events.rs,writes.rs,replay.rs}`,
  `crates/temper-services/src/backend/db_backend.rs`
- Test: `crates/temper-substrate/tests/block_content.rs` (new); `tests/e2e/` (see PR 3)

- [ ] **Step 1 (2a): the migration**

```sql
CREATE TABLE kb_block_content (
    block_revision_id uuid PRIMARY KEY
        REFERENCES kb_block_revisions(id) ON DELETE CASCADE,
    content      text NOT NULL,
    content_hash text NOT NULL   -- bare sha256 hex of content's raw bytes
);

ALTER TABLE kb_content_blocks
    ADD COLUMN current_revision_id uuid REFERENCES kb_block_revisions(id);

ALTER TABLE kb_resources
    ADD COLUMN body_storage text NOT NULL DEFAULT 'derived';
ALTER TABLE kb_resources
    ADD CONSTRAINT ck_kb_resources_body_storage
        CHECK (body_storage IN ('verbatim', 'derived'));
```

Then `CREATE OR REPLACE` **the two projectors only** (same signatures — re-derive from the LIVE bodies in
`20260713000040_chunk_embedding_provenance.sql`). In `_project_blocks`:

```sql
-- shape-tolerant sidecar access
v_chunks := CASE WHEN p_content ? 'chunks' THEN p_content->'chunks' ELSE p_content END;
v_blocks := CASE WHEN p_content ? 'chunks' THEN p_content->'blocks' ELSE '{}'::jsonb END;

INSERT INTO kb_block_revisions (block_id, block_body_hash, chunk_count, created)
    VALUES (v_block, v_block_hash, v_chunk_count, v_occurred)
    RETURNING id INTO v_revision;
UPDATE kb_content_blocks SET current_revision_id = v_revision WHERE id = v_block;

v_raw := v_blocks -> (v_block_json->>'seq');
IF v_raw IS NOT NULL THEN
    INSERT INTO kb_block_content (block_revision_id, content, content_hash)
        VALUES (v_revision, v_raw->>'content', v_raw->>'content_hash');
END IF;
```

Every existing `_insert_chunk` lookup must read `v_chunks`, not `p_content`.
`_project_block_mutated` does the same, keying `v_blocks -> v_block::text` (**block id**), and advancing
`current_revision_id` alongside `last_event_id`.

- [ ] **Step 2 (2a): coverage-verified `body_storage`.**

> **✗ v1 error D — the fix reintroduced the bug it was fixing.** v1 set `body_storage='verbatim'` from
> *any single block write*. But content is per-**revision** while the flag is per-**resource**, so a
> resource with **mixed coverage** (some blocks with bytes, some without) would be flagged `verbatim`
> and its `INNER JOIN` readback would **skip** the content-less blocks — returning a **short body that
> looks complete.** That is precisely the silent-truncation class this work exists to kill.

So the flag is **derived from coverage**, recomputed at the tail of both projectors, never asserted:

```sql
CREATE FUNCTION _recompute_body_storage(p_resource uuid) RETURNS void
LANGUAGE plpgsql AS $$
BEGIN
    UPDATE kb_resources r SET body_storage = CASE WHEN (
        SELECT count(*) = count(bc.block_revision_id)
          FROM kb_content_blocks b
          LEFT JOIN kb_block_content bc ON bc.block_revision_id = b.current_revision_id
         WHERE b.resource_id = p_resource AND NOT b.is_folded
    ) THEN 'verbatim' ELSE 'derived' END
    WHERE r.id = p_resource;
END; $$;
```

`'verbatim'` now means **every live block has bytes** — the only claim readback may rely on.
(An all-folded / zero-block resource yields `count(*) = 0 = count(...)` ⇒ `verbatim` with an empty body,
which is correct: nothing is missing.)

- [ ] **Step 3 (2a): apply + verify, then commit 2a alone.**

```bash
cargo make docker-up && sqlx migrate run --source migrations
psql "$DATABASE_URL" -c '\d kb_block_content'
cargo make test-artifacts   # must be GREEN with no Rust change — the projectors still read the legacy shape
```

- [ ] **Step 4 (2b): failing tests** — `crates/temper-substrate/tests/block_content.rs`

**Construct through the CHUNKS arm**, not the prose arm — otherwise the test proves nothing about
production (v1's exact mistake).

```rust
#![cfg(feature = "artifact-tests")]

#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn a_chunks_arm_create_stores_verbatim_bytes(pool: PgPool) {
    let body = "# T\r\n\r\nalpha\nbeta\n";                 // CRLF + trailing newline
    // chunks: Some(..) — the arm EVERY CLI write takes
    let id = create_via_chunks_arm(&pool, body).await;

    let (stored, hash): (String, String) = sqlx::query_as(
        "SELECT bc.content, bc.content_hash FROM kb_content_blocks b
           JOIN kb_block_content bc ON bc.block_revision_id = b.current_revision_id
          WHERE b.resource_id = $1").bind(id).fetch_one(&pool).await.unwrap();
    assert_eq!(stored, body);
    assert_eq!(hash, temper_core::hash::sha256_hex(body.as_bytes()));

    let storage: String = sqlx::query_scalar("SELECT body_storage FROM kb_resources WHERE id=$1")
        .bind(id).fetch_one(&pool).await.unwrap();
    assert_eq!(storage, "verbatim");
}

#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn mixed_coverage_is_derived_not_verbatim(pool: PgPool) {
    // one block WITH bytes, one WITHOUT (simulating a legacy block beside an upgraded one)
    let id = create_mixed_coverage_resource(&pool).await;
    let storage: String = sqlx::query_scalar("SELECT body_storage FROM kb_resources WHERE id=$1")
        .bind(id).fetch_one(&pool).await.unwrap();
    assert_eq!(storage, "derived",
        "partial coverage must NEVER be verbatim — that is how a short body looks complete");
}

#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn superseded_revisions_keep_their_own_bytes(pool: PgPool) {
    let id = create_via_chunks_arm(&pool, "v1 body\n").await;
    update_body(&pool, id, "v2 body\n").await;
    let all: Vec<String> = sqlx::query_scalar(
        "SELECT bc.content FROM kb_block_content bc
           JOIN kb_block_revisions r ON r.id = bc.block_revision_id
           JOIN kb_content_blocks b  ON b.id = r.block_id
          WHERE b.resource_id = $1 ORDER BY r.created").bind(id).fetch_all(&pool).await.unwrap();
    assert_eq!(all, vec!["v1 body\n".to_string(), "v2 body\n".to_string()]);
}
```

- [ ] **Step 5 (2b): thread the bytes at the call sites.**

Add `raw_text: Option<String>` to `PreparedBlock`. Then, **after each match**:

- `writes.rs:161` (create) — `block.raw_text = Some(p.body.to_owned());`
- `writes.rs:336` (update) — same, from `p.body`
- `writes.rs:595` — same
- `db_backend.rs:2562` (segmented append) — `Some(payload.content.clone())`
- `db_backend.rs:259` (telos/charter) — **`None`.** `ReconcileTelosBlock { role, chunks_packed }`
  (`temper-core/src/types/reconcile.rs`) carries **no prose**, so there is nothing to store. Charters stay
  `body_storage='derived'` until that wire type gains a `content` field. **State this, don't hide it.**

Add `raw: Option<&str>` to `SeedAction::BlockMutate` (`events.rs:260`) and pass `p.body` from
`writes.rs:346`. Add `payloads::block_content_sidecar` producing
`{ "<key>": { content, content_hash } }` with **bare** `sha256_hex`, and reshape the sidecar builders to
emit `{ "chunks": …, "blocks": … }`.

- [ ] **Step 6 (2b): replay.** Add `kb_block_content` to `PROJECTION_DUMPS` and re-supply the block sidecar
  from storage in `snapshot()` in the new shape. **Do not touch** the existing `kb_chunk_content` CAS
  retention assertion — it stays true.

- [ ] **Step 7 (2b):** `cargo make test-artifacts` (incl. replay equivalence) → regenerate all four sqlx
  caches → `cargo make check` → commit.

---

## PR 3 — Byte-exact readback

- [ ] **Step 1: Failing e2e** — `tests/e2e/tests/byte_exact_roundtrip_test.rs`, driving the **CLI**
  (the production caller). Assert against the API/`show` content, **not** the projected vault file
  (`normalize_body_for_vault`, `actions/ingest.rs:736`, prepends a `\n` for the frontmatter separator —
  a projection concern, not a storage one).

```rust
for body in [
    "# T\n\nalpha\nbeta\n", "# T\n\nalpha\nbeta",
    "# T\r\n\r\nalpha\r\nbeta\r\n", "# T\n\nnaïve — ünïcode ✅\n",
    &large_multi_block_body(),          // > SEGMENT_BUDGET_BYTES ⇒ segmented path
] {
    let id = cli_create(&h, body).await;
    assert_eq!(sha256_hex(cli_show_content(&h, id).await.as_bytes()),
               sha256_hex(body.as_bytes()));
    assert_eq!(show(&h, id).await.body_storage, "verbatim");   // guards v1's inertness bug
}
```

- [ ] **Step 2: Run it, watch it fail.** `cargo make test-e2e-embed`

- [ ] **Step 3: Branch on `body_storage`, and verify coverage anyway.**

> **✗ v1 error E.** v1 branched on *"did `string_agg` come back NULL"*. That is wrong for an empty body,
> an all-folded resource, and — fatally — a **mixed-coverage** resource, where the INNER JOIN happily
> returns a **short, plausible body**. Trust nothing; verify coverage in the query itself.

```rust
// verbatim ⇒ every live block has bytes (guaranteed by _recompute_body_storage, re-checked here).
// The HAVING makes a short body IMPOSSIBLE: incomplete coverage returns NO row, never a partial concat.
let verbatim: Option<String> = sqlx::query_scalar(
    "SELECT string_agg(bc.content, '' ORDER BY b.seq)
       FROM kb_content_blocks b
       LEFT JOIN kb_block_content bc ON bc.block_revision_id = b.current_revision_id
      WHERE b.resource_id = $1 AND NOT b.is_folded
     HAVING count(*) = count(bc.block_revision_id)",
).bind(new_id).fetch_optional(pool).await?.flatten();

match verbatim {
    Some(body) => Ok(body),
    // derived (legacy, or partial coverage) → the old reconstruction, unchanged.
    None => Ok(crate::content::reconstruct_body(&chunks)),
}
```

- [ ] **Step 4:** `cargo make test-e2e-embed` → PASS on all five bodies.
- [ ] **Step 5: Legacy guard** — a pre-PR-2 (`derived`) resource still reads back through
  `reconstruct_body`, unchanged.
- [ ] **Step 6: Surface `body_storage`** on the resource detail row; **regenerate the contract
  artifacts** — `cargo make openapi` (openapi.json + the temper-rb gem + temper-ts `schema.ts`) and
  `cargo make generate-ts-types`. **Stage them** or `cargo make check` fails on drift.
- [ ] **Step 7:** caches → `cargo make check` → commit.

---

## PR 4 — `ingest_state`: the missing projection

`resource_finalize()` records a **projection-less** `resource_finalized` event, so completeness is
currently only discoverable by scanning `kb_events`. Project it.

- [ ] **Step 1: The column + the projector (migration).**

```sql
ALTER TABLE kb_resources
    ADD COLUMN ingest_state text NOT NULL DEFAULT 'complete';
ALTER TABLE kb_resources
    ADD CONSTRAINT ck_kb_resources_ingest_state
        CHECK (ingest_state IN ('in_progress', 'complete'));
CREATE INDEX idx_kb_resources_incomplete
    ON kb_resources (owner_profile_id) WHERE ingest_state = 'in_progress';

CREATE FUNCTION _project_resource_finalized(p_event uuid, p_payload jsonb)
RETURNS void LANGUAGE plpgsql AS $$
BEGIN
    UPDATE kb_resources SET ingest_state = 'complete'
     WHERE id = (p_payload->>'resource_id')::uuid;
END; $$;
```
`CREATE OR REPLACE resource_finalize` (**same 4-arg signature**) to `PERFORM` it after `_event_append`.

- [ ] **Step 2: NO backfill to `in_progress`.**

> **✗ v1 error F — this would have hidden the cognitive maps.** v1 backfilled
> `count(live blocks) > 1 AND NOT EXISTS(resource_finalized)` ⇒ `in_progress`. Run against production it
> matches **exactly 4 resources — and all 4 are telos charters**, including the **L0 kernel "What Temper
> Is"** (12 blocks), "How this team works" (9), and two Storyteller charters (8 each). `charter_set`
> projects a **multi-block** role-tagged set and never fires `resource_finalized`, because it is not a
> segmented ingest. The backfill would have deleted every cognitive map's charter from list and search.
>
> **Multi-block does NOT mean segmented.** There is no historical signal that reliably identifies a
> genuinely-abandoned pre-existing upload, so **backfill nothing**: every existing row keeps the
> `complete` default. Only *new* segmented begins (Step 3) are ever born `in_progress`. The one known
> historical partial can be handled by hand.

- [ ] **Step 3: Born `in_progress` at segmented begin.**

> **✗ v1 error G.** v1 said "the payload already distinguishes a segmented begin." It does not —
> `begin_segmented_ingest` (`db_backend.rs:2509`) just calls `create_resource`.

Add `segmented: bool` to the create **payload** (a plain flag, no prose — safe for the ledger), set it in
`begin_segmented_ingest`, and have `_project_resource_created` set
`ingest_state = 'in_progress'` when it is true.

- [ ] **Step 4: Replay must see the finalize event.**

> **✗ v1 error H.** `replay.rs`'s event scan is a **whitelist**:
> `('cogmap_seeded','resource_created','block_created','block_mutated','charter_set')` — it **omits**
> `resource_finalized`. So a replayed DB would leave a segmented resource stuck at `in_progress` and the
> projection diff would fail.

Add `resource_finalized` to the scan and dispatch it to `_project_resource_finalized`.

- [ ] **Step 5: Failing test — an interrupted ingest is not a document.**

```rust
let id = begin_segmented(&h, &body).await;
append_block(&h, id, 0).await;
append_block(&h, id, 1).await;      // ...and then we die. No finalize.

assert!(!search_hits(&h, "distinctive phrase").await.contains(&id));
assert!(!list_ids(&h).await.contains(&id));
assert_eq!(show(&h, id).await.ingest_state, "in_progress");   // but still addressable

// and the cognitive maps are STILL visible — the v1 regression guard
assert!(list_ids(&h).await.contains(&l0_kernel_telos_id));
```

- [ ] **Step 6: Exclude partials from list.** Add `AND r.ingest_state = 'complete'` to the WHERE in
  `substrate_read::filtered_visible_page` (`:138-150`) — one edit covers `list_select` **and**
  `list_meta_select`. **Not** in `resources_visible_to`: visibility is *authorization*, completeness is
  *content*.

- [ ] **Step 7: Exclude partials from search — a MIGRATION, not a Rust edit.**

> **✗ v1 error I.** v1 called this a Rust change. `unified_search` and `search_vector_candidates` are
> **SQL functions**; this is a fourth migration.

Add the predicate to the `corpus` CTE **and** to `search_vector_candidates`. The vector arm is
deliberately scope-aware so a scoped corpus is not starved by a global top-k; a post-ANN-only filter
reintroduces exactly that starvation.

- [ ] **Step 8:** `cargo make test-e2e-embed && cargo make test-db` → caches → `cargo make openapi` →
  `cargo make check` → commit.

---

## PR 5 — The raw-bytes integrity check at finalize

`expected_body_hash` is **not** an integrity check — it is a **concurrency token** (the server-computed
chunk merkle, handed over because a non-chunking caller cannot derive it, and documented *"Opaque: echo
it back verbatim, never parse it"*). Repurposing it breaks MCP. **Add** a field.

- [ ] **Step 1: Additive wire field.**

```rust
pub struct FinalizePayload {
    pub expected_blocks: u32,
    /// The server-computed chunk merkle, echoed back verbatim. A CONCURRENCY token, not an
    /// integrity check. Never parse it.
    pub expected_body_hash: String,
    /// Bare sha256 hex of the FULL raw body uploaded — an INTEGRITY check over the bytes.
    /// `None` from a caller that does not hold the whole body (MCP) or predates this field;
    /// the check is then skipped.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_content_hash: Option<String>,
}
```

> **✗ v1 error J.** v1 claimed a raw hash "needs no chunker, so every surface can supply it." True of
> *hashing*; **false of availability** — MCP's finalize tool never sees the whole body, only per-block
> content. MCP is therefore **honestly exempt** (`None`), not universal. Say so rather than imply
> coverage we do not have. Adding the field must also not break `temper-mcp`'s construction of
> `FinalizePayload` — `#[serde(default)]` + `..Default::default()` at its call site.

- [ ] **Step 2: Verify in `resource_finalize`** (new migration, **same 4-arg signature**):

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

The exception rolls back, `_project_resource_finalized` never runs, and the resource stays
`in_progress` — **still resumable, never silently done.**

- [ ] **Step 3: Failing test** — a bad hash fails finalize and leaves it resumable; the honest hash then
  succeeds and flips it to `complete`.

- [ ] **Step 4: CLI sends it** — in `run_segmented_create`, `expected_content_hash:
  Some(sha256_hex(source_bytes))`. **Bare hex** — not `compute_body_hash` (which prefixes `"sha256:"`).
  After PR 1, `concat(segments) == source`, so the two are provably the same bytes.

- [ ] **Step 5: Amend the spec.** Its "`body_hash = sha256(concat(block content))`" line is **wrong** —
  `body_hash` is the two-level chunk merkle and redefining it breaks the dedup precheck, the charter
  diff, and finalize. The new raw hash is a **separate** value. Also correct the spec's "chunks are
  untouched, identical before and after": true for existing rows (no re-chunk, no re-embed), but a
  **CRLF source ingested after PR 1 chunks differently than it would have before**, because the chunker
  now receives the true bytes.

- [ ] **Step 6:** e2e → caches → `cargo make openapi` → `cargo make check` → commit.

---

## Not in scope (and why)

- **Idempotent append** — **already implemented.** `block_append` compares the incoming block merkle to
  the latest revision: identical ⇒ no-op; different ⇒ raises
  (`20260708000012_streaming_ingest.sql:51-69`). Note the no-op path returns **before** any projection,
  so it will not write block content either — a re-appended segment is a true no-op. Open question:
  whether that exception surfaces as **409** rather than 500.
- **Cogmap charters cannot be `verbatim`** until `ReconcileTelosBlock` carries prose. Stated, not hidden.
- **Dropping `kb_chunk_content` / `reconstruct_body`** — impossible while any `derived` row exists.
- **Semantic blocking** — out of scope; `body = concat(blocks)` keeps it cheap later.
