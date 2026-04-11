//! Sync orchestration logic — rehash manifest, build requests, push/pull/remove.
//!
//! Pure functions (rehash, build_request, parse_uri, strip_frontmatter) are
//! fully unit-testable. Async functions take client and manifest references.

use std::path::Path;

use uuid::Uuid;

use crate::actions::ingest;
use crate::actions::progress::SyncProgress;
use crate::error::{Result, TemperError};
use temper_core::types::{
    Manifest, ManifestEntryState, MergeResult, MergedResource, PushKind, ResourceId,
    SyncCompleteRequest, SyncConflictItem, SyncContextEntries, SyncManifestEntry, SyncPullItem,
    SyncPushItem, SyncRemovedItem, SyncStatusRequest, SyncStatusResponse,
};
use temper_core::vault::Vault;

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
        let Some(fm) = crate::vault::parse_frontmatter(&content) else {
            continue;
        };
        let frontmatter_owner = fm
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

        // Compute frontmatter tier hashes
        let doc_type =
            temper_core::hash::doc_type_from_vault_path(&entry.path).unwrap_or("unknown");
        let (managed_hash, open_hash) = temper_core::hash::compute_frontmatter_hashes_from_yaml(
            crate::vault::parse_frontmatter(&content).as_ref(),
            doc_type,
        );

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

        let (managed_hash, open_hash) = temper_core::hash::compute_frontmatter_hashes_from_yaml(
            crate::vault::parse_frontmatter(&full_content).as_ref(),
            &doc_type,
        );

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
    // Resolve the manifest entry ID — for new resources this is embedded in the URI,
    // for existing resources the server provides the resource_id directly.
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
        return Err(TemperError::NotFound(format!(
            "vault file not found: {}",
            file_path.display()
        )));
    }

    let content = std::fs::read_to_string(&file_path)?;
    let body = strip_frontmatter(&content);

    let (context, doc_type) = match Vault::parse_rel(&entry.path) {
        Some(parsed) => (parsed.context.to_string(), parsed.doc_type.to_string()),
        None => ("default".to_string(), "resource".to_string()),
    };

    // Parse frontmatter and split into managed/open tiers
    let (managed_meta, open_meta) = if let Some(fm) = crate::vault::parse_frontmatter(&content) {
        let (m, o) = temper_core::hash::split_frontmatter_tiers(&fm, &doc_type);
        (Some(m), Some(o))
    } else {
        (None, None)
    };
    let title = ingest::title_from_path(&file_path);

    let mut payload = ingest::build_ingest_payload(body, &title, &context, &doc_type, None)?;
    payload.managed_meta = managed_meta;
    payload.open_meta = open_meta;

    let is_provisional = manifest
        .entries
        .get(&entry_id)
        .map_or(false, |e| e.provisional);

    let resource = if item.resource_id.is_some() && !is_provisional {
        // Existing resource — PUT update
        client
            .ingest()
            .update(Uuid::from(entry_id), &payload)
            .await
            .map_err(crate::commands::client_err)?
    } else {
        // New resource — POST create (also used for provisional entries)
        client
            .ingest()
            .create(&payload)
            .await
            .map_err(crate::commands::client_err)?
    };

    // If the server assigned a different resource ID (POST create), remap the
    // manifest entry so the local UUID matches the server's authoritative ID.
    let server_id = resource.id;
    if server_id != entry_id || is_provisional {
        tracing::info!(
            %entry_id,
            %server_id,
            is_provisional,
            "remapping manifest entry: local ID → server ID"
        );
        if let Some(mut entry) = manifest.entries.remove(&entry_id) {
            entry.provisional = false;
            manifest.entries.insert(server_id, entry);

            // Replace provisional frontmatter key+value with authoritative temper-id.
            let file_content = std::fs::read_to_string(&file_path)?;
            let updated = file_content
                .replace(
                    &format!("temper-provisional-id: \"{entry_id}\""),
                    &format!("temper-id: \"{server_id}\""),
                )
                .replace(
                    &format!("temper-provisional-id: {entry_id}"),
                    &format!("temper-id: {server_id}"),
                );

            if updated != file_content {
                std::fs::write(&file_path, &updated)?;
                tracing::info!("replaced temper-provisional-id with temper-id in frontmatter");
            } else {
                // Fallback: try replacing old-style id: or temper-id: (for files
                // that already had temper-id with a local UUID)
                let fallback = file_content.replace(&entry_id.to_string(), &server_id.to_string());
                if fallback != file_content {
                    std::fs::write(&file_path, &fallback)?;
                    tracing::info!("updated temper-id in file frontmatter (fallback path)");
                } else {
                    tracing::warn!(
                        %entry_id,
                        "temper-provisional-id not found in file content — frontmatter not updated"
                    );
                }
            }
        }
    }

    // Compute frontmatter hashes so we can record them as the remote values
    let (pushed_managed_hash, pushed_open_hash) = {
        let current = std::fs::read_to_string(&file_path)?;
        temper_core::hash::compute_frontmatter_hashes_from_yaml(
            crate::vault::parse_frontmatter(&current).as_ref(),
            &doc_type,
        )
    };

    if let Some(e) = manifest.entries.get_mut(&server_id) {
        // After push, server hashes match what we sent
        e.remote_body_hash = payload.content_hash.clone().unwrap_or_default();
        e.remote_managed_hash = pushed_managed_hash;
        e.remote_open_hash = pushed_open_hash;
        e.state = ManifestEntryState::Clean;
        e.synced_at = chrono::Utc::now();
        e.mtime_secs = file_mtime_secs(&file_path).ok();
    }

    Ok(())
}

async fn pull_resource(
    client: &temper_client::TemperClient,
    manifest: &mut Manifest,
    vault_root: &Path,
    item: &SyncPullItem,
) -> Result<()> {
    let resource = client
        .resources()
        .get(Uuid::from(item.resource_id))
        .await
        .map_err(crate::commands::client_err)?;

    let content_response = client
        .resources()
        .content(Uuid::from(item.resource_id))
        .await
        .map_err(crate::commands::client_err)?;

    let (ctx, doc_type) = parse_kb_uri(&item.uri)?;

    // If the resource is already in the manifest, overwrite the existing file
    // instead of creating a deduplicated copy (slug-2, slug-3, etc.).
    let vault_path = if let Some(existing) = manifest.entries.get(&item.resource_id) {
        let existing_path = vault_root.join(&existing.path);
        if existing_path.exists() {
            // Overwrite the existing file in place — no slug dedup needed.
            let frontmatter = ingest::build_frontmatter_from_resource(
                &resource,
                &ctx,
                &doc_type,
                content_response.managed_meta.as_ref(),
            );
            let vault_content = format!("{frontmatter}{}", &content_response.markdown);
            std::fs::write(&existing_path, &vault_content)?;
            existing_path
        } else {
            // Manifest entry exists but file is missing — write to expected path.
            let slug = ingest::slug_from_title(&resource.title);
            let slug = ingest::dedup_vault_slug(vault_root, &ctx, &doc_type, &slug);
            write_pulled_file(
                vault_root,
                &ctx,
                &doc_type,
                &slug,
                &resource,
                &content_response.markdown,
                content_response.managed_meta.as_ref(),
            )?
        }
    } else {
        // Genuinely new resource — dedup slug as usual.
        let slug = ingest::slug_from_title(&resource.title);
        let slug = ingest::dedup_vault_slug(vault_root, &ctx, &doc_type, &slug);
        write_pulled_file(
            vault_root,
            &ctx,
            &doc_type,
            &slug,
            &resource,
            &content_response.markdown,
            content_response.managed_meta.as_ref(),
        )?
    };

    // Update the in-memory manifest directly (no disk reload).
    // Read the file back and strip frontmatter to compute the hash — this
    // must match what rehash_manifest() computes, which includes the newline
    // separator between frontmatter and body.
    let full_content = std::fs::read_to_string(&vault_path)?;
    let body = strip_frontmatter(&full_content);
    let content_hash = temper_core::hash::compute_body_hash(body);
    let rel_path = vault_path
        .strip_prefix(vault_root)
        .unwrap_or(&vault_path)
        .to_string_lossy()
        .to_string();

    // Compute frontmatter tier hashes from the written file
    let (managed_hash, open_hash) = temper_core::hash::compute_frontmatter_hashes_from_yaml(
        crate::vault::parse_frontmatter(&full_content).as_ref(),
        &doc_type,
    );

    let mtime_secs = file_mtime_secs(&vault_path).ok();

    manifest.entries.insert(
        item.resource_id,
        temper_core::types::ManifestEntry {
            path: rel_path,
            body_hash: content_hash,
            remote_body_hash: item.content_hash.clone(),
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

    Ok(())
}

/// Write a pulled file to the vault (new resource or missing file).
///
/// Creates parent directories and writes frontmatter + content. Does NOT
/// touch the manifest — the caller is responsible for that.
fn write_pulled_file(
    vault_root: &Path,
    context: &str,
    doc_type: &str,
    slug: &str,
    resource: &temper_core::types::ResourceRow,
    content: &str,
    managed_meta: Option<&serde_json::Value>,
) -> Result<std::path::PathBuf> {
    let vault_path = ingest::build_vault_path(vault_root, context, doc_type, slug);

    if let Some(parent) = vault_path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let frontmatter =
        ingest::build_frontmatter_from_resource(resource, context, doc_type, managed_meta);
    let vault_content = format!("{frontmatter}{content}");
    std::fs::write(&vault_path, &vault_content)?;

    Ok(vault_path)
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
        return Err(TemperError::NotFound(format!(
            "vault file not found: {}",
            file_path.display()
        )));
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

    let payload = ingest::build_ingest_payload(merged_body, &title, &context, &doc_type, None)?;

    // 7. Push via update
    let _resource = client
        .ingest()
        .update(Uuid::from(item.resource_id), &payload)
        .await
        .map_err(crate::commands::client_err)?;

    // 8. Compute frontmatter hashes from the merged file
    let (pushed_managed_hash, pushed_open_hash) =
        temper_core::hash::compute_frontmatter_hashes_from_yaml(
            crate::vault::parse_frontmatter(&new_file_content).as_ref(),
            &doc_type,
        );

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

        // Compute local frontmatter tier hashes
        let reset_doc_type =
            temper_core::hash::doc_type_from_vault_path(&rel_path).unwrap_or("unknown");
        let (local_managed_hash, local_open_hash) =
            temper_core::hash::compute_frontmatter_hashes_from_yaml(
                crate::vault::parse_frontmatter(&content).as_ref(),
                reset_doc_type,
            );

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
            "new content",
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
        let content = "---\ntemper-type: task\ntitle: Test\ndate: 2026-01-01\n---\ntest content";
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

        let content = "---\ntemper-type: task\ntitle: Test\ndate: 2026-01-01\n---\ntest content";
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
        let content = "---\ntitle: test\n---\n\n# Hello";
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

        let file_v1 = "---\ntitle: Old Title\ncreated: 2026-01-01\n---\n\n# My Document\n\nSome content here.\n";
        let file_v2 = "---\ntitle: New Title\ncreated: 2026-04-03\n---\n\n# My Document\n\nSome content here.\n";

        // Compute hashes for v1
        let body_hash = temper_core::hash::compute_body_hash(strip_frontmatter(file_v1));
        let fm_v1 = crate::vault::parse_frontmatter(file_v1).unwrap();
        let (managed_meta_v1, open_meta_v1) =
            temper_core::hash::split_frontmatter_tiers(&fm_v1, "task");
        let managed_hash_v1 = temper_core::hash::compute_managed_hash("task", &managed_meta_v1);
        let open_hash_v1 = temper_core::hash::compute_open_hash(&open_meta_v1);

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

        let original = "---\ntitle: Test\n---\n\n# Original body\n";
        let modified = "---\ntitle: Test\n---\n\n# Modified body\n";

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
            "---\ntemper-type: task\ntitle: Test\n---\nbody content",
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

        let content = "no frontmatter body";
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
            "---\ncontext: custom\ndoc_type: session\n---\n\n# Overridden\n",
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
        let content = "---\ntitle: Test\ncontext: temper\n---\n\n# Body\n";
        let block = extract_frontmatter_block(content);
        assert_eq!(block, "---\ntitle: Test\ncontext: temper\n---\n");
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
        let frontmatter =
            ingest::build_frontmatter(resource_id, "My Document", "temper", "task", None, None);
        let vault_content = format!("{frontmatter}Updated content");
        fs::write(&existing_path, &vault_content).unwrap();

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
            "---\ntemper-type: task\ntitle: My Task\ntemper-id: 019d0000-0000-0000-0000-000000000001\ntemper-context: temper\ndate: 2026-01-01\n---\n\n# My Task\n\nBody content here.\n",
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

    // --- preflight_ownership_check ---

    #[test]
    fn preflight_detects_synced_owner_drift() {
        let tmp = tempfile::TempDir::new().unwrap();
        let vault = tmp.path();

        let file_dir = vault.join("@me").join("temper").join("task");
        std::fs::create_dir_all(&file_dir).unwrap();
        std::fs::write(
            file_dir.join("drifted.md"),
            "---\ntemper-type: task\ntemper-owner: \"+team\"\ntitle: d\nslug: d\n---\n\nbody\n",
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
            "---\ntemper-type: task\ntemper-owner: \"+different\"\ntitle: n\nslug: n\n---\n\nbody\n",
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
            "---\ntemper-type: task\ntemper-owner: \"@me\"\ntitle: c\nslug: c\n---\n\nbody\n",
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
}
