//! Retention sweep for abandoned staged blob-upload sessions — `kb_blob_uploads` and its
//! `kb_blob_upload_segments`.
//!
//! A staged session is kept resumable through every finalize *failure* by design
//! (20260903000040: "the pair dies at finalize (success); every finalize FAILURE keeps it,
//! and abandonment is left to a TTL reaper — declared as a hole, not silently cleaned").
//! Until this existed that reaper was the hole: a session begun and never finalized lived
//! forever, and each of its segments is a `bytea` on disk. This closes the hole with the
//! rulings of 2026-09-05 (task 01a0715d), conformed to
//! [`crate::services::as_reap_service`] — the incumbent retention sweep this module is a
//! sibling of: an env-knob TTL that refuses rather than substitutes, the cron as its only
//! driver, capped batched passes, and the non-fatal posture.
//!
//! **No events, deliberately.** Staged sessions ride no events by design (witnessed in the
//! blobs register, S5) — pre-ledger transport state whose bytes never ride the trail — so
//! reaping is event-less row deletion, and observability is the sweep's own log line and
//! the cron summary, never the ledger. Nothing in the graph surfaces can see a staged session (the DDL's own contract:
//! not a blob, not a resource, not an edge), so deletion cannot orphan anything readable;
//! its only gate is owner-equality on the session row, which deletion simply ends.
//!
//! **No provider call is involved.** The bytes are in-DB `bytea` until finalize assembles
//! and puts them; an abandoned session never reached the provider, so reaping it is pure
//! database work.

use serde::Serialize;
use sqlx::PgPool;

use crate::error::{ApiError, ApiResult};

/// Names how long an untouched staged session is kept resumable, in seconds.
const STAGING_TTL_ENV: &str = "BLOB_UPLOAD_STAGING_TTL_SECONDS";

/// Assumed staging TTL when [`STAGING_TTL_ENV`] is unset — 24 hours.
///
/// The window is for a client that went away mid-upload, not for one that is slow: append
/// refreshes `updated`, so any live activity keeps the session past every threshold here.
/// A day covers a stalled browser tab, a retried CI run, a human who went to lunch; the
/// failure of guessing too LARGE is bounded storage per abandoned session, and guessing
/// too SMALL discards a resumable upload its owner may still finish.
const DEFAULT_STAGING_TTL_SECONDS: f64 = 86_400.0;

/// Ceiling on a configured staging TTL — 7 days. Not a policy, a units check, in the exact
/// role of `as_reap_service::MAX_ASSERTION_WINDOW_SECONDS`: a larger
/// value is a milliseconds-for-seconds slip, and it is refused rather than clamped for the
/// same reason the floor below refuses — a misconfigured cron must be diagnosable, not
/// silently reinterpreted.
const MAX_STAGING_TTL_SECONDS: f64 = 604_800.0;

/// Rows deleted per statement. Bounds the lock any one `DELETE` holds — same number, same
/// reasoning as `as_reap_service::BATCH_ROWS`.
const BATCH_ROWS: i64 = 5_000;

/// Rows deleted per run.
///
/// The first run is the one this exists for: the hole has been open since 20260903000040,
/// so an unbounded first pass would take out every session ever abandoned in one statement.
/// Capping the run lets the backlog drain over several scheduled passes — the incumbent's
/// `MAX_ROWS_PER_TABLE` posture, role for role.
const MAX_ROWS_PER_RUN: i64 = 50_000;

/// What one sweep reaped: abandoned sessions, and the staged bytes they took with them.
#[derive(Debug, Default, Serialize, PartialEq, Eq)]
pub struct BlobReapSummary {
    pub uploads_reaped: i64,
    /// Staged bytes freed, summed over the segments the reaped sessions cascaded away —
    /// in-DB `bytea`, so this is the storage the sweep actually returned.
    pub bytes_freed: i64,
    /// True when the run stopped because it hit its per-run cap rather than because it ran
    /// out of abandoned sessions — there is more to delete and the next scheduled pass will
    /// take it. Carried out of the sweep's own exit, never inferred by comparing the count
    /// to the cap; reachable at exactly one corner, where it errs toward announcing a
    /// follow-up pass that finds nothing: a table holding precisely `MAX_ROWS_PER_RUN`
    /// eligible rows drains on its last batch and still reports the cap.
    pub more_pending: bool,
}

/// The configured staging TTL, in seconds.
///
/// **REFUSES an unusable value rather than substituting the default**, naming the variable
/// so a misconfigured cron is diagnosable — the incumbent's posture verbatim
/// (`as_reap_service::assertion_window_seconds`). The deciding question
/// is that one's: what does a refusal cost, and is the number one an operator *states*?
/// It is — and a refusal is cheap here in a way it is not on a request path: it fails one
/// cron run, and a run that does not happen leaves rows on disk. **Not reaping is the
/// fail-safe direction** — resumable sessions are kept, which is this module's own
/// incumbent state and has never lost a caller bytes.
fn staging_ttl_seconds(raw: Option<String>) -> ApiResult<f64> {
    let Some(raw) = raw.filter(|s| !s.trim().is_empty()) else {
        return Ok(DEFAULT_STAGING_TTL_SECONDS);
    };
    let parsed: f64 = raw.trim().parse().unwrap_or(f64::NAN);
    if !parsed.is_finite() || parsed <= 0.0 || parsed > MAX_STAGING_TTL_SECONDS {
        return Err(ApiError::Internal(format!(
            "{STAGING_TTL_ENV} must be a positive number of seconds no greater than \
             {MAX_STAGING_TTL_SECONDS}; got {raw:?}"
        )));
    }
    Ok(parsed)
}

/// One retention sweep over abandoned staged sessions.
///
/// **Abandonment is age on `updated`, never `created`.** `updated` is the last begin/append
/// touch — the append path stamps it on every landed segment
/// (`temper_substrate::uploads::append_segment`) — so the window is judged on the session's
/// last live activity, and a long-lived session that is still being appended to is never
/// reaped whatever day it was begun.
///
/// Idempotent: a re-run reaps whatever is still past the TTL and nothing else. Sessions are
/// deleted in batches (`ctid`, no `ORDER BY` — the incumbent measured both choices and the
/// reasoning carries: an unordered LIMIT stops at the batch, and the freshness index keeps
/// the no-op pass cheap). A `ctid` gone stale under a concurrent append is simply re-checked
/// by the DELETE's predicate and skipped; a session refreshed between the subquery and the
/// delete no longer matches, so a live upload cannot be caught mid-append — the harmless
/// direction for a reaper.
///
/// The segments ride `ON DELETE CASCADE` (20260903000040) — deleting the session row is the
/// whole act. The freed bytes are summed from the deleted sessions' segments *inside the
/// same statement*: every sub-statement of a data-modifying CTE reads the statement's own
/// snapshot, so `freed`, placed after `gone`, still sees the segments the cascade is
/// removing — and joining on `gone` (not the victim set) keeps the count exact when the
/// DELETE's lock-wait re-check skips a session a concurrent append refreshed.
pub async fn reap_abandoned_blob_uploads(pool: &PgPool) -> ApiResult<BlobReapSummary> {
    let ttl = staging_ttl_seconds(std::env::var(STAGING_TTL_ENV).ok())?;

    let mut total_rows = 0i64;
    let mut total_bytes = 0i64;
    let mut more_pending = false;
    loop {
        // Two different reasons to stop, and the caller needs to tell them apart: a short
        // batch means the table is drained, the cap means there is more waiting for
        // tomorrow.
        let swept = sqlx::query!(
            r#"
            WITH victims AS (
                 SELECT ctid, id
                   FROM kb_blob_uploads
                  WHERE updated < now() - make_interval(secs => $1::double precision)
                  LIMIT $2
            ), gone AS (
                 DELETE FROM kb_blob_uploads
                  WHERE ctid IN (SELECT ctid FROM victims)
                  RETURNING id
            ), freed AS (
                 SELECT COALESCE(SUM(octet_length(s.bytes)), 0)::bigint AS "bytes"
                   FROM kb_blob_upload_segments s
                   JOIN gone g ON s.upload_id = g.id
            )
            SELECT (SELECT count(*) FROM gone)::bigint AS "uploads!",
                   (SELECT bytes FROM freed)           AS "bytes!"
            "#,
            ttl,
            BATCH_ROWS
        )
        .fetch_one(pool)
        .await?;

        total_rows += swept.uploads;
        total_bytes += swept.bytes;
        if swept.uploads < BATCH_ROWS {
            break;
        }
        if total_rows >= MAX_ROWS_PER_RUN {
            more_pending = true;
            break;
        }
    }
    let summary = BlobReapSummary {
        uploads_reaped: total_rows,
        bytes_freed: total_bytes,
        more_pending,
    };

    // Emitted every run, including the all-zero one — the incumbent's invariant verbatim:
    // a reaper that logs only when it deletes is indistinguishable from a reaper that is
    // not running, and Vercel Cron discards this endpoint's response body, so this line is
    // the sweep's only observable trail.
    tracing::info!(
        uploads_reaped = summary.uploads_reaped,
        bytes_freed = summary.bytes_freed,
        more_pending = summary.more_pending,
        staging_ttl_seconds = ttl,
        "blob staging retention sweep complete"
    );
    Ok(summary)
}

#[cfg(test)]
mod ttl_tests {
    use super::*;

    #[test]
    fn an_unset_ttl_falls_back_to_the_default_day() {
        assert_eq!(
            staging_ttl_seconds(None).unwrap(),
            DEFAULT_STAGING_TTL_SECONDS
        );
        assert_eq!(
            staging_ttl_seconds(Some("   ".to_string())).unwrap(),
            DEFAULT_STAGING_TTL_SECONDS
        );
    }

    #[test]
    fn a_configured_ttl_is_honoured_exactly() {
        assert_eq!(staging_ttl_seconds(Some("3600".into())).unwrap(), 3600.0);
        assert_eq!(
            staging_ttl_seconds(Some("604800".into())).unwrap(),
            MAX_STAGING_TTL_SECONDS
        );
    }

    // FAILS IF: an unusable value is silently swallowed and the default served instead —
    // an operator who states a TTL and gets a shorter one has resumable sessions deleted
    // early, with nothing anywhere disagreeing.
    #[test]
    fn an_unusable_ttl_refuses_rather_than_substituting() {
        for raw in ["8h", "0", "-1", "not-a-number", "60480000000"] {
            let err = staging_ttl_seconds(Some(raw.to_string()))
                .expect_err("must refuse rather than substitute the default");
            assert!(
                matches!(&err, ApiError::Internal(m) if m.contains(STAGING_TTL_ENV)),
                "the refusal must name the variable so a misconfigured cron is diagnosable; got {err:?}",
            );
        }
    }
}

#[cfg(all(test, feature = "test-db"))]
mod tests;
