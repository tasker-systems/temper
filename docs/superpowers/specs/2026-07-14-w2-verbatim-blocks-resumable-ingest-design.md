# W2 — Verbatim content blocks: byte-exact round-trip and a resumable ingest that completes honestly

**Task:** `019f5fef-bc3a-7851-8e7a-6fbc1a1cb265` (#420 set 3) · **Branch:** `jct/server-side-embed-drop-cli-onnx`
**Status:** design — approved · **Date:** 2026-07-14

Companion to [the embed-location measurement](2026-07-14-server-side-embed-drop-cli-onnx-design.md),
which established that embedding stays client-side. This spec covers **W2**: make the ingest path
byte-exact and make an interrupted ingest impossible to mistake for a finished one.

## The problem, in one line

**The body is never stored.** It is *reconstructed* from chunks, so fidelity is a property of the
chunker — and `body_hash`, being a merkle over **chunk** hashes, can never detect the difference.

Two defects follow, and both are live in production today:

1. **Round-trip is lossy.** `content::reconstruct_body` ends in `pieces.join("\n\n")`, so a chunk
   boundary that fell mid-paragraph returns an inserted blank line. Measured: a 1,202,908-byte body
   reads back as 1,203,710 bytes (+802). A differential over both write paths (client-embedded and
   server-embedded) returns **byte-identical, equally lossy** output — so this is pre-existing and
   path-independent, not a regression. The known heading-duplication bug (task `019f4694`) is its
   sibling: same root cause.
2. **An interrupted segmented ingest is indistinguishable from a complete document.** The resource is
   created at `begin`, so a killed upload leaves it listed, searchable, and `status: ok` holding 93%
   of its content. `kb_ingestion_records` has **no state column**; nothing in the schema can express
   "not finished".

Both are the same failure shape this task exists to kill — the silently-**partial** document (set 3),
the silently-**empty** one (the PDF session's F1), the silently-**unembedded** one. A knowledge base
that cannot tell you what it does not have is worse than one that fails loudly.

## Decisions taken

| Question | Decision |
|---|---|
| What must round-trip guarantee? | **Byte-exact.** `sha256(PUT) == sha256(GET)`. |
| Existing rows, whose original bytes were never kept? | **Mark them honestly.** A `body_storage` discriminator; never claim byte-exactness a row cannot back. No laundering backfill. |
| How should a pre-finalize partial behave? | **Invisible until finalized.** Excluded from search and list; `show` works and says it is incomplete. |
| Adopt `salvo-tus`? | **No.** We already have TUS's *shape*; we lack its *guarantees*. See "The TUS question". |

## Architecture: move the text, do not copy it

Today `kb_content_blocks` stores **no content** — it is `(id, resource_id, seq, is_folded,
genesis_event_id, last_event_id)`. The only text in the system is `kb_chunk_content`. A table named
*content blocks* that holds no content is the whole bug in miniature.

The naive fix — store raw block text *alongside* chunk text — doubles storage for no reason.
**Chunking has no overlap**, so chunk text is already a partition of the body: keeping both would be a
straight 2× duplication. Instead:

- **`kb_block_content(block_id, content)`** — a sidecar (same shape as the existing
  `kb_chunk_content`) and the **single home for text**.
- **`kb_chunks` gains `start_byte` / `end_byte`** — offsets into its block. Chunk text becomes
  `substr(block.content, …)`, derived on demand.
- **`kb_chunk_content` is retired** for verbatim rows.

Net storage is roughly **flat**: the body is added once, the chunk copy removed once. **No row holds
its text twice in either state.**

### Chunk ranges are a *gappy index*, not a partition

This is the load-bearing detail. Today a headed chunk's text has its `## Title` line **stripped** —
which is exactly why `reconstruct_body` must re-synthesize headings, and exactly why the
heading-duplication bug exists.

So chunk ranges deliberately **do not cover** the heading lines. Those bytes live in the block and
belong to no chunk. Consequences:

- **Chunk text stays semantically identical to today** (heading-stripped) — it is the same byte range.
  **No re-embed is required; existing vectors remain valid.**
- **The body is byte-exact** — headings live in the block, where they belong.
- **`reconstruct_body` is deleted, not fixed.** Both fidelity bugs die with it, because nothing
  reconstructs anything any more. `readback::body` becomes a concat of block content ordered by `seq`.

### The body is `concat(blocks)` with **no separator**

Each block already carries its own trailing newline. Joining with `"\n"` is precisely the class of bug
being removed.

This forces one upstream change: `temper_ingest::stream::segment_reader` currently emits `src.lines()`
(which **strips line endings**) and its test rejoins with `"\n"`. That silently normalizes CRLF to LF
and drops a trailing newline — "almost exact" is not exact. It must emit **exact byte ranges**, and
its test must assert `join("") == doc`.

## The data shape, in SQL

Grounded against the **live** schema (`\d`), not the origin migrations.

### 1. Text gets a single home: the block

A sidecar keyed by its parent, exactly mirroring the existing `kb_chunk_content` shape.

```sql
CREATE TABLE kb_block_content (
    block_id     uuid PRIMARY KEY REFERENCES kb_content_blocks(id) ON DELETE CASCADE,
    content      text NOT NULL,
    content_hash text NOT NULL   -- sha256 hex of content's RAW BYTES
);
```

`content_hash` is the block's raw-bytes sha256 — **the same value the client already sends** as
`AppendBlockPayload.content_hash` and which the server currently discards after checking. Storing it
makes the resume diff a straight comparison and removes a recompute.

### 2. Chunks become a gappy byte-range index into their block

```sql
ALTER TABLE kb_chunks
    ADD COLUMN start_byte integer,
    ADD COLUMN end_byte   integer;

ALTER TABLE kb_chunks
    ADD CONSTRAINT ck_kb_chunks_byte_range CHECK (
        (start_byte IS NULL AND end_byte IS NULL)      -- legacy: text lives in kb_chunk_content
     OR (start_byte >= 0 AND end_byte > start_byte)    -- verbatim: half-open [start_byte, end_byte)
    );
```

NULL offsets are the discriminator at chunk grain: a `derived` chunk keeps its text in
`kb_chunk_content`; a `verbatim` chunk derives it from the block. Ranges are **half-open** and need
**not** cover the block — heading lines are the gaps.

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

`body_storage` defaults to `derived` so that existing rows — **and anything written by an un-upgraded
server** — are labelled honestly; only the new write path opts *up* to `verbatim`. `ingest_state`
defaults to `complete` because existing rows genuinely are. Both are `ADD COLUMN … NOT NULL DEFAULT`,
which is catalog-only on PG11+ — **no table rewrite** on either PG17 (Neon prod) or PG18 (local/CI).

### 4. Readback: concat, with **no** separator

```sql
SELECT string_agg(bc.content, '' ORDER BY b.seq) AS body
  FROM kb_content_blocks b
  JOIN kb_block_content  bc ON bc.block_id = b.id
 WHERE b.resource_id = $1 AND NOT b.is_folded;
```

The `''` separator *is* the fix. `reconstruct_body` is then deleted.

### 5. Byte offsets are sliced in **Rust**, never in SQL

**Postgres `substr()` on `text` counts characters, not bytes.** Slicing a byte range in SQL would
corrupt any multibyte content. So chunk-text derivation happens in Rust (`&content[start..end]` —
byte-indexed, and it panics on a non-char boundary, which is the correct failure for a range that
should always land on one). The embed and readback paths already materialize rows into Rust, so this
costs nothing.

This is also why the FTS change below is **load-bearing rather than optional** — it is what keeps SQL
out of the slicing business entirely.

### 6. FTS: aggregate the block for `verbatim`, unchanged for `derived`

`_rebuild_resource_search_vector` currently does `string_agg(cc.content, ' ')` over chunk text. It
branches:

```sql
IF v_body_storage = 'verbatim' THEN
    SELECT COALESCE(string_agg(bc.content, ' ' ORDER BY b.seq), '')
      INTO v_body
      FROM kb_content_blocks b
      JOIN kb_block_content  bc ON bc.block_id = b.id
     WHERE b.resource_id = p_resource AND NOT b.is_folded;
ELSE
    -- legacy path, byte-for-byte unchanged
    SELECT COALESCE(string_agg(cc.content, ' '), '')
      INTO v_body
      FROM kb_chunks c
      JOIN kb_chunk_content cc ON cc.chunk_id = c.id
     WHERE c.resource_id = p_resource AND c.is_current;
END IF;
```

Note the **behavior change, stated plainly**: chunk text is heading-stripped, so headings are *not* in
FTS today. Aggregating the block puts them in (at weight `B`; titles keep `A`). That is strictly more
recall and, in my view, a fix — but it *is* a change to search results, and it is now unavoidable
rather than optional, because the alternative is slicing bytes in SQL. (This supersedes the earlier
"out of scope" note.)

### 7. Search and list exclude partials

```sql
  AND r.ingest_state = 'complete'
```

In the list/search query builders — **not** in `resources_visible_to`.

### Consequence of in-place block revision — state it, don't discover it

Blocks are revised **in place** (2,627 revisions across 2,442 live blocks; `kb_block_revisions` records
`block_body_hash` + `chunk_count` per revision, **not** content). So `kb_block_content` holds the
block's **current** content, and byte offsets are meaningful **only for `is_current` chunks**.

A superseded `verbatim` chunk therefore keeps its `content_hash` (its identity) but has **no
retrievable text** — its offsets would point into content that has since changed. This is consistent
with block history already being hash-only. **It is a real difference from today**, where
`kb_chunk_content` retains text for superseded chunks too, so implementation must first confirm nothing
reads it (`replay.rs` references `kb_chunk_content`). If something does, superseded chunks keep their
sidecar row and only current chunks move to offsets.

## The integrity chain becomes real

`body_hash` is currently `body_hash_from_chunk_hashes` — a merkle over **chunk** hashes. It attests
"the chunks you sent are the chunks I stored"; it structurally *cannot* catch a corruption introduced
after the chunks are already correct. Redefine it:

- **Per block:** `content_hash = sha256(raw block bytes)`. The client **already sends exactly this** on
  `append` — the server simply does not keep the bytes it hashes.
- **Per resource:** `body_hash = sha256(concat(block bytes))` — the hash of the actual document.
- **At finalize:** the server recomputes the hash over stored bytes and compares to the client's
  `expected_body_hash`. **A mismatch fails the finalize.**
- **`body_storage`** discriminates the guarantee: `verbatim` (byte-exact; `body_hash` is a true
  integrity check) vs `derived` (legacy; reconstructed; `body_hash` attests chunks only).

## Ingest lifecycle

Two **orthogonal** axes, deliberately not conflated:

- **`ingest_state`**: `in_progress` → `complete`. *Are all the bytes here?*
- **`embedding_status`** (already on the wire): `pending` → `ready`. *Are the vectors ready?*

A one-shot create is atomic and is born `complete`. A segmented `begin` sets `in_progress`; **only
`finalize` flips it to `complete`**, and only after verifying both `expected_blocks` and
`expected_body_hash`. On mismatch the resource **stays `in_progress`** — still resumable, never
silently done.

**Search and list exclude `in_progress`.** That filter belongs in the list/search queries, **not** in
`resources_visible_to`: visibility is an *authorization* predicate, completeness is a *content*
predicate, and folding one into the other would quietly change who can see what.

The exclusion also does most of the work by itself — a partial's chunks may carry vectors, but a
resource that cannot surface in search makes them harmless. So "defer embedding" reduces to one narrow
thing: **do not enqueue the server drain job until finalize**, so an abandoned upload never burns embed
compute.

**No garbage collection.** An `in_progress` resource is resumable indefinitely, and auto-deleting user
data on a timer is a poor default for a knowledge base. Partials are visible to their owner on request;
`resource delete` cleans up. A sweeper is a later, informed decision if they actually accumulate.

## The TUS question

The task asked us to evaluate [`salvo-tus`](https://crates.io/crates/salvo-tus). **Do not adopt TUS.**
The study's value was comparative, and the comparison says we already have the shape.

| TUS | temper today |
|---|---|
| `HEAD` → discover resume offset | `GET /api/resources/{id}/blocks` → landed blocks |
| `PATCH` → append at offset | `POST /api/resources/{id}/blocks` → append `seq` |
| completion | `finalize` |

Same handshake; our unit is a *semantic segment with a `seq`* rather than a byte offset. `salvo-tus`
does not drop in regardless (we are **Axum**, not Salvo), and more fundamentally **vanilla TUS models
an opaque byte stream** — it has no notion of our per-block merkle, chunks, embeddings, or provenance.
Adopting it would mean uploading an opaque blob and re-deriving everything server-side, which the
companion measurement shows is ~10× slower.

What we lack is not the protocol but its **guarantees**, and they are the rest of this spec:
byte-exact verification at completion, an explicit completion state, and **idempotent append** — a
re-POSTed block after a network blip must be safe. Today the unique index on `(resource_id, seq)` would
simply reject it. It must instead be: **same `content_hash` → no-op success; different `content_hash` →
conflict.**

## Migration

**Additive-only on `main`** (DEPLOYING.md):

- New table `kb_block_content`.
- New **nullable** `kb_chunks.start_byte` / `end_byte`.
- `kb_resources.body_storage`, defaulting to **`derived`** — so existing rows, *and anything written by
  an un-upgraded server*, are honestly labelled. Only the new write path opts **up** to `verbatim`.
- `kb_resources.ingest_state`, defaulting to **`complete`** — existing rows genuinely are.

Legacy `derived` rows keep `kb_chunk_content` and their existing embeddings, with `start_byte`/
`end_byte` NULL, and continue to read back through `reconstruct_body`. They **heal on their next
write**, not by migration.

Accepted cost: for a period, two text mechanisms coexist (offsets for `verbatim`, sidecar text for
`derived`). `kb_chunk_content` can only be dropped once no `derived` rows remain — possibly not without
a deliberate cutover. This is the price of not laundering unrecoverable data, and it is worth paying.

Wire contracts stay additive (a deployed instance must not hard-fail across version skew): new response
fields are optional.

## Testing

- **Byte-exact round-trip property test** — the hand-run differential, promoted. Random bodies
  including **CRLF**, **trailing newline**, **no trailing newline**, and unicode: PUT → GET → assert
  `sha256` identity.
- **Differential across write paths** — client-embedded and server-embedded readback must be identical
  *and* faithful (today they are identical and both lossy).
- **Kill-mid-ingest** — leaves an `in_progress` resource: absent from search, absent from list, readable
  via `show`, clearly incomplete. Resume completes it.
- **Bad `expected_body_hash` at finalize** → fails; resource stays `in_progress` and resumable.
- **Idempotent append** — same `seq` + same `content_hash` is a no-op; a different hash conflicts.
- **Legacy regression guard** — `derived` rows still read back through the old path.
- E2E driven through the **CLI**, the production caller.

## Open questions

- **Does anything read superseded (`is_current = false`) chunk text?** If yes, superseded chunks keep
  their `kb_chunk_content` row and only current chunks move to offsets. Must be settled before the
  migration — `replay.rs` is the first place to look.
- Whether `derived` rows ever get a deliberate cutover, or simply age out.
- FTS gaining heading text (§6) is a deliberate, unavoidable behavior change, not an open question —
  but its effect on ranking should be eyeballed on real queries once it lands.
