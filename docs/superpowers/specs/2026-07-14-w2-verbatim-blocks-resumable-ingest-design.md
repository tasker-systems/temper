# W2 — Block revisions own their content: byte-exact round-trip and an ingest that completes honestly

**Task:** `019f5fef-bc3a-7851-8e7a-6fbc1a1cb265` (#420 set 3) · **Branch:** `jct/server-side-embed-drop-cli-onnx`
**Status:** design — approved · **Date:** 2026-07-14

Companion to [the embed-location measurement](2026-07-14-server-side-embed-drop-cli-onnx-design.md),
which established that embedding stays client-side. This spec covers **W2**: make the ingest path
byte-exact, and make an interrupted ingest impossible to mistake for a finished one.

## What a content block is for

A `kb_content_block` is **not** a text container that happens to be badly named. It is:

- **A uniquely addressable site of attribution** — the anchor (think an HTML `#id`) that provenance
  hangs off. `kb_block_provenance` records *which sources this specific content was distilled from*.
  A flat, resource-level list of "everything that contributed" is useless for attribution, and you
  cannot recover citeability by regex-ing `[name](url)` out of markdown. The block is what makes
  "**this** claim came from **that** source" expressible at all — a first-class concern for
  agent-made statements of fact.
- **The unit of in-place partial update** — how a *part* of a document changes without rewriting the
  whole document.
- **A discrete semantic unit of intent** — a thing-to-itself, sequenced within its resource.

**Chunks live deliberately beneath that layer**: they are the units of *what is embeddable and
tsvector-able*. Derived, retrieval-oriented, replaceable.

So a block owning its own text is not a naming fix — it is **the natural completion of what a block
already is**. The unit that carries provenance should own the content that provenance is *about*.

> **Today's reality, named honestly.** The segmented ingest cuts blocks on a **256 KB size budget**, so
> a large raw ingest becomes arbitrary byte-cuts (and a small one, a single giant block). Those are poor
> attribution anchors — an artifact of the *transport*, not the model. This design does **not** cement
> that: `body = concat(blocks ORDER BY seq)` holds whether blocks are size-cuts *or* real semantic
> units, so re-blocking later is a pure restructure of boundaries over unchanged bytes.

## The problem

**The resource's text is not stored where it belongs.** It lives only in `kb_chunk_content`, and the
body is *reconstructed* from chunks on read. Three consequences, all live in production:

1. **Round-trip is lossy.** `content::reconstruct_body` ends in `pieces.join("\n\n")`, so a chunk
   boundary falling mid-paragraph returns an inserted blank line. Measured: a 1,202,908-byte body reads
   back as 1,203,710 (+802). A differential across **both** write paths returns **byte-identical,
   equally lossy** output — pre-existing and path-independent, not a regression. The heading-duplication
   bug (task `019f4694`) is its sibling: `reconstruct_body` must *re-synthesize* headings that chunk
   text strips.
2. **`body_hash` cannot detect any of it.** It is a merkle over *chunk* hashes — it attests "the chunks
   you sent are the chunks I stored", and is structurally blind to a corruption introduced *after* the
   chunks are already correct.
3. **An interrupted segmented ingest is indistinguishable from a complete document.** The resource is
   created at `begin`, so a killed upload leaves it listed, searchable, and `status: ok` holding 93% of
   its content. Nothing in the schema can say "not finished".

The same failure shape runs through all of it — the silently-**partial** document (set 3), the
silently-**empty** one (the PDF session's F1), the silently-**unembedded** one. **A knowledge base that
cannot tell you what it does not have is worse than one that fails loudly.**

## Decisions

| Question | Decision |
|---|---|
| What must round-trip guarantee? | **Byte-exact.** `sha256(PUT) == sha256(GET)`. |
| Where does text live? | **On the block *revision*.** One home, immutable once written. |
| Existing rows, whose original bytes were never kept? | **Mark them honestly.** A `body_storage` discriminator; no laundering backfill. |
| A pre-finalize partial? | **Invisible until finalized.** Excluded from search and list; `show` works and says it is incomplete. |
| Adopt `salvo-tus`? | **No.** We already have TUS's *shape*; we lack its *guarantees*. |

## Architecture: the revision owns the content

The schema is already most of the way here — this design **finishes a model that exists** rather than
imposing a new one. Verified against production:

- **`kb_block_revisions` already records `block_body_hash` + `chunk_count` per revision.** It is a
  **content-shaped record with no content** — a hash with nothing to attest.
- **`kb_chunks.version` already lines up 1:1 with a block's revision count**, exactly, at every N
  (blocks with 2 revisions have max chunk version 2; with 9 revisions, version 9).
- **Every block has a revision row** (2,442 of 2,442). No special cases.
- A mutation **re-versions the whole chunk cohort** of a block, so today the *entire* old block's text
  is already retained as superseded chunk rows.

So: **give the revision its content.**

- **`kb_block_content(block_revision_id, content, content_hash)`** — the single home for text.
  **Immutable once written**, because a revision is immutable.
- **`kb_chunks` gains `block_revision_id` + `start_char` / `end_char`** — a character range into the
  revision it belongs to.
- **`kb_content_blocks.current_revision_id`** — the anchor points at its current state.
- **`kb_chunk_content` is retired** for `verbatim` rows.

### Why this is the coherent choice

Because revision content is **immutable**, a chunk's offsets are **always valid** — for current *and*
superseded chunks, forever. That kills the whole problem class:

- **No freeze-on-supersession copy step**, and therefore no new silent-loss failure mode. (The rejected
  alternative — materialize superseded chunk prose into a sidecar at mutation time — would have added a
  fresh instance of exactly the bug class this task exists to eliminate: miss the write, lose the prose,
  silently.)
- **One mechanism, not two.** No branch on `is_current` to find a chunk's text.
- **Replay's proof obligation survives verbatim** — *"fold/supersede affect visibility, never
  existence"* stays literally true, because the revision's content row still exists. Nothing is
  reworded, nothing weakened.
- **Point-in-time block content, for free.** *What did this block say when that citation was anchored to
  it?* — answerable. For a system where provenance for agent-made statements of fact is first-class,
  that is not a bonus; it is the point.

**Storage is a wash.** Today already retains the whole superseded cohort's text; this retains the same
bytes in one row per revision instead of scattered across dozens — plus the gap characters (headings)
that today are simply thrown away.

### Offsets are CHARACTER offsets, and that is load-bearing

**Postgres `substr()` on `text` counts characters, not bytes.** Byte offsets would leave SQL unable to
slice a chunk safely (multibyte content corrupts), which would force FTS to aggregate *block* content —
quietly demoting chunks out of their role and pulling heading text into the search vector for the first
time.

Character offsets avoid all of it: **chunks remain the embeddable and tsvector-able units exactly as
today.** `_rebuild_resource_search_vector` still aggregates chunk text; it merely *derives* that text by
`substr` instead of reading a stored copy. **No search behavior change. No re-embed. Existing vectors
stay valid.**

### Chunk ranges are a *gappy* index, not a partition

A headed chunk's text has its `## Title` line **stripped** — precisely why `reconstruct_body` has to
re-synthesize headings, and precisely why the heading-duplication bug exists. So chunk ranges
deliberately **do not cover** heading lines: those characters live in the revision's content and belong
to no chunk. Therefore chunk text stays byte-identical to today, the body becomes byte-exact, and
**`reconstruct_body` is deleted, not fixed** — both fidelity bugs die with it.

### The body is `concat(blocks)` with **no** separator

Each block already carries its own trailing newline. Joining with `"\n"` *is* the bug class being
removed.

This forces one upstream change: `temper_ingest::stream::segment_reader` emits `src.lines()` — which
**strips line endings** — and its test rejoins with `"\n"`. That silently normalizes CRLF to LF and
drops a trailing newline. "Almost exact" is not exact. It must preserve line endings verbatim, and its
test must assert `join("") == doc`.

## The data shape, in SQL

Grounded against the **live** schema (`\d`), not the origin migrations.

### 1. The revision owns its content — immutable

```sql
CREATE TABLE kb_block_content (
    block_revision_id uuid PRIMARY KEY
        REFERENCES kb_block_revisions(id) ON DELETE CASCADE,
    content      text NOT NULL,
    content_hash text NOT NULL   -- sha256 hex of content's RAW BYTES
);
```

`content_hash` is the **raw-bytes** sha256 — the value the client already sends as
`AppendBlockPayload.content_hash` and which the server currently checks and then discards along with the
bytes. It is deliberately **distinct** from the sibling `kb_block_revisions.block_body_hash`, which is
the *chunk merkle*. Two hashes over the same content, answering different questions: "are these the
bytes?" and "are these the chunks?"

### 2. The anchor points at its current state

```sql
ALTER TABLE kb_content_blocks
    ADD COLUMN current_revision_id uuid REFERENCES kb_block_revisions(id);
```

The block-mutation path already `UPDATE`s this row (it maintains `last_event_id`), so this rides along
on a write that already happens. It also removes any dependence on "latest revision by `created`" for
correctness.

### 3. Chunks index into the revision they belong to

```sql
ALTER TABLE kb_chunks
    ADD COLUMN block_revision_id uuid REFERENCES kb_block_revisions(id),
    ADD COLUMN start_char        integer,
    ADD COLUMN end_char          integer;

ALTER TABLE kb_chunks
    ADD CONSTRAINT ck_kb_chunks_char_range CHECK (
        (block_revision_id IS NULL AND start_char IS NULL AND end_char IS NULL)  -- legacy: kb_chunk_content
     OR (block_revision_id IS NOT NULL AND start_char >= 0 AND end_char > start_char)
    );
```

The three columns move together — the CHECK makes "half-migrated chunk" unrepresentable. Ranges are
**half-open** and need **not** cover the revision; heading lines are the gaps.

### 4. The two discriminators, both defaulting to the honest legacy answer

```sql
ALTER TABLE kb_resources
    ADD COLUMN body_storage text NOT NULL DEFAULT 'derived',
    ADD COLUMN ingest_state text NOT NULL DEFAULT 'complete';

ALTER TABLE kb_resources
    ADD CONSTRAINT ck_kb_resources_body_storage
        CHECK (body_storage IN ('verbatim', 'derived')),
    ADD CONSTRAINT ck_kb_resources_ingest_state
        CHECK (ingest_state IN ('in_progress', 'complete'));

CREATE INDEX idx_kb_resources_incomplete
    ON kb_resources (owner_profile_id)
    WHERE ingest_state = 'in_progress';
```

`body_storage` defaults to `derived` so existing rows — **and anything written by an un-upgraded
server** — are labelled honestly; only the new write path opts *up* to `verbatim`. `ingest_state`
defaults to `complete` because existing rows genuinely are. Both are `ADD COLUMN … NOT NULL DEFAULT`,
catalog-only on PG11+ — **no table rewrite** on PG17 (Neon prod) or PG18 (local/CI).

### 5. Readback: concat, with **no** separator

```sql
SELECT string_agg(bc.content, '' ORDER BY b.seq) AS body
  FROM kb_content_blocks b
  JOIN kb_block_content  bc ON bc.block_revision_id = b.current_revision_id
 WHERE b.resource_id = $1 AND NOT b.is_folded;
```

The `''` separator *is* the fix. `reconstruct_body` is then deleted.

### 6. Chunk text is derived — with no `is_current` branch

```sql
SELECT c.id,
       substr(bc.content, c.start_char + 1, c.end_char - c.start_char) AS content
  FROM kb_chunks c
  JOIN kb_block_content bc ON bc.block_revision_id = c.block_revision_id
 WHERE c.resource_id = $1 AND c.is_current;
```

Drop the `is_current` filter and it works identically for **superseded** chunks — their revision's
content is still there, unchanged. That is the coherence win, in one line. The embed path and
`_rebuild_resource_search_vector` both switch from *reading* `kb_chunk_content` to *deriving* text this
way for `verbatim` rows; `derived` rows keep the existing query. **FTS and embeddings see exactly the
text they see today.**

### 7. Search and list exclude partials

```sql
  AND r.ingest_state = 'complete'
```

In the list/search query builders — **not** in `resources_visible_to`. Visibility is an *authorization*
predicate; completeness is a *content* predicate. Folding one into the other would quietly change who
can see what.

### 8. Replay

`kb_block_content` joins `PROJECTION_DUMPS` in `replay.rs`. The event sidecar lookup (which reconstructs
chunk prose + embedding from storage so the ledger diff stays total without re-running ONNX) swaps
`kb_chunk_content` for the `substr`-over-revision derivation. **The invariant is unchanged**, not
reworded: every chunk a content-bearing payload references still has its content, because its revision
still has it.

## The integrity chain becomes real

- **Per revision:** `content_hash = sha256(raw content bytes)`.
- **Per resource:** `body_hash = sha256(concat(current revision content ORDER BY seq))` — the hash of the
  actual document.
- **At finalize:** the server recomputes over stored content and compares to the client's
  `expected_body_hash`. **A mismatch fails the finalize.**
- **`body_storage`** discriminates the guarantee: `verbatim` (byte-exact; `body_hash` is a true integrity
  check) vs `derived` (legacy; reconstructed; `body_hash` attests chunks only).

## Ingest lifecycle

Two **orthogonal** axes, deliberately not conflated:

- **`ingest_state`**: `in_progress` → `complete`. *Are all the bytes here?*
- **`embedding_status`** (already derived, already on the wire): `pending` → `ready`. *Are the vectors
  ready?*

A one-shot create is atomic and is born `complete`. A segmented `begin` sets `in_progress`; **only
`finalize` flips it to `complete`**, and only after verifying both `expected_blocks` and
`expected_body_hash`. On mismatch the resource **stays `in_progress`** — still resumable, never silently
done.

Search and list exclude `in_progress`. That exclusion does most of the work by itself: a partial's chunks
may carry vectors, but a resource that cannot surface in search makes them harmless. So "defer embedding"
reduces to one narrow thing — **do not enqueue the server drain job until finalize**, so an abandoned
upload never burns embed compute.

**No garbage collection.** An `in_progress` resource is resumable indefinitely, and auto-deleting user
data on a timer is a poor default for a knowledge base. Partials are visible to their owner on request;
`resource delete` cleans up. A sweeper is a later, informed decision if they actually accumulate.

## The TUS question

The task asked us to evaluate [`salvo-tus`](https://crates.io/crates/salvo-tus). **Do not adopt TUS.**
Its value was comparative, and the comparison says we already have the shape.

| TUS | temper today |
|---|---|
| `HEAD` → discover resume point | `GET /api/resources/{id}/blocks` → landed blocks |
| `PATCH` → append at offset | `POST /api/resources/{id}/blocks` → append `seq` |
| completion | `finalize` |

Same handshake; our unit is a *semantic segment with a `seq`* rather than a byte offset. `salvo-tus`
does not drop in regardless (we are **Axum**, not Salvo), and **vanilla TUS models an opaque byte
stream** — it has no notion of blocks-as-attribution-anchors, provenance, chunks, or embeddings.
Adopting it would mean uploading an opaque blob and re-deriving everything server-side, which the
companion measurement shows is ~10× slower.

What we lack is not the protocol but its **guarantees**: byte-exact verification at completion, an
explicit completion state, and **idempotent append** — a re-POSTed block after a network blip must be
safe. Today the unique index on `(resource_id, seq)` would simply reject it. It must be: **same
`content_hash` → no-op success; different `content_hash` → conflict.**

## Migration

**Additive-only on `main`** (DEPLOYING.md). One new table (`kb_block_content`); four new nullable columns
(`kb_content_blocks.current_revision_id`, `kb_chunks.block_revision_id`/`start_char`/`end_char`); two new
defaulted columns on `kb_resources`. Use `uuid_generate_v7()`, never native `uuidv7()` — the latter passes
PG18 dev/CI and breaks Neon's PG17.

**No backfill.** Legacy rows keep `kb_chunk_content` and their embeddings, with NULL offsets, and continue
to read back through `reconstruct_body`. They are labelled `derived` and **heal on their next write**.

Accepted cost: for a period, two text mechanisms coexist (revision content for `verbatim`, sidecar text for
`derived`). `kb_chunk_content` can only be dropped once no `derived` rows remain — possibly not without a
deliberate cutover. That is the price of not laundering unrecoverable data, and it is worth paying.

Wire contracts stay additive (a deployed instance must not hard-fail across version skew): new response
fields are optional.

## Testing

- **Byte-exact round-trip property test** — the hand-run differential, promoted. Random bodies including
  **CRLF**, **trailing newline**, **no trailing newline**, and multibyte unicode: PUT → GET → assert
  `sha256` identity.
- **Chunk text is unchanged** — for a `verbatim` resource, `substr`-derived chunk text must equal what the
  old `kb_chunk_content` path produced. This is the guard that "no re-embed needed" is true and that search
  behavior did not move.
- **Superseded chunks still resolve** — mutate a block, then read a superseded chunk's text and assert it
  is the *old* revision's slice, not the new one. This is the invariant Option 3 buys, so it gets a test.
- **Kill-mid-ingest** — leaves an `in_progress` resource: absent from search, absent from list, readable via
  `show`, clearly incomplete. Resume completes it.
- **Bad `expected_body_hash` at finalize** → fails; the resource stays `in_progress` and resumable.
- **Idempotent append** — same `seq` + same `content_hash` is a no-op; a different hash conflicts.
- **Legacy regression guard** — `derived` rows still read back through the old path.
- **Replay equivalence** — unchanged, with `kb_block_content` in the projection diff.
- E2E driven through the **CLI**, the production caller.

## Open questions

- Whether `derived` rows ever get a deliberate cutover, or simply age out.
- **Semantic blocking is out of scope, and deliberately unblocked.** Today's size-cut blocks are a
  transport artifact. When the tooling learns to cut on semantic boundaries, the `body = concat(blocks)`
  invariant makes re-blocking a restructure of boundaries over unchanged bytes.

*(Resolved during design: "does anything read superseded chunk text?" — yes, `replay.rs` does, as a stated
proof obligation. Option 3 makes the question moot: superseded chunks resolve against their own immutable
revision, so nothing is lost and the obligation holds verbatim.)*
