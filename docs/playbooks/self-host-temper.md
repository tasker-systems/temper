# Self-Host Temper

**For operators** — someone standing up a Temper deployment on their own Vercel
project, Neon database, and Auth0 tenant, rather than using the hosted service
at `temperkb.io`.

## Outcome

By the end of this playbook you will have a running Temper instance — API, MCP
server, and CLI surface — deployed on your own Vercel project with a Neon
database and an Auth0 tenant. You will have verified it with a health check, a
CLI login, and an end-to-end resource round-trip. The optional web UI deploys
separately; see [Deploy the Web UI](./deploy-the-web-ui.md).

## Prerequisites

- **A Vercel account** and access to the Temper repository.
- **A Neon account** — you will create a PostgreSQL 17 project.
- **An Auth0 tenant** — you will create an API and two native applications.
  Using Okta instead? See [self-host with Okta](./self-host-with-okta.md).
- **The trust boundary Temper enforces** — read
  [the trust boundary](../concepts/trust-boundary.md) to understand the
  two-gate boundary every call crosses before you configure who it admits.
- **The auth-identity contract** — read
  [auth identity](../concepts/auth-identity.md) to understand the six
  variables that decide whose tokens your instance trusts. This playbook tells
  you how to set them; the concept page tells you why they must agree.

> **Doing a full ground-up enterprise install?** This playbook is one phase.
> For the single end-to-end sequence (deploy → SAML → org → agents) see
> [enterprise install](./enterprise-install.md).

## Topology

One Vercel project hosts **three Rust functions and a set of TypeScript
functions** from a single deployment:

```text
                              Vercel (one project)
                    ┌────────────────────────────────────────┐
 CLI / MCP client   │  api/mcp.rs        MCP server          │
 ──────────────────▶│  api/axum.rs       REST API            │
                    │  api/internal.rs   drains + crons      │
 Vercel cron (×7)   │  api/oauth/*.ts    OAuth + SAML        │
 ──────────────────▶│                                        │
                    └────────────────────────────────────────┘
                               │                │
                               ▼                ▼
                           Neon PG 17       Auth0 tenant
                           (pgvector)       (JWT issuer)
```

All four share the same database and Auth0 tenant. The routing lives in
`vercel.json` at the repo root; `handle: filesystem` runs first, then the
routes match in order:

| Route | Function | What it is |
|---|---|---|
| `/mcp`, `/mcp/(.*)` | `api/mcp.rs` | The MCP server. |
| `/api/embed/dispatch`, `/api/embed/warm`, `/api/region/dispatch`, `/api/slack/intents/reap` | `api/internal.rs` | Cron-driven work. **Not called by clients** — see below. |
| `/internal/(.*)` | `api/internal.rs` | Server-to-server, HMAC-gated (the SAML reconcile channel). |
| `/oauth/token`, `/oauth/jwks`, `/oauth/authorize`, `/oauth/saml/{login,acs,metadata}`, `/.well-known/oauth-authorization-server` | `api/oauth/*.ts` | The authorization server. TypeScript, not Rust. |
| `/oauth/(.*)`, `/.well-known/(.*)` | `api/mcp.rs` | Whatever the named OAuth routes above did not claim. |
| `/(.*)` | `api/axum.rs` | The REST API (catch-all). |

`framework` is `null`; there is no framework-level routing. `SQLX_OFFLINE=true`
is set in the build environment so the Rust macros compile against the
committed `.sqlx/` cache rather than a live database.

### The drains are not optional

`api/internal.rs` runs the background work, and **nothing invokes it unless the
crons are configured**. `vercel.json` declares seven:

| Schedule | Path | What stalls without it |
|---|---|---|
| every minute | `/api/embed/dispatch?shard=0..3` (four entries) | Embedding: new and changed resources are never vectorised, so semantic search does not see them. |
| every 2 minutes | `/api/embed/warm` | Cold-start latency on the embedding path. |
| every minute | `/api/region/dispatch` | Region materialization: cognitive-map regions never form. |
| hourly | `/api/slack/intents/reap` | Expired Slack link intents are never swept. |

A deployment that skips them accepts writes and looks healthy while search
results and cogmap regions silently stop advancing. `api/internal.rs` is given
`maxDuration: 300` for this reason — the drains are long-running relative to a
request. The queries for checking that the drains are keeping up are in
`drain-operator-queries`.

## Provision Neon

Create a new Neon project. Select **PostgreSQL 17** (Neon's GA version — the
local dev Docker image runs 18, but the cloud deployment targets 17).

### Enable extensions

Open a SQL console on the `neondb` database and run:

```sql
CREATE EXTENSION IF NOT EXISTS vector;
CREATE EXTENSION IF NOT EXISTS pg_uuidv7;
```

`vector` provides the `pgvector` embedding type used by the search pipeline.
`pg_uuidv7` provides in-database UUIDv7 generation. `plpgsql` is enabled by
default and does not require an explicit `CREATE EXTENSION`.

### Capture connection strings

From the Neon console, copy both connection strings:

- **Pooled URL** (`DATABASE_URL`) — the host contains `-pooler`. Used at
  runtime.
- **Direct URL** (`DATABASE_URL_UNPOOLED`) — no `-pooler` suffix. Used for
  migrations only.

Both take the form:

```text
postgresql://<user>:<password>@<host>/neondb?sslmode=require&channel_binding=require
```

### Run migrations

Migrations are a **deploy step**, not a startup step. The API server does not
auto-migrate on boot. After setting your Vercel environment variables (see
below), run migrations against the direct URL from your local machine or a CI
job:

```sh
DATABASE_URL=<DATABASE_URL_UNPOOLED> sqlx migrate run
```

Migration files live in `migrations/`. sqlx is the single migration authority —
never apply schema changes by other means.

Migrations provision the **schema**. Some content is delivered separately as an
operator step — notably the L0 kernel cogmap's landmarks + telos charter, which
is admin-gated and fail-closed. See
[l0-content-delivery](./deliver-l0-content.md) if you need a populated
L0 map on your instance.

### Neon × Vercel integration

If you connect your Neon project to Vercel via the Neon integration, Neon
automatically provisions `DATABASE_URL` and `DATABASE_URL_UNPOOLED` per preview
branch. Pull-request preview deployments therefore get isolated databases with
no manual wiring. The migration step still runs separately — Vercel does not
run it automatically.

## Provision Auth0

> **Using Okta instead?** This section is Auth0-specific. For standing up the
> same instance against an Okta tenant in an enterprise context, see
> [self-host with Okta](./self-host-with-okta.md) — it covers the custom
> authorization server, API Access Management requirement, and the
> Okta-specific environment and CLI configuration. The rest of this playbook
> (Neon, Vercel, verify) applies unchanged.

The contract is **one API resource server and two native applications** (plus
an optional confidential web-app client if you deploy the
[web UI](./deploy-the-web-ui.md)):

### 1. API resource server

Create an API in Auth0. The **identifier** you assign becomes the OAuth
audience — the one audience your REST surface validates. It appears as
`AUTH_AUDIENCE` in your Vercel environment; a conventional value is
`https://<instance>/api`. See [auth identity](../concepts/auth-identity.md) for
the full contract, including the optional `MCP_AUDIENCE` restatement.

**Recommended: create a second API for MCP.** An instance can serve a second
audience — `MCP_AUDIENCE`, conventionally `https://<instance>/mcp` — that the
MCP surface validates and advertises as its OAuth `resource`. This matters
because conformant MCP clients validate that the advertised `resource` equals
the MCP server URL or its origin, and refuse the sign-in flow otherwise. With
`MCP_AUDIENCE` set, MCP clients get tokens for `https://<instance>/mcp` while
the CLI and machine clients keep `https://<instance>/api` — nothing else
changes for either. Unset, both surfaces share the one API audience.

### 2. CLI native application

Create a **Native** application for the `temper` CLI:

- Grant types: `authorization_code`, `refresh_token`
- Allowed callback URL: `https://<instance>/api/auth/cli-callback`
- The application's `client_id` is what users supply when running `temper init`
  with `--auth-client-id`.

### 3. MCP native application

Create a second **Native** application for MCP clients (e.g. Claude Desktop):

- Allowed callbacks:
  - `https://claude.ai/api/mcp/auth_callback` — Claude Desktop (Connectors UI)
  - `https://claude.com/api/mcp/auth_callback` — Claude Desktop (alternate)
  - `http://localhost/mcp/oauth/callback` — CLI MCP clients via loopback proxy (opencode)
  - `http://127.0.0.1/mcp/oauth/callback` — same, IPv4 loopback variant
  - `http://localhost` — temper CLI relay (port-wildcard for `temper auth login`)
  - `https://<instance>/api/auth/mcp-callback` — the loopback redirect proxy relay
  - `https://<instance>/api/auth/cli-callback` — the temper CLI login relay

  CLI MCP clients (opencode, Claude Code) use `http://127.0.0.1:<port>/callback` as
  their redirect URI. Auth0's RFC 8252 loopback port-wildcard does not work on all
  tenants, so Temper proxies loopback redirects through `https://<instance>/oauth/authorize`
  → Auth0 → `https://<instance>/api/auth/mcp-callback` → `http://127.0.0.1:<port>/callback`.
  The `http://localhost/mcp/oauth/callback` entry (no port) matches any port via Auth0's
  Native app loopback wildcard when it works; the relay URL is the fallback when it doesn't.

- This application's `client_id` becomes `MCP_CLIENT_ID` in your Vercel
  environment.

### Reading values from a live tenant

If you already have a tenant configured, you can enumerate its values with the
`auth0` CLI:

```sh
auth0 apis list           # → shows identifier (your AUTH_AUDIENCE)
auth0 apps list           # → shows client_id for each application
```

The Auth0 MCP server (`@auth0/auth0-mcp-server`) provides the same information
in an agentic session.

### Env var mapping

| Auth0 value | Environment variable | Notes |
| ----------- | -------------------- | ----- |
| Tenant domain | `AUTH_ISSUER` | `https://<tenant>.auth0.com/` — trailing slash required |
| Tenant JWKS endpoint | `JWKS_URL` | `https://<tenant>.auth0.com/.well-known/jwks.json` |
| API identifier | `AUTH_AUDIENCE` | The API audience — validated by the REST API and (with `MCP_AUDIENCE` unset) the MCP server |
| Auth provider | `AUTH_PROVIDER_NAME` | Always `auth0` |
| API identifier (MCP) | `MCP_AUDIENCE` | Optional. The MCP surface's own OAuth `resource` (conventionally `https://<instance>/mcp`). Unset, it defaults to `AUTH_AUDIENCE` — see the [second API note](#1-api-resource-server) for why setting it is recommended |
| MCP app client_id | `MCP_CLIENT_ID` | The MCP native application's client_id |
| Instance base URL | `MCP_BASE_URL` | `https://<instance>` — no trailing slash |
| Proxy state secret | `MCP_PROXY_SECRET` | Not an Auth0 value — you generate it: a random string of **at least 32 characters** (`openssl rand -base64 48`). The Auth0 loopback redirect proxy derives its state-token encryption key from this secret alone, so keep it secret and out of source control |

## Deploy to Vercel

Import the repository into a new Vercel project. Set `framework` override to
**Other** (`vercel.json` sets `"framework": null`). Configure the following
environment variables in the Vercel project dashboard before the first
deployment.

### Environment variable contract

| Variable | Surface | Required | Notes |
| -------- | ------- | -------- | ----- |
| `DATABASE_URL` | api, mcp | Yes | Pooled Neon connection string (runtime) |
| `DATABASE_URL_UNPOOLED` | deploy step | Yes | Direct Neon connection string (migrations only) |
| `AUTH_ISSUER` | api, mcp | Yes | `https://<tenant>.auth0.com/` — trailing slash required |
| `JWKS_URL` | api, mcp | Yes | `https://<tenant>.auth0.com/.well-known/jwks.json` |
| `AUTH_AUDIENCE` | api, mcp | Yes | The API audience the REST surface validates (e.g. `https://<instance>/api`). Boot fails if unset or empty |
| `AUTH_PROVIDER_NAME` | api, mcp | Yes | Set to `auth0` |
| `MCP_AUDIENCE` | mcp | No | The MCP surface's own OAuth `resource` (e.g. `https://<instance>/mcp`). Unset, it defaults to `AUTH_AUDIENCE` |
| `MCP_CLIENT_ID` | mcp | Yes | MCP native application client_id |
| `MCP_BASE_URL` | mcp | Yes | `https://<instance>` — used in OAuth discovery responses |
| `MCP_PROXY_SECRET` | mcp | Yes | At least 32 characters (`openssl rand -base64 48`). Encrypts the state token in the loopback redirect proxy that every externally-fronted instance serves (Auth0 or Okta). Requests that need that key — a loopback `/oauth/authorize` and the `/api/auth/mcp-callback` relay — answer `503` naming this variable until it is set, so an MCP CLI client cannot sign in without it. `temper auth login` and browser clients with an HTTPS callback do not use the proxy's crypto and are unaffected. **Set it before the first deployment and redeploy after any change** — a function's environment is bound to its deployment. Rotating it fails sign-ins already in flight for up to 10 minutes; they succeed on retry. Not used in the SAML path |
| `API_BASE_URL` | ui | No | Only for the optional [web UI](./deploy-the-web-ui.md) (a separate Vercel project); not required for API + MCP + CLI |
| `BLOB_READ_WRITE_TOKEN` | api | Yes | Vercel Blob token — credentials for the binary-blob flow (`temper blob …`, `/api/blobs`, the MCP `blob_read`/`blob_manage` tools) and the upload/extract/embed pipeline. Unset (with no `BLOB_STORE_ID`), the blob endpoints and tools answer a disabled refusal naming this variable. Policy knobs, all optional with defaults: `BLOB_MAX_BYTES` (per-blob cap, 100 MB), `BLOB_CONTENT_TYPE_ALLOWLIST` (comma-separated media types; default png, jpeg, webp, svg, gif, pdf), `BLOB_SINGLE_REQUEST_MAX_BYTES` (single-request threshold, 4 MB — beyond it the CLI segments automatically) |
| `ENABLE_SWAGGER` | api | No | Set `true` to expose `/swagger-ui` in non-production deployments |
| `PORT` | api | No | Platform-injected by Vercel; defaults to `3000`. Only relevant for local or non-Vercel runs |
| `SQLX_OFFLINE` | build | Yes | Must be `true` — compile-time SQL checks run against the committed `.sqlx/` cache |
| `CORS_ORIGINS` | api | Situational | See note below |

**`CORS_ORIGINS` caveat:** This variable is required for any client that calls
the API **cross-origin** from a browser. When `CORS_ORIGINS` is unset, the API
returns no CORS headers and cross-origin requests fail. The bundled
`temper-ui` does **not** need it — it reverse-proxies API/MCP traffic
same-origin through its own server (see
[Deploy the Web UI](./deploy-the-web-ui.md)), so the browser never makes a
cross-origin call. Set `CORS_ORIGINS` only if you run a *separate* browser-based
client against the API directly. A permissive development value is `*`;
production should list only the specific origins that need access.

### Auth identity: setting the variables

The full contract — the two modes, the agreement rules, and why boot failure is
the control — is on [auth identity](../concepts/auth-identity.md). What follows
is the operator-specific practical content.

Your instance runs in one of two modes, decided by whether `AS_ISSUER` is set.
Under an external IdP (Auth0 or Okta), leave the `AS_*` variables unset — this
is the shape the rest of this playbook assumes. Under the Temper AS (the mode
that backs [SAML](./self-host-with-saml.md)), set them.

For AS mode, derive the values from the instance origin rather than retyping
them — the five variables carry two facts, and a typo in any one of them is a
typo in a fact you already stated:

```sh
# AS mode: issuer, JWKS, and audience all restate one instance. Derive, don't retype.
INSTANCE="https://temper.acme.com"

AS_ISSUER="$INSTANCE"                     # the mode signal
AUTH_ISSUER="$INSTANCE"                   # == AS_ISSUER
JWKS_URL="$INSTANCE/oauth/jwks"           # == $AS_ISSUER/oauth/jwks
AS_AUDIENCE="$INSTANCE/api"               # what the AS mints for API flows
AUTH_AUDIENCE="$INSTANCE/api"             # == AS_AUDIENCE — what the REST surface validates
MCP_AUDIENCE="$INSTANCE/mcp"              # optional — the MCP surface's own resource indicator
```

Confirm the mode the instance booted in — `temper-AS` or `external-IdP`:

```sh
vercel logs <deployment-url> | grep 'auth configured'
```

An incoherent auth config refuses to start, naming the offending variable and
the relation it must satisfy. See [auth identity](../concepts/auth-identity.md)
for why.

### The routing contract

The authoritative routing, function and cron configuration is `vercel.json` at
the repo root. It declares three Rust functions (`api/axum.rs`, `api/mcp.rs`,
`api/internal.rs`), eighteen routes and seven crons; the Topology section above
summarises what each one is for.

Do not hand-edit it without also updating the function it routes to.

## Configure the CLI

After deploying, users point the `temper` CLI at your instance. The CLI ships
unconfigured; `temper init` performs the setup.

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

These variables take precedence over `config.toml` and are suitable for CI/CD
and headless agent contexts:

| Variable | Purpose |
| -------- | ------- |
| `TEMPER_API_URL` | Override the API base URL |
| `TEMPER_PROVIDER` | Override the auth provider name |
| `TEMPER_TOKEN` | Inject a JWT directly — no OAuth flow, no disk state |

For a fully headless agent session, export `TEMPER_TOKEN` alongside
`TEMPER_API_URL` and no other configuration is needed. The token is used
in-memory; `~/.config/temper/auth.json` is not read or written.

## Connect MCP Clients

Point MCP clients at `https://<instance>/mcp`. OAuth discovery is served
automatically:

- `GET /.well-known/oauth-authorization-server` — RFC 8414 metadata
- `GET /.well-known/oauth-protected-resource` — RFC 9728 metadata
- `POST /oauth/register` — DCR proxy (returns the pre-registered MCP client_id)

Clients that support OAuth 2.0 dynamic client registration will discover the
authorization server automatically from the well-known endpoints.

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

The MCP server validates JWTs against `JWKS_URL` and checks the instance's one
audience — `AUTH_AUDIENCE`, the same value the REST API validates (see
[auth identity](../concepts/auth-identity.md)). Ensure `MCP_CLIENT_ID` matches
the Auth0 native application registered for your MCP clients and that the
client's callback URLs are allowlisted in that Auth0 application.

## Verify

Run these checks after the first deployment and migration.

### Health check

```sh
curl https://<instance>/api/health
```

A healthy response is HTTP 200 with a JSON body. A 500 or connection error
typically indicates a missing environment variable or a failed migration.

### The loopback redirect proxy

`temper auth login` and browser clients do not use the proxy's state-token key, so
they pass whether or not `MCP_PROXY_SECRET` is set. This check is the one that does:

```sh
curl -si "https://<instance>/oauth/authorize?response_type=code&state=x\
&client_id=<MCP_CLIENT_ID>&redirect_uri=http://127.0.0.1:1/callback" | head -1
```

Expect `HTTP/2 302`. A `503` names the variable to set — MCP CLI clients such as
opencode and Claude Code sign in through this path, so fix it before inviting them.

### CLI login

```sh
temper auth login
```

This runs the OAuth 2.0 Authorization Code + PKCE flow: it opens a browser to
the provider's `/authorize` endpoint, the provider redirects the authorization
code to `/api/auth/cli-callback` (a stateless relay), and that relay forwards
the code to a short-lived listener on `localhost`. The CLI then exchanges the
code for tokens, prints a confirmation, and caches the token locally. (There is
no device-code polling — `temper auth login` always uses a browser redirect.)

### End-to-end resource round-trip

```sh
# Create a resource
temper resource create --type session --title "smoke test"

# List to confirm it landed
temper resource list --type session

# Retrieve it by ref (UUID or decorated slug-<uuid>, printed as `ref`)
temper resource show <ref>
```

A successful round-trip confirms that the API, database writes, and read-back
path are all working against your instance. See
[contexts and refs](../concepts/contexts-and-refs.md) for the full ref grammar.

## Not covered

- **Multi-region or HA Neon** — This playbook targets a single Neon project in
  one region. Neon's branching and read-replica features are not covered.
- **Alternative messaging backends** — The deployment described here uses the
  default messaging configuration. RabbitMQ and other transports are not
  covered.

Single-instance self-hosting (one Vercel project + one Neon project + one
Auth0 tenant) is the supported target today.

## Further reading

- **The trust boundary your deployment enforces:**
  [The Trust Boundary](../concepts/trust-boundary.md).
- **The auth-identity contract you are configuring:**
  [Auth Identity](../concepts/auth-identity.md).
- **What the architecture fixes vs. what a deployment chooses:**
  [temperkb.io/operating/deployment](https://temperkb.io/operating/deployment).
- **Governance and administration:**
  [temperkb.io/operating/governance-and-administration](https://temperkb.io/operating/governance-and-administration).
