use axum::extract::{Query, State};
use axum::Json;

use crate::error::{ApiResult, ErrorBody};
use crate::middleware::auth::AuthUser;
use crate::services::search_service::{self, SearchParams, SearchResultRow};
use crate::state::AppState;

#[utoipa::path(
    get,
    path = "/api/search",
    tag = "Search",
    params(SearchParams),
    security(("bearer_auth" = [])),
    responses(
        (status = 200, description = "Search results", body = Vec<SearchResultRow>),
        (status = 401, description = "Unauthorized", body = ErrorBody),
    )
)]
pub async fn search(
    State(state): State<AppState>,
    auth: AuthUser,
    Query(params): Query<SearchParams>,
) -> ApiResult<Json<Vec<SearchResultRow>>> {
    search_service::search(&state.pool, auth.0.profile.id, params)
        .await
        .map(Json)
}
