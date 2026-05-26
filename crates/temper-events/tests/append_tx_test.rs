#![cfg(feature = "test-db")]
//! `append_event_tx` appends within a caller-owned transaction.

use temper_events::{append_event_tx, EventToWrite, EventType, MIGRATOR};

#[sqlx::test(migrator = "MIGRATOR")]
async fn append_event_tx_commits_with_caller_transaction(pool: sqlx::PgPool) {
    let profile_id = uuid::Uuid::now_v7();
    // kb_profiles requires a unique slug (NOT NULL) — generate one from the id.
    let slug = format!("test-{}", profile_id.as_hyphenated());
    sqlx::query("INSERT INTO kb_profiles (id, display_name, slug) VALUES ($1, $2, $3)")
        .bind(profile_id)
        .bind("Test")
        .bind(&slug)
        .execute(&pool)
        .await
        .expect("seed profile");

    // Task 5 (which seeds relationship_asserted into kb_event_types) has not
    // run yet — seed the row here so this test is self-contained and green.
    sqlx::query(
        "INSERT INTO kb_event_types (name) VALUES ('relationship_asserted') ON CONFLICT (name) DO NOTHING",
    )
    .execute(&pool)
    .await
    .expect("seed relationship_asserted event type");

    let topic = uuid::Uuid::parse_str("019e3d6f-2300-7000-8000-000000000040").unwrap();
    let scope = uuid::Uuid::parse_str("019e3d6f-2300-7000-8000-000000000010").unwrap();

    let mut tx = pool.begin().await.unwrap();
    let write = EventToWrite::new_root(
        EventType::RelationshipAsserted,
        profile_id,
        topic,
        scope,
        serde_json::json!({"probe": true}),
        chrono::Utc::now(),
    );
    let event = append_event_tx(&mut tx, write).await.expect("append in tx");
    tx.commit().await.unwrap();

    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM kb_events WHERE id = $1")
        .bind(event.id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(count, 1);
}
