//! DomainEvent — past-tense facts emitted by backend actions.
//!
//! Events are backend-qualified: `DbResourceCreated` / `VaultFileWritten`
//! describe state transitions in a specific backend. The `CliLocalVault`
//! surface composes events from both backends when its operation chains
//! them (e.g., write file + push, which emits Vault* + Db* events).
//!
//! Initial variant set covers the operations defined in Phase 1 commands.
//! Phase 6 (companion spec #3) adds state-machine-related variants.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::types::ids::ResourceId;

/// A past-tense fact about something that happened during command execution.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum DomainEvent {
    // -------- DbBackend events --------
    /// A new resource row was inserted in the database.
    DbResourceCreated { resource_id: ResourceId },
    /// A resource row was updated; version increments on the server side.
    DbResourceUpdated { resource_id: ResourceId },
    /// A resource row was soft-deleted (`is_active = false`).
    DbResourceSoftDeleted { resource_id: ResourceId },
    /// Chunks were regenerated for a resource (body changed).
    DbChunksGenerated { resource_id: ResourceId },
    /// Embedding was triggered (asynchronous on the server).
    DbEmbeddingTriggered { resource_id: ResourceId },

    // -------- Relationship-write events --------
    /// A new relationship was asserted; an edge row was projected.
    DbRelationshipAsserted { correlation_id: Uuid },
    /// An existing relationship was retyped.
    DbRelationshipRetyped { correlation_id: Uuid },
    /// An existing relationship was reweighted.
    DbRelationshipReweighted { correlation_id: Uuid },
    /// An existing relationship was folded (retracted from the default sheet).
    DbRelationshipFolded { correlation_id: Uuid },

    // -------- VaultBackend events --------
    /// A vault file was written (created or modified).
    VaultFileWritten { path: String },
    /// The manifest entry for a resource was updated.
    VaultManifestUpdated { path: String },
    /// A vault file was removed.
    VaultFileRemoved { path: String },

    // -------- Composite / cross-backend events --------
    /// A vault-side change was successfully pushed to the API (DbBackend).
    RemoteSynced { resource_id: ResourceId },
    /// A push attempt was deferred (offline / not authed); manifest tracks pending.
    PushDeferred { reason: PushDeferReason },
}

/// Reason a push was deferred to bulk-recovery sync.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PushDeferReason {
    Offline,
    NotAuthed,
    Other,
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    #[test]
    fn db_event_round_trips() {
        let e = DomainEvent::DbResourceCreated {
            resource_id: ResourceId(Uuid::nil()),
        };
        let s = serde_json::to_string(&e).unwrap();
        let back: DomainEvent = serde_json::from_str(&s).unwrap();
        assert_eq!(e, back);
    }

    #[test]
    fn vault_event_round_trips() {
        let e = DomainEvent::VaultFileWritten {
            path: "@me/temper/task/foo.md".to_string(),
        };
        let s = serde_json::to_string(&e).unwrap();
        let back: DomainEvent = serde_json::from_str(&s).unwrap();
        assert_eq!(e, back);
    }

    #[test]
    fn push_deferred_carries_reason() {
        let e = DomainEvent::PushDeferred {
            reason: PushDeferReason::Offline,
        };
        let s = serde_json::to_string(&e).unwrap();
        assert!(s.contains("offline"));
    }

    #[test]
    fn relationship_event_round_trips() {
        let e = DomainEvent::DbRelationshipAsserted {
            correlation_id: Uuid::nil(),
        };
        let s = serde_json::to_string(&e).unwrap();
        let back: DomainEvent = serde_json::from_str(&s).unwrap();
        assert_eq!(e, back);
    }
}
