//! Whether temper-cloud's fail-open internal calls are reaching this API — and telling an operator
//! when they have stopped.
//!
//! ## The failure this exists for
//!
//! A SAML membership reconcile can fail to happen three ways, and until this module only two of
//! them reached anybody. An error *inside* `/internal/saml/reconcile` is a span in Tempo like any
//! other request. An assertion carrying no group signal is recorded as
//! `kb_saml_principal_reconcile.last_skipped_at` (migration `20260827000030`). The third —
//! **temper-cloud never reaching the endpoint at all** — was swallowed by the fail-open catch at
//! `packages/temper-cloud/src/oauth/endpoints.ts:369-374` into a `logger.error` on a surface whose
//! entire telemetry pipeline is five lines of pino to stdout. De-provisioning could stop for every
//! principal on the deployment with nothing anywhere saying so.
//!
//! ## Why this surface reads it and temper-cloud does not report it
//!
//! **A surface cannot report on itself when it is the broken part.** The failures in question are
//! an outbound HTTPS call from a Vercel function not completing; an OTLP export from that same
//! function is the same shape over the same egress. So temper-cloud writes a fact to Postgres —
//! `kb_internal_call_health`, whose migration carries the reasoning for its shape — from inside the
//! catch that already keeps a failed call from blocking a login, and this side turns that fact into
//! a signal. Postgres is the one dependency proven reachable at the moment of failure: the same
//! request wrote `kb_saml_replay` through the same client two statements earlier.
//!
//! ## Sustained versus transient is decided by the CAUSE, not only by a clock
//!
//! The distinction the task turns on is *"this is weather"* versus *"de-provisioning has stopped"*,
//! and a threshold alone gets it wrong in both directions. Two of the four causes are **deployment
//! facts**: an unset `INTERNAL_RECONCILE_URL` does not heal, and neither does a secret this API
//! disagrees with. One occurrence settles either, and waiting for a second would leave a channel
//! that will never recover sitting in a state named after recovery. The other two — a transport
//! error, a non-2xx that is not an auth refusal — genuinely can be a blip, and must both **recur**
//! and **outlive a window** before they are called sustained.
//!
//! ## What this deliberately does NOT alarm on
//!
//! **Silence.** [`ChannelState::NoAttemptRecorded`] is a state, and it is never sustained. In a
//! login-triggered system the absence of a recorded attempt is indistinguishable from nobody having
//! logged in, so alarming on it would fire on every quiet weekend of every enterprise deployment.
//! That is the same defect `20260827000030` refused when it declined to record an
//! authentication-only IdP as signal-missing: *"a permanent false alarm from a table built to
//! surface real ones."* Every cause enumerated above occurs **during** a login and therefore leaves
//! a positive record, so nothing is lost by refusing to read silence as failure.
//!
//! **The stated cost of that refusal.** A deployment with very little login traffic can hold a
//! weather-capable failure at [`ChannelState::Transient`] indefinitely, because the second
//! occurrence that would settle it has not happened. That is the honest answer — with one login a
//! day there is no evidence distinguishing a broken channel from one flaky call — and it is why
//! `seconds_since_success` and `consecutive_failures` are both emitted: an operator who knows their
//! own traffic can write a tighter rule without a deploy. It is a real limit, not a bug.

use chrono::{DateTime, Utc};
use serde::Serialize;
use sqlx::PgPool;

use crate::error::ApiResult;

/// The channel the SAML membership reconcile call records under. Mirrors `RECONCILE_CHANNEL` in
/// `packages/temper-cloud/src/oauth/reconcile-health.ts`, which is the writer.
pub const RECONCILE_CHANNEL: &str = "saml_reconcile";

/// Every channel this check reports on, whether or not it has a row.
///
/// **Enumerated rather than read from the table**, which is the whole reason this constant exists:
/// a check that reports on the channels it finds cannot report a channel that has never written
/// one. Selecting `FROM kb_internal_call_health` would make a deployment whose reconcile has never
/// once been attempted look identical to one with nothing to say.
pub const WATCHED_CHANNELS: [&str; 1] = [RECONCILE_CHANNEL];

/// Fields every `internal_call_health` span declares unconditionally.
///
/// Pinned to a real exported span by `tests/internal_call_health_span_test.rs`, for the reason
/// [`crate::services::drain_span`] gives about its own two sets: a constant nothing asserts its
/// consumers against prevents no drift at all, and a renamed field silently empties a panel rather
/// than failing anything.
pub const INTERNAL_CALL_HEALTH_FIELDS: [&str; 4] =
    ["channel", "state", "consecutive_failures", "failures_total"];

/// Fields carried only when they have a value, and **absent rather than zero** when they do not.
///
/// Following the lesson `internal/development/drain-operator-queries.md` records against A2:
/// `oldest_pending_age_ms` is recorded only when the queue was non-empty, because writing a zero
/// for "nothing waiting" puts a false floor under every aggregate. The same applies here twice
/// over — a channel that has never succeeded has no `seconds_since_success`, and a zero would read
/// as *succeeded just now*, which is the opposite of the truth.
pub const INTERNAL_CALL_HEALTH_CONDITIONAL_FIELDS: [&str; 4] = [
    "failure_cause",
    "failure_detail",
    "seconds_since_success",
    "failing_for_seconds",
];

/// How long a weather-capable failure run must have lasted before it is called sustained.
///
/// One hour. Not an operator-stated number — unlike `AS_SAML_ASSERTION_MAX_SECONDS`, nothing about
/// a customer's estate determines it — so it is a constant rather than an environment variable, and
/// deliberately: a new fail-closed variable becomes a deploy-time prerequisite, the hazard
/// [`crate::services::as_reap_service`]'s secret reuse exists to avoid. The tunable half is the
/// alert rule, which reads the raw fields this check also emits and needs no deploy to change.
const SUSTAINED_AFTER_SECONDS: i64 = 3_600;

/// How many failures a weather-capable run must reach before it is called sustained.
///
/// Two. One failure is a single event that has not been contradicted or confirmed; the criterion
/// this serves says a single transient failure and a sustained one must be distinguishable, and
/// with a threshold of one they would not be.
const MIN_RECURRENCES: i32 = 2;

/// How old the most recent evidence may be before a verdict is reported as stale rather than
/// believed — one day.
///
/// **Every other state in this module describes a live condition, and none of them decays on its
/// own.** That is a hole in both directions, and it is not hypothetical. An operator paged for
/// `config_missing` who responds by turning group provisioning off stops the reconcile from being
/// attempted at all: no success is ever written, the row keeps its conclusive cause, and the check
/// reports `sustained` every fifteen minutes forever with manual SQL as the only remedy. A rule
/// that pages indefinitely after the operator has acted is worse than no rule, because it teaches
/// them to ignore the one signal this whole mechanism exists to send. The mirror is quieter and
/// worse: a channel that recorded one success and was then switched off reports `healthy` forever
/// while nothing is reconciling.
///
/// So a verdict resting on evidence nobody has refreshed in a day is not a claim about now, and it
/// stops being made. **A day, not an hour**: it must comfortably outlive the gap between logins on
/// a deployment that is genuinely in use, and it must be long enough that a real failure pages for
/// a full day before going quiet — an alert nobody acted on in twenty-four hours is not one that
/// hour seventy-two will rescue. A deployment that logs in less often than daily reads `stale`
/// between logins and re-fires on the next failing one, which is the honest cadence for a channel
/// nobody is exercising.
const STALE_AFTER_SECONDS: i64 = 86_400;

/// One channel's stored health, exactly as the writer left it.
#[derive(Debug, Clone)]
pub struct ChannelHealth {
    pub channel: String,
    pub last_success_at: Option<DateTime<Utc>>,
    pub last_failure_at: Option<DateTime<Utc>>,
    pub failing_since: Option<DateTime<Utc>>,
    pub consecutive_failures: i32,
    pub failures_total: i64,
    pub last_failure_cause: Option<String>,
    pub last_failure_detail: Option<String>,
}

/// What a channel's record amounts to.
///
/// A closed vocabulary rather than a boolean, for [`crate::services::drain_span::JobOutcome`]'s
/// reason: the middle states are neither healthy nor broken, and collapsing them into either
/// misreports one of the two.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ChannelState {
    /// A call on this channel last completed, and none has failed since.
    Healthy,
    /// Failing, but not yet on evidence that distinguishes a blip from a stopped channel.
    Transient,
    /// De-provisioning through this channel has stopped. **The alertable state.**
    Sustained,
    /// Nothing has ever been recorded for this channel. **Never alertable** — see the module doc.
    NoAttemptRecorded,
    /// Something was recorded, but not recently enough to be a claim about now. **Never alertable**,
    /// for [`NoAttemptRecorded`](ChannelState::NoAttemptRecorded)'s reason and one of its own: a
    /// channel nobody is exercising is not a channel that is failing, and the last thing it said is
    /// not evidence about the present. What it is NOT is a clean bill of health — a stale `healthy`
    /// reads here too, deliberately, because an affirmative claim resting on week-old evidence is
    /// the reading that flatters.
    Stale,
}

impl ChannelState {
    /// The wire string. The alert rule matches on these exact values.
    pub fn as_str(&self) -> &'static str {
        match self {
            ChannelState::Healthy => "healthy",
            ChannelState::Transient => "transient",
            ChannelState::Sustained => "sustained",
            ChannelState::NoAttemptRecorded => "no_attempt_recorded",
            ChannelState::Stale => "stale",
        }
    }
}

/// Whether one occurrence of this cause settles the question.
///
/// `config_missing` and `unauthorized` describe the deployment, not the moment: an unset variable
/// and a rejected signature fail identically on the next login and every login after it. They are
/// also the two whose operator action is unambiguous, which is the fourth acceptance criterion —
/// *set this variable* and *the secrets disagree* are different jobs.
///
/// An unrecognized cause is treated as weather-capable. It cannot arrive from the writer — the
/// CHECK on `kb_internal_call_health.last_failure_cause` refuses it — and this arm is the reason
/// that CHECK is worth having: without it a typo would land here and silently soften a conclusive
/// failure into one awaiting a recurrence that has already happened.
fn cause_is_conclusive(cause: Option<&str>) -> bool {
    matches!(cause, Some("config_missing") | Some("unauthorized"))
}

/// Decide what a stored record amounts to, at a given moment.
///
/// Pure and separate from the read so it can be exercised over every case without a database, and
/// so the rule an operator is being alerted on is one function rather than a predicate spread
/// across a query.
pub fn derive_state(row: Option<&ChannelHealth>, now: DateTime<Utc>) -> ChannelState {
    let Some(row) = row else {
        return ChannelState::NoAttemptRecorded;
    };
    // BEFORE anything is believed, ask whether it is still a claim about now. Every state below
    // describes a live condition, and a row is only touched by a login — so a channel that has
    // stopped being exercised freezes whatever it last said and repeats it forever. See
    // STALE_AFTER_SECONDS for the two ways that bites, one loud and one quiet.
    let freshest = row.last_success_at.max(row.last_failure_at);
    if freshest.is_none_or(|at| (now - at).num_seconds() >= STALE_AFTER_SECONDS) {
        return ChannelState::Stale;
    }
    if row.consecutive_failures == 0 {
        return ChannelState::Healthy;
    }
    if cause_is_conclusive(row.last_failure_cause.as_deref()) {
        return ChannelState::Sustained;
    }
    let outlived_window = row
        .failing_since
        .is_some_and(|since| (now - since).num_seconds() >= SUSTAINED_AFTER_SECONDS);
    if row.consecutive_failures >= MIN_RECURRENCES && outlived_window {
        ChannelState::Sustained
    } else {
        ChannelState::Transient
    }
}

/// One channel's verdict plus the numbers behind it.
#[derive(Debug, Clone, Serialize)]
pub struct ChannelReport {
    pub channel: String,
    pub state: ChannelState,
    pub consecutive_failures: i32,
    /// Every failure ever recorded on this channel. Monotonic — a success does not reset it, which
    /// is what makes an intermittent channel visible at all. See the column's own comment.
    pub failures_total: i64,
    pub failure_cause: Option<String>,
    pub failure_detail: Option<String>,
    /// `None` when this channel has never recorded a success — **not zero**, which would read as
    /// *succeeded just now*.
    pub seconds_since_success: Option<i64>,
    /// `None` when the channel is not currently failing.
    pub failing_for_seconds: Option<i64>,
}

/// What one check reported, in the order [`WATCHED_CHANNELS`] names them.
#[derive(Debug, Clone, Serialize)]
pub struct InternalCallHealthSummary {
    pub channels: Vec<ChannelReport>,
    /// True when any channel is [`ChannelState::Sustained`]. The one number a cron caller needs.
    pub any_sustained: bool,
}

/// Read every watched channel, decide its state, and emit one span per channel.
///
/// **Every channel every run, including the healthy ones.** A check that emits only on failure is
/// indistinguishable from a check that is not running — [`crate::services::as_reap_service`] makes
/// the same choice about its all-zero sweep, and here it matters more: the thing being watched for
/// is silence, so a watcher that goes silent to say "fine" is reporting in the same vocabulary as
/// the failure.
pub async fn check_internal_call_health(pool: &PgPool) -> ApiResult<InternalCallHealthSummary> {
    let rows = sqlx::query_as!(
        ChannelHealth,
        r#"SELECT channel,
                  last_success_at      AS "last_success_at: DateTime<Utc>",
                  last_failure_at      AS "last_failure_at: DateTime<Utc>",
                  failing_since        AS "failing_since: DateTime<Utc>",
                  consecutive_failures,
                  failures_total,
                  last_failure_cause,
                  last_failure_detail
             FROM kb_internal_call_health
            WHERE channel = ANY($1)"#,
        &WATCHED_CHANNELS.map(str::to_owned)[..]
    )
    .fetch_all(pool)
    .await?;

    let now = Utc::now();
    let mut channels = Vec::with_capacity(WATCHED_CHANNELS.len());
    for name in WATCHED_CHANNELS {
        let row = rows.iter().find(|r| r.channel == name);
        let state = derive_state(row, now);
        channels.push(ChannelReport {
            channel: name.to_owned(),
            state,
            consecutive_failures: row.map_or(0, |r| r.consecutive_failures),
            failures_total: row.map_or(0, |r| r.failures_total),
            failure_cause: row.and_then(|r| r.last_failure_cause.clone()),
            failure_detail: row.and_then(|r| r.last_failure_detail.clone()),
            seconds_since_success: row
                .and_then(|r| r.last_success_at)
                .map(|at| (now - at).num_seconds()),
            failing_for_seconds: row
                .and_then(|r| r.failing_since)
                .map(|since| (now - since).num_seconds()),
        });
    }

    for report in &channels {
        emit_channel_span(report);
    }

    Ok(InternalCallHealthSummary {
        any_sustained: channels.iter().any(|c| c.state == ChannelState::Sustained),
        channels,
    })
}

/// One `internal_call_health` span per channel, plus an error event for a sustained one.
///
/// `internal` kind, like the drain spans and for the same reason: Tempo derives RED metrics only
/// from `server`/`client` spans, and this is an observation rather than a request boundary. The
/// aggregation route is TraceQL metrics, which reads any span.
fn emit_channel_span(report: &ChannelReport) {
    let span = tracing::info_span!(
        "internal_call_health",
        channel = %report.channel,
        state = report.state.as_str(),
        consecutive_failures = report.consecutive_failures,
        failures_total = report.failures_total,
        failure_cause = tracing::field::Empty,
        failure_detail = tracing::field::Empty,
        seconds_since_success = tracing::field::Empty,
        failing_for_seconds = tracing::field::Empty,
    );
    let _entered = span.enter();
    if let Some(cause) = &report.failure_cause {
        span.record("failure_cause", cause.as_str());
    }
    if let Some(detail) = &report.failure_detail {
        span.record("failure_detail", detail.as_str());
    }
    if let Some(secs) = report.seconds_since_success {
        span.record("seconds_since_success", secs);
    }
    if let Some(secs) = report.failing_for_seconds {
        span.record("failing_for_seconds", secs);
    }

    // A second route to the same fact, for a reader who has logs and not Tempo. Only for the state
    // that means something has stopped — the other three would be noise at this level, and a level
    // that is usually noise is a level nobody reads.
    if report.state == ChannelState::Sustained {
        tracing::error!(
            channel = %report.channel,
            cause = report.failure_cause.as_deref().unwrap_or("unknown"),
            detail = report.failure_detail.as_deref().unwrap_or(""),
            consecutive_failures = report.consecutive_failures,
            "internal call channel has stopped reaching this API — IdP de-provisioning through it \
             is not happening"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;

    fn row(cause: &str, failures: i32, failing_for: Duration, now: DateTime<Utc>) -> ChannelHealth {
        ChannelHealth {
            channel: RECONCILE_CHANNEL.to_owned(),
            last_success_at: None,
            last_failure_at: Some(now),
            failing_since: Some(now - failing_for),
            consecutive_failures: failures,
            failures_total: failures.into(),
            last_failure_cause: Some(cause.to_owned()),
            last_failure_detail: Some("detail".to_owned()),
        }
    }

    #[test]
    fn no_record_is_never_sustained_however_long_the_silence() {
        // The refusal the module doc argues for. If this ever returns Sustained, every quiet
        // weekend on every enterprise deployment pages someone.
        assert_eq!(
            derive_state(None, Utc::now()),
            ChannelState::NoAttemptRecorded
        );
    }

    #[test]
    fn a_deployment_fact_is_sustained_on_its_first_occurrence() {
        let now = Utc::now();
        for cause in ["config_missing", "unauthorized"] {
            assert_eq!(
                derive_state(Some(&row(cause, 1, Duration::seconds(1), now)), now),
                ChannelState::Sustained,
                "{cause} does not heal, so one occurrence settles it"
            );
        }
    }

    #[test]
    fn one_weather_failure_is_transient_however_old() {
        let now = Utc::now();
        for cause in ["transport", "endpoint_error"] {
            assert_eq!(
                derive_state(Some(&row(cause, 1, Duration::days(30), now)), now),
                ChannelState::Transient,
                "{cause} that never recurred is one event, not a stopped channel"
            );
        }
    }

    #[test]
    fn recurring_weather_inside_the_window_is_still_transient() {
        let now = Utc::now();
        let inside = Duration::seconds(SUSTAINED_AFTER_SECONDS - 1);
        assert_eq!(
            derive_state(Some(&row("transport", 9, inside, now)), now),
            ChannelState::Transient,
        );
    }

    #[test]
    fn recurring_weather_past_the_window_is_sustained() {
        let now = Utc::now();
        let outside = Duration::seconds(SUSTAINED_AFTER_SECONDS);
        assert_eq!(
            derive_state(Some(&row("transport", MIN_RECURRENCES, outside, now)), now),
            ChannelState::Sustained,
        );
    }

    #[test]
    fn a_success_makes_the_channel_healthy_whatever_it_last_failed_with() {
        // The forensic columns survive a success by design, so the reader must key on the run and
        // not on their presence — otherwise a channel is reported broken forever after one blip.
        let now = Utc::now();
        let mut recovered = row("config_missing", 0, Duration::seconds(0), now);
        recovered.failing_since = None;
        recovered.last_success_at = Some(now);
        assert_eq!(derive_state(Some(&recovered), now), ChannelState::Healthy);
    }

    /// **A verdict is a claim about now, and a row is only touched by a login.**
    ///
    /// The loud half of what STALE_AFTER_SECONDS exists for: an operator paged for
    /// `config_missing` who responds by turning group provisioning off stops the reconcile being
    /// attempted at all. No success is ever written, the conclusive cause stays on the row, and
    /// without this the check would report `sustained` every fifteen minutes forever — a critical
    /// page that survives the fix, with manual SQL as the only remedy.
    #[test]
    fn a_sustained_verdict_nobody_has_refreshed_stops_being_made() {
        let now = Utc::now();
        let old = row("config_missing", 1, Duration::seconds(1), now);
        let stale = ChannelHealth {
            last_failure_at: Some(now - Duration::days(2)),
            failing_since: Some(now - Duration::days(2)),
            ..old
        };
        assert_eq!(derive_state(Some(&stale), now), ChannelState::Stale);
    }

    /// The quiet half, and the one that flatters.
    ///
    /// A channel that recorded a success and was then switched off would otherwise report
    /// `healthy` forever while nothing reconciles at all — an affirmative claim resting on
    /// week-old evidence. Stale is not a clean bill of health, and this is the difference.
    #[test]
    fn a_healthy_verdict_nobody_has_refreshed_is_not_a_clean_bill_of_health() {
        let now = Utc::now();
        let stale = ChannelHealth {
            channel: RECONCILE_CHANNEL.to_owned(),
            last_success_at: Some(now - Duration::days(2)),
            last_failure_at: None,
            failing_since: None,
            consecutive_failures: 0,
            failures_total: 0,
            last_failure_cause: None,
            last_failure_detail: None,
        };
        assert_eq!(derive_state(Some(&stale), now), ChannelState::Stale);
    }

    /// The boundary in the other direction: evidence inside the window is still believed, so a
    /// real failure is not talked out of existence by a channel that is being exercised.
    #[test]
    fn evidence_inside_the_window_is_still_a_verdict() {
        let now = Utc::now();
        let fresh = row(
            "config_missing",
            1,
            Duration::seconds(STALE_AFTER_SECONDS - 1),
            now,
        );
        assert_eq!(derive_state(Some(&fresh), now), ChannelState::Sustained);
    }

    #[test]
    fn an_unrecognized_cause_falls_to_the_weather_branch() {
        // Unreachable through the CHECK, and asserted anyway: this is the arm that decides whether
        // the database constraint is load-bearing or decoration. It is load-bearing.
        let now = Utc::now();
        assert_eq!(
            derive_state(Some(&row("gremlins", 1, Duration::days(30), now)), now),
            ChannelState::Transient,
        );
    }
}
