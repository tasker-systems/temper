# Self-Hosting Temper

This guide is for operators standing up their own Temper instance — on their own Vercel project, Neon database, and Auth0 tenant — rather than using the hosted service at `temperkb.io`.

**Scope:** This runbook covers the API + MCP surfaces, plus an optional [web UI](#deploy-the-ui-optional). The `temper-ui` web application (SvelteKit) deploys as its own Vercel project with its own confidential OIDC client; it is fully config-driven (no per-org fork) and is documented in the UI section below.

> **Doing a full ground-up enterprise install?** This guide is one phase. For the single
> end-to-end sequence (deploy → SAML → org → agents) see [enterprise-install.md](./enterprise-install.md).

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

Migrations provision the **schema**. Some content is delivered separately as an
operator step — notably the L0 kernel cogmap's landmarks + telos charter, which
is admin-gated and fail-closed. See
[l0-content-delivery.md](./l0-content-delivery.md) if you need a populated L0 map
on your instance.

### Neon × Vercel integration

If you connect your Neon project to Vercel via the Neon integration, Neon automatically provisions `DATABASE_URL` and `DATABASE_URL_UNPOOLED` per preview branch. Pull-request preview deployments therefore get isolated databases with no manual wiring. The migration step still runs separately — Vercel does not run it automatically.

## Provision Auth0

> **Using Okta instead?** This section is Auth0-specific. For standing up the same instance against
> an Okta tenant in an enterprise context, see [Self-Hosting with Okta](./self-hosting-okta.md) —
> it covers the custom authorization server, API Access Management requirement, and the
> Okta-specific environment and CLI configuration. The rest of this guide (Neon, Vercel, verify)
> applies unchanged.

The contract is **one API resource server and two native applications** (plus an optional confidential web-app client if you deploy the [web UI](#deploy-the-ui-optional)):

### 1. API resource server

Create an API in Auth0. The **identifier** you assign becomes the OAuth audience — the one audience your instance validates, on both surfaces. It appears as `AUTH_AUDIENCE` in your Vercel environment. A conventional value is `https://<instance>/api`. See [Auth identity: the variables that must agree](#auth-identity-the-variables-that-must-agree) for the full contract, including the optional `MCP_AUDIENCE` restatement.

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
| API identifier | `AUTH_AUDIENCE` | The one audience — validated by **both** the REST API and the MCP server |
| Auth provider | `AUTH_PROVIDER_NAME` | Always `auth0` |
| API identifier (MCP) | `MCP_AUDIENCE` | Optional. If set, it **must equal** `AUTH_AUDIENCE` — it restates the one audience, it does not add a second one |
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
| `AUTH_AUDIENCE` | api, mcp | Yes | The one audience both surfaces validate (e.g. `https://<instance>/api`). Boot fails if unset or empty |
| `AUTH_PROVIDER_NAME` | api, mcp | Yes | Set to `auth0` |
| `MCP_AUDIENCE` | api, mcp | No | An optional restatement of `AUTH_AUDIENCE`. If set it must **equal** it; unset is the normal configuration |
| `MCP_CLIENT_ID` | mcp | Yes | MCP native application client_id |
| `MCP_BASE_URL` | mcp | Yes | `https://<instance>` — used in OAuth discovery responses |
| `API_BASE_URL` | ui | No | Only for the optional [web UI](#deploy-the-ui-optional) (a separate Vercel project); not required for API + MCP + CLI |
| `BLOB_READ_WRITE_TOKEN` | api | Yes | Vercel Blob token — used by the upload/extract/embed pipeline |
| `ENABLE_SWAGGER` | api | No | Set `true` to expose `/swagger-ui` in non-production deployments |
| `PORT` | api | No | Platform-injected by Vercel; defaults to `3000`. Only relevant for local or non-Vercel runs |
| `SQLX_OFFLINE` | build | Yes | Must be `true` — compile-time SQL checks run against the committed `.sqlx/` cache |
| `CORS_ORIGINS` | api | Situational | See note below |

**`CORS_ORIGINS` caveat:** This variable is required for any client that calls the API **cross-origin** from a browser. When `CORS_ORIGINS` is unset, the API returns no CORS headers and cross-origin requests fail. Note the bundled `temper-ui` does **not** need it — it reverse-proxies API/MCP traffic same-origin through its own server (see [Deploy the UI](#deploy-the-ui-optional)), so the browser never makes a cross-origin call. Set `CORS_ORIGINS` only if you run a *separate* browser-based client against the API directly. A permissive development value is `*`; production should list only the specific origins that need access.

### Auth identity: the variables that must agree

**Audience:** every operator standing up an instance — Auth0, Okta, or Temper's own AS.

**Scope:** the six variables that decide *whose* tokens this instance trusts and *which* tokens it accepts. Six variables, but not six decisions — read the table below as one identity spelled several ways.

Your instance runs in exactly one of two **modes**, and the mode is decided by a single signal: whether `AS_ISSUER` is set.

- **External IdP** — `AS_ISSUER` unset. Auth0 or Okta mints tokens; Temper is a pure resource server. This is the shape the rest of this guide assumes, and the shape `temperkb.io` runs.
- **Temper AS** — `AS_ISSUER` set. Temper's own authorization server mints tokens: the mode that backs [SAML](./self-hosting-saml.md) and [temper-issued machine credentials](./machine-credentials.md).

| Variable | External IdP (`AS_ISSUER` unset) | Temper AS (`AS_ISSUER` set) |
| -------- | -------------------------------- | --------------------------- |
| `AUTH_ISSUER` | The IdP's issuer URL — `https://<tenant>.auth0.com/` | Your instance's origin — **must equal `AS_ISSUER`** |
| `JWKS_URL` | The IdP's JWKS endpoint — `https://<tenant>.auth0.com/.well-known/jwks.json` | **Must be `$AS_ISSUER/oauth/jwks`** — the AS publishes its own keys |
| `AUTH_AUDIENCE` | The IdP's API identifier. **The one audience**, validated on both surfaces | **Must equal `AS_AUDIENCE`** |
| `MCP_AUDIENCE` | Optional. If set, **must equal `AUTH_AUDIENCE`**; unset is normal | Optional. Same rule |
| `AS_ISSUER` | Leave unset — setting it flips the instance into AS mode | Required. Your instance's origin (no trailing slash needed) — its presence *is* the mode signal |
| `AS_AUDIENCE` | Leave unset — never read in this mode | Required. **Must equal `AUTH_AUDIENCE`** |

Trailing slashes are normalized before every comparison — Auth0 issuers conventionally end in `/` and the AS's own metadata strips them, so `https://temper.acme.com` and `https://temper.acme.com/` are the same issuer as far as the gate is concerned.

> **On an AS instance the three audiences are one value spelled three ways — not three independent knobs.** The Temper AS mints **every** token, human and machine, with the single server-side `AS_AUDIENCE`, ignoring any request-supplied `audience` (`packages/temper-cloud/src/oauth/mint.ts`). So `AS_AUDIENCE` is the audience *minted*, `AUTH_AUDIENCE` the audience *validated*, and `MCP_AUDIENCE` — if you set it at all — merely *restates* it. They are the same string or the instance verifies nothing. Under an external IdP there is no AS, so `AS_*` is unset entirely: that is why the agreement rules are mode-dependent, and why no operator should be expected to hold them in their head.

In AS mode, derive the values from the instance origin rather than retyping them — the five variables carry two facts, and a typo in any one of them is a typo in a fact you already stated:

```sh
# AS mode: issuer, JWKS, and audience all restate one instance. Derive, don't retype.
INSTANCE="https://temper.acme.com"

AS_ISSUER="$INSTANCE"                     # the mode signal
AUTH_ISSUER="$INSTANCE"                   # == AS_ISSUER
JWKS_URL="$INSTANCE/oauth/jwks"           # == $AS_ISSUER/oauth/jwks
AS_AUDIENCE="$INSTANCE/api"               # what the AS mints
AUTH_AUDIENCE="$INSTANCE/api"             # == AS_AUDIENCE — what both surfaces validate
```

> **An incoherent auth config fails the boot.** Both surfaces parse this identity once, at startup, through the same code (`crates/temper-services/src/auth_config.rs`), and an instance that violates any rule above **refuses to start** — naming the offending variable and the relation it must satisfy (it never prints a value). That is deliberate. The old behavior was a `warn` line and a served request; a warning in a serverless log is not a control.

The gate's messages are prescriptive, e.g. *"MCP_AUDIENCE is set but does not equal AUTH_AUDIENCE. This instance validates one audience on both surfaces. Set them to the same value, or unset MCP_AUDIENCE."* Boot also logs which mode it resolved — an operator who cannot tell which mode their instance is in is precisely the operator who mis-sets these variables:

```sh
# Confirm the mode the instance booted in — "temper-AS" or "external-IdP"
vercel logs <deployment-url> | grep 'auth configured'
```

**These are not new constraints.** A working AS instance already satisfied every one of them: a divergent audience verifies nothing, a divergent issuer trusts the wrong party, and a misdirected `JWKS_URL` checks no signature against the keys that actually signed the token. Temper now *names* rules that were already true, and fails fast when they are not — which is why a hard boot failure cannot break a working deployment. It can only refuse to start one that was already broken and had not noticed.

#### What breaks if they disagree

The old failures were quiet, and they were quiet in *opposite* directions — one typo, two behaviors:

| Misconfiguration | Old behavior | Now |
| ---------------- | ------------ | --- |
| `AUTH_AUDIENCE` unset or empty | The REST API set `validate_aud = false` and **accepted any token from the issuer**, regardless of which API it was minted for — audience validation silently off | Boot refuses |
| `MCP_AUDIENCE` empty | The MCP server enforced `aud == ""` and **rejected every token** | An empty value is treated as *absent*, uniformly — the instance's one audience still applies |
| `MCP_AUDIENCE` set to something else | The two surfaces validated two different audiences | Boot refuses |
| `AS_AUDIENCE` ≠ `AUTH_AUDIENCE` (AS mode) | Every AS-minted token 401s at the resource server | Boot refuses |
| `AUTH_ISSUER` ≠ `AS_ISSUER` (AS mode) | The API trusts a party that mints none of its tokens | Boot refuses |
| `JWKS_URL` not the AS's (AS mode) | No signature is ever checked against the keys that signed the token | Boot refuses |

Every one of those used to surface as a caller's 401 — or, worse, as a *missing* 401 — days later, far from the variable that caused it. Now it surfaces as a loud refusal at startup, with the variable named.

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
| `TEMPER_PROVIDER` | Override the auth provider name |
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

The MCP server validates JWTs against `JWKS_URL` and checks the instance's one audience — `AUTH_AUDIENCE`, the same value the REST API validates (see [Auth identity](#auth-identity-the-variables-that-must-agree)). Ensure `MCP_CLIENT_ID` matches the Auth0 native application registered for your MCP clients and that the client's callback URLs are allowlisted in that Auth0 application.

## Deploy the UI (optional)

The `temper-ui` SvelteKit app is an **optional** browser front-end. It deploys as a **second Vercel project** from the same monorepo (root directory `packages/temper-ui`) and talks to the API instance you stood up above. It is single-repo and config-driven: an operator points it at their own API origin and their own OIDC issuer entirely through environment variables — no source edits, no fork.

### Two couplings, both env-driven

- **Browser-facing API/MCP/OAuth traffic** is reverse-proxied by the UI's server (`hooks.server.ts`) to `API_BASE_URL`, rather than via a hardcoded `vercel.json` rewrite. Requests to `/api/*`, `/mcp`, `/oauth/*`, and `/.well-known/*` on the UI origin are forwarded server-side to your API host. Because this is a same-origin proxy (the browser only ever talks to the UI origin), **the UI does not require `CORS_ORIGINS` on the API** for its own traffic.

  > ⚠️ **`API_BASE_URL` must be the API backend's *own* origin, not the UI's public origin.** If the UI and API share a public domain (e.g. the UI serves both `temperkb.io` and proxies `temperkb.io/api`), pointing `API_BASE_URL` at that shared domain makes the proxy forward to *itself* — an infinite loop the platform terminates with `508 Loop Detected`. Set it to the distinct origin where the API actually runs (its own `*.vercel.app` URL, or a dedicated `api.` subdomain). The UI guards against this and returns a clear 500 rather than looping, but the value still needs to be correct for the proxy to work.
- **Login** is generic OIDC Authorization Code + PKCE. Endpoints are resolved from `OIDC_ISSUER`'s discovery document (`/.well-known/openid-configuration`), so any OIDC provider works. Logout uses the standard RP-initiated `end_session_endpoint`.

### Register a confidential OIDC client

In your identity provider, register a **Regular Web Application** (confidential client) for the UI, distinct from the CLI/MCP native apps:

- **Allowed callback / redirect URI:** `https://<ui-host>/auth/callback`
- **Allowed logout / post-logout redirect URI:** `https://<ui-host>`
- **Grant types:** Authorization Code + Refresh Token (the UI requests the `offline_access` scope)

### Environment variable contract (UI project)

| Variable | Required | Notes |
| -------- | -------- | ----- |
| `API_BASE_URL` | Yes | The API backend's **own** origin (not the UI's public origin — see the loop warning above), e.g. `https://<api-host>` — used by server loaders **and** the browser-facing reverse proxy |
| `OIDC_ISSUER` | Yes¹ | Issuer base URL, e.g. `https://<tenant>.auth0.com` or `https://<org>.okta.com/oauth2/<asId>`. Discovery resolved from `<issuer>/.well-known/openid-configuration` |
| `OIDC_CLIENT_ID` | Yes¹ | The UI confidential web-app client_id |
| `OIDC_CLIENT_SECRET` | Yes¹ | The UI confidential web-app client secret |
| `OIDC_AUDIENCE` | Situational | Required for Auth0 (the API identifier); omit for Okta custom auth servers, which carry it implicitly |
| `APP_URL` | Yes | The UI's own public origin, e.g. `https://<ui-host>` — used to build the redirect and post-logout URIs |
| `SESSION_SECRET` | Yes | ≥32 bytes of entropy (64-char hex or 44-char base64) — derives the JWE session-cookie key |

¹ **Back-compat fallback:** if `OIDC_*` are unset, the UI falls back to the canonical deployment's `AUTH0_DOMAIN` / `AUTH0_CLIENT_ID` / `AUTH0_CLIENT_SECRET` / `AUTH0_AUDIENCE` (with `OIDC_ISSUER` derived as `https://<AUTH0_DOMAIN>`). Self-hosters should set the `OIDC_*` variables directly; the fallback exists so the hosted `temperkb.io` project keeps working unchanged. A non-Auth0 provider is exercised end to end in [self-hosting-okta.md](self-hosting-okta.md).

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

This runs the OAuth 2.0 Authorization Code + PKCE flow: it opens a browser to the provider's `/authorize` endpoint, the provider redirects the authorization code to `/api/auth/cli-callback` (a stateless relay), and that relay forwards the code to a short-lived listener on `localhost`. The CLI then exchanges the code for tokens, prints a confirmation, and caches the token locally. (There is no device-code polling — `temper login` always uses a browser redirect.)

### End-to-end resource round-trip

```sh
# Create a resource
temper resource create --type session --title "smoke test"

# List to confirm it landed
temper resource list --type session

# Retrieve it by ref (UUID or decorated slug-<uuid>, printed as `ref`)
temper resource show <ref>
```

A successful round-trip confirms that the API, database writes, and read-back path are all working against your instance.

## Not Covered / Deferred

The following are outside the scope of this runbook:

- **Multi-region or HA Neon** — This guide targets a single Neon project in one region. Neon's branching and read-replica features are not covered.
- **Alternative messaging backends** — The deployment described here uses the default messaging configuration. RabbitMQ and other transports are not covered.

Single-instance self-hosting (one Vercel project + one Neon project + one Auth0 tenant) is the supported target today.
