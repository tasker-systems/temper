//! Retention sweep for the three Authorization Server tables that otherwise grow forever.
//!
//! `kb_saml_replay`, `kb_oauth_flow` and `kb_oauth_refresh_tokens` each carry an `expires_at` and,
//! until this existed, nothing ever deleted from any of them: every consumed SAML assertion, every
//! authorization-code flow and every refresh token ever issued was still on disk.
//! `idx_kb_saml_replay_expires` (20260701000006) has been shipping since the tables were created
//! with no consumer at all — it was built for this sweep.
//!
//! **Two of the three must not be reaped merely because `expires_at` has passed, and each has its
//! own reason.** Deleting on `expires_at` is the obvious implementation and it is wrong twice over;
//! the floors below are the whole point of this module.

use serde::Serialize;
use sqlx::PgPool;

use crate::error::{ApiError, ApiResult};

/// Names the widest assertion validity window the configured IdP issues, in seconds.
///
/// This is the number an operator reads off their IdP, not a number about temper.
const ASSERTION_WINDOW_ENV: &str = "AS_SAML_ASSERTION_MAX_SECONDS";

/// Assumed assertion window when [`ASSERTION_WINDOW_ENV`] is unset — one hour.
///
/// Generous on purpose, and the asymmetry is the reason. Too LARGE costs storage: a `kb_saml_replay`
/// row is an assertion id and a timestamp, so an extra hour of them is nothing. Too SMALL re-opens
/// replay. A default has to guess, so it guesses in the direction whose failure is cheap. One hour
/// comfortably covers the 5-minute `NotOnOrAfter` an IdP typically issues, and covers the 10 minutes
/// `REPLAY_TTL_SECONDS` (`packages/temper-cloud/src/oauth/endpoints.ts`) assumes.
const DEFAULT_ASSERTION_WINDOW_SECONDS: f64 = 3600.0;

/// Ceiling on a configured assertion window — 7 days. Not a policy, a units check: an IdP does not
/// issue an assertion valid for longer than a week, so a larger value is a milliseconds-for-seconds
/// slip. Same role as `MAX_REFRESH_CHAIN_SECONDS` in the AS.
const MAX_ASSERTION_WINDOW_SECONDS: f64 = 604_800.0;

/// How long past `expires_at` a dead authorization-code flow is kept — one day.
///
/// Nothing reads an expired flow row: `consumeCode` (`packages/temper-cloud/src/oauth/flow.ts:122`)
/// scopes its lookup to `expires_at > now()`, so an expired row cannot be redeemed and no
/// code-replay signal is derived from one. The day is for a human, not for the protocol — it leaves
/// a failed login's row in place long enough to be looked at.
const FLOW_GRACE_SECONDS: f64 = 86_400.0;

/// How long past a refresh chain's end its tokens are kept — 30 days.
///
/// A forensic window, not a protocol one: once `chain_expires_at` has passed, no token of the chain
/// can rotate, so these rows grant nothing. What they still do is answer *"was this credential
/// copied?"* — see [`reap_refresh_tokens`].
const REFRESH_FORENSIC_SECONDS: f64 = 2_592_000.0;

/// Rows deleted per statement. Bounds the lock any one `DELETE` holds.
const BATCH_ROWS: i64 = 5_000;

/// Rows deleted per table per run.
///
/// **The first run is the one this exists for.** These tables have accumulated since
/// 20260701000006 with no reaper, so an unbounded `DELETE` would take out months of rows in one
/// statement — on the same connection pool serving logins. Capping the run means the backlog drains
/// over several scheduled passes instead of one long one, and every pass is a short transaction.
const MAX_ROWS_PER_TABLE: i64 = 50_000;

/// What one sweep deleted, per table.
#[derive(Debug, Default, Serialize, PartialEq, Eq)]
pub struct AsReapSummary {
    pub saml_replay: i64,
    pub oauth_flow: i64,
    pub refresh_tokens: i64,
    /// True when any table hit its per-run cap — there is more to delete and the next
    /// scheduled pass will take it. Reported rather than inferred from the counts: a caller
    /// comparing a count to a constant is reconstructing something this already knows.
    pub more_pending: bool,
}

/// The configured IdP assertion window, in seconds.
///
/// **REFUSES an unusable value rather than substituting the default.** This parts company with the
/// AS's `refreshReplayGraceSeconds`, which substitutes and warns, and follows its
/// `refreshChainMaxSeconds`, which refuses — and the deciding question is the same one that split
/// those two: what does a refusal cost, and is the number one an operator *states*?
///
/// It is. This is the floor below which a consumed assertion must not be forgotten, so silently
/// serving one hour to an operator who wrote `AS_SAML_ASSERTION_MAX_SECONDS=8h` would delete their
/// rows seven hours early with nothing anywhere disagreeing. And a refusal is cheap here in a way it
/// is not on a login path: it fails one cron run, and a run that does not happen leaves rows on
/// disk. **Not reaping is the fail-safe direction** — it is the state this whole module is fixing,
/// and it has never weakened the replay guard.
fn assertion_window_seconds(raw: Option<String>) -> ApiResult<f64> {
    let Some(raw) = raw.filter(|s| !s.trim().is_empty()) else {
        return Ok(DEFAULT_ASSERTION_WINDOW_SECONDS);
    };
    let parsed: f64 = raw.trim().parse().unwrap_or(f64::NAN);
    if !parsed.is_finite() || parsed <= 0.0 || parsed > MAX_ASSERTION_WINDOW_SECONDS {
        return Err(ApiError::Internal(format!(
            "{ASSERTION_WINDOW_ENV} must be a positive number of seconds no greater than \
             {MAX_ASSERTION_WINDOW_SECONDS}; got {raw:?}"
        )));
    }
    Ok(parsed)
}

/// One retention sweep across all three AS tables.
///
/// Idempotent: a re-run deletes whatever is still past its floor and nothing else. Each table is
/// swept independently, so one table's backlog cannot starve another's.
pub async fn reap_expired_as_rows(pool: &PgPool) -> ApiResult<AsReapSummary> {
    let window = assertion_window_seconds(std::env::var(ASSERTION_WINDOW_ENV).ok())?;

    let saml_replay = reap_saml_replay(pool, window).await?;
    let oauth_flow = reap_oauth_flows(pool).await?;
    let refresh_tokens = reap_refresh_tokens(pool).await?;

    let summary = AsReapSummary {
        saml_replay,
        oauth_flow,
        refresh_tokens,
        more_pending: saml_replay >= MAX_ROWS_PER_TABLE
            || oauth_flow >= MAX_ROWS_PER_TABLE
            || refresh_tokens >= MAX_ROWS_PER_TABLE,
    };

    // Emitted every run, including the all-zero one. A reaper that logs only when it deletes is
    // indistinguishable from a reaper that is not running.
    tracing::info!(
        saml_replay = summary.saml_replay,
        oauth_flow = summary.oauth_flow,
        refresh_tokens = summary.refresh_tokens,
        more_pending = summary.more_pending,
        assertion_window_seconds = window,
        "AS retention sweep complete"
    );
    Ok(summary)
}

/// Sweep consumed SAML assertion ids — **the floor that must not fall**.
///
/// A row here is what stops a captured assertion being presented twice, and it may only be deleted
/// once the assertion can no longer be replayed at all. That is a property of the IdP's own
/// `NotOnOrAfter`, and the deciding detail is that **temper does not know it**: `toSamlConfig`
/// (`packages/temper-cloud/src/saml/config.ts`) sets no `acceptedClockSkewMs` and reads no window,
/// and `guardReplay` is handed `now + REPLAY_TTL_SECONDS` where that constant is the literal `600`.
/// So `expires_at` on this table is a GUESS at the window, not a statement of it, and reaping on
/// `expires_at` alone would forget a still-replayable assertion for any IdP issuing a window wider
/// than ten minutes.
///
/// **So the window is subtracted rather than trusted.** The predicate is
/// `expires_at < now() - window`, and because `expires_at >= created` for any positive TTL, no row
/// can be deleted before `created + window` **whatever the writer stamped on it**. That is what
/// makes the floor derived rather than hard-coded, and it holds without the AS being changed at
/// all: rows already on disk, and rows written by a lagging binary carrying the old literal, are
/// covered by the same arithmetic as rows written by a current one.
///
/// The direction of the existing bug is preserved deliberately. Unbounded retention was **fail-safe
/// for replay** — a consumed assertion id was never forgotten, so the guard never weakened. This
/// bounds the growth without inverting that.
async fn reap_saml_replay(pool: &PgPool, window_seconds: f64) -> ApiResult<i64> {
    let mut total = 0i64;
    loop {
        // No ORDER BY. On a table that has never been reaped almost every row matches, so an
        // unordered LIMIT lets the scan stop as soon as it has a batch, where an ORDER BY would
        // sort the backlog first. `idx_kb_saml_replay_expires` remains available to the planner for
        // the range predicate itself — this is the first thing that has ever asked for it.
        let deleted = sqlx::query!(
            r#"
            DELETE FROM kb_saml_replay
             WHERE assertion_id IN (
                   SELECT assertion_id
                     FROM kb_saml_replay
                    WHERE expires_at < now() - make_interval(secs => $1::double precision)
                    LIMIT $2
             )
            "#,
            window_seconds,
            BATCH_ROWS
        )
        .execute(pool)
        .await?
        .rows_affected() as i64;

        total += deleted;
        if deleted < BATCH_ROWS || total >= MAX_ROWS_PER_TABLE {
            return Ok(total);
        }
    }
}

/// Sweep dead authorization-code flows — the one table where `expires_at` plus a margin really is
/// enough.
///
/// Checked rather than assumed: `consumeCode` scopes its lookup to `expires_at > now()`, so an
/// expired row cannot be redeemed, and unlike the refresh table there is no detection path that
/// reads a spent row. `status` is not in the predicate — a `pending_saml` flow whose user simply
/// abandoned the login is as dead as a `consumed` one once it has expired, and keying on status
/// would leak exactly the abandoned ones forever.
async fn reap_oauth_flows(pool: &PgPool) -> ApiResult<i64> {
    let mut total = 0i64;
    loop {
        let deleted = sqlx::query!(
            r#"
            DELETE FROM kb_oauth_flow
             WHERE id IN (
                   SELECT id
                     FROM kb_oauth_flow
                    WHERE expires_at < now() - make_interval(secs => $1::double precision)
                    LIMIT $2
             )
            "#,
            FLOW_GRACE_SECONDS,
            BATCH_ROWS
        )
        .execute(pool)
        .await?
        .rows_affected() as i64;

        total += deleted;
        if deleted < BATCH_ROWS || total >= MAX_ROWS_PER_TABLE {
            return Ok(total);
        }
    }
}

/// Sweep spent refresh tokens — **the floor the draft did not know it needed**.
///
/// The task this came from was written when refresh-replay detection was still hypothetical and
/// says `expires_at` plus a margin is sufficient here. It landed (20260826000140), and two things
/// followed that make that predicate wrong.
///
/// **1. Detection does not stop at expiry.** `findRotatedToken`
/// (`packages/temper-cloud/src/oauth/flow.ts:353`) matches on `rotated_at IS NOT NULL` and nothing
/// else — no expiry filter — and `judgeRefusedRotation` runs after *any* refused rotation, expiry
/// included. A stolen token presented after its own TTL is therefore recognised as a replay today,
/// and the chain is ended. Deleting the row is precisely what would turn that into an unremarkable
/// "unknown token" refusal. So the floor is `chain_expires_at`, not `expires_at`: while a chain can
/// still be replayed into, its tokens stay.
///
/// **2. `kb_oauth_refresh_replays.token_id` is `ON DELETE CASCADE`.** Deleting a token row deletes
/// any replay already recorded against it — the row an operator reads through
/// `vw_oauth_refresh_replays`, which `docs/playbooks/self-host-with-saml.md` tells them to watch.
/// The `NOT EXISTS` arm holds those rows out of the sweep entirely, so collected evidence is never
/// destroyed by a retention job. It is deliberately unconditional on age: evidence of a copied
/// credential does not stop being evidence.
///
/// Both timestamps are required past their margin, rather than `chain_expires_at` alone. They can
/// disagree — an operator who raised `AS_REFRESH_TTL_SECONDS` above the chain maximum has tokens
/// outliving their own chain — and requiring both means the sweep waits for whichever says "still
/// live", in either direction.
async fn reap_refresh_tokens(pool: &PgPool) -> ApiResult<i64> {
    let mut total = 0i64;
    loop {
        let deleted = sqlx::query!(
            r#"
            DELETE FROM kb_oauth_refresh_tokens
             WHERE id IN (
                   SELECT t.id
                     FROM kb_oauth_refresh_tokens t
                    WHERE t.expires_at       < now() - make_interval(secs => $1::double precision)
                      AND t.chain_expires_at < now() - make_interval(secs => $1::double precision)
                      AND NOT EXISTS (
                            SELECT 1 FROM kb_oauth_refresh_replays r
                             WHERE r.token_id = t.id
                      )
                    LIMIT $2
             )
            "#,
            REFRESH_FORENSIC_SECONDS,
            BATCH_ROWS
        )
        .execute(pool)
        .await?
        .rows_affected() as i64;

        total += deleted;
        if deleted < BATCH_ROWS || total >= MAX_ROWS_PER_TABLE {
            return Ok(total);
        }
    }
}

#[cfg(test)]
mod window_tests {
    use super::*;

    #[test]
    fn an_unset_window_falls_back_to_the_generous_default() {
        assert_eq!(
            assertion_window_seconds(None).unwrap(),
            DEFAULT_ASSERTION_WINDOW_SECONDS
        );
        assert_eq!(
            assertion_window_seconds(Some("   ".to_string())).unwrap(),
            DEFAULT_ASSERTION_WINDOW_SECONDS
        );
    }

    #[test]
    fn a_configured_window_is_honoured_exactly() {
        assert_eq!(
            assertion_window_seconds(Some("28800".into())).unwrap(),
            28800.0
        );
    }

    // FAILS IF: an unusable value is silently swallowed and the default served instead. That is the
    // failure this floor exists to prevent — an operator who states a window and gets a shorter one
    // has their replay rows deleted early, with nothing anywhere disagreeing.
    #[test]
    fn an_unusable_window_refuses_rather_than_substituting() {
        for raw in ["8h", "0", "-1", "not-a-number", "28800000"] {
            let err = assertion_window_seconds(Some(raw.to_string()))
                .expect_err("must refuse rather than substitute the default");
            assert!(
                matches!(&err, ApiError::Internal(m) if m.contains(ASSERTION_WINDOW_ENV)),
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

    /// A consumed assertion, `age_seconds` old, stamped the way `guardReplay` stamps one: an
    /// `expires_at` of `created + REPLAY_TTL_SECONDS`, where that TTL is the AS's literal 600.
    async fn seed_replay(pool: &PgPool, id: &str, age_seconds: i64) {
        sqlx::query(
            "INSERT INTO kb_saml_replay (assertion_id, expires_at) \
             VALUES ($1, now() - make_interval(secs => $2::double precision) + interval '600 seconds')",
        )
        .bind(id)
        .bind(age_seconds as f64)
        .execute(pool)
        .await
        .expect("seed replay row");
    }

    async fn replay_ids(pool: &PgPool) -> Vec<String> {
        sqlx::query_scalar("SELECT assertion_id FROM kb_saml_replay ORDER BY assertion_id")
            .fetch_all(pool)
            .await
            .expect("read back replay rows")
    }

    /// **The retention floor, asserted in the direction that matters.**
    ///
    /// FAILS IF: the sweep reaps on `expires_at` alone. `recent` is 20 minutes old, so its stamped
    /// `expires_at` (created + 600s) passed ten minutes ago — an `expires_at < now()` predicate
    /// deletes it. But the IdP here issues a one-hour assertion, so it is still replayable, and
    /// deleting it re-opens the exact hole `kb_saml_replay` exists to close.
    #[sqlx::test(migrations = "../../migrations")]
    async fn a_replay_row_inside_the_assertion_window_is_not_deleted(pool: PgPool) {
        let window = 3600.0;

        // Past its stamped expires_at, inside the real assertion window.
        seed_replay(&pool, "inside-window", 1200).await;
        // Past both.
        seed_replay(&pool, "outside-window", 7200).await;
        // Not even past its stamp.
        seed_replay(&pool, "unexpired", 60).await;

        let swept = reap_saml_replay(&pool, window).await.expect("sweep");

        assert_eq!(
            swept, 1,
            "only the row past the assertion window may be swept"
        );
        assert_eq!(
            replay_ids(&pool).await,
            vec!["inside-window".to_string(), "unexpired".to_string()],
            "an assertion still inside the IdP's validity window must still be remembered",
        );
    }

    /// FAILS IF: the floor is read off the row's stamp rather than derived. Widening the configured
    /// window must move the floor for rows ALREADY on disk — that is what makes it survive a
    /// lagging writer still using the 600s literal.
    #[sqlx::test(migrations = "../../migrations")]
    async fn widening_the_configured_window_spares_rows_already_stamped(pool: PgPool) {
        seed_replay(&pool, "two-hours-old", 7200).await;

        let swept = reap_saml_replay(&pool, 10_800.0).await.expect("sweep");
        assert_eq!(
            swept, 0,
            "a 3h window must spare a 2h-old row whatever its stamp says"
        );

        let swept = reap_saml_replay(&pool, 3600.0).await.expect("sweep");
        assert_eq!(swept, 1, "a 1h window reaps the same row");
    }

    /// Seeds a refresh token. `expired` and `chain_dead` are ages in days past the respective
    /// deadlines; negative means still live.
    async fn seed_token(
        pool: &PgPool,
        hash: &str,
        expired_days: i64,
        chain_dead_days: i64,
        rotated: bool,
    ) -> Uuid {
        sqlx::query_scalar(
            "INSERT INTO kb_oauth_refresh_tokens \
                 (token_hash, client_id, claims, expires_at, chain_expires_at, rotated_at, revoked_at) \
             VALUES ($1, 'test-client', '{}'::jsonb, \
                     now() - make_interval(days => $2::int), \
                     now() - make_interval(days => $3::int), \
                     CASE WHEN $4 THEN now() ELSE NULL END, \
                     CASE WHEN $4 THEN now() ELSE NULL END) \
             RETURNING id",
        )
        .bind(hash)
        .bind(expired_days as i32)
        .bind(chain_dead_days as i32)
        .bind(rotated)
        .fetch_one(pool)
        .await
        .expect("seed refresh token")
    }

    async fn token_hashes(pool: &PgPool) -> Vec<String> {
        sqlx::query_scalar("SELECT token_hash FROM kb_oauth_refresh_tokens ORDER BY token_hash")
            .fetch_all(pool)
            .await
            .expect("read back tokens")
    }

    /// **The floor the draft did not know it needed.**
    ///
    /// FAILS IF: the sweep reaps on `expires_at` alone. `live-chain` expired 40 days ago but its
    /// chain runs for another 10 — `findRotatedToken` has no expiry filter, so presenting it today
    /// is still detected as a replay and still ends the chain. Deleting it makes that a silent
    /// "unknown token".
    #[sqlx::test(migrations = "../../migrations")]
    async fn a_spent_token_on_a_live_chain_is_not_deleted(pool: PgPool) {
        // Long expired, chain long dead — reapable.
        seed_token(&pool, "reapable", 400, 400, true).await;
        // Long expired, chain still live — detection still depends on it.
        seed_token(&pool, "live-chain", 40, -10, true).await;
        // Chain long dead, token itself only just expired.
        seed_token(&pool, "fresh-token", 1, 400, false).await;

        let swept = reap_refresh_tokens(&pool).await.expect("sweep");

        assert_eq!(
            swept, 1,
            "only the token past BOTH deadlines by the forensic margin"
        );
        assert_eq!(
            token_hashes(&pool).await,
            vec!["fresh-token".to_string(), "live-chain".to_string()],
            "a token whose chain can still be replayed into must survive the sweep",
        );
    }

    /// **FAILS IF: collected forensics are cascaded away by a retention job.**
    ///
    /// `kb_oauth_refresh_replays.token_id` is `ON DELETE CASCADE`, so deleting the token row takes
    /// the replay record with it — the row `vw_oauth_refresh_replays` shows an operator. This
    /// asserts on the replay table, not on the token table: a sweep that deleted the token would
    /// leave the token assertion satisfiable by a NOT EXISTS bug while silently emptying the
    /// evidence.
    #[sqlx::test(migrations = "../../migrations")]
    async fn a_token_with_recorded_evidence_is_never_swept(pool: PgPool) {
        let token_id = seed_token(&pool, "replayed", 400, 400, true).await;

        sqlx::query(
            "INSERT INTO kb_oauth_refresh_replays \
                 (token_id, chain_id, client_id, rotated_at, first_age_seconds) \
             VALUES ($1, gen_random_uuid(), 'test-client', now() - interval '1 hour', 3600)",
        )
        .bind(token_id)
        .execute(&pool)
        .await
        .expect("seed replay evidence");

        let swept = reap_refresh_tokens(&pool).await.expect("sweep");

        assert_eq!(
            swept, 0,
            "an evidence-bearing token is out of scope however old it is"
        );
        let evidence: i64 =
            sqlx::query_scalar("SELECT count(*) FROM kb_oauth_refresh_replays WHERE token_id = $1")
                .bind(token_id)
                .fetch_one(&pool)
                .await
                .expect("count evidence");
        assert_eq!(
            evidence, 1,
            "the replay record must survive the retention sweep"
        );
    }

    /// FAILS IF: `status` leaks into the flow predicate. An abandoned `pending_saml` flow is as
    /// dead as a consumed one once expired, and keying on status would keep the abandoned ones —
    /// the very rows an abandoned-login backlog is made of — forever.
    #[sqlx::test(migrations = "../../migrations")]
    async fn expired_flows_are_swept_whatever_their_status(pool: PgPool) {
        for (relay, status, age_days) in [
            ("old-pending", "pending_saml", 5),
            ("old-consumed", "consumed", 5),
            ("old-issued", "code_issued", 5),
            ("recent-consumed", "consumed", 0),
        ] {
            sqlx::query(
                "INSERT INTO kb_oauth_flow \
                     (relay_state, status, client_id, redirect_uri, code_challenge, \
                      code_challenge_method, oauth_state, audience, expires_at) \
                 VALUES ($1, $2, 'c', 'https://example.test/cb', 'ch', 'S256', 'st', 'aud', \
                         now() - make_interval(days => $3::int))",
            )
            .bind(relay)
            .bind(status)
            .bind(age_days)
            .execute(&pool)
            .await
            .expect("seed flow");
        }

        let swept = reap_oauth_flows(&pool).await.expect("sweep");

        assert_eq!(
            swept, 3,
            "every flow a day past expiry, regardless of status"
        );
        let survivors: Vec<String> =
            sqlx::query_scalar("SELECT relay_state FROM kb_oauth_flow ORDER BY relay_state")
                .fetch_all(&pool)
                .await
                .expect("read back flows");
        assert_eq!(survivors, vec!["recent-consumed".to_string()]);
    }

    /// FAILS IF: the sweep is not idempotent. A second pass over the same state must delete nothing
    /// and must not error — the cron runs it on a schedule forever.
    #[sqlx::test(migrations = "../../migrations")]
    async fn a_second_sweep_over_the_same_state_deletes_nothing(pool: PgPool) {
        seed_replay(&pool, "ancient", 100_000).await;
        seed_token(&pool, "ancient-token", 400, 400, true).await;

        let first = reap_expired_as_rows(&pool).await.expect("first sweep");
        assert_eq!(first.saml_replay, 1);
        assert_eq!(first.refresh_tokens, 1);
        assert!(!first.more_pending);

        let second = reap_expired_as_rows(&pool).await.expect("second sweep");
        assert_eq!(second, AsReapSummary::default(), "a re-run is a no-op");
    }
}
