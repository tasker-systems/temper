//! Command structs — declarative intent for a single operation.
//!
//! Commands are pure data. Surfaces translate user input (clap args, MCP
//! tool params, HTTP body) into a command; backends consume commands and
//! emit `CommandOutput`.
//!
//! Resource-action commands (`Show`, `Update`, `Delete`) carry a `ResourceId`
//! (globally unique). `Create` carries the new resource's identity
//! directly. `List` and `Search` carry filter/query inputs.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::types::managed_meta::ManagedMeta;
use temper_core::types::authorship::ActContext;
use temper_core::types::home::HomeAnchor;
use temper_core::types::ids::{CogmapId, ContextId, EdgeId, ResourceId};

use super::{
    inputs::{BodyUpdate, ListFilter, SearchQuery},
    surface::Surface,
};

/// Create a new resource. The resource's identity is specified directly
/// (not via a `ResourceId`) since it doesn't exist yet.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CreateResource {
    pub slug: String,
    pub doctype: String,
    /// Resolved home anchor — exactly one of a context or a cognitive map.
    /// Surfaces must parse+resolve the ref and select the correct variant before
    /// building this command.
    pub home: HomeAnchor,
    pub title: String,
    pub body: Option<BodyUpdate>,
    /// Caller-supplied managed_meta. Backends will apply doctype defaults
    /// for any required fields the caller omitted.
    pub managed_meta: ManagedMeta,
    /// Free-form user metadata (open_meta tier).
    pub open_meta: Option<Value>,
    /// Canonical resource URI for dedup and storage. Callers that derive a
    /// `kb://`-scheme URI from context/doctype/slug should set this; callers
    /// without a pre-computed URI may leave it `None` and the server will
    /// store an empty string (the existing behavior for the HTTP create path).
    pub origin_uri: Option<String>,
    /// Pre-computed chunks (base64-encoded MessagePack) supplied by the caller.
    /// When `Some`, passed through to `ingest_service::ingest` so the server
    /// does not need to run the embed pipeline. When `None`, the server
    /// recomputes via the pipeline (if enabled) or returns an error for
    /// non-empty bodies without the pipeline feature.
    pub chunks_packed: Option<String>,
    /// Caller-supplied content hash. Sync clients pre-compute this so the
    /// canonical body_hash round-trips verbatim into `kb_resource_audits`
    /// and the manifest. When `None`, the server recomputes from `body`.
    pub content_hash: Option<String>,
    /// Per-act correlation + authorship (`temper_core::types::authorship::ActContext`). Empty by
    /// default; surfaces fill it from caller-supplied invocation/authorship. The backend stamps the
    /// authored `resource_created` act's `kb_events.invocation_id`/`metadata`. Correlation here never
    /// authorizes the write — the resource-create authz runs independently.
    #[serde(default, skip_serializing_if = "ActContext::is_empty")]
    pub act: ActContext,
    pub origin: Surface,
}

/// Show a single resource.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ShowResource {
    pub resource: ResourceId,
    pub origin: Surface,
}

/// File-move spec for `UpdateResource`. Both fields are optional and
/// independent; supplying either (or both) triggers a move.
///
/// - `context_to`: resolved `ContextId` for the destination context. Set by
///   surfaces after resolving a context ref (UUID or `@owner/slug`) via
///   `resolve_context_ref`. `DbBackend` re-homes the resource to this context.
/// - `type_to`: move the resource to a new doc-type. Stored as the `doc_type`
///   property by `DbBackend`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MoveSpec {
    pub context_to: Option<ContextId>,
    pub type_to: Option<String>,
}

/// Update a resource — partial; any combination of body, managed_meta,
/// open_meta may be supplied.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UpdateResource {
    pub resource: ResourceId,
    pub body: Option<BodyUpdate>,
    pub managed_meta: Option<ManagedMeta>,
    pub open_meta: Option<Value>,
    /// Move spec: `DbBackend` re-homes the resource when `move_to.context_to`
    /// is `Some`. The `context_to` field carries a **resolved** `ContextId`;
    /// surfaces must parse+resolve a context ref before setting it. For the
    /// cloud CLI path the raw context ref string travels via `context_ref`
    /// below and is resolved by the API handler, so `move_to.context_to` is
    /// `None` on the CLI side.
    pub move_to: Option<MoveSpec>,
    /// Raw, unresolved context ref (UUID string or `@owner/slug`) supplied by
    /// the CLI surface. The cloud-backend translator forwards this verbatim
    /// as the `context_to` field of the HTTP wire payload; the API handler
    /// parses and resolves it server-side. `None` when not a CLI-originated
    /// context move (API handler builds `move_to` directly after resolution).
    pub context_ref: Option<String>,
    /// Per-act correlation + authorship — stamps every sub-event of the update fan-out
    /// (`block_mutated` / `property_set` / `resource_updated` / `resource_rehomed`). Empty by default;
    /// correlation never authorizes the write.
    #[serde(default, skip_serializing_if = "ActContext::is_empty")]
    pub act: ActContext,
    pub origin: Surface,
}

/// Delete a resource. In the cloud-first model this is a soft-delete on the
/// server.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DeleteResource {
    pub resource: ResourceId,
    /// Bypass the local-file confirmation prompt (required for non-TTY).
    pub force: bool,
    /// Per-act correlation + authorship — stamps the `resource_deleted` act. Empty by default;
    /// correlation never authorizes the write.
    #[serde(default, skip_serializing_if = "ActContext::is_empty")]
    pub act: ActContext,
    pub origin: Surface,
}

/// List resources, filtered.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ListResources {
    pub filter: ListFilter,
    pub origin: Surface,
}

/// Semantic / hybrid search.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SearchResources {
    pub query: SearchQuery,
    pub origin: Surface,
}

/// Assert a new relationship from `source` to `target`. Cloud-only —
/// emits a `relationship_asserted` event and projects an edge row. Both
/// endpoints are pre-resolved `ResourceId`s.
/// Forward-reference-by-slug targets are no longer expressible here — that
/// legacy capability lives only in the frontmatter-declaration ingest path.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AssertRelationship {
    pub source: ResourceId,
    pub target: ResourceId,
    pub edge_kind: temper_core::types::graph::EdgeKind,
    pub polarity: temper_core::types::graph::Polarity,
    pub label: String,
    pub weight: f64,
    /// Per-act correlation + authorship — stamps the authored `relationship_asserted` act. Empty by
    /// default; correlation never authorizes the write.
    #[serde(default, skip_serializing_if = "ActContext::is_empty")]
    pub act: ActContext,
    pub origin: Surface,
}

/// Retype an existing relationship (identified by its `edge_handle` — the
/// `kb_edges` id returned by `assert`) — changes `edge_kind` / `polarity`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RetypeRelationship {
    pub edge_handle: EdgeId,
    pub edge_kind: temper_core::types::graph::EdgeKind,
    pub polarity: temper_core::types::graph::Polarity,
    /// Per-act correlation + authorship — stamps the `relationship_retyped` act. Empty by default;
    /// correlation never authorizes the write.
    #[serde(default, skip_serializing_if = "ActContext::is_empty")]
    pub act: ActContext,
    pub origin: Surface,
}

/// Reweight an existing relationship — changes `weight`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReweightRelationship {
    pub edge_handle: EdgeId,
    pub weight: f64,
    /// Per-act correlation + authorship — stamps the `relationship_reweighted` act. Empty by default;
    /// correlation never authorizes the write.
    #[serde(default, skip_serializing_if = "ActContext::is_empty")]
    pub act: ActContext,
    pub origin: Surface,
}

/// Fold (retract) an existing relationship — sets `is_folded = true`.
/// Optional human-readable reason for audit.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FoldRelationship {
    pub edge_handle: EdgeId,
    pub reason: Option<String>,
    /// Per-act correlation + authorship — stamps the authored `relationship_folded` act. Empty by
    /// default; correlation never authorizes the write.
    #[serde(default, skip_serializing_if = "ActContext::is_empty")]
    pub act: ActContext,
    pub origin: Surface,
}

/// Set (upsert) a facet — a typed property row (`kb_properties`) attached to a
/// resource. `values` is the facet's JSON payload; `weight` is the facet's
/// salience/confidence weight.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SetFacet {
    pub resource: ResourceId,
    pub values: serde_json::Value,
    pub weight: f64,
    /// Per-act correlation + authorship — stamps the authored `facet_set` act. Empty by
    /// default; correlation never authorizes the write.
    #[serde(default, skip_serializing_if = "ActContext::is_empty")]
    pub act: ActContext,
    pub origin: Surface,
}

/// Open an invocation envelope — the trace primitive. `originating_cogmap` /
/// `parent_cogmap` are substrate cogmap ids (not resource refs). The
/// invocation id is minted by the backend and returned.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OpenInvocation {
    pub trigger_kind: String,
    pub originating_cogmap: CogmapId,
    pub parent_cogmap: Option<CogmapId>,
    pub origin: Surface,
}

/// Close an invocation with a terminal disposition + opaque outcome.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CloseInvocation {
    pub invocation: uuid::Uuid,
    pub disposition: temper_core::types::invocation::Disposition,
    pub outcome: serde_json::Value,
    pub origin: Surface,
}

/// Advance a team-self-cognition cogmap's steward ingest watermark to a given event id (T4a). The
/// stub write the future steward calls on run completion so the next delta counts only what landed
/// after this run. Gated on cogmap-write (`cogmap_authorable_by_profile`), auth before write.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AdvanceStewardWatermark {
    pub cogmap: CogmapId,
    pub event_id: uuid::Uuid,
    pub origin: Surface,
}

/// Compose one deterministic steward-dispatch pass (goal 019f3220): reap stale jobs, sweep drifted
/// maps the principal can read, enqueue each (deduped by the in-flight index), and claim up to `cap`
/// steward jobs for fan-out. Returns the claimed jobs — the caller starts one isolated session per
/// job, each tending a single cogmap. `threshold`/`cap` default when `None`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StewardDispatchTick {
    pub threshold: Option<i64>,
    pub cap: Option<i64>,
    pub origin: Surface,
}

/// Re-materialize a cognitive map's regions when its formation delta since the last materialize
/// clears `threshold` (T4b) — the cron-invokable trigger for the substrate's own (deterministic,
/// non-authored) region-formation cadence. Gated on cogmap-write (`cogmap_authorable_by_profile`),
/// auth before write. Below threshold it is a safe no-op; `threshold == None` uses the default.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MaterializeOnThreshold {
    pub cogmap: CogmapId,
    pub threshold: Option<i64>,
    pub origin: Surface,
}

/// Reconcile the L0-style kernel slice of a cognitive map to a pre-embedded
/// desired-state manifest. Idempotent + additive-only + provenance-scoped: the
/// `request` is the contract, the fired events are its consequence. `cogmap_id`
/// is a temper_next cogmap id (not a resource ref).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReconcileCognitiveMap {
    pub cogmap_id: CogmapId,
    pub request: temper_core::types::reconcile::ReconcileCogmapRequest,
    /// Authorship for the reconcile run — stamped on EVERY act the reconcile fires (the run's
    /// server-minted invocation correlates them; any caller-supplied `act.invocation` is ignored, the
    /// reconcile owns its envelope). Empty by default.
    #[serde(default, skip_serializing_if = "ActContext::is_empty")]
    pub act: ActContext,
    pub origin: Surface,
}

/// Genesis (create) a new cognitive map (cogmap + telos charter resource) from a manifest. The new
/// map's identity lives INSIDE the `request` (manifest-supplied uuidv7, or backend-minted when absent) —
/// there is no separate path id (unlike reconcile's `cogmap_id`). Idempotent at a given id: re-genesis
/// is a no-op returning `created: false`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CreateCognitiveMap {
    pub request: temper_core::types::reconcile::CreateCogmapRequest,
    pub origin: Surface,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn show_resource_carries_resource_id() {
        let id = ResourceId(uuid::Uuid::now_v7());
        let cmd = ShowResource {
            resource: id,
            origin: Surface::CliCloud,
        };
        // Exercises the type compiles + the field is reachable.
        assert_eq!(cmd.resource, id);
    }

    #[test]
    fn update_resource_all_optional_fields_default_none() {
        let cmd = UpdateResource {
            resource: ResourceId(uuid::Uuid::now_v7()),
            body: None,
            managed_meta: None,
            open_meta: None,
            move_to: None,
            context_ref: None,
            act: Default::default(),
            origin: Surface::ApiHttp,
        };
        assert!(cmd.body.is_none());
        assert!(cmd.managed_meta.is_none());
        assert!(cmd.open_meta.is_none());
        assert!(cmd.move_to.is_none());
        assert!(cmd.context_ref.is_none());
    }

    #[test]
    fn create_resource_does_not_use_resource_ref() {
        let cmd = CreateResource {
            slug: "new-task".to_string(),
            doctype: "task".to_string(),
            home: temper_core::types::home::HomeAnchor::Context(
                temper_core::types::ids::ContextId::new(),
            ),
            title: "New task".to_string(),
            body: None,
            managed_meta: ManagedMeta::default(),
            open_meta: None,
            origin_uri: None,
            chunks_packed: None,
            content_hash: None,
            act: Default::default(),
            origin: Surface::CliCloud,
        };
        assert_eq!(cmd.slug, "new-task");
    }

    #[test]
    fn list_resources_default_filter_serializes() {
        let cmd = ListResources {
            filter: ListFilter::default(),
            origin: Surface::Mcp,
        };
        let s = serde_json::to_string(&cmd).unwrap();
        let back: ListResources = serde_json::from_str(&s).unwrap();
        assert_eq!(cmd, back);
    }

    #[test]
    fn assert_relationship_command_round_trips() {
        let cmd = AssertRelationship {
            source: ResourceId(uuid::Uuid::now_v7()),
            target: ResourceId(uuid::Uuid::now_v7()),
            edge_kind: temper_core::types::graph::EdgeKind::LeadsTo,
            polarity: temper_core::types::graph::Polarity::Inverse,
            label: "depends_on".to_string(),
            weight: 1.0,
            act: Default::default(),
            origin: Surface::ApiHttp,
        };
        let s = serde_json::to_string(&cmd).unwrap();
        assert_eq!(serde_json::from_str::<AssertRelationship>(&s).unwrap(), cmd);
    }

    #[test]
    fn assert_relationship_carries_resolved_target_id() {
        let cmd = AssertRelationship {
            source: ResourceId(uuid::Uuid::nil()),
            target: ResourceId(uuid::Uuid::now_v7()),
            edge_kind: temper_core::types::graph::EdgeKind::Near,
            polarity: temper_core::types::graph::Polarity::Forward,
            label: "rel".into(),
            weight: 1.0,
            act: Default::default(),
            origin: Surface::Mcp,
        };
        assert_ne!(uuid::Uuid::from(cmd.target), uuid::Uuid::nil());
    }

    #[test]
    fn open_invocation_round_trips() {
        let cmd = OpenInvocation {
            trigger_kind: "manual".to_string(),
            originating_cogmap: CogmapId::new(),
            parent_cogmap: None,
            origin: Surface::Mcp,
        };
        let v = serde_json::to_value(&cmd).unwrap();
        let back: OpenInvocation = serde_json::from_value(v).unwrap();
        assert_eq!(back, cmd);
    }
}
