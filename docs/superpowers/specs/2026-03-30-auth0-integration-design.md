# Auth0 Integration Design

**Date**: 2026-03-30
**Task**: I5b — temper-client Crate + CLI Auth
**Goal**: temper-cloud-cli-api-usability
**Decision**: Replace Neon Auth with Auth0 as the sole authentication provider for both CLI and web surfaces.

## Context

Neon Auth (Better Auth) uses session cookies on the Neon Auth domain. These are treated as third-party cookies by Chrome and are blocked in cross-origin contexts, making CLI authentication impossible. After evaluating Auth0, Clerk, and Auth.js:

- **Auth.js** is an OAuth client, not server — no authorize/token endpoints, no verifiable JWTs
- **Clerk** provides OAuth2 server endpoints but has no official SvelteKit SDK
- **Auth0** provides standard OAuth2/OIDC with PKCE, RS256 JWTs, JWKS, and is framework-agnostic. Okta ownership provides a path to enterprise SAML/SSO if needed.

## Auth0 Tenant

- **Tenant**: `temperkb.us.auth0.com`
- **Social connection**: Google OAuth2 (enabled, additional providers like GitHub can be added)

### Applications

| App | Type | Client ID | Purpose |
|-----|------|-----------|---------|
| `temper-cli` | Native | `mWp8znLw2MUJNCiZNl8wwBv6SPJI2mfF` | CLI PKCE authentication |
| `temper-web` | SPA | `CJsNv3MerZSZKqi14eaqrK7sAy6Eg7fM` | SvelteKit web UI (I5e) |

### API Resource

| Field | Value |
|-------|-------|
| Name | `temper-api` |
| Identifier (audience) | `https://temperkb.io/api` |
| Signing algorithm | RS256 |
| Token lifetime | 86400 (24 hours) |
| Offline access | Enabled (refresh tokens) |

## CLI PKCE Flow

Direct OAuth2 Authorization Code + PKCE against Auth0. No intermediate relay pages.

```
CLI                         Browser                     Auth0
 |                            |                           |
 |-- generate code_verifier   |                           |
 |-- generate code_challenge  |                           |
 |-- bind localhost:{port}    |                           |
 |-- open browser ----------->|                           |
 |   GET /authorize           |                           |
 |   ?response_type=code      |                           |
 |   &client_id=CLI_ID        |                           |
 |   &redirect_uri=           |                           |
 |    http://localhost:{port}  |                           |
 |    /callback               |                           |
 |   &code_challenge=...      |                           |
 |   &code_challenge_method=  |                           |
 |    S256                    |                           |
 |   &audience=               |                           |
 |    https://temperkb.io/api |                           |
 |   &scope=openid+profile    |                           |
 |    +email+offline_access   |                           |
 |                            |                           |
 |                            |<-- Universal Login ------>|
 |                            |    (Google sign-in)       |
 |                            |                           |
 |<-- GET /callback?code=... -|                           |
 |                            |                           |
 |-- POST /oauth/token ---------------------------------->|
 |   grant_type=              |                           |
 |    authorization_code      |                           |
 |   &code=...                |                           |
 |   &code_verifier=...       |                           |
 |   &redirect_uri=           |                           |
 |    http://localhost:{port}  |                           |
 |    /callback               |                           |
 |   &client_id=CLI_ID        |                           |
 |                            |                           |
 |<-- { access_token,        |                           |
 |      id_token,             |                           |
 |      refresh_token,        |                           |
 |      expires_in } ------------------------------------ |
 |                            |                           |
 |-- store tokens in          |                           |
 |   ~/.config/temper/        |                           |
 |   auth.json                |                           |
 |-- close local server       |                           |
 |-- done                     |                           |
```

### Token Storage (`~/.config/temper/auth.json`)

```json
{
  "access_token": "eyJ...",
  "id_token": "eyJ...",
  "refresh_token": "v1...",
  "expires_at": 1711900800,
  "provider": "auth0"
}
```

### Token Refresh

The existing temper-client pre-expiry refresh logic (5-minute window) calls Auth0's `/oauth/token` with `grant_type=refresh_token`. No browser interaction required.

## Provider-Agnostic Configuration

The CLI and temper-client read all auth configuration from `~/.config/temper/config.toml`. This is a first-class design constraint: on-prem or alternative deployments can point at any standard OAuth2/OIDC provider by changing this file.

```toml
[auth]
provider = "auth0"

[auth.providers.auth0]
authorize_url = "https://temperkb.us.auth0.com/authorize"
token_url = "https://temperkb.us.auth0.com/oauth/token"
client_id = "mWp8znLw2MUJNCiZNl8wwBv6SPJI2mfF"
audience = "https://temperkb.io/api"
scopes = ["openid", "profile", "email", "offline_access"]

# On-prem example:
# [auth.providers.keycloak]
# authorize_url = "https://sso.example.com/realms/temper/protocol/openid-connect/auth"
# token_url = "https://sso.example.com/realms/temper/protocol/openid-connect/token"
# client_id = "temper-cli"
# audience = "temper-api"
# scopes = ["openid", "profile", "email", "offline_access"]
```

The temper-client code must:
- Ship with compiled-in defaults: `auth.provider = "auth0"` pointing at `temperkb.us.auth0.com` with the `temper-cli` client ID and `https://temperkb.io/api` audience. A fresh install should work without manual config. If `~/.config/temper/config.toml` exists, its values override the defaults.
- Read the active provider from `auth.provider`
- Look up the provider config under `auth.providers.{name}`
- Use `authorize_url`, `token_url`, `client_id`, `audience`, and `scopes` to drive the PKCE flow
- Allow config.toml overrides for on-prem or alternative deployments — but the default path is temperkb.io + Auth0

Similarly, temper-api uses environment variables (`JWKS_URL`, `AUTH_ISSUER`, `AUTH_AUDIENCE`) so server-side verification is also provider-agnostic.

## Code Changes

### Rust JWKS Middleware (`crates/temper-api/src/middleware/auth.rs`)

- Accept both RSA and OKP key types from JWKS (currently filters to OKP only)
- Add `RS256` to allowed algorithms in JWT validation
- No changes to claim extraction — `sub`, `email`, `exp` are standard OIDC claims

### TypeScript JWT Verification (`packages/temper-cloud/src/auth.ts`)

- Add `RS256` to the `algorithms` array in `jose` verification (currently `["EdDSA"]` only)

### CLI Login Flow (`crates/temper-client/src/login.rs`)

- Replace Neon Auth "sign-in/social" POST pattern with standard OAuth2 authorize URL construction
- Build `/authorize` URL with PKCE parameters, audience, and scopes from config
- Exchange authorization code for tokens at `/oauth/token` directly (no intermediate HTML pages)
- Store `refresh_token` in auth.json alongside access/id tokens
- Add refresh token exchange to the token refresh path

### Config (`crates/temper-client/src/config.rs`)

- Add `audience` field to `OAuthProviderConfig`
- Ensure all fields are read from config.toml, nothing hardcoded
- Remove debug `eprintln` statements

### Vercel Environment Variables

| Variable | Value |
|----------|-------|
| `JWKS_URL` | `https://temperkb.us.auth0.com/.well-known/jwks.json` |
| `AUTH_ISSUER` | `https://temperkb.us.auth0.com/` |
| `AUTH_AUDIENCE` | `https://temperkb.io/api` |

### Remove

- `api/auth-login.ts` — Neon Auth relay page, no longer needed
- `api/auth-callback.ts` — cross-domain cookie workaround, no longer needed
- Neon Auth environment variables from Vercel (`NEON_AUTH_URL`, `AUTH_PROVIDER_NAME`)
- `neon_auth` provider defaults from config.rs

## Testing

- `temper auth login` end-to-end: browser opens, Google sign-in, callback received, tokens stored
- `temper auth status` shows valid token with Auth0 issuer and audience claims
- `temper auth token <jwt>` still works for manual token injection
- `temper auth logout` clears auth.json
- API calls with the Auth0 access token succeed against temper-api
- Token refresh works silently when access token approaches expiry
- Integration tests against live API with Auth0 tokens

## Not In Scope

- SvelteKit web UI auth (I5e — uses the `temper-web` SPA app, same tenant)
- Custom Auth0 Universal Login branding
- RBAC / custom permissions / API scopes beyond audience
- Refresh token rotation configuration (can be enabled later via Auth0 dashboard)
- SAML/SSO enterprise features
