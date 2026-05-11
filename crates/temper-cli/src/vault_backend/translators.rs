//! Pure cmd → vault-flow translators (no I/O).

use std::path::{Path, PathBuf};

use temper_core::error::TemperError;
#[cfg(feature = "embed")]
use temper_core::hash::compute_body_hash;
use temper_core::operations::ResourceRef;
use temper_core::types::ids::ResourceId;
use temper_core::types::manifest::Manifest;

use crate::config::Config;

/// A resolved resource: its stable UUID and the absolute path to its vault file.
///
/// Returned by [`resolve_resource_ref`] so callers have both the identity key
/// (for manifest lookups and API calls) and the filesystem path (for reads /
/// writes / deletes) without a second parse.
///
/// First consumed by Task 5 (`show_resource`) and Tasks 7-9 (write/delete paths).
/// Remove `dead_code` suppression when Task 5 lands.
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "lib callers land in Tasks 5+ (show/create/update/delete); \
                  scaffolded now alongside resolve_resource_ref"
    )
)]
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
///
/// First called by Task 5 (`Backend::show_resource`); subsequent tasks consume
/// it for every write and delete path. Remove `dead_code` suppression when
/// Task 5 lands.
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "lib callers land in Tasks 5+ (show/create/update/delete); \
                  scaffolded now so the fn is in place for Task 5"
    )
)]
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
/// Callers land in Task 4 (`cmd_to_update_request`) and Tasks 7-8 (create/update).
/// Remove the `dead_code` suppressions when those tasks land.
#[expect(
    dead_code,
    reason = "callers land in Tasks 4/7/8 (Phase 4a); scaffolded now \
              so the type is in place for Task 4's cmd_to_update_request"
)]
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
#[expect(
    dead_code,
    reason = "callers land in Tasks 7-8 (Phase 4a create/update body path); \
              remove suppression when Task 7 lands"
)]
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
