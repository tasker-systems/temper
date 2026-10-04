//! The sensitivity sweep's door: one cron call, a loop of claimed ticks (sensitivity-sweep spec D8,
//! D9; build order 3a PR D; Q44–Q48).
//!
//! ## Two statements per tick, never one transaction
//!
//! `sensitivity_sweep_claim` commits the claimed job and the run row on its own. That commit is
//! what lets a tick that dies part-way keep its attempt and leave a run with no outcome behind as
//! the record of it (`migrations/20261002200000_sensitivity_scan.sql`). Wrapping the two calls in
//! one transaction would roll the run back with the failure, and the attempt would vanish.
//!
//! ## One call loops (Q46)
//!
//! A tick is bounded by its row budget, not its time: about 3 s against the function's 300 s. One
//! tick per call gave each surface one tick an hour. So a call claims and ticks again until
//! [`LOOP_BUDGET`] is spent, and stops early after a whole rotation in which no tick examined a row,
//! so an idle corpus costs one rotation per call, not hundreds of empty runs. The single-flight
//! slot keeps it to one scanning backend.
//!
//! ## Nothing crosses the wire
//!
//! The tick's `RETURNS TABLE` holds integers, a boolean and nothing else (D8, Q45). Its outcome and
//! failure come back as numbers and are named here from a closed vocabulary, so no string read from
//! the corpus reaches a span, a log line or the response. The response carries counts of ticks and
//! booleans only: the per-tick counts travel on the span, where no caller of the route reads them.
//! The service names the public functions and the queue table, never the schema behind them:
//! that is witness 12's grep gate.
//!
//! ## Off until an operator opts in (Q52, Q53)
//!
//! A deployment whose operator has not set `SENSITIVITY_SWEEP_ENABLED` gets a door that answers
//! and does nothing else: no reap, no claim, no tick, no row, and none of the error events below.
//! The cron entry stays in `vercel.json` on every deployment, so the variable is the opt-in.
//!
//! ## Loud, not quiet
//!
//! Every state in which the sweep is not scanning raises an error event: an unset salt (Q44), a
//! salt the server may be logging (Q47), a failed or errored tick, a lapsed lease, and a failed
//! job holding the single-flight slot, which stalls every surface while it backs off.

use std::time::{Duration, Instant};

use serde::Serialize;
use sqlx::{PgConnection, PgPool};
use uuid::Uuid;

use crate::error::ApiResult;
use crate::services::workflow_job_service;

/// How long one call keeps claiming. The function's ceiling is 300 s; this leaves room for the tick
/// in flight when the budget runs out.
pub const LOOP_BUDGET: Duration = Duration::from_secs(240);

/// The time a tick may need: its own 20 s budget (`sensitivity_sweep_tick`'s `p_budget_ms`) and a
/// margin. A call does not start a tick it cannot finish inside its budget.
const TICK_HEADROOM: Duration = Duration::from_secs(25);

/// Fields every `sensitivity_sweep` (per-tick) span declares.
pub const SENSITIVITY_SWEEP_FIELDS: [&str; 4] = ["run_id", "job_id", "ticked", "errored"];

/// Fields a tick span carries only when the tick returned: absent, not zero, otherwise.
pub const SENSITIVITY_SWEEP_RUN_FIELDS: [&str; 15] = [
    "outcome",
    "failed",
    "rows_examined",
    "hashes_examined",
    "cache_hits",
    "new_findings",
    "new_findings_head",
    "new_findings_backfill",
    "cursor_advances",
    "units_oversize",
    "head_holdback_seconds",
    "sev1",
    "sev2",
    "sev3",
    "sev4",
];

/// Fields every `sensitivity_sweep_call` span declares.
pub const SENSITIVITY_SWEEP_CALL_FIELDS: [&str; 5] = [
    "enabled",
    "salt_configured",
    "salt_may_be_logged",
    "ticks",
    "ended",
];

/// How a tick ended. Numbered in `sensitivity_sweep_tick` (Q45).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TickOutcome {
    Scanned,
    Idle,
    Failed,
    /// A number this binary does not know: a newer migration than the code.
    Unrecognised,
}

impl TickOutcome {
    pub fn from_code(code: Option<i16>) -> Self {
        match code {
            Some(1) => Self::Scanned,
            Some(2) => Self::Idle,
            Some(3) => Self::Failed,
            _ => Self::Unrecognised,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Scanned => "scanned",
            Self::Idle => "idle",
            Self::Failed => "failed",
            Self::Unrecognised => "unrecognised",
        }
    }
}

/// Why a tick or its job failed. The closed set `workflow_jobs_sensitivity_error_coded` admits into
/// `last_error`; the tick numbers all but the reaper's `lease expired`, and a test holds the
/// numbering equal to the tick's.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TickFailure {
    SaltMissing,
    StatementTimeout,
    DetectorPatternInvalid,
    StoreConstraint,
    ScanFailed,
    /// The reaper's code for a lease that lapsed: never returned by the tick.
    LeaseExpired,
    /// A value this binary does not know: a newer migration than the code.
    Unrecognised,
}

impl TickFailure {
    /// Every code the tick can return, with its number.
    pub const CODED: [(i16, Self); 5] = [
        (1, Self::SaltMissing),
        (2, Self::StatementTimeout),
        (3, Self::DetectorPatternInvalid),
        (4, Self::StoreConstraint),
        (5, Self::ScanFailed),
    ];

    pub fn from_code(code: i16) -> Self {
        Self::CODED
            .iter()
            .find(|(n, _)| *n == code)
            .map_or(Self::Unrecognised, |(_, f)| *f)
    }

    /// Names a job's coded `last_error`. The trigger keeps it to the closed set, and anything else
    /// is named `unrecognised`, never echoed.
    pub fn from_last_error(code: &str) -> Self {
        match code {
            "lease expired" => Self::LeaseExpired,
            other => Self::CODED
                .iter()
                .find(|(_, f)| f.as_str() == other)
                .map_or(Self::Unrecognised, |(_, f)| *f),
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::SaltMissing => "salt_missing",
            Self::StatementTimeout => "statement_timeout",
            Self::DetectorPatternInvalid => "detector_pattern_invalid",
            Self::StoreConstraint => "store_constraint",
            Self::ScanFailed => "scan_failed",
            Self::LeaseExpired => "lease_expired",
            Self::Unrecognised => "unrecognised",
        }
    }
}

/// One tick's facts: counts and ids, never text (D8).
#[derive(Debug, Clone)]
pub struct TickReport {
    pub run_id: Uuid,
    pub job_id: Uuid,
    pub outcome: TickOutcome,
    pub failed: bool,
    pub failure: Option<TickFailure>,
    pub rows_examined: i32,
    pub hashes_examined: i32,
    pub cache_hits: i32,
    pub new_findings: i32,
    pub new_findings_head: i32,
    pub new_findings_backfill: i32,
    pub cursor_advances: i32,
    pub units_oversize: i32,
    pub head_holdback_seconds: i32,
    pub sev1: i32,
    pub sev2: i32,
    pub sev3: i32,
    pub sev4: i32,
}

/// The single-flight slot when a claim found nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Slot {
    /// No sensitivity job is in flight: nothing was enabled to enqueue.
    Free,
    /// Another call's tick holds the lease.
    Leased,
    /// A failed job is backing off, and every surface waits behind it.
    Blocked {
        attempts: i32,
        failure: Option<TickFailure>,
    },
}

/// Why a call stopped claiming.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ended {
    /// The deployment has not opted in: nothing was reaped, claimed or ticked.
    Disabled,
    /// The loop's time budget ran out.
    Budget,
    /// A whole rotation examined no row.
    Idle,
    /// The claim found nothing; the slot says why.
    Unclaimed(Slot),
    /// A claim's lease lapsed before its tick read it.
    LeaseLapsed,
}

impl Ended {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Disabled => "disabled",
            Self::Budget => "budget",
            Self::Idle => "idle",
            Self::Unclaimed(Slot::Free) => "slot_free",
            Self::Unclaimed(Slot::Leased) => "slot_leased",
            Self::Unclaimed(Slot::Blocked { .. }) => "slot_blocked",
            Self::LeaseLapsed => "lease_lapsed",
        }
    }
}

/// What one call of the door did.
#[derive(Debug, Clone)]
pub struct SweepSummary {
    pub salt_configured: bool,
    /// The server's logging settings may write the salt bind parameter to its log (Q47). Not read,
    /// and false, when the deployment has not opted in.
    pub salt_may_be_logged: bool,
    pub ticks: Vec<TickReport>,
    pub ended: Ended,
}

/// The route's answer: counts of ticks and booleans, nothing a caller can learn the corpus from.
#[derive(Debug, Clone, Serialize)]
pub struct SweepAnswer {
    /// Whether the deployment has opted in. False means the call did nothing at all.
    pub enabled: bool,
    pub ticks: u32,
    pub failed_ticks: u32,
    pub salt_configured: bool,
    pub slot_blocked: bool,
}

impl SweepSummary {
    pub fn answer(&self) -> SweepAnswer {
        SweepAnswer {
            enabled: !matches!(self.ended, Ended::Disabled),
            ticks: self.ticks.len() as u32,
            failed_ticks: self.ticks.iter().filter(|t| t.failed).count() as u32,
            salt_configured: self.salt_configured,
            slot_blocked: matches!(self.ended, Ended::Unclaimed(Slot::Blocked { .. })),
        }
    }
}

struct Claim {
    run_id: Uuid,
    job_id: Uuid,
    surfaces: i32,
}

/// One call of the door, with the deployed loop budget. `enabled` is the deployment's opt-in:
/// false touches nothing in the database and raises no error event, only the call span.
pub async fn sweep(pool: &PgPool, salt: Option<&[u8]>, enabled: bool) -> ApiResult<SweepSummary> {
    if !enabled {
        let summary = SweepSummary {
            salt_configured: salt.is_some(),
            salt_may_be_logged: false,
            ticks: Vec::new(),
            ended: Ended::Disabled,
        };
        emit_call_span(&summary);
        return Ok(summary);
    }
    sweep_within(pool, salt, LOOP_BUDGET).await
}

/// One call of the door: reap, then claim and tick until `budget` is spent or a rotation is idle.
///
/// `salt` is passed through even when absent: the tick then records the `salt_missing` run, which
/// is the durable half of the signal. The spans and the error events are the other half.
pub async fn sweep_within(
    pool: &PgPool,
    salt: Option<&[u8]>,
    budget: Duration,
) -> ApiResult<SweepSummary> {
    let started = Instant::now();
    // The incumbent drains reap before they claim (`embed_service::dispatch`): a lapsed lease
    // holds the single-flight slot until something does.
    workflow_job_service::reap(pool, "sensitivity lease expired").await?;

    let mut conn = pool.acquire().await?;
    let salt_may_be_logged = guard_the_salt(&mut conn).await?;
    let mut summary = SweepSummary {
        salt_configured: salt.is_some(),
        salt_may_be_logged,
        ticks: Vec::new(),
        ended: Ended::Budget,
    };

    let mut idle_streak = 0;
    while started.elapsed() + TICK_HEADROOM <= budget {
        let Some(claim) = claim(&mut conn).await? else {
            summary.ended = Ended::Unclaimed(slot(&mut conn).await?);
            break;
        };
        let report = match tick(&mut conn, &claim, salt).await {
            Ok(report) => report,
            Err(e) => {
                emit_tick_span(&claim, None, true);
                emit_call_span(&summary);
                return Err(e);
            }
        };
        emit_tick_span(&claim, report.as_ref(), false);
        let Some(report) = report else {
            summary.ended = Ended::LeaseLapsed;
            break;
        };
        idle_streak = if report.rows_examined == 0 {
            idle_streak + 1
        } else {
            0
        };
        summary.ticks.push(report);
        if idle_streak >= claim.surfaces.max(1) {
            summary.ended = Ended::Idle;
            break;
        }
    }
    emit_call_span(&summary);
    Ok(summary)
}

/// Q47. The user-settable half is closed on this connection: a statement that errors no longer logs
/// its parameters. The rest needs a privilege this role need not hold, so it is read and reported:
/// true when the server's settings could write a bind parameter (the salt) to its log.
async fn guard_the_salt(conn: &mut PgConnection) -> ApiResult<bool> {
    sqlx::query_scalar!(
        r#"SELECT set_config('log_parameter_max_length_on_error', '0', false) AS "set!""#
    )
    .fetch_one(&mut *conn)
    .await?;
    let may_log = sqlx::query_scalar!(
        r#"
        WITH s AS (SELECT name, setting FROM pg_settings
                    WHERE name IN ('log_parameter_max_length', 'log_statement',
                                   'log_min_duration_statement', 'log_min_duration_sample'))
        -- `setting` is unitless; current_setting() answers '1s', which no cast reads.
        SELECT (SELECT setting::int FROM s WHERE name = 'log_parameter_max_length') <> 0
           AND ((SELECT setting FROM s WHERE name = 'log_statement') = 'all'
                OR (SELECT setting::int FROM s WHERE name = 'log_min_duration_statement') >= 0
                OR (SELECT setting::int FROM s WHERE name = 'log_min_duration_sample') >= 0)
            AS "may_log!"
        "#
    )
    .fetch_one(&mut *conn)
    .await?;
    Ok(may_log)
}

async fn claim(conn: &mut PgConnection) -> ApiResult<Option<Claim>> {
    let claim = sqlx::query_as!(
        Claim,
        r#"SELECT run_id AS "run_id!", job_id AS "job_id!", surfaces AS "surfaces!"
             FROM sensitivity_sweep_claim()"#
    )
    .fetch_optional(&mut *conn)
    .await?;
    Ok(claim)
}

/// Why the claim found nothing: the sensitivity job holding the single-flight slot, if any.
async fn slot(conn: &mut PgConnection) -> ApiResult<Slot> {
    let held = sqlx::query!(
        r#"
        SELECT status, attempts, last_error
          FROM kb_workflow_jobs
         WHERE persona = 'sensitivity'
           AND status IN ('pending', 'in_progress', 'waiting_for_retry')
         LIMIT 1
        "#
    )
    .fetch_optional(&mut *conn)
    .await?;
    Ok(match held {
        None => Slot::Free,
        Some(j) if j.status == "waiting_for_retry" => Slot::Blocked {
            attempts: j.attempts,
            failure: j.last_error.as_deref().map(TickFailure::from_last_error),
        },
        Some(_) => Slot::Leased,
    })
}

async fn tick(
    conn: &mut PgConnection,
    claim: &Claim,
    salt: Option<&[u8]>,
) -> ApiResult<Option<TickReport>> {
    let row = sqlx::query!(
        r#"
        SELECT rows_examined AS "rows_examined!", hashes_examined AS "hashes_examined!",
               cache_hits AS "cache_hits!", new_findings AS "new_findings!",
               cursor_advances AS "cursor_advances!", units_oversize AS "units_oversize!",
               failed AS "failed!", outcome, failure,
               new_findings_head AS "new_findings_head!",
               new_findings_backfill AS "new_findings_backfill!",
               head_holdback_seconds AS "head_holdback_seconds!",
               sev1 AS "sev1!", sev2 AS "sev2!", sev3 AS "sev3!", sev4 AS "sev4!"
          FROM sensitivity_sweep_tick($1, $2, $3)
        "#,
        claim.run_id,
        claim.job_id,
        salt,
    )
    .fetch_optional(&mut *conn)
    .await?;

    Ok(row.map(|r| TickReport {
        run_id: claim.run_id,
        job_id: claim.job_id,
        outcome: TickOutcome::from_code(r.outcome),
        failed: r.failed,
        failure: r.failure.map(TickFailure::from_code),
        rows_examined: r.rows_examined,
        hashes_examined: r.hashes_examined,
        cache_hits: r.cache_hits,
        new_findings: r.new_findings,
        new_findings_head: r.new_findings_head,
        new_findings_backfill: r.new_findings_backfill,
        cursor_advances: r.cursor_advances,
        units_oversize: r.units_oversize,
        head_holdback_seconds: r.head_holdback_seconds,
        sev1: r.sev1,
        sev2: r.sev2,
        sev3: r.sev3,
        sev4: r.sev4,
    }))
}

/// One `sensitivity_sweep` span per claim, plus an error event when the tick did not scan.
///
/// `internal` kind, like `internal_call_health`: an observation, not a request boundary. Alert
/// conditions over these fields are 3b's (Q45); this emits the raw facts they will read.
fn emit_tick_span(claim: &Claim, report: Option<&TickReport>, errored: bool) {
    let span = tracing::info_span!(
        "sensitivity_sweep",
        run_id = %claim.run_id,
        job_id = %claim.job_id,
        ticked = report.is_some(),
        errored,
        outcome = tracing::field::Empty,
        failed = tracing::field::Empty,
        failure = tracing::field::Empty,
        rows_examined = tracing::field::Empty,
        hashes_examined = tracing::field::Empty,
        cache_hits = tracing::field::Empty,
        new_findings = tracing::field::Empty,
        new_findings_head = tracing::field::Empty,
        new_findings_backfill = tracing::field::Empty,
        cursor_advances = tracing::field::Empty,
        units_oversize = tracing::field::Empty,
        head_holdback_seconds = tracing::field::Empty,
        sev1 = tracing::field::Empty,
        sev2 = tracing::field::Empty,
        sev3 = tracing::field::Empty,
        sev4 = tracing::field::Empty,
    );
    let _entered = span.enter();
    let Some(t) = report else {
        if errored {
            tracing::error!(
                run_id = %claim.run_id,
                "sensitivity sweep tick raised after its claim committed; the run is left without \
                 an outcome and its lease is reaped"
            );
        } else {
            tracing::error!(
                run_id = %claim.run_id,
                "sensitivity sweep claim's lease lapsed before its tick read it"
            );
        }
        return;
    };
    span.record("outcome", t.outcome.as_str());
    span.record("failed", t.failed);
    if let Some(f) = t.failure {
        span.record("failure", f.as_str());
    }
    span.record("rows_examined", t.rows_examined);
    span.record("hashes_examined", t.hashes_examined);
    span.record("cache_hits", t.cache_hits);
    span.record("new_findings", t.new_findings);
    span.record("new_findings_head", t.new_findings_head);
    span.record("new_findings_backfill", t.new_findings_backfill);
    span.record("cursor_advances", t.cursor_advances);
    span.record("units_oversize", t.units_oversize);
    span.record("head_holdback_seconds", t.head_holdback_seconds);
    span.record("sev1", t.sev1);
    span.record("sev2", t.sev2);
    span.record("sev3", t.sev3);
    span.record("sev4", t.sev4);
    if t.failed {
        tracing::error!(
            run_id = %t.run_id,
            failure = t.failure.map_or("unrecognised", TickFailure::as_str),
            "sensitivity sweep tick failed"
        );
    }
}

/// One `sensitivity_sweep_call` span per call, plus an error event for each state in which the
/// sweep cannot work: these hold whether or not anything was claimed.
fn emit_call_span(summary: &SweepSummary) {
    let enabled = !matches!(summary.ended, Ended::Disabled);
    let span = tracing::info_span!(
        "sensitivity_sweep_call",
        enabled,
        salt_configured = summary.salt_configured,
        salt_may_be_logged = summary.salt_may_be_logged,
        ticks = summary.ticks.len(),
        ended = summary.ended.as_str(),
        slot_attempts = tracing::field::Empty,
        slot_failure = tracing::field::Empty,
    );
    let _entered = span.enter();
    // A deployment that has not opted in is not a sweep that cannot work: it raises nothing.
    if !enabled {
        return;
    }
    if !summary.salt_configured {
        tracing::error!(
            "SENSITIVITY_SWEEP_SALT is unset: the sensitivity sweep cannot scan, and every tick \
             records salt_missing"
        );
    }
    if summary.salt_configured && summary.salt_may_be_logged {
        tracing::error!(
            "the database may log the sensitivity sweep's salt: set log_parameter_max_length = 0, \
             or disable log_statement = all and duration logging"
        );
    }
    if let Ended::Unclaimed(Slot::Blocked { attempts, failure }) = summary.ended {
        span.record("slot_attempts", attempts);
        let failure = failure.map_or("unrecorded", TickFailure::as_str);
        span.record("slot_failure", failure);
        tracing::error!(
            attempts,
            failure,
            "a failed sensitivity sweep job holds the single-flight slot: every surface waits \
             while it backs off"
        );
    }
}
