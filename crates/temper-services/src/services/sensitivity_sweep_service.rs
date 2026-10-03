//! The sensitivity sweep's door: one cron call, one claimed surface, one tick (sensitivity-sweep spec
//! D8, D9; build order 3a PR D).
//!
//! ## Two statements, never one transaction
//!
//! `sensitivity_sweep_claim` commits the claimed job and the run row on its own. That commit is
//! what lets a tick that dies part-way keep its attempt and leave a run with no outcome behind as
//! the record of it (`migrations/20261002200000_sensitivity_scan.sql`). Wrapping the two calls in
//! one transaction would roll the run back with the failure, and the attempt would vanish.
//!
//! ## Nothing crosses the wire
//!
//! The tick's `RETURNS TABLE` holds integers, a boolean and nothing else (D8, Q45). Its outcome and
//! failure come back as numbers and are named here from a closed vocabulary, so no string read from
//! the database reaches a span, a log line or the response. The service names only the two public
//! functions, never the schema behind them: that is witness 12's grep gate.
//!
//! ## A missing salt is loud
//!
//! A tick without a salt refuses with `salt_missing` and records a failed run (Q34). That failure
//! backs its job off for up to an hour, and while the job waits the claim finds nothing. So the
//! door raises its error event on the missing salt itself, whether or not anything was claimed.
//! Otherwise an unconfigured deployment would go quiet between retries, which is the hazard D8's
//! "no new secret" names (Q44).

use serde::Serialize;
use sqlx::PgPool;
use uuid::Uuid;

use crate::error::ApiResult;

/// Fields every `sensitivity_sweep` span declares, claimed or not.
pub const SENSITIVITY_SWEEP_FIELDS: [&str; 2] = ["claimed", "salt_configured"];

/// Fields carried only when a tick ran: absent, not zero, when nothing was claimed.
pub const SENSITIVITY_SWEEP_RUN_FIELDS: [&str; 17] = [
    "run_id",
    "job_id",
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

/// How a tick ended. Numbered in `sensitivity_sweep_tick` (Q45).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
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

/// Why a tick failed. The same closed set `workflow_jobs_sensitivity_error_coded` admits into
/// `last_error`, numbered in `sensitivity_sweep_tick`; a test holds the two equal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TickFailure {
    SaltMissing,
    StatementTimeout,
    DetectorPatternInvalid,
    StoreConstraint,
    ScanFailed,
    /// A number this binary does not know: a newer migration than the code.
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

    pub fn as_str(self) -> &'static str {
        match self {
            Self::SaltMissing => "salt_missing",
            Self::StatementTimeout => "statement_timeout",
            Self::DetectorPatternInvalid => "detector_pattern_invalid",
            Self::StoreConstraint => "store_constraint",
            Self::ScanFailed => "scan_failed",
            Self::Unrecognised => "unrecognised",
        }
    }
}

/// One tick's facts: counts and ids, never text (D8).
#[derive(Debug, Clone, Serialize)]
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

/// What one call of the door did.
#[derive(Debug, Clone, Serialize)]
pub struct SweepSummary {
    /// Whether a work order was claimed. False when a job is already in flight or backing off.
    pub claimed: bool,
    pub salt_configured: bool,
    /// The tick, when one ran. None also when the claim's lease lapsed before the tick read it.
    pub tick: Option<TickReport>,
}

struct Claim {
    run_id: Uuid,
    job_id: Uuid,
}

/// One sweep tick: claim a work order, then scan it, as two separate statements.
///
/// `salt` is passed through even when absent: the tick then records the `salt_missing` run, which
/// is the durable half of the signal. The span and the error event are the other half.
pub async fn sweep(pool: &PgPool, salt: Option<&[u8]>) -> ApiResult<SweepSummary> {
    let claim = sqlx::query_as!(
        Claim,
        r#"SELECT run_id AS "run_id!", job_id AS "job_id!" FROM sensitivity_sweep_claim()"#
    )
    .fetch_optional(pool)
    .await?;

    let claimed = claim.is_some();
    let tick = match claim {
        None => None,
        Some(claim) => tick(pool, &claim, salt).await?,
    };
    let summary = SweepSummary {
        claimed,
        salt_configured: salt.is_some(),
        tick,
    };
    emit_sweep_span(&summary);
    Ok(summary)
}

async fn tick(pool: &PgPool, claim: &Claim, salt: Option<&[u8]>) -> ApiResult<Option<TickReport>> {
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
    .fetch_optional(pool)
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

/// One `sensitivity_sweep` span per call, plus an error event when the sweep cannot work.
///
/// `internal` kind, like `internal_call_health`: an observation, not a request boundary. Alert
/// conditions over these fields are 3b's (Q45); this emits the raw facts they will read.
fn emit_sweep_span(summary: &SweepSummary) {
    let span = tracing::info_span!(
        "sensitivity_sweep",
        claimed = summary.claimed,
        salt_configured = summary.salt_configured,
        run_id = tracing::field::Empty,
        job_id = tracing::field::Empty,
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
    if let Some(t) = &summary.tick {
        span.record("run_id", tracing::field::display(t.run_id));
        span.record("job_id", tracing::field::display(t.job_id));
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
    }

    if !summary.salt_configured {
        tracing::error!(
            claimed = summary.claimed,
            "SENSITIVITY_SWEEP_SALT is unset: the sensitivity sweep cannot scan, and every tick \
             records salt_missing"
        );
    } else if let Some(t) = summary.tick.as_ref().filter(|t| t.failed) {
        tracing::error!(
            run_id = %t.run_id,
            failure = t.failure.map_or("unrecognised", TickFailure::as_str),
            "sensitivity sweep tick failed"
        );
    }
}
