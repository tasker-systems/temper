//! `temper import` — import a file into the vault (managed, frontmatter, sync-ready).
//!
//! Two modes:
//! 1. **File import**: extract file, upload to cloud, write vault file with frontmatter,
//!    register in manifest.
//! 2. **Promotion**: given a resource UUID (previously added), fetch from cloud, write
//!    vault file, register in manifest.

use std::path::PathBuf;

use uuid::Uuid;

use crate::actions::ingest;
use crate::error::TemperError;
use crate::format::OutputFormat;
use crate::output;

// Re-exports for backward compat (used by pull.rs).
pub use ingest::{build_frontmatter, build_vault_path};

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
    // Check if path is a UUID -> promotion flow
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

    if !file_path.exists() {
        return Err(TemperError::Config(format!(
            "file not found: {}",
            file_path.display()
        )));
    }

    let fmt = OutputFormat::parse(format);

    if fmt == OutputFormat::Text {
        output::progress("  Extracting... ");
    }

    let rt = tokio::runtime::Runtime::new()
        .map_err(|e| TemperError::Api(format!("tokio runtime: {e}")))?;

    let (resource, extracted_content) = rt.block_on(async {
        let client =
            temper_client::config::build_client().map_err(|e| TemperError::Api(e.to_string()))?;

        ingest::ingest_file(&client, &file_path, context, doc_type).await
    })?;

    if fmt == OutputFormat::Text {
        output::plain(format!(
            "done ({} KB markdown)",
            extracted_content.len() / 1024
        ));
    }

    // Write vault file and register in manifest.
    let vault_root = crate::config::resolve_vault(None)?;
    let canonical_path = std::fs::canonicalize(&file_path)
        .unwrap_or_else(|_| file_path.clone())
        .to_string_lossy()
        .to_string();

    let vault_path = ingest::write_vault_file_and_register(
        &vault_root,
        context,
        doc_type,
        &resource,
        &extracted_content,
        Some(&canonical_path),
    )?;

    match fmt {
        OutputFormat::Json => {
            let event = serde_json::json!({
                "event": "import",
                "file": path,
                "status": "done",
                "resource_id": resource.id,
                "vault_path": vault_path.display().to_string(),
            });
            output::plain(event);
        }
        OutputFormat::Text => {
            output::success(format!(
                "Imported: \"{}\" ({}) -> {}",
                resource.title,
                resource.id,
                vault_path.display()
            ));
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Promotion (UUID -> vault file)
// ---------------------------------------------------------------------------

fn promote_resource(
    resource_id: Uuid,
    context: Option<&str>,
    doc_type: &str,
    format: &str,
) -> crate::error::Result<()> {
    let fmt = OutputFormat::parse(format);

    let rt = tokio::runtime::Runtime::new()
        .map_err(|e| TemperError::Api(format!("tokio runtime: {e}")))?;

    let (resource, content_response) = rt.block_on(async {
        let client =
            temper_client::config::build_client().map_err(|e| TemperError::Api(e.to_string()))?;

        let resource = client
            .resources()
            .get(resource_id)
            .await
            .map_err(|e| TemperError::Api(e.to_string()))?;

        let content_response = client
            .resources()
            .content(resource_id)
            .await
            .map_err(|e| TemperError::Api(e.to_string()))?;

        Ok::<_, TemperError>((resource, content_response))
    })?;

    // Determine context: from flag or derive from resource URI
    let resolved_context = context
        .map(String::from)
        .or_else(|| ingest::derive_context_from_uri(&resource.origin_uri))
        .ok_or_else(|| {
            TemperError::Config(
                "--context is required when promoting a resource without a context in its URI"
                    .to_string(),
            )
        })?;

    let vault_root = crate::config::resolve_vault(None)?;

    let vault_path = ingest::write_vault_file_and_register(
        &vault_root,
        &resolved_context,
        doc_type,
        &resource,
        &content_response.markdown,
        None,
    )?;

    match fmt {
        OutputFormat::Json => {
            let event = serde_json::json!({
                "event": "promote",
                "resource_id": resource.id,
                "status": "done",
                "vault_path": vault_path.display().to_string(),
            });
            output::plain(event);
        }
        OutputFormat::Text => {
            output::success(format!(
                "Promoted: \"{}\" ({}) -> {}",
                resource.title,
                resource.id,
                vault_path.display()
            ));
        }
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
    use std::path::Path;

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

    let fmt = OutputFormat::parse(format);
    let json_mode = fmt == OutputFormat::Json;

    if files.is_empty() {
        if json_mode {
            let event = serde_json::json!({"event":"complete","added":0,"skipped":0,"failed":0});
            output::plain(event);
        } else {
            output::plain(format!("No matching files found in {path}"));
        }
        return Ok(());
    }

    // Single runtime for all files (fixes N-runtime issue).
    let rt = tokio::runtime::Runtime::new()
        .map_err(|e| TemperError::Api(format!("tokio runtime: {e}")))?;

    let mut added = 0u64;
    let mut failed = 0u64;

    let vault_root = crate::config::resolve_vault(None)?;

    rt.block_on(async {
        let client =
            temper_client::config::build_client().map_err(|e| TemperError::Api(e.to_string()))?;

        for file in &files {
            let file_name = file
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("unknown")
                .to_string();

            match ingest::ingest_file(&client, file, context, doc_type).await {
                Ok((resource, extracted_content)) => {
                    let canonical_path = std::fs::canonicalize(file)
                        .unwrap_or_else(|_| file.clone())
                        .to_string_lossy()
                        .to_string();

                    if let Err(e) = ingest::write_vault_file_and_register(
                        &vault_root,
                        context,
                        doc_type,
                        &resource,
                        &extracted_content,
                        Some(&canonical_path),
                    ) {
                        if json_mode {
                            let event = serde_json::json!({
                                "event": "error",
                                "file": file_name,
                                "error": e.to_string(),
                            });
                            output::plain(event);
                        } else {
                            output::error(format!("{file_name}: vault write failed -- {e}"));
                        }
                        failed += 1;
                    } else {
                        if json_mode {
                            let event = serde_json::json!({
                                "event": "import",
                                "file": file_name,
                                "status": "done",
                                "resource_id": resource.id,
                            });
                            output::plain(event);
                        } else {
                            output::success(file_name);
                        }
                        added += 1;
                    }
                }
                Err(err) => {
                    if json_mode {
                        let event = serde_json::json!({
                            "event": "error",
                            "file": file_name,
                            "error": err.to_string(),
                        });
                        output::plain(event);
                    } else {
                        output::error(format!("{file_name}: import failed -- {err}"));
                    }
                    failed += 1;
                }
            }
        }

        Ok::<_, TemperError>(())
    })?;

    if json_mode {
        let event = serde_json::json!({"event":"complete","added":added,"failed":failed});
        output::plain(event);
    } else {
        output::success(format!("{added} imported, {failed} failed"));
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

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

    // --- run() UUID detection integration ---

    #[test]
    fn run_with_uuid_path_without_vault_fails_gracefully() {
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
