use sqlx::PgPool;
use uuid::Uuid;

use crate::error::ApiResult;

pub use temper_core::types::api::{SearchParams, SearchResultRow};

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
