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

use temper_client::TemperClient;
use temper_core::types::resource::ResourceListParams;
use temper_core::types::ContentResponse;
use temper_core::types::ResourceRow;
use temper_core::vault::Vault;

use crate::config::Config;
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
        // An absent vault root means there is nothing to prune. Any other IO
        // failure (permissions, etc.) is a real error and must surface.
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(0),
        Err(e) => return Err(e.into()),
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

/// Assemble and write a resource's projection file from an already-fetched
/// row and content. The pure-write half of [`write_resource_file`] — it
/// makes no network call. `pull_context` reaches it via `write_resource_file`
/// (which fetches first); `temper resource show` calls it directly, because
/// its cloud branch already holds both the row and the content.
///
/// Frontmatter assembly reuses `actions::ingest::build_frontmatter_from_resource`
/// so projected files are byte-identical to sync-pulled ones. Returns the
/// absolute path written.
pub fn write_resource_file_from_parts(
    vault_root: &Path,
    row: &ResourceRow,
    content: &ContentResponse,
) -> Result<PathBuf> {
    use crate::actions::ingest;

    // `owner_handle` is literal "@me" for the requester's own resources and
    // "+team-slug" for team contexts — both are canonical vault directory
    // components. Empty handle defends against a sparse server row.
    let owner: &str = if row.owner_handle.is_empty() {
        "@me"
    } else {
        &row.owner_handle
    };
    let context = row.context_name.as_str();
    let doc_type = row.doc_type_name.as_str();

    let slug_owned;
    let slug: &str = match row.slug.as_deref() {
        Some(s) if !s.is_empty() => s,
        _ => {
            slug_owned = ingest::slug_from_title(&row.title);
            slug_owned.as_str()
        }
    };

    let managed_value = content
        .managed_meta
        .as_ref()
        .map(|m| serde_json::to_value(m).unwrap_or(serde_json::Value::Null));

    let fm = ingest::build_frontmatter_from_resource(
        row,
        context,
        doc_type,
        owner,
        ingest::normalize_body_for_vault(&content.markdown),
        managed_value.as_ref(),
        content.open_meta.as_ref(),
    )?;

    let path = Vault::new(vault_root).doc_file(owner, context, doc_type, slug);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    fm.write_to(&path)
        .map_err(|e| TemperError::Config(format!("projection write {}: {e}", path.display())))?;
    Ok(path)
}

/// Fetch a resource's content and write it as a complete markdown file at
/// its canonical vault path. Returns the absolute path written.
///
/// `row` is a resource summary already obtained from a `list` call; this
/// makes one further API call (`content`) for the body + frontmatter meta,
/// then delegates the assembly + write to [`write_resource_file_from_parts`].
pub async fn write_resource_file(
    client: &TemperClient,
    vault_root: &Path,
    row: &ResourceRow,
) -> Result<PathBuf> {
    let content = client
        .resources()
        .content(Uuid::from(row.id))
        .await
        .map_err(crate::commands::client_err)?;
    write_resource_file_from_parts(vault_root, row, &content)
}

/// Outcome of a `pull_context` call, for the command's output line.
#[derive(Debug, Clone)]
pub struct PullSummary {
    pub context: String,
    pub written: usize,
    pub pruned: usize,
}

/// Page size for listing a context's resources. Contexts are small (tens to
/// low hundreds of resources); this paginates defensively regardless of the
/// server's own list cap.
const PULL_PAGE_SIZE: i64 = 200;

/// Materialize a whole context's resources into the local projection:
/// list every resource, write each file, prune files for resources no
/// longer present, then record the per-context staleness cursor.
///
/// Idempotent — re-running produces the same tree.
pub async fn pull_context(
    client: &TemperClient,
    config: &Config,
    context: &str,
) -> Result<PullSummary> {
    // 1. List every resource in the context (paginated).
    let mut rows: Vec<ResourceRow> = Vec::new();
    let mut offset: i64 = 0;
    loop {
        let params = ResourceListParams {
            context_name: Some(context.to_string()),
            limit: Some(PULL_PAGE_SIZE),
            offset: Some(offset),
            ..Default::default()
        };
        let resp = client
            .resources()
            .list(&params)
            .await
            .map_err(crate::commands::client_err)?;
        let page_len = resp.rows.len() as i64;
        rows.extend(resp.rows);
        if page_len < PULL_PAGE_SIZE {
            break;
        }
        offset += PULL_PAGE_SIZE;
    }

    // 2. Write each resource's file.
    let mut keep: HashSet<PathBuf> = HashSet::new();
    for row in &rows {
        let path = write_resource_file(client, &config.vault_root, row).await?;
        keep.insert(path);
    }

    // 3. Prune files for resources no longer in the context.
    let pruned = prune_context(&config.vault_root, context, &keep)?;

    // 4. Record the staleness cursor. The context's UUID comes from any
    //    listed row; an empty context yields no event id.
    let context_id = rows.first().map(|r| Uuid::from(r.kb_context_id));
    let last_event_id = match context_id {
        Some(cid) => client
            .events()
            .latest_for_context(cid)
            .await
            .map_err(crate::commands::client_err)?,
        None => None,
    };
    write_cursor(
        &config.state_dir,
        context,
        &ProjectionCursor {
            last_event_id,
            pulled_at: Utc::now(),
        },
    )?;

    Ok(PullSummary {
        context: context.to_string(),
        written: keep.len(),
        pruned,
    })
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

    #[test]
    fn prune_returns_zero_when_vault_root_absent() {
        let dir = tempfile::TempDir::new().unwrap();
        let missing = dir.path().join("does-not-exist");
        let pruned = prune_context(&missing, "anyctx", &HashSet::new()).unwrap();
        assert_eq!(pruned, 0, "absent vault root prunes nothing, no error");
    }
}
