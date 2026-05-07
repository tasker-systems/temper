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

impl From<ApiError> for temper_core::error::TemperError {
    fn from(err: ApiError) -> Self {
        use temper_core::error::{CliAccessDetails, TemperError};
        match err {
            ApiError::NotFound => TemperError::NotFound("resource not found".to_string()),
            ApiError::Forbidden => TemperError::Forbidden,
            ApiError::Unauthorized(s) => TemperError::Unauthorized(s),
            ApiError::BadRequest(s) => TemperError::BadRequest(s),
            ApiError::Conflict(s) => TemperError::Conflict(s),
            ApiError::Internal(s) => TemperError::Api(format!("internal: {s}")),
            ApiError::SystemAccessRequired { details } => {
                let join_request_status = details
                    .join_request_status
                    .as_ref()
                    .map(|s| format!("{s:?}").to_lowercase());
                TemperError::SystemAccessRequired(Box::new(CliAccessDetails {
                    email: details.email,
                    display_name: details.display_name,
                    access_mode: details.access_mode,
                    join_request_status,
                    request_url: details.request_url,
                    cli_command: details.cli_command,
                }))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use temper_core::error::TemperError;

    #[test]
    fn api_error_not_found_maps_to_temper_not_found() {
        let api: ApiError = ApiError::NotFound;
        let t: TemperError = api.into();
        assert!(matches!(t, TemperError::NotFound(_)));
    }

    #[test]
    fn api_error_forbidden_maps_to_temper_forbidden() {
        let t: TemperError = ApiError::Forbidden.into();
        assert!(matches!(t, TemperError::Forbidden));
    }

    #[test]
    fn api_error_bad_request_carries_message() {
        let t: TemperError = ApiError::BadRequest("missing field".into()).into();
        match t {
            TemperError::BadRequest(s) => assert_eq!(s, "missing field"),
            other => panic!("expected BadRequest, got {other:?}"),
        }
    }

    #[test]
    fn api_error_conflict_carries_message() {
        let t: TemperError = ApiError::Conflict("duplicate".into()).into();
        match t {
            TemperError::Conflict(s) => assert_eq!(s, "duplicate"),
            other => panic!("expected Conflict, got {other:?}"),
        }
    }

    #[test]
    fn api_error_unauthorized_carries_message() {
        let t: TemperError = ApiError::Unauthorized("no token".into()).into();
        match t {
            TemperError::Unauthorized(s) => assert_eq!(s, "no token"),
            other => panic!("expected Unauthorized, got {other:?}"),
        }
    }

    #[test]
    fn api_error_internal_maps_to_temper_api() {
        let t: TemperError = ApiError::Internal("oops".into()).into();
        match t {
            TemperError::Api(s) => assert!(s.contains("oops")),
            other => panic!("expected Api(_), got {other:?}"),
        }
    }

    #[test]
    fn api_error_system_access_required_preserves_field_set() {
        use temper_core::types::access_gate::SystemAccessDetails;
        let api = ApiError::SystemAccessRequired {
            details: Box::new(SystemAccessDetails {
                email: Some("a@b.co".into()),
                display_name: Some("A".into()),
                access_mode: "join_request".into(),
                join_request_status: None,
                request_url: Some("https://x".into()),
                cli_command: Some("temper join".into()),
            }),
        };
        let t: TemperError = api.into();
        match t {
            TemperError::SystemAccessRequired(details) => {
                assert_eq!(details.email.as_deref(), Some("a@b.co"));
                assert_eq!(details.display_name.as_deref(), Some("A"));
                assert_eq!(details.access_mode, "join_request");
                assert_eq!(details.request_url.as_deref(), Some("https://x"));
                assert_eq!(details.cli_command.as_deref(), Some("temper join"));
            }
            other => panic!("expected SystemAccessRequired, got {other:?}"),
        }
    }
}
