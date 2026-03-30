//! `temper add` — ingest a single file or directory into the knowledge base.

use std::io::IsTerminal;
use std::path::Path;

use sha2::{Digest, Sha256};

/// Compute the SHA-256 content hash of a UTF-8 string, returned as a lowercase
/// hex string.
pub fn compute_content_hash(content: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(content.as_bytes());
    let bytes = hasher.finalize();
    bytes.iter().fold(String::new(), |mut acc, b| {
        acc.push_str(&format!("{b:02x}"));
        acc
    })
}

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

    // Verify the file exists.
    if !file_path.exists() {
        return Err(crate::error::TemperError::Config(format!(
            "file not found: {}",
            file_path.display()
        )));
    }

    let json_mode = format == "json";
    let file_name = file_path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or(path)
        .to_string();

    // Step 1: Extract to markdown.
    if !json_mode {
        eprint!("  Extracting... ");
    }

    let extraction = crate::extract::extract_to_markdown(&file_path)?;
    let size_bytes = extraction.content.len();

    if json_mode {
        let event = serde_json::json!({
            "event": "extract",
            "file": file_name,
            "status": "done",
            "size_bytes": size_bytes,
        });
        println!("{event}");
    } else {
        println!("done ({} KB markdown)", size_bytes / 1024);
    }

    // Step 2: Compute content hash (used for dedup / manifest tracking).
    let _content_hash = compute_content_hash(&extraction.content);

    // Step 3: Build the IngestRequest.
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

    let metadata = serde_json::json!({
        "device_id": device_id,
        "original_path": canonical_path,
        "content_hash": _content_hash,
    });

    let request = temper_core::types::IngestRequest {
        content: extraction.content,
        title: title.clone(),
        kb_context_id: uuid::Uuid::nil(),
        kb_doc_type_id: uuid::Uuid::nil(),
        uri,
        slug: None,
        mimetype: Some(extraction.mime_type),
        tags: None,
        metadata: Some(metadata),
        context_name: Some(context.to_string()),
        doc_type_name: Some(doc_type.to_string()),
    };

    // Step 4: Upload via the API.
    if !json_mode {
        eprint!("  Uploading... ");
    }

    let rt = tokio::runtime::Runtime::new()
        .map_err(|e| crate::error::TemperError::Config(format!("tokio runtime: {e}")))?;

    let resource = rt.block_on(async {
        let client = temper_client::config::build_client()
            .map_err(|e| crate::error::TemperError::Config(e.to_string()))?;

        client
            .ingest()
            .create(&request)
            .await
            .map_err(|e| crate::error::TemperError::Config(e.to_string()))
    })?;

    // Step 5: Print result.
    if json_mode {
        let event = serde_json::json!({
            "event": "upload",
            "file": file_name,
            "status": "done",
            "resource_id": resource.id,
        });
        println!("{event}");
    } else {
        println!("done");
        println!("\u{2713} Added: {:?} ({})", title, resource.id);
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
    let use_progress = std::io::stderr().is_terminal() && !json_mode;
    let max_concurrent = config.max_concurrent;
    let file_count = files.len();

    let rt = tokio::runtime::Runtime::new()
        .map_err(|e| crate::error::TemperError::Config(format!("tokio runtime: {e}")))?;

    rt.block_on(async move {
        use std::sync::Arc;
        use tokio::sync::{Mutex, Semaphore};

        let client = Arc::new(
            temper_client::config::build_client()
                .map_err(|e| crate::error::TemperError::Config(e.to_string()))?,
        );
        let semaphore = Arc::new(Semaphore::new(max_concurrent));

        // Counters shared across tasks.
        let added = Arc::new(Mutex::new(0u64));
        let skipped = Arc::new(Mutex::new(0u64));
        let failed = Arc::new(Mutex::new(0u64));

        // Progress bar (TTY + non-JSON mode only).
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

                // Extract.
                let extraction = match crate::extract::extract_to_markdown(&file_path) {
                    Ok(e) => e,
                    Err(err) => {
                        if json_mode {
                            let event = serde_json::json!({
                                "event": "error",
                                "file": file_name,
                                "error": err.to_string(),
                            });
                            println!("{event}");
                        } else if let Some(bar) = pb.as_ref() {
                            bar.set_message(format!("\u{2717} {file_name}: extract failed"));
                            bar.inc(1);
                        } else {
                            eprintln!("  \u{2717} {file_name}: extract failed \u{2014} {err}");
                        }
                        *failed.lock().await += 1;
                        return None;
                    }
                };

                let size_bytes = extraction.content.len();

                if json_mode {
                    let event = serde_json::json!({
                        "event": "extract",
                        "file": file_name,
                        "status": "done",
                        "size_bytes": size_bytes,
                    });
                    println!("{event}");
                }

                let title = title_from_path(&file_path);
                let uri = format!(
                    "kb://{context}/{doc_type}/{}",
                    title.to_lowercase().replace(' ', "-")
                );
                let _content_hash = compute_content_hash(&extraction.content);

                let device_id = load_device_id();
                let canonical_path = std::fs::canonicalize(&file_path)
                    .unwrap_or_else(|_| file_path.clone())
                    .to_string_lossy()
                    .to_string();

                let metadata = serde_json::json!({
                    "device_id": device_id,
                    "original_path": canonical_path,
                    "content_hash": _content_hash,
                });

                let request = temper_core::types::IngestRequest {
                    content: extraction.content,
                    title: title.clone(),
                    kb_context_id: uuid::Uuid::nil(),
                    kb_doc_type_id: uuid::Uuid::nil(),
                    uri,
                    slug: None,
                    mimetype: Some(extraction.mime_type),
                    tags: None,
                    metadata: Some(metadata),
                    context_name: Some(context.clone()),
                    doc_type_name: Some(doc_type.clone()),
                };

                match client.ingest().create(&request).await {
                    Ok(resource) => {
                        if json_mode {
                            let event = serde_json::json!({
                                "event": "upload",
                                "file": file_name,
                                "status": "done",
                                "resource_id": resource.id,
                            });
                            println!("{event}");
                        } else if let Some(bar) = pb.as_ref() {
                            bar.set_message(file_name.clone());
                            bar.inc(1);
                        } else {
                            println!("  \u{2713} {file_name}");
                        }
                        *added.lock().await += 1;
                        Some(resource.id)
                    }
                    Err(err) => {
                        let err_str = err.to_string();
                        // Treat duplicate / conflict as skipped.
                        if err_str.contains("409") || err_str.contains("duplicate") {
                            if json_mode {
                                let event = serde_json::json!({
                                    "event": "upload",
                                    "file": file_name,
                                    "status": "skipped",
                                    "reason": "duplicate",
                                });
                                println!("{event}");
                            } else if let Some(bar) = pb.as_ref() {
                                bar.set_message(format!("{file_name} (duplicate)"));
                                bar.inc(1);
                            } else {
                                println!("  \u{2192} {file_name} (duplicate, skipped)");
                            }
                            *skipped.lock().await += 1;
                        } else {
                            if json_mode {
                                let event = serde_json::json!({
                                    "event": "error",
                                    "file": file_name,
                                    "error": err_str,
                                });
                                println!("{event}");
                            } else if let Some(bar) = pb.as_ref() {
                                bar.set_message(format!("\u{2717} {file_name}: upload failed"));
                                bar.inc(1);
                            } else {
                                eprintln!("  \u{2717} {file_name}: upload failed \u{2014} {err_str}");
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

        // Finish and clear the progress bar before printing the summary line.
        if let Some(bar) = Arc::try_unwrap(pb).ok().and_then(|opt| opt) {
            bar.finish_and_clear();
        }

        let added = *added.lock().await;
        let skipped = *skipped.lock().await;
        let failed = *failed.lock().await;

        if json_mode {
            let event =
                serde_json::json!({"event":"complete","added":added,"skipped":skipped,"failed":failed});
            println!("{event}");
        } else {
            println!(
                "\u{2713} {added} added, {skipped} skipped (duplicate), {failed} failed"
            );
        }

        Ok(())
    })
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Load the device UUID string from `~/.config/temper/device.json`.
///
/// Returns `None` when the file is absent or cannot be parsed.
fn load_device_id() -> Option<String> {
    let path = dirs::home_dir()?
        .join(".config")
        .join("temper")
        .join("device.json");
    let content = std::fs::read_to_string(path).ok()?;
    let val: serde_json::Value = serde_json::from_str(&content).ok()?;
    val.get("client_id")?.as_str().map(String::from)
}

/// Extract a display title from a file path (stem only, no extension).
pub fn title_from_path(path: &Path) -> String {
    path.file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("Untitled")
        .to_string()
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

    // --- Content hash ---

    #[test]
    fn content_hash_is_deterministic() {
        let content = "# Hello\n\nThis is a test document.\n";
        let hash1 = compute_content_hash(content);
        let hash2 = compute_content_hash(content);
        assert_eq!(hash1, hash2);
        assert_eq!(hash1.len(), 64); // SHA-256 = 32 bytes = 64 hex chars
    }

    #[test]
    fn content_hash_differs_for_different_content() {
        let hash_a = compute_content_hash("content A");
        let hash_b = compute_content_hash("content B");
        assert_ne!(hash_a, hash_b);
    }

    #[test]
    fn content_hash_is_lowercase_hex() {
        let hash = compute_content_hash("test");
        assert!(hash.chars().all(|c| c.is_ascii_hexdigit()));
        assert!(hash.chars().all(|c| !c.is_uppercase()));
    }

    // --- Title extraction ---

    #[test]
    fn title_from_path_extracts_stem() {
        let path = Path::new("/home/user/docs/research-paper.pdf");
        assert_eq!(title_from_path(path), "research-paper");
    }

    #[test]
    fn title_from_path_handles_no_extension() {
        let path = Path::new("/home/user/notes/README");
        assert_eq!(title_from_path(path), "README");
    }

    #[test]
    fn title_from_path_handles_markdown() {
        let path = Path::new("my-document.md");
        assert_eq!(title_from_path(path), "my-document");
    }

    // --- Directory mode ---

    #[test]
    fn collect_files_respects_max_depth() {
        use std::fs;
        use tempfile::TempDir;

        let dir = TempDir::new().unwrap();
        let root = dir.path();

        // depth 0 (root): should be collected (depth 1 relative to root means the file is inside)
        fs::write(root.join("top.md"), "# Top").unwrap();

        // depth 1: one subdirectory
        let sub = root.join("sub");
        fs::create_dir(&sub).unwrap();
        fs::write(sub.join("inner.md"), "# Inner").unwrap();

        // depth 2: two subdirectories deep — beyond max_depth=2 the WalkBuilder won't descend
        let deep = sub.join("deep");
        fs::create_dir(&deep).unwrap();
        fs::write(deep.join("deep.md"), "# Deep").unwrap();

        // depth 3: three levels — should be excluded when max_depth=2
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

        // top.md (depth 1) and inner.md (depth 2) should be included
        assert!(names.contains(&"top.md"), "top.md not found in {names:?}");
        assert!(
            names.contains(&"inner.md"),
            "inner.md not found in {names:?}"
        );
        // deep.md is at depth 3 (root/sub/deep/deep.md) — excluded
        assert!(!names.contains(&"deep.md"), "deep.md should be excluded");
        // too_deep.md is at depth 4 — excluded
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

        // Write a file larger than a 1-byte limit.
        fs::write(root.join("big.md"), "# Big file with lots of content").unwrap();

        let config = DirectoryConfig {
            max_total_bytes: 1, // 1 byte limit — any real file will exceed this
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

        let config = DirectoryConfig::default(); // 50 MB limit
        let files = preflight_check(root, &config).unwrap();
        let names: Vec<_> = files
            .iter()
            .filter_map(|p| p.file_name().and_then(|n| n.to_str()))
            .collect();
        assert!(names.contains(&"small.md"));
    }

    #[test]
    fn run_directory_errors_on_non_directory() {
        // Pass a path that is a file or simply does not exist as a dir.
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
