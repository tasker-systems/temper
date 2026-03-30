//! `temper add` — ingest a single file or directory into the knowledge base.

use std::io::IsTerminal;
use std::path::Path;

use crate::actions::ingest;
use crate::format::OutputFormat;
use crate::output;

// Re-export for backward compat (used by directory config, tests, etc.)
pub use ingest::compute_content_hash;
pub use ingest::title_from_path;

/// Entry point for `temper add <path>`.
///
/// When `dir` is true the path is treated as a directory and forwarded to
/// [`run_directory`].  Otherwise a single-file ingest is performed.
pub fn run(
    path: &str,
    dir: bool,
    context: &str,
    doc_type: &str,
    format: &str,
    force: bool,
) -> crate::error::Result<()> {
    if path.starts_with("http://") || path.starts_with("https://") {
        return Err(crate::error::TemperError::Config(
            "URL support not yet implemented. Please provide a file path.".to_string(),
        ));
    }

    if dir {
        return run_directory(path, context, doc_type, format, force);
    }

    run_single_file(path, context, doc_type, format)
}

// ---------------------------------------------------------------------------
// Single-file ingest
// ---------------------------------------------------------------------------

fn run_single_file(
    path: &str,
    context: &str,
    doc_type: &str,
    format: &str,
) -> crate::error::Result<()> {
    let file_path = std::path::PathBuf::from(path);

    if !file_path.exists() {
        return Err(crate::error::TemperError::Config(format!(
            "file not found: {}",
            file_path.display()
        )));
    }

    let fmt = OutputFormat::parse(format);
    let file_name = file_path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or(path)
        .to_string();

    // Step 1: Extract + upload via shared action.
    if fmt == OutputFormat::Text {
        output::progress("  Extracting... ");
    }

    let rt = tokio::runtime::Runtime::new()
        .map_err(|e| crate::error::TemperError::Api(format!("tokio runtime: {e}")))?;

    let (resource, extraction_content) = rt.block_on(async {
        let client = temper_client::config::build_client()
            .map_err(|e| crate::error::TemperError::Api(e.to_string()))?;

        ingest::ingest_file(&client, &file_path, context, doc_type).await
    })?;

    // Step 2: Print result.
    match fmt {
        OutputFormat::Json => {
            let event = serde_json::json!({
                "event": "upload",
                "file": file_name,
                "status": "done",
                "resource_id": resource.id,
                "size_bytes": extraction_content.len(),
            });
            output::plain(event);
        }
        OutputFormat::Text => {
            output::plain(format!(
                "done ({} KB markdown)",
                extraction_content.len() / 1024
            ));
            output::success(format!("Added: \"{}\" ({})", resource.title, resource.id));
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Directory mode
// ---------------------------------------------------------------------------

/// Configuration for directory-mode ingestion.
pub struct DirectoryConfig {
    /// Maximum directory traversal depth (default: 2).
    pub max_depth: usize,
    /// Maximum total bytes to upload in one run (default: 50 MB).
    pub max_total_bytes: u64,
    /// Maximum number of concurrent uploads (default: 4).
    pub max_concurrent: usize,
    /// File extensions to include (without leading dot).
    pub allowed_extensions: Vec<String>,
}

impl Default for DirectoryConfig {
    fn default() -> Self {
        Self {
            max_depth: 2,
            max_total_bytes: 50 * 1024 * 1024,
            max_concurrent: 4,
            allowed_extensions: vec![
                "md", "markdown", "txt", "pdf", "docx", "doc", "html", "htm", "rst", "org", "tex",
                "rtf",
            ]
            .into_iter()
            .map(String::from)
            .collect(),
        }
    }
}

/// Walk `dir` up to `config.max_depth` levels, skipping hidden files,
/// respecting `.gitignore` and `.temperignore`, and filtering by allowed
/// extensions.
pub fn collect_files(
    dir: &Path,
    config: &DirectoryConfig,
) -> crate::error::Result<Vec<std::path::PathBuf>> {
    use ignore::WalkBuilder;

    let mut files = Vec::new();
    let walker = WalkBuilder::new(dir)
        .max_depth(Some(config.max_depth))
        .hidden(true)
        .git_ignore(true)
        .add_custom_ignore_filename(".temperignore")
        .build();

    for entry in walker {
        let entry = entry
            .map_err(|e| crate::error::TemperError::Config(format!("directory walk error: {e}")))?;

        if !entry.path().is_file() {
            continue;
        }

        let ext = entry
            .path()
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();

        if config.allowed_extensions.contains(&ext) {
            files.push(entry.path().to_path_buf());
        }
    }

    Ok(files)
}

/// Run `collect_files` and then verify the total size stays within
/// `config.max_total_bytes`.  Returns an error with a human-readable message
/// when the limit is exceeded.
pub fn preflight_check(
    dir: &Path,
    config: &DirectoryConfig,
) -> crate::error::Result<Vec<std::path::PathBuf>> {
    let files = collect_files(dir, config)?;

    let total_size: u64 = files
        .iter()
        .filter_map(|f| std::fs::metadata(f).ok())
        .map(|m| m.len())
        .sum();

    let limit_mb = config.max_total_bytes as f64 / (1024.0 * 1024.0);
    let total_mb = total_size as f64 / (1024.0 * 1024.0);

    if total_size > config.max_total_bytes {
        return Err(crate::error::TemperError::Config(format!(
            "Directory total size ({total_mb:.1} MB) exceeds limit ({limit_mb:.1} MB). \
             Use --force to skip this check."
        )));
    }

    Ok(files)
}

/// Run directory-mode ingest: walk, optionally pre-flight-check, then upload
/// all matching files with bounded concurrency.
pub fn run_directory(
    path: &str,
    context: &str,
    doc_type: &str,
    format: &str,
    force: bool,
) -> crate::error::Result<()> {
    let dir = Path::new(path);
    if !dir.is_dir() {
        return Err(crate::error::TemperError::Config(format!(
            "not a directory: {path}"
        )));
    }

    let config = DirectoryConfig::default();

    let files = if force {
        collect_files(dir, &config)?
    } else {
        preflight_check(dir, &config)?
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

    let use_progress = std::io::stderr().is_terminal() && !json_mode;
    let max_concurrent = config.max_concurrent;
    let file_count = files.len();

    let rt = tokio::runtime::Runtime::new()
        .map_err(|e| crate::error::TemperError::Api(format!("tokio runtime: {e}")))?;

    rt.block_on(async move {
        use std::sync::Arc;
        use tokio::sync::{Mutex, Semaphore};

        let client = Arc::new(
            temper_client::config::build_client()
                .map_err(|e| crate::error::TemperError::Api(e.to_string()))?,
        );
        let semaphore = Arc::new(Semaphore::new(max_concurrent));

        let added = Arc::new(Mutex::new(0u64));
        let skipped = Arc::new(Mutex::new(0u64));
        let failed = Arc::new(Mutex::new(0u64));

        let pb: Option<indicatif::ProgressBar> = if use_progress {
            let bar = indicatif::ProgressBar::new(file_count as u64);
            bar.set_style(
                indicatif::ProgressStyle::default_bar()
                    .template("  [{bar:40.cyan/blue}] {pos}/{len}  {msg}")
                    .unwrap()
                    .progress_chars("\u{2588}\u{2591}\u{2591}"),
            );
            Some(bar)
        } else {
            None
        };
        let pb = Arc::new(pb);

        let mut handles = Vec::with_capacity(files.len());

        for file_path in files {
            let client = Arc::clone(&client);
            let sem = Arc::clone(&semaphore);
            let added = Arc::clone(&added);
            let skipped = Arc::clone(&skipped);
            let failed = Arc::clone(&failed);
            let pb = Arc::clone(&pb);
            let context = context.to_string();
            let doc_type = doc_type.to_string();

            let handle = tokio::spawn(async move {
                let _permit = sem.acquire_owned().await.ok()?;

                let file_name = file_path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("unknown")
                    .to_string();

                match ingest::ingest_file(&client, &file_path, &context, &doc_type).await {
                    Ok((resource, _content)) => {
                        if json_mode {
                            let event = serde_json::json!({
                                "event": "upload",
                                "file": file_name,
                                "status": "done",
                                "resource_id": resource.id,
                            });
                            output::plain(event);
                        } else if let Some(bar) = pb.as_ref() {
                            bar.set_message(file_name.clone());
                            bar.inc(1);
                        } else {
                            output::success(file_name);
                        }
                        *added.lock().await += 1;
                        Some(resource.id)
                    }
                    Err(err) => {
                        let err_str = err.to_string();
                        if err_str.contains("409") || err_str.contains("duplicate") {
                            if json_mode {
                                let event = serde_json::json!({
                                    "event": "upload",
                                    "file": file_name,
                                    "status": "skipped",
                                    "reason": "duplicate",
                                });
                                output::plain(event);
                            } else if let Some(bar) = pb.as_ref() {
                                bar.set_message(format!("{file_name} (duplicate)"));
                                bar.inc(1);
                            } else {
                                output::dim(format!("{file_name} (duplicate, skipped)"));
                            }
                            *skipped.lock().await += 1;
                        } else {
                            if json_mode {
                                let event = serde_json::json!({
                                    "event": "error",
                                    "file": file_name,
                                    "error": err_str,
                                });
                                output::plain(event);
                            } else if let Some(bar) = pb.as_ref() {
                                bar.set_message(format!("{file_name}: upload failed"));
                                bar.inc(1);
                            } else {
                                output::error(format!("{file_name}: upload failed -- {err_str}"));
                            }
                            *failed.lock().await += 1;
                        }
                        None
                    }
                }
            });

            handles.push(handle);
        }

        for handle in handles {
            let _ = handle.await;
        }

        if let Some(bar) = Arc::try_unwrap(pb).ok().and_then(|opt| opt) {
            bar.finish_and_clear();
        }

        let added = *added.lock().await;
        let skipped = *skipped.lock().await;
        let failed = *failed.lock().await;

        if json_mode {
            let event =
                serde_json::json!({"event":"complete","added":added,"skipped":skipped,"failed":failed});
            output::plain(event);
        } else {
            output::success(format!(
                "{added} added, {skipped} skipped (duplicate), {failed} failed"
            ));
        }

        Ok(())
    })
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // --- URL detection ---

    #[test]
    fn url_http_returns_error() {
        let err = run(
            "http://example.com/doc.pdf",
            false,
            "work",
            "note",
            "text",
            false,
        )
        .unwrap_err();
        assert!(err.to_string().contains("not yet implemented"));
    }

    #[test]
    fn url_https_returns_error() {
        let err = run(
            "https://example.com/paper.md",
            false,
            "work",
            "note",
            "text",
            false,
        )
        .unwrap_err();
        assert!(err.to_string().contains("not yet implemented"));
    }

    // --- Nonexistent file ---

    #[test]
    fn nonexistent_file_returns_error() {
        let err = run(
            "/tmp/does-not-exist-xyz-12345.md",
            false,
            "work",
            "note",
            "text",
            false,
        )
        .unwrap_err();
        assert!(err.to_string().contains("file not found"));
    }

    // --- Directory mode ---

    #[test]
    fn collect_files_respects_max_depth() {
        use std::fs;
        use tempfile::TempDir;

        let dir = TempDir::new().unwrap();
        let root = dir.path();

        fs::write(root.join("top.md"), "# Top").unwrap();

        let sub = root.join("sub");
        fs::create_dir(&sub).unwrap();
        fs::write(sub.join("inner.md"), "# Inner").unwrap();

        let deep = sub.join("deep");
        fs::create_dir(&deep).unwrap();
        fs::write(deep.join("deep.md"), "# Deep").unwrap();

        let deeper = deep.join("deeper");
        fs::create_dir(&deeper).unwrap();
        fs::write(deeper.join("too_deep.md"), "# Too Deep").unwrap();

        let config = DirectoryConfig {
            max_depth: 2,
            ..DirectoryConfig::default()
        };
        let files = collect_files(root, &config).unwrap();
        let names: Vec<_> = files
            .iter()
            .filter_map(|p| p.file_name().and_then(|n| n.to_str()))
            .collect();

        assert!(names.contains(&"top.md"), "top.md not found in {names:?}");
        assert!(
            names.contains(&"inner.md"),
            "inner.md not found in {names:?}"
        );
        assert!(!names.contains(&"deep.md"), "deep.md should be excluded");
        assert!(
            !names.contains(&"too_deep.md"),
            "too_deep.md should be excluded"
        );
    }

    #[test]
    fn collect_files_filters_by_extension() {
        use std::fs;
        use tempfile::TempDir;

        let dir = TempDir::new().unwrap();
        let root = dir.path();

        fs::write(root.join("doc.md"), "# Markdown").unwrap();
        fs::write(root.join("notes.txt"), "plain text").unwrap();
        fs::write(root.join("image.png"), "binary").unwrap();
        fs::write(root.join("data.csv"), "a,b,c").unwrap();
        fs::write(root.join("page.html"), "<html/>").unwrap();

        let config = DirectoryConfig::default();
        let files = collect_files(root, &config).unwrap();
        let names: Vec<_> = files
            .iter()
            .filter_map(|p| p.file_name().and_then(|n| n.to_str()))
            .collect();

        assert!(names.contains(&"doc.md"), "doc.md should be included");
        assert!(names.contains(&"notes.txt"), "notes.txt should be included");
        assert!(names.contains(&"page.html"), "page.html should be included");
        assert!(
            !names.contains(&"image.png"),
            "image.png should be excluded"
        );
        assert!(!names.contains(&"data.csv"), "data.csv should be excluded");
    }

    #[test]
    fn preflight_check_rejects_oversized_directory() {
        use std::fs;
        use tempfile::TempDir;

        let dir = TempDir::new().unwrap();
        let root = dir.path();

        fs::write(root.join("big.md"), "# Big file with lots of content").unwrap();

        let config = DirectoryConfig {
            max_total_bytes: 1,
            ..DirectoryConfig::default()
        };

        let err = preflight_check(root, &config).unwrap_err();
        assert!(
            err.to_string().contains("exceeds limit"),
            "expected 'exceeds limit' in: {err}"
        );
    }

    #[test]
    fn preflight_check_accepts_within_limit() {
        use std::fs;
        use tempfile::TempDir;

        let dir = TempDir::new().unwrap();
        let root = dir.path();

        fs::write(root.join("small.md"), "# Small").unwrap();

        let config = DirectoryConfig::default();
        let files = preflight_check(root, &config).unwrap();
        let names: Vec<_> = files
            .iter()
            .filter_map(|p| p.file_name().and_then(|n| n.to_str()))
            .collect();
        assert!(names.contains(&"small.md"));
    }

    #[test]
    fn run_directory_errors_on_non_directory() {
        let err = run(
            "/tmp/not-a-real-directory-xyz-12345",
            true,
            "work",
            "note",
            "text",
            false,
        )
        .unwrap_err();
        assert!(
            err.to_string().contains("not a directory")
                || err.to_string().contains("exceeds limit")
                || err.to_string().contains("No matching"),
            "unexpected error: {err}"
        );
    }
}
