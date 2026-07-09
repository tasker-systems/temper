# MCP segmented ingest — begin/append/finalize on the agent surface

**Status:** Design approved 2026-07-09
**Predecessor:** [2026-07-08-streaming-resumable-ingestion-design.md](2026-07-08-streaming-resumable-ingestion-design.md) (PR #327, merged) — this spec delivers that design's §12 deferral and its final acceptance criterion.

## 1. Problem

PR #327 gave the CLI and API surfaces streaming, resumable, multi-block ingest. MCP was
explicitly deferred: it still has only the one-shot `create_resource`, so an MCP caller cannot
ingest a body that exceeds Vercel's ~4.5 MB buffered-request cap, and an interrupted MCP ingest
restarts from zero. The standing "bulk imports go through the CLI" guidance was the stopgap.

Closing this is not merely parity for its own sake. The repo's invariant is that MCP, API, and
CLI all reach the same logic layer; where a surface is missing an operation, the operation's
composition tends to silt up in whichever surface *does* have it. That has already happened here
(§4).

## 2. The obstacle — the shipped append contract assumes a client that chunks

Three facts about the merged surface, each verified against the tree:

1. **`AppendBlockPayload.chunks_packed` is a required `String`**
   (`crates/temper-core/src/types/ingest.rs:130`), unlike `IngestPayload`'s `Option<String>`.
2. **MCP has no embedder.** `crates/temper-mcp/src/tools/resources.rs:546` already sends
   `chunks_packed: None` on create and relies on the server-side chunk+embed path. An MCP caller
   — an LLM agent, or the Eve steward calling as code — cannot produce a `chunks_packed` blob.
3. **`AppendBlockPayload.content` and `.content_hash` are never read server-side.**
   `crates/temper-services/src/backend/db_backend.rs:2214-2216` consumes only `chunks_packed` and
   `seq` — it is the payload's sole consumer across the workspace. The prose rides inside the
   packed chunks; the declared text hash is verified against nothing. They are dead wire fields,
   and the test fixtures say so out loud: `segmented_backend_test.rs:107` passes
   `"unused-client-text-hash"` under a comment reading "Unused server-side … any value is
   accepted."

The tension is exact: the two fields an MCP caller *can* supply are the two the server ignores,
and the field it cannot supply is the one the server requires.

A fourth fact makes finalize the crux. `resource_finalize`
(`migrations/20260708000012_streaming_ingest.sql:104`) hard-compares `expected_body_hash` against
`kb_resources.body_hash` — a merkle over chunk hashes. A caller that never chunked cannot compute
it.

## 3. Decision — integrity moves to where the caller has the information

Rejected alternatives are in §8. The chosen shape:

**Per-segment, on the way in.** `append` verifies `sha256_hex_raw(content) == content_hash` and
rejects a mismatch with `BadRequest`. This is a genuine end-to-end transit check that *every*
caller can honor, including one that does not chunk. It runs for the existing CLI caller too,
which sends exactly these values today (`crates/temper-cli/src/actions/ingest.rs:314-315`:
`content` is the segment text verbatim, `content_hash` is bare-hex sha256 over its raw bytes) and
currently gains no check from them. Two dead fields become the integrity contract.

**Whole-body, at finalize, by echo-back.** `BlocksResponse` gains `body_hash: String`, carrying
the server's live `kb_resources.body_hash` after the landed set. Append and `list_blocks` both
return it. The caller finalizes by echoing back the value from its last response.
`kb_resources.body_hash` is recomputed on every block event, so the value is always current.

`expected_body_hash` therefore stays **required**, and `resource_finalize`'s SQL is **unchanged**.
There is **no new migration**. Finalize's hash assertion now means "nothing changed between my
last append and now" — a real consistency check against a dropped or concurrent write — rather
than an assertion the non-chunking caller must fake or be exempted from. One contract, all
callers.

## 4. Begin's composition must be hoisted before MCP can call it

`crates/temper-api/src/handlers/ingest.rs:150-182` performs three backend calls in sequence —
`create_resource`, then `record_ingestion_source` (a `DbBackend` inherent method, not on the
`Backend` trait), then `list_blocks` — and assembles the `SegmentedBeginResponse` inside the HTTP
handler. An MCP `ingest_begin` would have to duplicate all of it.

This also contradicts the standing rule that a surface dispatches **one** operations command per
inbound call.

Add `Backend::begin_segmented_ingest(cmd: CreateResource, seg: SegmentedBegin) ->
SegmentedBeginResponse`, owning those three steps. The HTTP handler collapses to a single
dispatch; the MCP tool makes the same one call; `record_ingestion_source` stops being a
surface-visible inherent method. This is the "lift API-bound functionality into the shared layer"
move the MCP surface exists to force.

## 5. The emitter-surface bug this beat exposes

`Backend::append_block` and `finalize_ingest` hardcode
`writes::resolve_emitter(…, surface_marker(Surface::ApiHttp))` (`db_backend.rs:2208,2247`), where
every other `Backend` write method threads `cmd.origin`. Today nothing notices, because the API
is the only caller. The moment MCP appends a block, it is attributed to the `web` emitter.

`append_block`, `finalize_ingest`, and `list_blocks` gain an `origin: Surface` parameter. This is
a pre-existing bug that only this beat's caller surfaces, so it is **bundled into this PR** per
the repo's convention on fixes whose story is "this PR's new code path surfaced it."

## 6. Change surface

### Wire types (`temper-core/src/types/ingest.rs`)
- `AppendBlockPayload.chunks_packed: Option<String>` — `Some` = caller chunked and embedded (CLI);
  `None` = server chunks (MCP, steward). Mirrors `IngestPayload`.
- `BlocksResponse.body_hash: String` — additive; the CLI ignores it.
- `SegmentedBeginResponse.body_hash: String` — the same value after block 0, so a session that
  appends nothing still has something to echo at finalize.
- `AppendBlockPayload.content` / `.content_hash` become load-bearing (§3), documented as such.

### Substrate (`temper-substrate`)
- `content::prepare_block_with_prefix` and `prepare_block_deferred_with_prefix`, taking an initial
  heading breadcrumb and delegating to the already-shipped
  `temper_ingest::chunk::chunk_markdown_with_prefix` (`crates/temper-ingest/src/chunk.rs:386`).
  With an empty breadcrumb these are byte-identical to `prepare_block` / `prepare_block_deferred`
  — the existing single-block path is untouched.
- A readback helper for the trailing `header_path` of a resource's last landed block's last chunk,
  which seeds the breadcrumb so a server-chunked segment beginning mid-section still carries its
  ancestor path.

### Services (`temper-services`)
- `append_block`: verify the content hash; when `chunks_packed` is `None`, derive the breadcrumb
  from the prior block and chunk server-side.
- Embedding reuses the create path's existing predicate verbatim (`db_backend.rs:1118`): defer
  when `chunks.is_none() && !body.is_empty() && async_embed_enabled()`. No new embed machinery; a
  server-chunked append behaves exactly as a server-chunked create does today.
- `begin_segmented_ingest` (§4); `origin: Surface` threading (§5); `landed_segments` returns
  `body_hash`.

### MCP (`temper-mcp`)
Four tools, named for the `invocation_*` family precedent (`crates/temper-mcp/src/service.rs:455`)
rather than an overload of `create_resource`. Overloading would make the return type polymorphic —
acceptable over HTTP with `#[serde(untagged)]`, actively bad for an LLM caller reading a tool
schema. Distinct names are the affordance.

| Tool | Wraps | Notes |
|---|---|---|
| `ingest_begin` | `begin_segmented_ingest` | `create_resource`'s inputs plus `block_budget`, `total_blocks_hint`, `source_hash`. Returns `resource_id`, `correlation_id`, landed blocks, `body_hash`. |
| `ingest_append` | `append_block` | `seq`, `content`, `content_hash`; `chunks_packed` omitted by agent callers. Returns the landed set + `body_hash`. |
| `ingest_finalize` | `finalize_ingest` | `expected_blocks` + `expected_body_hash`, echoed from the last response. |
| `ingest_blocks` | `list_blocks` | The resume/progress read. |

Tool descriptions steer actively: reach for `create_resource` unless the body genuinely exceeds
one segment; treat `ingest_blocks` as the recovery path after an interrupted run. No caller needs
to know what a merkle is — `body_hash` is opaque and echoed.

### API (`temper-api`)
Handler passes `Surface::ApiHttp` explicitly; begin collapses to one dispatch. No route changes.

### CLI (`temper-cli`, `temper-client`)
Unchanged behavior; the client's `chunks_packed` field becomes `Option` and is always set to
`Some`.

### Untouched
`resource_finalize` SQL. `block_append` SQL. Migrations. The one-shot `create_resource` path on
every surface.

## 7. Resume without a client manifest

The CLI resumes from `.temper/ingest/<resource_id>.json`. A stateless MCP caller resumes from the
server: `ingest_blocks` returns the landed `seq` set, the caller re-derives its segments, sends
the missing ones (append is idempotent on `(resource_id, seq, block merkle)`), and finalizes.

`source_hash` is checked against `kb_ingestion_records` for a programmatic caller that has a file.
An LLM agent composing in-context passes `None` and gets count-and-contiguity validation — exactly
as piped stdin does on the CLI today.

Segment sizing differs by caller and needs no server change: the CLI cuts at ~256 KB, while an LLM
agent's segments are bounded by its own output budget (tens of KB). `block_budget` is recorded
per-session, not enforced as a constant.

## 8. Rejected

- **Optional `expected_body_hash`, count-only validation at finalize.** Smallest diff, but it drops
  the strongest invariant the streaming design has, on precisely the surface where the caller is
  least trusted, and splits the finalize contract by caller type. Contracts that branch by caller
  rot.
- **Persist each append's declared segment hash in the `block_created` payload; verify the set at
  finalize.** The strongest end-to-end story, and the ledger is its right home. But it needs a new
  migration touching an event payload contract, and it adds little over §3: appends are already
  verified on arrival, so a finalize-time re-check only catches server-side corruption between
  append and finalize, which the block merkle already covers. Reach for this if we later want
  ledger-replayable proof of what a client claimed it sent.
- **Overloading `create_resource` with a `segmented` parameter** (the API's own shape). A
  polymorphic tool return is a liability on an LLM-facing surface.

## 9. Test coverage

- **`temper-core`** — `AppendBlockPayload` round-trips with `chunks_packed` absent;
  `BlocksResponse.body_hash` round-trips.
- **`temper-substrate`** — `prepare_block_with_prefix` with an empty breadcrumb is byte-identical
  to `prepare_block` (the no-regression guard); the breadcrumb carries across a mid-section block
  boundary; a server-chunked re-append of the same segment is an idempotent no-op.
- **`temper-services`** — append with `chunks_packed: None` lands chunks with continuous
  `header_path`; a `content_hash` mismatch is `BadRequest`; the emitter marker is `mcp` when
  `origin: Surface::Mcp` (the §5 regression guard).
- **`temper-api`** — new cases for the 400 on hash mismatch and for `body_hash` presence.

  The existing segments tests **must be updated, not preserved**: their fixtures pass
  `content_hash: "unused-client-text-hash"` (`segments_handler_test.rs:94`,
  `segmented_backend_test.rs:109`, `segments_client_test.rs:112`, `streaming_ingest_test.rs:502`),
  so enabling verification necessarily breaks them. Replacing those placeholders with real sha256
  values is the change that *proves* the check bites. CLI compatibility is instead demonstrated by
  the CLI's own path, which already computes the correct hash
  (`crates/temper-cli/src/actions/ingest.rs:315`) — the e2e round-trip is the honest witness, not
  an unmodified unit fixture.
- **e2e** — the load-bearing assertion: **a segmented, server-chunked ingest of document D
  produces a body and chunk set identical to a one-shot create of D.** That single equivalence
  covers breadcrumb continuity, segment reassembly, and merkle agreement at once. Plus interrupt →
  `ingest_blocks` → append-the-gap → finalize.

Server-side chunking calls ONNX, so the e2e cases are `test-embed`-gated and land in the Embed CI
job. Run locally with `cargo make test-e2e-embed`.

## 10. Acceptance criteria

- [ ] An MCP caller ingests a body exceeding one segment budget, via `ingest_begin` → N ×
      `ingest_append` → `ingest_finalize`, supplying no `chunks_packed`. This closes the
      predecessor spec's final open criterion.
- [ ] The resulting body and chunk set are identical to a one-shot `create_resource` of the same
      document — same `header_path` values across block boundaries, same `body_hash`.
- [ ] An interrupted MCP ingest resumes from `ingest_blocks` alone, with no client-side manifest,
      re-transmitting only the missing segments.
- [ ] A block appended over MCP is attributed to the `mcp` emitter, not `web`.
- [ ] An append whose `content` does not hash to its declared `content_hash` is rejected.
- [ ] The CLI and API segmented paths are behaviorally unchanged — witnessed by the CLI e2e
      round-trip, not by unmodified unit fixtures (§9).
