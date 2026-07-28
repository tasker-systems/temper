# Deploying an Eve agent to Vercel (the steward and the citation auditor)

This guide covers deploying the Temper **Eve agents** in `packages/agent-workflows/steward/` to
Vercel: the isolated-project tooling rules, how the deploy actually reaches Vercel (by **git**, and
why the CLI path is now a trap), the environment contract, registering the machine credentials, and
the verify loop.

**One Vercel project, two agents, and their separation is the product.** The project ships the
team-self-cognition **steward** (the root agent) and the Set 5 **citation auditor** (a declared
subagent at `agent/subagents/auditor/`). They are colocated but must never share a credential, a
model, or cogmap write reach — see [Two principals](#two-principals-never-one) below. Getting that
wrong produces no deploy-time error and no log line; it produces an auditor that 404s every audit it
attempts.

> **The auditor currently has NO schedule and cannot fire.** Its dispatcher lives at
> `disabled/auditor.schedule.ts`, outside the agent root, so eve registers no cron for it (eve
> creates one Vercel Cron Job per file in `agent/schedules/`). Enabling it is a `git mv` **plus an
> operator decision**, never a side effect of a merge. Everything below describes what to have in
> place *before* that decision.

Three prerequisites, all prior and separate:

- The agents' target cognitive map(s) must exist on the instance. Birthing and binding a map is
  covered in [team-self-cognition-bootstrap.md](./team-self-cognition-bootstrap.md).
- **Each machine credential must be registered before its first call.** The credential model — the
  two mint paths, reach, rotation, revocation — lives in
  [machine-credentials.md](./machine-credentials.md). This guide does not restate it; it tells you
  which values these agents read and when to register.
- **Each principal must be *admitted*, which registration does not do.** See
  [Admission](#admission-the-gate-with-no-deploy-time-symptom) — this is the gate that has bitten
  most recently, and it is invisible until every request 403s.

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

## Two principals, never one

The steward and the auditor each authenticate as their own **machine principal**. Three things must
never be shared, and eve's declared-subagent isolation is what enforces them at runtime:

| | steward (root agent) | citation auditor (declared subagent) |
|---|---|---|
| Credential | `TEMPER_M2M_*` / `TEMPER_CONNECT_CONNECTOR` / `TEMPER_TOKEN` | `TEMPER_AUDITOR_M2M_*` / `TEMPER_AUDITOR_TOKEN` — **no Connect** |
| Model | `STEWARD_MODEL` (default `minimax/minimax-m3`) | `AUDITOR_MODEL` (default `anthropic/claude-haiku-4.5`) |
| Cogmap reach | `--cogmap <ref>` — **write**, it authors into its map | `--cogmap <ref>:ro` — **read-only**, or omitted entirely |

Why each one, because none is arbitrary:

- **Credential.** One credential is one `kb_events.emitter_entity_id`. A shared client id leaves the
  ledger unable to tell an audit from the citation it audits, and makes every steward-authored
  finding *self-authored* to the auditor — `AuditAuthority::Author`, whose denial arm renders
  **404**. Spec §5.2.
- **Model.** Code colocation does not create epistemic dependence, but **shared trained priors do**.
  Running the auditor on a different model is the one lever that genuinely attacks that (spec §5.3),
  so the two defaults are deliberately **inverted** — each one's fallback is the other's primary, on
  the documented trade that an auditor which cannot run at all provides less independence than one
  that briefly runs on the same family. Set `AUDITOR_MODEL_FALLBACKS=""` if you would rather the
  tick fail than collapse the personas.
- **Reach.** A **writable** `--cogmap` grant classifies the auditor as `AuditAuthority::Author` for
  every finding in that map via `can_modify_resource`, and **404s every audit it attempts** — with
  nothing failing at deploy time and nothing in the logs naming the cause. `:ro`, or nothing. See
  `docs/auth/machine-token-contract.md` §C.

> **Connect is not an option for the auditor, structurally.** A Vercel Connect connector is
> *deployment*-scoped, and both agents share one deployment — so a Connect token would authenticate
> the auditor **as the steward**, which is precisely the collapse above. `auditorFetch` therefore
> offers only M2M and a static dev token.

## The machine credentials: register them BEFORE the first tick

Registration is **fail-closed**: `resolve_machine_from_claims` (`temper-services`, the single
machine entry point for both temper-api and temper-mcp) is a lookup in `kb_machine_clients` or a
**401**. There is **no just-in-time create branch**.

> **This invalidates the old two-phase "deploy, let the first tick create a blank profile, then
> grant it reach" flow.** That flow does not merely no-op now — every call 401s, forever, until the
> client id is registered. Register **first**, with **explicit reach**, then deploy.

Pick the mint path by **who owns the secret** — this is the one axis on which the two issuer
variants differ. The full model, including reach containment and rotation, is
[machine-credentials.md](./machine-credentials.md):

```bash
# ── Auth0-fronted instance (temperkb.io today) ───────────────────────────────
# You create the M2M application in Auth0; temper registers the client id it will present.
# TWO SEPARATE AUTH0 APPLICATIONS — one per principal. Never one app with two consumers.
temper admin machine provision --client-id <auth0-steward-client-id> --label "steward" \
  --owner-team <team> --team <team>:member --cogmap <cogmap-ref>

temper admin machine provision --client-id <auth0-auditor-client-id> --label "citation-auditor" \
  --owner-team <team> --team <team>:member --cogmap <cogmap-ref>:ro

# ── Instance where Temper is the Authorization Server (AS_ISSUER set) ────────
# Temper mints the credential itself. Prints a `tmpr_…` client id and a one-time secret.
temper admin machine issue --label "steward" \
  --owner-team <team> --team <team>:member --cogmap <cogmap-ref>

temper admin machine issue --label "citation-auditor" \
  --owner-team <team> --team <team>:member --cogmap <cogmap-ref>:ro
```

- **`issue` requires an instance whose AS is Temper's own.** An instance has exactly one issuer, so
  a temper-minted token will not validate on an Auth0-fronted instance. Use `provision` there.
- **Reach is explicit and plural, never inferred from `--owner-team`**, which records the machine's
  *owner* and is never consulted for authorization. The steward needs **read** on the sources it
  distills (via `--team` membership) and **write** on the map(s) it tends. The auditor needs read
  on both the findings and their cited sources — `--team` membership — and **must not** have
  cogmap write.
- If you need to widen reach after the fact, use `temper team add-member` and
  `temper cogmap grant --to-profile <agent-profile-id> --write` — the profile already exists,
  because registration created it. For the auditor, grant **without** `--write`.
- Rotating the IdP **secret** needs no temper action (the client id is unchanged, so authorship
  history stays continuous). Rotating the IdP **application** needs `temper admin machine rebind`,
  which binds the new client id to the existing agent profile.

### Admission: the gate with no deploy-time symptom

**Registration is not admission.** Since D11 (`20260720000110_repoint_predicates.sql`)
`has_system_access` reads `kb_principal_standing` alone, and **every mint door births a principal
`denied`** — reach does not clear it, and neither `provision` nor `issue` does.

```bash
temper admin access approve <agent-profile-id>   # per principal, both of them
```

This gate is issuer-independent — identical under Auth0 and AS — and it is the one that actually
bit on 2026-07-27. It has **no deploy-time signal whatsoever**: the token mints, the claims are
perfect, `kb_machine_clients` looks right, and every request returns `403 SYSTEM_ACCESS_REQUIRED`.
If you are debugging an agent whose credential is demonstrably valid, check standing before
anything else.

### The two issuer variants, side by side

Everything else in this guide is issuer-independent. This is the whole delta:

| | Auth0-fronted (`AUTH0_*`) | Temper-as-AS (`AS_ISSUER` set) |
|---|---|---|
| Who mints the secret | Auth0 — you create the M2M app | Temper — `machine issue`, printed **once** |
| Register with | `temper admin machine provision --client-id <auth0-id>` | `temper admin machine issue` (registers as it mints) |
| Client id shape | Auth0's opaque id | `tmpr_…` |
| `*_M2M_AUDIENCE` | **required** — must equal the API's `AUTH_AUDIENCE` | **omit it** — the AS ignores a request-supplied audience and mints with its server-side `AS_AUDIENCE` |
| `*_M2M_TOKEN_URL` | `https://<tenant>.auth0.com/oauth/token` | `https://<instance>/oauth/token` |
| Mint body encoding | form-encoded **or** JSON (Auth0 tolerates both) | **form-encoded only** |
| Admission (`access approve`) | required | required |
| `kb_machine_clients` registration | required | required |

> **Auth0 is the more permissive issuer, so it hides AS-mode defects.** Both rows above where the
> variants differ in *tolerance* rather than *value* are real bugs that stayed green for as long as
> Auth0 was the only issuer any client faced. The JSON-vs-form one is documented in
> `clients/temper-ts/src/credentials.ts`: RFC 6749 §4 mandates form encoding, Auth0 tolerates JSON,
> and Temper's AS reads the body with `req.formData()` — so a JSON mint never reaches its grant
> branch and fails only on AS. **Anything verified only against Auth0 is verified against the
> lenient case.** Use `ClientCredentials` from `temper-ts` rather than hand-rolling a mint; it is
> the one implementation that is correct against both.

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
| `TEMPER_CONNECT_CONNECTOR` | fallback | Vercel Connect connector id. Used **only** when `TEMPER_M2M_CLIENT_ID` is unset. **On the Auth0-fronted instance this cannot mint an app token** — see below. **Steward only** — a connector is deployment-scoped, so it can never identify the auditor. |
| `TEMPER_TOKEN` | dev only | An already-OAuth-obtained temper token. Drives `eve dev`. Cannot re-mint, so a 401 on it is terminal (by design — see `temperFetch`). |
| `TEMPER_AUDITOR_M2M_CLIENT_ID` | auditor, prod | The **auditor's own** machine client id — a second Auth0 M2M app, or a second `tmpr_…`. Never the steward's. |
| `TEMPER_AUDITOR_M2M_CLIENT_SECRET` | auditor, prod | The auditor's client secret. Vercel env only. |
| `TEMPER_AUDITOR_M2M_TOKEN_URL` | auditor, prod | Same issuer as the steward's — one instance, one issuer. Only the credential differs. |
| `TEMPER_AUDITOR_M2M_AUDIENCE` | **external IdP only** | Same rule as `TEMPER_M2M_AUDIENCE`: required for Auth0, **omitted** for a `tmpr_` credential. |
| `TEMPER_AUDITOR_TOKEN` | dev only | Static auditor bearer for `eve dev`. The unset-with-no-`CLIENT_ID` case **throws** rather than silently falling back to the steward's identity. |
| `AUDITOR_MODEL` | optional | The auditor's primary model. Defaults to `anthropic/claude-haiku-4.5` — deliberately **not** the steward's default. Same build-time freeze and redeploy-to-change semantics. |
| `AUDITOR_MODEL_FALLBACKS` | optional | Defaults to `minimax/minimax-m3` (the steward's primary — a documented availability trade). Set to `""` to make the tick fail rather than collapse the two personas onto one model. |

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
  Job, evaluated in **UTC**. Expect **two**, both hourly (`0 * * * *`): the steward dispatch tick
  and the region-materialize tick. **A third — the auditor's, at `30 * * * *` — appears only once
  `disabled/auditor.schedule.ts` is moved into `agent/schedules/`.** If you see three and did not
  make that decision, someone enabled a production cron in a merge; that is the thing the file
  lives outside the agent root to prevent.
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

### How an auditor tick works — the queue is cogmap-grained, the work is CITATION-grained

Same fan-out shape as the steward, with one extra server-side step that is the whole point of the
design. `POST /api/auditor/dispatch` runs **reap → sweep → expand → group → enqueue → claim**:

- `audit_drift_sweep` selects **findings** whose citations are uncovered or have gone stale.
- **`resource_auditable_citations(finding, principal)` then expands each one into the citations
  *this principal* has not already weighed** — live citations minus what it already audited, plus
  any that went stale since. This is the step that makes the invariant structural.
- Those citations group by cogmap into **one job per map**, because `kb_workflow_jobs` enforces
  single-flight on `(cogmap_id, persona, dispatch_type)` and a per-finding enqueue would have
  `ON CONFLICT DO NOTHING` silently discard all but the first.

So `ClaimedAuditJob.citations` is a list of `(finding, block, source)` triples, **every one of which
is work this credential has not done**. The session needs no skip logic and is given no opportunity
to exercise any — the guarantee is held by *what the session receives*, not by an instruction it is
asked to follow.

> **This is why the auditor must be its own principal, restated from the write side.** The filter is
> *per principal*, not per citation: a citation another principal weighed is still offered, because
> cross-principal audit is the entire premise. Point two agents at one client id and they become one
> principal — each silently suppressing the other's remaining work.

Before it shipped (`20260727000050`), the payload carried bare finding ids and the dispatch prompt
told the session it "does not need to re-check whether they need auditing" — so a finding with 1 of
8 citations weighed was re-worked in full, every tick. If you are reading logs from before that,
that is what you are seeing.

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

### Observing an auditor tick

The verdicts land in **`kb_citation_audits`**, one row per `(block, source)` weighed, each carrying
`audited_by_profile_id` (filled by the projector from the **owning event's** emitter, never from an
ambient principal — so a replay cannot re-attribute history). `GET /api/resources/{id}/citation-audits`
reads the attributed trail for any finding you can already see. The dispatch tick itself logs:

```
[auditor-dispatch] tick <corr>: claimed N job(s): <job>→<cogmap>(M citation(s) across K finding(s))
```

Four readings that look like failures but are not:

- **`claimed 0 job(s) (no auditable citations)` is the steady state, not a fault.** Once every live
  citation this principal can reach has been weighed and nothing has changed since, there is
  genuinely nothing to do. It is the *expected* output of a healthy corpus between edits.
- **A job carrying fewer citations than the finding has is the fix working.** A finding with 8
  citations and 1 already weighed contributes **7**. Compare against
  `resource_auditable_citations`, never against `resource_live_citations`.
- **An empty `citations` list on a claimed job means a pre-deploy payload.**
  `AuditJobPayload::citations` is `#[serde(default)]` precisely so a job enqueued before the grain
  change deserializes to empty rather than failing the claim; the schedule skips those, and the
  sweep re-offers their findings next tick. Self-healing — but if you see it on a *fresh* job, the
  expansion returned nothing and that is worth a look.
- **`404` on an audit write is the self-audit denial arm, and it is almost always a reach
  misconfiguration.** `AuditAuthority` is readability *minus* a self-audit denial, and both denial
  arms render `NotFound` to match the evidence read's zero-rows→404 (so the write can never become
  an existence oracle). If **every** audit 404s, the auditor almost certainly holds **write** on the
  cogmap — making it `Author` for every finding there — or is sharing the steward's client id.
  Check the `--cogmap` suffix before suspecting the data.

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
- `clients/temper-ts/src/credentials.ts` — the shared `ClientCredentials` mint, correct against
  **both** issuers. Read it before hand-rolling one.
- `docs/auth/machine-token-contract.md` §C — the auditor's credential + reach constraints, and why a
  writable cogmap grant 404s every audit.
- `packages/agent-workflows/steward/disabled/auditor.schedule.ts` — the auditor dispatcher, and the
  gate list to read **before** restoring it.
- `packages/agent-workflows/steward/agent/subagents/auditor/instructions.md` — what the auditor
  weighs, the scale, and why its work list is handed to it rather than derived.
- `docs/superpowers/specs/2026-07-01-t5-eve-steward-agent-directory-design.md` — the steward directory design.
- `docs/superpowers/specs/2026-07-05-steward-fan-out-drift-sweep-design.md` — the fan-out design.
- `docs/superpowers/specs/2026-07-23-set5-adversary-citation-audit-design.md` — the citation-audit
  design (§5.2 credential isolation, §5.3 model isolation, §6 dispatch).
