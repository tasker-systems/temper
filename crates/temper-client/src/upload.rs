//! Typed sub-client for the `/api/upload` endpoint.
//!
//! Uploads file content to the TypeScript upload endpoint (Vercel Blob).

use reqwest::Method;
use uuid::Uuid;

use crate::auth;
use crate::error::Result;
use crate::http::HttpClient;
use temper_core::types::upload::UploadResponse;

/// Sub-client for file upload operations.
pub struct UploadClient<'a> {
    http: &'a HttpClient,
}

impl std::fmt::Debug for UploadClient<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("UploadClient").finish_non_exhaustive()
    }
}

impl<'a> UploadClient<'a> {
    pub(crate) fn new(http: &'a HttpClient) -> Self {
        Self { http }
    }

    /// Upload file content for an existing resource (Tier 1: add).
    ///
    /// Posts a multipart form to `/api/upload` (TypeScript endpoint).
    /// Processing (extract, chunk, embed) happens asynchronously via
    /// Vercel Workflow.
    pub async fn add(
        &self,
        resource_id: Uuid,
        content: Vec<u8>,
        filename: &str,
    ) -> Result<UploadResponse> {
        let token = auth::current_token()?;
        let form = reqwest::multipart::Form::new()
            .text("resource_id", resource_id.to_string())
            .part(
                "file",
                reqwest::multipart::Part::bytes(content).file_name(filename.to_string()),
            );
        let req = self.http.post("/api/upload").multipart(form);
        self.http
            .send_json(&Method::POST, "/api/upload", req, Some(&token))
            .await
    }
}
