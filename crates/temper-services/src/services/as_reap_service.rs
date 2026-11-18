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
    /// True when any table stopped because it hit its per-run cap rather than because it ran
    /// out of rows — there is more to delete and the next scheduled pass will take it.
    ///
    /// Carried out of each sweep's own exit, not inferred by comparing its count to the cap.
    /// Those two differ at exactly one point, and it is a reachable one: a table holding
    /// precisely `MAX_ROWS_PER_TABLE` eligible rows drains completely on the last batch, and a
    /// count comparison would still report more to come.
    pub more_pending: bool,
}

/// What one table's sweep deleted, and whether it stopped early because of the per-run cap.
struct Swept {
    rows: i64,
    hit_cap: bool,
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
        saml_replay: saml_replay.rows,
        oauth_flow: oauth_flow.rows,
        refresh_tokens: refresh_tokens.rows,
        more_pending: saml_replay.hit_cap || oauth_flow.hit_cap || refresh_tokens.hit_cap,
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
/// (`packages/temper-cloud/src/saml/config.ts`) pins the window shape — `acceptedClockSkewMs` and
/// `maxAssertionAgeMs`, both 0, so the IdP's `NotOnOrAfter` is the expiry authority — but the
/// IDP-ISSUED WINDOW ITSELF is still not code anywhere, and `guardReplay` is handed
/// `now + REPLAY_TTL_SECONDS` where that constant is the literal `600`.
/// So `expires_at` on this table is a GUESS at the window, not a statement of it, and reaping on
/// `expires_at` alone would forget a still-replayable assertion for any IdP issuing a window wider
/// than ten minutes. Widening the pinned skew widens presentability with it, and must be reflected
/// in `AS_SAML_ASSERTION_MAX_SECONDS` — the coupling test (`tests/saml/config.test.ts`) binds the
/// temper-cloud half of that arithmetic.
///
/// **So the window is subtracted rather than trusted.** The predicate is
/// `expires_at < now() - window`, and because `expires_at` is stamped at or after the row was
/// consumed, no row can be deleted before `consumed + window` **whatever the writer stamped on
/// it**. That is what makes the floor derived rather than hard-coded, and it holds without the AS
/// being changed at all: rows already on disk, and rows written by a lagging binary carrying the
/// old literal, are covered by the same arithmetic as rows written by a current one.
///
/// **The two clocks are not the same clock**, and the guarantee is stated with that in rather than
/// asserted around it. `expires_at` is computed on the AS host (`Date.now()` at the `guardReplay`
/// call site) while `now()` here is the database's, so host/DB skew shifts the floor by that skew
/// — the same caveat `flow.ts` already names for `chain_expires_at`. The stamped TTL is the
/// margin absorbing it: the floor is `window` past a stamp that is itself `REPLAY_TTL_SECONDS`
/// past consumption, so the inequality survives up to that much clock disagreement.
///
/// The direction of the existing bug is preserved deliberately. Unbounded retention was **fail-safe
/// for replay** — a consumed assertion id was never forgotten, so the guard never weakened. This
/// bounds the growth without inverting that.
async fn reap_saml_replay(pool: &PgPool, window_seconds: f64) -> ApiResult<Swept> {
    let mut total = 0i64;
    loop {
        // Two choices here, and both were measured rather than reasoned about.
        //
        // No ORDER BY: on a table that has never been reaped almost every row matches, so an
        // unordered LIMIT lets the scan stop as soon as it has a batch (161 buffers, ~2 ms) where
        // an ORDER BY would sort the whole backlog first. In steady state, when nothing matches,
        // it is the `expires_at` index that keeps the pass cheap instead.
        //
        // `ctid`, not the primary key: handing the DELETE ids made it re-find them, and the plan
        // for that was a join whose outer side sequentially scanned the entire table — 13,334
        // buffers to delete 5,000 rows the subquery had already located. `ctid` makes the outer
        // node a Tid Scan straight to the rows. A ctid can go stale under a concurrent UPDATE, in
        // which case the row is simply missed and swept next run; for a reaper that is the
        // harmless direction.
        let deleted = sqlx::query!(
            r#"
            DELETE FROM kb_saml_replay
             WHERE ctid IN (
                   SELECT ctid
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
        // Two different reasons to stop, and the caller needs to tell them apart: a short batch
        // means the table is drained, the cap means there is more waiting for tomorrow.
        if deleted < BATCH_ROWS {
            return Ok(Swept {
                rows: total,
                hit_cap: false,
            });
        }
        if total >= MAX_ROWS_PER_TABLE {
            return Ok(Swept {
                rows: total,
                hit_cap: true,
            });
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
async fn reap_oauth_flows(pool: &PgPool) -> ApiResult<Swept> {
    let mut total = 0i64;
    loop {
        let deleted = sqlx::query!(
            r#"
            DELETE FROM kb_oauth_flow
             WHERE ctid IN (
                   SELECT ctid
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
        // Two different reasons to stop, and the caller needs to tell them apart: a short batch
        // means the table is drained, the cap means there is more waiting for tomorrow.
        if deleted < BATCH_ROWS {
            return Ok(Swept {
                rows: total,
                hit_cap: false,
            });
        }
        if total >= MAX_ROWS_PER_TABLE {
            return Ok(Swept {
                rows: total,
                hit_cap: true,
            });
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
/// (`packages/temper-cloud/src/oauth/flow.ts:349`) matches the presented `token_hash` and
/// `rotated_at IS NOT NULL` — and **carries no expiry filter of any kind** — while
/// `judgeRefusedRotation` runs after *any* refused rotation, expiry included. A stolen token presented after its own TTL is therefore recognised as a replay today,
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
/// Both timestamps are required past their margin, rather than `chain_expires_at` alone. Today
/// that is redundant: `storeRefreshToken` stamps `expires_at` as `LEAST(requested,
/// chain_expires_at)` (`flow.ts:200`), so a token cannot outlive its own chain and the chain arm
/// always decides. It is kept because the redundancy is free and points the safe way — if that
/// `LEAST` is ever relaxed, this predicate waits for whichever bound still says "live" rather than
/// silently reaping against the shorter one.
async fn reap_refresh_tokens(pool: &PgPool) -> ApiResult<Swept> {
    let mut total = 0i64;
    loop {
        let deleted = sqlx::query!(
            r#"
            DELETE FROM kb_oauth_refresh_tokens
             WHERE ctid IN (
                   SELECT t.ctid
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
        // Two different reasons to stop, and the caller needs to tell them apart: a short batch
        // means the table is drained, the cap means there is more waiting for tomorrow.
        if deleted < BATCH_ROWS {
            return Ok(Swept {
                rows: total,
                hit_cap: false,
            });
        }
        if total >= MAX_ROWS_PER_TABLE {
            return Ok(Swept {
                rows: total,
                hit_cap: true,
            });
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
mod tests;
