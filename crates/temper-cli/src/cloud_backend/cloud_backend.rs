//! `CloudBackend` — cloud-mode impl of [`temper_core::operations::Backend`].
//!
//! Each method translates the inbound `temper-core::operations` command
//! into a `temper-client` API call via the translators in `translators.rs`,
//! then projects the wire response back into `CommandOutput<...>`. No vault
//! file IO, no manifest IO — this backend is cloud-only.
//!
//! Show/list/search return explicit "reads stay surface-direct" errors per
//! the parent spec contract (reads are surface-direct in both modes; the
//! Backend trait is write-focused for now).

use std::sync::Arc;

use temper_client::TemperClient;
use temper_core::operations::Surface;

use super::ctx::CloudBackendCtx;
use crate::config::Config;

/// Cloud-mode backend for CLI dispatch.
///
/// Holds the per-request fields needed to translate `Backend` trait commands
/// into `temper_client` API calls.
pub struct CloudBackend {
    pub(crate) client: Arc<TemperClient>,
    pub(crate) owner: String,
    #[expect(
        dead_code,
        reason = "kept for forward-compat (per-request profile resolution); \
                  fields settled in Task 1 to mirror the backend shape"
    )]
    pub(crate) config: Arc<Config>,
    #[expect(dead_code, reason = "stored for Phase 6 telemetry/event tagging")]
    pub(crate) surface: Surface,
}

impl CloudBackend {
    pub fn new(ctx: CloudBackendCtx) -> Self {
        Self {
            client: ctx.client,
            owner: ctx.owner,
            config: ctx.config,
            surface: ctx.surface,
        }
    }
}

#[cfg(feature = "embed")]
mod embed_impl {
    use async_trait::async_trait;
    use temper_core::operations::{
        Backend, CommandOutput, CreateResource, DeleteResource, DomainEvent, ListResources,
        ResourceRef, SearchResources, ShowResource, UpdateResource,
    };
    use temper_core::operations::{ResourceSummary, SearchHit};
    use temper_core::types::resource::ResourceRow;

    use super::super::translators::{
        cmd_to_delete_args, cmd_to_ingest_payload, cmd_to_resource_update_request,
        wire_resource_to_resource_row,
    };
    use super::CloudBackend;
    use crate::error::TemperError;

    #[async_trait]
    impl Backend for CloudBackend {
        async fn create_resource(
            &self,
            cmd: CreateResource,
        ) -> Result<CommandOutput<ResourceRow>, TemperError> {
            let payload = cmd_to_ingest_payload(&cmd)?;
            let row = self
                .client
                .ingest()
                .create(&payload)
                .await
                .map_err(crate::actions::runtime::client_err_to_temper)?;
            let resource_id = row.id;
            Ok(CommandOutput {
                value: wire_resource_to_resource_row(&row),
                events: vec![DomainEvent::RemoteSynced { resource_id }],
            })
        }

        async fn update_resource(
            &self,
            cmd: UpdateResource,
        ) -> Result<CommandOutput<ResourceRow>, TemperError> {
            let (owner, ctx, doctype, slug) = extract_scoped_update_components(&cmd, &self.owner)?;
            let resolved = self
                .client
                .resources()
                .resolve_by_uri(owner, ctx, doctype, slug)
                .await
                .map_err(crate::actions::runtime::client_err_to_temper)?;
            let req = cmd_to_resource_update_request(&cmd)?;
            let updated = self
                .client
                .resources()
                .update(*resolved.id, &req)
                .await
                .map_err(crate::actions::runtime::client_err_to_temper)?;
            let resource_id = updated.id;
            Ok(CommandOutput {
                value: wire_resource_to_resource_row(&updated),
                events: vec![DomainEvent::RemoteSynced { resource_id }],
            })
        }

        async fn delete_resource(
            &self,
            cmd: DeleteResource,
        ) -> Result<CommandOutput<()>, TemperError> {
            let (owner, ctx, doctype, slug) = cmd_to_delete_args(&cmd, &self.owner)?;
            let resolved = self
                .client
                .resources()
                .resolve_by_uri(owner, ctx, doctype, slug)
                .await
                .map_err(crate::actions::runtime::client_err_to_temper)?;
            let resource_id = resolved.id;
            // Use the structured `commands::client_err` mapper (not the lossy
            // `client_err_to_temper` collapser) so a server-returned
            // `SystemAccessRequired` is preserved as
            // `TemperError::SystemAccessRequired { details }` and main.rs
            // renders the rich CLI UI (email, join-request status, request URL,
            // CLI hint). The pre-Phase-5 `delete_cloud` used `client_err` here
            // for this reason; CloudBackend now mirrors that on the delete step.
            self.client
                .resources()
                .delete(*resource_id)
                .await
                .map_err(crate::commands::client_err)?;
            Ok(CommandOutput {
                value: (),
                events: vec![DomainEvent::RemoteSynced { resource_id }],
            })
        }

        async fn show_resource(
            &self,
            _cmd: ShowResource,
        ) -> Result<CommandOutput<ResourceRow>, TemperError> {
            Err(TemperError::Project(
                "CloudBackend::show_resource not implemented — reads stay surface-direct"
                    .to_string(),
            ))
        }

        async fn list_resources(
            &self,
            _cmd: ListResources,
        ) -> Result<CommandOutput<Vec<ResourceSummary>>, TemperError> {
            Err(TemperError::Project(
                "CloudBackend::list_resources not implemented — reads stay surface-direct"
                    .to_string(),
            ))
        }

        async fn search_resources(
            &self,
            _cmd: SearchResources,
        ) -> Result<CommandOutput<Vec<SearchHit>>, TemperError> {
            Err(TemperError::Project(
                "CloudBackend::search_resources not implemented — reads stay surface-direct"
                    .to_string(),
            ))
        }
    }

    /// Helper: extract URI components from an UpdateResource cmd's ResourceRef.
    ///
    /// Mirrors `cmd_to_delete_args` shape but for UpdateResource. Lives here
    /// rather than in translators.rs because it's an inherent helper for
    /// the trait impl, not a pure translation.
    fn extract_scoped_update_components<'a>(
        cmd: &'a UpdateResource,
        fallback_owner: &'a str,
    ) -> Result<(&'a str, &'a str, &'a str, &'a str), TemperError> {
        match &cmd.resource {
            ResourceRef::Scoped {
                owner,
                context,
                doctype,
                slug,
            } => {
                let resolved_owner: &str = if owner.is_empty() {
                    fallback_owner
                } else {
                    owner.as_str()
                };
                Ok((
                    resolved_owner,
                    context.as_str(),
                    doctype.as_str(),
                    slug.as_str(),
                ))
            }
            ResourceRef::Uuid { .. } => Err(TemperError::Project(
                "cloud-mode update requires a scoped ResourceRef".to_string(),
            )),
        }
    }

    #[cfg(test)]
    mod tests {
        use std::sync::Arc;

        use temper_client::auth::MemoryTokenStore;
        use temper_client::TemperClient;
        use temper_core::operations::Surface;

        use super::super::super::ctx::CloudBackendCtx;
        use super::*;
        use crate::config::Config;

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

        fn make_test_client() -> Arc<TemperClient> {
            let store = Arc::new(MemoryTokenStore::empty());
            Arc::new(TemperClient::new("http://localhost:0", None, store))
        }

        fn make_test_backend() -> CloudBackend {
            let temp = tempfile::tempdir().unwrap();
            let ctx = CloudBackendCtx {
                client: make_test_client(),
                owner: "@me".to_string(),
                config: Arc::new(make_config(temp.path())),
                surface: Surface::CliCloud,
            };
            CloudBackend::new(ctx)
        }

        // Compile-level guard: confirm CloudBackend implements Backend.
        // Actual dispatch is exercised via tests/e2e/ at end of branch (Task 19).
        #[allow(dead_code)]
        fn assert_implements_backend<T: Backend>(_: &T) {}

        #[test]
        fn cloud_backend_constructor_preserves_ctx_fields() {
            let client = make_test_client();
            let temp = tempfile::tempdir().unwrap();
            let ctx = CloudBackendCtx {
                client: client.clone(),
                owner: "@me".to_string(),
                config: Arc::new(make_config(temp.path())),
                surface: Surface::CliCloud,
            };
            let backend = CloudBackend::new(ctx);
            assert_eq!(backend.owner, "@me");
            assert!(Arc::ptr_eq(&backend.client, &client));
        }

        #[test]
        fn cloud_backend_implements_backend_trait() {
            // Confirm CloudBackend satisfies the Backend bound at compile time.
            let backend = make_test_backend();
            assert_implements_backend(&backend);
            assert_eq!(backend.owner, "@me");
        }

        #[test]
        fn extract_scoped_update_components_uses_fallback_when_owner_empty() {
            use temper_core::operations::{ResourceRef, UpdateResource};

            let cmd = UpdateResource {
                resource: ResourceRef::scoped("", "temper", "task", "my-slug"),
                body: None,
                managed_meta: None,
                open_meta: None,
                move_to: None,
                origin: Surface::CliCloud,
            };
            let (owner, ctx, dt, slug) =
                extract_scoped_update_components(&cmd, "fallback-owner").unwrap();
            assert_eq!(owner, "fallback-owner");
            assert_eq!(ctx, "temper");
            assert_eq!(dt, "task");
            assert_eq!(slug, "my-slug");
        }

        #[test]
        fn extract_scoped_update_components_errors_on_uuid_ref() {
            use temper_core::operations::{ResourceRef, UpdateResource};
            use temper_core::types::ids::ResourceId;
            use uuid::Uuid;

            let cmd = UpdateResource {
                resource: ResourceRef::Uuid {
                    id: ResourceId(Uuid::nil()),
                },
                body: None,
                managed_meta: None,
                open_meta: None,
                move_to: None,
                origin: Surface::CliCloud,
            };
            let err = extract_scoped_update_components(&cmd, "fallback").unwrap_err();
            assert!(
                format!("{err:?}").contains("scoped ResourceRef"),
                "expected scoped-ref error, got: {err:?}"
            );
        }
    }
}

#[cfg(not(feature = "embed"))]
mod non_embed_impl {
    use async_trait::async_trait;
    use temper_core::operations::{
        Backend, CommandOutput, CreateResource, DeleteResource, ListResources, ResourceSummary,
        SearchHit, SearchResources, ShowResource, UpdateResource,
    };
    use temper_core::types::resource::ResourceRow;

    use super::CloudBackend;
    use crate::error::TemperError;

    /// Non-embed build: every Backend method errors with a clear message.
    /// Surfaces calling `build_backend()` in cloud mode under a no-embed
    /// build will hit this stub and surface the "needs embed feature" error
    /// to the user.
    #[async_trait]
    impl Backend for CloudBackend {
        async fn create_resource(
            &self,
            _cmd: CreateResource,
        ) -> Result<CommandOutput<ResourceRow>, TemperError> {
            Err(TemperError::BadRequest(
                "cloud mode requires --features embed".to_string(),
            ))
        }

        async fn update_resource(
            &self,
            _cmd: UpdateResource,
        ) -> Result<CommandOutput<ResourceRow>, TemperError> {
            Err(TemperError::BadRequest(
                "cloud mode requires --features embed".to_string(),
            ))
        }

        async fn delete_resource(
            &self,
            _cmd: DeleteResource,
        ) -> Result<CommandOutput<()>, TemperError> {
            Err(TemperError::BadRequest(
                "cloud mode requires --features embed".to_string(),
            ))
        }

        async fn show_resource(
            &self,
            _cmd: ShowResource,
        ) -> Result<CommandOutput<ResourceRow>, TemperError> {
            Err(TemperError::BadRequest(
                "cloud mode requires --features embed".to_string(),
            ))
        }

        async fn list_resources(
            &self,
            _cmd: ListResources,
        ) -> Result<CommandOutput<Vec<ResourceSummary>>, TemperError> {
            Err(TemperError::BadRequest(
                "cloud mode requires --features embed".to_string(),
            ))
        }

        async fn search_resources(
            &self,
            _cmd: SearchResources,
        ) -> Result<CommandOutput<Vec<SearchHit>>, TemperError> {
            Err(TemperError::BadRequest(
                "cloud mode requires --features embed".to_string(),
            ))
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use std::sync::Arc;
        use temper_client::auth::MemoryTokenStore;
        use temper_client::TemperClient;
        use temper_core::operations::Surface;

        use super::super::super::ctx::CloudBackendCtx;
        use super::super::CloudBackend;
        use crate::config::Config;

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

        #[tokio::test]
        async fn cloud_backend_create_errors_with_embed_message_in_no_embed_build() {
            let temp = tempfile::tempdir().unwrap();
            let store = Arc::new(MemoryTokenStore::empty());
            let client = Arc::new(TemperClient::new("http://localhost:0", None, store));
            let ctx = CloudBackendCtx {
                client,
                owner: "@me".to_string(),
                config: Arc::new(make_config(temp.path())),
                surface: Surface::CliCloud,
            };
            let backend = CloudBackend::new(ctx);
            use temper_core::operations::CreateResource;
            use temper_core::types::ManagedMeta;
            let cmd = CreateResource {
                slug: "test".to_string(),
                doctype: "task".to_string(),
                context: "temper".to_string(),
                title: "Test".to_string(),
                body: None,
                managed_meta: ManagedMeta::default(),
                open_meta: None,
                origin_uri: None,
                chunks_packed: None,
                content_hash: None,
                origin: Surface::CliCloud,
            };
            let err = backend.create_resource(cmd).await.unwrap_err();
            assert!(
                format!("{err:?}").contains("--features embed"),
                "expected embed-required error, got: {err:?}"
            );
        }
    }
}
