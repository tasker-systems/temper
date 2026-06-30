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
use temper_core::context_ref::{parse_context_ref, ContextOwnerRef, ContextRef};
use temper_workflow::types::resource::ResourceListParams;
use temper_workflow::types::ContentResponse;
use temper_workflow::types::ResourceRow;
use temper_workflow::vault::Vault;

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

/// Atomically write a context's cursor sidecar using the standard
/// temp-file-plus-rename pattern.
///
/// The `context` key may be a decorated ref (`@owner/slug`) whose `/`
/// causes `cursor_path` to introduce a subdirectory under the projection
/// directory. The temp path is derived from the cursor `path` directly
/// (via `set_extension`) rather than re-joining `context` as a string, so
/// a ref containing `/` never creates an unexpected second level of nesting.
pub fn write_cursor(state_dir: &Path, context: &str, cursor: &ProjectionCursor) -> Result<()> {
    let path = cursor_path(state_dir, context);
    let dir = path.parent().ok_or_else(|| {
        TemperError::Config(format!("cursor path has no parent: {}", path.display()))
    })?;
    std::fs::create_dir_all(dir)?;
    // Derive the temp path from `path` itself — do NOT re-join `context`
    // because a decorated ref like `@me/slug` contains `/` and
    // `dir.join("@me/slug.json.tmp")` would silently create a nested
    // subdirectory that `create_dir_all(dir)` did not prepare.
    let mut tmp_path = path.clone();
    tmp_path.set_extension("json.tmp");
    let content = serde_json::to_string_pretty(cursor)?;
    std::fs::write(&tmp_path, content)?;
    std::fs::rename(&tmp_path, &path)?;
    Ok(())
}

/// Outcome of a non-blocking staleness pre-flight for one context.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StalenessOutcome {
    /// No cursor sidecar — the context was never pulled. The check made no
    /// network call; the caller stays silent.
    NotProjected,
    /// A cursor exists and matches the server's latest event. Silent.
    Fresh,
    /// A cursor exists but the server has advanced past it. The caller warns.
    Stale,
    /// The check could not complete — offline, or the context could not be
    /// resolved. Silent (a debug log is emitted at the failure site).
    Skipped,
}

/// Compare a context's cursor against the server's latest event id for that
/// context. Pure: the staleness *decision*, with no IO. The server's id is
/// recorded into the cursor at pull time, so any divergence means at least
/// one event landed since the last pull.
fn evaluate_staleness(cursor: &ProjectionCursor, server_latest: Option<Uuid>) -> StalenessOutcome {
    if server_latest == cursor.last_event_id {
        StalenessOutcome::Fresh
    } else {
        StalenessOutcome::Stale
    }
}

/// Resolve a context ref to its UUID via the contexts list. Returns `None`
/// when the ref cannot be parsed, the context is not found, or the API call
/// fails — the caller treats any of these as "cannot check", not as an error.
///
/// Accepts a UUID or decorated `@owner/slug` / `+team/slug` form. Bare names
/// are rejected by the parser and return `None` (consistent with the arc's
/// hard-rejection of ambiguous bare-name addressing).
///
/// For `@me/slug` the row is matched by slug alone among profile-owned entries
/// (`owner_ref` starts with `@`). This is unambiguous because slug is unique
/// per `(owner_table, owner_id)` on the server, and profile-to-profile context
/// sharing is not supported — visible profile-owned contexts are always the
/// principal's own.
async fn resolve_context_id(client: &TemperClient, context: &str) -> Option<Uuid> {
    let r = parse_context_ref(context).ok()?;
    let rows = client.contexts().list().await.ok()?;
    match r {
        ContextRef::Id(id) => rows
            .into_iter()
            .find(|c| Uuid::from(c.id) == id)
            .map(|c| Uuid::from(c.id)),
        ContextRef::OwnerSlug { owner, slug } => rows
            .into_iter()
            .find(|c| {
                c.slug == slug
                    && match &owner {
                        // `@me` — all profile-owned contexts have an `@`-sigiled `owner_ref`.
                        // Slug uniqueness per owner means this is unambiguous.
                        ContextOwnerRef::Me => c.owner_ref.starts_with('@'),
                        ContextOwnerRef::Handle(h) => c.owner_ref == format!("@{h}"),
                        ContextOwnerRef::Team(t) => c.owner_ref == format!("+{t}"),
                    }
            })
            .map(|c| Uuid::from(c.id)),
    }
}

/// Non-blocking staleness pre-flight for one context. Reads the context's
/// cursor sidecar; only if one exists does it resolve the context id and
/// fetch the server's latest event id. Never errors and never blocks:
///
/// - no cursor             -> `NotProjected` (zero network calls)
/// - cursor + server even  -> `Fresh`
/// - cursor + server ahead -> `Stale`
/// - any failure           -> `Skipped` (debug log)
pub async fn check_context_staleness(
    client: &TemperClient,
    state_dir: &Path,
    context: &str,
) -> StalenessOutcome {
    let cursor = match read_cursor(state_dir, context) {
        Ok(Some(cursor)) => cursor,
        Ok(None) => return StalenessOutcome::NotProjected,
        Err(e) => {
            tracing::debug!("staleness check skipped: cursor read failed for {context}: {e}");
            return StalenessOutcome::Skipped;
        }
    };
    let Some(context_id) = resolve_context_id(client, context).await else {
        tracing::debug!("staleness check skipped: could not resolve context '{context}'");
        return StalenessOutcome::Skipped;
    };
    let server_latest = match client.events().latest_for_context(context_id).await {
        Ok(latest) => latest,
        Err(e) => {
            tracing::debug!("staleness check skipped: latest_for_context failed: {e}");
            return StalenessOutcome::Skipped;
        }
    };
    evaluate_staleness(&cursor, server_latest)
}

/// Run the staleness pre-flight and print one warning line if the context's
/// projection is stale. All other outcomes are silent. This is the
/// caller-facing entry point for context-touching commands.
pub async fn warn_if_context_stale(client: &TemperClient, state_dir: &Path, context: &str) {
    if check_context_staleness(client, state_dir, context).await == StalenessOutcome::Stale {
        crate::output::warning(format!(
            "projection for '{context}' is stale — run `temper pull {context}` to refresh"
        ));
    }
}

/// Remove projection `.md` files for resources no longer present in the
/// context. `keep` is the set of absolute file paths the current pull
/// wrote. Walks `<vault_root>/<owner>/<context_name>/<doc_type>/*.md` across
/// every owner directory. Only `.md` files are considered; other files
/// and other contexts are never touched. Returns the number of files removed.
///
/// `context` must be the **on-disk directory name** (the context's slug/name,
/// e.g. `"temper"`), not a decorated ref like `@me/temper`. Callers should
/// derive it from the listed rows' `context_name` field rather than forwarding
/// the raw command-line ref.
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
/// absolute path written, or `None` when the resource is cogmap-homed and
/// therefore skipped (see below).
pub fn write_resource_file_from_parts(
    vault_root: &Path,
    row: &ResourceRow,
    content: &ContentResponse,
) -> Result<Option<PathBuf>> {
    use crate::actions::ingest;

    // A cogmap-homed resource has no context path on disk; the local vault
    // projection layout for cogmap homes is a later beat. Skip projection —
    // the cloud stays authoritative; the local cache simply doesn't
    // materialize it. (Surface B follow-up.)
    let Some(context) = row.context_name.as_deref() else {
        tracing::debug!(
            resource = %Uuid::from(row.id),
            "projection skipped: cogmap-homed resources are not projected locally yet"
        );
        return Ok(None);
    };

    // `owner_handle` is literal "@me" for the requester's own resources and
    // "+team-slug" for team contexts — both are canonical vault directory
    // components. Empty handle defends against a sparse server row.
    let owner: &str = if row.owner_handle.is_empty() {
        "@me"
    } else {
        &row.owner_handle
    };
    let doc_type = row.doc_type_name.as_str();

    let slug = ingest::slug_from_title(&row.title);

    // Propagate a serialization failure rather than writing `null` into the projected file's
    // frontmatter — a silent `unwrap_or(Null)` here would corrupt the on-disk managed meta.
    let managed_value = content
        .managed_meta
        .as_ref()
        .map(serde_json::to_value)
        .transpose()
        .map_err(|e| TemperError::Config(format!("projection serialize managed_meta: {e}")))?;

    let fm = ingest::build_frontmatter_from_resource(ingest::BuildFrontmatterParams {
        resource: row,
        context,
        doc_type,
        canonical_owner: owner,
        body: ingest::normalize_body_for_vault(&content.markdown),
        managed_meta: managed_value.as_ref(),
        open_meta: content.open_meta.as_ref(),
    })?;

    let path = Vault::new(vault_root).doc_file(owner, context, doc_type, &slug);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    fm.write_to(&path)
        .map_err(|e| TemperError::Config(format!("projection write {}: {e}", path.display())))?;
    Ok(Some(path))
}

/// Fetch a resource's content and write it as a complete markdown file at
/// its canonical vault path. Returns the absolute path written.
///
/// `row` is a resource summary already obtained from a `list` call; this
/// makes one further API call (`content`) for the body + frontmatter meta,
/// then delegates the assembly + write to [`write_resource_file_from_parts`].
/// Returns `None` when the resource is cogmap-homed (projection skipped).
pub async fn write_resource_file(
    client: &TemperClient,
    vault_root: &Path,
    row: &ResourceRow,
) -> Result<Option<PathBuf>> {
    let content = client
        .resources()
        .content(Uuid::from(row.id))
        .await
        .map_err(crate::commands::client_err)?;
    write_resource_file_from_parts(vault_root, row, &content)
}

/// Remove a resource's projection file given a server [`ResourceRow`].
///
/// A by-row convenience over [`remove_resource_file`] for the id-addressed
/// `temper resource delete` path: derives `owner` from the row's context
/// subscription (`config.owner_for_context`) and `context`/`doctype`/`slug`
/// from the row. A row with no slug falls back to the title-derived slug so
/// the path matches what the projection writer would have produced.
pub fn remove_resource_file_for_row(
    vault_root: &Path,
    config: &crate::config::Config,
    row: &ResourceRow,
) -> Result<()> {
    use crate::actions::ingest;

    // A cogmap-homed resource was never projected to disk (no context path),
    // so there is nothing to remove. Skip — same bounded edge as the writer.
    let Some(context) = row.context_name.as_deref() else {
        tracing::debug!(
            resource = %Uuid::from(row.id),
            "projection removal skipped: cogmap-homed resources are not projected locally yet"
        );
        return Ok(());
    };

    let owner = config.owner_for_context(context);
    let slug = ingest::slug_from_title(&row.title);
    remove_resource_file(vault_root, &owner, context, &row.doc_type_name, &slug)
}

/// Remove a resource's projection file at its canonical vault path.
///
/// A best-effort counterpart to [`write_resource_file_from_parts`], used
/// by `temper resource delete` after a successful server-side delete. An
/// already-absent file is a silent success — the projection is
/// derivative, so "the file is gone" is the desired end state either way.
pub fn remove_resource_file(
    vault_root: &Path,
    owner: &str,
    context: &str,
    doc_type: &str,
    slug: &str,
) -> Result<()> {
    let path = Vault::new(vault_root).doc_file(owner, context, doc_type, slug);
    match std::fs::remove_file(&path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(TemperError::Config(format!(
            "projection remove {}: {e}",
            path.display()
        ))),
    }
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
    let rows = list_context_resources(client, context).await?;
    let keep = write_projection_files(client, &config.vault_root, &rows).await?;
    let pruned = prune_absent_files(&config.vault_root, context, &rows, &keep)?;
    record_context_cursor(client, &config.state_dir, context, &rows).await?;

    Ok(PullSummary {
        context: context.to_string(),
        written: keep.len(),
        pruned,
    })
}

/// List every resource in `context`, following the server's pagination.
async fn list_context_resources(client: &TemperClient, context: &str) -> Result<Vec<ResourceRow>> {
    let mut rows: Vec<ResourceRow> = Vec::new();
    let mut offset: i64 = 0;
    loop {
        let params = ResourceListParams {
            context_ref: Some(context.to_string()),
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
    Ok(rows)
}

/// Write each listed resource's projection file, returning the set of paths
/// that must be kept (used to drive pruning).
async fn write_projection_files(
    client: &TemperClient,
    vault_root: &Path,
    rows: &[ResourceRow],
) -> Result<HashSet<PathBuf>> {
    let mut keep: HashSet<PathBuf> = HashSet::new();
    for row in rows {
        if let Some(path) = write_resource_file(client, vault_root, row).await? {
            keep.insert(path);
        }
    }
    Ok(keep)
}

/// Prune projection files for resources no longer present in the context.
///
/// The on-disk directory component is the context's `context_name` field
/// (e.g. `"temper"`), not the raw ref (which may be `"@me/temper"`). Derive
/// it from any listed row; for an empty context fall back to parsing the
/// slug from the ref so that a context that has been emptied server-side
/// still prunes its local projection files.
fn prune_absent_files(
    vault_root: &Path,
    context: &str,
    rows: &[ResourceRow],
    keep: &HashSet<PathBuf>,
) -> Result<usize> {
    let context_dir_name: Option<String> = rows
        .first()
        .and_then(|r| r.context_name.clone())
        .or_else(|| {
            parse_context_ref(context).ok().and_then(|r| match r {
                ContextRef::OwnerSlug { slug, .. } => Some(slug),
                ContextRef::Id(_) => None,
            })
        });
    match context_dir_name.as_deref() {
        Some(name) => prune_context(vault_root, name, keep),
        None => Ok(0),
    }
}

/// Record the per-context staleness cursor. The context's UUID comes from
/// any listed row; an empty context yields no event id.
async fn record_context_cursor(
    client: &TemperClient,
    state_dir: &Path,
    context: &str,
    rows: &[ResourceRow],
) -> Result<()> {
    let context_id = rows.first().and_then(|r| r.kb_context_id.map(Uuid::from));
    let last_event_id = match context_id {
        Some(cid) => client
            .events()
            .latest_for_context(cid)
            .await
            .map_err(crate::commands::client_err)?,
        None => None,
    };
    write_cursor(
        state_dir,
        context,
        &ProjectionCursor {
            last_event_id,
            pulled_at: Utc::now(),
        },
    )
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
    fn evaluate_staleness_equal_ids_is_fresh() {
        let cursor = ProjectionCursor {
            last_event_id: Some(Uuid::nil()),
            pulled_at: Utc::now(),
        };
        assert_eq!(
            evaluate_staleness(&cursor, Some(Uuid::nil())),
            StalenessOutcome::Fresh
        );
    }

    #[test]
    fn evaluate_staleness_differing_ids_is_stale() {
        let cursor = ProjectionCursor {
            last_event_id: Some(Uuid::nil()),
            pulled_at: Utc::now(),
        };
        assert_eq!(
            evaluate_staleness(&cursor, Some(Uuid::from_u128(1))),
            StalenessOutcome::Stale
        );
    }

    #[test]
    fn evaluate_staleness_both_none_is_fresh() {
        let cursor = ProjectionCursor {
            last_event_id: None,
            pulled_at: Utc::now(),
        };
        assert_eq!(evaluate_staleness(&cursor, None), StalenessOutcome::Fresh);
    }

    #[test]
    fn evaluate_staleness_server_advanced_from_none_is_stale() {
        let cursor = ProjectionCursor {
            last_event_id: None,
            pulled_at: Utc::now(),
        };
        assert_eq!(
            evaluate_staleness(&cursor, Some(Uuid::nil())),
            StalenessOutcome::Stale
        );
    }

    #[test]
    fn prune_returns_zero_when_vault_root_absent() {
        let dir = tempfile::TempDir::new().unwrap();
        let missing = dir.path().join("does-not-exist");
        let pruned = prune_context(&missing, "anyctx", &HashSet::new()).unwrap();
        assert_eq!(pruned, 0, "absent vault root prunes nothing, no error");
    }

    #[test]
    fn remove_resource_file_deletes_the_canonical_file() {
        let dir = tempfile::TempDir::new().unwrap();
        let root = dir.path();
        let task_dir = root.join("@me/myctx/task");
        std::fs::create_dir_all(&task_dir).unwrap();
        let file = task_dir.join("doomed.md");
        std::fs::write(&file, "body").unwrap();

        remove_resource_file(root, "@me", "myctx", "task", "doomed").unwrap();

        assert!(!file.exists(), "projection file removed");
    }

    #[test]
    fn remove_resource_file_is_ok_when_file_absent() {
        let dir = tempfile::TempDir::new().unwrap();
        // Never-written file: removal is a silent no-op, not an error.
        remove_resource_file(dir.path(), "@me", "myctx", "task", "ghost").unwrap();
    }
}
