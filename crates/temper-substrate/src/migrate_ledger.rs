//! Applying migrations so that what happened to them is recorded, including when they fail.
//!
//! # Why this exists
//!
//! `kb_migration_ledger` has two axes: what a migration *claims* about itself, and what *happened*
//! to it. The claim is written by the migration, inside its own transaction. The outcome cannot be:
//!
//! `[observed — 2026-07-31]` sqlx wraps a migration's body and its `_sqlx_migrations` bookkeeping in
//! **one transaction** (`sqlx-postgres-0.8.6/src/migrate.rs:222-224`). So a `pending` row written
//! from inside a migration is rolled back by the very failure it exists to record — probed
//! directly, zero rows survive. **`failed` is unrecordable from inside a migration.**
//!
//! And sqlx will not tell us either. `_sqlx_migrations.success` is written in exactly one place,
//! hardcoded `TRUE` (`migrate.rs:288`); nothing ever writes `false`. So [`Migrate::dirty_version`],
//! whose only job is `SELECT version … WHERE success = false`, **can never return a row on
//! Postgres** — even though the trait's own comment promises "insert new row on completion (success
//! or failure)".
//!
//! # What this borrows rather than rebuilds
//!
//! [`sqlx::migrate::Migrator::run_direct`] is a thin loop: lock, ensure the table, check dirty, validate the
//! applied set, then `conn.apply(migration)` for each pending one. Every piece of that is public
//! API except one ~20-line private helper. So rather than writing a runner, this is a **decorator**
//! implementing [`Migrate`] over a `PgConnection` and handed straight to `run_direct` — which keeps
//! sqlx's advisory lock, its checksum validation, its `_sqlx_migrations` bookkeeping and its apply
//! loop, and adds only what sqlx lacks.
//!
//! Five methods delegate untouched. Two do not:
//!
//! - [`apply`](LedgerRecorder::apply) brackets the delegated apply with `pending` before and
//!   `success`/`failed` after. These land **outside** the migration's transaction because
//!   `PgConnection::apply` opens its own internally, so writes on either side of it autocommit.
//!   That is precisely the property the rollback probe showed we need.
//! - [`dirty_version`](LedgerRecorder::dirty_version) answers from the ledger: a `pending` with no
//!   later terminal entry. **This is the interesting half.** `run_direct` already turns `Some(v)`
//!   into `MigrateError::Dirty(v)` and refuses to run, so a previously-crashed migration blocks the
//!   next attempt with no new control flow of ours at all. We are not bolting state tracking onto
//!   sqlx; we are supplying a value its own design asks for and its Postgres driver leaves dead.
//!
//! # Known coupling
//!
//! `run_direct` is `#[doc(hidden)]` — public, but not part of sqlx's advertised surface. It is the
//! documented escape hatch for exactly this shape (its own comment: *"Getting around the annoying
//! `implementation of Acquire is not general enough` error"*), and `run` cannot be used because our
//! decorator is not reachable through `Acquire`. Worth re-checking on a sqlx upgrade.

use std::time::Duration;

use futures_core::future::BoxFuture;
use sqlx::migrate::{AppliedMigration, Migrate, MigrateError, Migration};
use sqlx::PgConnection;

/// The state tokens, mirroring the `migration_state` enum in `20260731000010`.
const PENDING: &str = "pending";
const SUCCESS: &str = "success";
const FAILED: &str = "failed";

/// A `PgConnection` that records what happens to each migration it applies.
pub struct LedgerRecorder<'c> {
    conn: &'c mut PgConnection,
}

impl<'c> LedgerRecorder<'c> {
    pub fn new(conn: &'c mut PgConnection) -> Self {
        Self { conn }
    }
}

/// Does the ledger exist yet?
///
/// **The bootstrap case is real, not defensive.** On a fresh database the runner reaches
/// `20260731000010` — the migration that *creates* the ledger — and tries to write its `pending`
/// entry before the table exists. Postgres resolves relation names at parse time, so no in-query
/// guard helps; the check has to come first.
///
/// The result is honest rather than awkward: the ledger-creating migration gets a `success` entry
/// and no `pending` one, because at the moment `pending` would have been written there was nowhere
/// to write it. Every migration after it has both.
async fn ledger_exists(conn: &mut PgConnection) -> Result<bool, sqlx::Error> {
    Ok(
        sqlx::query_scalar!("SELECT to_regclass('public.kb_migration_ledger') IS NOT NULL")
            .fetch_one(conn)
            .await?
            .unwrap_or(false),
    )
}

/// Append one state entry, in its own statement so it is not swept up by the migration's
/// transaction.
///
/// The `$2::text::migration_state` double cast is deliberate: a bare `$2::migration_state` makes
/// Postgres infer the parameter as the enum, which `query!` would then need a Rust type mapping
/// for. Going through `text` keeps the bind a `&str` and the macro compile-checked, which is the
/// repo's rule for production SQL.
async fn record_state(
    conn: &mut PgConnection,
    version: i64,
    state: &str,
    reason: &str,
) -> Result<(), sqlx::Error> {
    if !ledger_exists(&mut *conn).await? {
        return Ok(());
    }
    sqlx::query!(
        "SELECT record_migration_state($1, $2::text::migration_state, $3, $4)",
        version,
        state,
        reason,
        "runner"
    )
    .execute(conn)
    .await?;
    Ok(())
}

impl Migrate for LedgerRecorder<'_> {
    fn ensure_migrations_table(&mut self) -> BoxFuture<'_, Result<(), MigrateError>> {
        self.conn.ensure_migrations_table()
    }

    /// The half sqlx cannot answer for itself.
    ///
    /// A `pending` with no later terminal entry for the same version is a migration that started
    /// and never resolved — the crash case. `run_direct` turns a `Some` here into
    /// `MigrateError::Dirty` and refuses to proceed, so recovery is a deliberate act rather than a
    /// second attempt on top of half-applied state.
    ///
    /// The `to_regclass` guard is load-bearing: on a fresh database the ledger table does not exist
    /// until `20260731000010` applies, and Postgres resolves table names at parse time, so no
    /// in-query guard can save a statement that names a missing relation.
    ///
    /// The inner connection is still consulted. Its answer is provably inert on Postgres today, but
    /// "provably inert in 0.8.6" is not "inert forever", and deferring to it costs one query.
    fn dirty_version(&mut self) -> BoxFuture<'_, Result<Option<i64>, MigrateError>> {
        Box::pin(async move {
            let have_ledger = ledger_exists(&mut *self.conn)
                .await
                .map_err(MigrateError::Execute)?;

            if have_ledger {
                let stuck = sqlx::query_scalar!(
                    r#"
                    SELECT l.version
                      FROM kb_migration_ledger l
                     WHERE l.state = 'pending'
                       AND NOT EXISTS (
                           SELECT 1
                             FROM kb_migration_ledger t
                            WHERE t.version = l.version
                              AND t.state IN ('success', 'failed', 'cancelled')
                              AND t.id > l.id
                       )
                     ORDER BY l.version
                     LIMIT 1
                    "#
                )
                .fetch_optional(&mut *self.conn)
                .await
                .map_err(MigrateError::Execute)?;

                if stuck.is_some() {
                    return Ok(stuck);
                }
            }

            self.conn.dirty_version().await
        })
    }

    fn list_applied_migrations(
        &mut self,
    ) -> BoxFuture<'_, Result<Vec<AppliedMigration>, MigrateError>> {
        self.conn.list_applied_migrations()
    }

    fn lock(&mut self) -> BoxFuture<'_, Result<(), MigrateError>> {
        self.conn.lock()
    }

    fn unlock(&mut self) -> BoxFuture<'_, Result<(), MigrateError>> {
        self.conn.unlock()
    }

    /// `pending` before, terminal after — both outside the migration's own transaction.
    fn apply<'e: 'm, 'm>(
        &'e mut self,
        migration: &'m Migration,
    ) -> BoxFuture<'m, Result<Duration, MigrateError>> {
        Box::pin(async move {
            record_state(
                self.conn,
                migration.version,
                PENDING,
                &format!("Apply started by the runner: {}", migration.description),
            )
            .await
            .map_err(MigrateError::Execute)?;

            match self.conn.apply(migration).await {
                Ok(elapsed) => {
                    record_state(
                        self.conn,
                        migration.version,
                        SUCCESS,
                        &format!("Applied in {elapsed:?}."),
                    )
                    .await
                    .map_err(MigrateError::Execute)?;
                    Ok(elapsed)
                }
                Err(e) => {
                    // Best-effort: the apply's transaction has rolled back, so the connection
                    // should be usable — but if it is not, the ORIGINAL error is what the operator
                    // needs, not a failure to write about it. A lost `failed` entry still leaves
                    // the `pending` one, which is what `dirty_version` reads.
                    let _ = record_state(
                        self.conn,
                        migration.version,
                        FAILED,
                        &format!("Apply failed: {e}"),
                    )
                    .await;
                    Err(e)
                }
            }
        })
    }

    fn revert<'e: 'm, 'm>(
        &'e mut self,
        migration: &'m Migration,
    ) -> BoxFuture<'m, Result<Duration, MigrateError>> {
        // Not bracketed. This repo has no down-migrations — `kb_events` is append-only and
        // projection rebuild is the established repair shape — so there is no reverted state worth
        // a vocabulary we would be inventing rather than grounding.
        self.conn.revert(migration)
    }
}

/// Apply every pending migration, recording each apply's outcome.
///
/// Delegates the whole run to [`sqlx::migrate::Migrator::run_direct`], so checksum validation, the
/// advisory lock, the `_sqlx_migrations` ledger and the ordering are sqlx's, unchanged.
pub async fn run_with_ledger(
    migrator: &sqlx::migrate::Migrator,
    conn: &mut PgConnection,
) -> Result<(), MigrateError> {
    let mut recorder = LedgerRecorder::new(conn);
    migrator.run_direct(&mut recorder).await
}
