use axum::extract::{Path, State};
use axum::Json;
use uuid::Uuid;

use crate::middleware::auth::AuthUser;
use temper_core::types::api::EventCursorResponse;
use temper_core::types::element_trail::{ElementKind, EventTrail};
use temper_core::types::ids::{ContextId, ProfileId};
use temper_services::error::{ApiError, ApiResult, ErrorBody};
use temper_services::services::event_service;
use temper_services::state::AppState;

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
    let latest_event_id = event_service::latest_event_id_for_context(
        &state.pool,
        ProfileId::from(auth.0.profile().id),
        ContextId::from(kb_context_id),
    )
    .await?;
    Ok(Json(EventCursorResponse { latest_event_id }))
}

/// GET /api/graph/elements/{kind}/{id}/trail — R5 element event-trail. kind ∈ {node, edge}.
#[utoipa::path(
    get,
    path = "/api/graph/elements/{kind}/{id}/trail",
    tag = "Events",
    params(
        ("kind" = String, Path, description = "node | edge"),
        ("id" = Uuid, Path, description = "resource id (node) or edge id")
    ),
    security(("bearer_auth" = [])),
    responses(
        (status = 200, description = "Event trail", body = EventTrail),
        (status = 400, description = "Unknown element kind")
    )
)]
pub async fn element_trail(
    State(state): State<AppState>,
    auth: AuthUser,
    Path((kind, id)): Path<(String, Uuid)>,
) -> ApiResult<Json<EventTrail>> {
    let element_kind = match kind.as_str() {
        "node" => ElementKind::Node,
        "edge" => ElementKind::Edge,
        _ => {
            return Err(ApiError::BadRequest(
                "element kind must be 'node' or 'edge'".into(),
            ))
        }
    };
    event_service::element_trail(
        &state.pool,
        ProfileId::from(auth.0.profile().id),
        element_kind,
        id,
    )
    .await
    .map(Json)
}
