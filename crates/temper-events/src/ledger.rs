use serde_json::Value;
use sqlx::PgPool;

use crate::errors::LedgerError;
use crate::types::event::{Event, EventReference, EventToWrite, EventType, ReferenceKind};

pub async fn append_event(pool: &PgPool, write: EventToWrite) -> Result<Event, LedgerError> {
    let event_type_name = write.event_type.as_canonical_name();

    let event_type_id: uuid::Uuid = sqlx::query_scalar!(
        "SELECT id FROM kb_event_types WHERE name = $1",
        event_type_name,
    )
    .fetch_optional(pool)
    .await?
    .ok_or_else(|| LedgerError::UnknownEventType(event_type_name.to_string()))?;

    // Validate FKs explicitly so callers see typed errors instead of
    // raw Postgres foreign-key violations.
    let profile_exists: bool = sqlx::query_scalar!(
        "SELECT EXISTS (SELECT 1 FROM kb_profiles WHERE id = $1)",
        write.emitter_profile_id,
    )
    .fetch_one(pool)
    .await?
    .unwrap_or(false);
    if !profile_exists {
        return Err(LedgerError::UnknownProfile(write.emitter_profile_id));
    }

    let topic_exists: bool = sqlx::query_scalar!(
        "SELECT EXISTS (SELECT 1 FROM kb_topics WHERE id = $1)",
        write.topic_id,
    )
    .fetch_one(pool)
    .await?
    .unwrap_or(false);
    if !topic_exists {
        return Err(LedgerError::UnknownTopic(write.topic_id));
    }

    let scope_exists: bool = sqlx::query_scalar!(
        "SELECT EXISTS (SELECT 1 FROM kb_scopes WHERE id = $1)",
        write.scope_id,
    )
    .fetch_one(pool)
    .await?
    .unwrap_or(false);
    if !scope_exists {
        return Err(LedgerError::UnknownScope(write.scope_id));
    }

    // Reference invariants — type-specific.
    let supersedes_refs: Vec<&EventReference> = write
        .references
        .iter()
        .filter(|r| matches!(r.kind, ReferenceKind::Supersedes))
        .collect();

    match write.event_type {
        EventType::ConceptCreated => {
            if !supersedes_refs.is_empty() {
                return Err(LedgerError::SupersedesOnGenesis);
            }
        }
        EventType::ConceptMutated => match supersedes_refs.len() {
            0 => return Err(LedgerError::MissingSupersedes),
            1 => {}
            _ => return Err(LedgerError::MultipleSupersedes),
        },
        EventType::RelationshipAsserted
        | EventType::RelationshipRetyped
        | EventType::RelationshipReweighted
        | EventType::RelationshipFolded
        | EventType::RelationshipDecayed
        | EventType::RelationshipCorrected => {
            // Relationship lifecycle events impose no Supersedes invariant;
            // intra-lifecycle linkage is carried by correlation_id.
        }
    }

    // Validate every reference resolves to a real event.
    for reference in &write.references {
        let exists: bool = sqlx::query_scalar!(
            "SELECT EXISTS (SELECT 1 FROM kb_events WHERE id = $1)",
            reference.event_id,
        )
        .fetch_one(pool)
        .await?
        .unwrap_or(false);
        if !exists {
            return Err(LedgerError::DanglingReference {
                event_id: reference.event_id,
                kind: reference.kind,
            });
        }
    }

    let references_json: Value = serde_json::to_value(&write.references)
        .expect("EventReference serialization is infallible");

    let event = sqlx::query_as!(
        Event,
        r#"
        INSERT INTO kb_events (
            id, event_type_id, profile_id, device_id, topic_id, scope_id,
            payload, metadata, "references", correlation_id, occurred_at
        )
        VALUES ($1, $2, $3, 'ledger', $4, $5, $6, $7, $8, $9, $10)
        RETURNING
            id,
            event_type_id,
            profile_id        AS "emitter_profile_id!",
            topic_id          AS "topic_id!",
            scope_id          AS "scope_id!",
            payload           AS "payload!",
            metadata,
            "references",
            correlation_id    AS "correlation_id!",
            occurred_at,
            created           AS "recorded_at!"
        "#,
        write.id,
        event_type_id,
        write.emitter_profile_id,
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
