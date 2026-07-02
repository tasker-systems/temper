//! Shared-secret gate for the internal SAML reconcile endpoint (AS -> temper-api).
//! Not a JWT path: the caller is the co-deployed Authorization Server, trusted by a
//! constant-time-compared shared secret from `INTERNAL_RECONCILE_SECRET`.

use axum::body::Body;
use axum::extract::State;
use axum::http::Request;
use axum::middleware::Next;
use axum::response::Response;

use temper_services::error::ApiError;
use temper_services::state::AppState;

pub const INTERNAL_SECRET_HEADER: &str = "X-Temper-Internal-Secret";

/// Constant-time-ish comparison: equal length AND equal bytes, no early return on content.
/// `configured == None` means the endpoint is disabled and never matches.
fn secret_matches(presented: &str, configured: Option<&str>) -> bool {
    let Some(expected) = configured else {
        return false;
    };
    if expected.is_empty() || presented.len() != expected.len() {
        return false;
    }
    let mut diff: u8 = 0;
    for (a, b) in presented.bytes().zip(expected.bytes()) {
        diff |= a ^ b;
    }
    diff == 0
}

/// Rejects the request unless it carries the correct `X-Temper-Internal-Secret` header.
pub async fn require_internal_secret(
    State(state): State<AppState>,
    request: Request<Body>,
    next: Next,
) -> Result<Response, ApiError> {
    let presented = request
        .headers()
        .get(INTERNAL_SECRET_HEADER)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if !secret_matches(presented, state.config.internal_reconcile_secret.as_deref()) {
        tracing::warn!("internal reconcile: rejected (bad or missing shared secret)");
        return Err(ApiError::Unauthorized(
            "invalid internal secret".to_string(),
        ));
    }
    Ok(next.run(request).await)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn constant_time_eq_matches_only_identical_secrets() {
        assert!(secret_matches("hunter2", Some("hunter2")));
        assert!(!secret_matches("hunter2", Some("Hunter2")));
        assert!(!secret_matches("hunter2", Some("")));
        assert!(!secret_matches("hunter2", None)); // endpoint unconfigured
        assert!(!secret_matches("", Some("hunter2")));
    }
}
