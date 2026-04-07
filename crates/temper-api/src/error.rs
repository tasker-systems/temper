use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::Serialize;
use utoipa::ToSchema;

#[derive(Debug, thiserror::Error)]
pub enum ApiError {
    #[error("Not found")]
    NotFound,
    #[error("Unauthorized: {0}")]
    Unauthorized(String),
    #[error("Forbidden")]
    Forbidden,
    #[error("System access required")]
    SystemAccessRequired {
        details: Box<temper_core::types::access_gate::SystemAccessDetails>,
    },
    #[error("Bad request: {0}")]
    BadRequest(String),
    #[error("Conflict: {0}")]
    Conflict(String),
    #[error("Internal error: {0}")]
    Internal(String),
}

pub type ApiResult<T> = Result<T, ApiError>;

#[derive(Serialize, ToSchema)]
pub(crate) struct ErrorBody {
    error: ErrorDetail,
}

#[derive(Serialize, ToSchema)]
pub(crate) struct ErrorDetail {
    code: &'static str,
    message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    details: Option<serde_json::Value>,
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, code) = match &self {
            ApiError::NotFound => (StatusCode::NOT_FOUND, "NOT_FOUND"),
            ApiError::Unauthorized(_) => (StatusCode::UNAUTHORIZED, "UNAUTHORIZED"),
            ApiError::Forbidden => (StatusCode::FORBIDDEN, "FORBIDDEN"),
            ApiError::SystemAccessRequired { .. } => {
                (StatusCode::FORBIDDEN, "SYSTEM_ACCESS_REQUIRED")
            }
            ApiError::BadRequest(_) => (StatusCode::BAD_REQUEST, "BAD_REQUEST"),
            ApiError::Conflict(_) => (StatusCode::CONFLICT, "CONFLICT"),
            ApiError::Internal(_) => (StatusCode::INTERNAL_SERVER_ERROR, "INTERNAL_ERROR"),
        };

        let message = match &self {
            ApiError::SystemAccessRequired { .. } => {
                "This system requires approved access.".to_string()
            }
            other => other.to_string(),
        };
        let status_code = status.as_u16();

        match &self {
            ApiError::NotFound => {
                tracing::debug!(status_code, error_code = code, %message, "not found");
            }
            ApiError::Conflict(_) => {
                tracing::info!(status_code, error_code = code, %message, "conflict");
            }
            ApiError::Unauthorized(_) | ApiError::Forbidden => {
                tracing::warn!(status_code, error_code = code, %message, "auth error");
            }
            ApiError::SystemAccessRequired { .. } => {
                tracing::info!(status_code, error_code = code, "system access required");
            }
            ApiError::BadRequest(_) => {
                tracing::warn!(status_code, error_code = code, %message, "bad request");
            }
            ApiError::Internal(_) => {
                tracing::error!(status_code, error_code = code, %message, "internal error");
            }
        }

        let details_json = match &self {
            ApiError::SystemAccessRequired { details } => {
                Some(serde_json::to_value(details).unwrap_or_default())
            }
            _ => None,
        };

        let body = ErrorBody {
            error: ErrorDetail {
                code,
                message,
                details: details_json,
            },
        };
        (status, axum::Json(body)).into_response()
    }
}

impl From<sqlx::Error> for ApiError {
    fn from(err: sqlx::Error) -> Self {
        match &err {
            sqlx::Error::RowNotFound => ApiError::NotFound,
            sqlx::Error::Database(db_err) if db_err.code().as_deref() == Some("23505") => {
                ApiError::Conflict("Resource already exists".to_string())
            }
            _ => {
                tracing::error!("Database error: {err}");
                ApiError::Internal("An internal error occurred".to_string())
            }
        }
    }
}

impl From<serde_json::Error> for ApiError {
    fn from(err: serde_json::Error) -> Self {
        ApiError::BadRequest(format!("Invalid JSON: {err}"))
    }
}
