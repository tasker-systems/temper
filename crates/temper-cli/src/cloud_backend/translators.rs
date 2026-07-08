//! Pure cmd → wire translation functions for `CloudBackend`.
//!
//! Each function takes a `temper-core::operations` command struct and
//! produces the wire payload that `temper-client` accepts. Translators
//! are pure — they don't perform I/O or async work. The async dispatch
//! lives in `cloud_backend.rs::impl Backend`.

#[cfg(feature = "embed")]
use crate::error::{Result, TemperError};
#[cfg(feature = "embed")]
use temper_core::types::ingest::IngestPayload;
#[cfg(feature = "embed")]
use temper_workflow::operations::CreateResource;

/// Resolve the body content for a create.
///
/// The caller-provided body is used **verbatim** when non-empty; only an
/// absent/empty body falls back to a synthesized `# {title}\n` placeholder.
///
/// The title is deliberately *not* prepended to a user-supplied body: the
/// canonical title lives in frontmatter (`temper-title`), and a body that
/// already opens with its own H1 must not receive a second one. This guards
/// the historical duplicate-H1 bug where an older create path concatenated
/// `# {title}` ahead of a body that already started with `# {title}`,
/// mashing `# X# X` onto one line.
///
/// Pure (no ONNX), so the H1 behavior is regression-tested in normal CI
/// without requiring the `embed` runtime. Compiled when its callers exist:
/// `cmd_to_ingest_payload` (embed) or the test module.
#[cfg(any(feature = "embed", test))]
fn resolve_create_body(
    body: Option<&temper_workflow::operations::BodyUpdate>,
    title: &str,
) -> String {
    match body {
        Some(b) if !b.content.is_empty() => b.content.clone(),
        _ => format!("# {title}\n"),
    }
}

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
/// **Identity is first-class:** `title` travels as a top-level payload field from
/// the typed `cmd`; `managed_meta` is Property-only and is never injected with
/// identity keys. Slug is §7-dissolved and NOT sent — the server derives it from
/// the title (issue #307).
///
/// **`origin_uri`:** empty string today — the server owns URI construction.
#[cfg(feature = "embed")]
pub(crate) fn cmd_to_ingest_payload(
    cmd: &CreateResource,
    context_ref: &str,
) -> Result<IngestPayload> {
    // Resolve body content (verbatim when provided; placeholder otherwise).
    let content = resolve_create_body(cmd.body.as_ref(), &cmd.title);

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

    // managed_meta is Property-only; identity travels first-class as the payload's
    // top-level title/slug (set below). No identity injection into managed_meta.
    let managed_meta = Some(
        serde_json::to_value(&cmd.managed_meta)
            .map_err(|e| TemperError::Project(format!("serialize managed_meta: {e}")))?,
    );

    let open_meta = cmd
        .open_meta
        .as_ref()
        .map(serde_json::to_value)
        .transpose()
        .map_err(|e| TemperError::Project(format!("serialize open_meta: {e}")))?;

    // A cogmap home is the single source of truth: when present, the server
    // branches on `home_cogmap_id` first and ignores `context_ref` (sent empty).
    let home_cogmap_id = match &cmd.home {
        temper_core::types::home::HomeAnchor::Cogmap(m) => Some(m.uuid()),
        temper_core::types::home::HomeAnchor::Context(_) => None,
    };
    let context_ref = if home_cogmap_id.is_some() {
        String::new()
    } else {
        context_ref.to_owned()
    };

    Ok(IngestPayload {
        title: cmd.title.clone(),
        origin_uri: String::new(),
        context_ref,
        home_cogmap_id,
        doc_type_name: cmd.doctype.clone(),
        // First-class goal link (already resolved to an id by the CLI surface); the ingest
        // handler projects the live `advances`→goal edge after create.
        goal: cmd.goal.map(uuid::Uuid::from),
        content_hash,
        content,
        metadata: None,
        managed_meta,
        open_meta,
        chunks_packed,
        // Block-provenance sources travel with the body on the wire; the ingest handler
        // maps them back onto the CreateResource's BodyUpdate.
        sources: cmd
            .body
            .as_ref()
            .map(|b| b.sources.clone())
            .unwrap_or_default(),
        // Carry the per-act correlation + authorship from the command onto the wire (discrete
        // ActInput shape); the ingest handler reassembles it into the act on its CreateResource.
        act: cmd.act.clone().into(),
    })
}

/// Translate an `UpdateResource` command into a `ResourceUpdateRequest`
/// wire payload suitable for `PATCH /api/resources/{id}`.
///
/// **Partial-merge semantics:** only fields present in the cmd are
/// serialized on the wire.
///
/// **Context move:** `cmd.context_ref` (the raw `@owner/slug` or UUID ref
/// set by the CLI) is forwarded verbatim as `req.context_to` for the API
/// handler to parse and resolve server-side. `move_to.context_to` carries a
/// *resolved* `ContextId` (only set by the API handler, never the CLI); if
/// somehow present it is also forwarded as a UUID string. Bare names are
/// rejected 400 by the server (Decision 1).
///
/// **Type move:** `move_to.type_to` travels first-class as the request's
/// `type_to` field (never synthesized into `managed_meta` — type left the
/// managed vocabulary in Phase 2); the server rewrites the authoritative
/// doc-type from it.
///
/// **Body-trio:** computed only when `cmd.body` is `Some`. Short-circuits
/// when `BodyUpdate` already carries pre-computed `content_hash` and
/// `chunks_packed`; otherwise computes via `compute_body_chunks`.
#[cfg(feature = "embed")]
pub(crate) fn cmd_to_resource_update_request(
    cmd: &temper_workflow::operations::UpdateResource,
) -> Result<temper_workflow::types::ResourceUpdateRequest> {
    use temper_workflow::types::ManagedMeta;

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

    // Context move: prefer the raw ref from context_ref (CLI path), fall back
    // to converting a pre-resolved ContextId (if somehow present) to UUID string.
    let context_to = cmd.context_ref.clone().or_else(|| {
        cmd.move_to
            .as_ref()
            .and_then(|mv| mv.context_to.map(|id| id.to_string()))
    });

    // managed_meta is Property-only; type-move now travels first-class via `type_to`
    // (not synthesized as a temper-type key), and identity travels via title/slug.
    let managed_meta = cmd.managed_meta.clone().unwrap_or_default();
    let managed_meta_opt = if managed_meta == ManagedMeta::default() {
        None
    } else {
        Some(managed_meta)
    };

    let open_meta = cmd
        .open_meta
        .as_ref()
        .map(serde_json::to_value)
        .transpose()
        .map_err(|e| TemperError::Project(format!("serialize open_meta: {e}")))?;

    let type_to = cmd.move_to.as_ref().and_then(|m| m.type_to.clone());

    // Goal patch → wire tri-state: `Set` carries the resolved id in `goal`; `Clear` sets
    // `clear_goal: true`; `None` leaves both absent (goal edge untouched).
    let (goal, clear_goal) = match &cmd.goal {
        Some(temper_workflow::operations::GoalPatch::Set(id)) => {
            (Some(uuid::Uuid::from(*id)), None)
        }
        Some(temper_workflow::operations::GoalPatch::Clear) => (None, Some(true)),
        None => (None, None),
    };

    Ok(temper_workflow::types::ResourceUpdateRequest {
        title: cmd.title.clone(),
        managed_meta: managed_meta_opt,
        open_meta,
        content,
        content_hash,
        chunks_packed,
        context_to,
        type_to,
        goal,
        clear_goal,
        // Block-provenance sources travel with the body on the wire; the update handler
        // maps them back onto the UpdateResource's BodyUpdate.
        sources: cmd
            .body
            .as_ref()
            .map(|b| b.sources.clone())
            .unwrap_or_default(),
        // Which block the body+sources address (`--content-block`); the update handler maps it
        // back onto the UpdateResource's BodyUpdate alongside `sources`.
        content_block: cmd.body.as_ref().and_then(|b| b.content_block),
        // Carry the per-act correlation + authorship from the command onto the wire (discrete
        // ActInput shape); the update handler reassembles it into the act on its UpdateResource.
        act: cmd.act.clone().into(),
    })
}

/// Project a `ResourceRow` (returned by `temper-client` methods) into the
/// `ResourceRow` shape required by the `Backend` trait.
///
/// The temper-client already returns `temper_workflow::types::resource::ResourceRow`
/// directly — there is no separate wire `Resource` type. This function is a
/// clone and exists as a named boundary so the `CloudBackend` impl in Task 5
/// has a consistent translation call site matching the other translators, and
/// so the naming in the plan aligns with the actual code structure.
#[cfg(feature = "embed")]
pub(crate) fn wire_resource_to_resource_row(
    resource: &temper_workflow::types::resource::ResourceRow,
) -> temper_workflow::types::resource::ResourceRow {
    resource.clone()
}

#[cfg(feature = "embed")]
#[cfg(test)]
mod tests {
    use super::*;
    use temper_workflow::operations::{MoveSpec, Surface, UpdateResource};
    use temper_workflow::types::ManagedMeta;

    #[cfg(feature = "test-embed")]
    use temper_workflow::operations::{BodyUpdate, CreateResource};

    #[cfg(feature = "test-embed")]
    fn sample_cmd() -> CreateResource {
        CreateResource {
            slug: "2026-05-18-test".to_string(),
            doctype: "task".to_string(),
            home: temper_core::types::home::HomeAnchor::Context(
                temper_core::types::ids::ContextId::new(),
            ),
            title: "Test task".to_string(),
            body: Some(BodyUpdate {
                content: "# Test\n\nBody.\n".to_string(),
                content_hash: None,
                chunks_packed: None,
                sources: Vec::new(),
                content_block: None,
            }),
            managed_meta: ManagedMeta {
                mode: Some("plan".to_string()),
                effort: Some("small".to_string()),
                ..ManagedMeta::default()
            },
            open_meta: None,
            origin_uri: None,
            chunks_packed: None,
            content_hash: None,
            goal: None,
            act: Default::default(),
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
        let payload = cmd_to_ingest_payload(&cmd, "@me/temper").expect("should succeed");
        assert_eq!(payload.title, "Test task");
        assert_eq!(payload.context_ref, "@me/temper");
        assert_eq!(payload.doc_type_name, "task");
        assert_eq!(payload.content, "# Test\n\nBody.\n");
        assert!(payload.chunks_packed.is_some());
        assert!(payload.content_hash.is_some());
    }

    #[cfg(feature = "test-embed")]
    #[test]
    fn cmd_to_ingest_payload_serializes_managed_meta_to_json() {
        let cmd = sample_cmd();
        let payload = cmd_to_ingest_payload(&cmd, "@me/temper").expect("should succeed");
        let mm = payload
            .managed_meta
            .expect("managed_meta should be present");
        // ManagedMeta fields use temper-* serde renames.
        assert_eq!(mm["temper-mode"], "plan");
        assert_eq!(mm["temper-effort"], "small");
    }

    #[cfg(feature = "test-embed")]
    #[test]
    fn cmd_to_ingest_payload_synthesizes_body_when_absent() {
        let mut cmd = sample_cmd();
        cmd.body = None;
        let payload = cmd_to_ingest_payload(&cmd, "@me/temper").expect("should succeed");
        assert_eq!(
            payload.content, "# Test task\n",
            "placeholder body uses title"
        );
    }

    #[cfg(feature = "test-embed")]
    #[test]
    fn cmd_to_ingest_payload_carries_identity_top_level_not_in_managed_meta() {
        // Identity is first-class: `title` travels as a top-level payload field from
        // the typed cmd, and managed_meta is Property-only — it must NOT carry
        // `temper-title` / `temper-slug` (they left the vocabulary in Phase 2). Slug is
        // §7-dissolved and no longer on the wire at all (issue #307).
        let cmd = sample_cmd();
        let payload = cmd_to_ingest_payload(&cmd, "@me/temper").expect("should succeed");
        assert_eq!(payload.title, "Test task", "identity title is top-level");
        if let Some(mm) = &payload.managed_meta {
            assert!(
                mm.get("temper-title").is_none() && mm.get("temper-slug").is_none(),
                "managed_meta must be Property-only, no identity keys; got: {mm}"
            );
        }
    }

    fn sample_update() -> UpdateResource {
        UpdateResource {
            resource: temper_core::types::ids::ResourceId(uuid::Uuid::nil()),
            title: None,
            slug: None,
            body: None,
            managed_meta: None,
            open_meta: None,
            move_to: None,
            context_ref: None,
            goal: None,
            act: Default::default(),
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
        assert!(req.context_to.is_none());
    }

    #[test]
    fn cmd_to_resource_update_request_forwards_context_ref_to_context_to() {
        // CLI path: raw ref goes via context_ref; translator forwards it to
        // req.context_to verbatim (API handler resolves server-side).
        let mut cmd = sample_update();
        cmd.context_ref = Some("@me/knowledge".to_string());
        let req = cmd_to_resource_update_request(&cmd).expect("should succeed");
        assert_eq!(
            req.context_to.as_deref(),
            Some("@me/knowledge"),
            "raw context ref must be forwarded verbatim to req.context_to"
        );
    }

    #[test]
    fn cmd_to_resource_update_request_forwards_type_to_first_class() {
        // Type moves travel as the first-class `type_to` wire field (no longer
        // synthesized as a temper-type key inside managed_meta).
        let mut cmd = sample_update();
        cmd.move_to = Some(MoveSpec {
            context_to: None,
            type_to: Some("concept".to_string()),
        });
        let req = cmd_to_resource_update_request(&cmd).expect("should succeed");
        assert_eq!(req.type_to.as_deref(), Some("concept"));
        assert!(
            req.context_to.is_none(),
            "no context_to without context_ref"
        );
    }

    #[test]
    fn cmd_to_resource_update_request_preserves_caller_managed_meta() {
        // The resource is addressed by id; caller-supplied Property-only
        // managed_meta flows through unchanged.
        let mut cmd = sample_update();
        cmd.managed_meta = Some(ManagedMeta {
            stage: Some("done".to_string()),
            ..ManagedMeta::default()
        });
        let req = cmd_to_resource_update_request(&cmd).expect("should succeed");
        let mm = req.managed_meta.expect("present");
        assert_eq!(mm.stage.as_deref(), Some("done"));
    }

    #[test]
    fn cmd_to_resource_update_request_context_ref_wins_over_move_to_context_to() {
        // When both context_ref and move_to.context_to are set (unusual; move_to.context_to
        // is a resolved ContextId, normally only set server-side), context_ref wins since
        // or_else picks the first Some.
        let mut cmd = sample_update();
        cmd.context_ref = Some("@me/from-cli".to_string());
        cmd.move_to = Some(MoveSpec {
            context_to: Some(temper_core::types::ids::ContextId::new()),
            type_to: None,
        });
        let req = cmd_to_resource_update_request(&cmd).expect("should succeed");
        assert_eq!(
            req.context_to.as_deref(),
            Some("@me/from-cli"),
            "context_ref (CLI raw ref) wins over move_to.context_to UUID"
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
            sources: Vec::new(),
            content_block: None,
        });
        let req = cmd_to_resource_update_request(&cmd).expect("should succeed");
        assert_eq!(req.content.as_deref(), Some("# Updated\n"));
        assert!(req.content_hash.is_some());
        assert!(req.chunks_packed.is_some());
    }

    // ── Task 4 tests ─────────────────────────────────────────────────────────

    use temper_core::types::ids::{ContextId, ProfileId, ResourceId};
    use temper_workflow::types::resource::ResourceRow;
    use uuid::Uuid;

    fn sample_resource_row() -> ResourceRow {
        let nil = Uuid::nil();
        ResourceRow {
            id: ResourceId(nil),
            kb_context_id: Some(ContextId(nil)),
            origin_uri: "kb://@me/temper/task/test-task".to_string(),
            title: "Test Task".to_string(),
            originator_profile_id: ProfileId(nil),
            owner_profile_id: ProfileId(nil),
            is_active: true,
            created: chrono::DateTime::UNIX_EPOCH,
            updated: chrono::DateTime::UNIX_EPOCH,
            context_name: Some("temper".to_string()),
            doc_type_name: "task".to_string(),
            owner_handle: "@me".to_string(),
            context_slug: Some("temper".to_string()),
            context_owner_ref: Some("@me".to_string()),
            cogmap_id: None,
            cogmap_name: None,
            stage: Some("active".to_string()),
            seq: None,
            mode: None,
            effort: None,
            body_hash: Some("abc123".to_string()),
        }
    }

    #[test]
    fn wire_resource_to_resource_row_maps_basic_fields() {
        let wire = sample_resource_row();
        let row = wire_resource_to_resource_row(&wire);
        assert_eq!(row.title, "Test Task");
        assert_eq!(row.id, ResourceId(Uuid::nil()));
        assert_eq!(row.context_name.as_deref(), Some("temper"));
        assert_eq!(row.doc_type_name, "task");
        assert_eq!(row.body_hash, Some("abc123".to_string()));
        assert_eq!(row.owner_handle, "@me");
    }
}

/// Ungated tests for `resolve_create_body` — no `embed`/ONNX needed, so the
/// duplicate-H1 regression runs in normal CI.
#[cfg(test)]
mod body_resolution_tests {
    use super::resolve_create_body;
    use temper_workflow::operations::BodyUpdate;

    fn body(content: &str) -> BodyUpdate {
        BodyUpdate {
            content: content.to_string(),
            content_hash: None,
            chunks_packed: None,
            sources: Vec::new(),
            content_block: None,
        }
    }

    #[test]
    fn body_with_h1_matching_title_is_not_double_prepended() {
        // The historical bug: `# X# X` mashed onto one line. The body must be
        // used verbatim, carrying exactly one H1 — not the title prepended on
        // top of the body's own matching H1.
        let title = "Unify resource delete";
        let user_body = "# Unify resource delete\n\nCloud-first, explicit-only.\n";
        let resolved = resolve_create_body(Some(&body(user_body)), title);
        assert_eq!(resolved, user_body, "body must be used verbatim");
        assert_eq!(
            resolved.matches("# Unify resource delete").count(),
            1,
            "exactly one H1; no doubled title"
        );
        assert!(
            !resolved.contains("delete# "),
            "no two H1s mashed onto one line"
        );
    }

    #[test]
    fn body_with_nonmatching_h1_is_respected() {
        let resolved = resolve_create_body(
            Some(&body("# A different heading\n\nText.\n")),
            "Task title",
        );
        assert_eq!(resolved, "# A different heading\n\nText.\n");
        assert!(
            !resolved.contains("Task title"),
            "title is not injected into a body that already has its own H1"
        );
    }

    #[test]
    fn body_without_h1_is_used_verbatim_not_title_prepended() {
        // Cloud-only design: the canonical title lives in frontmatter, so a
        // body lacking an H1 is passed through unchanged rather than having
        // `# {title}` prepended.
        let resolved =
            resolve_create_body(Some(&body("Just a paragraph, no heading.\n")), "Some title");
        assert_eq!(resolved, "Just a paragraph, no heading.\n");
    }

    #[test]
    fn absent_body_synthesizes_title_h1_placeholder() {
        assert_eq!(resolve_create_body(None, "My title"), "# My title\n");
    }

    #[test]
    fn empty_body_synthesizes_title_h1_placeholder() {
        assert_eq!(
            resolve_create_body(Some(&body("")), "My title"),
            "# My title\n"
        );
    }
}
