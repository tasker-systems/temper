# Structured Logging and Observability — Design Spec

**Task:** `2026-04-01-structured-logging-and-observability-for-api-and-cli`
**Date:** 2026-04-02
**Scope:** Rust API (temper-api on Vercel), Rust HTTP client (temper-client)
**Approach:** Approach B — auth-aware request spans with full error logging (option C enrichment)

## Problem

The Rust Axum API running on Vercel produces zero visible logs in the Vercel observability panel. The tracing subscriber uses default text format and has no `RUST_LOG` fallback, so nothing appears. When batch imports produce mixed 409 conflicts and 401 auth errors, there is no server-side trace to diagnose root cause.

The temper-client HTTP layer also has no request/response logging, making it impossible to correlate client-side errors with server-side behavior.

## Decisions

- **Always JSON** — both `api/axum.rs` (Vercel) and `main.rs` (local API) emit structured JSON to stdout. No environment-aware format switching.
- **Profile ID in spans** — wherever resolvable, `profile_id` is attached to the request span for customer support tracing.
- **No secrets in logs** — auth token presence is logged (`has_auth: true/false`), never token values. Error messages use sanitized strings already present in `ApiError` variants.
- **CLI tracing unchanged** — `temper-cli` keeps its current text subscriber. CLI log format configuration is a follow-up task.
- **TypeScript/pino out of scope** — follow-up task with surface area evaluation.

## Design

### 1. JSON Tracing Subscriber

**Files:** `api/axum.rs`, `crates/temper-api/src/main.rs`

Switch from:
```rust
tracing_subscriber::fmt()
    .with_env_filter(EnvFilter::from_default_env())
    .init();
```

To:
```rust
tracing_subscriber::fmt()
    .json()
    .with_env_filter(
        EnvFilter::try_from_default_env()
            .unwrap_or_else(|_| EnvFilter::new("info"))
    )
    .init();
```

Key changes:
- `.json()` — structured JSON output that Vercel's log collector captures
- Fallback to `info` level when `RUST_LOG` is not set (Vercel environment likely doesn't have it)
- Both binaries use identical initialization

### 2. Enriched TraceLayer

**File:** `crates/temper-api/src/routes.rs`

Replace `TraceLayer::new_for_http()` with a customized version:

**`make_span_from`** — creates a span per request with:
- `method` (GET, POST, etc.)
- `path` (request URI path)
- `version` (HTTP version)
- `profile_id` (`tracing::field::Empty` — filled by auth middleware)

**`on_response`** — logs on every response:
- `status` (numeric status code)
- `latency_ms` (request duration)
- Level: `info`

**`on_failure`** — logs on server errors / connection drops:
- Level: `error`

The span is created before auth runs. Auth enriches it with `profile_id` after profile resolution. Every log line within the request (handler, service, error conversion) automatically carries all span fields.

### 3. Auth Middleware Span Enrichment

**File:** `crates/temper-api/src/middleware/auth.rs`

After profile resolution succeeds (~line 110), inject `profile_id` into the current span:

```rust
tracing::Span::current().record(
    "profile_id",
    tracing::field::display(profile.id),
);
```

On auth failure paths (JWT invalid, JWKS unavailable, email missing), the span retains method/path but `profile_id` remains empty — which is itself diagnostic information (the request never authenticated).

### 4. Structured Error Logging in ApiError

**File:** `crates/temper-api/src/error.rs`

Add structured logging in `ApiError::into_response` for all error variants:

| Variant | Level | Rationale |
|---------|-------|-----------|
| `Unauthorized` | `warn` | Expected during auth failures, not a server error |
| `Forbidden` | `warn` | Access control decision |
| `NotFound` | `debug` | Routine, high-volume |
| `BadRequest` | `warn` | Client sent invalid data |
| `Conflict` | `info` | Dedup during import is expected but useful to track |
| `Internal` | `error` | Always a problem |

Each log event includes `status_code` and `error_code` as structured fields. The message is the error's `Display` string. Combined with the request span, every error response carries method, path, profile_id, and latency automatically.

### 5. temper-client HTTP Observability

**File:** `crates/temper-client/src/http.rs`

**Introduce `ApiRequest` struct** (internal to the `http` module, lifetime-bound to the `send` call):

```rust
struct ApiRequest<'a> {
    method: &'a reqwest::Method,
    path: &'a str,
    has_auth: bool,
}

impl fmt::Display for ApiRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} {}", self.method, self.path)
    }
}
```

**Modify `send` and `send_json` signatures** to accept method and path:

```rust
pub async fn send(
    &self,
    method: Method,
    path: &str,
    req: RequestBuilder,
    token: Option<&str>,
) -> Result<Response>
```

Inside `send`:
- Construct `ApiRequest` from parameters
- Open `tracing::debug_span!("http_request", %request, status = Empty, latency_ms = Empty)`
- Measure elapsed time via `Instant::now()`
- Record `status` and `latency_ms` after response
- On error responses: `tracing::warn!` with status and parsed error message
- On success: span closes at `debug` level

**Call site changes** across sub-clients (~15 sites):
```rust
// Before:
let req = self.http.post("/api/ingest").json(payload);
self.http.send_json(req, Some(&token)).await

// After:
let req = self.http.post("/api/ingest").json(payload);
self.http.send_json(Method::POST, "/api/ingest", req, Some(&token)).await
```

## Files Changed

| File | Change |
|------|--------|
| `api/axum.rs` | JSON subscriber with info fallback |
| `crates/temper-api/src/main.rs` | JSON subscriber with info fallback |
| `crates/temper-api/src/routes.rs` | Customized TraceLayer with make_span, on_response, on_failure |
| `crates/temper-api/src/middleware/auth.rs` | Span::current().record("profile_id", ...) |
| `crates/temper-api/src/error.rs` | Structured logging in into_response |
| `crates/temper-client/src/http.rs` | ApiRequest struct, instrumented send/send_json |
| `crates/temper-client/src/ingest.rs` | Pass method + path to send_json |
| `crates/temper-client/src/resources.rs` | Pass method + path to send_json |
| `crates/temper-client/src/contexts.rs` | Pass method + path to send_json |
| `crates/temper-client/src/search.rs` | Pass method + path to send_json |
| `crates/temper-client/src/sync.rs` | Pass method + path to send_json |
| `crates/temper-client/src/upload.rs` | Pass method + path to send_json |
| `crates/temper-client/src/events.rs` | Pass method + path to send_json |
| `crates/temper-client/src/profile.rs` | Pass method + path to send_json |

## Dependencies

No new crate dependencies. Everything needed is already in the workspace:
- `tracing` and `tracing-subscriber` (with `env-filter` feature) — workspace deps
- `tower-http` (with `trace` feature) — temper-api dep
- `reqwest::Method` — already available in temper-client

## What This Does NOT Cover

These are explicit follow-up tasks:

### Follow-up 1: CLI Log Format and Config Enhancements
- `--log-format` CLI flag, `TEMPER_LOGGING_FORMAT` env var
- `CliConfig` enum tightening (`log_format`, `progress` as enums instead of strings)
- `validator` crate integration for config validation
- `check` → `doctor` rename with richer reporting
- Scope: `build/small`

### Follow-up 2: TypeScript Surface Area Evaluation and Pino Logging
- **Pre-task:** Evaluate the remaining TypeScript surface area. The only piece currently confirmed to need TypeScript is the blob upload → Vercel Workflow invocation path. Assess whether other TypeScript code (API routes, middleware helpers, workflow steps) should remain TS or migrate to the Rust Axum handler path.
- **Conditional:** If TS code remains, add pino for structured JSON logging across Vercel functions and workflow steps. Log request/response in API routes, auth middleware decisions, and workflow step execution.
- Scope: `plan/medium` (evaluation) → `build/medium` (if pino work needed)

## Verification

After implementation:
1. `cargo check --all-features` — compiles cleanly
2. `cargo clippy --all-features` — no warnings
3. `cargo test -p temper-api -p temper-client` — existing tests pass
4. Manual: deploy to Vercel preview, run `temper import` with a few files, confirm JSON log lines appear in Vercel observability panel with method, path, status, latency, and profile_id
5. Manual: run local API server, make requests, confirm JSON output to stdout
