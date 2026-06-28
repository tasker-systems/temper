//! Typed sub-client for the `/api/cognitive-maps` endpoints.
//!
//! Reconcile is the L0 delivery & lifecycle write path: the operator embeds a committed manifest
//! client-side and PUTs a PRE-EMBEDDED desired-state request. The server diffs it against the map's
//! current `provenance: kernel` slice and applies additive-only mutations.

use reqwest::Method;
use uuid::Uuid;

use crate::error::Result;
use crate::http::HttpClient;
use temper_core::types::cognitive_maps::CogmapRegionRow;
use temper_core::types::reconcile::{ReconcileCogmapRequest, ReconcileOutcome};

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
    ) -> Result<ReconcileOutcome> {
        let token = self.http.resolve_token()?;
        let path = format!("/api/cognitive-maps/{cogmap_id}");
        let req = self.http.put(&path).json(payload);
        self.http
            .send_json(&Method::PUT, &path, req, Some(&token))
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
}

/// `/api/cognitive-maps/{id}/shape` with an optional `?lens=` query — shared by the method and its test.
fn shape_path(cogmap_id: Uuid, lens: Option<Uuid>) -> String {
    let base = format!("/api/cognitive-maps/{cogmap_id}/shape");
    match lens {
        Some(l) => format!("{base}?lens={l}"),
        None => base,
    }
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
}
