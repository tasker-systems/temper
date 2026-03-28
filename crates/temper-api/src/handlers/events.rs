use axum::extract::{Query, State};
use axum::Json;

use crate::error::{ApiResult, ErrorBody};
use crate::middleware::auth::AuthUser;
use crate::services::event_service::{self, EventListParams, EventRow};
use crate::state::AppState;

#[utoipa::path(
    get,
    path = "/api/events",
    tag = "Events",
    params(EventListParams),
    security(("bearer_auth" = [])),
    responses(
        (status = 200, description = "List of visible events", body = Vec<EventRow>),
        (status = 401, description = "Unauthorized", body = ErrorBody),
    )
)]
pub async fn list(
    State(state): State<AppState>,
    auth: AuthUser,
    Query(params): Query<EventListParams>,
) -> ApiResult<Json<Vec<EventRow>>> {
    event_service::list_visible(&state.pool, auth.0.profile.id, params)
        .await
        .map(Json)
}
