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
        // Sourced from Cargo at compile time so the reported version can never
        // drift from the crate's actual version.
        version: env!("CARGO_PKG_VERSION"),
    }))
}
