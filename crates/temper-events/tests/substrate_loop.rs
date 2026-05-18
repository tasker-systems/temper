#![cfg(feature = "test-db")]

use sqlx::PgPool;
use temper_events::{create_entity, MIGRATOR};

#[sqlx::test(migrator = "MIGRATOR")]
async fn create_entity_creates_default_profile(pool: PgPool) {
    let (entity, profile) = create_entity(&pool, "alice").await.expect("create_entity");

    assert_eq!(entity.name, "alice");
    assert_eq!(entity.profile_id, profile.id);
    assert_eq!(profile.name, "default profile for alice");
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn move_entity_to_other_profile(pool: PgPool) {
    use temper_events::move_entity;

    let (entity, source_profile) = create_entity(&pool, "alice").await.unwrap();
    let (_, target_profile) = create_entity(&pool, "bob").await.unwrap();

    let moved = move_entity(&pool, entity.id, target_profile.id)
        .await
        .unwrap();
    assert_eq!(moved.profile_id, target_profile.id);

    // Source profile still exists, just unreferenced.
    let source_still_present: bool = sqlx::query_scalar!(
        "SELECT EXISTS (SELECT 1 FROM event_substrate.profiles WHERE id = $1)",
        source_profile.id,
    )
    .fetch_one(&pool)
    .await
    .unwrap()
    .unwrap_or(false);
    assert!(
        source_still_present,
        "move_entity must not auto-discard source profile"
    );
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn discard_empty_profile_succeeds(pool: PgPool) {
    use temper_events::{discard_profile, move_entity};

    let (entity, source_profile) = create_entity(&pool, "alice").await.unwrap();
    let (_, target_profile) = create_entity(&pool, "bob").await.unwrap();
    move_entity(&pool, entity.id, target_profile.id)
        .await
        .unwrap();

    discard_profile(&pool, source_profile.id).await.unwrap();

    let still_present: bool = sqlx::query_scalar!(
        "SELECT EXISTS (SELECT 1 FROM event_substrate.profiles WHERE id = $1)",
        source_profile.id,
    )
    .fetch_one(&pool)
    .await
    .unwrap()
    .unwrap_or(false);
    assert!(!still_present);
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn discard_profile_with_entities_errors(pool: PgPool) {
    use temper_events::{discard_profile, LedgerError};

    let (_, profile) = create_entity(&pool, "alice").await.unwrap();
    let err = discard_profile(&pool, profile.id).await.unwrap_err();
    assert!(matches!(err, LedgerError::ProfileNotEmpty(id) if id == profile.id));
}

use chrono::Utc;
use serde_json::json;
use temper_events::{append_event, EventToWrite, EventType};

const PUBLIC_SCOPE_ID: uuid::Uuid = uuid::uuid!("019e3d6f-2300-7000-8000-000000000010");
const SYSTEM_ENTITY_ID: uuid::Uuid = uuid::uuid!("019e3d6f-2300-7000-8000-000000000030");
const BOOTSTRAP_TOPIC_ID: uuid::Uuid = uuid::uuid!("019e3d6f-2300-7000-8000-000000000040");

#[sqlx::test(migrator = "MIGRATOR")]
async fn append_concept_created_writes_to_ledger(pool: PgPool) {
    let payload = json!({
        "definition": "the digital cognitive map artifact model",
        "elaboration": "events + richly-related resources; markdown is one projection",
    });
    let write = EventToWrite::new_root(
        EventType::ConceptCreated,
        SYSTEM_ENTITY_ID,
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
    assert_eq!(event.emitter_entity_id, SYSTEM_ENTITY_ID);
    assert_eq!(event.payload, payload);

    // The row is in the ledger.
    let row_count: i64 = sqlx::query_scalar!(
        "SELECT count(*) FROM event_substrate.events WHERE id = $1",
        write.id,
    )
    .fetch_one(&pool)
    .await
    .unwrap()
    .unwrap_or(0);
    assert_eq!(row_count, 1);
}

use temper_events::LedgerError;
use uuid::Uuid;

#[sqlx::test(migrator = "MIGRATOR")]
async fn unknown_entity_errors(pool: PgPool) {
    let bogus = Uuid::now_v7();
    let write = EventToWrite::new_root(
        EventType::ConceptCreated,
        bogus,
        BOOTSTRAP_TOPIC_ID,
        PUBLIC_SCOPE_ID,
        json!({"definition": "x"}),
        Utc::now(),
    );
    let err = append_event(&pool, write).await.unwrap_err();
    assert!(matches!(err, LedgerError::UnknownEntity(id) if id == bogus));
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn unknown_topic_errors(pool: PgPool) {
    let bogus = Uuid::now_v7();
    let write = EventToWrite::new_root(
        EventType::ConceptCreated,
        SYSTEM_ENTITY_ID,
        bogus,
        PUBLIC_SCOPE_ID,
        json!({"definition": "x"}),
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
        SYSTEM_ENTITY_ID,
        BOOTSTRAP_TOPIC_ID,
        bogus,
        json!({"definition": "x"}),
        Utc::now(),
    );
    let err = append_event(&pool, write).await.unwrap_err();
    assert!(matches!(err, LedgerError::UnknownScope(id) if id == bogus));
}

use temper_events::{EventReference, ReferenceKind};

#[sqlx::test(migrator = "MIGRATOR")]
async fn dangling_reference_errors(pool: PgPool) {
    let bogus_event = Uuid::now_v7();
    let id = Uuid::now_v7();
    let write = EventToWrite {
        id,
        event_type: EventType::ConceptMutated,
        emitter_entity_id: SYSTEM_ENTITY_ID,
        topic_id: BOOTSTRAP_TOPIC_ID,
        scope_id: PUBLIC_SCOPE_ID,
        payload: json!({"definition": "x"}),
        metadata: json!({}),
        references: vec![EventReference {
            kind: ReferenceKind::Supersedes,
            event_id: bogus_event,
        }],
        correlation_id: id,
        occurred_at: Utc::now(),
    };
    let err = append_event(&pool, write).await.unwrap_err();
    assert!(matches!(
        err,
        LedgerError::DanglingReference { event_id, kind: ReferenceKind::Supersedes }
            if event_id == bogus_event
    ));
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn append_concept_created_with_supersedes_errors(pool: PgPool) {
    // First, write a real ConceptCreated so the Supersedes target exists.
    let root = EventToWrite::new_root(
        EventType::ConceptCreated,
        SYSTEM_ENTITY_ID,
        BOOTSTRAP_TOPIC_ID,
        PUBLIC_SCOPE_ID,
        json!({"definition": "root"}),
        Utc::now(),
    );
    append_event(&pool, root.clone()).await.unwrap();

    let id = Uuid::now_v7();
    let bad = EventToWrite {
        id,
        event_type: EventType::ConceptCreated,
        emitter_entity_id: SYSTEM_ENTITY_ID,
        topic_id: BOOTSTRAP_TOPIC_ID,
        scope_id: PUBLIC_SCOPE_ID,
        payload: json!({"definition": "x"}),
        metadata: json!({}),
        references: vec![EventReference {
            kind: ReferenceKind::Supersedes,
            event_id: root.id,
        }],
        correlation_id: id,
        occurred_at: Utc::now(),
    };
    let err = append_event(&pool, bad).await.unwrap_err();
    assert!(matches!(err, LedgerError::SupersedesOnGenesis));
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn append_concept_mutated_without_supersedes_errors(pool: PgPool) {
    let id = Uuid::now_v7();
    let bad = EventToWrite {
        id,
        event_type: EventType::ConceptMutated,
        emitter_entity_id: SYSTEM_ENTITY_ID,
        topic_id: BOOTSTRAP_TOPIC_ID,
        scope_id: PUBLIC_SCOPE_ID,
        payload: json!({"definition": "x"}),
        metadata: json!({}),
        references: vec![],
        correlation_id: id,
        occurred_at: Utc::now(),
    };
    let err = append_event(&pool, bad).await.unwrap_err();
    assert!(matches!(err, LedgerError::MissingSupersedes));
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn append_concept_mutated_with_multiple_supersedes_errors(pool: PgPool) {
    let root = EventToWrite::new_root(
        EventType::ConceptCreated,
        SYSTEM_ENTITY_ID,
        BOOTSTRAP_TOPIC_ID,
        PUBLIC_SCOPE_ID,
        json!({"definition": "root"}),
        Utc::now(),
    );
    append_event(&pool, root.clone()).await.unwrap();

    let id = Uuid::now_v7();
    let bad = EventToWrite {
        id,
        event_type: EventType::ConceptMutated,
        emitter_entity_id: SYSTEM_ENTITY_ID,
        topic_id: BOOTSTRAP_TOPIC_ID,
        scope_id: PUBLIC_SCOPE_ID,
        payload: json!({"definition": "x"}),
        metadata: json!({}),
        references: vec![
            EventReference {
                kind: ReferenceKind::Supersedes,
                event_id: root.id,
            },
            EventReference {
                kind: ReferenceKind::Supersedes,
                event_id: root.id,
            },
        ],
        correlation_id: id,
        occurred_at: Utc::now(),
    };
    let err = append_event(&pool, bad).await.unwrap_err();
    assert!(matches!(err, LedgerError::MultipleSupersedes));
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn events_table_is_append_only(pool: PgPool) {
    let root = EventToWrite::new_root(
        EventType::ConceptCreated,
        SYSTEM_ENTITY_ID,
        BOOTSTRAP_TOPIC_ID,
        PUBLIC_SCOPE_ID,
        json!({"definition": "trigger-test"}),
        Utc::now(),
    );
    append_event(&pool, root.clone()).await.unwrap();

    let update_err = sqlx::query!(
        "UPDATE event_substrate.events SET metadata = $1 WHERE id = $2",
        json!({"tampered": true}),
        root.id,
    )
    .execute(&pool)
    .await
    .unwrap_err();
    assert!(
        update_err
            .to_string()
            .contains("event ledger is append-only"),
        "expected append-only trigger to raise; got: {}",
        update_err
    );

    let delete_err = sqlx::query!("DELETE FROM event_substrate.events WHERE id = $1", root.id,)
        .execute(&pool)
        .await
        .unwrap_err();
    assert!(
        delete_err
            .to_string()
            .contains("event ledger is append-only"),
        "expected append-only trigger to raise on DELETE; got: {}",
        delete_err
    );
}

use temper_events::project_concept;

#[sqlx::test(migrator = "MIGRATOR")]
async fn append_concept_created_projects_to_concept(pool: PgPool) {
    let root = EventToWrite::new_root(
        EventType::ConceptCreated,
        SYSTEM_ENTITY_ID,
        BOOTSTRAP_TOPIC_ID,
        PUBLIC_SCOPE_ID,
        json!({
            "definition": "the LLM-wiki is the wrong artifact model",
            "elaboration": "markdown is one lossy projection of a richer substrate",
        }),
        Utc::now(),
    );
    let event = append_event(&pool, root.clone()).await.unwrap();

    let concept = project_concept(&pool, event.id)
        .await
        .expect("project_concept");

    assert_eq!(
        concept.current_definition,
        "the LLM-wiki is the wrong artifact model"
    );
    assert_eq!(
        concept.current_elaboration.as_deref(),
        Some("markdown is one lossy projection of a richer substrate")
    );
    assert_eq!(concept.scope_id, PUBLIC_SCOPE_ID);
    assert_eq!(concept.topic_id, BOOTSTRAP_TOPIC_ID);
    assert_eq!(concept.created_by_event_id, event.id);
    assert_eq!(concept.last_event_id, event.id);
}
