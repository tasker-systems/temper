//! Typed sub-client for the `/api/contexts` endpoints.

use uuid::Uuid;

use crate::error::Result;
use crate::http::HttpClient;
use crate::ops;
use temper_core::context_ref::ContextOwnerRef;
use temper_core::types::cognitive_maps::{AnchorShape, CogmapRegionMetricsRow, CogmapStaleness};
use temper_core::types::context::{
    ContextCreateRequest, ContextResolution, ContextRow, ContextRowWithCounts,
    ReassignContextOutcome, ReassignContextRequest, RenameContextOutcome, RenameContextRequest,
    RestoreContextOutcome, RetireContextOutcome, ShareContextOutcome, ShareContextRequest,
    UnshareContextOutcome,
};
use temper_core::types::materialize::{MaterializeAck, MaterializeDelta, MaterializeRequest};

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
        let op = &ops::LIST_CONTEXTS;
        let path = op.path(&[])?;
        let req = self.http.request(op, &path);
        self.http
            .send_json(&op.method(), &path, req, Some(&token))
            .await
    }

    /// GET `/api/contexts?retired=true` — list retired contexts the caller ADMINISTERS, not the
    /// ones they can read. This rides the admin axis, not `list`'s visibility axis: a retired
    /// context is invisible to the read predicate by construction (that is what retirement
    /// means), so it can only ever be listed by someone who could have retired it.
    pub async fn list_retired(&self) -> Result<Vec<ContextRowWithCounts>> {
        let token = self.http.resolve_token()?;
        let op = &ops::LIST_CONTEXTS;
        let path = format!("{}?retired=true", op.path(&[])?);
        let req = self.http.request(op, &path);
        self.http
            .send_json(&op.method(), &path, req, Some(&token))
            .await
    }

    /// GET `/api/contexts/resolve` — resolve a context ref (`@me/<slug>`, `@<handle>/<slug>`,
    /// `+<team>/<slug>`, or a bare UUID) to the context id, within the caller's visibility. A
    /// context the caller cannot read answers as absent (404), and a malformed ref as 400 with
    /// the parser's sentence.
    ///
    /// The ref rides the query string, not the logged path: this client records
    /// `"{method} {path}"` as an exported span attribute, and a ref names an owner and a slug.
    pub async fn resolve(&self, context_ref: &str) -> Result<ContextResolution> {
        let token = self.http.resolve_token()?;
        let op = &ops::RESOLVE_CONTEXT;
        let path = op.path(&[])?;
        let req = resolve_request(self.http, &path, context_ref);
        self.http
            .send_json(&op.method(), &path, req, Some(&token))
            .await
    }

    /// Get a single context by ID.
    pub async fn get(&self, id: Uuid) -> Result<ContextRow> {
        let token = self.http.resolve_token()?;
        let op = &ops::GET_CONTEXT;
        let path = op.path(&[&id])?;
        let req = self.http.request(op, &path);
        self.http
            .send_json(&op.method(), &path, req, Some(&token))
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
        let op = &ops::CREATE_CONTEXT;
        let path = op.path(&[])?;
        let req = self.http.request(op, &path).json(&body);
        self.http
            .send_json(&op.method(), &path, req, Some(&token))
            .await
    }

    /// DELETE /api/contexts/{id} — retire the context (admin-gated). **Not a permanent
    /// delete**: the context stops being visible on the read axis and stops being writeable,
    /// but every row it homes is preserved untouched, and the slug is freed for immediate
    /// reuse. The returned [`RetireContextOutcome`] carries the mangled `context_ref` —
    /// `restore` accepts that ref (or the bare context id), not the original one, because the
    /// original address no longer resolves once the row is hidden and the slug has moved.
    pub async fn delete(&self, context_id: Uuid) -> Result<RetireContextOutcome> {
        let token = self.http.resolve_token()?;
        let op = &ops::DELETE_CONTEXT;
        let path = op.path(&[&context_id])?;
        let req = self.http.request(op, &path);
        self.http
            .send_json(&op.method(), &path, req, Some(&token))
            .await
    }

    /// POST /api/contexts/{id}/restore — reverse a retirement. Re-derives the address from the
    /// untouched name rather than trying to recover whatever `delete` mangled the slug to, so
    /// the returned slug can differ from the one the caller retired under.
    pub async fn restore(&self, context_id: Uuid) -> Result<RestoreContextOutcome> {
        let token = self.http.resolve_token()?;
        let op = &ops::RESTORE_CONTEXT;
        let path = op.path(&[&context_id])?;
        let req = self.http.request(op, &path);
        self.http
            .send_json(&op.method(), &path, req, Some(&token))
            .await
    }

    /// POST /api/contexts/{id}/teams — share the context into a team (admin-gated, idempotent).
    pub async fn share_team(
        &self,
        context_id: Uuid,
        body: &ShareContextRequest,
    ) -> Result<ShareContextOutcome> {
        let token = self.http.resolve_token()?;
        let op = &ops::SHARE_TEAM;
        let path = op.path(&[&context_id])?;
        let req = self.http.request(op, &path).json(body);
        self.http
            .send_json(&op.method(), &path, req, Some(&token))
            .await
    }

    /// DELETE /api/contexts/{id}/teams/{team_id} — unshare (admin-gated, no-op safe).
    pub async fn unshare_team(
        &self,
        context_id: Uuid,
        team_id: Uuid,
    ) -> Result<UnshareContextOutcome> {
        let token = self.http.resolve_token()?;
        let op = &ops::UNSHARE_TEAM;
        let path = op.path(&[&context_id, &team_id])?;
        let req = self.http.request(op, &path);
        self.http
            .send_json(&op.method(), &path, req, Some(&token))
            .await
    }

    /// POST /api/contexts/{id}/reassign — transfer the context's ownership to a team.
    pub async fn reassign(
        &self,
        context_id: Uuid,
        body: &ReassignContextRequest,
    ) -> Result<ReassignContextOutcome> {
        let token = self.http.resolve_token()?;
        let op = &ops::REASSIGN;
        let path = op.path(&[&context_id])?;
        let req = self.http.request(op, &path).json(body);
        self.http
            .send_json(&op.method(), &path, req, Some(&token))
            .await
    }

    /// POST /api/contexts/{id}/rename — change the context's name; the slug is re-derived from it.
    ///
    /// The rename **re-addresses** the context: the outcome carries the composed `context_ref` the
    /// caller should use from now on, because the old `@owner/slug` no longer resolves.
    pub async fn rename(
        &self,
        context_id: Uuid,
        body: &RenameContextRequest,
    ) -> Result<RenameContextOutcome> {
        let token = self.http.resolve_token()?;
        let op = &ops::RENAME;
        let path = op.path(&[&context_id])?;
        let req = self.http.request(op, &path).json(body);
        self.http
            .send_json(&op.method(), &path, req, Some(&token))
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
    /// most salient first, wrapped in an [`AnchorShape`] envelope.
    ///
    /// An empty answer is no longer mute: `emptiness` names the cause. A caller who cannot read the
    /// context gets `emptiness: unreadable_or_absent` with `population: 0` and no clock — the gate
    /// is in the SQL and stays a 200, so this is still no existence oracle. An un-materialized
    /// context is `never_clustered`; a materialized one that yielded nothing readable —
    /// because it formed no regions, or because none of them holds a member this caller can read,
    /// two causes the member gate keeps deliberately indistinguishable — is `nothing_visible`; and
    /// a `lens` that matched nothing is `lens_narrowed`. Four cases that were one bare `[]` before.
    pub async fn shape(&self, context_id: Uuid, lens: Option<Uuid>) -> Result<AnchorShape> {
        let token = self.http.resolve_token()?;
        let op = &ops::CONTEXT_SHAPE;
        let path = context_shape_path(context_id, lens)?;
        let req = self.http.request(op, &path);
        self.http
            .send_json(&op.method(), &path, req, Some(&token))
            .await
    }

    /// GET `/api/contexts/{id}/region-metrics[?lens=]` — the per-region analytics tier.
    pub async fn region_metrics(
        &self,
        context_id: Uuid,
        lens: Option<Uuid>,
    ) -> Result<Vec<CogmapRegionMetricsRow>> {
        let token = self.http.resolve_token()?;
        let op = &ops::CONTEXT_REGION_METRICS;
        let path = context_region_metrics_path(context_id, lens)?;
        let req = self.http.request(op, &path);
        self.http
            .send_json(&op.method(), &path, req, Some(&token))
            .await
    }

    /// GET `/api/contexts/{id}/analytics` — the context-level staleness readout: when the shape was
    /// last materialized, the latest touch to the regions and edges homed on it **that this caller
    /// may read**, and whether the read is stale.
    ///
    /// Three fields, not the five its cogmap peer returns: a context has no charter resource and no
    /// regulation set, so `telos_resource_id` and `regulation` would be null peer fields reporting
    /// "nothing found" about two things that cannot exist.
    ///
    /// **Deny is an error here, not an empty envelope** — 404, collapsed with "does not exist", the
    /// same posture as `materialize_delta` below and the cogmap peer.
    pub async fn analytics(&self, context_id: Uuid) -> Result<CogmapStaleness> {
        let token = self.http.resolve_token()?;
        let op = &ops::CONTEXT_ANALYTICS;
        let path = op.path(&[&context_id])?;
        let req = self.http.request(op, &path);
        self.http
            .send_json(&op.method(), &path, req, Some(&token))
            .await
    }

    /// GET `/api/contexts/{id}/materialize-delta[?threshold=]` — how many formation events have landed
    /// on the context since its last materialize, and whether that clears the threshold. The read peer
    /// of `materialize` below.
    ///
    /// **Deny is an error here, not an empty envelope.** A context the caller cannot read — and one
    /// that does not exist — both come back as 404, collapsed so this is still no existence oracle.
    /// `shape` next door denies by answering 200 with `emptiness: unreadable_or_absent`; the two
    /// postures are deliberately different and neither travels to the other.
    pub async fn materialize_delta(
        &self,
        context_id: Uuid,
        threshold: Option<i64>,
    ) -> Result<MaterializeDelta> {
        let token = self.http.resolve_token()?;
        let op = &ops::CONTEXT_MATERIALIZE_DELTA;
        let path = context_materialize_delta_path(context_id, threshold)?;
        let req = self.http.request(op, &path);
        self.http
            .send_json(&op.method(), &path, req, Some(&token))
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
        let op = &ops::CONTEXT_MATERIALIZE;
        let path = op.path(&[&context_id])?;
        let body = MaterializeRequest { threshold };
        let req = self.http.request(op, &path).json(&body);
        self.http
            .send_json(&op.method(), &path, req, Some(&token))
            .await
    }
}

/// `/api/contexts/{id}/shape` with an optional `?lens=` query — shared by the method and its test.
fn context_shape_path(context_id: Uuid, lens: Option<Uuid>) -> Result<String> {
    let base = ops::CONTEXT_SHAPE.path(&[&context_id])?;
    Ok(match lens {
        Some(l) => format!("{base}?lens={l}"),
        None => base,
    })
}

/// `/api/contexts/{id}/region-metrics` with an optional `?lens=` query.
fn context_region_metrics_path(context_id: Uuid, lens: Option<Uuid>) -> Result<String> {
    let base = ops::CONTEXT_REGION_METRICS.path(&[&context_id])?;
    Ok(match lens {
        Some(l) => format!("{base}?lens={l}"),
        None => base,
    })
}

/// `/api/contexts/{id}/materialize-delta` with an optional `?threshold=` query — shared by the method
/// and its test.
fn context_materialize_delta_path(context_id: Uuid, threshold: Option<i64>) -> Result<String> {
    let base = ops::CONTEXT_MATERIALIZE_DELTA.path(&[&context_id])?;
    Ok(match threshold {
        Some(t) => format!("{base}?threshold={t}"),
        None => base,
    })
}

/// The resolve request: the fixed path, with the ref as the `context_ref` query parameter
/// (percent-encoded by reqwest — a ref carries `@`, `+` and `/`).
fn resolve_request(http: &HttpClient, path: &str, context_ref: &str) -> reqwest::RequestBuilder {
    http.request(&ops::RESOLVE_CONTEXT, path)
        .query(&[("context_ref", context_ref)])
}

#[cfg(test)]
mod resolve_request_tests {
    use super::*;
    use temper_workflow::operations::Surface;

    fn http() -> HttpClient {
        HttpClient::new("http://127.0.0.1:9", None, Surface::CliCloud, None)
            .expect("loopback client")
    }

    #[test]
    fn resolve_carries_the_ref_as_an_encoded_query_parameter() {
        let req = resolve_request(&http(), "/api/contexts/resolve", "+tasker-systems/general")
            .build()
            .expect("request builds");
        assert_eq!(req.method(), reqwest::Method::GET);
        assert_eq!(req.url().path(), "/api/contexts/resolve");
        assert_eq!(
            req.url().query(),
            Some("context_ref=%2Btasker-systems%2Fgeneral")
        );
        let pairs: Vec<(String, String)> = req.url().query_pairs().into_owned().collect();
        assert_eq!(
            pairs,
            vec![(
                "context_ref".to_owned(),
                "+tasker-systems/general".to_owned()
            )],
            "the ref survives the encoding round trip byte for byte"
        );
    }

    #[test]
    fn resolve_encodes_the_at_me_form_too() {
        let req = resolve_request(&http(), "/api/contexts/resolve", "@me/temper")
            .build()
            .expect("request builds");
        let pairs: Vec<(String, String)> = req.url().query_pairs().into_owned().collect();
        assert_eq!(
            pairs,
            vec![("context_ref".to_owned(), "@me/temper".to_owned())]
        );
    }
}

#[cfg(test)]
mod orientation_path_tests {
    use super::*;

    #[test]
    fn shape_path_appends_lens_only_when_present() {
        let ctx = Uuid::nil();
        assert_eq!(
            context_shape_path(ctx, None).unwrap(),
            "/api/contexts/00000000-0000-0000-0000-000000000000/shape"
        );
        assert_eq!(
            context_shape_path(ctx, Some(Uuid::nil())).unwrap(),
            "/api/contexts/00000000-0000-0000-0000-000000000000/shape?lens=00000000-0000-0000-0000-000000000000"
        );
    }

    #[test]
    fn region_metrics_path_appends_lens_only_when_present() {
        let ctx = Uuid::nil();
        assert_eq!(
            context_region_metrics_path(ctx, None).unwrap(),
            "/api/contexts/00000000-0000-0000-0000-000000000000/region-metrics"
        );
        assert!(context_region_metrics_path(ctx, Some(Uuid::nil()))
            .unwrap()
            .contains("?lens="));
    }

    #[test]
    fn materialize_delta_path_appends_threshold_only_when_present() {
        let ctx = Uuid::nil();
        assert_eq!(
            context_materialize_delta_path(ctx, None).unwrap(),
            "/api/contexts/00000000-0000-0000-0000-000000000000/materialize-delta"
        );
        assert_eq!(
            context_materialize_delta_path(ctx, Some(5)).unwrap(),
            "/api/contexts/00000000-0000-0000-0000-000000000000/materialize-delta?threshold=5"
        );
    }
}
