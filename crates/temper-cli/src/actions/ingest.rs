//! Shared business logic for cloud ingest operations.
//!
//! Pure helpers consumed by cloud-mode paths: body chunking, frontmatter
//! construction from server resources, body normalization, and URL fetch.
//! Manifest-coupled and local-vault helpers were removed in Chunk 7
//! (Tasks 5 + 8); the sync/manifest stack is retired in Task 7.

use crate::error::{Result, TemperError};

// ---------------------------------------------------------------------------
// Slug / body helpers
// ---------------------------------------------------------------------------

/// Slugify a title for use in URIs and slugs.
///
/// Delegates to `temper_core::operations::sluggify` — the one slug function,
/// shared with decorated-ref decoration so URIs/filenames and ref decorations
/// can never drift apart.
pub fn slug_from_title(title: &str) -> String {
    temper_core::operations::sluggify(title)
}

/// Body trio extracted from raw markdown — the chunk + hash output that
/// goes onto IngestPayload (cloud create) or ResourceUpdateRequest (cloud update).
pub struct BodyChunks {
    pub content_hash: String,
    pub chunks_packed: String,
}

/// Compute (content_hash, chunks_packed) from raw markdown without
/// vault/manifest side effects. Single source of truth for chunk + hash
/// extraction; used by `cmd_to_ingest_payload` (cloud create) and the
/// cloud-mode update path in `cloud_backend/translators.rs`.
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
// Frontmatter construction
// ---------------------------------------------------------------------------

/// Build a complete `Frontmatter` from a server `ResourceRow` plus the
/// caller-resolved canonical owner sigil.
///
/// `canonical_owner` is the value to write into `temper-owner`. The caller
/// is responsible for resolving the API's `@me` shorthand to
/// `@<profile.slug>`.
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
    canonical_owner: &str,
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
    fm.set_managed_field(
        "temper-title",
        serde_json::Value::String(resource.title.clone()),
    );
    if let Some(slug) = &resource.slug {
        fm.set_managed_field("temper-slug", serde_json::Value::String(slug.clone()));
    }
    if !canonical_owner.is_empty() {
        fm.set_managed_field(
            "temper-owner",
            serde_json::Value::String(canonical_owner.to_string()),
        );
    }
    if let Some(obj) = managed_meta.and_then(|m| m.as_object()) {
        for (k, v) in obj {
            // System fields plus temper-title/temper-slug are set above from
            // resource-row columns; skip them as defense-in-depth so a drifted
            // managed_meta payload can't overwrite the canonical values.
            if temper_core::frontmatter::fields::SYSTEM_MANAGED_FIELDS.contains(&k.as_str())
                || k == "temper-title"
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
            managed_hash: None,
            open_hash: None,
        }
    }

    #[test]
    fn build_frontmatter_from_resource_writes_canonical_owner_for_at_me() {
        let resource = test_resource_row();
        // Caller is responsible for resolving @me -> @<slug> before calling.

        let fm = build_frontmatter_from_resource(
            &resource,
            "temper",
            "research",
            "@j-cole-taylor",
            String::new(),
            None,
            None,
        )
        .unwrap();

        let owner = fm
            .value()
            .get("temper-owner")
            .and_then(|v| v.as_str())
            .expect("temper-owner must be set");
        assert_eq!(
            owner, "@j-cole-taylor",
            "frontmatter must record the canonical owner the caller passed in, \
             not the API's @me shorthand"
        );
    }

    #[test]
    fn build_frontmatter_from_resource_passes_team_handle_through() {
        let resource = test_resource_row();

        let fm = build_frontmatter_from_resource(
            &resource,
            "temper",
            "research",
            "+platform-eng",
            String::new(),
            None,
            None,
        )
        .unwrap();

        let owner = fm
            .value()
            .get("temper-owner")
            .and_then(|v| v.as_str())
            .expect("temper-owner must be set");
        assert_eq!(owner, "+platform-eng");
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
            "@me",
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
            "@me",
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
            "@me",
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
            "@me",
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
            "@me",
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
