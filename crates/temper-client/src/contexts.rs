//! Typed sub-client for the `/api/contexts` endpoints.

use reqwest::Method;
use uuid::Uuid;

use crate::error::Result;
use crate::http::HttpClient;
use temper_core::context_ref::ContextOwnerRef;
use temper_core::types::cognitive_maps::{CogmapRegionMetricsRow, CogmapRegionRow};
use temper_core::types::context::{
    ContextCreateRequest, ContextRow, ContextRowWithCounts, ShareContextOutcome,
    ShareContextRequest, UnshareContextOutcome,
};
use temper_core::types::materialize::{MaterializeAck, MaterializeRequest};

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

    /// Create a new context. `owner` is `None` for a profile-owned context (the
    /// default) or `Some(ContextOwnerRef::Team(slug))` for a team-owned one
    /// (role-gated server-side).
    pub async fn create(&self, name: &str, owner: Option<ContextOwnerRef>) -> Result<ContextRow> {
        let token = self.http.resolve_token()?;
        let body = ContextCreateRequest {
            name: name.to_owned(),
            owner,
        };
        let req = self.http.post("/api/contexts").json(&body);
        self.http
            .send_json(&Method::POST, "/api/contexts", req, Some(&token))
            .await
    }

    /// POST /api/contexts/{id}/teams — share the context into a team (admin-gated, idempotent).
    pub async fn share_team(
        &self,
        context_id: Uuid,
        body: &ShareContextRequest,
    ) -> Result<ShareContextOutcome> {
        let token = self.http.resolve_token()?;
        let path = format!("/api/contexts/{context_id}/teams");
        let req = self.http.post(&path).json(body);
        self.http
            .send_json(&Method::POST, &path, req, Some(&token))
            .await
    }

    /// DELETE /api/contexts/{id}/teams/{team_id} — unshare (admin-gated, no-op safe).
    pub async fn unshare_team(
        &self,
        context_id: Uuid,
        team_id: Uuid,
    ) -> Result<UnshareContextOutcome> {
        let token = self.http.resolve_token()?;
        let path = format!("/api/contexts/{context_id}/teams/{team_id}");
        let req = self.http.delete(&path);
        self.http
            .send_json(&Method::DELETE, &path, req, Some(&token))
            .await
    }
}

// ── Context orientation reads (spec §3.7, T8) ────────────────────────────────
//
// The peers of `CognitiveMapClient`'s shape / region-metrics / materialize. The response types are
// shared with the cogmap side on purpose: a region row carries nothing cogmap-specific, so
// `CogmapRegionRow` describes a context's region exactly as well. The `cogmap_*` naming is what M3
// retires, not the shape.

impl ContextClient<'_> {
    /// GET `/api/contexts/{id}/shape[?lens=]` — the context's materialized regions (surface tier),
    /// most salient first. Empty if the caller cannot read the context (gate is in the SQL — no
    /// existence oracle), which is also what an un-materialized context returns.
    pub async fn shape(
        &self,
        context_id: Uuid,
        lens: Option<Uuid>,
    ) -> Result<Vec<CogmapRegionRow>> {
        let token = self.http.resolve_token()?;
        let path = context_shape_path(context_id, lens);
        let req = self.http.get(&path);
        self.http
            .send_json(&Method::GET, &path, req, Some(&token))
            .await
    }

    /// GET `/api/contexts/{id}/region-metrics[?lens=]` — the per-region analytics tier.
    pub async fn region_metrics(
        &self,
        context_id: Uuid,
        lens: Option<Uuid>,
    ) -> Result<Vec<CogmapRegionMetricsRow>> {
        let token = self.http.resolve_token()?;
        let path = context_region_metrics_path(context_id, lens);
        let req = self.http.get(&path);
        self.http
            .send_json(&Method::GET, &path, req, Some(&token))
            .await
    }

    /// POST `/api/contexts/{id}/materialize` — re-form the context's regions when its formation delta
    /// clears the threshold; an idempotent no-op below it (`materialized: false`). Requires write on
    /// the context.
    pub async fn materialize(
        &self,
        context_id: Uuid,
        threshold: Option<i64>,
    ) -> Result<MaterializeAck> {
        let token = self.http.resolve_token()?;
        let path = format!("/api/contexts/{context_id}/materialize");
        let body = MaterializeRequest { threshold };
        let req = self.http.post(&path).json(&body);
        self.http
            .send_json(&Method::POST, &path, req, Some(&token))
            .await
    }
}

/// `/api/contexts/{id}/shape` with an optional `?lens=` query — shared by the method and its test.
fn context_shape_path(context_id: Uuid, lens: Option<Uuid>) -> String {
    let base = format!("/api/contexts/{context_id}/shape");
    match lens {
        Some(l) => format!("{base}?lens={l}"),
        None => base,
    }
}

/// `/api/contexts/{id}/region-metrics` with an optional `?lens=` query.
fn context_region_metrics_path(context_id: Uuid, lens: Option<Uuid>) -> String {
    let base = format!("/api/contexts/{context_id}/region-metrics");
    match lens {
        Some(l) => format!("{base}?lens={l}"),
        None => base,
    }
}

#[cfg(test)]
mod orientation_path_tests {
    use super::*;

    #[test]
    fn shape_path_appends_lens_only_when_present() {
        let ctx = Uuid::nil();
        assert_eq!(
            context_shape_path(ctx, None),
            "/api/contexts/00000000-0000-0000-0000-000000000000/shape"
        );
        assert_eq!(
            context_shape_path(ctx, Some(Uuid::nil())),
            "/api/contexts/00000000-0000-0000-0000-000000000000/shape?lens=00000000-0000-0000-0000-000000000000"
        );
    }

    #[test]
    fn region_metrics_path_appends_lens_only_when_present() {
        let ctx = Uuid::nil();
        assert_eq!(
            context_region_metrics_path(ctx, None),
            "/api/contexts/00000000-0000-0000-0000-000000000000/region-metrics"
        );
        assert!(context_region_metrics_path(ctx, Some(Uuid::nil())).contains("?lens="));
    }
}
