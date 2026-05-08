//! Pure cmd → service-request translators.
//!
//! Each function is total (no I/O) and infallible at the type level; runtime
//! validation is the caller's responsibility (it lives in the operations
//! module's pure actions).
//!
//! Translators are added incrementally as their consumers come online.

use sqlx::PgPool;
use temper_core::error::TemperError;
#[cfg(feature = "ingest-pipeline")]
use temper_core::hash::compute_body_hash;
use temper_core::operations::{
    CreateResource, ListFilter, ResourceRef, ResourceSummary, SearchHit, SearchQuery,
    UpdateResource,
};
use temper_core::types::api::{SearchParams, UnifiedSearchResultRow};
use temper_core::types::ids::{ProfileId, ResourceId};
use temper_core::types::ingest::IngestPayload;
use temper_core::types::resource::{ResourceListParams, ResourceRow, ResourceUpdateRequest};

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

/// Translate `ListFilter` → `ResourceListParams`.
///
/// Only the filters represented in both shapes are forwarded. `stage` and
/// `goal` are not first-class params on `ResourceListParams` today and would
/// require a `q`-string extension or a service-layer change — captured in
/// the spec's "Open Questions" as a follow-up; for 3a they're ignored.
pub(crate) fn list_filter_to_params(filter: ListFilter) -> ResourceListParams {
    ResourceListParams {
        kb_context_id: None,
        kb_doc_type_id: None,
        context_name: filter.context,
        doc_type_name: filter.doctype,
        owner: Some("@me".to_string()),
        q: None,
        sort: None,
        order: None,
        limit: filter.limit.map(|n| n as i64),
        offset: None,
    }
}

/// Project a `ResourceRow` into the trait's `ResourceSummary`.
pub(crate) fn resource_row_to_summary(row: &ResourceRow) -> ResourceSummary {
    ResourceSummary {
        slug: row.slug.clone().unwrap_or_default(),
        doctype: row.doc_type_name.clone(),
        context: row.context_name.clone(),
        title: row.title.clone(),
    }
}

/// Translate `SearchQuery` → `SearchParams` for `search_service::search`.
pub(crate) fn search_query_to_params(q: SearchQuery) -> SearchParams {
    SearchParams {
        query: Some(q.query),
        context_name: q.context,
        doc_type: q.doctype,
        limit: q.limit.map(|n| n as i64),
        ..Default::default()
    }
}

/// Project a search-service row into the trait's `SearchHit`.
///
/// `UnifiedSearchResultRow` is defined in `temper-core/src/types/api.rs`.
/// Field set: `resource_id`, `title`, `slug: String` (not Option),
/// `kb_uri`, `origin_uri`, `context: Option<String>`, `doc_type: String`,
/// `fts_score`, `vector_score`, `combined_score: f32`, `origin: String`.
/// The summary's `context` falls back to empty when absent.
pub(crate) fn unified_hit_to_search_hit(row: &UnifiedSearchResultRow) -> SearchHit {
    SearchHit {
        summary: ResourceSummary {
            slug: row.slug.clone(),
            doctype: row.doc_type.clone(),
            context: row.context.clone().unwrap_or_default(),
            title: row.title.clone(),
        },
        score: row.combined_score,
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

/// Compute `(content_hash, chunks_packed)` for an update-resource body. Mirrors
/// the in-place pipeline at `ingest_service::ingest:663-682` (body-trio
/// computation) so DbBackend's `update_resource` can populate the trio when
/// `cmd.body.is_some()`. Gated on the `ingest-pipeline` feature; without it,
/// returns `BadRequest` preserving the contract from `ingest_service.rs:678-683`.
#[cfg(feature = "ingest-pipeline")]
#[expect(dead_code)]
pub(crate) fn prepare_body_trio(body: &str) -> Result<(String, String), TemperError> {
    let hash = compute_body_hash(body);
    let packed_chunks = temper_ingest::pipeline::prepare_markdown(body)
        .map_err(|e| TemperError::Api(format!("embed: {e}")))?;
    let packed = temper_core::types::ingest::pack_chunks(&packed_chunks)
        .map_err(|e| TemperError::Api(format!("pack: {e}")))?;
    Ok((hash, packed))
}

#[cfg(not(feature = "ingest-pipeline"))]
#[expect(dead_code)]
pub(crate) fn prepare_body_trio(_body: &str) -> Result<(String, String), TemperError> {
    Err(TemperError::BadRequest(
        "chunks_packed required when server-side pipeline is not available".to_owned(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(feature = "test-embed")]
    #[test]
    fn prepare_body_trio_computes_hash_and_packs_chunks() {
        let body = "# heading\n\nparagraph text.\n";
        let (hash, packed) = prepare_body_trio(body).expect("pipeline ok");
        assert!(
            hash.starts_with("sha256:"),
            "hash should be sha256: prefixed"
        );
        assert_eq!(hash.len(), 71, "sha256:<64-char-hex>"); // "sha256:" (7) + 64 hex chars
        assert!(!packed.is_empty(), "packed chunks should be non-empty");
    }

    #[cfg(feature = "test-embed")]
    #[test]
    fn prepare_body_trio_empty_body_ok() {
        let (hash, _packed) = prepare_body_trio("").expect("empty body still hashable");
        assert!(hash.starts_with("sha256:"));
        assert_eq!(hash.len(), 71);
    }

    #[cfg(not(feature = "ingest-pipeline"))]
    #[test]
    fn prepare_body_trio_no_pipeline_returns_bad_request() {
        let err = prepare_body_trio("body").expect_err("no-pipeline path");
        assert!(matches!(
            err,
            temper_core::error::TemperError::BadRequest(_)
        ));
    }
}
