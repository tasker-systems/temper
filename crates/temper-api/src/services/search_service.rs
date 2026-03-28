use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use uuid::Uuid;

use crate::error::ApiResult;

/// Query parameters for search.
#[derive(Debug, Deserialize)]
pub struct SearchParams {
    /// Full-text or semantic query string.
    pub q: String,
    /// Limit to a specific context.
    pub kb_context_id: Option<Uuid>,
    /// Maximum results to return (default 20, max 100).
    pub limit: Option<i64>,
}

/// A single search result.
#[derive(Debug, Serialize)]
pub struct SearchResultRow {
    pub resource_id: Uuid,
    pub title: String,
    pub uri: String,
    pub snippet: String,
    pub score: f32,
}

/// Search resources visible to the given profile.
///
/// Stub implementation — returns empty results. The contract (types, params,
/// response shape) is in place for Task 8 / vector search integration.
#[expect(
    unused_variables,
    reason = "stub — params will drive vector search in a future task"
)]
pub async fn search(
    pool: &PgPool,
    profile_id: Uuid,
    params: SearchParams,
) -> ApiResult<Vec<SearchResultRow>> {
    Ok(vec![])
}
