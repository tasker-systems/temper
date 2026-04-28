//! Shared business logic for cloud ingest operations (add and pull).
//!
//! This module holds the domain logic that was previously duplicated across
//! `commands::add` and `commands::pull`. Command
//! modules are now thin wrappers that call into these functions.

use std::path::{Path, PathBuf};

use temper_core::vault::Vault;
use uuid::Uuid;

use crate::error::{Result, TemperError};

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
// Source frontmatter parsing
// ---------------------------------------------------------------------------

/// Structured metadata parsed from a source file's YAML frontmatter.
///
/// Field names are normalised from legacy formats (e.g. `type` → `doc_type`,
/// `id` → `legacy_id`).
#[derive(Debug, Default)]
pub struct ParsedFrontmatter {
    pub title: Option<String>,
    pub doc_type: Option<String>,
    pub context: Option<String>,
    pub slug: Option<String>,
    pub date: Option<String>,
    pub legacy_id: Option<String>,
    pub provisional_id: Option<String>,
    pub goal: Option<String>,
    pub stage: Option<String>,
    pub mode: Option<String>,
    pub effort: Option<String>,
    pub status: Option<String>,
}

/// Parse YAML frontmatter from a source markdown file and return structured
/// metadata.  Maps legacy field names (`type` → `doc_type`, `id` →
/// `legacy_id`).
pub fn parse_source_frontmatter(content: &str) -> Option<ParsedFrontmatter> {
    let yaml = temper_core::frontmatter::parse_yaml_block(content)?;

    let s = |key: &str| yaml.get(key).and_then(|v| v.as_str()).map(String::from);

    Some(ParsedFrontmatter {
        title: s("title"),
        doc_type: s("temper-type")
            .or_else(|| s("doc_type"))
            .or_else(|| s("type")),
        context: s("temper-context").or_else(|| s("context")),
        slug: s("slug"),
        date: s("date")
            .or_else(|| s("temper-created").map(|c| c[..10].to_string()))
            .or_else(|| s("created").map(|c| c[..10].to_string())),
        legacy_id: s("temper-id").or_else(|| s("id")),
        provisional_id: s("temper-provisional-id"),
        goal: s("temper-goal").or_else(|| s("goal")),
        stage: s("temper-stage").or_else(|| s("stage")),
        mode: s("temper-mode").or_else(|| s("mode")),
        effort: s("temper-effort").or_else(|| s("effort")),
        status: s("temper-status").or_else(|| s("status")),
    })
}

/// Strip YAML frontmatter from markdown content, returning only the body.
///
/// If the content does not start with `---`, returns the original content
/// unchanged.
pub fn strip_frontmatter(content: &str) -> &str {
    let trimmed = content.trim_start();
    if !trimmed.starts_with("---") {
        return content;
    }
    let rest = &trimmed[3..];
    if let Some(end) = rest.find("\n---") {
        // Skip past the closing `---` and the newline after it.
        let after = &rest[end + 4..];
        after.strip_prefix('\n').unwrap_or(after)
    } else {
        content
    }
}

// ---------------------------------------------------------------------------
// IngestPayload construction
// ---------------------------------------------------------------------------

/// Slugify a title for use in URIs and slugs.
pub fn slug_from_title(title: &str) -> String {
    title
        .to_lowercase()
        .replace(|c: char| !c.is_alphanumeric() && c != '-', "-")
        .trim_matches('-')
        .to_owned()
}

/// Body trio extracted from raw markdown — the chunk + hash output that
/// goes onto IngestPayload (cloud create) or ResourceUpdateRequest (cloud update).
pub struct BodyChunks {
    pub content_hash: String,
    pub chunks_packed: String,
}

/// Compute (content_hash, chunks_packed) from raw markdown without
/// vault/manifest side effects. Single source of truth for chunk + hash
/// extraction; used by both `build_ingest_payload` (cloud and local create)
/// and the cloud-mode update path.
#[cfg(feature = "embed")]
pub fn compute_body_chunks(content: &str) -> Result<BodyChunks> {
    use temper_core::types::ingest::pack_chunks;
    use temper_ingest::pipeline::prepare_markdown;

    let content_hash = temper_core::hash::compute_body_hash(content);
    let packed_chunks = prepare_markdown(content)
        .map_err(|e| TemperError::Extraction(format!("embedding failed: {e}")))?;
    let chunks_packed = pack_chunks(&packed_chunks)
        .map_err(|e| TemperError::Extraction(format!("chunk packing failed: {e}")))?;
    Ok(BodyChunks {
        content_hash,
        chunks_packed,
    })
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
    metadata: Option<serde_json::Value>,
    managed_meta: Option<temper_core::types::ManagedMeta>,
    open_meta: Option<serde_json::Value>,
) -> Result<temper_core::types::IngestPayload> {
    let slug = slug_from_title(title);
    let origin_uri = build_uri(context, doc_type, &slug);
    let body = compute_body_chunks(content)?;

    let managed_meta_value = managed_meta
        .map(|m| serde_json::to_value(m))
        .transpose()
        .map_err(|e| TemperError::Extraction(format!("managed_meta serialization failed: {e}")))?;

    Ok(temper_core::types::IngestPayload {
        title: title.to_owned(),
        origin_uri,
        context_name: context.to_owned(),
        doc_type_name: doc_type.to_owned(),
        content_hash: Some(body.content_hash),
        slug,
        content: content.to_owned(),
        metadata,
        managed_meta: managed_meta_value,
        open_meta,
        chunks_packed: Some(body.chunks_packed),
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
) -> Result<(temper_core::types::ResourceRow, String)> {
    let extraction = crate::extract::extract_to_markdown(file_path).await?;
    let extracted_content = extraction.content.clone();

    let title = title_from_path(file_path);

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
        Some(metadata),
        None,
        None,
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
) -> Result<(temper_core::types::ResourceRow, String)> {
    let (temp_path, display_name) = fetch_url_to_tempfile(url).await?;

    let extraction = crate::extract::extract_to_markdown(temp_path.as_ref()).await?;
    let extracted_content = extraction.content.clone();

    let title = display_name;

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
        Some(metadata),
        None,
        None,
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
/// `{vault_root}/{context}/{doc_type}/{slug}.md`
///
/// The slug is a human-readable identifier derived from the resource title.
/// Falls back to the UUID string when no slug is available.
pub fn build_vault_path(vault_root: &Path, context: &str, doc_type: &str, slug: &str) -> PathBuf {
    // TODO(owner-scoped): thread owner through when subscriptions sync lands.
    // Until then the stub matches Config::owner_for_context's @me fallback.
    Vault::new(vault_root).doc_file("@me", context, doc_type, slug)
}

/// De-duplicate a vault slug by appending `-2`, `-3`, etc. when the target
/// path already exists.
pub fn dedup_vault_slug(vault_root: &Path, context: &str, doc_type: &str, slug: &str) -> String {
    let base_path = build_vault_path(vault_root, context, doc_type, slug);
    if !base_path.exists() {
        return slug.to_string();
    }
    for i in 2..1000 {
        let candidate = format!("{slug}-{i}");
        let path = build_vault_path(vault_root, context, doc_type, &candidate);
        if !path.exists() {
            return candidate;
        }
    }
    // Extremely unlikely — fall back to UUID-suffixed slug.
    format!("{slug}-{}", Uuid::now_v7())
}

/// Generate YAML frontmatter for a new vault file with a provisional ID.
///
/// Uses `temper-provisional-id` instead of `temper-id` to indicate the ID
/// hasn't been confirmed by the server yet.
pub fn build_provisional_frontmatter(
    id: impl std::fmt::Display,
    title: &str,
    context: &str,
    doc_type: &str,
) -> String {
    let now = chrono::Utc::now().to_rfc3339();
    format!(
        "---\ntemper-provisional-id: {id}\ntemper-type: {doc_type}\ntemper-context: {context}\ntemper-created: {now}\ntitle: \"{title}\"\n---\n\n"
    )
}

/// Construct a fresh `Frontmatter` for a vault file. Caller can mutate
/// further or write to disk via `Frontmatter::write_to`.
///
/// `extra_fields` allows callers to inject additional managed-tier
/// key-value pairs (e.g. `temper-stage`, `temper-mode`) without bloating
/// the signature.
pub fn build_frontmatter(
    id: impl std::fmt::Display,
    title: &str,
    context: &str,
    doc_type: &str,
    body: String,
    ingestion_source: Option<&str>,
    extra_fields: Option<&[(&str, &str)]>,
) -> crate::error::Result<temper_core::frontmatter::Frontmatter> {
    use temper_core::frontmatter::{DocType, Frontmatter};

    let dt = DocType::from_str(doc_type)?;
    let now = chrono::Utc::now().to_rfc3339();
    let mut fm = Frontmatter::new(dt, body);
    fm.set_managed_field("temper-id", serde_json::Value::String(id.to_string()));
    fm.set_managed_field(
        "temper-context",
        serde_json::Value::String(context.to_string()),
    );
    fm.set_managed_field("temper-created", serde_json::Value::String(now));
    fm.set_managed_field("title", serde_json::Value::String(title.to_string()));
    if let Some(source) = ingestion_source {
        fm.set_managed_field(
            "temper-source",
            serde_json::Value::String(source.to_string()),
        );
    }
    if let Some(fields) = extra_fields {
        for (key, value) in fields {
            fm.set_managed_field(key, serde_json::Value::String(value.to_string()));
        }
    }
    Ok(fm)
}

/// Generate YAML frontmatter for a vault file from server data.
///
/// Combines resource-level fields (id, type, context, created, title) with
/// managed_meta fields (temper-* keys, stage, mode, effort, etc.) and
/// open_meta fields (user-defined keys: tags, relates_to, extends,
/// depends_on, and any other custom frontmatter) for complete frontmatter
/// that matches what the CLI would produce locally.
pub fn build_frontmatter_from_resource(
    resource: &temper_core::types::ResourceRow,
    context: &str,
    doc_type: &str,
    body: String,
    managed_meta: Option<&serde_json::Value>,
    open_meta: Option<&serde_json::Value>,
) -> crate::error::Result<temper_core::frontmatter::Frontmatter> {
    use temper_core::frontmatter::{DocType, Frontmatter};

    let dt = DocType::from_str(doc_type)?;
    let mut fm = Frontmatter::new(dt, body);
    fm.set_managed_field(
        "temper-id",
        serde_json::Value::String(resource.id.to_string()),
    );
    fm.set_managed_field(
        "temper-context",
        serde_json::Value::String(context.to_string()),
    );
    fm.set_managed_field(
        "temper-created",
        serde_json::Value::String(resource.created.to_rfc3339()),
    );
    fm.set_managed_field("title", serde_json::Value::String(resource.title.clone()));
    if let Some(slug) = &resource.slug {
        fm.set_managed_field("slug", serde_json::Value::String(slug.clone()));
    }
    if !resource.owner_handle.is_empty() {
        fm.set_managed_field(
            "temper-owner",
            serde_json::Value::String(resource.owner_handle.clone()),
        );
    }
    if let Some(obj) = managed_meta.and_then(|m| m.as_object()) {
        for (k, v) in obj {
            // System fields plus `title` are set above from resource-row
            // columns; skip them as defense-in-depth so a drifted
            // managed_meta payload can't overwrite the canonical values.
            //
            // `title` is not part of SYSTEM_MANAGED_FIELDS (that constant
            // describes fields the CLI user cannot edit, not fields that
            // are resource-row-sourced) — we hardcode the skip locally.
            if temper_core::frontmatter::fields::SYSTEM_MANAGED_FIELDS.contains(&k.as_str())
                || k == "title"
            {
                continue;
            }
            fm.set_managed_field(k, v.clone());
        }
    }
    if let Some(obj) = open_meta.and_then(|m| m.as_object()) {
        for (k, v) in obj {
            fm.set_open_field(k, v.clone());
        }
    }
    Ok(fm)
}

/// Normalize the markdown body to include the blank-line separator the
/// historical text-level `build_frontmatter` emitted between the closing
/// `---` and the first content line.
///
/// Old flow: `format!("---\n<yaml>---\n\n{content}")` — always a blank
/// line between the frontmatter fence and the body.
///
/// New flow: `Frontmatter::serialize()` produces `---\n<yaml>---\n{body}`,
/// so the caller must include the leading newline to preserve the
/// separator. This helper does that normalization conservatively: prepend
/// `\n` only if the content doesn't already start with one.
pub fn normalize_body_for_vault(content: &str) -> String {
    if content.is_empty() || content.starts_with('\n') {
        content.to_string()
    } else {
        format!("\n{content}")
    }
}

/// Write a vault file and register the resource in the manifest.
///
/// `slug` determines the vault filename (`{slug}.md`).  Pass
/// `slug_from_title(&resource.title)` when no better slug is available.
///
/// Returns the absolute vault path.
#[expect(
    clippy::too_many_arguments,
    reason = "vault write needs context, slug, resource, content, source, and extra fields"
)]
pub fn write_vault_file_and_register(
    vault_root: &Path,
    context: &str,
    doc_type: &str,
    slug: &str,
    resource: &temper_core::types::ResourceRow,
    content: &str,
    ingestion_source: Option<&str>,
    extra_fields: Option<&[(&str, &str)]>,
) -> Result<PathBuf> {
    let vault_path = build_vault_path(vault_root, context, doc_type, slug);

    if let Some(parent) = vault_path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let fm = build_frontmatter(
        resource.id,
        &resource.title,
        context,
        doc_type,
        normalize_body_for_vault(content),
        ingestion_source,
        extra_fields,
    )?;
    fm.write_to(&vault_path).map_err(|e| {
        crate::error::TemperError::Vault(format!("ingest write {}: {e}", vault_path.display()))
    })?;

    // Register in manifest.
    let temper_dir = vault_root.join(".temper");
    let device_id_str = crate::config::load_device_id().unwrap_or_else(|| "unknown".to_string());
    let mut manifest = crate::manifest_io::load_manifest(&temper_dir, &device_id_str)?;

    let content_hash = temper_core::hash::compute_body_hash(content);
    // After ingest, server body_hash matches our local content_hash
    let remote_hash = content_hash.clone();
    let rel_path = vault_path
        .strip_prefix(vault_root)
        .unwrap_or(&vault_path)
        .to_string_lossy()
        .to_string();

    let mtime_secs = std::fs::metadata(&vault_path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs() as i64);

    manifest.entries.insert(
        resource.id,
        temper_core::types::ManifestEntry {
            path: rel_path,
            body_hash: content_hash,
            remote_body_hash: remote_hash,
            managed_hash: String::new(),
            open_hash: String::new(),
            remote_managed_hash: String::new(),
            remote_open_hash: String::new(),
            synced_at: chrono::Utc::now(),
            state: temper_core::types::ManifestEntryState::Clean,
            mtime_secs,
            last_audit_id: None,
            provisional: false,
        },
    );
    crate::manifest_io::save_manifest(&temper_dir, &manifest)?;

    Ok(vault_path)
}

// ---------------------------------------------------------------------------
// Vault path inference
// ---------------------------------------------------------------------------

/// Infer context and doc_type for a vault file.
///
/// Uses frontmatter overrides if provided, otherwise infers from the file's
/// position in the owner-scoped vault hierarchy:
/// `{vault}/{owner}/{context}/{doc_type}/{slug}.md`.
pub fn infer_context_and_doctype(
    vault_root: &Path,
    file_path: &Path,
    fm_context: Option<&str>,
    fm_doc_type: Option<&str>,
) -> Result<(String, String)> {
    let rel = file_path
        .strip_prefix(vault_root)
        .map_err(|_| {
            TemperError::Config(format!(
                "file {} is not inside vault {}",
                file_path.display(),
                vault_root.display()
            ))
        })?
        .to_string_lossy()
        .to_string();

    let dir_parsed = Vault::parse_rel(&rel);

    let context = fm_context
        .map(|s| s.to_string())
        .or_else(|| dir_parsed.as_ref().map(|p| p.context.to_string()))
        .ok_or_else(|| {
            TemperError::Config(format!("cannot infer context for {}", file_path.display()))
        })?;

    let doc_type = fm_doc_type
        .map(|s| s.to_string())
        .or_else(|| dir_parsed.as_ref().map(|p| p.doc_type.to_string()))
        .ok_or_else(|| {
            TemperError::Config(format!(
            "cannot infer doc_type for {} (file must be at {{owner}}/{{context}}/{{doc_type}}/{{slug}}.md)",
            file_path.display()
        ))
        })?;

    Ok((context, doc_type))
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
        let hash1 = temper_core::hash::compute_body_hash(content);
        let hash2 = temper_core::hash::compute_body_hash(content);
        assert_eq!(hash1, hash2);
        assert!(hash1.starts_with("sha256:"));
        // "sha256:" prefix (7 chars) + 64 hex chars = 71 total
        assert_eq!(hash1.len(), 71);
    }

    #[test]
    fn content_hash_differs_for_different_content() {
        let hash_a = temper_core::hash::compute_body_hash("content A");
        let hash_b = temper_core::hash::compute_body_hash("content B");
        assert_ne!(hash_a, hash_b);
    }

    #[test]
    fn content_hash_has_sha256_prefix() {
        let hash = temper_core::hash::compute_body_hash("test");
        assert!(hash.starts_with("sha256:"));
        let hex_part = &hash[7..];
        assert!(hex_part.chars().all(|c| c.is_ascii_hexdigit()));
        assert!(hex_part.chars().all(|c| !c.is_uppercase()));
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
        let path = build_vault_path(root, "work", "note", "my-document");
        assert_eq!(path, PathBuf::from("/vault/@me/work/note/my-document.md"));
    }

    #[test]
    fn build_vault_path_nested_context() {
        let root = Path::new("/home/user/kb");
        let path = build_vault_path(root, "personal", "resource", "research-paper");
        assert_eq!(
            path,
            PathBuf::from("/home/user/kb/@me/personal/resource/research-paper.md")
        );
    }

    // --- build_frontmatter ---

    #[test]
    fn build_frontmatter_includes_required_fields() {
        let id = Uuid::nil();
        let fm = build_frontmatter(
            id,
            "My Title",
            "work",
            "research",
            String::new(),
            None,
            None,
        )
        .unwrap();
        let v = fm.value();
        assert!(
            v.get("temper-id").is_some(),
            "temper-id missing. value:\n{v:?}"
        );
        assert_eq!(
            v.get("title").and_then(|x| x.as_str()),
            Some("My Title"),
            "title mismatch"
        );
        assert_eq!(
            v.get("temper-context").and_then(|x| x.as_str()),
            Some("work"),
            "temper-context mismatch"
        );
        assert_eq!(
            v.get("temper-type").and_then(|x| x.as_str()),
            Some("research"),
            "temper-type mismatch"
        );
        assert!(v.get("temper-created").is_some(), "temper-created missing");
    }

    #[test]
    fn build_frontmatter_includes_ingestion_source_when_provided() {
        let id = Uuid::nil();
        let fm = build_frontmatter(
            id,
            "My Title",
            "work",
            "research",
            String::new(),
            Some("/home/user/file.pdf"),
            None,
        )
        .unwrap();
        let v = fm.value();
        assert_eq!(
            v.get("temper-source").and_then(|x| x.as_str()),
            Some("/home/user/file.pdf"),
            "expected temper-source in frontmatter"
        );
    }

    #[test]
    fn build_frontmatter_omits_ingestion_source_when_absent() {
        let id = Uuid::nil();
        let fm = build_frontmatter(
            id,
            "My Title",
            "work",
            "research",
            String::new(),
            None,
            None,
        )
        .unwrap();
        let v = fm.value();
        assert!(
            v.get("temper-source").is_none(),
            "unexpected temper-source in frontmatter"
        );
    }

    #[test]
    fn build_frontmatter_includes_extra_fields() {
        let id = Uuid::nil();
        let extras = [("legacy_id", "abc-123"), ("goal", "temper-cloud")];
        let fm = build_frontmatter(
            id,
            "Title",
            "work",
            "task",
            String::new(),
            None,
            Some(&extras),
        )
        .unwrap();
        let v = fm.value();
        assert_eq!(
            v.get("legacy_id").and_then(|x| x.as_str()),
            Some("abc-123"),
            "legacy_id mismatch"
        );
        assert_eq!(
            v.get("goal").and_then(|x| x.as_str()),
            Some("temper-cloud"),
            "goal mismatch"
        );
    }

    // --- parse_source_frontmatter ---

    #[test]
    fn parse_frontmatter_task() {
        let content = r#"---
id: "019d17fd-c400-72c1-8c8a-a1ed6c25a158"
type: task
title: "Prettier Temper CLI"
slug: "2026-03-23-prettier-temper-cli"
context: "temper"
goal: "temper-maintenance"
stage: in-progress
mode: build
effort: medium
---

# Prettier Temper CLI
"#;
        let fm = parse_source_frontmatter(content).expect("should parse");
        assert_eq!(fm.title.as_deref(), Some("Prettier Temper CLI"));
        assert_eq!(fm.doc_type.as_deref(), Some("task"));
        assert_eq!(fm.slug.as_deref(), Some("2026-03-23-prettier-temper-cli"));
        assert_eq!(fm.context.as_deref(), Some("temper"));
        assert_eq!(fm.goal.as_deref(), Some("temper-maintenance"));
        assert_eq!(fm.stage.as_deref(), Some("in-progress"));
        assert_eq!(fm.mode.as_deref(), Some("build"));
        assert_eq!(fm.effort.as_deref(), Some("medium"));
        assert_eq!(
            fm.legacy_id.as_deref(),
            Some("019d17fd-c400-72c1-8c8a-a1ed6c25a158")
        );
    }

    #[test]
    fn parse_frontmatter_goal() {
        let content = r#"---
id: "019d20f9-6a90-7e52-80d7-20c2f36cabb1"
type: goal
title: "Maintenance"
slug: "temper-maintenance"
context: "temper"
status: active
created: 2026-03-23
---

# Maintenance
"#;
        let fm = parse_source_frontmatter(content).expect("should parse");
        assert_eq!(fm.doc_type.as_deref(), Some("goal"));
        assert_eq!(fm.status.as_deref(), Some("active"));
        assert_eq!(fm.date.as_deref(), Some("2026-03-23"));
    }

    #[test]
    fn parse_frontmatter_session_minimal() {
        let content = "---\ntype: session\ndate: 2026-03-27\ncontext: temper\n---\n\n# Session\n";
        let fm = parse_source_frontmatter(content).expect("should parse");
        assert_eq!(fm.doc_type.as_deref(), Some("session"));
        assert_eq!(fm.date.as_deref(), Some("2026-03-27"));
        assert!(fm.legacy_id.is_none());
    }

    #[test]
    fn parse_frontmatter_new_format() {
        let content =
            "---\ntemper-id: abc-123\ntemper-type: research\ntemper-context: work\n---\n\nBody\n";
        let fm = parse_source_frontmatter(content).expect("should parse");
        assert_eq!(fm.doc_type.as_deref(), Some("research"));
        assert_eq!(fm.legacy_id.as_deref(), Some("abc-123"));
        assert_eq!(fm.context.as_deref(), Some("work"));
    }

    #[test]
    fn parse_frontmatter_returns_none_without_frontmatter() {
        let content = "# Just a heading\n\nSome text.\n";
        assert!(parse_source_frontmatter(content).is_none());
    }

    // --- strip_frontmatter ---

    #[test]
    fn strip_frontmatter_removes_yaml_block() {
        let content = "---\ntype: task\ntitle: Test\n---\n# Body\n";
        let body = strip_frontmatter(content);
        assert_eq!(body, "# Body\n");
    }

    #[test]
    fn strip_frontmatter_preserves_blank_line_gap() {
        let content = "---\ntype: task\n---\n\n# Body\n";
        let body = strip_frontmatter(content);
        assert_eq!(body, "\n# Body\n");
    }

    #[test]
    fn strip_frontmatter_returns_content_without_frontmatter() {
        let content = "# No frontmatter here\n";
        assert_eq!(strip_frontmatter(content), content);
    }

    #[test]
    fn strip_frontmatter_handles_no_trailing_newline() {
        let content = "---\ntype: task\n---\nBody text";
        let body = strip_frontmatter(content);
        assert_eq!(body, "Body text");
    }

    // --- infer_context_and_doctype ---

    #[test]
    fn infer_context_doctype_from_path() {
        let vault = Path::new("/vault");
        let file = Path::new("/vault/@me/temper/research/my-notes.md");
        let (ctx, dt) = infer_context_and_doctype(vault, file, None, None).unwrap();
        assert_eq!(ctx, "temper");
        assert_eq!(dt, "research");
    }

    #[test]
    fn infer_context_doctype_frontmatter_override() {
        let vault = Path::new("/vault");
        let file = Path::new("/vault/@me/temper/research/my-notes.md");
        let (ctx, dt) =
            infer_context_and_doctype(vault, file, Some("custom-context"), Some("session"))
                .unwrap();
        assert_eq!(ctx, "custom-context");
        assert_eq!(dt, "session");
    }

    #[test]
    fn infer_context_doctype_rejects_shallow() {
        let vault = Path::new("/vault");
        let file = Path::new("/vault/orphan.md");
        let result = infer_context_and_doctype(vault, file, None, None);
        assert!(result.is_err());
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

    // --- provisional_id parsing ---

    #[test]
    fn test_parse_provisional_id() {
        let content = "---\ntemper-provisional-id: \"019d6088-3a3b-71a3-b26c-d38b8338773e\"\ntitle: \"Test\"\n---\n\nBody";
        let fm = parse_source_frontmatter(content).unwrap();
        assert_eq!(
            fm.provisional_id.as_deref(),
            Some("019d6088-3a3b-71a3-b26c-d38b8338773e")
        );
        assert!(fm.legacy_id.is_none());
    }

    #[test]
    fn test_parse_both_ids_prefers_temper_id() {
        let content =
            "---\ntemper-id: \"aaa\"\ntemper-provisional-id: \"bbb\"\ntitle: \"Test\"\n---\n\nBody";
        let fm = parse_source_frontmatter(content).unwrap();
        assert_eq!(fm.legacy_id.as_deref(), Some("aaa"));
        assert_eq!(fm.provisional_id.as_deref(), Some("bbb"));
    }

    fn test_resource_row() -> temper_core::types::ResourceRow {
        use temper_core::types::ids::{ContextId, DocTypeId, ProfileId, ResourceId};
        temper_core::types::ResourceRow {
            id: ResourceId(uuid::Uuid::nil()),
            kb_context_id: ContextId(uuid::Uuid::nil()),
            kb_doc_type_id: DocTypeId(uuid::Uuid::nil()),
            origin_uri: "test://origin".to_string(),
            title: "Test".to_string(),
            slug: Some("test-slug".to_string()),
            originator_profile_id: ProfileId(uuid::Uuid::nil()),
            owner_profile_id: ProfileId(uuid::Uuid::nil()),
            is_active: true,
            created: chrono::Utc::now(),
            updated: chrono::Utc::now(),
            context_name: "temper".to_string(),
            doc_type_name: "research".to_string(),
            owner_handle: "@me".to_string(),
            stage: None,
            seq: None,
            mode: None,
            effort: None,
            body_hash: None,
        }
    }

    #[test]
    fn test_build_frontmatter_from_resource_preserves_arrays_and_objects() {
        let resource = test_resource_row();

        let meta = serde_json::json!({
            "depends_on": ["slug-a", "slug-b"],
            "extends": ["parent-doc"],
            "tags": ["rust", "graph"],
            "config": {"key": "value", "nested": true}
        });

        let fm = build_frontmatter_from_resource(
            &resource,
            "temper",
            "research",
            String::new(),
            Some(&meta),
            None,
        )
        .unwrap();
        let v = fm.value();

        let depends = v
            .get("depends_on")
            .and_then(|x| x.as_sequence())
            .expect("depends_on should be a sequence");
        let slugs: Vec<&str> = depends.iter().filter_map(|x| x.as_str()).collect();
        assert!(
            slugs.contains(&"slug-a"),
            "depends_on should contain slug-a. Got:\n{v:?}"
        );
        assert!(
            slugs.contains(&"slug-b"),
            "depends_on should contain slug-b. Got:\n{v:?}"
        );
        assert!(
            v.get("extends").is_some(),
            "extends array should be present. Got:\n{v:?}"
        );
        assert!(
            v.get("config").is_some(),
            "config object should be present. Got:\n{v:?}"
        );
    }

    #[test]
    fn test_build_frontmatter_emits_open_meta_arrays() {
        let resource = test_resource_row();

        let open_meta = serde_json::json!({
            "relates_to": ["task://foo", "task://bar"],
            "tags": ["alpha", "beta"],
        });

        let fm = build_frontmatter_from_resource(
            &resource,
            "temper",
            "research",
            String::new(),
            None,
            Some(&open_meta),
        )
        .unwrap();
        let v = fm.value();

        let relates = v
            .get("relates_to")
            .and_then(|x| x.as_sequence())
            .expect("relates_to should be a sequence");
        let entries: Vec<&str> = relates.iter().filter_map(|x| x.as_str()).collect();
        assert!(
            entries.contains(&"task://foo"),
            "relates_to should contain task://foo. Got:\n{v:?}"
        );
        assert!(
            entries.contains(&"task://bar"),
            "relates_to should contain task://bar. Got:\n{v:?}"
        );
        let tags = v
            .get("tags")
            .and_then(|x| x.as_sequence())
            .expect("tags should be a sequence");
        let tag_strs: Vec<&str> = tags.iter().filter_map(|x| x.as_str()).collect();
        assert!(
            tag_strs.contains(&"alpha"),
            "tags should contain alpha. Got:\n{v:?}"
        );
        assert!(
            tag_strs.contains(&"beta"),
            "tags should contain beta. Got:\n{v:?}"
        );
    }

    #[test]
    fn test_build_frontmatter_emits_open_meta_nested_objects() {
        let resource = test_resource_row();

        let open_meta = serde_json::json!({
            "custom_block": {"key": "value", "nested": {"inner": true}},
        });

        let fm = build_frontmatter_from_resource(
            &resource,
            "temper",
            "research",
            String::new(),
            None,
            Some(&open_meta),
        )
        .unwrap();
        let v = fm.value();

        let block = v
            .get("custom_block")
            .expect("custom_block should be present");
        assert_eq!(
            block.get("key").and_then(|x| x.as_str()),
            Some("value"),
            "nested key should be 'value'. Got:\n{block:?}"
        );
        let nested = block.get("nested").expect("nested should be present");
        assert_eq!(
            nested.get("inner").and_then(|x| x.as_bool()),
            Some(true),
            "deeply nested inner should be true. Got:\n{nested:?}"
        );
    }

    #[test]
    fn test_build_frontmatter_emits_both_tiers() {
        let resource = test_resource_row();

        let managed_meta = serde_json::json!({
            "stage": "draft",
            "effort": "M",
        });
        let open_meta = serde_json::json!({
            "relates_to": ["task://alpha"],
            "custom_tag": "hello",
        });

        let fm = build_frontmatter_from_resource(
            &resource,
            "temper",
            "research",
            String::new(),
            Some(&managed_meta),
            Some(&open_meta),
        )
        .unwrap();
        let v = fm.value();

        // Both tiers present
        assert!(
            v.get("stage").is_some(),
            "managed stage missing. Got:\n{v:?}"
        );
        assert!(
            v.get("effort").is_some(),
            "managed effort missing. Got:\n{v:?}"
        );
        assert!(
            v.get("relates_to").is_some(),
            "open relates_to missing. Got:\n{v:?}"
        );
        assert!(
            v.get("custom_tag").is_some(),
            "open custom_tag missing. Got:\n{v:?}"
        );

        // Canonical serialization places known open fields (Tier 3) before
        // schema-extra managed fields (Tier 4). Verify that identity/system
        // fields come before everything else — that's the invariant the
        // canonical ordering function guarantees.
        let serialized = fm.serialize().unwrap();
        let id_pos = serialized.find("temper-id:").expect("temper-id: present");
        let stage_pos = serialized.find("stage:").expect("stage: present");
        let effort_pos = serialized.find("effort:").expect("effort: present");
        let relates_pos = serialized.find("relates_to:").expect("relates_to: present");
        // Identity field must precede all data fields.
        assert!(
            id_pos < stage_pos.min(effort_pos).min(relates_pos),
            "identity fields must precede data fields. Got:\n{serialized}"
        );
    }

    #[test]
    #[cfg(feature = "test-embed")]
    fn build_ingest_payload_attaches_managed_meta_when_some() {
        let mm = temper_core::types::ManagedMeta {
            stage: Some("backlog".to_string()),
            ..Default::default()
        };
        let payload = build_ingest_payload(
            "# Test\nBody",
            "Test Title",
            "temper",
            "task",
            None,
            Some(mm.clone()),
            None,
        )
        .expect("payload");
        // managed_meta is serialized to serde_json::Value; stage is renamed to
        // "temper-stage" by the ManagedMeta serde attribute.
        assert_eq!(
            payload
                .managed_meta
                .as_ref()
                .and_then(|m| m.get("temper-stage"))
                .and_then(|v| v.as_str()),
            Some("backlog")
        );
        assert!(payload.open_meta.is_none());
    }

    #[test]
    #[cfg(feature = "test-embed")]
    fn build_ingest_payload_attaches_open_meta_when_some() {
        let om = serde_json::json!({"tags": ["rust"]});
        let payload = build_ingest_payload("# X", "T", "ctx", "session", None, None, Some(om))
            .expect("payload");
        assert_eq!(
            payload.open_meta.as_ref().and_then(|o| o.get("tags")),
            Some(&serde_json::json!(["rust"]))
        );
    }

    #[test]
    #[cfg(feature = "test-embed")]
    fn build_ingest_payload_uses_compute_body_chunks() {
        let content = "# Test\n\nBody.";
        let payload = build_ingest_payload(content, "Title", "ctx", "session", None, None, None)
            .expect("payload");
        let direct = compute_body_chunks(content).expect("direct compute");
        assert_eq!(
            payload.content_hash.as_deref(),
            Some(direct.content_hash.as_str())
        );
        assert_eq!(
            payload.chunks_packed.as_deref(),
            Some(direct.chunks_packed.as_str())
        );
    }

    #[test]
    #[cfg(feature = "test-embed")]
    fn compute_body_chunks_returns_hash_and_packed_chunks() {
        let content = "# Heading\n\nParagraph one.\n\nParagraph two.";
        let result = compute_body_chunks(content).expect("compute should succeed");
        assert_eq!(
            result.content_hash,
            temper_core::hash::compute_body_hash(content)
        );
        assert!(!result.chunks_packed.is_empty());
    }

    #[test]
    fn test_build_frontmatter_tolerates_none_open_meta() {
        let resource = test_resource_row();

        let managed_meta = serde_json::json!({
            "stage": "draft",
            "effort": "M",
        });

        let fm = build_frontmatter_from_resource(
            &resource,
            "temper",
            "research",
            String::new(),
            Some(&managed_meta),
            None,
        )
        .unwrap();
        let v = fm.value();

        assert_eq!(
            v.get("stage").and_then(|x| x.as_str()),
            Some("draft"),
            "stage should be rendered. Got:\n{v:?}"
        );
        assert_eq!(
            v.get("effort").and_then(|x| x.as_str()),
            Some("M"),
            "effort should be rendered. Got:\n{v:?}"
        );
        // Serialized form should have no blank lines inside the frontmatter block.
        let serialized = fm.serialize().unwrap();
        let inside = serialized
            .strip_prefix("---\n")
            .expect("leading ---")
            .split("\n---\n")
            .next()
            .expect("closing ---");
        for line in inside.lines() {
            assert!(
                !line.trim().is_empty(),
                "no blank lines expected inside frontmatter. Got:\n{serialized}"
            );
        }
    }
}
