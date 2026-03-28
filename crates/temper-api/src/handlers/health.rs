use axum::Json;
use serde::Serialize;
use utoipa::ToSchema;

use crate::error::ApiResult;

#[derive(Debug, Serialize, ToSchema)]
pub struct HealthResponse {
    pub status: &'static str,
    pub version: &'static str,
}

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
