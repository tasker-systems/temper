//! Resource API types — shared between temper-api and temper-client.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use uuid::Uuid;

/// Row type matching the `resources` table.
#[derive(Debug, Clone, Serialize, FromRow)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
pub struct ResourceRow {
    pub id: Uuid,
    pub kb_context_id: Uuid,
    pub kb_doc_type_id: Uuid,
    pub uri: String,
    pub title: String,
    pub slug: Option<String>,
    pub content_hash: Option<String>,
    pub mimetype: Option<String>,
    pub originator_profile_id: Uuid,
    pub owner_profile_id: Uuid,
    pub is_active: bool,
    pub created: DateTime<Utc>,
    pub updated: DateTime<Utc>,
}

/// Query parameters for listing visible resources.
#[derive(Debug, Clone, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::IntoParams))]
pub struct ResourceListParams {
    /// Filter by context ID.
    pub kb_context_id: Option<Uuid>,
    /// Maximum results to return (default 50, max 200).
    pub limit: Option<i64>,
    /// Offset for pagination.
    pub offset: Option<i64>,
}

/// Request body for creating a resource.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
pub struct ResourceCreateRequest {
    pub kb_context_id: Uuid,
    pub kb_doc_type_id: Uuid,
    pub uri: String,
    pub title: String,
    pub slug: Option<String>,
    pub mimetype: Option<String>,
}

/// Request body for updating a resource.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
pub struct ResourceUpdateRequest {
    pub title: Option<String>,
    pub slug: Option<String>,
    pub mimetype: Option<String>,
}

/// Chunk used to reconstitute markdown content.
#[derive(Debug, Clone, Serialize, FromRow)]
pub struct ContentChunk {
    pub chunk_index: i32,
    pub header_path: String,
    pub content: String,
}

/// Response body for resource content.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
pub struct ContentResponse {
    pub resource_id: Uuid,
    pub markdown: String,
}

/// Response body for resource deletion.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
pub struct DeleteResponse {
    pub deleted: bool,
}
