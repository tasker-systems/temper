//! Sync orchestration logic — rehash manifest, build requests, push/pull/remove.
//!
//! Pure functions (rehash, build_request, parse_uri, strip_frontmatter) are
//! fully unit-testable. Async functions take client and manifest references.

use std::path::Path;

use uuid::Uuid;

use crate::actions::ingest;
use crate::actions::progress::SyncProgress;
use crate::error::{Result, TemperError};
use temper_core::frontmatter::Frontmatter;
use temper_core::types::managed_meta::MetaUpdatePayload;
use temper_core::types::sync::SyncItemKind;
use temper_core::types::{
    Manifest, ManifestEntry, ManifestEntryState, MergeResult, MergedResource, PushKind, ResourceId,
    SyncCompleteRequest, SyncConflictItem, SyncContextEntries, SyncManifestEntry, SyncPullItem,
    SyncPushItem, SyncRemovedItem, SyncStatusRequest, SyncStatusResponse,
};
use temper_core::vault::Vault;

/// Build the standard "vault file missing for tracked entry" error, with
/// two-pronged recovery guidance (explicit delete vs. resync from server).
///
/// `rel_path` is the manifest entry's relative path
/// (e.g. `task/2026-04-29-some-slug.md`). The slug is derived from the
/// filename stem so the user can paste it directly into
/// `temper resource delete`.
fn vault_file_missing_err(rel_path: &str) -> TemperError {
    let slug = std::path::Path::new(rel_path)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or(rel_path);
    TemperError::NotFound(format!(
        "vault file missing for {slug} at {rel_path}\n\nEither:\n  • To delete the resource, run: temper resource delete {slug}\n  • To recover the file from the server, run: temper sync refresh"
    ))
}

// ---------------------------------------------------------------------------
// Ownership preflight
// ---------------------------------------------------------------------------

/// An entry whose frontmatter `temper-owner` disagrees with the owner segment
/// of its manifest path. Skipped from the sync upload set until resolved.
#[derive(Debug, Clone)]
pub struct OwnershipMismatch {
    pub file_path: String,
    pub frontmatter_owner: String,
    pub manifest_owner: String,
}

// ---------------------------------------------------------------------------
// Single-resource push/pull primitives
//
// These are the factored-out, per-resource orchestration that sync_orchestration
// batches over its diff sets. They also power `temper push <id|path>` and
// `temper pull <id>` as first-class commands.
//
// The `manifest: Option<&mut Manifest>` parameter is the mode switch:
// - Some(...) — local-vault mode, updates the manifest entry in place
// - None     — cloud mode / raw push, no manifest side effects
// ---------------------------------------------------------------------------

/// What a single push targets. `Path` reads frontmatter to locate the id;
/// `Id` requires a manifest to resolve the on-disk path.
#[derive(Debug)]
pub enum PushTarget<'a> {
    Path(&'a std::path::Path),
    Id(ResourceId),
}

/// Per-resource push outcome.
///
/// `kind` reflects the REQUEST shape — `PushKind::New` when the client POSTed
/// (frontmatter had a provisional or missing id), `PushKind::Modified` when
/// the client PUT (canonical id). A PUT that the server responds to with 404
/// currently surfaces as an error; fallback-on-404 is deferred to the
/// cloud-mode work (Unit B.2).
#[derive(Debug, Clone)]
pub struct PushResult {
    pub resource_id: ResourceId,
    pub path: std::path::PathBuf,
    pub kind: PushKind,
}

/// Which pull branch ran.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PullBranch {
    /// Wrote to the manifest-resolved vault path and updated the entry.
    ManifestTracked,
    /// First-sync path: a manifest was available but the id was not yet
    /// tracked. Reconstructed `{owner}/{context}/{doc_type}/{slug}.md` from
    /// server data, wrote full frontmatter, and inserted a manifest entry so
    /// subsequent pulls hit `ManifestTracked`.
    NewlyTracked,
    /// Wrote as `{id}.md` under the caller-provided write root. Reserved for
    /// the no-manifest case (CLI `pull` wrapper passing CWD); sync run never
    /// hits this branch because it always supplies a manifest.
    Snapshot,
}

/// Per-resource pull outcome.
#[derive(Debug, Clone)]
pub struct PullResult {
    pub resource_id: ResourceId,
    pub path: std::path::PathBuf,
    pub branch: PullBranch,
    pub title: String,
}

/// Validate every non-provisional manifest entry: the file's frontmatter
/// `temper-owner` must match the owner segment of its manifest path.
///
/// Returns a list of mismatches to exclude from the upload set. Provisional
/// entries are skipped — their frontmatter IS the authoritative ownership
/// source until they're first synced. Files missing their frontmatter,
/// unreadable, or with a malformed manifest path are also skipped (surfaced
/// by other code paths).
pub fn preflight_ownership_check(manifest: &Manifest, vault_root: &Path) -> Vec<OwnershipMismatch> {
    let mut mismatches = Vec::new();

    for entry in manifest.entries.values() {
        if entry.provisional {
            continue;
        }

        let Some(parsed) = Vault::parse_rel(&entry.path) else {
            continue;
        };
        let manifest_owner = parsed.owner.to_string();

        let abs_path = vault_root.join(&entry.path);
        let content = match std::fs::read_to_string(&abs_path) {
            Ok(c) => c,
            Err(_) => continue,
        };
        let Ok(fm) = temper_core::frontmatter::Frontmatter::try_from(content.as_str()) else {
            continue;
        };
        let frontmatter_owner = fm
            .value()
            .get("temper-owner")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .unwrap_or_else(|| "@me".to_string());

        if frontmatter_owner != manifest_owner {
            mismatches.push(OwnershipMismatch {
                file_path: entry.path.clone(),
                frontmatter_owner,
                manifest_owner,
            });
        }
    }

    mismatches
}

// ---------------------------------------------------------------------------
// Sync result
// ---------------------------------------------------------------------------

/// Summary of a completed sync round.
#[derive(Debug)]
pub struct SyncResult {
    pub push_count: usize,
    pub pull_count: usize,
    pub conflict_count: usize,
    pub removed_count: usize,
    pub scan_count: usize,
    pub merge_auto_count: usize,
    pub merge_conflict_count: usize,
    pub error_count: usize,
}

// ---------------------------------------------------------------------------
// Pure functions (no client, no async — fully unit-testable)
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// Owner sigil resolution
// ---------------------------------------------------------------------------

/// Resolve the API's `owner_handle` shorthand to the canonical owner sigil
/// used in vault paths and `kb_resource_uri()`.
///
/// The API returns the literal string `"@me"` for the requester's own
/// resources (see `OWNER_HANDLE_EXPR` in `resource_service.rs`). The vault
/// layout and the server's `kb_resource_uri()` SQL function use
/// `@<profile.slug>` as the canonical owner segment. This helper closes the
/// gap: callers pass `resource.owner_handle` plus the requester's own
/// `profile.slug` (without leading `@`) and get back the canonical sigil.
///
/// Team handles (`+<team-slug>`) are already canonical and pass through
/// unchanged; so do other users' personal handles.
pub fn resolve_owner_for_frontmatter(handle: &str, profile_slug: &str) -> String {
    if handle == "@me" {
        format!("@{profile_slug}")
    } else {
        handle.to_string()
    }
}

// ---------------------------------------------------------------------------
// normalize_all_entries report
// ---------------------------------------------------------------------------

/// Summary of a full vault normalize pass.
///
/// Returned by [`normalize_all_entries`]. Counts are cumulative over every
/// manifest entry the function iterated over. `issues_by_path` preserves the
/// iteration order of the manifest so callers can print the first few blocked
/// files without re-sorting.
#[derive(Debug, Default)]
pub struct NormalizeReport {
    /// Number of manifest entries processed (regardless of outcome).
    pub scanned: usize,
    /// Number of files rewritten to disk by normalize.
    pub rewritten: usize,
    /// Number of files that had at least one non-auto-fixable issue.
    /// These files still had their hashes updated to reflect on-disk
    /// reality, but sync must block them until the user resolves the
    /// issues.
    pub blocked: usize,
    /// Number of entries whose underlying file was missing on disk.
    pub missing: usize,
    /// Per-file issues keyed by the manifest entry's relative path.
    /// Order is preserved from the iteration order of the manifest.
    pub issues_by_path: Vec<(String, Vec<temper_core::schema::ValidationIssue>)>,
}

/// Normalize every manifest entry's vault file in place, persisting the
/// manifest to disk after each entry so an interrupt loses at most one
/// file's work.
///
/// For each entry:
/// 1. Resolve the entry's path against `vault_root`. If the file is missing,
///    mark the entry `LocalModified` with an empty `body_hash` and continue.
/// 2. Derive doc_type from the entry's vault path via
///    [`temper_core::hash::doc_type_from_vault_path`]. If the path does not
///    yield a known doc type, skip with a warning.
/// 3. Call [`temper_core::normalize::normalize_file`]. On Err, record the
///    error on the report as a "blocked" entry with a synthetic issue
///    containing the error message, and continue.
/// 4. Update the entry's `body_hash`, `managed_hash`, `open_hash`,
///    `mtime_secs`, and `state` from the
///    [`temper_core::normalize::NormalizeOutcome`].
/// 5. Persist the manifest to disk immediately via
///    [`crate::manifest_io::save_manifest`].
/// 6. If the outcome has non-empty `issues`, increment `report.blocked` and
///    record the issues.
///
/// Returns a [`NormalizeReport`]; does not return an error for per-file
/// problems. Only returns `Err` for problems that prevent iteration itself.
pub fn normalize_all_entries(
    manifest: &mut Manifest,
    vault_root: &Path,
    temper_dir: &Path,
    progress: Option<&dyn SyncProgress>,
) -> Result<NormalizeReport> {
    let mut report = NormalizeReport::default();

    // Snapshot the resource ids in iteration order so we can mutate entries
    // without juggling the iterator.
    let ids: Vec<ResourceId> = manifest.entries.keys().copied().collect();
    let total = ids.len();

    for (idx, id) in ids.iter().enumerate() {
        report.scanned += 1;

        // Clone out the fields we need up-front so we can drop the borrow
        // before calling save_manifest.
        let (rel_path, prior_remote_body, prior_remote_managed, prior_remote_open) = {
            let Some(entry) = manifest.entries.get(id) else {
                continue;
            };
            (
                entry.path.clone(),
                entry.remote_body_hash.clone(),
                entry.remote_managed_hash.clone(),
                entry.remote_open_hash.clone(),
            )
        };

        let abs_path = vault_root.join(&rel_path);

        // Missing file: mirror rehash_manifest's prior behavior.
        if !abs_path.exists() {
            report.missing += 1;
            if let Some(entry) = manifest.entries.get_mut(id) {
                if entry.state != ManifestEntryState::LocalModified {
                    entry.state = ManifestEntryState::LocalModified;
                }
                entry.body_hash = String::new();
                entry.mtime_secs = None;
            }
            crate::manifest_io::save_manifest(temper_dir, manifest)?;
            if let Some(p) = progress {
                p.rehash_progress(idx + 1, total, 0);
            }
            continue;
        }

        // Derive doc type from the vault path. Unknown types: warn and skip.
        let Some(doc_type) = temper_core::hash::doc_type_from_vault_path(&rel_path) else {
            tracing::warn!(
                "normalize_all_entries: skipping {} — cannot derive doc_type from path",
                rel_path
            );
            if let Some(p) = progress {
                p.rehash_progress(idx + 1, total, 0);
            }
            continue;
        };
        let doc_type = doc_type.to_string();

        // Run the normalize primitive.
        match temper_core::normalize::normalize_file(&abs_path, &doc_type) {
            Ok(outcome) => {
                if outcome.changed {
                    report.rewritten += 1;
                }
                let has_issues = !outcome.issues.is_empty();
                if has_issues {
                    report.blocked += 1;
                    report
                        .issues_by_path
                        .push((rel_path.clone(), outcome.issues.clone()));
                }

                let new_mtime = file_mtime_secs(&abs_path).ok();

                if let Some(entry) = manifest.entries.get_mut(id) {
                    entry.body_hash = outcome.body_hash.clone();
                    entry.managed_hash = outcome.managed_hash.clone();
                    entry.open_hash = outcome.open_hash.clone();
                    entry.mtime_secs = new_mtime;

                    // Only touch state when it is Clean or LocalModified;
                    // leave Conflict / Pending / RemoteModified markers
                    // alone. Match the hashes against the known remote
                    // triple to decide between Clean and LocalModified.
                    let touches_state = matches!(
                        entry.state,
                        ManifestEntryState::Clean | ManifestEntryState::LocalModified
                    );
                    if touches_state {
                        let remote_matches = !prior_remote_body.is_empty()
                            && outcome.body_hash == prior_remote_body
                            && outcome.managed_hash == prior_remote_managed
                            && outcome.open_hash == prior_remote_open;
                        entry.state = if remote_matches {
                            ManifestEntryState::Clean
                        } else {
                            ManifestEntryState::LocalModified
                        };
                    }
                }
            }
            Err(e) => {
                // Record the error as a synthetic blocked issue.
                report.blocked += 1;
                let issue = temper_core::schema::ValidationIssue {
                    path: rel_path.clone(),
                    message: format!("normalize_file failed: {e}"),
                    auto_fixable: false,
                };
                report.issues_by_path.push((rel_path.clone(), vec![issue]));
            }
        }

        // Per-entry atomic save. If we're interrupted after this point, at
        // most the NEXT entry's normalize is lost — everything up to here
        // is durably persisted.
        crate::manifest_io::save_manifest(temper_dir, manifest)?;

        if let Some(p) = progress {
            p.rehash_progress(idx + 1, total, 0);
        }
    }

    Ok(report)
}

/// Rehash manifest entries by reading vault files and computing SHA-256.
/// Skips files whose mtime hasn't changed since the last manifest update.
/// Returns the count of entries whose state changed.
pub fn rehash_manifest(manifest: &mut Manifest, vault_root: &Path) -> Result<usize> {
    let mut changed = 0;
    for (_id, entry) in manifest.entries.iter_mut() {
        let file_path = vault_root.join(&entry.path);
        if !file_path.exists() {
            if entry.state != ManifestEntryState::LocalModified {
                entry.state = ManifestEntryState::LocalModified;
                entry.body_hash = String::new();
                entry.mtime_secs = None;
                changed += 1;
            }
            continue;
        }

        let file_mtime = file_mtime_secs(&file_path)?;

        // Skip rehash if mtime hasn't changed AND all hashes are populated.
        // If managed_hash or open_hash are empty, we must recompute them even
        // if the file hasn't been modified (backfill for entries created before
        // three-tier hashing was wired in).
        let hashes_complete = !entry.managed_hash.is_empty() && !entry.open_hash.is_empty();
        if entry.mtime_secs == Some(file_mtime) && hashes_complete {
            continue;
        }

        let content = std::fs::read_to_string(&file_path)?;
        let body = strip_frontmatter(&content);
        let current_hash = temper_core::hash::compute_body_hash(body);

        // Compute frontmatter tier hashes via the authoritative module.
        // If the file can't be parsed as a Frontmatter — e.g. broken YAML,
        // missing `temper-type`, legacy `type:` key — preserve the entry's
        // existing hashes untouched and warn loudly so the user can review.
        // Clobbering the hashes with empty-JSON values (the historical
        // silent-swallow behavior) would misreport the file as
        // "meta-unchanged against server" on the next sync.
        let fm = match Frontmatter::try_from(content.as_str()) {
            Ok(fm) => fm,
            Err(e) => {
                tracing::warn!(
                    path = %file_path.display(),
                    error = %e,
                    "skipping rehash: frontmatter parse failed — existing manifest hashes preserved; run `temper doctor` to diagnose"
                );
                continue;
            }
        };
        let (managed_hash, open_hash) = fm.hashes();

        entry.mtime_secs = Some(file_mtime);

        if current_hash != entry.body_hash
            || managed_hash != entry.managed_hash
            || open_hash != entry.open_hash
        {
            entry.body_hash = current_hash;
            entry.managed_hash = managed_hash;
            entry.open_hash = open_hash;
            entry.state = ManifestEntryState::LocalModified;
            changed += 1;
        }
    }
    Ok(changed)
}

/// Extract file modification time as seconds since the Unix epoch.
fn file_mtime_secs(path: &Path) -> Result<i64> {
    let metadata = std::fs::metadata(path)?;
    let mtime = metadata.modified().map_err(|e| {
        TemperError::Config(format!("cannot read mtime for {}: {e}", path.display()))
    })?;
    Ok(mtime
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64)
}

/// Build a SyncStatusRequest from the manifest, optionally filtered by contexts.
pub fn build_status_request(manifest: &Manifest, context_filter: &[String]) -> SyncStatusRequest {
    let mut context_map: std::collections::HashMap<String, Vec<SyncManifestEntry>> =
        std::collections::HashMap::new();

    for (id, entry) in &manifest.entries {
        let Some(parsed) = Vault::parse_rel(&entry.path) else {
            // Malformed manifest entry — skip with a warning.
            tracing::warn!("skipping malformed manifest path: {}", entry.path);
            continue;
        };

        let ctx = parsed.context.to_string();
        let doc_type = parsed.doc_type.to_string();

        if !context_filter.is_empty() && !context_filter.contains(&ctx) {
            continue;
        }

        let uri = Vault::canonical_uri(parsed.owner, &ctx, &doc_type, &id.to_string());

        context_map.entry(ctx).or_default().push(SyncManifestEntry {
            uri,
            local_hash: entry.body_hash.clone(),
            remote_hash: entry.remote_body_hash.clone(),
            managed_hash: entry.managed_hash.clone(),
            remote_managed_hash: entry.remote_managed_hash.clone(),
            open_hash: entry.open_hash.clone(),
            remote_open_hash: entry.remote_open_hash.clone(),
        });
    }

    let contexts = context_map
        .into_iter()
        .map(|(name, entries)| SyncContextEntries { name, entries })
        .collect();

    SyncStatusRequest { contexts }
}

/// Build a SyncCompleteRequest.
pub fn build_complete_request(device_id: &str, merged: Vec<MergedResource>) -> SyncCompleteRequest {
    SyncCompleteRequest {
        device_id: device_id.to_string(),
        merged_resources: merged,
    }
}

/// Strip YAML frontmatter from vault file content.
pub fn strip_frontmatter(content: &str) -> &str {
    if let Some(after_open) = content.strip_prefix("---\n") {
        if let Some(end) = after_open.find("\n---\n") {
            return &after_open[end + 5..];
        }
    }
    content
}

/// Extract the frontmatter block (including delimiters) from file content.
fn extract_frontmatter_block(content: &str) -> &str {
    if let Some(after_open) = content.strip_prefix("---\n") {
        if let Some(end) = after_open.find("\n---\n") {
            return &content[..4 + end + 5]; // "---\n" + content + "\n---\n"
        }
    }
    ""
}

/// Parse a kb:// URI into (context, doc_type).
pub fn parse_kb_uri(uri: &str) -> Result<(String, String)> {
    let parsed = Vault::parse_uri(uri).ok_or_else(|| {
        TemperError::Config(format!(
            "invalid kb:// URI (expected kb://<owner>/<context>/<doc_type>/<ident>): {uri}"
        ))
    })?;
    Ok((parsed.context.to_string(), parsed.doc_type.to_string()))
}

/// Extract resource UUID from last segment of a kb:// URI.
pub fn extract_resource_id(uri: &str) -> Result<ResourceId> {
    let uuid_str = uri
        .rsplit('/')
        .next()
        .ok_or_else(|| TemperError::Config(format!("no UUID segment in URI: {uri}")))?;
    Uuid::parse_str(uuid_str)
        .map(ResourceId::from)
        .map_err(|e| TemperError::Config(format!("invalid UUID in URI {uri}: {e}")))
}

// ---------------------------------------------------------------------------
// Vault scanning
// ---------------------------------------------------------------------------

/// Scan the vault directory for untracked markdown files.
pub fn scan_vault_for_untracked(
    manifest: &mut temper_core::types::Manifest,
    vault_root: &Path,
    progress: &dyn SyncProgress,
) -> Result<usize> {
    let known_paths: std::collections::HashSet<String> =
        manifest.entries.values().map(|e| e.path.clone()).collect();

    let mut found = 0;

    for entry in ignore::WalkBuilder::new(vault_root)
        .hidden(true)
        .build()
        .filter_map(|e| e.ok())
    {
        let path = entry.path();

        if path.extension().and_then(|e| e.to_str()) != Some("md") {
            continue;
        }

        if path.starts_with(vault_root.join(".temper")) {
            continue;
        }

        let rel_path = path
            .strip_prefix(vault_root)
            .unwrap_or(path)
            .to_string_lossy()
            .to_string();

        if Vault::parse_rel(&rel_path).is_none() {
            tracing::warn!(
                "scanned path is not owner-scoped — the vault may need migration: {rel_path}"
            );
            continue;
        }

        if known_paths.contains(&rel_path) {
            continue;
        }

        let content = std::fs::read_to_string(path)?;
        let fm = ingest::parse_source_frontmatter(&content);

        let fm_context = fm.as_ref().and_then(|f| f.context.as_deref());
        let fm_doc_type = fm.as_ref().and_then(|f| f.doc_type.as_deref());

        let (context, doc_type) =
            match ingest::infer_context_and_doctype(vault_root, path, fm_context, fm_doc_type) {
                Ok(pair) => pair,
                Err(e) => {
                    progress.scan_skipped(&rel_path, &e.to_string());
                    continue;
                }
            };

        // Determine resource ID and provisional status:
        // - temper-id present → server-confirmed, not provisional
        // - temper-provisional-id present → locally-generated, provisional
        // - neither → mint new UUID, provisional
        let (resource_id, is_provisional) = if let Some(tid) = fm
            .as_ref()
            .and_then(|f| f.legacy_id.as_deref())
            .and_then(|id| Uuid::parse_str(id).ok())
        {
            (ResourceId::from(tid), false)
        } else if let Some(pid) = fm
            .as_ref()
            .and_then(|f| f.provisional_id.as_deref())
            .and_then(|id| Uuid::parse_str(id).ok())
        {
            (ResourceId::from(pid), true)
        } else {
            (ResourceId::new(), true)
        };
        if fm.is_none() {
            let frontmatter = ingest::build_provisional_frontmatter(
                resource_id,
                &ingest::title_from_path(path),
                &context,
                &doc_type,
            );
            let new_content = format!("{frontmatter}{content}");
            std::fs::write(path, &new_content)?;
        }

        let full_content = std::fs::read_to_string(path)?;
        let body = strip_frontmatter(&full_content);
        let content_hash = temper_core::hash::compute_body_hash(body);
        let mtime = file_mtime_secs(path).ok();

        // Files that passed the lenient `parse_source_frontmatter` check
        // above may still fail the stricter `Frontmatter::try_from` if they
        // carry legacy-form frontmatter (e.g. `type:` instead of
        // `temper-type:`). Skip such files during untracked discovery —
        // they need `temper doctor fix` to migrate before they can be
        // tracked. Don't insert them into the manifest with empty hashes.
        let fm = match Frontmatter::try_from(full_content.as_str()) {
            Ok(fm) => fm,
            Err(e) => {
                tracing::warn!(
                    path = %path.display(),
                    error = %e,
                    "skipping untracked file: legacy or malformed frontmatter — run `temper doctor fix` to migrate before syncing"
                );
                continue;
            }
        };
        let (managed_hash, open_hash) = fm.hashes();

        manifest.entries.insert(
            resource_id,
            temper_core::types::ManifestEntry {
                path: rel_path.clone(),
                body_hash: content_hash,
                remote_body_hash: String::new(),
                managed_hash,
                open_hash,
                remote_managed_hash: String::new(),
                remote_open_hash: String::new(),
                synced_at: chrono::Utc::now(),
                state: temper_core::types::ManifestEntryState::Pending,
                mtime_secs: mtime,
                last_audit_id: None,
                provisional: is_provisional,
            },
        );

        progress.scan_found(&rel_path, &context, &doc_type);
        found += 1;
    }

    Ok(found)
}

// ---------------------------------------------------------------------------
// Orchestration (async, uses client + manifest)
// ---------------------------------------------------------------------------

/// Run the full sync orchestration.
///
/// Called from `sync_cmd.rs` with a single tokio runtime. The command handles
/// manifest load/save and output formatting.
pub async fn sync_orchestration(
    client: &temper_client::TemperClient,
    manifest: &mut Manifest,
    vault_root: &Path,
    context_filter: &[String],
    progress: &dyn SyncProgress,
    skip_paths: &std::collections::HashSet<String>,
) -> Result<SyncResult> {
    // Step 1: Scan vault for untracked files
    let scan_count = scan_vault_for_untracked(manifest, vault_root, progress)?;
    progress.phase_summary("scan", scan_count);

    // Step 2: Rehash manifest
    rehash_manifest(manifest, vault_root)?;

    // Step 3: Request diff
    let request = build_status_request(manifest, context_filter);
    let diff = client
        .sync()
        .status(&request)
        .await
        .map_err(crate::commands::client_err)?;

    let push_count = diff.to_push.len();
    let pull_count = diff.to_pull.len();
    let removed_count = diff.removed.len();

    // Step 4: Push
    let mut error_count = 0;
    for item in &diff.to_push {
        // Skip items whose resolved path is in the ownership-mismatch set.
        if let Some(path) = resolve_push_entry_path(manifest, item) {
            if skip_paths.contains(&path) {
                continue;
            }
        }
        let kind = if item.resource_id.is_some() {
            PushKind::Modified
        } else {
            PushKind::New
        };
        let entry_path = resolve_push_entry_path(manifest, item);
        if let Some(path) = &entry_path {
            progress.push_start(path, kind);
        }
        match push_resource(client, manifest, vault_root, item).await {
            Ok(()) => {
                if let Some(path) = &entry_path {
                    progress.push_done(path);
                }
            }
            Err(e) => {
                let (path, context, doc_type) = push_error_context(manifest, item);
                progress.push_error(&path, &context, &doc_type, &e.to_string());
                error_count += 1;
            }
        }
    }
    progress.phase_summary("push", push_count);

    // Step 5: Pull
    for item in &diff.to_pull {
        progress.pull_start(&item.uri);
        match pull_resource(client, manifest, vault_root, item).await {
            Ok(()) => {
                if let Some(entry) = manifest.entries.get(&item.resource_id) {
                    progress.pull_done(&entry.path);
                }
            }
            Err(e) => {
                progress.pull_error(&item.uri, &e.to_string());
                error_count += 1;
            }
        }
    }
    progress.phase_summary("pull", pull_count);

    // Step 6-7: Merge conflicts and push merged
    let mut merge_auto_count = 0;
    let mut merge_conflict_count = 0;
    for item in &diff.conflicts {
        match merge_and_push_resource(client, manifest, vault_root, item, progress).await {
            Ok(merge_result) => match merge_result {
                MergeResult::AutoMerged { .. } => merge_auto_count += 1,
                MergeResult::ConflictAnnotated { .. } => merge_conflict_count += 1,
            },
            Err(e) => {
                let path = manifest
                    .entries
                    .get(&item.resource_id)
                    .map(|entry| entry.path.as_str())
                    .unwrap_or(&item.uri);
                progress.merge_error(path, &e.to_string());
                error_count += 1;
            }
        }
    }
    let conflict_count = diff.conflicts.len();
    progress.phase_summary("merge", conflict_count);

    // Step 8: Handle removed
    for item in &diff.removed {
        remove_resource(manifest, vault_root, item)?;
    }
    progress.phase_summary("remove", removed_count);

    // Step 9: Complete
    let complete_req = build_complete_request(&manifest.device_id, vec![]);
    let complete_resp = client
        .sync()
        .complete(&complete_req)
        .await
        .map_err(crate::commands::client_err)?;

    // Step 10: Update manifest timestamp
    manifest.last_sync = Some(complete_resp.last_sync_at);

    Ok(SyncResult {
        push_count,
        pull_count,
        conflict_count,
        removed_count,
        scan_count,
        merge_auto_count,
        merge_conflict_count,
        error_count,
    })
}

/// Run a dry-run sync (rehash + status only, no changes).
pub async fn sync_status_check(
    client: &temper_client::TemperClient,
    manifest: &mut Manifest,
    vault_root: &Path,
    context_filter: &[String],
    progress: &dyn SyncProgress,
) -> Result<SyncStatusResponse> {
    scan_vault_for_untracked(manifest, vault_root, progress)?;
    rehash_manifest(manifest, vault_root)?;

    let request = build_status_request(manifest, context_filter);
    client
        .sync()
        .status(&request)
        .await
        .map_err(crate::commands::client_err)
}

// ---------------------------------------------------------------------------
// Push / Pull / Remove
// ---------------------------------------------------------------------------

/// Resolve the vault path for a push item (for progress reporting).
fn resolve_push_entry_path(manifest: &Manifest, item: &SyncPushItem) -> Option<String> {
    item.resource_id
        .and_then(|id| manifest.entries.get(&id))
        .or_else(|| {
            extract_resource_id(&item.uri)
                .ok()
                .and_then(|id| manifest.entries.get(&id))
        })
        .map(|entry| entry.path.clone())
}

/// Extract context info for a push error message.
fn push_error_context(manifest: &Manifest, item: &SyncPushItem) -> (String, String, String) {
    let entry = item
        .resource_id
        .and_then(|id| manifest.entries.get(&id))
        .or_else(|| {
            extract_resource_id(&item.uri)
                .ok()
                .and_then(|id| manifest.entries.get(&id))
        });

    if let Some(entry) = entry {
        if let Some(parsed) = Vault::parse_rel(&entry.path) {
            return (
                entry.path.clone(),
                parsed.context.to_string(),
                parsed.doc_type.to_string(),
            );
        }
        return (
            entry.path.clone(),
            "unknown".to_string(),
            "unknown".to_string(),
        );
    }

    (
        item.uri.clone(),
        "unknown".to_string(),
        "unknown".to_string(),
    )
}

async fn push_resource(
    client: &temper_client::TemperClient,
    manifest: &mut Manifest,
    vault_root: &Path,
    item: &SyncPushItem,
) -> Result<()> {
    match item.kind {
        SyncItemKind::Body => {
            // Delegate body pushes to the unified primitive. Sync always has
            // a manifest and the entry is guaranteed to exist (it's what
            // surfaced this push in the sync diff). Resolve the id the same
            // way push_resource_body used to.
            //
            // Semantic note vs. the old push_resource_body:
            // - Old code rewrote the local file (provisional→canonical) only
            //   inside the manifest-remove branch. The primitive rewrites
            //   unconditionally when server_id differs from entry_id. Sync
            //   always has a manifest + entry, so the difference only affects
            //   the rare "manifest entry vanishes mid-push" race — in which
            //   case rewriting the file is still the right thing to do
            //   because the server took the payload.
            // - The primitive also cross-checks frontmatter id against the
            //   manifest-hinted id for PushTarget::Id. In sync, these are
            //   guaranteed to match (the manifest entry is the sole source
            //   of the id we resolved), but the check catches any future
            //   corruption.
            let entry_id = match item.resource_id {
                Some(id) => id,
                None => extract_resource_id(&item.uri)?,
            };
            push_one_resource(client, vault_root, PushTarget::Id(entry_id), Some(manifest))
                .await
                .map(|_| ())
        }
        SyncItemKind::MetaOnly => push_resource_meta_only(client, manifest, vault_root, item).await,
    }
}

/// Build a meta-only update payload from a parsed Frontmatter.
///
/// Splits frontmatter into managed/open tiers, computes their hashes, and
/// returns a typed `MetaUpdatePayload` ready to send to the server. The
/// managed tier round-trips through `ManagedMeta`'s `extra` flatten bucket
/// so the pre-deserialized JSON hash stays stable.
///
/// **Caller contract:** `fm.doc_type()` is the authoritative doctype for
/// the resulting payload's tier routing. Callers that derive a separate
/// `doc_type` from elsewhere (e.g. manifest path) must verify the two
/// agree before calling — see `push_resource_meta_only` for the reference
/// guard pattern.
fn build_meta_update_payload(fm: &Frontmatter, resource_id: Uuid) -> MetaUpdatePayload {
    let managed_meta_json = fm.managed_json();
    let open_meta = fm.open_json();
    let (managed_hash, open_hash) = fm.hashes();
    let managed_meta: temper_core::types::managed_meta::ManagedMeta =
        serde_json::from_value(managed_meta_json).unwrap_or_default();
    MetaUpdatePayload {
        resource_id: ResourceId::from(resource_id),
        managed_meta,
        open_meta,
        managed_hash,
        open_hash,
    }
}

async fn push_resource_meta_only(
    client: &temper_client::TemperClient,
    manifest: &mut Manifest,
    vault_root: &Path,
    item: &SyncPushItem,
) -> Result<()> {
    let entry_id = match item.resource_id {
        Some(id) => id,
        None => extract_resource_id(&item.uri)?,
    };

    let entry = manifest
        .entries
        .get(&entry_id)
        .ok_or_else(|| TemperError::NotFound(format!("manifest entry not found: {entry_id}")))?;

    let file_path = vault_root.join(&entry.path);
    if !file_path.exists() {
        return Err(vault_file_missing_err(&entry.path));
    }

    let content = std::fs::read_to_string(&file_path)?;

    // Unlike the body push path, we cannot fall back to a default doc_type
    // here — `Frontmatter::managed_json` uses the parsed doctype to decide
    // which fields are managed vs open, and a doc_type mismatch would
    // misclassify fields and corrupt the server-side meta state.
    let doc_type = Vault::parse_rel(&entry.path)
        .map(|parsed| parsed.doc_type.to_string())
        .ok_or_else(|| {
            TemperError::Config(format!(
                "meta-only push: manifest path does not parse: {}",
                entry.path
            ))
        })?;

    let fm = Frontmatter::try_from(content.as_str()).map_err(|e| {
        TemperError::Config(format!(
            "meta-only push requires parseable frontmatter at {}: {e}",
            file_path.display()
        ))
    })?;

    // Sanity check: the manifest-derived doc_type should agree with the
    // parsed frontmatter. Mismatch here means the manifest path is out
    // of sync with file contents — refuse the push rather than corrupt
    // the server's tier routing.
    if fm.doc_type().as_str() != doc_type {
        return Err(TemperError::Config(format!(
            "meta-only push: manifest path says doc_type '{}' but file frontmatter says '{}': {}",
            doc_type,
            fm.doc_type().as_str(),
            file_path.display()
        )));
    }

    let payload = build_meta_update_payload(&fm, entry_id.into());

    client
        .resources()
        .update_meta(entry_id.into(), &payload)
        .await
        .map_err(crate::commands::client_err)?;

    // body_hash / remote_body_hash intentionally untouched: the diff was
    // meta-only, so the body on disk is identical to what the server holds.
    if let Some(e) = manifest.entries.get_mut(&entry_id) {
        e.remote_managed_hash = payload.managed_hash.clone();
        e.remote_open_hash = payload.open_hash.clone();
        e.state = ManifestEntryState::Clean;
        e.synced_at = chrono::Utc::now();
        e.mtime_secs = file_mtime_secs(&file_path).ok();
    }

    Ok(())
}

/// Try to extract a resource id from frontmatter. Returns `Some((id,
/// is_provisional))` if exactly one of `temper-id` / `temper-provisional-id`
/// is present, `None` if neither is present, and an error if both are present
/// or a uuid fails to parse.
///
/// `Frontmatter` has no dedicated accessor for these keys, so we read them
/// straight out of `fm.value()` (the parsed YAML mapping) — see
/// `crates/temper-core/src/frontmatter/projections.rs:67-78` for the
/// reference pattern.
fn try_extract_id_from_frontmatter(fm: &Frontmatter) -> Result<Option<(ResourceId, bool)>> {
    let mapping = fm
        .value()
        .as_mapping()
        .ok_or_else(|| TemperError::Config("frontmatter is not a mapping".into()))?;
    let canonical = mapping
        .get(serde_yaml::Value::String("temper-id".into()))
        .and_then(|v| v.as_str());
    let provisional = mapping
        .get(serde_yaml::Value::String("temper-provisional-id".into()))
        .and_then(|v| v.as_str());

    match (canonical, provisional) {
        (Some(s), None) => {
            let uuid = Uuid::parse_str(s)
                .map_err(|e| TemperError::Config(format!("invalid temper-id uuid: {e}")))?;
            Ok(Some((ResourceId::from(uuid), false)))
        }
        (None, Some(s)) => {
            let uuid = Uuid::parse_str(s).map_err(|e| {
                TemperError::Config(format!("invalid temper-provisional-id uuid: {e}"))
            })?;
            Ok(Some((ResourceId::from(uuid), true)))
        }
        (Some(_), Some(_)) => Err(TemperError::Config(
            "frontmatter has both temper-id and temper-provisional-id (invalid state)".into(),
        )),
        (None, None) => Ok(None),
    }
}

/// Push a single resource.
///
/// `PushTarget::Path` resolves the id from the file's frontmatter (either
/// `temper-id` canonical → PUT, or `temper-provisional-id` → POST).
/// `PushTarget::Id` requires a manifest to resolve the on-disk path; the
/// entry's `provisional` flag determines POST vs PUT.
///
/// If `manifest` is `Some`, the entry is updated in place: on a
/// provisional→canonical transition the key is remapped from the local id
/// to the server-assigned id and the `provisional` flag is cleared; on every
/// push the full nine-field entry state is refreshed (body/managed/open
/// hashes for both local and remote, state, synced_at, mtime_secs). If
/// `manifest` is `None`, the file is still rewritten when the server
/// assigns a new id, but no manifest side effects occur — this is the
/// cloud-mode / raw-push shape.
///
/// The "remote" hashes mirror the locally-computed values on push: the
/// client-sent body IS the server's authoritative source after a
/// successful POST/PUT, so there is no divergence to track (unlike pull,
/// where `expected_remote_hash` threads the server-declared hash
/// separately).
pub async fn push_one_resource(
    client: &temper_client::TemperClient,
    vault_root: &Path,
    target: PushTarget<'_>,
    manifest: Option<&mut Manifest>,
) -> Result<PushResult> {
    // ---- Step A — resolve file_path (+ optional manifest hint) ------------
    //
    // The manifest hint (entry_id, provisional flag) is used for the
    // `PushTarget::Id` branch as a cross-check against frontmatter: we read
    // and parse the file exactly once in Step B, and frontmatter remains
    // the authoritative source of the id. If a caller asks us to push by
    // id and the on-disk file's frontmatter disagrees, that's surfaced as
    // an error rather than silently pushing the wrong resource.
    let (file_path, manifest_hint): (std::path::PathBuf, Option<(ResourceId, bool)>) =
        match target {
            PushTarget::Path(p) => {
                let abs: std::path::PathBuf = if p.is_absolute() {
                    p.to_path_buf()
                } else {
                    vault_root.join(p)
                };
                if !abs.exists() {
                    return Err(TemperError::NotFound(format!(
                        "file not found: {}",
                        abs.display()
                    )));
                }
                (abs, None)
            }
            PushTarget::Id(id) => {
                let m = manifest.as_ref().ok_or_else(|| {
                    TemperError::Config(
                        "push by id requires a manifest; pass a path for manifest-less push".into(),
                    )
                })?;
                let entry = m.entries.get(&id).ok_or_else(|| {
                    TemperError::NotFound(format!("manifest entry not found: {id}"))
                })?;
                let abs = vault_root.join(&entry.path);
                if !abs.exists() {
                    return Err(vault_file_missing_err(&entry.path));
                }
                (abs, Some((id, entry.provisional)))
            }
        };

    // ---- Step B — single file read + single frontmatter parse -------------
    let content = std::fs::read_to_string(&file_path)?;
    let fm = Frontmatter::try_from(content.as_str()).map_err(|e| {
        TemperError::Config(format!(
            "push requires parseable frontmatter at {}: {e}",
            file_path.display()
        ))
    })?;
    let fm_id = try_extract_id_from_frontmatter(&fm)?;

    // Resolve the authoritative id + provisional flag. For `PushTarget::Path`
    // the file's frontmatter is the sole source. For `PushTarget::Id` the
    // manifest entry is authoritative (sync and other manifest-driven paths
    // may legitimately push files whose frontmatter never received a
    // temper-id — e.g. server-seeded resources whose vault file was written
    // without id echo). When both are present we cross-check and surface any
    // divergence as an error rather than silently pushing under the wrong id.
    let (entry_id, is_provisional) = match (manifest_hint, fm_id) {
        (Some((hinted_id, hinted_prov)), Some((fm_entry_id, fm_prov))) => {
            if hinted_id != fm_entry_id {
                return Err(TemperError::Config(format!(
                    "push-by-id mismatch: manifest entry points to {} but file frontmatter says {}",
                    Uuid::from(hinted_id),
                    Uuid::from(fm_entry_id)
                )));
            }
            // Prefer the manifest's provisional flag — it's the state machine
            // of record. Any drift between fm and manifest on the provisional
            // bit would be caught the next rehash/status pass.
            let _ = fm_prov;
            (hinted_id, hinted_prov)
        }
        (Some((hinted_id, hinted_prov)), None) => (hinted_id, hinted_prov),
        (None, Some(pair)) => pair,
        (None, None) => {
            return Err(TemperError::Config(format!(
                "push requires a resource id: {} has neither temper-id nor temper-provisional-id, and no manifest hint was supplied",
                file_path.display()
            )));
        }
    };

    let body = crate::actions::ingest::strip_frontmatter(&content);

    // Prefer vault-relative path parsing; fall back to frontmatter fields
    // for files outside the `@owner/context/doc-type/slug.md` layout (the
    // case that motivates manifest-less push).
    let rel_parsed = file_path
        .strip_prefix(vault_root)
        .ok()
        .and_then(|p| p.to_str().map(String::from))
        .and_then(|s| {
            Vault::parse_rel(&s).map(|p| (p.context.to_string(), p.doc_type.to_string()))
        });

    let (context, doc_type) = match rel_parsed {
        Some(cd) => cd,
        None => {
            let mapping = fm
                .value()
                .as_mapping()
                .ok_or_else(|| TemperError::Config("frontmatter is not a mapping".into()))?;
            let ctx = mapping
                .get(serde_yaml::Value::String("temper-context".into()))
                .and_then(|v| v.as_str())
                .unwrap_or("default")
                .to_string();
            let doctype = fm.doc_type().as_str().to_string();
            (ctx, doctype)
        }
    };

    let managed_meta = Some(fm.managed_json());
    let open_meta = Some(fm.open_json());
    // Title comes from frontmatter (parse-time normalize_aliases means both
    // legacy `title:` and canonical `temper-title:` files surface as
    // `temper-title` in managed_json). The path stem is the slug, not the
    // title — using it here would propagate slug-as-title to payload.title,
    // and Phase 5's receive-side ensure_managed_identity_keys would then
    // overwrite the (correct) temper-title in managed_meta with that slug.
    let title = managed_meta
        .as_ref()
        .and_then(|m| m.get("temper-title"))
        .and_then(|v| v.as_str())
        .map(String::from)
        .unwrap_or_else(|| crate::actions::ingest::title_from_path(&file_path));
    let mut payload = crate::actions::ingest::build_ingest_payload(
        body, &title, &context, &doc_type, None, None, None,
    )?;
    payload.managed_meta = managed_meta;
    payload.open_meta = open_meta;

    // ---- Step C — POST (provisional / new) or PUT (canonical) -------------
    let push_kind = if is_provisional {
        PushKind::New
    } else {
        PushKind::Modified
    };
    let resource = if is_provisional {
        client
            .ingest()
            .create(&payload)
            .await
            .map_err(crate::commands::client_err)?
    } else {
        client
            .ingest()
            .update(Uuid::from(entry_id), &payload)
            .await
            .map_err(crate::commands::client_err)?
    };
    let server_id = ResourceId::from(Uuid::from(resource.id));

    // ---- Step D — provisional → canonical rewrite (file + manifest) ------
    let mut manifest = manifest;
    if server_id != entry_id || is_provisional {
        let file_content = std::fs::read_to_string(&file_path)?;
        let entry_uuid = Uuid::from(entry_id);
        let server_uuid = Uuid::from(server_id);
        let updated = file_content
            .replace(
                &format!("temper-provisional-id: \"{entry_uuid}\""),
                &format!("temper-id: \"{server_uuid}\""),
            )
            .replace(
                &format!("temper-provisional-id: {entry_uuid}"),
                &format!("temper-id: {server_uuid}"),
            );
        let updated = if updated != file_content {
            updated
        } else {
            // Fallback: file already had temper-id with the local UUID.
            file_content.replace(&entry_uuid.to_string(), &server_uuid.to_string())
        };
        if updated != file_content {
            std::fs::write(&file_path, &updated)?;
        } else {
            tracing::warn!(
                %entry_id,
                "provisional id not found in file content — frontmatter not updated"
            );
        }

        if let Some(m) = manifest.as_mut() {
            if let Some(mut entry) = m.entries.remove(&entry_id) {
                entry.provisional = false;
                m.entries.insert(server_id, entry);
            }
        }
    }

    // ---- Step E — post-write hashes + full manifest entry update ---------
    // The file was just rewritten (possibly with a new canonical id in the
    // frontmatter), so re-parse from disk to get hashes reflecting what's
    // actually there now.
    let fm_written = Frontmatter::parse_file(&file_path).map_err(|e| {
        TemperError::Vault(format!(
            "push_one_resource post-write hash compute {}: {e}",
            file_path.display()
        ))
    })?;
    let (managed_hash, open_hash) = fm_written.hashes();
    // Compute body hash from the on-disk body directly. We have `body`
    // (stripped frontmatter) in scope from Step B, and this avoids
    // depending on build_ingest_payload's Option<String> contract.
    let body_hash = temper_core::hash::compute_body_hash(body);

    if let Some(m) = manifest.as_mut() {
        if let Some(e) = m.entries.get_mut(&server_id) {
            e.body_hash = body_hash.clone();
            e.remote_body_hash = body_hash;
            e.managed_hash = managed_hash.clone();
            e.open_hash = open_hash.clone();
            e.remote_managed_hash = managed_hash;
            e.remote_open_hash = open_hash;
            e.state = ManifestEntryState::Clean;
            e.synced_at = chrono::Utc::now();
            e.mtime_secs = file_mtime_secs(&file_path).ok();
        }
    }

    Ok(PushResult {
        resource_id: server_id,
        path: file_path,
        kind: push_kind,
    })
}

/// Publish a freshly-written local file to the server. For Local mode
/// only. Loads the manifest, pushes via `push_one_resource(PushTarget::Path)`,
/// saves the manifest.
///
/// Precondition: `file_path` exists and has either `temper-provisional-id`
/// or `temper-id` in frontmatter.
///
/// Postcondition: server has the latest content; manifest entry reflects
/// the canonical `temper-id` and current hashes.
pub async fn publish_local_write(
    client: &temper_client::TemperClient,
    vault_root: &std::path::Path,
    file_path: &std::path::Path,
) -> Result<PushResult> {
    use crate::actions::runtime;
    use crate::manifest_io;

    let temper_dir = vault_root.join(".temper");
    let device_id = runtime::require_device_id()?;
    let mut manifest = manifest_io::load_manifest(&temper_dir, &device_id)?;

    let result = push_one_resource(
        client,
        vault_root,
        PushTarget::Path(file_path),
        Some(&mut manifest),
    )
    .await?;

    manifest_io::save_manifest(&temper_dir, &manifest)?;
    Ok(result)
}

async fn pull_resource(
    client: &temper_client::TemperClient,
    manifest: &mut Manifest,
    vault_root: &Path,
    item: &SyncPullItem,
) -> Result<()> {
    match item.kind {
        SyncItemKind::Body => {
            // Delegate body pulls to the unified primitive. The primitive
            // writes the file, populates body_hash / remote_body_hash /
            // state / synced_at on the tracked entry. The sync engine
            // always has a manifest, so we pass Some(manifest).
            //
            // Semantic note vs. the old pull_resource_body:
            // - If the id IS in the manifest, ManifestTracked branch fires
            //   and writes to the manifest-resolved path (unchanged behavior).
            // - If the id is NOT in the manifest (rare; sync diff says
            //   pull but we have no entry yet), Snapshot branch writes
            //   {id}.md under vault_root. The old code would have
            //   slug-deduped into a doc-type dir; here it lands at the
            //   vault root. That surface is rare enough that the
            //   simplification is acceptable, and the manifest is the
            //   authoritative path source going forward.
            pull_one_resource(
                client,
                vault_root,
                item.resource_id,
                Some(manifest),
                Some(item.content_hash.clone()),
            )
            .await
            .map(|_| ())
        }
        SyncItemKind::MetaOnly => pull_resource_meta_only(client, manifest, vault_root, item).await,
    }
}

/// Pull a single resource from the server.
///
/// With `Some(manifest)` and a tracked entry, writes to the manifest-resolved
/// vault path (under `vault_root`) and updates the entry's hashes, state, and
/// synced_at. With `None` or an untracked id, writes a snapshot as `{id}.md`
/// under `vault_root` directly — the caller chooses where that is (the CLI
/// wrapper uses CWD; the sync engine uses the vault root).
///
/// `expected_remote_hash` is the server-declared body hash for this resource
/// (as carried on `SyncPullItem.content_hash`). When provided, it is stored
/// verbatim as `remote_body_hash` on the manifest entry — this preserves the
/// invariant that `remote_body_hash` mirrors the server's canonical hash,
/// even when local vault normalization yields a different byte sequence than
/// what the server stored. When `None` (e.g. from the CLI `pull` wrapper
/// that has no sync-diff context), the locally-computed hash of the written
/// body is used as a best-effort fallback.
pub async fn pull_one_resource(
    client: &temper_client::TemperClient,
    vault_root: &Path,
    resource_id: ResourceId,
    manifest: Option<&mut Manifest>,
    expected_remote_hash: Option<String>,
) -> Result<PullResult> {
    let id = Uuid::from(resource_id);

    let resource = client
        .resources()
        .get(id)
        .await
        .map_err(crate::commands::client_err)?;
    let content_response = client
        .resources()
        .content(id)
        .await
        .map_err(crate::commands::client_err)?;

    if let Some(manifest) = manifest {
        if manifest.entries.contains_key(&resource_id) {
            // Manifest-tracked branch: write to the entry's recorded path.
            let entry = manifest
                .entries
                .get_mut(&resource_id)
                .expect("contains_key returned true");
            let vault_path = vault_root.join(&entry.path);
            if let Some(parent) = vault_path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            let (ctx, dtype) = match Vault::parse_rel(&entry.path) {
                Some(parsed) => (parsed.context.to_string(), parsed.doc_type.to_string()),
                None => ("default".to_string(), "resource".to_string()),
            };
            let managed_value = content_response
                .managed_meta
                .as_ref()
                .map(|m| serde_json::to_value(m).unwrap_or(serde_json::Value::Null));
            let fm = ingest::build_frontmatter_from_resource(
                &resource,
                &ctx,
                &dtype,
                ingest::normalize_body_for_vault(&content_response.markdown),
                managed_value.as_ref(),
                content_response.open_meta.as_ref(),
            )?;
            fm.write_to(&vault_path).map_err(|e| {
                TemperError::Vault(format!("pull write {}: {e}", vault_path.display()))
            })?;

            let content_hash = temper_core::hash::compute_body_hash(fm.body());
            let (managed_hash, open_hash) = fm.hashes();

            entry.body_hash = content_hash.clone();
            entry.remote_body_hash = expected_remote_hash.unwrap_or(content_hash);
            entry.managed_hash = managed_hash.clone();
            entry.open_hash = open_hash.clone();
            entry.remote_managed_hash = managed_hash;
            entry.remote_open_hash = open_hash;
            entry.synced_at = chrono::Utc::now();
            entry.state = ManifestEntryState::Clean;
            entry.mtime_secs = file_mtime_secs(&vault_path).ok();

            return Ok(PullResult {
                resource_id,
                path: vault_path,
                branch: PullBranch::ManifestTracked,
                title: resource.title.clone(),
            });
        }

        // First-sync branch: manifest is loaded but this id isn't tracked yet
        // (cross-device sync, fresh device, or any resource ingested elsewhere
        // since last pull). Reconstruct the canonical layout from server data
        // and insert a manifest entry — never dump `<uuid>.md` at vault_root.
        //
        // `resource.owner_handle` shorthands the requester's own profile to
        // literal "@me" (see OWNER_HANDLE_EXPR in resource_service.rs). Vault
        // layout uses the actual profile slug (matching the canonical kb://
        // URIs that sync_refresh parses), so resolve "@me" → @{profile.slug}.
        // Team handles ("+team-slug") are already canonical.
        let context = resource.context_name.as_str();
        let doc_type = resource.doc_type_name.as_str();
        let slug_owned;
        let slug = match resource.slug.as_deref() {
            Some(s) if !s.is_empty() => s,
            _ => {
                slug_owned = ingest::slug_from_title(&resource.title);
                slug_owned.as_str()
            }
        };
        let owner_owned;
        let owner = if resource.owner_handle == "@me" {
            let profile = client
                .profile()
                .get()
                .await
                .map_err(crate::commands::client_err)?;
            owner_owned = format!("@{}", profile.slug);
            owner_owned.as_str()
        } else {
            resource.owner_handle.as_str()
        };
        if owner.is_empty() || context.is_empty() || doc_type.is_empty() || slug.is_empty() {
            return Err(TemperError::Vault(format!(
                "pull untracked id {id}: server response missing routing info \
                 (owner={owner:?}, context={context:?}, doc_type={doc_type:?}, slug={slug:?})"
            )));
        }

        let vault = Vault::new(vault_root);
        let rel_path = vault.rel_path(owner, context, doc_type, slug);
        let vault_path = vault.doc_file(owner, context, doc_type, slug);
        if let Some(parent) = vault_path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let managed_value = content_response
            .managed_meta
            .as_ref()
            .map(|m| serde_json::to_value(m).unwrap_or(serde_json::Value::Null));
        let fm = ingest::build_frontmatter_from_resource(
            &resource,
            context,
            doc_type,
            ingest::normalize_body_for_vault(&content_response.markdown),
            managed_value.as_ref(),
            content_response.open_meta.as_ref(),
        )?;
        fm.write_to(&vault_path)
            .map_err(|e| TemperError::Vault(format!("pull write {}: {e}", vault_path.display())))?;

        let content_hash = temper_core::hash::compute_body_hash(fm.body());
        let (managed_hash, open_hash) = fm.hashes();
        let remote_body_hash = expected_remote_hash.unwrap_or_else(|| content_hash.clone());
        let mtime_secs = file_mtime_secs(&vault_path).ok();

        manifest.entries.insert(
            resource_id,
            ManifestEntry {
                path: rel_path,
                body_hash: content_hash,
                remote_body_hash,
                managed_hash: managed_hash.clone(),
                open_hash: open_hash.clone(),
                remote_managed_hash: managed_hash,
                remote_open_hash: open_hash,
                synced_at: chrono::Utc::now(),
                state: ManifestEntryState::Clean,
                mtime_secs,
                last_audit_id: None,
                provisional: false,
            },
        );

        return Ok(PullResult {
            resource_id,
            path: vault_path,
            branch: PullBranch::NewlyTracked,
            title: resource.title.clone(),
        });
    }

    // Snapshot branch: no manifest at all. CLI `pull` wrapper passes CWD here
    // when no manifest can be loaded — file lands as `{id}.md` under the
    // caller-provided root. Sync run never reaches this branch.
    let filename = format!("{id}.md");
    let snapshot_path = vault_root.join(&filename);
    std::fs::write(&snapshot_path, &content_response.markdown)?;
    Ok(PullResult {
        resource_id,
        path: snapshot_path,
        branch: PullBranch::Snapshot,
        title: resource.title.clone(),
    })
}

/// Relocation guard for meta-only pulls.
///
/// A meta-only pull must not change the file's vault location — doing so
/// would require moving the file on disk and updating the manifest path.
/// If the incoming managed_meta carries a `temper-context` that disagrees
/// with the current on-disk context, reject with a user-facing error so
/// the caller can run a full body pull (which takes the slug-dedup path)
/// or use `temper move` explicitly.
fn check_relocation_guard(
    current_ctx: &str,
    new_managed_meta: Option<&temper_core::types::managed_meta::ManagedMeta>,
) -> Result<()> {
    let Some(meta) = new_managed_meta else {
        return Ok(());
    };
    let Some(new_ctx) = meta.context.as_deref() else {
        return Ok(());
    };
    if new_ctx == current_ctx {
        return Ok(());
    }
    Err(TemperError::Config(format!(
        "meta-only pull would change temper-context from {current_ctx} to {new_ctx}; \
         file relocation via meta-only pull is not supported in v1. \
         Run a full body pull or use temper-move instead."
    )))
}

/// Convert an optional typed `ManagedMeta` into an optional JSON
/// `Value` for the generic frontmatter-emitter callers in this
/// module (`build_frontmatter_from_resource`, `apply_pull_meta_only`).
/// Those functions take `Option<&Value>` because they also need to
/// emit arbitrary per-doc-type fields from the flatten bucket and
/// from open_meta via the same YAML path.
///
/// This is a pure boundary shim — it does not affect hash stability
/// because the hash travels alongside the meta as its own field.
fn managed_meta_to_value(
    meta: Option<&temper_core::types::managed_meta::ManagedMeta>,
) -> Option<serde_json::Value> {
    meta.map(|m| serde_json::to_value(m).unwrap_or(serde_json::Value::Null))
}

/// Rebuild a file's content with server-sourced frontmatter, preserving the
/// local body.
///
/// `Frontmatter::serialize()` produces `---\n<yaml>---\n{body}`, so the
/// blank separator line between the closing fence and the first content
/// line must live at the start of the body. `strip_frontmatter`, however,
/// returns everything after the closing `---\n`, so a `local_body` derived
/// from a well-formed file starts with a leading `\n` (the blank separator).
/// Passing that straight to `normalize_body_for_vault` would leave it alone
/// (it already starts with `\n`); but on subsequent pulls the separator
/// would accumulate (one extra blank line per pull cycle) unless we make
/// the stripping idempotent. Strip a single leading `\n` from `local_body`,
/// then let `normalize_body_for_vault` re-add exactly one `\n` so the
/// operation is a fixed point.
fn rebuild_file_with_new_meta(
    local_body: &str,
    resource: &temper_core::types::ResourceRow,
    ctx: &str,
    doc_type: &str,
    managed_meta: Option<&serde_json::Value>,
    open_meta: Option<&serde_json::Value>,
) -> Result<String> {
    let body_after_separator = local_body.strip_prefix('\n').unwrap_or(local_body);
    let fm = ingest::build_frontmatter_from_resource(
        resource,
        ctx,
        doc_type,
        ingest::normalize_body_for_vault(body_after_separator),
        managed_meta,
        open_meta,
    )?;
    fm.serialize().map_err(|e| {
        crate::error::TemperError::Vault(format!("rebuild_file_with_new_meta serialize: {e}"))
    })
}

/// Parameters for the pure (non-async) half of `pull_resource_meta_only`.
///
/// The async wrapper fetches `resource` and meta blobs via the HTTP client;
/// this struct groups everything the disk-write + hash-recompute + manifest
/// update needs. Factored out so we can unit-test the disk/hash/manifest
/// logic without a live server.
struct ApplyPullMetaOnly<'a> {
    file_path: &'a Path,
    local_body: &'a str,
    resource: &'a temper_core::types::ResourceRow,
    ctx: &'a str,
    doc_type: &'a str,
    managed_meta: Option<&'a serde_json::Value>,
    open_meta: Option<&'a serde_json::Value>,
}

/// Write the rebuilt file, normalize it to enforce doc-type invariants,
/// and update the manifest entry in place with post-normalize hashes.
///
/// body_hash / remote_body_hash are intentionally NOT touched: the body
/// agreed before the pull (precondition for a MetaOnly diff), and
/// normalize_file only rewrites frontmatter, so the body is byte-identical
/// after this call.
fn apply_pull_meta_only(params: ApplyPullMetaOnly<'_>, entry: &mut ManifestEntry) -> Result<()> {
    let ApplyPullMetaOnly {
        file_path,
        local_body,
        resource,
        ctx,
        doc_type,
        managed_meta,
        open_meta,
    } = params;

    let rebuilt =
        rebuild_file_with_new_meta(local_body, resource, ctx, doc_type, managed_meta, open_meta)?;
    std::fs::write(file_path, &rebuilt)?;

    let outcome = temper_core::normalize::normalize_file(file_path, doc_type)?;
    if !outcome.issues.is_empty() {
        let summary = outcome
            .issues
            .iter()
            .map(|i| format!("{}: {}", i.path, i.message))
            .collect::<Vec<_>>()
            .join("; ");
        return Err(TemperError::Config(format!(
            "meta-only pull: normalize reported issues for {}: {summary}",
            file_path.display()
        )));
    }

    // normalize_file just rewrote the file through the Frontmatter
    // pipeline, so it must parse cleanly here — any failure would be a
    // bug in normalize_file, not a user data issue. Propagate with path
    // context.
    let (managed_hash, open_hash) = Frontmatter::parse_file(file_path)
        .map_err(|e| {
            TemperError::Vault(format!(
                "apply_pull_meta_only post-normalize hash compute {}: {e}",
                file_path.display()
            ))
        })?
        .hashes();

    entry.managed_hash = managed_hash.clone();
    entry.open_hash = open_hash.clone();
    entry.remote_managed_hash = managed_hash;
    entry.remote_open_hash = open_hash;
    entry.state = ManifestEntryState::Clean;
    entry.synced_at = chrono::Utc::now();
    entry.mtime_secs = file_mtime_secs(file_path).ok();

    Ok(())
}

async fn pull_resource_meta_only(
    client: &temper_client::TemperClient,
    manifest: &mut Manifest,
    vault_root: &Path,
    item: &SyncPullItem,
) -> Result<()> {
    // A MetaOnly diff presupposes the client already knows this resource
    // — if we have no manifest entry, the server's body-hash agreement
    // claim cannot hold. Fall-through to the body path would risk slug
    // dedup collisions. Surface the inconsistency instead.
    let existing = manifest.entries.get(&item.resource_id).ok_or_else(|| {
        TemperError::NotFound(format!(
            "meta-only pull requires existing manifest entry; got none for {}",
            item.resource_id
        ))
    })?;

    let file_path = vault_root.join(&existing.path);
    if !file_path.exists() {
        return Err(TemperError::NotFound(format!(
            "meta-only pull: local file missing at {}",
            file_path.display()
        )));
    }

    // Use the manifest path as the source of truth for (ctx, doc_type) —
    // that's where the file actually lives on disk, which is what the
    // relocation guard must compare against.
    let parsed = Vault::parse_rel(&existing.path).ok_or_else(|| {
        TemperError::Config(format!(
            "meta-only pull: manifest path does not parse: {}",
            existing.path
        ))
    })?;
    let ctx = parsed.context.to_string();
    let doc_type = parsed.doc_type.to_string();

    let resource = client
        .resources()
        .get(Uuid::from(item.resource_id))
        .await
        .map_err(crate::commands::client_err)?;

    let meta_response = client
        .resources()
        .get_meta(Uuid::from(item.resource_id))
        .await
        .map_err(crate::commands::client_err)?;

    check_relocation_guard(&ctx, meta_response.managed_meta.as_ref())?;

    let existing_content = std::fs::read_to_string(&file_path)?;
    let local_body = strip_frontmatter(&existing_content).to_string();

    let entry = manifest.entries.get_mut(&item.resource_id).ok_or_else(|| {
        TemperError::NotFound(format!(
            "meta-only pull: manifest entry vanished mid-pull: {}",
            item.resource_id
        ))
    })?;

    // Serialize the typed ManagedMeta back to JSON Value for the
    // generic frontmatter emitter below. The `extra` flatten bucket
    // on ManagedMeta makes this round-trip lossless.
    let managed_value = managed_meta_to_value(meta_response.managed_meta.as_ref());

    apply_pull_meta_only(
        ApplyPullMetaOnly {
            file_path: &file_path,
            local_body: &local_body,
            resource: &resource,
            ctx: &ctx,
            doc_type: &doc_type,
            managed_meta: managed_value.as_ref(),
            open_meta: meta_response.open_meta.as_ref(),
        },
        entry,
    )?;

    Ok(())
}

fn remove_resource(
    manifest: &mut Manifest,
    vault_root: &Path,
    item: &SyncRemovedItem,
) -> Result<()> {
    if let Some(entry) = manifest.entries.get(&item.resource_id) {
        let file_path = vault_root.join(&entry.path);
        if file_path.exists() {
            std::fs::remove_file(&file_path)?;
        }
    }
    manifest.entries.remove(&item.resource_id);
    Ok(())
}

// ---------------------------------------------------------------------------
// Merge + Push
// ---------------------------------------------------------------------------

/// Merge a conflicting resource: fetch remote, run merge pipeline, write back, push.
async fn merge_and_push_resource(
    client: &temper_client::TemperClient,
    manifest: &mut Manifest,
    vault_root: &Path,
    item: &SyncConflictItem,
    progress: &dyn SyncProgress,
) -> Result<MergeResult> {
    let entry = manifest.entries.get(&item.resource_id).ok_or_else(|| {
        TemperError::NotFound(format!("manifest entry not found: {}", item.resource_id))
    })?;

    let file_path = vault_root.join(&entry.path);
    if !file_path.exists() {
        return Err(vault_file_missing_err(&entry.path));
    }

    // 1. Read local file, strip frontmatter
    let local_content = std::fs::read_to_string(&file_path)?;
    let frontmatter_block = extract_frontmatter_block(&local_content);
    let local_body = strip_frontmatter(&local_content);

    // 2. Fetch remote content
    let content_response = client
        .resources()
        .content(Uuid::from(item.resource_id))
        .await
        .map_err(crate::commands::client_err)?;
    let remote_body = &content_response.markdown;

    // 3. Run merge pipeline
    let merge_result = temper_ingest::merge::attempt_merge(local_body, remote_body);

    // 4. Report merge result to progress
    progress.merge_result(&entry.path, &merge_result);

    // 5. Get merged content and write back (preserve frontmatter block)
    let merged_body = match &merge_result {
        MergeResult::AutoMerged { content, .. } => content.as_str(),
        MergeResult::ConflictAnnotated { content, .. } => content.as_str(),
    };

    let new_file_content = format!("{frontmatter_block}{merged_body}");
    std::fs::write(&file_path, &new_file_content)?;

    // 6. Build ingest payload with strip_frontmatter on merged file
    let (context, doc_type) = match Vault::parse_rel(&entry.path) {
        Some(parsed) => (parsed.context.to_string(), parsed.doc_type.to_string()),
        None => ("default".to_string(), "resource".to_string()),
    };
    let title = ingest::title_from_path(&file_path);

    let payload =
        ingest::build_ingest_payload(merged_body, &title, &context, &doc_type, None, None, None)?;

    // 7. Push via update
    let _resource = client
        .ingest()
        .update(Uuid::from(item.resource_id), &payload)
        .await
        .map_err(crate::commands::client_err)?;

    // 8. Compute frontmatter hashes from the merged file. The merge
    // pipeline just produced new_file_content and we wrote it to disk; if
    // the re-parse fails, the merge generated invalid frontmatter — that's
    // a bug in the merge pipeline, not a user data issue. Propagate with
    // path context.
    let (pushed_managed_hash, pushed_open_hash) = Frontmatter::try_from(new_file_content.as_str())
        .map_err(|e| {
            TemperError::Vault(format!(
                "merge_and_push_resource post-merge hash compute {}: {e}",
                file_path.display()
            ))
        })?
        .hashes();

    // 9. Update manifest entry
    if let Some(e) = manifest.entries.get_mut(&item.resource_id) {
        e.body_hash = temper_core::hash::compute_body_hash(merged_body);
        // After push, server hashes match what we sent
        e.remote_body_hash = payload.content_hash.clone().unwrap_or_default();
        e.remote_managed_hash = pushed_managed_hash;
        e.remote_open_hash = pushed_open_hash;
        e.state = ManifestEntryState::Clean;
        e.synced_at = chrono::Utc::now();
        e.mtime_secs = file_mtime_secs(&file_path).ok();
    }

    // 9. Return the MergeResult
    Ok(merge_result)
}

// ---------------------------------------------------------------------------
// Manifest refresh / reset
// ---------------------------------------------------------------------------

/// Result of a `sync refresh` operation.
#[derive(Debug)]
pub struct RefreshResult {
    pub matched: usize,
    pub added: usize,
    pub orphaned: usize,
    pub pending_preserved: usize,
}

/// Result of a `sync reset` operation.
#[derive(Debug)]
pub struct ResetResult {
    pub matched_by_id: usize,
    pub matched_by_hash: usize,
    pub unmatched_local: usize,
    pub unmatched_remote: usize,
}

/// Back up manifest.json before a destructive reset.
pub fn backup_manifest(temper_dir: &Path) -> Result<()> {
    let manifest_path = temper_dir.join("manifest.json");
    if manifest_path.exists() {
        let backup_name = format!(
            "manifest.backup.{}.json",
            chrono::Utc::now().format("%Y%m%dT%H%M%S")
        );
        let backup_path = temper_dir.join(backup_name);
        std::fs::copy(&manifest_path, &backup_path)?;
    }
    Ok(())
}

/// Extract the owner sigil (`@slug` or `+slug`) from a server manifest item's
/// canonical `kb://` URI.
///
/// The server's `fetch_manifest` emits owner-scoped URIs via the
/// `kb_resource_uri()` SQL function, so `Vault::parse_uri` reliably yields the
/// authoritative owner — including for team contexts (`+slug`) where silently
/// defaulting to `@me` would mis-route the resource into the personal vault
/// directory and break ownership invariants downstream.
///
/// Returns `None` if the URI is malformed, rather than guessing. Callers are
/// expected to skip the offending server item with a `tracing::warn!`, which
/// matches the existing pattern for malformed local manifest paths in
/// `build_status_request` (line ~175).
fn owner_for_server_item(item: &temper_core::types::SyncManifestItem) -> Option<String> {
    Vault::parse_uri(&item.uri).map(|parsed| parsed.owner.to_string())
}

/// Refresh: fetch server manifest and interleave into local manifest.
///
/// - De-duplicate by resource UUID (server wins for matching IDs)
/// - De-duplicate by content hash within same context/doc_type
/// - Preserve local-only Pending entries that haven't been pushed yet
/// - Update remote hashes (body, managed, open) for all matched entries
pub async fn sync_refresh(
    client: &temper_client::TemperClient,
    manifest: &mut Manifest,
    vault_root: &Path,
) -> Result<RefreshResult> {
    // Rehash local manifest first so content_hash values are current
    rehash_manifest(manifest, vault_root)?;

    let server = client
        .sync()
        .manifest()
        .await
        .map_err(crate::commands::client_err)?;

    let mut matched = 0;
    let mut added = 0;

    // Build a content-hash index for de-duplication:
    // (context, doc_type, content_hash) -> manifest entry UUID
    let mut hash_index: std::collections::HashMap<(String, String, String), ResourceId> =
        std::collections::HashMap::new();
    for (id, entry) in &manifest.entries {
        if !entry.body_hash.is_empty() {
            let (ctx, doc_type) = match Vault::parse_rel(&entry.path) {
                Some(parsed) => (parsed.context.to_string(), parsed.doc_type.to_string()),
                None => ("default".to_string(), "resource".to_string()),
            };
            hash_index.insert((ctx, doc_type, entry.body_hash.clone()), *id);
        }
    }

    // Track which server items were matched
    let mut matched_server_ids: std::collections::HashSet<ResourceId> =
        std::collections::HashSet::new();

    for item in &server.items {
        if manifest.entries.contains_key(&item.resource_id) {
            // UUID match — update remote hashes
            if let Some(entry) = manifest.entries.get_mut(&item.resource_id) {
                entry.remote_body_hash = item.content_hash.clone();
                entry.remote_managed_hash = item.managed_hash.clone();
                entry.remote_open_hash = item.open_hash.clone();
                entry.last_audit_id = item.last_audit_id;
                if entry.body_hash == item.content_hash {
                    entry.state = ManifestEntryState::Clean;
                }
            }
            matched += 1;
            matched_server_ids.insert(item.resource_id);
        } else {
            // Check content hash dedup
            let key = (
                item.context.clone(),
                item.doc_type.clone(),
                item.content_hash.clone(),
            );
            if let Some(&existing_id) = hash_index.get(&key) {
                // Content match — migrate the manifest entry to the server's resource_id
                if let Some(entry) = manifest.entries.remove(&existing_id) {
                    let mut updated = entry;
                    updated.remote_body_hash = item.content_hash.clone();
                    updated.remote_managed_hash = item.managed_hash.clone();
                    updated.remote_open_hash = item.open_hash.clone();
                    updated.last_audit_id = item.last_audit_id;
                    updated.state = ManifestEntryState::Clean;
                    manifest.entries.insert(item.resource_id, updated);
                }
                matched += 1;
                matched_server_ids.insert(item.resource_id);
            } else {
                // Genuinely new from server — add as Pending (to pull on next sync).
                // Path must be the owner-scoped 4-segment form that Vault::parse_rel
                // accepts. The owner segment is derived from the server-supplied
                // canonical `kb://@<owner>/<ctx>/<type>/<ident>` URI, which
                // kb_resource_uri() on the server now guarantees. A None return
                // means the server sent a URI we can't parse — skip the entry
                // rather than guess the owner and mis-route a team resource.
                let Some(owner) = owner_for_server_item(item) else {
                    tracing::warn!(
                        "sync_refresh: skipping server item with unparseable URI {:?} \
                         (resource_id: {})",
                        item.uri,
                        item.resource_id
                    );
                    continue;
                };
                manifest.entries.insert(
                    item.resource_id,
                    temper_core::types::ManifestEntry {
                        path: format!(
                            "{}/{}/{}/{}.md",
                            owner, item.context, item.doc_type, item.slug
                        ),
                        body_hash: String::new(),
                        remote_body_hash: item.content_hash.clone(),
                        managed_hash: String::new(),
                        open_hash: String::new(),
                        remote_managed_hash: item.managed_hash.clone(),
                        remote_open_hash: item.open_hash.clone(),
                        synced_at: chrono::Utc::now(),
                        state: ManifestEntryState::Pending,
                        mtime_secs: None,
                        last_audit_id: item.last_audit_id,
                        provisional: false,
                    },
                );
                added += 1;
            }
        }
    }

    // Count orphaned entries (local entries with no server match, excluding Pending)
    let orphaned = manifest
        .entries
        .iter()
        .filter(|(id, entry)| {
            !matched_server_ids.contains(id) && entry.state != ManifestEntryState::Pending
        })
        .count();

    // Count preserved Pending entries (were already Pending before refresh)
    let pending_preserved = manifest
        .entries
        .iter()
        .filter(|(id, entry)| {
            !matched_server_ids.contains(id) && entry.state == ManifestEntryState::Pending
        })
        .count();

    Ok(RefreshResult {
        matched,
        added,
        orphaned,
        pending_preserved,
    })
}

/// Reset: rebuild manifest from scratch using server manifest + vault scan.
///
/// 1. Pull full resource list from server
/// 2. Keep only device_id from current manifest
/// 3. Walk vault files, match to server by temper-id frontmatter or content hash
/// 4. Rebuild all local content hashes
/// 5. Mark unmatched local files as Pending (new)
/// 6. Mark unmatched server resources for pull (Pending with empty content_hash)
pub async fn sync_reset(
    client: &temper_client::TemperClient,
    old_manifest: &Manifest,
    vault_root: &Path,
) -> Result<(Manifest, ResetResult)> {
    let server = client
        .sync()
        .manifest()
        .await
        .map_err(crate::commands::client_err)?;

    let mut new_manifest = Manifest::new(old_manifest.device_id.clone());
    let mut matched_by_id = 0;
    let mut matched_by_hash = 0;

    // Build server index by resource_id
    let server_by_id: std::collections::HashMap<ResourceId, &temper_core::types::SyncManifestItem> =
        server.items.iter().map(|i| (i.resource_id, i)).collect();

    // Build server index by content_hash for fallback matching
    // Key: (context, doc_type, content_hash) -> server item
    let mut server_by_hash: std::collections::HashMap<
        (String, String, String),
        &temper_core::types::SyncManifestItem,
    > = std::collections::HashMap::new();
    for item in &server.items {
        if !item.content_hash.is_empty() {
            server_by_hash.insert(
                (
                    item.context.clone(),
                    item.doc_type.clone(),
                    item.content_hash.clone(),
                ),
                item,
            );
        }
    }

    // Track which server resources have been matched
    let mut matched_server_ids: std::collections::HashSet<ResourceId> =
        std::collections::HashSet::new();

    // Walk vault files
    for entry in ignore::WalkBuilder::new(vault_root)
        .hidden(true)
        .build()
        .filter_map(|e| e.ok())
    {
        let path = entry.path();

        if path.extension().and_then(|e| e.to_str()) != Some("md") {
            continue;
        }

        if path.starts_with(vault_root.join(".temper")) {
            continue;
        }

        let rel_path = path
            .strip_prefix(vault_root)
            .unwrap_or(path)
            .to_string_lossy()
            .to_string();

        let content = std::fs::read_to_string(path)?;
        let body = strip_frontmatter(&content);
        let content_hash = temper_core::hash::compute_body_hash(body);
        let mtime = file_mtime_secs(path).ok();

        // Compute local frontmatter tier hashes. Files with malformed or
        // legacy frontmatter are skipped from the reset matching pass — we
        // cannot produce meaningful hashes for them, and matching with
        // empty-JSON hashes would guarantee a spurious mismatch against
        // every server record. Warn so the user can fix the file before
        // the next reset.
        let strict_fm = match Frontmatter::try_from(content.as_str()) {
            Ok(fm) => fm,
            Err(e) => {
                tracing::warn!(
                    path = %path.display(),
                    error = %e,
                    "skipping reset hash compute: frontmatter parse failed — run `temper doctor fix` to migrate"
                );
                continue;
            }
        };
        let (local_managed_hash, local_open_hash) = strict_fm.hashes();

        let fm = ingest::parse_source_frontmatter(&content);

        // Try matching by temper-id frontmatter first
        let temper_id = fm
            .as_ref()
            .and_then(|f| f.legacy_id.as_deref())
            .and_then(|id| Uuid::parse_str(id).ok());

        let provisional_id = fm
            .as_ref()
            .and_then(|f| f.provisional_id.as_deref())
            .and_then(|id| Uuid::parse_str(id).ok());

        if let Some(tid) = temper_id {
            let tid_resource = ResourceId::from(tid);
            if let Some(server_item) = server_by_id.get(&tid_resource) {
                // Match by temper-id
                let state = if content_hash == server_item.content_hash
                    && local_managed_hash == server_item.managed_hash
                    && local_open_hash == server_item.open_hash
                {
                    ManifestEntryState::Clean
                } else {
                    ManifestEntryState::LocalModified
                };
                new_manifest.entries.insert(
                    tid_resource,
                    temper_core::types::ManifestEntry {
                        path: rel_path,
                        body_hash: content_hash,
                        remote_body_hash: server_item.content_hash.clone(),
                        managed_hash: local_managed_hash,
                        open_hash: local_open_hash,
                        remote_managed_hash: server_item.managed_hash.clone(),
                        remote_open_hash: server_item.open_hash.clone(),
                        synced_at: chrono::Utc::now(),
                        state,
                        mtime_secs: mtime,
                        last_audit_id: server_item.last_audit_id,
                        provisional: false,
                    },
                );
                matched_by_id += 1;
                matched_server_ids.insert(tid_resource);
                continue;
            }
        }

        // Provisional files — skip server matching entirely, mark Pending
        if temper_id.is_none() && provisional_id.is_some() {
            let resource_id = ResourceId::from(provisional_id.unwrap());
            new_manifest.entries.insert(
                resource_id,
                temper_core::types::ManifestEntry {
                    path: rel_path,
                    body_hash: content_hash,
                    remote_body_hash: String::new(),
                    managed_hash: local_managed_hash,
                    open_hash: local_open_hash,
                    remote_managed_hash: String::new(),
                    remote_open_hash: String::new(),
                    synced_at: chrono::Utc::now(),
                    state: ManifestEntryState::Pending,
                    mtime_secs: mtime,
                    last_audit_id: None,
                    provisional: true,
                },
            );
            continue;
        }

        // Try matching by content hash
        let fm_context = fm.as_ref().and_then(|f| f.context.as_deref());
        let fm_doc_type = fm.as_ref().and_then(|f| f.doc_type.as_deref());

        let (ctx, doc_type) =
            match ingest::infer_context_and_doctype(vault_root, path, fm_context, fm_doc_type) {
                Ok(pair) => pair,
                Err(_) => continue,
            };

        let hash_key = (ctx, doc_type, content_hash.clone());
        if let Some(server_item) = server_by_hash.get(&hash_key) {
            if !matched_server_ids.contains(&server_item.resource_id) {
                let state = if local_managed_hash == server_item.managed_hash
                    && local_open_hash == server_item.open_hash
                {
                    ManifestEntryState::Clean
                } else {
                    ManifestEntryState::LocalModified
                };
                new_manifest.entries.insert(
                    server_item.resource_id,
                    temper_core::types::ManifestEntry {
                        path: rel_path,
                        body_hash: content_hash,
                        remote_body_hash: server_item.content_hash.clone(),
                        managed_hash: local_managed_hash,
                        open_hash: local_open_hash,
                        remote_managed_hash: server_item.managed_hash.clone(),
                        remote_open_hash: server_item.open_hash.clone(),
                        synced_at: chrono::Utc::now(),
                        state,
                        mtime_secs: mtime,
                        last_audit_id: server_item.last_audit_id,
                        provisional: false,
                    },
                );
                matched_by_hash += 1;
                matched_server_ids.insert(server_item.resource_id);
                continue;
            }
        }

        // Unmatched local file — mark as Pending (new, will push on next sync).
        // Use the file's temper-id if present so push_resource can remap it
        // after the server assigns an authoritative ID.
        let (resource_id, is_provisional) = if let Some(tid) = temper_id {
            (ResourceId::from(tid), false)
        } else {
            (ResourceId::new(), true)
        };
        new_manifest.entries.insert(
            resource_id,
            temper_core::types::ManifestEntry {
                path: rel_path,
                body_hash: content_hash,
                remote_body_hash: String::new(),
                managed_hash: local_managed_hash,
                open_hash: local_open_hash,
                remote_managed_hash: String::new(),
                remote_open_hash: String::new(),
                synced_at: chrono::Utc::now(),
                state: ManifestEntryState::Pending,
                mtime_secs: mtime,
                last_audit_id: None,
                provisional: is_provisional,
            },
        );
    }

    // Unmatched server resources — add as Pending entries (will pull on next sync)
    let unmatched_remote = server
        .items
        .iter()
        .filter(|item| !matched_server_ids.contains(&item.resource_id))
        .count();

    for item in &server.items {
        if !matched_server_ids.contains(&item.resource_id) {
            let Some(owner) = owner_for_server_item(item) else {
                tracing::warn!(
                    "sync_reset: skipping server item with unparseable URI {:?} \
                     (resource_id: {})",
                    item.uri,
                    item.resource_id
                );
                continue;
            };
            new_manifest.entries.insert(
                item.resource_id,
                temper_core::types::ManifestEntry {
                    path: format!(
                        "{}/{}/{}/{}.md",
                        owner, item.context, item.doc_type, item.slug
                    ),
                    body_hash: String::new(),
                    remote_body_hash: item.content_hash.clone(),
                    managed_hash: String::new(),
                    open_hash: String::new(),
                    remote_managed_hash: item.managed_hash.clone(),
                    remote_open_hash: item.open_hash.clone(),
                    synced_at: chrono::Utc::now(),
                    state: ManifestEntryState::Pending,
                    mtime_secs: None,
                    last_audit_id: item.last_audit_id,
                    provisional: false,
                },
            );
        }
    }

    let unmatched_local = new_manifest
        .entries
        .values()
        .filter(|e| e.state == ManifestEntryState::Pending && e.remote_body_hash.is_empty())
        .count();

    Ok((
        new_manifest,
        ResetResult {
            matched_by_id,
            matched_by_hash,
            unmatched_local,
            unmatched_remote,
        },
    ))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use std::fs;
    use temper_core::types::ManifestEntry;
    use tempfile::TempDir;

    fn sample_manifest() -> Manifest {
        let mut m = Manifest::new("device-test".to_string());
        let id = ResourceId::from(Uuid::parse_str("12345678-1234-1234-1234-123456789abc").unwrap());
        m.entries.insert(
            id,
            ManifestEntry {
                path: "@me/temper/task/12345678-1234-1234-1234-123456789abc.md".to_string(),
                body_hash: "oldhash".to_string(),
                remote_body_hash: "oldhash".to_string(),
                managed_hash: String::new(),
                open_hash: String::new(),
                remote_managed_hash: String::new(),
                remote_open_hash: String::new(),
                synced_at: Utc::now(),
                state: ManifestEntryState::Clean,
                mtime_secs: None,
                last_audit_id: None,
                provisional: false,
            },
        );
        m
    }

    #[test]
    fn rehash_detects_local_modification() {
        let dir = TempDir::new().unwrap();
        let vault = dir.path();
        let mut manifest = sample_manifest();

        let file_dir = vault.join("@me/temper/task");
        fs::create_dir_all(&file_dir).unwrap();
        fs::write(
            file_dir.join("12345678-1234-1234-1234-123456789abc.md"),
            "---\ntemper-type: task\ntemper-context: temper\ntemper-title: t\ntemper-slug: t\n---\n\nnew content\n",
        )
        .unwrap();

        let changed = rehash_manifest(&mut manifest, vault).unwrap();
        assert_eq!(changed, 1);

        let id = ResourceId::from(Uuid::parse_str("12345678-1234-1234-1234-123456789abc").unwrap());
        let entry = manifest.entries.get(&id).unwrap();
        assert_eq!(entry.state, ManifestEntryState::LocalModified);
        assert_ne!(entry.body_hash, "oldhash");
    }

    #[test]
    fn rehash_marks_deleted_files() {
        let dir = TempDir::new().unwrap();
        let mut manifest = sample_manifest();

        let changed = rehash_manifest(&mut manifest, dir.path()).unwrap();
        assert_eq!(changed, 1);

        let id = ResourceId::from(Uuid::parse_str("12345678-1234-1234-1234-123456789abc").unwrap());
        let entry = manifest.entries.get(&id).unwrap();
        assert_eq!(entry.state, ManifestEntryState::LocalModified);
        assert!(entry.body_hash.is_empty());
    }

    #[test]
    fn rehash_skips_unchanged_files_with_complete_hashes() {
        let dir = TempDir::new().unwrap();
        let vault = dir.path();
        let mut manifest = sample_manifest();

        // File with frontmatter so we can compute all three hashes
        let content =
            "---\ntemper-type: task\ntemper-title: Test\ndate: 2026-01-01\n---\ntest content";
        let body = strip_frontmatter(content);
        let hash = temper_core::hash::compute_body_hash(body);

        let id = ResourceId::from(Uuid::parse_str("12345678-1234-1234-1234-123456789abc").unwrap());
        let entry = manifest.entries.get_mut(&id).unwrap();
        entry.body_hash = hash;
        // Set non-empty managed/open hashes so the skip condition is met
        entry.managed_hash = "sha256:abc".to_string();
        entry.open_hash = "sha256:def".to_string();

        let file_dir = vault.join("@me/temper/task");
        fs::create_dir_all(&file_dir).unwrap();
        fs::write(
            file_dir.join("12345678-1234-1234-1234-123456789abc.md"),
            content,
        )
        .unwrap();

        // First rehash sets mtime
        let changed = rehash_manifest(&mut manifest, vault).unwrap();
        // Hashes differ from "sha256:abc"/"sha256:def" so it recomputes
        assert!(changed > 0);

        // Reset state to Clean for the skip test
        let entry = manifest.entries.get_mut(&id).unwrap();
        entry.state = ManifestEntryState::Clean;

        // Second rehash should skip — mtime matches and hashes are complete
        let changed = rehash_manifest(&mut manifest, vault).unwrap();
        assert_eq!(changed, 0);
    }

    #[test]
    fn rehash_backfills_empty_managed_open_hashes() {
        let dir = TempDir::new().unwrap();
        let vault = dir.path();
        let mut manifest = sample_manifest();

        let content =
            "---\ntemper-type: task\ntemper-title: Test\ndate: 2026-01-01\n---\ntest content";
        let body = strip_frontmatter(content);
        let hash = temper_core::hash::compute_body_hash(body);

        let id = ResourceId::from(Uuid::parse_str("12345678-1234-1234-1234-123456789abc").unwrap());
        let entry = manifest.entries.get_mut(&id).unwrap();
        entry.body_hash = hash;
        // Leave managed_hash and open_hash empty — simulating the old bug

        let file_dir = vault.join("@me/temper/task");
        fs::create_dir_all(&file_dir).unwrap();
        fs::write(
            file_dir.join("12345678-1234-1234-1234-123456789abc.md"),
            content,
        )
        .unwrap();

        // First pass: sets mtime AND backfills hashes
        let changed = rehash_manifest(&mut manifest, vault).unwrap();
        assert_eq!(changed, 1);

        let entry = manifest.entries.get(&id).unwrap();
        assert!(
            !entry.managed_hash.is_empty(),
            "managed_hash should be populated"
        );
        assert!(!entry.open_hash.is_empty(), "open_hash should be populated");
        assert!(entry.managed_hash.starts_with("sha256:"));
        assert!(entry.open_hash.starts_with("sha256:"));
    }

    #[test]
    fn build_status_request_groups_by_context() {
        let manifest = sample_manifest();
        let req = build_status_request(&manifest, &[]);
        assert_eq!(req.contexts.len(), 1);
        assert_eq!(req.contexts[0].name, "temper");
        assert_eq!(req.contexts[0].entries.len(), 1);
        assert!(req.contexts[0].entries[0]
            .uri
            .starts_with("kb://@me/temper/task/"));
    }

    #[test]
    fn build_status_request_filters_contexts() {
        let manifest = sample_manifest();
        let req = build_status_request(&manifest, &["other".to_string()]);
        assert!(req.contexts.is_empty());
    }

    #[test]
    fn parse_kb_uri_extracts_parts() {
        let (ctx, dt) =
            parse_kb_uri("kb://@me/temper/task/12345678-1234-1234-1234-123456789abc").unwrap();
        assert_eq!(ctx, "temper");
        assert_eq!(dt, "task");
    }

    #[test]
    fn parse_kb_uri_rejects_non_kb() {
        assert!(parse_kb_uri("https://example.com").is_err());
    }

    #[test]
    fn parse_kb_uri_rejects_missing_doc_type() {
        assert!(parse_kb_uri("kb://@me/temper").is_err());
    }

    #[test]
    fn extract_resource_id_works() {
        let id =
            extract_resource_id("kb://temper/task/12345678-1234-1234-1234-123456789abc").unwrap();
        assert_eq!(
            id,
            ResourceId::from(Uuid::parse_str("12345678-1234-1234-1234-123456789abc").unwrap())
        );
    }

    #[test]
    fn extract_resource_id_rejects_invalid() {
        assert!(extract_resource_id("kb://temper/task/not-a-uuid").is_err());
    }

    #[test]
    fn strip_frontmatter_removes_yaml() {
        let content = "---\ntemper-title: test\n---\n\n# Hello";
        assert_eq!(strip_frontmatter(content), "\n# Hello");
    }

    #[test]
    fn strip_frontmatter_passes_through_no_frontmatter() {
        let content = "# Hello\nWorld";
        assert_eq!(strip_frontmatter(content), "# Hello\nWorld");
    }

    #[test]
    fn strip_frontmatter_handles_empty_frontmatter() {
        // "---\n---\n" has the closing delimiter immediately after opening,
        // so content[4..] = "---\n\nContent" which doesn't contain "\n---\n".
        // A proper empty frontmatter needs a newline before the closing delimiter.
        let content = "---\n\n---\n\nContent";
        assert_eq!(strip_frontmatter(content), "\nContent");
    }

    #[test]
    fn build_complete_request_sets_fields() {
        let req = build_complete_request("device-1", vec![]);
        assert_eq!(req.device_id, "device-1");
        assert!(req.merged_resources.is_empty());
    }

    #[test]
    fn remove_resource_deletes_file_and_entry() {
        let dir = TempDir::new().unwrap();
        let vault = dir.path();
        let mut manifest = sample_manifest();

        let id = ResourceId::from(Uuid::parse_str("12345678-1234-1234-1234-123456789abc").unwrap());
        let file_dir = vault.join("@me/temper/task");
        fs::create_dir_all(&file_dir).unwrap();
        let file_path = file_dir.join("12345678-1234-1234-1234-123456789abc.md");
        fs::write(&file_path, "content").unwrap();

        let item = SyncRemovedItem {
            uri: "kb://temper/task/12345678-1234-1234-1234-123456789abc".to_string(),
            resource_id: id,
        };
        remove_resource(&mut manifest, vault, &item).unwrap();

        assert!(!file_path.exists());
        assert!(!manifest.entries.contains_key(&id));
    }

    // --- Frontmatter hash fix tests ---

    #[test]
    fn rehash_detects_frontmatter_changes() {
        let dir = TempDir::new().unwrap();
        let vault = dir.path();
        let id = ResourceId::from(Uuid::parse_str("12345678-1234-1234-1234-123456789abc").unwrap());

        // Fixtures carry temper-type so Frontmatter::try_from succeeds.
        let file_v1 = "---\ntemper-type: task\ntemper-title: Old Title\ncreated: 2026-01-01\n---\n\n# My Document\n\nSome content here.\n";
        let file_v2 = "---\ntemper-type: task\ntemper-title: New Title\ncreated: 2026-04-03\n---\n\n# My Document\n\nSome content here.\n";

        // Compute hashes for v1 via the authoritative frontmatter module.
        let body_hash = temper_core::hash::compute_body_hash(strip_frontmatter(file_v1));
        let fm_v1 = Frontmatter::try_from(file_v1).expect("parse v1");
        let (managed_hash_v1, open_hash_v1) = fm_v1.hashes();

        let file_dir = vault.join("@me/temper/task");
        fs::create_dir_all(&file_dir).unwrap();
        let file_path = file_dir.join("12345678-1234-1234-1234-123456789abc.md");
        fs::write(&file_path, file_v1).unwrap();

        let mut manifest = Manifest::new("device-test".to_string());
        manifest.entries.insert(
            id,
            ManifestEntry {
                path: "@me/temper/task/12345678-1234-1234-1234-123456789abc.md".to_string(),
                body_hash: body_hash.clone(),
                remote_body_hash: body_hash.clone(),
                managed_hash: managed_hash_v1.clone(),
                open_hash: open_hash_v1.clone(),
                remote_managed_hash: managed_hash_v1,
                remote_open_hash: open_hash_v1,
                synced_at: Utc::now(),
                state: ManifestEntryState::Clean,
                mtime_secs: None, // Force rehash
                last_audit_id: None,
                provisional: false,
            },
        );

        // Rehash v1 — nothing changed, should detect 0 changes
        let changed = rehash_manifest(&mut manifest, vault).unwrap();
        assert_eq!(changed, 0, "v1 unchanged — should not trigger");

        // Now write v2 (frontmatter changed, body identical)
        fs::write(&file_path, file_v2).unwrap();
        manifest.entries.get_mut(&id).unwrap().mtime_secs = None; // Force rehash

        let changed = rehash_manifest(&mut manifest, vault).unwrap();
        assert_eq!(
            changed, 1,
            "frontmatter changed — should trigger dirty with three-tier hashing"
        );
        assert_eq!(
            manifest.entries[&id].state,
            ManifestEntryState::LocalModified
        );
    }

    #[test]
    fn rehash_detects_body_change_with_frontmatter() {
        let dir = TempDir::new().unwrap();
        let vault = dir.path();
        let id = ResourceId::from(Uuid::parse_str("12345678-1234-1234-1234-123456789abc").unwrap());

        let original = "---\ntemper-type: task\ntemper-context: temper\ntemper-title: Test\ntemper-slug: test\n---\n\n# Original body\n";
        let modified = "---\ntemper-type: task\ntemper-context: temper\ntemper-title: Test\ntemper-slug: test\n---\n\n# Modified body\n";

        let original_body_hash = temper_core::hash::compute_body_hash(strip_frontmatter(original));

        let file_dir = vault.join("@me/temper/task");
        fs::create_dir_all(&file_dir).unwrap();
        let file_path = file_dir.join("12345678-1234-1234-1234-123456789abc.md");
        fs::write(&file_path, original).unwrap();

        let mut manifest = Manifest::new("device-test".to_string());
        manifest.entries.insert(
            id,
            ManifestEntry {
                path: "@me/temper/task/12345678-1234-1234-1234-123456789abc.md".to_string(),
                body_hash: original_body_hash,
                remote_body_hash: "somehash".to_string(),
                managed_hash: String::new(),
                open_hash: String::new(),
                remote_managed_hash: String::new(),
                remote_open_hash: String::new(),
                synced_at: Utc::now(),
                state: ManifestEntryState::Clean,
                mtime_secs: None,
                last_audit_id: None,
                provisional: false,
            },
        );

        // Write modified content — body has changed
        fs::write(&file_path, modified).unwrap();
        manifest.entries.get_mut(&id).unwrap().mtime_secs = None;

        let changed = rehash_manifest(&mut manifest, vault).unwrap();
        assert_eq!(changed, 1, "body changed — should detect modification");
        assert_eq!(
            manifest.entries[&id].state,
            ManifestEntryState::LocalModified
        );
    }

    // --- Mtime optimization tests ---

    #[test]
    fn rehash_skips_file_when_mtime_matches_and_hashes_complete() {
        let dir = TempDir::new().unwrap();
        let vault = dir.path();
        let id = ResourceId::from(Uuid::parse_str("12345678-1234-1234-1234-123456789abc").unwrap());

        let file_dir = vault.join("@me/temper/task");
        fs::create_dir_all(&file_dir).unwrap();
        let file_path = file_dir.join("12345678-1234-1234-1234-123456789abc.md");
        fs::write(&file_path, "body content").unwrap();

        let file_mtime = file_mtime_secs(&file_path).unwrap();

        let mut manifest = Manifest::new("device-test".to_string());
        manifest.entries.insert(
            id,
            ManifestEntry {
                path: "@me/temper/task/12345678-1234-1234-1234-123456789abc.md".to_string(),
                body_hash: "stale-hash-that-would-trigger-if-read".to_string(),
                remote_body_hash: "stale-hash-that-would-trigger-if-read".to_string(),
                // Hashes must be non-empty for skip to apply
                managed_hash: "sha256:abc".to_string(),
                open_hash: "sha256:def".to_string(),
                remote_managed_hash: String::new(),
                remote_open_hash: String::new(),
                synced_at: Utc::now(),
                state: ManifestEntryState::Clean,
                mtime_secs: Some(file_mtime),
                last_audit_id: None,
                provisional: false,
            },
        );

        // Mtime matches AND hashes are complete — rehash should skip
        let changed = rehash_manifest(&mut manifest, vault).unwrap();
        assert_eq!(changed, 0);
        assert_eq!(
            manifest.entries[&id].body_hash,
            "stale-hash-that-would-trigger-if-read"
        );
    }

    #[test]
    fn rehash_backfills_when_mtime_matches_but_hashes_empty() {
        let dir = TempDir::new().unwrap();
        let vault = dir.path();
        let id = ResourceId::from(Uuid::parse_str("12345678-1234-1234-1234-123456789abc").unwrap());

        let file_dir = vault.join("@me/temper/task");
        fs::create_dir_all(&file_dir).unwrap();
        let file_path = file_dir.join("12345678-1234-1234-1234-123456789abc.md");
        fs::write(
            &file_path,
            "---\ntemper-type: task\ntemper-title: Test\n---\nbody content",
        )
        .unwrap();

        let file_mtime = file_mtime_secs(&file_path).unwrap();

        let mut manifest = Manifest::new("device-test".to_string());
        manifest.entries.insert(
            id,
            ManifestEntry {
                path: "@me/temper/task/12345678-1234-1234-1234-123456789abc.md".to_string(),
                body_hash: temper_core::hash::compute_body_hash("body content"),
                remote_body_hash: String::new(),
                // Empty hashes — must backfill even though mtime matches
                managed_hash: String::new(),
                open_hash: String::new(),
                remote_managed_hash: String::new(),
                remote_open_hash: String::new(),
                synced_at: Utc::now(),
                state: ManifestEntryState::Clean,
                mtime_secs: Some(file_mtime),
                last_audit_id: None,
                provisional: false,
            },
        );

        // Mtime matches but hashes are empty — should backfill
        let changed = rehash_manifest(&mut manifest, vault).unwrap();
        assert_eq!(changed, 1);
        assert!(!manifest.entries[&id].managed_hash.is_empty());
        assert!(!manifest.entries[&id].open_hash.is_empty());
    }

    #[test]
    fn rehash_processes_file_when_mtime_is_none() {
        let dir = TempDir::new().unwrap();
        let vault = dir.path();
        let id = ResourceId::from(Uuid::parse_str("12345678-1234-1234-1234-123456789abc").unwrap());

        let content = "---\ntemper-type: task\ntemper-context: temper\ntemper-title: t\ntemper-slug: t\n---\n\nbody content\n";
        let file_dir = vault.join("@me/temper/task");
        fs::create_dir_all(&file_dir).unwrap();
        fs::write(
            file_dir.join("12345678-1234-1234-1234-123456789abc.md"),
            content,
        )
        .unwrap();

        let mut manifest = Manifest::new("device-test".to_string());
        manifest.entries.insert(
            id,
            ManifestEntry {
                path: "@me/temper/task/12345678-1234-1234-1234-123456789abc.md".to_string(),
                body_hash: "oldhash".to_string(),
                remote_body_hash: "oldhash".to_string(),
                managed_hash: String::new(),
                open_hash: String::new(),
                remote_managed_hash: String::new(),
                remote_open_hash: String::new(),
                synced_at: Utc::now(),
                state: ManifestEntryState::Clean,
                mtime_secs: None, // No mtime — must rehash
                last_audit_id: None,
                provisional: false,
            },
        );

        let changed = rehash_manifest(&mut manifest, vault).unwrap();
        assert_eq!(changed, 1);
        assert!(
            manifest.entries[&id].mtime_secs.is_some(),
            "mtime should be recorded"
        );
    }

    // --- scan_vault_for_untracked ---

    #[test]
    fn scan_vault_discovers_untracked_files() {
        let dir = TempDir::new().unwrap();
        let vault = dir.path();
        let file_dir = vault.join("@me/temper/research");
        fs::create_dir_all(&file_dir).unwrap();
        fs::write(
            file_dir.join("new-discovery.md"),
            "# New Discovery\n\nSome content.",
        )
        .unwrap();

        let mut manifest = Manifest::new("device-test".to_string());
        let progress = crate::actions::progress::CollectingProgress::default();
        let found = scan_vault_for_untracked(&mut manifest, vault, &progress).unwrap();
        assert_eq!(found, 1);
        assert_eq!(manifest.entries.len(), 1);
    }

    #[test]
    fn scan_vault_skips_files_already_in_manifest() {
        let dir = TempDir::new().unwrap();
        let vault = dir.path();
        let file_dir = vault.join("@me/temper/research");
        fs::create_dir_all(&file_dir).unwrap();
        fs::write(file_dir.join("existing.md"), "# Existing\n\nContent.").unwrap();

        let mut manifest = Manifest::new("device-test".to_string());
        let id = ResourceId::new();
        manifest.entries.insert(
            id,
            ManifestEntry {
                path: "@me/temper/research/existing.md".to_string(),
                body_hash: "somehash".to_string(),
                remote_body_hash: "somehash".to_string(),
                managed_hash: String::new(),
                open_hash: String::new(),
                remote_managed_hash: String::new(),
                remote_open_hash: String::new(),
                synced_at: Utc::now(),
                state: ManifestEntryState::Clean,
                mtime_secs: None,
                last_audit_id: None,
                provisional: false,
            },
        );

        let progress = crate::actions::progress::CollectingProgress::default();
        let found = scan_vault_for_untracked(&mut manifest, vault, &progress).unwrap();
        assert_eq!(found, 0);
    }

    #[test]
    fn scan_vault_skips_unmappable_files() {
        let dir = TempDir::new().unwrap();
        let vault = dir.path();
        fs::write(vault.join("orphan.md"), "# Orphan").unwrap();

        let mut manifest = Manifest::new("device-test".to_string());
        let progress = crate::actions::progress::CollectingProgress::default();
        let found = scan_vault_for_untracked(&mut manifest, vault, &progress).unwrap();
        assert_eq!(found, 0);
    }

    #[test]
    fn scan_vault_respects_frontmatter_override() {
        let dir = TempDir::new().unwrap();
        let vault = dir.path();
        let file_dir = vault.join("@me/temper/research");
        fs::create_dir_all(&file_dir).unwrap();
        fs::write(
            file_dir.join("overridden.md"),
            "---\ntemper-type: session\ntemper-context: custom\ntemper-title: overridden\ntemper-slug: overridden\n---\n\n# Overridden\n",
        )
        .unwrap();

        let mut manifest = Manifest::new("device-test".to_string());
        let progress = crate::actions::progress::CollectingProgress::default();
        let found = scan_vault_for_untracked(&mut manifest, vault, &progress).unwrap();
        assert_eq!(found, 1);
    }

    #[test]
    fn manifest_backward_compat_missing_mtime() {
        // Old manifests won't have mtime_secs — #[serde(default)] handles it
        let json = r#"{
            "device_id": "test",
            "last_sync": null,
            "entries": {
                "12345678-1234-1234-1234-123456789abc": {
                    "path": "@me/temper/task/test.md",
                    "content_hash": "abc",
                    "remote_hash": "abc",
                    "synced_at": "2026-01-01T00:00:00Z",
                    "state": "clean"
                }
            }
        }"#;
        let manifest: Manifest = serde_json::from_str(json).unwrap();
        let id = ResourceId::from(Uuid::parse_str("12345678-1234-1234-1234-123456789abc").unwrap());
        assert_eq!(manifest.entries[&id].mtime_secs, None);
    }

    // --- extract_frontmatter_block ---

    #[test]
    fn extract_frontmatter_block_returns_block() {
        let content = "---\ntemper-title: Test\ncontext: temper\n---\n\n# Body\n";
        let block = extract_frontmatter_block(content);
        assert_eq!(block, "---\ntemper-title: Test\ncontext: temper\n---\n");
    }

    #[test]
    fn extract_frontmatter_block_returns_empty_for_no_frontmatter() {
        let content = "# No frontmatter\n";
        let block = extract_frontmatter_block(content);
        assert_eq!(block, "");
    }

    #[test]
    fn pull_existing_resource_overwrites_in_place() {
        // Simulate the bug scenario: a file already exists at the slug path
        // AND the manifest knows about it. The fixed pull logic should
        // overwrite in place, NOT create my-document-2.md.
        let dir = TempDir::new().unwrap();
        let vault = dir.path();
        let resource_id = ResourceId::new();

        // Create the existing file on disk
        let file_dir = vault.join("@me/temper/task");
        fs::create_dir_all(&file_dir).unwrap();
        let existing_file = file_dir.join("my-document.md");
        fs::write(&existing_file, "---\ntemper-id: old\n---\n\nOld content").unwrap();

        // Set up manifest with entry pointing to this file
        let mut manifest = Manifest::new("device-test".to_string());
        manifest.entries.insert(
            resource_id,
            ManifestEntry {
                path: "@me/temper/task/my-document.md".to_string(),
                body_hash: temper_core::hash::compute_body_hash("Old content"),
                remote_body_hash: "remote-hash-1".to_string(),
                managed_hash: String::new(),
                open_hash: String::new(),
                remote_managed_hash: String::new(),
                remote_open_hash: String::new(),
                synced_at: Utc::now(),
                state: ManifestEntryState::Clean,
                mtime_secs: None,
                last_audit_id: None,
                provisional: false,
            },
        );

        // Check manifest — resource exists and file is on disk.
        let existing_entry = manifest.entries.get(&resource_id).unwrap();
        let existing_path = vault.join(&existing_entry.path);
        assert!(existing_path.exists());

        // Overwrite in place (this is what the fixed pull_resource does
        // when it finds an existing manifest entry with a valid path).
        let fm = ingest::build_frontmatter(
            resource_id,
            "My Document",
            "temper",
            "task",
            ingest::normalize_body_for_vault("Updated content"),
            None,
            None,
        )
        .unwrap();
        fm.write_to(&existing_path).unwrap();

        // Update manifest entry (matches what pull_resource now does).
        let content_hash = temper_core::hash::compute_body_hash("Updated content");
        manifest.entries.insert(
            resource_id,
            ManifestEntry {
                path: "@me/temper/task/my-document.md".to_string(),
                body_hash: content_hash,
                remote_body_hash: "remote-hash-2".to_string(),
                managed_hash: String::new(),
                open_hash: String::new(),
                remote_managed_hash: String::new(),
                remote_open_hash: String::new(),
                synced_at: Utc::now(),
                state: ManifestEntryState::Clean,
                mtime_secs: None,
                last_audit_id: None,
                provisional: false,
            },
        );

        // No deduplicated file was created
        assert!(!file_dir.join("my-document-2.md").exists());
        // The original file was updated
        let content = fs::read_to_string(&existing_path).unwrap();
        assert!(content.contains("Updated content"));
        assert!(!content.contains("Old content"));

        // A subsequent scan should NOT pick up the file as untracked
        let progress = crate::actions::progress::CollectingProgress::default();
        let found = scan_vault_for_untracked(&mut manifest, vault, &progress).unwrap();
        assert_eq!(found, 0, "overwritten file should not appear as untracked");
    }

    #[test]
    fn scan_untracked_computes_all_three_hashes() {
        let dir = TempDir::new().unwrap();
        let vault = dir.path();
        let mut manifest = Manifest::new("test-device".to_string());

        let file_dir = vault.join("@me/temper/task");
        fs::create_dir_all(&file_dir).unwrap();
        fs::write(
            file_dir.join("my-task.md"),
            "---\ntemper-type: task\ntemper-title: My Task\ntemper-id: 019d0000-0000-0000-0000-000000000001\ntemper-context: temper\ndate: 2026-01-01\n---\n\n# My Task\n\nBody content here.\n",
        )
        .unwrap();

        let progress = crate::actions::progress::CollectingProgress::default();
        let found = scan_vault_for_untracked(&mut manifest, vault, &progress).unwrap();
        assert_eq!(found, 1);

        // The entry should have all three hashes populated
        let entry = manifest.entries.values().next().unwrap();
        assert!(!entry.body_hash.is_empty(), "body_hash should be populated");
        assert!(
            !entry.managed_hash.is_empty(),
            "managed_hash should be populated"
        );
        assert!(!entry.open_hash.is_empty(), "open_hash should be populated");
        assert!(entry.body_hash.starts_with("sha256:"));
        assert!(entry.managed_hash.starts_with("sha256:"));
        assert!(entry.open_hash.starts_with("sha256:"));
    }

    #[test]
    fn dedup_only_applies_to_genuinely_new_resources() {
        // When pulling a resource NOT in the manifest, and the slug
        // already exists on disk, dedup should still work correctly.
        let dir = TempDir::new().unwrap();
        let vault = dir.path();

        let file_dir = vault.join("@me/temper/task");
        fs::create_dir_all(&file_dir).unwrap();
        fs::write(file_dir.join("my-document.md"), "existing content").unwrap();

        let slug = ingest::dedup_vault_slug(vault, "temper", "task", "my-document");
        assert_eq!(
            slug, "my-document-2",
            "new resource should get deduplicated slug"
        );
    }

    // --- resolve_owner_for_frontmatter ---

    #[test]
    fn resolve_owner_for_frontmatter_resolves_at_me() {
        assert_eq!(
            resolve_owner_for_frontmatter("@me", "j-cole-taylor"),
            "@j-cole-taylor"
        );
    }

    #[test]
    fn resolve_owner_for_frontmatter_passes_through_team_handle() {
        assert_eq!(
            resolve_owner_for_frontmatter("+platform-eng", "j-cole-taylor"),
            "+platform-eng"
        );
    }

    #[test]
    fn resolve_owner_for_frontmatter_passes_through_other_user() {
        assert_eq!(
            resolve_owner_for_frontmatter("@some-other-user", "j-cole-taylor"),
            "@some-other-user"
        );
    }

    // --- preflight_ownership_check ---

    #[test]
    fn preflight_detects_synced_owner_drift() {
        let tmp = tempfile::TempDir::new().unwrap();
        let vault = tmp.path();

        let file_dir = vault.join("@me").join("temper").join("task");
        std::fs::create_dir_all(&file_dir).unwrap();
        std::fs::write(
            file_dir.join("drifted.md"),
            "---\ntemper-type: task\ntemper-owner: \"+team\"\ntemper-title: d\ntemper-slug: d\n---\n\nbody\n",
        )
        .unwrap();

        let mut manifest = Manifest::new("dev".to_string());
        let id = ResourceId::from(Uuid::now_v7());
        manifest.entries.insert(
            id,
            ManifestEntry {
                path: "@me/temper/task/drifted.md".to_string(),
                body_hash: "h".to_string(),
                remote_body_hash: "h".to_string(),
                managed_hash: String::new(),
                open_hash: String::new(),
                remote_managed_hash: String::new(),
                remote_open_hash: String::new(),
                synced_at: Utc::now(),
                state: ManifestEntryState::Clean,
                mtime_secs: None,
                last_audit_id: None,
                provisional: false,
            },
        );

        let mismatches = preflight_ownership_check(&manifest, vault);
        assert_eq!(mismatches.len(), 1);
        assert_eq!(mismatches[0].frontmatter_owner, "+team");
        assert_eq!(mismatches[0].manifest_owner, "@me");
    }

    #[test]
    fn preflight_ignores_provisional_entries() {
        let tmp = tempfile::TempDir::new().unwrap();
        let vault = tmp.path();

        let file_dir = vault.join("@me").join("temper").join("task");
        std::fs::create_dir_all(&file_dir).unwrap();
        std::fs::write(
            file_dir.join("new.md"),
            "---\ntemper-type: task\ntemper-owner: \"+different\"\ntemper-title: n\ntemper-slug: n\n---\n\nbody\n",
        )
        .unwrap();

        let mut manifest = Manifest::new("dev".to_string());
        let id = ResourceId::from(Uuid::now_v7());
        manifest.entries.insert(
            id,
            ManifestEntry {
                path: "@me/temper/task/new.md".to_string(),
                body_hash: "h".to_string(),
                remote_body_hash: String::new(),
                managed_hash: String::new(),
                open_hash: String::new(),
                remote_managed_hash: String::new(),
                remote_open_hash: String::new(),
                synced_at: Utc::now(),
                state: ManifestEntryState::Pending,
                mtime_secs: None,
                last_audit_id: None,
                provisional: true,
            },
        );

        let mismatches = preflight_ownership_check(&manifest, vault);
        assert!(
            mismatches.is_empty(),
            "provisional entries should be ignored"
        );
    }

    #[test]
    fn preflight_clean_manifest_returns_empty() {
        let tmp = tempfile::TempDir::new().unwrap();
        let vault = tmp.path();

        let file_dir = vault.join("@me").join("temper").join("task");
        std::fs::create_dir_all(&file_dir).unwrap();
        std::fs::write(
            file_dir.join("clean.md"),
            "---\ntemper-type: task\ntemper-owner: \"@me\"\ntemper-title: c\ntemper-slug: c\n---\n\nbody\n",
        )
        .unwrap();

        let mut manifest = Manifest::new("dev".to_string());
        let id = ResourceId::from(Uuid::now_v7());
        manifest.entries.insert(
            id,
            ManifestEntry {
                path: "@me/temper/task/clean.md".to_string(),
                body_hash: "h".to_string(),
                remote_body_hash: "h".to_string(),
                managed_hash: String::new(),
                open_hash: String::new(),
                remote_managed_hash: String::new(),
                remote_open_hash: String::new(),
                synced_at: Utc::now(),
                state: ManifestEntryState::Clean,
                mtime_secs: None,
                last_audit_id: None,
                provisional: false,
            },
        );

        let mismatches = preflight_ownership_check(&manifest, vault);
        assert!(mismatches.is_empty());
    }

    // -----------------------------------------------------------------
    // normalize_all_entries tests
    // -----------------------------------------------------------------

    /// Build a minimal manifest entry pointing at `rel_path` with all
    /// string fields empty (no remote hashes, no mtime, Clean state).
    fn blank_entry(rel_path: &str) -> ManifestEntry {
        ManifestEntry {
            path: rel_path.to_string(),
            body_hash: String::new(),
            remote_body_hash: String::new(),
            managed_hash: String::new(),
            open_hash: String::new(),
            remote_managed_hash: String::new(),
            remote_open_hash: String::new(),
            synced_at: Utc::now(),
            state: ManifestEntryState::Clean,
            mtime_secs: None,
            last_audit_id: None,
            provisional: false,
        }
    }

    /// Create `<vault>/<rel_path>` with the given content, creating parents.
    fn write_vault_file(vault_root: &Path, rel_path: &str, content: &str) {
        let abs = vault_root.join(rel_path);
        if let Some(parent) = abs.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(&abs, content).unwrap();
    }

    #[test]
    fn normalize_all_entries_rewrites_missing_defaults() {
        let dir = TempDir::new().unwrap();
        let vault_root = dir.path();
        let temper_dir = vault_root.join(".temper");
        fs::create_dir_all(&temper_dir).unwrap();

        let id = ResourceId::from(Uuid::now_v7());
        let rel_path = format!("@me/temper/task/{id}.md");
        let content = format!(
            "---\n\
             temper-id: \"{id}\"\n\
             temper-type: task\n\
             temper-context: temper\n\
             temper-created: \"2026-04-12T00:00:00Z\"\n\
             temper-title: Test\n\
             temper-slug: test\n\
             ---\n\
             body\n",
        );
        write_vault_file(vault_root, &rel_path, &content);

        let mut manifest = Manifest::new("device-test".to_string());
        manifest.entries.insert(id, blank_entry(&rel_path));

        let report = normalize_all_entries(&mut manifest, vault_root, &temper_dir, None).unwrap();

        assert_eq!(report.scanned, 1);
        assert_eq!(report.rewritten, 1);
        assert_eq!(report.blocked, 0);
        assert_eq!(report.missing, 0);

        let on_disk = fs::read_to_string(vault_root.join(&rel_path)).unwrap();
        assert!(
            on_disk.contains("temper-stage: backlog"),
            "file should contain temper-stage: backlog, got:\n{on_disk}"
        );

        let entry = manifest.entries.get(&id).unwrap();
        assert!(entry.body_hash.starts_with("sha256:"));
        assert!(entry.managed_hash.starts_with("sha256:"));
        assert!(entry.open_hash.starts_with("sha256:"));
    }

    #[test]
    fn normalize_all_entries_blocks_invalid_enum() {
        let dir = TempDir::new().unwrap();
        let vault_root = dir.path();
        let temper_dir = vault_root.join(".temper");
        fs::create_dir_all(&temper_dir).unwrap();

        let id = ResourceId::from(Uuid::now_v7());
        let rel_path = format!("@me/temper/task/{id}.md");
        let content = format!(
            "---\n\
             temper-id: \"{id}\"\n\
             temper-type: task\n\
             temper-context: temper\n\
             temper-created: \"2026-04-12T00:00:00Z\"\n\
             temper-title: Test\n\
             temper-slug: test\n\
             temper-stage: frobnicate\n\
             ---\n\
             body\n",
        );
        write_vault_file(vault_root, &rel_path, &content);
        let before = fs::read_to_string(vault_root.join(&rel_path)).unwrap();

        let mut manifest = Manifest::new("device-test".to_string());
        manifest.entries.insert(id, blank_entry(&rel_path));

        let report = normalize_all_entries(&mut manifest, vault_root, &temper_dir, None).unwrap();

        let after = fs::read_to_string(vault_root.join(&rel_path)).unwrap();
        assert_eq!(before, after, "file should not be rewritten on block");

        assert_eq!(report.blocked, 1);
        assert_eq!(report.rewritten, 0);
        assert_eq!(report.issues_by_path.len(), 1);
        assert_eq!(report.issues_by_path[0].0, rel_path);
        assert!(!report.issues_by_path[0].1.is_empty());

        let entry = manifest.entries.get(&id).unwrap();
        assert!(
            entry.body_hash.starts_with("sha256:"),
            "hashes still populated on block"
        );
        assert!(entry.managed_hash.starts_with("sha256:"));
        assert!(entry.open_hash.starts_with("sha256:"));
    }

    #[test]
    fn normalize_all_entries_persists_per_entry() {
        let dir = TempDir::new().unwrap();
        let vault_root = dir.path();
        let temper_dir = vault_root.join(".temper");
        fs::create_dir_all(&temper_dir).unwrap();

        // File A: canonical, clean.
        let id_a = ResourceId::from(Uuid::now_v7());
        let rel_a = format!("@me/temper/task/{id_a}.md");
        let content_a = format!(
            "---\n\
             temper-id: \"{id_a}\"\n\
             temper-type: task\n\
             temper-context: temper\n\
             temper-created: \"2026-04-12T00:00:00Z\"\n\
             temper-title: A\n\
             temper-slug: a\n\
             temper-stage: backlog\n\
             ---\n\
             body A\n",
        );
        write_vault_file(vault_root, &rel_a, &content_a);

        // File B: missing temper-stage, triggers rewrite.
        let id_b = ResourceId::from(Uuid::now_v7());
        let rel_b = format!("@me/temper/task/{id_b}.md");
        let content_b = format!(
            "---\n\
             temper-id: \"{id_b}\"\n\
             temper-type: task\n\
             temper-context: temper\n\
             temper-created: \"2026-04-12T00:00:00Z\"\n\
             temper-title: B\n\
             temper-slug: b\n\
             ---\n\
             body B\n",
        );
        write_vault_file(vault_root, &rel_b, &content_b);

        let mut manifest = Manifest::new("device-test".to_string());
        manifest.entries.insert(id_a, blank_entry(&rel_a));
        manifest.entries.insert(id_b, blank_entry(&rel_b));

        normalize_all_entries(&mut manifest, vault_root, &temper_dir, None).unwrap();

        // Read manifest back from disk — both entries should have
        // post-normalize hashes. This proves save happened before return.
        let reloaded = crate::manifest_io::load_manifest(&temper_dir, "device-test").unwrap();
        assert_eq!(reloaded.entries.len(), 2);
        for (reloaded_id, reloaded_entry) in &reloaded.entries {
            assert!(
                reloaded_entry.body_hash.starts_with("sha256:"),
                "entry {reloaded_id} body_hash missing on disk"
            );
            assert!(
                reloaded_entry.managed_hash.starts_with("sha256:"),
                "entry {reloaded_id} managed_hash missing on disk"
            );
            assert!(
                reloaded_entry.open_hash.starts_with("sha256:"),
                "entry {reloaded_id} open_hash missing on disk"
            );
        }
    }

    #[test]
    fn normalize_all_entries_marks_missing_files() {
        let dir = TempDir::new().unwrap();
        let vault_root = dir.path();
        let temper_dir = vault_root.join(".temper");
        fs::create_dir_all(&temper_dir).unwrap();

        let id = ResourceId::from(Uuid::now_v7());
        let rel_path = format!("@me/temper/task/{id}.md");
        // Note: file is NOT created on disk.

        let mut manifest = Manifest::new("device-test".to_string());
        manifest.entries.insert(id, blank_entry(&rel_path));

        let report = normalize_all_entries(&mut manifest, vault_root, &temper_dir, None).unwrap();

        assert_eq!(report.scanned, 1);
        assert_eq!(report.missing, 1);
        assert_eq!(report.rewritten, 0);
        assert_eq!(report.blocked, 0);

        let entry = manifest.entries.get(&id).unwrap();
        assert_eq!(entry.state, ManifestEntryState::LocalModified);
        assert!(entry.body_hash.is_empty());
    }

    #[test]
    fn normalize_all_entries_preserves_clean_entries() {
        let dir = TempDir::new().unwrap();
        let vault_root = dir.path();
        let temper_dir = vault_root.join(".temper");
        fs::create_dir_all(&temper_dir).unwrap();

        let id = ResourceId::from(Uuid::now_v7());
        let rel_path = format!("@me/temper/task/{id}.md");

        // Use normalize_file against a throwaway path to compute what the
        // canonical hashes will be after normalize runs (we want to seed
        // the remote triple so state resolves to Clean).
        let scratch_dir = TempDir::new().unwrap();
        let scratch_rel = format!("@me/temper/task/{id}.md");
        let canonical_content = format!(
            "---\n\
             temper-id: \"{id}\"\n\
             temper-type: task\n\
             temper-context: temper\n\
             temper-created: \"2026-04-12T00:00:00Z\"\n\
             temper-title: Test\n\
             temper-slug: test\n\
             temper-stage: backlog\n\
             ---\n\
             body\n",
        );
        write_vault_file(scratch_dir.path(), &scratch_rel, &canonical_content);
        // First normalize: may rewrite to match the YAML emitter's format.
        let _ =
            temper_core::normalize::normalize_file(&scratch_dir.path().join(&scratch_rel), "task")
                .expect("scratch normalize ok");
        // Second normalize: stable canonical form.
        let outcome =
            temper_core::normalize::normalize_file(&scratch_dir.path().join(&scratch_rel), "task")
                .expect("scratch normalize ok");
        assert!(!outcome.changed, "second normalize should be a no-op");
        let stable_content = fs::read_to_string(scratch_dir.path().join(&scratch_rel)).unwrap();

        // Write the stable canonical content to the real vault.
        write_vault_file(vault_root, &rel_path, &stable_content);
        let before = fs::read_to_string(vault_root.join(&rel_path)).unwrap();

        let mut manifest = Manifest::new("device-test".to_string());
        let mut entry = blank_entry(&rel_path);
        entry.body_hash = outcome.body_hash.clone();
        entry.managed_hash = outcome.managed_hash.clone();
        entry.open_hash = outcome.open_hash.clone();
        entry.remote_body_hash = outcome.body_hash.clone();
        entry.remote_managed_hash = outcome.managed_hash.clone();
        entry.remote_open_hash = outcome.open_hash.clone();
        entry.state = ManifestEntryState::Clean;
        manifest.entries.insert(id, entry);

        let report = normalize_all_entries(&mut manifest, vault_root, &temper_dir, None).unwrap();

        assert_eq!(report.scanned, 1);
        assert_eq!(report.rewritten, 0, "clean canonical file must not rewrite");
        assert_eq!(report.blocked, 0);
        assert_eq!(report.missing, 0);

        let after = fs::read_to_string(vault_root.join(&rel_path)).unwrap();
        assert_eq!(before, after, "canonical file byte-identical");

        let entry = manifest.entries.get(&id).unwrap();
        assert_eq!(entry.state, ManifestEntryState::Clean);
        assert_eq!(entry.body_hash, outcome.body_hash);
        assert_eq!(entry.managed_hash, outcome.managed_hash);
        assert_eq!(entry.open_hash, outcome.open_hash);
    }

    // -----------------------------------------------------------------
    // Phase E1b: meta-only push/pull
    // -----------------------------------------------------------------

    fn meta_test_resource_row(id: ResourceId) -> temper_core::types::ResourceRow {
        use temper_core::types::ids::{ContextId, DocTypeId, ProfileId};
        temper_core::types::ResourceRow {
            id,
            kb_context_id: ContextId(Uuid::nil()),
            kb_doc_type_id: DocTypeId(Uuid::nil()),
            origin_uri: format!("kb://@me/temper/task/{id}"),
            title: "Meta Test".to_string(),
            slug: Some("meta-test".to_string()),
            originator_profile_id: ProfileId(Uuid::nil()),
            owner_profile_id: ProfileId(Uuid::nil()),
            is_active: true,
            created: chrono::Utc::now(),
            updated: chrono::Utc::now(),
            context_name: "temper".to_string(),
            doc_type_name: "task".to_string(),
            owner_handle: "@me".to_string(),
            stage: Some("backlog".to_string()),
            seq: Some(1),
            mode: None,
            effort: None,
            body_hash: None,
            managed_hash: None,
            open_hash: None,
        }
    }

    #[test]
    fn push_meta_only_payload_roundtrip() {
        let id = ResourceId::from(Uuid::now_v7());
        let fm_text = format!(
            "---\n\
             temper-id: \"{id}\"\n\
             temper-type: task\n\
             temper-context: temper\n\
             temper-title: Payload Roundtrip\n\
             temper-slug: payload-roundtrip\n\
             temper-stage: backlog\n\
             tags: [rust, meta]\n\
             notes: hello\n\
             ---\n\
             body\n",
        );
        let fm = Frontmatter::try_from(fm_text.as_str()).expect("parse fm");
        let payload = build_meta_update_payload(&fm, id.into());

        // Direct comparison against the parsed Frontmatter's hashes —
        // same input must produce identical (managed_hash, open_hash).
        let (expected_managed, expected_open) = fm.hashes();
        assert_eq!(payload.managed_hash, expected_managed);
        assert_eq!(payload.open_hash, expected_open);

        // Direct comparison against the Frontmatter tier projections.
        // Round-trip the managed side through the typed ManagedMeta via
        // the flatten extras bucket so the hash stays stable.
        let expected_managed_meta_json = fm.managed_json();
        let expected_open_meta = fm.open_json();
        let expected_managed_meta: temper_core::types::managed_meta::ManagedMeta =
            serde_json::from_value(expected_managed_meta_json).expect("expected → typed");
        assert_eq!(payload.managed_meta, expected_managed_meta);
        assert_eq!(payload.open_meta, expected_open_meta);

        // resource_id round-trips through ResourceId::from(Uuid).
        assert_eq!(payload.resource_id, id);

        // Structural checks via the typed accessors — title + stage are
        // managed tier, tags + notes are open tier.
        assert_eq!(
            payload.managed_meta.title.as_deref(),
            Some("Payload Roundtrip")
        );
        assert_eq!(payload.managed_meta.stage.as_deref(), Some("backlog"));
        assert!(payload.open_meta.get("tags").is_some());
        assert_eq!(payload.open_meta["notes"], "hello");
    }

    #[test]
    fn pull_meta_only_rebuild_preserves_body() {
        let id = ResourceId::from(Uuid::now_v7());
        let resource = meta_test_resource_row(id);

        // Build a realistic existing vault file, then derive its body the
        // same way `pull_resource_meta_only` does — via `strip_frontmatter`.
        // This is the ground-truth "local body" that must round-trip.
        //
        // The body carries edge characters: blank lines, a trailing newline,
        // and a YAML-looking fake-frontmatter block inside a code fence that
        // must NOT be mistaken for real frontmatter.
        let original_file = "---\n\
                             temper-id: \"019d0000-0000-7000-8000-000000000001\"\n\
                             temper-type: task\n\
                             temper-title: Original\n\
                             ---\n\
                             \n\
                             # Heading\n\
                             \n\
                             Some prose with `inline` code.\n\
                             \n\
                             ```yaml\n\
                             ---\n\
                             fake: frontmatter\n\
                             ---\n\
                             ```\n\
                             \n\
                             Trailing paragraph.\n";
        let local_body = strip_frontmatter(original_file).to_string();
        // Sanity: the code-fence "frontmatter" is still intact inside the body.
        assert!(local_body.contains("fake: frontmatter"));
        assert!(local_body.contains("# Heading"));

        // The rebuild helper takes JSON Values (same shape that
        // `build_frontmatter_from_resource` consumes), so construct the
        // meta directly as Values here. The typed `ManagedMeta` path is
        // exercised by `build_meta_update_payload_roundtrip` above.
        let managed = serde_json::json!({
            "temper-type": "task",
            "temper-context": "temper",
            "temper-stage": "in_progress",
            "title": "Meta Test",
            "slug": "meta-test",
        });
        let open = serde_json::json!({
            "tags": ["rust", "graph"],
        });

        let rebuilt = rebuild_file_with_new_meta(
            &local_body,
            &resource,
            "temper",
            "task",
            Some(&managed),
            Some(&open),
        )
        .unwrap();

        // After rebuild, stripping the new file must yield the same body
        // byte-for-byte — no normalization, no swallowed lines, no
        // characters absorbed into the frontmatter block.
        let stripped = strip_frontmatter(&rebuilt);
        assert_eq!(stripped, local_body);

        // Both meta tiers must appear in the rebuilt frontmatter block.
        let block = extract_frontmatter_block(&rebuilt);
        assert!(
            block.contains("temper-stage"),
            "managed tier missing:\n{block}"
        );
        assert!(block.contains("tags:"), "open tier missing:\n{block}");
    }

    #[test]
    fn pull_meta_only_relocation_guard() {
        use temper_core::types::managed_meta::ManagedMeta;

        // (a) None → Ok
        assert!(check_relocation_guard("temper", None).is_ok());

        // (b) Some without temper-context → Ok
        let meta = ManagedMeta {
            title: Some("No ctx here".to_string()),
            ..Default::default()
        };
        assert!(check_relocation_guard("temper", Some(&meta)).is_ok());

        // (c) Some with matching temper-context → Ok
        let meta = ManagedMeta {
            context: Some("temper".to_string()),
            title: Some("match".to_string()),
            ..Default::default()
        };
        assert!(check_relocation_guard("temper", Some(&meta)).is_ok());

        // (d) Some with differing temper-context → Err, and the error
        // message must contain both the old and new context names.
        let meta = ManagedMeta {
            context: Some("research".to_string()),
            title: Some("moved".to_string()),
            ..Default::default()
        };
        let err = check_relocation_guard("temper", Some(&meta)).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("temper"), "err missing old ctx: {msg}");
        assert!(msg.contains("research"), "err missing new ctx: {msg}");
    }

    #[test]
    fn pull_meta_only_advances_mtime() {
        let dir = TempDir::new().unwrap();
        let vault_root = dir.path();

        let id = ResourceId::from(Uuid::now_v7());
        let rel_path = format!("@me/temper/task/{id}.md");
        let abs = vault_root.join(&rel_path);
        fs::create_dir_all(abs.parent().unwrap()).unwrap();

        let initial = format!(
            "---\n\
             temper-id: \"{id}\"\n\
             temper-type: task\n\
             temper-context: temper\n\
             temper-created: \"2026-04-12T00:00:00Z\"\n\
             temper-title: Initial\n\
             temper-slug: initial\n\
             temper-stage: backlog\n\
             ---\n\
             initial body\n",
        );
        fs::write(&abs, &initial).unwrap();

        // Baseline normalize so the starting file is canonical —
        // apply_pull_meta_only will re-normalize after its rewrite, and
        // we want to ensure it is the normalize call, not disk-flush
        // timing, that advances mtime.
        let _ = temper_core::normalize::normalize_file(&abs, "task").unwrap();

        let mut entry = blank_entry(&rel_path);
        // Deliberately stale mtime so we can assert it advances.
        entry.mtime_secs = Some(0);

        let resource = meta_test_resource_row(id);
        let managed = serde_json::json!({
            "temper-type": "task",
            "temper-context": "temper",
            "temper-stage": "in-progress",
            "title": "Initial",
            "slug": "initial",
        });
        let open = serde_json::json!({});

        let local_content = fs::read_to_string(&abs).unwrap();
        let local_body = strip_frontmatter(&local_content).to_string();

        // Small sleep to guarantee a distinct filesystem mtime even on
        // coarse-granularity platforms.
        std::thread::sleep(std::time::Duration::from_millis(10));

        apply_pull_meta_only(
            ApplyPullMetaOnly {
                file_path: &abs,
                local_body: &local_body,
                resource: &resource,
                ctx: "temper",
                doc_type: "task",
                managed_meta: Some(&managed),
                open_meta: Some(&open),
            },
            &mut entry,
        )
        .unwrap();

        let new_mtime = entry.mtime_secs.expect("mtime must be set");
        assert!(
            new_mtime > 0,
            "mtime_secs must advance past the stale sentinel 0, got {new_mtime}"
        );

        // The manifest entry hashes are now populated and state is Clean.
        assert_eq!(entry.state, ManifestEntryState::Clean);
        assert!(entry.managed_hash.starts_with("sha256:"));
        assert!(entry.open_hash.starts_with("sha256:"));
        assert_eq!(entry.managed_hash, entry.remote_managed_hash);
        assert_eq!(entry.open_hash, entry.remote_open_hash);

        // And on-disk content still contains the local body.
        let final_content = fs::read_to_string(&abs).unwrap();
        assert!(
            final_content.contains("initial body"),
            "body must be preserved across meta-only pull"
        );
    }

    #[test]
    fn vault_file_missing_err_includes_both_recovery_hints() {
        let err = super::vault_file_missing_err("task/2026-04-29-some-slug.md");
        let msg = format!("{err}");
        assert!(
            msg.contains("2026-04-29-some-slug"),
            "expected derived slug in message, got: {msg}"
        );
        assert!(
            msg.contains("temper resource delete"),
            "expected delete hint, got: {msg}"
        );
        assert!(
            msg.contains("temper sync refresh"),
            "expected refresh hint, got: {msg}"
        );
        assert!(
            msg.contains("task/2026-04-29-some-slug.md"),
            "expected original path in message, got: {msg}"
        );
    }
}
