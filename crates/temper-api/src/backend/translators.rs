//! Pure cmd → service-request translators.
//!
//! Each function is total (no I/O) and infallible at the type level; runtime
//! validation is the caller's responsibility (it lives in the operations
//! module's pure actions).
//!
//! Translators are added incrementally as their consumers come online.

use temper_core::operations::CreateResource;
use temper_core::types::ingest::IngestPayload;

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
