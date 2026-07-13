# Deploying an Eve agent to Vercel (the steward)

This guide covers deploying a Temper **Eve agent** — currently the team-self-cognition
**steward** (`packages/agent-workflows/steward/`) — to Vercel: the isolated-project tooling
rules, how the deploy actually reaches Vercel (by **git**, and why the CLI path is now a trap),
the environment contract, registering the agent's machine credential, and the verify loop.

Two prerequisites, both prior and separate:

- The agent's target cognitive map(s) must exist on the instance. Birthing and binding a map is
  covered in [team-self-cognition-bootstrap.md](./team-self-cognition-bootstrap.md).
- The agent's **machine credential must be registered before its first call.** The credential model
  — the two mint paths, reach, rotation, revocation — lives in
  [machine-credentials.md](./machine-credentials.md). This guide does not restate it; it tells you
  which values the steward reads and when to register.

## The agent is a workspace-isolated Eve project

Each agent under `packages/agent-workflows/` is a **self-contained Eve project** with its own
`package.json`, npm lockfile, and TypeScript toolchain. It is deliberately **not** a Bun
`workspaces` member, so it never collides with `temper-cloud`'s toolchain and the repo
pre-commit never touches it. Three consequences:

- **Run all tooling from inside the agent directory**, never the repo root:

  ```bash
  cd packages/agent-workflows/steward
  npm install
  ```

  A root `npm install` inherits the repo's Bun `overrides` (e.g. `onnxruntime-common`) and
  fails with `EOVERRIDE`.

- **Never `npx eve@latest`.** The project pins a specific `eve` version (0.18.1 at time of
  writing, per `package.json`). `@latest` pulls a different version *and* resolves dependencies
  against the repo-root `package.json`, tripping the same `EOVERRIDE`. Use the locally installed
  binary:

  ```bash
  npx eve <command>          # resolves the local eve from inside the agent dir
  ./node_modules/.bin/eve <command>
  npm run build|dev          # the package scripts, equivalent
  ```

- **The agent has tests, and they gate CI.** `npm test` (vitest, `tests/`) from inside the agent
  dir; `npm run typecheck` for `tsc`. Both run in CI via `.github/workflows/test-agents-ts.yml`
  (the `steward` job), alongside the `temper-ts` suite.

### The `file:` dependency on `clients/temper-ts` — and what it costs

The steward's M2M mint is **not** its own: it takes `temper-ts` as an npm `file:` dependency
(`"temper-ts": "file:../../../clients/temper-ts"` in `package.json`) and composes
`ClientCredentials` from it, so the TypeScript client and the Ruby gem cannot drift on how a
machine token is minted. This is a deliberate bridge until `temper-ts` publishes, at which point
the dependency becomes a normal version range.

Two things follow, and both are load-bearing:

- **A fresh clone just works.** `prebuild`, `pretest`, and `pretypecheck` all run `build:dep`,
  which does `npm ci && npm run build` in `clients/temper-ts`. You never have to remember to build
  the dependency by hand.
- **The dependency lives OUTSIDE the Vercel Root Directory.** The `steward-agent` Vercel project's
  root directory is `packages/agent-workflows/steward`; `clients/temper-ts` is a sibling several
  levels up. This builds only because the project has **"Include source files outside of the Root
  Directory"** enabled. Turn that off and the build fails to resolve `temper-ts`.

## Deploying: by git, not by CLI

**`steward-agent` is git-connected.** The Vercel project is wired to `github.com/tasker-systems/temper`
with production branch `main`. So:

- **A merge to `main` produces a production deployment.** The steward *does* auto-deploy on
  monorepo merge.
- **A push to any other branch produces a preview deployment.**
- **Vercel env changes still require a redeploy to take effect.** Setting a new `TEMPER_M2M_*` or
  `STEWARD_MODEL` value in the dashboard does nothing until the next deployment. A cron running
  against stale env looks exactly like a code bug.

> **Do not run `eve deploy` / `vercel deploy` from inside the agent directory.** The CLI uploads
> only the directory it is invoked from. Since `temper-ts` is a `file:` sibling **outside** the
> Root Directory, a deploy launched from `packages/agent-workflows/steward` **cannot carry the
> dependency** — the upload has no `clients/temper-ts` in it and the build breaks on an
> unresolvable import. The git path has no such problem: Vercel clones the whole repo and honors
> "include source files outside the Root Directory".
>
> **Deploy by pushing.** If you genuinely must deploy from the CLI, run it from the **repo root**
> so the sibling directory is in the upload — never from the agent directory.

## Linking to Vercel

You still need a link (`.vercel/project.json`) for `vercel env pull`, `vercel env add`, and
`eve dev`. Getting one has a sharp edge.

### Gotcha: `eve link` and single-team accounts

`eve link`'s interactive picker enumerates **all** your Vercel scopes, including your
**personal account**, and runs `vercel project ls --scope <account>` for each. Vercel
**forbids** using a personal account as a scope:

```text
Could not list Vercel projects in <username>. vercel project ls --format json --scope <username> exited with code 1.
# → Error: You cannot set your Personal Account as the scope.
```

If your login has one team plus a personal account (the common case), eve hits the personal
account and treats the failure as fatal. `vercel switch <team>` does **not** help — the picker
enumerates the personal scope regardless.

### The fix: link with the Vercel CLI

Vercel's own `link` picker handles the personal account correctly:

```bash
cd packages/agent-workflows/steward

vercel link
#   → pick the TEAM scope, then select the existing project (steward-agent).
#     Writes .vercel/project.json.

# non-interactive equivalent:
vercel link --project steward-agent --team <team-slug> --yes
```

You do **not** need `eve link` at all: its jobs are (a) link the project — `vercel link` covers it
— and (b) fetch an AI Gateway credential, which the deployed agent gets automatically via Vercel
OIDC (see below).

## The machine credential: register it BEFORE the first tick

The steward authenticates as a **machine principal** — its own agent profile, not a proxied human.
Registration is **fail-closed**: `resolve_machine_from_claims` (`temper-services`, the single
machine entry point for both temper-api and temper-mcp) is a lookup in `kb_machine_clients` or a
**401**. There is **no just-in-time create branch**.

> **This invalidates the old two-phase "deploy, let the first tick create a blank profile, then
> grant it reach" flow.** That flow does not merely no-op now — every call 401s, forever, until the
> client id is registered. Register **first**, with **explicit reach**, then deploy.

Pick the mint path by who owns the secret — the full model, including reach containment and
rotation, is [machine-credentials.md](./machine-credentials.md):

```bash
# Auth0-fronted instance (temperkb.io today): register the Auth0 M2M app you created.
temper admin machine provision --client-id <auth0-client-id> --label "steward" \
  --owner-team <team> \
  --team <team>:member \
  --cogmap <cogmap-ref>

# Instance where Temper is the Authorization Server (AS_ISSUER set): Temper mints the
# credential itself. Prints a `tmpr_…` client id and a one-time secret.
temper admin machine issue --label "steward" \
  --owner-team <team> \
  --team <team>:member \
  --cogmap <cogmap-ref>
```

- **`issue` requires an instance whose AS is Temper's own.** An instance has exactly one issuer, so
  a temper-minted token will not validate on an Auth0-fronted instance. Use `provision` there.
- **Reach is explicit and plural, never inferred from `--owner-team`.** The steward needs two
  things and both must be granted here: **read** on the sources it distills (via `--team`
  membership) and **write** on the map(s) it tends (via `--cogmap`, which grants read+write unless
  you suffix `:ro`). Clearing the ingest-delta threshold is not enough — the agent must be able to
  *read* the corpus, which is a property of its profile.
- If you need to widen reach after the fact, use `temper team add-member` and
  `temper cogmap grant --to-profile <agent-profile-id> --write` — the profile already exists,
  because registration created it.

## Environment contract

Set these on the Vercel project (dashboard, or `vercel env add <NAME>`) **before** deploying.
Several are read at **build/discovery** time and fail fast when missing (e.g. `TEMPER_MCP_URL is
required`, thrown by the connection's `requireEnv` guard — working as designed).

| Variable | Required | Value / purpose |
|----------|----------|-----------------|
| `TEMPER_MCP_URL` | yes | The temper-mcp endpoint, e.g. `https://temperkb.io/mcp`. The agent's sole model-facing seam to Temper. One agent dir points at temperkb.io or a self-hosted instance by this value alone. |
| `TEMPER_API_URL` | yes | The temper REST base, e.g. `https://temperkb.io`. Distinct from `TEMPER_MCP_URL`; used by the code schedules' direct `POST /api/steward/dispatch`, `GET /api/steward/candidates`, and `POST /api/cognitive-maps/{id}/materialize`. |
| `TEMPER_M2M_CLIENT_ID` | prod | The machine client id — the Auth0 M2M app's id, or the `tmpr_…` id from `machine issue`. **When set, the agent mints its own `client_credentials` token** and this strategy wins over Connect and `TEMPER_TOKEN`. |
| `TEMPER_M2M_CLIENT_SECRET` | prod | The client secret. A Vercel env var only — never in code, never seen by the model. |
| `TEMPER_M2M_TOKEN_URL` | prod | The issuer's token endpoint: `https://<tenant>.auth0.com/oauth/token` for `provision`, or **your own instance's** `https://<instance>/oauth/token` for `issue`. |
| `TEMPER_M2M_AUDIENCE` | **only for an external IdP** | The API audience the minted token targets (must equal the API's `AUTH_AUDIENCE`). **OMIT it for a temper-issued (`tmpr_`) credential** — Temper's AS ignores a request-supplied audience entirely and mints with its server-side `AS_AUDIENCE`. Requiring this var is exactly what previously made the steward unable to consume a temper-issued credential. |
| `STEWARD_MODEL` | optional | The primary model, as an AI Gateway model id (same form as the default, `minimax/minimax-m3`). See below — a change needs a **redeploy**, and a typo fails the **build**. |
| `STEWARD_MODEL_FALLBACKS` | optional | Comma-separated AI Gateway model ids, tried in order after the primary fails. Defaults to `anthropic/claude-haiku-4.5`. Deduped, and the primary is dropped from the list if repeated there. |
| `TEMPER_CONNECT_CONNECTOR` | fallback | Vercel Connect connector id. Used **only** when `TEMPER_M2M_CLIENT_ID` is unset. **On the Auth0-fronted instance this cannot mint an app token** — see below. |
| `TEMPER_TOKEN` | dev only | An already-OAuth-obtained temper token. Drives `eve dev`. Cannot re-mint, so a 401 on it is terminal (by design — see `temperFetch`). |

The auth strategy is resolved once, in `agent/lib/temper-auth.ts`, and is **machine-identity-first**:

1. `TEMPER_M2M_CLIENT_ID` present → mint via the OAuth `client_credentials` grant
   (`ClientCredentials` from `temper-ts`). The production path.
2. else `TEMPER_CONNECT_CONNECTOR` → a Vercel Connect app token.
3. else `TEMPER_TOKEN` → a static bearer.

The **same** helper serves both the MCP connection (`agent/connections/temper.ts`, which hands
`mintM2mToken` to eve as `auth.getToken`) and the code schedules (via `temperFetch`), so the two
can never drift on how they authenticate. They did once: the schedules went Connect-first while the
connection went M2M-first, and on the Auth0-fronted instance the schedules' REST fetches silently
failed while MCP worked.

> **`temperFetch` re-mints once on a 401 and retries.** Refresh-ahead-of-expiry is not sufficient:
> a schedule resolves a token, then fans out N fetches, and Temper's AS mints **900-second** tokens
> by default — a tick outliving its token is ordinary, not exotic. Exactly one re-mint: a 401 that
> survives a fresh token is a real authorization failure (revoked credential, missing reach), and
> retrying forever would only bury it. A strategy that cannot mint (`TEMPER_TOKEN`) gets its 401
> back untouched. `temperFetch` also carries the 5xx cold-start retry — **use it, never a bare
> `fetch`.**

### The model is config, and it is frozen at build time

eve executes `agent.ts` at **BUILD** time (`compileAgentConfig`) and freezes the resolved model
into the compiled manifest. There is no session, no request context, no DB anywhere near that
resolution. Consequences (`agent/lib/model-config.ts`):

- **Changing the model takes a REDEPLOY, not a restart.** Env is the only lever eve offers.
- **The primary is validated against the AI Gateway catalog at compile time** — a typo in
  `STEWARD_MODEL` **fails the build**, not a 3am cron tick.
- **The fallbacks are not so validated.** They ride through the compile untouched inside
  `providerOptions.gateway.models`, so a typo there surfaces at runtime, only when it is needed.
- **Fallbacks cover availability, never quality.** The Gateway walks the list on a 5xx, a rate
  limit, a model that is gone. No gateway can detect that a model fumbled a tool sequence — the
  mechanism for *that* is changing `STEWARD_MODEL` and redeploying, which is what making it
  configurable buys.

The default (`minimax/minimax-m3`, falling back to `anthropic/claude-haiku-4.5`) is a cost choice
for the dev/community tier, where the loop runs hourly. Enterprise deployments override it.

### AI Gateway credential

The agent's model calls run through the **Vercel AI Gateway**. On a deployed Vercel project this
authenticates automatically via **OIDC** (`VERCEL_OIDC_TOKEN` is injected at runtime) — no
credential to set. You only need a gateway key for **local** `eve dev`; after `vercel link`,
`vercel env pull` writes it into `.env.local`.

## Vercel Connect (the fallback path, and its dead end here)

`TEMPER_CONNECT_CONNECTOR` is still a live strategy in the code, used when `TEMPER_M2M_CLIENT_ID`
is unset. temper-mcp is a full OAuth 2.0 server and serves the discovery endpoints (RFC 8414/9728),
so Connect discovers what it needs from the MCP URL — you do not hand it a client id/secret:

```bash
cd packages/agent-workflows/steward
vercel connect create https://temperkb.io/mcp --name steward
```

- The URL is the same value as `TEMPER_MCP_URL`.
- The command **opens a browser** to complete the OAuth authorization.
- On success it prints a **connector ID** (`scl_…`) and a **UID** of the form `<host>/<name>`
  (here `temperkb.io/steward`, **not** `mcp.temperkb.io/steward`). Either form is a valid
  `TEMPER_CONNECT_CONNECTOR` value.
- `vercel connect token … --subject app` from the CLI returns *"Token subject is not accessible to
  this requester"* — app-subject tokens are mintable only by the deployed project's runtime (its
  Vercel OIDC), not by a human at the CLI. Use `--subject user --yes` to smoke-test interactively.

> **On the Auth0-fronted instance the Connect `app` path cannot mint a token, and this is not
> fixable from Temper's side.** Auth0 issues `client_credentials` only for a registered M2M
> application, and the Connect connector has no Auth0 M2M app behind it — its dynamic registration
> does not create one (confirmed: the connector produces no app in `auth0 apps list`). Advertising
> the grant on the MCP server is necessary but not sufficient. **The `TEMPER_M2M_*` vars are the
> real path.** Connect remains in the code for instances where it does work.

## Verify

- **Cron Jobs** (Vercel → *Settings → Cron Jobs*): every `defineSchedule` becomes a Vercel Cron
  Job, evaluated in **UTC**. Expect two, both hourly (`0 * * * *`): the steward dispatch tick and
  the region-materialize tick.
- **Logs** (Vercel → *Observability → Logs*): the dispatch tick logs
  `[steward-dispatch] tick <correlation-id> starting`, then the claimed-job count (or
  `(no drift)`), then fans out. `[steward-materialize]` logs its candidate count. An unregistered
  or under-reached credential shows up here as a `401` on `/dispatch`, not as silence.
- **The DB** — see below. It is the only place a tick's actual work is visible.

### How a tick works now (there is no env-pinned map)

The steward **fans out**. There is no single map it tends, and no env var naming one:

- `agent/schedules/steward.ts` is a **code** handler, not a model prompt. It `POST`s
  `/api/steward/dispatch` — a server-side reap → sweep → enqueue → claim that returns the claimed
  jobs, each carrying its own `cogmap_id` — then starts **one isolated agent session per claimed
  job** (`receive(worker, …)`), each tending a single map. Single-flight and lease-reaping live in
  the server (`kb_workflow_jobs`), so a fixed hourly cadence is safe: a still-running map is not
  re-claimed, and a crashed run's lease expires and requeues.
- `agent/schedules/materialize.ts` enumerates `GET /api/steward/candidates` (the readable
  team-joined maps) and POSTs a **self-gating** materialize per map — the server no-ops below its
  formation threshold, so no lease or queue is needed.
- Each tick mints a `correlationId`, logs it, sends it as `x-steward-correlation-id`, and the
  server stamps it onto every claimed job; each session's `invocation_open` inherits it. So a
  tick's runs are **queryable** (`kb_invocations.correlation_id`), not merely greppable — and the
  trace survives a hop that dies before any DB row exists.

Widening what the steward tends is therefore a **grant**, not a redeploy: add the agent profile to
a team, or grant it write on a map, and the next sweep picks it up.

## Observing a tick — the DB is the source of truth, not the logs

**eve markdown task-mode discards the agent's own output.** The model's reasoning and tool results
never reach Vercel logs, so logs cannot tell you what a tick *did* (they tell you what was
*dispatched*). The temper DB is the source of truth: the **invocation envelope**
(`kb_invocations` — status / outcome / `closed_at` / `correlation_id`) and its **acts**
(`kb_events` joined on `invocation_id`). Read them with the MCP tools `invocation_show <id>`
(envelope + acts + outcome payload) and `invocation_list --status open` (any orphaned envelopes),
or over psql.

Three things that read as bugs but aren't:

- **Ticks are long — an open envelope with a null outcome mid-run is NORMAL, not a stall.** A tick
  that clears the threshold on a large delta runs for **many minutes** (the first prod tick ran ~11
  minutes: opened `01:47:34`, closed `01:58:38`, 17 nodes + 17 facets). If you query the DB partway
  through, you see an `open` invocation with no outcome and (depending on timing) few or no acts yet
  — that is a tick *in progress*, not a hang. Only suspect a real stall when the envelope stays
  `open` **past the function's max execution duration** AND no new acts are landing. Confirm with
  `invocation_show` (is `closed_at` set? are acts still accruing?) and `invocation_list --status
  open` — don't conclude from a single mid-run snapshot.

- **An orphaned open invocation** (still `open` well after the function could have run) means a tick
  died mid-loop — a function timeout, or the model stopping after a tool call without reaching
  `invocation_close`. It is harmless cruft (append-only), but it is a signal worth checking. The
  server's reaper expires the corresponding job's lease and requeues the map, so the next tick
  retries it.

- **`steward_ingest_delta: cognitive map not found` is an access-scoped not-found, NOT an auth
  failure.** Auth succeeding while read reach is missing surfaces as "not found," not `401`. It
  means the credential authenticated but its profile has no reach to that map — go back and check
  the `--team` / `--cogmap` reach you registered it with. A genuine auth failure (unregistered or
  revoked client id) is a `401` with an explicit message naming the client id.

> **Historical note (edges).** An early prod tick authored 17 nodes but **0 edges**:
> `assert_relationship` failed for every cogmap-homed source with *"no rows returned by a query that
> expected to return at least one row."* The edge-home lookup hard-filtered `anchor_table='kb_contexts'`,
> but a steward's authored-4 nodes are **cogmap-homed**, so it returned zero rows. Fixed: the backend
> now home-detects the source and branches kernel-vs-context (`assert_edge_from_source_home` /
> `assert_kernel_edge` in `DbBackend`). If you are looking at nodes authored edgeless in that window,
> a later tick retrofits `derived_from` + inter-node edges onto them.

## See also

- [machine-credentials.md](./machine-credentials.md) — the credential model: mint paths, reach,
  rotation, revocation. **Read this before deploying** — an unregistered client id 401s.
- [team-self-cognition-bootstrap.md](./team-self-cognition-bootstrap.md) — birth + bind a map (prerequisite).
- `packages/agent-workflows/steward/agent/lib/temper-auth.ts` — the strategy order, `temperFetch`,
  and why `TEMPER_M2M_AUDIENCE` is optional.
- `packages/agent-workflows/steward/agent/lib/model-config.ts` — the model resolution and why it is
  build-time.
- `packages/agent-workflows/steward/agent/schedules/steward.ts` — the fan-out dispatcher.
- `clients/temper-ts/src/credentials.ts` — the shared `ClientCredentials` mint.
- `docs/superpowers/specs/2026-07-01-t5-eve-steward-agent-directory-design.md` — the steward directory design.
- `docs/superpowers/specs/2026-07-05-steward-fan-out-drift-sweep-design.md` — the fan-out design.
