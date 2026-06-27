//! Backend trait — the contract every operations backend implements.
//!
//! Two impls are planned:
//! - `DbBackend` in `temper-api` (Postgres persistence + chunking + embedding)
//! - `CloudBackend` in `temper-cli` (cloud-mode dispatch via `temper-client`)
//!
//! Both produce `CommandOutput<T>` per command — typed value + events emitted.
//!
//! The trait is intentionally minimal in Phase 1: each method takes a command
//! and returns a `CommandOutput<T>`. Backend-specific operations (manifest
//! refresh, sync push/pull) live on the backend's concrete type, not on
//! the shared trait.

use async_trait::async_trait;

use crate::types::resource::ResourceRow;
use temper_core::error::TemperError;
use temper_core::types::ids::EdgeId;

use super::commands::{
    AssertRelationship, CreateResource, DeleteResource, FoldRelationship, ListResources,
    ReconcileCognitiveMap, RetypeRelationship, ReweightRelationship, SearchResources, ShowResource,
    UpdateResource,
};
use super::output::CommandOutput;

/// Lightweight summary of a resource for `list` results.
#[derive(Debug, Clone)]
pub struct ResourceSummary {
    pub slug: String,
    pub doctype: String,
    pub context: String,
    pub title: String,
}

/// A search hit — a resource summary plus relevance metadata.
#[derive(Debug, Clone)]
pub struct SearchHit {
    pub summary: ResourceSummary,
    pub score: f32,
}

/// The shared contract for both DbBackend (in temper-api) and CloudBackend
/// (in temper-cli). Each command method takes a command struct, executes it
/// against the backend's persistence, and returns a `CommandOutput<T>` with
/// the typed value plus emitted events.
#[async_trait]
pub trait Backend: Send + Sync {
    async fn create_resource(
        &self,
        cmd: CreateResource,
    ) -> Result<CommandOutput<ResourceRow>, TemperError>;

    async fn show_resource(
        &self,
        cmd: ShowResource,
    ) -> Result<CommandOutput<ResourceRow>, TemperError>;

    async fn update_resource(
        &self,
        cmd: UpdateResource,
    ) -> Result<CommandOutput<ResourceRow>, TemperError>;

    async fn delete_resource(&self, cmd: DeleteResource) -> Result<CommandOutput<()>, TemperError>;

    async fn list_resources(
        &self,
        cmd: ListResources,
    ) -> Result<CommandOutput<Vec<ResourceSummary>>, TemperError>;

    async fn search_resources(
        &self,
        cmd: SearchResources,
    ) -> Result<CommandOutput<Vec<SearchHit>>, TemperError>;

    // ── relationship/edge writes (WS6 4c) ──
    // Brought under the trait so surfaces dispatch them through `select_backend` to the selected
    // substrate. Each returns the `kb_edges` id (`EdgeId`), fed back into retype/reweight/fold.
    // Post-WS6-flip there is a single substrate-backed backend, so the returned value is always a
    // real edge row id (the earlier backend-opaque correlation-id split is gone).

    async fn assert_relationship(
        &self,
        cmd: AssertRelationship,
    ) -> Result<CommandOutput<EdgeId>, TemperError>;

    async fn retype_relationship(
        &self,
        cmd: RetypeRelationship,
    ) -> Result<CommandOutput<EdgeId>, TemperError>;

    async fn reweight_relationship(
        &self,
        cmd: ReweightRelationship,
    ) -> Result<CommandOutput<EdgeId>, TemperError>;

    async fn fold_relationship(
        &self,
        cmd: FoldRelationship,
    ) -> Result<CommandOutput<EdgeId>, TemperError>;

    // ── L0 cognitive-map content reconcile (L0 delivery & lifecycle, Task 4) ──
    // Idempotent, additive-only, provenance-scoped desired-state reconcile of a cognitive map's
    // kernel slice to a pre-embedded manifest. Dispatched by `PUT /api/cognitive-maps/{id}`.

    async fn reconcile_cognitive_map(
        &self,
        cmd: ReconcileCognitiveMap,
    ) -> Result<CommandOutput<temper_core::types::reconcile::ReconcileOutcome>, TemperError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Verify the trait is object-safe (callable via `dyn Backend`).
    /// If this compiles, dispatch through trait objects works.
    #[expect(
        dead_code,
        reason = "object-safety compile guard; intentionally never invoked"
    )]
    fn assert_object_safe(_: &dyn Backend) {}

    #[test]
    fn resource_summary_can_be_constructed() {
        let s = ResourceSummary {
            slug: "x".to_string(),
            doctype: "task".to_string(),
            context: "temper".to_string(),
            title: "X".to_string(),
        };
        assert_eq!(s.slug, "x");
    }
}
