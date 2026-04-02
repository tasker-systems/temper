# Structured Logging and Observability — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add structured JSON logging to the Rust Axum API (local + Vercel) and HTTP client so that every request produces observable, queryable log lines in the Vercel observability panel with method, path, status, latency, and profile_id.

**Architecture:** JSON tracing subscriber → customized TraceLayer (request spans) → auth middleware enrichment (profile_id) → structured error logging (ApiError). Separately, temper-client gets an `ApiRequest` struct for HTTP observability. No new crates; only adding the `json` feature to `tracing-subscriber`.

**Tech Stack:** tracing, tracing-subscriber (json + env-filter), tower-http (trace), reqwest

---

## Subagent Guidance

> **CRITICAL: Read this section before starting any task.** These patterns have caused repeated failures in past subagent work. Each task below references specific guidance items by number.

### SG-1: Follow Existing Patterns
Before writing any code, read the file you are modifying AND at least one sibling file in the same module. Match the style: naming conventions, import ordering, doc comment format, error handling patterns, visibility modifiers, trait implementations. **Do not invent new patterns.**

### SG-2: Single Responsibility
Each function does one thing. If you find yourself writing a function that (a) constructs something AND (b) performs logic on it AND (c) formats output — split it. The existing codebase follows this: handlers delegate to services, services delegate to DB queries. Follow the same layering.

### SG-3: No Logic Duplication
If two places need the same behavior, extract a shared function. But don't create premature abstractions for things that happen once. The test: would two implementations drift independently over time? If yes, extract. If no, leave them inline.

### SG-4: Test Strategy
Unit tests go in `#[cfg(test)] mod tests` at the bottom of the file being tested. Integration tests go in `crates/<crate>/tests/`. Each test tests ONE behavior with a descriptive name: `test_<what>_<condition>_<expected>` (e.g., `test_api_request_display_formats_method_and_path`). Tests must compile and run — verify by running `cargo test -p <crate> -- <test_name>`.

### SG-5: Don't Over-Build
Implement exactly what the task says. Don't add "nice to have" features, extra error handling for impossible cases, or defensive code "just in case." The spec defines the scope.

### SG-6: Verify Before Claiming Done
After each task, run the specified verification command. If it fails, fix the issue before marking complete. Do not claim a task is done based on what you think the code does — run the command and read the output.

---

## File Structure

| File | Responsibility | Change Type |
|------|----------------|-------------|
| `Cargo.toml` (workspace root) | Workspace dependency declarations | Modify |
| `crates/temper-api/Cargo.toml` | temper-api crate dependencies | Modify |
| `api/axum.rs` | Vercel serverless entry point | Modify |
| `crates/temper-api/src/main.rs` | Local API server entry point | Modify |
| `crates/temper-api/src/routes.rs` | Router + middleware layers | Modify |
| `crates/temper-api/src/middleware/auth.rs` | JWT auth + profile resolution | Modify |
| `crates/temper-api/src/error.rs` | ApiError type + IntoResponse | Modify |
| `crates/temper-client/src/http.rs` | HttpClient + ApiRequest | Modify |
| `crates/temper-client/src/ingest.rs` | Ingest sub-client | Modify |
| `crates/temper-client/src/resources.rs` | Resources sub-client | Modify |
| `crates/temper-client/src/contexts.rs` | Contexts sub-client | Modify |
| `crates/temper-client/src/search.rs` | Search sub-client | Modify |
| `crates/temper-client/src/sync.rs` | Sync sub-client | Modify |
| `crates/temper-client/src/upload.rs` | Upload sub-client | Modify |
| `crates/temper-client/src/events.rs` | Events sub-client | Modify |
| `crates/temper-client/src/profile.rs` | Profile sub-client | Modify |

---

## Task 1: Add `json` feature to `tracing-subscriber`

**Guidance:** SG-1, SG-5

**Files:**
- Modify: `Cargo.toml` (workspace root, line 21)
- Modify: `crates/temper-api/Cargo.toml` (line 22)

- [ ] **Step 1: Update workspace root `Cargo.toml`**

In `Cargo.toml` at line 21, change:
```toml
tracing-subscriber = { version = "0.3", features = ["env-filter"] }
```
to:
```toml
tracing-subscriber = { version = "0.3", features = ["env-filter", "json"] }
```

- [ ] **Step 2: Update temper-api `Cargo.toml`**

In `crates/temper-api/Cargo.toml` at line 22, change:
```toml
tracing-subscriber = { version = "0.3", features = ["env-filter"] }
```
to:
```toml
tracing-subscriber = { version = "0.3", features = ["env-filter", "json"] }
```

- [ ] **Step 3: Verify it compiles**

Run: `cargo check -p temper-api`
Expected: compiles with no errors. The `json` feature enables `tracing_subscriber::fmt().json()`.

- [ ] **Step 4: Commit**

```bash
git add Cargo.toml crates/temper-api/Cargo.toml
git commit -m "build: add json feature to tracing-subscriber for structured logging"
```

---

## Task 2: Switch API entry points to JSON tracing subscriber

**Guidance:** SG-1, SG-3, SG-5

**Files:**
- Modify: `api/axum.rs` (lines 16-18)
- Modify: `crates/temper-api/src/main.rs` (lines 11-14)

Both files currently have nearly identical tracing init code. Both get the same change. This is intentional WET — these are two separate binaries with different deployment targets, and keeping the init inline makes each entry point self-contained.

- [ ] **Step 1: Update `api/axum.rs` (Vercel entry point)**

Replace lines 16-18:
```rust
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .init();
```
with:
```rust
    tracing_subscriber::fmt()
        .json()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();
```

- [ ] **Step 2: Update `crates/temper-api/src/main.rs` (local server)**

Replace lines 12-14:
```rust
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .init();
```
with:
```rust
    tracing_subscriber::fmt()
        .json()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();
```

- [ ] **Step 3: Verify both binaries compile**

Run: `cargo check -p temper-api && cargo check -p temper-cloud`
Expected: both compile cleanly. (temper-cloud is the workspace package that builds `api/axum.rs`.)

- [ ] **Step 4: Commit**

```bash
git add api/axum.rs crates/temper-api/src/main.rs
git commit -m "feat: switch API tracing subscriber to JSON with info-level fallback"
```

---

## Task 3: Customize TraceLayer with request spans

**Guidance:** SG-1, SG-2, SG-4, SG-5

**Files:**
- Modify: `crates/temper-api/src/routes.rs` (lines 4, 66-68)

The existing `TraceLayer::new_for_http()` at line 68 uses defaults. We replace it with a customized version that creates a span per request containing method, path, HTTP version, and a placeholder `profile_id` field (filled later by auth middleware in Task 4).

**Important:** The `TraceLayer` wraps outside the auth middleware layer. This means the span is created BEFORE authentication runs. The `profile_id` field starts empty and is populated by the auth middleware using `Span::current().record()`. This ordering is already correct in the existing code — the `TraceLayer` is applied after the `protected` router's auth layer (outer layers run first in tower/axum).

- [ ] **Step 1: Update imports in `routes.rs`**

Replace the existing `tower_http::trace::TraceLayer` import at line 4:
```rust
use tower_http::trace::TraceLayer;
```
with:
```rust
use tower_http::trace::{DefaultOnFailure, TraceLayer};
```

Also add these imports after the existing `use` block (after line 5):
```rust
use tracing::Span;
use std::time::Duration;
```

- [ ] **Step 2: Replace `TraceLayer::new_for_http()` with customized version**

Replace line 68:
```rust
        .layer(TraceLayer::new_for_http())
```
with:
```rust
        .layer(
            TraceLayer::new_for_http()
                .make_span_with(|request: &axum::extract::Request| {
                    tracing::info_span!(
                        "http_request",
                        method = %request.method(),
                        path = %request.uri().path(),
                        version = ?request.version(),
                        profile_id = tracing::field::Empty,
                    )
                })
                .on_response(
                    |response: &axum::response::Response,
                     latency: Duration,
                     _span: &Span| {
                        tracing::info!(
                            status = response.status().as_u16(),
                            latency_ms = latency.as_millis() as u64,
                            "response",
                        );
                    },
                )
                .on_failure(DefaultOnFailure::new().level(tracing::Level::ERROR)),
        )
```

- [ ] **Step 3: Verify it compiles**

Run: `cargo check -p temper-api`
Expected: compiles cleanly. The `make_span_with` closure creates a span that tower-http populates. The `on_response` closure logs a structured event inside that span.

- [ ] **Step 4: Verify existing tests still pass**

Run: `cargo test -p temper-api`
Expected: all existing tests pass. The TraceLayer change doesn't affect test behavior — tests hit the routes, and tracing just adds structured output.

- [ ] **Step 5: Commit**

```bash
git add crates/temper-api/src/routes.rs
git commit -m "feat: customize TraceLayer with method, path, status, latency, and profile_id span"
```

---

## Task 4: Enrich auth middleware spans with profile_id

**Guidance:** SG-1, SG-2, SG-5

**Files:**
- Modify: `crates/temper-api/src/middleware/auth.rs` (after line 110)

After profile resolution in `require_auth`, we record the profile ID into the current span created by TraceLayer (Task 3). This means every subsequent log event in this request — handler logs, service logs, error logs — automatically carries the `profile_id` field.

- [ ] **Step 1: Add `profile_id` to the current span after profile resolution**

In `crates/temper-api/src/middleware/auth.rs`, find the comment at line 109:
```rust
    // 5. Resolve (or auto-provision) the profile.
    let profile = profile_service::resolve_from_claims(&state.pool, &claims).await?;
```

Add the following IMMEDIATELY after `let profile = ...` and BEFORE the device_id extraction at line 112:
```rust
    tracing::Span::current().record(
        "profile_id",
        tracing::field::display(profile.id),
    );
```

No new imports needed — `tracing` is already imported via the existing `tracing::error!`, `tracing::debug!`, and `tracing::warn!` calls in this file.

- [ ] **Step 2: Verify it compiles**

Run: `cargo check -p temper-api`
Expected: compiles cleanly. `profile.id` is a `Uuid` which implements `Display`, so `tracing::field::display()` works.

- [ ] **Step 3: Verify existing tests still pass**

Run: `cargo test -p temper-api`
Expected: all tests pass. The span recording is additive — it enriches existing spans without changing behavior.

- [ ] **Step 4: Commit**

```bash
git add crates/temper-api/src/middleware/auth.rs
git commit -m "feat: record profile_id in request span after auth resolution"
```

---

## Task 5: Add structured error logging to ApiError

**Guidance:** SG-1, SG-2, SG-4, SG-5

**Files:**
- Modify: `crates/temper-api/src/error.rs` (inside `into_response`, lines 36-52)

Currently `into_response` silently converts errors to HTTP responses. We add a structured log event for every error response. The log levels are intentional:
- `debug` for NotFound (routine, high-volume)
- `info` for Conflict (dedup during import is expected)
- `warn` for Unauthorized, Forbidden, BadRequest (client errors worth knowing)
- `error` for Internal (always a problem)

Because this runs inside the request span from Task 3, every error log automatically carries method, path, profile_id, and latency.

- [ ] **Step 1: Add structured logging to `into_response`**

In `crates/temper-api/src/error.rs`, replace the `into_response` method body (lines 37-52):

```rust
    fn into_response(self) -> Response {
        let (status, code) = match &self {
            ApiError::NotFound => (StatusCode::NOT_FOUND, "NOT_FOUND"),
            ApiError::Unauthorized(_) => (StatusCode::UNAUTHORIZED, "UNAUTHORIZED"),
            ApiError::Forbidden => (StatusCode::FORBIDDEN, "FORBIDDEN"),
            ApiError::BadRequest(_) => (StatusCode::BAD_REQUEST, "BAD_REQUEST"),
            ApiError::Conflict(_) => (StatusCode::CONFLICT, "CONFLICT"),
            ApiError::Internal(_) => (StatusCode::INTERNAL_SERVER_ERROR, "INTERNAL_ERROR"),
        };
        let body = ErrorBody {
            error: ErrorDetail {
                code,
                message: self.to_string(),
            },
        };
        (status, axum::Json(body)).into_response()
    }
```

with:

```rust
    fn into_response(self) -> Response {
        let (status, code) = match &self {
            ApiError::NotFound => (StatusCode::NOT_FOUND, "NOT_FOUND"),
            ApiError::Unauthorized(_) => (StatusCode::UNAUTHORIZED, "UNAUTHORIZED"),
            ApiError::Forbidden => (StatusCode::FORBIDDEN, "FORBIDDEN"),
            ApiError::BadRequest(_) => (StatusCode::BAD_REQUEST, "BAD_REQUEST"),
            ApiError::Conflict(_) => (StatusCode::CONFLICT, "CONFLICT"),
            ApiError::Internal(_) => (StatusCode::INTERNAL_SERVER_ERROR, "INTERNAL_ERROR"),
        };

        let message = self.to_string();
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
            ApiError::BadRequest(_) => {
                tracing::warn!(status_code, error_code = code, %message, "bad request");
            }
            ApiError::Internal(_) => {
                tracing::error!(status_code, error_code = code, %message, "internal error");
            }
        }

        let body = ErrorBody {
            error: ErrorDetail {
                code,
                message,
            },
        };
        (status, axum::Json(body)).into_response()
    }
```

Note: `message` is now bound as a `String` variable (moved from `self.to_string()`) and reused in both the log event and the response body. This avoids calling `to_string()` twice.

- [ ] **Step 2: Verify it compiles**

Run: `cargo check -p temper-api`
Expected: compiles cleanly.

- [ ] **Step 3: Verify existing tests pass**

Run: `cargo test -p temper-api`
Expected: all tests pass. Error responses still return the same status codes and JSON bodies — we've only added logging.

- [ ] **Step 4: Commit**

```bash
git add crates/temper-api/src/error.rs
git commit -m "feat: add structured error logging to ApiError with level-appropriate severity"
```

---

## Task 6: Add `ApiRequest` struct and instrument `HttpClient::send`

**Guidance:** SG-1, SG-2, SG-3, SG-4, SG-5

**Files:**
- Modify: `crates/temper-client/src/http.rs`

This is the largest single task. We add an `ApiRequest` struct (internal to the module) and modify `send` and `send_json` to accept `method` and `path` parameters for observability. The struct is constructed inside `send` — callers just pass the two extra args.

**Important patterns to follow (SG-1):**
- The existing `HttpClient` tests at the bottom of this file use `map_status_to_error` as a pure function. Keep that pattern — test observable behavior, not tracing output.
- The existing `HttpClient` methods (`get`, `post`, etc.) return `RequestBuilder`. Do NOT change their signatures — they are clean factories.

- [ ] **Step 1: Write tests for `ApiRequest::display`**

Add these tests to the existing `#[cfg(test)] mod tests` block at the bottom of `crates/temper-client/src/http.rs`:

```rust
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
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p temper-client -- test_api_request_display`
Expected: FAIL — `ApiRequest` is not yet defined.

- [ ] **Step 3: Implement `ApiRequest` struct**

Add the following ABOVE the `impl HttpClient` block (before line 23) in `crates/temper-client/src/http.rs`:

```rust
use std::fmt;
use std::time::Instant;

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
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p temper-client -- test_api_request_display`
Expected: PASS — both tests should succeed.

- [ ] **Step 5: Update `send` signature and add instrumentation**

Replace the existing `send` method (lines 72-90) with:

```rust
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
            status = tracing::field::Empty,
            latency_ms = tracing::field::Empty,
        );
        let _guard = span.enter();

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

        span.record("status", status.as_u16());
        span.record("latency_ms", latency_ms);

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
```

- [ ] **Step 6: Update `send_json` signature**

Replace the existing `send_json` method (lines 93-102) with:

```rust
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
```

- [ ] **Step 7: Verify it compiles (client crate only)**

Run: `cargo check -p temper-client`
Expected: FAIL — sub-client callers still use the old 2-arg signature. This is expected; we fix them in Task 7.

- [ ] **Step 8: Commit (partial — will not compile workspace-wide until Task 7)**

```bash
git add crates/temper-client/src/http.rs
git commit -m "feat: add ApiRequest struct and instrument HttpClient::send with tracing"
```

---

## Task 7: Update all sub-client call sites

**Guidance:** SG-1, SG-3, SG-4, SG-6

**Files:**
- Modify: `crates/temper-client/src/ingest.rs`
- Modify: `crates/temper-client/src/resources.rs`
- Modify: `crates/temper-client/src/contexts.rs`
- Modify: `crates/temper-client/src/search.rs`
- Modify: `crates/temper-client/src/sync.rs`
- Modify: `crates/temper-client/src/upload.rs`
- Modify: `crates/temper-client/src/events.rs`
- Modify: `crates/temper-client/src/profile.rs`

Every sub-client call to `send_json` or `send` needs two new leading arguments: `&reqwest::Method::XXX` and the path string. The path is already visible at each call site (it's the argument to `self.http.get(...)` etc.).

**Pattern to follow:** Each call site currently looks like:
```rust
let req = self.http.post("/api/ingest").json(payload);
self.http.send_json(req, Some(&token)).await
```

Becomes:
```rust
let req = self.http.post("/api/ingest").json(payload);
self.http.send_json(&reqwest::Method::POST, "/api/ingest", req, Some(&token)).await
```

Add `use reqwest::Method;` at the top of each file to keep call sites clean, then use `&Method::POST` etc.

- [ ] **Step 1: Update `ingest.rs`**

Add import at top:
```rust
use reqwest::Method;
```

Update `create` method:
```rust
    pub async fn create(&self, payload: &IngestPayload) -> Result<ResourceRow> {
        let token = auth::current_token()?;
        let req = self.http.post("/api/ingest").json(payload);
        self.http.send_json(&Method::POST, "/api/ingest", req, Some(&token)).await
    }
```

Update `update` method:
```rust
    pub async fn update(&self, id: Uuid, payload: &IngestPayload) -> Result<ResourceRow> {
        let token = auth::current_token()?;
        let path = format!("/api/ingest/{id}");
        let req = self.http.put(&path).json(payload);
        self.http.send_json(&Method::PUT, &path, req, Some(&token)).await
    }
```

- [ ] **Step 2: Update `resources.rs`**

Add import at top:
```rust
use reqwest::Method;
```

Update all methods. Note: for dynamic paths, bind the path to a variable so it can be passed to both `self.http.get()` and `send_json()`:

```rust
    pub async fn list(&self, params: &ResourceListParams) -> Result<Vec<ResourceRow>> {
        let token = auth::current_token()?;
        let req = self.http.get("/api/resources").query(params);
        self.http.send_json(&Method::GET, "/api/resources", req, Some(&token)).await
    }

    pub async fn get(&self, id: Uuid) -> Result<ResourceRow> {
        let token = auth::current_token()?;
        let path = format!("/api/resources/{id}");
        let req = self.http.get(&path);
        self.http.send_json(&Method::GET, &path, req, Some(&token)).await
    }

    pub async fn create(&self, request: &ResourceCreateRequest) -> Result<ResourceRow> {
        let token = auth::current_token()?;
        let req = self.http.post("/api/resources").json(request);
        self.http.send_json(&Method::POST, "/api/resources", req, Some(&token)).await
    }

    pub async fn update(&self, id: Uuid, request: &ResourceUpdateRequest) -> Result<ResourceRow> {
        let token = auth::current_token()?;
        let path = format!("/api/resources/{id}");
        let req = self.http.patch(&path).json(request);
        self.http.send_json(&Method::PATCH, &path, req, Some(&token)).await
    }

    pub async fn delete(&self, id: Uuid) -> Result<DeleteResponse> {
        let token = auth::current_token()?;
        let path = format!("/api/resources/{id}");
        let req = self.http.delete(&path);
        self.http.send_json(&Method::DELETE, &path, req, Some(&token)).await
    }

    pub async fn content(&self, id: Uuid) -> Result<ContentResponse> {
        let token = auth::current_token()?;
        let path = format!("/api/resources/{id}/content");
        let req = self.http.get(&path);
        self.http.send_json(&Method::GET, &path, req, Some(&token)).await
    }
```

- [ ] **Step 3: Update `contexts.rs`**

Add import at top:
```rust
use reqwest::Method;
```

Update all methods:
```rust
    pub async fn list(&self) -> Result<Vec<ContextRow>> {
        let token = auth::current_token()?;
        let req = self.http.get("/api/contexts");
        self.http.send_json(&Method::GET, "/api/contexts", req, Some(&token)).await
    }

    pub async fn get(&self, id: Uuid) -> Result<ContextRow> {
        let token = auth::current_token()?;
        let path = format!("/api/contexts/{id}");
        let req = self.http.get(&path);
        self.http.send_json(&Method::GET, &path, req, Some(&token)).await
    }

    pub async fn create(&self, name: &str) -> Result<ContextRow> {
        let token = auth::current_token()?;
        let body = ContextCreateRequest {
            name: name.to_owned(),
        };
        let req = self.http.post("/api/contexts").json(&body);
        self.http.send_json(&Method::POST, "/api/contexts", req, Some(&token)).await
    }
```

- [ ] **Step 4: Update `search.rs`**

Add import at top:
```rust
use reqwest::Method;
```

Update `query` method:
```rust
    pub async fn query(
        &self,
        embedding: Vec<f32>,
        context_name: Option<String>,
        doc_type: Option<String>,
        limit: Option<i64>,
    ) -> Result<Vec<SearchResultRow>> {
        let token = auth::current_token()?;
        let params = SearchParams {
            embedding,
            context_name,
            doc_type,
            limit,
        };
        let req = self.http.post("/api/search").json(&params);
        self.http.send_json(&Method::POST, "/api/search", req, Some(&token)).await
    }
```

- [ ] **Step 5: Update `sync.rs`**

Add import at top:
```rust
use reqwest::Method;
```

Update both methods:
```rust
    pub async fn status(&self, request: &SyncStatusRequest) -> Result<SyncStatusResponse> {
        let token = auth::current_token()?;
        let req = self.http.post("/api/sync/status").json(request);
        self.http.send_json(&Method::POST, "/api/sync/status", req, Some(&token)).await
    }

    pub async fn complete(&self, request: &SyncCompleteRequest) -> Result<SyncCompleteResponse> {
        let token = auth::current_token()?;
        let req = self.http.post("/api/sync/complete").json(request);
        self.http.send_json(&Method::POST, "/api/sync/complete", req, Some(&token)).await
    }
```

- [ ] **Step 6: Update `upload.rs`**

Add import at top:
```rust
use reqwest::Method;
```

Update `add` method. Note: `upload.rs` uses `send_json` not `send`, and builds a multipart form:
```rust
    pub async fn add(
        &self,
        resource_id: Uuid,
        content: Vec<u8>,
        filename: &str,
    ) -> Result<UploadResponse> {
        let token = auth::current_token()?;
        let form = reqwest::multipart::Form::new()
            .text("resource_id", resource_id.to_string())
            .part(
                "file",
                reqwest::multipart::Part::bytes(content).file_name(filename.to_string()),
            );
        let req = self.http.post("/api/upload").multipart(form);
        self.http.send_json(&Method::POST, "/api/upload", req, Some(&token)).await
    }
```

- [ ] **Step 7: Update `events.rs`**

Add import at top:
```rust
use reqwest::Method;
```

Update `list` method:
```rust
    pub async fn list(&self, params: &EventListParams) -> Result<Vec<EventRow>> {
        let token = auth::current_token()?;
        let req = self.http.get("/api/events").query(params);
        self.http.send_json(&Method::GET, "/api/events", req, Some(&token)).await
    }
```

- [ ] **Step 8: Update `profile.rs`**

Add import at top:
```rust
use reqwest::Method;
```

Update all methods:
```rust
    pub async fn get(&self) -> Result<Profile> {
        let token = auth::current_token()?;
        let req = self.http.get("/api/profile");
        self.http.send_json(&Method::GET, "/api/profile", req, Some(&token)).await
    }

    pub async fn update(&self, request: &ProfileUpdateRequest) -> Result<Profile> {
        let token = auth::current_token()?;
        let req = self.http.patch("/api/profile").json(request);
        self.http.send_json(&Method::PATCH, "/api/profile", req, Some(&token)).await
    }

    pub async fn auth_links(&self) -> Result<Vec<ProfileAuthLink>> {
        let token = auth::current_token()?;
        let req = self.http.get("/api/profile/auth-links");
        self.http.send_json(&Method::GET, "/api/profile/auth-links", req, Some(&token)).await
    }
```

- [ ] **Step 9: Verify full workspace compiles**

Run: `cargo check --all-features`
Expected: compiles cleanly. All sub-clients now pass method and path to `send`/`send_json`.

- [ ] **Step 10: Run all tests**

Run: `cargo test -p temper-client -p temper-api`
Expected: all tests pass. The `map_status_to_error` tests and `sync_client_is_debug` test are unaffected by the signature change (they don't call `send`/`send_json`).

- [ ] **Step 11: Commit**

```bash
git add crates/temper-client/src/ingest.rs crates/temper-client/src/resources.rs crates/temper-client/src/contexts.rs crates/temper-client/src/search.rs crates/temper-client/src/sync.rs crates/temper-client/src/upload.rs crates/temper-client/src/events.rs crates/temper-client/src/profile.rs
git commit -m "feat: pass method and path to HttpClient::send for request tracing"
```

---

## Task 8: Create follow-up tasks in the knowledge base

**Guidance:** SG-5

**Files:** None (vault operations only)

- [ ] **Step 1: Create CLI config enhancement task**

```bash
cat <<'TASK_EOF' | temper task create --title "CLI log format configuration and config enhancements" --context temper --mode build --effort small
# CLI Log Format Configuration and Config Enhancements

## Scope

- Add `--log-format <text|json>` global CLI flag
- Add `TEMPER_LOGGING_FORMAT` env var support
- Add `log_format` field to `CliConfig` in `temper-core::types::config`
- Convert `CliConfig::progress` from `String` to a proper enum (`Bar` | `Json`)
- Convert `log_format` to enum (`Text` | `Json`)
- Integrate `validator` crate into `temper-core` for config validation
- Rename `check` command to `doctor` with richer health reporting
- Resolution priority: flag > env var > config file > default (`text`)

## Context

Split from structured-logging task to keep scope focused. The API and client now emit JSON logs (2026-04-02 structured-logging task). CLI needs configurable format to match.

## Related

- Design spec: docs/superpowers/specs/2026-04-02-structured-logging-and-observability-design.md
- Current `CliConfig`: crates/temper-core/src/types/config.rs (lines 38-56)
- Current CLI arg parsing: crates/temper-cli/src/cli.rs
- Current tracing init: crates/temper-cli/src/main.rs (lines 11-15)
TASK_EOF
```

- [ ] **Step 2: Create TypeScript surface area evaluation task**

```bash
cat <<'TASK_EOF' | temper task create --title "Evaluate TypeScript surface area and add pino logging" --context temper --mode plan --effort medium
# Evaluate TypeScript Surface Area and Add Pino Logging

## Pre-Task: Surface Area Evaluation

Before adding pino, evaluate whether the remaining TypeScript code is justified. The only piece currently confirmed to need TypeScript is the blob upload → Vercel Workflow invocation path (`api/upload.ts` → `api/workflows/process-upload.ts`).

Assess each remaining TypeScript file:
- `api/upload.ts` — blob upload endpoint (confirmed TS)
- `api/workflows/process-upload.ts` — Vercel durable workflow (confirmed TS)
- `api/workflows/process-ingest.ts` — ingest workflow (may be redundant with Rust ingest)
- `api/auth/cli-callback.ts` — OAuth callback (could this be a Rust handler?)
- `packages/temper-cloud/src/middleware.ts` — auth helpers (duplicates Rust auth?)
- `packages/temper-cloud/src/db.ts` — Neon client init
- `packages/temper-cloud/src/sync.ts` — sync schemas (duplicates Rust sync?)
- `packages/temper-cloud/src/processing/` — chunk, embed, extract (duplicates Rust ingest?)
- `packages/temper-cloud/src/workflow/` — workflow step helpers

For each file: is this actively used? Does it duplicate Rust functionality? Can it be removed or migrated?

## Conditional: Pino Integration

If TypeScript code remains after evaluation, add pino for structured JSON logging:
- Install pino as dependency in packages/temper-cloud
- Create shared logger instance
- Add logging to Vercel function handlers and workflow steps
- Log auth decisions, request/response, processing status

## Context

Split from structured-logging task. Rust API and client now have full observability (2026-04-02). TypeScript may be partially redundant after Rust ingest work.

## Related

- Design spec: docs/superpowers/specs/2026-04-02-structured-logging-and-observability-design.md
- Current TS files: packages/temper-cloud/src/, api/
TASK_EOF
```

- [ ] **Step 3: Commit (no code changes — vault operations only)**

No git commit needed for vault task creation.

---

## Task 9: Final verification

**Guidance:** SG-6

- [ ] **Step 1: Full build check**

Run: `cargo check --all-features`
Expected: clean compilation, no warnings.

- [ ] **Step 2: Clippy**

Run: `cargo clippy --all-features`
Expected: no warnings. If clippy flags anything in the new code, fix it before proceeding.

- [ ] **Step 3: Run all tests**

Run: `cargo test -p temper-api -p temper-client`
Expected: all tests pass.

- [ ] **Step 4: Spot-check JSON output locally**

Run the local API server and confirm JSON output:
```bash
RUST_LOG=info cargo run -p temper-api 2>&1 | head -5
```
Expected: JSON-formatted log lines to stderr, including the startup message with structured fields.

- [ ] **Step 5: Review the diff**

Run: `git log --oneline main..HEAD` to see all commits from this task.
Run: `git diff main..HEAD --stat` to see files changed.

Verify: no files outside the spec's "Files Changed" table were modified. No unexpected dependencies were added.
