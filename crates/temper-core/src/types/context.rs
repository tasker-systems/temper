//! Context types — API request/response types for context CRUD.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use uuid::Uuid;

/// Response row for context endpoints.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "context.ts"))]
#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
pub struct ContextRow {
    pub id: Uuid,
    pub name: String,
    pub kb_owner_table: String,
    pub kb_owner_id: Uuid,
    pub created: DateTime<Utc>,
    pub updated: DateTime<Utc>,
}

/// Request body for POST /api/contexts.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
pub struct ContextCreateRequest {
    pub name: String,
}
