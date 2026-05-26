use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum EventType {
    ConceptCreated,
    ConceptMutated,
    RelationshipAsserted,
    RelationshipRetyped,
    RelationshipReweighted,
    RelationshipFolded,
    RelationshipDecayed,
    RelationshipCorrected,
}

impl EventType {
    pub fn as_canonical_name(self) -> &'static str {
        match self {
            EventType::ConceptCreated => "ConceptCreated",
            EventType::ConceptMutated => "ConceptMutated",
            EventType::RelationshipAsserted => "relationship_asserted",
            EventType::RelationshipRetyped => "relationship_retyped",
            EventType::RelationshipReweighted => "relationship_reweighted",
            EventType::RelationshipFolded => "relationship_folded",
            EventType::RelationshipDecayed => "relationship_decayed",
            EventType::RelationshipCorrected => "relationship_corrected",
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
    pub emitter_profile_id: Uuid,
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
    pub emitter_profile_id: Uuid,
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
        emitter_profile_id: Uuid,
        topic_id: Uuid,
        scope_id: Uuid,
        payload: serde_json::Value,
        occurred_at: DateTime<Utc>,
    ) -> Self {
        let id = Uuid::now_v7();
        Self {
            id,
            event_type,
            emitter_profile_id,
            topic_id,
            scope_id,
            payload,
            metadata: serde_json::json!({}),
            references: Vec::new(),
            correlation_id: id,
            occurred_at,
        }
    }

    /// Construct a non-root event that joins an existing lifecycle: `id` is
    /// fresh, `correlation_id` is the caller-supplied lifecycle root id.
    pub fn new_correlated(
        event_type: EventType,
        emitter_profile_id: Uuid,
        topic_id: Uuid,
        scope_id: Uuid,
        payload: serde_json::Value,
        correlation_id: Uuid,
        occurred_at: DateTime<Utc>,
    ) -> Self {
        Self {
            id: Uuid::now_v7(),
            event_type,
            emitter_profile_id,
            topic_id,
            scope_id,
            payload,
            metadata: serde_json::json!({}),
            references: Vec::new(),
            correlation_id,
            occurred_at,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relationship_event_canonical_names_are_snake_case() {
        assert_eq!(
            EventType::RelationshipAsserted.as_canonical_name(),
            "relationship_asserted"
        );
        assert_eq!(
            EventType::RelationshipRetyped.as_canonical_name(),
            "relationship_retyped"
        );
        assert_eq!(
            EventType::RelationshipReweighted.as_canonical_name(),
            "relationship_reweighted"
        );
        assert_eq!(
            EventType::RelationshipFolded.as_canonical_name(),
            "relationship_folded"
        );
        assert_eq!(
            EventType::RelationshipDecayed.as_canonical_name(),
            "relationship_decayed"
        );
        assert_eq!(
            EventType::RelationshipCorrected.as_canonical_name(),
            "relationship_corrected"
        );
    }

    #[test]
    fn new_correlated_keeps_supplied_correlation_id() {
        let corr = Uuid::now_v7();
        let w = EventToWrite::new_correlated(
            EventType::RelationshipRetyped,
            Uuid::nil(),
            Uuid::nil(),
            Uuid::nil(),
            serde_json::json!({}),
            corr,
            Utc::now(),
        );
        assert_eq!(w.correlation_id, corr);
        assert_ne!(w.id, corr);
    }
}
