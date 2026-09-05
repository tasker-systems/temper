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
mod tests {
    use super::*;
    use sqlx::PgPool;
    use uuid::Uuid;

    // Survivor-style assertions throughout, per the AS reaper suite's own note: they name
    // the rows they mean and cannot be moved by a row the test did not create.

    /// A staged session `updated_age_seconds` old by `updated` and
    /// `created_age_seconds` old by `created` (the two are independent — the
    /// fresh-but-old witness passes them unequal on purpose), with `segment_count`
    /// segments of `segment_bytes` bytes each. Stamped the way the real path stamps:
    /// `created` at begin, `updated` advancing on every landed append — here expressed
    /// directly, since the witnesses are about what the sweep reads, not about the append
    /// path's own UPDATE (witnessed in substrate).
    async fn seed_session(
        pool: &PgPool,
        id: &Uuid,
        updated_age_seconds: i64,
        created_age_seconds: i64,
        segment_count: i32,
        segment_bytes: i64,
    ) {
        let owner = Uuid::now_v7();
        sqlx::query("INSERT INTO kb_profiles (id, handle, display_name) VALUES ($1, $2, 'Probe')")
            .bind(owner)
            .bind(format!("probe-{owner}"))
            .execute(pool)
            .await
            .expect("seed owner profile");
        sqlx::query(
            "INSERT INTO kb_blob_uploads (id, owner_profile_id, home_table, home_id, \
                                        content_type, created, updated) \
             VALUES ($1, $2, 'kb_contexts', $2, 'application/octet-stream', \
                     now() - make_interval(secs => $3::double precision), \
                     now() - make_interval(secs => $4::double precision))",
        )
        .bind(id)
        .bind(owner)
        .bind(created_age_seconds as f64)
        .bind(updated_age_seconds as f64)
        .execute(pool)
        .await
        .expect("seed staged session");

        for seq in 0..segment_count {
            sqlx::query(
                "INSERT INTO kb_blob_upload_segments (upload_id, seq, bytes, segment_hash) \
                 VALUES ($1, $2, $3, $4)",
            )
            .bind(id)
            .bind(seq)
            .bind(vec![b'x'; segment_bytes as usize])
            .bind(format!("hash-{id}-{seq}"))
            .execute(pool)
            .await
            .expect("seed segment");
        }
    }

    async fn survivor_ids(pool: &PgPool) -> Vec<Uuid> {
        sqlx::query_scalar("SELECT id FROM kb_blob_uploads ORDER BY id")
            .fetch_all(pool)
            .await
            .expect("read back sessions")
    }

    async fn survivor_segments(pool: &PgPool) -> i64 {
        sqlx::query_scalar("SELECT count(*) FROM kb_blob_upload_segments")
            .fetch_one(pool)
            .await
            .expect("read back segments")
    }

    /// **The headline ruling, asserted in the direction that matters.**
    ///
    /// FAILS IF: the sweep reaps on `created`. `fresh-but-old` was begun three days ago and
    /// appended to an hour ago — old by `created`, alive by `updated` — and deleting it
    /// discards a resumable upload its owner is still working on. Its stale sibling, whose
    /// last touch is past the window, is the row this sweep exists for.
    #[sqlx::test(migrations = "../../migrations")]
    async fn a_session_past_the_ttl_is_reaped_and_a_touched_one_survives(pool: PgPool) {
        let stale = Uuid::now_v7();
        let fresh_but_old = Uuid::now_v7();
        seed_session(&pool, &stale, 2 * 86_400, 2 * 86_400, 2, 512).await;
        seed_session(&pool, &fresh_but_old, 3_600, 3 * 86_400, 1, 256).await;

        let summary = reap_abandoned_blob_uploads(&pool).await.expect("sweep");

        assert_eq!(
            survivor_ids(&pool).await,
            vec![fresh_but_old],
            "a session past the TTL on its last touch goes; one touched inside the window \
             stays, however old its begin"
        );
        assert_eq!(
            survivor_segments(&pool).await,
            1,
            "the surviving session keeps its segments"
        );
        assert_eq!(
            summary,
            BlobReapSummary {
                uploads_reaped: 1,
                bytes_freed: 2 * 512,
                more_pending: false,
            },
            "the summary reports the reaped rows and the bytes they freed"
        );
    }

    /// FAILS IF: the cascade does not carry, or the sweep stopped at the session row and
    /// left the bytes. The freed storage is the segments; a reaper that reports bytes but
    /// leaves `bytea` rows behind is a lie with a summary.
    #[sqlx::test(migrations = "../../migrations")]
    async fn a_reaped_session_takes_its_segments_with_it(pool: PgPool) {
        let doomed = Uuid::now_v7();
        seed_session(&pool, &doomed, 2 * 86_400, 2 * 86_400, 3, 1024).await;

        reap_abandoned_blob_uploads(&pool).await.expect("sweep");

        assert!(
            survivor_ids(&pool).await.is_empty(),
            "the past-TTL session is gone"
        );
        assert_eq!(
            survivor_segments(&pool).await,
            0,
            "the session's staged bytes are gone with it"
        );
    }

    /// FAILS IF: the sweep is not idempotent. The cron runs it on a schedule forever; a
    /// second pass over the same state must delete nothing and must not error.
    #[sqlx::test(migrations = "../../migrations")]
    async fn a_second_sweep_over_the_same_state_deletes_nothing(pool: PgPool) {
        let ancient = Uuid::now_v7();
        seed_session(&pool, &ancient, 100 * 86_400, 100 * 86_400, 1, 64).await;

        let first = reap_abandoned_blob_uploads(&pool)
            .await
            .expect("first sweep");
        assert!(
            survivor_ids(&pool).await.is_empty(),
            "the first sweep clears what is past the TTL"
        );
        assert_eq!(first.uploads_reaped, 1);

        let second = reap_abandoned_blob_uploads(&pool)
            .await
            .expect("second sweep");
        assert_eq!(
            second,
            BlobReapSummary::default(),
            "a re-run is a no-op, and says so in its summary"
        );
    }

    /// The read-back the summary's `bytes_freed` rests on: `octet_length` over the victims'
    /// segments, summed in the delete's own statement. A live session (untouched by the
    /// sweep) contributes nothing to either count — checked from the summary rather than a
    /// hand-tallied constant, so a seeding bug cannot flatter the sweep.
    #[sqlx::test(migrations = "../../migrations")]
    async fn the_summary_counts_only_what_the_sweep_reaped(pool: PgPool) {
        let doomed = Uuid::now_v7();
        let survivor = Uuid::now_v7();
        seed_session(&pool, &doomed, 2 * 86_400, 2 * 86_400, 2, 100).await;
        seed_session(&pool, &survivor, 60, 60, 4, 1_000).await;

        let summary = reap_abandoned_blob_uploads(&pool).await.expect("sweep");

        assert_eq!(
            summary,
            BlobReapSummary {
                uploads_reaped: 1,
                bytes_freed: 200,
                more_pending: false,
            },
            "the survivor's four live segments count toward neither row nor byte"
        );
        assert_eq!(survivor_ids(&pool).await, vec![survivor]);
    }

    // The per-run cap (`more_pending`) is the incumbent's measured posture carried over
    // wholesale; driving 5_001 sessions through seeding to bite it would be ceremony.
    // Unwitnessed HERE, and — checked before writing this note — unwitnessed in the
    // incumbent's suite too: no test anywhere drives the cap exit. The loop is carried by
    // inspection, line for line. Declared, not silently absent.
}
