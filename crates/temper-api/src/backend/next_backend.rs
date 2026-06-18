//! `NextBackend` (WS6 chunk 4b) — the `temper_next.*` substrate behind the `Backend` trait.
//! Feature-gated behind `next-backend` (pulls temper-next + onnx). Reads delegate to
//! `temper_next::readback`; writes stub `NotImplemented` until 4c.
//!
//! The full-row read (`show_resource`) reconstructs the migration-invariant subset of `ResourceRow`
//! from `temper_next.*` (`readback::resource_row`) and fills the non-invariant fields best-effort:
//! re-minted ids verbatim, `kb_doc_type_id` via a transitional `public.kb_doc_types` name→id lookup,
//! `slug`/`managed_hash`/`open_hash` = `None`, `created`/`updated` = read-time `Utc::now()`. See the
//! 4b spec parity-floor amendment.

use async_trait::async_trait;
use chrono::Utc;
use sqlx::{PgPool, Row};

use temper_core::error::TemperError;
use temper_core::operations::{
    AssertRelationship, Backend, CommandOutput, CreateResource, DeleteResource, FoldRelationship,
    ListResources, ResourceSummary, RetypeRelationship, ReweightRelationship, SearchHit,
    SearchResources, ShowResource, Surface, UpdateResource,
};
use temper_core::types::graph;
use temper_core::types::ids::{ContextId, DocTypeId, ProfileId, ResourceId};
use temper_core::types::resource::ResourceRow;

use temper_next::readback;
use temper_next::synthesis::key_fate::{key_fate, KeyFate};
use temper_next::writes;

/// Bridge a temper-next (`anyhow`) error into `TemperError` without naming `anyhow` (temper-api does not
/// depend on it) — `anyhow::Error: Display`, so `to_string()` carries the message.
fn api_err(e: impl std::fmt::Display) -> TemperError {
    TemperError::Api(e.to_string())
}

/// Map a typed [`readback::ReadbackError`] to the surface status, splitting the two deny modes the
/// single-resource reads can return: not-visible is the leak-safe deny → **404** (`NotFound`), never 403
/// (403 confirms existence) and never 500 (it is not a system failure); a genuine fault stays **500**
/// (`Api`). Collapsing both into NotFound — the pre-typing behavior on every `temper_next` single-read
/// surface — masked real faults as 404. Shared by `reconstruct_resource_row` and the read selector's
/// `get_content`/`get_meta` arms so the mapping lives in exactly one place.
pub(crate) fn map_readback_err(e: readback::ReadbackError) -> TemperError {
    match e {
        readback::ReadbackError::NotVisible { resource_id, .. } => {
            TemperError::NotFound(format!("resource {resource_id} not found"))
        }
        readback::ReadbackError::Fault(inner) => TemperError::Api(inner.to_string()),
    }
}

/// graph::EdgeKind → temper-next's affinity::EdgeKind (identical 4-variant taxonomy — 1:1, no §4 remap).
fn map_edge_kind(k: graph::EdgeKind) -> temper_next::affinity::EdgeKind {
    use temper_next::affinity::EdgeKind as N;
    match k {
        graph::EdgeKind::Express => N::Express,
        graph::EdgeKind::Contains => N::Contains,
        graph::EdgeKind::LeadsTo => N::LeadsTo,
        graph::EdgeKind::Near => N::Near,
    }
}

/// graph::Polarity → temper-next's payloads::EdgePolarity.
fn map_polarity(p: graph::Polarity) -> temper_next::payloads::EdgePolarity {
    use temper_next::payloads::EdgePolarity as N;
    match p {
        graph::Polarity::Forward => N::Forward,
        graph::Polarity::Inverse => N::Inverse,
    }
}

/// Map an inbound [`Surface`] to the synthesized per-surface emitter marker (`pete@<marker>`, §1b).
/// The HTTP/API surface maps to the web emitter (temperkb.io's surface).
fn surface_marker(s: Surface) -> &'static str {
    match s {
        Surface::CliCloud => "cli",
        Surface::Mcp => "mcp",
        Surface::ApiHttp => "web",
    }
}

/// Split a command's `managed_meta` + `open_meta` into the `(key, value)` property pairs the live write
/// path asserts: Property-fated managed keys (§7) + every open key, dropping nulls. `doc_type` rides the
/// `ResourceCreate` separately, and Die/Edge/ReconcileToDocType managed keys are excluded by fate.
fn properties_from_meta(
    managed_meta: &serde_json::Value,
    open_meta: Option<&serde_json::Value>,
) -> Vec<(String, serde_json::Value)> {
    let mut out: Vec<(String, serde_json::Value)> = Vec::new();
    if let Some(obj) = managed_meta.as_object() {
        for (k, v) in obj {
            if !v.is_null() && key_fate(k) == KeyFate::Property {
                out.push((k.clone(), v.clone()));
            }
        }
    }
    if let Some(obj) = open_meta.and_then(|o| o.as_object()) {
        for (k, v) in obj {
            if !v.is_null() {
                out.push((k.clone(), v.clone()));
            }
        }
    }
    out
}

/// Transitional `public.kb_doc_types` name→id lookup (valid during the migration window; `public`
/// still exists pre-flip). §7 dissolved the typed `DocTypeId`; the substrate keeps only the name.
/// Free function so both `NextBackend` and the read selector (full-row `list`) can reuse it.
pub(crate) async fn doc_type_id_by_name(
    pool: &PgPool,
    name: &str,
) -> Result<DocTypeId, TemperError> {
    let row = sqlx::query("SELECT id FROM public.kb_doc_types WHERE name = $1")
        .bind(name)
        .fetch_one(pool)
        .await
        .map_err(|e| TemperError::Api(format!("doc_type lookup for {name:?}: {e}")))?;
    Ok(DocTypeId::from(row.get::<uuid::Uuid, _>("id")))
}

/// Reconstruct a full production-shaped `ResourceRow` from a synthesized (`temper_next`) resource id,
/// at the §9 invariant floor. The invariant fields come from `readback::resource_row`; the
/// non-invariant fields are filled best-effort (re-minted ids verbatim, `kb_doc_type_id` via the
/// transitional `public` lookup, `slug`/hashes `None`, timestamps read-time `now()`). Shared by
/// `NextBackend::show_resource` and the read selector's full-row `list`.
pub(crate) async fn reconstruct_resource_row(
    pool: &PgPool,
    principal: uuid::Uuid,
    new_id: uuid::Uuid,
) -> Result<ResourceRow, TemperError> {
    let p = readback::resource_row(pool, principal, new_id)
        .await
        .map_err(map_readback_err)?;
    let kb_doc_type_id = doc_type_id_by_name(pool, &p.doc_type_name).await?;
    let now = Utc::now();
    Ok(ResourceRow {
        id: ResourceId::from(p.re_minted_id),
        kb_context_id: ContextId::from(p.re_minted_context_id),
        kb_doc_type_id,
        origin_uri: p.origin_uri,
        title: p.title,
        slug: None,
        originator_profile_id: ProfileId::from(p.originator_profile_id),
        owner_profile_id: ProfileId::from(p.owner_profile_id),
        is_active: p.is_active,
        created: now,
        updated: now,
        context_name: p.context_name,
        doc_type_name: p.doc_type_name,
        owner_handle: p.owner_handle,
        stage: p.stage,
        seq: p.seq,
        mode: p.mode,
        effort: p.effort,
        body_hash: p.body_hash,
        managed_hash: None,
        open_hash: None,
    })
}

/// The `temper_next.*` backend. Holds a pool + the caller profile (for symmetry with `DbBackend`;
/// 4b reads are visibility-UNSCOPED per the §9 floor — access-scoping is a named flip prerequisite,
/// tracked to WS2).
pub struct NextBackend {
    pool: PgPool,
    /// The caller profile — resolved to the synthesized `temper_next` profile by handle on each write
    /// (4c). Reads are visibility-UNSCOPED at the §9 floor (access-scoping is the WS2 flip prerequisite).
    profile_id: ProfileId,
}

impl NextBackend {
    pub fn new(pool: PgPool, profile_id: ProfileId) -> Self {
        Self { pool, profile_id }
    }

    /// Resolve the new-substrate resource id for a `ResourceId`. The production
    /// id is mapped to the synthesized id by `origin_uri`.
    async fn resolve_new_id(&self, id: ResourceId) -> Result<uuid::Uuid, TemperError> {
        let ids = readback::ResolvedIds::load(&self.pool)
            .await
            .map_err(|e| TemperError::Api(e.to_string()))?;
        ids.to_new(uuid::Uuid::from(id))
            .ok_or_else(|| TemperError::NotFound(format!("resource {id} not in temper_next")))
    }

    /// Auth-before-writes gate (WS2): the caller (`self.profile_id`, a production profile id that
    /// synthesis preserves verbatim into `temper_next`, so it resolves directly as the principal) must
    /// be able to modify the target `temper_next` resource. Returns `Forbidden` otherwise. CONFORMs to
    /// production's `check_can_modify`. Runtime, schema-unqualified query inside a `SET LOCAL search_path`
    /// txn: `can_modify_resource`'s body calls `profile_effective_teams`/`team_ancestors` UNQUALIFIED, so
    /// they resolve against the connection search_path — `public` on the bare pool, where the
    /// `temper_next` helpers do not exist (the readback gate-call discipline, WS2 Task 2).
    async fn check_can_modify_next(&self, new_id: uuid::Uuid) -> Result<(), TemperError> {
        let mut tx = self.pool.begin().await.map_err(api_err)?;
        sqlx::query("SET LOCAL search_path TO temper_next, public")
            .execute(&mut *tx)
            .await
            .map_err(api_err)?;
        let can: Option<bool> = sqlx::query_scalar("SELECT can_modify_resource($1, $2)")
            .bind(*self.profile_id)
            .bind(new_id)
            .fetch_one(&mut *tx)
            .await
            .map_err(api_err)?;
        tx.commit().await.map_err(api_err)?;
        if can.unwrap_or(false) {
            Ok(())
        } else {
            Err(TemperError::Forbidden)
        }
    }

    /// The source resource an edge mutation is authorized against. Production gates edge
    /// retype/reweight/fold on "can modify the SOURCE resource" (`handlers::edges` → 403 "Cannot modify
    /// source resource"); the parity-era write path only ever asserts resource→resource edges, so the
    /// source is always a `kb_resources` endpoint.
    async fn edge_source_resource(&self, edge_id: uuid::Uuid) -> Result<uuid::Uuid, TemperError> {
        sqlx::query_scalar::<_, uuid::Uuid>(
            "SELECT source_id FROM temper_next.kb_edges \
             WHERE id = $1 AND source_table = 'kb_resources'",
        )
        .bind(edge_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(api_err)?
        .ok_or_else(|| TemperError::NotFound(format!("edge {edge_id} not found")))
    }
}

#[async_trait]
impl Backend for NextBackend {
    async fn create_resource(
        &self,
        cmd: CreateResource,
    ) -> Result<CommandOutput<ResourceRow>, TemperError> {
        // Resolve the caller's synthesized identity (natural-key) and the home context.
        let prod_profile: uuid::Uuid = *self.profile_id;
        let owner = writes::resolve_profile(&self.pool, prod_profile)
            .await
            .map_err(api_err)?;
        let emitter = writes::resolve_emitter(&self.pool, owner, surface_marker(cmd.origin))
            .await
            .map_err(api_err)?;
        let home = writes::resolve_context(&self.pool, owner, &cmd.context)
            .await
            .map_err(api_err)?;

        let body = cmd
            .body
            .as_ref()
            .map(|b| b.content.clone())
            .unwrap_or_default();
        let origin_uri = cmd.origin_uri.clone().unwrap_or_default();
        let managed =
            serde_json::to_value(&cmd.managed_meta).map_err(|e| TemperError::Api(e.to_string()))?;
        let properties = properties_from_meta(&managed, cmd.open_meta.as_ref());

        let new_id = writes::create_resource(
            &self.pool,
            writes::CreateParams {
                title: &cmd.title,
                origin_uri: &origin_uri,
                body: &body,
                doc_type: &cmd.doctype,
                home,
                owner,
                // A fresh create's originator is its owner (the caller); a distinct originator only
                // arises via synthesis carrying a production row's history.
                originator: owner,
                emitter,
                properties: &properties,
            },
        )
        .await
        .map_err(api_err)?;

        let row = reconstruct_resource_row(&self.pool, *self.profile_id, new_id.uuid()).await?;
        Ok(CommandOutput::new(row))
    }

    async fn show_resource(
        &self,
        cmd: ShowResource,
    ) -> Result<CommandOutput<ResourceRow>, TemperError> {
        let new_id = self.resolve_new_id(cmd.resource).await?;
        // `reconstruct_resource_row` gates visibility (WS2) and maps the typed `ReadbackError` via
        // `map_readback_err`: not-visible → NotFound (404, the leak-safe deny — never 403, no
        // existence-leak oracle), a genuine fault → Api (500). The earlier blanket `|_| NotFound`
        // collapse masked real faults as 404.
        let row = reconstruct_resource_row(&self.pool, *self.profile_id, new_id).await?;
        Ok(CommandOutput::new(row))
    }

    async fn update_resource(
        &self,
        cmd: UpdateResource,
    ) -> Result<CommandOutput<ResourceRow>, TemperError> {
        // Address the target by its public id via ResolvedIds (the transitional parity model 4b reads
        // use; native-id addressing without a legacy twin is a chunk-5 flip concern).
        let new_id = self.resolve_new_id(cmd.resource).await?;
        // Auth before any write (WS2): the caller must be able to modify this resource.
        self.check_can_modify_next(new_id).await?;
        let owner = writes::resolve_profile(&self.pool, *self.profile_id)
            .await
            .map_err(api_err)?;
        let emitter = writes::resolve_emitter(&self.pool, owner, surface_marker(cmd.origin))
            .await
            .map_err(api_err)?;

        let body = cmd.body.as_ref().map(|b| b.content.clone());
        // temper-title (a §7-Die managed key) maps to the kb_resources.title column, not a property.
        let mut title: Option<String> = None;
        let mut properties: Vec<(String, serde_json::Value)> = Vec::new();
        if let Some(mm) = &cmd.managed_meta {
            let managed = serde_json::to_value(mm).map_err(|e| TemperError::Api(e.to_string()))?;
            title = managed
                .get("temper-title")
                .and_then(|v| v.as_str())
                .map(str::to_owned);
            properties = properties_from_meta(&managed, cmd.open_meta.as_ref());
        } else if cmd.open_meta.is_some() {
            properties = properties_from_meta(&serde_json::Value::Null, cmd.open_meta.as_ref());
        }

        // A type-move sets the authoritative `doc_type` property; a context-move re-homes.
        let mut rehome_to = None;
        if let Some(mv) = &cmd.move_to {
            if let Some(type_to) = &mv.type_to {
                properties.push(("doc_type".to_owned(), serde_json::json!(type_to)));
            }
            if let Some(ctx_to) = &mv.context_to {
                rehome_to = Some(
                    writes::resolve_context(&self.pool, owner, ctx_to)
                        .await
                        .map_err(api_err)?,
                );
            }
        }

        writes::update_resource(
            &self.pool,
            writes::UpdateParams {
                resource: temper_next::ids::ResourceId::from(new_id),
                body: body.as_deref(),
                title: title.as_deref(),
                origin_uri: None,
                properties: &properties,
                rehome_to,
                emitter,
            },
        )
        .await
        .map_err(api_err)?;

        let row = reconstruct_resource_row(&self.pool, *self.profile_id, new_id).await?;
        Ok(CommandOutput::new(row))
    }

    async fn delete_resource(&self, cmd: DeleteResource) -> Result<CommandOutput<()>, TemperError> {
        let new_id = self.resolve_new_id(cmd.resource).await?;
        // Auth before any write (WS2).
        self.check_can_modify_next(new_id).await?;
        let owner = writes::resolve_profile(&self.pool, *self.profile_id)
            .await
            .map_err(api_err)?;
        let emitter = writes::resolve_emitter(&self.pool, owner, surface_marker(cmd.origin))
            .await
            .map_err(api_err)?;
        writes::delete_resource(
            &self.pool,
            temper_next::ids::ResourceId::from(new_id),
            emitter,
        )
        .await
        .map_err(api_err)?;
        Ok(CommandOutput::new(()))
    }

    async fn list_resources(
        &self,
        _cmd: ListResources,
    ) -> Result<CommandOutput<Vec<ResourceSummary>>, TemperError> {
        let rows = readback::list(&self.pool, *self.profile_id)
            .await
            .map_err(|e| TemperError::Api(e.to_string()))?;
        let summaries = rows
            .into_iter()
            .map(|r| ResourceSummary {
                // slug is §7-dissolved; the list summary uses origin_uri as the stable handle.
                slug: r.origin_uri,
                doctype: r.doc_type,
                // Context scoping over temper_next is a flip prerequisite (WS2); unscoped at the §9 floor.
                context: String::new(),
                title: r.title,
            })
            .collect();
        Ok(CommandOutput::new(summaries))
    }

    async fn search_resources(
        &self,
        cmd: SearchResources,
    ) -> Result<CommandOutput<Vec<SearchHit>>, TemperError> {
        // 4b: FTS only (the text query). Vector search needs a query embedding this layer does not
        // carry; the HTTP search selector handles vector mode directly (read selector, Task 6/8).
        let uris = readback::fts_search(&self.pool, *self.profile_id, &cmd.query.query)
            .await
            .map_err(|e| TemperError::Api(e.to_string()))?;
        let hits = uris
            .into_iter()
            .map(|uri| SearchHit {
                summary: ResourceSummary {
                    slug: uri,
                    doctype: String::new(),
                    context: String::new(),
                    title: String::new(),
                },
                // §9 floor asserts the matching SET, not the score.
                score: 0.0,
            })
            .collect();
        Ok(CommandOutput::new(hits))
    }

    async fn assert_relationship(
        &self,
        cmd: AssertRelationship,
    ) -> Result<CommandOutput<uuid::Uuid>, TemperError> {
        // Source is pre-resolved.
        let source_pub = uuid::Uuid::from(cmd.source);
        let ids = readback::ResolvedIds::load(&self.pool)
            .await
            .map_err(api_err)?;
        let src_next = ids.to_new(source_pub).ok_or_else(|| {
            TemperError::NotFound(format!("source {source_pub} not in temper_next"))
        })?;
        // Auth before any write (WS2): edge mutations gate on the SOURCE resource (production's
        // "Cannot modify source resource"). Gate before resolving the target / writing the edge.
        self.check_can_modify_next(src_next).await?;

        // The target is pre-resolved — map its public id to its temper_next id.
        let target_pub = uuid::Uuid::from(cmd.target);
        let tgt_next = ids.to_new(target_pub).ok_or_else(|| {
            TemperError::NotFound(format!("target {target_pub} not in temper_next"))
        })?;

        // The edge homes in the source's temper_next context (its home anchor).
        let home_next: uuid::Uuid = sqlx::query_scalar(
            "SELECT anchor_id FROM temper_next.kb_resource_homes \
             WHERE resource_id=$1 AND anchor_table='kb_contexts'",
        )
        .bind(src_next)
        .fetch_one(&self.pool)
        .await
        .map_err(|e| TemperError::Api(e.to_string()))?;

        let owner = writes::resolve_profile(&self.pool, *self.profile_id)
            .await
            .map_err(api_err)?;
        let emitter = writes::resolve_emitter(&self.pool, owner, surface_marker(cmd.origin))
            .await
            .map_err(api_err)?;

        let label = (!cmd.label.is_empty()).then_some(cmd.label.as_str());
        let edge = writes::assert_relationship(
            &self.pool,
            writes::AssertParams {
                src: temper_next::ids::ResourceId::from(src_next),
                tgt: temper_next::ids::ResourceId::from(tgt_next),
                kind: map_edge_kind(cmd.edge_kind),
                polarity: map_polarity(cmd.polarity),
                label,
                weight: cmd.weight,
                home: temper_next::ids::ContextId::from(home_next),
                emitter,
            },
        )
        .await
        .map_err(api_err)?;
        Ok(CommandOutput::new(edge.uuid()))
    }

    async fn retype_relationship(
        &self,
        cmd: RetypeRelationship,
    ) -> Result<CommandOutput<uuid::Uuid>, TemperError> {
        // The edge handle on the next backend IS the temper_next edge id (returned by assert).
        // Auth before any write (WS2): gate on the edge's source resource.
        let src = self.edge_source_resource(cmd.correlation_id).await?;
        self.check_can_modify_next(src).await?;
        let owner = writes::resolve_profile(&self.pool, *self.profile_id)
            .await
            .map_err(api_err)?;
        let emitter = writes::resolve_emitter(&self.pool, owner, surface_marker(cmd.origin))
            .await
            .map_err(api_err)?;
        writes::retype_relationship(
            &self.pool,
            temper_next::ids::EdgeId::from(cmd.correlation_id),
            map_edge_kind(cmd.edge_kind),
            map_polarity(cmd.polarity),
            emitter,
        )
        .await
        .map_err(api_err)?;
        Ok(CommandOutput::new(cmd.correlation_id))
    }

    async fn reweight_relationship(
        &self,
        cmd: ReweightRelationship,
    ) -> Result<CommandOutput<uuid::Uuid>, TemperError> {
        // Auth before any write (WS2): gate on the edge's source resource.
        let src = self.edge_source_resource(cmd.correlation_id).await?;
        self.check_can_modify_next(src).await?;
        let owner = writes::resolve_profile(&self.pool, *self.profile_id)
            .await
            .map_err(api_err)?;
        let emitter = writes::resolve_emitter(&self.pool, owner, surface_marker(cmd.origin))
            .await
            .map_err(api_err)?;
        writes::reweight_relationship(
            &self.pool,
            temper_next::ids::EdgeId::from(cmd.correlation_id),
            cmd.weight,
            emitter,
        )
        .await
        .map_err(api_err)?;
        Ok(CommandOutput::new(cmd.correlation_id))
    }

    async fn fold_relationship(
        &self,
        cmd: FoldRelationship,
    ) -> Result<CommandOutput<uuid::Uuid>, TemperError> {
        // Auth before any write (WS2): gate on the edge's source resource.
        let src = self.edge_source_resource(cmd.correlation_id).await?;
        self.check_can_modify_next(src).await?;
        let owner = writes::resolve_profile(&self.pool, *self.profile_id)
            .await
            .map_err(api_err)?;
        let emitter = writes::resolve_emitter(&self.pool, owner, surface_marker(cmd.origin))
            .await
            .map_err(api_err)?;
        writes::fold_relationship(
            &self.pool,
            temper_next::ids::EdgeId::from(cmd.correlation_id),
            cmd.reason.as_deref(),
            emitter,
        )
        .await
        .map_err(api_err)?;
        Ok(CommandOutput::new(cmd.correlation_id))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_object_safe() {
        fn assert_obj(_: &dyn Backend) {}
        let _ = assert_obj;
        // If NextBackend were not object-safe, `select_backend`'s next arm (Task 5) would not compile.
    }
}
