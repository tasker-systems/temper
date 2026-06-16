# Self-Hosting Temper

This guide is for operators standing up their own Temper instance — on their own Vercel project, Neon database, and Auth0 tenant — rather than using the hosted service at `temperkb.io`.

**Scope:** This runbook covers the API + MCP surfaces only. The `temper-ui` web application (SvelteKit) requires its own Auth0 Regular-Web-App and a separate Vercel project; that deployment is deferred and not documented here.

## Topology

One Vercel project hosts two Rust services from a single deployment:

```text
                           Vercel
                    ┌──────────────────────────────────┐
 CLI / MCP client   │                                  │
 ──────────────────▶│  /.well-known/*  ─┐              │
 temper resource    │  /oauth/*         ├─▶ api/mcp    │
 temper login       │  /mcp             │   (MCP srv)  │
                    │  /mcp/*          ─┘              │
                    │                                  │
                    │  /(.*)           ────▶ api/axum  │
                    │                       (REST API) │
                    └──────────────────────────────────┘
                               │                │
                               ▼                ▼
                           Neon PG 17       Auth0 tenant
                           (pgvector)       (JWT issuer)
```

Both services share the same database and Auth0 tenant. The routing lives in `vercel.json` at the repo root:

- `handle: filesystem` runs first (static files if any).
- `/mcp`, `/mcp/*`, `/oauth/*`, `/.well-known/*` route to `api/mcp` (the MCP server).
- `/(.*)` (catch-all) routes to `api/axum` (the REST API).

`framework` is `null`; there is no framework-level routing. `SQLX_OFFLINE=true` is set in the build environment so the Rust macros compile against the committed `.sqlx/` cache rather than a live database.

## Provision Neon

Create a new Neon project. Select **PostgreSQL 17** (Neon's GA version — the local dev Docker image runs 18, but the cloud deployment targets 17).

### Enable extensions

Open a SQL console on the `neondb` database and run:

```sql
CREATE EXTENSION IF NOT EXISTS vector;
CREATE EXTENSION IF NOT EXISTS pg_uuidv7;
```

`vector` provides the `pgvector` embedding type used by the search pipeline. `pg_uuidv7` provides in-database UUIDv7 generation. `plpgsql` is enabled by default and does not require an explicit `CREATE EXTENSION`.

### Capture connection strings

From the Neon console, copy both connection strings:

- **Pooled URL** (`DATABASE_URL`) — the host contains `-pooler`. Used at runtime.
- **Direct URL** (`DATABASE_URL_UNPOOLED`) — no `-pooler` suffix. Used for migrations only.

Both take the form:

```text
postgresql://<user>:<password>@<host>/neondb?sslmode=require&channel_binding=require
```

### Run migrations

Migrations are a **deploy step**, not a startup step. The API server does not auto-migrate on boot. After setting your Vercel environment variables (see below), run migrations against the direct URL from your local machine or a CI job:

```sh
DATABASE_URL=<DATABASE_URL_UNPOOLED> sqlx migrate run
```

Migration files live in `migrations/`. sqlx is the single migration authority — never apply schema changes by other means.

### Neon × Vercel integration

If you connect your Neon project to Vercel via the Neon integration, Neon automatically provisions `DATABASE_URL` and `DATABASE_URL_UNPOOLED` per preview branch. Pull-request preview deployments therefore get isolated databases with no manual wiring. The migration step still runs separately — Vercel does not run it automatically.

## Provision Auth0

The contract is **one API resource server and two native applications** (the web app is out of scope):

### 1. API resource server

Create an API in Auth0. The **identifier** you assign becomes the OAuth audience. This identifier appears as `AUTH_AUDIENCE` (for the REST API) and `MCP_AUDIENCE` (for the MCP server) in your Vercel environment. A conventional value is `https://<instance>/api`.

### 2. CLI native application

Create a **Native** application for the `temper` CLI:

- Grant types: `authorization_code`, `refresh_token`
- Allowed callback URL: `https://<instance>/api/auth/cli-callback`
- The application's `client_id` is what users supply when running `temper init` with `--auth-client-id`.

### 3. MCP native application

Create a second **Native** application for MCP clients (e.g. Claude Desktop):

- Allowed callbacks: callback URLs for the MCP clients you support (e.g. `https://claude.ai/api/mcp/auth_callback`, `https://claude.com/api/mcp/auth_callback`, `http://localhost`).
- This application's `client_id` becomes `MCP_CLIENT_ID` in your Vercel environment.

### Reading values from a live tenant

If you already have a tenant configured, you can enumerate its values with the `auth0` CLI:

```sh
auth0 apis list           # → shows identifier (your AUTH_AUDIENCE)
auth0 apps list           # → shows client_id for each application
```

The Auth0 MCP server (`@auth0/auth0-mcp-server`) provides the same information in an agentic session.

### Env var mapping

| Auth0 value | Environment variable | Notes |
| ----------- | -------------------- | ----- |
| Tenant domain | `AUTH_ISSUER` | `https://<tenant>.auth0.com/` — trailing slash required |
| Tenant JWKS endpoint | `JWKS_URL` | `https://<tenant>.auth0.com/.well-known/jwks.json` |
| API identifier | `AUTH_AUDIENCE` | Set to the same value as `MCP_AUDIENCE` if using one API |
| Auth provider | `AUTH_PROVIDER_NAME` | Always `auth0` |
| API identifier (MCP) | `MCP_AUDIENCE` | Typically same as `AUTH_AUDIENCE` |
| MCP app client_id | `MCP_CLIENT_ID` | The MCP native application's client_id |
| Instance base URL | `MCP_BASE_URL` | `https://<instance>` — no trailing slash |

## Deploy to Vercel

Import the repository into a new Vercel project. Set `framework` override to **Other** (the `vercel.json` sets `"framework": null`). Configure the following environment variables in the Vercel project dashboard before the first deployment.

### Environment variable contract

| Variable | Surface | Required | Notes |
| -------- | ------- | -------- | ----- |
| `DATABASE_URL` | api, mcp | Yes | Pooled Neon connection string (runtime) |
| `DATABASE_URL_UNPOOLED` | deploy step | Yes | Direct Neon connection string (migrations only) |
| `AUTH_ISSUER` | api, mcp | Yes | `https://<tenant>.auth0.com/` — trailing slash required |
| `JWKS_URL` | api, mcp | Yes | `https://<tenant>.auth0.com/.well-known/jwks.json` |
| `AUTH_AUDIENCE` | api | Yes | Auth0 API identifier (e.g. `https://<instance>/api`) |
| `AUTH_PROVIDER_NAME` | api, mcp | Yes | Set to `auth0` |
| `MCP_AUDIENCE` | mcp | Yes | Auth0 API identifier for MCP token validation |
| `MCP_CLIENT_ID` | mcp | Yes | MCP native application client_id |
| `MCP_BASE_URL` | mcp | Yes | `https://<instance>` — used in OAuth discovery responses |
| `API_BASE_URL` | api | Yes | `https://<instance>` — used in redirect generation |
| `BLOB_READ_WRITE_TOKEN` | api | Yes | Vercel Blob token — used by the upload/extract/embed pipeline |
| `ENABLE_SWAGGER` | api | No | Set `true` to expose `/swagger-ui` in non-production deployments |
| `SQLX_OFFLINE` | build | Yes | Must be `true` — compile-time SQL checks run against the committed `.sqlx/` cache |
| `CORS_ORIGINS` | api | Situational | See note below |

**`CORS_ORIGINS` caveat:** This variable is required for any cross-origin client (browser-based UI, browser-based MCP client). When `CORS_ORIGINS` is unset, the API returns no CORS headers and cross-origin requests fail. The live `temperkb.io` deployment sets this on Preview and Dev environments but not Production (which serves requests only from the CLI and MCP clients, not a browser UI). Operators hosting a web UI — even the deferred `temper-ui` — must set this explicitly. A permissive development value is `*`; production should list only the specific origins that need access.

### vercel.json summary

The routing contract at the repo root is:

```json
{
  "framework": null,
  "build": { "env": { "SQLX_OFFLINE": "true" } },
  "routes": [
    { "handle": "filesystem" },
    { "src": "/mcp",          "dest": "/api/mcp" },
    { "src": "/mcp/(.*)",     "dest": "/api/mcp" },
    { "src": "/oauth/(.*)",   "dest": "/api/mcp" },
    { "src": "/.well-known/(.*)", "dest": "/api/mcp" },
    { "src": "/(.*)",         "dest": "/api/axum" }
  ]
}
```

Do not modify this file unless you are also updating `api/axum.rs` or `api/mcp.rs`.

## Configure the CLI

After deploying, users point the `temper` CLI at your instance. The CLI ships unconfigured; `temper init` performs the setup.

### Interactive setup

```sh
temper init
```

Select **self-hosted** at the instance-type prompt. You will be asked for:

1. Instance URL — `https://<instance>`
2. Auth0 domain — `<tenant>.auth0.com`
3. Auth0 client ID — the CLI native application's `client_id`
4. Auth0 audience — the API identifier (e.g. `https://<instance>/api`)

The resulting `~/.config/temper/config.toml` looks like:

```toml
[cloud]
api_url = "https://<instance>"

[auth]
provider = "auth0"

[[auth.providers]]
name = "auth0"
authorize_url = "https://<tenant>.auth0.com/authorize"
token_url = "https://<tenant>.auth0.com/oauth/token"
client_id = "<cli-app-client-id>"
audience = "https://<instance>/api"
callback_url = "https://<instance>/api/auth/cli-callback"
scopes = ["openid", "profile", "email", "offline_access"]
```

### Headless / scripted setup

For CI pipelines or automated provisioning, skip the interactive prompts:

```sh
temper init \
  --no-interactive \
  --instance-url https://<instance> \
  --auth-domain <tenant>.auth0.com \
  --auth-client-id <cli-app-client-id> \
  --auth-audience https://<instance>/api
```

### Environment variable overrides

These variables take precedence over `config.toml` and are suitable for CI/CD and headless agent contexts:

| Variable | Purpose |
| -------- | ------- |
| `TEMPER_API_URL` | Override the API base URL |
| `TEMPER_PROVIDER_ENV` | Override the auth provider name |
| `TEMPER_TOKEN` | Inject a JWT directly — no OAuth flow, no disk state |

For a fully headless agent session, export `TEMPER_TOKEN` alongside `TEMPER_API_URL` and no other configuration is needed. The token is used in-memory; `~/.config/temper/auth.json` is not read or written.

## Connect MCP Clients

Point MCP clients at `https://<instance>/mcp`. OAuth discovery is served automatically:

- `GET /.well-known/oauth-authorization-server` — RFC 8414 metadata
- `GET /.well-known/oauth-protected-resource` — RFC 9728 metadata
- `POST /oauth/register` — DCR proxy (returns the pre-registered MCP client_id)

Clients that support OAuth 2.0 dynamic client registration will discover the authorization server automatically from the well-known endpoints.

For manual configuration (e.g. Claude Desktop's `claude_desktop_config.json`):

```json
{
  "mcpServers": {
    "temper": {
      "url": "https://<instance>/mcp"
    }
  }
}
```

The MCP server validates JWTs against `JWKS_URL` and checks `MCP_AUDIENCE`. Ensure `MCP_CLIENT_ID` matches the Auth0 native application registered for your MCP clients and that the client's callback URLs are allowlisted in that Auth0 application.

## Verify

Run these checks after the first deployment and migration.

### Health check

```sh
curl https://<instance>/api/health
```

A healthy response is HTTP 200 with a JSON body. A 500 or connection error typically indicates a missing environment variable or a failed migration.

### CLI login

```sh
temper login
```

This opens a browser to complete the Auth0 device authorization flow. On success the CLI prints a confirmation and caches the token locally.

### End-to-end resource round-trip

```sh
# Create a resource
temper resource create --type session --title "smoke test"

# List to confirm it landed
temper resource list --type session

# Retrieve it by slug
temper resource show <slug>
```

A successful round-trip confirms that the API, database writes, and read-back path are all working against your instance.

## Not Covered / Deferred

The following are outside the scope of this runbook:

- **temper-ui web application** — The SvelteKit app (`packages/temper-ui`) requires a separate Vercel project and an Auth0 Regular-Web-App (distinct from the native apps above). This deployment path is deferred; no runbook exists for it yet.
- **Multi-region or HA Neon** — This guide targets a single Neon project in one region. Neon's branching and read-replica features are not covered.
- **Alternative messaging backends** — The deployment described here uses the default messaging configuration. RabbitMQ and other transports are not covered.

Single-instance self-hosting (one Vercel project + one Neon project + one Auth0 tenant) is the supported target today.
