# W2 — The block revision owns the bytes; chunks are the index. Byte-exact round-trip and an ingest that completes honestly

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
  "**this** claim came from **that** source" expressible at all — a first-class concern for agent-made
  statements of fact.
- **The unit of in-place partial update** — how a *part* of a document changes without rewriting the
  whole document.
- **A discrete semantic unit of intent** — a thing-to-itself, sequenced within its resource.

**Chunks live deliberately beneath that layer**: they are the units of *what is embeddable and
tsvector-able*.

> **Today's reality, named honestly.** The segmented ingest cuts blocks on a **256 KB size budget**, so
> a large raw ingest becomes arbitrary byte-cuts (and a small one, a single giant block). Those are poor
> attribution anchors — an artifact of the *transport*, not the model. This design does **not** cement
> that: `body = concat(blocks ORDER BY seq)` holds whether blocks are size-cuts *or* real semantic
> units, so re-blocking later is a pure restructure of boundaries over unchanged bytes.

## The problem: the only copy of the document lives inside the index

The resource's text exists **nowhere but `kb_chunk_content`**, and the body is *reconstructed* from
chunks on read. That is the root defect, and everything else follows from it.

**Chunks are a lossy transform of the source, not a slice of it.** `chunk.rs`:

```rust
let full_text = lines.join("\n");          // CRLF → LF; rebuilt from lines
let trimmed = full_text.trim();            // leading/trailing whitespace gone
if trimmed.is_empty() { return vec![] }    // whitespace-only regions produce NO chunk
```

…and `content_hash` is `sha256(content.trim())`. Heading lines are stripped out entirely and
*re-synthesized* on read. So the source bytes are **destroyed at chunk time**. No amount of fixing the
*join* recovers them.

Three consequences, all live in production:

1. **Round-trip is lossy.** `content::reconstruct_body` ends in `pieces.join("\n\n")`, so a chunk
   boundary falling mid-paragraph returns an inserted blank line. Measured: a 1,202,908-byte body reads
   back as 1,203,710 (+802). A differential across **both** write paths returns **byte-identical,
   equally lossy** output — pre-existing and path-independent, not a regression. The heading-duplication
   bug (task `019f4694`) is its sibling: same root cause.
2. **`body_hash` cannot detect any of it.** It is a merkle over *chunk* hashes — it attests "the chunks
   you sent are the chunks I stored", and is structurally blind to a corruption introduced *after* the
   chunks are already correct.
3. **An interrupted segmented ingest is indistinguishable from a complete document.** The resource is
   created at `begin`, so a killed upload leaves it listed, searchable, and `status: ok` holding 93% of
   its content. Nothing in the schema can say "not finished".

The same failure shape runs through all of it — the silently-**partial** document (set 3), the
silently-**empty** one (the PDF session's F1), the silently-**unembedded** one. **A knowledge base that
cannot tell you what it does not have is worse than one that fails loudly.**

## The decision: store the bytes; keep the index

**The block revision owns the authoritative bytes. Chunks stay exactly as they are — the derived
retrieval index.**

Chunk text is a second copy of the same words, and **that is correct, not wasteful.** "Chunks are the
units of what is embeddable and tsvector-able" *is a description of an index*, and an index containing a
copy of what it indexes is what an index **is** — Postgres's own GIN index does precisely this. The
duplication becomes **intentional and named** — authoritative bytes on the revision, derived retrieval
copy in the chunks — rather than today's accident, where *the only copy of the document lives inside the
index*. That accident is exactly why the body got corrupted.

The deciding argument is **where fragility lives**. This task exists because a **chunker-coupled
reconstruction silently corrupted documents.** Persisting "glue" on chunks so they can be re-joined
exactly would fix *this bug* and preserve *the class*: the body's fidelity would remain a property of
the chunker forever, and any future chunking change could silently break it again. Storing the bytes
severs the coupling outright — **the chunker may change however it likes and the document is
untouched.**

Scale makes it cheap: the entire knowledge base is **33 MB** of chunk text. This takes it to ~66 MB.

| | |
|---|---|
| What must round-trip guarantee? | **Byte-exact.** `sha256(PUT) == sha256(GET)`. |
| Where do the authoritative bytes live? | **On the block revision.** Immutable once written. |
| What are chunks? | **The derived index.** Unchanged — no re-chunk, no re-embed, no `chunk.rs` change. |
| Legacy rows, whose original bytes were never kept? | **Marked honestly.** A `body_storage` discriminator; no laundering backfill. |
| A pre-finalize partial? | **Invisible until finalized.** Excluded from search and list; `show` works and says it is incomplete. |
| Adopt `salvo-tus`? | **No.** We already have TUS's *shape*; we lack its *guarantees*. |

## Why the *revision*, not the block

The schema already carries this model — verified against production:

- **`kb_block_revisions` already records `block_body_hash` + `chunk_count` per revision.** It is a
  **content-shaped record with no content** — a hash with nothing to attest.
- **`kb_chunks.version` already lines up 1:1 with a block's revision count**, exactly, at every N.
- **Every block has a revision row** (2,442 of 2,442). No special cases.
- A mutation **re-versions the whole chunk cohort**, so the entire old block's text is *already* retained
  today as superseded chunk rows.

Keying content on the revision (rather than the block) means content is **immutable once written**. That
buys:

- **Point-in-time block content, for free.** *What did this block say when that citation was anchored to
  it?* — answerable. For a system where provenance for agent-made statements of fact is first-class, that
  is the point, not a bonus.
- **No copy-on-supersession step**, and therefore no new silent-loss failure mode.
- **Replay's proof obligation survives untouched** — *"fold/supersede affect visibility, never
  existence"* — because `kb_chunk_content` is not modified at all by this design.

## The data shape, in SQL

Grounded against the **live** schema (`\d`), not the origin migrations.

### 1. The revision owns the authoritative bytes — immutable

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
bytes. It is deliberately **distinct** from the sibling `kb_block_revisions.block_body_hash`, which is the
*chunk merkle*. Two hashes over the same content, answering different questions: *"are these the bytes?"*
and *"are these the chunks?"*

### 2. The anchor points at its current state

```sql
ALTER TABLE kb_content_blocks
    ADD COLUMN current_revision_id uuid REFERENCES kb_block_revisions(id);
```

The block-mutation path already `UPDATE`s this row (it maintains `last_event_id`), so this rides along on
a write that already happens — and it removes any dependence on "latest revision by `created`" for
correctness.

### 3. `kb_chunks` and `kb_chunk_content` are **not touched**

No offsets. No new columns. No re-chunking. No re-embedding. **No `chunk.rs` change.** Chunks remain the
derived retrieval index, byte-for-byte as they are today, and every existing vector stays valid.

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
server** — are labelled honestly; only the new write path opts *up* to `verbatim`. `ingest_state` defaults
to `complete` because existing rows genuinely are. Both are `ADD COLUMN … NOT NULL DEFAULT`, catalog-only
on PG11+ — **no table rewrite** on PG17 (Neon prod) or PG18 (local/CI).

### 5. Readback: concat, with **no** separator

```sql
SELECT string_agg(bc.content, '' ORDER BY b.seq) AS body
  FROM kb_content_blocks b
  JOIN kb_block_content  bc ON bc.block_revision_id = b.current_revision_id
 WHERE b.resource_id = $1 AND NOT b.is_folded;
```

The `''` separator *is* the fix. For a `verbatim` resource, `reconstruct_body` is never called; it
survives only to serve `derived` rows, and dies with them.

### 6. Search and list exclude partials

```sql
  AND r.ingest_state = 'complete'
```

In the list/search query builders — **not** in `resources_visible_to`. Visibility is an *authorization*
predicate; completeness is a *content* predicate. Folding one into the other would quietly change who can
see what.

### 7. Untouched by design

**FTS** (`_rebuild_resource_search_vector`) keeps aggregating chunk text — no behavior change, no heading
text entering the search vector. **Embedding** reads chunk text exactly as today. **Replay** keeps its
`kb_chunk_content` CAS sidecar and its proof obligation **verbatim**; the only change is that
`kb_block_content` joins `PROJECTION_DUMPS` so block bytes are covered by the equivalence diff.

## The integrity chain becomes real

- **Per revision:** `content_hash = sha256(raw content bytes)`.
- **Per resource:** `body_hash = sha256(concat(current revision content ORDER BY seq))` — the hash of the
  actual document.
- **At finalize:** the server recomputes over stored content and compares to the client's
  `expected_body_hash`. **A mismatch fails the finalize.**
- **`body_storage`** discriminates the guarantee: `verbatim` (byte-exact; `body_hash` is a true integrity
  check) vs `derived` (legacy; reconstructed; `body_hash` attests chunks only).

### One upstream change: the segmenter must stop normalizing

`temper_ingest::stream::segment_reader` emits `src.lines()` — which **strips line endings** — and its test
rejoins with `"\n"`. That silently normalizes CRLF to LF and drops a trailing newline. "Almost exact" is not
exact. It must preserve line endings verbatim, and its test must assert `join("") == doc`.

This is the *only* ingest-pipeline code change. `chunk.rs` is untouched — it may keep trimming, dropping
empties, and normalizing, because it now feeds only the index.

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

**No garbage collection.** An `in_progress` resource is resumable indefinitely, and auto-deleting user data
on a timer is a poor default for a knowledge base. Partials are visible to their owner on request;
`resource delete` cleans up. A sweeper is a later, informed decision if they actually accumulate.

## The TUS question

The task asked us to evaluate [`salvo-tus`](https://crates.io/crates/salvo-tus). **Do not adopt TUS.** Its
value was comparative, and the comparison says we already have the shape.

| TUS | temper today |
|---|---|
| `HEAD` → discover resume point | `GET /api/resources/{id}/blocks` → landed blocks |
| `PATCH` → append at offset | `POST /api/resources/{id}/blocks` → append `seq` |
| completion | `finalize` |

Same handshake; our unit is a *semantic segment with a `seq`* rather than a byte offset. `salvo-tus` does
not drop in regardless (we are **Axum**, not Salvo), and **vanilla TUS models an opaque byte stream** — it
has no notion of blocks-as-attribution-anchors, provenance, chunks, or embeddings.

What we lack is not the protocol but its **guarantees**: byte-exact verification at completion, an explicit
completion state, and **idempotent append** — a re-POSTed block after a network blip must be safe. Today the
unique index on `(resource_id, seq)` would simply reject it. It must be: **same `content_hash` → no-op
success; different `content_hash` → conflict.**

## Migration

**Additive-only on `main`** (DEPLOYING.md). One new table (`kb_block_content`); one nullable column
(`kb_content_blocks.current_revision_id`); two defaulted columns on `kb_resources`. Nothing else. Use
`uuid_generate_v7()`, never native `uuidv7()` — the latter passes PG18 dev/CI and breaks Neon's PG17.

**No backfill.** Legacy rows have no `kb_block_content`, are labelled `derived`, and continue to read back
through `reconstruct_body`. They **heal on their next write**. Their original bytes are unrecoverable — they
were never stored — and this design refuses to launder a lossy reconstruction into "authoritative".

`reconstruct_body` (and `kb_chunk_content`'s role as the *only* copy of the document) can be retired once no
`derived` rows remain — possibly not without a deliberate cutover.

Wire contracts stay additive (a deployed instance must not hard-fail across version skew): new response
fields are optional.

## Testing

- **Byte-exact round-trip property test** — the hand-run differential, promoted. Random bodies including
  **CRLF**, **trailing newline**, **no trailing newline**, and multibyte unicode: PUT → GET → assert
  `sha256` identity.
- **Chunks are untouched** — for a body ingested both before and after, the chunk rows (text, hashes,
  vectors) must be identical. This is the guard that "no re-embed, no search behavior change" is true.
- **Point-in-time content** — mutate a block, then read the prior revision's content and assert it is the
  old bytes.
- **Kill-mid-ingest** — leaves an `in_progress` resource: absent from search, absent from list, readable via
  `show`, clearly incomplete. Resume completes it.
- **Bad `expected_body_hash` at finalize** → fails; the resource stays `in_progress` and resumable.
- **Idempotent append** — same `seq` + same `content_hash` is a no-op; a different hash conflicts.
- **Legacy regression guard** — `derived` rows still read back through `reconstruct_body`, unchanged.
- **Replay equivalence** — with `kb_block_content` in the projection diff.
- E2E driven through the **CLI**, the production caller.

## Open questions

- Whether `derived` rows ever get a deliberate cutover, or simply age out (and with them,
  `reconstruct_body`).
- **Semantic blocking is out of scope, and deliberately unblocked.** Today's size-cut blocks are a transport
  artifact. When the tooling learns to cut on semantic boundaries, the `body = concat(blocks)` invariant makes
  re-blocking a restructure of boundaries over unchanged bytes.

*(Resolved during design: an earlier draft had chunks carry character offsets into stored block content, to
avoid a second copy of the text. It was abandoned — `chunk.rs` trims, drops empty regions, and rejoins lines,
so chunk text is a lossy transform rather than a slice, and offsets would have required rewriting the chunker
to emit exact source spans. The duplication that approach avoided is, correctly understood, just an index.)*
