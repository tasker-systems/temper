use axum::Json;
use serde::Serialize;

use crate::error::ApiResult;

#[derive(Debug, Serialize)]
pub struct HealthResponse {
    pub status: &'static str,
    pub version: &'static str,
}

pub async fn health_check() -> ApiResult<Json<HealthResponse>> {
    Ok(Json(HealthResponse {
        status: "ok",
        version: "0.1.0",
    }))
}
