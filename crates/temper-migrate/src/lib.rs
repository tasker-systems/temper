//! `temper-migrate` — the migration chain, and the runner that records what happens to it.
//!
//! # The one declaration of `sqlx::migrate!`
//!
//! [`MIGRATOR`] used to be declared three times — in temper-substrate, temper-api and
//! temper-services — each embedding the same `migrations/` directory. It lives here once now, and
//! those three crates re-export it, so the ~930 `#[sqlx::test(migrator = "temper_api::MIGRATOR")]`
//! and friends across the test suites keep working unchanged: `sqlx::test`'s `migrator` argument is
//! a path string and resolves through a re-export.
//!
//! # Why this is its own crate, and what must stay out of it
//!
//! The deploy applies its own additive schema during the build phase, before the API functions are
//! built (`scripts/vercel-build.sh`). This crate is what that step compiles. Its dependency list is
//! therefore a budget, not an accident — see the note in `Cargo.toml`. Adding anything heavy here
//! puts it on the critical path *ahead of the schema being applied*, which is the position that
//! most wants to stay cheap.
//!
//! The applying itself is still sqlx's; [`ledger`] is a [`Migrate`](sqlx::migrate::Migrate)
//! decorator, not a reimplementation.

pub mod ledger;

use anyhow::Result;
use sqlx::{PgConnection, PgPool};

/// The shared sqlx migration chain (workspace `migrations/`).
///
/// Re-exported by temper-substrate, temper-api and temper-services so that
/// `#[sqlx::test(migrator = "…::MIGRATOR")]` keeps resolving under every crate that already named
/// it. Exposed for self-contained integration tests that spin up an isolated ephemeral database
/// with the full chain applied.
pub static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("../../migrations");

/// Connect to the database the migration runner should act on.
///
/// The connection's `search_path` is the database default (`public`) in production, dev, and tests.
/// No per-connection `SET search_path` is needed.
///
/// `scripts/vercel-build.sh` chooses between Neon's pooled and unpooled endpoints and exports the
/// winner as `DATABASE_URL`, so the Vercel-specific variable knowledge stays in that script and
/// this side has one way to be told.
pub async fn connect() -> Result<PgPool> {
    let url = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgresql://temper:temper@localhost:5437/temper_development".into());
    Ok(PgPool::connect(&url).await?)
}

/// Why this database is unsafe to serve, empty when it is safe (`replay_posture()`,
/// 20261203000010): the replay-target marker, a database or role default for `temper.replaying`,
/// or this session carrying it. The guards stand down only under `_replay_mode()`, which needs the
/// marker, so a deployed database must never hold one; the deploy refuses on any reason.
///
/// Call it on a connection outside a transaction, so a `temper.replaying` it reports is
/// session-level. A database a halted additive run left below 20261203000010 has no
/// `replay_posture()` yet, and no marker table either: it reports nothing, so the run's own exit
/// (the halt) is what the caller sees.
pub async fn replay_posture(conn: &mut PgConnection) -> Result<Vec<String>> {
    let present = sqlx::query_scalar!(
        r#"SELECT to_regprocedure('public.replay_posture()') IS NOT NULL AS "present!""#
    )
    .fetch_one(&mut *conn)
    .await?;
    if !present {
        return Ok(Vec::new());
    }
    Ok(
        sqlx::query_scalar!(r#"SELECT replay_posture() AS "violations!""#)
            .fetch_one(conn)
            .await?,
    )
}
