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
- On success it prints the **connector id**, of the form `mcp.<host>/<name>` (here
  `mcp.temperkb.io/steward`). That id is the value of `TEMPER_CONNECT_CONNECTOR` below.
- `vercel connect list` shows existing connectors; `vercel connect token <id> --subject app`
  mints an app token to smoke-test the connector.

The connection (`agent/connections/temper.ts`) reads `TEMPER_CONNECT_CONNECTOR` and, when set,
authenticates via `connect({ connector, principalType: "app" })` — falling back to `TEMPER_TOKEN`
only when the connector env is absent (local dev).

> The connector's app principal resolves to a temper profile. Ensure that profile has **read
> reach** over the corpus the agent tends (e.g. the shared ingest context) — clearing the
> ingest-delta gate is not enough; the agent must be able to *read* the sources it distills.

## Environment contract

Set these on the Vercel project (dashboard, or `vercel env add <NAME>`) **before** deploying —
the build reads them at discovery time and fails fast if a required one is missing (e.g.
`TEMPER_MCP_URL is required`, thrown by the connection's guard, working as designed).

| Variable | Required | Value / purpose |
|----------|----------|-----------------|
| `TEMPER_MCP_URL` | yes | The temper-mcp endpoint, e.g. `https://temperkb.io/mcp`. The agent's sole seam to Temper. One agent dir points at temperkb.io or a self-hosted instance by this value alone. |
| `TEMPER_API_URL` | yes | The temper REST base, e.g. `https://temperkb.io`. Distinct from `TEMPER_MCP_URL`; used by the region-materialize schedule's direct POST. |
| `TEMPER_SELF_COGMAP_ID` | yes | The cognitive map this agent tends, by id (minted at genesis). See the design note below. |
| `TEMPER_CONNECT_CONNECTOR` | prod | The Vercel Connect connector **id** for temper-mcp — the `mcp.<host>/<name>` string printed by `vercel connect create` (e.g. `mcp.temperkb.io/steward`). See [Registering the temper-mcp connector](#registering-the-temper-mcp-connector-vercel-connect). When set, the connection authenticates via Connect (app subject) — no secret in code. When unset, it falls back to `TEMPER_TOKEN`. |
| `TEMPER_TOKEN` | dev only | An already-OAuth-obtained temper token. Drives `eve dev` and the pre-connector boot path. Not for production — use the Connect connector instead. |
| `TEMPER_MCP_AUDIENCE` | optional | Only when the token audience varies by target and is not discovery-derived. |

### AI Gateway credential

The agent's model calls run through the **Vercel AI Gateway**. On a deployed Vercel project
this authenticates automatically via **OIDC** (`VERCEL_OIDC_TOKEN` is injected at runtime) —
no credential to set. You only need a gateway key for **local** `eve dev`; after `vercel link`,
`vercel env pull` writes it into `.env.local`.

## Deploy and verify

```bash
cd packages/agent-workflows/steward
eve deploy            # rides the existing .vercel link; no eve picker
```

Then confirm:

- **Cron Jobs** (Vercel → *Settings → Cron Jobs*): every `defineSchedule` becomes a Vercel
  Cron Job, evaluated in **UTC**. Expect the steward tick and the region-materialize tick.
- **Execution** (Vercel → *Observability → Cron Jobs* / *Logs*): a tick that clears the ingest
  threshold produces a **closed invocation envelope** with correlated mutation events; a tick
  under threshold opens and closes the envelope with a no-op outcome.

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
