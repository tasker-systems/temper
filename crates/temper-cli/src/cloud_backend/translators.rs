//! Pure cmd → wire translation functions for `CloudBackend`.
//!
//! Each function takes a `temper-core::operations` command struct and
//! produces the wire payload that `temper-client` accepts. Translators
//! are pure — they don't perform I/O or async work. The async dispatch
//! lives in `cloud_backend.rs::impl Backend`.
//!
//! Mirror of `vault_backend/translators.rs`.

#[cfg(feature = "embed")]
use crate::error::{Result, TemperError};
#[cfg(feature = "embed")]
use temper_core::operations::CreateResource;
#[cfg(feature = "embed")]
use temper_core::types::ingest::IngestPayload;

/// Translate a `CreateResource` command into an `IngestPayload` wire
/// payload suitable for `POST /api/ingest`.
///
/// **Body resolution:** If `cmd.body` is present and non-empty, use it.
/// Otherwise synthesize `# {title}\n` (matches existing cloud_mode_create
/// behavior in `commands/resource.rs:214`).
///
/// **Body-trio computation:** If `cmd.body` already carries pre-computed
/// `content_hash` and `chunks_packed`, they are forwarded directly. Otherwise
/// runs `compute_body_chunks` to fill them. Mirror of existing cloud_mode_create
/// at `commands/resource.rs:226-234`.
///
/// **managed_meta/open_meta:** serialized via `serde_json::to_value`. When
/// `managed_meta == ManagedMeta::default()`, omitted from the wire.
///
/// **`origin_uri`:** empty string today — server constructs the canonical
/// URI from `(owner, context, doctype, slug)`.
#[cfg(feature = "embed")]
pub(crate) fn cmd_to_ingest_payload(cmd: &CreateResource) -> Result<IngestPayload> {
    // Resolve body content.
    let content = match &cmd.body {
        Some(b) if !b.content.is_empty() => b.content.clone(),
        _ => format!("# {}\n", cmd.title),
    };

    // Body-trio computation: short-circuit if pre-computed, else embed.
    let (content_hash, chunks_packed) = match &cmd.body {
        Some(b) if b.content_hash.is_some() && b.chunks_packed.is_some() => {
            (b.content_hash.clone(), b.chunks_packed.clone())
        }
        _ => {
            let chunks = crate::actions::ingest::compute_body_chunks(&content)?;
            (Some(chunks.content_hash), Some(chunks.chunks_packed))
        }
    };

    // Serialize managed_meta (omit when default).
    let managed_meta = if cmd.managed_meta == temper_core::types::ManagedMeta::default() {
        None
    } else {
        Some(
            serde_json::to_value(&cmd.managed_meta)
                .map_err(|e| TemperError::Project(format!("serialize managed_meta: {e}")))?,
        )
    };

    let open_meta = cmd
        .open_meta
        .as_ref()
        .map(serde_json::to_value)
        .transpose()
        .map_err(|e| TemperError::Project(format!("serialize open_meta: {e}")))?;

    Ok(IngestPayload {
        title: cmd.title.clone(),
        origin_uri: String::new(),
        context_name: cmd.context.clone(),
        doc_type_name: cmd.doctype.clone(),
        content_hash,
        slug: cmd.slug.clone(),
        content,
        metadata: None,
        managed_meta,
        open_meta,
        chunks_packed,
    })
}

#[cfg(feature = "embed")]
#[cfg(test)]
mod tests {
    use super::*;
    use temper_core::operations::{BodyUpdate, CreateResource, Surface};
    use temper_core::types::ManagedMeta;

    fn sample_cmd() -> CreateResource {
        CreateResource {
            slug: "2026-05-18-test".to_string(),
            doctype: "task".to_string(),
            context: "temper".to_string(),
            title: "Test task".to_string(),
            body: Some(BodyUpdate {
                content: "# Test\n\nBody.\n".to_string(),
                content_hash: None,
                chunks_packed: None,
            }),
            managed_meta: ManagedMeta {
                mode: Some("plan".to_string()),
                effort: Some("small".to_string()),
                goal: Some("temper-maintenance".to_string()),
                ..ManagedMeta::default()
            },
            open_meta: None,
            origin_uri: None,
            chunks_packed: None,
            content_hash: None,
            origin: Surface::CliCloud,
        }
    }

    #[test]
    fn cmd_to_ingest_payload_round_trips_basic_fields() {
        let cmd = sample_cmd();
        let payload = cmd_to_ingest_payload(&cmd).expect("should succeed");
        assert_eq!(payload.slug, "2026-05-18-test");
        assert_eq!(payload.title, "Test task");
        assert_eq!(payload.context_name, "temper");
        assert_eq!(payload.doc_type_name, "task");
        assert_eq!(payload.content, "# Test\n\nBody.\n");
        assert!(payload.chunks_packed.is_some());
        assert!(payload.content_hash.is_some());
    }

    #[test]
    fn cmd_to_ingest_payload_serializes_managed_meta_to_json() {
        let cmd = sample_cmd();
        let payload = cmd_to_ingest_payload(&cmd).expect("should succeed");
        let mm = payload
            .managed_meta
            .expect("managed_meta should be present");
        // ManagedMeta fields use temper-* serde renames.
        assert_eq!(mm["temper-mode"], "plan");
        assert_eq!(mm["temper-effort"], "small");
        assert_eq!(mm["temper-goal"], "temper-maintenance");
    }

    #[test]
    fn cmd_to_ingest_payload_synthesizes_body_when_absent() {
        let mut cmd = sample_cmd();
        cmd.body = None;
        let payload = cmd_to_ingest_payload(&cmd).expect("should succeed");
        assert_eq!(
            payload.content, "# Test task\n",
            "placeholder body uses title"
        );
    }

    #[test]
    fn cmd_to_ingest_payload_skips_managed_meta_when_default() {
        let mut cmd = sample_cmd();
        cmd.managed_meta = ManagedMeta::default();
        let payload = cmd_to_ingest_payload(&cmd).expect("should succeed");
        assert!(
            payload.managed_meta.is_none(),
            "default managed_meta omitted from wire"
        );
    }
}
