//! Typed sub-client for the `/api/resources` endpoints.

use reqwest::Method;
use uuid::Uuid;

use crate::error::Result;
use crate::http::HttpClient;
use temper_core::types::graph::GraphEdgeRow;
use temper_core::types::managed_meta::{
    MetaUpdatePayload, ResourceMetaListResponse, ResourceMetaResponse,
};
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

    /// List visible resources with meta projection (`Vec<ResourceMetaResponse>`
    /// rows). Sibling of [`ResourceClient::list`]; forces
    /// `meta_only=true` on the wire.
    pub async fn list_meta(&self, params: &ResourceListParams) -> Result<ResourceMetaListResponse> {
        let mut params = params.clone();
        params.meta_only = Some(true);
        let token = self.http.resolve_token()?;
        let req = self.http.get("/api/resources").query(&params);
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

    /// List edges connected to a resource.
    pub async fn edges(&self, resource_id: Uuid) -> Result<Vec<GraphEdgeRow>> {
        let token = self.http.resolve_token()?;
        let path = format!("/api/resources/{resource_id}/edges");
        let req = self.http.get(&path);
        self.http
            .send_json(&Method::GET, &path, req, Some(&token))
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

    /// GET /api/resources/{id}/meta — fetch just the manifest meta tier
    /// (managed_meta, open_meta, managed_hash, open_hash) without
    /// reconstructing markdown from chunks. Used by the metadata-only
    /// sync pull path to avoid paying for server-side body reconstruction
    /// when only the meta side has drifted.
    pub async fn get_meta(&self, id: Uuid) -> Result<ResourceMetaResponse> {
        let token = self.http.resolve_token()?;
        let path = format!("/api/resources/{id}/meta");
        let req = self.http.get(&path);
        self.http
            .send_json(&Method::GET, &path, req, Some(&token))
            .await
    }

    /// PUT /api/resources/{id}/meta — update managed_meta and open_meta
    /// without re-chunking. Used by the metadata-only sync path.
    ///
    /// The server reconciles frontmatter-provenance edges from the new
    /// open_meta on success; errors during reconciliation are logged
    /// server-side and do not fail this call.
    pub async fn update_meta(
        &self,
        id: Uuid,
        payload: &MetaUpdatePayload,
    ) -> Result<serde_json::Value> {
        let token = self.http.resolve_token()?;
        let path = format!("/api/resources/{id}/meta");
        let req = self.http.put(&path).json(payload);
        self.http
            .send_json(&Method::PUT, &path, req, Some(&token))
            .await
    }
}

#[cfg(test)]
mod meta_list_tests {
    use super::*;

    // Signature-level guard: confirms list_meta exists with the
    // expected types. Use a named helper (not a closure) to avoid
    // 'fn pointer lifetime' constraints; this still fails to compile
    // if the signature drifts.
    fn _assert_callable<'a>(
        client: &'a ResourceClient<'a>,
        params: &'a temper_core::types::resource::ResourceListParams,
    ) -> impl std::future::Future<
        Output = crate::error::Result<temper_core::types::managed_meta::ResourceMetaListResponse>,
    > + 'a {
        client.list_meta(params)
    }

    #[test]
    fn list_meta_signature_check() {
        // Compile-time only.
    }
}
