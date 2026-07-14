# Where embeddings get computed: the measurement, and why ONNX stays in the CLI

**Task:** `019f5fef-bc3a-7851-8e7a-6fbc1a1cb265` (#420 set 3) · **Branch:** `jct/server-side-embed-drop-cli-onnx`
**Status:** plan · **Date:** 2026-07-14

#420 set 3's second section proposed inverting a documented invariant: let the server embed large
bodies via the per-minute drain, so the CLI returns as soon as the bytes land — and, if that held,
**remove ONNX from the CLI entirely** and reduce the client to a TUS-style resumable upload.

We drove it end-to-end in production before designing anything. **The proposal's premise did not
survive the measurement.** This document records what was measured, the decision that follows, and
the three workstreams that replace it.

> **Decision: ONNX stays in the CLI.** Dropping it was only ever on the table because server-side
> embedding was *estimated* to be profoundly faster. It is not — it is **~10× slower in wall-clock**.
> We already bind ONNX today and it works. Removing it would buy a smaller binary at the cost of
> making every ingest an order of magnitude slower to become searchable.

## What was measured (2026-07-14, prod)

### The drain is live and healthy — the cron gate is cleared

Task `019f6043`'s open Q2 ("deploying a cron ≠ the cron running") is answered:

- **320 embed jobs, all `done`** — zero pending, zero `dead`, zero `waiting_for_retry`.
- **`avg_attempts = 1.00`** — not one embed has ever needed a retry.
- **0 of 33,760 chunks index-wide un-embedded.** Nothing has ever been stranded.
- All three switches are already on in prod: jobs are enqueued (`TEMPER_ASYNC_EMBED=1`), claimed (the
  cron's `CRON_SECRET` bearer matches `EMBED_DISPATCH_SECRET`), and completed.

### The large-document path works — and the partial re-enqueue design holds at size

A 1,202,908-byte body (**939 chunks**) was POSTed to `/api/ingest` with **`chunks_packed` absent** —
the exact path a no-ONNX CLI would use, and one the drain had **never run at size** (its largest real
job was 58 chunks; p50 = 1, p95 = 8). It converged with nothing stranded, `attempts` pinned at 1
throughout. `dispatch_tick`'s complete-then-enqueue works exactly as its comment claims.

### …but the server is ~10× slower than the client, and that kills the premise

**The correction that changed the plan.** An early read of `leased_at → completed_at` on *small* jobs
suggested ~156 ms/chunk, implying the server was ~1.6× faster than the CLI. That was **wrong**, and
the error is instructive: per-chunk cost *falls* with document size —

| chunks in resource | jobs | ms/chunk |
|---|---|---|
| 1–4 | 270 | 1633 |
| 5–16 | 30 | 678 |
| 17–64 | 21 | **176** ← where the bogus estimate came from |

— which is **fixed per-invocation overhead being amortized**, not economy of scale. Small documents
have *short* chunks; BERT inference cost scales with sequence length. Measured on real **full-size**
chunks (two clean passes of the 939-chunk probe, from the job rows):

| pass | chunks | work (leased→completed) | ms/chunk |
|---|---|---|---|
| 1 | 512 | **288.5 s** | 563 |
| 2 | 427 | **241.5 s** | 566 |

> **~565 ms per full-size chunk, single-threaded.** The 939-chunk doc costs **~530 s (~8.8 min) of
> serial server compute.**

The same body on the release CLI: **55.3 s wall, 208 s CPU, 378% — it is multi-core.** (The task's
"~4 min / single-core / 94% of wall" figure came from a **debug-build CI e2e**, not the release
binary.)

| | client-side (today) | server-side (measured) |
|---|---|---|
| CLI returns in | 55.3 s | ~3 s |
| **Fully searchable after** | **55.3 s** | **~10 min** |
| Server CPU per 1.2 MB doc | 0 | **~530 s** |

Server-side wins the *client wait* and loses *time-to-searchable* by ~10×, while adding real compute
spend. **And it is not tunable away: we are compute-bound, not cron-bound.** The clean budget-512 run
took 9.9 min wall against ~8.8 min of pure compute — there is almost no idle left to reclaim. Raising
the chunk budget only rearranges the same work.

### Root cause of the 10×: the server pins ONNX to one core, on purpose

`temper-ingest/src/embed.rs` already carries the full intra-op threading lever
(`TEMPER_ONNX_INTRA_THREADS`, `set_intra_op_threads`, the CLI's `--embed-threads`), and says plainly:

> *"The right count still differs by surface… The server (temper-api) may run N concurrent ingests,
> where 'every embed grabs every core' risks oversubscription — its ideal count is an open question
> pending a measurement under concurrent load (task `019f5892`), so **the server does NOT opt in here
> and inherits the conservative pinned default below**."* → `INTRA_THREADS_DEFAULT: usize = 1`

So the server is single-threaded **by choice**, pending exactly the measurement above. The CLI, by
contrast, resolves its default from the detected **performance-core** count — and that file already
documents why the optimum is P-cores, not all cores (efficiency cores drag the intra-op barrier).

**Vercel sells cores by the gigabyte: 2048 MB of memory per vCPU.** So server multi-core is not a free
env toggle — it needs provisioned memory (`functions.memory`), and Vercel's Active-CPU pricing bills
provisioned memory *and* active CPU.

### Round-trip consistency is already broken — on both paths, identically

`readback::body` does **not** return stored bytes. **The body is never stored raw**; it is
*reconstructed from chunks* by `content::reconstruct_body`, which ends in `pieces.join("\n\n")`. Where
the chunker split mid-paragraph, a `\n` returns as `\n\n`.

Differential — the same 1.2 MB body through both paths, read back via the CLI's `resource show`:

| | bytes | sha256 | faithful |
|---|---|---|---|
| original | 1,202,908 | `6d566393fa638123` | — |
| **server**-embed readback | 1,203,710 | `9527442c67f26157` | ✗ |
| **client**-embed readback | 1,203,710 | `9527442c67f26157` | ✗ |

**Both paths read back byte-identical and equally lossy.** So this is *pre-existing*, not a regression
— but it is the load-bearing finding for the TUS work: **because the body is derived rather than
stored, `body_hash` can never be a true end-to-end integrity check.** It is a sibling of the known
heading-duplication bug (task `019f4694`): same root cause — `reconstruct_body` is not an exact
inverse of the chunker.

## Decision

**Keep client-side embedding. ONNX stays in the CLI.** The three prerequisites that a removal would
have needed (the drain's non-atomic complete-then-enqueue; the uncounted `cogmap create`/`reconcile`
embed consumers; server-side parallelism) are all real, but they were only worth paying for if the
server were faster. It isn't.

Effort redirects to three workstreams.

## Workstreams

### W1 — Make client-side embedding faster and more effective

The client is now confirmed as the *right* place for this, so make it good. The CLI already resolves
intra-op threads from detected performance cores and pipelines embed→upload at depth 1. Open ground:
per-chunk cost is dominated by sequence length, so chunking strategy is a throughput lever, not just a
retrieval one; and `embed_and_pack` still runs one segment at a time on a single `spawn_blocking`.

### W2 — TUS-style resumable upload + genuine round-trip consistency

The client's job is "get the bytes there, and verify what came back." That is the natural home for the
TUS-shaped resumable upload #420 set 3 proposes, and for **fixing the fidelity gap above** — storing
content blocks verbatim and treating chunks as a *derived index*, so `body_hash` means something.
This also carries set 3's original goal: an interrupted ingest must be **detectable**, never presented
as a complete, authoritative, searchable document.

### W3 — Server-side multi-core ONNX (for the fallback path)

With the CLI keeping ONNX, the server embed path serves MCP and embedder-less clients — lower stakes,
but 565 ms/chunk is still bad. The lever exists (`TEMPER_ONNX_INTRA_THREADS`, no rebuild); what it
needs is cores (memory) and the concurrent-load measurement task `019f5892` already names. Weigh
against Active-CPU spend.

## Immediate operational items

- **`TEMPER_EMBED_CHUNK_BUDGET=512` is unsafe and is live in prod right now.** Pass 1 of the probe ran
  **288.5 s** against a presumed **300 s** default ceiling — **96% of the limit**, surviving on luck.
  When a pass does blow the timeout the failure is *slow and quiet*: `DEFAULT_EMBED_LEASE_SECONDS` is
  600 s, so the job outlives the dead function and sits leased for ten minutes before the reaper can
  retry it. **Reduced to 256 (~145 s/pass); pending the next deploy to take effect.**
- **Pin `maxDuration` (and `memory`) explicitly in `vercel.json`.** Nothing in the repo sets either —
  the entire safety margin above rests on an *assumed* platform default we never chose. Whatever value
  we pick, it should be ours and it should be visible.

## Open questions

- The 300 s ceiling is assumed, not verified. Pinning `maxDuration` resolves this by construction.
- The lease (600 s) outliving the function timeout (~300 s) is a latent slow-failure mode independent
  of budget. Worth fixing on its own.
- Server-side embed spend, if W3 proceeds: cores cost memory, memory costs money.
