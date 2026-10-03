#![cfg(feature = "test-db")]
//! The sensitivity sweep's door (build order 3a PR D): spec D8, D9, Q44–Q48, and witness 2's
//! signature half. The span half of witness 2 is `sensitivity_sweep_span_test.rs`, in its own file
//! because it installs a process-global subscriber.
//!
//! The service runs the tick with its real five-minute lag, so planted content is backdated: a row
//! older than the bound is what the backfill lane reads on a new detector version's first tick.

use std::time::Duration;

use sqlx::postgres::PgPoolOptions;
use sqlx::{PgPool, Row};
use uuid::Uuid;

use temper_services::services::sensitivity_sweep_service::{
    sweep, sweep_within, Ended, Slot, TickFailure, TickOutcome, TickReport,
};

const SALT: &[u8] = b"door-witness-salt-of-thirty-two-plus-bytes";
const SSN: &str = "219-45-6789";

/// Put `surface`'s work order in flight, so the claim's own pick finds the single-flight slot taken
/// and claims this one first.
async fn order(pool: &PgPool, surface: &str) {
    sqlx::query("SELECT workflow_job_enqueue_system('sensitivity', 'sensitivity-sweep', $1)")
        .bind(serde_json::json!({ "surface": surface, "budget": 1000 }))
        .execute(pool)
        .await
        .unwrap();
}

/// A resource titled `title`, stamped an hour ago so it sits below the head's bound.
async fn old_resource(pool: &PgPool, title: &str) -> Uuid {
    let id: Uuid = sqlx::query_scalar(
        "INSERT INTO kb_resources (title, origin_uri) VALUES ($1, 'test://door') RETURNING id",
    )
    .bind(title)
    .fetch_one(pool)
    .await
    .unwrap();
    sqlx::query("UPDATE kb_resources SET updated = now() - interval '1 hour' WHERE id = $1")
        .bind(id)
        .execute(pool)
        .await
        .unwrap();
    id
}

fn with_findings(ticks: &[TickReport]) -> Vec<&TickReport> {
    ticks.iter().filter(|t| t.new_findings > 0).collect()
}

/// Witness 2, the signature half: neither function the door calls can return text. Read from the
/// catalog, so a column added later in any type but these fails here.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn the_door_s_functions_return_no_text(pool: PgPool) {
    for function in ["sensitivity_sweep_claim", "sensitivity_sweep_tick"] {
        let types: Vec<(String, String)> = sqlx::query_as(
            "SELECT a.name, format_type(a.typ, NULL)
               FROM pg_proc p,
                    LATERAL unnest(p.proallargtypes, p.proargmodes, p.proargnames)
                            AS a(typ, mode, name)
              WHERE p.proname = $1 AND p.pronamespace = 'public'::regnamespace AND a.mode = 't'",
        )
        .bind(function)
        .fetch_all(&pool)
        .await
        .unwrap();
        assert!(!types.is_empty(), "{function} returns no table");
        for (name, ty) in types {
            assert!(
                matches!(ty.as_str(), "integer" | "smallint" | "boolean" | "uuid"),
                "{function}.{name} is {ty}: the door's functions return integers, booleans and ids \
                 only, so no content can cross the wire (spec D8, witness 2)"
            );
        }
    }
}

/// The numbers the tick returns mean what the Rust names say, and the failure set is the one the
/// job trigger admits into `last_error`. A code added on one side only fails here.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn the_tick_s_codes_are_the_rust_vocabulary(pool: PgPool) {
    let tick: String = sqlx::query_scalar(
        "SELECT pg_get_functiondef('sensitivity_sweep_tick(uuid, uuid, bytea, interval, int)'::regprocedure)",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    let trigger: String = sqlx::query_scalar(
        "SELECT pg_get_functiondef('workflow_jobs_sensitivity_error_coded()'::regprocedure)",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    for (n, failure) in TickFailure::CODED {
        let code = failure.as_str();
        assert!(
            tick.contains(&format!("WHEN '{code}' THEN {n}")),
            "the tick does not number `{code}` as {n}"
        );
        assert!(
            trigger.contains(&format!("'{code}'")),
            "the trigger does not admit `{code}`"
        );
        assert_eq!(TickFailure::from_code(n), failure);
        assert_eq!(TickFailure::from_last_error(code), failure);
    }
    assert!(trigger.contains("'lease expired'"));
    assert_eq!(
        TickFailure::from_last_error("lease expired"),
        TickFailure::LeaseExpired
    );
    let case = tick
        .split_once("failure := CASE v_code")
        .and_then(|(_, rest)| rest.split_once("END;"))
        .map(|(case, _)| case)
        .expect("the tick numbers its failure in one CASE");
    assert_eq!(
        case.matches("WHEN '").count(),
        TickFailure::CODED.len(),
        "the failure CASE has an arm the Rust vocabulary does not know"
    );
    for (n, outcome) in [
        (1, TickOutcome::Scanned),
        (2, TickOutcome::Idle),
        (3, TickOutcome::Failed),
    ] {
        assert!(
            tick.contains(&format!("WHEN '{}' THEN {n}", outcome.as_str())),
            "the tick does not number outcome `{}` as {n}",
            outcome.as_str()
        );
        assert_eq!(TickOutcome::from_code(Some(n)), outcome);
    }
}

/// Every number a tick reports is its run row's, field by field. A transaction held open across the
/// call makes the head's holdback non-zero, and the title carries two severities.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn a_tick_reports_its_run_row_field_by_field(pool: PgPool) {
    old_resource(&pool, &format!("Payroll {SSN} in /Users/jdoe/notes.md")).await;
    order(&pool, "kb_resources.title").await;
    let mut held = pool.begin().await.unwrap();
    sqlx::query("SELECT 1").execute(&mut *held).await.unwrap();
    tokio::time::sleep(Duration::from_millis(1_100)).await;

    let summary = sweep(&pool, Some(SALT)).await.expect("sweep runs");
    held.rollback().await.unwrap();
    let t = with_findings(&summary.ticks)[0];

    let run = sqlx::query(
        "SELECT rows_examined, hashes_examined, cache_hits, new_findings, cursor_advances,
                units_oversize, new_findings_head, new_findings_backfill, head_holdback_seconds,
                coalesce((by_severity ->> '1')::int, 0), coalesce((by_severity ->> '2')::int, 0),
                coalesce((by_severity ->> '3')::int, 0), coalesce((by_severity ->> '4')::int, 0),
                outcome
           FROM sensitivity.runs WHERE id = $1",
    )
    .bind(t.run_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    let reported = [
        t.rows_examined,
        t.hashes_examined,
        t.cache_hits,
        t.new_findings,
        t.cursor_advances,
        t.units_oversize,
        t.new_findings_head,
        t.new_findings_backfill,
        t.head_holdback_seconds,
        t.sev1,
        t.sev2,
        t.sev3,
        t.sev4,
    ];
    let stored: Vec<i32> = (0..13).map(|i| run.get::<i32, _>(i)).collect();
    assert_eq!(
        reported.to_vec(),
        stored,
        "a reported field is not its run's"
    );
    assert_eq!(run.get::<String, _>(13), t.outcome.as_str());
    // Non-vacuity: the comparison above can only tell fields apart if they differ.
    assert!(
        t.head_holdback_seconds >= 1,
        "the held transaction must hold the head back: {t:?}"
    );
    assert!(
        t.sev1 >= 1 && t.sev4 >= 1 && t.sev2 == 0,
        "the title carries a severity-1 and a severity-4 value: {t:?}"
    );
}

/// A run over planted content finds it, and the door's answer carries none of it, nor any count a
/// caller could learn the corpus from.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn a_found_ssn_is_not_in_the_door_s_answer(pool: PgPool) {
    old_resource(&pool, &format!("Payroll note for {SSN}")).await;
    order(&pool, "kb_resources.title").await;

    let summary = sweep(&pool, Some(SALT)).await.expect("sweep runs");
    assert!(
        with_findings(&summary.ticks)
            .iter()
            .any(|t| t.outcome == TickOutcome::Scanned && t.sev4 >= 1),
        "the planted SSN must be found, or this test proves nothing about the answer"
    );

    let wire = serde_json::to_value(summary.answer()).unwrap();
    let mut keys: Vec<&str> = wire
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    keys.sort_unstable();
    assert_eq!(
        keys,
        ["failed_ticks", "salt_configured", "slot_blocked", "ticks"],
        "the answer is counts of ticks and booleans, nothing per tick"
    );
    let wire = wire.to_string();
    for needle in [SSN, "219456789", "6789"] {
        assert!(
            !wire.contains(needle),
            "the door's answer carries `{needle}`: {wire}"
        );
    }
}

/// Q46: one call keeps claiming, so content on two surfaces is found in one call, and it stops
/// after a rotation in which no tick examined a row.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn one_call_sweeps_every_surface_and_stops_when_a_rotation_is_idle(pool: PgPool) {
    old_resource(&pool, &format!("Payroll note for {SSN}")).await;
    let surfaces: i64 =
        sqlx::query_scalar("SELECT count(*) FROM sensitivity.surfaces WHERE enabled")
            .fetch_one(&pool)
            .await
            .unwrap();

    let summary = sweep(&pool, Some(SALT)).await.expect("sweep runs");
    assert_eq!(summary.ended, Ended::Idle, "an idle rotation ends the call");
    assert!(
        summary.ticks.len() as i64 > surfaces,
        "every surface gets a tick, and the idle rotation follows the one that found: {} ticks \
         over {surfaces} surfaces",
        summary.ticks.len()
    );
    assert!(
        (summary.ticks.len() as i64) < 3 * surfaces,
        "an idle corpus costs about one rotation, not a busy loop: {} ticks",
        summary.ticks.len()
    );
    assert!(
        summary
            .ticks
            .iter()
            .rev()
            .take(surfaces as usize)
            .all(|t| t.rows_examined == 0),
        "the call stopped before a full idle rotation"
    );
    assert!(!with_findings(&summary.ticks).is_empty());

    let again = sweep(&pool, Some(SALT)).await.expect("sweep runs");
    assert_eq!(again.ended, Ended::Idle);
    assert_eq!(
        again.ticks.len() as i64,
        surfaces,
        "a quiet corpus costs exactly one rotation per call"
    );
}

/// Q46: a call never starts a tick it cannot finish inside its budget.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn a_call_starts_no_tick_it_cannot_finish(pool: PgPool) {
    let summary = sweep_within(&pool, Some(SALT), Duration::from_secs(1))
        .await
        .expect("sweep runs");
    assert_eq!(summary.ended, Ended::Budget);
    assert!(summary.ticks.is_empty());
}

/// The door reaps before it claims, as the incumbent drains do. Unreaped, a lapsed lease reads as a
/// live one (`slot_leased`, another call at work) for as long as no other cron reaps it. Reaped, it
/// is what it is: a failed job backing off, which the door reports as a blocked slot.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn a_lapsed_lease_is_reaped_before_the_claim(pool: PgPool) {
    order(&pool, "kb_resources.title").await;
    sqlx::query(
        "UPDATE kb_workflow_jobs SET status = 'in_progress', attempts = 1,
                lease_expires_at = now() - interval '1 minute'
          WHERE persona = 'sensitivity'",
    )
    .execute(&pool)
    .await
    .unwrap();

    let summary = sweep(&pool, Some(SALT)).await.expect("sweep runs");
    assert_eq!(
        summary.ended,
        Ended::Unclaimed(Slot::Blocked {
            attempts: 1,
            failure: Some(TickFailure::LeaseExpired)
        }),
        "a stale lease must read as a failed job, never as a live tick"
    );
}

/// The claim commits on its own: a tick that errors leaves the claimed run behind as its record.
/// Wrapped in one transaction with the tick, the run would roll back with the error.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn the_claim_commits_before_the_tick_runs(pool: PgPool) {
    order(&pool, "kb_resources.title").await;
    sqlx::query(
        "ALTER FUNCTION sensitivity_sweep_tick(uuid, uuid, bytea, interval, int) RENAME TO tick_gone",
    )
    .execute(&pool)
    .await
    .unwrap();

    assert!(
        sweep(&pool, Some(SALT)).await.is_err(),
        "the tick call must fail"
    );

    let row = sqlx::query(
        "SELECT r.outcome, j.status FROM sensitivity.runs r JOIN kb_workflow_jobs j ON j.id = r.job_id",
    )
    .fetch_one(&pool)
    .await
    .expect("the claimed run survives the failed tick");
    assert_eq!(row.get::<Option<String>, _>(0), None, "no tick finished it");
    assert_eq!(
        row.get::<String, _>(1),
        "in_progress",
        "the claim's lease stands"
    );
}

/// Q44: an unset salt is recorded, not skipped. The run fails with `salt_missing`, and the failed
/// job then holds the slot, which the call reports as blocked rather than as nothing to do.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn an_unset_salt_is_recorded_and_its_job_blocks_the_slot(pool: PgPool) {
    old_resource(&pool, &format!("Payroll note for {SSN}")).await;
    order(&pool, "kb_resources.title").await;

    let summary = sweep(&pool, None).await.expect("sweep runs");
    assert!(!summary.salt_configured);
    let tick = &summary.ticks[0];
    assert_eq!(tick.outcome, TickOutcome::Failed);
    assert_eq!(tick.failure, Some(TickFailure::SaltMissing));
    assert_eq!(tick.new_findings, 0, "nothing is scanned without a salt");
    assert_eq!(summary.ticks.len(), 1, "the failed job holds the slot");
    assert_eq!(
        summary.ended,
        Ended::Unclaimed(Slot::Blocked {
            attempts: 1,
            failure: Some(TickFailure::SaltMissing)
        })
    );
    assert!(summary.answer().slot_blocked);

    let last_error: Option<String> =
        sqlx::query_scalar("SELECT last_error FROM kb_workflow_jobs WHERE id = $1")
            .bind(tick.job_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(last_error.as_deref(), Some("salt_missing"));
}

/// Q47: the door turns off the parameter logging a session may turn off, and reports the logging it
/// cannot. Both are set on the database, so every new connection starts with them.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn the_door_mutes_what_it_can_and_reports_what_it_cannot(pool: PgPool) {
    let quiet = sweep_within(&pool, Some(SALT), Duration::from_secs(1))
        .await
        .expect("sweep runs");
    assert!(
        !quiet.salt_may_be_logged,
        "a stock server logs no parameters"
    );

    sqlx::query(
        "DO $$ BEGIN
             EXECUTE format('ALTER DATABASE %I SET log_parameter_max_length_on_error = 64',
                            current_database());
             EXECUTE format('ALTER DATABASE %I SET log_min_duration_statement = 1000',
                            current_database());
         END $$",
    )
    .execute(&pool)
    .await
    .unwrap();
    let one = PgPoolOptions::new()
        .max_connections(1)
        .connect_with(pool.connect_options().as_ref().clone())
        .await
        .unwrap();
    let before: String = sqlx::query_scalar("SHOW log_parameter_max_length_on_error")
        .fetch_one(&one)
        .await
        .unwrap();
    assert_eq!(
        before, "64B",
        "the connection starts with the database's setting"
    );

    let loud = sweep_within(&one, Some(SALT), Duration::from_secs(1))
        .await
        .expect("sweep runs");
    assert!(
        loud.salt_may_be_logged,
        "duration logging with unlimited parameters may write the salt"
    );
    let after: String = sqlx::query_scalar("SHOW log_parameter_max_length_on_error")
        .fetch_one(&one)
        .await
        .unwrap();
    assert_eq!(
        after, "0",
        "the door's connection must log no parameters on error"
    );
}
