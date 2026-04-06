//! Search business logic — embedding, API call, manifest enrichment, formatting.
//!
//! All testable functions. The CLI command is a thin wrapper over these.

use serde::Serialize;
use temper_core::types::api::UnifiedSearchResultRow;
use temper_core::types::Manifest;
use uuid::Uuid;

use crate::error::{Result, TemperError};

/// A search result enriched with local vault information.
#[derive(Debug, Clone, Serialize)]
pub struct EnrichedSearchResult {
    pub resource_id: Uuid,
    pub title: String,
    pub slug: String,
    pub kb_uri: String,
    pub origin_uri: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context: Option<String>,
    pub doc_type: String,
    pub score: f32,
    /// Which search mode(s) found this result: "fts", "vector", or "both"
    pub origin: String,
    pub local: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vault_path: Option<String>,
}

/// Embed query text locally via temper-ingest.
#[cfg(feature = "embed")]
pub fn embed_query(text: &str) -> Result<Vec<f32>> {
    temper_ingest::embed::embed_text(text)
        .map_err(|e| TemperError::Extraction(format!("embedding failed: {e}")))
}

#[cfg(not(feature = "embed"))]
pub fn embed_query(_text: &str) -> Result<Vec<f32>> {
    Err(TemperError::Config(
        "search requires the 'embed' feature — rebuild with --features embed".into(),
    ))
}

/// Call the search API with a pre-computed embedding.
pub async fn query_api(
    client: &temper_client::TemperClient,
    embedding: Vec<f32>,
    context_name: Option<String>,
    doc_type: Option<String>,
    limit: Option<i64>,
) -> Result<Vec<UnifiedSearchResultRow>> {
    client
        .search()
        .query(embedding, context_name, doc_type, limit)
        .await
        .map_err(|e| TemperError::Api(e.to_string()))
}

/// Call the search API with a plain text query (no embedding needed).
pub async fn text_query_api(
    client: &temper_client::TemperClient,
    query: &str,
    context_name: Option<String>,
    doc_type: Option<String>,
    limit: Option<i64>,
) -> Result<Vec<UnifiedSearchResultRow>> {
    client
        .search()
        .text_query(query, context_name, doc_type, limit)
        .await
        .map_err(|e| TemperError::Api(e.to_string()))
}

/// Enrich API results with local manifest data.
pub fn enrich_results(
    results: Vec<UnifiedSearchResultRow>,
    manifest: &Manifest,
) -> Vec<EnrichedSearchResult> {
    results
        .into_iter()
        .map(|row| {
            let entry = manifest.entries.get(&row.resource_id);
            EnrichedSearchResult {
                resource_id: row.resource_id,
                title: row.title,
                slug: row.slug,
                kb_uri: row.kb_uri,
                origin_uri: row.origin_uri,
                context: row.context,
                doc_type: row.doc_type,
                score: row.combined_score,
                origin: row.origin,
                local: entry.is_some(),
                vault_path: entry.map(|e| e.path.clone()),
            }
        })
        .collect()
}

/// Truncate a snippet to max_chars (character count), breaking at word boundaries.
pub fn truncate_snippet(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text.to_string();
    }
    // Find the byte offset of the max_chars-th character
    let byte_offset = text
        .char_indices()
        .nth(max_chars)
        .map(|(i, _)| i)
        .unwrap_or(text.len());
    let truncated = &text[..byte_offset];
    match truncated.rfind(' ') {
        Some(pos) => format!("{}...", &text[..pos]),
        None => format!("{truncated}..."),
    }
}

/// Format results as human-readable text lines.
pub fn format_text(results: &[EnrichedSearchResult]) -> Vec<String> {
    let mut lines = Vec::new();
    for (i, r) in results.iter().enumerate() {
        let local_marker = if r.local { " [local]" } else { "" };
        lines.push(format!(
            "{}. {} (score: {:.2}, via {}){local_marker}",
            i + 1,
            r.title,
            r.score,
            r.origin
        ));
        lines.push(format!("   {}", r.slug));
        if let Some(ref path) = r.vault_path {
            lines.push(format!("   vault: {path}"));
        }
        lines.push(String::new());
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use temper_core::types::{ManifestEntry, ManifestEntryState};

    fn sample_result(resource_id: Uuid, title: &str) -> UnifiedSearchResultRow {
        UnifiedSearchResultRow {
            resource_id,
            title: title.to_string(),
            slug: "test-task".to_string(),
            kb_uri: format!("kb://temper/task/{resource_id}"),
            origin_uri: "file:///vault/temper/tasks/test.md".to_string(),
            context: Some("temper".to_string()),
            doc_type: "task".to_string(),
            fts_score: 0.85,
            vector_score: 0.0,
            combined_score: 0.85,
            origin: "fts".to_string(),
        }
    }

    fn sample_manifest() -> Manifest {
        let mut manifest = Manifest::new("test-device".to_string());
        manifest.entries.insert(
            Uuid::nil(),
            ManifestEntry {
                path: "temper/tasks/test-task.md".to_string(),
                body_hash: "sha256:abc".to_string(),
                remote_body_hash: "sha256:abc".to_string(),
                managed_hash: String::new(),
                open_hash: String::new(),
                remote_managed_hash: String::new(),
                remote_open_hash: String::new(),
                synced_at: Utc::now(),
                state: ManifestEntryState::Clean,
                mtime_secs: None,
                provisional: false,
                last_audit_id: None,
            },
        );
        manifest
    }

    #[test]
    fn test_enrich_marks_local_resources() {
        let results = vec![
            sample_result(Uuid::nil(), "Local Task"),
            sample_result(Uuid::from_u128(1), "Remote Task"),
        ];
        let enriched = enrich_results(results, &sample_manifest());
        assert!(enriched[0].local);
        assert_eq!(
            enriched[0].vault_path.as_deref(),
            Some("temper/tasks/test-task.md")
        );
        assert!(!enriched[1].local);
        assert!(enriched[1].vault_path.is_none());
    }

    #[test]
    fn test_enrich_preserves_kb_uri() {
        let id = Uuid::nil();
        let results = vec![sample_result(id, "Task")];
        let enriched = enrich_results(results, &Manifest::new("d".into()));
        assert_eq!(enriched[0].kb_uri, format!("kb://temper/task/{id}"));
    }

    #[test]
    fn test_enrich_empty_inputs() {
        assert!(enrich_results(vec![], &sample_manifest()).is_empty());
    }

    #[test]
    fn test_truncate_short() {
        assert_eq!(truncate_snippet("short", 200), "short");
    }

    #[test]
    fn test_truncate_long() {
        let long = "word ".repeat(100);
        let result = truncate_snippet(&long, 20);
        assert!(result.ends_with("..."));
        assert!(result.len() < 30);
    }

    #[test]
    fn test_truncate_no_space() {
        assert_eq!(truncate_snippet("aaaaaaaaaaaa", 5), "aaaaa...");
    }

    #[test]
    fn test_format_text_output() {
        let results = vec![sample_result(Uuid::nil(), "My Task")];
        let enriched = enrich_results(results, &sample_manifest());
        let lines = format_text(&enriched);
        assert!(lines[0].contains("1. My Task"));
        assert!(lines[0].contains("0.85"));
        assert!(lines[0].contains("[local]"));
    }

    #[test]
    fn test_format_text_no_local() {
        let results = vec![sample_result(Uuid::from_u128(99), "Remote")];
        let enriched = enrich_results(results, &Manifest::new("d".into()));
        let lines = format_text(&enriched);
        assert!(!lines[0].contains("[local]"));
    }

    #[test]
    fn test_enriched_json_shape() {
        let results = vec![sample_result(Uuid::nil(), "Test")];
        let enriched = enrich_results(results, &sample_manifest());
        let json = serde_json::to_value(&enriched[0]).unwrap();
        assert!(json.get("resource_id").is_some());
        assert!(json.get("kb_uri").is_some());
        assert!(json.get("origin_uri").is_some());
        assert!(json.get("local").is_some());
        assert!(json.get("score").is_some());
        assert!(json.get("origin").is_some());
        assert!(json.get("slug").is_some());
    }
}
