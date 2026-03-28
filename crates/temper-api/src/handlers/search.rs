use axum::extract::{Query, State};
use axum::Json;

use crate::error::ApiResult;
use crate::middleware::auth::AuthUser;
use crate::services::search_service::{self, SearchParams, SearchResultRow};
use crate::state::AppState;

pub async fn search(
    State(state): State<AppState>,
    auth: AuthUser,
    Query(params): Query<SearchParams>,
) -> ApiResult<Json<Vec<SearchResultRow>>> {
    search_service::search(&state.pool, auth.0.profile.id, params)
        .await
        .map(Json)
}
