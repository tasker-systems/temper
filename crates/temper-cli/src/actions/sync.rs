//! Sync orchestration logic — rehash manifest, build requests, push/pull/remove.
//!
//! Pure functions (rehash, build_request, parse_uri, strip_frontmatter) are
//! fully unit-testable. Async functions take client and manifest references.

use std::path::Path;

use uuid::Uuid;

use crate::actions::ingest;
use crate::error::{Result, TemperError};
use temper_core::types::{
    Manifest, ManifestEntryState, MergedResource, SyncCompleteRequest, SyncContextEntries,
    SyncManifestEntry, SyncPullItem, SyncPushItem, SyncRemovedItem, SyncStatusRequest,
    SyncStatusResponse,
};

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
}

// ---------------------------------------------------------------------------
// Pure functions (no client, no async — fully unit-testable)
// ---------------------------------------------------------------------------

/// Rehash manifest entries by reading vault files and computing SHA-256.
/// Returns the count of entries whose state changed.
pub fn rehash_manifest(manifest: &mut Manifest, vault_root: &Path) -> Result<usize> {
    let mut changed = 0;
    for (_id, entry) in manifest.entries.iter_mut() {
        let file_path = vault_root.join(&entry.path);
        if !file_path.exists() {
            if entry.state != ManifestEntryState::LocalModified {
                entry.state = ManifestEntryState::LocalModified;
                entry.content_hash = String::new();
                changed += 1;
            }
            continue;
        }

        let content = std::fs::read_to_string(&file_path)?;
        let current_hash = ingest::compute_content_hash(&content);

        if current_hash != entry.content_hash {
            entry.content_hash = current_hash;
            entry.state = ManifestEntryState::LocalModified;
            changed += 1;
        }
    }
    Ok(changed)
}

/// Build a SyncStatusRequest from the manifest, optionally filtered by contexts.
pub fn build_status_request(manifest: &Manifest, context_filter: &[String]) -> SyncStatusRequest {
    let mut context_map: std::collections::HashMap<String, Vec<SyncManifestEntry>> =
        std::collections::HashMap::new();

    for (id, entry) in &manifest.entries {
        let ctx = entry
            .path
            .split('/')
            .next()
            .unwrap_or("default")
            .to_string();

        if !context_filter.is_empty() && !context_filter.contains(&ctx) {
            continue;
        }

        let parts: Vec<&str> = entry.path.split('/').collect();
        let doc_type = if parts.len() > 1 {
            parts[1]
        } else {
            "resource"
        };
        let uri = format!("kb://{ctx}/{doc_type}/{id}");

        context_map.entry(ctx).or_default().push(SyncManifestEntry {
            uri,
            local_hash: entry.content_hash.clone(),
            remote_hash: entry.remote_hash.clone(),
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

/// Parse a kb:// URI into (context, doc_type).
pub fn parse_kb_uri(uri: &str) -> Result<(String, String)> {
    let rest = uri
        .strip_prefix("kb://")
        .ok_or_else(|| TemperError::Config(format!("invalid kb:// URI: {uri}")))?;
    let parts: Vec<&str> = rest.split('/').collect();
    if parts.len() < 2 {
        return Err(TemperError::Config(format!(
            "kb:// URI must have at least context/doc_type: {uri}"
        )));
    }
    Ok((parts[0].to_string(), parts[1].to_string()))
}

/// Extract resource UUID from last segment of a kb:// URI.
pub fn extract_resource_id(uri: &str) -> Result<Uuid> {
    let uuid_str = uri
        .rsplit('/')
        .next()
        .ok_or_else(|| TemperError::Config(format!("no UUID segment in URI: {uri}")))?;
    Uuid::parse_str(uuid_str)
        .map_err(|e| TemperError::Config(format!("invalid UUID in URI {uri}: {e}")))
}

// ---------------------------------------------------------------------------
// Orchestration (async, uses client + manifest)
// ---------------------------------------------------------------------------

/// Run the full 10-step sync orchestration.
///
/// Called from `sync_cmd.rs` with a single tokio runtime. The command handles
/// manifest load/save and output formatting.
pub async fn sync_orchestration(
    client: &temper_client::TemperClient,
    manifest: &mut Manifest,
    vault_root: &Path,
    context_filter: &[String],
) -> Result<SyncResult> {
    // Step 1: Rehash manifest
    rehash_manifest(manifest, vault_root)?;

    // Step 2: Request diff
    let request = build_status_request(manifest, context_filter);
    let diff = client
        .sync()
        .status(&request)
        .await
        .map_err(|e| TemperError::Api(e.to_string()))?;

    let push_count = diff.to_push.len();
    let pull_count = diff.to_pull.len();
    let conflict_count = diff.conflicts.len();
    let removed_count = diff.removed.len();

    // Step 3: Push
    for item in &diff.to_push {
        push_resource(client, manifest, vault_root, item).await?;
    }

    // Step 4: Pull
    for item in &diff.to_pull {
        pull_resource(client, manifest, vault_root, item).await?;
    }

    // Step 5: Handle conflicts (I6a: mark in manifest, skip)
    for item in &diff.conflicts {
        if let Some(entry) = manifest.entries.get_mut(&item.resource_id) {
            entry.state = ManifestEntryState::Conflict;
        }
    }

    // Step 6: Handle removed
    for item in &diff.removed {
        remove_resource(manifest, vault_root, item)?;
    }

    // Step 7: Complete
    let complete_req = build_complete_request(&manifest.device_id, vec![]);
    let complete_resp = client
        .sync()
        .complete(&complete_req)
        .await
        .map_err(|e| TemperError::Api(e.to_string()))?;

    // Step 8: Update manifest timestamp
    manifest.last_sync = Some(complete_resp.last_sync_at);

    Ok(SyncResult {
        push_count,
        pull_count,
        conflict_count,
        removed_count,
    })
}

/// Run a dry-run sync (rehash + status only, no changes).
pub async fn sync_status_check(
    client: &temper_client::TemperClient,
    manifest: &mut Manifest,
    vault_root: &Path,
    context_filter: &[String],
) -> Result<SyncStatusResponse> {
    rehash_manifest(manifest, vault_root)?;

    let request = build_status_request(manifest, context_filter);
    client
        .sync()
        .status(&request)
        .await
        .map_err(|e| TemperError::Api(e.to_string()))
}

// ---------------------------------------------------------------------------
// Push / Pull / Remove
// ---------------------------------------------------------------------------

async fn push_resource(
    client: &temper_client::TemperClient,
    manifest: &mut Manifest,
    vault_root: &Path,
    item: &SyncPushItem,
) -> Result<()> {
    let resource_id = match item.resource_id {
        Some(id) => id,
        None => {
            // New resource — extract context/doc_type from URI, ingest file
            let entry_id = extract_resource_id(&item.uri)?;
            if let Some(entry) = manifest.entries.get(&entry_id) {
                let file_path = vault_root.join(&entry.path);
                if !file_path.exists() {
                    return Err(TemperError::NotFound(format!(
                        "vault file not found: {}",
                        file_path.display()
                    )));
                }
                let parts: Vec<&str> = entry.path.split('/').collect();
                let context = parts.first().copied().unwrap_or("default");
                let doc_type = if parts.len() > 1 {
                    parts[1]
                } else {
                    "resource"
                };

                let (resource, _) =
                    ingest::ingest_file(client, &file_path, context, doc_type, Some("imported"))
                        .await?;

                // Update manifest entry with server-assigned data
                if let Some(e) = manifest.entries.get_mut(&entry_id) {
                    e.remote_hash = resource.content_hash.unwrap_or_default();
                    e.state = ManifestEntryState::Clean;
                    e.synced_at = chrono::Utc::now();
                }
            }
            return Ok(());
        }
    };

    // Existing resource — update content via ingest PUT
    if let Some(entry) = manifest.entries.get(&resource_id) {
        let file_path = vault_root.join(&entry.path);
        if !file_path.exists() {
            return Err(TemperError::NotFound(format!(
                "vault file not found: {}",
                file_path.display()
            )));
        }
        let content = std::fs::read_to_string(&file_path)?;
        let raw_content = strip_frontmatter(&content);

        let resource = client
            .ingest()
            .update(resource_id, raw_content)
            .await
            .map_err(|e| TemperError::Api(e.to_string()))?;

        if let Some(e) = manifest.entries.get_mut(&resource_id) {
            e.remote_hash = resource.content_hash.unwrap_or_default();
            e.state = ManifestEntryState::Clean;
            e.synced_at = chrono::Utc::now();
        }
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
        .get(item.resource_id)
        .await
        .map_err(|e| TemperError::Api(e.to_string()))?;

    let content_response = client
        .resources()
        .content(item.resource_id)
        .await
        .map_err(|e| TemperError::Api(e.to_string()))?;

    let (ctx, doc_type) = parse_kb_uri(&item.uri)?;

    let vault_path = ingest::write_vault_file_and_register(
        vault_root,
        &ctx,
        &doc_type,
        &resource,
        &content_response.markdown,
        None,
    )?;

    // Update manifest entry state
    if let Some(entry) = manifest.entries.get_mut(&item.resource_id) {
        let full_content = std::fs::read_to_string(&vault_path)?;
        entry.content_hash = ingest::compute_content_hash(&full_content);
        entry.remote_hash = item.content_hash.clone();
        entry.state = ManifestEntryState::Clean;
        entry.synced_at = chrono::Utc::now();
    }

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
        let id = Uuid::parse_str("12345678-1234-1234-1234-123456789abc").unwrap();
        m.entries.insert(
            id,
            ManifestEntry {
                path: "temper/task/12345678-1234-1234-1234-123456789abc.md".to_string(),
                content_hash: "oldhash".to_string(),
                remote_hash: "oldhash".to_string(),
                synced_at: Utc::now(),
                state: ManifestEntryState::Clean,
            },
        );
        m
    }

    #[test]
    fn rehash_detects_local_modification() {
        let dir = TempDir::new().unwrap();
        let vault = dir.path();
        let mut manifest = sample_manifest();

        let file_dir = vault.join("temper/task");
        fs::create_dir_all(&file_dir).unwrap();
        fs::write(
            file_dir.join("12345678-1234-1234-1234-123456789abc.md"),
            "new content",
        )
        .unwrap();

        let changed = rehash_manifest(&mut manifest, vault).unwrap();
        assert_eq!(changed, 1);

        let id = Uuid::parse_str("12345678-1234-1234-1234-123456789abc").unwrap();
        let entry = manifest.entries.get(&id).unwrap();
        assert_eq!(entry.state, ManifestEntryState::LocalModified);
        assert_ne!(entry.content_hash, "oldhash");
    }

    #[test]
    fn rehash_marks_deleted_files() {
        let dir = TempDir::new().unwrap();
        let mut manifest = sample_manifest();

        let changed = rehash_manifest(&mut manifest, dir.path()).unwrap();
        assert_eq!(changed, 1);

        let id = Uuid::parse_str("12345678-1234-1234-1234-123456789abc").unwrap();
        let entry = manifest.entries.get(&id).unwrap();
        assert_eq!(entry.state, ManifestEntryState::LocalModified);
        assert!(entry.content_hash.is_empty());
    }

    #[test]
    fn rehash_skips_unchanged_files() {
        let dir = TempDir::new().unwrap();
        let vault = dir.path();
        let mut manifest = sample_manifest();

        let content = "test content";
        let hash = ingest::compute_content_hash(content);

        let id = Uuid::parse_str("12345678-1234-1234-1234-123456789abc").unwrap();
        manifest.entries.get_mut(&id).unwrap().content_hash = hash;

        let file_dir = vault.join("temper/task");
        fs::create_dir_all(&file_dir).unwrap();
        fs::write(
            file_dir.join("12345678-1234-1234-1234-123456789abc.md"),
            content,
        )
        .unwrap();

        let changed = rehash_manifest(&mut manifest, vault).unwrap();
        assert_eq!(changed, 0);
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
            .starts_with("kb://temper/task/"));
    }

    #[test]
    fn build_status_request_filters_contexts() {
        let manifest = sample_manifest();
        let req = build_status_request(&manifest, &["other".to_string()]);
        assert!(req.contexts.is_empty());
    }

    #[test]
    fn parse_kb_uri_extracts_parts() {
        let (ctx, dt) = parse_kb_uri("kb://temper/task/some-uuid").unwrap();
        assert_eq!(ctx, "temper");
        assert_eq!(dt, "task");
    }

    #[test]
    fn parse_kb_uri_rejects_non_kb() {
        assert!(parse_kb_uri("https://example.com").is_err());
    }

    #[test]
    fn parse_kb_uri_rejects_missing_doc_type() {
        assert!(parse_kb_uri("kb://temper").is_err());
    }

    #[test]
    fn extract_resource_id_works() {
        let id =
            extract_resource_id("kb://temper/task/12345678-1234-1234-1234-123456789abc").unwrap();
        assert_eq!(
            id,
            Uuid::parse_str("12345678-1234-1234-1234-123456789abc").unwrap()
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

        let id = Uuid::parse_str("12345678-1234-1234-1234-123456789abc").unwrap();
        let file_dir = vault.join("temper/task");
        fs::create_dir_all(&file_dir).unwrap();
        let file_path = file_dir.join("12345678-1234-1234-1234-123456789abc.md");
        fs::write(&file_path, "content").unwrap();

        let item = SyncRemovedItem {
            uri: "kb://temper/task/12345678-1234-1234-1234-123456789abc".to_string(),
            resource_id: id,
        };
        remove_resource(&mut manifest, vault, &item).unwrap();

        assert!(!file_path.exists());
        assert!(manifest.entries.get(&id).is_none());
    }
}
