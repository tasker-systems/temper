//! `temper import` — import a file into the vault (managed, frontmatter, sync-ready).
//!
//! Three flows:
//! 1. **File import**: extract file, upload to cloud, write vault file with frontmatter,
//!    register in manifest.
//! 2. **Promotion**: given a resource UUID (previously added), fetch from cloud, write
//!    vault file, register in manifest.
//! 3. **Frontmatter-aware import** (`--doc-type auto`): read YAML frontmatter from each
//!    file to derive doc_type, title, context, and slug automatically.

use std::path::PathBuf;

use uuid::Uuid;

use crate::actions::{ingest, runtime};
use crate::error::TemperError;
use crate::format::OutputFormat;
use crate::output;

// Re-exports for backward compat (used by pull.rs).
pub use ingest::{build_frontmatter, build_vault_path};

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

#[expect(
    clippy::too_many_arguments,
    reason = "thin CLI entry point — arguments map 1:1 to clap flags"
)]
pub fn run(
    path: &str,
    dir: bool,
    context: Option<&str>,
    doc_type: &str,
    format: &str,
    force: bool,
    dry_run: bool,
    ignore: Option<&str>,
) -> crate::error::Result<()> {
    // Compile --ignore pattern up front so we fail fast on bad regex.
    let ignore_re = ignore
        .map(|pat| {
            regex::Regex::new(pat)
                .map_err(|e| TemperError::Config(format!("invalid --ignore pattern: {e}")))
        })
        .transpose()?;

    // Check if path is a UUID -> promotion flow
    if let Ok(resource_id) = Uuid::parse_str(path) {
        if dry_run {
            output::plain(format!("dry-run: would promote resource {resource_id}"));
            return Ok(());
        }
        return promote_resource(resource_id, context, doc_type, format);
    }

    // File/directory import: --context is required unless --doc-type auto
    // (auto can derive context from frontmatter).
    let is_auto = doc_type == "auto";
    if !is_auto && context.is_none() {
        return Err(TemperError::Config(
            "--context is required for file imports (or use --doc-type auto)".to_string(),
        ));
    }

    if dir {
        return run_directory_import(
            path,
            context,
            doc_type,
            format,
            force,
            dry_run,
            ignore_re.as_ref(),
        );
    }

    run_single_import(path, context, doc_type, format, dry_run)
}

// ---------------------------------------------------------------------------
// Single-file import
// ---------------------------------------------------------------------------

fn run_single_import(
    path: &str,
    context: Option<&str>,
    doc_type: &str,
    format: &str,
    dry_run: bool,
) -> crate::error::Result<()> {
    let file_path = PathBuf::from(path);

    if !file_path.exists() {
        return Err(TemperError::Config(format!(
            "file not found: {}",
            file_path.display()
        )));
    }

    let fmt = OutputFormat::parse(format);
    let is_auto = doc_type == "auto";

    if is_auto {
        return run_single_auto_import(&file_path, context, fmt, dry_run);
    }

    // Non-auto: context is guaranteed present by caller.
    let context =
        context.ok_or_else(|| TemperError::Config("--context is required".to_string()))?;

    if dry_run {
        let title = ingest::title_from_path(&file_path);
        let slug = ingest::slug_from_title(&title);
        output::plain(format!(
            "[{doc_type}] {slug} → {context}/{doc_type}/{slug}.md"
        ));
        return Ok(());
    }

    if fmt == OutputFormat::Text {
        output::progress("  Extracting... ");
    }

    let (rt, client) = runtime::build_runtime_and_client()?;
    rt.block_on(runtime::ensure_profile(&client))?;

    let (resource, extracted_content) = rt.block_on(async {
        ingest::ingest_file(&client, &file_path, context, doc_type, Some("imported")).await
    })?;

    if fmt == OutputFormat::Text {
        output::plain(format!(
            "done ({} KB markdown)",
            extracted_content.len() / 1024
        ));
    }

    let vault_root = crate::config::resolve_vault(None)?;
    let slug = ingest::slug_from_title(&resource.title);
    let slug = ingest::dedup_vault_slug(&vault_root, context, doc_type, &slug);
    let canonical_path = std::fs::canonicalize(&file_path)
        .unwrap_or_else(|_| file_path.clone())
        .to_string_lossy()
        .to_string();

    let vault_path = ingest::write_vault_file_and_register(
        &vault_root,
        context,
        doc_type,
        &slug,
        &resource,
        &extracted_content,
        Some(&canonical_path),
        None,
    )?;

    emit_import_event(fmt, path, &resource, &vault_path);
    Ok(())
}

// ---------------------------------------------------------------------------
// Single-file auto import (frontmatter-aware)
// ---------------------------------------------------------------------------

fn run_single_auto_import(
    file_path: &PathBuf,
    context_override: Option<&str>,
    fmt: OutputFormat,
    dry_run: bool,
) -> crate::error::Result<()> {
    let raw_content = std::fs::read_to_string(file_path)?;
    let parsed = ingest::parse_source_frontmatter(&raw_content);
    let body = ingest::strip_frontmatter(&raw_content);

    let title = parsed
        .as_ref()
        .and_then(|fm| fm.title.clone())
        .unwrap_or_else(|| ingest::title_from_path(file_path));
    let doc_type = parsed
        .as_ref()
        .and_then(|fm| fm.doc_type.clone())
        .unwrap_or_else(|| "resource".to_string());
    let context = context_override
        .map(String::from)
        .or_else(|| parsed.as_ref().and_then(|fm| fm.context.clone()))
        .ok_or_else(|| {
            TemperError::Config(format!(
                "--context is required (no context in frontmatter of {})",
                file_path.display()
            ))
        })?;
    let slug = parsed
        .as_ref()
        .and_then(|fm| fm.slug.clone())
        .unwrap_or_else(|| ingest::slug_from_title(&title));

    if dry_run {
        output::plain(format!(
            "[{doc_type}] {slug} → {context}/{doc_type}/{slug}.md"
        ));
        return Ok(());
    }

    if fmt == OutputFormat::Text {
        output::progress("  Extracting... ");
    }

    let (rt, client) = runtime::build_runtime_and_client()?;
    rt.block_on(runtime::ensure_profile(&client))?;

    let device_id = crate::config::load_device_id();
    let canonical_path = std::fs::canonicalize(file_path)
        .unwrap_or_else(|_| file_path.to_path_buf())
        .to_string_lossy()
        .to_string();

    let mut metadata = serde_json::json!({
        "device_id": device_id,
        "original_path": canonical_path,
    });
    // Preserve legacy metadata fields.
    if let Some(ref fm) = parsed {
        if let Some(ref id) = fm.legacy_id {
            metadata["legacy_id"] = serde_json::Value::String(id.clone());
        }
    }

    let payload = ingest::build_ingest_payload(
        body,
        &title,
        &context,
        &doc_type,
        "imported",
        "text/markdown",
        Some(metadata),
    )?;

    let resource = rt.block_on(async {
        client
            .ingest()
            .create(&payload)
            .await
            .map_err(|e| TemperError::Api(e.to_string()))
    })?;

    if fmt == OutputFormat::Text {
        output::plain(format!("done ({} KB markdown)", body.len() / 1024));
    }

    let vault_root = crate::config::resolve_vault(None)?;
    let slug = ingest::dedup_vault_slug(&vault_root, &context, &doc_type, &slug);

    // Build extra frontmatter fields from legacy metadata.
    let extra_fields = build_extra_fields(parsed.as_ref());
    let extra_refs: Vec<(&str, &str)> = extra_fields
        .iter()
        .map(|(k, v)| (k.as_str(), v.as_str()))
        .collect();

    let vault_path = ingest::write_vault_file_and_register(
        &vault_root,
        &context,
        &doc_type,
        &slug,
        &resource,
        body,
        Some(&canonical_path),
        if extra_refs.is_empty() {
            None
        } else {
            Some(&extra_refs)
        },
    )?;

    emit_import_event(fmt, &file_path.to_string_lossy(), &resource, &vault_path);
    Ok(())
}

/// Collect extra frontmatter fields from parsed legacy frontmatter.
fn build_extra_fields(parsed: Option<&ingest::ParsedFrontmatter>) -> Vec<(String, String)> {
    let mut fields = Vec::new();
    if let Some(fm) = parsed {
        if let Some(ref v) = fm.legacy_id {
            fields.push(("legacy_id".to_string(), v.clone()));
        }
        if let Some(ref v) = fm.goal {
            fields.push(("goal".to_string(), v.clone()));
        }
        if let Some(ref v) = fm.stage {
            fields.push(("stage".to_string(), v.clone()));
        }
        if let Some(ref v) = fm.mode {
            fields.push(("mode".to_string(), v.clone()));
        }
        if let Some(ref v) = fm.effort {
            fields.push(("effort".to_string(), v.clone()));
        }
        if let Some(ref v) = fm.status {
            fields.push(("status".to_string(), v.clone()));
        }
    }
    fields
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

    let (rt, client) = runtime::build_runtime_and_client()?;

    let (resource, content_response) = rt.block_on(async {
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
    let slug = ingest::slug_from_title(&resource.title);
    let slug = ingest::dedup_vault_slug(&vault_root, &resolved_context, doc_type, &slug);

    let vault_path = ingest::write_vault_file_and_register(
        &vault_root,
        &resolved_context,
        doc_type,
        &slug,
        &resource,
        &content_response.markdown,
        None,
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
    context: Option<&str>,
    doc_type: &str,
    format: &str,
    force: bool,
    dry_run: bool,
    ignore_re: Option<&regex::Regex>,
) -> crate::error::Result<()> {
    use std::collections::HashMap;
    use std::path::Path;

    let dir = Path::new(path);
    if !dir.is_dir() {
        return Err(TemperError::Config(format!("not a directory: {path}")));
    }

    let config = crate::commands::add::DirectoryConfig::default();
    let all_files = if force {
        crate::commands::add::collect_files(dir, &config)?
    } else {
        crate::commands::add::preflight_check(dir, &config)?
    };

    // Apply --ignore filter against filenames.
    let files: Vec<_> = all_files
        .into_iter()
        .filter(|f| {
            let name = f.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if let Some(re) = ignore_re {
                !re.is_match(name)
            } else {
                true
            }
        })
        .collect();

    let fmt = OutputFormat::parse(format);
    let json_mode = fmt == OutputFormat::Json;
    let is_auto = doc_type == "auto";

    if files.is_empty() {
        if json_mode {
            let event = serde_json::json!({"event":"complete","added":0,"skipped":0,"failed":0});
            output::plain(event);
        } else {
            output::plain(format!("No matching files found in {path}"));
        }
        return Ok(());
    }

    // Dry-run: just resolve and print each file.
    if dry_run {
        let mut type_counts: HashMap<String, u64> = HashMap::new();
        let mut skipped = 0u64;
        for file in &files {
            if is_auto {
                match resolve_auto_fields(file, context) {
                    Some((resolved_title, resolved_doc_type, resolved_context, resolved_slug)) => {
                        *type_counts.entry(resolved_doc_type.clone()).or_default() += 1;
                        output::plain(format!(
                            "[{resolved_doc_type}] {resolved_slug} → {resolved_context}/{resolved_doc_type}/{resolved_slug}.md  (title: \"{resolved_title}\")"
                        ));
                    }
                    None => {
                        let name = file.file_name().and_then(|n| n.to_str()).unwrap_or("?");
                        output::warning(format!("skipped (no usable frontmatter): {name}"));
                        skipped += 1;
                    }
                }
            } else {
                let ctx = context
                    .ok_or_else(|| TemperError::Config("--context is required".to_string()))?;
                let title = ingest::title_from_path(file);
                let slug = ingest::slug_from_title(&title);
                *type_counts.entry(doc_type.to_string()).or_default() += 1;
                output::plain(format!(
                    "[{doc_type}] {slug} → {ctx}/{doc_type}/{slug}.md  (title: \"{title}\")"
                ));
            }
        }

        // Summary
        output::plain(String::new());
        let summary: Vec<String> = type_counts
            .iter()
            .map(|(k, v)| format!("{v} {k}"))
            .collect();
        let skip_msg = if skipped > 0 {
            format!(", {skipped} skipped")
        } else {
            String::new()
        };
        output::success(format!(
            "dry-run: {} files ({}){skip_msg}",
            files.len() - skipped as usize,
            summary.join(", ")
        ));
        return Ok(());
    }

    let mut added = 0u64;
    let mut failed = 0u64;
    let mut skipped = 0u64;
    let mut type_counts: HashMap<String, u64> = HashMap::new();

    let vault_root = crate::config::resolve_vault(None)?;

    if is_auto {
        let (rt, client) = runtime::build_runtime_and_client()?;
        rt.block_on(runtime::ensure_profile(&client))?;

        rt.block_on(async {
            for file in &files {
                let file_name = file
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("unknown")
                    .to_string();

                match import_single_auto_file(&client, file, context, &vault_root).await {
                    Ok(Some((doc_type_resolved, _vault_path))) => {
                        *type_counts.entry(doc_type_resolved).or_default() += 1;
                        if !json_mode {
                            output::success(&file_name);
                        }
                        added += 1;
                    }
                    Ok(None) => {
                        // Skipped — no usable frontmatter.
                        if json_mode {
                            let event = serde_json::json!({
                                "event": "skip",
                                "file": file_name,
                                "reason": "no usable frontmatter",
                            });
                            output::plain(event);
                        } else {
                            output::warning(format!(
                                "skipped (no usable frontmatter): {file_name}"
                            ));
                        }
                        skipped += 1;
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
                            output::error(format!("{file_name}: {err}"));
                        }
                        failed += 1;
                    }
                }
            }
            Ok::<_, TemperError>(())
        })?;
    } else {
        let context =
            context.ok_or_else(|| TemperError::Config("--context is required".to_string()))?;

        let (rt, client) = runtime::build_runtime_and_client()?;
        rt.block_on(runtime::ensure_profile(&client))?;

        rt.block_on(async {
            for file in &files {
                let file_name = file
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("unknown")
                    .to_string();

                match ingest::ingest_file(&client, file, context, doc_type, Some("imported")).await
                {
                    Ok((resource, extracted_content)) => {
                        let slug = ingest::slug_from_title(&resource.title);
                        let slug = ingest::dedup_vault_slug(&vault_root, context, doc_type, &slug);
                        let canonical_path = std::fs::canonicalize(file)
                            .unwrap_or_else(|_| file.clone())
                            .to_string_lossy()
                            .to_string();

                        if let Err(e) = ingest::write_vault_file_and_register(
                            &vault_root,
                            context,
                            doc_type,
                            &slug,
                            &resource,
                            &extracted_content,
                            Some(&canonical_path),
                            None,
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
                                output::success(&file_name);
                            }
                            *type_counts.entry(doc_type.to_string()).or_default() += 1;
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
    }

    if json_mode {
        let event = serde_json::json!({
            "event": "complete",
            "added": added,
            "skipped": skipped,
            "failed": failed,
            "by_type": type_counts,
        });
        output::plain(event);
    } else {
        let summary: Vec<String> = type_counts
            .iter()
            .map(|(k, v)| format!("{v} {k}"))
            .collect();
        let detail = if summary.is_empty() {
            String::new()
        } else {
            format!(" ({})", summary.join(", "))
        };
        let skip_msg = if skipped > 0 {
            format!(", {skipped} skipped")
        } else {
            String::new()
        };
        output::success(format!(
            "{added} imported, {failed} failed{skip_msg}{detail}"
        ));
    }

    Ok(())
}

/// Resolve title, doc_type, context, and slug from a file's frontmatter.
///
/// Returns `None` when the file has no usable frontmatter and no context
/// override was provided — the caller should skip the file with a warning
/// rather than aborting the entire batch.
fn resolve_auto_fields(
    file: &std::path::Path,
    context_override: Option<&str>,
) -> Option<(String, String, String, String)> {
    let raw = std::fs::read_to_string(file).ok()?;
    let parsed = ingest::parse_source_frontmatter(&raw);

    let title = parsed
        .as_ref()
        .and_then(|fm| fm.title.clone())
        .unwrap_or_else(|| ingest::title_from_path(file));
    let doc_type = parsed
        .as_ref()
        .and_then(|fm| fm.doc_type.clone())
        .unwrap_or_else(|| "resource".to_string());
    let context = context_override
        .map(String::from)
        .or_else(|| parsed.as_ref().and_then(|fm| fm.context.clone()))?;
    let slug = parsed
        .as_ref()
        .and_then(|fm| fm.slug.clone())
        .unwrap_or_else(|| ingest::slug_from_title(&title));

    Some((title, doc_type, context, slug))
}

/// Import a single file with frontmatter-aware resolution (used in directory auto mode).
///
/// Returns `Ok(None)` when the file should be skipped (no usable frontmatter
/// and no context override).  The caller emits a warning rather than failing
/// the entire batch.
#[cfg(feature = "embed")]
async fn import_single_auto_file(
    client: &temper_client::TemperClient,
    file: &std::path::Path,
    context_override: Option<&str>,
    vault_root: &std::path::Path,
) -> crate::error::Result<Option<(String, PathBuf)>> {
    let raw = std::fs::read_to_string(file)?;
    let parsed = ingest::parse_source_frontmatter(&raw);
    let body = ingest::strip_frontmatter(&raw);

    let title = parsed
        .as_ref()
        .and_then(|fm| fm.title.clone())
        .unwrap_or_else(|| ingest::title_from_path(file));
    let doc_type = parsed
        .as_ref()
        .and_then(|fm| fm.doc_type.clone())
        .unwrap_or_else(|| "resource".to_string());
    let context = match context_override
        .map(String::from)
        .or_else(|| parsed.as_ref().and_then(|fm| fm.context.clone()))
    {
        Some(ctx) => ctx,
        None => return Ok(None), // skip — no context available
    };
    let slug = parsed
        .as_ref()
        .and_then(|fm| fm.slug.clone())
        .unwrap_or_else(|| ingest::slug_from_title(&title));

    let canonical_path = std::fs::canonicalize(file)
        .unwrap_or_else(|_| file.to_path_buf())
        .to_string_lossy()
        .to_string();

    let device_id = crate::config::load_device_id();
    let mut metadata = serde_json::json!({
        "device_id": device_id,
        "original_path": canonical_path,
    });
    if let Some(ref fm) = parsed {
        if let Some(ref id) = fm.legacy_id {
            metadata["legacy_id"] = serde_json::Value::String(id.clone());
        }
    }

    let payload = ingest::build_ingest_payload(
        body,
        &title,
        &context,
        &doc_type,
        "imported",
        "text/markdown",
        Some(metadata),
    )?;

    let resource = client
        .ingest()
        .create(&payload)
        .await
        .map_err(|e| TemperError::Api(e.to_string()))?;

    let slug = ingest::dedup_vault_slug(vault_root, &context, &doc_type, &slug);

    let extra_fields = build_extra_fields(parsed.as_ref());
    let extra_refs: Vec<(&str, &str)> = extra_fields
        .iter()
        .map(|(k, v)| (k.as_str(), v.as_str()))
        .collect();

    let vault_path = ingest::write_vault_file_and_register(
        vault_root,
        &context,
        &doc_type,
        &slug,
        &resource,
        body,
        Some(&canonical_path),
        if extra_refs.is_empty() {
            None
        } else {
            Some(&extra_refs)
        },
    )?;

    Ok(Some((doc_type, vault_path)))
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn emit_import_event(
    fmt: OutputFormat,
    path: &str,
    resource: &temper_core::types::ResourceRow,
    vault_path: &std::path::Path,
) {
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
            false,
            None,
        );
        assert!(result.is_err());
    }

    #[test]
    fn run_file_without_context_returns_error() {
        let result = run(
            "/tmp/some-file.md",
            false,
            None,
            "resource",
            "text",
            false,
            false,
            None,
        );
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("--context is required"));
    }

    #[test]
    fn run_auto_without_context_does_not_require_context_upfront() {
        // auto mode defers context check to per-file frontmatter parsing.
        // This should fail at a later stage (file not found), not at context validation.
        let result = run(
            "/tmp/nonexistent.md",
            false,
            None,
            "auto",
            "text",
            false,
            false,
            None,
        );
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            !err.contains("--context is required"),
            "auto mode should not require --context upfront: {err}"
        );
    }
}
