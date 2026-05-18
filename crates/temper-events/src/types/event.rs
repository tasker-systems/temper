use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum EventType {
    ConceptCreated,
    ConceptMutated,
}

impl EventType {
    pub fn as_canonical_name(self) -> &'static str {
        match self {
            EventType::ConceptCreated => "ConceptCreated",
            EventType::ConceptMutated => "ConceptMutated",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReferenceKind {
    Supersedes,
    DerivedFrom,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EventReference {
    pub kind: ReferenceKind,
    pub event_id: Uuid,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, sqlx::FromRow)]
pub struct Event {
    pub id: Uuid,
    pub event_type_id: Uuid,
    pub emitter_entity_id: Uuid,
    pub topic_id: Uuid,
    pub scope_id: Uuid,
    pub payload: serde_json::Value,
    pub metadata: serde_json::Value,
    #[sqlx(rename = "references")]
    pub references: serde_json::Value,
    pub correlation_id: Uuid,
    pub occurred_at: DateTime<Utc>,
    pub recorded_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct EventToWrite {
    pub id: Uuid,
    pub event_type: EventType,
    pub emitter_entity_id: Uuid,
    pub topic_id: Uuid,
    pub scope_id: Uuid,
    pub payload: serde_json::Value,
    pub metadata: serde_json::Value,
    pub references: Vec<EventReference>,
    pub correlation_id: Uuid,
    pub occurred_at: DateTime<Utc>,
}

impl EventToWrite {
    /// Construct a root event whose `id` and `correlation_id` are equal
    /// and freshly generated.
    pub fn new_root(
        event_type: EventType,
        emitter_entity_id: Uuid,
        topic_id: Uuid,
        scope_id: Uuid,
        payload: serde_json::Value,
        occurred_at: DateTime<Utc>,
    ) -> Self {
        let id = Uuid::now_v7();
        Self {
            id,
            event_type,
            emitter_entity_id,
            topic_id,
            scope_id,
            payload,
            metadata: serde_json::json!({}),
            references: Vec::new(),
            correlation_id: id,
            occurred_at,
        }
    }
}
