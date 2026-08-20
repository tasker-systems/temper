# Deploy the Web UI

**For operators** — someone who has already stood up the core Temper deployment
(API, MCP, database, IdP) and wants to add the optional browser front-end.

## Outcome

By the end of this playbook you will have the `temper-ui` SvelteKit app deployed
as a second Vercel project, pointed at your running API instance, with OIDC
login working through your identity provider. The UI is fully config-driven: no
source edits, no fork.

## Prerequisites

- **A running Temper deployment** — see
  [self-host Temper](./self-host-temper.md). You need the API origin and the
  IdP tenant from that deployment.
- **An OIDC-capable identity provider** — Auth0, Okta, or any provider that
  publishes a standard OpenID Connect discovery document.
- **A second Vercel project** — the UI deploys separately from the API, from
  the same repository (root directory `packages/temper-ui`).

## Two couplings, both env-driven

- **Browser-facing API/MCP/OAuth traffic** is reverse-proxied by the UI's
  server (`hooks.server.ts`) to `API_BASE_URL`, rather than via a hardcoded
  `vercel.json` rewrite. Requests to `/api/*`, `/mcp`, `/oauth/*`, and
  `/.well-known/*` on the UI origin are forwarded server-side to your API host.
  Because this is a same-origin proxy (the browser only ever talks to the UI
  origin), **the UI does not require `CORS_ORIGINS` on the API** for its own
  traffic.

  > ⚠️ **`API_BASE_URL` must be the API backend's *own* origin, not the UI's
  > public origin.** If the UI and API share a public domain (e.g. the UI
  > serves both `temperkb.io` and proxies `temperkb.io/api`), pointing
  > `API_BASE_URL` at that shared domain makes the proxy forward to *itself* —
  > an infinite loop the platform terminates with `508 Loop Detected`. Set it
  > to the distinct origin where the API actually runs (its own `*.vercel.app`
  > URL, or a dedicated `api.` subdomain). The UI guards against this and
  > returns a clear 500 rather than looping, but the value still needs to be
  > correct for the proxy to work.

- **Login** is generic OIDC Authorization Code + PKCE. Endpoints are resolved
  from `OIDC_ISSUER`'s discovery document
  (`/.well-known/openid-configuration`), so any OIDC provider works. Logout
  uses the standard RP-initiated `end_session_endpoint`.

## Register a confidential OIDC client

In your identity provider, register a **Regular Web Application** (confidential
client) for the UI, distinct from the CLI/MCP native apps:

- **Allowed callback / redirect URI:** `https://<ui-host>/auth/callback`
- **Allowed logout / post-logout redirect URI:** `https://<ui-host>`
- **Grant types:** Authorization Code + Refresh Token (the UI requests the
  `offline_access` scope)

## Environment variable contract (UI project)

| Variable | Required | Notes |
| -------- | -------- | ----- |
| `API_BASE_URL` | Yes | The API backend's **own** origin (not the UI's public origin — see the loop warning above), e.g. `https://<api-host>` — used by server loaders **and** the browser-facing reverse proxy |
| `OIDC_ISSUER` | Yes¹ | Issuer base URL, e.g. `https://<tenant>.auth0.com` or `https://<org>.okta.com/oauth2/<asId>`. Discovery resolved from `<issuer>/.well-known/openid-configuration` |
| `OIDC_CLIENT_ID` | Yes¹ | The UI confidential web-app client_id |
| `OIDC_CLIENT_SECRET` | Yes¹ | The UI confidential web-app client secret |
| `OIDC_AUDIENCE` | Situational | Required for Auth0 (the API identifier); omit for Okta custom auth servers, which carry it implicitly |
| `APP_URL` | Yes | The UI's own public origin, e.g. `https://<ui-host>` — used to build the redirect and post-logout URIs |
| `SESSION_SECRET` | Yes | ≥32 bytes of entropy (64-char hex or 44-char base64) — derives the JWE session-cookie key |

¹ **Back-compat fallback:** if `OIDC_*` are unset, the UI falls back to the
canonical deployment's `AUTH0_DOMAIN` / `AUTH0_CLIENT_ID` /
`AUTH0_CLIENT_SECRET` / `AUTH0_AUDIENCE` (with `OIDC_ISSUER` derived as
`https://<AUTH0_DOMAIN>`). Self-hosters should set the `OIDC_*` variables
directly; the fallback exists so the hosted `temperkb.io` project keeps working
unchanged. A non-Auth0 provider is exercised end to end in
[self-host with Okta](./self-host-with-okta.md).

## Further reading

- **The core deployment this UI connects to:**
  [Self-host Temper](./self-host-temper.md).
- **The auth-identity contract behind the OIDC client:**
  [Auth Identity](../concepts/auth-identity.md).
- **Self-host with Okta (non-Auth0 provider, end to end):**
  [self-host with Okta](./self-host-with-okta.md).
