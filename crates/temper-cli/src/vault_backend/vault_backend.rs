//! `VaultBackend` struct + `impl Backend` for vault-file persistence.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use chrono::Utc;
use tokio::sync::Mutex;
use uuid::Uuid;

use temper_client::TemperClient;
use temper_core::error::TemperError;
use temper_core::frontmatter::Frontmatter;
use temper_core::operations::{
    Backend, CommandOutput, CreateResource, DeleteResource, ListResources, ResourceRef,
    ResourceSummary, SearchHit, SearchResources, ShowResource, Surface, UpdateResource,
};
use temper_core::types::ids::{ContextId, DocTypeId, ProfileId, ResourceId};
use temper_core::types::manifest::Manifest;
use temper_core::types::resource::ResourceRow;

use crate::config::Config;

/// Local-file-backed backend impl. Constructed per inbound CLI invocation.
///
/// See `docs/superpowers/specs/2026-05-11-wave1-phase4-vaultbackend-design.md`.
///
/// Fields exceed the project's 5-arg threshold — `VaultBackendCtx` builder
/// is mandatory for construction. Fields above the project's 5-param threshold;
/// already using a params struct per the design spec.
pub struct VaultBackend {
    vault_root: PathBuf,
    manifest: Arc<Mutex<Manifest>>,
    client: Option<Arc<TemperClient>>,
    /// Owner sigil (e.g. `"@me"` or `"+team-..."`) used for vault path
    /// construction via `Vault::doc_file`. Stored as a string because
    /// `OwnerHandle` does not yet exist in `temper-core` (aspirational in
    /// the spec; deferred to a future refactor).
    owner: String,
    config: Arc<Config>,
    /// Origin of the inbound command. Today always `CliLocalVault`; stored
    /// for forward-compat (Phase 6 telemetry/event tagging).
    #[expect(dead_code, reason = "stored for Phase 6 telemetry; not yet consumed")]
    surface: Surface,
}

/// Builder / context for constructing a `VaultBackend`.
///
/// All fields are public so call-sites can build the struct directly without a
/// further builder method. The ctx struct is the 6-field params struct required
/// by the project's "params structs at 5+ args" rule.
pub struct VaultBackendCtx {
    pub vault_root: PathBuf,
    pub manifest: Arc<Mutex<Manifest>>,
    pub client: Option<Arc<TemperClient>>,
    pub owner: String,
    pub config: Arc<Config>,
    pub surface: Surface,
}

impl VaultBackend {
    /// Construct from a fully-populated `VaultBackendCtx`.
    pub fn new(ctx: VaultBackendCtx) -> Self {
        Self {
            vault_root: ctx.vault_root,
            manifest: ctx.manifest,
            client: ctx.client,
            owner: ctx.owner,
            config: ctx.config,
            surface: ctx.surface,
        }
    }

    #[expect(
        dead_code,
        reason = "getter used by Tasks 6+ (list_resources/create_resource); \
                  direct field access used inside this file"
    )]
    pub(crate) fn vault_root(&self) -> &Path {
        &self.vault_root
    }

    #[expect(
        dead_code,
        reason = "getter used by Tasks 6+ (list_resources); \
                  direct field access used inside this file"
    )]
    pub(crate) fn manifest(&self) -> &Arc<Mutex<Manifest>> {
        &self.manifest
    }

    #[expect(
        dead_code,
        reason = "getter used by Tasks 7+ (create_resource push tail); \
                  direct field access used inside this file"
    )]
    pub(crate) fn client(&self) -> Option<&Arc<TemperClient>> {
        self.client.as_ref()
    }

    // Dead in lib target; called from tests only in Task 2. Real callers land
    // in Task 3+. Remove the cfg_attr suppression when Task 3 lands.
    #[cfg_attr(not(test), expect(dead_code, reason = "lib callers land in Task 3+"))]
    pub(crate) fn owner(&self) -> &str {
        &self.owner
    }

    #[expect(
        dead_code,
        reason = "getter used by Tasks 3+ (resolve_resource_ref); \
                  direct field access used inside this file"
    )]
    pub(crate) fn config(&self) -> &Config {
        &self.config
    }

    /// API fallback for `show_resource` when the local vault file is missing
    /// and a client is available. Resolves content via the cloud API.
    ///
    /// For `ResourceRef::Uuid`, calls `client.resources().content` directly.
    /// For `ResourceRef::Scoped`, first resolves to a UUID via
    /// `client.resources().resolve_by_uri`, then fetches content.
    async fn fallback_show_via_api(
        &self,
        rref: &ResourceRef,
        client: &Arc<TemperClient>,
    ) -> Result<ResourceRow, TemperError> {
        use crate::actions::runtime::client_err_to_temper;

        let resource_id: ResourceId = match rref {
            ResourceRef::Uuid { id } => *id,
            ResourceRef::Scoped {
                owner,
                context,
                doctype,
                slug,
            } => {
                let row = client
                    .resources()
                    .resolve_by_uri(owner, context, doctype, slug)
                    .await
                    .map_err(client_err_to_temper)?;
                row.id
            }
        };

        let content = client
            .resources()
            .content(*resource_id.as_uuid())
            .await
            .map_err(client_err_to_temper)?;

        // Project ContentResponse → ResourceRow. DB-scoped IDs are nil because
        // VaultBackend read paths don't have a pool; callers that need the DB
        // row must use DbBackend directly. These nil sentinels signal
        // "vault-sourced row" and are documented in the struct's field comments.
        let managed = content.managed_meta.unwrap_or_default();
        let title = managed
            .title
            .clone()
            .unwrap_or_else(|| "Untitled".to_string());
        let now = Utc::now();
        let updated = managed
            .updated
            .as_deref()
            .and_then(|s| s.parse().ok())
            .unwrap_or(now);
        let context_name = managed.context.clone().unwrap_or_default();
        let doc_type_name = managed
            .doc_type
            .clone()
            .unwrap_or_else(|| "task".to_string());

        Ok(ResourceRow {
            id: resource_id,
            kb_context_id: ContextId::from(Uuid::nil()),
            kb_doc_type_id: DocTypeId::from(Uuid::nil()),
            origin_uri: String::new(),
            title,
            slug: managed.slug.clone(),
            originator_profile_id: ProfileId::from(Uuid::nil()),
            owner_profile_id: ProfileId::from(Uuid::nil()),
            is_active: true,
            created: now,
            updated,
            context_name,
            doc_type_name,
            owner_handle: self.owner.clone(),
            stage: managed.stage.clone(),
            seq: managed.seq,
            mode: managed.mode.clone(),
            effort: managed.effort.clone(),
            body_hash: None,
            managed_hash: None,
            open_hash: None,
        })
    }
}

#[async_trait]
impl Backend for VaultBackend {
    async fn create_resource(
        &self,
        _cmd: CreateResource,
    ) -> Result<CommandOutput<ResourceRow>, TemperError> {
        unimplemented!("wave1-4a Task 7 lands this")
    }

    async fn show_resource(
        &self,
        cmd: ShowResource,
    ) -> Result<CommandOutput<ResourceRow>, TemperError> {
        use crate::vault_backend::translators;

        // Resolve under lock; release before any I/O.
        let resolved = {
            let manifest = self.manifest.lock().await;
            translators::resolve_resource_ref(
                &self.vault_root,
                &manifest,
                &self.config,
                &cmd.resource,
            )
        };

        match resolved {
            Ok(r) if r.path.exists() => {
                let fm = Frontmatter::parse_file(&r.path)?;
                let row = vault_file_to_resource_row(
                    &r.path,
                    &fm,
                    r.resource_id,
                    &self.vault_root,
                    &self.owner,
                );
                Ok(CommandOutput::new(row))
            }
            // LocallyMissing or NotFound — try API fallback when client is available.
            _ if self.client.is_some() => {
                let client = self.client.as_ref().expect("just checked is_some");
                let row = self.fallback_show_via_api(&cmd.resource, client).await?;
                Ok(CommandOutput::new(row))
            }
            Ok(_) => Err(TemperError::NotFound(format!(
                "local file missing and no client available: {:?}",
                cmd.resource
            ))),
            Err(e) => Err(e),
        }
    }

    async fn update_resource(
        &self,
        _cmd: UpdateResource,
    ) -> Result<CommandOutput<ResourceRow>, TemperError> {
        unimplemented!("wave1-4a Task 8 lands this")
    }

    async fn delete_resource(
        &self,
        _cmd: DeleteResource,
    ) -> Result<CommandOutput<()>, TemperError> {
        unimplemented!("wave1-4a Task 9 lands this")
    }

    async fn list_resources(
        &self,
        cmd: ListResources,
    ) -> Result<CommandOutput<Vec<ResourceSummary>>, TemperError> {
        use crate::commands::resource::{filter_rows, scan_rows, sort_rows, ListFilters};

        let filter = cmd.filter;
        let doctype = filter.doctype.as_deref().ok_or_else(|| {
            TemperError::BadRequest("list_resources requires a doctype filter".to_owned())
        })?;
        let context_str = filter.context.as_deref();

        let rows = scan_rows(&self.config, doctype, context_str)?;

        let filters = ListFilters {
            stage: filter.stage.as_deref(),
            goal: filter.goal.as_deref(),
            status: None, // ListFilter does not carry a status field
        };
        let mut rows = filter_rows(rows, filters);
        sort_rows(&mut rows);

        if let Some(limit) = filter.limit {
            rows.truncate(limit as usize);
        }

        let summaries: Vec<ResourceSummary> = rows
            .into_iter()
            .map(|row| {
                let get_str = |key: &str| -> String {
                    row.frontmatter
                        .get(key)
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string()
                };
                ResourceSummary {
                    slug: get_str("temper-slug"),
                    doctype: get_str("temper-type"),
                    context: get_str("temper-context"),
                    title: get_str("temper-title"),
                }
            })
            .collect();

        // Read path — emit no events (per Phase 3 / Task 5 precedent).
        Ok(CommandOutput::new(summaries))
    }

    async fn search_resources(
        &self,
        cmd: SearchResources,
    ) -> Result<CommandOutput<Vec<SearchHit>>, TemperError> {
        use crate::actions::runtime::client_err_to_temper;

        let client = self.client.as_ref().ok_or_else(|| {
            TemperError::BadRequest(
                "search requires an authenticated client (local search is unavailable)".to_owned(),
            )
        })?;

        let q = &cmd.query;
        let rows = client
            .search()
            .text_query(
                &q.query,
                q.context.clone(),
                q.doctype.clone(),
                q.limit.map(|n| n as i64),
            )
            .await
            .map_err(client_err_to_temper)?;

        let hits: Vec<SearchHit> = rows
            .into_iter()
            .map(|row| SearchHit {
                summary: ResourceSummary {
                    slug: row.slug,
                    doctype: row.doc_type,
                    context: row.context.unwrap_or_default(),
                    title: row.title,
                },
                score: row.combined_score,
            })
            .collect();

        // Read path — emit no events.
        Ok(CommandOutput::new(hits))
    }
}

/// Project a parsed vault file into a `ResourceRow`.
///
/// DB-scoped ID fields (`kb_context_id`, `kb_doc_type_id`,
/// `originator_profile_id`, `owner_profile_id`) are set to `Uuid::nil()`
/// because `VaultBackend` read paths have no database pool. Callers that need
/// the authoritative DB row must use `DbBackend` directly. The nil sentinel
/// signals "vault-sourced row".
fn vault_file_to_resource_row(
    path: &Path,
    fm: &Frontmatter,
    resource_id: ResourceId,
    _vault_root: &Path,
    owner: &str,
) -> ResourceRow {
    let value = fm.value();
    let get_str = |key: &str| -> Option<String> {
        value
            .get(serde_yaml::Value::String(key.to_string()))
            .and_then(|v| v.as_str())
            .map(str::to_string)
    };
    let get_i64 = |key: &str| -> Option<i64> {
        value
            .get(serde_yaml::Value::String(key.to_string()))
            .and_then(|v| v.as_i64())
    };

    let title = get_str("temper-title").unwrap_or_else(|| {
        path.file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("Untitled")
            .to_string()
    });
    let slug = get_str("temper-slug");
    let context_name = get_str("temper-context").unwrap_or_default();
    let doc_type_name = fm.doc_type().as_str().to_string();
    let stage = get_str("temper-stage");
    let mode = get_str("temper-mode");
    let effort = get_str("temper-effort");
    let seq = get_i64("temper-seq");

    let now = Utc::now();
    let updated = get_str("temper-updated")
        .as_deref()
        .and_then(|s| s.parse().ok())
        .unwrap_or(now);
    let created = get_str("temper-created")
        .as_deref()
        .and_then(|s| s.parse().ok())
        .unwrap_or(now);

    ResourceRow {
        id: resource_id,
        kb_context_id: ContextId::from(Uuid::nil()),
        kb_doc_type_id: DocTypeId::from(Uuid::nil()),
        origin_uri: String::new(),
        title,
        slug,
        originator_profile_id: ProfileId::from(Uuid::nil()),
        owner_profile_id: ProfileId::from(Uuid::nil()),
        is_active: true,
        created,
        updated,
        context_name,
        doc_type_name,
        owner_handle: owner.to_string(),
        stage,
        seq,
        mode,
        effort,
        body_hash: None,
        managed_hash: None,
        open_hash: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_test_ctx() -> VaultBackendCtx {
        let tmp = tempfile::tempdir().unwrap();
        let vault_root = tmp.path().to_path_buf();
        let manifest = Arc::new(Mutex::new(Manifest::new("test-device".to_string())));
        let config = Arc::new(Config {
            vault_root: vault_root.clone(),
            state_dir: vault_root.join(".temper"),
            contexts: vec![],
            subscriptions: vec![],
            skill_output: vault_root.join("skills"),
            profile_slug: None,
        });
        VaultBackendCtx {
            vault_root,
            manifest,
            client: None,
            owner: "@me".to_string(),
            config,
            surface: Surface::CliLocalVault,
        }
    }

    #[test]
    fn vault_backend_new_constructs_from_ctx() {
        let ctx = make_test_ctx();
        let backend = VaultBackend::new(ctx);
        assert_eq!(backend.owner(), "@me");
    }
}
