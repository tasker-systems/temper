//! What the `api/internal` drains report about themselves.
//!
//! Both drains (region materialization, embedding) emit the same two-level span shape: a
//! `*_dispatch` tick span carrying queue state and tallies, and a `*_job` span per claimed job. The
//! field vocabulary lives here, in one declaration, for the reason [`crate::services::region_service`]'s
//! module doc gives about the twins generally — two copies drift, and a drifted field name silently
//! empties an operator's panel rather than failing anything.
//!
//! These spans are deliberately `internal` kind. Tempo's span-metrics processor derives RED only
//! from `server`/`client` spans, so none of this appears in `traces_spanmetrics_*` — that is
//! correct. They are not request boundaries, and the aggregation route is TraceQL metrics, which
//! reads any span. See `docs/guides/drain-operator-queries.md`.

use std::collections::HashMap;

use sqlx::PgPool;
use uuid::Uuid;

use crate::error::ApiResult;

/// Fields every `*_dispatch` tick span declares. Tied to the attribute by the drain-span gate
/// (`crates/temper-services/tests/drain_span_test.rs`) — a constant nothing asserts its consumers
/// against prevents no drift at all, the lesson `ACT_SPAN_FIELDS` records.
pub const DRAIN_DISPATCH_FIELDS: [&str; 6] = [
    "backlog_depth",
    "oldest_pending_age_ms",
    "claimed",
    "completed",
    "deferred",
    "failed",
];

/// Fields every `*_job` span declares, whichever drain emitted it. Per-drain identity
/// (`anchor_id` / `resource_id`) and per-drain outcome detail are deliberately NOT here: they differ
/// by drain, and forcing them into the shared set would make one drain declare a field it can never
/// fill.
pub const DRAIN_JOB_FIELDS: [&str; 3] = ["queue_wait_ms", "attempts", "outcome"];

/// How one claimed job ended.
///
/// A closed vocabulary, not a boolean, because the middle two are neither success nor failure and
/// collapsing them into either misreports a healthy drain under load as a failing one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JobOutcome {
    /// Work done, job marked done.
    Completed,
    /// The invocation's wall-clock deadline was already spent when this job came up. Handed back
    /// untouched, **no work attempted**.
    Deferred,
    /// Work was done but did not finish this claim's budget; re-enqueued to resume. Embed only — a
    /// region tick has no partial state (see [`crate::services::region_service`]'s module doc).
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

/// The queue as a tick found it.
#[derive(Debug, Clone, Copy, Default)]
pub struct QueueDepth {
    /// Jobs this tick *could have claimed*, counted before it claimed any.
    pub backlog_depth: i64,
    /// Age of the oldest such job, in ms. `None` when the queue is empty — which is not zero, and
    /// must not be recorded as zero: "nothing waiting" and "something arrived this instant" are
    /// different states, and an operator reads the difference.
    pub oldest_pending_age_ms: Option<i64>,
}

/// Read the claimable backlog for one `(persona, dispatch_type)`.
///
/// **The predicate is copied from `workflow_job_claim_anchor`'s inner `SELECT`**
/// (`migrations/20260802000020_workflow_jobs_anchor_scope.sql`) — status in
/// `('pending','waiting_for_retry')` and `next_visible_at <= now()`. Counting anything the claim
/// would not have taken reports a backlog that is not this drain's to drain: a retry scheduled two
/// minutes out is not work the current tick is behind on.
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
/// **On the `now()`-is-transaction-start question.** `enqueued_at` defaults to `now()`, which in
/// Postgres is transaction-start rather than statement time — the trap that made a 23s salience
/// refresh look instant when read through `kb_events.occurred_at`. Here it is **not** a bias:
/// `DbBackend::queue_region_clocks` is called on `&self.pool` *after* the write's transaction has
/// committed, so the enqueue's transaction contains only its own `INSERT` and `now()` is effectively
/// statement time. If a future caller ever enqueues from inside a longer transaction, this number
/// starts overstating the wait by that transaction's duration, and this doc comment is the thing to
/// come back and correct.
///
/// **What the number measures under single-flight, which is not what it looks like.**
/// `workflow_job_enqueue_anchor` is `ON CONFLICT DO NOTHING`, so when N arrivals land on one anchor
/// inside a settling window they fold into **one** job and `enqueued_at` stays the *first*
/// arrival's. So this is the age of the oldest unsettled change, not of the most recent one — which
/// is the right quantity for "how far behind are we", and the wrong one for "how long did my write
/// wait". An operator reading a p95 here is reading the former.
///
/// Jobs whose `leased_at` is somehow NULL are omitted from the map rather than defaulted to `0`; a
/// zero wait is a real measurement (a claim that arrived instantly), and inventing one would put a
/// false floor in the operator's p50.
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
        for set in [
            DRAIN_DISPATCH_FIELDS.as_slice(),
            DRAIN_JOB_FIELDS.as_slice(),
        ] {
            for (i, f) in set.iter().enumerate() {
                assert!(!f.is_empty(), "empty field name at index {i}");
                assert!(
                    !set[..i].contains(f),
                    "`{f}` is declared twice — a duplicate makes the tie-assertion pass vacuously \
                     for one of them"
                );
            }
        }
    }
}
