# Enterprise Install

**For operators** — someone doing a ground-up enterprise install. This is an annotated
variant of [Self-hosting Temper](./self-host-temper.md), not a standalone guide. The base
playbook covers the core API + MCP deployment; this one adds the enterprise-specific
configuration on top: SAML SSO, group provisioning, and org bootstrap. Start with the base
playbook for the deploy, then follow this one for the enterprise extensions.

By the end you will have a deployed Temper instance behind Okta SAML SSO, with a first
system admin, instance settings, an everyone-team every member auto-joins, and an
org-identity cognitive map — ready for your users to sign in and work.

**Primary path:** Temper's native Authorization Server fronting your Okta SAML app (see
[Self-hosting with SAML](./self-host-with-saml.md)). Auth0 and Okta-OAuth are noted
variants — see [Self-hosting Temper](./self-host-temper.md) and
[Self-hosting with Okta](./self-host-with-okta.md) if your organization uses one of those
instead.

## Prerequisites

- **The base deployment stood up.** This playbook assumes you have followed
  [Self-hosting Temper](./self-host-temper.md) and have the API + MCP surfaces deployed.
- **An `embed`-capable `temper` binary.** Org bootstrap's `cogmap create` / `cogmap reconcile`
  embed the charter client-side (ONNX). The default install bundles it; if you built from
  source, reinstall with `cargo install --path crates/temper-cli --locked --force`.
- **`psql` and `DATABASE_URL_UNPOOLED`** for the DB-only steps — running migrations and the
  irreducible SQL root step that promotes the first system admin.
- **Okta admin access** to create the SAML app and configure the AS.
- **A Vercel project** to host the API + MCP surfaces (and, optionally, a second project for
  the web UI).
- **A Neon project** (PostgreSQL 17) for the instance database.
- **Familiarity with the auth-identity contract** — see
  [Auth Identity](../concepts/auth-identity.md) for the issuer/audience agreement rules the
  server enforces at boot.
- **Familiarity with the trust boundary** — see
  [The Trust Boundary](../concepts/trust-boundary.md) for the two-gate access model every
  call crosses.

## What you end up with

| Outcome | Produced by |
|---------|-------------|
| Deployed API + MCP behind Okta-SAML SSO | [Self-hosting Temper](./self-host-temper.md) deploy + [Self-hosting with SAML](./self-host-with-saml.md) |
| A first system admin | the SQL root step (irreducible) |
| Instance settings (name, gating, mode) | `temper admin settings` |
| An everyone-team every member auto-joins | `temper team create … --auto-join-role watcher` |
| An org-identity telos-charter cognitive map, born + bound | `temper cogmap create` → `temper cogmap reconcile` → `temper cogmap bind` |
| (optional) The web UI | [Self-hosting Temper](./self-host-temper.md) |
| (deferred) The Eve steward | deferred — not sequenced in this playbook |

## Four phases

- **(A) Install the `temper` binary** — a prerequisite for every phase below; see
  [Install Temper](./install-temper.md).
- **(B) Backend deploy + auth** — stand up the API + MCP surfaces on Vercel + Neon, wired to
  Okta SAML. See [Self-hosting Temper](./self-host-temper.md) and
  [Self-hosting with SAML](./self-host-with-saml.md).
- **(C) Org bootstrap** — take the blank-but-stable install to a usable org: first admin,
  instance settings, everyone-team, org-identity cognitive map. See
  [Bootstrap an Org](./bootstrap-an-org.md).
- **(D) Agents [deferred]** — deploying an Eve agent (the team-self-cognition steward) against
  the instance. Not sequenced in this playbook.

## Environment matrix

One consolidated table across all three surfaces. Set the **api+mcp** and SAML-AS rows
before Phase B; the **temper-ui** rows only if you deploy the optional UI. The **eve** column
is **deferred** — surfaced for completeness, not a step in this playbook.

Sources: [Self-hosting Temper](./self-host-temper.md),
[Self-hosting with SAML](./self-host-with-saml.md).

| Variable | temper-cloud (api+mcp) | temper-ui | eve (deferred) | Notes |
| --- | --- | --- | --- | --- |
| **Database** | | | | |
| `DATABASE_URL` | Yes (pooled, runtime) | Yes (same pooled string; read-only nav chrome) | — | One Neon connection string shared by api/mcp/ui |
| `DATABASE_URL_UNPOOLED` | Yes (deploy step only) | — | — | Direct Neon connection string; migrations only |
| **Auth (issuer / audience / provider)** | | | | |
| `AUTH_ISSUER` | Yes | — | — | Auth0 tenant, or `AS_ISSUER` value in the SAML path |
| `JWKS_URL` | Yes | — | — | Auth0 JWKS, or `https://<instance>/oauth/jwks` in the SAML path |
| `AUTH_AUDIENCE` | Yes | — | — | Must equal `AS_AUDIENCE` / `MCP_AUDIENCE` / UI `OIDC_AUDIENCE` |
| `AUTH_PROVIDER_NAME` | Yes | — | — | `auth0`, or `saml:<idp-key>` in the SAML path (max 32 chars) |
| `MCP_AUDIENCE` | No | — | — | **Optional.** An instance has ONE audience; both surfaces read `AUTH_AUDIENCE`. If set, must **equal** it — enforced at boot, not by discipline. |
| `MCP_CLIENT_ID` | Yes | — | — | Auth0 MCP native app client_id; n/a in the SAML path (client allowlisting is `AS_CLIENTS` instead) |
| `MCP_BASE_URL` | Yes | — | — | `https://<instance>` — used in OAuth discovery responses |
| **SAML Authorization Server (AS) block** | | | | |
| `AS_ISSUER` | Yes (SAML path) | — | — | Setting this flips the instance into AS mode |
| `AS_AUDIENCE` | Yes (SAML path) | — | — | Must equal `AUTH_AUDIENCE` |
| `AS_SIGNING_KEY_PKCS8` | Yes (SAML path) | — | — | Ed25519 signing key, PKCS#8 PEM — secret |
| `AS_SIGNING_KID` | Yes (SAML path) | — | — | Key id published in the JWKS |
| `AS_CLIENTS` | Yes (SAML path) | — | — | JSON `client_id → [redirect_uris]` allowlist; unset = fail-closed |
| `AS_ACCESS_TTL_SECONDS` | Optional (default `900`) | — | — | Access-token lifetime |
| `AS_REFRESH_TTL_SECONDS` | Optional (default `2592000`, 30d) | — | — | Refresh-token lifetime |
| **Group provisioning / reconcile channel (SAML Phase 2)** | | | | |
| `INTERNAL_RECONCILE_SECRET` | Yes (SAML path; shared AS+API) | — | — | Same value on both; unset disables reconcile (auth still works) |
| `INTERNAL_RECONCILE_URL` | Yes (SAML path; AS side) | — | — | Full URL of the API's `/internal/saml/reconcile` |
| **Slack account link (optional; needed only to run the @temper mention agent)** | | | | |
| `SLACK_LINK_CLIENT_ID` | Yes (Slack path) | — | — | OAuth client the link flow authorizes as. Auth0: a native/PKCE app's client_id. SAML path: a `client_id` present in `AS_CLIENTS` |
| `SLACK_LINK_SECRET` | Yes (Slack path; shared API+agent) | — | — | Shared secret gating `/internal/slack/link-state`; same value on the mention agent. Unset ⇒ the endpoint is disabled (auth still works) |
| `PUBLIC_BASE_URL` | Yes (Slack path) | — | — | `https://<instance>` — the origin the link `redirect_uri` is built from³. All four unset together is the supported "no Slack" state |
| `SLACK_VAULT_ENC_KEY` | Yes (Slack path) | — | — | 32-byte base64 AEAD key (`openssl rand -base64 32`) encrypting each stored per-user refresh token. Malformed ⇒ the whole Slack flow disables. Rotation is flag-day today (users re-link) |
| **Storage / build** | | | | |
| `BLOB_READ_WRITE_TOKEN` | Yes | — | — | Vercel Blob token for the upload/extract/embed pipeline |
| `SQLX_OFFLINE` | Yes (build) | — | — | Must be `true` |
| **Optional / situational (api+mcp)** | | | | |
| `ENABLE_SWAGGER` | Optional | — | — | Exposes `/swagger-ui` in non-production |
| `PORT` | Optional | — | — | Platform-injected by Vercel |
| `CORS_ORIGINS` | Situational | — | — | Only for a *separate* cross-origin browser client — the bundled UI same-origin-proxies and does not need it |
| **UI connectivity** | | | | |
| `API_BASE_URL` | — | Yes | — | The API's **own** origin, not the UI's public origin (loop-detection warning in the base playbook) |
| `APP_URL` | — | Yes | — | The UI's own public origin |
| **UI OIDC client** | | | | |
| `OIDC_ISSUER` | — | Yes¹ | — | Must resolve the same issuer as `AUTH_ISSUER` / `AS_ISSUER` |
| `OIDC_CLIENT_ID` | — | Yes¹ | — | `temper-ui` in the SAML AS path |
| `OIDC_CLIENT_SECRET` | — | Yes¹ (omit in the SAML AS path) | — | The AS registers `temper-ui` as a public PKCE client — no secret |
| `OIDC_AUDIENCE` | — | Situational | — | Required for Auth0; omit for Okta custom AS / the SAML AS (carried implicitly) |
| `OIDC_PUBLIC_CLIENT` | — | Yes (SAML AS path) | — | Declares the secret-less PKCE path; without it the UI fails fast at startup |
| `OIDC_DISCOVERY_URL`² | — | Yes (SAML AS path) | — | Points the UI at the AS's RFC 8414 metadata — the AS has no `/.well-known/openid-configuration` |
| **Session / storefront** | | | | |
| `SESSION_SECRET` | — | Yes | — | ≥32 bytes of entropy (64-char hex or 44-char base64) |
| `STOREFRONT_ENABLED` | — | Optional | — | Set falsy to disable the public marketing route group on app-only installs |
| **Eve (DEFERRED — not a step in this playbook)** | | | | |
| `TEMPER_MCP_URL` | — | — | Yes | The temper-mcp endpoint, e.g. `https://<instance>/mcp` |
| `TEMPER_API_URL` | — | — | Yes | The temper REST base, e.g. `https://<instance>` |
| `TEMPER_M2M_CLIENT_ID` | — | — | Production | The agent's machine-principal client id. **Register it first** — authentication is fail-closed, with no just-in-time create. On this (AS-mode) install, mint it with `temper admin machine issue`, which returns a `tmpr_…` id |
| `TEMPER_M2M_CLIENT_SECRET` | — | — | Production | The one-time secret from `issue`. Never in code; rotate with `rotate-secret` |
| `TEMPER_M2M_TOKEN_URL` | — | — | Production | The issuer's token endpoint. **On this install that is your own instance's `/oauth/token`** — Temper *is* the Authorization Server (`AS_ISSUER` set) |
| `TEMPER_M2M_AUDIENCE` | — | — | External IdP only | **Omit it here.** Auth0 requires an audience; Temper's own AS ignores a request-supplied one entirely and mints with `AS_AUDIENCE` |
| `TEMPER_CONNECT_CONNECTOR` | — | — | Fallback | Vercel Connect connector id. Used only when `TEMPER_M2M_CLIENT_ID` is unset, and it **cannot mint an app token against an Auth0-fronted instance** |
| `TEMPER_TOKEN` | — | — | Dev only | Pre-obtained token; drives `eve dev`. Not for production |
| `STEWARD_MODEL` | — | — | Optional | The agent's primary model (default `minimax/minimax-m3`). Resolved at **build** time, so a change needs a **redeploy**; an unknown id fails the build |
| `STEWARD_MODEL_FALLBACKS` | — | — | Optional | Comma-separated, tried in order after the primary fails. Covers **availability** (5xx, rate limit), never quality |

¹ Back-compat fallback: if `OIDC_*` are unset, the UI falls back to the canonical deployment's
`AUTH0_*` variables — see [Self-hosting Temper](./self-host-temper.md). Self-hosters on the
SAML-primary path should set `OIDC_*` directly.

² `OIDC_DISCOVERY_URL` is required for the UI on the SAML-AS path — the AS has no
`/.well-known/openid-configuration`, so the UI needs the explicit discovery URL.

³ **The link client's `redirect_uri` must be registered, or the IdP refuses the authorize request
before temper ever sees it.** The flow derives it as `<PUBLIC_BASE_URL>/api/auth/slack/callback`.
On this (AS-mode) install, add that exact URL to `SLACK_LINK_CLIENT_ID`'s entry in `AS_CLIENTS`
— unset `AS_CLIENTS` is fail-closed. On an Auth0-fronted install, add it to the application's
**Allowed Callback URLs**. It is an exact-match allowlist on both: a trailing slash or an `http://`
scheme is a different URL.

### Must-match by construction

> **The audience and issuer joins are enforced at boot.** Temper parses them once and **refuses to
> start** if they disagree, naming the offending variable and the relation it must satisfy. You do
> not have to hold this table in your head — but the values below are what it checks, and
> `JWKS_URL` must be `$AS_ISSUER/oauth/jwks` on an AS instance.

| Join | Values that must be equal |
|------|---------------------------|
| Audience | `AS_AUDIENCE` = `AUTH_AUDIENCE` = `MCP_AUDIENCE` (if set) = UI `OIDC_AUDIENCE` |
| Issuer | `AS_ISSUER` = `AUTH_ISSUER`; UI `OIDC_ISSUER` resolves the same issuer |
| Provider label | `AUTH_PROVIDER_NAME` = `saml:<idp-key>` |
| Reconcile secret | `INTERNAL_RECONCILE_SECRET` identical on the AS and API env (same Vercel project) |
| Database | `DATABASE_URL` (pooled) shared api/mcp/ui; `DATABASE_URL_UNPOOLED` migrations only |

`temper admin saml provision` renders the `AS_*` + reconcile block so these are consistent by
construction — it is the reason the SAML env is emitted, not hand-written.

## The timeline

`temper admin saml provision` is an **inert emitter** — it never touches a running instance. It
runs early (step 3, before the deploy) *only* because it generates the Ed25519 AS signing key and
the `INTERNAL_RECONCILE_SECRET` that must already be in the env when the backend deploys. Emitting
early does not mean applying early: `provision` produces two artifacts that land at different
points in the timeline. The **env bundle** (`--env-out`) is consumed pre-deploy, at step 4 (Vercel
env). The **`kb_saml_idp` INSERT** (`--sql-out`) can only be applied post-migrate, at step 6 —
`kb_saml_idp` is a table created by the migrations run at step 5, so applying it any earlier is
impossible, not just out of order.

| # | Step | Owner | Detail link |
| --- | --- | --- | --- |
| 1 | Provision Neon (PG17, `vector` + `pg_uuidv7`, pooled/unpooled) | manual | [Self-hosting Temper](./self-host-temper.md) |
| 2 | Register Okta SAML app; capture cert / SSO URL / entity ids / group attribute statement | manual | [Self-hosting with SAML](./self-host-with-saml.md) + [Okta SAML app](#okta-saml-app) below |
| 3 | `temper admin saml provision` → generate keys, `--env-out` bundle, `--sql-out` kb_saml_idp SQL (inert; early for the env keys) | `saml-setup.sh` (emit) | [Self-hosting with SAML](./self-host-with-saml.md) |
| 4 | Set Vercel env (matrix + emitted bundle) on api + mcp | manual | [Environment matrix](#environment-matrix) |
| 5 | Deploy backend; `sqlx migrate run` against `DATABASE_URL_UNPOOLED` | manual | [Self-hosting Temper](./self-host-temper.md) |
| 6 | Apply the `kb_saml_idp` row (`saml-setup.sh --apply-db`, or `psql` the `--sql-out` file by hand) | `saml-setup.sh` (`--apply-db`) | [Self-hosting with SAML](./self-host-with-saml.md) |
| 7 | First admin signs in via SAML → JIT `kb_profiles` row | manual | [Self-hosting with SAML](./self-host-with-saml.md) |
| 8 | SQL root step: gating team + first admin; VERIFY `has_system_access(<uuid>)` AND `is_system_admin(<uuid>)` both true | `system-bootstrap.sh --run-root` | [Bootstrap an Org](./bootstrap-an-org.md) |
| 9 | `temper admin settings` (instance name, gating team, mode) | `system-bootstrap.sh` | [Bootstrap an Org](./bootstrap-an-org.md) |
| 10 | `temper team create everyone --auto-join-role watcher` | `system-bootstrap.sh` | [Bootstrap an Org](./bootstrap-an-org.md) |
| 11 | `temper admin saml map-group` (after teams exist) | `saml-setup.sh` (emit / `--apply-db`) | [Self-hosting with SAML](./self-host-with-saml.md) |
| 12 | `temper admin saml verify` | `saml-setup.sh` | [Self-hosting with SAML](./self-host-with-saml.md) |
| 13 | Telos-charter: `temper cogmap create` → `temper cogmap reconcile` → bind `+everyone` | `system-bootstrap.sh` | [Bootstrap an Org](./bootstrap-an-org.md) |
| 14 | (optional) UI deploy: confidential OIDC client, `API_BASE_URL`, `SESSION_SECRET` | manual | [Self-hosting Temper](./self-host-temper.md) |
| 15 | Verify: health, `temper auth login`, resource round-trip | manual | [Self-hosting Temper](./self-host-temper.md) |
| — | → team-self-cognition + Eve steward: **DEFERRED** | — | deferred |

**The expected path.** The happy path is: run `saml-setup.sh` (step 3, default emit — writes the
env bundle consumed at step 4 and holds the `kb_saml_idp` SQL for step 6), do the two platform
steps by hand (4–5, Vercel env + deploy/migrate), then run `system-bootstrap.sh --run-root`
(steps 8–10 and 13) and re-run `saml-setup.sh --apply-db` (steps 6, 11, and 12 — applies the
`kb_saml_idp` row, maps the now-existing teams' IdP groups, and verifies against the live DB).
The numbered breakdown above is the reference an operator reads to understand what each script
does, or falls back to when running by hand. The two scripts are kept separate so
`system-bootstrap.sh` (steps 8–10, 13) works unchanged for Auth0/Okta-OAuth installs, which swap
steps 2–3, 6, and 11–12 for the Auth0 app registration documented in
[Self-hosting Temper](./self-host-temper.md) instead.

### Okta SAML app

> In Okta, create a **SAML 2.0 app** and capture four values off it:
>
> - the **SSO URL** → `idp_sso_url` / `--idp-sso-url`
> - the **signing certificate** (PEM) → `idp_cert_file` / `--idp-cert-file`
> - the **IdP entity id** → `idp_entity_id` / `--idp-entity-id`
> - a **group attribute statement** exposing the user's groups → `groups_attr` / `--groups-attr`
>   (e.g. `groups`)
>
> This note covers only what to pull out of Okta's app screen. The generic SAML-IdP side — the SP
> ACS URL and entity id Temper's AS expects the IdP to send assertions to — is documented in
> [Self-hosting with SAML](./self-host-with-saml.md), and is the same regardless of which IdP you use.

## Traps

Five ways this install silently misbehaves instead of failing loudly. Each has bitten a real
install; read this before step 8.

> **Admission and admin-ness are two independent rows, and each reads exactly one table.**
> `has_system_access` reads `kb_principal_standing` (an `approved` row); `is_system_admin` reads
> `kb_principal_governance`. Neither reads team membership, and a fresh instance has neither row,
> so it denies **everyone** — silently, as 403s rather than a startup failure. **Write both rows
> at step 8** and verify **both** predicates — `has_system_access(<uuid>)` and
> `is_system_admin(<uuid>)` each returning true — before moving on, not just that the SQL ran.
> One without the other is a silent half-state.
>
> **`API_BASE_URL` pointed at the UI's own public origin creates a self-proxy loop → `508 Loop
> Detected`.** It must be the API backend's own distinct origin — its `*.vercel.app` URL or a
> dedicated `api.` subdomain — never the shared public domain the UI also serves
> ([Self-hosting Temper](./self-host-temper.md)).
>
> **`AS_CLIENTS` unset rejects every `/oauth/authorize` call (fail-closed); `INTERNAL_RECONCILE_SECRET`
> unset silently disables group provisioning while auth still works.** The first fails loud, the
> second doesn't — nothing errors, groups just never sync, so verify reconcile explicitly rather
> than trusting a clean login ([Self-hosting with SAML](./self-host-with-saml.md)).
>
> **`cogmap create` / `cogmap reconcile` require an `embed`-feature `temper` binary.** A
> non-`embed` build fails with a clear `requires the 'embed' feature` error rather than a cryptic
> one, but only at step 13, well after the rest of the install has succeeded — check this
> up front instead ([Bootstrap an Org](./bootstrap-an-org.md)).
>
> **Migrations are a deploy step, not a startup step — the API never auto-migrates.** Run
> `sqlx migrate run` against `DATABASE_URL_UNPOOLED` (step 5) yourself, and back up the database
> first — there is no automatic rollback if a migration fails partway
> ([Self-hosting Temper](./self-host-temper.md)).

## Scripted vs. manual, and what's deferred

The **expected path** is the two scripts, not the numbered table read step-by-step — the table
is the reference an operator falls back to when a script needs debugging or the install deviates
from the happy path (SAML variant swaps, a failed step to re-run by hand, etc.).

| Steps | Automated by | Status |
| --- | --- | --- |
| 8–10, 13 | `system-bootstrap.sh --run-root` | Exists today |
| 3, 6, 11, 12 | `saml-setup.sh` (`--apply-db` for 6, 11, 12) | Exists today |
| 1–2, 4–5, 7, 14–15 | — (manual) | Platform-console and human-in-the-loop steps: provisioning Neon and the Okta app, setting Vercel env, deploying, the first SAML login, and the optional UI deploy/verify — none of these are things a script can safely do on an operator's behalf |

**What's deferred beyond this playbook** — the roadmap tail, not steps to sequence here:

- **Eve / machine-to-machine auth.** The `app` principal needs `client_credentials` (M2M) support
  that doesn't exist yet; until then Eve can't reach temper-mcp unattended.
- **`plan`/`diff` applier semantics.** `system-bootstrap.sh` has no state backend — re-applying a
  profile converges because every step is idempotent, but there's no Terraform-like plan/diff
  preview ([Bootstrap an Org](./bootstrap-an-org.md)).
- **SCIM (Phase 3).** Group provisioning today is JIT on login; immediate deprovisioning needs
  SCIM, not yet available ([Self-hosting with SAML](./self-host-with-saml.md)).
- **Cogmap-write-by-team-role.** Authorial (write) RBAC for team contexts and team cognitive maps
  is still undefined — de facto, any team member can write, not just admins/owners. This playbook's
  `is_system_admin` gate covers the L0 kernel only, not team-scoped cogmaps.

## Further reading

- **The base deployment this playbook extends:**
  [Self-hosting Temper](./self-host-temper.md).
- **The org bootstrap steps in detail:**
  [Bootstrap an Org](./bootstrap-an-org.md).
- **The auth-identity contract (issuer, audience, boot enforcement):**
  [Auth Identity](../concepts/auth-identity.md).
- **The trust boundary every call crosses:**
  [The Trust Boundary](../concepts/trust-boundary.md).
- **SAML SSO configuration:**
  [Self-hosting with SAML](./self-host-with-saml.md).
- **Okta OAuth configuration:**
  [Self-hosting with Okta](./self-host-with-okta.md).
- **What the architecture fixes vs. what a deployment chooses:**
  [temperkb.io/operating/deployment](https://temperkb.io/operating/deployment).
