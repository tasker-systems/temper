//! Typed sub-client for the `/api/contexts` endpoints.

use reqwest::Method;
use uuid::Uuid;

use crate::error::Result;
use crate::http::HttpClient;
use temper_core::types::context::{ContextCreateRequest, ContextRow, ContextRowWithCounts};

/// Sub-client for context operations.
pub struct ContextClient<'a> {
    http: &'a HttpClient,
}

impl std::fmt::Debug for ContextClient<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ContextClient").finish_non_exhaustive()
    }
}

impl<'a> ContextClient<'a> {
    pub(crate) fn new(http: &'a HttpClient) -> Self {
        Self { http }
    }

    /// List all visible contexts with resource counts.
    pub async fn list(&self) -> Result<Vec<ContextRowWithCounts>> {
        let token = self.http.resolve_token()?;
        let req = self.http.get("/api/contexts");
        self.http
            .send_json(&Method::GET, "/api/contexts", req, Some(&token))
            .await
    }

    /// Get a single context by ID.
    pub async fn get(&self, id: Uuid) -> Result<ContextRow> {
        let token = self.http.resolve_token()?;
        let path = format!("/api/contexts/{id}");
        let req = self.http.get(&path);
        self.http
            .send_json(&Method::GET, &path, req, Some(&token))
            .await
    }

    /// Create a new context.
    pub async fn create(&self, name: &str) -> Result<ContextRow> {
        let token = self.http.resolve_token()?;
        let body = ContextCreateRequest {
            name: name.to_owned(),
        };
        let req = self.http.post("/api/contexts").json(&body);
        self.http
            .send_json(&Method::POST, "/api/contexts", req, Some(&token))
            .await
    }
}
