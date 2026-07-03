# Enterprise Install — Ground Up

This is the spine for a first real enterprise install — it flattens the phase guides into
one sequence. Each detailed step links to its phase guide; this document is the order and
the joins, not the detail.

**Primary path:** Temper's native Authorization Server fronting your Okta SAML app (see
[self-hosting-saml.md](./self-hosting-saml.md)). Auth0 and Okta-OAuth are noted variants —
see [self-hosting.md](./self-hosting.md) and [self-hosting-okta.md](./self-hosting-okta.md)
if your organization uses one of those instead.

## What you end up with

| Outcome | Produced by |
|---------|-------------|
| Deployed API + MCP behind Okta-SAML SSO | [self-hosting.md](./self-hosting.md) deploy + [self-hosting-saml.md](./self-hosting-saml.md) |
| A first system admin | the SQL root step (irreducible) |
| Instance settings (name, gating, mode) | `temper admin settings` |
| An everyone-team every member auto-joins | `temper team create … --auto-join-role watcher` |
| An org-identity telos-charter cognitive map, born + bound | `temper cogmap create` → `temper cogmap reconcile` → `temper cogmap bind` |
| (optional) The web UI | [self-hosting.md#deploy-the-ui-optional](./self-hosting.md#deploy-the-ui-optional) |
| (deferred) The Eve steward | [vercel-eve.md](./vercel-eve.md) |

## Four phases

- **(A) Install the `temper` binary** — a prerequisite for every phase below; see
  [install.md](./install.md).
- **(B) Backend deploy + auth** — stand up the API + MCP surfaces on Vercel + Neon, wired to
  Okta SAML. See [self-hosting.md](./self-hosting.md) and [self-hosting-saml.md](./self-hosting-saml.md).
- **(C) Org bootstrap** — take the blank-but-stable install to a usable org: first admin,
  instance settings, everyone-team, org-identity cognitive map. See
  [org-bootstrap.md](./org-bootstrap.md).
- **(D) Agents [deferred]** — deploying an Eve agent (the team-self-cognition steward) against
  the instance. Not sequenced in this runbook; see [vercel-eve.md](./vercel-eve.md).

## Prerequisites

- **An `embed`-capable `temper` binary.** Org bootstrap's `cogmap create` / `cogmap reconcile`
  embed the charter client-side (ONNX). The default install bundles it; if you built from
  source, reinstall with `cargo install --path crates/temper-cli --locked --force` (see
  [org-bootstrap.md § Prerequisites](./org-bootstrap.md#prerequisites)).
- **`psql` and `DATABASE_URL_UNPOOLED`** for the DB-only steps — running migrations and the
  irreducible SQL root step that promotes the first system admin.
- **Okta admin access** to create the SAML app and configure the AS.
- **A Vercel project** to host the API + MCP surfaces (and, optionally, a second project for
  the web UI).
- **A Neon project** (PostgreSQL 17) for the instance database.

## Environment matrix

One consolidated table across all three surfaces, sourced from the phase guides — this section
does not restate their prose, only where each variable lives and which other variables it must
match. Set the **api+mcp** and SAML-AS rows before Phase B; the **temper-ui** rows only if you
deploy the optional UI. The **eve** column is **DEFERRED** — surfaced for completeness, not a
step in this runbook (see [Four phases § D](#four-phases) and [vercel-eve.md](./vercel-eve.md)).

Sources: [self-hosting.md § Environment variable contract](./self-hosting.md#environment-variable-contract),
[self-hosting.md § Environment variable contract (UI project)](./self-hosting.md#environment-variable-contract-ui-project),
[self-hosting-saml.md § 4 Environment variables](./self-hosting-saml.md#4-environment-variables),
[`packages/temper-ui/.env.example`](../../packages/temper-ui/.env.example),
[vercel-eve.md § Environment contract](./vercel-eve.md#environment-contract).

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
| `MCP_AUDIENCE` | Yes | — | — | Typically the same value as `AUTH_AUDIENCE` |
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
| **Storage / build** | | | | |
| `BLOB_READ_WRITE_TOKEN` | Yes | — | — | Vercel Blob token for the upload/extract/embed pipeline |
| `SQLX_OFFLINE` | Yes (build) | — | — | Must be `true` |
| **Optional / situational (api+mcp)** | | | | |
| `ENABLE_SWAGGER` | Optional | — | — | Exposes `/swagger-ui` in non-production |
| `PORT` | Optional | — | — | Platform-injected by Vercel |
| `CORS_ORIGINS` | Situational | — | — | Only for a *separate* cross-origin browser client — the bundled UI same-origin-proxies and does not need it |
| **UI connectivity** | | | | |
| `API_BASE_URL` | — | Yes | — | The API's **own** origin, not the UI's public origin (loop-detection warning in self-hosting.md) |
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
| **Eve (DEFERRED — not a step in this runbook)** | | | | |
| `TEMPER_MCP_URL` | — | — | Yes | The temper-mcp endpoint, e.g. `https://<instance>/mcp` |
| `TEMPER_API_URL` | — | — | Yes | The temper REST base, e.g. `https://<instance>` |
| `TEMPER_SELF_COGMAP_ID` | — | — | Yes | The cognitive map this agent tends, minted at genesis |
| `TEMPER_CONNECT_CONNECTOR` | — | — | Production | Vercel Connect connector id; falls back to `TEMPER_TOKEN` when unset |
| `TEMPER_TOKEN` | — | — | Dev only | Pre-obtained token; not for production |
| `TEMPER_MCP_AUDIENCE` | — | — | Optional | Only when token audience varies by target and isn't discovery-derived |

¹ Back-compat fallback: if `OIDC_*` are unset, the UI falls back to the canonical deployment's
`AUTH0_*` variables — see [self-hosting.md](./self-hosting.md#environment-variable-contract-ui-project).
Self-hosters on the SAML-primary path should set `OIDC_*` directly.

² `OIDC_DISCOVERY_URL` is not part of this document's source A2 variable enumeration but is
required for the UI on the SAML-AS path (`self-hosting-saml.md` § 6, `.env.example`) — added here
because omitting it would misconfigure this guide's primary (SAML) path.

### Must-match by construction

| Join | Values that must be equal |
|------|---------------------------|
| Audience | `AS_AUDIENCE` = `AUTH_AUDIENCE` = `MCP_AUDIENCE` = UI `OIDC_AUDIENCE` |
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
| 1 | Provision Neon (PG17, `vector` + `pg_uuidv7`, pooled/unpooled) | manual | [self-hosting.md § Provision Neon](./self-hosting.md#provision-neon) |
| 2 | Register Okta SAML app; capture cert / SSO URL / entity ids / group attribute statement | manual | [self-hosting-saml.md](./self-hosting-saml.md) + [Okta SAML app note](#okta-saml-app) below |
| 3 | `temper admin saml provision` → generate keys, `--env-out` bundle, `--sql-out` kb_saml_idp SQL (inert; early for the env keys) | `saml-setup.sh` (emit) | [self-hosting-saml.md](./self-hosting-saml.md) |
| 4 | Set Vercel env (matrix + emitted bundle) on api + mcp | manual | [Environment matrix](#environment-matrix) |
| 5 | Deploy backend; `sqlx migrate run` against `DATABASE_URL_UNPOOLED` | manual | [self-hosting.md § Run migrations](./self-hosting.md#run-migrations) |
| 6 | Apply the `kb_saml_idp` row (`--apply`, or `psql` the `--sql-out` file) | `saml-setup.sh` (apply) | [self-hosting-saml.md](./self-hosting-saml.md) |
| 7 | First admin signs in via SAML → JIT `kb_profiles` row | manual | [self-hosting-saml.md](./self-hosting-saml.md) |
| 8 | SQL root step: gating team + first admin; VERIFY `is_system_admin(<uuid>) = true` | `system-bootstrap.sh --run-root` | [org-bootstrap.md § 0](./org-bootstrap.md#0-the-irreducible-sql-root-step-operator-with-db-credentials) |
| 9 | `temper admin settings` (instance name, gating team, mode) | `system-bootstrap.sh` | [org-bootstrap.md § 1](./org-bootstrap.md#1-instance-settings) |
| 10 | `temper team create everyone --auto-join-role watcher` | `system-bootstrap.sh` | [org-bootstrap.md § 2](./org-bootstrap.md#2-create-the-everyone-team) |
| 11 | `temper admin saml map-group` (after teams exist) | `saml-setup.sh` (emit/apply) | [self-hosting-saml.md](./self-hosting-saml.md) |
| 12 | `temper admin saml verify` | `saml-setup.sh` | [self-hosting-saml.md](./self-hosting-saml.md) |
| 13 | Telos-charter: `temper cogmap create` → `temper cogmap reconcile` → bind `+everyone` | `system-bootstrap.sh` | [org-bootstrap.md §§ 3–5](./org-bootstrap.md#3-birth-the-org-identity-cognitive-map) |
| 14 | (optional) UI deploy: confidential OIDC client, `API_BASE_URL`, `SESSION_SECRET` | manual | [self-hosting.md § Deploy the UI (optional)](./self-hosting.md#deploy-the-ui-optional) |
| 15 | Verify: health, `temper login`, resource round-trip | manual | [self-hosting.md § Verify](./self-hosting.md#verify) |
| — | → team-self-cognition + Eve steward: **DEFERRED** | — | [vercel-eve.md](./vercel-eve.md) |

**The expected path.** The happy path is: run `temper admin saml provision` (step 3), do the two
platform steps by hand (4–5, Vercel env + deploy/migrate), then run the two scripts — `saml-setup.sh`
for steps 6, 11, and 12, and `system-bootstrap.sh --run-root` for steps 8–10 and 13. The numbered
breakdown above is the reference an operator reads to understand what each script does, or falls
back to when running by hand. The two scripts are kept separate so `system-bootstrap.sh` (steps
8–10, 13) works unchanged for Auth0/Okta-OAuth installs, which swap steps 2–3, 6, and 11–12 for the
Auth0 app registration documented in [self-hosting.md](./self-hosting.md) instead.

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
> [self-hosting-saml.md](./self-hosting-saml.md), and is the same regardless of which IdP you use.
