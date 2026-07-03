# Deploying an Eve agent to Vercel (the steward)

This guide covers deploying a Temper **Eve agent** — currently the team-self-cognition
**steward** (`packages/agent-workflows/steward/`) — to Vercel: the isolated-project tooling
rules, linking to Vercel (including a gotcha with single-team accounts), the environment
contract, and the deploy + verify loop.

It assumes the agent's target cognitive map already exists on the instance. Birthing and
binding that map is a separate, prior step — see
[team-self-cognition-bootstrap.md](./team-self-cognition-bootstrap.md).

## The agent is a workspace-isolated Eve project

Each agent under `packages/agent-workflows/` is a **self-contained Eve project** with its own
`package.json`, npm lockfile, and TypeScript toolchain. It is deliberately **not** a Bun
`workspaces` member, so it never collides with `temper-cloud`'s toolchain and the repo
pre-commit never touches it. Two consequences:

- **Run all tooling from inside the agent directory**, never the repo root:
  ```bash
  cd packages/agent-workflows/steward
  ```
  A root `npm install` inherits the repo's Bun `overrides` (e.g. `onnxruntime-common`) and
  fails with `EOVERRIDE`.

- **Never `npx eve@latest`.** The project pins a specific `eve` version (0.18.1 at time of
  writing). `@latest` pulls a different version *and* resolves dependencies against the
  repo-root `package.json`, tripping the same `EOVERRIDE`. Use the locally installed binary:
  ```bash
  npx eve <command>          # resolves the local eve from inside the agent dir
  ./node_modules/.bin/eve <command>
  npm run build|dev          # the package scripts, equivalent
  ```

## Linking to Vercel

`eve deploy` deploys to a linked Vercel project. **If the directory is already linked (a
`.vercel/project.json` exists), `eve deploy` uses that link directly.** Only when the
directory is *unlinked* does eve run its own interactive project picker.

### Gotcha: `eve link` and single-team accounts

`eve link`'s interactive picker enumerates **all** your Vercel scopes, including your
**personal account**, and runs `vercel project ls --scope <account>` for each. Vercel
**forbids** using a personal account as a scope:

```
Could not list Vercel projects in <username>. vercel project ls --format json --scope <username> exited with code 1.
# → Error: You cannot set your Personal Account as the scope.
```

If your login has one team plus a personal account (the common case), eve hits the personal
account and treats the failure as fatal. `vercel switch <team>` does **not** help — the picker
enumerates the personal scope regardless.

### The fix: pre-link with the Vercel CLI

Vercel's own `link` picker handles the personal account correctly. Link once, and `eve deploy`
rides that link (skipping eve's picker):

```bash
cd packages/agent-workflows/steward

vercel link
#   → pick the TEAM scope (e.g. "your-team's projects"), then create or select a project
#     (e.g. "temper-steward"). Writes .vercel/project.json.

# non-interactive equivalent (existing project):
vercel link --project temper-steward --team <team-slug> --yes
```

You do **not** need `eve link` at all: its jobs are (a) link the project — `vercel link`
covers it — and (b) fetch an AI Gateway credential, which the deployed agent gets
automatically via Vercel OIDC (see below).

## Registering the temper-mcp connector (Vercel Connect)

Production auth is **platform-carried**: the agent authenticates to temper-mcp through a
**Vercel Connect connector** (app subject), so no token lives in code or env. temper-mcp is a
full OAuth 2.0 server and serves the OAuth discovery endpoints (RFC 8414/9728), so Connect
discovers everything it needs from the MCP URL — you do **not** hand it a client id/secret.

Register the connector by its **full MCP endpoint URL**, from the agent directory (Connect reads
the local project context to auto-configure project access and the connector `uid`):

```bash
cd packages/agent-workflows/steward
vercel connect create https://temperkb.io/mcp --name steward
```

- The URL is the same value as `TEMPER_MCP_URL`. For a self-hosted instance, use its MCP URL.
- The command **opens a browser** to complete the OAuth authorization — finish it there; the CLI
  waits until you do.
- On success it prints a **connector ID** (`scl_…`) and a **UID** of the form `<host>/<name>`
  (here `temperkb.io/steward`). Either the `scl_…` id or the `<host>/<name>` UID is a valid
  value for `TEMPER_CONNECT_CONNECTOR` — but note the UID is `temperkb.io/steward`, **not**
  `mcp.temperkb.io/steward`.
- `vercel connect list` shows the connector and both forms. Note that `vercel connect token …
  --subject app` run from the CLI (a human requester) returns *"Token subject is not accessible
  to this requester"* — app-subject tokens are mintable only by the deployed project's runtime
  (its Vercel OIDC), not by a user at the CLI. Use `--subject user --yes` to smoke-test the
  connector interactively.

The connection (`agent/connections/temper.ts`) authenticates **machine-identity-first**: when
`TEMPER_M2M_CLIENT_ID` is set it mints its own token (`mintM2mToken`), else it falls back to a
Vercel Connect connector (`TEMPER_CONNECT_CONNECTOR`), else a static `TEMPER_TOKEN` (local dev).

### Status (2026-07-03): `app` principal via direct M2M mint — the Connect app-path is a dead end here

Auth-seam Stage 4 shipped (`normalize_machine` + agent-profile provisioning + the
`client_credentials` advertisement). But on the **Auth0-fronted** instance the Vercel Connect
`app` path **cannot** mint a token: Auth0 issues `client_credentials` only for a registered
**M2M application**, and the Connect connector has no Auth0 M2M app behind it — its dynamic
registration does not create one (confirmed: the connector produces no app in `auth0 apps
list`). Advertising the grant is necessary but not sufficient.

So the steward mints its own token **directly**: a dedicated Auth0 M2M application (`Temper
Steward M2M`), and `agent/connections/temper.ts` performs the `client_credentials` grant itself
(`mintM2mToken`, keyed on the `TEMPER_M2M_*` env). This is the distinct machine principal the
design wants, without depending on Connect. Provision the M2M app + audience grant once via the
Auth0 CLI — see the [operator runbook](../auth/machine-token-contract.md#operator-runbook-provisioning-an-auth0-m2m-agent).
The `authorization_code + refresh_token` bridge under `user` below remains an escape hatch.

### Which subject: `app` vs `user`

temper-mcp resolves the caller's **profile from the token's `sub` claim**
(`profile_service::resolve_from_claims`): a `sub` with an existing auth link loads that profile;
a `sub` with none **creates a brand-new blank profile** (its own empty default context, no reach
to anyone else's corpus). So the connector subject decides *who the agent is* in temper:

- **`user`** (the human who authorized in the browser) → the agent acts **as that person**, with
  their profile and read reach. Simplest reach, but it conflates agent-authored nodes with the
  human's identity — at odds with the invocation envelope's authorship discipline.
- **`app`** (the agent's own machine identity) → a **distinct principal**, which is the right
  shape for authorship accountability — but its `sub` maps to a fresh empty profile, so it has
  **no read reach** until you grant it. After the first run, find that profile and grant it read
  on the ingest context (the same shape as sharing a context into a team during bootstrap), or
  pre-provision + grant it before deploy.

Clearing the ingest-delta gate is **not** enough — the agent must be able to *read* the sources
it distills, which is a property of the resolved profile, not the delta.

## Environment contract

Set these on the Vercel project (dashboard, or `vercel env add <NAME>`) **before** deploying —
the build reads them at discovery time and fails fast if a required one is missing (e.g.
`TEMPER_MCP_URL is required`, thrown by the connection's guard, working as designed).

| Variable | Required | Value / purpose |
|----------|----------|-----------------|
| `TEMPER_MCP_URL` | yes | The temper-mcp endpoint, e.g. `https://temperkb.io/mcp`. The agent's sole seam to Temper. One agent dir points at temperkb.io or a self-hosted instance by this value alone. |
| `TEMPER_API_URL` | yes | The temper REST base, e.g. `https://temperkb.io`. Distinct from `TEMPER_MCP_URL`; used by the region-materialize schedule's direct POST. |
| `TEMPER_SELF_COGMAP_ID` | yes | The cognitive map this agent tends, by id (minted at genesis). See the design note below. |
| `TEMPER_M2M_CLIENT_ID` | prod | Auth0 M2M app client id. **When set, the connection mints its own `client_credentials` token (the app principal)** — the production path on the Auth0-fronted instance. Takes precedence over `TEMPER_CONNECT_CONNECTOR`. |
| `TEMPER_M2M_CLIENT_SECRET` | prod | The M2M app client secret. A Vercel env var only — never in code, never seen by the model. |
| `TEMPER_M2M_TOKEN_URL` | prod | The issuer's token endpoint, e.g. `https://<tenant>.auth0.com/oauth/token`. |
| `TEMPER_M2M_AUDIENCE` | prod | The API audience the minted token targets, e.g. `https://temperkb.io/api` (== the mcp audience). |
| `TEMPER_CONNECT_CONNECTOR` | fallback | Vercel Connect connector id (`vercel connect create`). Used only when `TEMPER_M2M_CLIENT_ID` is unset. **On the Auth0-fronted instance this cannot mint an app token** (see Status above) — the M2M vars are the real path. |
| `TEMPER_TOKEN` | dev only | An already-OAuth-obtained temper token. Drives `eve dev`. Not for production. |
| `TEMPER_MCP_AUDIENCE` | optional | Only when the token audience varies by target and is not discovery-derived. |

### AI Gateway credential

The agent's model calls run through the **Vercel AI Gateway**. On a deployed Vercel project
this authenticates automatically via **OIDC** (`VERCEL_OIDC_TOKEN` is injected at runtime) —
no credential to set. You only need a gateway key for **local** `eve dev`; after `vercel link`,
`vercel env pull` writes it into `.env.local`.

## Deploy and verify (two-phase, app principal)

Because the agent runs as its own **app** principal, its temper profile does not exist until
its first authenticated call creates it — so reach is granted in a second phase, after deploy.

### Phase 1 — deploy

Set every env var (see the contract above) on the Vercel project first, then:

```bash
cd packages/agent-workflows/steward
eve deploy            # rides the existing .vercel link; no eve picker
```

On the first cron tick the agent authenticates to temper-mcp; temper-mcp resolves its `sub` to
a **new, empty profile** (`resolve_from_claims`). That tick does no useful work yet (no reach,
no cogmap write grant), but the profile now exists.

**Verify the app token actually minted** (the one open risk): check Vercel → *Observability →
Logs* for the tick. Success = a profile was resolved / an auth line, not a token error. If the
connector cannot issue an app token (temper-mcp's OAuth server must support the app/client-
credentials exchange), no profile is created — pivot before granting.

### Phase 2 — grant the agent's profile reach

Find the just-created profile (newest row after deploy), then grant it the two capabilities it
needs: **read** on the ingest sources (via team membership) and **write/author** on the map.

```bash
# Identify the steward profile (via neonctl → psql):
#   SELECT p.id, p.handle, p.created, l.auth_provider_user_id AS sub
#     FROM kb_profiles p JOIN kb_profile_auth_links l ON l.profile_id = p.id
#    ORDER BY p.created DESC LIMIT 5;   -- the newest is the steward's

# 1. Source read reach — join the team the ingest context is shared into (watcher = read-only):
temper team add-member <team-id> <steward-profile-id> --role watcher
#    (e.g. team personal-j-cole-taylor 019eea5e-… — the bound team from genesis)

# 2. Cogmap authoring — explicit write grant (post-Q-A, authoring is not conferred by membership):
temper cogmap grant <cogmap-ref> --to-profile <steward-profile-id> --write
```

### Verify

- **Cron Jobs** (Vercel → *Settings → Cron Jobs*): every `defineSchedule` becomes a Vercel
  Cron Job, evaluated in **UTC**. Expect the steward tick and the region-materialize tick.
- **Execution** (Vercel → *Observability → Cron Jobs* / *Logs*): a tick that clears the ingest
  threshold produces a **closed invocation envelope** with correlated mutation events, authored
  by the steward's own profile; a tick under threshold opens and closes with a no-op outcome.

## Design note — `TEMPER_SELF_COGMAP_ID` is an MVP binding

The cognitive-map id is not intrinsically agent *config* — it is the **subject of an
invocation**. It is already modeled that way: `invocation open --cogmap <ref>` is required,
and every steward act (`steward_ingest_delta`, `create_resource`, `assert_relationship`,
`facet_set`, `advance_watermark`) takes the cogmap as a first-class parameter. The env var
simply pins that subject for a single steward tending a single map — the 1:1 MVP shape.

The general agent-invocation framework has cleaner homes for it, and the MVP env var should be
read as a temporary stand-in for one of them:

- **Discovery from the principal's authorable grants.** The agent authenticates (via Connect)
  as a principal; the access model already governs authorship by explicit write-grant. A
  steward's targets are then "the cogmaps my principal is granted to author," queried at wake
  and looped over — so adding a team/map is a *grant*, not a redeploy with a new env value.
- **The invocation carries the cogmap.** A dispatcher opens the envelope against a specific map
  and hands the agent "tend this invocation's subject," making the agent a stateless worker
  over whatever map the run targets.

Either is required for fan-out across multiple maps/teams (explicitly deferred in the steward
MVP). When that lands, `TEMPER_SELF_COGMAP_ID` goes away in favor of grant-discovery, with the
invocation naming the specific map per run.

## See also

- [team-self-cognition-bootstrap.md](./team-self-cognition-bootstrap.md) — birth + bind the map (prerequisite).
- `packages/agent-workflows/steward/agent/connections/temper.ts` — the connection (env URL, dual-path auth, allow-list).
- `docs/superpowers/specs/2026-07-01-t5-eve-steward-agent-directory-design.md` — the steward directory design.
