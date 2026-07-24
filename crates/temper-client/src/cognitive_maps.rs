//! Typed sub-client for the `/api/cognitive-maps` endpoints.
//!
//! Reconcile is the L0 delivery & lifecycle write path: the operator embeds a committed manifest
//! client-side and PUTs a PRE-EMBEDDED desired-state request. The server diffs it against the map's
//! current `provenance: kernel` slice and applies additive-only mutations.

use reqwest::Method;
use uuid::Uuid;

use crate::error::Result;
use crate::http::HttpClient;
use temper_core::types::cognitive_maps::{
    BindTeamOutcome, BindTeamRequest, CogmapAnalyticsRow, CogmapDetail, CogmapGrantBody,
    CogmapRegionMetricsRow, CogmapRegionRow, CogmapRevokeBody, CogmapRow, GrantOutcome,
    RevokeOutcome, UnbindTeamOutcome,
};
use temper_core::types::materialize::{MaterializeAck, MaterializeDelta, MaterializeRequest};
use temper_core::types::reconcile::{
    CreateCogmapOutcome, CreateCogmapRequest, ReconcileCogmapRequest, ReconcileOutcome,
};

/// Sub-client for cognitive-map operations.
pub struct CognitiveMapClient<'a> {
    http: &'a HttpClient,
}

impl std::fmt::Debug for CognitiveMapClient<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CognitiveMapClient").finish_non_exhaustive()
    }
}

impl<'a> CognitiveMapClient<'a> {
    pub(crate) fn new(http: &'a HttpClient) -> Self {
        Self { http }
    }

    /// PUT /api/cognitive-maps/{id} — reconcile a cognitive map's content to the desired manifest
    /// (admin-gated, idempotent). Returns the run outcome (`created`/`updated`/`folded`/`unchanged`).
    pub async fn reconcile_cognitive_map(
        &self,
        cogmap_id: Uuid,
        payload: &ReconcileCogmapRequest,
        act: &temper_core::types::authorship::ActInput,
    ) -> Result<ReconcileOutcome> {
        let token = self.http.resolve_token()?;
        let path = format!("/api/cognitive-maps/{cogmap_id}");
        // The manifest body stays pure; authorship rides query params. An empty `ActInput`
        // serializes to nothing and appends no query string.
        let req = self.http.put(&path).json(payload).query(act);
        self.http
            .send_json(&Method::PUT, &path, req, Some(&token))
            .await
    }

    /// POST /api/cognitive-maps — genesis (create) a new cognitive map (cogmap + telos charter
    /// resource) from a manifest (admin-gated, idempotent at a given id). Returns the realized identity
    /// plus whether this call created it (`created: false` ⇒ the map already existed).
    pub async fn create_cognitive_map(
        &self,
        payload: &CreateCogmapRequest,
    ) -> Result<CreateCogmapOutcome> {
        let token = self.http.resolve_token()?;
        let path = "/api/cognitive-maps";
        let req = self.http.post(path).json(payload);
        self.http
            .send_json(&Method::POST, path, req, Some(&token))
            .await
    }

    /// GET /api/cognitive-maps — list every cognitive map visible to the authenticated principal, each
    /// with identity + charter statement. Self-scoped server-side (empty list when the caller can see
    /// no maps); never 404.
    pub async fn list(&self) -> Result<Vec<CogmapRow>> {
        let token = self.http.resolve_token()?;
        let path = "/api/cognitive-maps";
        let req = self.http.get(path);
        self.http
            .send_json(&Method::GET, path, req, Some(&token))
            .await
    }

    /// GET /api/cognitive-maps/{id} — one map's full orientation: identity, charter blocks, and
    /// foundational (homed) resources with the telos flagged. 404 if the map is not found or not
    /// readable by the caller.
    pub async fn show(&self, cogmap_id: Uuid) -> Result<CogmapDetail> {
        let token = self.http.resolve_token()?;
        let path = format!("/api/cognitive-maps/{cogmap_id}");
        let req = self.http.get(&path);
        self.http
            .send_json(&Method::GET, &path, req, Some(&token))
            .await
    }

    /// GET /api/cognitive-maps/{id}/shape[?lens=] — the surface-tier read of a map's materialized
    /// regions. Returns the non-folded regions visible to the authenticated principal (empty if the
    /// principal cannot read the map).
    pub async fn shape(
        &self,
        cogmap_id: Uuid,
        lens_id: Option<Uuid>,
    ) -> Result<Vec<CogmapRegionRow>> {
        let token = self.http.resolve_token()?;
        let path = shape_path(cogmap_id, lens_id);
        let req = self.http.get(&path);
        self.http
            .send_json(&Method::GET, &path, req, Some(&token))
            .await
    }

    /// GET /api/cognitive-maps/{id}/materialize-delta[?threshold=] — how many formation events have
    /// landed since the last materialize, and whether that clears the threshold. 404 if the map is not
    /// readable by the caller.
    pub async fn materialize_delta(
        &self,
        cogmap_id: Uuid,
        threshold: Option<i64>,
    ) -> Result<MaterializeDelta> {
        let token = self.http.resolve_token()?;
        let path = materialize_delta_path(cogmap_id, threshold);
        let req = self.http.get(&path);
        self.http
            .send_json(&Method::GET, &path, req, Some(&token))
            .await
    }

    /// POST /api/cognitive-maps/{id}/materialize — re-materialize the map when its delta clears the
    /// threshold; a no-op below (`materialized: false`). Requires cogmap-write.
    pub async fn materialize(
        &self,
        cogmap_id: Uuid,
        threshold: Option<i64>,
    ) -> Result<MaterializeAck> {
        let token = self.http.resolve_token()?;
        let path = format!("/api/cognitive-maps/{cogmap_id}/materialize");
        let body = MaterializeRequest { threshold };
        let req = self.http.post(&path).json(&body);
        self.http
            .send_json(&Method::POST, &path, req, Some(&token))
            .await
    }

    /// GET /api/cognitive-maps/{id}/region-metrics[?lens=] — the per-region analytics tier (the five
    /// scalar metrics). Empty if the principal cannot read the map.
    pub async fn region_metrics(
        &self,
        cogmap_id: Uuid,
        lens_id: Option<Uuid>,
    ) -> Result<Vec<CogmapRegionMetricsRow>> {
        let token = self.http.resolve_token()?;
        let path = region_metrics_path(cogmap_id, lens_id);
        let req = self.http.get(&path);
        self.http
            .send_json(&Method::GET, &path, req, Some(&token))
            .await
    }

    /// GET /api/cognitive-maps/{id}/analytics — the map-level analytics picture (telos id, staleness,
    /// regulation). 404 if the map is not found or not readable.
    pub async fn analytics(&self, cogmap_id: Uuid) -> Result<CogmapAnalyticsRow> {
        let token = self.http.resolve_token()?;
        let path = analytics_path(cogmap_id);
        let req = self.http.get(&path);
        self.http
            .send_json(&Method::GET, &path, req, Some(&token))
            .await
    }

    /// POST /api/cognitive-maps/{id}/teams — bind the map to a team (admin-gated, idempotent).
    /// Returns whether this call inserted the binding (`bound: false` ⇒ it already existed).
    pub async fn bind_team(
        &self,
        cogmap_id: Uuid,
        body: &BindTeamRequest,
    ) -> Result<BindTeamOutcome> {
        let token = self.http.resolve_token()?;
        let path = format!("/api/cognitive-maps/{cogmap_id}/teams");
        let req = self.http.post(&path).json(body);
        self.http
            .send_json(&Method::POST, &path, req, Some(&token))
            .await
    }

    /// DELETE /api/cognitive-maps/{id}/teams/{team_id} — unbind the map from a team (admin-gated,
    /// no-op safe). Returns whether this call deleted a binding (`unbound: false` ⇒ none existed).
    pub async fn unbind_team(&self, cogmap_id: Uuid, team_id: Uuid) -> Result<UnbindTeamOutcome> {
        let token = self.http.resolve_token()?;
        let path = format!("/api/cognitive-maps/{cogmap_id}/teams/{team_id}");
        let req = self.http.delete(&path);
        self.http
            .send_json(&Method::DELETE, &path, req, Some(&token))
            .await
    }

    /// POST /api/cognitive-maps/{id}/grants — mint/update a capability grant on the map
    /// (admin or a can_grant holder). `granted: false` ⇒ an existing grant was updated in place.
    pub async fn grant(&self, cogmap_id: Uuid, body: &CogmapGrantBody) -> Result<GrantOutcome> {
        let token = self.http.resolve_token()?;
        let path = format!("/api/cognitive-maps/{cogmap_id}/grants");
        let req = self.http.post(&path).json(body);
        self.http
            .send_json(&Method::POST, &path, req, Some(&token))
            .await
    }

    /// DELETE /api/cognitive-maps/{id}/grants — revoke a capability grant (no-op safe).
    /// `revoked: false` ⇒ no matching grant existed.
    pub async fn revoke(&self, cogmap_id: Uuid, body: &CogmapRevokeBody) -> Result<RevokeOutcome> {
        let token = self.http.resolve_token()?;
        let path = format!("/api/cognitive-maps/{cogmap_id}/grants");
        let req = self.http.delete(&path).json(body);
        self.http
            .send_json(&Method::DELETE, &path, req, Some(&token))
            .await
    }
}

/// `/api/cognitive-maps/{id}/shape` with an optional `?lens=` query — shared by the method and its test.
fn shape_path(cogmap_id: Uuid, lens: Option<Uuid>) -> String {
    let base = format!("/api/cognitive-maps/{cogmap_id}/shape");
    match lens {
        Some(l) => format!("{base}?lens={l}"),
        None => base,
    }
}

/// `/api/cognitive-maps/{id}/materialize-delta` with an optional `?threshold=` query — shared by the
/// method and its test.
fn materialize_delta_path(cogmap_id: Uuid, threshold: Option<i64>) -> String {
    let base = format!("/api/cognitive-maps/{cogmap_id}/materialize-delta");
    match threshold {
        Some(t) => format!("{base}?threshold={t}"),
        None => base,
    }
}

/// `/api/cognitive-maps/{id}/region-metrics` with an optional `?lens=` query.
fn region_metrics_path(cogmap_id: Uuid, lens: Option<Uuid>) -> String {
    let base = format!("/api/cognitive-maps/{cogmap_id}/region-metrics");
    match lens {
        Some(l) => format!("{base}?lens={l}"),
        None => base,
    }
}

/// `/api/cognitive-maps/{id}/analytics`.
fn analytics_path(cogmap_id: Uuid) -> String {
    format!("/api/cognitive-maps/{cogmap_id}/analytics")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shape_path_omits_lens_when_none() {
        let id = Uuid::from_u128(7);
        assert_eq!(
            shape_path(id, None),
            format!("/api/cognitive-maps/{id}/shape")
        );
    }

    #[test]
    fn shape_path_includes_lens_when_some() {
        let id = Uuid::from_u128(7);
        let lens = Uuid::from_u128(9);
        assert_eq!(
            shape_path(id, Some(lens)),
            format!("/api/cognitive-maps/{id}/shape?lens={lens}")
        );
    }

    #[test]
    fn region_metrics_path_omits_and_includes_lens() {
        let id = Uuid::from_u128(7);
        assert_eq!(
            region_metrics_path(id, None),
            format!("/api/cognitive-maps/{id}/region-metrics")
        );
        let lens = Uuid::from_u128(9);
        assert_eq!(
            region_metrics_path(id, Some(lens)),
            format!("/api/cognitive-maps/{id}/region-metrics?lens={lens}")
        );
    }

    #[test]
    fn analytics_path_is_plain() {
        let id = Uuid::from_u128(7);
        assert_eq!(
            analytics_path(id),
            format!("/api/cognitive-maps/{id}/analytics")
        );
    }

    #[test]
    fn materialize_delta_path_omits_and_includes_threshold() {
        let id = Uuid::from_u128(7);
        assert_eq!(
            materialize_delta_path(id, None),
            format!("/api/cognitive-maps/{id}/materialize-delta")
        );
        assert_eq!(
            materialize_delta_path(id, Some(5)),
            format!("/api/cognitive-maps/{id}/materialize-delta?threshold=5")
        );
    }
}
