//! Wire types for data artifact reads and writes — the shape API, MCP, and CLI surfaces use.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::types::authorship::ActInput;
use crate::types::ids::{DataArtifactId, ResourceId};

/// One data artifact, as returned by the API and MCP surfaces.
///
/// This is the readback `RetrievedArtifact` projected onto a wire shape: the typed enums
/// (`intent`, `shape_state`, `kind_owner`) are rendered as their serde-renamed strings so a
/// consumer that does not share the Rust types still gets a self-describing response.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "data_artifact.ts"))]
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
pub struct ArtifactView {
    pub artifact_id: DataArtifactId,
    pub resource_id: ResourceId,
    /// `"kb_profiles"` or `"kb_teams"` — the namespace half of the family name.
    pub kind_owner_table: String,
    pub kind_owner_id: Uuid,
    pub artifact_kind: String,
    /// `"current"` / `"member"` / `"pinned"` — the closed selection vocabulary.
    pub intent: String,
    pub precedence: f64,
    pub content_hash: String,
    pub content_bytes: i64,
    /// `"never_declared"` today; will carry conformance verdicts when the shape registry lands.
    pub shape_state: String,
    pub is_folded: bool,
    pub created: DateTime<Utc>,
    /// The content payload. `null` when the artifact was committed with no bytes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content: Option<serde_json::Value>,
}

/// One row of per-family counts, as returned by the counts endpoint.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "data_artifact.ts"))]
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
pub struct ArtifactCountRow {
    pub kind_owner_table: String,
    pub kind_owner_id: Uuid,
    pub artifact_kind: String,
    pub count: i64,
    pub total_bytes: i64,
}

/// Query parameters for listing artifacts on a resource.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::IntoParams))]
pub struct ArtifactListParams {
    /// Filter by the bare family name (e.g. `"measurement"`).
    #[serde(default)]
    pub kind: Option<String>,
    /// Filter by selection intent: `"current"`, `"member"`, or `"pinned"`.
    #[serde(default)]
    pub intent: Option<String>,
    /// Include folded (superseded) artifacts in the result. Default: `false`.
    #[serde(default)]
    pub include_folded: Option<bool>,
    /// Return per-family counts instead of full artifacts. No content hydration.
    /// Default: `false` (full artifacts with content).
    #[serde(default)]
    pub counts: Option<bool>,
}

/// The namespace half of the family name — mirrors `temper_substrate::payloads::KindOwner`
/// in a temper-core-native shape so the wire type does not depend on the substrate crate.
/// `None` (the ordinary case) lets the SQL wrapper default the namespace from the owning
/// resource's home.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "data_artifact.ts"))]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
pub enum KindOwnerInput {
    #[serde(rename = "kb_profiles")]
    Profile(Uuid),
    #[serde(rename = "kb_teams")]
    Team(Uuid),
}

/// Request body for `POST /api/resources/{id}/artifacts` — commit one data artifact.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "data_artifact.ts"))]
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
pub struct ArtifactCommitRequest {
    /// The bare family name, qualified by `kind_owner` (or defaulted from the resource's home).
    pub kind: String,
    /// Override the namespace half of the family name. Omit to let the server default it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind_owner: Option<KindOwnerInput>,
    /// Selection intent: `"current"`, `"member"`, or `"pinned"`.
    pub intent: String,
    /// Ordering among peers. Meaningful for `member`; carried for all. Default: `0.0`.
    #[serde(default)]
    pub precedence: f64,
    /// The structured payload as JSON. Hashed and stored verbatim — the hash is the proof.
    pub content: serde_json::Value,
    /// Artifacts this one replaces, named explicitly by the writer. Empty = replaces nothing.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub supersedes: Vec<DataArtifactId>,
    /// Per-act correlation + authorship flags. Flattened into the request body.
    #[serde(flatten)]
    pub act: ActInput,
}

/// Response body for `POST /api/resources/{id}/artifacts` — the committed artifact's ID
/// and its full readback view.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "data_artifact.ts"))]
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
pub struct ArtifactCommitResponse {
    pub artifact_id: DataArtifactId,
    pub artifact: ArtifactView,
}
