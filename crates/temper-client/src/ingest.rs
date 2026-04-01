//! Typed sub-client for the `/api/ingest` endpoint.
//!
//! Creates a resource and uploads markdown content in a single multipart
//! request. Processing (chunk, embed, index) happens asynchronously server-side.

use uuid::Uuid;

use crate::auth;
use crate::error::Result;
use crate::http::HttpClient;
use temper_core::types::resource::ResourceRow;
use temper_core::types::IngestRequest;

/// Sub-client for ingest operations.
pub struct IngestClient<'a> {
    http: &'a HttpClient,
}

impl std::fmt::Debug for IngestClient<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("IngestClient").finish_non_exhaustive()
    }
}

impl<'a> IngestClient<'a> {
    pub(crate) fn new(http: &'a HttpClient) -> Self {
        Self { http }
    }

    /// POST /api/ingest — create resource + trigger async processing.
    ///
    /// Posts a multipart form with two fields:
    /// - `metadata`: JSON string of all request fields except `content`
    /// - `content`: the raw markdown text
    pub async fn create(&self, request: &IngestRequest) -> Result<ResourceRow> {
        let token = auth::current_token()?;

        let metadata = serde_json::json!({
            "title": request.title,
            "kb_context_id": request.kb_context_id,
            "kb_doc_type_id": request.kb_doc_type_id,
            "origin_uri": request.origin_uri,
            "slug": request.slug,
            "mimetype": request.mimetype,
            "tags": request.tags,
            "metadata": request.metadata,
            "context_name": request.context_name,
            "doc_type_name": request.doc_type_name,
        });

        let form = reqwest::multipart::Form::new()
            .text("metadata", serde_json::to_string(&metadata)?)
            .text("content", request.content.clone());

        let req = self.http.post("/api/ingest").multipart(form);
        self.http.send_json(req, Some(&token)).await
    }

    /// PUT /api/ingest/:id — update content for an existing resource.
    ///
    /// Posts a multipart form with a single `content` field containing the
    /// updated markdown text. Processing is re-triggered asynchronously.
    pub async fn update(&self, id: Uuid, content: &str) -> Result<ResourceRow> {
        let token = auth::current_token()?;

        let form = reqwest::multipart::Form::new().text("content", content.to_owned());

        let req = self.http.put(&format!("/api/ingest/{id}")).multipart(form);
        self.http.send_json(req, Some(&token)).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_uuid() -> Uuid {
        Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap()
    }

    fn sample_request() -> IngestRequest {
        IngestRequest {
            content: "# Test\n\nContent".to_owned(),
            title: "Test Doc".to_owned(),
            kb_context_id: sample_uuid(),
            kb_doc_type_id: sample_uuid(),
            origin_uri: "kb://temper/resource/test-doc".to_owned(),
            slug: None,
            mimetype: None,
            tags: None,
            metadata: None,
            context_name: None,
            doc_type_name: None,
            resource_mode: None,
        }
    }

    #[test]
    fn test_metadata_json_excludes_content() {
        let req = sample_request();
        let metadata = serde_json::json!({
            "title": req.title,
            "kb_context_id": req.kb_context_id,
            "kb_doc_type_id": req.kb_doc_type_id,
            "origin_uri": req.origin_uri,
            "slug": req.slug,
            "mimetype": req.mimetype,
            "tags": req.tags,
            "metadata": req.metadata,
            "context_name": req.context_name,
            "doc_type_name": req.doc_type_name,
        });

        let obj = metadata.as_object().unwrap();
        assert!(
            !obj.contains_key("content"),
            "metadata must not include content"
        );
        assert_eq!(metadata["title"], "Test Doc");
        assert_eq!(metadata["origin_uri"], "kb://temper/resource/test-doc");
    }

    #[test]
    fn test_metadata_json_with_optional_fields() {
        let req = IngestRequest {
            slug: Some("test-doc".to_owned()),
            mimetype: Some("text/markdown".to_owned()),
            tags: Some(vec!["tag1".to_owned()]),
            context_name: Some("work".to_owned()),
            doc_type_name: Some("note".to_owned()),
            ..sample_request()
        };

        let metadata = serde_json::json!({
            "title": req.title,
            "kb_context_id": req.kb_context_id,
            "kb_doc_type_id": req.kb_doc_type_id,
            "origin_uri": req.origin_uri,
            "slug": req.slug,
            "mimetype": req.mimetype,
            "tags": req.tags,
            "metadata": req.metadata,
            "context_name": req.context_name,
            "doc_type_name": req.doc_type_name,
        });

        assert_eq!(metadata["slug"], "test-doc");
        assert_eq!(metadata["mimetype"], "text/markdown");
        assert_eq!(metadata["context_name"], "work");
        assert_eq!(metadata["doc_type_name"], "note");
    }

    #[test]
    fn test_ingest_client_is_debug() {
        let client = HttpClient::new("https://example.com", None);
        let ingest = IngestClient::new(&client);
        let debug_str = format!("{ingest:?}");
        assert!(debug_str.contains("IngestClient"));
    }
}
