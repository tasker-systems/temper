//! `DbBackend` struct + `impl Backend`. Per-request construction.

use async_trait::async_trait;
use chrono::Utc;
use sqlx::PgPool;
use uuid::Uuid;

use temper_core::error::TemperError;
use temper_core::operations::{
    AssertRelationship, Backend, CommandOutput, CreateResource, DeleteResource, DomainEvent,
    FoldRelationship, ListResources, ResourceRef, ResourceSummary, RetypeRelationship,
    ReweightRelationship, SearchHit, SearchResources, ShowResource, Surface, UpdateResource,
};
use temper_core::types::ids::ProfileId;
use temper_core::types::relationship_events::{
    RelationshipAsserted, RelationshipFolded, RelationshipRetyped, RelationshipReweighted,
    TargetEndpoint,
};
use temper_core::types::resource::ResourceRow;
use temper_events::types::event::{EventToWrite, EventType};

use crate::error::ApiError;
use crate::services::{ingest_service, relationship_service, resource_service};

use super::translators::create_resource_to_ingest_payload;

/// Convert a `sqlx::Error` to `TemperError` via the `ApiError` bridge.
fn sqlx_err(e: sqlx::Error) -> TemperError {
    TemperError::from(ApiError::from(e))
}

/// Convert a `serde_json::Error` to `TemperError` via the `ApiError` bridge.
fn json_err(e: serde_json::Error) -> TemperError {
    TemperError::from(ApiError::from(e))
}

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

    /// Parse the fixed scope UUID used by all relationship events.
    fn public_scope_id() -> Uuid {
        Uuid::parse_str("019e3d6f-2300-7000-8000-000000000010")
            .expect("PUBLIC_SCOPE_ID constant is valid UUID")
    }

    /// Stamp `intent = "explicit"` on an event write — all API/CLI/MCP
    /// user-driven relationship writes use this provenance marker.
    fn explicit_intent_metadata() -> serde_json::Value {
        serde_json::json!({ "intent": "explicit" })
    }

    /// Assert a new relationship from `cmd.source` to `cmd.target_slug`.
    ///
    /// If the target slug resolves to a resource in the same context as the
    /// source, an edge row is projected immediately. If not, the event is
    /// still appended (with a `Slug` target endpoint); the edge will be
    /// projected by Task 13's re-projection pass once the target is created.
    ///
    /// **Re-assert semantics:**
    /// - Active edge + re-assert → diverts to a `reweight` under the existing
    ///   correlation chain. Returns `DbRelationshipReweighted`.
    /// - Folded edge + re-assert → fresh assertion (ON CONFLICT transfers
    ///   ownership of `asserted_by_event_id` to the new chain).
    /// - Slug-target re-assert (target not yet resolved) → fresh assert as
    ///   normal; Task 13 handles the slug-resolves-later case.
    pub async fn assert_relationship(
        &self,
        cmd: AssertRelationship,
    ) -> Result<CommandOutput<Uuid>, TemperError> {
        // 1. Resolve source.
        let source_resource_id =
            *super::translators::resolve_resource_ref(self.pool(), self.profile_id(), cmd.source)
                .await?;

        // 2. Auth before write.
        resource_service::check_can_modify(self.pool(), *self.profile_id(), source_resource_id)
            .await
            .map_err(TemperError::from)?;

        // 3. Validate label.
        relationship_service::validate_assertion_label(cmd.edge_kind, &cmd.label)
            .map_err(TemperError::BadRequest)?;

        // 4. Pre-tx: resolve source context + target slug (read-only).
        //    This lookup is repeated inside the write tx below for consistency.
        //    We do it here so we can check for an existing active edge before
        //    opening a write transaction.
        let source_context_id: Uuid = sqlx::query_scalar!(
            r#"SELECT kb_context_id AS "kb_context_id!: Uuid" FROM kb_resources WHERE id = $1"#,
            source_resource_id,
        )
        .fetch_one(self.pool())
        .await
        .map_err(sqlx_err)?;

        let pre_target_id = relationship_service::find_slug_in_context_pool(
            self.pool(),
            source_context_id,
            &cmd.target_slug,
        )
        .await
        .map_err(TemperError::from)?;

        // 5. If target resolved AND an active (non-folded) edge row exists,
        //    divert to a reweight under the existing correlation chain.
        if let Some(target_id) = pre_target_id {
            let existing = relationship_service::find_active_edge(
                self.pool(),
                source_resource_id,
                target_id,
                cmd.edge_kind,
                &cmd.label,
                cmd.polarity,
            )
            .await
            .map_err(TemperError::from)?;

            if let Some(active_edge) = existing {
                if !active_edge.is_folded {
                    // Divert: reweight under the existing correlation chain.
                    let reweight_payload =
                        serde_json::to_value(&RelationshipReweighted { weight: cmd.weight })
                            .map_err(json_err)?;

                    let declaration_topic_id =
                        Uuid::parse_str(relationship_service::TOPIC_DECLARATION)
                            .expect("TOPIC_DECLARATION constant is valid UUID");
                    let mut reweight_write = EventToWrite::new_correlated(
                        EventType::RelationshipReweighted,
                        *self.profile_id(),
                        declaration_topic_id,
                        Self::public_scope_id(),
                        reweight_payload,
                        active_edge.correlation_id,
                        Utc::now(),
                    );
                    reweight_write.metadata = Self::explicit_intent_metadata();

                    let mut tx = self.pool().begin().await.map_err(sqlx_err)?;
                    relationship_service::append_and_project(
                        &mut tx,
                        reweight_write,
                        EventType::RelationshipReweighted,
                    )
                    .await
                    .map_err(TemperError::from)?;
                    tx.commit().await.map_err(sqlx_err)?;

                    let correlation_id = active_edge.correlation_id;
                    return Ok(CommandOutput::with_events(
                        correlation_id,
                        vec![DomainEvent::DbRelationshipReweighted { correlation_id }],
                    ));
                }
                // Folded edge: fall through to a fresh assertion below.
                // ON CONFLICT will revive the row with new ownership.
            }
        }

        // 6. Open write transaction; re-resolve slug within tx for consistency.
        let mut tx = self.pool().begin().await.map_err(sqlx_err)?;

        let target_endpoint = match relationship_service::find_slug_in_context(
            &mut tx,
            source_context_id,
            &cmd.target_slug,
        )
        .await
        .map_err(TemperError::from)?
        {
            Some(target_id) => TargetEndpoint::Resource(target_id),
            None => TargetEndpoint::Slug(cmd.target_slug),
        };

        // 7. Build payload.
        let payload = serde_json::to_value(&RelationshipAsserted {
            source_resource_id,
            target: target_endpoint,
            edge_kind: cmd.edge_kind,
            polarity: cmd.polarity,
            label: cmd.label,
            weight: cmd.weight,
        })
        .map_err(json_err)?;

        // 8. Build event write (root — starts a new correlation chain).
        let declaration_topic_id = Uuid::parse_str(relationship_service::TOPIC_DECLARATION)
            .expect("TOPIC_DECLARATION constant is valid UUID");
        let mut write = EventToWrite::new_root(
            EventType::RelationshipAsserted,
            *self.profile_id(),
            declaration_topic_id,
            Self::public_scope_id(),
            payload,
            Utc::now(),
        );
        write.metadata = Self::explicit_intent_metadata();

        // 9–10. Append + project + commit.
        let event = relationship_service::append_and_project(
            &mut tx,
            write,
            EventType::RelationshipAsserted,
        )
        .await
        .map_err(TemperError::from)?;

        tx.commit().await.map_err(sqlx_err)?;

        // 11. Return correlation_id (== event.id for root events).
        let correlation_id = event.correlation_id;
        Ok(CommandOutput::with_events(
            correlation_id,
            vec![DomainEvent::DbRelationshipAsserted { correlation_id }],
        ))
    }

    /// Retype an existing relationship — changes `edge_kind` / `polarity`.
    /// Identified by the original assertion's `correlation_id`.
    pub async fn retype_relationship(
        &self,
        cmd: RetypeRelationship,
    ) -> Result<CommandOutput<Uuid>, TemperError> {
        // 1. Find source resource for auth check.
        let edge = relationship_service::edge_auth_row(self.pool(), cmd.correlation_id)
            .await
            .map_err(TemperError::from)?;

        // 2. Auth before write.
        resource_service::check_can_modify(
            self.pool(),
            *self.profile_id(),
            edge.source_resource_id,
        )
        .await
        .map_err(TemperError::from)?;

        // 3–4. Build payload (kind + polarity only; label is unchanged on retype).
        let payload = serde_json::to_value(&RelationshipRetyped {
            edge_kind: cmd.edge_kind,
            polarity: cmd.polarity,
        })
        .map_err(json_err)?;

        // 5. Build correlated event write.
        let declaration_topic_id = Uuid::parse_str(relationship_service::TOPIC_DECLARATION)
            .expect("TOPIC_DECLARATION constant is valid UUID");
        let mut write = EventToWrite::new_correlated(
            EventType::RelationshipRetyped,
            *self.profile_id(),
            declaration_topic_id,
            Self::public_scope_id(),
            payload,
            cmd.correlation_id,
            Utc::now(),
        );
        write.metadata = Self::explicit_intent_metadata();

        // 6. Append + project + commit.
        let mut tx = self.pool().begin().await.map_err(sqlx_err)?;
        relationship_service::append_and_project(&mut tx, write, EventType::RelationshipRetyped)
            .await
            .map_err(TemperError::from)?;
        tx.commit().await.map_err(sqlx_err)?;

        // 7. Return.
        Ok(CommandOutput::with_events(
            cmd.correlation_id,
            vec![DomainEvent::DbRelationshipRetyped {
                correlation_id: cmd.correlation_id,
            }],
        ))
    }

    /// Reweight an existing relationship — changes `weight`.
    /// Identified by the original assertion's `correlation_id`.
    pub async fn reweight_relationship(
        &self,
        cmd: ReweightRelationship,
    ) -> Result<CommandOutput<Uuid>, TemperError> {
        // 1. Find source resource for auth check.
        let edge = relationship_service::edge_auth_row(self.pool(), cmd.correlation_id)
            .await
            .map_err(TemperError::from)?;

        // 2. Auth before write.
        resource_service::check_can_modify(
            self.pool(),
            *self.profile_id(),
            edge.source_resource_id,
        )
        .await
        .map_err(TemperError::from)?;

        // 3–4. Build payload.
        let payload = serde_json::to_value(&RelationshipReweighted { weight: cmd.weight })
            .map_err(json_err)?;

        // 5. Build correlated event write.
        let declaration_topic_id = Uuid::parse_str(relationship_service::TOPIC_DECLARATION)
            .expect("TOPIC_DECLARATION constant is valid UUID");
        let mut write = EventToWrite::new_correlated(
            EventType::RelationshipReweighted,
            *self.profile_id(),
            declaration_topic_id,
            Self::public_scope_id(),
            payload,
            cmd.correlation_id,
            Utc::now(),
        );
        write.metadata = Self::explicit_intent_metadata();

        // 6. Append + project + commit.
        let mut tx = self.pool().begin().await.map_err(sqlx_err)?;
        relationship_service::append_and_project(&mut tx, write, EventType::RelationshipReweighted)
            .await
            .map_err(TemperError::from)?;
        tx.commit().await.map_err(sqlx_err)?;

        // 7. Return.
        Ok(CommandOutput::with_events(
            cmd.correlation_id,
            vec![DomainEvent::DbRelationshipReweighted {
                correlation_id: cmd.correlation_id,
            }],
        ))
    }

    /// Fold (retract) an existing relationship — sets `is_folded = true`.
    /// Identified by the original assertion's `correlation_id`.
    pub async fn fold_relationship(
        &self,
        cmd: FoldRelationship,
    ) -> Result<CommandOutput<Uuid>, TemperError> {
        // 1. Find source resource for auth check.
        let edge = relationship_service::edge_auth_row(self.pool(), cmd.correlation_id)
            .await
            .map_err(TemperError::from)?;

        // 2. Auth before write.
        resource_service::check_can_modify(
            self.pool(),
            *self.profile_id(),
            edge.source_resource_id,
        )
        .await
        .map_err(TemperError::from)?;

        // 3–4. Build payload.
        let payload = serde_json::to_value(&RelationshipFolded {
            reason: cmd.reason.clone(),
        })
        .map_err(json_err)?;

        // 5. Build correlated event write (deformation topic for fold).
        let deformation_topic_id = Uuid::parse_str(relationship_service::TOPIC_DEFORMATION)
            .expect("TOPIC_DEFORMATION constant is valid UUID");
        let mut write = EventToWrite::new_correlated(
            EventType::RelationshipFolded,
            *self.profile_id(),
            deformation_topic_id,
            Self::public_scope_id(),
            payload,
            cmd.correlation_id,
            Utc::now(),
        );
        write.metadata = Self::explicit_intent_metadata();

        // 6. Append + project + commit.
        let mut tx = self.pool().begin().await.map_err(sqlx_err)?;
        relationship_service::append_and_project(&mut tx, write, EventType::RelationshipFolded)
            .await
            .map_err(TemperError::from)?;
        tx.commit().await.map_err(sqlx_err)?;

        // 7. Return.
        Ok(CommandOutput::with_events(
            cmd.correlation_id,
            vec![DomainEvent::DbRelationshipFolded {
                correlation_id: cmd.correlation_id,
            }],
        ))
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
                owner,
                context,
                doctype,
                slug,
            } => {
                let params = resource_service::ResolveByUriParams {
                    owner,
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
        cmd: UpdateResource,
    ) -> Result<CommandOutput<ResourceRow>, TemperError> {
        let resource_id = super::translators::resolve_resource_ref(
            self.pool(),
            self.profile_id(),
            cmd.resource.clone(),
        )
        .await?;
        let req = super::translators::update_resource_to_request(cmd)?;
        let row = resource_service::update(
            self.pool(),
            *self.profile_id(),
            *resource_id,
            self.device_id(),
            req,
        )
        .await
        .map_err(TemperError::from)?;
        let event = DomainEvent::DbResourceUpdated {
            resource_id: row.id,
        };
        Ok(CommandOutput::with_events(row, vec![event]))
    }

    async fn delete_resource(&self, cmd: DeleteResource) -> Result<CommandOutput<()>, TemperError> {
        let resource_id =
            super::translators::resolve_resource_ref(self.pool(), self.profile_id(), cmd.resource)
                .await?;
        resource_service::delete(
            self.pool(),
            self.profile_id(),
            resource_id,
            self.device_id(),
        )
        .await
        .map_err(TemperError::from)?;
        let event = DomainEvent::DbResourceSoftDeleted { resource_id };
        Ok(CommandOutput::with_events((), vec![event]))
    }

    async fn list_resources(
        &self,
        cmd: ListResources,
    ) -> Result<CommandOutput<Vec<ResourceSummary>>, TemperError> {
        let params = super::translators::list_filter_to_params(cmd.filter);
        let response = resource_service::list_visible(self.pool(), *self.profile_id(), params)
            .await
            .map_err(TemperError::from)?;
        let summaries: Vec<ResourceSummary> = response
            .rows
            .iter()
            .map(super::translators::resource_row_to_summary)
            .collect();
        Ok(CommandOutput::new(summaries))
    }

    async fn search_resources(
        &self,
        cmd: SearchResources,
    ) -> Result<CommandOutput<Vec<SearchHit>>, TemperError> {
        let params = super::translators::search_query_to_params(cmd.query);
        let rows = crate::services::search_service::search(self.pool(), *self.profile_id(), params)
            .await
            .map_err(TemperError::from)?;
        let hits: Vec<SearchHit> = rows
            .iter()
            .map(super::translators::unified_hit_to_search_hit)
            .collect();
        Ok(CommandOutput::new(hits))
    }
}
