//! Typed sub-client for the `/api/resources` endpoints.

use reqwest::Method;
use uuid::Uuid;

use crate::error::Result;
use crate::http::HttpClient;
use temper_core::types::resource::{
    ContentResponse, DeleteResponse, ResourceCreateRequest, ResourceListParams,
    ResourceListResponse, ResourceRow, ResourceUpdateRequest,
};

/// Sub-client for resource CRUD operations.
pub struct ResourceClient<'a> {
    http: &'a HttpClient,
}

impl std::fmt::Debug for ResourceClient<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ResourceClient").finish_non_exhaustive()
    }
}

impl<'a> ResourceClient<'a> {
    pub(crate) fn new(http: &'a HttpClient) -> Self {
        Self { http }
    }

    /// List visible resources, optionally filtered by context.
    pub async fn list(&self, params: &ResourceListParams) -> Result<ResourceListResponse> {
        let token = self.http.resolve_token()?;
        let req = self.http.get("/api/resources").query(params);
        self.http
            .send_json(&Method::GET, "/api/resources", req, Some(&token))
            .await
    }

    /// Get a single resource by ID.
    pub async fn get(&self, id: Uuid) -> Result<ResourceRow> {
        let token = self.http.resolve_token()?;
        let path = format!("/api/resources/{id}");
        let req = self.http.get(&path);
        self.http
            .send_json(&Method::GET, &path, req, Some(&token))
            .await
    }

    /// Create a new resource.
    pub async fn create(&self, request: &ResourceCreateRequest) -> Result<ResourceRow> {
        let token = self.http.resolve_token()?;
        let req = self.http.post("/api/resources").json(request);
        self.http
            .send_json(&Method::POST, "/api/resources", req, Some(&token))
            .await
    }

    /// Update an existing resource.
    pub async fn update(&self, id: Uuid, request: &ResourceUpdateRequest) -> Result<ResourceRow> {
        let token = self.http.resolve_token()?;
        let path = format!("/api/resources/{id}");
        let req = self.http.patch(&path).json(request);
        self.http
            .send_json(&Method::PATCH, &path, req, Some(&token))
            .await
    }

    /// Delete a resource.
    pub async fn delete(&self, id: Uuid) -> Result<DeleteResponse> {
        let token = self.http.resolve_token()?;
        let path = format!("/api/resources/{id}");
        let req = self.http.delete(&path);
        self.http
            .send_json(&Method::DELETE, &path, req, Some(&token))
            .await
    }

    /// Get the reconstituted markdown content for a resource.
    pub async fn content(&self, id: Uuid) -> Result<ContentResponse> {
        let token = self.http.resolve_token()?;
        let path = format!("/api/resources/{id}/content");
        let req = self.http.get(&path);
        self.http
            .send_json(&Method::GET, &path, req, Some(&token))
            .await
    }
}
