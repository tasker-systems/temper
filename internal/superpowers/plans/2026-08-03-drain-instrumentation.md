# Drain Instrumentation Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Give an operator backlog depth, oldest-pending age and per-job queue wait for the two
`api/internal` drains, so *"is the drain keeping up?"* is answerable from spans alone.

**Architecture:** A shared `drain_span` module owns the field vocabulary and the two queue reads.
Each drain gets a `*_dispatch` tick span (child of the existing HTTP root, which stays transport-only)
and a `*_job` span per claimed job. Aggregation is TraceQL metrics over these spans — there is no
metrics pipeline in this repo and this plan does not add one.

**Tech Stack:** Rust, `tracing` + `tracing-opentelemetry`, `sqlx` (compile-time checked), nextest,
`opentelemetry_sdk::trace::InMemorySpanExporter` for span assertions.

**Spec:** `internal/superpowers/specs/2026-08-03-drain-instrumentation-design.md`. Read it before Task 1
— this plan is an index over it, not a replacement for it.

## Global Constraints

- **Zero migrations.** `workflow_job_claim_anchor`'s `RETURNS TABLE` must not be widened —
  that needs `DROP FUNCTION`, which is non-additive and cannot ride the additive-only-on-`main`
  deploy. Queue wait comes from a plain `SELECT` on the claimed ids.
- **The `*_job` and `*_dispatch` spans stay `internal` kind.** Never set `otel.kind = "server"` on
  them. They will correctly not appear in `traces_spanmetrics_*`; that is expected, not a bug.
- **Backlog is read BEFORE the claim loop**, using **the same predicate as the claim**. A backlog
  count over jobs the drain could not have claimed reports a queue that is not the drain's to drain.
- **No act fields, and no job fields, on the HTTP root span.** CLAUDE.md's span-field convention;
  `tests/e2e/tests/logging_test.rs` already asserts the act half.
- `#[expect(lint, reason = "...")]`, never `#[allow]`.
- All public types derive `Debug`.
- More than 5 domain-related parameters ⇒ params struct.
- **After any new/changed `sqlx::query!`, regenerate the cache.** Read the `sqlx-query-cache` skill;
  the workspace ritual does **not** cover test-target queries.
- `DATABASE_URL=postgresql://temper:temper@localhost:5437/temper_development` and
  `cargo make docker-up` before any `test-db` run.

---

## File Structure

| File | Responsibility |
|---|---|
| `crates/temper-services/src/services/drain_span.rs` **(create)** | The drains' self-report: field-name constants, the two queue reads, and the job-outcome vocabulary. Owns nothing about *how* a drain works. |
| `crates/temper-services/src/services/mod.rs` **(modify)** | `pub mod drain_span;` |
| `crates/temper-services/src/services/region_service.rs` **(modify)** | Tick span; per-job work extracted into an instrumented fn. |
| `crates/temper-services/src/services/embed_service.rs` **(modify)** | Same shape, embed's fields. |
| `crates/temper-services/tests/drain_span_test.rs` **(create)** | The span witness. Owns a process-global subscriber, so it must be its own file. |

**Why `drain_span` is its own module rather than a helper in each service:** the twins' whole design
premise (`region_service.rs` module doc) is that they must not drift. Two copies of the field list
drift; that is the failure `ACT_SPAN_FIELDS` exists to prevent, and this follows its precedent.

---

## Task 1: The shared `drain_span` module

**Files:**
- Create: `crates/temper-services/src/services/drain_span.rs`
- Modify: `crates/temper-services/src/services/mod.rs`
- Test: in-file `#[cfg(test)]` for the pure parts; DB parts covered in Task 4.

**Interfaces:**
- Produces:
  - `pub const DRAIN_DISPATCH_FIELDS: [&str; 6]`
  - `pub const DRAIN_JOB_FIELDS: [&str; 3]`
  - `pub struct QueueDepth { pub backlog_depth: i64, pub oldest_pending_age_ms: Option<i64> }`
  - `pub async fn read_queue_depth(pool: &PgPool, persona: &str, dispatch_type: &str) -> ApiResult<QueueDepth>`
  - `pub async fn read_queue_waits(pool: &PgPool, ids: &[Uuid]) -> ApiResult<HashMap<Uuid, i64>>`
  - `pub enum JobOutcome { Completed, Deferred, Partial, Failed }` with `pub fn as_str(&self) -> &'static str`

- [x] **Step 1: Write the failing test for the outcome vocabulary**

Create `crates/temper-services/src/services/drain_span.rs` with only this test module at the bottom:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn job_outcome_strings_are_the_closed_vocabulary_the_queries_group_by() {
        // docs/guides/drain-operator-queries.md D1 groups by these exact strings. A rename here
        // silently empties an operator's panel, so the vocabulary is asserted, not just declared.
        assert_eq!(JobOutcome::Completed.as_str(), "completed");
        assert_eq!(JobOutcome::Deferred.as_str(), "deferred");
        assert_eq!(JobOutcome::Partial.as_str(), "partial");
        assert_eq!(JobOutcome::Failed.as_str(), "failed");
    }

    #[test]
    fn declared_field_names_have_no_duplicates_and_no_empties() {
        for set in [DRAIN_DISPATCH_FIELDS.as_slice(), DRAIN_JOB_FIELDS.as_slice()] {
            for (i, f) in set.iter().enumerate() {
                assert!(!f.is_empty(), "empty field name at index {i}");
                assert!(
                    !set[..i].contains(f),
                    "`{f}` is declared twice — a duplicate makes the Task 4 tie-assertion pass \
                     vacuously for one of them"
                );
            }
        }
    }
}
```

- [x] **Step 2: Run it to verify it fails**

Run: `cargo nextest run -p temper-services drain_span`
Expected: FAIL to compile — `JobOutcome`, `DRAIN_DISPATCH_FIELDS`, `DRAIN_JOB_FIELDS` not found.

- [x] **Step 3: Write the module head — constants and the outcome enum**

Put this **above** the test module in `drain_span.rs`:

```rust
//! What the `api/internal` drains report about themselves.
//!
//! Both drains (region materialization, embedding) emit the same two-level span shape: a
//! `*_dispatch` tick span carrying queue state and tallies, and a `*_job` span per claimed job. The
//! field vocabulary lives here, in one declaration, for the reason `region_service`'s module doc
//! gives about the twins generally — two copies drift, and a drifted field name silently empties an
//! operator's panel rather than failing anything.
//!
//! These spans are deliberately `internal` kind. Tempo's span-metrics processor derives RED only
//! from `server`/`client` spans, so none of this appears in `traces_spanmetrics_*` — that is
//! correct. They are not request boundaries, and the aggregation route is TraceQL metrics, which
//! reads any span. See `docs/guides/drain-operator-queries.md`.

use std::collections::HashMap;

use sqlx::PgPool;
use uuid::Uuid;

use crate::error::ApiResult;

/// Fields every `*_dispatch` tick span declares. Tied to the attribute by
/// `dispatch_span_declares_every_shared_field` — a constant nothing asserts its consumers against
/// prevents no drift at all (the lesson `ACT_SPAN_FIELDS` records).
pub const DRAIN_DISPATCH_FIELDS: [&str; 6] = [
    "backlog_depth",
    "oldest_pending_age_ms",
    "claimed",
    "completed",
    "deferred",
    "failed",
];

/// Fields every `*_job` span declares, whichever drain emitted it. Per-drain identity
/// (`anchor_id`/`resource_id`) and per-drain outcome detail are deliberately NOT here: they differ
/// by drain, and forcing them into the shared set would make one drain declare a field it can never
/// fill.
pub const DRAIN_JOB_FIELDS: [&str; 3] = ["queue_wait_ms", "attempts", "outcome"];

/// How one claimed job ended. A closed vocabulary, not a boolean, because the middle two are neither
/// success nor failure and collapsing them into either misreports a healthy drain under load.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JobOutcome {
    /// Work done, job marked done.
    Completed,
    /// The invocation's wall-clock deadline was already spent when this job came up. Handed back
    /// untouched, **no work attempted**.
    Deferred,
    /// Work was done but did not finish this claim's budget; re-enqueued to resume. Embed only —
    /// a region tick has no partial state (see `region_service`'s module doc).
    Partial,
    /// The unit of work errored. Left in-flight for the reaper.
    Failed,
}

impl JobOutcome {
    /// The wire string. `docs/guides/drain-operator-queries.md` D1 groups by these.
    pub fn as_str(&self) -> &'static str {
        match self {
            JobOutcome::Completed => "completed",
            JobOutcome::Deferred => "deferred",
            JobOutcome::Partial => "partial",
            JobOutcome::Failed => "failed",
        }
    }
}
```

- [x] **Step 4: Run the tests to verify they pass**

Run: `cargo nextest run -p temper-services drain_span`
Expected: PASS, 2 tests.

- [x] **Step 5: Add the two queue reads**

Append to `drain_span.rs`, above the test module:

```rust
/// The queue as a tick found it.
#[derive(Debug, Clone, Copy, Default)]
pub struct QueueDepth {
    /// Jobs this tick *could have claimed*, counted before it claimed any.
    pub backlog_depth: i64,
    /// Age of the oldest such job, in ms. `None` when the queue is empty — which is not zero, and
    /// must not be recorded as zero: "nothing waiting" and "something arrived this instant" are
    /// different states and an operator reads the difference.
    pub oldest_pending_age_ms: Option<i64>,
}

/// Read the claimable backlog for one (persona, dispatch_type).
///
/// **The predicate is copied from `workflow_job_claim_anchor`'s inner `SELECT`**
/// (`migrations/20260802000020_workflow_jobs_anchor_scope.sql:93-99`) — status in
/// `('pending','waiting_for_retry')` and `next_visible_at <= now()`. Counting anything the claim
/// would not have taken reports a backlog that is not this drain's to drain: a retry scheduled for
/// two minutes out is not work the current tick is behind on.
///
/// Call this BEFORE the claim loop. Called after, a tick reports the queue it has just emptied, and
/// a drain falling behind looks healthy at exactly the moment it is not.
pub async fn read_queue_depth(
    pool: &PgPool,
    persona: &str,
    dispatch_type: &str,
) -> ApiResult<QueueDepth> {
    let row = sqlx::query!(
        r#"
        SELECT count(*)                                                        AS "depth!",
               (extract(epoch FROM (now() - min(enqueued_at))) * 1000)::bigint AS "oldest_age_ms"
          FROM kb_workflow_jobs
         WHERE persona = $1
           AND dispatch_type = $2
           AND status IN ('pending', 'waiting_for_retry')
           AND next_visible_at <= now()
        "#,
        persona,
        dispatch_type,
    )
    .fetch_one(pool)
    .await?;

    Ok(QueueDepth {
        backlog_depth: row.depth,
        oldest_pending_age_ms: row.oldest_age_ms,
    })
}

/// Enqueue-to-lease, in ms, for the jobs just claimed.
///
/// A separate `SELECT` rather than a widened `workflow_job_claim_anchor`, because changing a
/// function's `RETURNS TABLE` requires `DROP FUNCTION` — non-additive, and it would force an
/// operator cutover for a telemetry field.
///
/// Jobs whose `leased_at` is somehow NULL are omitted from the map rather than defaulted to 0; a
/// zero wait is a claim, and inventing one would put a false floor in the operator's p50.
pub async fn read_queue_waits(pool: &PgPool, ids: &[Uuid]) -> ApiResult<HashMap<Uuid, i64>> {
    if ids.is_empty() {
        return Ok(HashMap::new());
    }
    let rows = sqlx::query!(
        r#"
        SELECT id AS "id!",
               (extract(epoch FROM (leased_at - enqueued_at)) * 1000)::bigint AS "wait_ms"
          FROM kb_workflow_jobs
         WHERE id = ANY($1)
        "#,
        ids,
    )
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .filter_map(|r| r.wait_ms.map(|w| (r.id, w)))
        .collect())
}
```

- [x] **Step 6: Register the module**

In `crates/temper-services/src/services/mod.rs`, add alongside the existing `pub mod` lines:

```rust
pub mod drain_span;
```

- [x] **Step 7: Regenerate the sqlx cache and check**

Read the `sqlx-query-cache` skill first. Then:

```bash
cargo make docker-up
cargo sqlx prepare --workspace -- --all-features
cargo make check
```

Expected: clean. If `error[E0282]: type annotations needed` appears on a `query!` you did not touch,
your dev DB is behind `migrations/` — run `cargo make db-migrate`, do not re-run prepare.

- [x] **Step 8: Verify the `enqueued_at` bias question the spec left open**

The spec (§"Two things that must be written down") requires this be **resolved, not inherited**.
`enqueued_at` defaults to `now()` = transaction-start, so if the enqueue runs inside a long write
transaction the wait is overstated by that transaction's duration.

Run: `rg -n "queue_region_clocks" crates/temper-services/src/backend/db_backend.rs`

Read the call site and determine whether it runs on `&self.pool` after the create's `tx.commit()`
(statement-accurate) or inside a transaction (biased). Then **write the answer into
`read_queue_waits`'s doc comment** as either "unbiased because …" or "overstates by … because …".
Do not leave the question open.

- [x] **Step 9: Commit**

```bash
git add crates/temper-services/src/services/drain_span.rs \
        crates/temper-services/src/services/mod.rs .sqlx
git commit -m "drain instrumentation: the shared span vocabulary and the two queue reads"
```

---

## Task 2: Instrument the region drain

**Files:**
- Modify: `crates/temper-services/src/services/region_service.rs`

**Interfaces:**
- Consumes: `drain_span::{read_queue_depth, read_queue_waits, JobOutcome, DRAIN_DISPATCH_FIELDS, DRAIN_JOB_FIELDS}`
- Produces: `region_dispatch` and `region_job` spans; no public Rust API change. `RegionDispatchSummary` is **unchanged** — this task adds observation, not behaviour.

- [x] **Step 1: Add the tick span to `dispatch_tick_inner`**

Replace the existing bare `async fn dispatch_tick_inner(` signature line with the attribute plus the
same signature. The existing doc comment above it stays.

```rust
#[tracing::instrument(
    name = "region_dispatch",
    skip_all,
    fields(
        backlog_depth = tracing::field::Empty,
        oldest_pending_age_ms = tracing::field::Empty,
        claimed = tracing::field::Empty,
        completed = tracing::field::Empty,
        deferred = tracing::field::Empty,
        failed = tracing::field::Empty,
        materialized = tracing::field::Empty,
        salience_refreshed = tracing::field::Empty,
    )
)]
async fn dispatch_tick_inner(
    pool: &PgPool,
    cap: Option<i32>,
    deadline: std::time::Duration,
) -> ApiResult<RegionDispatchSummary> {
```

`skip_all` is mandatory, matching `#[act_span]`: a drain's arguments are a pool and a duration, and
defaulting to `Debug`-formatting arguments onto spans is the habit that eventually puts a body on one.

- [x] **Step 2: Read the backlog before the claim loop**

Immediately after the existing `let start = std::time::Instant::now();` and **before** `loop {`:

```rust
    // Before the first claim, deliberately: this is "how deep was the queue when this tick arrived".
    // Read after the loop it would describe the queue this tick just drained.
    let depth = drain_span::read_queue_depth(pool, persona, dispatch).await?;
    let span = tracing::Span::current();
    span.record("backlog_depth", depth.backlog_depth);
    if let Some(age) = depth.oldest_pending_age_ms {
        span.record("oldest_pending_age_ms", age);
    }
```

Add `use crate::services::drain_span;` to the imports at the top.

- [x] **Step 3: Extract the per-job body into an instrumented fn**

Add this above `dispatch_tick_inner`. It is the existing loop body, moved verbatim except that the
two tally sites become a returned value.

```rust
/// One claimed job, as its own span.
///
/// Extracted from the claim loop rather than inlined so the per-job facts land on a span of their
/// own. With no child span, `Span::current().record(..)` resolves to the tick span and *works* —
/// right up until it silently doesn't, which is the trap CLAUDE.md's span-field convention exists
/// for and `tests/e2e/tests/logging_test.rs` asserts against for act fields.
///
/// `past_deadline` is passed rather than recomputed here so the caller keeps ownership of the
/// invocation clock — this fn must stay a pure function of its inputs for the Task 4 witness.
#[tracing::instrument(
    name = "region_job",
    skip_all,
    fields(
        anchor_id = %job.anchor.uuid(),
        anchor_kind = job.anchor.table(),
        attempts = job.attempts,
        queue_wait_ms = tracing::field::Empty,
        outcome = tracing::field::Empty,
        materialized = tracing::field::Empty,
        salience_refreshed = tracing::field::Empty,
    )
)]
async fn run_region_job(
    pool: &PgPool,
    job: &ClaimedAnchorJob,
    queue_wait_ms: Option<i64>,
    past_deadline: bool,
) -> ApiResult<RegionJobResult> {
    let span = tracing::Span::current();
    if let Some(wait) = queue_wait_ms {
        span.record("queue_wait_ms", wait);
    }

    let persona = Persona::Region.as_str();
    let dispatch = DispatchType::Materialize.as_str();

    if past_deadline {
        // Past the deadline: hand this job back untouched rather than start a settling we cannot
        // finish. Complete-then-re-enqueue instead of holding the lease, so the reaper's attempt
        // count stays clean — the same reasoning the embed drain's deferral path gives.
        workflow_job_service::complete_anchor(pool, job.anchor, persona, dispatch).await?;
        workflow_job_service::enqueue_anchor(
            pool,
            job.anchor,
            persona,
            dispatch,
            temper_core::types::workflow_job::RegionJobPayload {
                emitter: job.emitter,
            },
        )
        .await?;
        span.record("outcome", JobOutcome::Deferred.as_str());
        tracing::info!("region dispatch hit its wall-clock deadline; re-enqueued job for the next tick");
        return Ok(RegionJobResult::Deferred);
    }

    match region_clocks::tick(pool, job.anchor, job.emitter.into(), None).await {
        Ok(tick) => {
            workflow_job_service::complete_anchor(pool, job.anchor, persona, dispatch).await?;
            span.record("outcome", JobOutcome::Completed.as_str());
            span.record("materialized", tick.materialized);
            span.record("salience_refreshed", tick.salience_refreshed);
            Ok(RegionJobResult::Completed {
                materialized: tick.materialized,
                salience_refreshed: tick.salience_refreshed,
            })
        }
        Err(e) => {
            // Leave the job in_progress; the reaper's lease-expiry sweep retries it (then dead at
            // max attempts). One bad anchor never aborts the pass.
            span.record("outcome", JobOutcome::Failed.as_str());
            tracing::warn!(error = %e, "region clock tick failed; left in-flight for the reaper to retry");
            Ok(RegionJobResult::Failed)
        }
    }
}

/// What one claimed job did, so the caller can tally without re-reading the span.
#[derive(Debug, Clone, Copy)]
enum RegionJobResult {
    Deferred,
    Completed {
        materialized: bool,
        salience_refreshed: bool,
    },
    Failed,
}
```

Add to the imports: `use temper_core::types::workflow_job::ClaimedAnchorJob;` and
`use crate::services::drain_span::JobOutcome;`.

- [x] **Step 4: Rewrite the claim loop to call it**

Replace the whole `for job in claimed { ... }` block with:

```rust
        let ids: Vec<uuid::Uuid> = claimed.iter().map(|j| j.id).collect();
        let waits = drain_span::read_queue_waits(pool, &ids).await?;

        for job in claimed {
            let past_deadline = start.elapsed() >= deadline;
            match run_region_job(pool, &job, waits.get(&job.id).copied(), past_deadline).await? {
                RegionJobResult::Deferred => summary.deferred += 1,
                RegionJobResult::Completed {
                    materialized,
                    salience_refreshed,
                } => {
                    summary.completed += 1;
                    if materialized {
                        summary.materialized += 1;
                    }
                    if salience_refreshed {
                        summary.salience_refreshed += 1;
                    }
                }
                RegionJobResult::Failed => summary.failed += 1,
            }
        }
```

Behaviour is preserved exactly: once `start.elapsed() >= deadline` is true it stays true, so every
subsequent job in the batch defers, as the original `continue` did.

- [x] **Step 5: Record the tallies onto the tick span before returning**

Replace the final `Ok(summary)` with:

```rust
    span.record("claimed", summary.claimed);
    span.record("completed", summary.completed);
    span.record("deferred", summary.deferred);
    span.record("failed", summary.failed);
    span.record("materialized", summary.materialized);
    span.record("salience_refreshed", summary.salience_refreshed);
    Ok(summary)
```

- [x] **Step 6: Run the existing region drain tests — behaviour must be unchanged**

Run: `cargo nextest run -p temper-services --features test-db --test region_tick_off_request_test`
Expected: PASS, unchanged. This task adds observation only; a red here means the loop rewrite
changed behaviour.

- [x] **Step 7: Commit**

```bash
git add crates/temper-services/src/services/region_service.rs
git commit -m "drain instrumentation: the region drain reports its queue and its jobs"
```

---

## Task 3: Instrument the embed drain

**Files:**
- Modify: `crates/temper-services/src/services/embed_service.rs`

**Interfaces:**
- Consumes: the same `drain_span` items as Task 2.
- Produces: `embed_dispatch` and `embed_job` spans. `EmbedDispatchSummary` unchanged.

- [x] **Step 1: Add the tick span**

Same shape as Task 2 Step 1, on `embed_service::dispatch_tick_inner`, with embed's tail fields:

```rust
#[tracing::instrument(
    name = "embed_dispatch",
    skip_all,
    fields(
        backlog_depth = tracing::field::Empty,
        oldest_pending_age_ms = tracing::field::Empty,
        claimed = tracing::field::Empty,
        completed = tracing::field::Empty,
        deferred = tracing::field::Empty,
        failed = tracing::field::Empty,
        redriven = tracing::field::Empty,
        partial = tracing::field::Empty,
        chunks_embedded = tracing::field::Empty,
    )
)]
```

Note `deferred` is declared even though `EmbedDispatchSummary` has no such field — see Step 3.

- [x] **Step 2: Read the backlog before the claim loop**

After `let start = std::time::Instant::now();`, before `loop {` — identical to Task 2 Step 2 but
with embed's `persona`/`dispatch` locals already in scope.

```rust
    let depth = drain_span::read_queue_depth(pool, persona, dispatch).await?;
    let span = tracing::Span::current();
    span.record("backlog_depth", depth.backlog_depth);
    if let Some(age) = depth.oldest_pending_age_ms {
        span.record("oldest_pending_age_ms", age);
    }
```

- [x] **Step 3: Split the two states embed currently conflates**

`EmbedDispatchSummary.partial` is incremented on **two different things**: the deadline-deferral path
(0 chunks embedded, job untouched) and the budget-exhausted path (chunks embedded, job resumed).
`region_service`'s deadline path calls the same state `deferred`.

**Do not change the summary** — that is a wire type with consumers, and this plan adds observation
only. Instead the span's `outcome` distinguishes them:

- deadline path → `JobOutcome::Deferred`, and the tick span's `deferred` field counts them
- budget-exhausted path → `JobOutcome::Partial`

Track a local `let mut deferred: u32 = 0;` beside `summary`, incremented on the deadline path only,
and record it onto the tick span in Step 5. The summary's `partial` keeps counting both, exactly as
today.

- [x] **Step 4: Extract the per-job body into `run_embed_job`**

Follow Task 2 Step 3's shape exactly. The span fields differ:

```rust
#[tracing::instrument(
    name = "embed_job",
    skip_all,
    fields(
        resource_id = %job.resource_id,
        attempts = job.attempts,
        queue_wait_ms = tracing::field::Empty,
        outcome = tracing::field::Empty,
        chunks_embedded = tracing::field::Empty,
    )
)]
```

The body is the existing loop body moved verbatim, with each tally site replaced by a returned
value, and `span.record("outcome", …)` on each exit path. Because embed's per-job path mutates the
per-claim `budget`, the fn returns the chunks embedded so the caller can decrement it:

```rust
#[derive(Debug, Clone, Copy)]
enum EmbedJobResult {
    Deferred,
    Completed { chunks_embedded: i64 },
    Partial { chunks_embedded: i64 },
    Failed,
}
```

Keep `budget` owned by the caller. Passing `&mut budget` into an instrumented fn would make the
witness in Task 4 depend on mutation order.

- [x] **Step 5: Record tallies and the local `deferred` before returning**

```rust
    span.record("claimed", summary.claimed);
    span.record("completed", summary.completed);
    span.record("deferred", deferred);
    span.record("failed", summary.failed);
    span.record("redriven", summary.redriven);
    span.record("partial", summary.partial);
    span.record("chunks_embedded", summary.chunks_embedded);
    Ok(summary)
```

- [x] **Step 6: Run the embed drain tests**

Run: `cargo nextest run -p temper-services --features test-db embed`
Then: `cargo nextest run -p tests-e2e --features test-db --test async_embed_drain_e2e` if that target
builds locally; otherwise note it for CI.
Expected: PASS, unchanged.

- [x] **Step 7: Commit**

```bash
git add crates/temper-services/src/services/embed_service.rs
git commit -m "drain instrumentation: the embed drain reports the same shape, and stops conflating deferred with partial on the span"
```

---

## Task 4: The convention gates

**Files:**
- Create: `crates/temper-services/tests/drain_span_test.rs`
- Modify: `crates/temper-services/Cargo.toml` — `[dev-dependencies]` gains `opentelemetry` (for the
  `SpanKind` type the root-span negative asserts on) and `opentelemetry_sdk` (for
  `InMemorySpanExporter`), if not already present. PR #638 added `opentelemetry` as a dev-dependency
  to `temper-api` and `tests/e2e` for exactly this reason; check whether `temper-services` has it
  before adding, and use the workspace-pinned version (`[workspace.dependencies]` in the root
  `Cargo.toml`), never a fresh version string.

This test file owns a **process-global** tracing subscriber, which cannot be installed twice — the
same constraint `crates/temper-api/tests/telemetry_flush_test.rs` documents. It must be its own file,
and it must contain exactly one test that installs the subscriber.

- [x] **Step 1: Write the failing witness**

```rust
#![cfg(feature = "test-db")]
//! The drain-span gate.
//!
//! One test, own file: it installs a process-global tracing subscriber and tracer provider, neither
//! of which can be installed twice. nextest gives each test its own process, so this one owns both —
//! the same ownership `temper-api/tests/telemetry_flush_test.rs` documents.
//!
//! ## What bites here, and what would not
//!
//! Asserting "a `region_job` span exists" fails today against the *absence of the feature*, which is
//! a bite against nothing: any code emitting a span of that name satisfies it. So the assertion
//! below is on the **value** of `queue_wait_ms` against a controlled enqueue-to-claim gap. It fails
//! if the field is missing AND if it is computed wrongly, which is the failure that actually ships.

use std::time::Duration;

use opentelemetry_sdk::trace::InMemorySpanExporter;
use sqlx::PgPool;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

use temper_services::services::drain_span::{DRAIN_DISPATCH_FIELDS, DRAIN_JOB_FIELDS};

#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn a_drained_job_reports_its_queue_wait_on_its_own_span(pool: PgPool) {
    let exporter = InMemorySpanExporter::default();
    assert!(
        temper_telemetry::export::install_test_provider(exporter.clone()),
        "a provider was already installed — this test owns the process"
    );
    let layer = temper_telemetry::export::test_export_layer()
        .expect("the layer must exist once a provider is installed");
    tracing_subscriber::registry().with(layer).init();

    // Seed an anchor with a queued region job, then wait a known interval before draining so the
    // measured queue wait has a floor the assertion can bite on.
    let anchor = seed_anchor_with_queued_job(&pool).await;
    tokio::time::sleep(Duration::from_millis(1_200)).await;

    temper_services::services::region_service::dispatch_tick(&pool, Some(1))
        .await
        .expect("a drain pass");

    // Unlike `telemetry_flush_test`, nothing here is behind an HTTP middleware, so no flush happens
    // for us — a batch processor holds unflushed spans forever and the assertions below would read
    // an empty exporter. `force_flush_spans` is the real symbol (`temper_telemetry::lib.rs:89`).
    temper_telemetry::force_flush_spans();
    let spans = exporter.get_finished_spans().expect("exporter readable");

    let job = spans
        .iter()
        .find(|s| s.name == "region_job")
        .expect("the drain emitted no `region_job` span");

    // THE BITE: a value, not an existence. The job sat in the queue for at least the sleep above.
    let wait = attr_i64(job, "queue_wait_ms").expect("`queue_wait_ms` absent from the job span");
    assert!(
        wait >= 1_000,
        "queue_wait_ms = {wait}ms, but the job was queued for at least 1200ms before the drain \
         claimed it — the field is present but computed wrong"
    );

    // Identity and outcome travel with it, or an operator cannot split the aggregate by anchor.
    assert_eq!(
        attr_str(job, "anchor_id").as_deref(),
        Some(anchor.to_string().as_str())
    );
    assert_eq!(attr_str(job, "outcome").as_deref(), Some("completed"));

    // The tick span exists and declares the shared vocabulary.
    let tick = spans
        .iter()
        .find(|s| s.name == "region_dispatch")
        .expect("the drain emitted no `region_dispatch` span");
    for field in DRAIN_DISPATCH_FIELDS {
        assert!(
            has_attr(tick, field),
            "`{field}` is in DRAIN_DISPATCH_FIELDS but no `region_dispatch` span carries it, so the \
             convention names a field the operator queries will never find"
        );
    }
    for field in DRAIN_JOB_FIELDS {
        assert!(
            has_attr(job, field),
            "`{field}` is in DRAIN_JOB_FIELDS but no `region_job` span carries it"
        );
    }

    // THE NEGATIVE, and the reason the job span exists at all: none of this may be on the request
    // root. With no child span `Span::current().record(..)` silently resolves to the root, so this
    // is what keeps the convention a property of the code rather than of remembering.
    if let Some(root) = spans
        .iter()
        .find(|s| s.span_kind == opentelemetry::trace::SpanKind::Server)
    {
        for field in DRAIN_JOB_FIELDS {
            assert!(
                !has_attr(root, field),
                "`{field}` landed on the request ROOT span. Job fields belong on `region_job`; \
                 recording onto the root works only until something nests below it."
            );
        }
    }
}
```

Helpers `attr_i64` / `attr_str` / `has_attr` read `SpanData.attributes` by key; write them at the
bottom of this file. `seed_anchor_with_queued_job` creates a context anchor and calls
`workflow_job_service::enqueue_anchor` — reuse the seeding already in
`crates/temper-services/tests/region_tick_off_request_test.rs` rather than inventing a second one.

- [x] **Step 2: Run it to verify it fails for the right reason**

Run: `cargo nextest run -p temper-services --features test-db --test drain_span_test`
Expected before Tasks 2–3 are in: FAIL on *"the drain emitted no `region_job` span"*.
Expected with Tasks 2–3 in: **PASS**.

If it passes but you have not yet implemented Task 2, stop — the test is finding something else.

- [x] **Step 3: Verify the bite by mutation**

Temporarily change `read_queue_waits`'s SQL from `(leased_at - enqueued_at)` to
`(leased_at - leased_at)`. Re-run.
Expected: **FAIL** with "the field is present but computed wrong".
Then restore the SQL from your file copy — do not `git checkout` to undo a probe edit.

Record the observed RED and GREEN output in the commit message. A witness whose bite was never
observed is a witness nobody has verified.

- [x] **Step 4: Run the whole services suite**

```bash
cargo nextest run -p temper-services --features test-db
cargo make check
```

- [x] **Step 5: Commit**

```bash
git add crates/temper-services/tests/drain_span_test.rs
git commit -m "drain instrumentation: the gate — a value bite on queue wait, and job fields stay off the root"
```

---

## Task 5: Close the loop on the docs

**Files:**
- Modify: `docs/guides/drain-operator-queries.md`

- [x] **Step 1: Re-mark every query that can now be verified**

With the spans emitting locally, the `[blind]` marks on A1, A2, A3, B1, B2, C2, D1 can be upgraded to
`[shape]` only if you actually ran the aggregation form against real spans. **Do not upgrade a mark
you have not run.** A query that reaches production still marked `[blind]` is honest; one marked
`[live]` that nobody ran is not.

Locally you can verify field *presence* but not the TraceQL, since local spans do not reach Tempo.
So the honest local upgrade is: leave the marks, and add one line under the status table recording
that field presence was verified by `drain_span_test.rs` on <date>.

- [x] **Step 2: Note the post-deploy follow-up**

Add to the end of `docs/guides/drain-operator-queries.md`:

```markdown
## Post-deploy follow-up

Once these spans are flowing in production, re-run every query on this page against Tempo and
re-mark it. A query still marked `[blind]` after the spans exist is a query nobody has run — and the
answer to "is the drain keeping up?" should not rest on one of those.
```

- [x] **Step 3: Commit**

```bash
git add docs/guides/drain-operator-queries.md
git commit -m "drain instrumentation: record what the local gate verified, and what still needs production"
```

---

## Self-review notes

**Spec coverage.** Every spec section maps to a task: span shape → T2/T3; field set → T1 constants +
T4 tie-assertion; sources table → T1 reads; the two "must be written down" items → T1 module doc
(span kind) and T1 Step 8 (the `enqueued_at` bias, required to be *resolved*); testing → T4;
*What this does not do* → nothing to implement, by construction.

**Deliberately not in this plan.** The steward drain (spec exclusion). Alerting thresholds (spec:
the range is uncharacterized). Any change to `RegionDispatchSummary` / `EmbedDispatchSummary` — they
are wire types with consumers and this plan adds observation, not behaviour. Anything touching
`projection-lag-is-readable`, which stays declared uncovered under goal `019fc46c`.

**The one open question carried from the spec**, unresolved and deliberately so:
`oldest_pending_age_ms` is unbounded — a never-claimed job grows it forever, which is a cardinality
question for whatever aggregates it. It is recorded as a raw value here because bucketing it before
anyone has seen its range would be guessing. Revisit after the post-deploy pass in Task 5.

---

## Execution record — 2026-08-03

Executed inline in the authoring session. Four deviations from the plan as written, each with its
reason, so a later reader is not misled by the ticked boxes.

**1. The module was registered in Task 1 Step 1, not Step 6.** As written, Steps 1–2 run a test in a
module `mod.rs` does not yet declare — so nothing compiles it and the RED is not observed. Registering
first made the RED real: `use of undeclared type JobOutcome`, plus two `E0282`s.

**2. Tasks 1–3 were batched into one compile.** A `temper-services` build is ~6–11 minutes here, and
Tasks 2 and 3 add no new tests (their check is *"the existing tests still pass"*). No bite was lost:
Task 1's RED was observed before implementing, and Task 4's mutation bite was run separately.

**3. `EmbedJobResult` carries `u64`, not `i64`.** The plan's sketch had `i64`; the real types are
`ChunkProgress::embedded: u64` and `EmbedDispatchSummary::chunks_embedded: u64`, with only the
per-claim `budget` as `i64`. Four compile errors, fixed by matching the incumbents and casting at
the budget decrement — exactly as the original code did.

**4. Task 3's Step 6 named a test target that does not exist.** Embed's drain tests are **lib** tests
(`embed_service.rs`, `mod tests`), not an integration target — `--test embed_dispatch_test` fails
with *"no test target named"*. The correct invocation is
`cargo nextest run -p temper-services --features test-db --lib -E 'test(embed)'`.

### What was actually verified

| | |
|---|---|
| `drain_span_test` | **PASS** — and the bite confirmed by mutation (below) |
| `region_tick_off_request_test` (4 tests) | **PASS**, unchanged — the loop restructure preserved behaviour |
| `embed_service` lib tests (20, incl. deferral, budget, redrive, concurrency) | **PASS**, unchanged |
| `cargo make check` | see the commit — run after `cargo fmt --all` |

**The mutation bite, observed.** With `(leased_at - enqueued_at)` changed to
`(leased_at - leased_at)`, the witness failed with:

```
queue_wait_ms = 0ms, but the job sat in the queue for at least 1200ms before the drain
claimed it — the field is present but computed wrong
```

The span still existed and still carried the field. Only the computation was wrong, and the test
caught it — which is the whole point of a value assertion over an existence one. Restored from a file
copy, not `git checkout`.

### Not run, and why

**The full `temper-services` suite did not complete locally.** It stalls on macOS Gatekeeper —
`syspolicyd` at 98% CPU with nextest at 0%, across ~40 freshly-signed test binaries, zero tests run
in 20 minutes. That is an environment property, not a signal about this change. The targeted suites
above cover every file this change touches; CI runs the rest.

### Carried forward, unresolved

`oldest_pending_age_ms` is still an unbounded raw value. Named in the spec as open and unchanged
here — the range it needs to be bucketed against does not exist until these spans run in production.
