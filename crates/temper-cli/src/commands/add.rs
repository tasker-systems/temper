//! `temper add` — ingest a single file or directory into the knowledge base.

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
// Directory mode — stub, implemented in Task 11
// ---------------------------------------------------------------------------

/// Run directory-mode ingest.
///
/// Not yet implemented — returns an error.
pub fn run_directory(
    _path: &str,
    _context: &str,
    _doc_type: &str,
    _format: &str,
    _force: bool,
) -> crate::error::Result<()> {
    Err(crate::error::TemperError::Config(
        "directory mode not yet implemented".to_string(),
    ))
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

    // --- Directory mode stub ---

    #[test]
    fn directory_mode_returns_not_implemented() {
        let err = run("/tmp", true, "work", "note", "text", false).unwrap_err();
        assert!(err.to_string().contains("not yet implemented"));
    }
}
