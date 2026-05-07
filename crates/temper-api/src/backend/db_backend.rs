//! `DbBackend` struct + `impl Backend`. Per-request construction.

use async_trait::async_trait;
use sqlx::PgPool;

use temper_core::error::TemperError;
use temper_core::operations::{
    Backend, CommandOutput, CreateResource, DeleteResource, DomainEvent, ListResources,
    ResourceRef, ResourceSummary, SearchHit, SearchResources, ShowResource, Surface,
    UpdateResource,
};
use temper_core::types::ids::ProfileId;
use temper_core::types::resource::ResourceRow;

use crate::services::{ingest_service, resource_service};

use super::translators::create_resource_to_ingest_payload;

/// Postgres-backed backend impl. Constructed per inbound request.
pub struct DbBackend {
    pool: PgPool,
    profile_id: ProfileId,
    device_id: String,
    /// Origin of the inbound command. Stored for forward-compat (Phase 6
    /// telemetry/event tagging); not used by Phase 3a's coarse events.
    #[allow(dead_code)]
    surface: Surface,
}

impl DbBackend {
    pub fn new(pool: PgPool, profile_id: ProfileId, device_id: String, surface: Surface) -> Self {
        Self {
            pool,
            profile_id,
            device_id,
            surface,
        }
    }

    pub(crate) fn pool(&self) -> &PgPool {
        &self.pool
    }

    pub(crate) fn profile_id(&self) -> ProfileId {
        self.profile_id
    }

    pub(crate) fn device_id(&self) -> &str {
        &self.device_id
    }
}

#[async_trait]
impl Backend for DbBackend {
    async fn create_resource(
        &self,
        cmd: CreateResource,
    ) -> Result<CommandOutput<ResourceRow>, TemperError> {
        let payload = create_resource_to_ingest_payload(cmd);
        let row = ingest_service::ingest(self.pool(), self.profile_id(), self.device_id(), payload)
            .await
            .map_err(TemperError::from)?;
        let event = DomainEvent::DbResourceCreated {
            resource_id: row.id,
        };
        Ok(CommandOutput::with_events(row, vec![event]))
    }

    async fn show_resource(
        &self,
        cmd: ShowResource,
    ) -> Result<CommandOutput<ResourceRow>, TemperError> {
        let row = match cmd.resource {
            ResourceRef::Uuid { id } => {
                resource_service::get_visible(self.pool(), *self.profile_id(), *id)
                    .await
                    .map_err(TemperError::from)?
            }
            ResourceRef::Scoped {
                slug,
                doctype,
                context,
            } => {
                let params = resource_service::ResolveByUriParams {
                    owner: "@me".to_string(),
                    context,
                    doc_type: doctype,
                    ident: slug,
                };
                resource_service::resolve_by_uri(self.pool(), *self.profile_id(), &params)
                    .await
                    .map_err(TemperError::from)?
            }
        };
        Ok(CommandOutput::new(row))
    }

    async fn update_resource(
        &self,
        _cmd: UpdateResource,
    ) -> Result<CommandOutput<ResourceRow>, TemperError> {
        Err(TemperError::Api(
            "update_resource not yet implemented".to_string(),
        ))
    }

    async fn delete_resource(
        &self,
        _cmd: DeleteResource,
    ) -> Result<CommandOutput<()>, TemperError> {
        Err(TemperError::Api(
            "delete_resource not yet implemented".to_string(),
        ))
    }

    async fn list_resources(
        &self,
        _cmd: ListResources,
    ) -> Result<CommandOutput<Vec<ResourceSummary>>, TemperError> {
        Err(TemperError::Api(
            "list_resources not yet implemented".to_string(),
        ))
    }

    async fn search_resources(
        &self,
        _cmd: SearchResources,
    ) -> Result<CommandOutput<Vec<SearchHit>>, TemperError> {
        Err(TemperError::Api(
            "search_resources not yet implemented".to_string(),
        ))
    }
}
