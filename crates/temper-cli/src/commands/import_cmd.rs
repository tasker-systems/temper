//! `temper import` — import a file into the vault (managed, frontmatter, sync-ready).
//!
//! Two modes:
//! 1. **File import**: extract file, upload to cloud, write vault file with frontmatter,
//!    register in manifest.
//! 2. **Promotion**: given a resource UUID (previously added), fetch from cloud, write
//!    vault file, register in manifest.

use std::path::{Path, PathBuf};

use uuid::Uuid;

use crate::error::TemperError;

// ---------------------------------------------------------------------------
// Public helpers (reused by pull.rs)
// ---------------------------------------------------------------------------

/// Canonical vault path for an imported resource.
///
/// `{vault_root}/{context}/{doc_type}/{uuid}.md`
pub fn build_vault_path(vault_root: &Path, context: &str, doc_type: &str, id: Uuid) -> PathBuf {
    vault_root
        .join(context)
        .join(doc_type)
        .join(format!("{id}.md"))
}

/// Generate YAML frontmatter for an imported resource.
pub fn build_frontmatter(
    id: Uuid,
    title: &str,
    context: &str,
    doc_type: &str,
    ingestion_source: Option<&str>,
) -> String {
    let now = chrono::Utc::now().to_rfc3339();
    let mut fm = format!(
        "---\ntemper-id: {id}\ntitle: \"{title}\"\ncontext: {context}\ndoc_type: {doc_type}\n"
    );
    if let Some(source) = ingestion_source {
        fm.push_str(&format!("ingestion_source: \"{source}\"\n"));
    }
    fm.push_str(&format!("created: {now}\n---\n\n"));
    fm
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

pub fn run(
    path: &str,
    dir: bool,
    context: Option<&str>,
    doc_type: &str,
    format: &str,
    force: bool,
) -> crate::error::Result<()> {
    // Check if path is a UUID → promotion flow
    if let Ok(resource_id) = Uuid::parse_str(path) {
        return promote_resource(resource_id, context, doc_type, format);
    }

    // File/directory import requires --context
    let context = context
        .ok_or_else(|| TemperError::Config("--context is required for file imports".to_string()))?;

    if dir {
        return run_directory_import(path, context, doc_type, format, force);
    }

    run_single_import(path, context, doc_type, format)
}

// ---------------------------------------------------------------------------
// Single-file import
// ---------------------------------------------------------------------------

fn run_single_import(
    path: &str,
    context: &str,
    doc_type: &str,
    format: &str,
) -> crate::error::Result<()> {
    let file_path = PathBuf::from(path);

    // Verify the file exists.
    if !file_path.exists() {
        return Err(TemperError::Config(format!(
            "file not found: {}",
            file_path.display()
        )));
    }

    let json_mode = format == "json";

    // Step 1: Extract to markdown.
    if !json_mode {
        eprint!("  Extracting... ");
    }

    let extraction = crate::extract::extract_to_markdown(&file_path)?;
    let size_bytes = extraction.content.len();

    if json_mode {
        let event = serde_json::json!({
            "event": "extract",
            "file": path,
            "status": "done",
            "size_bytes": size_bytes,
        });
        println!("{event}");
    } else {
        println!("done ({} KB markdown)", size_bytes / 1024);
    }

    // Step 2: Build the IngestRequest.
    let title = file_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("Untitled")
        .to_string();

    let uri = format!(
        "kb://{context}/{doc_type}/{}",
        title.to_lowercase().replace(' ', "-")
    );

    let device_id = load_device_id();

    let canonical_path = std::fs::canonicalize(&file_path)
        .unwrap_or_else(|_| file_path.clone())
        .to_string_lossy()
        .to_string();

    let content_hash_pre = crate::commands::add::compute_content_hash(&extraction.content);

    let metadata = serde_json::json!({
        "device_id": device_id,
        "original_path": canonical_path,
        "content_hash": content_hash_pre,
    });

    let request = temper_core::types::IngestRequest {
        content: extraction.content.clone(),
        title: title.clone(),
        kb_context_id: Uuid::nil(),
        kb_doc_type_id: Uuid::nil(),
        uri,
        slug: None,
        mimetype: Some(extraction.mime_type),
        tags: None,
        metadata: Some(metadata),
        context_name: Some(context.to_string()),
        doc_type_name: Some(doc_type.to_string()),
    };

    // Step 3: Upload via the API.
    if !json_mode {
        eprint!("  Uploading... ");
    }

    let rt = tokio::runtime::Runtime::new()
        .map_err(|e| TemperError::Config(format!("tokio runtime: {e}")))?;

    let resource = rt.block_on(async {
        let client = temper_client::config::build_client()
            .map_err(|e| TemperError::Config(e.to_string()))?;

        client
            .ingest()
            .create(&request)
            .await
            .map_err(|e| TemperError::Config(e.to_string()))
    })?;

    if !json_mode {
        println!("done");
    }

    // Step 4: Resolve vault root and write vault file.
    let vault_root = crate::config::resolve_vault(None)?;
    let vault_path = build_vault_path(&vault_root, context, doc_type, resource.id);

    if let Some(parent) = vault_path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let frontmatter = build_frontmatter(
        resource.id,
        &title,
        context,
        doc_type,
        Some(&canonical_path),
    );
    let vault_content = format!("{frontmatter}{}", extraction.content);
    std::fs::write(&vault_path, &vault_content)?;

    // Step 5: Register in manifest.
    let temper_dir = vault_root.join(".temper");
    let device_id_str = load_device_id().unwrap_or_else(|| "unknown".to_string());
    let mut manifest = crate::manifest_io::load_manifest(&temper_dir, &device_id_str)?;

    let content_hash = crate::commands::add::compute_content_hash(&vault_content);
    let remote_hash = resource.content_hash.clone().unwrap_or_default();
    let rel_path = vault_path
        .strip_prefix(&vault_root)
        .unwrap_or(&vault_path)
        .to_string_lossy()
        .to_string();

    manifest.entries.insert(
        resource.id,
        temper_core::types::ManifestEntry {
            path: rel_path,
            content_hash,
            remote_hash,
            synced_at: chrono::Utc::now(),
            state: temper_core::types::ManifestEntryState::Clean,
        },
    );
    crate::manifest_io::save_manifest(&temper_dir, &manifest)?;

    // Step 6: Print result.
    if json_mode {
        let event = serde_json::json!({
            "event": "import",
            "file": path,
            "status": "done",
            "resource_id": resource.id,
            "vault_path": vault_path.display().to_string(),
        });
        println!("{event}");
    } else {
        println!(
            "\u{2713} Imported: {:?} ({}) → {}",
            title,
            resource.id,
            vault_path.display()
        );
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Promotion (UUID → vault file)
// ---------------------------------------------------------------------------

fn promote_resource(
    resource_id: Uuid,
    context: Option<&str>,
    doc_type: &str,
    format: &str,
) -> crate::error::Result<()> {
    let json_mode = format == "json";

    let rt = tokio::runtime::Runtime::new()
        .map_err(|e| TemperError::Config(format!("tokio runtime: {e}")))?;

    let (resource, content_response) = rt.block_on(async {
        let client = temper_client::config::build_client()
            .map_err(|e| TemperError::Config(e.to_string()))?;

        let resource = client
            .resources()
            .get(resource_id)
            .await
            .map_err(|e| TemperError::Config(e.to_string()))?;

        let content_response = client
            .resources()
            .content(resource_id)
            .await
            .map_err(|e| TemperError::Config(e.to_string()))?;

        Ok::<_, TemperError>((resource, content_response))
    })?;

    // Determine context: from flag or derive from resource URI
    let resolved_context = context
        .map(String::from)
        .or_else(|| derive_context_from_uri(&resource.uri))
        .ok_or_else(|| {
            TemperError::Config(
                "--context is required when promoting a resource without a context in its URI"
                    .to_string(),
            )
        })?;

    let vault_root = crate::config::resolve_vault(None)?;
    let vault_path = build_vault_path(&vault_root, &resolved_context, doc_type, resource.id);

    if let Some(parent) = vault_path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let frontmatter = build_frontmatter(
        resource.id,
        &resource.title,
        &resolved_context,
        doc_type,
        None,
    );
    let vault_content = format!("{frontmatter}{}", content_response.markdown);
    std::fs::write(&vault_path, &vault_content)?;

    // Register in manifest.
    let temper_dir = vault_root.join(".temper");
    let device_id_str = load_device_id().unwrap_or_else(|| "unknown".to_string());
    let mut manifest = crate::manifest_io::load_manifest(&temper_dir, &device_id_str)?;

    let content_hash = crate::commands::add::compute_content_hash(&vault_content);
    let remote_hash = resource.content_hash.clone().unwrap_or_default();
    let rel_path = vault_path
        .strip_prefix(&vault_root)
        .unwrap_or(&vault_path)
        .to_string_lossy()
        .to_string();

    manifest.entries.insert(
        resource.id,
        temper_core::types::ManifestEntry {
            path: rel_path,
            content_hash,
            remote_hash,
            synced_at: chrono::Utc::now(),
            state: temper_core::types::ManifestEntryState::Clean,
        },
    );
    crate::manifest_io::save_manifest(&temper_dir, &manifest)?;

    // Print result.
    if json_mode {
        let event = serde_json::json!({
            "event": "promote",
            "resource_id": resource.id,
            "status": "done",
            "vault_path": vault_path.display().to_string(),
        });
        println!("{event}");
    } else {
        println!(
            "\u{2713} Promoted: {:?} ({}) → {}",
            resource.title,
            resource.id,
            vault_path.display()
        );
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Directory import
// ---------------------------------------------------------------------------

fn run_directory_import(
    path: &str,
    context: &str,
    doc_type: &str,
    format: &str,
    force: bool,
) -> crate::error::Result<()> {
    let dir = Path::new(path);
    if !dir.is_dir() {
        return Err(TemperError::Config(format!("not a directory: {path}")));
    }

    let config = crate::commands::add::DirectoryConfig::default();
    let files = if force {
        crate::commands::add::collect_files(dir, &config)?
    } else {
        crate::commands::add::preflight_check(dir, &config)?
    };

    if files.is_empty() {
        if format == "json" {
            let event = serde_json::json!({"event":"complete","added":0,"skipped":0,"failed":0});
            println!("{event}");
        } else {
            println!("No matching files found in {path}");
        }
        return Ok(());
    }

    let json_mode = format == "json";
    let mut added = 0u64;
    let mut failed = 0u64;

    for file in &files {
        match run_single_import(&file.to_string_lossy(), context, doc_type, format) {
            Ok(()) => {
                added += 1;
            }
            Err(err) => {
                if json_mode {
                    let event = serde_json::json!({
                        "event": "error",
                        "file": file.display().to_string(),
                        "error": err.to_string(),
                    });
                    println!("{event}");
                } else {
                    eprintln!("  \u{2717} {}: import failed — {err}", file.display());
                }
                failed += 1;
            }
        }
    }

    if json_mode {
        let event = serde_json::json!({"event":"complete","added":added,"failed":failed});
        println!("{event}");
    } else {
        println!("\u{2713} {added} imported, {failed} failed");
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Load the device UUID string from `~/.config/temper/device.json`.
fn load_device_id() -> Option<String> {
    let path = dirs::home_dir()?
        .join(".config")
        .join("temper")
        .join("device.json");
    let content = std::fs::read_to_string(path).ok()?;
    let val: serde_json::Value = serde_json::from_str(&content).ok()?;
    val.get("client_id")?.as_str().map(String::from)
}

/// Attempt to extract a context name from a `kb://{context}/...` URI.
fn derive_context_from_uri(uri: &str) -> Option<String> {
    let rest = uri.strip_prefix("kb://")?;
    let segment = rest.split('/').next()?;
    if segment.is_empty() {
        None
    } else {
        Some(segment.to_string())
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // --- build_vault_path ---

    #[test]
    fn build_vault_path_produces_correct_path() {
        let root = Path::new("/vault");
        let id = Uuid::nil();
        let path = build_vault_path(root, "work", "note", id);
        assert_eq!(
            path,
            PathBuf::from("/vault/work/note/00000000-0000-0000-0000-000000000000.md")
        );
    }

    #[test]
    fn build_vault_path_nested_context() {
        let root = Path::new("/home/user/kb");
        let id = Uuid::parse_str("12345678-1234-1234-1234-123456789abc").unwrap();
        let path = build_vault_path(root, "personal", "resource", id);
        assert_eq!(
            path,
            PathBuf::from(
                "/home/user/kb/personal/resource/12345678-1234-1234-1234-123456789abc.md"
            )
        );
    }

    // --- build_frontmatter ---

    #[test]
    fn build_frontmatter_includes_required_fields() {
        let id = Uuid::nil();
        let fm = build_frontmatter(id, "My Title", "work", "note", None);
        assert!(fm.contains("temper-id:"));
        assert!(fm.contains("title: \"My Title\""));
        assert!(fm.contains("context: work"));
        assert!(fm.contains("doc_type: note"));
        assert!(fm.contains("created:"));
        assert!(fm.starts_with("---\n"));
        assert!(fm.contains("\n---\n"));
    }

    #[test]
    fn build_frontmatter_includes_ingestion_source_when_provided() {
        let id = Uuid::nil();
        let fm = build_frontmatter(id, "My Title", "work", "note", Some("/home/user/file.pdf"));
        assert!(
            fm.contains("ingestion_source: \"/home/user/file.pdf\""),
            "expected ingestion_source in frontmatter:\n{fm}"
        );
    }

    #[test]
    fn build_frontmatter_omits_ingestion_source_when_absent() {
        let id = Uuid::nil();
        let fm = build_frontmatter(id, "My Title", "work", "note", None);
        assert!(
            !fm.contains("ingestion_source"),
            "unexpected ingestion_source in frontmatter:\n{fm}"
        );
    }

    // --- UUID detection ---

    #[test]
    fn uuid_path_detected_as_uuid() {
        let uuid_str = "12345678-1234-1234-1234-123456789abc";
        assert!(
            Uuid::parse_str(uuid_str).is_ok(),
            "should parse as UUID: {uuid_str}"
        );
    }

    #[test]
    fn file_path_not_detected_as_uuid() {
        let file_path = "/home/user/documents/my-notes.pdf";
        assert!(
            Uuid::parse_str(file_path).is_err(),
            "file path should not parse as UUID: {file_path}"
        );
    }

    #[test]
    fn relative_file_path_not_detected_as_uuid() {
        let file_path = "notes/my-document.md";
        assert!(
            Uuid::parse_str(file_path).is_err(),
            "relative path should not parse as UUID: {file_path}"
        );
    }

    // --- derive_context_from_uri ---

    #[test]
    fn derive_context_extracts_from_kb_uri() {
        let ctx = derive_context_from_uri("kb://work/note/my-doc");
        assert_eq!(ctx, Some("work".to_string()));
    }

    #[test]
    fn derive_context_returns_none_for_non_kb_uri() {
        let ctx = derive_context_from_uri("https://example.com/doc");
        assert_eq!(ctx, None);
    }

    #[test]
    fn derive_context_returns_none_for_empty_context() {
        let ctx = derive_context_from_uri("kb:///note/my-doc");
        assert_eq!(ctx, None);
    }

    // --- run() UUID detection integration ---

    #[test]
    fn run_with_uuid_path_without_vault_fails_gracefully() {
        // With no vault set and no auth, the promote path should fail at vault
        // resolution or auth — not panic. We just verify it returns an Err.
        let result = run(
            "12345678-1234-1234-1234-123456789abc",
            false,
            None,
            "resource",
            "text",
            false,
        );
        assert!(result.is_err());
    }

    #[test]
    fn run_file_without_context_returns_error() {
        let result = run("/tmp/some-file.md", false, None, "resource", "text", false);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("--context is required"));
    }
}
