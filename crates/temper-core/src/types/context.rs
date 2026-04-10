//! Context types — API request/response types for context CRUD.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use uuid::Uuid;

use super::ids::ContextId;

/// Response row for context endpoints.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "context.ts"))]
#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
pub struct ContextRow {
    pub id: ContextId,
    pub name: String,
    pub kb_owner_table: String,
    pub kb_owner_id: Uuid,
    pub created: DateTime<Utc>,
    pub updated: DateTime<Utc>,
}

/// Context with resource count — used by the list endpoint.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "context.ts"))]
#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
pub struct ContextRowWithCounts {
    pub id: ContextId,
    pub name: String,
    pub kb_owner_table: String,
    pub kb_owner_id: Uuid,
    pub created: DateTime<Utc>,
    pub updated: DateTime<Utc>,
    pub resource_count: i64,
}

/// Request body for POST /api/contexts.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
pub struct ContextCreateRequest {
    pub name: String,
}
