# Streaming, resumable multi-block ingestion

**Issue:** https://github.com/tasker-systems/temper/issues/313
**Status:** Design approved 2026-07-08
**Related:** #316 (chunking size-budget hard-split — landed), #299 (async embedding off the request path)

## 1. Problem

Every ingestion surface treats a document body as one atomic unit: read (or receive) the
whole body, chunk it, embed it, persist it, return — one call, no durable intermediate
checkpoint. That was fine for short notes. It does not hold for genuinely long-form
documents (contracts, transcripts, manuals, extracted-from-PDF text):

- The full body is resident in memory at multiple points on both sides of the wire.
- On the API/MCP surfaces the body must cross a Vercel serverless boundary that **caps the
  request body at ~4.5 MB and buffers it whole** — a multi-MB document physically cannot be
  sent in one request.
- An interrupted ingest (network drop, process kill, server restart) restarts from zero.

**Scope of this spec (narrowed during brainstorming).** The acute crash/OOM hazard the issue
opens with was a *distinct* bug — an ONNX Runtime ABI mismatch plus an unbounded-chunk escape —
already fixed by #316 (`hard_split_line` + tokenizer `with_truncation`, on `main`). What
remains, and what this spec addresses, is the **architecture**: bounded memory end-to-end,
transport that survives the 4.5 MB wall, and resumability. This spec covers the **CLI and API**
surfaces. MCP begin/append/finalize tools are deferred to a follow-on beat (§12).

## 2. The reframe — reject "chunk = the one true boundary"

The issue proposes making the embedding chunk the single unit for transport and storage. We
reject that: conflating the two is exactly what would put semantic chunk quality at risk. The
header-aware chunker (`chunk_markdown`) is what makes cosine similarity meaningful — chunks
align to document structure and carry a `header_path` breadcrumb. We keep two **decoupled**
granularities, each already a table:

| Unit | Table | Role | Size |
|---|---|---|---|
| **Segment** | `kb_content_blocks` (`seq`) | transport + durability + **resume checkpoint** | ~256 KB text (one tunable budget) |
| **Chunk** | `kb_chunks` (`chunk_index`) | semantic embedding unit — **unchanged** | ~510 tokens, header-path-aware |

A large body becomes an ordered sequence of blocks (`seq 0..N`); each block holds its own
header-aware chunks. A small body stays exactly one block (`seq 0`) — today's path, untouched.

**Why streaming does not degrade semantic chunking.** The only *global* state the semantic
chunker needs is the heading breadcrumb stack, which is **O(depth), not O(document)**. A
streaming pass carries that stack while holding only the current segment in memory, and
produces **byte-identical chunk boundaries and `header_path` values** to today's whole-document
`chunk_markdown` on native markdown. Structure-poor extraction (headerless PDF) chunks exactly
as it does today — flat, empty `header_path` — bounded by #316's size-based `hard_split`. We
explicitly do **not** invest in structure-inference for headerless content (§11, rejected): it
bakes in assumptions for marginal, unmeasured gains.

**The unifying insight.** A structure boundary (heading) is simultaneously the semantic-chunk
boundary, the natural streaming/segment boundary, and the resume checkpoint. When headings are
present everything works and streaming is free; when absent (raw extraction) all three degrade
together, and #316's size-based `hard_split` is the shared fallback boundary for all three.

## 3. Key insight — the persistence and event surfaces were built for this

Two audits of the current code (both confirmed against the tree) establish that this is an
**additive** change, not an invasive one.

### 3a. The SQL substrate is already multi-block

Everything downstream of the block manifest already tolerates a body spanning N blocks:

- **Readback / reassembly** — `resource_body_text` (`migrations/20260624000002_canonical_functions.sql:339`)
  and `readback::body` (`crates/temper-substrate/src/readback/mod.rs:457`) fetch all non-folded
  blocks and concatenate `ORDER BY b.seq, c.chunk_index`.
- **Body-hash merkle** — `_recompute_resource_body_hash`
  (`canonical_functions.sql:587`) iterates all non-folded blocks, `GROUP BY b.seq`, merkles
  per-block hashes `ORDER BY seq`. Genuinely multi-block.
- **Search** — resource-level throughout; `search_vector_candidates` collapses to best chunk per
  resource (`MIN(dist) GROUP BY resource_id`); `chunk_index` is never assumed globally unique.
  The stored FTS vector already iterates live blocks `ORDER BY b.seq, c.chunk_index`
  (`migrations/20260708000005_search_vector_live_blocks_only.sql:25`).
- **Per-block supersede** — `_project_block_mutated` (`canonical_functions.sql:914`) scopes
  `is_current`/`version` per `block_id`, not per resource — multi-block-safe.
- **Block projection** — `_project_blocks` (`canonical_functions.sql:619`) loops a manifest and
  inserts each block with parameterized `seq`. The partial unique index
  `(resource_id, seq) WHERE NOT is_folded` (`migrations/20260629000001_cogmap_charter_set.sql:18`)
  already exists.
- **Working precedent** — cognitive-map charters already create multi-block resources via
  `content::prepare_blocks` (`crates/temper-substrate/src/content.rs:302`).

The single-block assumption lives only in the Rust create wrapper: `create_resource_impl`
(`crates/temper-substrate/src/writes.rs:155`) hardcodes `prepare_block(0, …)` and
`let blocks = [block]`. `SeedAction::ResourceCreate` already takes a slice of blocks — only that
call site fixes it to one.

### 3b. The event ledger has a dormant hook for exactly this

- **`kb_events`** is the single append-only ledger (`_event_append` is "THE ONE EVENT WRITER",
  `canonical_functions.sql:765`; append-only enforced by trigger). `id` is uuidv7
  (monotonic/time-sortable); `correlation_id` is documented as grouping a multi-event act;
  `payload`/`metadata` are arbitrary jsonb.
- **`block_created`** is a *seeded event type with a `{resource_id, block_id, seq}` payload struct,
  wired into replay — but never fired** (`canonical_seed.sql:47`; `payloads.rs:554`;
  `replay.rs:407`). Today all blocks land folded inside one `resource_created` event. It is a
  ready-made hook sitting dormant, and it carries exactly the `seq` we need.
- There is **no** finalize/completed event type; adding one is one `kb_event_types` seed row +
  one `_event_append` call. `_event_append` explicitly supports projection-less events.

### 3c. `kb_ingestion_records` is orphaned but purpose-fit

Defined once (`canonical_schema.sql:426`, PK `resource_id`, columns `source_uri,
source_mimetype, conversion_tool, conversion_version, fetched_at, converted_at, source_hash`),
read only by a schema smoke test, **never written by any live path**. Its section header names
it "Ingestion idempotency". One row per resource; no status column, no jsonb. Perfect for its
*designed* role — per-resource source provenance + `source_hash` for idempotency — which this
spec finally uses.

## 4. Completion state — event-native, zero new columns or tables

"In-progress vs complete vs abandoned" is **derived from the append-only ledger**, not stored
as a flag. No column is added to `kb_resources`; no new table is created.

1. **Begin** fires `resource_created` for block 0 (as today), tagged with a `correlation_id`
   that is the ingest-session id.
2. **Each append** fires the dormant **`block_created`** event — payload
   `{resource_id, block_id, seq, content_hash}`, same `correlation_id`. Landed segments become
   ledger-derivable (and cross-check against `kb_content_blocks.seq`). This activates a type
   that was built and left unfired.
3. **Finalize** validates the landed set (all `seq 0..N` present; recomputed merkle matches the
   client's `expected_body_hash`) then fires one **new event type `resource_finalized`** —
   payload `{resource_id, expected_blocks, body_hash, source_hash}`, same `correlation_id`.
4. **`kb_ingestion_records`** is written for the first time: upsert one row per resource
   (`source_uri`, `source_hash`, `conversion_tool`/`version`, `fetched_at`, `converted_at`) at
   begin/finalize — an O(1) PK lookup for the resume source-integrity check and re-ingest
   idempotency.

Derived predicates, no stored status:

- **In-progress / abandoned** — a resource whose latest ingest-session `correlation_id` has
  `block_created` events but **no** `resource_finalized` event. A future reaper (§12, deferred)
  queries exactly this.
- **Complete** — a `resource_finalized` event exists for the session.
- **Push-over-push history** — the ledger itself: each push is a new `correlation_id`;
  re-pushes are new sessions, permanently recorded. `kb_ingestion_records` holds only the
  current source identity (upserted); the ledger holds the full history.

The one-shot small-body path is unaffected: it stays a single `resource_created` carrying one
block, fires no `block_created`, needs no `resource_finalized`.

## 5. Segment boundary rule — one constant

The **segment (block) budget is also the one-shot/segmented threshold** — a single tunable
constant (default ~256 KB of text, config-overridable), chosen so a single append request
(segment text + packed chunks + base64 inflation) stays comfortably under Vercel's 4.5 MB cap.

- A body that fits in one budget → **one-shot** single-block `/api/ingest` (today's path, zero
  new round-trips, no regression).
- A body exceeding one budget → **segmented** begin/append/finalize.

Segment boundaries are chosen by accumulating text up to the budget and **preferring to cut at a
heading boundary** within the budget window; in a headerless region that exceeds the budget,
fall back to #316's `hard_split` point (where `header_path` is empty anyway).

**Determinism is load-bearing.** Resume requires that re-reading the same source with the same
budget re-derives identical segment boundaries and hashes. Size-bounded + heading-preferred with
fixed constants gives that. The budget in effect for a resource is recorded in the
`resource_finalized`/begin event payload and the client `.temper/` manifest so a resume uses the
same value.

## 6. Client streaming chunker (`temper-ingest`)

A new incremental path alongside `prepare_markdown` (which stays for the one-shot path):

- Read the source with a `BufReader` — file, stdin, or URL-fetched tempfile — **never
  `read_to_string` the whole body**.
- Maintain the O(depth) heading breadcrumb stack while scanning.
- Emit a segment at the budget boundary (heading-preferred, `hard_split` fallback), then chunk
  and embed **that segment only** → a `chunks_packed` payload per block. Peak memory = one
  segment's text + its chunks + its vectors.
- **Additive `chunk_markdown` variant that accepts an initial breadcrumb** so a segment that
  begins mid-section (the fallback case) still carries its ancestor `header_path`. On native
  markdown with heading-aligned cuts this is a no-op; it exists to preserve full breadcrumbs
  across block boundaries and keep streaming output byte-identical to whole-document chunking.

Embedding composes with #299: the CLI computes vectors client-side per segment (`chunks_packed`);
segments could alternatively land null-embedding and be backfilled by the `kb_workflow_jobs`
queue. Finalize enqueues any deferred blocks. No new embed machinery.

## 7. Wire / API — three small endpoints, each bounded

Every request carries one segment, comfortably under the 4.5 MB cap. Segment payloads reuse the
existing `IngestPayload` chunk shape (`content`, `content_hash`, `chunks_packed`).

- **Begin** — `POST /api/ingest` gains a segmented mode (first segment + a `total` hint +
  `block_budget`): creates the resource, lands block 0 via the existing create path, records
  `kb_ingestion_records`, returns `{ resource_id, correlation_id, blocks: [{ seq, content_hash }] }`.
  Here `content_hash` is the sha256 of the segment's text (the same hash `IngestPayload`
  already carries for a one-shot body) — the per-segment identity used for idempotency and
  resume. (Distinct from the merkle per-block hash `_recompute_resource_body_hash` derives from
  chunk hashes, which stays server-internal.)
- **Append** — `POST /api/resources/{id}/blocks` with `{ seq, content, content_hash,
  chunks_packed }` → new `SeedAction::BlockAppend` (inserts one block at `seq=N` via the
  `_project_blocks` insert shape, single-element manifest) + fires `block_created`.
  **Idempotent on `(resource_id, seq, content_hash)`** — re-appending a landed segment is a
  no-op success. This idempotency is what makes retry and resume safe.
- **Finalize** — `POST /api/resources/{id}/finalize` with `{ expected_blocks, expected_body_hash }`
  → validates the landed set, fires `resource_finalized`, dispatches deferred embeds.
- **Resume query** — `GET /api/resources/{id}/blocks` → landed `[{ seq, content_hash }]`.

Auth-before-writes and context/cogmap visibility gating on begin/append/finalize mirror the
existing `/api/ingest` handler (`crates/temper-api/src/handlers/ingest.rs`).

## 8. `.temper/` client manifest + resume

- The client writes `.temper/ingest/<resource_id>.json`:
  `{ source_hash, block_budget, correlation_id, blocks: [{ seq, content_hash }], finalized }`.
- **Resume** = re-scan the source → re-derive segment hashes (deterministic, §5) →
  `GET /api/resources/{id}/blocks` → send only the missing `seq`s (idempotent append) →
  finalize. A `source_hash` mismatch (the file changed since the interrupted attempt) → clean
  restart, not a corrupt merge.
- The server-side source-integrity check compares the client's `source_hash` against
  `kb_ingestion_records.source_hash`.

## 9. The change surface (concentrated)

Persistence / SQL:
- New `SeedAction::BlockAppend` + its projection (reuses `_project_blocks` insert shape) firing
  the dormant `block_created` event.
- New `kb_event_types` seed row `resource_finalized` + an `_event_append` call at finalize.
- First writes of `kb_ingestion_records` (upsert at begin/finalize).
- A multi-block-aware body-hash path — the server-side `_recompute_resource_body_hash` already
  handles it per event; the Rust `body_hash_for_body`/`_for_chunks` single-block shortcuts
  (`content.rs:252`) stay for the one-shot path.

Client:
- Streaming chunker + `chunk_markdown` initial-breadcrumb variant (`temper-ingest`).
- CLI orchestration for begin/append/finalize + `.temper/` manifest + resume
  (`crates/temper-cli`, `crates/temper-client`).

API:
- Append / finalize / list-blocks handlers (`crates/temper-api`), segmented mode on `/api/ingest`.

Untouched:
- One-shot `/api/ingest` for small bodies. `update_resource_in_tx`'s >1-block guard (segmented
  ingest is a create-path concern, not the revise path). All multi-block SQL read/merkle/search
  functions (already correct).

## 10. Test coverage

- **`temper-ingest`** — streaming chunker produces byte-identical chunks and `header_path` to
  `chunk_markdown` on native markdown (golden test across a corpus); bounded, non-oscillating
  peak memory on a headerless multi-MB input; the initial-breadcrumb variant reconstructs
  ancestor paths across a mid-section cut.
- **Persistence** — `BlockAppend` idempotency (re-append same `(seq, block_hash)` is a no-op);
  multi-block `resource_body_text` round-trips (N-block reassembly equals the original body);
  recomputed merkle equals the whole-body hash; `block_created` and `resource_finalized` events
  land with correct `correlation_id`/payload; the in-progress-vs-finalized derived predicate.
- **e2e** — multi-MB CLI ingest → reassembled body correct + searchable; kill mid-append →
  resume sends only the gap, re-embeds nothing already durable; changed source between attempts →
  clean restart; small body still one-shot (single request, single block, no
  `block_created`/`resource_finalized`).

## 11. Non-goals / rejected

- **Chunk-as-transport-unit** (the issue's framing) — conflates the two boundaries and risks
  semantic chunk quality. Rejected in favor of the segment/chunk split (§2).
- **Single streaming request** — impossible on Vercel serverless (buffered body, 4.5 MB cap).
  Multi-request append is the only physically available shape for the API surface.
- **Structure-inference for headerless extraction** (issue "option b") — heuristics for
  recovering semantic boundaries from structure-poor PDFs/transcripts. Marginal, unmeasured
  gains; bakes in assumptions. Explicitly out of scope; such content chunks as it does today,
  memory-bounded by #316's `hard_split`. A separate brainstorm if ever pursued.
- **Vercel Blob staging** (reviving the retired TS upload workflow) — considered; rejected. It
  reintroduces the split that got it retired: kreuzberg + bge in TypeScript, duplicated
  chunk/write strategy across two runtimes, and separately-generated wire types diverging from
  the shared `temper-core` types. Content-blocks give the same durable-staging property inside
  tables and types we already own.
- **`ingest_complete_at` (or any status column on `kb_resources`)** — superseded by the
  event-derived completion predicate (§4).

## 12. Phasing

- **Beat 1 — persistence + events.** `SeedAction::BlockAppend`, fire `block_created`, add and
  fire `resource_finalized`, write `kb_ingestion_records`. Unit + persistence tests. (sqlx cache
  regen; new event type needs a seed migration.)
- **Beat 2 — API.** Segmented mode on `/api/ingest`, append/finalize/list-blocks handlers, auth
  gating. Per-crate sqlx cache (`cargo make prepare-api`).
- **Beat 3 — client.** Streaming chunker + initial-breadcrumb `chunk_markdown` variant; CLI
  begin/append/finalize orchestration; `.temper/` manifest + resume.
- **Beat 4 — e2e + threshold tuning.** Full multi-MB round-trip, interrupt/resume, one-shot
  no-regression; validate the block budget against real packed sizes under the 4.5 MB cap.

**Deferred (separate specs):**
- MCP begin/append/finalize tools (three tools). Until then MCP keeps its one-shot
  `create_resource` and the standing "bulk imports go through the CLI" guidance holds.
- A reaper sweep for abandoned in-progress resources — the derived predicate (§4) enables it,
  but the sweep itself is later.

## 13. Acceptance criteria (from the issue)

- [ ] A multi-MB document ingests via CLI + API without the full body resident in memory at any
      single point on either side. *(streaming chunker §6; multi-request transport §7)*
- [ ] An interrupted ingest resumes without re-transmitting or re-embedding already-durable
      chunks. *(idempotent append §7; `.temper/` resume §8)*
- [ ] Per-request cost stays bounded and small regardless of total document size. *(one segment
      per request, budgeted under 4.5 MB §5/§7)*
- [ ] The CLI local path shows stable, non-oscillating peak memory on hundreds-of-KB documents.
      *(one-segment working set §6; #316 already removed the unbounded-chunk hang)*
- [ ] Existing single-shot small-body creates work unchanged — no regression, no new round-trips
      for the short-note case. *(budget-as-threshold one-shot path §5)*
- [ ] (MCP arbitrary-size ingest) — **deferred** to the MCP beat (§12).
