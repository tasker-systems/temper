use axum::extract::{Path, Query, State};
use axum::Json;
use uuid::Uuid;

use crate::error::{ApiResult, ErrorBody};
use crate::middleware::auth::AuthUser;
use crate::services::event_service::{self, EventListParams, EventRow};
use crate::state::AppState;
use temper_core::types::api::EventCursorResponse;

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
    path = "/api/events/{kb_context_id}/cursor",
    tag = "Events",
    params(
        ("kb_context_id" = Uuid, Path, description = "The context whose latest event id is requested"),
    ),
    security(("bearer_auth" = [])),
    responses(
        (status = 200, description = "Latest event id for the context", body = EventCursorResponse),
        (status = 401, description = "Unauthorized", body = ErrorBody),
    )
)]
pub async fn cursor(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(kb_context_id): Path<Uuid>,
) -> ApiResult<Json<EventCursorResponse>> {
    let latest_event_id =
        event_service::latest_event_id_for_context(&state.pool, auth.0.profile.id, kb_context_id)
            .await?;
    Ok(Json(EventCursorResponse { latest_event_id }))
}
