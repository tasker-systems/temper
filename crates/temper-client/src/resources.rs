//! Typed sub-client for the `/api/resources` endpoints.

use reqwest::Method;
use uuid::Uuid;

use crate::error::Result;
use crate::http::HttpClient;
use temper_core::types::citation_audit::CitationAuditRequest;
use temper_core::types::cognitive_maps::{GrantOutcome, RevokeOutcome};
use temper_core::types::lineage::ResourceLineage;
use temper_core::types::provenance::BlockProvenanceRow;
use temper_core::types::reassign::{ReassignAck, ReassignResourceRequest};
use temper_core::types::resource_grant::{ResourceGrantBody, ResourceRevokeBody};
use temper_core::types::resource_view::{ResourceSection, ResourceView};
use temper_core::types::standing::StandingShape;
use temper_workflow::types::graph::GraphEdgeRow;
use temper_workflow::types::managed_meta::MetaUpdatePayload;
use temper_workflow::types::resource::{
    ContentResponse, DeleteResponse, ResourceAnnotateRequest, ResourceCreateRequest,
    ResourceListParams, ResourceListResponse, ResourceUpdateRequest,
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

    /// List visible resources with the open metadata tier filled on every row.
    ///
    /// Same endpoint, same envelope and same row type as [`ResourceClient::list`] — the only
    /// difference is that it asks for the `open-meta` section. It used to force `meta_only=true`,
    /// which selected a *second* response type (`ResourceMetaListResponse` over `ResourceDetail`);
    /// there is one shape now, so this is a section request, not a projection switch.
    pub async fn list_meta(&self, params: &ResourceListParams) -> Result<ResourceListResponse> {
        let mut params = params.clone();
        params.sections = Some(ResourceSection::OpenMeta.to_string());
        let token = self.http.resolve_token()?;
        let req = self.http.get("/api/resources").query(&params);
        self.http
            .send_json(&Method::GET, "/api/resources", req, Some(&token))
            .await
    }

    /// Get a single resource by ID, with both metadata tiers.
    ///
    /// The same [`ResourceView`] a `list` row is — `show` asks for the `open-meta` section, a
    /// default `list` does not, and that is the whole difference.
    pub async fn get(&self, id: Uuid) -> Result<ResourceView> {
        let token = self.http.resolve_token()?;
        let path = format!("/api/resources/{id}");
        let req = self.http.get(&path);
        self.http
            .send_json(&Method::GET, &path, req, Some(&token))
            .await
    }

    /// Create a new resource.
    pub async fn create(&self, request: &ResourceCreateRequest) -> Result<ResourceView> {
        let token = self.http.resolve_token()?;
        let req = self.http.post("/api/resources").json(request);
        self.http
            .send_json(&Method::POST, "/api/resources", req, Some(&token))
            .await
    }

    /// Update an existing resource.
    pub async fn update(&self, id: Uuid, request: &ResourceUpdateRequest) -> Result<ResourceView> {
        let token = self.http.resolve_token()?;
        let path = format!("/api/resources/{id}");
        let req = self.http.patch(&path).json(request);
        self.http
            .send_json(&Method::PATCH, &path, req, Some(&token))
            .await
    }

    /// Annotate a resource's block with provenance sources — no body revise (issue #355).
    ///
    /// `POST /api/resources/{id}/provenance`. Records `kb_block_provenance` rows without re-chunking
    /// or re-embedding; returns the (content-unchanged) resource row.
    pub async fn annotate(
        &self,
        id: Uuid,
        request: &ResourceAnnotateRequest,
    ) -> Result<ResourceView> {
        let token = self.http.resolve_token()?;
        let path = format!("/api/resources/{id}/provenance");
        let req = self.http.post(&path).json(request);
        self.http
            .send_json(&Method::POST, &path, req, Some(&token))
            .await
    }

    /// Delete a resource.
    ///
    /// DELETE has no body, so per-act authorship (`act`) rides query params; an empty
    /// `ActInput` serializes to nothing and appends no query string.
    pub async fn delete(
        &self,
        id: Uuid,
        act: &temper_core::types::authorship::ActInput,
    ) -> Result<DeleteResponse> {
        let token = self.http.resolve_token()?;
        let path = format!("/api/resources/{id}");
        let req = self.http.delete(&path).query(act);
        self.http
            .send_json(&Method::DELETE, &path, req, Some(&token))
            .await
    }

    /// POST /api/resources/{id}/grants — mint/update a capability grant on the resource
    /// (system-admin, a can_grant holder, OR the resource owner). `granted: false` ⇒ an
    /// existing grant was updated in place.
    pub async fn grant(&self, id: Uuid, body: &ResourceGrantBody) -> Result<GrantOutcome> {
        let token = self.http.resolve_token()?;
        let path = format!("/api/resources/{id}/grants");
        let req = self.http.post(&path).json(body);
        self.http
            .send_json(&Method::POST, &path, req, Some(&token))
            .await
    }

    /// DELETE /api/resources/{id}/grants — revoke a capability grant (no-op safe).
    /// `revoked: false` ⇒ no matching grant existed.
    pub async fn revoke(&self, id: Uuid, body: &ResourceRevokeBody) -> Result<RevokeOutcome> {
        let token = self.http.resolve_token()?;
        let path = format!("/api/resources/{id}/grants");
        let req = self.http.delete(&path).json(body);
        self.http
            .send_json(&Method::DELETE, &path, req, Some(&token))
            .await
    }

    /// POST /api/resources/{id}/reassign — reassign a resource's owner/team.
    pub async fn reassign(&self, id: Uuid, body: &ReassignResourceRequest) -> Result<ReassignAck> {
        let token = self.http.resolve_token()?;
        let path = format!("/api/resources/{id}/reassign");
        let req = self.http.post(&path).json(body);
        self.http
            .send_json(&Method::POST, &path, req, Some(&token))
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

    /// Read a resource's bidirectional `derived_from` lineage (ancestors +
    /// descendants), access-gated. `depth` bounds the walk when supplied.
    pub async fn lineage(&self, resource_id: Uuid, depth: Option<i32>) -> Result<ResourceLineage> {
        let token = self.http.resolve_token()?;
        let path = match depth {
            Some(d) => format!("/api/resources/{resource_id}/lineage?depth={d}"),
            None => format!("/api/resources/{resource_id}/lineage"),
        };
        let req = self.http.get(&path);
        self.http
            .send_json(&Method::GET, &path, req, Some(&token))
            .await
    }

    /// Get the itemized per-block provenance for a resource.
    pub async fn provenance(&self, resource_id: Uuid) -> Result<Vec<BlockProvenanceRow>> {
        let token = self.http.resolve_token()?;
        let path = format!("/api/resources/{resource_id}/provenance");
        let req = self.http.get(&path);
        self.http
            .send_json(&Method::GET, &path, req, Some(&token))
            .await
    }

    /// Read a resource's evidential-standing shape (the shape vector + lossy band chip),
    /// access-gated. GET /api/resources/{id}/evidence.
    pub async fn evidence(&self, resource_id: Uuid) -> Result<StandingShape> {
        let token = self.http.resolve_token()?;
        let path = format!("/api/resources/{resource_id}/evidence");
        let req = self.http.get(&path);
        self.http
            .send_json(&Method::GET, &path, req, Some(&token))
            .await
    }

    /// Record an auditor's signed verdict on one `(block, source)` citation of this finding.
    /// POST /api/resources/{id}/citation-audits, returning the new `kb_citation_audits.id`.
    ///
    /// `resource_id` is the routing address only: the server derives the authorization subject from
    /// `request.block_id` and refuses a block belonging to a different finding, so passing a finding
    /// here confers nothing. The gate is readability of the finding plus NOT having authored it —
    /// an audit is a claim of independence (spec §7).
    pub async fn record_citation_audit(
        &self,
        resource_id: Uuid,
        request: &CitationAuditRequest,
    ) -> Result<Uuid> {
        let token = self.http.resolve_token()?;
        let path = format!("/api/resources/{resource_id}/citation-audits");
        let req = self.http.post(&path).json(request);
        self.http
            .send_json(&Method::POST, &path, req, Some(&token))
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
    pub async fn get_meta(&self, id: Uuid) -> Result<ResourceView> {
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
    pub async fn update_meta(&self, id: Uuid, payload: &MetaUpdatePayload) -> Result<ResourceView> {
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
    // if the signature drifts. It now guards the convergence too — the
    // meta walk and the default walk return the SAME envelope type.
    fn _assert_callable<'a>(
        client: &'a ResourceClient<'a>,
        params: &'a temper_workflow::types::resource::ResourceListParams,
    ) -> impl std::future::Future<
        Output = crate::error::Result<temper_workflow::types::resource::ResourceListResponse>,
    > + 'a {
        client.list_meta(params)
    }

    #[test]
    fn list_meta_signature_check() {
        // Compile-time only.
    }
}
