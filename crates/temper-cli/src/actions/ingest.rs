//! Shared business logic for cloud ingest operations (add, import, pull).
//!
//! This module holds the domain logic that was previously duplicated across
//! `commands::add`, `commands::import_cmd`, and `commands::pull`. Command
//! modules are now thin wrappers that call into these functions.

use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::error::{Result, TemperError};

// ---------------------------------------------------------------------------
// Content hashing
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// Title / URI helpers
// ---------------------------------------------------------------------------

/// Extract a display title from a file path (stem only, no extension).
pub fn title_from_path(path: &Path) -> String {
    path.file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("Untitled")
        .to_string()
}

/// Build a `kb://` URI from context, doc_type, and title.
pub fn build_uri(context: &str, doc_type: &str, title: &str) -> String {
    format!(
        "kb://{context}/{doc_type}/{}",
        title.to_lowercase().replace(' ', "-")
    )
}

// ---------------------------------------------------------------------------
// IngestPayload construction
// ---------------------------------------------------------------------------

/// Slugify a title for use in URIs and slugs.
fn slug_from_title(title: &str) -> String {
    title
        .to_lowercase()
        .replace(|c: char| !c.is_alphanumeric() && c != '-', "-")
        .trim_matches('-')
        .to_owned()
}

/// Build a wire-ready `IngestPayload` from extracted markdown.
///
/// Performs chunk → embed → pack locally, producing a payload ready
/// for POST /api/ingest.
#[cfg(feature = "embed")]
pub fn build_ingest_payload(
    content: &str,
    title: &str,
    context: &str,
    doc_type: &str,
    resource_mode: &str,
    mime_type: &str,
    metadata: Option<serde_json::Value>,
) -> Result<temper_core::types::IngestPayload> {
    use temper_core::types::ingest::{pack_chunks, PackedChunk};
    use temper_ingest::chunk::chunk_markdown;
    use temper_ingest::embed::embed_texts;

    let content_hash = compute_content_hash(content);
    let slug = slug_from_title(title);
    let origin_uri = build_uri(context, doc_type, &slug);

    // Chunk
    let chunk_data = chunk_markdown(content);

    // Embed
    let texts: Vec<&str> = chunk_data.iter().map(|c| c.content.as_str()).collect();
    let embeddings = embed_texts(&texts)
        .map_err(|e| TemperError::Extraction(format!("embedding failed: {e}")))?;

    // Pack
    let packed_chunks: Vec<PackedChunk> = chunk_data
        .into_iter()
        .zip(embeddings)
        .map(|(cd, emb)| PackedChunk {
            chunk_index: cd.chunk_index,
            header_path: cd.header_path,
            content: cd.content,
            content_hash: cd.content_hash,
            embedding: emb,
        })
        .collect();

    let chunks_packed = pack_chunks(&packed_chunks)
        .map_err(|e| TemperError::Extraction(format!("chunk packing failed: {e}")))?;

    Ok(temper_core::types::IngestPayload {
        title: title.to_owned(),
        origin_uri,
        context_name: context.to_owned(),
        doc_type_name: doc_type.to_owned(),
        resource_mode: resource_mode.to_owned(),
        content_hash,
        slug,
        mimetype: mime_type.to_owned(),
        content: content.to_owned(),
        metadata,
        chunks_packed,
    })
}

// ---------------------------------------------------------------------------
// URL fetch
// ---------------------------------------------------------------------------

/// Fetch a URL to a temporary file, returning the path and inferred filename.
///
/// The response body is written to a temp file with the appropriate extension
/// (`.html` for HTML content, derived from URL path otherwise). The temp file
/// persists as long as the returned `TempPath` is alive.
pub async fn fetch_url_to_tempfile(url: &str) -> Result<(tempfile::TempPath, String)> {
    let response = reqwest::get(url)
        .await
        .map_err(|e| TemperError::Api(format!("fetch {url}: {e}")))?;

    if !response.status().is_success() {
        return Err(TemperError::Api(format!(
            "fetch {url}: HTTP {}",
            response.status()
        )));
    }

    // Determine file extension from content-type or URL path.
    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();

    let extension = extension_from_content_type(&content_type)
        .or_else(|| extension_from_url(url))
        .unwrap_or("html");

    // Derive a display name from the URL path.
    let display_name = display_name_from_url(url);

    let mut tmp = tempfile::Builder::new()
        .suffix(&format!(".{extension}"))
        .tempfile()
        .map_err(|e| TemperError::Extraction(format!("create temp file: {e}")))?;

    let bytes = response
        .bytes()
        .await
        .map_err(|e| TemperError::Api(format!("read response body: {e}")))?;

    std::io::Write::write_all(&mut tmp, &bytes)
        .map_err(|e| TemperError::Extraction(format!("write temp file: {e}")))?;

    let path = tmp.into_temp_path();
    Ok((path, display_name))
}

/// Map a Content-Type header to a file extension.
fn extension_from_content_type(ct: &str) -> Option<&'static str> {
    let ct = ct.split(';').next().unwrap_or("").trim();
    match ct {
        "text/html" => Some("html"),
        "text/plain" => Some("txt"),
        "text/markdown" => Some("md"),
        "application/pdf" => Some("pdf"),
        _ => None,
    }
}

/// Extract a file extension from the URL path.
fn extension_from_url(url: &str) -> Option<&'static str> {
    let path = url.split('?').next().unwrap_or(url);
    let last_segment = path.rsplit('/').next().unwrap_or("");
    let ext = last_segment.rsplit('.').next().unwrap_or("");
    match ext {
        "html" | "htm" => Some("html"),
        "md" | "markdown" => Some("md"),
        "txt" => Some("txt"),
        "pdf" => Some("pdf"),
        _ => None,
    }
}

/// Derive a human-readable display name from a URL.
fn display_name_from_url(url: &str) -> String {
    let path = url
        .split("://")
        .nth(1)
        .unwrap_or(url)
        .split('?')
        .next()
        .unwrap_or(url);
    // Use the last meaningful path segment, or the domain
    let segments: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    match segments.last() {
        Some(&seg) if seg.contains('.') => {
            // Strip extension for title
            seg.rsplit_once('.')
                .map(|(name, _)| name)
                .unwrap_or(seg)
                .to_string()
        }
        Some(&seg) => seg.to_string(),
        None => path.to_string(),
    }
}

// ---------------------------------------------------------------------------
// Cloud ingest
// ---------------------------------------------------------------------------

/// Extract a file and upload it via the ingest API.
///
/// Performs extract → chunk → embed → pack → upload locally.
/// Returns `(resource, extracted_content)` — the content is needed by callers
/// that write vault files.
#[cfg(feature = "embed")]
pub async fn ingest_file(
    client: &temper_client::TemperClient,
    file_path: &Path,
    context: &str,
    doc_type: &str,
    resource_mode: Option<&str>,
) -> Result<(temper_core::types::ResourceRow, String)> {
    let extraction = crate::extract::extract_to_markdown(file_path)?;
    let extracted_content = extraction.content.clone();

    let title = title_from_path(file_path);
    let mode = resource_mode.unwrap_or("added");

    let device_id = crate::config::load_device_id();
    let canonical_path = std::fs::canonicalize(file_path)
        .unwrap_or_else(|_| file_path.to_path_buf())
        .to_string_lossy()
        .to_string();
    let metadata = serde_json::json!({
        "device_id": device_id,
        "original_path": canonical_path,
    });

    let payload = build_ingest_payload(
        &extraction.content,
        &title,
        context,
        doc_type,
        mode,
        &extraction.mime_type,
        Some(metadata),
    )?;

    let resource = client
        .ingest()
        .create(&payload)
        .await
        .map_err(|e| TemperError::Api(e.to_string()))?;

    Ok((resource, extracted_content))
}

/// Fetch a URL and ingest its content via the same pipeline as local files.
///
/// Downloads to a temp file, extracts via kreuzberg, then uploads. The origin_uri
/// is set to the original URL (not the temp file path).
#[cfg(feature = "embed")]
pub async fn ingest_url(
    client: &temper_client::TemperClient,
    url: &str,
    context: &str,
    doc_type: &str,
    resource_mode: Option<&str>,
) -> Result<(temper_core::types::ResourceRow, String)> {
    let (temp_path, display_name) = fetch_url_to_tempfile(url).await?;

    let extraction = crate::extract::extract_to_markdown(temp_path.as_ref())?;
    let extracted_content = extraction.content.clone();

    let title = display_name;
    let mode = resource_mode.unwrap_or("added");

    let device_id = crate::config::load_device_id();
    let metadata = serde_json::json!({
        "device_id": device_id,
        "original_url": url,
    });

    let mut payload = build_ingest_payload(
        &extraction.content,
        &title,
        context,
        doc_type,
        mode,
        &extraction.mime_type,
        Some(metadata),
    )?;
    // Override origin_uri with the original URL
    payload.origin_uri = url.to_string();

    let resource = client
        .ingest()
        .create(&payload)
        .await
        .map_err(|e| TemperError::Api(e.to_string()))?;

    Ok((resource, extracted_content))
}

// ---------------------------------------------------------------------------
// Vault file helpers
// ---------------------------------------------------------------------------

/// Canonical vault path for a managed resource.
///
/// `{vault_root}/{context}/{doc_type}/{uuid}.md`
pub fn build_vault_path(vault_root: &Path, context: &str, doc_type: &str, id: Uuid) -> PathBuf {
    vault_root
        .join(context)
        .join(doc_type)
        .join(format!("{id}.md"))
}

/// Generate YAML frontmatter for a vault file.
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

/// Write a vault file and register the resource in the manifest.
///
/// Returns the absolute vault path.
pub fn write_vault_file_and_register(
    vault_root: &Path,
    context: &str,
    doc_type: &str,
    resource: &temper_core::types::ResourceRow,
    content: &str,
    ingestion_source: Option<&str>,
) -> Result<PathBuf> {
    let vault_path = build_vault_path(vault_root, context, doc_type, resource.id);

    if let Some(parent) = vault_path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let frontmatter = build_frontmatter(
        resource.id,
        &resource.title,
        context,
        doc_type,
        ingestion_source,
    );
    let vault_content = format!("{frontmatter}{content}");
    std::fs::write(&vault_path, &vault_content)?;

    // Register in manifest.
    let temper_dir = vault_root.join(".temper");
    let device_id_str = crate::config::load_device_id().unwrap_or_else(|| "unknown".to_string());
    let mut manifest = crate::manifest_io::load_manifest(&temper_dir, &device_id_str)?;

    let content_hash = compute_content_hash(&vault_content);
    let remote_hash = resource.content_hash.clone().unwrap_or_default();
    let rel_path = vault_path
        .strip_prefix(vault_root)
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

    Ok(vault_path)
}

// ---------------------------------------------------------------------------
// URI parsing
// ---------------------------------------------------------------------------

/// Derive a context name from a `kb://{context}/...` URI.
pub fn derive_context_from_uri(uri: &str) -> Option<String> {
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

    // --- build_uri ---

    #[test]
    fn build_uri_formats_correctly() {
        let uri = build_uri("work", "note", "My Document");
        assert_eq!(uri, "kb://work/note/my-document");
    }

    #[test]
    fn build_uri_handles_spaces() {
        let uri = build_uri("personal", "resource", "Research Paper");
        assert_eq!(uri, "kb://personal/resource/research-paper");
    }

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

    // --- URL helpers ---

    #[test]
    fn extension_from_content_type_html() {
        assert_eq!(extension_from_content_type("text/html"), Some("html"));
        assert_eq!(
            extension_from_content_type("text/html; charset=utf-8"),
            Some("html")
        );
    }

    #[test]
    fn extension_from_content_type_plain() {
        assert_eq!(extension_from_content_type("text/plain"), Some("txt"));
    }

    #[test]
    fn extension_from_content_type_unknown() {
        assert_eq!(extension_from_content_type("application/json"), None);
        assert_eq!(extension_from_content_type(""), None);
    }

    #[test]
    fn extension_from_url_with_extension() {
        assert_eq!(
            extension_from_url("https://example.com/docs/guide.html"),
            Some("html")
        );
        assert_eq!(
            extension_from_url("https://example.com/paper.pdf"),
            Some("pdf")
        );
    }

    #[test]
    fn extension_from_url_no_extension() {
        assert_eq!(extension_from_url("https://example.com/docs/guide"), None);
        assert_eq!(extension_from_url("https://example.com/"), None);
    }

    #[test]
    fn extension_from_url_with_query() {
        assert_eq!(
            extension_from_url("https://example.com/doc.html?version=2"),
            Some("html")
        );
    }

    #[test]
    fn display_name_from_url_path_segment() {
        assert_eq!(
            display_name_from_url("https://example.com/docs/getting-started.html"),
            "getting-started"
        );
    }

    #[test]
    fn display_name_from_url_no_extension() {
        assert_eq!(display_name_from_url("https://example.com/about"), "about");
    }

    #[test]
    fn display_name_from_url_root() {
        // Domain "example.com" is treated as a filename — dot stripped → "example"
        assert_eq!(display_name_from_url("https://example.com/"), "example");
    }
}
