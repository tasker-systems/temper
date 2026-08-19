# Okta Self-Hosting Addendum + Guide Cleanup + SAML Research — Design

**Date:** 2026-06-24
**Branch:** `jct/okta-self-hosting-addendum`
**Status:** Approved design, pre-implementation

## Context

`docs/guides/self-hosting.md` documents standing up a self-hosted Temper instance against
**Auth0**. It has never been dogfooded, so its claims were unverified. The goals of this work:

1. **Cleanup** — code-grounded accuracy audit of the existing guide; fix all drift.
2. **Okta addendum** — a separate guide for hosting with **Okta** in enterprise contexts.
3. **SAML research** — a forward-looking research spec for a future SSO-via-SAML task
   (context only; not built here).

The research that grounds this design is captured below with `file:line` citations against the
repo at the time of writing. The headline discovery: temper's auth is **provider-neutral at the
JWT-validation layer** (issuer + audience + JWKS), but several **peripheral paths are
Auth0-shaped** (CLI init URL templating, the userinfo fallback URL), which is what makes the Okta
story non-trivial.

## Ground Truth (audit results)

The existing guide is **mostly accurate**. Routes, CLI flags, and `config.toml` shape all
verified correct. Genuine drift found:

| Claim | Verdict | Reality (citation) |
| ----- | ------- | ------------------ |
| Line 274: `temper login` is "Auth0 device authorization flow" | ❌ Wrong | It is **Authorization Code + PKCE** with a loopback relay (`crates/temper-client/src/login.rs:1`; `grant_type=authorization_code` + `code_verifier` at login.rs:104-106) |
| Line 227 table: env var `TEMPER_PROVIDER_ENV` | ❌ Wrong | The actual env var is **`TEMPER_PROVIDER`** (the Rust *const* is named `TEMPER_PROVIDER_ENV` but its value is `"TEMPER_PROVIDER"`, `crates/temper-client/src/auth.rs:15`) |
| `PORT` | ❌ Missing | API reads `PORT`, defaults 3000 (`crates/temper-api/src/config.rs:58`). Platform-injected on Vercel; relevant only for local runs |
| `DATABASE_URL_UNPOOLED`, `API_BASE_URL`, `SQLX_OFFLINE` | ⚠️ Already correctly hedged | Doc already scopes these to migrations / temper-ui / build env respectively. No change needed |

Provider-neutrality confirmation (drives the Okta mapping):
- JWT validation uses only `auth_issuer` + `auth_audience` + JWKS (`crates/temper-api/src/middleware/auth.rs:76-84`). `auth_provider_name` is used **only** as a claims label (auth.rs:118) and the email-cache lookup key (auth.rs:95) — it does **not** gate validation.
- `/oauth/register` (MCP DCR proxy) returns the pre-registered `MCP_CLIENT_ID` (`crates/temper-mcp/src/discovery.rs`, router.rs:50).

Auth0-shaped peripheral paths (the Okta gotchas):
- **CLI init is hardcoded to Auth0.** `provider_and_cloud_sections` writes `provider = "auth0"`, `name = "auth0"`, `authorize_url = https://{domain}/authorize`, `token_url = https://{domain}/oauth/token` (`crates/temper-cli/src/commands/init.rs:420-431`). There is no way for `temper init` to emit Okta's `/oauth2/{id}/v1/*` path shapes.
- **Userinfo fallback is Auth0-shaped.** `fetch_email_from_userinfo` builds `{issuer-trimmed}/userinfo` (`auth.rs:189`). For Auth0 (`https://tenant.auth0.com/`) → `.../userinfo` ✅. For an Okta custom auth server (`https://org.okta.com/oauth2/{id}`) → `.../oauth2/{id}/userinfo`, which **404s** — Okta's real endpoint is `.../oauth2/{id}/v1/userinfo`. Consequence: on Okta the email-claim **must** be present on the access token; the fallback won't save it.

## Deliverable 1 — `self-hosting.md` fixes

Three edits, all from the audit table above:

1. **Line 274** (critical): replace the device-flow sentence with an accurate description of the
   Authorization Code + PKCE flow (browser → provider `/authorize` → `/api/auth/cli-callback`
   relay → localhost listener → token exchange).
2. **Line ~227**: rename the env var `TEMPER_PROVIDER_ENV` → `TEMPER_PROVIDER` in the overrides
   table.
3. **`PORT`**: add a one-line note (platform-injected on Vercel; defaults to 3000 for local runs).
   Keep it out of the "required" rows since operators don't set it on Vercel.

No other changes — the guide's other claims verified accurate.

## Deliverable 2 — `docs/guides/self-hosting-okta.md` (addendum)

A **separate file** (per decision), scoped to the **auth deltas only**. Neon provisioning, Vercel
deployment, CLI install, and verification are identical to the main guide and are referenced, not
repeated. The main guide gets a single pointer link to this addendum.

### Outline

1. **Intro + when to use** — choose Okta when the org standardizes on Okta IdP. Scope note: only
   the Auth0 → Okta auth substitution differs; all other steps follow the main guide.

2. **Prerequisite gate: API Access Management (mandatory)** — Temper *requires* a custom
   audience (`AUTH_AUDIENCE`/`MCP_AUDIENCE`). Okta's free **org** authorization server cannot set
   a custom audience; only a **custom authorization server** can, and that requires the **API
   Access Management** add-on (paid in production). Lead with this — it's the enterprise gating
   constraint.

3. **Provision the custom authorization server**
   - Set **Audience** = the value you will use for `AUTH_AUDIENCE` / `MCP_AUDIENCE`.
   - Issuer is `https://<okta-domain>/oauth2/<authServerId>` — **no trailing slash** (contrast:
     Auth0 requires a trailing slash). `AUTH_ISSUER` must match the token `iss` exactly.
   - **Add an access policy + rule** allowing grant types `authorization_code` and
     `refresh_token`. Custom auth servers default-deny (no policy match → auth fails); this has no
     Auth0 equivalent.

4. **⚠️ Add an `email` claim to the access token (mandatory on Okta)** — temper's userinfo
   fallback URL is Auth0-shaped and 404s on Okta (`auth.rs:189`), so the access token *must* carry
   `email`. In the custom auth server's claims config, add an access-token claim mapping
   `email` → `user.email`. Without this, login fails with "Token missing email claim and userinfo
   lookup failed."

5. **CLI native application** — OIDC **Native** app; grant types **Authorization Code** +
   **Refresh Token** (PKCE is implied for native); redirect URI
   `https://<instance>/api/auth/cli-callback`. **Not** the device authorization grant — temper uses
   the loopback-relay authcode+PKCE flow (Deliverable-1 finding). Assign the app to the custom
   auth server's access policy.

6. **MCP native application** — second Native app; allowed callbacks for the MCP clients
   (`https://claude.ai/api/mcp/auth_callback`, `https://claude.com/api/mcp/auth_callback`,
   `http://localhost`); its `client_id` becomes `MCP_CLIENT_ID`. **Note the DCR sidestep:** raw
   Okta + MCP integrations hit a 403 on dynamic client registration against custom auth servers;
   temper's `/oauth/register` proxy returns the pre-registered `MCP_CLIENT_ID`, so temper avoids
   that failure mode entirely.

7. **Env var mapping (Okta column)** — table mirroring the main guide's contract:

   | Variable | Okta value |
   | -------- | ---------- |
   | `AUTH_ISSUER` | `https://<okta-domain>/oauth2/<authServerId>` — **no trailing slash** |
   | `JWKS_URL` | `https://<okta-domain>/oauth2/<authServerId>/v1/keys` |
   | `AUTH_AUDIENCE` | the custom auth server's **Audience** value |
   | `MCP_AUDIENCE` | same as `AUTH_AUDIENCE` (one auth server) |
   | `AUTH_PROVIDER_NAME` | **keep `auth0`** — it's a label + email-cache key, not a validation switch; the client `Provider` enum only models Auth0 |
   | `MCP_CLIENT_ID` | MCP native app client_id |
   | `MCP_BASE_URL` | `https://<instance>` |

8. **⚠️ Configure the CLI by hand** — `temper init` is hardcoded to Auth0 URL shapes and writes
   `provider = "auth0"` (`init.rs:420-431`); it cannot emit Okta's `/oauth2/<id>/v1/*` paths. Okta
   users must **hand-write** `~/.config/temper/config.toml`. Provide the exact block:

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

   (Keep `provider`/`name` as `auth0` — it must match between `[auth]` and the provider block, and
   the value is a label only.)

9. **Known limitation / follow-up** — flag the userinfo-URL Auth0-assumption (`auth.rs:189`) as a
   latent bug affecting any non-Auth0 OIDC provider, with a pointer that a future code fix
   (issuer-shape-aware userinfo URL, or provider-configurable userinfo endpoint) would remove the
   mandatory-email-claim workaround. Not fixed on this docs branch.

## Deliverable 3 — SAML/SSO research spec (vault)

Saved via `temper resource create --type research --context temper`. Forward-looking; no build.

### Outline

- **Problem framing** — temper authenticates API/CLI/MCP calls with **bearer JWTs validated
  against JWKS (OIDC)**. SAML is a browser-POST XML-assertion protocol with no bearer token, so it
  cannot directly authenticate temper's non-browser surfaces.
- **Architecture A (recommended): upstream federation, zero temper change** — the enterprise IdP
  federates into Okta/Auth0 via inbound SAML; the OIDC broker (Okta/Auth0) remains the issuer
  temper sees. Users SSO with enterprise credentials; temper still receives OIDC JWTs. `samael` not
  needed.
- **Architecture B: temper as a native SAML SP (`samael`'s domain)** — temper-api grows a SAML ACS
  endpoint and **must self-issue JWTs** after SAML login (temper becomes its own token issuer:
  signing keys, JWKS, token lifetimes, refresh). `samael` (`ServiceProviderBuilder`, `xmlsec`
  feature for sign/verify) is the crate. Large lift; only justified if customers refuse to put any
  OIDC broker in the path.
- **What B would touch** — new ACS route + SP metadata endpoint; SAML assertion verification
  (`xmlsec`); a token-minting service; a JWKS the existing middleware can validate against; key
  management. Effectively makes temper an identity provider for its own tokens.
- **Recommendation** — lead with Architecture A for every enterprise SSO ask; reserve B for a
  scoped future task only if a concrete customer constraint forces it.
- **Reference** — `samael`: https://github.com/njaremko/samael

## Out of Scope

### Rejected (load-bearing decisions)
- **Restructuring `self-hosting.md` into provider-neutral + per-provider tabs** — rejected in favor
  of a separate addendum, to avoid churning a guide that is otherwise verified-accurate.
- **Introducing an `okta` provider name** — rejected; validation is provider-neutral and the client
  `Provider` enum only models Auth0. `auth0` stays as the label.

### Deferred (in scope elsewhere / later)
- **Fixing the userinfo-URL Auth0-assumption** (`auth.rs:189`) — a code change, not docs; flagged as
  a follow-up task.
- **Building SAML support** — Deliverable 3 is research only.
- **`temper init` Okta support** (emit `/oauth2/<id>/v1/*` URL shapes / non-Auth0 provider) — a code
  change; the addendum works around it with hand-written config.

## Execution Note

All three deliverables' content is the code-grounded research already gathered in this session
(with exact `file:line` citations). The docs will be authored directly rather than dispatched to
context-blind subagents, since the fidelity depends on findings held in the authoring context. A
consolidated review follows at the end.
