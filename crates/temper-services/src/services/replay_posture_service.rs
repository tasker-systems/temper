//! The startup check that a deployed database is not a replay target.
//!
//! The guards that keep erasure and the ledger honest stand down only under `_replay_mode()`
//! (migration 20261203000010): `temper.replaying` on AND a `kb_replay_target` row, which only a
//! replay walk's scratch database holds. `replay_posture()` names what would make a deployed
//! database unsafe: the marker, a database or role default for the setting, or the session
//! carrying it.
//!
//! The deploy's hard stop is temper-migrate, which refuses (exit 5) on any reason. This logs the
//! same reasons at API startup, so a database that gained one after its deploy is on the record.
//! It never blocks boot: without the marker the guards stay up, so a reason here is a state to
//! investigate rather than one the guards depend on.

use sqlx::PgPool;

/// Read `replay_posture()` once and log every reason at error level. A query error is a warning.
/// Returns the reasons read (empty when safe, or when the check could not run), for tests.
///
/// The connection comes from the pool outside any transaction, so a session-level setting on it
/// is reported; one a transaction-mode pooler carries on another server connection is not
/// visible from here, and lowers no guard without the marker.
pub async fn check(pool: &PgPool) -> Vec<String> {
    let reasons = match sqlx::query_scalar!(r#"SELECT replay_posture() AS "reasons!""#)
        .fetch_one(pool)
        .await
    {
        Ok(reasons) => reasons,
        Err(e) => {
            tracing::warn!(error = %e, "replay posture check: could not read replay_posture(); skipped");
            return Vec::new();
        }
    };
    for reason in &reasons {
        tracing::error!(
            reason = %reason,
            "replay posture check: this database is unsafe to serve ({reason}); the replay \
             guards stand down only for a replay walk's scratch database"
        );
    }
    reasons
}
