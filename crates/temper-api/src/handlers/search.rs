use axum::extract::State;
use axum::Json;

use crate::middleware::auth::AuthUser;
use temper_core::types::api::{SearchParams, UnifiedSearchResultRow};
use temper_core::types::ids::ProfileId;
use temper_services::error::{ApiResult, ErrorBody};
use temper_services::state::AppState;

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
    crate::backend::substrate_read::search_select(
        &state.pool,
        ProfileId::from(auth.0.profile.id),
        params,
    )
    .await
    .map(Json)
}
