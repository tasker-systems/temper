#![cfg(feature = "artifact-tests")]
//! Ledger envelope invariants (payload-first design §1): append-only enforcement and the
//! root-correlation convention (a root event's correlation_id is its own id). Isolated ephemeral DB
//! via `temper_substrate::MIGRATOR`.

mod common;

use temper_substrate::scenario::bootseed;

#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn ledger_is_append_only_and_roots_self_correlate(pool: sqlx::PgPool) {
    // the boot-seed fires lens_created events through _event_append — real ledger rows to test against
    bootseed::seed_system(&pool).await.unwrap();

    let (id, correlation): (uuid::Uuid, Option<uuid::Uuid>) =
        sqlx::query_as("SELECT id, correlation_id FROM kb_events ORDER BY id LIMIT 1")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(
        correlation,
        Some(id),
        "a root event's correlation_id is its own id"
    );

    let upd = sqlx::query("UPDATE kb_events SET payload_version = 2 WHERE id = $1")
        .bind(id)
        .execute(&pool)
        .await;
    let err = upd.expect_err("UPDATE must be rejected").to_string();
    assert!(err.contains("append-only"), "got: {err}");

    let del = sqlx::query("DELETE FROM kb_events WHERE id = $1")
        .bind(id)
        .execute(&pool)
        .await;
    assert!(del.is_err(), "DELETE must be rejected");
}
