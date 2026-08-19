# System Access Gate Phase 1: CLI + MCP Surfaces — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Surface the Phase 1 system access gate to CLI and MCP clients via structured error handling and `temper team` commands.

**Architecture:** Two independent slices. Slice 1 enriches the API's 403 response with profile and access context, adds a `SystemAccessRequired` client error variant, renders friendly CLI messages, and propagates non-retryable MCP errors. Slice 2 adds `temper team {join,status,leave}` commands backed by a new `AccessClient` in temper-client.

**Tech Stack:** Rust (axum, clap, reqwest, rmcp), PostgreSQL, temper workspace crates

**Spec:** `docs/superpowers/specs/2026-04-07-system-access-gate-phase1-cli-mcp-design.md`

---

## Project Fundamentals

- **Typed structs over inline JSON** — never use `serde_json::json!()` for data with a known structure
- **Service layer owns SQL** — all SQL lives in `temper-api/src/services/`, never inline in handlers or MCP tools
- **Params structs** — functions with more than 5 domain-related parameters get a params struct
- **Auth before writes** — authorization checks go before any mutations
- **SQL macros** — use `sqlx::query!()` / `sqlx::query_as!()` for compile-time verification
- **Subagents must use superpowers skills** — TDD for implementation, run `cargo make check` before claiming done

## Subagent Guidance (include verbatim in all subagent prompts)

- **SG-1: Follow Existing Patterns** — Read the file you're modifying AND a sibling in the same module. Match style.
- **SG-2: Single Responsibility** — Each function does one thing.
- **SG-3: No Logic Duplication** — Extract only if two implementations would drift independently.
- **SG-4: Test Strategy** — Unit tests co-located. Integration tests separate. One behavior per test. Tests must actually run.
- **SG-5: Don't Over-Build** — Implement exactly what the task says.
- **SG-6: Verify Before Claiming Done** — Run the verification command. Read the output.
- **SG-7: Prefer Native Solutions** — Use framework/platform tools over hand-rolled alternatives.
- **SG-8: Front-Load Constraints** — Check existing abstractions and platform limits before writing code.
- **SG-9: Don't Dismiss Owned Failures** — Debug the full stack.
- **SG-10: Checkpoint Before Continuing** — Report what's done, what's next, any concerns.

---

## File Map

### Slice 1: Structured Error Path
| Action | File | Responsibility |
|--------|------|---------------|
| Modify | `crates/temper-core/src/types/access_gate.rs` | Add `SystemAccessDetails` struct |
| Modify | `crates/temper-api/src/error.rs` | Extend `SystemAccessRequired` to carry details |
| Modify | `crates/temper-api/src/middleware/system_access.rs` | Populate details from profile + access service |
| Modify | `crates/temper-client/src/error.rs` | Add `SystemAccessRequired` variant |
| Modify | `crates/temper-client/src/http.rs` | Parse enriched 403 into new variant |
| Modify | `crates/temper-core/src/error.rs` | Add `SystemAccessRequired` variant to `TemperError` |
| Modify | `crates/temper-mcp/src/service.rs` | Add system access check after profile resolution |

### Slice 2: Team Commands + AccessClient
| Action | File | Responsibility |
|--------|------|---------------|
| Create | `crates/temper-client/src/access.rs` | `AccessClient` sub-client |
| Modify | `crates/temper-client/src/lib.rs` | Expose `client.access()` |
| Modify | `crates/temper-cli/src/cli.rs` | Add `TeamAction` enum and `Commands::Team` |
| Create | `crates/temper-cli/src/commands/team.rs` | Team command implementations |
| Modify | `crates/temper-cli/src/commands/mod.rs` | Register `team` module |
| Modify | `crates/temper-cli/src/main.rs` | Wire `Commands::Team` dispatch |

---

## Task 1: Add SystemAccessDetails to temper-core

**Files:**
- Modify: `crates/temper-core/src/types/access_gate.rs` (after line 96)

- [ ] **Step 1: Add `SystemAccessDetails` struct**

Add to the end of `crates/temper-core/src/types/access_gate.rs`:

```rust
/// Details included in the SystemAccessRequired error response.
///
/// SECURITY NOTE: The `email` and `display_name` fields are safe to include
/// because the caller already proved ownership of this identity through OAuth.
/// We are reflecting the caller's own profile back — not disclosing another
/// user's information. Do not add fields that reveal other users' data.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SystemAccessDetails {
    pub email: Option<String>,
    pub display_name: Option<String>,
    pub access_mode: String,
    pub join_request_status: Option<JoinRequestStatus>,
    pub request_url: Option<String>,
    pub cli_command: Option<String>,
}
```

- [ ] **Step 2: Verify it compiles**

Run: `cargo check -p temper-core`
Expected: compiles with no errors

- [ ] **Step 3: Commit**

```bash
git add crates/temper-core/src/types/access_gate.rs
git commit -m "feat(core): add SystemAccessDetails struct for enriched 403 responses"
```

---

## Task 2: Enrich the API's SystemAccessRequired error with details

**Files:**
- Modify: `crates/temper-api/src/error.rs` (lines 14-15, 43, 50-53, 79-82)
- Modify: `crates/temper-api/src/middleware/system_access.rs` (lines 25-44)

- [ ] **Step 1: Read both files and the access_service to understand current patterns**

Read:
- `crates/temper-api/src/error.rs` — current `SystemAccessRequired` variant and `IntoResponse`
- `crates/temper-api/src/middleware/system_access.rs` — current middleware
- `crates/temper-api/src/services/access_service.rs` — `get_own_request` and `get_public_settings`

- [ ] **Step 2: Update `ApiError::SystemAccessRequired` to carry details**

In `crates/temper-api/src/error.rs`, change the variant from:

```rust
#[error("System access required")]
SystemAccessRequired,
```

to:

```rust
#[error("System access required")]
SystemAccessRequired {
    details: temper_core::types::access_gate::SystemAccessDetails,
},
```

- [ ] **Step 3: Update `IntoResponse` to serialize details**

In the `IntoResponse` impl, update the `ErrorBody`/`ErrorDetail` structure to support an optional `details` field. Add a `details` field to `ErrorDetail`:

```rust
#[derive(Serialize, ToSchema)]
pub(crate) struct ErrorDetail {
    code: &'static str,
    message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    details: Option<serde_json::Value>,
}
```

In the match arm for `SystemAccessRequired`, serialize the details:

```rust
ApiError::SystemAccessRequired { details } => {
    "This system requires approved access.".to_string()
}
```

And when constructing `ErrorBody`, add the details:

```rust
let details_json = match &self {
    ApiError::SystemAccessRequired { details } => {
        Some(serde_json::to_value(details).unwrap_or_default())
    }
    _ => None,
};

let body = ErrorBody {
    error: ErrorDetail { code, message, details: details_json },
};
```

- [ ] **Step 4: Update the middleware to build SystemAccessDetails**

In `crates/temper-api/src/middleware/system_access.rs`, the middleware already has the profile and access to the pool. Populate the details:

```rust
pub async fn require_system_access(
    State(state): State<AppState>,
    request: Request<Body>,
    next: Next,
) -> Result<Response, ApiError> {
    let profile = request
        .extensions()
        .get::<AuthenticatedProfile>()
        .ok_or_else(|| {
            ApiError::Internal("AuthenticatedProfile not found in request extensions".to_string())
        })?;

    let has_access = access_service::has_system_access(&state.pool, profile.profile.id).await?;

    if !has_access {
        let settings = access_service::get_public_settings(&state.pool).await?;
        let own_request = access_service::get_own_request(&state.pool, profile.profile.id).await?;

        // SECURITY NOTE: email and display_name are safe to return here because
        // the caller already proved ownership of this identity through OAuth.
        // We are reflecting their own profile data back to them.
        let details = temper_core::types::access_gate::SystemAccessDetails {
            email: profile.profile.email.clone(),
            display_name: Some(profile.profile.display_name.clone()),
            access_mode: settings.access_mode,
            join_request_status: own_request.map(|r| r.status),
            request_url: Some("https://temperkb.io/request-access".to_string()),
            cli_command: Some("temper team join --message \"...\"".to_string()),
        };
        return Err(ApiError::SystemAccessRequired { details });
    }

    Ok(next.run(request).await)
}
```

- [ ] **Step 5: Fix any compilation errors from the variant change**

The variant changed from unit to struct — find any existing `ApiError::SystemAccessRequired` references:

Run: `cargo check -p temper-api 2>&1 | head -40`

Fix any remaining pattern match errors (e.g. in the `IntoResponse` impl pattern matching or tests). The logging arm needs updating too:

```rust
ApiError::SystemAccessRequired { .. } => {
    tracing::info!(status_code, error_code = code, "system access required");
}
```

- [ ] **Step 6: Run tests**

Run: `cargo nextest run -p temper-api --workspace 2>&1 | tail -20`
Expected: all existing tests pass (e2e tests will validate the enriched response)

- [ ] **Step 7: Commit**

```bash
git add crates/temper-api/src/error.rs crates/temper-api/src/middleware/system_access.rs
git commit -m "feat(api): enrich SystemAccessRequired 403 with profile and access details"
```

---

## Task 3: Add SystemAccessRequired to temper-client error and HTTP parsing

**Files:**
- Modify: `crates/temper-client/src/error.rs` (after line 12)
- Modify: `crates/temper-client/src/http.rs` (lines 191-227, the `map_status_to_error` function)

- [ ] **Step 1: Read both files and the existing test patterns in http.rs**

Read:
- `crates/temper-client/src/error.rs` — full file
- `crates/temper-client/src/http.rs` — `map_status_to_error` function and its tests

- [ ] **Step 2: Write the failing test first**

In `crates/temper-client/src/http.rs`, add to the existing `#[cfg(test)] mod tests`:

```rust
#[test]
fn test_403_system_access_required_parses_details() {
    let body = r#"{"error":{"code":"SYSTEM_ACCESS_REQUIRED","message":"This system requires approved access.","details":{"email":"pete@example.com","display_name":"Pete Taylor","access_mode":"invite_only","join_request_status":"pending","request_url":"https://temperkb.io/request-access","cli_command":"temper team join --message \"...\""}}}"#;
    let err = map_status_to_error(status(403), body);
    match err {
        ClientError::SystemAccessRequired { email, access_mode, .. } => {
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
```

- [ ] **Step 3: Run tests to verify they fail**

Run: `cargo nextest run -p temper-client test_403 2>&1`
Expected: FAIL — `SystemAccessRequired` variant doesn't exist yet

- [ ] **Step 4: Add `SystemAccessRequired` variant to `ClientError`**

In `crates/temper-client/src/error.rs`, add after the `Forbidden` variant:

```rust
#[error("system access required")]
SystemAccessRequired {
    email: Option<String>,
    display_name: Option<String>,
    access_mode: String,
    join_request_status: Option<String>,
    request_url: Option<String>,
    cli_command: Option<String>,
},
```

- [ ] **Step 5: Update `map_status_to_error` to parse enriched 403**

In `crates/temper-client/src/http.rs`, add a helper struct for parsing the details and update the 403 branch:

```rust
/// Details from a SystemAccessRequired 403 response.
#[derive(Deserialize)]
struct SystemAccessErrorDetails {
    email: Option<String>,
    display_name: Option<String>,
    access_mode: Option<String>,
    join_request_status: Option<String>,
    request_url: Option<String>,
    cli_command: Option<String>,
}

/// Try to parse SystemAccessRequired details from a 403 response body.
fn parse_system_access_details(body: &str) -> Option<SystemAccessErrorDetails> {
    let v: Value = serde_json::from_str(body).ok()?;
    let code = v.get("error")?.get("code")?.as_str()?;
    if code != "SYSTEM_ACCESS_REQUIRED" {
        return None;
    }
    let details = v.get("error")?.get("details")?;
    serde_json::from_value(details.clone()).ok()
}
```

Update the 403 branch in `map_status_to_error`:

```rust
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
```

Add `use serde::Deserialize;` to the imports if not already present.

- [ ] **Step 6: Run tests to verify they pass**

Run: `cargo nextest run -p temper-client test_403 2>&1`
Expected: both tests PASS

- [ ] **Step 7: Commit**

```bash
git add crates/temper-client/src/error.rs crates/temper-client/src/http.rs
git commit -m "feat(client): parse SystemAccessRequired from enriched 403 responses"
```

---

## Task 4: Add SystemAccessRequired to TemperError and render friendly CLI message

**Files:**
- Modify: `crates/temper-core/src/error.rs` (add variant)
- Modify: `crates/temper-cli/src/main.rs` (add error interception in the `run` function's error path)

- [ ] **Step 1: Read the current error handling flow**

Read:
- `crates/temper-core/src/error.rs` — the `TemperError` enum
- `crates/temper-cli/src/main.rs` — specifically lines 14-20 where errors are rendered

- [ ] **Step 2: Add `SystemAccessRequired` variant to `TemperError`**

In `crates/temper-core/src/error.rs`, add a new variant (the `#[error]` message is a fallback — the CLI will render a richer message):

```rust
#[error("system access required")]
SystemAccessRequired {
    email: Option<String>,
    display_name: Option<String>,
    access_mode: String,
    join_request_status: Option<String>,
    request_url: Option<String>,
    cli_command: Option<String>,
},
```

- [ ] **Step 3: Add a helper to convert `ClientError::SystemAccessRequired` to `TemperError::SystemAccessRequired`**

Create a `From<temper_client::error::ClientError>` impl for `TemperError` in `crates/temper-core/src/error.rs`, or add a helper function. Since `temper-core` doesn't depend on `temper-client`, the conversion belongs in the CLI crate instead.

Add a helper function in `crates/temper-cli/src/commands/mod.rs` (or a new `crates/temper-cli/src/client_error.rs`):

```rust
/// Convert a ClientError to a TemperError, preserving SystemAccessRequired details.
pub fn client_err(e: temper_client::error::ClientError) -> crate::error::TemperError {
    match e {
        temper_client::error::ClientError::SystemAccessRequired {
            email,
            display_name,
            access_mode,
            join_request_status,
            request_url,
            cli_command,
        } => crate::error::TemperError::SystemAccessRequired {
            email,
            display_name,
            access_mode,
            join_request_status,
            request_url,
            cli_command,
        },
        other => crate::error::TemperError::Api(other.to_string()),
    }
}
```

- [ ] **Step 4: Update CLI error rendering in main.rs**

In `crates/temper-cli/src/main.rs`, update the error handling in `main()` to render a rich message for `SystemAccessRequired`:

```rust
fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "warn".into()),
        )
        .init();

    let cli = Cli::parse();

    if let Err(e) = run(cli) {
        match &e {
            temper_cli::error::TemperError::SystemAccessRequired {
                email,
                display_name: _,
                access_mode: _,
                join_request_status,
                request_url,
                cli_command,
            } => {
                render_system_access_required(
                    email.as_deref(),
                    join_request_status.as_deref(),
                    request_url.as_deref(),
                    cli_command.as_deref(),
                );
            }
            _ => {
                temper_cli::output::error(format!("temper: {e}"));
            }
        }
        std::process::exit(1);
    }
}

fn render_system_access_required(
    email: Option<&str>,
    join_request_status: Option<&str>,
    request_url: Option<&str>,
    cli_command: Option<&str>,
) {
    use temper_cli::output;

    let identity = email.unwrap_or("your account");
    output::error(format!(
        "You're signed in as {identity}, but this temper instance\n  requires approved access."
    ));
    output::blank();

    match join_request_status {
        Some("pending") => {
            output::plain("  Your access request is pending review.");
            output::hint("  Run `temper team status` to check for updates.");
        }
        Some("rejected") => {
            output::plain("  Your previous request was not approved. You can submit a new one:");
            if let Some(cmd) = cli_command {
                output::hint(format!("    {cmd}"));
            }
        }
        Some("withdrawn") => {
            output::plain("  You withdrew your previous request. Submit a new one:");
            if let Some(cmd) = cli_command {
                output::hint(format!("    {cmd}"));
            }
        }
        _ => {
            output::plain("  To request access, run:");
            if let Some(cmd) = cli_command {
                output::hint(format!("    {cmd}"));
            }
            if let Some(url) = request_url {
                output::blank();
                output::plain(format!("  Or visit: {url}"));
            }
        }
    }
}
```

- [ ] **Step 5: Verify it compiles**

Run: `cargo check -p temper-cli`
Expected: compiles with no errors

- [ ] **Step 6: Commit**

```bash
git add crates/temper-core/src/error.rs crates/temper-cli/src/main.rs crates/temper-cli/src/commands/mod.rs
git commit -m "feat(cli): render friendly message for SystemAccessRequired errors"
```

---

## Task 5: Add system access check to MCP service

**Files:**
- Modify: `crates/temper-mcp/src/service.rs` (the `ensure_profile_from_parts` method, around line 75-94)

- [ ] **Step 1: Read the current MCP service and understand the tool dispatch flow**

Read:
- `crates/temper-mcp/src/service.rs` — `ensure_profile_from_parts` and a couple tool handlers
- `crates/temper-mcp/src/router.rs` — confirm no system access middleware exists

- [ ] **Step 2: Add system access check in `ensure_profile_from_parts`**

This is the natural place — every tool handler calls `ensure_profile_from_parts` before doing work. After profile resolution succeeds, check system access:

```rust
pub async fn ensure_profile_from_parts(
    &self,
    parts: &http::request::Parts,
) -> Result<(), rmcp::ErrorData> {
    let claims = parts.extensions.get::<McpClaims>().ok_or_else(|| {
        tracing::warn!("McpClaims not found in HTTP request extensions");
        rmcp::ErrorData::internal_error("Not authenticated".to_string(), None)
    })?;

    let profile = self.resolve_profile(claims).await?;
    tracing::debug!(
        profile_id = %profile.id,
        sub = %claims.sub,
        "Profile resolved from request"
    );

    // Check system access before allowing any tool use.
    let has_access =
        temper_api::services::access_service::has_system_access(&self.api_state.pool, profile.id)
            .await
            .map_err(|e| {
                rmcp::ErrorData::internal_error(
                    format!("Failed to check system access: {e}"),
                    None,
                )
            })?;

    if !has_access {
        return Err(rmcp::ErrorData::new(
            rmcp::model::ErrorCode::INVALID_REQUEST,
            "Access to this temper instance requires approval. \
             Visit https://temperkb.io/request-access or run \
             `temper team join` in the CLI to request access. \
             This error is terminal and should not be retried."
                .to_string(),
            None,
        ));
    }

    let mut guard = self.profile.lock().await;
    *guard = Some(profile);
    Ok(())
}
```

- [ ] **Step 3: Verify it compiles**

Run: `cargo check -p temper-mcp`
Expected: compiles with no errors

- [ ] **Step 4: Commit**

```bash
git add crates/temper-mcp/src/service.rs
git commit -m "feat(mcp): add system access check with non-retryable error message"
```

---

## Task 6: Add AccessClient to temper-client

**Files:**
- Create: `crates/temper-client/src/access.rs`
- Modify: `crates/temper-client/src/lib.rs` (add `pub mod access;` and `client.access()` method)

- [ ] **Step 1: Read the ProfileClient as the pattern to follow**

Read:
- `crates/temper-client/src/profile.rs` — full file (the sub-client pattern)
- `crates/temper-client/src/lib.rs` — how `profile()` is exposed

- [ ] **Step 2: Create `access.rs` following the ProfileClient pattern**

Create `crates/temper-client/src/access.rs`:

```rust
//! Typed sub-client for the `/api/access` endpoints.

use reqwest::Method;

use crate::error::Result;
use crate::http::HttpClient;
use temper_core::types::access_gate::{JoinRequest, PublicSystemSettings};

/// Request body for creating a join request.
#[derive(serde::Serialize)]
struct CreateRequestBody<'a> {
    message: Option<&'a str>,
    source: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    accepted_terms_version: Option<&'a str>,
}

/// Sub-client for system access operations.
pub struct AccessClient<'a> {
    http: &'a HttpClient,
}

impl std::fmt::Debug for AccessClient<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AccessClient").finish_non_exhaustive()
    }
}

impl<'a> AccessClient<'a> {
    pub(crate) fn new(http: &'a HttpClient) -> Self {
        Self { http }
    }

    /// Submit a join request for the system gating team.
    pub async fn create_request(
        &self,
        message: Option<&str>,
        source: &str,
        accepted_terms_version: Option<&str>,
    ) -> Result<JoinRequest> {
        let token = self.http.resolve_token()?;
        let body = CreateRequestBody {
            message,
            source,
            accepted_terms_version,
        };
        let req = self.http.post("/api/access/requests").json(&body);
        self.http
            .send_json(&Method::POST, "/api/access/requests", req, Some(&token))
            .await
    }

    /// Get the caller's most recent join request (if any).
    pub async fn get_own_request(&self) -> Result<Option<JoinRequest>> {
        let token = self.http.resolve_token()?;
        let req = self.http.get("/api/access/requests/me");
        self.http
            .send_json(&Method::GET, "/api/access/requests/me", req, Some(&token))
            .await
    }

    /// Withdraw a pending join request.
    pub async fn withdraw_request(&self) -> Result<()> {
        let token = self.http.resolve_token()?;
        let req = self.http.delete("/api/access/requests/me");
        self.http
            .send(&Method::DELETE, "/api/access/requests/me", req, Some(&token))
            .await?;
        Ok(())
    }

    /// Get the public system settings (access mode, terms info).
    pub async fn get_settings(&self) -> Result<PublicSystemSettings> {
        let token = self.http.resolve_token()?;
        let req = self.http.get("/api/access/settings");
        self.http
            .send_json(&Method::GET, "/api/access/settings", req, Some(&token))
            .await
    }
}
```

- [ ] **Step 3: Add Deserialize to PublicSystemSettings in temper-core**

In `crates/temper-core/src/types/access_gate.rs`, add `Deserialize` to `PublicSystemSettings`:

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PublicSystemSettings {
```

- [ ] **Step 4: Expose the AccessClient from TemperClient**

In `crates/temper-client/src/lib.rs`, add `pub mod access;` to the module declarations. Then add the accessor method to `TemperClient`:

```rust
pub fn access(&self) -> access::AccessClient<'_> {
    access::AccessClient::new(&self.http)
}
```

- [ ] **Step 5: Verify it compiles**

Run: `cargo check -p temper-client`
Expected: compiles with no errors

- [ ] **Step 6: Commit**

```bash
git add crates/temper-client/src/access.rs crates/temper-client/src/lib.rs crates/temper-core/src/types/access_gate.rs
git commit -m "feat(client): add AccessClient sub-client for system access endpoints"
```

---

## Task 7: Add `temper team` CLI commands

**Files:**
- Modify: `crates/temper-cli/src/cli.rs` (add `TeamAction` enum and `Commands::Team`)
- Create: `crates/temper-cli/src/commands/team.rs`
- Modify: `crates/temper-cli/src/commands/mod.rs` (add `pub mod team;`)
- Modify: `crates/temper-cli/src/main.rs` (add `Commands::Team` dispatch)

- [ ] **Step 1: Read the existing command patterns**

Read:
- `crates/temper-cli/src/cli.rs` — `Commands` enum and `AuthAction` as the pattern
- `crates/temper-cli/src/commands/auth.rs` — how commands call client methods
- `crates/temper-cli/src/main.rs` — how `Commands::Auth` is dispatched (lines 205-212)
- `crates/temper-cli/src/actions/runtime.rs` — `with_client` pattern

- [ ] **Step 2: Add `TeamAction` enum and `Commands::Team` to cli.rs**

In `crates/temper-cli/src/cli.rs`, add the import of `TeamAction` to the use statements in `main.rs` later. First, add the enum and command variant:

Add `Commands::Team` after the `Auth` variant:

```rust
/// Manage team membership and access
Team {
    #[command(subcommand)]
    action: TeamAction,
},
```

Add the `TeamAction` enum after `AuthAction`:

```rust
#[derive(Subcommand)]
pub enum TeamAction {
    /// Request to join a team (defaults to system access)
    Join {
        /// Team slug (default: system gating team)
        #[arg(long)]
        team: Option<String>,
        /// Message for the admin reviewing your request
        #[arg(long)]
        message: Option<String>,
    },
    /// Check your request or membership status
    Status {
        /// Team slug (default: system gating team)
        #[arg(long)]
        team: Option<String>,
    },
    /// Withdraw a pending request or leave a team
    Leave {
        /// Team slug (default: system gating team)
        #[arg(long)]
        team: Option<String>,
    },
}
```

- [ ] **Step 3: Create `commands/team.rs`**

Create `crates/temper-cli/src/commands/team.rs`:

```rust
//! Team membership commands: join, status, leave.

use crate::output;

use temper_core::types::access_gate::JoinRequestStatus;

/// Submit a join request for a team (defaults to system gating team).
pub fn join(message: Option<&str>) -> crate::error::Result<()> {
    crate::actions::runtime::with_client(|client| {
        Box::pin(async move {
            let result = client
                .access()
                .create_request(message, "cli", None)
                .await
                .map_err(crate::commands::client_err)?;

            output::success("Access request submitted.");
            output::plain("  You'll gain access once an admin approves your request.");
            output::hint("  Run `temper team status` to check.");
            output::blank();
            output::dim(format!("  Request ID: {}", result.id));

            Ok(())
        })
    })
}

/// Check the status of the caller's join request.
pub fn status() -> crate::error::Result<()> {
    crate::actions::runtime::with_client(|client| {
        Box::pin(async move {
            let request = client
                .access()
                .get_own_request()
                .await
                .map_err(crate::commands::client_err)?;

            match request {
                None => {
                    output::plain("You haven't requested access yet.");
                    output::hint("Run `temper team join` to get started.");
                }
                Some(req) => match req.status {
                    JoinRequestStatus::Pending => {
                        output::plain(format!(
                            "Your request is pending review (submitted {}).",
                            req.created.format("%Y-%m-%d")
                        ));
                    }
                    JoinRequestStatus::Approved => {
                        let reviewed = req
                            .reviewed_at
                            .map(|d| d.format("%Y-%m-%d").to_string())
                            .unwrap_or_else(|| "unknown date".to_string());
                        output::success(format!("You have access. Approved on {reviewed}."));
                    }
                    JoinRequestStatus::Rejected => {
                        output::warning("Your previous request was not approved.");
                        output::hint(
                            "You can submit a new one with `temper team join --message \"...\"`.",
                        );
                    }
                    JoinRequestStatus::Withdrawn => {
                        output::plain("You withdrew your request.");
                        output::hint(
                            "Submit a new one with `temper team join --message \"...\"`.",
                        );
                    }
                },
            }

            Ok(())
        })
    })
}

/// Withdraw a pending request or leave a team.
pub fn leave() -> crate::error::Result<()> {
    crate::actions::runtime::with_client(|client| {
        Box::pin(async move {
            // Check current request state first
            let request = client
                .access()
                .get_own_request()
                .await
                .map_err(crate::commands::client_err)?;

            match request {
                None => {
                    output::plain("Nothing to leave — you don't have a pending request.");
                }
                Some(req) => match req.status {
                    JoinRequestStatus::Pending => {
                        client
                            .access()
                            .withdraw_request()
                            .await
                            .map_err(crate::commands::client_err)?;
                        output::success("Request withdrawn.");
                    }
                    JoinRequestStatus::Approved => {
                        output::plain(
                            "To leave a team after approval, contact an admin.",
                        );
                    }
                    _ => {
                        output::plain("Nothing to leave — no active request or membership.");
                    }
                },
            }

            Ok(())
        })
    })
}
```

- [ ] **Step 4: Register the module and wire dispatch**

In `crates/temper-cli/src/commands/mod.rs`, add:

```rust
pub mod team;
```

In `crates/temper-cli/src/main.rs`, add `TeamAction` to the import from `temper_cli::cli`:

```rust
use temper_cli::cli::{
    AuthAction, Cli, Commands, ContextAction, DoctorAction, ResourceAction, SkillAction,
    SyncAction, TeamAction,
};
```

Add the dispatch arm in the `match cli.command` block (after Auth):

```rust
Commands::Team { action } => match action {
    TeamAction::Join { team: _, message } => {
        temper_cli::commands::team::join(message.as_deref())
    }
    TeamAction::Status { team: _ } => temper_cli::commands::team::status(),
    TeamAction::Leave { team: _ } => temper_cli::commands::team::leave(),
},
```

Note: the `team` parameter is accepted but ignored for now — all operations target the system gating team. The parameter exists so the CLI interface is forward-compatible with team-specific operations.

- [ ] **Step 5: Verify it compiles**

Run: `cargo check -p temper-cli`
Expected: compiles with no errors

- [ ] **Step 6: Verify the help output looks right**

Run: `cargo run -p temper-cli -- team --help`
Expected: shows `join`, `status`, `leave` subcommands with descriptions

Run: `cargo run -p temper-cli -- team join --help`
Expected: shows `--team` and `--message` options

- [ ] **Step 7: Commit**

```bash
git add crates/temper-cli/src/cli.rs crates/temper-cli/src/commands/team.rs crates/temper-cli/src/commands/mod.rs crates/temper-cli/src/main.rs
git commit -m "feat(cli): add temper team {join,status,leave} commands"
```

---

## Task 8: Update existing CLI commands to use `client_err` helper

**Files:**
- Modify: select files in `crates/temper-cli/src/commands/` and `crates/temper-cli/src/actions/`

This task updates high-traffic code paths to use the `client_err` helper from Task 4 instead of the generic `.map_err(|e| TemperError::Api(e.to_string()))`. This ensures `SystemAccessRequired` is preserved through any command that hits the API (sync, search, add, pull, etc).

- [ ] **Step 1: Identify the most critical paths**

The most important are commands that hit the gated router — these are the ones that will actually receive a `SystemAccessRequired` response:
- `crates/temper-cli/src/actions/sync.rs` — sync operations hit gated endpoints
- `crates/temper-cli/src/actions/search.rs` — search hits gated endpoints
- `crates/temper-cli/src/commands/add.rs` — add hits gated endpoints
- `crates/temper-cli/src/commands/pull.rs` — pull hits gated endpoints
- `crates/temper-cli/src/commands/remove.rs` — remove hits gated endpoints
- `crates/temper-cli/src/commands/context_cmd.rs` — context create hits gated endpoints

- [ ] **Step 2: Replace `.map_err(|e| TemperError::Api(e.to_string()))` with `.map_err(crate::commands::client_err)`**

In each file listed above, find all `.map_err(|e| TemperError::Api(e.to_string()))` calls where the error originates from a temper-client call and replace with `.map_err(crate::commands::client_err)`.

Note: only replace instances where the error is a `ClientError`. Some `.map_err` calls convert other error types (like `reqwest::Error` in ingest.rs) — leave those as-is.

- [ ] **Step 3: Verify it compiles**

Run: `cargo check -p temper-cli`
Expected: compiles with no errors

- [ ] **Step 4: Commit**

```bash
git add crates/temper-cli/src/
git commit -m "refactor(cli): use client_err helper to preserve SystemAccessRequired in all commands"
```

---

## Task 9: Regenerate sqlx cache and run full verification

**Files:**
- Modify: `.sqlx/` (regenerated cache files)

- [ ] **Step 1: Start docker if not running**

Run: `cargo make docker-up`

- [ ] **Step 2: Regenerate sqlx query cache**

Run: `cargo sqlx prepare --workspace -- --all-features`
Expected: cache regenerated (may show "no changes" if no SQL changed)

- [ ] **Step 3: Run full quality checks**

Run: `cargo make check`
Expected: all formatting, clippy, docs, TypeScript checks pass

- [ ] **Step 4: Run all Rust tests**

Run: `cargo make test`
Expected: all unit tests pass

- [ ] **Step 5: Run integration tests**

Run: `cargo make test-db`
Expected: all integration and e2e tests pass

- [ ] **Step 6: Commit any sqlx cache changes**

```bash
git add .sqlx/
git commit -m "chore: regenerate sqlx query cache for access gate CLI/MCP changes"
```

---

## Task 10: E2E test for structured 403 and team commands

**Files:**
- Modify: `crates/temper-e2e/tests/` (add to existing access gate e2e tests)

- [ ] **Step 1: Read the existing e2e test structure**

Read the existing access gate e2e tests to understand the test setup pattern (invite-only mode, second user, etc).

- [ ] **Step 2: Add e2e test for enriched 403 response**

Add a test that:
1. Enables invite-only mode
2. Creates a second authenticated user (who is NOT approved)
3. Makes an API request that hits the gated router
4. Asserts the 403 response body contains `SYSTEM_ACCESS_REQUIRED` code and `details` with `email`, `access_mode`, and `join_request_status`

- [ ] **Step 3: Add e2e test for MCP system access check**

If the e2e framework supports MCP testing, add a test that verifies an unapproved user gets the terminal error message when calling an MCP tool. If MCP e2e is not set up, skip this and note it.

- [ ] **Step 4: Run the e2e tests**

Run: `cargo nextest run -p temper-e2e --features test-db`
Expected: all tests pass including the new ones

- [ ] **Step 5: Commit**

```bash
git add crates/temper-e2e/
git commit -m "test: add e2e tests for enriched 403 response and MCP access check"
```
