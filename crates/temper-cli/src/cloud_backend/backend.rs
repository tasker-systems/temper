//! `CloudBackend` — cloud-mode impl of [`temper_workflow::operations::Backend`].
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
use temper_workflow::operations::Surface;

use super::ctx::CloudBackendCtx;
use crate::config::Config;

/// Cloud-mode backend for CLI dispatch.
///
/// Holds the per-request fields needed to translate `Backend` trait commands
/// into `temper_client` API calls.
pub struct CloudBackend {
    pub(crate) client: Arc<TemperClient>,
    #[expect(
        dead_code,
        reason = "carried from CloudBackendCtx; addressing is by ResourceId now \
                  so the resolve-by-uri owner is no longer read on the write path"
    )]
    pub(crate) owner: String,
    /// Context ref for the create path. Sent verbatim as `IngestPayload.context_ref`;
    /// the server parses+resolves it (UUID or @owner/slug) at the ingest boundary.
    pub(crate) context_ref: String,
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
            context_ref: ctx.context_ref,
            config: ctx.config,
            surface: ctx.surface,
        }
    }
}

#[cfg(feature = "embed")]
mod embed_impl {
    use async_trait::async_trait;
    use temper_workflow::operations::{
        AdvanceStewardWatermark, AssertRelationship, Backend, CloseInvocation, CommandOutput,
        CreateCognitiveMap, CreateResource, DeleteResource, DomainEvent, FoldRelationship,
        ListResources, MaterializeOnThreshold, OpenInvocation, ReconcileCognitiveMap,
        RetypeRelationship, ReweightRelationship, SearchResources, ShowResource,
        StewardDispatchTick, UpdateResource,
    };
    use temper_workflow::operations::{ResourceSummary, SearchHit};
    use temper_workflow::types::resource::ResourceRow;

    use super::super::translators::{
        cmd_to_ingest_payload, cmd_to_resource_update_request, wire_resource_to_resource_row,
    };
    use super::CloudBackend;
    use crate::error::TemperError;

    #[async_trait]
    impl Backend for CloudBackend {
        async fn create_resource(
            &self,
            cmd: CreateResource,
        ) -> Result<CommandOutput<ResourceRow>, TemperError> {
            let payload = cmd_to_ingest_payload(&cmd, &self.context_ref)?;
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
            let req = cmd_to_resource_update_request(&cmd)?;
            // The resource is addressed by id — dispatch straight to the by-id PATCH.
            let id = uuid::Uuid::from(cmd.resource);
            let updated = self
                .client
                .resources()
                .update(id, &req)
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
            // The resource is addressed by id — dispatch straight to the by-id delete.
            //
            // The delete call uses the structured `commands::client_err` mapper
            // (not the lossy `client_err_to_temper` collapser) so a
            // server-returned `SystemAccessRequired` is preserved as
            // `TemperError::SystemAccessRequired { details }` and main.rs
            // renders the rich CLI UI (email, join-request status, request URL,
            // CLI hint). The pre-Phase-5 `delete_cloud` used `client_err` here
            // for this reason; CloudBackend now mirrors that on the delete step.
            let id = uuid::Uuid::from(cmd.resource);
            // Carry the per-act correlation + authorship from the command onto the wire (discrete
            // ActInput shape, query params); the delete handler reassembles it into the act.
            let act: temper_core::types::ActInput = cmd.act.clone().into();
            self.client
                .resources()
                .delete(id, &act)
                .await
                .map_err(crate::commands::client_err)?;
            let resource_id = cmd.resource;
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

        // Relationship writes are on the trait (WS6 4c) but the CLI still routes them through
        // temper-client directly; these stubs satisfy the trait until the cutover wires them.
        async fn assert_relationship(
            &self,
            _cmd: AssertRelationship,
        ) -> Result<CommandOutput<temper_core::types::ids::EdgeId>, TemperError> {
            Err(TemperError::Project(
                "CloudBackend::assert_relationship not wired until cutover".to_string(),
            ))
        }

        async fn retype_relationship(
            &self,
            _cmd: RetypeRelationship,
        ) -> Result<CommandOutput<temper_core::types::ids::EdgeId>, TemperError> {
            Err(TemperError::Project(
                "CloudBackend::retype_relationship not wired until cutover".to_string(),
            ))
        }

        async fn reweight_relationship(
            &self,
            _cmd: ReweightRelationship,
        ) -> Result<CommandOutput<temper_core::types::ids::EdgeId>, TemperError> {
            Err(TemperError::Project(
                "CloudBackend::reweight_relationship not wired until cutover".to_string(),
            ))
        }

        async fn fold_relationship(
            &self,
            _cmd: FoldRelationship,
        ) -> Result<CommandOutput<temper_core::types::ids::EdgeId>, TemperError> {
            Err(TemperError::Project(
                "CloudBackend::fold_relationship not wired until cutover".to_string(),
            ))
        }

        async fn set_facet(
            &self,
            _cmd: temper_workflow::operations::SetFacet,
        ) -> Result<CommandOutput<temper_core::types::ids::PropertyId>, TemperError> {
            Err(TemperError::Project(
                "CloudBackend::set_facet not wired until cutover".to_string(),
            ))
        }

        // L0 reconcile is an admin/operator path that PUTs directly via the client (Task 7); the
        // CLI does not dispatch it through CloudBackend.
        async fn reconcile_cognitive_map(
            &self,
            _cmd: ReconcileCognitiveMap,
        ) -> Result<CommandOutput<temper_core::types::reconcile::ReconcileOutcome>, TemperError>
        {
            Err(TemperError::Project(
                "CloudBackend::reconcile_cognitive_map not dispatched through the backend"
                    .to_string(),
            ))
        }

        // Cognitive-map genesis is an admin/operator path that POSTs directly via the client; the CLI
        // does not dispatch it through CloudBackend (mirrors reconcile).
        async fn create_cognitive_map(
            &self,
            _cmd: CreateCognitiveMap,
        ) -> Result<CommandOutput<temper_core::types::reconcile::CreateCogmapOutcome>, TemperError>
        {
            Err(TemperError::Project(
                "CloudBackend::create_cognitive_map not dispatched through the backend".to_string(),
            ))
        }

        // Invocation-envelope writes are on the trait but the CLI does not yet dispatch them through
        // CloudBackend; these stubs satisfy the trait until the cutover wires them.
        async fn open_invocation(
            &self,
            _cmd: OpenInvocation,
        ) -> Result<CommandOutput<uuid::Uuid>, TemperError> {
            Err(TemperError::Project(
                "CloudBackend::open_invocation not wired until cutover".to_string(),
            ))
        }

        async fn close_invocation(
            &self,
            _cmd: CloseInvocation,
        ) -> Result<CommandOutput<()>, TemperError> {
            Err(TemperError::Project(
                "CloudBackend::close_invocation not wired until cutover".to_string(),
            ))
        }

        async fn advance_steward_watermark(
            &self,
            _cmd: AdvanceStewardWatermark,
        ) -> Result<CommandOutput<uuid::Uuid>, TemperError> {
            Err(TemperError::Project(
                "CloudBackend::advance_steward_watermark not wired until cutover".to_string(),
            ))
        }

        async fn steward_dispatch_tick(
            &self,
            _cmd: StewardDispatchTick,
        ) -> Result<CommandOutput<Vec<temper_core::types::workflow_job::ClaimedJob>>, TemperError>
        {
            Err(TemperError::Project(
                "CloudBackend::steward_dispatch_tick not wired until cutover".to_string(),
            ))
        }

        async fn materialize_on_threshold(
            &self,
            _cmd: MaterializeOnThreshold,
        ) -> Result<CommandOutput<temper_core::types::materialize::MaterializeAck>, TemperError>
        {
            Err(TemperError::Project(
                "CloudBackend::materialize_on_threshold not wired until cutover".to_string(),
            ))
        }

        // Segmented (multi-block) ingest is on the trait (Beat 2) but the CLI's streaming
        // begin/append/finalize orchestration is Beat 3 — these stubs satisfy the trait until
        // that cutover wires them.
        async fn append_block(
            &self,
            _resource: temper_core::types::ids::ResourceId,
            _payload: temper_core::types::ingest::AppendBlockPayload,
        ) -> Result<CommandOutput<temper_core::types::ingest::BlocksResponse>, TemperError>
        {
            Err(TemperError::Project(
                "CloudBackend::append_block not wired until cutover".to_string(),
            ))
        }

        async fn finalize_ingest(
            &self,
            _resource: temper_core::types::ids::ResourceId,
            _payload: temper_core::types::ingest::FinalizePayload,
        ) -> Result<CommandOutput<()>, TemperError> {
            Err(TemperError::Project(
                "CloudBackend::finalize_ingest not wired until cutover".to_string(),
            ))
        }

        async fn list_blocks(
            &self,
            _resource: temper_core::types::ids::ResourceId,
        ) -> Result<CommandOutput<temper_core::types::ingest::BlocksResponse>, TemperError>
        {
            Err(TemperError::Project(
                "CloudBackend::list_blocks not wired until cutover".to_string(),
            ))
        }
    }

    #[cfg(test)]
    mod tests {
        use std::sync::Arc;

        use temper_client::auth::MemoryTokenStore;
        use temper_client::TemperClient;
        use temper_workflow::operations::Surface;

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
                context_ref: "@me/temper".to_string(),
                config: Arc::new(make_config(temp.path())),
                surface: Surface::CliCloud,
            };
            CloudBackend::new(ctx)
        }

        // Compile-level guard: confirm CloudBackend implements Backend.
        // Actual dispatch is exercised via tests/e2e/ at end of branch (Task 19).
        fn assert_implements_backend<T: Backend>(_: &T) {}

        #[test]
        fn cloud_backend_constructor_preserves_ctx_fields() {
            let client = make_test_client();
            let temp = tempfile::tempdir().unwrap();
            let ctx = CloudBackendCtx {
                client: client.clone(),
                owner: "@me".to_string(),
                context_ref: "@me/temper".to_string(),
                config: Arc::new(make_config(temp.path())),
                surface: Surface::CliCloud,
            };
            let backend = CloudBackend::new(ctx);
            assert!(Arc::ptr_eq(&backend.client, &client));
        }

        #[test]
        fn cloud_backend_implements_backend_trait() {
            // Confirm CloudBackend satisfies the Backend bound at compile time.
            let backend = make_test_backend();
            assert_implements_backend(&backend);
        }
    }
}

#[cfg(not(feature = "embed"))]
mod non_embed_impl {
    use async_trait::async_trait;
    use temper_workflow::operations::{
        AdvanceStewardWatermark, AssertRelationship, Backend, CloseInvocation, CommandOutput,
        CreateCognitiveMap, CreateResource, DeleteResource, FoldRelationship, ListResources,
        MaterializeOnThreshold, OpenInvocation, ReconcileCognitiveMap, ResourceSummary,
        RetypeRelationship, ReweightRelationship, SearchHit, SearchResources, ShowResource,
        StewardDispatchTick, UpdateResource,
    };
    use temper_workflow::types::resource::ResourceRow;

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

        async fn assert_relationship(
            &self,
            _cmd: AssertRelationship,
        ) -> Result<CommandOutput<temper_core::types::ids::EdgeId>, TemperError> {
            Err(TemperError::BadRequest(
                "cloud mode requires --features embed".to_string(),
            ))
        }

        async fn retype_relationship(
            &self,
            _cmd: RetypeRelationship,
        ) -> Result<CommandOutput<temper_core::types::ids::EdgeId>, TemperError> {
            Err(TemperError::BadRequest(
                "cloud mode requires --features embed".to_string(),
            ))
        }

        async fn reweight_relationship(
            &self,
            _cmd: ReweightRelationship,
        ) -> Result<CommandOutput<temper_core::types::ids::EdgeId>, TemperError> {
            Err(TemperError::BadRequest(
                "cloud mode requires --features embed".to_string(),
            ))
        }

        async fn fold_relationship(
            &self,
            _cmd: FoldRelationship,
        ) -> Result<CommandOutput<temper_core::types::ids::EdgeId>, TemperError> {
            Err(TemperError::BadRequest(
                "cloud mode requires --features embed".to_string(),
            ))
        }

        async fn set_facet(
            &self,
            _cmd: temper_workflow::operations::SetFacet,
        ) -> Result<CommandOutput<temper_core::types::ids::PropertyId>, TemperError> {
            Err(TemperError::BadRequest(
                "cloud mode requires --features embed".to_string(),
            ))
        }

        async fn reconcile_cognitive_map(
            &self,
            _cmd: ReconcileCognitiveMap,
        ) -> Result<CommandOutput<temper_core::types::reconcile::ReconcileOutcome>, TemperError>
        {
            Err(TemperError::BadRequest(
                "cloud mode requires --features embed".to_string(),
            ))
        }

        async fn create_cognitive_map(
            &self,
            _cmd: CreateCognitiveMap,
        ) -> Result<CommandOutput<temper_core::types::reconcile::CreateCogmapOutcome>, TemperError>
        {
            Err(TemperError::BadRequest(
                "cloud mode requires --features embed".to_string(),
            ))
        }

        async fn open_invocation(
            &self,
            _cmd: OpenInvocation,
        ) -> Result<CommandOutput<uuid::Uuid>, TemperError> {
            Err(TemperError::BadRequest(
                "cloud mode requires --features embed".to_string(),
            ))
        }

        async fn close_invocation(
            &self,
            _cmd: CloseInvocation,
        ) -> Result<CommandOutput<()>, TemperError> {
            Err(TemperError::BadRequest(
                "cloud mode requires --features embed".to_string(),
            ))
        }

        async fn advance_steward_watermark(
            &self,
            _cmd: AdvanceStewardWatermark,
        ) -> Result<CommandOutput<uuid::Uuid>, TemperError> {
            Err(TemperError::BadRequest(
                "cloud mode requires --features embed".to_string(),
            ))
        }

        async fn steward_dispatch_tick(
            &self,
            _cmd: StewardDispatchTick,
        ) -> Result<CommandOutput<Vec<temper_core::types::workflow_job::ClaimedJob>>, TemperError>
        {
            Err(TemperError::BadRequest(
                "cloud mode requires --features embed".to_string(),
            ))
        }

        async fn materialize_on_threshold(
            &self,
            _cmd: MaterializeOnThreshold,
        ) -> Result<CommandOutput<temper_core::types::materialize::MaterializeAck>, TemperError>
        {
            Err(TemperError::BadRequest(
                "cloud mode requires --features embed".to_string(),
            ))
        }

        async fn append_block(
            &self,
            _resource: temper_core::types::ids::ResourceId,
            _payload: temper_core::types::ingest::AppendBlockPayload,
        ) -> Result<CommandOutput<temper_core::types::ingest::BlocksResponse>, TemperError>
        {
            Err(TemperError::BadRequest(
                "cloud mode requires --features embed".to_string(),
            ))
        }

        async fn finalize_ingest(
            &self,
            _resource: temper_core::types::ids::ResourceId,
            _payload: temper_core::types::ingest::FinalizePayload,
        ) -> Result<CommandOutput<()>, TemperError> {
            Err(TemperError::BadRequest(
                "cloud mode requires --features embed".to_string(),
            ))
        }

        async fn list_blocks(
            &self,
            _resource: temper_core::types::ids::ResourceId,
        ) -> Result<CommandOutput<temper_core::types::ingest::BlocksResponse>, TemperError>
        {
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
        use temper_workflow::operations::Surface;

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
            use temper_workflow::operations::CreateResource;
            use temper_workflow::types::ManagedMeta;
            let cmd = CreateResource {
                slug: "test".to_string(),
                doctype: "task".to_string(),
                home: temper_core::types::home::HomeAnchor::Context(
                    temper_core::types::ids::ContextId::new(),
                ),
                title: "Test".to_string(),
                body: None,
                managed_meta: ManagedMeta::default(),
                open_meta: None,
                origin_uri: None,
                chunks_packed: None,
                content_hash: None,
                goal: None,
                act: Default::default(),
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
