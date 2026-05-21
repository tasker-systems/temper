//! The read-only local projection of cloud vault state.
//!
//! `temper pull <context>` materializes every resource in a context as an
//! on-disk markdown file and records a per-context staleness cursor. The
//! projection is read-only by convention: editing a projected file changes
//! nothing on the server. See
//! `docs/superpowers/specs/2026-05-21-cloud-only-vault-deprecation-design.md`.

use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::error::{Result, TemperError};

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

/// Absolute path of a context's cursor sidecar.
fn cursor_path(state_dir: &Path, context: &str) -> PathBuf {
    state_dir.join("projection").join(format!("{context}.json"))
}

/// Read a context's cursor sidecar. Returns `None` when the file is absent
/// or unparseable (a corrupt sidecar is treated as "never pulled" rather
/// than a hard error — the next pull overwrites it).
pub fn read_cursor(state_dir: &Path, context: &str) -> Result<Option<ProjectionCursor>> {
    let path = cursor_path(state_dir, context);
    if !path.exists() {
        return Ok(None);
    }
    let content = std::fs::read_to_string(&path)?;
    Ok(serde_json::from_str::<ProjectionCursor>(&content).ok())
}

/// Atomically write a context's cursor sidecar (temp file + rename, the
/// pattern used by `manifest_io::save_manifest`).
pub fn write_cursor(state_dir: &Path, context: &str, cursor: &ProjectionCursor) -> Result<()> {
    let path = cursor_path(state_dir, context);
    let dir = path.parent().ok_or_else(|| {
        TemperError::Config(format!("cursor path has no parent: {}", path.display()))
    })?;
    std::fs::create_dir_all(dir)?;
    let tmp_path = dir.join(format!("{context}.json.tmp"));
    let content = serde_json::to_string_pretty(cursor)?;
    std::fs::write(&tmp_path, content)?;
    std::fs::rename(&tmp_path, &path)?;
    Ok(())
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

    #[test]
    fn cursor_write_then_read_round_trips() {
        let dir = tempfile::TempDir::new().unwrap();
        let state_dir = dir.path().join(".temper");
        let cursor = ProjectionCursor {
            last_event_id: Some(Uuid::nil()),
            pulled_at: Utc::now(),
        };
        write_cursor(&state_dir, "myctx", &cursor).unwrap();
        let back = read_cursor(&state_dir, "myctx").unwrap();
        assert!(back.is_some());
        assert_eq!(back.unwrap().last_event_id, cursor.last_event_id);
    }

    #[test]
    fn read_cursor_returns_none_when_absent() {
        let dir = tempfile::TempDir::new().unwrap();
        let state_dir = dir.path().join(".temper");
        assert!(read_cursor(&state_dir, "never-pulled").unwrap().is_none());
    }
}
