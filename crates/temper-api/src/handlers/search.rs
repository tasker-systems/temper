use axum::extract::State;
use axum::Json;

use crate::error::{ApiResult, ErrorBody};
use crate::middleware::auth::AuthUser;
use crate::services::search_service::{SearchParams, UnifiedSearchResultRow};
use crate::state::AppState;

#[utoipa::path(
    post,
    path = "/api/search",
    tag = "Search",
    request_body = SearchParams,
    security(("bearer_auth" = [])),
    responses(
        (status = 200, description = "Search results", body = Vec<UnifiedSearchResultRow>),
        (status = 400, description = "Invalid request", body = ErrorBody),
        (status = 401, description = "Unauthorized", body = ErrorBody),
    )
)]
pub async fn search(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(params): Json<SearchParams>,
) -> ApiResult<Json<Vec<UnifiedSearchResultRow>>> {
    crate::backend::read_selector::search_select(
        state.backend_selection,
        &state.pool,
        auth.0.profile.id,
        params,
    )
    .await
    .map(Json)
}
