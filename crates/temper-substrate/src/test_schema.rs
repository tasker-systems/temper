//! Test-only: drop a test database's `public` and `sensitivity` schemas, and empty its data tables,
//! without holding thousands of locks in one transaction.
//!
//! Postgres keeps every lock in one shared table, sized
//! `max_locks_per_transaction × (max_connections + max_prepared_transactions)`: 6,400 under the
//! defaults (64 × 100). A transaction that would need more than what is left fails with
//! `53200 out of shared memory`. The local docker-compose Postgres and CI's service container both
//! run `max_locks_per_transaction=256`; this module keeps the fixtures inside the default as well,
//! so neither setting is what keeps them green.
//!
//! Measured over the full migration chain at `20261201000040`, the lock count each transaction
//! holds:
//!
//! | Transaction | Locks |
//! |---|---|
//! | `DROP SCHEMA public CASCADE`, in one statement | about 3,700 |
//! | `TRUNCATE` of every `kb_` table, in one transaction | about 640 |
//! | after this module's split, the largest: `TRUNCATE kb_profiles … CASCADE` | about 470 |
//! | the largest single migration (`canonical_schema`) | about 320 |
//!
//! [`drop_test_schemas`] drops the routines in batches of [`ROUTINE_BATCH`], then each table, view
//! and materialized view in its own transaction, then the schemas, by then holding little.
//! [`truncate_kb_tables`] truncates one table per transaction. The end states are those of the
//! one-statement forms: neither schema exists, and every `kb_` table is empty.

use sqlx::{Executor, PgPool};

/// Routines dropped per transaction: about 190 locks, with the triggers and defaults a drop
/// cascades to.
pub const ROUTINE_BATCH: i64 = 100;

/// The routines in `public` and `sensitivity` that no extension owns, `ROUTINE_BATCH` at a time.
const ROUTINES: &str = "SELECT p.oid::regprocedure::text \
       FROM pg_proc p JOIN pg_namespace n ON n.oid = p.pronamespace \
      WHERE n.nspname IN ('public', 'sensitivity') AND p.prokind IN ('f', 'p') \
        AND NOT EXISTS (SELECT 1 FROM pg_depend d \
                         WHERE d.classid = 'pg_proc'::regclass AND d.objid = p.oid \
                           AND d.deptype = 'e') \
      ORDER BY p.oid LIMIT $1";

/// One table, view or materialized view in `public` or `sensitivity` that no extension owns, with
/// the `DROP` verb its kind takes. A partition goes with its parent.
const NEXT_RELATION: &str = "SELECT c.oid::regclass::text, \
            CASE c.relkind WHEN 'v' THEN 'VIEW' WHEN 'm' THEN 'MATERIALIZED VIEW' ELSE 'TABLE' END \
       FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace \
      WHERE n.nspname IN ('public', 'sensitivity') AND c.relkind IN ('r', 'p', 'v', 'm') \
        AND NOT c.relispartition \
        AND NOT EXISTS (SELECT 1 FROM pg_depend d \
                         WHERE d.classid = 'pg_class'::regclass AND d.objid = c.oid \
                           AND d.deptype = 'e') \
      ORDER BY c.oid LIMIT 1";

/// Drop `sensitivity` and `public` in the current database, a batch at a time; `public` is NOT
/// recreated. Panics on any failure, as a test fixture should.
pub async fn drop_test_schemas(pool: &PgPool) {
    // Routines first: a routine's drop cascades to the triggers, defaults and constraints that name
    // it. An extension's members are left to the extension, which the schema drop removes.
    loop {
        let routines: Vec<String> = sqlx::query_scalar(ROUTINES)
            .bind(ROUTINE_BATCH)
            .fetch_all(pool)
            .await
            .expect("list the routines to drop");
        if routines.is_empty() {
            break;
        }
        pool.execute(format!("DROP ROUTINE IF EXISTS {} CASCADE", routines.join(", ")).as_str())
            .await
            .expect("drop a batch of routines");
    }
    // Then each relation alone: a drop cascades into the constraints and indexes of the tables that
    // reference it, so even ten tables at once hold over six hundred locks. Each query reads what
    // the previous drops left.
    while let Some((relation, verb)) = sqlx::query_as::<_, (String, String)>(NEXT_RELATION)
        .fetch_optional(pool)
        .await
        .expect("find the next relation to drop")
    {
        pool.execute(format!("DROP {verb} IF EXISTS {relation} CASCADE").as_str())
            .await
            .unwrap_or_else(|e| panic!("drop {relation}: {e}"));
    }
    pool.execute("DROP SCHEMA IF EXISTS sensitivity CASCADE; DROP SCHEMA public CASCADE;")
        .await
        .expect("drop the sensitivity and public schemas");
}

/// Empty every `kb_` table in `public` (`TRUNCATE … RESTART IDENTITY CASCADE`), one table per
/// transaction.
pub async fn truncate_kb_tables(pool: &PgPool) {
    let tables: Vec<String> = sqlx::query_scalar(
        "SELECT quote_ident(tablename) FROM pg_tables \
          WHERE schemaname = 'public' AND tablename LIKE 'kb\\_%' ORDER BY tablename",
    )
    .fetch_all(pool)
    .await
    .expect("list the kb_ tables to truncate");
    for table in tables {
        pool.execute(format!("TRUNCATE TABLE {table} RESTART IDENTITY CASCADE").as_str())
            .await
            .unwrap_or_else(|e| panic!("truncate {table}: {e}"));
    }
}
