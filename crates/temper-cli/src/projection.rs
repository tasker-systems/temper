//! The read-only local projection of cloud vault state.
//!
//! `temper pull <context>` materializes every resource in a context as an
//! on-disk markdown file and records a per-context staleness cursor. The
//! projection is read-only by convention: editing a projected file changes
//! nothing on the server. See
//! `docs/superpowers/specs/2026-05-21-cloud-only-vault-deprecation-design.md`.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// The per-context staleness cursor, written to
/// `.temper/projection/<context>.json` after every successful pull.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectionCursor {
    /// Server's latest event id for the context at pull time. `None` when
    /// the context had no events.
    pub last_event_id: Option<Uuid>,
    /// When the projection for this context was last refreshed.
    pub pulled_at: DateTime<Utc>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn projection_cursor_round_trips() {
        let cursor = ProjectionCursor {
            last_event_id: Some(Uuid::nil()),
            pulled_at: Utc::now(),
        };
        let json = serde_json::to_string(&cursor).unwrap();
        let back: ProjectionCursor = serde_json::from_str(&json).unwrap();
        assert_eq!(back.last_event_id, cursor.last_event_id);
        assert_eq!(back.pulled_at, cursor.pulled_at);
    }
}
