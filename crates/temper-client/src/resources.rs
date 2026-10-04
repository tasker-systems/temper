//! Typed sub-client for the `/api/resources` endpoints.

use reqwest::StatusCode;
use uuid::Uuid;

use crate::error::{ClientError, Result};
use crate::http::HttpClient;
use crate::ops;
use temper_core::types::citation_audit::{BlockCitationAuditRequest, CitationAuditRequest};
use temper_core::types::cognitive_maps::{GrantOutcome, RevokeOutcome};
use temper_core::types::lineage::ResourceLineage;
use temper_core::types::provenance::{BlockProvenanceRow, BlockRead};
use temper_core::types::reassign::{ReassignAck, ReassignResourceRequest};
use temper_core::types::resource_grant::{ResourceGrantBody, ResourceRevokeBody};
use temper_core::types::resource_view::{ResourceSection, ResourceView, SectionSet};
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
        let op = &ops::LIST_RESOURCES;
        let path = op.path(&[]);
        let req = self.http.request(op, &path).query(params);
        self.http
            .send_json(&op.method(), &path, req, Some(&token))
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
        let op = &ops::LIST_RESOURCES;
        let path = op.path(&[]);
        let req = self.http.request(op, &path).query(&params);
        self.http
            .send_json(&op.method(), &path, req, Some(&token))
            .await
    }

    /// Get a single resource by ID, with both metadata tiers.
    ///
    /// The same [`ResourceView`] a `list` row is — `show` asks for the `open-meta` section, a
    /// default `list` does not, and that is the whole difference.
    ///
    /// `sections` is additive: the door unions the named sections onto its `open-meta`
    /// baseline, so `None` (or an empty set) answers with the incumbent shape. The set renders
    /// through [`SectionSet::to_csv`], whose `None` for the empty set is what keeps "no extra
    /// sections" from becoming an empty `?sections=` parameter.
    pub async fn get(&self, id: Uuid, sections: Option<&SectionSet>) -> Result<ResourceView> {
        let token = self.http.resolve_token()?;
        let op = &ops::GET_RESOURCE;
        let base = op.path(&[&id]);
        let path = match sections.and_then(SectionSet::to_csv) {
            Some(csv) => format!("{base}?sections={csv}"),
            None => base,
        };
        let req = self.http.request(op, &path);
        self.http
            .send_json(&op.method(), &path, req, Some(&token))
            .await
    }

    /// Create a new resource.
    pub async fn create(&self, request: &ResourceCreateRequest) -> Result<ResourceView> {
        let token = self.http.resolve_token()?;
        let op = &ops::CREATE_RESOURCE;
        let path = op.path(&[]);
        let req = self.http.request(op, &path).json(request);
        self.http
            .send_json(&op.method(), &path, req, Some(&token))
            .await
    }

    /// Update an existing resource.
    pub async fn update(&self, id: Uuid, request: &ResourceUpdateRequest) -> Result<ResourceView> {
        let token = self.http.resolve_token()?;
        let op = &ops::UPDATE_RESOURCE;
        let path = op.path(&[&id]);
        let req = self.http.request(op, &path).json(request);
        self.http
            .send_json(&op.method(), &path, req, Some(&token))
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
        let op = &ops::ANNOTATE_RESOURCE;
        let path = op.path(&[&id]);
        let req = self.http.request(op, &path).json(request);
        self.http
            .send_json(&op.method(), &path, req, Some(&token))
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
        let op = &ops::DELETE_RESOURCE;
        let path = op.path(&[&id]);
        let req = self.http.request(op, &path).query(act);
        self.http
            .send_json(&op.method(), &path, req, Some(&token))
            .await
    }

    /// POST /api/resources/{id}/grants — mint/update a capability grant on the resource
    /// (system-admin, a can_grant holder, OR the resource owner). `granted: false` ⇒ an
    /// existing grant was updated in place.
    pub async fn grant(&self, id: Uuid, body: &ResourceGrantBody) -> Result<GrantOutcome> {
        let token = self.http.resolve_token()?;
        let op = &ops::GRANT_RESOURCE_ACCESS;
        let path = op.path(&[&id]);
        let req = self.http.request(op, &path).json(body);
        self.http
            .send_json(&op.method(), &path, req, Some(&token))
            .await
    }

    /// DELETE /api/resources/{id}/grants — revoke a capability grant (no-op safe).
    /// `revoked: false` ⇒ no matching grant existed.
    pub async fn revoke(&self, id: Uuid, body: &ResourceRevokeBody) -> Result<RevokeOutcome> {
        let token = self.http.resolve_token()?;
        let op = &ops::REVOKE_RESOURCE_ACCESS;
        let path = op.path(&[&id]);
        let req = self.http.request(op, &path).json(body);
        self.http
            .send_json(&op.method(), &path, req, Some(&token))
            .await
    }

    /// POST /api/resources/{id}/reassign — reassign a resource's owner/team.
    pub async fn reassign(&self, id: Uuid, body: &ReassignResourceRequest) -> Result<ReassignAck> {
        let token = self.http.resolve_token()?;
        let op = &ops::REASSIGN_RESOURCE;
        let path = op.path(&[&id]);
        let req = self.http.request(op, &path).json(body);
        self.http
            .send_json(&op.method(), &path, req, Some(&token))
            .await
    }

    /// List edges connected to a resource.
    pub async fn edges(&self, resource_id: Uuid) -> Result<Vec<GraphEdgeRow>> {
        let token = self.http.resolve_token()?;
        let op = &ops::LIST_RESOURCE_EDGES;
        let path = op.path(&[&resource_id]);
        let req = self.http.request(op, &path);
        self.http
            .send_json(&op.method(), &path, req, Some(&token))
            .await
    }

    /// Read a resource's bidirectional `derived_from` lineage (ancestors +
    /// descendants), access-gated. `depth` bounds the walk when supplied.
    pub async fn lineage(&self, resource_id: Uuid, depth: Option<i32>) -> Result<ResourceLineage> {
        let token = self.http.resolve_token()?;
        let op = &ops::RESOURCE_LINEAGE;
        let base = op.path(&[&resource_id]);
        let path = match depth {
            Some(d) => format!("{base}?depth={d}"),
            None => base,
        };
        let req = self.http.request(op, &path);
        self.http
            .send_json(&op.method(), &path, req, Some(&token))
            .await
    }

    /// Get the itemized per-block provenance for a resource.
    pub async fn provenance(&self, resource_id: Uuid) -> Result<Vec<BlockProvenanceRow>> {
        let token = self.http.resolve_token()?;
        let op = &ops::PROVENANCE;
        let path = op.path(&[&resource_id]);
        let req = self.http.request(op, &path);
        self.http
            .send_json(&op.method(), &path, req, Some(&token))
            .await
    }

    /// Read one content block by address — the three-state resolution (D-D1): the
    /// returned [`BlockRead`] states `live`, `folded`, or `absent` BY NAME.
    ///
    /// `GET /api/resources/{id}/blocks/{block_id}`. `200` and `410 Gone` carry the
    /// same `BlockRead` envelope — the `state` tag distinguishes them — so both
    /// parse as data (the 410 rides [`HttpClient::send_admitting`], which returns
    /// the body the plain error mapping would discard). The route's own block-naming
    /// `404` — no such block under a visible home — synthesizes [`BlockRead::Absent`].
    /// A home resource the caller cannot see answers the resource-naming `404`, which
    /// stays [`ClientError::NotFound`], denying existence. A home resource that was
    /// erased answers `410` under `RESOURCE_ERASED` to a caller who held it, and that
    /// comes back as [`ClientError::ResourceErased`]. Every other failure (auth,
    /// transport, 5xx) stays an `Err`, exactly as the sibling reads report.
    pub async fn read_block(&self, resource_id: Uuid, block_id: Uuid) -> Result<BlockRead> {
        let token = self.http.resolve_token()?;
        let op = &ops::READ_BLOCK;
        let path = op.path(&[&resource_id, &block_id]);
        let req = self.http.request(op, &path);
        match self
            .http
            .send_admitting(&op.method(), &path, req, Some(&token), StatusCode::GONE)
            .await
        {
            Ok(resp) => {
                let status = resp.status();
                let bytes = resp.bytes().await?;
                // The admitted 410 is not always a `BlockRead`: the server answers an erased
                // HOME resource's holder 410 too, under `RESOURCE_ERASED` and the error envelope
                // (`substrate_read::block_read_select` classifies its miss through `erased_or`).
                // The code is checked BEFORE the parse — otherwise the erasure surfaces as a JSON
                // error about a missing `state` tag, and the one answer that names it is lost.
                if status == StatusCode::GONE {
                    let body = String::from_utf8_lossy(&bytes);
                    if crate::http::is_resource_erased_body(&body) {
                        return Err(crate::http::map_status_to_error(status, &body));
                    }
                }
                Ok(serde_json::from_slice(&bytes)?)
            }
            // The route's OWN 404 names the block ("content block {id} not found") — that is
            // the defined absent face. Any OTHER 404 (an unmatched route on an older server,
            // a proxy fallback) propagates as an error: synthesizing `absent` from it would
            // tell a caller "no such row, ever" about a block the skewing server simply
            // cannot address — the one lie in the worst direction.
            Err(ClientError::NotFound { message })
                if message.contains(&block_id.to_string()) && message.contains("not found") =>
            {
                Ok(BlockRead::Absent { block_id })
            }
            Err(e) => Err(e),
        }
    }

    /// Read a resource's evidential-standing shape (the shape vector + lossy band chip),
    /// access-gated. GET /api/resources/{id}/evidence.
    pub async fn evidence(&self, resource_id: Uuid) -> Result<StandingShape> {
        let token = self.http.resolve_token()?;
        let op = &ops::RESOURCE_EVIDENCE;
        let path = op.path(&[&resource_id]);
        let req = self.http.request(op, &path);
        self.http
            .send_json(&op.method(), &path, req, Some(&token))
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
        let op = &ops::RECORD_CITATION_AUDIT;
        let path = op.path(&[&resource_id]);
        let req = self.http.request(op, &path).json(request);
        self.http
            .send_json(&op.method(), &path, req, Some(&token))
            .await
    }

    /// Record an auditor's signed verdict by block address alone — the block-addressed audit
    /// write beside [`Self::record_citation_audit`].
    ///
    /// POST /api/citation-audits, returning the new `kb_citation_audits.id`. There is no finding
    /// argument and none may be added: the server derives the authorization subject from
    /// `request.block_id`, so a caller can only ever audit the citation it addresses. The request
    /// carries the act envelope, and the write keeps it — authorship and correlation ride the
    /// ledger row; only `value` moves standing.
    pub async fn record_citation_audit_for_block(
        &self,
        request: &BlockCitationAuditRequest,
    ) -> Result<Uuid> {
        let token = self.http.resolve_token()?;
        let op = &ops::RECORD_CITATION_AUDIT_FOR_BLOCK;
        let path = op.path(&[]);
        let req = self.http.request(op, &path).json(request);
        self.http
            .send_json(&op.method(), &path, req, Some(&token))
            .await
    }

    /// Get the reconstituted markdown content for a resource.
    pub async fn content(&self, id: Uuid) -> Result<ContentResponse> {
        let token = self.http.resolve_token()?;
        let op = &ops::GET_CONTENT;
        let path = op.path(&[&id]);
        let req = self.http.request(op, &path);
        self.http
            .send_json(&op.method(), &path, req, Some(&token))
            .await
    }

    /// GET /api/resources/{id}/meta — fetch just the manifest meta tier
    /// (managed_meta, open_meta, managed_hash, open_hash) without
    /// reconstructing markdown from chunks. Used by the metadata-only
    /// sync pull path to avoid paying for server-side body reconstruction
    /// when only the meta side has drifted.
    pub async fn get_meta(&self, id: Uuid) -> Result<ResourceView> {
        let token = self.http.resolve_token()?;
        let op = &ops::GET_META;
        let path = op.path(&[&id]);
        let req = self.http.request(op, &path);
        self.http
            .send_json(&op.method(), &path, req, Some(&token))
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
        let op = &ops::UPDATE_META;
        let path = op.path(&[&id]);
        let req = self.http.request(op, &path).json(payload);
        self.http
            .send_json(&op.method(), &path, req, Some(&token))
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
