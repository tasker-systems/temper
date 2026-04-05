use axum::extract::{Path, State};
use axum::Json;
use serde_json::Value;
use uuid::Uuid;

use crate::error::{ApiResult, ErrorBody};
use crate::middleware::auth::AuthUser;
use crate::services::meta_service;
use crate::state::AppState;

use temper_core::types::managed_meta::MetaUpdatePayload;

#[utoipa::path(
    put,
    path = "/api/resources/{id}/meta",
    tag = "Meta",
    params(("id" = Uuid, Path, description = "Resource ID")),
    security(("bearer_auth" = [])),
    request_body = MetaUpdatePayload,
    responses(
        (status = 200, description = "Meta updated", body = Value),
        (status = 401, description = "Unauthorized", body = ErrorBody),
        (status = 403, description = "Forbidden", body = ErrorBody),
        (status = 404, description = "Not found", body = ErrorBody),
    )
)]
pub async fn update_meta(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(resource_id): Path<Uuid>,
    Json(payload): Json<MetaUpdatePayload>,
) -> ApiResult<Json<Value>> {
    meta_service::update_meta(&state.pool, auth.0.profile.id, resource_id, payload)
        .await
        .map(Json)
}
