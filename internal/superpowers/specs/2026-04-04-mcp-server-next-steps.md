# MCP Server — Next Steps

Status: Phase 1 (scaffold + auth + initial tools) is complete.
Date: 2026-04-04

## What We Shipped

The `temper-mcp` crate implements a remote MCP server deployed as a Vercel serverless function. It uses Streamable HTTP transport (rmcp 1.3.0), Auth0 as the OAuth Authorization Server via RFC 8414/9728 discovery, and the existing `JwksKeyStore` infrastructure for JWT validation. Six tools are available: `list_resources`, `get_resource`, `create_resource`, `search`, `list_contexts`, `get_context`.

The server runs in stateless mode (`json_response=true`, `stateful_mode=false`) so each Vercel invocation is independent — no session state survives across cold starts.

---

## Phase 2 — Auth Hardening & Deployment Verification

### Auth0 Dashboard Configuration

Before the MCP server can be used in production, verify the Auth0 dashboard configuration:

- [ ] **API**: Confirm the Auth0 API with identifier `https://temperkb.io/` exists and `MCP_AUDIENCE` matches it
- [ ] **Application**: Ensure the Auth0 application allows callback URLs used by MCP clients — Claude Desktop uses `http://localhost:PORT/callback`-style redirects; verify this pattern is in the allowed callback URLs list
- [ ] **Scopes**: Define `mcp:read` and `mcp:write` scopes on the API, or confirm that the existing scopes suffice
- [ ] **PKCE**: Confirm S256 code challenge method is enabled (should be by default)

### Vercel Environment Variables

- [ ] Add `MCP_BASE_URL=https://temperkb.io` to the Vercel project
- [ ] Add `MCP_AUDIENCE` (or confirm it can fall back to `AUTH_AUDIENCE`)

### Deployment Smoke Test

- [ ] Deploy the branch to a Vercel preview URL
- [ ] Verify `GET /.well-known/oauth-protected-resource` returns correct Auth0 domain
- [ ] Verify `GET /.well-known/oauth-authorization-server` returns valid Auth0 endpoints
- [ ] Verify `GET /mcp/health` returns `ok`
- [ ] Test full OAuth flow: Claude Desktop → Auth0 login → token → `POST /mcp` with `tools/list`
- [ ] Check Vercel function logs for cold-start behavior and JWT validation

---

## Phase 3 — More Tools

Add tools that map to existing temper-api routes but aren't yet exposed:

| Tool | Maps to | Priority |
|------|---------|----------|
| `update_resource` | `PATCH /api/resources/{id}` | High — agents need to update titles/metadata |
| `delete_resource` | `DELETE /api/resources/{id}` | Medium — soft-delete |
| `get_resource_content` | `GET /api/resources/{id}/content` | High — currently folded into `get_resource` with `include_content` flag; may be cleaner as a separate tool |
| `create_context` | `POST /api/contexts` | Medium — agents may want to create workspaces |
| `list_events` | `GET /api/events` | Low — useful for debugging |
| `get_profile` | `GET /api/profile` | Low — useful for identity/settings |

The `ResourceCreateRequest` type from temper-core is already used directly as the tool parameter. The same pattern applies for new tools: reuse core types where they exist, add MCP-specific input types only where needed (e.g., the `include_content` flag on `get_resource`).

---

## Phase 4 — Persistent Sessions

The current implementation uses `LocalSessionManager` in stateless mode — each Vercel invocation creates a fresh service. This works because Streamable HTTP handles stateless request/response natively.

If multi-turn sessions become important (e.g., agents that maintain long conversations with tool calls spread across many requests), consider:

- **Redis-backed session manager**: Replace `LocalSessionManager` with a Redis implementation. The `SessionManager` trait in rmcp is designed for this — implement `create_session`, `has_session`, `close_session`, `create_stream`, `accept_message`.
- **Vercel Fluid Compute**: Vercel's newer compute model may allow longer-lived processes. Evaluate whether `stateful_mode=true` becomes viable.
- **Session timeout**: Even with persistent sessions, set a reasonable TTL (e.g., 30 minutes) to avoid unbounded state growth.

---

## Phase 5 — MCP Resources Protocol

The current implementation exposes vault content through *tools* (function calls). The MCP spec also supports a *resources* protocol — a way for servers to expose structured data that clients can inject into context without explicit tool calls.

Potential resource URIs:

```
temper://contexts                          → list of contexts
temper://contexts/{name}/resources         → resources in a context
temper://resources/{id}                    → single resource with content
temper://resources/{id}/content            → raw markdown content
```

This would allow MCP clients to browse the vault structure through the resources panel and inject relevant content directly, complementing the tool-based interface.

---

## Phase 6 — Streaming & Subscriptions

### Streaming Tool Responses

For large search result sets or full-content retrievals, the current approach returns everything in a single JSON response. With SSE mode enabled, individual tool calls could stream results incrementally:

- Return search results as they're found
- Stream large resource content in chunks
- Provide progress updates for long-running operations

### Resource Subscriptions

The MCP spec supports `subscribe` / `unsubscribe` for resource change notifications. If a client subscribes to `temper://contexts/{name}/resources`, the server could notify when new resources are added or existing ones change. This requires persistent sessions (Phase 4).

---

## Phase 7 — Local MCP Server

The current server is remote (Vercel). A local MCP server could run alongside the CLI for agents that work with the local vault directly, without cloud sync:

- Backed by the local filesystem vault, not the database
- No auth required (local trust model)
- Uses stdio transport (standard for local MCP servers)
- Reuses the same `TemperMcpService` tool implementations, backed by a different storage layer

This would be a separate binary target in temper-cli or temper-mcp with a `local` feature flag.
