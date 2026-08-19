#![cfg(feature = "test-db")]

//! The deploy applies its own additive schema, and refuses the rest — against a real database.
//!
//! Spec §3 (`internal/superpowers/specs/2026-07-30-schema-binary-pairing-design.md`), step 4.
//!
//! # Why this exists alongside the unit tests
//!
//! `migrate_ledger::plan_additive_run` is pure and unit-tested, so the ROUTING is already covered
//! without a database. What only a database can show is the half the routing depends on and cannot
//! see: that the truncated `Migrator` still satisfies sqlx's own `validate_applied_migrations`,
//! that the applied migration lands in `_sqlx_migrations`, and — the acceptance criterion this task
//! was written for — that the ledger records `pending` → `success` **from the runner**.
//!
//! Production has zero `recorded_by = 'runner'` rows today, because migrations there are applied by
//! hand `[observed — 2026-07-31]`. These are the first ones anything produces.
//!
//! # Why the probe migrations are synthetic
//!
//! `#[sqlx::test(migrator = …)]` hands over a database with the whole real chain already applied,
//! so there is nothing pending to route. Appending a probe to the real set is what creates a
//! pending migration to decide about — and it keeps the test honest about the real chain, since the
//! probe is applied on top of it rather than instead of it.

use std::borrow::Cow;

use sqlx::migrate::{Migration, MigrationType, Migrator};
use sqlx::PgPool;
use temper_substrate::migrate_ledger::{self, HaltReason};

/// A version far beyond anything `migrations/` will ever hold, so a probe can never collide with a
/// real migration.
const PROBE_A: i64 = 29_999_999_999_901;
const PROBE_B: i64 = 29_999_999_999_902;
const PROBE_C: i64 = 29_999_999_999_903;

/// Build a migrator holding the real chain plus the given probes, in order.
fn with_probes(probes: Vec<Migration>) -> Migrator {
    let mut migrations: Vec<Migration> = temper_substrate::MIGRATOR.iter().cloned().collect();
    migrations.extend(probes);
    Migrator {
        migrations: Cow::Owned(migrations),
        ..Migrator::DEFAULT
    }
}

fn probe(version: i64, class: &str, body: &str) -> Migration {
    Migration::new(
        version,
        Cow::Owned(format!("probe_{version}")),
        MigrationType::Simple,
        Cow::Owned(format!(
            "{body}\nSELECT declare_migration({version}, '{class}', 'Test-only probe migration.');"
        )),
        false,
    )
}

fn creates(table: &str) -> String {
    format!("CREATE TABLE {table} (id int);")
}

async fn table_exists(pool: &PgPool, table: &str) -> bool {
    sqlx::query_scalar::<_, Option<String>>("SELECT to_regclass($1)::text")
        .bind(format!("public.{table}"))
        .fetch_one(pool)
        .await
        .unwrap()
        .is_some()
}

/// Every ledger entry for one version, oldest first, as `(state, recorded_by)`.
async fn ledger_trail(pool: &PgPool, version: i64) -> Vec<(Option<String>, String)> {
    sqlx::query_as::<_, (Option<String>, String)>(
        "SELECT state::text, recorded_by FROM kb_migration_ledger \
         WHERE version = $1 AND state IS NOT NULL ORDER BY id",
    )
    .bind(version)
    .fetch_all(pool)
    .await
    .unwrap()
}

#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn an_additive_migration_applies_and_the_runner_records_pending_then_success(pool: PgPool) {
    let migrator = with_probes(vec![probe(PROBE_A, "additive", &creates("zz_probe_a"))]);
    let mut conn = pool.acquire().await.unwrap();

    let run = migrate_ledger::run_additive_only(&migrator, &mut conn)
        .await
        .expect("an all-additive pending set must apply");

    assert_eq!(run.applied, vec![PROBE_A]);
    assert!(run.halt.is_none());
    assert!(
        table_exists(&pool, "zz_probe_a").await,
        "the DDL did not run"
    );

    // The acceptance criterion: the outcome axis, written by the runner, from outside the
    // migration's own transaction.
    assert_eq!(
        ledger_trail(&pool, PROBE_A).await,
        vec![
            (Some("pending".to_string()), "runner".to_string()),
            (Some("success".to_string()), "runner".to_string()),
        ]
    );

    // And the claim axis, written by the migration itself, resolved through the view.
    let class: Option<String> =
        sqlx::query_scalar("SELECT class::text FROM migration_current WHERE version = $1")
            .bind(PROBE_A)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(class.as_deref(), Some("additive"));
}

#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn a_shape_breaking_migration_halts_the_run_and_is_not_applied(pool: PgPool) {
    let migrator = with_probes(vec![probe(
        PROBE_A,
        "shape-breaking",
        &creates("zz_probe_a"),
    )]);
    let mut conn = pool.acquire().await.unwrap();

    let run = migrate_ledger::run_additive_only(&migrator, &mut conn)
        .await
        .expect("a halt is a refusal, not an error — it must not surface as one");

    assert!(run.applied.is_empty());
    let halt = run
        .halt
        .expect("the shape-breaking probe must halt the run");
    assert_eq!(halt.version, PROBE_A);
    assert_eq!(halt.reason, HaltReason::ShapeBreaking);

    assert!(
        !table_exists(&pool, "zz_probe_a").await,
        "the operator-gated migration ran anyway"
    );
    assert!(
        ledger_trail(&pool, PROBE_A).await.is_empty(),
        "a refused migration must leave no outcome — it was never attempted"
    );
}

#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn the_run_stops_at_the_halt_rather_than_skipping_past_it(pool: PgPool) {
    // The failure a naive `MIGRATOR.run()` produces, and the one this whole mechanism exists to
    // prevent: C is additive, so a filter would happily apply it — on top of a schema B never made.
    let migrator = with_probes(vec![
        probe(PROBE_A, "additive", &creates("zz_probe_a")),
        probe(PROBE_B, "shape-breaking", &creates("zz_probe_b")),
        probe(PROBE_C, "additive", &creates("zz_probe_c")),
    ]);
    let mut conn = pool.acquire().await.unwrap();

    let run = migrate_ledger::run_additive_only(&migrator, &mut conn)
        .await
        .unwrap();

    assert_eq!(run.applied, vec![PROBE_A]);
    assert_eq!(run.halt.unwrap().version, PROBE_B);

    assert!(table_exists(&pool, "zz_probe_a").await);
    assert!(!table_exists(&pool, "zz_probe_b").await);
    assert!(
        !table_exists(&pool, "zz_probe_c").await,
        "an additive migration after the halt was applied — halting must stop, not skip"
    );
}

#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn an_undeclared_migration_halts_because_silence_is_not_safety(pool: PgPool) {
    let undeclared = Migration::new(
        PROBE_A,
        Cow::Borrowed("probe_undeclared"),
        MigrationType::Simple,
        Cow::Owned(creates("zz_probe_a")),
        false,
    );
    let migrator = with_probes(vec![undeclared]);
    let mut conn = pool.acquire().await.unwrap();

    let run = migrate_ledger::run_additive_only(&migrator, &mut conn)
        .await
        .unwrap();

    assert!(run.applied.is_empty());
    assert_eq!(run.halt.unwrap().reason, HaltReason::Undeclared);
    assert!(!table_exists(&pool, "zz_probe_a").await);
}

#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn a_fully_applied_chain_applies_nothing_and_halts_at_nothing(pool: PgPool) {
    // The ordinary deploy: no migration in the PR. This must be a no-op rather than an error, and
    // in particular the truncated migrator must not trip sqlx's `validate_applied_migrations` on a
    // database that is already at head.
    let mut conn = pool.acquire().await.unwrap();

    let run = migrate_ledger::run_additive_only(&temper_substrate::MIGRATOR, &mut conn)
        .await
        .unwrap();

    assert!(run.applied.is_empty());
    assert!(run.halt.is_none());
}

#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn the_real_chain_is_routable_end_to_end_on_a_database_at_head(pool: PgPool) {
    // Pointed at the REAL migration set rather than a fixture: every one of the 150 versions must
    // resolve to a class the router recognises. A router green on probes and wrong about the repo
    // is a shape this goal has already met more than once.
    let mut conn = pool.acquire().await.unwrap();
    let applied: std::collections::HashSet<i64> =
        sqlx::query_scalar("SELECT version FROM _sqlx_migrations")
            .fetch_all(&mut *conn)
            .await
            .unwrap()
            .into_iter()
            .collect();

    let plan = migrate_ledger::plan_additive_run(&temper_substrate::MIGRATOR, &applied);
    assert!(
        plan.halt.is_none(),
        "the real chain halts at {:?} on a database that already applied it",
        plan.halt
    );
    assert_eq!(
        plan.migrations.len(),
        temper_substrate::MIGRATOR.iter().count(),
        "nothing may be truncated away when there is no halt"
    );
}
