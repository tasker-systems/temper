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

use std::borrow::Cow;
use std::collections::{BTreeMap, HashSet};
use std::fmt;
use std::time::Duration;

use futures_core::future::BoxFuture;
use sqlx::migrate::{AppliedMigration, Migrate, MigrateError, Migration, Migrator};
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

// ══════════════════════════════════════════════════════════════════════════════════════════════
// Routing on the declaration — spec §3, "the deploy applies additive migrations; nothing else"
// ══════════════════════════════════════════════════════════════════════════════════════════════
//
// > "The build applies the pending set only while every member of it is additive, and halts at the
// >  first shape-breaking migration rather than running past it. This is not a detail —
// >  `MIGRATOR.run()` applies *all* pending migrations, so a naive call would apply an
// >  operator-gated migration as a side effect of some later, unrelated, additive deploy."
//
// # Why the class is read from the SQL and not from `migration_current`
//
// `declare_migration` runs INSIDE the migration's own transaction, so a pending migration's class
// is not in the database yet — by the time the view could answer, the migration has already been
// applied and the routing decision is moot. The claim has to be read from the SQL the binary
// carries (`Migration.sql`), which is exactly what `MIGRATOR` embeds.
//
// # Why the whole carried set is scanned, not each migration's own file
//
// The declaration rule is SET-shaped, not per-file: `20260731000020` declares 148 other versions,
// because those shipped before the mechanism existed and a shipped migration cannot be edited
// (sqlx checksum-verifies at `sqlx-core/src/migrate/migrator.rs:175`). So the map is built over
// every embedded migration, latest claim winning — which is also what makes `reclassify_migration`
// route correctly rather than being ignored.
//
// # Why this parser answers to a shared corpus
//
// `.github/scripts/audit-migration-declarations.sh` reads the same calls in awk, over the FILES,
// with no database and no toolchain. Two implementations of one rule in two languages is the shape
// that drifts silently, and here the drift has teeth in one direction: a Rust half that read
// `additive` where the call says `shape-breaking` would auto-apply an operator-gated migration —
// the 2026-07-30 outage class, re-entered through the mechanism built to prevent it. Both halves
// are held to `scripts/migration-declaration-corpus.txt`, the shape already proven by the manifest
// containment rule.

/// The vocabulary, mirroring the `migration_class` enum in `20260731000010`.
///
/// The fourth spelling of two tokens — the SQL enum, `DEPLOYING.md`'s prose, the awk checker's
/// `VALID_CLASSES`, and this. It cannot read the enum, for the same reason the whole routing
/// decision cannot: a PENDING migration's class is not in the database yet. Unlike the parser
/// duplication this module shares with the awk checker, this one announces itself — a token renamed
/// in SQL and not here halts every deploy rather than misrouting one migration.
const ADDITIVE: &str = "additive";
const SHAPE_BREAKING: &str = "shape-breaking";

/// Which function was called. Both are parsed, and the distinction is carried, because a
/// reclassification revises a claim rather than making a first one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeclarationKind {
    Declare,
    Reclassify,
}

/// One `declare_migration` / `reclassify_migration` call, as read out of SQL.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParsedCall {
    /// A call whose arguments could not be read. Reported rather than skipped: a parser that
    /// ignored what it could not read would turn a typo into silence, which is the failure this
    /// whole mechanism exists to make loud.
    Malformed,
    Call {
        kind: DeclarationKind,
        version: i64,
        /// The class token exactly as written. An unrecognised token is carried through rather
        /// than dropped, so the halt can name what it rejected instead of reporting the version as
        /// undeclared — a different and less informative failure.
        class: String,
    },
}

/// Why the run stopped short of the full pending set.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HaltReason {
    /// Declared `shape-breaking`. Operator-gated cutover, per spec §3 and `DEPLOYING.md`.
    ShapeBreaking,
    /// Nothing in the carried set declares this version. Silence is not safety — spec §1: "a
    /// migration that says nothing is not thereby safe".
    Undeclared,
    /// Declared, with a token that is not in the vocabulary.
    Unrecognized(String),
}

impl fmt::Display for HaltReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ShapeBreaking => write!(f, "declared `{SHAPE_BREAKING}`"),
            Self::Undeclared => write!(
                f,
                "declared by nothing in this binary's migration set — silence is not safety"
            ),
            Self::Unrecognized(token) => write!(
                f,
                "declared `{token}`, which is not `{ADDITIVE}` or `{SHAPE_BREAKING}`"
            ),
        }
    }
}

/// The migration the run refused to apply, and why.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Halt {
    pub version: i64,
    pub description: String,
    pub reason: HaltReason,
}

impl fmt::Display for Halt {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} ({}) — {}",
            self.version, self.description, self.reason
        )
    }
}

/// What an additive-only run will do, decided before anything is applied.
#[derive(Debug, Clone)]
pub struct AdditivePlan {
    /// The list to hand `run_direct`: every migration before the halt, plus every already-applied
    /// migration wherever it sits.
    pub migrations: Vec<Migration>,
    /// The versions this run will actually apply, in order.
    pub to_apply: Vec<i64>,
    /// `None` when the whole pending set is additive.
    pub halt: Option<Halt>,
}

/// What an additive-only run did.
#[derive(Debug, Clone)]
pub struct AdditiveRun {
    pub applied: Vec<i64>,
    pub halt: Option<Halt>,
}

/// Is this byte one of POSIX `[[:space:]]` in the C locale?
///
/// Deliberately not `char::is_whitespace`, which is the Unicode set and therefore a wider one than
/// the awk half recognises. Parity with awk is the property under test.
fn is_posix_space(b: u8) -> bool {
    matches!(b, b' ' | b'\t' | b'\n' | b'\r' | 0x0b | 0x0c)
}

/// Read every declaration call out of a migration's SQL.
///
/// **Mirrors `parse_declarations` in `.github/scripts/audit-migration-declarations.sh`**, including
/// its two load-bearing parsing decisions:
///
///   * declarations are read ACROSS LINES — every real one in this repo spans three or four,
///     because the reason is a sentence, and a line-oriented scan would report every migration as
///     undeclared while passing its own unit tests;
///   * FULL-LINE `--` COMMENTS ARE STRIPPED FIRST — migration headers here are long and quote SQL
///     freely, and `DEPLOYING.md`'s worked example is a `declare_migration` call, so a parser that
///     read commented text would let a header claim a class the file does not have. Trailing inline
///     comments are NOT stripped, because `--` also appears inside string literals and a naive
///     strip would truncate a reason.
///
/// The reason argument is consumed even though nothing here reads it, because leaving it in the
/// stream would let text inside it be rescanned as a call.
pub fn parse_declarations(sql: &str) -> Vec<ParsedCall> {
    // Strip full-line comments, then join with a space — the awk half's `buf = buf " " $0`.
    let mut buf = String::with_capacity(sql.len() + 8);
    for line in sql.lines() {
        if line.trim_start().starts_with("--") {
            continue;
        }
        buf.push(' ');
        buf.push_str(line);
    }

    let mut out = Vec::new();
    let mut rest: &str = &buf;

    while let Some((after_paren, kind)) = find_call(rest) {
        rest = &rest[after_paren..];

        let Some((version, after)) = take_version(rest) else {
            out.push(ParsedCall::Malformed);
            continue;
        };
        rest = after;

        let Some((class, after)) = take_quoted(rest) else {
            out.push(ParsedCall::Malformed);
            continue;
        };
        rest = after;

        out.push(ParsedCall::Call {
            kind,
            version,
            class: class.to_string(),
        });

        // Consume the reason if there is one. Its absence is not malformed — the awk half emits the
        // call with an empty reason and moves on, and the reason's own validation is CI's job.
        if let Some((_, after)) = take_quoted(rest) {
            rest = after;
        } else {
            break;
        }
    }

    out
}

/// Find the leftmost `(SELECT|PERFORM) <ws> (declare|reclassify)_migration <ws?> (`.
///
/// Anchoring on the invoking keyword is what separates a CALL from its definition: `20260731000010`
/// both defines these functions and `COMMENT ON`s them at length, and neither is a declaration of
/// anything. Returns the offset just past the `(`.
fn find_call(hay: &str) -> Option<(usize, DeclarationKind)> {
    let bytes = hay.as_bytes();
    // `char_indices` rather than a byte range: migration bodies carry non-ASCII prose (the first
    // `§` in `migrations/` is at byte 4585 of the very first migration), and slicing a byte offset
    // that lands mid-character panics.
    for (start, ch) in hay.char_indices() {
        let keyword = if ch == 'S' && hay[start..].starts_with("SELECT") {
            "SELECT"
        } else if ch == 'P' && hay[start..].starts_with("PERFORM") {
            "PERFORM"
        } else {
            continue;
        };
        let mut i = start + keyword.len();

        let ws_start = i;
        while i < bytes.len() && is_posix_space(bytes[i]) {
            i += 1;
        }
        if i == ws_start {
            continue; // `[[:space:]]+` — at least one
        }

        let kind = if hay[i..].starts_with("declare_migration") {
            i += "declare_migration".len();
            DeclarationKind::Declare
        } else if hay[i..].starts_with("reclassify_migration") {
            i += "reclassify_migration".len();
            DeclarationKind::Reclassify
        } else {
            continue;
        };

        while i < bytes.len() && is_posix_space(bytes[i]) {
            i += 1;
        }
        if i < bytes.len() && bytes[i] == b'(' {
            return Some((i + 1, kind));
        }
    }
    None
}

/// `^[[:space:]]*[0-9]+` — anchored, as in the awk half. A call whose first argument is not a
/// number is malformed, not skipped.
fn take_version(hay: &str) -> Option<(i64, &str)> {
    let bytes = hay.as_bytes();
    let mut i = 0;
    while i < bytes.len() && is_posix_space(bytes[i]) {
        i += 1;
    }
    let digits_start = i;
    while i < bytes.len() && bytes[i].is_ascii_digit() {
        i += 1;
    }
    if i == digits_start {
        return None;
    }
    hay[digits_start..i].parse().ok().map(|v| (v, &hay[i..]))
}

/// `'[^']*'` — the NEXT quoted run anywhere ahead, unanchored, as in the awk half.
fn take_quoted(hay: &str) -> Option<(&str, &str)> {
    let open = hay.find('\'')?;
    let close = open + 1 + hay[open + 1..].find('\'')?;
    Some((&hay[open + 1..close], &hay[close + 1..]))
}

/// Every version the carried migration set says something about, latest claim winning.
///
/// Iterating in `MIGRATOR` order (version-sorted) and overwriting is what makes a
/// `reclassify_migration` in a later migration supersede the original `declare_migration` — the
/// same latest-wins resolution `migration_current` performs in SQL, done here over claims the
/// database has not seen yet.
pub fn declared_classes(migrator: &Migrator) -> BTreeMap<i64, String> {
    let mut classes = BTreeMap::new();
    for migration in migrator.iter() {
        for call in parse_declarations(&migration.sql) {
            if let ParsedCall::Call { version, class, .. } = call {
                classes.insert(version, class);
            }
        }
    }
    classes
}

/// Decide what an additive-only run applies, and where it stops. Pure: no database, no I/O.
///
/// # Why the plan is a PREFIX and not a filtered subset
///
/// `run_direct` calls `validate_applied_migrations` (`sqlx-core-0.8.6/src/migrate/migrator.rs:161`),
/// which errors `VersionMissing` for any migration recorded in `_sqlx_migrations` that is absent
/// from the list it is given. So handing it "just the additive ones" fails on every database that
/// has ever migrated. Truncating at the halt keeps history intact — and every already-applied
/// migration is retained regardless of where it sits, because an out-of-order merge can leave one
/// with a version above the halt.
///
/// Retained-but-applied migrations are not re-run: `run_direct`'s loop checksum-validates them and
/// moves on (`migrator.rs:173-178`).
pub fn plan_additive_run(migrator: &Migrator, applied: &HashSet<i64>) -> AdditivePlan {
    let classes = declared_classes(migrator);

    let mut halt = None;
    for migration in migrator.iter() {
        if migration.migration_type.is_down_migration() || applied.contains(&migration.version) {
            continue;
        }
        let reason = match classes.get(&migration.version).map(String::as_str) {
            Some(ADDITIVE) => continue,
            Some(SHAPE_BREAKING) => HaltReason::ShapeBreaking,
            Some(other) => HaltReason::Unrecognized(other.to_string()),
            None => HaltReason::Undeclared,
        };
        halt = Some(Halt {
            version: migration.version,
            description: migration.description.to_string(),
            reason,
        });
        break;
    }

    let cutoff = halt.as_ref().map(|h| h.version);
    let migrations: Vec<Migration> = migrator
        .iter()
        .filter(|m| applied.contains(&m.version) || cutoff.is_none_or(|c| m.version < c))
        .cloned()
        .collect();
    let to_apply = migrations
        .iter()
        .filter(|m| !m.migration_type.is_down_migration() && !applied.contains(&m.version))
        .map(|m| m.version)
        .collect();

    AdditivePlan {
        migrations,
        to_apply,
        halt,
    }
}

/// Apply the leading run of additive migrations, recording each apply's outcome, and stop at the
/// first that is not additive.
///
/// This is what the deploy calls. `run_with_ledger` — apply everything — remains what a developer
/// and an operator call, because both of those are the environments where taking a shape-breaking
/// migration is a deliberate act rather than a side effect.
///
/// # The read outside the lock
///
/// The applied set is read before `run_direct` takes sqlx's advisory lock, so a concurrent build
/// could apply something in between. That can only make the plan STALER, never bolder: truncation
/// exclusively removes migrations, so the worst case is `VersionMissing` from sqlx's own
/// validation — a loud failure, never a shape-breaking migration applied by surprise.
pub async fn run_additive_only(
    migrator: &Migrator,
    conn: &mut PgConnection,
) -> Result<AdditiveRun, MigrateError> {
    conn.ensure_migrations_table().await?;
    let applied: HashSet<i64> = conn
        .list_applied_migrations()
        .await?
        .into_iter()
        .map(|m| m.version)
        .collect();

    let plan = plan_additive_run(migrator, &applied);

    if !plan.to_apply.is_empty() {
        let truncated = Migrator {
            migrations: Cow::Owned(plan.migrations),
            ignore_missing: migrator.ignore_missing,
            locking: migrator.locking,
            no_tx: migrator.no_tx,
        };
        let mut recorder = LedgerRecorder::new(conn);
        truncated.run_direct(&mut recorder).await?;
    }

    Ok(AdditiveRun {
        applied: plan.to_apply,
        halt: plan.halt,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::migrate::MigrationType;

    /// The shared statement of the parsing rule. The awk half reads the same file through
    /// `.github/scripts/test-migration-declaration-parity.sh`. Neither harness checks the other
    /// directly; both check the corpus, which is what makes them agree.
    const CORPUS: &str = include_str!("../../../scripts/migration-declaration-corpus.txt");

    struct Case {
        name: String,
        sql: String,
        expect: Vec<String>,
    }

    /// Render a parse the way the corpus writes it, so the two sides compare strings rather than
    /// two different notions of equality.
    fn render(call: &ParsedCall) -> String {
        match call {
            ParsedCall::Malformed => "!MALFORMED".to_string(),
            ParsedCall::Call {
                kind,
                version,
                class,
            } => {
                let kind = match kind {
                    DeclarationKind::Declare => "declare",
                    DeclarationKind::Reclassify => "reclassify",
                };
                format!("{kind} {version} {class}").trim_end().to_string()
            }
        }
    }

    fn parse_corpus() -> Vec<Case> {
        let mut cases: Vec<Case> = Vec::new();
        for line in CORPUS.lines() {
            let trimmed = line.trim_start();
            if trimmed.is_empty() || trimmed.starts_with('#') {
                continue;
            }
            if let Some(name) = trimmed.strip_prefix("=== ") {
                cases.push(Case {
                    name: name.trim().to_string(),
                    sql: String::new(),
                    expect: Vec::new(),
                });
            } else if let Some(sql) = trimmed.strip_prefix("sql:") {
                let case = cases
                    .last_mut()
                    .expect("`sql:` before any `===` case header");
                case.sql.push_str(sql.strip_prefix(' ').unwrap_or(sql));
                case.sql.push('\n');
            } else if let Some(expect) = trimmed.strip_prefix("expect:") {
                let case = cases
                    .last_mut()
                    .expect("`expect:` before any `===` case header");
                case.expect.push(expect.trim().to_string());
            } else {
                panic!("corpus line is neither a case header, `sql:`, nor `expect:`: {line}");
            }
        }
        cases
    }

    #[test]
    fn declaration_corpus_matches_the_shell() {
        let cases = parse_corpus();

        // A corpus that parses to nothing would let this test pass while checking nothing — the
        // exact failure the containment corpus was written to refuse.
        assert!(
            cases.len() >= 20,
            "corpus parsed to only {} cases — did the format change?",
            cases.len()
        );

        for case in &cases {
            let got: Vec<String> = parse_declarations(&case.sql).iter().map(render).collect();
            assert_eq!(
                got, case.expect,
                "corpus case `{}` disagrees with the Rust parser",
                case.name
            );
        }
    }

    #[test]
    fn the_parser_is_live_before_any_corpus_verdict_is_trusted() {
        // A parser that always returned an empty vec would satisfy the negative half of the corpus
        // silently. A corpus is not a control.
        assert_eq!(
            parse_declarations("SELECT declare_migration(1, 'additive', 'why');"),
            vec![ParsedCall::Call {
                kind: DeclarationKind::Declare,
                version: 1,
                class: ADDITIVE.to_string(),
            }]
        );
        assert!(parse_declarations("CREATE TABLE t (id int);").is_empty());
    }

    fn migration(version: i64, sql: &str) -> Migration {
        Migration::new(
            version,
            Cow::Owned(format!("m{version}")),
            MigrationType::Simple,
            Cow::Owned(sql.to_string()),
            false,
        )
    }

    fn declares(version: i64, class: &str) -> String {
        format!("SELECT declare_migration({version}, '{class}', 'because');")
    }

    fn migrator(migrations: Vec<Migration>) -> Migrator {
        Migrator {
            migrations: Cow::Owned(migrations),
            ..Migrator::DEFAULT
        }
    }

    #[test]
    fn an_all_additive_pending_set_applies_whole() {
        let m = migrator(vec![
            migration(1, &declares(1, ADDITIVE)),
            migration(2, &declares(2, ADDITIVE)),
            migration(3, &declares(3, ADDITIVE)),
        ]);
        let plan = plan_additive_run(&m, &HashSet::from([1]));

        assert_eq!(plan.to_apply, vec![2, 3]);
        assert!(plan.halt.is_none());
    }

    #[test]
    fn the_run_halts_at_the_first_shape_breaking_migration() {
        let m = migrator(vec![
            migration(1, &declares(1, ADDITIVE)),
            migration(2, &declares(2, ADDITIVE)),
            migration(3, &declares(3, SHAPE_BREAKING)),
            migration(4, &declares(4, ADDITIVE)),
        ]);
        let plan = plan_additive_run(&m, &HashSet::from([1]));

        // 4 is additive and later. It is NOT reached — halting means stopping, not skipping, or a
        // migration would apply against a schema its predecessor never made.
        assert_eq!(plan.to_apply, vec![2]);
        let halt = plan
            .halt
            .expect("a shape-breaking migration must halt the run");
        assert_eq!(halt.version, 3);
        assert_eq!(halt.reason, HaltReason::ShapeBreaking);
    }

    #[test]
    fn an_undeclared_migration_halts_the_run() {
        let m = migrator(vec![
            migration(1, &declares(1, ADDITIVE)),
            migration(2, "CREATE TABLE t (id int);"),
        ]);
        let plan = plan_additive_run(&m, &HashSet::from([1]));

        assert!(plan.to_apply.is_empty());
        assert_eq!(plan.halt.unwrap().reason, HaltReason::Undeclared);
    }

    #[test]
    fn an_unrecognised_class_token_halts_and_names_itself() {
        let m = migrator(vec![migration(1, &declares(1, "destructive"))]);
        let plan = plan_additive_run(&m, &HashSet::new());

        assert!(plan.to_apply.is_empty());
        assert_eq!(
            plan.halt.unwrap().reason,
            HaltReason::Unrecognized("destructive".to_string())
        );
    }

    #[test]
    fn a_version_declared_by_another_migration_routes_on_that_declaration() {
        // The backfill shape: 1 says nothing about itself and is declared by 2. The set-shaped rule
        // is what makes the 148 pre-mechanism migrations routable at all.
        let m = migrator(vec![
            migration(1, "CREATE TABLE t (id int);"),
            migration(
                2,
                &format!("{}{}", declares(2, ADDITIVE), declares(1, ADDITIVE)),
            ),
        ]);
        let plan = plan_additive_run(&m, &HashSet::new());

        assert_eq!(plan.to_apply, vec![1, 2]);
        assert!(plan.halt.is_none());
    }

    #[test]
    fn a_later_reclassification_wins_over_the_original_declaration() {
        let m = migrator(vec![
            migration(1, &declares(1, ADDITIVE)),
            migration(
                2,
                "SELECT reclassify_migration(1, 'shape-breaking', 'Re-read: the return type moved.');",
            ),
        ]);
        let plan = plan_additive_run(&m, &HashSet::new());

        assert!(plan.to_apply.is_empty());
        let halt = plan
            .halt
            .expect("the reclassification must route, not be ignored");
        assert_eq!(halt.version, 1);
        assert_eq!(halt.reason, HaltReason::ShapeBreaking);
    }

    #[test]
    fn the_plan_keeps_every_applied_migration_so_sqlx_validation_still_passes() {
        // An out-of-order merge can leave an APPLIED migration above the halt. Dropping it from
        // the truncated list would make `validate_applied_migrations` error `VersionMissing`, which
        // turns a routine additive deploy into a failure with an unrelated message.
        let m = migrator(vec![
            migration(1, &declares(1, ADDITIVE)),
            migration(2, &declares(2, SHAPE_BREAKING)),
            migration(3, &declares(3, ADDITIVE)),
        ]);
        let plan = plan_additive_run(&m, &HashSet::from([1, 3]));

        let kept: Vec<i64> = plan.migrations.iter().map(|m| m.version).collect();
        assert_eq!(kept, vec![1, 3], "the applied 3 must survive truncation");
        assert!(
            plan.to_apply.is_empty(),
            "and must not be re-applied by being kept"
        );
        assert_eq!(plan.halt.unwrap().version, 2);
    }

    #[test]
    fn a_header_that_documents_a_declaration_does_not_make_one() {
        // Migration headers here quote SQL freely, and `DEPLOYING.md`'s worked example is a
        // `declare_migration` call. A header claiming `additive` over a file that declares nothing
        // must halt.
        let m = migrator(vec![migration(
            1,
            "-- SELECT declare_migration(1, 'additive', 'Call this to declare.');\n\
             CREATE TABLE t (id int);",
        )]);
        let plan = plan_additive_run(&m, &HashSet::new());

        assert_eq!(plan.halt.unwrap().reason, HaltReason::Undeclared);
    }

    #[test]
    fn the_real_migration_set_is_fully_declared_and_entirely_routable() {
        // The strongest available check that the two parsers agree, and the one a fixture cannot
        // give: the Rust half, pointed at the REAL corpus the awk half passes in CI. A parser green
        // on fixtures and wrong about the repo is a shape this goal has already met twice.
        let classes = declared_classes(&crate::MIGRATOR);

        let undeclared: Vec<i64> = crate::MIGRATOR
            .iter()
            .filter(|m| !classes.contains_key(&m.version))
            .map(|m| m.version)
            .collect();
        assert!(
            undeclared.is_empty(),
            "the Rust parser finds no declaration for {undeclared:?}, which CI's awk half passes"
        );

        let unrecognised: Vec<(&i64, &String)> = classes
            .iter()
            .filter(|(_, c)| c.as_str() != ADDITIVE && c.as_str() != SHAPE_BREAKING)
            .collect();
        assert!(
            unrecognised.is_empty(),
            "class tokens outside the vocabulary: {unrecognised:?}"
        );
    }
}
