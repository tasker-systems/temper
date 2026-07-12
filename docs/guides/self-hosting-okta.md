# Self-Hosting Temper with Okta

This guide is the **Okta variant** of [Self-Hosting Temper](./self-hosting.md). It covers only the parts that differ when your identity provider is **Okta** instead of Auth0 — the authorization server, the application registrations, and the resulting environment and CLI configuration.

**Read the [main self-hosting guide](./self-hosting.md) first.** Everything outside of auth — the topology, provisioning Neon, deploying to Vercel, the `vercel.json` routing contract, and the verification steps — is identical. Only the "Provision Auth0" and "Configure the CLI" sections are replaced by what follows here.

**Scope:** API + MCP surfaces, same as the main guide, plus the optional [web UI](#deploy-the-ui-okta) configured against an Okta custom authorization server.

## How Temper validates tokens (why Okta works at all)

Temper's API validates every request's JWT against three things only: the **issuer** (`AUTH_ISSUER`), the **audience** (`AUTH_AUDIENCE`), and the signing keys at `JWKS_URL`. The `AUTH_PROVIDER_NAME` value is a label attached to resolved profiles and the key used to cache resolved emails — it does **not** switch validation logic. This is why Okta works as a drop-in issuer: get the issuer, audience, and JWKS right and the tokens validate.

Two consequences shape the rest of this guide:

1. Temper requires a **custom audience** on its access tokens. Okta can only mint custom audiences from a **custom authorization server**, which requires the **API Access Management** add-on.
2. Okta's authorization-server URLs differ from Auth0's (an issuer with no trailing slash; `/oauth2/<authServerId>/v1/*` endpoints). `temper init` emits these for you, and Temper resolves the `/userinfo` endpoint via OIDC discovery, so the differences surface only in the values you configure — not as manual workarounds.

## Prerequisite: API Access Management

Okta gives every org a built-in **org authorization server**, but it **cannot customize the audience** (`aud`) claim, and its access tokens are intended for Okta's own APIs — not for validation by your services. Temper needs a custom audience, so you **must** use a **custom authorization server**.

Custom authorization servers are part of Okta's **API Access Management** product — an optional, paid add-on in production orgs. **Confirm your tenant has API Access Management enabled before continuing.** Without it, there is no supported way to host Temper on Okta.

## Provision the custom authorization server

In the Okta Admin Console: **Security → API → Authorization Servers → Add Authorization Server.**

1. **Name** — e.g. `temper`.
2. **Audience** — set this to the value you will use for `AUTH_AUDIENCE` and `MCP_AUDIENCE` (e.g. `https://<instance>/api`). The access-token `aud` claim will carry this value, and Temper checks it on every request.

Once created, note the authorization server's **issuer URI**, shown on its Settings tab. It has the form:

```text
https://<okta-domain>/oauth2/<authServerId>
```

`<okta-domain>` is your Okta org domain (`<org>.okta.com`, `<org>.oktapreview.com`, or a custom domain). `<authServerId>` is the server's ID (the built-in default server uses the literal `default`; a server you create gets a generated ID like `aus1a2b3c...`).

> **No trailing slash.** Okta's issuer is `https://<okta-domain>/oauth2/<authServerId>` with no trailing slash. Auth0's issuer *requires* a trailing slash; Okta's must *not* have one. `AUTH_ISSUER` must match the token's `iss` claim exactly, so copy the issuer URI verbatim.

### Add an access policy and rule

Custom authorization servers **deny by default** — if a client matches no access policy, the token request fails. On the authorization server's **Access Policies** tab:

1. **Add Policy** — assign it to the apps you will create below (or to **All clients**).
2. **Add Rule** — in the rule's grant-type conditions, allow **Authorization Code** and **Refresh Token**. (These are the only grants Temper's CLI uses; see below.)

Without at least one policy + rule, login will fail even when every URL and ID is correct.

### Add an `email` claim to the access token (recommended)

Temper resolves the user's email from the access token's `email` claim. When that claim is absent it falls back to the OIDC `/userinfo` endpoint, which Temper now resolves via discovery (`{issuer}/.well-known/openid-configuration`), so the fallback works against Okta. Putting `email` directly on the access token is still **recommended** — it's the fast path and avoids a per-process discovery + userinfo round-trip — but it is no longer mandatory.

On the authorization server's **Claims** tab: **Add Claim** —

- **Name:** `email`
- **Include in token type:** **Access Token**
- **Value type:** Expression
- **Value:** `user.email`
- **Include in:** the scopes/policies your apps use (or "Any scope")

With neither the claim nor a reachable `/userinfo` (e.g. the token lacks the `email` scope), login fails with `Token missing email claim and userinfo lookup failed`.

## Provision the applications

The contract mirrors the main guide: **two native applications** (CLI + MCP), plus an **optional confidential web application** if you deploy the [web UI](#deploy-the-ui-okta). The two native apps are created under **Applications → Create App Integration → OIDC - OpenID Connect → Native Application**.

### 1. CLI native application

The `temper` CLI uses the **Authorization Code + PKCE** flow with a loopback relay — **not** the device authorization grant. Configure the app accordingly:

- **Grant types:** **Authorization Code** and **Refresh Token**. (Do not enable Device Authorization — Temper does not use it.)
- **Sign-in redirect URI:** `https://<instance>/api/auth/cli-callback`
- PKCE is required for native apps by default, which is exactly what the CLI sends.
- Assign the app to the custom authorization server's access policy (above).

The app's **Client ID** is the CLI client ID used in `config.toml` (below).

### 2. MCP native application

Create a second Native application for MCP clients (e.g. Claude Desktop):

- **Sign-in redirect URIs:** the callbacks for the MCP clients you support, e.g. `https://claude.ai/api/mcp/auth_callback`, `https://claude.com/api/mcp/auth_callback`, `http://localhost`.
- Assign it to the custom authorization server's access policy.

This app's **Client ID** becomes `MCP_CLIENT_ID`.

> **Dynamic client registration is handled for you.** MCP's OAuth flow normally expects dynamic client registration (DCR), but Okta's DCR endpoint returns `403` for custom authorization servers unless called with an admin API token — which arbitrary MCP clients cannot do. Temper sidesteps this entirely: its `/oauth/register` endpoint is a proxy that returns the pre-registered `MCP_CLIENT_ID`. You do not need to enable or configure Okta DCR.

### 3. UI web application (optional)

Only if you deploy the [web UI](#deploy-the-ui-okta). Create a **Web Application** (confidential client) under **Applications → Create App Integration → OIDC - OpenID Connect → Web Application**:

- **Grant types:** **Authorization Code** and **Refresh Token**.
- **Sign-in redirect URI:** `https://<ui-host>/auth/callback`
- **Sign-out redirect URI:** `https://<ui-host>` (enables RP-initiated logout via the authorization server's `end_session_endpoint`).
- Assign the app to the custom authorization server's access policy (above), and ensure the `email` claim is reachable (per the claim/scope note earlier) so the UI can populate the user identity.

The app's **Client ID** and **Client secret** become the UI's `OIDC_CLIENT_ID` / `OIDC_CLIENT_SECRET`.

### Reading values from a live tenant

If your tenant is already configured, the `okta` CLI and the Okta management API can enumerate these values (authorization server issuer and audience, application client IDs). The Admin Console shows the same information on each authorization server's Settings tab and each application's General tab.

## Environment variable contract (Okta values)

Set these in your Vercel project. They follow the same contract as the [main guide's table](./self-hosting.md#environment-variable-contract); only the auth values differ.

| Variable | Surface | Okta value |
| -------- | ------- | ---------- |
| `AUTH_ISSUER` | api, mcp | `https://<okta-domain>/oauth2/<authServerId>` — **no trailing slash** |
| `JWKS_URL` | api, mcp | `https://<okta-domain>/oauth2/<authServerId>/v1/keys` |
| `AUTH_AUDIENCE` | api | The custom authorization server's **Audience** value (e.g. `https://<instance>/api`) |
| `MCP_AUDIENCE` | — | **Optional.** An instance has one audience; both surfaces read `AUTH_AUDIENCE`. If you set this, it must **equal** `AUTH_AUDIENCE` or the instance refuses to boot. |
| `AUTH_PROVIDER_NAME` | api, mcp | **Keep `auth0`.** It is a profile label and email-cache key, not a validation switch; leave it at the default rather than inventing an `okta` value |
| `MCP_CLIENT_ID` | mcp | The MCP native application's Client ID |
| `MCP_BASE_URL` | mcp | `https://<instance>` — no trailing slash |

Everything else in the main guide's environment contract (`DATABASE_URL`, `DATABASE_URL_UNPOOLED`, `BLOB_READ_WRITE_TOKEN`, `SQLX_OFFLINE`, `CORS_ORIGINS`, etc.) is provider-independent — set those exactly as the main guide describes.

### UI project (Okta values)

If you deploy the [web UI](#deploy-the-ui-okta), set these in its **separate** Vercel project. These are the Okta-specific values for the main guide's [UI contract](./self-hosting.md#environment-variable-contract-ui-project):

| Variable | Okta value |
| -------- | ---------- |
| `API_BASE_URL` | The API backend's **own** origin (not the UI's public origin — see the loop warning in the [main guide](./self-hosting.md#deploy-the-ui-optional)), e.g. `https://<api-host>` |
| `OIDC_ISSUER` | `https://<okta-domain>/oauth2/<authServerId>` — **no trailing slash**. Discovery is served at `<issuer>/.well-known/openid-configuration` |
| `OIDC_CLIENT_ID` | The UI web application's Client ID |
| `OIDC_CLIENT_SECRET` | The UI web application's Client secret |
| `OIDC_AUDIENCE` | **Omit.** An Okta custom authorization server stamps its configured Audience on tokens implicitly, so no `audience` request param is needed; omitting it makes the access-token `aud` match `AUTH_AUDIENCE` automatically |
| `APP_URL` | `https://<ui-host>` — the UI's own public origin |
| `SESSION_SECRET` | ≥32 bytes of entropy (64-char hex or 44-char base64) |

Set `OIDC_*` directly — do **not** rely on the `AUTH0_*` fallback for an Okta install. Because the UI proxies browser-facing API/MCP traffic same-origin to `API_BASE_URL`, the UI does not require `CORS_ORIGINS` on the API for its own traffic.

## Configure the CLI (Okta)

> **`temper init` now supports Okta.** Interactively, choose **self-hosted → Okta** and enter your authorization server ID. Headless, pass `--idp okta --auth-server-id <authServerId>` alongside the existing self-host flags:
>
> ```sh
> temper init --no-interactive \
>   --instance-url https://<instance> \
>   --auth-domain <okta-domain> \
>   --idp okta --auth-server-id <authServerId> \
>   --auth-client-id <cli-app-client-id> \
>   --auth-audience <custom-auth-server-audience>
> ```
>
> The hand-written block below remains valid as a reference (note `provider`/`name` stay `auth0`).

```toml
[cloud]
api_url = "https://<instance>"

[auth]
provider = "auth0"

[[auth.providers]]
name = "auth0"
authorize_url = "https://<okta-domain>/oauth2/<authServerId>/v1/authorize"
token_url = "https://<okta-domain>/oauth2/<authServerId>/v1/token"
client_id = "<cli-app-client-id>"
audience = "<custom-auth-server-audience>"
callback_url = "https://<instance>/api/auth/cli-callback"
scopes = ["openid", "profile", "email", "offline_access"]
```

Notes:

- **`provider` and `name` must both be `auth0`** and must match each other. The value is a label the CLI uses to select the provider block; it does not need to read "okta". The OAuth flow is identical regardless of the name.
- `audience` is the custom authorization server's Audience value — the same string as `AUTH_AUDIENCE`.
- `scopes` includes `offline_access` so Okta issues a refresh token; keep it.

### Environment variable overrides

The same overrides documented in the main guide apply unchanged: `TEMPER_API_URL`, `TEMPER_PROVIDER`, and `TEMPER_TOKEN`. For a fully headless agent session, export `TEMPER_TOKEN` (a JWT minted by your Okta authorization server) alongside `TEMPER_API_URL`; no `config.toml` is needed.

## Connect MCP clients

Identical to the main guide — point MCP clients at `https://<instance>/mcp`. OAuth discovery (`/.well-known/oauth-authorization-server`, `/.well-known/oauth-protected-resource`) and the `/oauth/register` DCR proxy are served by Temper itself, not by Okta, so MCP clients discover and register against your instance regardless of the upstream IdP. Ensure `MCP_CLIENT_ID` matches the Okta MCP native application and that the MCP clients' callback URLs are listed as sign-in redirect URIs on that application.

## Deploy the UI (Okta)

The web UI is provider-agnostic: its login is generic OIDC Authorization Code + PKCE resolved from `OIDC_ISSUER`'s discovery document, so it works against an Okta custom authorization server with no UI source changes. Follow the [main guide's UI section](./self-hosting.md#deploy-the-ui-optional) for the deployment mechanics (separate Vercel project, root `packages/temper-ui`, same-origin reverse proxy), using the [Okta UI env values](#ui-project-okta-values) above and the [confidential web application](#3-ui-web-application-optional) you registered.

The only Okta-specific points: set `OIDC_ISSUER` to `https://<okta-domain>/oauth2/<authServerId>` and **omit** `OIDC_AUDIENCE` (the custom authorization server stamps its Audience implicitly). Discovery resolves `authorization_endpoint`, `token_endpoint`, and `end_session_endpoint` from Okta automatically, so login, refresh, and RP-initiated logout all work without any provider-specific configuration in the UI.

## Verify

Use the same verification steps as the [main guide](./self-hosting.md#verify): `/api/health`, `temper login`, and a resource round-trip. `temper login` opens a browser to your Okta authorization server's `/v1/authorize` endpoint and completes the Authorization Code + PKCE flow.

If login fails with `Token missing email claim and userinfo lookup failed`, either add the access-token `email` claim (above) or ensure the CLI's granted scopes include `email` so the `/userinfo` fallback can return it.

### UI login (if deployed)

Visit `https://<ui-host>` and sign in. The UI redirects to Okta's `/v1/authorize`, returns to `/auth/callback`, and lands you in the vault — exercising discovery, the token exchange, and the same-origin API proxy end to end against Okta. Sign out and confirm you're returned to `https://<ui-host>` via the authorization server's `end_session_endpoint`. If the callback errors, check that `https://<ui-host>/auth/callback` is a registered sign-in redirect URI on the UI web application and that `APP_URL` exactly matches the UI origin.

## Not covered

The exclusions from the [main guide](./self-hosting.md#not-covered--deferred) apply here too (multi-region Neon, alternative messaging backends).

If your IdP speaks **SAML 2.0** and you'd rather integrate it natively than bridge through Okta's OIDC endpoints, Temper can also act as a **native SAML service provider** — it fronts your SAML IdP with a built-in OAuth Authorization Server. See [Self-Hosting with a SAML IdP](./self-hosting-saml.md). (This Okta/OIDC guide remains the simplest path when Okta is already your OIDC issuer.)
