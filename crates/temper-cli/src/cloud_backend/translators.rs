//! Pure cmd → wire translation functions for `CloudBackend`.
//!
//! Each function takes a `temper-core::operations` command struct and
//! produces the wire payload that `temper-client` accepts. Translators
//! are pure — they don't perform I/O or async work. The async dispatch
//! lives in `cloud_backend.rs::impl Backend`.

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
/// runs `compute_body_chunks` to fill them.
///
/// **Symmetric defense (CLAUDE.md "Phase 5's symmetric defense pattern"):**
/// always serializes managed_meta to JSON and runs
/// `ensure_managed_identity_keys` with `cmd.title` + `Some(cmd.slug)` from the
/// typed cmd, so the wire payload's `temper-title` and `temper-slug` always
/// derive from the same typed source as the server-side receive-side fill.
///
/// **`origin_uri`:** empty string today — server constructs the canonical
/// URI from `(owner, context, doctype, slug)`.
#[cfg(feature = "embed")]
pub(crate) fn cmd_to_ingest_payload(cmd: &CreateResource) -> Result<IngestPayload> {
    use temper_core::operations::ensure_managed_identity_keys;

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

    // Serialize managed_meta to JSON and inject canonical identity keys from
    // the typed cmd — symmetric defense per CLAUDE.md. Always emit Some(...);
    // the identity keys make it non-default by construction.
    let mut managed_value = serde_json::to_value(&cmd.managed_meta)
        .map_err(|e| TemperError::Project(format!("serialize managed_meta: {e}")))?;
    ensure_managed_identity_keys(&mut managed_value, &cmd.title, Some(&cmd.slug));
    let managed_meta = Some(managed_value);

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

/// Translate an `UpdateResource` command into a `ResourceUpdateRequest`
/// wire payload suitable for `PATCH /api/resources/{id}`.
///
/// **Partial-merge semantics:** only fields present in the cmd are
/// serialized on the wire.
///
/// **Move-to → managed_meta synthesis:** when the cmd carries
/// `move_to: Some(MoveSpec { context_to, type_to })` but no
/// `managed_meta.context` / `managed_meta.doc_type`, synthesizes
/// minimal managed_meta entries so the server-side row reflects the
/// move. Explicit caller-supplied values always win.
///
/// **Body-trio:** computed only when `cmd.body` is `Some`. Short-circuits
/// when `BodyUpdate` already carries pre-computed `content_hash` and
/// `chunks_packed`; otherwise computes via `compute_body_chunks`.
#[cfg(feature = "embed")]
pub(crate) fn cmd_to_resource_update_request(
    cmd: &temper_core::operations::UpdateResource,
) -> Result<temper_core::types::ResourceUpdateRequest> {
    use temper_core::types::ManagedMeta;

    // Body-trio computation (only when body present).
    let (content, content_hash, chunks_packed) = match &cmd.body {
        Some(b) => {
            let (hash, packed) = if b.content_hash.is_some() && b.chunks_packed.is_some() {
                (b.content_hash.clone(), b.chunks_packed.clone())
            } else {
                let chunks = crate::actions::ingest::compute_body_chunks(&b.content)?;
                (Some(chunks.content_hash), Some(chunks.chunks_packed))
            };
            (Some(b.content.clone()), hash, packed)
        }
        None => (None, None, None),
    };

    // Move_to → managed_meta synthesis: explicit caller fields always win.
    let mut managed_meta = cmd.managed_meta.clone().unwrap_or_default();
    if let Some(move_to) = &cmd.move_to {
        if managed_meta.context.is_none() {
            if let Some(ctx_to) = &move_to.context_to {
                managed_meta.context = Some(ctx_to.clone());
            }
        }
        if managed_meta.doc_type.is_none() {
            if let Some(type_to) = &move_to.type_to {
                managed_meta.doc_type = Some(type_to.clone());
            }
        }
    }

    let managed_meta_opt = if managed_meta == ManagedMeta::default() {
        None
    } else {
        // The resource is addressed by a `ResourceId` now — there is no slug in
        // the command to defend against, so `managed_meta.slug` is left as the
        // caller supplied it (the server reconciles slug from the resolved row).
        Some(managed_meta)
    };

    let open_meta = cmd
        .open_meta
        .as_ref()
        .map(serde_json::to_value)
        .transpose()
        .map_err(|e| TemperError::Project(format!("serialize open_meta: {e}")))?;

    // title field: lift managed_meta.title to the request's title field for
    // symmetry with today's cloud_mode_update path (commands/resource.rs:1524).
    let title = managed_meta_opt.as_ref().and_then(|mm| mm.title.clone());

    Ok(temper_core::types::ResourceUpdateRequest {
        title,
        slug: None,
        managed_meta: managed_meta_opt,
        open_meta,
        content,
        content_hash,
        chunks_packed,
    })
}

/// Project a `ResourceRow` (returned by `temper-client` methods) into the
/// `ResourceRow` shape required by the `Backend` trait.
///
/// The temper-client already returns `temper_core::types::resource::ResourceRow`
/// directly — there is no separate wire `Resource` type. This function is a
/// clone and exists as a named boundary so the `CloudBackend` impl in Task 5
/// has a consistent translation call site matching the other translators, and
/// so the naming in the plan aligns with the actual code structure.
#[cfg(feature = "embed")]
pub(crate) fn wire_resource_to_resource_row(
    resource: &temper_core::types::resource::ResourceRow,
) -> temper_core::types::resource::ResourceRow {
    resource.clone()
}

#[cfg(feature = "embed")]
#[cfg(test)]
mod tests {
    use super::*;
    use temper_core::operations::{MoveSpec, Surface, UpdateResource};
    use temper_core::types::ManagedMeta;

    #[cfg(feature = "test-embed")]
    use temper_core::operations::{BodyUpdate, CreateResource};

    #[cfg(feature = "test-embed")]
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

    // cmd_to_ingest_payload calls compute_body_chunks which requires the
    // ONNX runtime. Gate tests that exercise it behind `test-embed` so
    // they only run in the Embed CI job (where ONNX is installed).
    #[cfg(feature = "test-embed")]
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

    #[cfg(feature = "test-embed")]
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

    #[cfg(feature = "test-embed")]
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

    #[cfg(feature = "test-embed")]
    #[test]
    fn cmd_to_ingest_payload_always_injects_identity_keys() {
        // Symmetric defense (CLAUDE.md): even when caller-supplied managed_meta
        // is default, the wire payload must carry `temper-title` and `temper-slug`
        // injected from the typed cmd.
        let mut cmd = sample_cmd();
        cmd.managed_meta = ManagedMeta::default();
        let payload = cmd_to_ingest_payload(&cmd).expect("should succeed");
        let mm = payload
            .managed_meta
            .expect("identity keys make managed_meta non-default by construction");
        assert_eq!(mm["temper-title"], "Test task");
        assert_eq!(mm["temper-slug"], "2026-05-18-test");
    }

    #[cfg(feature = "test-embed")]
    #[test]
    fn cmd_to_ingest_payload_identity_keys_from_typed_source_not_caller_managed_meta() {
        // If a future refactor passes a managed_meta with title/slug that differs
        // from the cmd's typed title/slug, the typed cmd wins — preventing drift.
        let mut cmd = sample_cmd();
        cmd.managed_meta.title = Some("Drift!".to_string());
        cmd.managed_meta.slug = Some("drift-slug".to_string());
        let payload = cmd_to_ingest_payload(&cmd).expect("should succeed");
        let mm = payload.managed_meta.expect("present");
        assert_eq!(
            mm["temper-title"], "Test task",
            "typed cmd.title wins over managed_meta.title"
        );
        assert_eq!(
            mm["temper-slug"], "2026-05-18-test",
            "typed cmd.slug wins over managed_meta.slug"
        );
    }

    fn sample_update() -> UpdateResource {
        UpdateResource {
            resource: temper_core::types::ids::ResourceId(uuid::Uuid::nil()),
            body: None,
            managed_meta: None,
            open_meta: None,
            move_to: None,
            origin: Surface::CliCloud,
        }
    }

    #[test]
    fn cmd_to_resource_update_request_omits_absent_fields() {
        let cmd = sample_update();
        let req = cmd_to_resource_update_request(&cmd).expect("should succeed");
        assert!(req.title.is_none());
        assert!(req.managed_meta.is_none());
        assert!(req.open_meta.is_none());
        assert!(req.content.is_none());
        assert!(req.content_hash.is_none());
        assert!(req.chunks_packed.is_none());
    }

    #[test]
    fn cmd_to_resource_update_request_synthesizes_managed_meta_from_move_to() {
        let mut cmd = sample_update();
        cmd.move_to = Some(MoveSpec {
            context_to: Some("knowledge".to_string()),
            type_to: Some("concept".to_string()),
        });
        let req = cmd_to_resource_update_request(&cmd).expect("should succeed");
        let mm = req.managed_meta.expect("synthesized from move_to");
        assert_eq!(mm.context.as_deref(), Some("knowledge"));
        assert_eq!(mm.doc_type.as_deref(), Some("concept"));
    }

    #[test]
    fn cmd_to_resource_update_request_preserves_caller_managed_meta() {
        // The resource is addressed by id, so caller-supplied managed_meta
        // (including slug + title) flows through unchanged.
        let mut cmd = sample_update();
        cmd.managed_meta = Some(ManagedMeta {
            slug: Some("caller-slug".to_string()),
            title: Some("New Title".to_string()),
            ..ManagedMeta::default()
        });
        let req = cmd_to_resource_update_request(&cmd).expect("should succeed");
        let mm = req.managed_meta.expect("present");
        assert_eq!(mm.slug.as_deref(), Some("caller-slug"));
        // Title from managed_meta is preserved (the typed field is the source).
        assert_eq!(mm.title.as_deref(), Some("New Title"));
    }

    #[test]
    fn cmd_to_resource_update_request_does_not_overwrite_explicit_managed_meta() {
        let mut cmd = sample_update();
        cmd.managed_meta = Some(ManagedMeta {
            context: Some("explicit-context".to_string()),
            ..ManagedMeta::default()
        });
        cmd.move_to = Some(MoveSpec {
            context_to: Some("from-move-to".to_string()),
            type_to: None,
        });
        let req = cmd_to_resource_update_request(&cmd).expect("should succeed");
        let mm = req.managed_meta.expect("present");
        assert_eq!(
            mm.context.as_deref(),
            Some("explicit-context"),
            "explicit value wins over move_to synthesis"
        );
    }

    #[cfg(feature = "test-embed")]
    #[test]
    fn cmd_to_resource_update_request_computes_body_trio_when_body_present() {
        let mut cmd = sample_update();
        cmd.body = Some(BodyUpdate {
            content: "# Updated\n".to_string(),
            content_hash: None,
            chunks_packed: None,
        });
        let req = cmd_to_resource_update_request(&cmd).expect("should succeed");
        assert_eq!(req.content.as_deref(), Some("# Updated\n"));
        assert!(req.content_hash.is_some());
        assert!(req.chunks_packed.is_some());
    }

    // ── Task 4 tests ─────────────────────────────────────────────────────────

    use temper_core::types::ids::{ContextId, DocTypeId, ProfileId, ResourceId};
    use temper_core::types::resource::ResourceRow;
    use uuid::Uuid;

    fn sample_resource_row() -> ResourceRow {
        let nil = Uuid::nil();
        ResourceRow {
            id: ResourceId(nil),
            kb_context_id: ContextId(nil),
            kb_doc_type_id: DocTypeId(nil),
            origin_uri: "kb://@me/temper/task/test-task".to_string(),
            title: "Test Task".to_string(),
            slug: Some("test-task".to_string()),
            originator_profile_id: ProfileId(nil),
            owner_profile_id: ProfileId(nil),
            is_active: true,
            created: chrono::DateTime::UNIX_EPOCH,
            updated: chrono::DateTime::UNIX_EPOCH,
            context_name: "temper".to_string(),
            doc_type_name: "task".to_string(),
            owner_handle: "@me".to_string(),
            stage: Some("active".to_string()),
            seq: None,
            mode: None,
            effort: None,
            body_hash: Some("abc123".to_string()),
            managed_hash: None,
            open_hash: None,
        }
    }

    #[test]
    fn wire_resource_to_resource_row_maps_basic_fields() {
        let wire = sample_resource_row();
        let row = wire_resource_to_resource_row(&wire);
        assert_eq!(row.slug, Some("test-task".to_string()));
        assert_eq!(row.title, "Test Task");
        assert_eq!(row.id, ResourceId(Uuid::nil()));
        assert_eq!(row.context_name, "temper");
        assert_eq!(row.doc_type_name, "task");
        assert_eq!(row.body_hash, Some("abc123".to_string()));
        assert_eq!(row.owner_handle, "@me");
    }
}
