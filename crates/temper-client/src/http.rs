use std::fmt;
use std::sync::Arc;
use std::time::{Duration, Instant};

use reqwest::{
    header::{HeaderValue, AUTHORIZATION},
    Client, RequestBuilder, Response, StatusCode,
};
use secrecy::ExposeSecret;
use serde::de::DeserializeOwned;
use serde::Deserialize;
use serde_json::Value;

use tracing::Instrument;

use crate::auth::TokenStore;
use crate::error::{ClientError, Result};

/// Total attempts (initial request + retries) for safe, idempotent requests
/// that fail transiently. Mirrors the retry convention in `temper-llm`'s
/// providers. Absorbs a Vercel cold-start / Neon compute-resume blip — the
/// failure mode where a cold serverless instance 500s (or never answers) until
/// it warms — without masking persistent server faults.
const MAX_ATTEMPTS: u32 = 3;

/// Base backoff before the first retry, in milliseconds; doubled each
/// subsequent retry (200ms before retry #2, 400ms before retry #3). Short
/// enough not to noticeably slow a genuine failure, long enough to let a cold
/// instance/DB finish warming between attempts.
const RETRY_BASE_DELAY_MS: u64 = 200;

/// Backoff to wait after `after_attempt` (1-indexed) has failed, before the
/// next attempt. Doubles each retry. Pure so the schedule is unit-testable.
fn retry_delay(after_attempt: u32) -> Duration {
    Duration::from_millis(RETRY_BASE_DELAY_MS << (after_attempt - 1))
}

/// Whether a failed request is safe to retry.
///
/// Only **safe** methods (GET/HEAD — no server-side effects) are retried, so a
/// retry can never duplicate a write. Within those, retries fire only on
/// transient failures: a transport error (the cold serverless function never
/// answered) or an HTTP 5xx (e.g. a cold-start / cold-DB-resume 500). Every
/// 4xx, auth, conflict, and rate-limit error is permanent and propagates
/// immediately.
fn should_retry(method: &reqwest::Method, err: &ClientError) -> bool {
    let safe = matches!(*method, reqwest::Method::GET | reqwest::Method::HEAD);
    safe && match err {
        ClientError::Network(_) => true,
        ClientError::Server { status, .. } => *status >= 500,
        _ => false,
    }
}

/// Wraps a `reqwest::Client` with base URL and optional device identity.
///
/// All request methods prepend `base_url` to the given path and inject the
/// `X-Temper-Device-Id` header when a device ID has been set.
///
/// Bearer tokens are resolved through [`TokenStore`] — cloud sessions use
/// `MemoryTokenStore`, local sessions use `DiskTokenStore`. There is no
/// disk fallback inside `resolve_token`; callers that have an explicit
/// token (tests, one-off scripts) use [`HttpClient::with_token_override`].
#[derive(Clone)]
pub struct HttpClient {
    inner: Client,
    base_url: String,
    device_id: Option<String>,
    token_override: Option<String>,
    token_store: Option<Arc<dyn TokenStore>>,
}

impl fmt::Debug for HttpClient {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("HttpClient")
            .field("base_url", &self.base_url)
            .field("device_id", &self.device_id)
            .field("has_token_override", &self.token_override.is_some())
            .field("has_token_store", &self.token_store.is_some())
            .finish()
    }
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
    /// Construct an `HttpClient` with a [`TokenStore`] for bearer-token
    /// resolution. Cloud sessions pass an `Arc<MemoryTokenStore>`; local
    /// sessions pass an `Arc<DiskTokenStore>`. Tests that don't care about
    /// auth can pass `None` and use [`HttpClient::with_token_override`] instead.
    pub fn new(
        base_url: &str,
        device_id: Option<String>,
        token_store: Option<Arc<dyn TokenStore>>,
    ) -> Self {
        let inner = Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .expect("failed to build reqwest client");

        Self {
            inner,
            base_url: base_url.trim_end_matches('/').to_owned(),
            device_id,
            token_override: None,
            token_store,
        }
    }

    /// Construct an `HttpClient` with a fixed token that bypasses the store.
    ///
    /// Intended for testing and scripting contexts, or for client
    /// construction where the token was already resolved upstream (e.g.
    /// `build_client_from` after a successful `store.load()`).
    pub fn with_token_override(base_url: &str, device_id: Option<String>, token: String) -> Self {
        Self {
            token_override: Some(token),
            ..Self::new(base_url, device_id, None)
        }
    }

    /// Return the token to use for authenticated requests.
    ///
    /// Returns the token override if one was set at construction time;
    /// otherwise calls [`TokenStore::load`] on the configured store and
    /// returns the stored access token (or `TokenExpired` / `NotAuthenticated`
    /// as appropriate). Returns `NotAuthenticated` when neither an override
    /// nor a store is configured.
    pub fn resolve_token(&self) -> Result<String> {
        if let Some(tok) = &self.token_override {
            return Ok(tok.clone());
        }
        let store = self
            .token_store
            .as_ref()
            .ok_or(ClientError::NotAuthenticated)?;
        let auth = store.load()?.ok_or(ClientError::NotAuthenticated)?;
        if auth.expires_at <= chrono::Utc::now() {
            return Err(ClientError::TokenExpired);
        }
        Ok(auth.access_token.expose_secret().to_string())
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
            // Inject bearer auth once; the resulting builder is cloned per
            // attempt so retries replay an identical request.
            let req = if let Some(tok) = token {
                let value = HeaderValue::from_str(&format!("Bearer {tok}"))
                    .map_err(|e| ClientError::Other(format!("invalid token header: {e}")))?;
                req.header(AUTHORIZATION, value)
            } else {
                req
            };

            let mut attempt = 0u32;
            loop {
                attempt += 1;
                // Clone so a transient failure can be retried. A non-cloneable
                // (streaming) body can't be replayed — send it directly; such
                // bodies only occur on non-safe methods, which never retry.
                let Some(this_attempt) = req.try_clone() else {
                    return send_once(req).await;
                };
                match send_once(this_attempt).await {
                    Ok(resp) => return Ok(resp),
                    Err(err) => {
                        if attempt < MAX_ATTEMPTS && should_retry(method, &err) {
                            let delay = retry_delay(attempt);
                            tracing::warn!(
                                attempt,
                                max_attempts = MAX_ATTEMPTS,
                                delay_ms = delay.as_millis() as u64,
                                error = %err,
                                "transient request failure, retrying",
                            );
                            tokio::time::sleep(delay).await;
                            continue;
                        }
                        return Err(err);
                    }
                }
            }
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

/// Issue a single attempt of an already-prepared request and map the outcome.
///
/// Records status/latency on the current tracing span and returns the mapped
/// [`ClientError`] for any non-success status. The retry loop in
/// [`HttpClient::send`] calls this once per attempt.
async fn send_once(req: RequestBuilder) -> Result<Response> {
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

/// Maps an HTTP status code and raw response body to a [`ClientError`].
///
/// Extracted as a pure function so it can be unit-tested without network calls.
pub fn map_status_to_error(status: StatusCode, body: &str) -> ClientError {
    match status.as_u16() {
        401 => ClientError::NotAuthenticated,
        403 => {
            if let Some(details) = parse_system_access_details(body) {
                ClientError::SystemAccessRequired(Box::new(temper_core::error::CliAccessDetails {
                    email: details.email,
                    display_name: details.display_name,
                    access_mode: details.access_mode.unwrap_or_else(|| "unknown".to_string()),
                    join_request_status: details.join_request_status,
                    request_url: details.request_url,
                    cli_command: details.cli_command,
                }))
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
        let client = HttpClient::new("https://api.example.com/", None, None);
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
            ClientError::SystemAccessRequired(details) => {
                assert_eq!(details.email.as_deref(), Some("pete@example.com"));
                assert_eq!(details.access_mode, "invite_only");
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
    fn resolve_token_errors_when_no_override_and_no_store() {
        let client = HttpClient::new("https://api.example.com", None, None);
        let err = client
            .resolve_token()
            .expect_err("no override, no store → NotAuthenticated");
        assert!(matches!(err, ClientError::NotAuthenticated));
    }

    fn server_err(status: u16) -> ClientError {
        ClientError::Server {
            status,
            message: "boom".to_owned(),
        }
    }

    #[test]
    fn should_retry_safe_methods_on_5xx() {
        assert!(should_retry(&reqwest::Method::GET, &server_err(500)));
        assert!(should_retry(&reqwest::Method::GET, &server_err(503)));
        assert!(should_retry(&reqwest::Method::HEAD, &server_err(502)));
    }

    #[test]
    fn should_not_retry_safe_methods_below_500() {
        // 422 is mapped to ClientError::Server but is a permanent client error.
        assert!(!should_retry(&reqwest::Method::GET, &server_err(422)));
    }

    #[test]
    fn should_not_retry_safe_methods_on_permanent_errors() {
        assert!(!should_retry(
            &reqwest::Method::GET,
            &ClientError::NotFound {
                resource: "x".to_owned()
            }
        ));
        assert!(!should_retry(
            &reqwest::Method::GET,
            &ClientError::Forbidden
        ));
        assert!(!should_retry(
            &reqwest::Method::GET,
            &ClientError::NotAuthenticated
        ));
        assert!(!should_retry(
            &reqwest::Method::GET,
            &ClientError::RateLimited {
                retry_after: Duration::from_secs(1)
            }
        ));
    }

    #[test]
    fn should_not_retry_unsafe_methods_even_on_5xx() {
        // Retrying a write could duplicate a server-side effect.
        assert!(!should_retry(&reqwest::Method::POST, &server_err(500)));
        assert!(!should_retry(&reqwest::Method::PATCH, &server_err(503)));
        assert!(!should_retry(&reqwest::Method::PUT, &server_err(500)));
        assert!(!should_retry(&reqwest::Method::DELETE, &server_err(500)));
    }

    #[test]
    fn retry_delay_doubles_each_attempt() {
        assert_eq!(retry_delay(1), Duration::from_millis(200));
        assert_eq!(retry_delay(2), Duration::from_millis(400));
    }

    #[test]
    fn resolve_token_uses_store_when_no_override() {
        use crate::auth::{MemoryTokenStore, Provider, StoredAuth, TokenStore};
        let store = MemoryTokenStore::empty();
        store
            .save(&StoredAuth {
                provider: Provider::auth0("test.auth0.com"),
                access_token: "at_from_store".to_string().into(),
                refresh_token: None,
                expires_at: chrono::Utc::now() + chrono::Duration::hours(1),
                profile_id: None,
                device_id: None,
            })
            .unwrap();
        let client = HttpClient::new("https://api.example.com", None, Some(Arc::new(store)));
        assert_eq!(client.resolve_token().unwrap(), "at_from_store");
    }
}
