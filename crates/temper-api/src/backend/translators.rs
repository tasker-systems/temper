//! Pure cmd → service-request translators.
//!
//! Each function is total (no I/O) and infallible at the type level; runtime
//! validation is the caller's responsibility (it lives in the operations
//! module's pure actions).
//!
//! Translators are added incrementally as their consumers come online.

use sqlx::PgPool;
use temper_core::error::TemperError;
use temper_core::operations::{CreateResource, ResourceRef, UpdateResource};
use temper_core::types::ids::{ProfileId, ResourceId};
use temper_core::types::ingest::IngestPayload;
use temper_core::types::resource::ResourceUpdateRequest;

use crate::services::resource_service;

/// Translate `CreateResource` → `IngestPayload` for `ingest_service::ingest`.
///
/// `content_hash` and `chunks_packed` are left `None` so the server runs the
/// shared pipeline (when the `ingest-pipeline` feature is enabled). `metadata`
/// is the legacy unstructured field — left absent for new commands.
pub(crate) fn create_resource_to_ingest_payload(cmd: CreateResource) -> IngestPayload {
    let body = cmd.body.map(|b| b.content).unwrap_or_default();

    IngestPayload {
        title: cmd.title,
        origin_uri: String::new(),
        context_name: cmd.context,
        doc_type_name: cmd.doctype,
        content_hash: None,
        slug: cmd.slug,
        content: body,
        metadata: None,
        managed_meta: Some(serde_json::to_value(&cmd.managed_meta).unwrap_or_default()),
        open_meta: cmd.open_meta,
        chunks_packed: None,
    }
}

/// Translate `UpdateResource` → `ResourceUpdateRequest` for
/// `resource_service::update`. The body trio is all-or-nothing in the service
/// layer; the operations command surfaces it via `body: Option<BodyUpdate>`,
/// so when a body is present we recompute its hash and pack chunks here.
///
/// 3a-only behavior: if `body` is `Some`, the translator leaves
/// `content_hash` and `chunks_packed` as `None`. This forces the service to
/// reject the update with `BadRequest` because today's `resource_service::update`
/// requires the trio to be all-Some-or-all-None and the handler-layer guard
/// asserts that. **The 3b handler migration must take over hash/chunk
/// computation before passing through DbBackend** — until then, body-bearing
/// UpdateResource commands cannot be fulfilled. This is acceptable for 3a
/// because no caller dispatches through DbBackend yet (it's dark-launched).
pub(crate) fn update_resource_to_request(cmd: UpdateResource) -> ResourceUpdateRequest {
    let (title, slug) = cmd
        .managed_meta
        .as_ref()
        .map(|m| (m.title.clone(), m.slug.clone()))
        .unwrap_or((None, None));

    ResourceUpdateRequest {
        title,
        slug,
        managed_meta: cmd.managed_meta,
        open_meta: cmd.open_meta,
        content: cmd.body.as_ref().map(|b| b.content.clone()),
        content_hash: None,
        chunks_packed: None,
    }
}

/// Resolve a `ResourceRef` to a concrete `ResourceId`.
///
/// `Uuid` short-circuits without I/O; `Scoped` queries via `resolve_by_uri`
/// with `owner="@me"` (the self-scope idiom — see `push_owner` in
/// `resource_service.rs`).
pub(crate) async fn resolve_resource_ref(
    pool: &PgPool,
    profile_id: ProfileId,
    rref: ResourceRef,
) -> Result<ResourceId, TemperError> {
    match rref {
        ResourceRef::Uuid { id } => Ok(id),
        ResourceRef::Scoped {
            slug,
            doctype,
            context,
        } => {
            let params = resource_service::ResolveByUriParams {
                owner: "@me".to_string(),
                context,
                doc_type: doctype,
                ident: slug,
            };
            let row = resource_service::resolve_by_uri(pool, *profile_id, &params)
                .await
                .map_err(TemperError::from)?;
            Ok(row.id)
        }
    }
}
