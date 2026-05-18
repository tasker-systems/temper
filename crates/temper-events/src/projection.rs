use sqlx::PgPool;
use uuid::Uuid;

use crate::errors::LedgerError;
use crate::payloads::ConceptCreatedPayload;
use crate::types::concept::Concept;
use crate::types::event::{Event, EventType};

pub async fn project_concept(pool: &PgPool, event_id: Uuid) -> Result<Concept, LedgerError> {
    let event = load_event(pool, event_id).await?;
    let event_type = resolve_event_type(pool, event.event_type_id).await?;

    match event_type {
        EventType::ConceptCreated => project_created(pool, &event).await,
        EventType::ConceptMutated => unimplemented!("ConceptMutated projection lands in Task 14"),
    }
}

async fn project_created(pool: &PgPool, event: &Event) -> Result<Concept, LedgerError> {
    let payload: ConceptCreatedPayload = serde_json::from_value(event.payload.clone())
        .map_err(|e| LedgerError::Database(sqlx::Error::Decode(Box::new(e))))?;

    let concept_id = Uuid::now_v7();
    let concept = sqlx::query_as!(
        Concept,
        r#"
        INSERT INTO event_substrate.concepts (
            id, current_definition, current_elaboration,
            scope_id, topic_id,
            created_by_event_id, last_event_id, latest_event_recorded_at
        )
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
        RETURNING
            id, current_definition, current_elaboration,
            scope_id, topic_id,
            created_by_event_id, last_event_id, latest_event_recorded_at
        "#,
        concept_id,
        payload.definition,
        payload.elaboration,
        event.scope_id,
        event.topic_id,
        event.id,
        event.id,
        event.recorded_at,
    )
    .fetch_one(pool)
    .await?;

    Ok(concept)
}

async fn load_event(pool: &PgPool, event_id: Uuid) -> Result<Event, LedgerError> {
    sqlx::query_as!(
        Event,
        r#"
        SELECT
            id, event_type_id, emitter_entity_id, topic_id, scope_id,
            payload, metadata, "references", correlation_id,
            occurred_at, recorded_at
        FROM event_substrate.events
        WHERE id = $1
        "#,
        event_id,
    )
    .fetch_optional(pool)
    .await?
    .ok_or(LedgerError::DanglingReference {
        event_id,
        kind: crate::types::event::ReferenceKind::Supersedes,
    })
}

async fn resolve_event_type(pool: &PgPool, event_type_id: Uuid) -> Result<EventType, LedgerError> {
    let name: String = sqlx::query_scalar!(
        "SELECT name FROM event_substrate.event_types WHERE id = $1",
        event_type_id,
    )
    .fetch_optional(pool)
    .await?
    .ok_or_else(|| LedgerError::UnknownEventType(format!("(by id) {event_type_id}")))?;

    match name.as_str() {
        "ConceptCreated" => Ok(EventType::ConceptCreated),
        "ConceptMutated" => Ok(EventType::ConceptMutated),
        other => Err(LedgerError::UnknownEventType(other.to_string())),
    }
}
