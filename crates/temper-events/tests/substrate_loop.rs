#![cfg(feature = "test-db")]

use chrono::Utc;
use serde_json::json;
use sqlx::PgPool;
use uuid::Uuid;

use temper_events::{
    append_event, EventReference, EventToWrite, EventType, LedgerError, ReferenceKind, MIGRATOR,
};

// Seeded by migrations/20260330000002_seed.sql (kb_profiles) and
// migrations/20260522000001_event_ledger_unification.sql (topic, scope).
const SYSTEM_PROFILE_ID: Uuid = uuid::uuid!("00000000-0000-0000-0004-000000000001");
const BOOTSTRAP_TOPIC_ID: Uuid = uuid::uuid!("019e3d6f-2300-7000-8000-000000000040");
const PUBLIC_SCOPE_ID: Uuid = uuid::uuid!("019e3d6f-2300-7000-8000-000000000010");

fn mutation(id: Uuid, supersedes: Vec<Uuid>, correlation_id: Uuid) -> EventToWrite {
    EventToWrite {
        id,
        event_type: EventType::ConceptMutated,
        emitter_profile_id: SYSTEM_PROFILE_ID,
        topic_id: BOOTSTRAP_TOPIC_ID,
        scope_id: PUBLIC_SCOPE_ID,
        payload: json!({ "definition": "x" }),
        metadata: json!({}),
        references: supersedes
            .into_iter()
            .map(|event_id| EventReference {
                kind: ReferenceKind::Supersedes,
                event_id,
            })
            .collect(),
        correlation_id,
        occurred_at: Utc::now(),
    }
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn append_writes_to_ledger(pool: PgPool) {
    let payload = json!({ "definition": "the disciplined ledger" });
    let write = EventToWrite::new_root(
        EventType::ConceptCreated,
        SYSTEM_PROFILE_ID,
        BOOTSTRAP_TOPIC_ID,
        PUBLIC_SCOPE_ID,
        payload.clone(),
        Utc::now(),
    );
    let event = append_event(&pool, write.clone())
        .await
        .expect("append_event");

    assert_eq!(event.id, write.id);
    assert_eq!(event.correlation_id, write.id);
    assert_eq!(event.emitter_profile_id, SYSTEM_PROFILE_ID);
    assert_eq!(event.payload, payload);

    // Runtime query: test-target macros can't be cached by `cargo sqlx prepare`
    // (the test-fixture convention).
    let row_count: i64 = sqlx::query_scalar("SELECT count(*) FROM kb_events WHERE id = $1")
        .bind(write.id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(row_count, 1);
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn unknown_profile_errors(pool: PgPool) {
    let bogus = Uuid::now_v7();
    let write = EventToWrite::new_root(
        EventType::ConceptCreated,
        bogus,
        BOOTSTRAP_TOPIC_ID,
        PUBLIC_SCOPE_ID,
        json!({ "definition": "x" }),
        Utc::now(),
    );
    let err = append_event(&pool, write).await.unwrap_err();
    assert!(matches!(err, LedgerError::UnknownProfile(id) if id == bogus));
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn unknown_topic_errors(pool: PgPool) {
    let bogus = Uuid::now_v7();
    let write = EventToWrite::new_root(
        EventType::ConceptCreated,
        SYSTEM_PROFILE_ID,
        bogus,
        PUBLIC_SCOPE_ID,
        json!({ "definition": "x" }),
        Utc::now(),
    );
    let err = append_event(&pool, write).await.unwrap_err();
    assert!(matches!(err, LedgerError::UnknownTopic(id) if id == bogus));
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn unknown_scope_errors(pool: PgPool) {
    let bogus = Uuid::now_v7();
    let write = EventToWrite::new_root(
        EventType::ConceptCreated,
        SYSTEM_PROFILE_ID,
        BOOTSTRAP_TOPIC_ID,
        bogus,
        json!({ "definition": "x" }),
        Utc::now(),
    );
    let err = append_event(&pool, write).await.unwrap_err();
    assert!(matches!(err, LedgerError::UnknownScope(id) if id == bogus));
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn dangling_reference_errors(pool: PgPool) {
    let bogus_event = Uuid::now_v7();
    let id = Uuid::now_v7();
    let err = append_event(&pool, mutation(id, vec![bogus_event], id))
        .await
        .unwrap_err();
    assert!(matches!(
        err,
        LedgerError::DanglingReference { event_id, kind: ReferenceKind::Supersedes }
            if event_id == bogus_event
    ));
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn concept_created_with_supersedes_errors(pool: PgPool) {
    let root = EventToWrite::new_root(
        EventType::ConceptCreated,
        SYSTEM_PROFILE_ID,
        BOOTSTRAP_TOPIC_ID,
        PUBLIC_SCOPE_ID,
        json!({ "definition": "root" }),
        Utc::now(),
    );
    append_event(&pool, root.clone()).await.unwrap();

    let id = Uuid::now_v7();
    let bad = EventToWrite {
        event_type: EventType::ConceptCreated,
        ..mutation(id, vec![root.id], id)
    };
    let err = append_event(&pool, bad).await.unwrap_err();
    assert!(matches!(err, LedgerError::SupersedesOnGenesis));
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn concept_mutated_without_supersedes_errors(pool: PgPool) {
    let id = Uuid::now_v7();
    let err = append_event(&pool, mutation(id, vec![], id))
        .await
        .unwrap_err();
    assert!(matches!(err, LedgerError::MissingSupersedes));
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn concept_mutated_with_multiple_supersedes_errors(pool: PgPool) {
    let root = EventToWrite::new_root(
        EventType::ConceptCreated,
        SYSTEM_PROFILE_ID,
        BOOTSTRAP_TOPIC_ID,
        PUBLIC_SCOPE_ID,
        json!({ "definition": "root" }),
        Utc::now(),
    );
    append_event(&pool, root.clone()).await.unwrap();

    let id = Uuid::now_v7();
    let err = append_event(&pool, mutation(id, vec![root.id, root.id], id))
        .await
        .unwrap_err();
    assert!(matches!(err, LedgerError::MultipleSupersedes));
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn ledger_is_append_only(pool: PgPool) {
    let root = EventToWrite::new_root(
        EventType::ConceptCreated,
        SYSTEM_PROFILE_ID,
        BOOTSTRAP_TOPIC_ID,
        PUBLIC_SCOPE_ID,
        json!({ "definition": "trigger-test" }),
        Utc::now(),
    );
    append_event(&pool, root.clone()).await.unwrap();

    let update_err = sqlx::query("UPDATE kb_events SET metadata = $1 WHERE id = $2")
        .bind(json!({ "tampered": true }))
        .bind(root.id)
        .execute(&pool)
        .await
        .unwrap_err();
    assert!(
        update_err
            .to_string()
            .contains("event ledger is append-only"),
        "expected append-only trigger on UPDATE; got: {update_err}"
    );

    let delete_err = sqlx::query("DELETE FROM kb_events WHERE id = $1")
        .bind(root.id)
        .execute(&pool)
        .await
        .unwrap_err();
    assert!(
        delete_err
            .to_string()
            .contains("event ledger is append-only"),
        "expected append-only trigger on DELETE; got: {delete_err}"
    );
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn correlation_id_groups_fan_out(pool: PgPool) {
    let root = EventToWrite::new_root(
        EventType::ConceptCreated,
        SYSTEM_PROFILE_ID,
        BOOTSTRAP_TOPIC_ID,
        PUBLIC_SCOPE_ID,
        json!({ "definition": "fan-out root" }),
        Utc::now(),
    );
    let created = append_event(&pool, root.clone()).await.unwrap();

    for _ in 0..2 {
        let id = Uuid::now_v7();
        append_event(
            &pool,
            mutation(id, vec![created.id], created.correlation_id),
        )
        .await
        .unwrap();
    }

    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM kb_events WHERE correlation_id = $1")
        .bind(created.correlation_id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(count, 3, "root + 2 mutations share one correlation_id");
}
