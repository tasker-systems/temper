use axum::extract::{Path, State};
use axum::Json;
use uuid::Uuid;

use crate::middleware::auth::AuthUser;
use temper_core::types::standing::StandingShape;
use temper_services::error::{ApiResult, ErrorBody};
use temper_services::services::evidential_standing_service;
use temper_services::state::AppState;

#[utoipa::path(
    get,
    operation_id = "resource_evidence",
    path = "/api/resources/{id}/evidence",
    tag = "Resources",
    params(("id" = Uuid, Path, description = "Resource ID")),
    security(("bearer_auth" = [])),
    responses(
        (status = 200, description = "Resource evidential-standing shape", body = StandingShape),
        (status = 401, description = "Unauthorized", body = ErrorBody),
        (status = 404, description = "Not found", body = ErrorBody),
    )
)]
pub async fn evidence(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(resource_id): Path<Uuid>,
) -> ApiResult<Json<StandingShape>> {
    evidential_standing_service::resource_evidence(&state.pool, auth.0.profile.id, resource_id)
        .await
        .map(Json)
}
