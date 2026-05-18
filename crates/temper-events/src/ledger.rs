use serde_json::Value;
use sqlx::PgPool;

use crate::errors::LedgerError;
use crate::types::event::{Event, EventToWrite};

pub async fn append_event(pool: &PgPool, write: EventToWrite) -> Result<Event, LedgerError> {
    let event_type_name = write.event_type.as_canonical_name();

    let event_type_id: uuid::Uuid = sqlx::query_scalar!(
        "SELECT id FROM event_substrate.event_types WHERE name = $1",
        event_type_name,
    )
    .fetch_optional(pool)
    .await?
    .ok_or_else(|| LedgerError::UnknownEventType(event_type_name.to_string()))?;

    let references_json: Value = serde_json::to_value(&write.references)
        .expect("EventReference serialization is infallible");

    let event = sqlx::query_as!(
        Event,
        r#"
        INSERT INTO event_substrate.events (
            id, event_type_id, emitter_entity_id, topic_id, scope_id,
            payload, metadata, "references", correlation_id, occurred_at
        )
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
        RETURNING
            id,
            event_type_id,
            emitter_entity_id,
            topic_id,
            scope_id,
            payload,
            metadata,
            "references",
            correlation_id,
            occurred_at,
            recorded_at
        "#,
        write.id,
        event_type_id,
        write.emitter_entity_id,
        write.topic_id,
        write.scope_id,
        write.payload,
        write.metadata,
        references_json,
        write.correlation_id,
        write.occurred_at,
    )
    .fetch_one(pool)
    .await?;

    Ok(event)
}
