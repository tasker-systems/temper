//! The query-string parameters of the published doors.
//!
//! Each struct is the `?…` half of one route's contract: the server deserializes it from the query
//! string, the client serializes it onto one, and `utoipa::IntoParams` (behind `web-api`) documents
//! each field — from its doc comment — as that route's parameters in `openapi.json`. They live here
//! because they are wire types, and every wire type is temper-core's.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Query parameters for `GET /api/graph/cogmaps/{id}/panorama`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::IntoParams))]
pub struct CogmapPanoramaQuery {
    /// Optional lens override; defaults to the cogmap's primary lens.
    pub lens_id: Option<Uuid>,
}

/// Query parameters for `GET /api/graph/regions/composition`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::IntoParams))]
pub struct RegionCompositionQuery {
    /// Comma-separated region ids — one region, or a shift-selected union.
    pub ids: String,
    /// Composition depth; defaults to 1, clamped to 3 by the service.
    pub depth: Option<i32>,
}

/// Query parameters for `GET /api/graph/entry`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::IntoParams))]
pub struct EntryQuery {
    /// How many marks to draw. Defaults to the ruled K and is clamped by the service.
    pub k: Option<i32>,
    /// Comma-separated anchor ids (contexts or cogmaps) to confine the ranking to.
    ///
    /// Omitted means the reader's whole visible corpus. Present, it answers *"a place, and no
    /// question at all"* — ranking within the place rather than across everything, which is what
    /// lets a named place with no question be served by this read instead of by the recency page.
    #[serde(rename = "in")]
    pub places: Option<String>,
}

/// Query parameters for `GET /api/graph/traverse`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::IntoParams))]
pub struct TraverseQuery {
    /// Comma-separated node ids to hop from.
    ///
    /// Named `from` to match the page grammar the split ruled (spec §10.2):
    /// `/graph/@me?q=<grounding question>&from=<node-ids>&depth=<n>`, so the address says exactly
    /// what read produced the screen.
    pub from: String,
    /// Hops to walk. Defaults to 1, clamped to 3 by the service.
    pub depth: Option<i32>,
}

/// Query parameters for `GET /api/graph/contexts/panorama`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::IntoParams))]
pub struct ContextPanoramaQuery {
    /// Context ref in decorated form (`@me/<slug>`, `@<handle>/<slug>`, `+<team-slug>/<slug>`)
    /// or a bare UUID.
    pub context_ref: String,
    /// Property key the residual tray groups by. Defaults to `doc_type` (spec D2 — a
    /// parameter, not a constant).
    pub group_by: Option<String>,
    /// Comma-separated doc-types treated as containers. Defaults to `goal` (spec D4).
    pub container_types: Option<String>,
    /// Container-walk depth; defaults to 2, clamped to 3 by the SQL.
    pub depth: Option<i32>,
}

/// Query parameters for `GET /api/graph/contexts/composition`. Exactly one of `container` /
/// `group` is required.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::IntoParams))]
pub struct ContextCompositionQuery {
    /// Context ref (decorated or bare UUID) — the drill's home context.
    pub context_ref: String,
    /// Container resource id to drill.
    pub container: Option<Uuid>,
    /// Residual bucket to drill, as `<group_key>:<group_value>`.
    pub group: Option<String>,
    /// Comma-separated doc-types treated as containers. Defaults to `goal` (spec D4).
    pub container_types: Option<String>,
    /// Composition (drill) depth; defaults to 1, clamped to 3 by the service.
    pub depth: Option<i32>,
    /// Container-walk depth used to resolve a `group` bucket's members. Must match the `depth`
    /// the panorama was called with, or the drill yields a different set than the tray showed.
    /// Ignored for a `container` drill. Defaults to 2.
    pub container_depth: Option<i32>,
}

/// Query params for the context list. `retired = true` switches the read from the visibility
/// axis to the ADMIN axis: a retired context is invisible to `contexts_readable_by_teams` by
/// construction, so it can only be listed by someone who could have retired it.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::IntoParams))]
pub struct ListContextsQuery {
    /// List retired contexts you administer instead of the contexts you can read.
    pub retired: Option<bool>,
}

/// Query params for [`resolve`].
// The doc comment names an item in the server crate it moved from, and rustdoc cannot resolve it
// from here. Kept byte for byte: utoipa publishes it into openapi.json (and ts-rs into the TS
// types), so rewording it moves the published contract.
#[allow(rustdoc::broken_intra_doc_links)]
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::IntoParams))]
pub struct ResolveContextQuery {
    /// The context ref to resolve: `@me/<slug>`, `@<handle>/<slug>`, `+<team>/<slug>`, or a bare
    /// UUID. One grammar — `temper_core::context_ref::parse_context_ref`, the parser the CLI and
    /// the MCP tools use.
    pub context_ref: String,
}

/// Query params for the context shape / region-metrics reads.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextShapeQuery {
    /// Optional lens filter; omit for all lenses.
    pub lens: Option<Uuid>,
}

/// Query params for the context materialize-delta read. `threshold` is optional (omit → the
/// service default).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextMaterializeDeltaQuery {
    /// Materialize threshold to gate on; the service default applies when omitted.
    pub threshold: Option<i64>,
}

/// Query params for the bounded connections read — an optional limit on the page.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::IntoParams))]
pub struct ConnectionsQuery {
    /// Max connections to return (default 50, clamped to 1..=200).
    pub limit: Option<i32>,
}

/// Query params for the lineage read — an optional depth bound on the walk.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::IntoParams))]
pub struct LineageQuery {
    /// Max hop distance to walk from the seed (default 16, clamped to 1..=64).
    pub depth: Option<i32>,
}

/// Query flags for `GET /api/connections`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::IntoParams))]
pub struct ConnectionListQuery {
    /// Include revoked connections. Default `false`.
    #[serde(default)]
    pub include_revoked: bool,
}

/// Query flags for `GET /api/machine-clients`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::IntoParams))]
pub struct MachineClientListQuery {
    /// Include revoked machine clients. Default `false`.
    #[serde(default)]
    pub include_revoked: bool,
}

/// Query parameters for `GET /api/invocations`. Both filters are optional (omit → unfiltered).
///
/// No `IntoParams` derive: the route documents these inline in its `params(...)`, and deriving
/// would change `openapi.json`. It lives here because it is the route's wire shape all the same.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct InvocationListQuery {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cogmap: Option<Uuid>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
}

/// Query flags for `GET /api/subscriptions`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::IntoParams))]
pub struct SubscriptionListQuery {
    /// Include revoked subscriptions. Default `false`.
    #[serde(default)]
    pub include_revoked: bool,
    /// Optional filter: only subscriptions against this connection.
    pub connection_id: Option<Uuid>,
}

/// Query parameters for [`count_mine`].
// The doc comment names an item in the server crate it moved from, and rustdoc cannot resolve it
// from here. Kept byte for byte: utoipa publishes it into openapi.json (and ts-rs into the TS
// types), so rewording it moves the published contract.
#[allow(rustdoc::broken_intra_doc_links)]
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::IntoParams))]
pub struct CountMineQuery {
    /// Restrict the `matching` half of the answer to invitations to this team.
    pub team_slug: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlobListQuery {
    /// Optional home scope: `kb_contexts` or `kb_cogmaps`. With `home_id`, scopes the
    /// list to blobs homed in that anchor; absent, the list is every blob the caller
    /// can read — which is the caller's own view, never a discovery oracle.
    pub home_table: Option<String>,
    pub home_id: Option<Uuid>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SegmentQuery {
    pub seq: u32,
}

/// Query params for `GET /api/resources/{id}` — additive over the incumbent shape.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceShowQuery {
    /// Comma-separated extra sections to fill on the view (`ResourceSection` names,
    /// kebab-case). `open-meta` is the door's baseline and is always filled;
    /// `embedding-status` is the additive one (B1). Unknown names are a `400` naming this
    /// door's vocabulary.
    pub sections: Option<String>,
}

/// Query params for the materialize-delta read. `threshold` is optional (omit → the service default).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MaterializeDeltaQuery {
    pub threshold: Option<i64>,
}

/// Query params for the shape read. `lens` is optional (omit → all lenses).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShapeQuery {
    pub lens: Option<Uuid>,
}

/// Query params for the delta read. `threshold` is optional (omit → the service default).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeltaQuery {
    pub threshold: Option<i64>,
}

/// Query params for the sweep read. `cap` is optional (omit → the service default).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SweepQuery {
    pub cap: Option<i64>,
}
