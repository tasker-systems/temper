#![cfg(feature = "test-db")]
//! The sensitivity sweep's door (build order 3a PR D): spec D8, D9, Q44, Q45, and witness 2's
//! signature half. The span half of witness 2 is `sensitivity_sweep_span_test.rs`, in its own file
//! because it installs a process-global subscriber.
//!
//! The service runs the tick with its real five-minute lag, so planted content is backdated: a row
//! older than the bound is what the backfill lane reads on a new detector version's first tick.

use sqlx::{PgPool, Row};
use uuid::Uuid;

use temper_services::services::sensitivity_sweep_service::{sweep, TickFailure, TickOutcome};

const SALT: &[u8] = b"door-witness-salt-of-sixteen-plus";
const SSN: &str = "219-45-6789";

/// Put `surface`'s work order in flight, so the claim's own pick finds the single-flight slot taken
/// and claims this one.
async fn order(pool: &PgPool, surface: &str) {
    sqlx::query("SELECT workflow_job_enqueue_system('sensitivity', 'sensitivity-sweep', $1)")
        .bind(serde_json::json!({ "surface": surface, "budget": 1000 }))
        .execute(pool)
        .await
        .unwrap();
}

/// A resource whose title carries an SSN, stamped an hour ago so it sits below the head's bound.
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
    }
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

/// A run over planted content finds it, and the door's answer carries none of it.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn a_found_ssn_is_not_in_the_door_s_answer(pool: PgPool) {
    old_resource(&pool, &format!("Payroll note for {SSN}")).await;
    order(&pool, "kb_resources.title").await;

    let summary = sweep(&pool, Some(SALT)).await.expect("sweep runs");
    let tick = summary
        .tick
        .as_ref()
        .expect("the ordered surface was claimed and ticked");
    assert_eq!(tick.outcome, TickOutcome::Scanned);
    assert!(
        tick.new_findings >= 1 && tick.new_findings_backfill >= 1,
        "the planted SSN must be found, or this test proves nothing about the answer: {tick:?}"
    );
    assert!(tick.sev4 >= 1, "an SSN is severity 4 (Q18): {tick:?}");

    let wire = serde_json::to_string(&summary).unwrap();
    for needle in [SSN, "219456789", "6789"] {
        assert!(
            !wire.contains(needle),
            "the door's answer carries `{needle}`: {wire}"
        );
    }
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

/// Q44: an unset salt is recorded, not skipped. The run fails with `salt_missing` and its job
/// carries the code.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn an_unset_salt_is_recorded_as_salt_missing(pool: PgPool) {
    old_resource(&pool, &format!("Payroll note for {SSN}")).await;
    order(&pool, "kb_resources.title").await;

    let summary = sweep(&pool, None).await.expect("sweep runs");
    assert!(summary.claimed && !summary.salt_configured);
    let tick = summary.tick.expect("the claim was ticked");
    assert_eq!(tick.outcome, TickOutcome::Failed);
    assert_eq!(tick.failure, Some(TickFailure::SaltMissing));
    assert_eq!(tick.new_findings, 0, "nothing is scanned without a salt");

    let last_error: Option<String> =
        sqlx::query_scalar("SELECT last_error FROM kb_workflow_jobs WHERE id = $1")
            .bind(tick.job_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(last_error.as_deref(), Some("salt_missing"));
}
