use axum::extract::{Query, State};
use axum::Json;

use crate::error::{ApiResult, ErrorBody};
use crate::middleware::auth::AuthUser;
use crate::services::event_service::{self, EventListParams, EventRow};
use crate::state::AppState;
use temper_core::types::api::{EventCursorParams, EventCursorResponse};

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

#[utoipa::path(
    get,
    path = "/api/events/cursor",
    tag = "Events",
    params(EventCursorParams),
    security(("bearer_auth" = [])),
    responses(
        (status = 200, description = "Latest event id for the context", body = EventCursorResponse),
        (status = 401, description = "Unauthorized", body = ErrorBody),
    )
)]
pub async fn cursor(
    State(state): State<AppState>,
    auth: AuthUser,
    Query(params): Query<EventCursorParams>,
) -> ApiResult<Json<EventCursorResponse>> {
    let latest_event_id = event_service::latest_event_id_for_context(
        &state.pool,
        auth.0.profile.id,
        params.kb_context_id,
    )
    .await?;
    Ok(Json(EventCursorResponse { latest_event_id }))
}
