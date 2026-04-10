//! Resource API types — shared between temper-api and temper-client.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use uuid::Uuid;

use super::ids::{ContextId, DocTypeId, ProfileId, ResourceId};

/// Row type for resource listings — includes joined display fields
/// and managed_meta projections from `vault_resources_browse` view.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "resource.ts"))]
#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
pub struct ResourceRow {
    pub id: ResourceId,
    pub kb_context_id: ContextId,
    pub kb_doc_type_id: DocTypeId,
    pub origin_uri: String,
    pub title: String,
    pub slug: Option<String>,
    pub originator_profile_id: ProfileId,
    pub owner_profile_id: ProfileId,
    pub is_active: bool,
    pub created: DateTime<Utc>,
    pub updated: DateTime<Utc>,
    // Joined display fields
    pub context_name: String,
    pub doc_type_name: String,
    pub owner_handle: String,
    // Managed meta projections
    pub stage: Option<String>,
    #[cfg_attr(feature = "typescript", ts(type = "number | null"))]
    pub seq: Option<i64>,
    pub mode: Option<String>,
    pub effort: Option<String>,
}

/// Sort field for resource listing.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "resource.ts"))]
#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum ResourceSortField {
    #[default]
    Updated,
    Created,
    Title,
    Stage,
    Seq,
    ContextName,
    DocTypeName,
}

/// Sort direction.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "resource.ts"))]
#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum SortOrder {
    #[default]
    Desc,
    Asc,
}

/// Query parameters for listing visible resources.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "resource.ts"))]
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[cfg_attr(feature = "web-api", derive(utoipa::IntoParams))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
pub struct ResourceListParams {
    pub kb_context_id: Option<Uuid>,
    pub kb_doc_type_id: Option<Uuid>,
    pub context_name: Option<String>,
    pub doc_type_name: Option<String>,
    pub owner: Option<String>,
    pub q: Option<String>,
    pub sort: Option<ResourceSortField>,
    pub order: Option<SortOrder>,
    #[cfg_attr(feature = "typescript", ts(type = "number | null"))]
    pub limit: Option<i64>,
    #[cfg_attr(feature = "typescript", ts(type = "number | null"))]
    pub offset: Option<i64>,
}

/// Aggregated doc-type facet counts for the current filter set.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "resource.ts"))]
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
pub struct ResourceFacets {
    pub doc_type: std::collections::HashMap<String, i64>,
}

/// Paginated response for resource list endpoints, with doc-type facets.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "resource.ts"))]
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
pub struct ResourceListResponse {
    pub rows: Vec<ResourceRow>,
    pub total: i64,
    pub facets: ResourceFacets,
}

/// Request body for creating a resource.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
pub struct ResourceCreateRequest {
    pub kb_context_id: Uuid,
    pub kb_doc_type_id: Uuid,
    pub origin_uri: String,
    pub title: String,
    pub slug: Option<String>,
}

/// Request body for updating a resource.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
pub struct ResourceUpdateRequest {
    pub title: Option<String>,
    pub slug: Option<String>,
}

/// Chunk used to reconstitute markdown content.
#[derive(Debug, Clone, Serialize, FromRow)]
pub struct ContentChunk {
    pub chunk_index: i32,
    pub header_path: String,
    pub content: String,
}

/// Response body for resource content.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "resource.ts"))]
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
pub struct ContentResponse {
    pub resource_id: ResourceId,
    pub markdown: String,
}

/// Response body for resource deletion.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
pub struct DeleteResponse {
    pub deleted: bool,
}
