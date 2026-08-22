//! Wire types for data-artifact shape registry reads and writes — the API, MCP, and CLI surfaces use.
//!
//! These mirror [`crate::types::data_artifact`] in structure: the read response projects the
//! substrate readback's typed enums onto serde-renamed strings so a consumer that does not share
//! the Rust types gets a self-describing response, and the write request carries the same
//! `ActInput` flatten for attributed authorship.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::types::authorship::ActInput;
use crate::types::data_artifact::KindOwnerInput;
use crate::types::ids::ShapeId;

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
    /// The JSON Schema (draft 2020-12) governing this family.
    pub schema: serde_json::Value,
    /// `"advisory"` or `"enforcing"`.
    pub enforcement: EnforcementMode,
    /// The chain depth of the assert/fold lineage — 1 for the first declaration, N for the Nth.
    pub shape_version: i32,
    pub is_folded: bool,
    pub created: DateTime<Utc>,
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
    /// The JSON Schema (draft 2020-12) governing this family. Validated Rust-side.
    pub schema: serde_json::Value,
    /// Whether a non-conforming commit is refused (`enforcing`) or merely recorded (`advisory`).
    pub enforcement: EnforcementMode,
    /// Per-act correlation + authorship flags. Flattened into the request body.
    #[serde(flatten)]
    pub act: ActInput,
}
