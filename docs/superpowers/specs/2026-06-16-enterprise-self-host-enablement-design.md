# Enterprise Self-Host Enablement + Cloud-Ops Runbook

**Date:** 2026-06-16
**Context:** temper
**Scope:** feature (code enablement) + documentation (operator runbook)
**Status:** design approved; spec under review

## Summary

Make a non-`temperkb.io` deployment a first-class, documented path for the
**API + MCP + CLI** surfaces. Today the deployable stack is already largely
env-driven, but a handful of `temperkb.io`/`temperkb.us.auth0.com` defaults are
baked into the shipped CLI binary and config layer, and there is no operator
documentation for standing up an independent instance.

The deliverable is twofold:

1. **Enablement** — strip the baked-in `temperkb` defaults so the open-source
   binary does not phone home to the hosted SaaS, and make instance + Auth0
   configuration explicit (via `temper init`). After this change `temperkb.io`
   is "just another deployment" configured through the same path an enterprise
   uses — which dogfoods the self-host story.
2. **Runbook** — `docs/guides/self-hosting.md`, an operator-facing cloud-ops
   guide for provisioning Neon + Auth0 and deploying `temper-api` + `temper-mcp`
   to Vercel, then pointing the CLI and MCP clients at the instance. The
   contract tables are **grounded by introspecting the live temperkb.io
   deployment** (Auth0 tenant, Vercel project, Neon project) so the documented
   shape matches what actually had to be configured.

## Scope

**In scope:** `temper-api` (Axum, deployed via `temper-cloud` on Vercel),
`temper-mcp` (deployed via `api/mcp.rs` on Vercel), `temper-cli` (+ its
server-side OAuth relay `api/auth/cli-callback.ts`), the shared config in
`temper-core`, and env templates.

**Out of scope (documented as such):** the `temper-ui` SvelteKit web app and its
Auth0 *Regular Web Application* login flow. The web UI is not the core use case
for enterprise / non-`temperkb.io` deployments. The runbook will name this
explicitly in a "Not covered / deferred" section so operators know the boundary.
A follow-up can extend the runbook to the UI later.

## Current State (grounding)

### Already generic — no `temperkb` hardcoding (env-driven)

- **`temper-api`** (`crates/temper-api/src/config.rs`): `DATABASE_URL`,
  `JWKS_URL`, `AUTH_ISSUER`, `AUTH_AUDIENCE` (optional; absence disables
  audience validation with a warning), `CORS_ORIGINS`, `PORT`,
  `AUTH_PROVIDER_NAME` (defaults to `auth0`), `ENABLE_SWAGGER`, `RUST_LOG`.
- **`temper-mcp`** (`crates/temper-mcp/src/config.rs`): `MCP_BASE_URL`,
  `AUTH_ISSUER` (reused as the Auth0 domain), `MCP_AUDIENCE` (falls back to
  `AUTH_AUDIENCE`), `MCP_CLIENT_ID`. Fully parameterized already.
- **Build/deploy:** `SQLX_OFFLINE=true` (`vercel.json`), `vercel.json` routing
  (`/mcp` → `api/mcp`, `/.well-known/*` + `/oauth/*` → `api/mcp`, catch-all →
  `api/axum`).

### Baked-in `temperkb` defaults to remove (the enablement work)

- **`crates/temper-core/src/types/config.rs`**
  - `default_api_url()` → `https://temperkb.io`
  - `AuthConfig::default()` → a provider vec hardcoding
    `temperkb.us.auth0.com` authorize/token URLs, the temperkb `client_id`,
    and `audience = https://temperkb.io/api`
  - `default_callback_url()` → `https://temperkb.io/api/auth/cli-callback`
- **`crates/temper-cli/src/commands/init.rs`** — the wizard writes the temperkb
  config template (the same constants, duplicated in the TOML emitter).
- **`api/auth/cli-callback.ts`** — parses the request against a hardcoded
  `new URL(req.url, "https://temperkb.io")` base.
- **`crates/temper-api/.env.template`** — stale: still carries **Neon Auth**
  placeholders (`AUTH_ISSUER=https://neonauth.example.com`) even though auth
  migrated to Auth0. (The project fundamentals note "Neon Auth JWKS" is
  likewise stale — out of scope for code, but worth correcting in passing.)

### Live-config introspection (confirmed working)

- **Auth0** — the `auth0` CLI (v1.28.0) is authenticated to the active tenant
  `temperkb.us.auth0.com`. `auth0 apis list` confirms two resource servers:
  the built-in *Auth0 Management API* and **`temper-api`** with identifier /
  audience `https://temperkb.io/api`. The runbook's Auth0 section is grounded
  against the live tenant (resource server + applications + grant types) using
  the authenticated CLI. Enabling the **Auth0 MCP server** for in-session pulls
  is planned (see Open Questions §2); the CLI is the equivalent fallback and is
  already working.
- **Vercel** — Vercel MCP connected; `vercel` CLI (50.37.0) installed. The
  env-var contract table is validated against the live project's env inventory
  (`vercel env ls` once linked, or the Vercel MCP) — checking **shape and
  need** (required vs optional, which are temperkb-specific), not values.
- **Neon** — Neon MCP connected; `neonctl` installed. The database section is
  validated against the live project's shape (Postgres version, pgvector
  extension, connection-string form, pooled vs direct).

### Introspection findings (confirmed against live temperkb deployment)

- **Auth0** (`temperkb.us.auth0.com`): one resource server **`temper-api`**
  (audience `https://temperkb.io/api`); native app **`temper-cli`** (client_id
  `mWp8znLw…` — the exact value currently hardcoded in `config.rs`; callback
  `https://temperkb.io/api/auth/cli-callback`; grants authorization_code +
  refresh_token); native app **`Temper MCP`** (the `MCP_CLIENT_ID`; callbacks
  for claude.ai/claude.com + localhost). The `TemperKB Web` regular-web app is
  the UI — out of scope. → the runbook's Auth0 contract is: **1 API + 1 CLI
  native app + 1 MCP native app** (the web app is the deferred UI).
- **Neon** (project `temper-cloud`, `crimson-fog-23541670`): **Postgres 17.10**
  (note: docs/CLAUDE.md say "18" — that is the *local Docker* version; Neon
  cloud is 17, and the runbook must state 17). Extensions: **`vector`**
  (pgvector), **`pg_uuidv7`** (in-DB UUIDv7), `plpgsql` — an enterprise must
  install `vector` + `pg_uuidv7`. DB `neondb`, role `neondb_owner`. Connection
  form `…@<host>/neondb?sslmode=require&channel_binding=require`; pooled host
  adds `-pooler`. Per-preview Neon branches are wired to Vercel preview deploys.
- **Vercel**: **not reachable** from the authenticated `vercel` CLI login
  (`jcoletaylor` / `jcoletaylors-projects` has no projects — the production
  project lives under a different Vercel account/team), and the claude.ai
  Vercel MCP tools are not exposed in-session. The env-var contract is therefore
  grounded from code + templates (canonical and complete); the live "which vars
  are actually set" cross-check is an open item (see Open Questions §3).

## Design

### 1. Strip baked-in defaults (`temper-core`)

A freshly-constructed `TemperConfig` ships with **no cloud provider
configured**:

- `auth.provider` defaults to `"none"`; `auth.providers` defaults to empty.
- `cloud.api_url` has no `temperkb.io` default. Because `api_url` carries a
  `#[validate(url)]` constraint, the field becomes effectively "unset until
  configured": cloud operations surface the existing clear error from
  `oauth_config` ("cloud sync is disabled for this vault — run `temper init`…")
  rather than silently targeting a default host.
- The temperkb constants (authorize/token URLs, `client_id`, audience,
  callback) move out of `config.rs` defaults and into the `init` wizard's
  **hosted preset** (below) — they live in exactly one place.

This is the load-bearing decision: the OSS binary must not default to the
hosted SaaS. `temperkb.io` users are not special-cased in the binary; they pick
the hosted preset during `init` like anyone else.

### 2. `temper init` gains an instance step

The wizard's auth-provider prompt (currently `auth0` / `none`) becomes a
three-way instance choice:

- **(a) temperkb.io hosted** — a *preset* that fills the current temperkb
  values (api_url, authorize/token URLs, client_id, audience, callback). One
  keypress; hosted onboarding stays as smooth as today.
- **(b) self-hosted / custom** — prompts for the instance base URL, Auth0
  domain, Auth0 client_id, and audience; derives `authorize_url`/`token_url`
  (`https://<domain>/authorize`, `/oauth/token`) and
  `callback_url` (`<instance>/api/auth/cli-callback`) from those inputs.
- **(c) none / offline** — writes `provider = "none"` (cloud sync disabled),
  unchanged from today.

The non-interactive (`--no-interactive`) path no longer emits a temperkb
config; it writes the `none` posture (or accepts the values via flags/env if we
choose to add them — see Open Questions).

### 3. `api/auth/cli-callback.ts` — host-relative parsing

Replace the hardcoded `https://temperkb.io` parse base with the request's
actual host (derived from the incoming request / `x-forwarded-host`), so the
OAuth code relay works on any deployment domain. The localhost relay target is
already carried in the OAuth `state`, so only the parse base changes.

### 4. Env templates

- Fix `crates/temper-api/.env.template`: replace the Neon-Auth example with the
  Auth0 contract (issuer, JWKS, audience), generic (no temperkb host).
- Reorganize the root `.env.template` so the API/MCP self-host contract is
  clearly delineated; mark UI-only vars as out-of-scope for this guide.

### 5. Runbook — `docs/guides/self-hosting.md`

Operator-facing, ordered by provisioning dependency:

1. **Overview & topology** — one Vercel project hosts `temper-api` +
   `temper-mcp`; Neon Postgres; an Auth0 tenant. Diagram of request flow. What
   is *not* covered (the web UI).
2. **Provision Neon** — Postgres 18 + pgvector; connection string (pooled vs
   direct); run sqlx migrations as a deploy step (`DATABASE_URL` + `sqlx
   migrate run`), not at runtime. Grounded against the live Neon project shape.
3. **Provision Auth0** — value contract, not click-by-click:
   - a **resource server / API** (the `audience`, e.g. the live `temper-api`
     identifier `https://temperkb.io/api` → your `https://<instance>/api`);
   - a **native application** for the CLI (Authorization Code + PKCE, device
     relay), with its `client_id`, allowed callback `…/api/auth/cli-callback`;
   - the MCP application / client registration.
   Each value mapped to the exact env var (`AUTH_ISSUER`, `JWKS_URL`,
   `AUTH_AUDIENCE`, `MCP_AUDIENCE`, `MCP_CLIENT_ID`) and to the CLI
   `[[auth.providers]]` field. Pulled from the live tenant via the Auth0 MCP /
   CLI so the documented shape is real.
4. **Deploy to Vercel** — the env-var contract table (var, surface, source,
   required?, example/placeholder — **no secret values**), `vercel.json`
   routing, `SQLX_OFFLINE=true`. Validated against the live project's env
   inventory for shape and need.
5. **Configure the CLI** — `temper init` → self-hosted path; the resulting
   `config.toml`; `TEMPER_API_URL` / `TEMPER_PROVIDER_ENV` / `TEMPER_TOKEN`
   overrides for CI and headless agents.
6. **Connect MCP clients** — point an MCP client at `https://<instance>/mcp`;
   OAuth discovery (`/.well-known/*`) and the `MCP_*` registration.
7. **Verify** — `/api/health`, `temper login`, round-trip a resource end to end.
8. **Not covered / deferred** — the web UI and its Auth0 Regular-Web-App flow.

## Testing

- Update `temper-core` config tests that currently assert temperkb defaults:
  they now assert the "unconfigured" posture (no provider, api_url unset/empty),
  and the hosted-preset constants are asserted where they now live (the `init`
  wizard emitter).
- New `temper init` tests for the self-hosted path: given instance URL + Auth0
  inputs, the emitted `config.toml` has the derived authorize/token/callback
  URLs and the correct provider block.
- No new e2e required: the API/MCP env contract is already exercised by the
  existing suites; this work changes defaults and docs, not request behavior.
- `cargo make check` + `cargo make test` gate the code changes.

## Out of Scope

### Rejected (load-bearing — resist scope creep)

- **Keeping a `temperkb.io` default in the binary.** Decided against: the OSS
  binary must not phone home to the hosted SaaS. (See §1.)
- **Build-time / per-enterprise branded binaries.** Not pursued; a single OSS
  binary configured at runtime via `init` covers both hosted and self-host.
- **The web UI deployment path.** Explicitly deferred (see Scope).

### Deferred (in scope elsewhere / later)

- Extending the runbook to the `temper-ui` web app.
- RabbitMQ / alternative messaging, multi-region, HA Neon — single-instance
  self-host is the target.

## Open Questions

1. **`--no-interactive` self-host config** — should `temper init
   --no-interactive` accept instance/Auth0 values via flags or env (for
   scripted enterprise provisioning), or is the `none` posture + manual
   `config.toml` edit sufficient for v1? (Lean: env/flags, since enterprise
   provisioning is often scripted — but confirm.)
2. **Auth0 MCP enablement scope** — wire the Auth0 MCP server into this repo's
   MCP config (committed) or keep it a local/dev convenience? (Lean: local
   convenience; the runbook documents the CLI path which needs no MCP.)
3. **Vercel live cross-check** — the production Vercel project isn't reachable
   from the current CLI login. To validate the env contract against the live
   project, either (a) authenticate the `vercel` CLI to the right account /
   team and run `vercel env ls`, or (b) ship the runbook with the contract
   derived from code + templates (canonical) and cross-check via the dashboard
   later. (Lean: (b) for now — code is the source of truth for *which* vars
   exist; the live check only confirms they're populated.)
