# Embed-dispatch Throughput Scaling Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Scale the async-embed drain to enterprise bulk-ingest rates by draining the full invocation wall-clock (loop-drain), using the function's spare vCPU (intra-op threads), and fanning out across N concurrent SKIP-LOCKED shard drainers.

**Architecture:** Three composable, per-deploy-tunable levers over the existing concurrency-safe (`FOR UPDATE SKIP LOCKED`) job queue. `dispatch_tick` changes from claim-once-and-return to a claim loop bounded by a per-invocation deadline; the deadline default is retuned so each invocation finishes inside its one-minute cron cadence (concurrency = number of shards, "predictable N"); N shard cron lines fan the drain out. No queue-logic or SQL changes.

**Tech Stack:** Rust, sqlx (Postgres), axum, ONNX Runtime (ort), Vercel Functions + Cron, Neon Postgres (pooled endpoint), cargo-make + cargo-nextest.

**Spec:** `docs/superpowers/specs/2026-07-15-embed-dispatch-throughput-scaling-design.md`

## Global Constraints

- **cargo-make** for all tasks: `cargo make check` (fmt + clippy `-D warnings` + docs + machete + TS), `cargo make fmt`, `cargo make test-db` (integration, needs Docker Postgres on port 5437).
- **`--all-features`** for builds and clippy. Lint suppression uses `#[expect(lint, reason = "...")]`, never `#[allow]`.
- All public types implement `Debug`. All MPSC channels are bounded.
- **Tests always run against a real database** (Docker Postgres locally). The new integration tests live in the existing `#[cfg(all(test, feature = "test-db"))]` module in `embed_service.rs`; export `DATABASE_URL=postgresql://temper:temper@localhost:5437/temper_development` if running via bare `cargo`.
- **No new `sqlx::query!` macros** are introduced (tasks reuse existing runtime-query test helpers), so **no `.sqlx` cache regeneration is required**. If that changes, run `cargo sqlx prepare --workspace -- --all-features` then `cargo make prepare-services`.
- **Branch naming:** `jct/<scope>`. This plan ships as **two PRs** off the current branch `jct/embed-dispatch-throughput-scaling` (which already carries the spec): PR A (loop-drain + threads), then PR B (shard fan-out) stacked on A.
- **Commits** end with:
  `Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>`
- **Finishing a PR:** `git merge origin/main` first, push, open a PR — never merge locally. Pushing/PR is gated on explicit user approval at execution time.

---

## Task 0 (spike): Measure real ms/chunk on the throttled bench

**Not a code task — a measurement that sets Task 2's deadline default and Task 5's N.** The "~5 ms warm" figure in the post-mortem was a 1-token string; `embed_bench.rs` already measures per-chunk cost on realistic 510-token chunks (it chunks a ~1.2 MB synthetic body via `chunk_markdown`). Run it under the Vercel-like CPU cap at 1 and 2 intra-op threads.

**Files:**
- Use: `crates/temper-ingest/bench/` (Docker bed), `crates/temper-ingest/examples/embed_bench.rs`
- Record result into: `docs/superpowers/specs/2026-07-15-embed-dispatch-throughput-scaling-design.md` (the "measurement prerequisite" section)

- [ ] **Step 1: Build the bench image (once)**

Run:
```bash
REBUILD=1 crates/temper-ingest/bench/run.sh
```
Expected: image `temper-embed-bench` builds; a cold-load sweep prints. (This warms the cargo target volume so the next step is fast.)

- [ ] **Step 2: Measure per-chunk cost at threads 1 and 2 under the deploy's CPU slice**

Run (from repo root):
```bash
for t in 1 2; do
  echo "=== TEMPER_ONNX_INTRA_THREADS=$t ==="
  docker run --rm --cpus=1.47 --memory=3009m \
    -e TEMPER_ONNX_INTRA_THREADS="$t" \
    -e TEMPER_ONNX_MODEL_PATH=/repo/crates/temper-ingest/models/bge-base-en-v1.5/model_quantized.onnx \
    -v "$PWD:/repo:ro" -v temper-embed-bench-target:/target -v temper-embed-bench-registry:/usr/local/cargo/registry \
    temper-embed-bench \
    cargo run --release --locked -p temper-ingest --no-default-features --features embed,embed-download \
      --example embed_bench -- /tmp/quant-$t.json
done
```
Expected: each run prints a per-chunk cost (ms/chunk) and peak RSS. `--cpus=1.47` ≈ the `api/internal` function's 3009 MB slice.

> **Feature flags:** `embed_bench` has `required-features = ["embed"]`, but the arm64 bed needs the
> download runtime, so enable **both** (`--features embed,embed-download`). The lib gates the bundled
> `.so` branch as `all(embed, not(embed-download))`, so with both on the download path wins at runtime
> while `required-features` is still satisfied — no Cargo.toml change needed.

**Measured 2026-07-16** (arm64 bed, 1.47 vCPU, 945 chunks): threads=1 → 181.5 ms/chunk; threads=2 →
128.2 ms/chunk (1.42× speedup); peak RSS 0.97 GB; cold load ~0.5 s. See the spec's *Results* section
for the corrected throughput math (loop-drain ≈ 6.7×, N=4 ≈ 27×).

- [ ] **Step 3: Record and derive**

Record the two ms/chunk numbers in the spec's "measurement prerequisite" section, then compute:
- **Per-shard chunks in a 55 s invocation** ≈ `55_000 ms / ms_per_chunk` (at threads=2).
- **N to hit the target rate:** `target_chunks_per_min / (per-shard chunks per 55 s)`, rounded up, clamped to a judicious 2–4 to start.

Note the arm64/x86_64 fidelity caveat from the bench README (the bed is arm64; the deploy is x86_64 — the compute *mechanism* is identical, absolute numbers are indicative). If a precise x86_64 number is needed, add a `--platform linux/amd64` emulated run.

- [ ] **Step 4: Commit the recorded measurement**

```bash
git add docs/superpowers/specs/2026-07-15-embed-dispatch-throughput-scaling-design.md
git commit -m "docs(embed): record measured ms/chunk from throttled bench (task 019f5892)

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## PR A — loop-drain + intra-op threads

### Task 1: Loop-drain `dispatch_tick`

Turn the single-claim pass into a claim loop bounded by the deadline, so one invocation fills its wall-clock instead of returning after ~2–5 s.

**Files:**
- Modify: `crates/temper-services/src/services/embed_service.rs` (`dispatch_tick_inner`, ~lines 254–388)
- Test: same file, the `#[cfg(all(test, feature = "test-db"))]` module (add one test; existing tests must still pass unchanged)

**Interfaces:**
- Consumes: `workflow_job_service::{reap, claim_resource, complete_resource, enqueue_resource, redrive_resource}`; `temper_substrate::embed::{resolve_chunk_budget, embed_resource_chunks}`; `EmbedDispatchSummary`; `DEFAULT_EMBED_DISPATCH_CAP`, `DEFAULT_EMBED_LEASE_SECONDS`.
- Produces: unchanged public signatures `dispatch_tick(pool, cap: Option<i32>, redrive: bool)` and `dispatch_tick_inner(pool, cap, redrive, deadline: Duration)`. Only the internal drain loop changes.

- [ ] **Step 1: Write the failing test — loop-drain claims across multiple iterations**

Add to the test module in `embed_service.rs`:
```rust
    /// **The loop-drain gate.** Three enqueued resources with cap=1 → one resource per claim. A
    /// single-claim pass (the old behavior) would drain exactly ONE and return; loop-drain must drain
    /// all THREE inside one invocation by re-claiming until the queue is empty. Chunkless resources
    /// keep this ONNX-free (embed is a clean no-op).
    #[sqlx::test(migrations = "../../migrations")]
    async fn dispatch_tick_loop_drains_multiple_claims_in_one_pass(pool: PgPool) {
        for name in ["a", "b", "c"] {
            let r = a_named_resource(&pool, name).await;
            workflow_job_service::enqueue_resource(&pool, r, "embed", "embed")
                .await
                .unwrap()
                .expect("enqueue");
        }

        let summary = dispatch_tick(&pool, Some(1), false).await.unwrap();
        assert_eq!(
            summary.claimed, 3,
            "loop-drain re-claims until the queue is empty — not just one claim"
        );
        assert_eq!(summary.completed, 3);
        assert_eq!(summary.failed, 0);

        // Idempotent: a second pass finds nothing.
        let again = dispatch_tick(&pool, Some(1), false).await.unwrap();
        assert_eq!(again.claimed, 0);
    }
```

- [ ] **Step 2: Run it to verify it fails**

Run:
```bash
cargo nextest run -p temper-services --features test-db dispatch_tick_loop_drains_multiple_claims_in_one_pass
```
Expected: FAIL — `assert_eq!(summary.claimed, 3)` sees `1` (the current single-claim pass drains only one resource with cap=1).

- [ ] **Step 3: Replace the drain body with a claim loop**

In `dispatch_tick_inner`, replace everything from the `let cap = cap.unwrap_or(...)` line through the end of the `for job in claimed { ... }` loop (i.e. the single claim + budget + per-job loop) with the loop below. Keep the `redrive` prelude and the single `reap` call above it exactly as they are.

```rust
    let cap = cap.unwrap_or(DEFAULT_EMBED_DISPATCH_CAP);

    let mut summary = EmbedDispatchSummary {
        redriven,
        ..Default::default()
    };

    // Hard wall-clock ceiling on the whole invocation (see `resolve_dispatch_deadline`). With
    // loop-drain the deadline is the *invocation lifetime*, not a single-pass guard: we keep
    // claiming until the queue is empty or the deadline is hit, so one invocation fills its
    // wall-clock instead of returning after a single ~64-chunk claim.
    let start = std::time::Instant::now();

    // Do-while shape: always run at least one claim, then stop once past the deadline. Checking the
    // deadline AFTER a full claim (not before the first) guarantees every invocation makes progress
    // even under a pathologically small deadline — and preserves the ZERO-deadline defer semantics
    // the wall-clock test asserts (claim once, defer the batch, break).
    loop {
        let claimed = workflow_job_service::claim_resource(
            pool,
            persona,
            dispatch,
            cap,
            DEFAULT_EMBED_LEASE_SECONDS,
        )
        .await?;
        if claimed.is_empty() {
            break; // queue drained — nothing left to do this invocation
        }
        summary.claimed += claimed.len() as u32;

        // ONE chunk allowance per CLAIM (see `EMBED_CHUNK_BUDGET`), refreshed each iteration. The
        // deadline bounds the invocation; this bounds one claim's inference so no single
        // `embed_texts` call is ever large — which is what retires the single-large-resource cliff:
        // a 939-chunk resource embeds 64, re-enqueues, and is simply re-claimed on a later iteration.
        let mut budget = temper_substrate::embed::resolve_chunk_budget();

        for job in claimed {
            if start.elapsed() >= deadline {
                // Past the deadline: defer this job (and every one after it) — re-enqueue untouched
                // to resume next tick rather than hold a lease (a held lease looks like a crash to
                // the reaper and is not reclaimable for DEFAULT_EMBED_LEASE_SECONDS). Tallied
                // `partial`, not `failed` — 0 chunks embedded, job resumed later.
                workflow_job_service::complete_resource(pool, job.resource_id, persona, dispatch)
                    .await?;
                workflow_job_service::enqueue_resource(pool, job.resource_id, persona, dispatch)
                    .await?;
                tracing::info!(
                    resource_id = %job.resource_id,
                    elapsed_ms = start.elapsed().as_millis() as u64,
                    "embed dispatch hit its wall-clock deadline; re-enqueued job for the next tick"
                );
                summary.partial += 1;
                continue;
            }
            match temper_substrate::embed::embed_resource_chunks(pool, job.resource_id, budget).await
            {
                Ok(progress) => {
                    summary.chunks_embedded += progress.embedded;
                    budget -= progress.embedded as i64;
                    if progress.is_complete() {
                        workflow_job_service::complete_resource(
                            pool,
                            job.resource_id,
                            persona,
                            dispatch,
                        )
                        .await?;
                        summary.completed += 1;
                    } else {
                        // More stale chunks than this claim's budget: complete + re-enqueue so a
                        // later iteration (this invocation, or the next tick) resumes it with a
                        // fresh budget. Complete-then-enqueue (not hold the lease) keeps the reaper's
                        // attempt count clean for large resources.
                        workflow_job_service::complete_resource(
                            pool,
                            job.resource_id,
                            persona,
                            dispatch,
                        )
                        .await?;
                        workflow_job_service::enqueue_resource(
                            pool,
                            job.resource_id,
                            persona,
                            dispatch,
                        )
                        .await?;
                        tracing::info!(
                            resource_id = %job.resource_id,
                            embedded = progress.embedded,
                            remaining = progress.remaining,
                            "embed job partially drained; re-enqueued for the next tick"
                        );
                        summary.partial += 1;
                    }
                }
                Err(e) => {
                    // Leave the job in_progress; the reaper's lease-expiry sweep retries it (then
                    // dead at max attempts). One bad resource never aborts the pass.
                    tracing::warn!(
                        resource_id = %job.resource_id,
                        attempts = job.attempts,
                        error = %e,
                        "embed job failed; leaving for reaper retry"
                    );
                    summary.failed += 1;
                }
            }
        }

        // Stop looping once past the deadline — checked after a full claim, so ≥1 claim always runs.
        if start.elapsed() >= deadline {
            break;
        }
    }

    Ok(summary)
```

- [ ] **Step 4: Run the new test and the existing drain tests to verify they pass**

Run:
```bash
cargo nextest run -p temper-services --features test-db \
  dispatch_tick_loop_drains_multiple_claims_in_one_pass \
  dispatch_tick_claims_embeds_and_completes \
  dispatch_tick_empty_queue_is_noop \
  dispatch_tick_defers_jobs_past_the_wall_clock_deadline \
  dispatch_tick_with_redrive_revives_and_drains
```
Expected: all PASS. (The ZERO-deadline defer test still holds: one claim runs, the per-job check defers the batch, the post-claim check breaks — `claimed=1, partial=1, completed=0`.)

- [ ] **Step 5: fmt + check**

Run:
```bash
cargo make fmt && cargo make check
```
Expected: clean.

- [ ] **Step 6: Commit**

```bash
git add crates/temper-services/src/services/embed_service.rs
git commit -m "feat(embed): loop-drain the dispatch invocation until its wall-clock deadline

One cron fire used ~2-5s of its 300s budget by claiming once and returning.
dispatch_tick now re-claims until the queue is empty or the deadline is hit,
filling the invocation. Deadline-safe by construction (checked between claims,
each claim bounded by the chunk budget); retires the single-large-resource
cliff since throughput comes from more iterations, not bigger batches.

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

### Task 2: Retune the deadline default to the cron cadence

Make each invocation finish inside its one-minute cron window, so back-to-back fires tile the timeline and concurrency equals the number of shards (predictable N).

**Files:**
- Modify: `crates/temper-services/src/services/embed_service.rs` (`DEFAULT_EMBED_DISPATCH_DEADLINE_SECONDS` + its doc comment, ~lines 65–73)

**Interfaces:**
- Consumes/Produces: the constant `DEFAULT_EMBED_DISPATCH_DEADLINE_SECONDS: u64` and env `TEMPER_EMBED_DISPATCH_DEADLINE_SECONDS` are unchanged in name/type; only the default value and doc change.

- [ ] **Step 1: Change the constant and rewrite its doc**

Replace the existing `DEFAULT_EMBED_DISPATCH_DEADLINE_SECONDS` item (value + doc comment) with:
```rust
/// Default per-invocation wall-clock lifetime for one loop-draining dispatch pass, in seconds.
///
/// Under loop-drain this is the **invocation lifetime**, not a single-pass guard: `dispatch_tick`
/// keeps claiming until the queue is empty or this deadline is hit. Set to **55s** — just under the
/// one-minute cron cadence — so each fire finishes before the next fires. Back-to-back invocations
/// then tile the timeline with no stacking, making effective concurrency equal to the number of
/// shard cron lines (the "predictable N" model, spec Option A): concurrency = N, cost linear in N,
/// Neon connections bounded at ~N x 2.
///
/// `maxDuration` (300s in `vercel.json`) stays a pure safety ceiling far above this. To trade
/// predictable concurrency for higher per-shard throughput (spec Option B), raise
/// `TEMPER_EMBED_DISPATCH_DEADLINE_SECONDS` toward ~250 on a specific deploy: invocations then run
/// near `maxDuration` and a minute-cadence cron stacks ~4 per shard (effective ~4N). No code change.
pub const DEFAULT_EMBED_DISPATCH_DEADLINE_SECONDS: u64 = 55;
```

- [ ] **Step 2: Verify no test hard-codes the old default**

Run:
```bash
rg -n "200" crates/temper-services/src/services/embed_service.rs
```
Expected: no test asserts `200` as the deadline (the deadline tests inject `Duration::ZERO` via `dispatch_tick_inner`, and the default-path tests assert claim/complete counts, not timing). If any match is a deadline assertion, update it to 55; otherwise no change.

- [ ] **Step 3: check**

Run:
```bash
cargo make check
```
Expected: clean (doc-test/clippy pass).

- [ ] **Step 4: Commit**

```bash
git add crates/temper-services/src/services/embed_service.rs
git commit -m "feat(embed): retune dispatch deadline default to 55s for predictable-N concurrency

With loop-drain the deadline is the invocation lifetime. 55s finishes each fire
inside the one-minute cron cadence, so N shard crons give exactly N concurrent
drainers. maxDuration 300 stays the safety ceiling; raise the env toward 250 for
the Option-B stacking model per-deploy.

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

### Task 3: Document + set the intra-op thread knob on the deploy

Use the function's ~1.7 vCPU inside each invocation. Pure env — `resolve_intra_op_threads()` already reads `TEMPER_ONNX_INTRA_THREADS`; no code change.

**Files:**
- Modify: `DEPLOYING.md` (add an embed-throughput / thread-count note)

- [ ] **Step 1: Add the deploy note**

Add a subsection to `DEPLOYING.md` (near the function-timeouts material) — verify the exact surrounding heading first with `rg -n "maxDuration|Function [Tt]imeout|embed" DEPLOYING.md`, then insert:
```markdown
### Embed-drain throughput knobs (per deploy)

The async-embed drain scales by three env/cron knobs, tuned per deploy — none need a rebuild:

- `TEMPER_ONNX_INTRA_THREADS=2` — use the `api/internal` function's ~1.7 vCPU (3009 MB) for ONNX
  inference. Default is 1 (single core). Set to 2 on a bulk-ingest deploy for ~1.5–2× per invocation.
  Applies process-wide, so the public function's in-request query-embed benefits too.
- `TEMPER_EMBED_DISPATCH_DEADLINE_SECONDS` — per-invocation lifetime (default 55s = one cron cadence,
  giving concurrency == number of shard crons). Raise toward 250 for the Option-B stacking model.
- Shard cron count (`vercel.json`) — the number of concurrent drainers. See the throughput-scaling
  spec for sizing N against a target chunks/min.

**Safety floor: the embed function's `maxDuration` must stay ≥ 72 s.** A claim's ~64-chunk
`embed_texts` is uninterruptible (~8 s at the measured 128 ms/chunk, threads=2), so a claim starting
just under the 55 s deadline can run to ~63 s. 72 s leaves an ~8 s buffer. `api/internal` is 300 s
today (far above), so this is a guardrail against anyone lowering it for the embed cron — surface it
in the "operating temper" guidelines too.

Set the env var with:

    vercel env add TEMPER_ONNX_INTRA_THREADS production   # enter: 2
```

- [ ] **Step 2: Commit**

```bash
git add DEPLOYING.md
git commit -m "docs(deploy): document embed-drain throughput knobs (intra-threads, deadline, shards)

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

- [ ] **Step 3: (ops, at deploy time — not a repo change) set the env var**

On the bulk-ingest Vercel project: `vercel env add TEMPER_ONNX_INTRA_THREADS production` → `2`, then redeploy. This is an operator action; record it in the deploy runbook rather than committing.

### PR A finalization

- [ ] **Step 1: Full check + integration tests**

Run:
```bash
cargo make check && cargo make docker-up && cargo make test-db
```
Expected: clean; embed_service tests green.

- [ ] **Step 2: Merge main, push, open PR A** *(gated on user approval)*

```bash
git fetch origin && git merge origin/main
git push -u origin jct/embed-dispatch-throughput-scaling
gh pr create --title "feat(embed): loop-drain + intra-op threads (throughput scaling PR A)" --body "$(cat <<'EOF'
Implements PR A of the embed-dispatch throughput-scaling spec.

- Loop-drain: `dispatch_tick` re-claims until the queue is empty or the wall-clock
  deadline is hit, filling each invocation instead of returning after one ~64-chunk
  claim. Deadline-safe; retires the single-large-resource cliff.
- Deadline default retuned to 55s → predictable-N concurrency (each fire finishes
  inside the one-minute cron cadence).
- Documented `TEMPER_ONNX_INTRA_THREADS=2` deploy knob (pure env, ~1.5–2×).

Spec: docs/superpowers/specs/2026-07-15-embed-dispatch-throughput-scaling-design.md

🤖 Generated with [Claude Code](https://claude.com/claude-code)
EOF
)"
```

---

## PR B — shard fan-out

Stacks on PR A's branch. If PR A is still open, branch PR B off it (`jct/embed-shard-fanout`) so review stays separable; per the stacked-PR + squash-cascade caveat, land PR A first, then rebase PR B onto `main`.

### Task 4: Accept and log the cosmetic `shard` param

The `?shard=k` param exists only to distinguish N cron lines; drainers partition the queue via SKIP LOCKED. Accept it and echo it into the trace for per-shard observability.

**Files:**
- Modify: `crates/temper-api/src/handlers/embed.rs` (`DispatchQuery`, ~lines 24–33; `dispatch` tracing, ~lines 96–103)
- Test: same file's `#[cfg(test)] mod tests` (a deserialize unit test — no DB)

**Interfaces:**
- Produces: `DispatchQuery { cap: Option<i32>, redrive: bool, shard: Option<i32> }`. The handler's behavior is unchanged by `shard`; it is trace-only.

- [ ] **Step 1: Write the failing test — DispatchQuery parses `shard`**

Add to the `mod tests` in `handlers/embed.rs`:
```rust
    /// The shard param is cosmetic — it distinguishes the N fan-out cron lines and is echoed to the
    /// trace, never used for logic (drainers partition via SKIP LOCKED). It must deserialize from the
    /// query string, and its absence must remain valid (the single-cron and manual-trigger cases).
    #[test]
    fn dispatch_query_accepts_optional_shard() {
        let with: DispatchQuery = serde_urlencoded::from_str("shard=3&cap=5").unwrap();
        assert_eq!(with.shard, Some(3));
        assert_eq!(with.cap, Some(5));

        let without: DispatchQuery = serde_urlencoded::from_str("").unwrap();
        assert_eq!(without.shard, None);
        assert!(!without.redrive);
    }
```
(`serde_urlencoded` backs axum's `Query`; confirm it's a dev-dependency with `rg -n "serde_urlencoded" crates/temper-api/Cargo.toml` — axum re-exports it, but the test needs a direct dep. If absent, add `serde_urlencoded` under `[dev-dependencies]`.)

- [ ] **Step 2: Run it to verify it fails**

Run:
```bash
cargo nextest run -p temper-api dispatch_query_accepts_optional_shard
```
Expected: FAIL — `DispatchQuery` has no field `shard`.

- [ ] **Step 3: Add the field and log it**

In `DispatchQuery`, add after `redrive`:
```rust
    /// Cosmetic shard id — distinguishes the N cron lines that fan the drain out. Carries NO logic:
    /// concurrent drainers partition the queue via the claim's `FOR UPDATE SKIP LOCKED`, so this is
    /// only echoed into the trace for per-shard observability. See the throughput-scaling spec.
    pub shard: Option<i32>,
```
In `dispatch`, add `shard = q.shard,` to the `tracing::info!` field list:
```rust
    tracing::info!(
        shard = q.shard,
        redriven = summary.redriven,
        claimed = summary.claimed,
        completed = summary.completed,
        failed = summary.failed,
        chunks = summary.chunks_embedded,
        "embed dispatch pass complete"
    );
```

- [ ] **Step 4: Run test to verify it passes**

Run:
```bash
cargo nextest run -p temper-api dispatch_query_accepts_optional_shard
```
Expected: PASS.

- [ ] **Step 5: Write the concurrency invariant test (test-db)**

Add to the test module in `crates/temper-services/src/services/embed_service.rs`:
```rust
    /// **The fan-out safety invariant.** Two drainers running concurrently against the same queue
    /// must claim DISJOINT resources — `FOR UPDATE SKIP LOCKED` guarantees no resource is claimed by
    /// both. With cap=1 the two passes interleave claim-by-claim; together they complete each
    /// resource exactly once (sum of completed == the enqueued count), never twice, never missing one.
    #[sqlx::test(migrations = "../../migrations")]
    async fn concurrent_dispatch_ticks_claim_disjoint_resources(pool: PgPool) {
        for i in 0..10 {
            let r = a_named_resource(&pool, &format!("r{i}")).await;
            workflow_job_service::enqueue_resource(&pool, r, "embed", "embed")
                .await
                .unwrap()
                .expect("enqueue");
        }

        let (s1, s2) = tokio::join!(
            dispatch_tick(&pool, Some(1), false),
            dispatch_tick(&pool, Some(1), false),
        );
        let (s1, s2) = (s1.unwrap(), s2.unwrap());

        assert_eq!(
            s1.completed + s2.completed,
            10,
            "two concurrent drainers drain every resource exactly once — no double-claim, none missed"
        );
    }
```

- [ ] **Step 6: Run the concurrency test**

Run:
```bash
cargo nextest run -p temper-services --features test-db concurrent_dispatch_ticks_claim_disjoint_resources
```
Expected: PASS.

- [ ] **Step 7: fmt + check + commit**

```bash
cargo make fmt && cargo make check
git add crates/temper-api/src/handlers/embed.rs crates/temper-api/Cargo.toml \
        crates/temper-services/src/services/embed_service.rs
git commit -m "feat(embed): accept a cosmetic shard param + prove concurrent drainers claim disjoint

The ?shard=k query param distinguishes the N fan-out cron lines and is echoed to
the trace; it carries no logic (SKIP LOCKED partitions the queue). Adds the
concurrency invariant test: two drainers drain every resource exactly once.

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

### Task 5: Add N shard cron lines to `vercel.json`

Fan the per-minute drain out to N concurrent instances.

**Files:**
- Modify: `vercel.json` (`crons`)
- Verify: `vercel.json` (`routes` — the existing `/api/embed/dispatch → /api/internal` route must match the query-string form)

**Interfaces:**
- Produces: N cron entries `/api/embed/dispatch?shard=k` at `* * * * *`. N is a committed default (start at **4**); a deploy needing a different N edits its deploy-branch `vercel.json`.

- [ ] **Step 1: Confirm Vercel accepts a query string in a cron `path`**

Run:
```bash
mcp: search Vercel docs for "cron job path query string parameters" (or check https://vercel.com/docs/cron-jobs/manage-cron-jobs)
```
Expected: confirm `path` may carry `?shard=k`. **If it may NOT**, use the path-segment fallback instead: cron paths `/api/embed/dispatch/0..3`, add a route `{ "src": "/api/embed/dispatch/(\\d+)", "dest": "/api/internal" }` above the existing dispatch route, and read the segment via an axum `Path` extractor in `dispatch` (still cosmetic). Record which form was chosen.

- [ ] **Step 2: Replace the single dispatch cron with 4 shard crons**

In `vercel.json`, change the `crons` array so the single `/api/embed/dispatch` entry becomes four shard entries (keep `warm` unchanged):
```json
  "crons": [
    { "path": "/api/embed/dispatch?shard=0", "schedule": "* * * * *" },
    { "path": "/api/embed/dispatch?shard=1", "schedule": "* * * * *" },
    { "path": "/api/embed/dispatch?shard=2", "schedule": "* * * * *" },
    { "path": "/api/embed/dispatch?shard=3", "schedule": "* * * * *" },
    { "path": "/api/embed/warm", "schedule": "*/2 * * * *" }
  ],
```
(If the path-segment fallback was chosen in Step 1, use `/api/embed/dispatch/0..3` and add the route.)

- [ ] **Step 3: Validate the JSON**

Run:
```bash
jq '.crons | length' vercel.json
```
Expected: `5` (4 shards + warm). A parse error here means malformed JSON — fix before committing. (Cron changes only take effect on the next Vercel deploy; there is no local runtime test.)

- [ ] **Step 4: Document N-sizing + the shared-config caveat**

Append to the `DEPLOYING.md` embed-throughput subsection from Task 3:
```markdown
Shard cron count is the number of concurrent drainers. The committed default is **4** — cheap for a
low-traffic deploy (empty claims return fast) and effective for bulk ingest. `vercel.json` is shared
across all deploys of the repo, so a deploy needing a different N (higher for a heavy enterprise,
lower for a quiet site) edits N on its own deploy branch. Size N from the bench's measured ms/chunk:
`N ≈ target_chunks_per_min / (55s worth of chunks per shard)`. Vercel allows 100 crons/project, so
headroom is large — but prefer effective-not-overkill: raise N only when the measured rate trails.
```

- [ ] **Step 5: Commit**

```bash
git add vercel.json DEPLOYING.md
git commit -m "feat(embed): fan the drain out to 4 shard cron lines

Four /api/embed/dispatch?shard=k crons run concurrent SKIP-LOCKED drainers →
~4x the single-cron throughput, on top of loop-drain. Committed default is 4
(cheap for small deploys); a deploy tunes N on its own branch. Documents sizing.

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

### PR B finalization

- [ ] **Step 1: Full check + integration tests**

Run:
```bash
cargo make check && cargo make test-db
```
Expected: clean; new tests green.

- [ ] **Step 2: Rebase onto landed PR A, push, open PR B** *(gated on user approval)*

```bash
git fetch origin && git rebase origin/main
git push -u origin jct/embed-shard-fanout
gh pr create --title "feat(embed): shard fan-out (throughput scaling PR B)" --body "$(cat <<'EOF'
Implements PR B of the embed-dispatch throughput-scaling spec (stacks on PR A).

- Cosmetic ?shard=k param (trace-only; SKIP LOCKED does the partitioning) + a
  concurrency invariant test (two drainers drain every resource exactly once).
- 4 shard cron lines in vercel.json → ~4x throughput on top of loop-drain.
- N-sizing + shared-config guidance in DEPLOYING.md.

Spec: docs/superpowers/specs/2026-07-15-embed-dispatch-throughput-scaling-design.md

🤖 Generated with [Claude Code](https://claude.com/claude-code)
EOF
)"
```

---

## Self-Review

**Spec coverage:**
- Lever 1 loop-drain → Task 1. ✓
- Lever 2 intra-op threads → Task 3 (env + docs; no code, per plumbing). ✓
- Lever 3 shard fan-out → Tasks 4 (param + concurrency test) + 5 (cron lines). ✓
- Concurrency model A (55s deadline, predictable N) → Task 2. ✓
- Option B documented as a per-deploy env change → Task 2 doc + DEPLOYING note. ✓
- Measurement prerequisite (task 019f5892) → Task 0. ✓
- Bounds stay meaningful (cap/budget per-claim; deadline retuned) → Tasks 1 + 2 code + comments. ✓
- Constraints verified (Neon pooled, Vercel crons) → reflected in DEPLOYING sizing note (Task 5). ✓
- PR split A/B → task grouping + two finalizations. ✓
- Query-string cron-path risk → Task 5 Step 1 with fallback. ✓

**Placeholder scan:** No TBD/TODO; every code step shows full code; every command has expected output. Task 0 is an explicit spike (measurement), not a placeholder. ✓

**Type consistency:** `dispatch_tick(pool, Option<i32>, bool)` / `dispatch_tick_inner(.., Duration)` unchanged; `EmbedDispatchSummary` fields (`claimed`, `completed`, `partial`, `failed`, `chunks_embedded`, `redriven`) match `embed_service.rs`; `DispatchQuery` gains `shard: Option<i32>` used consistently in the handler and test; test helpers (`a_named_resource`, `workflow_job_service::enqueue_resource`) match existing signatures. ✓
