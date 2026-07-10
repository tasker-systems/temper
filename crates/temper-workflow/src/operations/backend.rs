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

use crate::types::resource::{ResourceDetail, ResourceRow};
use temper_core::error::TemperError;
use temper_core::types::ids::{EdgeId, PropertyId, ResourceId};
use temper_core::types::ingest::{
    AppendBlockPayload, BlocksResponse, FinalizePayload, SegmentedBegin, SegmentedBeginResponse,
};
use temper_core::types::materialize::MaterializeAck;

use super::commands::{
    AdvanceStewardWatermark, AnnotateResource, AssertRelationship, CloseInvocation,
    CreateCognitiveMap, CreateResource, DeleteResource, FoldRelationship, ListResources,
    MaterializeOnThreshold, OpenInvocation, ReconcileCognitiveMap, RetypeRelationship,
    ReweightRelationship, SearchResources, SetFacet, ShowResource, StewardDispatchTick,
    UpdateResource,
};
use super::output::CommandOutput;
use super::surface::Surface;

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

    /// Read one resource with both metadata tiers (`managed_meta` + `open_meta`).
    /// `list_resources` keeps the lean `ResourceRow`; only the single-resource read pays
    /// for the tiers.
    async fn show_resource(
        &self,
        cmd: ShowResource,
    ) -> Result<CommandOutput<ResourceDetail>, TemperError>;

    async fn update_resource(
        &self,
        cmd: UpdateResource,
    ) -> Result<CommandOutput<ResourceRow>, TemperError>;

    async fn delete_resource(&self, cmd: DeleteResource) -> Result<CommandOutput<()>, TemperError>;

    /// Attach provenance sources to an existing resource's block without a body revise (issue #355).
    /// Records `kb_block_provenance` rows only — no re-chunk/re-embed, body_hash unchanged. Gated on
    /// `can_modify_resource` (auth before write), like every other resource mutation. Returns the
    /// updated resource row (the resource's own state is unchanged, but the row keeps the surface's
    /// response shape uniform with `update_resource`).
    async fn annotate_resource(
        &self,
        cmd: AnnotateResource,
    ) -> Result<CommandOutput<ResourceRow>, TemperError>;

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

    // ── facet writes (T1 Sequence B — facet_set vertical slice) ──
    // Upserts a typed property row (`kb_properties`) on a resource. Returns the property id.

    async fn set_facet(&self, cmd: SetFacet) -> Result<CommandOutput<PropertyId>, TemperError>;

    // ── L0 cognitive-map content reconcile (L0 delivery & lifecycle, Task 4) ──
    // Idempotent, additive-only, provenance-scoped desired-state reconcile of a cognitive map's
    // kernel slice to a pre-embedded manifest. Dispatched by `PUT /api/cognitive-maps/{id}`.

    async fn reconcile_cognitive_map(
        &self,
        cmd: ReconcileCognitiveMap,
    ) -> Result<CommandOutput<temper_core::types::reconcile::ReconcileOutcome>, TemperError>;

    // ── cognitive-map genesis (org-provisioning Chunk 4) ──
    // Create a new cognitive map (cogmap + telos charter resource) from a manifest, under the system
    // actor. Idempotent at a manifest-supplied id (re-genesis → `created: false`, no event). Dispatched
    // by `POST /api/cognitive-maps`. The HTTP/MCP surfaces gate on `is_system_admin` before dispatch.

    async fn create_cognitive_map(
        &self,
        cmd: CreateCognitiveMap,
    ) -> Result<CommandOutput<temper_core::types::reconcile::CreateCogmapOutcome>, TemperError>;

    // ── agent-invocation envelope (trace primitive) ──
    // Open mints a server-side invocation id and returns it; close terminates the envelope with a
    // disposition + opaque outcome. Both gate on the acting profile's read access to the originating
    // cogmap (auth before write). Surfaces (API/MCP/CLI) dispatch through these so the envelope write
    // path is shared.

    async fn open_invocation(
        &self,
        cmd: OpenInvocation,
    ) -> Result<CommandOutput<uuid::Uuid>, TemperError>;

    async fn close_invocation(
        &self,
        cmd: CloseInvocation,
    ) -> Result<CommandOutput<()>, TemperError>;

    // ── team-self-cognition steward ingest watermark (T4a) ──
    // Advance a cogmap's ingest cursor to a given event id. The stub the future steward calls on run
    // completion so the next `steward_ingest_delta` counts only what landed after this run. Gated on
    // cogmap-write (auth before write). Returns the watermark now held.

    async fn advance_steward_watermark(
        &self,
        cmd: AdvanceStewardWatermark,
    ) -> Result<CommandOutput<uuid::Uuid>, TemperError>;

    /// Run one deterministic steward-dispatch pass (reap → sweep → enqueue → claim) and return the
    /// claimed jobs for fan-out. See [`StewardDispatchTick`].
    async fn steward_dispatch_tick(
        &self,
        cmd: StewardDispatchTick,
    ) -> Result<CommandOutput<Vec<temper_core::types::workflow_job::ClaimedJob>>, TemperError>;

    // ── cron-driven region materialize-on-threshold (T4b) ──
    // Re-materialize a cogmap's regions when its formation delta since the last materialize clears
    // the threshold; a safe no-op below threshold. Gated on cogmap-write (auth before write). This is
    // the substrate's deterministic region-formation cadence, distinct from the steward's authored
    // acts — invoked by a cron, not by the agent.

    async fn materialize_on_threshold(
        &self,
        cmd: MaterializeOnThreshold,
    ) -> Result<CommandOutput<MaterializeAck>, TemperError>;

    // ── segmented (multi-block) ingest — streaming/resumable ingestion ──
    // The whole session: `begin_segmented_ingest` creates the resource with block 0 and records the
    // source row; `append_block` lands `seq >= 1`; `finalize_ingest` declares it complete;
    // `list_blocks` reads the landed set back (the resume/progress read). Block 0 still lands
    // through the ordinary create path internally — `begin` composes it — so the create semantics
    // are shared with every other resource.
    //
    // The three post-begin methods gate on `can_modify_resource` before touching anything: an
    // in-progress segmented ingest is caller-private, including the read. `begin` gates through the
    // create path's own home/authorship checks.

    /// Begin a segmented (multi-block) ingest: create the resource with segment 0 as its body
    /// block, record the per-resource source-provenance row (`kb_ingestion_records`), and return
    /// the landed set plus the live `body_hash`.
    ///
    /// One command per inbound call — surfaces do not compose these steps themselves. `cmd.origin`
    /// attributes the create; `seg.total_blocks_hint` and `seg.block_budget` are recorded by the
    /// caller's own bookkeeping and are deliberately not validated here (the budget is a client-side
    /// determinism aid, not a server-enforced limit).
    async fn begin_segmented_ingest(
        &self,
        cmd: CreateResource,
        seg: SegmentedBegin,
    ) -> Result<CommandOutput<SegmentedBeginResponse>, TemperError>;

    /// Append one segment to a resource whose block 0 already landed. Idempotent in the substrate
    /// on `(resource, seq, block merkle)` — re-appending an already-landed segment is a no-op that
    /// still returns the current landed set.
    ///
    /// `payload.chunks_packed` may be absent, in which case the backend chunks the segment text
    /// itself (the MCP surface has no embedder). `origin` attributes the emitted `block_created`
    /// event to the calling surface — a parameter, not a constant, because CLI, API, and MCP all
    /// reach this path.
    async fn append_block(
        &self,
        resource: ResourceId,
        payload: AppendBlockPayload,
        origin: Surface,
    ) -> Result<CommandOutput<BlocksResponse>, TemperError>;

    /// Declare a segmented ingest complete: validates the landed block count + body merkle
    /// against the caller's expectation, then fires `resource_finalized`. `origin` attributes that
    /// event to the calling surface, as on [`Self::append_block`].
    async fn finalize_ingest(
        &self,
        resource: ResourceId,
        payload: FinalizePayload,
        origin: Surface,
    ) -> Result<CommandOutput<()>, TemperError>;

    /// The currently landed segment set for a resource, plus its live `body_hash` — backs the
    /// resume/progress read `GET /api/resources/{id}/blocks`. Takes no `origin`: it emits no event,
    /// so it resolves no emitter, and a parameter nothing consumes would be a lie.
    async fn list_blocks(
        &self,
        resource: ResourceId,
    ) -> Result<CommandOutput<BlocksResponse>, TemperError>;
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
