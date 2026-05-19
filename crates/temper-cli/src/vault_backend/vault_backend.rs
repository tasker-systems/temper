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
    Backend, CommandOutput, CreateResource, DeleteResource, DomainEvent, ListResources,
    PushDeferReason, ResourceRef, ResourceSummary, SearchHit, SearchResources, ShowResource,
    Surface, UpdateResource,
};
use temper_core::types::ids::{ContextId, DocTypeId, ProfileId, ResourceId};
use temper_core::types::manifest::{Manifest, ManifestEntry, ManifestEntryState};
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

/// Backend-specific compute results for a `CreateResource` command.
///
/// Holds per-doctype default values that require filesystem access
/// (next_seq for tasks). Populated by `compute_task_defaults` before
/// dispatching to `per_doctype::write_for`.
#[derive(Debug, Default)]
pub(crate) struct TaskDefaults {
    pub(crate) seq: Option<u32>,
}

/// Compute backend-specific defaults for a Create cmd.
///
/// For tasks, walks the vault to find `next_seq` and verifies that the
/// referenced goal exists. For goals, computes the next sequential seq so
/// goals sort predictably (matches the behavior `actions::goal::create` had
/// before it was removed in Phase 5c). For other doctypes, returns an empty
/// `TaskDefaults`. Returns an error variant if the referenced goal is
/// missing or the doctype is malformed.
pub(crate) fn compute_task_defaults(
    config: &Config,
    cmd: &temper_core::operations::CreateResource,
) -> Result<TaskDefaults, TemperError> {
    let doctype = temper_core::frontmatter::DocType::from_str(&cmd.doctype)
        .map_err(|e| TemperError::BadRequest(format!("invalid doctype '{}': {e}", cmd.doctype)))?;

    match doctype {
        temper_core::frontmatter::DocType::Task => {
            let goal_slug = cmd.managed_meta.goal.as_deref().ok_or_else(|| {
                TemperError::BadRequest(
                    "task create requires managed_meta.goal (the parent goal slug)".to_string(),
                )
            })?;

            // Goal-exists check.
            if crate::actions::goal::find_goal(config, goal_slug, Some(&cmd.context))?.is_none() {
                return Err(TemperError::NotFound(format!(
                    "goal '{goal_slug}' not found in context '{}'",
                    cmd.context
                )));
            }

            let seq = crate::actions::task::next_seq(config, &cmd.context, goal_slug)?;
            Ok(TaskDefaults { seq: Some(seq) })
        }
        temper_core::frontmatter::DocType::Goal => {
            // Auto-assign seq when the caller doesn't supply one. This matches
            // what the deleted `actions::goal::create` did before Phase 5c.
            // Without this, all goals get seq=0 and sort non-deterministically.
            if cmd.managed_meta.seq.is_none() {
                let seq = crate::actions::goal::next_seq(config, &cmd.context)?;
                Ok(TaskDefaults { seq: Some(seq) })
            } else {
                Ok(TaskDefaults::default())
            }
        }
        _ => Ok(TaskDefaults::default()),
    }
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

    /// Access the manifest; used by tests and the Task 8+ update/delete paths.
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "lib callers land in Tasks 8-9 (update_resource/delete_resource); \
                      tests call this for manifest entry assertions"
        )
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

    /// Push a freshly-created vault file to the API as a tail action.
    ///
    /// Returns the `DomainEvent` to append: `RemoteSynced` on success,
    /// `PushDeferred` otherwise. Never fails — push errors are classified
    /// and returned as events rather than propagated.
    async fn push_create(
        &self,
        cmd: &CreateResource,
        body: &str,
        written: &crate::vault_backend::per_doctype::WriteResult,
    ) -> DomainEvent {
        let Some(client) = self.client.as_ref() else {
            return DomainEvent::PushDeferred {
                reason: PushDeferReason::Offline,
            };
        };

        // Compute body trio when body is non-empty and the embed feature is present.
        let body_trio = if !body.is_empty() {
            match crate::vault_backend::translators::prepare_body_trio(body) {
                Ok(t) => Some(t),
                Err(_) => {
                    // prepare_body_trio errors without the embed feature — treat as
                    // deferred rather than a hard failure.
                    return DomainEvent::PushDeferred {
                        reason: PushDeferReason::Other,
                    };
                }
            }
        } else {
            None
        };

        let payload =
            crate::vault_backend::translators::cmd_to_ingest_payload(cmd, body, body_trio.as_ref());

        match client.ingest().create(&payload).await {
            Ok(row) => {
                // Promote manifest entry to Clean / non-provisional with the server's id.
                let _ = self
                    .promote_manifest_after_push(row.id, &written.rel_path)
                    .await;
                // Rewrite the on-disk frontmatter so `temper-provisional-id` becomes
                // the server-canonical `temper-id`. Mirrors `actions::sync::publish_local_write`'s
                // post-push file rewrite (lines 1460-1486) — the gap that the
                // pre-Phase-5 `publish_local_write_best_effort` tail-call closed.
                let _ = self
                    .update_file_id_after_push(&written.abs_path, row.id)
                    .await;
                DomainEvent::RemoteSynced {
                    resource_id: row.id,
                }
            }
            Err(e) => self.classify_push_error(&e),
        }
    }

    /// Promote a manifest entry from Provisional→Clean after a successful push.
    ///
    /// Locates the provisional entry by `rel_path` (stable across id changes if
    /// the server deduplicated onto an existing resource), removes the provisional
    /// keyed entry, and re-inserts under the server-returned `server_id`.
    /// Silently ignores errors — promotion is best-effort; `sync run` reconciles.
    async fn promote_manifest_after_push(
        &self,
        server_id: ResourceId,
        rel_path: &str,
    ) -> Result<(), TemperError> {
        let mut manifest = self.manifest.lock().await;

        let provisional_id = manifest
            .entries
            .iter()
            .find(|(_, e)| e.path == rel_path)
            .map(|(id, _)| *id);

        if let Some(prov_id) = provisional_id {
            if let Some(entry) = manifest.entries.remove(&prov_id) {
                let promoted = ManifestEntry {
                    state: ManifestEntryState::Clean,
                    provisional: false,
                    ..entry
                };
                manifest.entries.insert(server_id, promoted);
                let _ = crate::manifest_io::save_manifest(&self.config.state_dir, &manifest);
            }
        }
        Ok(())
    }

    /// Rewrite the on-disk frontmatter after a successful push: replace
    /// `temper-provisional-id` with the server-canonical `temper-id`.
    ///
    /// Mirrors `actions::sync::publish_local_write`'s file-rewrite step
    /// (sync.rs lines 1460-1486). Silently ignores read/write/parse errors —
    /// `sync run` reconciles any drift, same policy as
    /// `promote_manifest_after_push`.
    async fn update_file_id_after_push(
        &self,
        abs_path: &std::path::Path,
        server_id: ResourceId,
    ) -> Result<(), TemperError> {
        let Ok(content) = std::fs::read_to_string(abs_path) else {
            return Ok(());
        };
        let server_uuid = uuid::Uuid::from(server_id);
        // Use a regex-free string replace mirroring sync.rs's pattern: try
        // quoted form first (the template renders `temper-provisional-id: "{uuid}"`),
        // then unquoted as a fallback.
        let updated = match Self::rewrite_provisional_to_canonical(&content, server_uuid) {
            Some(s) => s,
            None => return Ok(()),
        };
        if updated != content {
            let _ = std::fs::write(abs_path, &updated);
        }
        Ok(())
    }

    /// Pure string-replace half of `update_file_id_after_push`. Returns
    /// `Some(updated)` when a replacement occurred, `None` when no
    /// `temper-provisional-id` line was found.
    fn rewrite_provisional_to_canonical(content: &str, server_uuid: uuid::Uuid) -> Option<String> {
        // Search the YAML frontmatter for `temper-provisional-id:` and capture
        // the UUID value (quoted or unquoted), then replace the whole line
        // with `temper-id: "{server_uuid}"` (template uses the quoted form).
        for line in content.lines() {
            let trimmed = line.trim_start();
            let Some(rest) = trimmed.strip_prefix("temper-provisional-id:") else {
                continue;
            };
            let value = rest.trim().trim_matches('"');
            if uuid::Uuid::parse_str(value).is_err() {
                continue;
            }
            // Found it — replace the line.
            let needle = line;
            let replacement = {
                // Preserve original indentation.
                let indent_len = line.len() - trimmed.len();
                let indent = &line[..indent_len];
                format!("{indent}temper-id: \"{server_uuid}\"")
            };
            return Some(content.replacen(needle, &replacement, 1));
        }
        None
    }

    /// Classify a `ClientError` into a `DomainEvent::PushDeferred` with the
    /// appropriate reason.
    fn classify_push_error(&self, e: &temper_client::error::ClientError) -> DomainEvent {
        use temper_client::error::ClientError;
        let reason = match e {
            ClientError::NotAuthenticated | ClientError::TokenExpired | ClientError::Forbidden => {
                PushDeferReason::NotAuthed
            }
            ClientError::Network(_) => PushDeferReason::Offline,
            _ => PushDeferReason::Other,
        };
        DomainEvent::PushDeferred { reason }
    }

    /// Push an updated vault file to the API as a tail action.
    ///
    /// Returns the `DomainEvent` to append: `RemoteSynced` on success,
    /// `PushDeferred` otherwise. Never fails — push errors are classified
    /// and returned as events rather than propagated.
    async fn push_update(&self, cmd: &UpdateResource, resource_id: ResourceId) -> DomainEvent {
        let Some(client) = self.client.as_ref() else {
            return DomainEvent::PushDeferred {
                reason: PushDeferReason::Offline,
            };
        };

        // Compute body trio when body is present and embed feature is active.
        let body_trio = if let Some(body_update) = cmd.body.as_ref() {
            if body_update.content.is_empty() {
                None
            } else {
                match crate::vault_backend::translators::prepare_body_trio(&body_update.content) {
                    Ok(t) => Some(t),
                    Err(_) => {
                        return DomainEvent::PushDeferred {
                            reason: PushDeferReason::Other,
                        };
                    }
                }
            }
        } else {
            None
        };

        let req =
            match crate::vault_backend::translators::cmd_to_update_request(cmd, body_trio.as_ref())
            {
                Ok(r) => r,
                Err(_) => {
                    return DomainEvent::PushDeferred {
                        reason: PushDeferReason::Other,
                    };
                }
            };

        match client.resources().update(*resource_id, &req).await {
            Ok(_row) => DomainEvent::RemoteSynced { resource_id },
            Err(e) => self.classify_push_error(&e),
        }
    }
}

#[async_trait]
impl Backend for VaultBackend {
    async fn create_resource(
        &self,
        mut cmd: CreateResource,
    ) -> Result<CommandOutput<ResourceRow>, TemperError> {
        use temper_core::operations::{
            apply_defaults_value, ensure_managed_identity_keys, validate_create,
        };

        // 1. Validate (shared) — slug, doctype, context, title all checked.
        validate_create(&cmd).map_err(|e| TemperError::BadRequest(e.to_string()))?;

        // 1b. Compute backend-specific defaults (filesystem access):
        //     - For tasks: walks vault for next_seq and verifies goal exists.
        //     - For non-tasks: no-op (returns empty TaskDefaults).
        //   Runs before managed_meta defaults so seq is in place when
        //   extract_doctype_fields_for_create reads managed_meta.seq.
        let task_defaults = compute_task_defaults(&self.config, &cmd)?;
        if let Some(seq) = task_defaults.seq {
            if cmd.managed_meta.seq.is_none() {
                cmd.managed_meta.seq = Some(seq as i64);
            }
        }

        // 2. Apply doctype defaults + identity keys onto the Value form of managed_meta.
        let mut managed_value = serde_json::to_value(&cmd.managed_meta)
            .map_err(|e| TemperError::BadRequest(format!("managed_meta serialize: {e}")))?;
        apply_defaults_value(&cmd.doctype, &mut managed_value);
        ensure_managed_identity_keys(&mut managed_value, &cmd.title, Some(&cmd.slug));

        // 3. Per-doctype file write dispatch.
        let body_str = cmd.body.as_ref().map(|b| b.content.as_str()).unwrap_or("");
        let written = crate::vault_backend::per_doctype::write_for(
            crate::vault_backend::per_doctype::WriteArgs {
                doctype: &cmd.doctype,
                title: &cmd.title,
                slug: &cmd.slug,
                context: &cmd.context,
                body: body_str,
                open_meta: cmd.open_meta.as_ref(),
                vault_root: &self.vault_root,
                owner: &self.owner,
                config: &self.config,
                // B5a wires task/goal/session/research-specific fields here from
                // `cmd.managed_meta` via `extract_doctype_fields_for_create`.
                // Concept/decision return `None`; task/goal/session/research
                // return the appropriate `DoctypeFields` variant. Task with
                // missing required fields returns `None` and `write_task`
                // hard-errors with `BadRequest` (preserves error visibility).
                doctype_fields:
                    crate::vault_backend::per_doctype::extract_doctype_fields_for_create(&cmd),
            },
        )?;

        let mut events = vec![DomainEvent::VaultFileWritten {
            path: written.rel_path.clone(),
        }];

        // 4. Manifest entry insert (Provisional until push confirms).
        {
            let mut manifest = self.manifest.lock().await;
            let now = chrono::Utc::now();
            let body_hash = if body_str.is_empty() {
                String::new()
            } else {
                temper_core::hash::compute_body_hash(body_str)
            };
            let entry = ManifestEntry {
                path: written.rel_path.clone(),
                body_hash,
                remote_body_hash: String::new(),
                managed_hash: String::new(),
                open_hash: String::new(),
                remote_managed_hash: String::new(),
                remote_open_hash: String::new(),
                synced_at: now,
                state: ManifestEntryState::Pending,
                mtime_secs: None,
                provisional: true,
                last_audit_id: None,
            };
            manifest.entries.insert(written.resource_id, entry);
            crate::manifest_io::save_manifest(&self.config.state_dir, &manifest)?;
        }
        events.push(DomainEvent::VaultManifestUpdated {
            path: written.rel_path.clone(),
        });

        // 5. Push as tail action (if client present).
        let push_event = self.push_create(&cmd, body_str, &written).await;
        events.push(push_event);

        // 6. Project written file → ResourceRow (read-back confirms disk write).
        let fm = Frontmatter::parse_file(&written.abs_path)?;
        let row = vault_file_to_resource_row(
            &written.abs_path,
            &fm,
            written.resource_id,
            &self.vault_root,
            &self.owner,
        );

        Ok(CommandOutput::with_events(row, events))
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
        cmd: UpdateResource,
    ) -> Result<CommandOutput<ResourceRow>, TemperError> {
        use temper_core::operations::{
            apply_defaults_value, ensure_managed_identity_keys, validate_update, DomainEvent,
        };

        // 1. Pre-flight validation (shared).
        validate_update(&cmd).map_err(|e| TemperError::BadRequest(e.to_string()))?;

        // 2. Resolve target file under lock; release before I/O.
        let resolved = {
            let manifest = self.manifest.lock().await;
            crate::vault_backend::translators::resolve_resource_ref(
                &self.vault_root,
                &manifest,
                &self.config,
                &cmd.resource,
            )?
        };

        // 3. Parse on-disk frontmatter.
        let mut fm = Frontmatter::parse_file(&resolved.path)?;

        // 3a. Symmetric-defense receive-side healing (spec §"Schema-required defaults").
        //
        // Pre-Phase-4 files may be missing canonical keys that every new write
        // guarantees. Heal them now — at the next round-trip — before applying
        // the caller's update. Both `apply_defaults_value` and
        // `ensure_managed_identity_keys` are no-ops for fields already present.
        //
        // Approach (a): build a serde_json::Value of the managed-meta tier,
        // run both shared actions, write any newly-injected keys back via
        // `set_managed_field`. The tier round-trip is lossless for managed keys.
        {
            let mut managed = fm.managed_json();

            // Heal doc-type defaults (e.g., temper-stage: backlog for tasks).
            apply_defaults_value(fm.doc_type().as_str(), &mut managed);

            // Heal canonical identity keys from on-disk title/slug. Use the
            // same fallback logic as `vault_file_to_resource_row` so the
            // healed value always matches what the row projection would return.
            let title = fm
                .value()
                .get(serde_yaml::Value::String("temper-title".to_string()))
                .and_then(|v| v.as_str())
                .unwrap_or_else(|| {
                    resolved
                        .path
                        .file_stem()
                        .and_then(|s| s.to_str())
                        .unwrap_or("Untitled")
                })
                .to_string();
            // Slug fallback: use the on-disk value when present; otherwise
            // derive from the filename stem. `ensure_managed_identity_keys`
            // removes temper-slug from managed when passed None — always pass
            // Some so the healed managed Value carries the key.
            let slug = fm
                .value()
                .get(serde_yaml::Value::String("temper-slug".to_string()))
                .and_then(|v| v.as_str())
                .map(str::to_string)
                .or_else(|| {
                    resolved
                        .path
                        .file_stem()
                        .and_then(|s| s.to_str())
                        .map(str::to_string)
                });
            ensure_managed_identity_keys(&mut managed, &title, slug.as_deref());

            // Write back only keys that managed_json did not previously contain
            // (i.e., keys the healing injected). Existing fields are untouched.
            // Collect first to release the immutable borrow on `fm.value()`
            // before calling the mutable `fm.set_managed_field`.
            let to_inject: Vec<(String, serde_json::Value)> = if let Some(obj) = managed.as_object()
            {
                let existing = fm.value();
                obj.iter()
                    .filter(|(key, _)| {
                        existing
                            .get(&serde_yaml::Value::String((*key).clone()))
                            .is_none()
                    })
                    .map(|(k, v)| (k.clone(), v.clone()))
                    .collect()
            } else {
                vec![]
            };
            for (key, val) in to_inject {
                fm.set_managed_field(&key, val);
            }
        }

        // 4. Determine current doctype/context from on-disk frontmatter.
        let get_str = |fm: &Frontmatter, key: &str| -> String {
            fm.value()
                .get(serde_yaml::Value::String(key.to_string()))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string()
        };
        let current_doctype = get_str(&fm, "temper-type");
        let current_context = get_str(&fm, "temper-context");

        // 5. Apply scalar + array updates and compute final path.
        let final_path = crate::vault_backend::translators::apply_updates(
            &mut fm,
            &cmd,
            &resolved.path,
            &self.vault_root,
            &self.owner,
            &self.config,
            &current_doctype,
            &current_context,
        )?;

        // 6. Refresh temper-updated timestamp.
        let now = chrono::Local::now().to_rfc3339();
        fm.set_managed_field("temper-updated", serde_json::Value::String(now));

        // 7. Optional body update.
        if let Some(body_update) = cmd.body.as_ref() {
            fm.set_body(body_update.content.clone());
        }

        // 8. Create parent dir if needed (for moves), then write.
        if let Some(parent) = final_path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| TemperError::Vault(e.to_string()))?;
        }
        fm.write_to(&final_path)?;

        // 9. If moved, remove old file.
        if final_path != resolved.path && resolved.path.exists() {
            std::fs::remove_file(&resolved.path).map_err(|e| TemperError::Vault(e.to_string()))?;
        }

        // 10. Compute rel_path for events and manifest.
        let rel_path = final_path
            .strip_prefix(&self.vault_root)
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_else(|_| final_path.to_string_lossy().to_string());

        let body_str = fm.body();

        // 11. Manifest rehash.
        {
            let mut manifest = self.manifest.lock().await;
            if let Some(entry) = manifest.entries.get_mut(&resolved.resource_id) {
                entry.path = rel_path.clone();
                entry.body_hash = temper_core::hash::compute_body_hash(body_str);
                entry.synced_at = chrono::Utc::now();
                entry.state = temper_core::types::manifest::ManifestEntryState::LocalModified;
            }
            crate::manifest_io::save_manifest(&self.config.state_dir, &manifest)?;
        }

        let mut events = vec![
            DomainEvent::VaultFileWritten {
                path: rel_path.clone(),
            },
            DomainEvent::VaultManifestUpdated {
                path: rel_path.clone(),
            },
        ];

        // 12. Push as tail action.
        let push_event = self.push_update(&cmd, resolved.resource_id).await;
        events.push(push_event);

        // 13. Project to ResourceRow.
        let row = vault_file_to_resource_row(
            &final_path,
            &fm,
            resolved.resource_id,
            &self.vault_root,
            &self.owner,
        );
        Ok(CommandOutput::with_events(row, events))
    }

    async fn delete_resource(&self, cmd: DeleteResource) -> Result<CommandOutput<()>, TemperError> {
        // Resolve under lock; release before any I/O.
        // If resolve fails (no manifest entry + no on-disk file), we have no
        // UUID to address the API with — return NotFound (mirrors the existing
        // commands/resource.rs::delete behavior which requires a resolved UUID
        // before calling the API).
        let resolved = {
            let manifest = self.manifest.lock().await;
            crate::vault_backend::translators::resolve_resource_ref(
                &self.vault_root,
                &manifest,
                &self.config,
                &cmd.resource,
            )?
        };

        let mut events = Vec::new();

        // Cloud-first: API soft-delete first when a client is available.
        // On API failure we never mutate local state.
        if let Some(client) = self.client.as_ref() {
            match client.resources().delete(*resolved.resource_id).await {
                Ok(_) => events.push(DomainEvent::RemoteSynced {
                    resource_id: resolved.resource_id,
                }),
                Err(e) => return Err(crate::actions::runtime::client_err_to_temper(e)),
            }
        }

        // Local-tail: file removal.
        // Backend assumes cmd.force is authoritative — the TTY guard and
        // [y/N] prompt are surface concerns handled by the clap layer in 4b.
        let _ = cmd.force; // acknowledged; prompt handled at surface

        if resolved.path.exists() {
            std::fs::remove_file(&resolved.path)
                .map_err(|e| TemperError::Vault(format!("remove file: {e}")))?;
            let rel_path = resolved
                .path
                .strip_prefix(&self.vault_root)
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_else(|_| resolved.path.to_string_lossy().to_string());
            events.push(DomainEvent::VaultFileRemoved { path: rel_path });
        }

        // Manifest entry removal.
        {
            let mut manifest = self.manifest.lock().await;
            if manifest.entries.remove(&resolved.resource_id).is_some() {
                crate::manifest_io::save_manifest(&self.config.state_dir, &manifest)?;
                let rel_path = resolved
                    .path
                    .strip_prefix(&self.vault_root)
                    .map(|p| p.to_string_lossy().to_string())
                    .unwrap_or_else(|_| resolved.path.to_string_lossy().to_string());
                events.push(DomainEvent::VaultManifestUpdated { path: rel_path });
            }
        }

        Ok(CommandOutput::with_events((), events))
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
mod compute_task_defaults_tests {
    use std::fs;

    use temper_core::operations::{CreateResource, Surface};
    use temper_core::types::managed_meta::ManagedMeta;

    use crate::config::Config;

    /// Build a minimal `Config` pointing at `vault_root` with one context ("temper").
    fn make_config(vault_root: &std::path::Path) -> Config {
        Config {
            vault_root: vault_root.to_path_buf(),
            state_dir: vault_root.join(".temper"),
            contexts: vec!["temper".to_string()],
            subscriptions: vec![],
            skill_output: vault_root.join("skills"),
            profile_slug: None,
        }
    }

    /// Write a minimal goal `.md` file at `<vault_root>/@me/<context>/goal/<slug>.md`.
    fn write_goal_file(vault_root: &std::path::Path, context: &str, slug: &str) {
        let dir = vault_root.join("@me").join(context).join("goal");
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join(format!("{slug}.md"));
        // GoalInfo requires: temper-title, temper-slug, temper-context,
        // temper-status (non-optional). temper-seq has a default.
        let content = format!(
            "---\ntemper-type: goal\ntemper-context: {context}\ntemper-slug: {slug}\ntemper-title: Test Goal\ntemper-status: active\ntemper-seq: 10\n---\n\nGoal body.\n"
        );
        fs::write(path, content).unwrap();
    }

    /// Build a minimal `CreateResource` cmd for a task referencing the given goal.
    fn make_task_cmd(context: &str, goal_slug: &str) -> CreateResource {
        CreateResource {
            slug: "my-task".to_string(),
            doctype: "task".to_string(),
            context: context.to_string(),
            title: "My Task".to_string(),
            body: None,
            managed_meta: ManagedMeta {
                goal: Some(goal_slug.to_string()),
                ..Default::default()
            },
            open_meta: None,
            origin_uri: None,
            chunks_packed: None,
            content_hash: None,
            origin: Surface::CliLocalVault,
        }
    }

    #[test]
    fn compute_task_defaults_returns_seq_for_task_with_existing_goal() {
        let tmp = tempfile::tempdir().unwrap();
        let config = make_config(tmp.path());
        write_goal_file(tmp.path(), "temper", "temper-maintenance");
        let cmd = make_task_cmd("temper", "temper-maintenance");

        let defaults = super::compute_task_defaults(&config, &cmd).unwrap();
        // No existing tasks for this goal → next_seq = 0 + 10 = 10.
        assert_eq!(defaults.seq, Some(10));
    }

    #[test]
    fn compute_task_defaults_errors_when_goal_missing() {
        let tmp = tempfile::tempdir().unwrap();
        let config = make_config(tmp.path());
        // No goal file written — goal is absent.
        let cmd = make_task_cmd("temper", "nonexistent-goal");

        let err = super::compute_task_defaults(&config, &cmd).unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("nonexistent-goal"),
            "error message should mention the missing goal slug; got: {msg}"
        );
    }

    #[test]
    fn compute_task_defaults_is_noop_for_non_task() {
        let tmp = tempfile::tempdir().unwrap();
        let config = make_config(tmp.path());
        let cmd = CreateResource {
            slug: "my-research".to_string(),
            doctype: "research".to_string(),
            context: "temper".to_string(),
            title: "My Research".to_string(),
            body: None,
            managed_meta: ManagedMeta::default(),
            open_meta: None,
            origin_uri: None,
            chunks_packed: None,
            content_hash: None,
            origin: Surface::CliLocalVault,
        };

        let defaults = super::compute_task_defaults(&config, &cmd).unwrap();
        assert_eq!(defaults.seq, None);
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
