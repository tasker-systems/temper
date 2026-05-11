//! Pure cmd → vault-flow translators (no I/O).

use std::path::{Path, PathBuf};

use temper_core::error::TemperError;
#[cfg(feature = "embed")]
use temper_core::hash::compute_body_hash;
use temper_core::operations::{CreateResource, ResourceRef, UpdateResource};
use temper_core::types::ids::ResourceId;
use temper_core::types::ingest::IngestPayload;
use temper_core::types::manifest::Manifest;
use temper_core::types::resource::ResourceUpdateRequest;

use crate::config::Config;

/// A resolved resource: its stable UUID and the absolute path to its vault file.
///
/// Returned by [`resolve_resource_ref`] so callers have both the identity key
/// (for manifest lookups and API calls) and the filesystem path (for reads /
/// writes / deletes) without a second parse.
#[derive(Debug, Clone)]
pub(crate) struct ResolvedResource {
    pub resource_id: ResourceId,
    pub path: PathBuf,
}

/// Resolve a `ResourceRef` to a `ResolvedResource` using the local vault.
///
/// - `Uuid { id }`: looks up `id` in the manifest reverse-index. Returns
///   `NotFound` when the id is absent.
/// - `Scoped { owner, context, doctype, slug }`: delegates to
///   [`crate::lookup::find_resource`], which walks the vault filesystem.
///   Returns `NotFound` when the file exists but carries no `temper-id` in
///   frontmatter or manifest (a provisional-only file).
///
/// The function performs no network I/O. `find_resource` is synchronous
/// filesystem walking, which is acceptable in CLI context.
pub(crate) fn resolve_resource_ref(
    vault_root: &Path,
    manifest: &Manifest,
    config: &Config,
    rref: &ResourceRef,
) -> Result<ResolvedResource, TemperError> {
    match rref {
        ResourceRef::Uuid { id } => match manifest.entries.get(id) {
            Some(entry) => Ok(ResolvedResource {
                resource_id: *id,
                path: vault_root.join(&entry.path),
            }),
            None => Err(TemperError::NotFound(format!(
                "no manifest entry for resource {id}"
            ))),
        },
        ResourceRef::Scoped {
            owner,
            context,
            doctype,
            slug,
        } => {
            let dt = temper_core::frontmatter::DocType::from_str(doctype)?;
            let resolved = crate::lookup::find_resource(crate::lookup::FindableResource {
                config,
                manifest: Some(manifest),
                owner: Some(owner.clone()),
                context: Some(context.clone()),
                doc_type: dt,
                slug_or_suffix: slug.clone(),
            })?;
            let resource_id = resolved.resource_id.ok_or_else(|| {
                TemperError::NotFound(format!(
                    "resource has no temper-id in frontmatter or manifest: {slug}"
                ))
            })?;
            Ok(ResolvedResource {
                resource_id,
                path: resolved.path,
            })
        }
    }
}

/// Pre-computed body trio: SHA-256 content hash + packed chunks.
/// Mirrors the trio rule from `resource_service::update`: when a body
/// update is present, all three of (content, content_hash, chunks_packed)
/// must be supplied together.
///
/// Consumed by `cmd_to_ingest_payload`, `cmd_to_update_request`, and the
/// Task 7-8 create/update body paths in `vault_backend.rs`.
#[derive(Debug, Clone)]
pub(crate) struct BodyTrio {
    pub content_hash: String,
    pub chunks_packed: String,
}

/// Compute (content_hash, chunks_packed) for a body update.
///
/// **Duplicated from `temper-api/src/backend/translators.rs::prepare_body_trio`.**
/// Lift to `temper-core::operations::body` deferred to a focused cleanup
/// (vault task `lift-prepare-body-trio-to-temper-core-shared-helper`) because
/// it requires adding `temper-ingest` as an optional dep of `temper-core`,
/// which is a structural feature-graph change outside Phase 4a's scope.
///
/// In `temper-cli`, the relevant feature gate is `embed` (mirrors
/// `ingest-pipeline` in `temper-api`): the `embed` feature wires
/// `temper-ingest/embed-download` which provides `pipeline::prepare_markdown`.
#[cfg(feature = "embed")]
pub(crate) fn prepare_body_trio(body: &str) -> Result<BodyTrio, TemperError> {
    let content_hash = compute_body_hash(body);
    let packed_chunks = temper_ingest::pipeline::prepare_markdown(body)
        .map_err(|e| TemperError::Api(format!("embed: {e}")))?;
    let chunks_packed = temper_core::types::ingest::pack_chunks(&packed_chunks)
        .map_err(|e| TemperError::Api(format!("pack: {e}")))?;
    Ok(BodyTrio {
        content_hash,
        chunks_packed,
    })
}

#[cfg(not(feature = "embed"))]
pub(crate) fn prepare_body_trio(_body: &str) -> Result<BodyTrio, TemperError> {
    Err(TemperError::BadRequest(
        "chunks_packed required when embed pipeline is not available".to_owned(),
    ))
}

/// Translate `CreateResource` → `IngestPayload` for the push-as-tail-action path.
///
/// Mirrors `temper-api/src/backend/translators.rs::create_resource_to_ingest_payload`
/// but takes `body` as a separate `&str` (the vault caller has already written
/// the file to disk and has the body in hand) and accepts a pre-computed
/// `body_trio` so the caller decides when to run the embed pipeline.
///
/// When `body_trio` is `Some`, its `content_hash` and `chunks_packed` take
/// priority over any fields on `cmd` (caller-supplied trio is authoritative).
pub(crate) fn cmd_to_ingest_payload(
    cmd: &CreateResource,
    body: &str,
    body_trio: Option<&BodyTrio>,
) -> IngestPayload {
    IngestPayload {
        title: cmd.title.clone(),
        origin_uri: cmd.origin_uri.clone().unwrap_or_default(),
        context_name: cmd.context.clone(),
        doc_type_name: cmd.doctype.clone(),
        content_hash: body_trio
            .map(|t| t.content_hash.clone())
            .or_else(|| cmd.content_hash.clone()),
        slug: cmd.slug.clone(),
        content: body.to_string(),
        metadata: None,
        managed_meta: Some(serde_json::to_value(&cmd.managed_meta).unwrap_or_default()),
        open_meta: cmd.open_meta.clone(),
        chunks_packed: body_trio
            .map(|t| t.chunks_packed.clone())
            .or_else(|| cmd.chunks_packed.clone()),
    }
}

/// Translate `UpdateResource` → `ResourceUpdateRequest` for the push-as-tail-action path.
///
/// Mirrors `temper-api/src/backend/translators.rs::update_resource_to_request`
/// but the body pipeline is the **caller's responsibility**: when `cmd.body` is
/// `Some`, the caller must supply a pre-computed `body_trio`; the translator
/// errors with `BadRequest` if the trio is absent. This differs from the
/// DbBackend translator, which runs `prepare_body_trio` inline when no
/// pre-computed trio is present.
///
/// Open_meta keys are validated via
/// `temper_core::operations::validate_open_meta_keys`; an unknown key surfaces
/// as `TemperError::BadRequest`.
///
/// Callers land in Task 8 (`update_resource`); remove `dead_code` suppression
/// when that task lands.
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "caller lands in Task 8 (VaultBackend::update_resource); \
                  scaffolded now for Task 4"
    )
)]
pub(crate) fn cmd_to_update_request(
    cmd: &UpdateResource,
    body_trio: Option<&BodyTrio>,
) -> Result<ResourceUpdateRequest, TemperError> {
    let (title, slug) = cmd
        .managed_meta
        .as_ref()
        .map(|m| (m.title.clone(), m.slug.clone()))
        .unwrap_or((None, None));

    // Validate open_meta keys upfront — fires for both body-bearing and meta-only updates.
    if let Some(open_meta) = cmd.open_meta.as_ref() {
        if let Err(bad_key) = temper_core::operations::validate_open_meta_keys(open_meta) {
            return Err(TemperError::BadRequest(format!(
                "unknown open_meta key '{bad_key}'"
            )));
        }
    }

    let (content, content_hash, chunks_packed) = if let Some(body) = cmd.body.as_ref() {
        let trio = body_trio.ok_or_else(|| {
            TemperError::BadRequest(
                "body update requires precomputed body_trio (vault caller responsibility)"
                    .to_owned(),
            )
        })?;
        (
            Some(body.content.clone()),
            Some(trio.content_hash.clone()),
            Some(trio.chunks_packed.clone()),
        )
    } else {
        (None, None, None)
    };

    Ok(ResourceUpdateRequest {
        title,
        slug,
        managed_meta: cmd.managed_meta.clone(),
        open_meta: cmd.open_meta.clone(),
        content,
        content_hash,
        chunks_packed,
    })
}

// Tests for resolve_resource_ref. These are unconditional (no embed dependency).
#[cfg(test)]
mod resolve_tests {
    use std::collections::HashMap;
    use std::fs;

    use chrono::Utc;
    use temper_core::operations::ResourceRef;
    use temper_core::types::ids::ResourceId;
    use temper_core::types::manifest::{Manifest, ManifestEntry, ManifestEntryState};
    use uuid::Uuid;

    use super::resolve_resource_ref;
    use crate::config::Config;

    fn make_test_config(vault_root: &std::path::Path) -> Config {
        Config {
            vault_root: vault_root.to_path_buf(),
            state_dir: vault_root.join(".temper"),
            contexts: vec!["temper".to_string()],
            subscriptions: Vec::new(),
            skill_output: vault_root.join("skill-output"),
            profile_slug: None,
        }
    }

    fn make_manifest_entry(rel_path: &str) -> ManifestEntry {
        ManifestEntry {
            path: rel_path.to_string(),
            body_hash: "sha256:abc".to_string(),
            remote_body_hash: "sha256:abc".to_string(),
            managed_hash: String::new(),
            open_hash: String::new(),
            remote_managed_hash: String::new(),
            remote_open_hash: String::new(),
            synced_at: Utc::now(),
            state: ManifestEntryState::Clean,
            mtime_secs: None,
            provisional: false,
            last_audit_id: None,
        }
    }

    #[test]
    fn resolve_uuid_hits_manifest_entry() {
        let tmp = tempfile::tempdir().unwrap();
        let id = ResourceId::from(Uuid::now_v7());
        let rel = "@me/temper/task/foo.md";

        let mut manifest = Manifest::new("test-device".to_string());
        manifest.entries.insert(id, make_manifest_entry(rel));

        let config = make_test_config(tmp.path());
        let rref = ResourceRef::Uuid { id };
        let resolved = resolve_resource_ref(tmp.path(), &manifest, &config, &rref).unwrap();
        assert_eq!(resolved.resource_id, id);
        assert_eq!(resolved.path, tmp.path().join(rel));
    }

    #[test]
    fn resolve_uuid_missing_entry_returns_not_found() {
        let tmp = tempfile::tempdir().unwrap();
        let manifest = Manifest {
            device_id: "test-device".to_string(),
            last_sync: None,
            entries: HashMap::new(),
        };
        let config = make_test_config(tmp.path());
        let id = ResourceId::from(Uuid::now_v7());
        let rref = ResourceRef::Uuid { id };
        let err = resolve_resource_ref(tmp.path(), &manifest, &config, &rref).unwrap_err();
        assert!(
            matches!(err, temper_core::error::TemperError::NotFound(_)),
            "expected NotFound, got: {err:?}"
        );
    }

    #[test]
    fn resolve_scoped_delegates_to_find_resource() {
        let tmp = tempfile::tempdir().unwrap();
        let task_dir = tmp.path().join("@me").join("temper").join("task");
        fs::create_dir_all(&task_dir).unwrap();
        let task_path = task_dir.join("hello-world.md");

        let id = ResourceId::from(Uuid::now_v7());
        let content = format!(
            "---\ntemper-id: {}\ntemper-context: temper\ntemper-type: task\ntemper-title: 'Hello world'\ntemper-slug: hello-world\n---\n\n# Hello\n",
            *id
        );
        fs::write(&task_path, content).unwrap();

        let manifest = Manifest::new("test-device".to_string());
        let config = make_test_config(tmp.path());
        let rref = ResourceRef::Scoped {
            owner: "@me".to_string(),
            context: "temper".to_string(),
            doctype: "task".to_string(),
            slug: "hello-world".to_string(),
        };
        let resolved = resolve_resource_ref(tmp.path(), &manifest, &config, &rref).unwrap();
        assert_eq!(resolved.path, task_path);
        assert_eq!(resolved.resource_id, id);
    }

    #[test]
    fn resolve_scoped_no_id_in_frontmatter_or_manifest_returns_not_found() {
        let tmp = tempfile::tempdir().unwrap();
        let task_dir = tmp.path().join("@me").join("temper").join("task");
        fs::create_dir_all(&task_dir).unwrap();
        let task_path = task_dir.join("no-id-task.md");

        // File with no temper-id and no temper-provisional-id
        fs::write(
            &task_path,
            "---\ntemper-context: temper\ntemper-type: task\ntemper-title: 'No ID'\ntemper-slug: no-id-task\n---\n\n",
        ).unwrap();

        let manifest = Manifest::new("test-device".to_string());
        let config = make_test_config(tmp.path());
        let rref = ResourceRef::Scoped {
            owner: "@me".to_string(),
            context: "temper".to_string(),
            doctype: "task".to_string(),
            slug: "no-id-task".to_string(),
        };
        let err = resolve_resource_ref(tmp.path(), &manifest, &config, &rref).unwrap_err();
        assert!(
            matches!(err, temper_core::error::TemperError::NotFound(_)),
            "expected NotFound for file with no temper-id, got: {err:?}"
        );
    }
}

// Only one test exists here and it's gated on not(embed), so the whole
// test module is guarded to avoid an unused-import warning under --all-features.
#[cfg(all(test, not(feature = "embed")))]
mod tests {
    use super::*;

    #[test]
    fn prepare_body_trio_no_embed_returns_bad_request() {
        let err = prepare_body_trio("body").expect_err("no-embed path");
        assert!(matches!(
            err,
            temper_core::error::TemperError::BadRequest(_)
        ));
    }
}

#[cfg(test)]
mod translator_tests {
    use temper_core::operations::{
        BodyUpdate, CreateResource, ResourceRef, Surface, UpdateResource,
    };
    use temper_core::types::ids::ResourceId;
    use temper_core::types::managed_meta::ManagedMeta;
    use uuid::Uuid;

    use super::{cmd_to_ingest_payload, cmd_to_update_request, BodyTrio};

    fn make_create_cmd() -> CreateResource {
        CreateResource {
            slug: "my-task".to_string(),
            doctype: "task".to_string(),
            context: "temper".to_string(),
            title: "My Task".to_string(),
            body: None,
            managed_meta: ManagedMeta::default(),
            open_meta: None,
            origin_uri: Some("kb://temper/task/my-task".to_string()),
            chunks_packed: None,
            content_hash: None,
            origin: Surface::CliLocalVault,
        }
    }

    fn make_update_cmd() -> UpdateResource {
        UpdateResource {
            resource: ResourceRef::Uuid {
                id: ResourceId(Uuid::now_v7()),
            },
            body: None,
            managed_meta: None,
            open_meta: None,
            origin: Surface::CliLocalVault,
        }
    }

    // ── cmd_to_ingest_payload tests ───────────────────────────────────────────

    #[test]
    fn cmd_to_ingest_payload_carries_managed_meta_and_body() {
        let mut cmd = make_create_cmd();
        cmd.managed_meta = ManagedMeta {
            stage: Some("backlog".to_string()),
            ..Default::default()
        };
        let body = "# My Task\n\nsome content";
        let payload = cmd_to_ingest_payload(&cmd, body, None);

        assert_eq!(payload.title, "My Task");
        assert_eq!(payload.context_name, "temper");
        assert_eq!(payload.doc_type_name, "task");
        assert_eq!(payload.slug, "my-task");
        assert_eq!(payload.content, body);
        assert_eq!(payload.origin_uri, "kb://temper/task/my-task");
        // managed_meta serializes with serde renames (e.g. "temper-stage")
        let mm = payload.managed_meta.expect("managed_meta present");
        assert_eq!(
            mm.get("temper-stage").and_then(|v| v.as_str()),
            Some("backlog")
        );
        // No body_trio supplied → hash + chunks stay None
        assert!(payload.content_hash.is_none());
        assert!(payload.chunks_packed.is_none());
        assert!(payload.metadata.is_none());
    }

    #[test]
    fn cmd_to_ingest_payload_empty_body_when_no_body_supplied() {
        let cmd = make_create_cmd();
        let payload = cmd_to_ingest_payload(&cmd, "", None);
        assert_eq!(payload.content, "");
        assert!(payload.content_hash.is_none());
        assert!(payload.chunks_packed.is_none());
    }

    #[test]
    fn cmd_to_ingest_payload_uses_supplied_body_trio_over_cmd_fields() {
        let mut cmd = make_create_cmd();
        // Populate cmd-level hash/chunks that the trio should shadow
        cmd.content_hash = Some("sha256:cmd-level-hash".to_string());
        cmd.chunks_packed = Some("cmd-level-chunks".to_string());

        let trio = BodyTrio {
            content_hash: "sha256:trio-hash".to_string(),
            chunks_packed: "trio-chunks".to_string(),
        };
        let payload = cmd_to_ingest_payload(&cmd, "body text", Some(&trio));

        assert_eq!(payload.content_hash.as_deref(), Some("sha256:trio-hash"));
        assert_eq!(payload.chunks_packed.as_deref(), Some("trio-chunks"));
    }

    // ── cmd_to_update_request tests ──────────────────────────────────────────

    #[test]
    fn cmd_to_update_request_meta_only_branch_leaves_body_fields_none() {
        let mut cmd = make_update_cmd();
        cmd.managed_meta = Some(ManagedMeta {
            stage: Some("done".to_string()),
            ..Default::default()
        });
        cmd.open_meta = Some(serde_json::json!({"tags": ["x"]}));

        let req = cmd_to_update_request(&cmd, None).expect("meta-only ok");
        assert!(req.content.is_none());
        assert!(req.content_hash.is_none());
        assert!(req.chunks_packed.is_none());
        assert!(req.managed_meta.is_some());
        assert!(req.open_meta.is_some());
    }

    #[test]
    fn cmd_to_update_request_rejects_unknown_open_meta_key() {
        let mut cmd = make_update_cmd();
        cmd.open_meta = Some(serde_json::json!({"totally_made_up": 1}));

        let err = cmd_to_update_request(&cmd, None).expect_err("unknown key");
        match err {
            temper_core::error::TemperError::BadRequest(msg) => {
                assert!(msg.contains("totally_made_up"), "msg = {msg}");
                assert!(msg.contains("unknown open_meta key"), "msg = {msg}");
            }
            other => panic!("expected BadRequest, got {other:?}"),
        }
    }

    #[test]
    fn cmd_to_update_request_body_branch_requires_trio() {
        let mut cmd = make_update_cmd();
        cmd.body = Some(BodyUpdate::new("hello world"));

        let err = cmd_to_update_request(&cmd, None).expect_err("body without trio");
        assert!(
            matches!(err, temper_core::error::TemperError::BadRequest(_)),
            "expected BadRequest, got {err:?}"
        );
    }

    #[test]
    fn cmd_to_update_request_body_branch_populates_trio_fields() {
        let mut cmd = make_update_cmd();
        cmd.body = Some(BodyUpdate::new("hello world"));
        cmd.managed_meta = Some(ManagedMeta {
            title: Some("Updated Title".to_string()),
            slug: Some("updated-slug".to_string()),
            ..Default::default()
        });

        let trio = BodyTrio {
            content_hash: "sha256:abc123".to_string(),
            chunks_packed: "packed-data".to_string(),
        };
        let req = cmd_to_update_request(&cmd, Some(&trio)).expect("with trio ok");

        assert_eq!(req.content.as_deref(), Some("hello world"));
        assert_eq!(req.content_hash.as_deref(), Some("sha256:abc123"));
        assert_eq!(req.chunks_packed.as_deref(), Some("packed-data"));
        assert_eq!(req.title.as_deref(), Some("Updated Title"));
        assert_eq!(req.slug.as_deref(), Some("updated-slug"));
    }
}
