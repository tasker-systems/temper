#![cfg(feature = "test-db")]
//! temper-migrate's refusal of an unsafe replay posture (exit 5), and its skip on a database that
//! predates `replay_posture()` (20261203000010).

use sqlx::{Executor, PgPool};
use temper_migrate::replay_posture;

/// This test database's URL: the server's `DATABASE_URL` with the database `sqlx::test` made.
fn url_of(pool: &PgPool) -> String {
    let base = std::env::var("DATABASE_URL").expect("DATABASE_URL names the test server");
    let (server, _) = base
        .rsplit_once('/')
        .expect("DATABASE_URL has a database path");
    let db = pool
        .connect_options()
        .get_database()
        .expect("sqlx::test names its database")
        .to_string();
    format!("{server}/{db}")
}

/// FAILS IF: a fresh database reports a reason, or a database that predates the posture function
/// fails the check instead of reporting nothing.
#[sqlx::test(migrator = "temper_migrate::MIGRATOR")]
async fn a_fresh_database_is_safe(pool: PgPool) {
    let mut conn = pool.acquire().await.unwrap();
    assert!(replay_posture(&mut conn).await.unwrap().is_empty());
}

#[sqlx::test(migrations = false)]
async fn a_database_without_the_posture_function_reports_nothing(pool: PgPool) {
    let mut conn = pool.acquire().await.unwrap();
    assert!(
        replay_posture(&mut conn).await.unwrap().is_empty(),
        "a run halted below 20261203000010 is reported by its halt, not by this check"
    );
}

/// FAILS IF: temper-migrate serves a marked database without refusing, in either mode, or refuses
/// it with another code than 5.
#[sqlx::test(migrator = "temper_migrate::MIGRATOR")]
async fn a_marked_database_is_refused_with_exit_5(pool: PgPool) {
    // The trigger admits the marker only on an empty ledger, so empty it first, as a replay
    // target is.
    pool.execute("TRUNCATE kb_events CASCADE; INSERT INTO kb_replay_target (id) VALUES (true)")
        .await
        .unwrap();
    let mut conn = pool.acquire().await.unwrap();
    assert_eq!(
        replay_posture(&mut conn).await.unwrap(),
        ["kb_replay_target carries the replay-target marker"]
    );
    drop(conn);

    for args in [&["--additive-only"][..], &[][..]] {
        let out = std::process::Command::new(env!("CARGO_BIN_EXE_temper-migrate"))
            .args(args)
            .env("DATABASE_URL", url_of(&pool))
            .output()
            .expect("run temper-migrate");
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert_eq!(out.status.code(), Some(5), "{args:?}: {stderr}");
        assert!(
            stderr.contains("UNSAFE REPLAY POSTURE")
                && stderr.contains("kb_replay_target carries the replay-target marker"),
            "{args:?}: {stderr}"
        );
    }
}
