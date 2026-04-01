//! Typed sub-client for the `/api/contexts` endpoints.

use uuid::Uuid;

use crate::auth;
use crate::error::Result;
use crate::http::HttpClient;
use temper_core::types::context::{ContextCreateRequest, ContextRow};

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

    /// List all visible contexts.
    pub async fn list(&self) -> Result<Vec<ContextRow>> {
        let token = auth::current_token()?;
        let req = self.http.get("/api/contexts");
        self.http.send_json(req, Some(&token)).await
    }

    /// Get a single context by ID.
    pub async fn get(&self, id: Uuid) -> Result<ContextRow> {
        let token = auth::current_token()?;
        let req = self.http.get(&format!("/api/contexts/{id}"));
        self.http.send_json(req, Some(&token)).await
    }

    /// Create a new context.
    pub async fn create(&self, name: &str) -> Result<ContextRow> {
        let token = auth::current_token()?;
        let body = ContextCreateRequest {
            name: name.to_owned(),
        };
        let req = self.http.post("/api/contexts").json(&body);
        self.http.send_json(req, Some(&token)).await
    }
}
