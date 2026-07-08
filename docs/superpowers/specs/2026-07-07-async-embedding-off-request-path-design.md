# Async embedding — move server-side embed off the MCP/HTTP request path

**Status:** design (approved for planning — Phase 1 in progress)
**Date:** 2026-07-07
**Issue:** [#299](https://github.com/tasker-systems/temper/issues/299) — MCP `create_resource` (and HTTP-raw ingest) runs the ONNX embed pipeline synchronously inside the serverless request; large bodies lag or brush the function timeout.
**Predecessors:**
- Steward fan-out drift-sweep (`019f3220`) — introduced `kb_workflow_jobs`, the lease-based queue we generalize here.
- Issue #297 (`b51a5bf`) — server-side query embedding; makes MCP/HTTP search's *query* arm live. This design is its write-side twin.

## 1. Problem

The two ingest surfaces split on **where embeddings are computed**:

- **CLI → `/api/ingest`** ships a fully-processed payload — content + chunks + **embeddings** (`chunks_packed`). The server persists vectors verbatim; no server-side ONNX. The expensive embed ran on the client's (fast) CPU.
- **MCP `create_resource`** and **HTTP-raw ingest** (content only, no `chunks_packed`) run `temper_ingest::embed::embed_texts` **synchronously inside the Vercel function** — reached from the `None` arm of `temper_substrate::writes::create_resource_with` (`writes.rs:129`) / `update_resource_in_tx` (`writes.rs:271-274`) via `content::prepare_block` (`content.rs:147`). Cost scales with chunk count; a cold instance also pays first-call model load (ONNX `OnceLock` init + writing the bundled `libonnxruntime.so` to `/tmp`).

An MCP client (an LLM host) **cannot** precompute embeddings — it has no model — so the compute must stay server-side. The fix is to move it **off the request path**, without reintroducing the bimodal return contract that sank the previous async attempt (a small body returned the resource; a large body returned a UUID to poll — two shapes from one tool).

## 2. The reframe (from the issue, confirmed against the code)

`create_resource` returns an **`EnrichedResource`** (`build_enriched`: row + `managed_meta` + `open_meta`). **It contains no embeddings.** The synchronous embed blocks the response for data the response does not carry. The resource is fully formed the instant its row + chunk *text* are written; only *vector searchability* depends on embedding.

So the return contract stays **uniform and synchronous** — always the formed `EnrichedResource`, never a UUID-to-poll — while only the embedding becomes eventually-consistent. Asynchrony is exposed (if at all) as a *field* on the same type, never as a different type.

## 3. Key insight — the persistence layer already supports this

The code map (three-agent trace, 2026-07-07) established that "chunk text now, vector later" needs **no schema change to the chunk tables**:

| Fact | Location | Consequence |
|------|----------|-------------|
| `kb_chunks.embedding vector(768)` is **nullable** | `20260624000001_canonical_schema.sql:559-580` | A chunk can exist with no vector. |
| The projector `_insert_chunk` **already writes `NULL`** when the sidecar `embedding` is JSON-null | `20260624000002_canonical_functions.sql:573` | The write path already tolerates null-embedding chunks; text + hash still land. |
| Chunk **text** is a separate INSERT into `kb_chunk_content` | `canonical_functions.sql` `_insert_chunk` | Text persists unconditionally, independent of the vector. |
| The **FTS index** `kb_resource_search_index` is rebuilt **synchronously in the chunk-write projector**, purely from title + chunk text | `20260626000001_fts_search_index.sql:19-39,80,118` | "FTS immediate" is free — it already happens in the write transaction. |
| `unified_search` blends arms with `FULL OUTER JOIN` + `LEFT JOIN … COALESCE(…,0)` | `20260629000004_search_scope_ids.sql:17-59` | A null-embedding resource scores `vector_score=0` and still ranks on FTS. **Graceful degradation already holds.** No search change. |
| A backfill primitive **already exists**: `embed_chunks()` selects `WHERE is_current AND embedding IS NULL` → `UPDATE kb_chunks SET embedding` | `temper-substrate/src/embed.rs:11-48` | The async worker productionizes an eval-only tool that already exists. |
| Server-side **query** embedding already landed (#297) | `substrate_read.rs:398-426` | The read side is done; this is the write-side twin. |

**The one deferral fork point** is `writes.rs:129` (create) and `writes.rs:271-274` (update): the `None` arm calls `prepare_block` (ONNX). Chunking itself (`plan_chunks` → `chunk_markdown`) is ONNX-free. The deferred path chunks, emits **null-embedding** chunks, and enqueues a backfill job.

### What is genuinely net-new
Only two things:
1. **Generalize `kb_workflow_jobs`** from cogmap-scoped to also resource-scoped (embed jobs key on `resource_id`, not `cogmap_id`).
2. **A drain trigger** — a Vercel Cron hitting an internal claim→embed→complete endpoint. (No prior Vercel cron exists; the steward queue is drained by an external Eve agent tick.)

Everything else (deferred write path, readiness field, re-drive) is small.

## 4. Non-goal / retired-design guardrail

The earlier (TypeScript) deferred-ingest workflow was retired because it made the tool contract **bimodal** (poll-for-the-resource). **Any proposal that reintroduces a poll-for-the-resource contract is rejected on the same grounds.** Here the create return shape is identical whether embedding ran or was deferred; a contract test enforces it (§9).

## 5. Threshold decision (settled)

- **Server-computed embeds always defer.** MCP `create_resource` and HTTP-raw ingest with `content` only never run ONNX on the request. One code path, simplest to reason about and test.
- **Caller-supplied vectors stay synchronous.** When a caller ships `chunks_packed` (the CLI, or any API-backed client with a model), the server persists those vectors verbatim, on-request, exactly as today — no queue, no deferral. This is the existing "bring your own vectors" short-circuit (`db_backend.rs:860-864`, `writes.rs:128-129 Some` arm), preserved untouched.

The only consistency traded is **read-after-write for *semantic* search of the just-created doc** — FTS + graph are immediate; vector recall lands within a drain interval. Interactive MCP use rarely creates-then-immediately-semantic-searches the *same* doc in one turn.

## 6. Drain mechanism (settled)

A new **Vercel Cron → internal `/api/embed/dispatch`** endpoint. Rationale:

- Embedding needs **no LLM** — unlike steward, the ONNX embedder is already linked into `temper-api` (via `temper-substrate → temper-ingest(embed)`). So the whole claim→embed→complete loop runs **inline in `temper-api`**; no external agent, no second app.
- `waitUntil` / background-tokio is **not viable on the Rust surface**: `waitUntil` is a Node/`@vercel/functions` primitive, and `temper-api` is a Rust Axum app wrapped as a Vercel function (`api/axum.rs`). A fire-and-forget `tokio::spawn` after responding is not guaranteed to complete — Vercel can freeze/reclaim the instance once the response is sent. The durable queue is the robust path.
- Additive-only and per-target-safe: a `crons` entry in the root `vercel.json` applies to every independent Vercel project consuming the repo.

Worst-case latency to first drain is ~one cron interval (target: every minute on Pro).

## 7. Components

| Unit | Layer | Responsibility |
|------|-------|----------------|
| deferred `prepare_block` variant | temper-substrate `content.rs` | chunk prose (ONNX-free `plan_chunks`), emit `PreparedChunk`s with **empty embedding** → sidecar `embedding: null`. |
| `writes` defer flag | temper-substrate `writes.rs` | in the `None`-chunks arm, choose deferred-vs-inline embed; on defer, persist null-embedding chunks + return the resource id for the enqueue. |
| `DispatchType::Embed` (+ persona) | temper-core `workflow_job.rs` | new bounded-set variant; code change, no column migration (columns are `text`). |
| `kb_workflow_jobs` resource-scoping | migration (additive) | nullable `resource_id`; relax `cogmap_id` NOT NULL; single-flight partial-unique index on `(resource_id, persona, dispatch_type)`; a CHECK that exactly one of `cogmap_id`/`resource_id` is set. |
| payload threading | temper-services `workflow_job_service` + `ClaimedJob` | expose the existing `payload jsonb` (enqueue arg + claim return) so a job carries `{ resource_id }`; add `resource_id`/`payload` to `ClaimedJob`. |
| resource-keyed enqueue/complete | temper-services | `enqueue_embed(resource_id)` and a `resource_id`-keyed `complete` (the steward path stays tuple-keyed on cogmap). |
| `embed_dispatch_tick` | temper-services `DbBackend` | reap → claim embed jobs → per job: load the resource's `is_current AND embedding IS NULL` chunks, `embed_texts`, `UPDATE kb_chunks SET embedding`, complete. Productionizes `embed_chunks()`. |
| `POST /api/embed/dispatch` | temper-api handler + route | CRON_SECRET-gated; builds the tick command, returns a small summary (claimed / embedded / failed counts). |
| `crons` entry | root `vercel.json` | schedule the dispatch (per-minute). |
| readiness field | temper-mcp / temper-api enrichment | `embedding_status: pending \| ready \| failed` on `EnrichedResource`, **derived** (see §8) — no new resource column in v1. |
| re-drive | temper-services (sweep) | re-enqueue `dead` embed jobs (a `reindex`-style operation / periodic sweep). |
| interim operator docs | DEPLOYING/guide | "bulk/large import goes through the CLI." Zero-code, ships first, good permanently. |

**What does NOT change:** the create/update **return shape**; the `chunks_packed` synchronous path; the FTS write; `unified_search`; the query-embedding path (#297); the steward queue's existing cogmap-keyed enqueue/claim/complete call sites.

## 8. Readiness exposure (derived, no new resource column in v1)

`embedding_status` on `EnrichedResource` is computed, not stored:

- **`ready`** — the resource has ≥1 current chunk and no current chunk has a NULL embedding (or it has no chunks at all — an empty body is trivially "ready").
- **`pending`** — ≥1 current chunk has a NULL embedding **and** a live embed job exists (`status IN (pending, in_progress, waiting_for_retry)`).
- **`failed`** — ≥1 current chunk has a NULL embedding and the embed job is `dead` (or none exists after a supersede race).

This is one cheap read alongside the existing meta fetch. Most callers ignore the field. If profiling later shows this read is hot, a denormalized `kb_resources.embedding_status` column is a v2 additive migration — deliberately deferred to keep v1 additive and small.

## 9. Consistency & failure handling

- **FTS immediate, vector eventual.** Chunk text + FTS land in the create transaction; only vector recall waits for the drain.
- **Failure is off-request.** A failed embed leaves a `pending`/`failed` resource that is FTS-only until re-driven. The reaper's retry→dead path (`workflow_job_reap`, already global — covers embed jobs) handles transient failures; `dead` jobs are observable (§8) and re-drivable (§7).
- **Update supersedes pending — naturally.** A body revise makes new chunks `is_current=true` (embedding NULL) and old chunks `is_current=false`. The backfill targets `WHERE is_current AND embedding IS NULL`, so an in-flight job — whenever it runs — embeds the *current* generation, which is correct. Single-flight on `(resource_id, …)` prevents pile-up; a re-enqueue on update dedups against the in-flight row. No stale-vector write is possible.
- **Downstream `avg(embedding)` readers degrade quietly.** `cogmap_region_content_cohesion`, `cogmap_region_telos_alignment`, and region re-materialization centroid average `kb_chunks.embedding`; a transiently-null chunk is skipped (their existing NULL handling). Worth a note in region docs; not a blocker.
- **Optional index tightening (measure first).** `idx_kb_chunks_embedding` is partial on `WHERE is_current` only. If a large fraction of current chunks are transiently embedding-less, consider `WHERE embedding IS NOT NULL AND is_current`. Defer until measured.

## 10. Test coverage

- Large-body `create_resource` (MCP) returns the formed `EnrichedResource` promptly (well under timeout); an embed job is enqueued. (integration)
- The created resource is **FTS-findable immediately**; **vector-findable after the drain** completes. (integration, `test-embed`)
- Embed-job failure: resource lands `pending`/`failed`, reaper retries then marks `dead`, re-drive re-enqueues and succeeds. (integration)
- create-then-update supersedes the first generation's pending embed; the final vectors match the **updated** body (single-flight on `resource_id`). (integration, `test-embed`)
- **Contract test:** the create return shape is byte-identical whether embedding ran synchronously (`chunks_packed` supplied) or was deferred (server-computed). No bimodality. (unit/integration)
- Caller-supplied `chunks_packed` still persists vectors synchronously (regression: no queue, no deferral). (integration)
- Deferred `prepare_block` variant emits null-embedding chunks with correct text/hash/heading metadata. (unit, ONNX-free)

## 11. Acceptance criteria (from the issue)

- [ ] Large-content `create_resource` over MCP returns promptly, no timeout, same response shape as a small one.
- [ ] Created resources are immediately FTS-searchable; vector search over them becomes available once embedding completes.
- [ ] No polling and no alternate return type on any create surface.
- [ ] Embedding failures are observable and re-drivable; the reaper handles transient failures.
- [ ] Interim: operator docs state bulk import should use the CLI path.

## 12. Phasing

| Phase | Deliverable | Gate |
|-------|-------------|------|
| **0** | Interim operator docs (bulk → CLI). Zero-code. | ships first |
| **1** | Deferred write path: null-embedding `prepare_block` variant + `writes` defer branch; **feature-flagged / behind an enqueue no-op** so it's inert until the queue + drain exist. Unit tests (ONNX-free). | this session |
| **2** | Queue generalization: `DispatchType::Embed`, resource-scoping migration, payload threading, resource-keyed enqueue/complete. | |
| **3** | Drain: `embed_dispatch_tick`, `POST /api/embed/dispatch` (CRON_SECRET), `vercel.json` cron. Wire Phase 1's enqueue live. | |
| **4** | Readiness field (derived) + re-drive path + integration/contract tests (`test-embed`). | |

Phases 1–4 are sequenced so the return contract never breaks: until Phase 3 wires the drain, the deferred path is not enabled on the request surfaces (they keep embedding inline), so there is never a window where chunks are written null with no drainer. The switch flips in Phase 3.

## 13. sqlx cache note

Phase 2's migration + new/changed `sqlx::query!` macros require the full prepare ritual (per CLAUDE.md): `cargo sqlx prepare --workspace -- --all-features` → `cargo make prepare-services` → `cargo make prepare-api`, and `cargo make prepare-e2e` for e2e test SQL. The Embed CI job (ONNX installed) is the one that exercises `test-embed`.
