//! `NextBackend` (WS6 chunk 4b) — the `temper_next.*` substrate behind the `Backend` trait.
//! Feature-gated behind `next-backend` (pulls temper-next + onnx). Reads delegate to
//! `temper_next::readback`; writes stub `NotImplemented` until 4c.
//!
//! The full-row read (`show_resource`) reconstructs the migration-invariant subset of `ResourceRow`
//! from `temper_next.*` (`readback::resource_row`) and fills the non-invariant fields best-effort:
//! re-minted ids verbatim, `kb_doc_type_id` via a transitional `public.kb_doc_types` name→id lookup,
//! `slug`/`managed_hash`/`open_hash` = `None`, `created`/`updated` = read-time `Utc::now()`. See the
//! 4b spec parity-floor amendment.
#![cfg(feature = "next-backend")]

use async_trait::async_trait;
use chrono::Utc;
use sqlx::{PgPool, Row};

use temper_core::error::TemperError;
use temper_core::operations::{
    Backend, CommandOutput, CreateResource, DeleteResource, ListResources, ResourceRef,
    ResourceSummary, SearchHit, SearchResources, ShowResource, UpdateResource,
};
use temper_core::types::ids::{ContextId, DocTypeId, ProfileId, ResourceId};
use temper_core::types::resource::ResourceRow;

use temper_next::readback;

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
    new_id: uuid::Uuid,
) -> Result<ResourceRow, TemperError> {
    let p = readback::resource_row(pool, new_id)
        .await
        .map_err(|e| TemperError::Api(e.to_string()))?;
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
    #[allow(dead_code)] // used once access-scoping lands (WS2 / flip prerequisite)
    profile_id: ProfileId,
}

impl NextBackend {
    pub fn new(pool: PgPool, profile_id: ProfileId) -> Self {
        Self { pool, profile_id }
    }

    /// Resolve the new-substrate resource id for a `ResourceRef`. 4b supports `Uuid` refs only (the
    /// HTTP show path always passes a `Uuid`); the production id is mapped to the synthesized id by
    /// `origin_uri`. `Scoped` refs land in 4c alongside the write paths that need them.
    async fn resolve_new_id(&self, refr: &ResourceRef) -> Result<uuid::Uuid, TemperError> {
        match refr {
            ResourceRef::Uuid { id } => {
                let ids = readback::ResolvedIds::load(&self.pool)
                    .await
                    .map_err(|e| TemperError::Api(e.to_string()))?;
                ids.to_new(uuid::Uuid::from(*id)).ok_or_else(|| {
                    TemperError::NotFound(format!("resource {id} not in temper_next"))
                })
            }
            ResourceRef::Scoped { .. } => Err(TemperError::NotImplemented(
                "scoped resource refs on the next backend (WS6 4c)".into(),
            )),
        }
    }
}

#[async_trait]
impl Backend for NextBackend {
    async fn create_resource(
        &self,
        _cmd: CreateResource,
    ) -> Result<CommandOutput<ResourceRow>, TemperError> {
        Err(TemperError::NotImplemented(
            "create over the next backend (WS6 4c)".into(),
        ))
    }

    async fn show_resource(
        &self,
        cmd: ShowResource,
    ) -> Result<CommandOutput<ResourceRow>, TemperError> {
        let new_id = self.resolve_new_id(&cmd.resource).await?;
        let row = reconstruct_resource_row(&self.pool, new_id).await?;
        Ok(CommandOutput::new(row))
    }

    async fn update_resource(
        &self,
        _cmd: UpdateResource,
    ) -> Result<CommandOutput<ResourceRow>, TemperError> {
        Err(TemperError::NotImplemented(
            "update over the next backend (WS6 4c)".into(),
        ))
    }

    async fn delete_resource(
        &self,
        _cmd: DeleteResource,
    ) -> Result<CommandOutput<()>, TemperError> {
        Err(TemperError::NotImplemented(
            "delete over the next backend (WS6 4c)".into(),
        ))
    }

    async fn list_resources(
        &self,
        _cmd: ListResources,
    ) -> Result<CommandOutput<Vec<ResourceSummary>>, TemperError> {
        let rows = readback::list(&self.pool)
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
        let uris = readback::fts_search(&self.pool, &cmd.query.query)
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
