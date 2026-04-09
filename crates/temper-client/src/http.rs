use std::fmt;
use std::time::{Duration, Instant};

use reqwest::{
    header::{HeaderValue, AUTHORIZATION},
    Client, RequestBuilder, Response, StatusCode,
};
use serde::de::DeserializeOwned;
use serde::Deserialize;
use serde_json::Value;

use tracing::Instrument;

use crate::error::{ClientError, Result};

/// Wraps a `reqwest::Client` with base URL and optional device identity.
///
/// All request methods prepend `base_url` to the given path and inject the
/// `X-Temper-Device-Id` header when a device ID has been set.
#[derive(Debug, Clone)]
pub struct HttpClient {
    inner: Client,
    base_url: String,
    device_id: Option<String>,
    token_override: Option<String>,
}

/// Describes an outgoing HTTP request for structured logging.
///
/// Constructed inside [`HttpClient::send`] from method and path parameters.
/// Never contains sensitive data (tokens, bodies).
struct ApiRequest<'a> {
    method: &'a reqwest::Method,
    path: &'a str,
    has_auth: bool,
}

impl fmt::Display for ApiRequest<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} {}", self.method, self.path)
    }
}

impl HttpClient {
    pub fn new(base_url: &str, device_id: Option<String>) -> Self {
        let inner = Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .expect("failed to build reqwest client");

        Self {
            inner,
            base_url: base_url.trim_end_matches('/').to_owned(),
            device_id,
            token_override: None,
        }
    }

    /// Construct an `HttpClient` with a fixed token that bypasses `auth.json`.
    ///
    /// Intended for testing and scripting contexts where reading from the
    /// filesystem is undesirable.
    pub fn with_token_override(base_url: &str, device_id: Option<String>, token: String) -> Self {
        Self {
            token_override: Some(token),
            ..Self::new(base_url, device_id)
        }
    }

    /// Return the token to use for authenticated requests.
    ///
    /// Returns the token override if one was set at construction time;
    /// otherwise falls back to [`crate::auth::current_token`], which reads
    /// `~/.config/temper/auth.json`.
    pub fn resolve_token(&self) -> Result<String> {
        if let Some(tok) = &self.token_override {
            return Ok(tok.clone());
        }
        crate::auth::current_token()
    }

    fn url(&self, path: &str) -> String {
        format!("{}/{}", self.base_url, path.trim_start_matches('/'))
    }

    fn apply_device_header(&self, req: RequestBuilder) -> RequestBuilder {
        if let Some(id) = &self.device_id {
            req.header("X-Temper-Device-Id", id.as_str())
        } else {
            req
        }
    }

    pub fn get(&self, path: &str) -> RequestBuilder {
        self.apply_device_header(self.inner.get(self.url(path)))
    }

    pub fn post(&self, path: &str) -> RequestBuilder {
        self.apply_device_header(self.inner.post(self.url(path)))
    }

    pub fn patch(&self, path: &str) -> RequestBuilder {
        self.apply_device_header(self.inner.patch(self.url(path)))
    }

    pub fn delete(&self, path: &str) -> RequestBuilder {
        self.apply_device_header(self.inner.delete(self.url(path)))
    }

    pub fn put(&self, path: &str) -> RequestBuilder {
        self.apply_device_header(self.inner.put(self.url(path)))
    }

    /// Send a request, injecting `Bearer` auth if `token` is provided.
    ///
    /// `method` and `path` are for observability only — they describe the
    /// request for structured logging. They must match the `RequestBuilder`
    /// but are not validated against it.
    pub async fn send(
        &self,
        method: &reqwest::Method,
        path: &str,
        req: RequestBuilder,
        token: Option<&str>,
    ) -> Result<Response> {
        let api_req = ApiRequest {
            method,
            path,
            has_auth: token.is_some(),
        };
        let span = tracing::debug_span!(
            "http_request",
            request = %api_req,
            has_auth = api_req.has_auth,
            status = tracing::field::Empty,
            latency_ms = tracing::field::Empty,
        );

        async move {
            let req = if let Some(tok) = token {
                let value = HeaderValue::from_str(&format!("Bearer {tok}"))
                    .map_err(|e| ClientError::Other(format!("invalid token header: {e}")))?;
                req.header(AUTHORIZATION, value)
            } else {
                req
            };

            let start = Instant::now();
            let resp = req.send().await?;
            let status = resp.status();
            let latency_ms = start.elapsed().as_millis() as u64;

            tracing::Span::current().record("status", status.as_u16());
            tracing::Span::current().record("latency_ms", latency_ms);

            if status.is_success() {
                return Ok(resp);
            }

            let body_text = resp.text().await.unwrap_or_default();
            let err = map_status_to_error(status, &body_text);
            tracing::warn!(
                status = status.as_u16(),
                latency_ms,
                error = %err,
                "request failed",
            );
            Err(err)
        }
        .instrument(span)
        .await
    }

    /// Send a request and deserialize the JSON body on success.
    pub async fn send_json<T: DeserializeOwned>(
        &self,
        method: &reqwest::Method,
        path: &str,
        req: RequestBuilder,
        token: Option<&str>,
    ) -> Result<T> {
        let resp = self.send(method, path, req, token).await?;
        let bytes = resp.bytes().await?;
        let value: T = serde_json::from_slice(&bytes)?;
        Ok(value)
    }
}

/// Maps an HTTP status code and raw response body to a [`ClientError`].
///
/// Extracted as a pure function so it can be unit-tested without network calls.
pub fn map_status_to_error(status: StatusCode, body: &str) -> ClientError {
    match status.as_u16() {
        401 => ClientError::NotAuthenticated,
        403 => {
            if let Some(details) = parse_system_access_details(body) {
                ClientError::SystemAccessRequired {
                    email: details.email,
                    display_name: details.display_name,
                    access_mode: details.access_mode.unwrap_or_else(|| "unknown".to_string()),
                    join_request_status: details.join_request_status,
                    request_url: details.request_url,
                    cli_command: details.cli_command,
                }
            } else {
                ClientError::Forbidden
            }
        }
        404 => {
            let resource =
                parse_error_field(body, "resource").unwrap_or_else(|| "unknown".to_owned());
            ClientError::NotFound { resource }
        }
        409 => {
            let message =
                parse_error_field(body, "message").unwrap_or_else(|| "conflict".to_owned());
            ClientError::Conflict { message }
        }
        429 => {
            // Parse `Retry-After` value from body if embedded, or fall back to 60 s.
            let secs = body.parse::<u64>().unwrap_or(60);
            ClientError::RateLimited {
                retry_after: Duration::from_secs(secs),
            }
        }
        s if s >= 500 => {
            let message = parse_error_message(body).unwrap_or_else(|| {
                status
                    .canonical_reason()
                    .unwrap_or("server error")
                    .to_owned()
            });
            ClientError::Server { status: s, message }
        }
        s => {
            let message =
                parse_error_message(body).unwrap_or_else(|| format!("unexpected status {s}"));
            ClientError::Server { status: s, message }
        }
    }
}

/// Details from a `SystemAccessRequired` 403 response.
#[derive(Deserialize)]
struct SystemAccessErrorDetails {
    email: Option<String>,
    display_name: Option<String>,
    access_mode: Option<String>,
    join_request_status: Option<String>,
    request_url: Option<String>,
    cli_command: Option<String>,
}

/// Try to parse `SystemAccessRequired` details from a 403 response body.
fn parse_system_access_details(body: &str) -> Option<SystemAccessErrorDetails> {
    let v: Value = serde_json::from_str(body).ok()?;
    let code = v.get("error")?.get("code")?.as_str()?;
    if code != "SYSTEM_ACCESS_REQUIRED" {
        return None;
    }
    let details = v.get("error")?.get("details")?;
    serde_json::from_value(details.clone()).ok()
}

/// Try to extract `{ "error": { "message": "..." } }` from an API error body.
fn parse_error_message(body: &str) -> Option<String> {
    let v: Value = serde_json::from_str(body).ok()?;
    v.get("error")?
        .get("message")?
        .as_str()
        .map(ToOwned::to_owned)
}

/// Try to extract a named field from `{ "error": { "<field>": "..." } }`.
fn parse_error_field(body: &str, field: &str) -> Option<String> {
    let v: Value = serde_json::from_str(body).ok()?;
    v.get("error")?.get(field)?.as_str().map(ToOwned::to_owned)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn status(code: u16) -> StatusCode {
        StatusCode::from_u16(code).unwrap()
    }

    #[test]
    fn test_401_maps_to_not_authenticated() {
        let err = map_status_to_error(status(401), "");
        assert!(matches!(err, ClientError::NotAuthenticated));
    }

    #[test]
    fn test_403_maps_to_forbidden() {
        let err = map_status_to_error(status(403), "");
        assert!(matches!(err, ClientError::Forbidden));
    }

    #[test]
    fn test_404_with_resource_field() {
        let body =
            r#"{"error":{"code":"not_found","resource":"workspace/abc","message":"not found"}}"#;
        let err = map_status_to_error(status(404), body);
        assert!(matches!(err, ClientError::NotFound { resource } if resource == "workspace/abc"));
    }

    #[test]
    fn test_404_without_resource_falls_back_to_unknown() {
        let err = map_status_to_error(status(404), "{}");
        assert!(matches!(err, ClientError::NotFound { resource } if resource == "unknown"));
    }

    #[test]
    fn test_409_with_message_field() {
        let body = r#"{"error":{"code":"conflict","message":"already exists"}}"#;
        let err = map_status_to_error(status(409), body);
        assert!(matches!(err, ClientError::Conflict { message } if message == "already exists"));
    }

    #[test]
    fn test_409_without_message_falls_back() {
        let err = map_status_to_error(status(409), "{}");
        assert!(matches!(err, ClientError::Conflict { message } if message == "conflict"));
    }

    #[test]
    fn test_429_parses_retry_after_seconds() {
        let err = map_status_to_error(status(429), "30");
        assert!(
            matches!(err, ClientError::RateLimited { retry_after } if retry_after == Duration::from_secs(30))
        );
    }

    #[test]
    fn test_429_defaults_to_60_seconds() {
        let err = map_status_to_error(status(429), "not-a-number");
        assert!(
            matches!(err, ClientError::RateLimited { retry_after } if retry_after == Duration::from_secs(60))
        );
    }

    #[test]
    fn test_500_maps_to_server_error_with_message() {
        let body = r#"{"error":{"code":"internal","message":"something went wrong"}}"#;
        let err = map_status_to_error(status(500), body);
        assert!(
            matches!(err, ClientError::Server { status: 500, message } if message == "something went wrong")
        );
    }

    #[test]
    fn test_500_without_body_uses_canonical_reason() {
        let err = map_status_to_error(status(500), "");
        assert!(matches!(err, ClientError::Server { status: 500, .. }));
    }

    #[test]
    fn test_422_maps_to_server_error_with_unexpected_status_message() {
        let err = map_status_to_error(status(422), "{}");
        assert!(matches!(err, ClientError::Server { status: 422, .. }));
    }

    #[test]
    fn test_api_request_display_formats_method_and_path() {
        let req = ApiRequest {
            method: &reqwest::Method::GET,
            path: "/api/resources",
            has_auth: true,
        };
        assert_eq!(req.to_string(), "GET /api/resources");
    }

    #[test]
    fn test_api_request_display_post() {
        let req = ApiRequest {
            method: &reqwest::Method::POST,
            path: "/api/ingest",
            has_auth: true,
        };
        assert_eq!(req.to_string(), "POST /api/ingest");
    }

    #[test]
    fn url_building_strips_trailing_and_leading_slashes() {
        let client = HttpClient::new("https://api.example.com/", None);
        let url = client.url("/v1/tasks");
        assert_eq!(url, "https://api.example.com/v1/tasks");
    }

    #[test]
    fn resolve_token_returns_override_when_set() {
        let token = "test-token-abc123".to_owned();
        let client =
            HttpClient::with_token_override("https://api.example.com", None, token.clone());
        let result = client.resolve_token();
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), token);
    }

    #[test]
    fn test_403_system_access_required_parses_details() {
        let body = r#"{"error":{"code":"SYSTEM_ACCESS_REQUIRED","message":"This system requires approved access.","details":{"email":"pete@example.com","display_name":"Pete Taylor","access_mode":"invite_only","join_request_status":"pending","request_url":"https://temperkb.io/request-access","cli_command":"temper team join --message \"...\""}}}"#;
        let err = map_status_to_error(status(403), body);
        match err {
            ClientError::SystemAccessRequired {
                email, access_mode, ..
            } => {
                assert_eq!(email.as_deref(), Some("pete@example.com"));
                assert_eq!(access_mode, "invite_only");
            }
            other => panic!("expected SystemAccessRequired, got {other:?}"),
        }
    }

    #[test]
    fn test_403_generic_falls_back_to_forbidden() {
        let body = r#"{"error":{"code":"FORBIDDEN","message":"Forbidden"}}"#;
        let err = map_status_to_error(status(403), body);
        assert!(matches!(err, ClientError::Forbidden));
    }

    #[test]
    fn resolve_token_without_override_falls_back_to_auth() {
        // When no override is set, resolve_token delegates to current_token().
        // Both must return the same result regardless of whether auth.json exists.
        let client = HttpClient::new("https://api.example.com", None);
        let from_client = client.resolve_token();
        let from_auth = crate::auth::current_token();
        match (from_client, from_auth) {
            (Ok(a), Ok(b)) => assert_eq!(
                a, b,
                "resolve_token must return the same token as current_token"
            ),
            (Err(_), Err(_)) => {} // both failed — fallback path exercised correctly
            (Ok(_), Err(_)) | (Err(_), Ok(_)) => {
                panic!("resolve_token and current_token must agree when no override is set")
            }
        }
    }
}
