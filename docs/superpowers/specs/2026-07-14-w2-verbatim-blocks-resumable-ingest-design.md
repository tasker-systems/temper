# W2 — Content blocks own their content: byte-exact round-trip and an ingest that completes honestly

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

So the block owning its own text is not a naming fix — it is **the natural completion of what a block
already is**. A discrete semantic unit that carries its own provenance and can be updated in place
should own the content that provenance is *about*. Today it does not, and that is the defect.

> **Today's reality, named honestly.** The segmented ingest cuts blocks on a **256 KB size budget**, so
> a large raw ingest becomes arbitrary byte-cuts (and a small one, a single giant block). Those are
> poor attribution anchors — an artifact of the *transport*, not the model. This design does **not**
> cement that: `body = concat(blocks ORDER BY seq)` holds whether blocks are size-cuts *or* real
> semantic units, so re-blocking a document later is a pure restructure of boundaries over the same
> bytes, and the body is invariant under it. The wart stays cheap to fix.

## The problem

**The resource's text is not stored where it belongs.** It lives only in `kb_chunk_content`, and the
body is *reconstructed* from chunks on read. Two consequences, both live in production:

1. **Round-trip is lossy.** `content::reconstruct_body` ends in `pieces.join("\n\n")`, so a chunk
   boundary falling mid-paragraph returns an inserted blank line. Measured: a 1,202,908-byte body reads
   back as 1,203,710 (+802). A differential across **both** write paths (client-embedded and
   server-embedded) returns **byte-identical, equally lossy** output — pre-existing and
   path-independent, not a regression. The heading-duplication bug (task `019f4694`) is its sibling:
   same root cause, because `reconstruct_body` must *re-synthesize* headings that chunk text strips.
2. **`body_hash` cannot detect any of it.** It is `body_hash_from_chunk_hashes` — a merkle over *chunk*
   hashes. It attests "the chunks you sent are the chunks I stored", and is structurally blind to a
   corruption introduced *after* the chunks are already correct.
3. **An interrupted segmented ingest is indistinguishable from a complete document.** The resource is
   created at `begin`, so a killed upload leaves it listed, searchable, and `status: ok` holding 93% of
   its content. `kb_ingestion_records` has **no state column**; nothing in the schema can say "not
   finished".

The same failure shape runs through all of it — the silently-**partial** document (set 3), the
silently-**empty** one (the PDF session's F1), the silently-**unembedded** one. **A knowledge base that
cannot tell you what it does not have is worse than one that fails loudly.**

## Decisions

| Question | Decision |
|---|---|
| What must round-trip guarantee? | **Byte-exact.** `sha256(PUT) == sha256(GET)`. |
| Where does text live? | **On the block.** One home. Chunks reference it by offset. |
| Existing rows, whose original bytes were never kept? | **Mark them honestly.** A `body_storage` discriminator; no laundering backfill of bytes nobody kept. |
| A pre-finalize partial? | **Invisible until finalized.** Excluded from search and list; `show` works and says it is incomplete. |
| Adopt `salvo-tus`? | **No.** We already have TUS's *shape*; we lack its *guarantees*. |

## Architecture: move the text, do not copy it

**Chunking has no overlap**, so chunk text is already a partition of the body — storing raw block text
*alongside* it would be a straight 2× duplication for nothing. So the text **moves**:

- **`kb_block_content(block_id, content, content_hash)`** — a sidecar (same shape as the existing
  `kb_chunk_content`) and the single home for text. The block now owns the content its provenance
  refers to.
- **`kb_chunks` gains `start_char` / `end_char`** — offsets into its block's content.
- **`kb_chunk_content` is retired** for `verbatim` rows.

Net storage is roughly **flat**: the body is added once, the chunk copy removed once. **No row holds
its text twice in either state.**

### Offsets are CHARACTER offsets, and that is load-bearing

**Postgres `substr()` on `text` counts characters, not bytes.** Had the offsets been *bytes*, SQL could
not slice a chunk safely (multibyte content would corrupt), which would have forced FTS to aggregate
**block** content instead — quietly demoting chunks out of their role and, as a side effect, pulling
heading text into the search vector for the first time.

Character offsets avoid all of that:

- **Chunks remain the embeddable and tsvector-able units, exactly as today.** `_rebuild_resource_search_vector`
  keeps aggregating chunk text; it just derives that text by `substr` instead of reading a stored copy.
- **No search behavior change.** No re-embed. Existing vectors stay valid.

The layering is preserved: **block = addressable semantic unit that owns its content and its
provenance; chunk = the derived retrieval unit beneath it.**

### Chunk ranges are a *gappy* index, not a partition

Today a headed chunk's text has its `## Title` line **stripped** — precisely why `reconstruct_body` has
to re-synthesize headings, and precisely why the heading-duplication bug exists.

So chunk ranges deliberately **do not cover** heading lines. Those characters live in the block and
belong to no chunk. Therefore:

- **Chunk text stays byte-identical to today** (heading-stripped) — same slice. **No re-embed.**
- **The body is byte-exact** — headings live in the block, where they belong.
- **`reconstruct_body` is deleted, not fixed.** Both fidelity bugs die with it, because nothing
  reconstructs anything any more.

### The body is `concat(blocks)` with **no** separator

Each block already carries its own trailing newline. Joining with `"\n"` *is* the bug class being
removed.

This forces one upstream change: `temper_ingest::stream::segment_reader` emits `src.lines()` — which
**strips line endings** — and its test rejoins with `"\n"`. That silently normalizes CRLF to LF and
drops a trailing newline. "Almost exact" is not exact. It must preserve line endings verbatim, and its
test must assert `join("") == doc`.

## The data shape, in SQL

Grounded against the **live** schema (`\d`), not the origin migrations.

### 1. The block owns its content

```sql
CREATE TABLE kb_block_content (
    block_id     uuid PRIMARY KEY REFERENCES kb_content_blocks(id) ON DELETE CASCADE,
    content      text NOT NULL,
    content_hash text NOT NULL   -- sha256 hex of the content's raw bytes
);
```

`content_hash` is the block's raw-bytes sha256 — **the same value the client already sends** as
`AppendBlockPayload.content_hash`, and which the server currently checks and then discards along with
the bytes. Storing it makes the resume diff a direct comparison and removes a recompute.

### 2. Chunks become a gappy character-range index into their block

```sql
ALTER TABLE kb_chunks
    ADD COLUMN start_char integer,
    ADD COLUMN end_char   integer;

ALTER TABLE kb_chunks
    ADD CONSTRAINT ck_kb_chunks_char_range CHECK (
        (start_char IS NULL AND end_char IS NULL)      -- legacy: text lives in kb_chunk_content
     OR (start_char >= 0 AND end_char > start_char)    -- verbatim: half-open [start_char, end_char)
    );
```

NULL offsets are the discriminator at chunk grain. Ranges are **half-open** and need **not** cover the
block — heading lines are the gaps.

### 3. The two discriminators, both defaulting to the honest legacy answer

```sql
ALTER TABLE kb_resources
    ADD COLUMN body_storage text NOT NULL DEFAULT 'derived',
    ADD COLUMN ingest_state text NOT NULL DEFAULT 'complete';

ALTER TABLE kb_resources
    ADD CONSTRAINT ck_kb_resources_body_storage
        CHECK (body_storage IN ('verbatim', 'derived')),
    ADD CONSTRAINT ck_kb_resources_ingest_state
        CHECK (ingest_state IN ('in_progress', 'complete'));

-- The owner's "what did I leave half-uploaded?" query.
CREATE INDEX idx_kb_resources_incomplete
    ON kb_resources (owner_profile_id)
    WHERE ingest_state = 'in_progress';
```

`body_storage` defaults to `derived` so existing rows — **and anything written by an un-upgraded
server** — are labelled honestly; only the new write path opts *up* to `verbatim`. `ingest_state`
defaults to `complete` because existing rows genuinely are. Both are `ADD COLUMN … NOT NULL DEFAULT`,
catalog-only on PG11+ — **no table rewrite** on PG17 (Neon prod) or PG18 (local/CI).

### 4. Readback: concat, with **no** separator

```sql
SELECT string_agg(bc.content, '' ORDER BY b.seq) AS body
  FROM kb_content_blocks b
  JOIN kb_block_content  bc ON bc.block_id = b.id
 WHERE b.resource_id = $1 AND NOT b.is_folded;
```

The `''` separator *is* the fix. `reconstruct_body` is then deleted.

### 5. Chunk text is derived — and chunks keep their job

```sql
SELECT c.id,
       substr(bc.content, c.start_char + 1, c.end_char - c.start_char) AS content
  FROM kb_chunks c
  JOIN kb_block_content bc ON bc.block_id = c.block_id
 WHERE c.resource_id = $1 AND c.is_current AND c.start_char IS NOT NULL;
```

`substr` is character-based, and so are the offsets — correct for multibyte content. The embed path and
`_rebuild_resource_search_vector` both switch from *reading* `kb_chunk_content` to *deriving* the same
text this way, for `verbatim` rows; `derived` rows keep the existing query unchanged. **The FTS vector
and the embeddings see exactly the text they see today.**

### 6. Search and list exclude partials

```sql
  AND r.ingest_state = 'complete'
```

In the list/search query builders — **not** in `resources_visible_to`. Visibility is an *authorization*
predicate; completeness is a *content* predicate. Folding one into the other would quietly change who
can see what.

### Consequence of in-place block revision — state it, don't discover it

Blocks are revised **in place** (2,627 revisions across 2,442 live blocks; `kb_block_revisions` records
`block_body_hash` + `chunk_count` per revision, **not** content). So `kb_block_content` holds the
block's **current** content, and offsets are meaningful **only for `is_current` chunks**.

A superseded `verbatim` chunk therefore keeps its `content_hash` (its identity) but has **no retrievable
text** — its offsets would point into content that has since changed. This is consistent with block
history already being hash-only. **It is a real difference from today**, where `kb_chunk_content` retains
text for superseded chunks. Implementation must first confirm nothing reads it (`replay.rs` references
`kb_chunk_content`). If something does, superseded chunks keep their sidecar row and only current chunks
move to offsets.

## The integrity chain becomes real

- **Per block:** `content_hash = sha256(raw block bytes)`.
- **Per resource:** `body_hash = sha256(concat(block content))` — the hash of the actual document.
- **At finalize:** the server recomputes over stored content and compares to the client's
  `expected_body_hash`. **A mismatch fails the finalize.**
- **`body_storage`** discriminates the guarantee: `verbatim` (byte-exact; `body_hash` is a true
  integrity check) vs `derived` (legacy; reconstructed; `body_hash` attests chunks only).

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
stream** — it has no notion of our blocks-as-attribution-anchors, provenance, chunks, or embeddings.
Adopting it would mean uploading an opaque blob and re-deriving everything server-side, which the
companion measurement shows is ~10× slower.

What we lack is not the protocol but its **guarantees**: byte-exact verification at completion, an
explicit completion state, and **idempotent append** — a re-POSTed block after a network blip must be
safe. Today the unique index on `(resource_id, seq)` would simply reject it. It must be: **same
`content_hash` → no-op success; different `content_hash` → conflict.**

## Migration

**Additive-only on `main`** (DEPLOYING.md). New table `kb_block_content`; new nullable
`kb_chunks.start_char`/`end_char`; `kb_resources.body_storage` (default `derived`) and `ingest_state`
(default `complete`). Use `uuid_generate_v7()`, never native `uuidv7()` — the latter passes PG18
dev/CI and breaks Neon's PG17.

Legacy `derived` rows keep `kb_chunk_content` and their embeddings, with NULL offsets, and continue to
read back through `reconstruct_body`. They **heal on their next write**, not by migration.

Accepted cost: for a period, two text mechanisms coexist (offsets for `verbatim`, sidecar text for
`derived`). `kb_chunk_content` can only be dropped once no `derived` rows remain — possibly not without
a deliberate cutover. That is the price of not laundering unrecoverable data, and it is worth paying.

Wire contracts stay additive (a deployed instance must not hard-fail across version skew): new response
fields are optional.

## Testing

- **Byte-exact round-trip property test** — the hand-run differential, promoted. Random bodies including
  **CRLF**, **trailing newline**, **no trailing newline**, and multibyte unicode: PUT → GET → assert
  `sha256` identity.
- **Chunk text is unchanged** — for a `verbatim` resource, the `substr`-derived chunk text must equal
  what the old `kb_chunk_content` path produced. This is the guard that "no re-embed needed" is true and
  that search behavior did not move.
- **Kill-mid-ingest** — leaves an `in_progress` resource: absent from search, absent from list, readable
  via `show`, clearly incomplete. Resume completes it.
- **Bad `expected_body_hash` at finalize** → fails; the resource stays `in_progress` and resumable.
- **Idempotent append** — same `seq` + same `content_hash` is a no-op; a different hash conflicts.
- **Legacy regression guard** — `derived` rows still read back through the old path.
- E2E driven through the **CLI**, the production caller.

## Open questions

- **Does anything read superseded (`is_current = false`) chunk text?** If yes, superseded chunks keep
  their `kb_chunk_content` row and only current chunks move to offsets. Must be settled before the
  migration — `replay.rs` first.
- Whether `derived` rows ever get a deliberate cutover, or simply age out.
- **Semantic blocking is out of scope, and deliberately unblocked by this design.** Today's size-cut
  blocks are a transport artifact. When the tooling learns to cut blocks on semantic boundaries, the
  `body = concat(blocks)` invariant means re-blocking is a restructure of boundaries over unchanged
  bytes.
