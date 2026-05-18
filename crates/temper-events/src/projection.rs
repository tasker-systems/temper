use sqlx::PgPool;
use uuid::Uuid;

use crate::errors::LedgerError;
use crate::payloads::ConceptCreatedPayload;
use crate::types::concept::Concept;
use crate::types::event::{Event, EventType};

// Concept identity is derived from the genesis event id so that rebuild_concept
// (replay) always produces the same concept id without any extra plumbing.
// Using Uuid::now_v7() here would make replay non-deterministic.

pub async fn project_concept(pool: &PgPool, event_id: Uuid) -> Result<Concept, LedgerError> {
    let event = load_event(pool, event_id).await?;
    let event_type = resolve_event_type(pool, event.event_type_id).await?;

    // Idempotency short-circuit: if a concept already has this event as
    // its last_event_id, return it unchanged. Makes the projection function
    // safe to call multiple times.
    let already_projected = sqlx::query_as!(
        Concept,
        r#"
        SELECT
            id, current_definition, current_elaboration,
            scope_id, topic_id,
            created_by_event_id, last_event_id, latest_event_recorded_at
        FROM event_substrate.concepts
        WHERE last_event_id = $1
        "#,
        event_id,
    )
    .fetch_optional(pool)
    .await?;
    if let Some(concept) = already_projected {
        return Ok(concept);
    }

    match event_type {
        EventType::ConceptCreated => project_created(pool, &event).await,
        EventType::ConceptMutated => project_mutated(pool, &event).await,
    }
}

async fn project_created(pool: &PgPool, event: &Event) -> Result<Concept, LedgerError> {
    let payload: ConceptCreatedPayload = serde_json::from_value(event.payload.clone())
        .map_err(|e| LedgerError::Database(sqlx::Error::Decode(Box::new(e))))?;

    // Use the genesis event's id as the concept id — deterministic from event history.
    let concept_id = event.id;
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

async fn project_mutated(pool: &PgPool, event: &Event) -> Result<Concept, LedgerError> {
    let payload: crate::payloads::ConceptMutatedPayload =
        serde_json::from_value(event.payload.clone())
            .map_err(|e| LedgerError::Database(sqlx::Error::Decode(Box::new(e))))?;

    let root_event_id = walk_to_root(pool, event.id).await?;

    // Locate the concept row by its genesis event.
    let concept = sqlx::query_as!(
        Concept,
        r#"
        SELECT
            id, current_definition, current_elaboration,
            scope_id, topic_id,
            created_by_event_id, last_event_id, latest_event_recorded_at
        FROM event_substrate.concepts
        WHERE created_by_event_id = $1
        "#,
        root_event_id,
    )
    .fetch_optional(pool)
    .await?
    .ok_or(LedgerError::ConceptNotFound(root_event_id))?;

    let updated = sqlx::query_as!(
        Concept,
        r#"
        UPDATE event_substrate.concepts
           SET current_definition       = COALESCE($2, current_definition),
               current_elaboration      = CASE WHEN $3::boolean THEN $4 ELSE current_elaboration END,
               last_event_id            = $5,
               latest_event_recorded_at = $6
         WHERE id = $1
        RETURNING
            id, current_definition, current_elaboration,
            scope_id, topic_id,
            created_by_event_id, last_event_id, latest_event_recorded_at
        "#,
        concept.id,
        payload.definition,
        payload.elaboration.is_some(),
        payload.elaboration,
        event.id,
        event.recorded_at,
    )
    .fetch_one(pool)
    .await?;

    Ok(updated)
}

/// Walks `Supersedes` references back from the given event until we reach
/// a `ConceptCreated` event. Returns that genesis event's id.
async fn walk_to_root(pool: &PgPool, event_id: Uuid) -> Result<Uuid, LedgerError> {
    let mut current = event_id;
    loop {
        let event = load_event(pool, current).await?;
        let event_type = resolve_event_type(pool, event.event_type_id).await?;
        if matches!(event_type, EventType::ConceptCreated) {
            return Ok(current);
        }
        let refs: Vec<crate::types::event::EventReference> =
            serde_json::from_value(event.references.clone())
                .map_err(|e| LedgerError::Database(sqlx::Error::Decode(Box::new(e))))?;
        let supersedes = refs
            .into_iter()
            .find(|r| matches!(r.kind, crate::types::event::ReferenceKind::Supersedes))
            .ok_or(LedgerError::MissingSupersedes)?;
        current = supersedes.event_id;
    }
}
