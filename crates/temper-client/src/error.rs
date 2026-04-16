use std::time::Duration;

use temper_core::error::CliAccessDetails;

#[derive(Debug, thiserror::Error)]
pub enum ClientError {
    #[error("not authenticated — run `temper auth login`")]
    NotAuthenticated,

    #[error("token expired")]
    TokenExpired,

    #[error("forbidden")]
    Forbidden,

    #[error("system access required")]
    SystemAccessRequired(Box<CliAccessDetails>),

    #[error("{resource} not found")]
    NotFound { resource: String },

    #[error("conflict: {message}")]
    Conflict { message: String },

    #[error("rate limited — retry after {retry_after:?}")]
    RateLimited { retry_after: Duration },

    #[error("server error ({status}): {message}")]
    Server { status: u16, message: String },

    #[error("network error: {0}")]
    Network(#[from] reqwest::Error),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("{0}")]
    Other(String),
}

pub type Result<T> = std::result::Result<T, ClientError>;
