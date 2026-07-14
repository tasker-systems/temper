# W2 — Honest ingest completion + verbatim block content: Implementation Plan (v3)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make an interrupted segmented ingest impossible to mistake for a complete document, and make the ingest path byte-exact (`sha256(PUT) == sha256(GET)`).

**Spec:** [`../specs/2026-07-14-w2-verbatim-blocks-resumable-ingest-design.md`](../specs/2026-07-14-w2-verbatim-blocks-resumable-ingest-design.md)
**Field guide (read this first):** temper research doc *"How temper's content write path actually works"* — `019f6280-f57f-7cb0-9289-1d40cc6ceb58`.

> **v3.** v2 fixed v1's *bugs* (a feature inert in production; a backfill that would have hidden every
> cognitive map's charter) but kept v1's *shape*. A second review found the shape itself carried
> avoidable cost. Three changes, each verified against the live code and against production:
>
> 1. **`ingest_state` moves to PR 1 and ships alone.** It is the task (#420 set 3). It depends on
>    nothing else in this plan, and it is what blocks cutting a release. v2 sequenced it *fourth*,
>    behind three PRs of core-write-path surgery.
> 2. **The sidecar is extended, not reshaped** — which deletes the shape-sniffing branch v2 would have
>    left in both projectors forever, and deletes the 2a/2b two-deploy split entirely.
> 3. **Coverage is verified in the read query, not asserted by a flag.** `body_storage` becomes a
>    surfaced signal, not a read-path branch — so there is exactly one mechanism enforcing the
>    invariant, and it is the one that cannot drift.

## Global Constraints

- **Additive-only on `main`.** `main` auto-deploys. A running instance must survive **both** skew
  directions: old app + new DB, and new app + old DB. Migrations are **operator-run**
  (back up → migrate → verify → deploy; DEPLOYING.md).
- **NEVER change a SQL mutation-function signature.** `CREATE OR REPLACE FUNCTION` **cannot add a
  parameter** — it mints a *second overload*, leaves the old function callable, and can make existing
  calls ambiguous. A signature change is also a **write outage** across deploy skew.
- **Never add a new `kb_event_types` row for a hot write path.** A new event type is a **write outage**
  in the new-app/old-DB direction (`_event_append` cannot resolve the type). This is why `ingest_state`
  rides an *additive payload key* on the existing `resource_created` act rather than a new
  `ingest_begun` event, which would otherwise have been the tidier ledger.
- **No Rust code INSERTs into `kb_content_blocks` / `kb_chunks` / `kb_block_revisions` /
  `kb_chunk_content` / `kb_resources`.** All writes go through plpgsql projectors. A projection-table
  column must be **payload-carried**, never recomputed by a surface.
- **The CAS rule:** chunk prose and vectors never enter the ledger. Content travels in the transient
  sidecar (`p_content`), never the event payload. Raw block bytes obey the same rule. A plain `bool`
  flag is *not* content and is safe in the payload.
- **`body_hash` is a two-level chunk merkle and MUST NOT be redefined.** It is consumed by the
  create-dedup precheck, the telos charter diff, and finalize.
- **Hash-format footgun:** `compute_body_hash` → `"sha256:<hex>"`; `sha256_hex` → **bare** hex. The DB
  stores **bare** hex. Never compare across forms.
- **Shipped migrations are checksum-locked.** Never edit one; add a new one. `uuid_generate_v7()`, never
  native `uuidv7()`.
- **Re-derive replaced functions from the LIVE body**, not the canonical one. Live definitions:
  `_project_blocks` / `_project_block_mutated` → `20260713000040_chunk_embedding_provenance.sql`;
  `_project_resource_created` → `20260624000002_canonical_functions.sql`; `unified_search` /
  `search_vector_candidates` → `20260711000050_search_vector_scope_aware.sql`; `resource_finalize` →
  `20260708000012_streaming_ingest.sql`.
- **After any SQL change:** `cargo sqlx prepare --workspace -- --all-features`, then
  `cargo make prepare-services`, `cargo make prepare-api`, `cargo make prepare-e2e`. **Stage** the
  regenerated `.sqlx/` — the drift gate diffs against git.
- **Run `cargo make check` before every commit.**

---

## PR 1 — `ingest_state`: the missing projection *(this is the task)*

`resource_finalize()` records a **projection-less** `resource_finalized` event, so "is this document
complete?" is answerable only by scanning `kb_events`. A killed segmented upload therefore stays listed,
searchable, and `status: ok` while holding 93% of its content. **Project the event.**

**Depends on nothing else in this plan.** Ships alone.

### Files
- Create: `migrations/20260714000001_ingest_state.sql`
- Modify: `crates/temper-substrate/src/{events.rs,payloads.rs,writes.rs,replay.rs}`,
  `crates/temper-services/src/backend/{db_backend.rs,substrate_read.rs}`
- Test: `tests/e2e/tests/` (kill-mid-ingest), `crates/temper-substrate/tests/`

- [ ] **Step 1: The migration.**

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

`ADD COLUMN … NOT NULL DEFAULT` is catalog-only on PG11+ — no table rewrite on PG17 (Neon prod) or
PG18 (local/CI).

Then, all `CREATE OR REPLACE`, **all keeping their exact existing signatures**:

- **`resource_finalize`** (4-arg) — `PERFORM _project_resource_finalized(v_ev, p_payload)` after
  `_event_append`. It already validates `expected_blocks` and `expected_body_hash` and raises on
  mismatch; a raise rolls back, so the projection never runs and the resource **stays `in_progress` —
  still resumable, never silently done.**
- **`_project_resource_created`** (3-arg) — born-state from the payload:

```sql
INSERT INTO kb_resources (id, title, origin_uri, created, updated, ingest_state)
    VALUES (v_resource, p_payload->>'title', p_payload->>'origin_uri', v_occurred, v_occurred,
            CASE WHEN coalesce((p_payload->>'segmented')::boolean, false)
                 THEN 'in_progress' ELSE 'complete' END);
```

  `coalesce(…, false)` is the skew hinge: an old app's payload has no `segmented` key ⇒ `complete`,
  exactly as today.

- **`search_vector_candidates`** (5-arg) and **`unified_search`** (13-arg) — see Step 5.

- [ ] **Step 2: NO backfill. Every existing row keeps the `complete` default.**

> **This is load-bearing, not laziness.** The obvious heuristic — *more than one live block AND no
> `resource_finalized` event ⇒ an incomplete upload* — matches **exactly 4 production resources, and all
> 4 are telos charters**, including the **L0 kernel "What Temper Is"**. `charter_set` projects a
> multi-block role-tagged set and never fires `resource_finalized`, because it is not a segmented
> ingest. A backfill would have deleted every cognitive map's charter from list and search.
>
> **Multi-block does NOT mean segmented.** Confirmed against prod (2026-07-14): of 2,223 active
> resources, **2,219 are single-block**; the only multi-block rows are those 4 charters (8, 8, 9, 12).
> There is no historical signal that reliably identifies an abandoned pre-existing upload. Backfill
> nothing. Only *new* segmented begins (Step 3) are ever born `in_progress`. The one known historical
> partial is handled by hand.

- [ ] **Step 3: Born `in_progress` at segmented begin.**

`begin_segmented_ingest` (`db_backend.rs:2509`) just calls `create_resource` — nothing distinguishes it
today. Thread a flag:

- `SeedAction::ResourceCreate` (`events.rs:168`) gains `segmented: bool`; the payload builder
  (`events.rs:562`) emits `"segmented": true` **only when set** (keep the key absent otherwise, so
  existing payloads are byte-identical and the dedup precheck is untouched).
- `create_resource_impl` (`writes.rs:154`) takes `segmented` as a **function parameter**, exactly as
  `defer` already is — **not** a `CreateParams` field. `CreateParams` has 36 construction sites and
  segmentation is a property of *how this create was initiated*, not of the resource's content.
  Expose one new entry point for the segmented begin; the three existing `create_resource*` fns
  delegate with `false`.
- `db_backend::begin_segmented_ingest` calls it with `true`.

- [ ] **Step 4: Replay must see the finalize event.**

> **The field guide is wrong about the mechanism here, and the truth is worse.** It says "replay's event
> scan is a whitelist … it omits `resource_finalized`". That conflates two different scans. The
> **sidecar** scan (`replay.rs:134`) *is* a whitelist, and correctly excludes `resource_finalized` —
> that event carries no content. The **replay** scan (`replay.rs:250`) has **no whitelist at all**: it
> reads every event and dispatches on `EventKind::from_canonical_name`.
>
> The actual defect is that **`EventKind` had no `ResourceFinalized` variant**, so
> `from_canonical_name` returned `None` and replay bailed with *"replay: no projector for event type
> resource_finalized"* — **against any database that had ever run a segmented ingest.** That is a
> latent break today, independent of this work; PR 1 makes the event load-bearing, so it must be fixed
> here regardless.

Add the `EventKind::ResourceFinalized` variant (both name mappings), a `replay` dispatch arm to
`_project_resource_finalized`, and an arm in the sidecar-scan's explicit "carries no chunk manifests"
list. The projector takes **two** args (no content sidecar) — unlike its content-bearing neighbours.

- [ ] **Step 5: Exclude partials from list and search.**

- **List** — add `AND r.ingest_state = 'complete'` to the WHERE in
  `substrate_read::filtered_visible_page` (`:138`). One edit covers `list_select` **and**
  `list_meta_select`. **Not** in `resources_visible_to`: visibility is *authorization*, completeness is
  *content*. Folding one into the other would quietly change who can see what.
- **Search** — a **migration**, not a Rust edit: `unified_search`, `search_vector_candidates` and
  `search_fts_candidates` are all SQL functions. **The rule: `ingest_state = 'complete'` goes exactly
  where `r.is_active` already goes.** Same semantics (a row that exists but must not surface), same
  placement. That rule lands it in three places, each for its own reason:
  - **`unified_search`'s `corpus` CTE** — the *sufficient* gate. Everything scored funnels through it,
    from all four arms (fts, vec, graph, seed), so this alone keeps a partial out of the results.
  - **`search_vector_candidates`** (both branches) — *anti-starvation*. Without it an `in_progress`
    resource's chunks can occupy slots in the global top-k ANN and crowd complete ones out of the
    candidate set. Note the predicate lands *after* `LIMIT p_k`, not inside the `ann` CTE: applying it
    inside would force a seq-scan and defeat `idx_kb_chunks_embedding` — precisely how `is_active` is
    already handled there.
  - **`search_fts_candidates`** — *seed hygiene*. `blend0` (fts ∪ vec) feeds `seeds`, and `seeds`
    anchors graph expansion, so a partial left in the lexical arm can consume auto-seed slots and lend
    `graph_score` to its neighbours while never surfacing itself. A document that is not here yet must
    not shape the ranking of documents that are. (No top-k here, so no starvation concern — the
    predicate sits plainly in the WHERE. The asymmetry with the vector arm is deliberate.)

- [ ] **Step 6: Surface `ingest_state`** on the resource detail row so `show` can say a resource is
  incomplete. It stays **addressable and readable** by its owner — hidden from list/search, never from
  `show`. Regenerate the contract artifacts: `cargo make openapi` (openapi.json + temper-rb gem +
  temper-ts `schema.ts`) and `cargo make generate-ts-types`. **Stage them** or `cargo make check` fails
  on drift.

- [ ] **Step 7: Failing test — an interrupted ingest is not a document.**

```rust
let id = begin_segmented(&h, &body).await;
append_block(&h, id, 1).await;      // ...and then we die. No finalize.

assert!(!search_hits(&h, "distinctive phrase").await.contains(&id));
assert!(!list_ids(&h).await.contains(&id));
assert_eq!(show(&h, id).await.ingest_state, "in_progress");   // but still addressable

// resume completes it, and it reappears
finalize(&h, id).await;
assert_eq!(show(&h, id).await.ingest_state, "complete");
assert!(list_ids(&h).await.contains(&id));
```

Plus the **regression guard that v1 would have failed**: the cognitive maps are still visible.

```rust
assert!(list_ids(&h).await.contains(&l0_kernel_telos_id));
assert!(search_hits(&h, "temper").await.contains(&l0_kernel_telos_id));
```

And a **one-shot create is born `complete`** (it is atomic — there is nothing to finalize).

- [ ] **Step 8:** `cargo make test-e2e-embed && cargo make test-db && cargo make test-artifacts` →
  regenerate all four sqlx caches → `cargo make openapi` → `cargo make check` → commit.

---

## PR 2 — The segmenter must stop normalizing (prerequisite for PR 3)

**Why before PR 3:** `segment_reader` reads via `src.lines()`, which **strips line endings**, and the CLI
ships those segments as the block text. Storing bytes before fixing this would label rows `verbatim`
while CRLF and trailing newlines had *already* been lost.

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
  but never mutate the bytes emitted. **Budget accounting now counts the terminator** — correct and
  intended.

- [ ] **Step 4: Invalidate stale resume manifests.**

> A `.temper/` manifest is keyed on `(source_hash, block_budget)`. After this PR a CRLF source produces
> *different segment bytes*, so a pre-PR-2 manifest would still **match** on that key while its recorded
> per-block `content_hash`es disagree with the newly-cut segments — a resume that silently mixes old and
> new bytes.

Add `segmenter_version: u32` to the manifest identity (`find_resumable` must require equality). Set it
to `2`. A pre-PR-2 manifest now simply never matches and the CLI begins a fresh session — the correct,
safe behaviour.

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

## PR 3 — Store the bytes

### The trap this PR is built around

> **`prepare_block_from_chunks(seq, role, chunks)` receives NO prose** — and that is the arm **every CLI
> write** lands on (the CLI always sends `chunks_packed`): `writes.rs:161` (create), `writes.rs:336`
> (update), `writes.rs:595`, `db_backend.rs:2562` (segmented append). Only the *server-embed* arms
> (`prepare_block`, `prepare_block_deferred` — the MCP path) ever see prose.
>
> So a design that hangs raw text off `PreparedBlock` and populates it "in the prepare helpers" is
> **inert for the production caller**, and passes a substrate test that happens to construct via the
> prose arm. **The bytes are already in scope at every call site** (`CreateParams.body`,
> `UpdateParams.body`, `AppendBlockPayload.content`) — set `block.raw_text` **right after the match**,
> exactly as `block.incorporated = p.sources` already is (`writes.rs:166`, `writes.rs:345`).

### The sidecar: **extend, do not reshape**

Today `p_content` is a flat map `{ "<chunk-uuid>": {content, embedding, …} }`.

**Verified:** both projectors read the sidecar **only by keyed lookup** —
`p_content->(v_chunk_json->>'chunk_id')` (`_project_blocks:124`, `_project_block_mutated:168`). Nothing
iterates it; `jsonb_each` / `jsonb_object_keys` appear **nowhere** in `migrations/`. Nothing counts or
validates its keys.

Therefore: **add one reserved key, `"__blocks"`, alongside the chunk UUIDs.** Do not restructure the map.

```
{ "<chunk-uuid>": {...}, …, "__blocks": { "<block-key>": {content, content_hash} } }
```

Both skew directions are then safe **by construction, with no shape-sniffing and no deploy split**:

| | |
|---|---|
| **New app + old DB** | old projector looks up chunk UUIDs, never sees `__blocks` → no bytes → honestly `derived` |
| **Old app + new DB** | `p_content->'__blocks'` is NULL → no bytes → honestly `derived` |

Key collision is impossible: every other key is a UUID.

> **This is the correction that shrank the PR.** An earlier draft *reshaped* the sidecar to
> `{chunks: …, blocks: …}`, which breaks the old projector — which then forced a
> `CASE WHEN p_content ? 'chunks'` branch living in both projectors forever, *and* a two-deploy split
> (migration first, Rust second) to survive skew. All of that was self-inflicted.

- **`<block-key>`** is the block's **`seq`** on the create/append paths, and the block's **id** on the
  mutate path.
  > `SeedAction::BlockMutate` (`events.rs:260`) carries **no `PreparedBlock` and no seq**, and the update
  > path builds `prepare_block_from_chunks(0, …)` — **seq is hardcoded 0** regardless of which block is
  > revised. Blocks are addressed by **id** there. So the mutate sidecar is keyed by block id, and the raw
  > text rides on the `SeedAction::BlockMutate` variant (add `raw: Option<&str>`), not on `PreparedBlock`.

### Files
- Create: `migrations/20260714000002_block_content_verbatim.sql`
- Modify: `crates/temper-substrate/src/{content.rs,payloads.rs,events.rs,writes.rs,replay.rs,scenario/runner.rs}`,
  `crates/temper-services/src/backend/db_backend.rs`
- Test: `crates/temper-substrate/tests/block_content.rs` (new)

- [ ] **Step 1: The migration.**

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
v_blocks := coalesce(p_content->'__blocks', '{}'::jsonb);   -- absent ⇒ legacy caller ⇒ no bytes

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

Every existing chunk lookup is **unchanged** — it still reads `p_content->(chunk_id)`.
`_project_block_mutated` does the same, keying `v_blocks -> v_block::text` (**block id**), and advancing
`current_revision_id` alongside `last_event_id`.

- [ ] **Step 2: `body_storage` is derived from coverage, never asserted.**

> **The trap this closes.** Content is per-**revision**; the flag is per-**resource**. A resource with
> **mixed coverage** (some blocks with bytes, some without) flagged `verbatim` would have its `INNER JOIN`
> readback **skip** the content-less blocks — returning a **short body that looks complete.** That is
> precisely the silent-truncation class this work exists to kill.

Recompute at the tail of both projectors:

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

**`body_storage` is a *surfaced signal*, not a read-path branch.** PR 4's readback verifies coverage in
the query itself and does not consult this column. That is deliberate: one invariant, one enforcement
mechanism, and it is the one that cannot drift.

- [ ] **Step 3: Thread the bytes at the call sites — all of them.**

Add `raw_text: Option<String>` to `PreparedBlock`. Then, **after each match**:

- `writes.rs:161` (create) — `block.raw_text = Some(p.body.to_owned());`
- `writes.rs:336` (update) — same, from `p.body` (the new content of the addressed block)
- `writes.rs:595` (kernel create) — same
- `db_backend.rs:2562` (segmented append) — `Some(payload.content.clone())`
- `scenario/runner.rs:308` (CONFORM DSL revise) — it has `body` in scope; supply it
- `db_backend.rs:259` (telos/charter) — **`None`.** `ReconcileTelosBlock { role, chunks_packed }`
  carries **no prose**, so there is nothing to store. Charters stay `body_storage='derived'` until that
  wire type gains a `content` field. **State this, don't hide it.**

> **Every path that mints a `kb_block_revisions` row without bytes silently downgrades a resource to
> `derived`.** That is the enumeration this step must be complete about. `annotate_block_sources` is
> safe — it touches no chunks and mints no revision.

- [ ] **Step 4: Delete `mutate_block` (`writes.rs:766`).** It is a **public** revision-minting fn taking
  only chunks, no prose — and it has **zero callers** anywhere in `crates/` or `tests/` (verified). Leaving
  it is leaving a loaded gun: the next caller silently downgrades its resource.

- [ ] **Step 5: Failing tests** — `crates/temper-substrate/tests/block_content.rs`.

**Construct through the CHUNKS arm**, not the prose arm — otherwise the test proves nothing about
production.

```rust
#![cfg(feature = "artifact-tests")]

#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn a_chunks_arm_create_stores_verbatim_bytes(pool: PgPool) {
    let body = "# T\r\n\r\nalpha\nbeta\n";                 // CRLF + trailing newline
    let id = create_via_chunks_arm(&pool, body).await;     // the arm EVERY CLI write takes

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
    // one block WITH bytes, one WITHOUT — reachable in production: a legacy multi-block resource
    // whose block 3 is revised via `--content-block`.
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

- [ ] **Step 6: Replay.** Add `kb_block_content` to `PROJECTION_DUMPS` and re-supply the block sidecar
  from storage in `snapshot()` under the `__blocks` key. **Do not touch** the existing `kb_chunk_content`
  CAS retention assertion — it stays true.

- [ ] **Step 7:** `cargo make test-artifacts` (incl. replay equivalence) → regenerate all four sqlx
  caches → `cargo make check` → commit.

---

## PR 4 — Byte-exact readback

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
    assert_eq!(show(&h, id).await.body_storage, "verbatim");   // guards against an inert feature
}
```

- [ ] **Step 2: Run it, watch it fail.** `cargo make test-e2e-embed`

- [ ] **Step 3: The read query verifies its own coverage.**

> Do **not** branch on `body_storage`, and do **not** branch on "did `string_agg` come back NULL". Both
> are wrong for a **mixed-coverage** resource, where an INNER JOIN happily returns a **short, plausible
> body**. The `HAVING` makes a short body *impossible*: incomplete coverage returns **no row**, never a
> partial concat.

```rust
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

The `''` separator *is* the fix. `reconstruct_body` survives only to serve `derived` rows.

- [ ] **Step 4:** `cargo make test-e2e-embed` → PASS on all five bodies.
- [ ] **Step 5: Legacy guard** — a pre-PR-3 (`derived`) resource still reads back through
  `reconstruct_body`, unchanged.
- [ ] **Step 6: Surface `body_storage`** on the resource detail row — the honest answer to *"what
  guarantee does this body carry?"*. Regenerate the contract artifacts (`cargo make openapi`,
  `cargo make generate-ts-types`) and **stage them**.
- [ ] **Step 7:** caches → `cargo make check` → commit.

---

## PR 5 — The raw-bytes integrity check at finalize

`expected_body_hash` is **not** an integrity check — it is a **concurrency token** (the server-computed
chunk merkle, handed over because a non-chunking caller cannot derive it, and documented *"Opaque: echo
it back verbatim, never parse it"*). Repurposing it breaks MCP. **Add** a field.

**Depends on PR 1 (for `ingest_state`) and PR 3 (for the stored bytes).**

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

> **MCP is honestly exempt, not covered.** Hashing needs no chunker — but MCP's finalize tool never sees
> the whole body, only per-block content. So `None` is the truth there, and the check is skipped. Say so
> rather than imply coverage we do not have. `#[serde(default)]` + `..Default::default()` at MCP's call
> site keeps its `FinalizePayload` construction compiling.

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
  After PR 2, `concat(segments) == source`, so the two are provably the same bytes.

- [ ] **Step 5: Name the gap.** A **one-shot create has no finalize**, so this check never runs on it —
  and every non-charter resource in production today is one-shot/single-block. That is defensible (a
  one-shot create is a single transaction; there is no interruption window to detect), but the integrity
  chain is closed **for the segmented path only**. Say it in the spec rather than claiming a guarantee
  that isn't there.

- [ ] **Step 6: Amend the spec.**
  - Its "`body_hash = sha256(concat(block content))`" line is **wrong** — `body_hash` is the two-level
    chunk merkle and redefining it breaks the dedup precheck, the charter diff, and finalize. The new raw
    hash is a **separate** value.
  - Correct "chunks are untouched, identical before and after": true for existing rows (no re-chunk, no
    re-embed), but a **CRLF source ingested after PR 2 chunks differently than it would have before**,
    because the chunker now receives the true bytes.
  - Correct **"they heal on their next write"**: `resolve_target_block` (`writes.rs:313`) **refuses** a
    whole-body update on a multi-block resource (`">1 block (pass --content-block to address one)"`). So
    multi-block resources cannot heal into `verbatim` at all; they can only be revised block-by-block.
    The claim holds today *only because* all 2,219 non-charter production resources are single-block —
    state the reason, don't rely on the coincidence silently.

- [ ] **Step 7:** e2e → caches → `cargo make openapi` → `cargo make check` → commit.

---

## Not in scope (and why)

- **Idempotent append** — **already implemented.** `block_append` compares the incoming block merkle to
  the latest revision: identical ⇒ no-op; different ⇒ raises
  (`20260708000012_streaming_ingest.sql:51-69`). The no-op path returns **before** any projection, so it
  will not write block content either — a re-appended segment is a true no-op. Open question: whether
  that exception surfaces as **409** rather than 500.
- **Cogmap charters cannot be `verbatim`** until `ReconcileTelosBlock` carries prose. Stated, not hidden.
- **Dropping `kb_chunk_content` / `reconstruct_body`** — impossible while any `derived` row exists, and
  multi-block rows cannot heal (PR 5 Step 6). Plan for `derived` to persist indefinitely.
- **Semantic blocking** — out of scope; `body = concat(blocks)` keeps it cheap later.
