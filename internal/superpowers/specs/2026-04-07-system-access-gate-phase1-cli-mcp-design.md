# System Access Gate Phase 1: CLI + MCP Surfaces

**Date:** 2026-04-07
**Task:** 2026-04-07-implement-temper-system-access-gate (Session 2)
**Depends on:** Phase 1 Data + API (Session 1, complete)
**Branch:** jct/temper-system-access-gate

---

## Summary

This spec covers two independent slices that surface the Phase 1 system access gate
to CLI and MCP clients:

1. **Structured error handling** — Enrich the API's 403 response, add a
   `SystemAccessRequired` client error variant, render friendly CLI messages, and
   propagate non-retryable errors through MCP.
2. **`temper team` CLI commands** — `join`, `status`, and `leave` subcommands backed
   by a new `AccessClient` in temper-client.

Both slices are independently shippable. The error handling slice improves every
existing CLI command immediately; the team commands are additive.

---

## Slice 1: Structured SystemAccessRequired Error Path

### API Response Enrichment

The `require_system_access` middleware already has the authenticated profile. When
it returns 403, the response body is enriched with profile and access context:

```json
{
  "error": {
    "code": "SYSTEM_ACCESS_REQUIRED",
    "message": "This system requires approved access.",
    "details": {
      "email": "pete@example.com",
      "display_name": "Pete Taylor",
      "access_mode": "invite_only",
      "join_request_status": "pending",
      "request_url": "https://temperkb.io/request-access",
      "cli_command": "temper team join --message \"...\""
    }
  }
}
```

**Security invariant:** The `email` and `display_name` fields are safe to return
**only** because the caller already proved ownership of this identity through the
OAuth flow. We are reflecting the caller's own profile back to them, not disclosing
another user's information. This invariant must be documented in the code with a
comment at the point of construction.

The `join_request_status` field is null if the user has never requested access, or
one of `pending`, `rejected`, `withdrawn`. The value `approved` never appears here
because an approved user would pass the gate.

Implementation: The middleware needs access to the database pool to look up the
join request status and system settings. It already has the profile from the auth
layer. A new `SystemAccessDetails` struct in temper-core carries the typed fields.
The middleware calls `access_service::get_own_request()` and
`access_service::get_public_settings()` to populate it.

### temper-client: SystemAccessRequired Error Variant

New variant in `ClientError`:

```rust
SystemAccessRequired {
    email: Option<String>,
    display_name: Option<String>,
    access_mode: String,
    join_request_status: Option<String>,
    request_url: Option<String>,
    cli_command: Option<String>,
}
```

Parsed in `http.rs::map_status_to_error` when a 403 response body contains
`"code": "SYSTEM_ACCESS_REQUIRED"`. The details object is deserialized into the
variant fields. Falls back to existing `Forbidden` for any other 403 (which remains
intentionally opaque to avoid information leakage in non-access-gate contexts).

A helper struct `SystemAccessErrorDetails` (with serde Deserialize) handles the
JSON parsing of the details object.

### temper-cli: Error Rendering

When any CLI command hits `ClientError::SystemAccessRequired`, render a
human-friendly message instead of a generic error. The message adapts based on
`join_request_status`:

**Never requested (status is null):**
```
✗ You're signed in as pete@example.com, but this temper instance
  requires approved access.

  To request access, run:
    temper team join --message "what you'd like to do with temper"

  Or visit: https://temperkb.io/request-access
```

**Pending:**
```
✗ You're signed in as pete@example.com, but this temper instance
  requires approved access.

  Your access request is pending review.
  Run `temper team status` to check for updates.
```

**Rejected:**
```
✗ You're signed in as pete@example.com, but this temper instance
  requires approved access.

  Your previous request was not approved. You can submit a new one:
    temper team join --message "what you'd like to do with temper"
```

**Withdrawn:**
```
✗ You're signed in as pete@example.com, but this temper instance
  requires approved access.

  You withdrew your previous request. Submit a new one:
    temper team join --message "what you'd like to do with temper"
```

This rendering lives in the CLI's error handling layer (likely `error.rs` or a
dedicated `display` impl), not in temper-client.

### temper-mcp: Non-Retryable Error Propagation

When any MCP tool call results in a `SystemAccessRequired` error (either from
the middleware intercepting the request or from a service call), the MCP server
returns an `rmcp::ErrorData` with:

- A clear, actionable message including the instance URL
- An explicit "do not retry" instruction for agents
- No new MCP tools — agents cannot submit join requests on behalf of users

Example error message:
```
Access to this temper instance requires approval.
Visit https://temperkb.io/request-access or run `temper team join`
in the CLI to request access.
This error is terminal and should not be retried.
```

The MCP server already wraps service errors into `rmcp::ErrorData`. The change is
to detect the specific `SystemAccessRequired` case (either via the API error code
or the service-level error) and format a targeted message rather than a generic
internal error.

---

## Slice 2: `temper team` CLI Commands

### Command Structure

New `commands/team.rs` module with a `TeamAction` clap enum:

```rust
#[derive(Subcommand)]
enum TeamAction {
    /// Request to join a team (defaults to system access)
    Join {
        #[arg(long)]
        team: Option<String>,
        #[arg(long)]
        message: Option<String>,
    },
    /// Check your request or membership status
    Status {
        #[arg(long)]
        team: Option<String>,
    },
    /// Withdraw a pending request or leave a team
    Leave {
        #[arg(long)]
        team: Option<String>,
    },
}
```

Registered in the top-level CLI as `temper team {join,status,leave}`.

### `temper team join [--team <slug>] [--message "..."]`

Calls `POST /api/access/requests` via `AccessClient`. When no `--team` is
provided, targets the system gating team. The client sends `source: "cli"`.

**Success:**
```
Access request submitted to temperkb.io.
You'll gain access once an admin approves your request.
Run `temper team status` to check.
```

**Conflict (409 — pending request exists):**
```
You already have a pending request.
Run `temper team status` to check its status.
```

### `temper team status [--team <slug>]`

Calls `GET /api/access/requests/me` via `AccessClient`. Renders based on state:

| State | Output |
|-------|--------|
| No request | "You haven't requested access yet. Run `temper team join` to get started." |
| Pending | "Your request is pending review (submitted 2026-04-05)." |
| Approved | "You have access. Approved on 2026-04-06." |
| Rejected | "Your previous request was not approved. Submit a new one with `temper team join`." |
| Withdrawn | "You withdrew your request. Submit a new one with `temper team join`." |

### `temper team leave [--team <slug>]`

Context-aware behavior based on current state:

- **Pending request:** Calls `DELETE /api/access/requests/me` (withdraw). Prints "Request withdrawn."
- **Approved member:** The leave-team API endpoint doesn't exist yet. Render
  "To leave a team after approval, contact an admin." (Endpoint added in the
  team management phase.)
- **No request/membership:** "Nothing to leave — you don't have a pending request."

### temper-client: AccessClient

Follows the existing `ProfileClient` sub-client pattern:

```rust
pub struct AccessClient<'a> {
    http: &'a HttpClient,
}

impl<'a> AccessClient<'a> {
    pub(crate) fn new(http: &'a HttpClient) -> Self { ... }

    pub async fn create_request(
        &self,
        message: Option<&str>,
        source: &str,
        accepted_terms_version: Option<&str>,
    ) -> Result<JoinRequest>;

    pub async fn get_own_request(&self) -> Result<Option<JoinRequest>>;

    pub async fn withdraw_request(&self) -> Result<()>;

    pub async fn get_settings(&self) -> Result<PublicSystemSettings>;
}
```

Exposed from `TemperClient` as `client.access()`.

The `JoinRequest` and `PublicSystemSettings` types already exist in temper-core
with serde Deserialize derives. If `JoinRequest` is missing Deserialize (it
currently only has Serialize for API responses), add it.

---

## Files Changed

### Slice 1 (error handling)
- `crates/temper-core/src/types/access_gate.rs` — Add `SystemAccessDetails` struct
- `crates/temper-api/src/middleware/system_access.rs` — Enrich 403 with details
- `crates/temper-api/src/error.rs` — Update `SystemAccessRequired` serialization
- `crates/temper-client/src/error.rs` — Add `SystemAccessRequired` variant
- `crates/temper-client/src/http.rs` — Parse enriched 403 into new variant
- `crates/temper-cli/src/error.rs` — Render friendly SystemAccessRequired message
- `crates/temper-mcp/src/service.rs` — Detect and format access-required errors

### Slice 2 (team commands)
- `crates/temper-client/src/access.rs` — New `AccessClient` sub-client
- `crates/temper-client/src/lib.rs` — Expose `client.access()`
- `crates/temper-cli/src/commands/team.rs` — New team command module
- `crates/temper-cli/src/commands/mod.rs` — Register team commands
- `crates/temper-cli/src/main.rs` — Wire up `TeamAction`

---

## Out of Scope

- Admin CLI commands (`temper team requests`, `temper team review`) — later session
- Team leave after approval (requires new API endpoint) — team management phase
- SvelteKit UI for request-access and admin queue — Session 5+
- `--team` flag targeting non-system teams — works structurally but the API
  currently only handles the system gating team's join requests
- Terms acceptance flow (`--accept-terms`) — can be added when terms are configured
