#![cfg(feature = "test-db")]
//! The read half of the reconcile-channel health check, against a real database.
//!
//! The rule that turns a stored record into a verdict is exercised without a database in
//! `internal_call_health_service`'s own module tests, because it is pure and every case is worth
//! having. What only a database can show is the part those cannot: that the check reports on the
//! channels it WATCHES rather than the channels it FINDS, and that a stored row reaches the verdict
//! with its numbers intact.

use sqlx::PgPool;
use temper_services::services::internal_call_health_service::{
    check_internal_call_health, ChannelState, RECONCILE_CHANNEL, WATCHED_CHANNELS,
};

/// **The property that makes this check able to report on silence at all.**
///
/// A deployment whose reconcile has never once been attempted has no row. If the check selected
/// its channels from the table it would return an empty list — indistinguishable from a healthy
/// deployment, and from a check that is not running. Enumerating [`WATCHED_CHANNELS`] is what makes
/// the absence of a row into a reported state rather than an absent one.
///
/// The state it reports is deliberately NOT alertable. In a login-triggered system, no recorded
/// attempt is indistinguishable from nobody having logged in, and alarming on it would fire on
/// every quiet weekend. Reported, and never paged on.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn a_channel_that_has_never_been_attempted_is_reported_not_omitted(pool: PgPool) {
    let summary = check_internal_call_health(&pool).await.expect("check runs");

    assert_eq!(
        summary.channels.len(),
        WATCHED_CHANNELS.len(),
        "every watched channel is reported, whether or not it has a row"
    );
    let reconcile = &summary.channels[0];
    assert_eq!(reconcile.channel, RECONCILE_CHANNEL);
    assert_eq!(reconcile.state, ChannelState::NoAttemptRecorded);
    assert!(
        !summary.any_sustained,
        "silence must never raise the alertable flag"
    );
    // Absent, not zero. A `seconds_since_success` of 0 would read as *succeeded just now*, which is
    // the opposite of the truth — the same lesson `oldest_pending_age_ms` records for the drains.
    assert_eq!(reconcile.seconds_since_success, None);
    assert_eq!(reconcile.failing_for_seconds, None);
}

/// A stored deployment fact reaches the verdict as sustained, on one occurrence, with the detail
/// that names the operator's action.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn a_stored_config_failure_is_sustained_with_the_variable_named(pool: PgPool) {
    sqlx::query(
        "INSERT INTO kb_internal_call_health
             (channel, last_failure_at, failing_since, consecutive_failures, failures_total,
              last_failure_cause, last_failure_detail)
         VALUES ($1, now(), now(), 1, 1, 'config_missing', 'INTERNAL_RECONCILE_URL')",
    )
    .bind(RECONCILE_CHANNEL)
    .execute(&pool)
    .await
    .expect("seed the failure");

    let summary = check_internal_call_health(&pool).await.expect("check runs");
    let reconcile = &summary.channels[0];

    assert_eq!(reconcile.state, ChannelState::Sustained);
    assert!(summary.any_sustained);
    assert_eq!(reconcile.consecutive_failures, 1);
    assert_eq!(reconcile.failure_cause.as_deref(), Some("config_missing"));
    assert_eq!(
        reconcile.failure_detail.as_deref(),
        Some("INTERNAL_RECONCILE_URL"),
        "the signal must say WHICH variable, or the operator learns only that something is wrong"
    );
    assert!(reconcile.failing_for_seconds.is_some());
}

/// A recent, un-recurred weather failure is reported and does NOT raise the alertable flag.
///
/// The second acceptance criterion, asserted where it can fail: a single transient failure and a
/// sustained one must be distinguishable, and the one that means de-provisioning has stopped is the
/// second. A check that paged on this would page on every deploy blip.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn a_single_recent_transport_failure_does_not_raise_the_alert(pool: PgPool) {
    sqlx::query(
        "INSERT INTO kb_internal_call_health
             (channel, last_failure_at, failing_since, consecutive_failures, failures_total,
              last_failure_cause, last_failure_detail)
         VALUES ($1, now(), now(), 1, 1, 'transport', 'TypeError')",
    )
    .bind(RECONCILE_CHANNEL)
    .execute(&pool)
    .await
    .expect("seed the failure");

    let summary = check_internal_call_health(&pool).await.expect("check runs");

    assert_eq!(summary.channels[0].state, ChannelState::Transient);
    assert!(!summary.any_sustained);
}

/// A verdict nobody has refreshed stops being made — read through the real query, not just the rule.
///
/// The scenario is ordinary: an operator paged for `config_missing` turns group provisioning off.
/// Nothing then attempts the reconcile, so no success is ever written and the conclusive cause sits
/// on the row forever. Without the staleness bound this returns `sustained` on every tick, and the
/// critical alert survives the fix that resolved it.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn a_verdict_nobody_has_refreshed_is_reported_stale_and_does_not_alert(pool: PgPool) {
    sqlx::query(
        "INSERT INTO kb_internal_call_health
             (channel, last_failure_at, failing_since, consecutive_failures, failures_total,
              last_failure_cause, last_failure_detail)
         VALUES ($1, now() - interval '3 days', now() - interval '3 days', 6, 6,
                 'config_missing', 'INTERNAL_RECONCILE_URL')",
    )
    .bind(RECONCILE_CHANNEL)
    .execute(&pool)
    .await
    .expect("seed a frozen failure");

    let summary = check_internal_call_health(&pool).await.expect("check runs");

    assert_eq!(summary.channels[0].state, ChannelState::Stale);
    assert!(
        !summary.any_sustained,
        "a critical page must not outlive the evidence for it — this is the alert surviving the \
         fix that resolved it"
    );
    // Stale is not amnesia: what it last failed with is still readable, which is what an operator
    // arriving after the fact actually needs.
    assert_eq!(
        summary.channels[0].failure_cause.as_deref(),
        Some("config_missing")
    );
    assert_eq!(summary.channels[0].failures_total, 6);
}
