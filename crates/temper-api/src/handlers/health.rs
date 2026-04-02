use axum::Json;

use temper_core::types::api::HealthResponse;

use crate::error::ApiResult;

#[utoipa::path(
    get,
    path = "/api/health",
    tag = "Health",
    responses(
        (status = 200, description = "Service is healthy", body = HealthResponse),
    )
)]
pub async fn health_check() -> ApiResult<Json<HealthResponse>> {
    Ok(Json(HealthResponse {
        status: "ok",
        version: "0.1.0",
    }))
}
