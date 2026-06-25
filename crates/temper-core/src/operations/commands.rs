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

use crate::types::ids::ResourceId;
use crate::types::managed_meta::ManagedMeta;

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
    pub context: String,
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
    pub origin: Surface,
}

/// Show a single resource.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ShowResource {
    pub resource: ResourceId,
    pub origin: Surface,
}

/// File-move spec for `UpdateResource`. Both fields are optional and
/// independent; supplying either (or both) triggers a filesystem move.
///
/// - `context_to`: move the file to a new context directory and update
///   `temper-context` in frontmatter. The DB backend ignores this field —
///   the new context is communicated via `managed_meta.context`.
/// - `type_to`: move the file to a new doc-type directory and update
///   `temper-type` in frontmatter. Likewise ignored by DbBackend.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MoveSpec {
    pub context_to: Option<String>,
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
    /// File-move spec (local-mode only). `DbBackend` ignores `move_to` —
    /// the new context/type is conveyed via `managed_meta` which DbBackend
    /// already handles. This field carries no SQL and does not affect the
    /// `.sqlx/` query cache.
    pub move_to: Option<MoveSpec>,
    pub origin: Surface,
}

/// Delete a resource. In the cloud-first model this is a soft-delete on the
/// server.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DeleteResource {
    pub resource: ResourceId,
    /// Bypass the local-file confirmation prompt (required for non-TTY).
    pub force: bool,
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
    pub edge_kind: crate::types::graph::EdgeKind,
    pub polarity: crate::types::graph::Polarity,
    pub label: String,
    pub weight: f64,
    pub origin: Surface,
}

/// Retype an existing relationship (identified by its `edge_handle` — the
/// backend-opaque edge handle from `assert`) — changes `edge_kind` / `polarity`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RetypeRelationship {
    pub edge_handle: uuid::Uuid,
    pub edge_kind: crate::types::graph::EdgeKind,
    pub polarity: crate::types::graph::Polarity,
    pub origin: Surface,
}

/// Reweight an existing relationship — changes `weight`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReweightRelationship {
    pub edge_handle: uuid::Uuid,
    pub weight: f64,
    pub origin: Surface,
}

/// Fold (retract) an existing relationship — sets `is_folded = true`.
/// Optional human-readable reason for audit.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FoldRelationship {
    pub edge_handle: uuid::Uuid,
    pub reason: Option<String>,
    pub origin: Surface,
}

/// Open an invocation envelope — the trace primitive. `originating_cogmap` /
/// `parent_cogmap` are temper_next cogmap ids (not resource refs). The
/// invocation id is minted by the backend and returned.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OpenInvocation {
    pub trigger_kind: String,
    pub originating_cogmap: uuid::Uuid,
    pub parent_cogmap: Option<uuid::Uuid>,
    pub origin: Surface,
}

/// Close an invocation with a terminal disposition + opaque outcome.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CloseInvocation {
    pub invocation: uuid::Uuid,
    pub disposition: crate::types::invocation::Disposition,
    pub outcome: serde_json::Value,
    pub origin: Surface,
}

/// Reconcile the L0-style kernel slice of a cognitive map to a pre-embedded
/// desired-state manifest. Idempotent + additive-only + provenance-scoped: the
/// `request` is the contract, the fired events are its consequence. `cogmap_id`
/// is a temper_next cogmap id (not a resource ref).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReconcileCognitiveMap {
    pub cogmap_id: uuid::Uuid,
    pub request: crate::types::reconcile::ReconcileCogmapRequest,
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
            origin: Surface::ApiHttp,
        };
        assert!(cmd.body.is_none());
        assert!(cmd.managed_meta.is_none());
        assert!(cmd.open_meta.is_none());
        assert!(cmd.move_to.is_none());
    }

    #[test]
    fn create_resource_does_not_use_resource_ref() {
        let cmd = CreateResource {
            slug: "new-task".to_string(),
            doctype: "task".to_string(),
            context: "temper".to_string(),
            title: "New task".to_string(),
            body: None,
            managed_meta: ManagedMeta::default(),
            open_meta: None,
            origin_uri: None,
            chunks_packed: None,
            content_hash: None,
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
            edge_kind: crate::types::graph::EdgeKind::LeadsTo,
            polarity: crate::types::graph::Polarity::Inverse,
            label: "depends_on".to_string(),
            weight: 1.0,
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
            edge_kind: crate::types::graph::EdgeKind::Near,
            polarity: crate::types::graph::Polarity::Forward,
            label: "rel".into(),
            weight: 1.0,
            origin: Surface::Mcp,
        };
        assert_ne!(uuid::Uuid::from(cmd.target), uuid::Uuid::nil());
    }

    #[test]
    fn open_invocation_round_trips() {
        let cmd = OpenInvocation {
            trigger_kind: "manual".to_string(),
            originating_cogmap: uuid::Uuid::now_v7(),
            parent_cogmap: None,
            origin: Surface::Mcp,
        };
        let v = serde_json::to_value(&cmd).unwrap();
        let back: OpenInvocation = serde_json::from_value(v).unwrap();
        assert_eq!(back, cmd);
    }
}
