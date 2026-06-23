//! `NextBackend` (WS6 chunk 4b) — the `temper_next.*` substrate behind the `Backend` trait.
//! Feature-gated behind `next-backend` (pulls temper-next + onnx). Reads delegate to
//! `temper_next::readback`; writes stub `NotImplemented` until 4c.
//!
//! The full-row read (`show_resource`) reconstructs the migration-invariant subset of `ResourceRow`
//! from `temper_next.*` (`readback::resource_row`) and fills the non-invariant fields best-effort:
//! re-minted ids verbatim, `kb_doc_type_id` re-minted nil (the doc_type NAME is authoritative — §7
//! dissolved the typed `DocTypeId`, so the substrate keeps only the name; no cross-namespace read),
//! `slug`/`managed_hash`/`open_hash` = `None`, `created`/`updated` = read-time `Utc::now()`. See the
//! 4b spec parity-floor amendment.

use async_trait::async_trait;
use chrono::Utc;
use sqlx::PgPool;

use crate::services::ingest_service;
use temper_core::error::TemperError;
use temper_core::operations::{
    AssertRelationship, Backend, CommandOutput, CreateResource, DeleteResource, FoldRelationship,
    ListResources, ResourceSummary, RetypeRelationship, ReweightRelationship, SearchHit,
    SearchResources, ShowResource, Surface, UpdateResource,
};
use temper_core::types::graph;
use temper_core::types::ids::{ContextId, DocTypeId, ProfileId, ResourceId};
use temper_core::types::resource::ResourceRow;

use temper_next::keys::{key_fate, KeyFate};
use temper_next::readback;
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

/// Reconstruct a full production-shaped `ResourceRow` from a synthesized (`temper_next`) resource id,
/// at the §9 invariant floor. The invariant fields come from `readback::resource_row`; the
/// non-invariant fields are filled best-effort (re-minted ids verbatim, `kb_doc_type_id` re-minted nil
/// — the doc_type NAME is authoritative, §7 dissolved the typed id — `slug`/hashes `None`, timestamps
/// read-time `now()`). No `public.*` read: the readback path is wholly `temper_next.*`, so it survives
/// the chunk-5 cutover renaming `public.*` aside. Shared by `NextBackend::show_resource` and the read
/// selector's full-row `list`/`search`. CONFORMs to the `list_enriched` Next arm's same nil fill.
pub(crate) async fn reconstruct_resource_row(
    pool: &PgPool,
    principal: uuid::Uuid,
    new_id: uuid::Uuid,
) -> Result<ResourceRow, TemperError> {
    let p = readback::resource_row(pool, principal, new_id)
        .await
        .map_err(map_readback_err)?;
    let now = Utc::now();
    Ok(ResourceRow {
        id: ResourceId::from(p.re_minted_id),
        kb_context_id: ContextId::from(p.re_minted_context_id),
        // §7-dissolved typed DocTypeId → re-minted nil; `doc_type_name` (below) is authoritative.
        kb_doc_type_id: DocTypeId::from(uuid::Uuid::nil()),
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

        // Create-time guards (WS6 collapse Task F). The substrate create path applies the same
        // strip → defaults → identity-keys → validate → body-hash-dedup pipeline the legacy
        // `ingest_service::ingest` ran (`:433-502`), calling the surviving pure helpers WHERE THEY
        // LIVE (no helper move — the flip owns relocation). Purely additive over the prior no-guard
        // create.
        let mut managed =
            serde_json::to_value(&cmd.managed_meta).map_err(|e| TemperError::Api(e.to_string()))?;
        // 1. Strip identity / tier-1 system keys a caller may have echoed back from a prior read.
        managed = ingest_service::strip_system_managed_fields(managed);
        // 2. Apply doc-type managed-tier defaults (e.g. task → `temper-stage: backlog`).
        temper_core::operations::apply_defaults_value(&cmd.doctype, &mut managed);
        // 3. Inject the canonical identity keys (`temper-title`/`temper-slug`) before validation, the
        //    same send/receive-symmetric discipline ingest uses. An empty slug removes `temper-slug`
        //    (mirrors ingest's `injected_slug` at `:444-453`).
        let injected_slug = (!cmd.slug.is_empty()).then_some(cmd.slug.as_str());
        temper_core::operations::ensure_managed_identity_keys(
            &mut managed,
            &cmd.title,
            injected_slug,
        );
        // 4. Validate the assembled managed_meta against the doc-type schema; PROPAGATE the typed
        //    validation error (never swallow it). A fresh canonical id + `now()` seed the validation
        //    document exactly as ingest does (`:457-467`); that id is not persisted from here — the
        //    substrate mints the resource id in `writes::create_resource`.
        let validate_params = ingest_service::ValidateParams {
            id: uuid::Uuid::now_v7(),
            created: Utc::now(),
            doc_type: &cmd.doctype,
            managed_meta: Some(&managed),
            slug: &cmd.slug,
            title: &cmd.title,
            context_name: &cmd.context,
        };
        ingest_service::validate_managed_meta(&validate_params).map_err(|e| {
            match crate::error::ApiError::from(e) {
                crate::error::ApiError::BadRequest(m) => TemperError::BadRequest(m),
                other => TemperError::Api(other.to_string()),
            }
        })?;

        // 5. Body-hash dedup (non-empty body only, matching legacy `:497-502`): if a visible active
        //    resource already carries the same substrate body_hash merkle, return IT instead of
        //    creating a twin — reconstructing the same `CommandOutput<ResourceRow>` the create path
        //    returns for a fresh row (`:254-255`).
        if !body.is_empty() {
            let body_hash = temper_next::content::body_hash_for_body(&body);
            if let Some(existing) =
                readback::find_by_body_hash(&self.pool, *self.profile_id, &body_hash)
                    .await
                    .map_err(api_err)?
            {
                let row = reconstruct_resource_row(&self.pool, *self.profile_id, existing).await?;
                return Ok(CommandOutput::new(row));
            }
        }

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
        // The inbound id IS the `temper_next` id — synthesis preserves resource ids verbatim, so there
        // is no origin_uri remap (the prior bimap collapsed empty-origin_uri resources onto one id).
        let new_id = uuid::Uuid::from(cmd.resource);
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
        // The inbound id IS the preserved `temper_next` id (native-id addressing — synthesis carries
        // resource ids verbatim, so no origin_uri remap).
        let new_id = uuid::Uuid::from(cmd.resource);
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
        // The inbound id IS the preserved `temper_next` id (no origin_uri remap).
        let new_id = uuid::Uuid::from(cmd.resource);
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
        // `fts_search` returns the preserved resource ids (origin_uri is non-unique — empty for
        // CLI/agent-created resources, so it cannot identify a match). Each id reconstructs to its
        // summary (origin_uri verbatim as the stable handle, like `list_resources`).
        let ids = readback::fts_search(&self.pool, *self.profile_id, &cmd.query.query)
            .await
            .map_err(|e| TemperError::Api(e.to_string()))?;
        let mut hits = Vec::with_capacity(ids.len());
        for new_id in ids {
            let row = reconstruct_resource_row(&self.pool, *self.profile_id, new_id).await?;
            hits.push(SearchHit {
                summary: ResourceSummary {
                    // slug is §7-dissolved; the summary uses origin_uri as the stable handle.
                    slug: row.origin_uri,
                    doctype: row.doc_type_name,
                    context: row.context_name,
                    title: row.title,
                },
                // §9 floor asserts the matching SET, not the score.
                score: 0.0,
            });
        }
        Ok(CommandOutput::new(hits))
    }

    async fn assert_relationship(
        &self,
        cmd: AssertRelationship,
    ) -> Result<CommandOutput<uuid::Uuid>, TemperError> {
        // Source and target ids ARE the preserved `temper_next` ids (synthesis carries resource ids
        // verbatim) — used directly, no origin_uri remap (the prior bimap collapsed empty-origin_uri
        // resources onto one arbitrary id).
        let src_next = uuid::Uuid::from(cmd.source);
        // Auth before any write (WS2): edge mutations gate on the SOURCE resource (production's
        // "Cannot modify source resource"). Gate before resolving the target / writing the edge.
        self.check_can_modify_next(src_next).await?;

        let tgt_next = uuid::Uuid::from(cmd.target);

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
        let src = self.edge_source_resource(cmd.edge_handle).await?;
        self.check_can_modify_next(src).await?;
        let owner = writes::resolve_profile(&self.pool, *self.profile_id)
            .await
            .map_err(api_err)?;
        let emitter = writes::resolve_emitter(&self.pool, owner, surface_marker(cmd.origin))
            .await
            .map_err(api_err)?;
        writes::retype_relationship(
            &self.pool,
            temper_next::ids::EdgeId::from(cmd.edge_handle),
            map_edge_kind(cmd.edge_kind),
            map_polarity(cmd.polarity),
            emitter,
        )
        .await
        .map_err(api_err)?;
        Ok(CommandOutput::new(cmd.edge_handle))
    }

    async fn reweight_relationship(
        &self,
        cmd: ReweightRelationship,
    ) -> Result<CommandOutput<uuid::Uuid>, TemperError> {
        // Auth before any write (WS2): gate on the edge's source resource.
        let src = self.edge_source_resource(cmd.edge_handle).await?;
        self.check_can_modify_next(src).await?;
        let owner = writes::resolve_profile(&self.pool, *self.profile_id)
            .await
            .map_err(api_err)?;
        let emitter = writes::resolve_emitter(&self.pool, owner, surface_marker(cmd.origin))
            .await
            .map_err(api_err)?;
        writes::reweight_relationship(
            &self.pool,
            temper_next::ids::EdgeId::from(cmd.edge_handle),
            cmd.weight,
            emitter,
        )
        .await
        .map_err(api_err)?;
        Ok(CommandOutput::new(cmd.edge_handle))
    }

    async fn fold_relationship(
        &self,
        cmd: FoldRelationship,
    ) -> Result<CommandOutput<uuid::Uuid>, TemperError> {
        // Auth before any write (WS2): gate on the edge's source resource.
        let src = self.edge_source_resource(cmd.edge_handle).await?;
        self.check_can_modify_next(src).await?;
        let owner = writes::resolve_profile(&self.pool, *self.profile_id)
            .await
            .map_err(api_err)?;
        let emitter = writes::resolve_emitter(&self.pool, owner, surface_marker(cmd.origin))
            .await
            .map_err(api_err)?;
        writes::fold_relationship(
            &self.pool,
            temper_next::ids::EdgeId::from(cmd.edge_handle),
            cmd.reason.as_deref(),
            emitter,
        )
        .await
        .map_err(api_err)?;
        Ok(CommandOutput::new(cmd.edge_handle))
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
