#![cfg(feature = "artifact-tests")]
//! Invocation envelope + agent-authorship metadata. Each test resets the artifact (01+02 via psql),
//! boot-seeds the system actor, and exercises the new substrate. Serialized via the
//! `temper-next-write` nextest group (it owns the namespace).

mod common;

use temper_next::substrate;

/// Reset the artifact (01+02), connect, boot-seed the system actor. Standard write-path preamble.
async fn setup() -> sqlx::PgPool {
    common::reset_artifact();
    let pool = substrate::connect().await.unwrap();
    temper_next::scenario::bootseed::seed_system(&pool).await.unwrap();
    pool
}

#[tokio::test]
async fn schema_has_invocations_table_and_event_column() {
    let pool = setup().await;
    // kb_events.invocation_id exists and is nullable UUID
    let col: Option<String> = sqlx::query_scalar(
        "SELECT data_type FROM information_schema.columns \
         WHERE table_schema='temper_next' AND table_name='kb_events' AND column_name='invocation_id'",
    )
    .fetch_optional(&pool)
    .await
    .unwrap();
    assert_eq!(col.as_deref(), Some("uuid"), "kb_events.invocation_id must be uuid");

    // kb_invocations table exists
    let tbl: Option<String> = sqlx::query_scalar(
        "SELECT table_name FROM information_schema.tables \
         WHERE table_schema='temper_next' AND table_name='kb_invocations'",
    )
    .fetch_optional(&pool)
    .await
    .unwrap();
    assert_eq!(tbl.as_deref(), Some("kb_invocations"), "kb_invocations table must exist");
}

#[tokio::test]
async fn event_append_persists_metadata_and_invocation() {
    let pool = setup().await;
    let emitter: uuid::Uuid =
        sqlx::query_scalar("SELECT id FROM kb_entities WHERE name='system'")
            .fetch_one(&pool).await.unwrap();
    let inv = uuid::Uuid::now_v7();
    // Call _event_append directly with named args for the two new params.
    let ev: uuid::Uuid = sqlx::query_scalar(
        "SELECT _event_append('cogmap_seeded', $1, NULL, NULL, '{}'::jsonb, \
                p_metadata => $2::jsonb, p_invocation => $3)",
    )
    .bind(emitter)
    .bind(serde_json::json!({"reasoning": "SENTINEL"}))
    .bind(inv)
    .fetch_one(&pool).await.unwrap();

    let (meta, got_inv): (serde_json::Value, Option<uuid::Uuid>) = sqlx::query_as(
        "SELECT metadata, invocation_id FROM kb_events WHERE id=$1",
    )
    .bind(ev)
    .fetch_one(&pool).await.unwrap();
    assert_eq!(meta["reasoning"], "SENTINEL");
    assert_eq!(got_inv, Some(inv));
}
