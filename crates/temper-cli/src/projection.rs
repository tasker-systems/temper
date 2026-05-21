//! The read-only local projection of cloud vault state.
//!
//! `temper pull <context>` materializes every resource in a context as an
//! on-disk markdown file and records a per-context staleness cursor. The
//! projection is read-only by convention: editing a projected file changes
//! nothing on the server. See
//! `docs/superpowers/specs/2026-05-21-cloud-only-vault-deprecation-design.md`.

use std::collections::HashSet;
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

/// Remove projection `.md` files for resources no longer present in the
/// context. `keep` is the set of absolute file paths the current pull
/// wrote. Walks `<vault_root>/<owner>/<context>/<doc_type>/*.md` across
/// every owner directory. Only `.md` files are considered; other files
/// and other contexts are never touched. Returns the number of files removed.
pub fn prune_context(vault_root: &Path, context: &str, keep: &HashSet<PathBuf>) -> Result<usize> {
    let mut removed = 0usize;
    let owner_iter = match std::fs::read_dir(vault_root) {
        Ok(iter) => iter,
        Err(_) => return Ok(0), // vault root absent → nothing to prune
    };
    for owner_entry in owner_iter.flatten() {
        if !owner_entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
            continue;
        }
        // Skip hidden dirs such as `.temper`.
        if owner_entry.file_name().to_string_lossy().starts_with('.') {
            continue;
        }
        let context_dir = owner_entry.path().join(context);
        if !context_dir.is_dir() {
            continue;
        }
        for doctype_entry in std::fs::read_dir(&context_dir)?.flatten() {
            if !doctype_entry
                .file_type()
                .map(|t| t.is_dir())
                .unwrap_or(false)
            {
                continue;
            }
            for file_entry in std::fs::read_dir(doctype_entry.path())?.flatten() {
                let path = file_entry.path();
                if path.extension().and_then(|e| e.to_str()) != Some("md") {
                    continue;
                }
                if !keep.contains(&path) {
                    std::fs::remove_file(&path)?;
                    removed += 1;
                }
            }
        }
    }
    Ok(removed)
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

    #[test]
    fn prune_removes_stale_md_keeps_listed_and_other_contexts() {
        let dir = tempfile::TempDir::new().unwrap();
        let root = dir.path();

        let task_dir = root.join("@me/myctx/task");
        std::fs::create_dir_all(&task_dir).unwrap();
        let keep = task_dir.join("keep.md");
        let stale = task_dir.join("stale.md");
        let notes = task_dir.join("notes.txt");
        std::fs::write(&keep, "keep").unwrap();
        std::fs::write(&stale, "stale").unwrap();
        std::fs::write(&notes, "notes").unwrap();

        let other_ctx = root.join("@me/otherctx/task");
        std::fs::create_dir_all(&other_ctx).unwrap();
        let other = other_ctx.join("other.md");
        std::fs::write(&other, "other").unwrap();

        let mut keep_set = HashSet::new();
        keep_set.insert(keep.clone());

        let pruned = prune_context(root, "myctx", &keep_set).unwrap();

        assert_eq!(pruned, 1, "exactly one stale .md removed");
        assert!(keep.exists(), "listed file kept");
        assert!(!stale.exists(), "unlisted .md removed");
        assert!(notes.exists(), "non-.md file untouched");
        assert!(other.exists(), "other context untouched");
    }
}
