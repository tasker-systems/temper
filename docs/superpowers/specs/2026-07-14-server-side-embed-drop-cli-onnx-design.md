# Moving embedding server-side, and dropping ONNX from the CLI

**Task:** `019f5fef-bc3a-7851-8e7a-6fbc1a1cb265` (#420 set 3) · **Branch:** `jct/server-side-embed-drop-cli-onnx`
**Status:** plan · **Date:** 2026-07-14

The task's second section proposed inverting a documented invariant: today the CLI is the primary
embed path and the server stores client-supplied chunks verbatim, embedding only when chunks are
absent. The proposal is to let the server embed large bodies via the existing per-minute drain, so
the CLI returns as soon as the bytes land — and, if that holds, to remove ONNX from the CLI entirely
and reduce the client to a TUS-style chunked upload with verified round-trip.

This document records what was **measured in production** before any of that was designed, and the
plan that follows from it.

## What was measured (2026-07-14, prod)

The task made the plan conditional on one thing: *"any plan that leans on the server cron must first
confirm `/api/embed/dispatch` actually fires and does work in production."* It does.

### The drain is live and healthy

- **320 embed jobs, all `done`.** Zero pending, zero `dead`, zero `waiting_for_retry`.
- **`avg_attempts = 1.00`** across all 320 — not one embed has ever needed a retry.
- **0 of 33,760 chunks index-wide have a NULL embedding.** Nothing has ever been stranded.
- All three switches are already on in prod: jobs are *enqueued* (so `TEMPER_ASYNC_EMBED=1`),
  *claimed* (so the Vercel cron's `CRON_SECRET` bearer matches `EMBED_DISPATCH_SECRET`), and
  *completed* (so the drain does work). This answers Q2 of task `019f6043-4668-7423-b6d0-14595592f5e9`.

### Server embed throughput — measured, not extrapolated

From `leased_at → completed_at` on real prod jobs: 58 chunks in 9s, 57 in 7s, 44 in 8s.

> **~6.4 chunks/sec ≈ 156 ms/chunk.**

This is the number `EMBED_CHUNK_BUDGET`'s own comment blocks on — *"the right value depends on the
function's real wall-clock limit and the box's real ms/chunk, neither of which is measured yet (task
`019f5892`). Raise it once they are."* It is now measured.

### The large-document path, driven end-to-end in prod

A 1,202,908-byte body (**939 chunks** — the same count as the existing `BENCH` rows, so this is the
real large-doc case) was POSTed to `/api/ingest` with **`chunks_packed` deliberately absent**, which
is exactly the code path a no-ONNX CLI would use. The drain had never once run at this size: its
largest real job to date was 58 chunks (p50 = 1, p95 = 8). Every 939-chunk doc in the index had been
client-embedded.

| | |
|---|---|
| Client wait (upload returns) | **16.9 s** |
| Drain convergence | **15 passes × 64 chunks**, exactly linear |
| `attempts` throughout | **1** — never retried, never `dead` |
| Chunks stranded | **0** — 939/939 embedded |
| Time to fully searchable | **~15.5 min** (default 64-chunk budget) |

Two findings from this:

1. **The partial re-enqueue design works at size.** `dispatch_tick`'s complete-then-enqueue keeps the
   attempt count clean instead of burning toward `dead`, exactly as its comment claims.
2. **Throughput is linear.** Issue #420 item 4 observed "superlinear-looking cost" on large bodies.
   That **does not reproduce server-side** — every pass embedded precisely 64 chunks.

### The real trade is not the one the task assumed

The task framed this as "the user waits on single-core embedding." Measured on this laptop, the
release CLI embeds the same body in **55.3 s wall (208 s CPU, 378% — it is multi-core)**. The
"~4 minutes / single-core / 94% of wall" figure in the task came from the **e2e test on CI** (debug
build), not the release binary.

So the honest trade:

| | client-side (today) | server-side (measured) |
|---|---|---|
| CLI returns in | 55.3 s | **16.9 s** |
| Fully searchable after | 55.3 s (immediate) | **~15.5 min** (at 64-chunk budget) |

Server-side wins the *client wait* by ~3.3×, but **loses search freshness by ~17×** at the current
budget. That is not an argument against the move — it is an argument that **the budget is the thing
to fix, and it is the cheapest fix available.** Each pass does ~10 s of work inside a function that
gets 300 s, then idles ~50 s waiting for the next cron tick: **~84% of the wall is idle.** At
156 ms/chunk, a budget near **1,200 chunks ≈ 187 s** fits one pass comfortably inside the timeout and
collapses a 939-chunk doc to a **single pass**. `TEMPER_EMBED_CHUNK_BUDGET` is an env var — no
rebuild, no migration.

### Round-trip consistency is already broken — on both paths

`readback::body` does not return stored bytes. **The body is never stored raw**; it is *reconstructed
from chunks* via `content::reconstruct_body`, which joins pieces with `pieces.join("\n\n")`. Wherever
the chunker split mid-paragraph, a `\n` comes back as `\n\n`.

The differential (same 1.2 MB body through both paths, read back through the CLI's `resource show`):

| | bytes | sha256 | faithful |
|---|---|---|---|
| original | 1,202,908 | `6d566393fa638123` | — |
| **server**-embed readback | 1,203,710 | `9527442c67f26157` | ✗ |
| **client**-embed readback | 1,203,710 | `9527442c67f26157` | ✗ |

**Both paths produce byte-identical readback.** So:

- Moving embedding server-side **does not change the stored document** — fidelity is not a regression
  and does not block this plan.
- But round-trip infidelity **already exists today**, on both paths, and it is squarely inside the
  "round trip consistency" half of what we want to build. It is a sibling of the known heading-
  duplication bug (task `019f4694`): same root cause — `reconstruct_body` is not an exact inverse of
  the chunker.

For a knowledge base this is the load-bearing observation: **because the body is derived rather than
stored, `body_hash` can never be a true end-to-end integrity check.** That is the strongest argument
for the TUS-shaped design storing content blocks verbatim and treating chunks as a *derived index*.

## What blocks dropping ONNX (and what doesn't)

Three CLI consumers of `temper-ingest/embed`, not one:

| consumer | status |
|---|---|
| `resource create` / `update` (ingest) | ✅ **Covered** — the drain handles it; proven above. |
| `search` | ✅ **Free** — the server already embeds text-only queries (#297), and *surfaces* the degraded case ("vector ranking was unavailable… FTS + graph only") rather than silently returning keyword hits. |
| `cogmap create` (genesis) + `cogmap reconcile` | ❌ **Not covered.** These PUT/POST *pre-embedded* payloads; the code states the server "stays embed-free on the PUT path." Nobody had counted these. |

And one durability hole, named in `dispatch_tick`'s own comment: the partial path's
**complete-then-enqueue is not atomic**. A crash between the two statements leaves the resource with
**no job**, and there is no automatic stale sweep (`enqueue_stale` is operator-triggered by design).
The chunks simply sit stale until an operator runs `reembed`.

That is tolerable *today* precisely because the CLI embeds client-side and the drain is a fallback.
**Drop ONNX and the drain becomes the only embed path — and that hole becomes a document that is
stored, `status: ok`, listed, and silently unsearchable.** Which is the exact bug class this task
exists to kill:

- set 3 = a silently **partial** document,
- the PDF session's F1 = a silently **empty** one,
- this would be a silently **unembedded** one.

So "drop ONNX" is not a deletion. It has a prerequisite.

## Plan

### P1 — Raise the chunk budget, and measure (no code, no migration)

Set `TEMPER_EMBED_CHUNK_BUDGET` in prod (start ~512, step toward ~1,200) and re-run the 939-chunk
probe, watching passes and the function's wall time. Deliverable: the budget that keeps a single pass
comfortably inside the function timeout, plus the resulting **time-to-searchable SLO** the task asks
for. This alone takes a large doc from ~15.5 min to ~2–3 min and is reversible by an env edit.

Also worth resolving here: the lease (`DEFAULT_EMBED_LEASE_SECONDS = 600`) is longer than the
function timeout (300 s), so a timed-out pass sits leased for 10 minutes before the reaper can retry.

### P2 — Close the durability hole (**prerequisite for P4**)

Make the partial path's complete-then-enqueue durable — a transactional hand-off, or a stale sweep
that heals without an operator. Until this lands, the drain is not safe as the *only* embed path.
Pair it with a detectable-partial signal so an un-embedded resource is never presented as a complete,
authoritative, searchable document (this is the same completeness marker set 3 already wants, and
`embedding_status` (`pending`/`ready`) is already on the wire to carry it).

### P3 — Server-side embed for the cogmap genesis / reconcile paths

Extend deferred embedding to the two paths that today assume a pre-embedded payload, or route them
through the drain. Without this, ONNX cannot leave the CLI.

### P4 — Drop ONNX from the CLI

Remove the `embed` feature and its consumers, `embed-download`, and the model-sha-drift machinery
(`temper-ingest/build.rs`'s hash gate exists to keep *two* embedders in lockstep; with one embedder
the whole class of problem disappears). Shrinks the binary and deletes a maintenance burden.

Note this **re-adds server compute spend** — the cost the client-side choice was made to avoid. That
trade is now quantified: ~156 ms/chunk of server CPU per ingested chunk.

### P5 — TUS-style chunked upload + genuine round-trip consistency

With embedding gone from the client, the CLI's job collapses to "get the bytes there and verify what
came back." That is the natural home for the TUS-shaped resumable upload the task proposes — and for
fixing `reconstruct_body` so that what you read back is what you wrote, byte for byte, which is the
prerequisite for `body_hash` meaning anything at all.

## Open questions

- **P1's budget ceiling.** 156 ms/chunk is measured, but the function's *real* wall-clock limit under
  Fluid Compute is assumed (300 s default), not verified. Verify before committing a budget.
- **Server spend.** Re-adding embed cost to the server is the thing client-side embedding was chosen
  to avoid. The per-chunk figure is now known; the monthly figure is not.
- **Small bodies.** Even with ONNX gone, a small doc embedded by the drain is not searchable until the
  next tick (≤60 s), versus instantly today. Probably fine — but it is a real regression in freshness
  for the common case, and should be stated rather than discovered.
