//! Wire types for data-artifact shape registry reads and writes — the API, MCP, and CLI surfaces use.
//!
//! These mirror [`crate::types::data_artifact`] in structure: the read response projects the
//! substrate readback's typed enums onto serde-renamed strings so a consumer that does not share
//! the Rust types gets a self-describing response, and the write request carries the same
//! `ActInput` flatten for attributed authorship.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::types::artifact_ask::Vocabulary;
use crate::types::authorship::ActInput;
use crate::types::data_artifact::KindOwnerInput;
use crate::types::ids::ShapeId;
use crate::types::query::PlanRefusal;

/// Whether a non-conforming commit is refused or merely recorded.
///
/// The closed vocabulary the register closes over (spec §6): `advisory` (default) or `enforcing`.
/// Mirrors `temper_substrate::payloads::EnforcementMode` in a temper-core-native shape so the wire
/// type does not depend on the substrate crate, with the full derive stack the wire surfaces require.
/// `schemars(inline)` is required for MCP tool inputs (the same fix applied to `ConfidenceBand` and
/// the graph enums — a `$ref` into `$defs` reaches the Anthropic tool-use layer with no type signal).
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "typescript",
    ts(export, export_to = "data_artifact_shape.ts")
)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
#[cfg_attr(feature = "mcp", schemars(inline))]
#[serde(rename_all = "snake_case")]
pub enum EnforcementMode {
    /// Default. A non-conforming commit succeeds and is recorded as non-conforming.
    Advisory,
    /// A non-conforming commit is refused, and the refusal carries what failed.
    Enforcing,
}

/// One declared shape, as returned by the API and MCP surfaces.
///
/// This is the readback `RetrievedShape` projected onto a wire shape: `HomeAnchor` and `KindOwner`
/// are rendered as their `(table, id)` scalar pairs (the same split `ArtifactView` uses for
/// `kind_owner_table`/`kind_owner_id`) so a consumer that does not share the Rust types still gets a
/// self-describing response.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "typescript",
    ts(export, export_to = "data_artifact_shape.ts")
)]
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
pub struct ShapeView {
    pub shape_id: ShapeId,
    /// `"kb_contexts"` or `"kb_cogmaps"` — the home anchor discriminant.
    pub home_anchor_table: String,
    pub home_anchor_id: Uuid,
    /// `"kb_profiles"` or `"kb_teams"` — the namespace half of the family name.
    pub kind_owner_table: String,
    pub kind_owner_id: Uuid,
    pub artifact_kind: String,
    /// The family's shape: a field tree, which is a subset of JSON Schema 2020-12 (see the
    /// data-artifacts skill for its vocabulary).
    pub schema: serde_json::Value,
    /// `"advisory"` or `"enforcing"`.
    pub enforcement: EnforcementMode,
    /// The chain depth of the assert/fold lineage — 1 for the first declaration, N for the Nth.
    pub shape_version: i32,
    pub is_folded: bool,
    pub created: DateTime<Utc>,
    /// What an artifact ask may name against this version of the shape: each askable path with
    /// its declared types, its `enum`/`const` values, and the operators feasible at it — the
    /// vocabulary a refusal's `teaches` narrows, derived by the same walk that validates an ask.
    /// Bounded, and a truncated vocabulary names the bound in `truncated`.
    ///
    /// **Present only on the by-id read** (`GET /api/shapes/{id}`, `get_data_artifact_shape`,
    /// `temper data-artifact schema show`). Listings leave it **absent, which is not empty**: a
    /// listing never pays one enumeration per shape, and an absent field never says that nothing
    /// is askable. Empty when this version's schema cannot be walked — `unaskable` then says why —
    /// and `truncated: [schema_bytes]` when the schema is past the byte bound the walk reads.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "typescript", ts(optional))]
    pub askable: Option<Vocabulary>,
    /// Why this version's schema cannot be walked: the refusal step 3 of asking gives it —
    /// `shape_schema_uncompilable`, with its sentence for a schema that does not compile under the
    /// no-fetch compiler (naming the first keyword or cap refused and its schema path), or with a sentence naming the byte bound and
    /// `teaches.beyond_bound: schema_bytes` for one too large to read, which is never compiled.
    /// What an ask naming this shape is told can differ: an ask naming a folded version is refused
    /// `shape_superseded` first (`is_folded`). Absent whenever the schema can be walked, and on
    /// listings.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "typescript", ts(optional))]
    pub unaskable: Option<PlanRefusal>,
}

/// Request body for declaring a shape — the write surface.
///
/// The home anchor is resolved by the handler from the URL path (API) or tool input (MCP), not
/// carried in this body — the same split `ArtifactCommitRequest` uses for the resource id. `kind`
/// is the bare family name, qualified by `kind_owner` (or defaulted from the home).
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "typescript",
    ts(export, export_to = "data_artifact_shape.ts")
)]
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
pub struct ShapeDeclareRequest {
    /// The bare family name, qualified by `kind_owner` (or defaulted from the home).
    pub kind: String,
    /// Override the namespace half of the family name. Omit to let the server default it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind_owner: Option<KindOwnerInput>,
    /// The family's shape: a field tree, which is a subset of JSON Schema 2020-12 (see the
    /// data-artifacts skill for its vocabulary). Validated Rust-side.
    pub schema: serde_json::Value,
    /// Whether a non-conforming commit is refused (`enforcing`) or merely recorded (`advisory`).
    pub enforcement: EnforcementMode,
    /// Per-act correlation + authorship flags. Flattened into the request body.
    #[serde(flatten)]
    pub act: ActInput,
}
