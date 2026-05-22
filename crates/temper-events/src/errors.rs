use uuid::Uuid;

use crate::types::event::ReferenceKind;

#[derive(Debug, thiserror::Error)]
pub enum LedgerError {
    #[error("unknown profile: {0}")]
    UnknownProfile(Uuid),
    #[error("unknown topic: {0}")]
    UnknownTopic(Uuid),
    #[error("unknown scope: {0}")]
    UnknownScope(Uuid),
    #[error("unknown event type: {0}")]
    UnknownEventType(String),
    #[error("dangling reference: event {event_id} ({kind:?}) does not exist")]
    DanglingReference { event_id: Uuid, kind: ReferenceKind },
    #[error("ConceptMutated must include exactly one Supersedes reference; found none")]
    MissingSupersedes,
    #[error("ConceptMutated must include exactly one Supersedes reference; found multiple")]
    MultipleSupersedes,
    #[error("ConceptCreated must not include a Supersedes reference")]
    SupersedesOnGenesis,
    #[error("database error: {0}")]
    Database(#[from] sqlx::Error),
}
