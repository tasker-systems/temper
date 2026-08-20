# Deploy a Steward Agent

**For operators** — someone who has stood up a Temper instance and wants to deploy the Eve
steward agent (and, optionally, the citation auditor) against it on Vercel.

## Outcome

By the end of this playbook you will have the steward agent deployed on its own Vercel
project, git-connected to your fork of the Temper repository, authenticated to your instance
as its own machine principal, and verified via a cron tick whose work is visible in the
database. The optional citation auditor deploys from the same project under a separate
credential and model.

One Vercel project ships **two agents**, and their separation is the product: the
**steward** (the root agent) distills a team's resources into cognitive-map nodes; the
**citation auditor** (a declared subagent) independently weighs the steward's citations.
They are colocated but must never share a credential, a model, or cogmap write reach.
Getting that wrong produces no deploy-time error and no log line — it produces an auditor
that 404s every audit it attempts.

> **The auditor's cron is live, but the auditor itself is optional — and that combination is
> safe.** Eve creates one Vercel Cron Job per file in `agent/schedules/`, so a schedule's
> *location* is the on/off switch; enabling or withdrawing it is a `git mv` plus an operator
> decision, never a side effect of a merge. A deployment that sets no auditor credential
> no-ops the tick with a log line instead of failing, so you can run the steward with no
> auditor at all and get a quiet, green cron. A deployment that sets a *partial* credential
> still fails loudly — that is a misconfiguration, not an absence.

## Prerequisites

- **A running Temper instance** — API, MCP, database, and IdP all stood up. See
  [self-host Temper](./self-host-temper.md). You need the instance origin and the IdP
  tenant from that deployment.
- **The enterprise configuration, or its equivalent** — the agent reads the same
  auth-identity contract as the instance. See [enterprise install](./enterprise-install.md)
  for the Temper-as-Authorization-Server variant; the env differences are called out below.
- **The agent's target cognitive map(s) must exist on the instance.** Birthing and binding a
  map is covered in `bootstrap-team-self-cognition.md`.
- **Each machine credential must be registered before its first call.** The credential model
  lives in [machine tokens](../concepts/machine-tokens.md). This playbook tells you which
  values these agents read and when to register.
- **Each principal must be admitted**, which registration does not do — see
  [Admission](#admission-the-gate-with-no-deploy-time-symptom) below. This is the gate that
  is invisible until every request 403s.

## Fork and deploy from your fork

The steward ships in the Temper repository as a self-contained Eve project with its own
`package.json`, npm lockfile, and TypeScript toolchain. The canonical deployment is
git-connected to the upstream repository; a self-hoster deploys from a fork.

### 1. Fork the repository

Fork the Temper repository on GitHub and clone your fork locally. The agent is a workspace
package inside the monorepo; it is deliberately **not** a Bun `workspaces` member, so it
never collides with the instance's toolchain.

### 2. Import your fork into a new Vercel project

Create a new Vercel project connected to your fork. Set the **Root Directory** to the
steward agent package — the directory whose `vercel.json` is the manifest inlined below.
Enable **Include source files outside of the Root Directory**: the agent takes a sibling
TypeScript client (`temper-ts`) as an npm `file:` dependency that lives outside its root
directory, and the build can only resolve it with that option on.

The Vercel project manifest at the agent root directory is:

```json
{
  "$schema": "https://openapi.vercel.sh/vercel.json",
  "installCommand": "npm install",
  "buildCommand": "npm run build"
}
```

It is already present in your fork. Modify the install/build commands only if your fork's
toolchain differs. A fresh clone builds the sibling dependency automatically — the `prebuild`
hook runs `build:dep`, which installs and builds `temper-ts` before the agent compiles, so
you never have to build the dependency by hand.

### 3. Deploy by pushing

The project is git-connected to your fork: a merge to your production branch produces a
production deployment; a push to any other branch produces a preview. Set the environment
variables (below) **before** the first deployment — several are read at build/discovery time
and fail fast when missing (e.g. `TEMPER_MCP_URL is required`, thrown by the connection's
`requireEnv` guard).

Vercel env changes still require a redeploy to take effect: setting a new `TEMPER_M2M_*` or
`STEWARD_MODEL` value in the dashboard does nothing until the next deployment. A cron running
against stale env looks exactly like a code bug.

> **Do not run `eve deploy` / `vercel deploy` from inside the agent directory.** The CLI
> uploads only the directory it is invoked from, and the sibling `temper-ts` dependency lives
> outside the root directory — a CLI deploy launched from the agent directory cannot carry
> it, and the build breaks on an unresolvable import. The git path clones the whole repo and
> honors "include source files outside the Root Directory". **Deploy by pushing.**

### Linking for local development

You still need a project link (`.vercel/project.json`) for `vercel env pull` and `eve dev`.
Use the Vercel CLI's own `link` picker rather than `eve link`: `eve link` enumerates all your
Vercel scopes including your personal account, and Vercel forbids using a personal account as
a scope, so on a login with one team plus a personal account `eve link` hits the personal
account and treats the failure as fatal.

```bash
vercel link
#   → pick the TEAM scope, then select your steward project.
#     Writes .vercel/project.json.

# non-interactive equivalent:
vercel link --project <your-steward-project> --team <team-slug> --yes
```

After linking, `vercel env pull` writes the AI Gateway key into `.env.local` for local
`eve dev`. You do not need `eve link` at all.

## Two principals, never one

The steward and the auditor each authenticate as their own **machine principal**. Three
things must never be shared, and eve's declared-subagent isolation is what enforces them at
runtime:

| | steward (root agent) | citation auditor (declared subagent) |
|---|---|---|
| Credential | `TEMPER_M2M_*` / `TEMPER_CONNECT_CONNECTOR` / `TEMPER_TOKEN` | `TEMPER_AUDITOR_M2M_*` / `TEMPER_AUDITOR_TOKEN` — **no Connect** |
| Model | `STEWARD_MODEL` (default `minimax/minimax-m3`) | `AUDITOR_MODEL` (default `anthropic/claude-haiku-4.5`) |
| Cogmap reach | `--cogmap <ref>` — **write**, it authors into its map | `--cogmap <ref>:ro` — **read-only**, or omitted entirely |

Why each one, because none is arbitrary:

- **Credential.** One credential is one `kb_events.emitter_entity_id`. A shared client id
  leaves the ledger unable to tell an audit from the citation it audits, and makes every
  steward-authored finding *self-authored* to the auditor — `AuditAuthority::Author`, whose
  denial arm renders **404**.
- **Model.** Code colocation does not create epistemic dependence, but **shared trained
  priors do**. Running the auditor on a different model is the one lever that genuinely
  attacks that, so the two defaults are deliberately **inverted** — each one's fallback is
  the other's primary, on the documented trade that an auditor which cannot run at all
  provides less independence than one that briefly runs on the same family. Set
  `AUDITOR_MODEL_FALLBACKS=""` if you would rather the tick fail than collapse the personas.
- **Reach.** A **writable** `--cogmap` grant classifies the auditor as
  `AuditAuthority::Author` for every finding in that map via `can_modify_resource`, and
  **404s every audit it attempts** — with nothing failing at deploy time and nothing in the
  logs naming the cause. `:ro`, or nothing. See `machine-token-contract.md` §C.

> **Connect is not an option for the auditor, structurally.** A Vercel Connect connector is
> *deployment*-scoped, and both agents share one deployment — so a Connect token would
> authenticate the auditor **as the steward**, which is precisely the collapse above. The
> auditor's fetch helper therefore offers only M2M and a static dev token.

## Register the machine credentials before the first tick

Registration is **fail-closed**: `resolve_machine_from_claims` (the single machine entry
point for both the API and MCP surfaces) is a lookup in `kb_machine_clients` or a **401**.
There is **no just-in-time create branch**. Register **first**, with **explicit reach**, then
deploy — a credential that has not been registered 401s on every call, forever.

Pick the mint path by **who owns the secret** — this is the one axis on which the two issuer
variants differ. The full model, including reach containment and rotation, is in
[machine tokens](../concepts/machine-tokens.md):

```bash
# ── Auth0-fronted instance ────────────────────────────────────────────────────
# You create the M2M application in Auth0; temper registers the client id it will present.
# TWO SEPARATE AUTH0 APPLICATIONS — one per principal. Never one app with two consumers.
temper admin machine provision --client-id <auth0-steward-client-id> --label "steward" \
  --owner-team <team> --team <team>:member --cogmap <cogmap-ref>

temper admin machine provision --client-id <auth0-auditor-client-id> --label "citation-auditor" \
  --owner-team <team> --team <team>:member --cogmap <cogmap-ref>:ro

# ── Instance where Temper is the Authorization Server (AS_ISSUER set) ─────────
# Temper mints the credential itself. Prints a `tmpr_…` client id and a one-time secret.
temper admin machine issue --label "steward" \
  --owner-team <team> --team <team>:member --cogmap <cogmap-ref>

temper admin machine issue --label "citation-auditor" \
  --owner-team <team> --team <team>:member --cogmap <cogmap-ref>:ro
```

- **`issue` requires an instance whose AS is Temper's own.** An instance has exactly one
  issuer, so a temper-minted token will not validate on an Auth0-fronted instance. Use
  `provision` there.
- **Reach is explicit and plural, never inferred from `--owner-team`**, which records the
  machine's *owner* and is never consulted for authorization. The steward needs **read** on
  the sources it distills (via `--team` membership) and **write** on the map(s) it tends. The
  auditor needs read on both the findings and their cited sources — `--team` membership — and
  **must not** have cogmap write.
- If you need to widen reach after the fact, use `temper team add-member` and
  `temper cogmap grant --to-profile <agent-profile-id> --write` — the profile already exists,
  because registration created it. For the auditor, grant **without** `--write`.
- Rotating the IdP **secret** needs no temper action (the client id is unchanged, so
  authorship history stays continuous). Rotating the IdP **application** needs
  `temper admin machine rebind`, which binds the new client id to the existing agent profile.

### Admission: the gate with no deploy-time symptom

**Registration is not admission.** `has_system_access` reads `kb_principal_standing` alone,
and **every mint door births a principal `denied`** — reach does not clear it, and neither
`provision` nor `issue` does.

```bash
temper admin access approve <agent-profile-id>   # per principal, both of them
```

This gate is issuer-independent — identical under Auth0 and the Temper AS — and it has **no
deploy-time signal whatsoever**: the token mints, the claims are perfect,
`kb_machine_clients` looks right, and every request returns `403 SYSTEM_ACCESS_REQUIRED`. If
you are debugging an agent whose credential is demonstrably valid, check standing before
anything else.

### The two issuer variants, side by side

Everything else in this playbook is issuer-independent. This is the whole delta:

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

> **Auth0 is the more permissive issuer, so it hides AS-mode defects.** Both rows above where
> the variants differ in *tolerance* rather than *value* are real bugs that stayed green for
> as long as Auth0 was the only issuer any client faced. The JSON-vs-form one is documented in
> `credentials.ts`: RFC 6749 §4 mandates form encoding, Auth0 tolerates JSON, and Temper's AS
> reads the body with `req.formData()` — so a JSON mint never reaches its grant branch and
> fails only on AS. **Anything verified only against Auth0 is verified against the lenient
> case.** Use `ClientCredentials` from `temper-ts` rather than hand-rolling a mint; it is the
> one implementation that is correct against both.

## Environment contract

Set these on the Vercel project (dashboard, or `vercel env add <NAME>`) **before** deploying.

| Variable | Required | Value / purpose |
|----------|----------|-----------------|
| `TEMPER_MCP_URL` | yes | The temper-mcp endpoint, e.g. `https://<instance>/mcp`. The agent's sole model-facing seam to Temper. Points at your instance by this value alone. |
| `TEMPER_API_URL` | yes | The temper REST base, e.g. `https://<instance>`. Distinct from `TEMPER_MCP_URL`; used by the code schedules' direct `POST /api/steward/dispatch`, `GET /api/steward/candidates`, and `POST /api/cognitive-maps/{id}/materialize`. |
| `TEMPER_M2M_CLIENT_ID` | prod | The machine client id — the Auth0 M2M app's id, or the `tmpr_…` id from `machine issue`. **When set, the agent mints its own `client_credentials` token** and this strategy wins over Connect and `TEMPER_TOKEN`. |
| `TEMPER_M2M_CLIENT_SECRET` | prod | The client secret. A Vercel env var only — never in code, never seen by the model. |
| `TEMPER_M2M_TOKEN_URL` | prod | The issuer's token endpoint: `https://<tenant>.auth0.com/oauth/token` for `provision`, or **your own instance's** `https://<instance>/oauth/token` for `issue`. |
| `TEMPER_M2M_AUDIENCE` | **only for an external IdP** | The API audience the minted token targets (must equal the API's `AUTH_AUDIENCE`). **OMIT it for a temper-issued (`tmpr_`) credential** — Temper's AS ignores a request-supplied audience entirely and mints with its server-side `AS_AUDIENCE`. |
| `STEWARD_MODEL` | optional | The primary model, as an AI Gateway model id (same form as the default, `minimax/minimax-m3`). A change needs a **redeploy**, and a typo fails the **build**. |
| `STEWARD_MODEL_FALLBACKS` | optional | Comma-separated AI Gateway model ids, tried in order after the primary fails. Defaults to `anthropic/claude-haiku-4.5`. Deduped, and the primary is dropped from the list if repeated there. |
| `TEMPER_CONNECT_CONNECTOR` | fallback | Vercel Connect connector id. Used **only** when `TEMPER_M2M_CLIENT_ID` is unset. **On an Auth0-fronted instance this cannot mint an app token** — see below. **Steward only** — a connector is deployment-scoped, so it can never identify the auditor. |
| `TEMPER_TOKEN` | dev only | An already-OAuth-obtained temper token. Drives `eve dev`. Cannot re-mint, so a 401 on it is terminal. |
| `TEMPER_AUDITOR_M2M_CLIENT_ID` | auditor, prod | The **auditor's own** machine client id — a second Auth0 M2M app, or a second `tmpr_…`. Never the steward's. |
| `TEMPER_AUDITOR_M2M_CLIENT_SECRET` | auditor, prod | The auditor's client secret. Vercel env only. |
| `TEMPER_AUDITOR_M2M_TOKEN_URL` | auditor, prod | Same issuer as the steward's — one instance, one issuer. Only the credential differs. |
| `TEMPER_AUDITOR_M2M_AUDIENCE` | **external IdP only** | Same rule as `TEMPER_M2M_AUDIENCE`: required for Auth0, **omitted** for a `tmpr_` credential. |
| `TEMPER_AUDITOR_TOKEN` | dev only | Static auditor bearer for `eve dev`. The unset-with-no-`CLIENT_ID` case **throws** rather than silently falling back to the steward's identity. |
| `AUDITOR_MODEL` | optional | The auditor's primary model. Defaults to `anthropic/claude-haiku-4.5` — deliberately **not** the steward's default. Same build-time freeze and redeploy-to-change semantics. |
| `AUDITOR_MODEL_FALLBACKS` | optional | Defaults to `minimax/minimax-m3` (the steward's primary — a documented availability trade). Set to `""` to make the tick fail rather than collapse the two personas onto one model. |

The auth strategy is resolved once, in `temper-auth.ts`, and is **machine-identity-first**:

1. `TEMPER_M2M_CLIENT_ID` present → mint via the OAuth `client_credentials` grant
   (`ClientCredentials` from `temper-ts`). The production path.
2. else `TEMPER_CONNECT_CONNECTOR` → a Vercel Connect app token.
3. else `TEMPER_TOKEN` → a static bearer.

The **same** helper serves both the MCP connection (which hands `mintM2mToken` to eve as
`auth.getToken`) and the code schedules (via `temperFetch`), so the two can never drift on how
they authenticate.

> **`temperFetch` re-mints once on a 401 and retries.** Refresh-ahead-of-expiry is not
> sufficient: a schedule resolves a token, then fans out N fetches, and Temper's AS mints
> **900-second** tokens by default — a tick outliving its token is ordinary, not exotic.
> Exactly one re-mint: a 401 that survives a fresh token is a real authorization failure
> (revoked credential, missing reach), and retrying forever would only bury it. A strategy
> that cannot mint (`TEMPER_TOKEN`) gets its 401 back untouched. `temperFetch` also carries
> the 5xx cold-start retry — **use it, never a bare `fetch`.**

### The model is config, and it is frozen at build time

Eve executes `agent.ts` at **build** time and freezes the resolved model into the compiled
manifest. There is no session, no request context, no DB anywhere near that resolution.
Consequences (`model-config.ts`):

- **Changing the model takes a redeploy, not a restart.** Env is the only lever eve offers.
- **The primary is validated against the AI Gateway catalog at compile time** — a typo in
  `STEWARD_MODEL` fails the build, not a 3am cron tick.
- **The fallbacks are not so validated.** They ride through the compile untouched, so a typo
  there surfaces at runtime, only when it is needed.
- **Fallbacks cover availability, never quality.** The Gateway walks the list on a 5xx, a
  rate limit, a model that is gone. No gateway can detect that a model fumbled a tool
  sequence — the mechanism for *that* is changing `STEWARD_MODEL` and redeploying.

The default (`minimax/minimax-m3`, falling back to `anthropic/claude-haiku-4.5`) is a cost
choice for the dev/community tier, where the loop runs hourly. Enterprise deployments
override it.

### AI Gateway credential

The agent's model calls run through the **Vercel AI Gateway**. On a deployed Vercel project
this authenticates automatically via **OIDC** (`VERCEL_OIDC_TOKEN` is injected at runtime) —
no credential to set. You only need a gateway key for **local** `eve dev`; after
`vercel link`, `vercel env pull` writes it into `.env.local`.

## Vercel Connect (the fallback path, and its dead end on Auth0)

`TEMPER_CONNECT_CONNECTOR` is still a live strategy in the code, used when
`TEMPER_M2M_CLIENT_ID` is unset. temper-mcp is a full OAuth 2.0 server and serves the
discovery endpoints (RFC 8414/9728), so Connect discovers what it needs from the MCP URL —
you do not hand it a client id/secret:

```bash
vercel connect create https://<instance>/mcp --name steward
```

- The URL is the same value as `TEMPER_MCP_URL`.
- The command **opens a browser** to complete the OAuth authorization.
- On success it prints a **connector ID** (`scl_…`) and a **UID** of the form
  `<host>/<name>`. Either form is a valid `TEMPER_CONNECT_CONNECTOR` value.

> **On an Auth0-fronted instance the Connect `app` path cannot mint a token, and this is not
> fixable from Temper's side.** Auth0 issues `client_credentials` only for a registered M2M
> application, and the Connect connector has no Auth0 M2M app behind it — its dynamic
> registration does not create one. **The `TEMPER_M2M_*` vars are the real path.** Connect
> remains in the code for instances where it does work.

## Verify

### Cron Jobs

Vercel → *Settings → Cron Jobs*: every `defineSchedule` becomes a Vercel Cron Job,
evaluated in **UTC**. Expect **three**: the steward dispatch tick and the region-materialize
tick, both hourly at `0 * * * *`, and the auditor dispatch tick at `30 * * * *` — half an
hour behind, so citations a steward tick authors are auditable within the same hour without
the two writing concurrently over one map. **The auditor's cron exists whether or not you run
an auditor**; with no auditor credential it logs `no auditor credential on this deployment —
skipping tick` and returns green. That is the intended resting state for a deployment that
does not use one.

### Logs

Vercel → *Observability → Logs*: the dispatch tick logs
`[steward-dispatch] tick <correlation-id> starting`, then the claimed-job count (or
`(no drift)`), then fans out. `[steward-materialize]` logs its candidate count. An
unregistered or under-reached credential shows up here as a `401` on `/dispatch`, not as
silence.

### The database is the source of truth, not the logs

Eve markdown task-mode discards the agent's own output: the model's reasoning and tool
results never reach Vercel logs, so logs cannot tell you what a tick *did* (they tell you what
was *dispatched*). The temper database is the source of truth — the **invocation envelope**
(`kb_invocations`: status / outcome / `closed_at` / `correlation_id`) and its **acts**
(`kb_events` joined on `invocation_id`). Read them with the MCP tools `invocation_show <id>`
(envelope + acts + outcome payload) and `invocation_list --status open` (any orphaned
envelopes), or over psql.

To connect to your instance's database, print a connection string from your Neon project:

```bash
neonctl connection-string main \
  --project-id <your-neon-project-id> \
  --org-id <your-neon-org-id> \
  --role-name <your-owner-role>
```

> **Always snapshot prod before a hand-run DDL/data change.** Create a copy-on-write Neon
> backup branch first (`neonctl branches create … --parent main`); restore with
> `neonctl branches restore main <backup-name>`.

Three things that read as bugs but aren't:

- **Ticks are long — an open envelope with a null outcome mid-run is normal, not a stall.**
  A tick that clears the threshold on a large delta runs for many minutes. If you query the
  database partway through, you see an `open` invocation with no outcome and (depending on
  timing) few or no acts yet — that is a tick *in progress*, not a hang. Only suspect a real
  stall when the envelope stays `open` **past the function's max execution duration** AND no
  new acts are landing. Confirm with `invocation_show` (is `closed_at` set? are acts still
  accruing?) and `invocation_list --status open` — don't conclude from a single mid-run
  snapshot.
- **An orphaned open invocation** (still `open` well after the function could have run) means
  a tick died mid-loop — a function timeout, or the model stopping after a tool call without
  reaching `invocation_close`. It is harmless cruft (append-only), but it is a signal worth
  checking. The server's reaper expires the corresponding job's lease and requeues the map,
  so the next tick retries it.
- **`steward_ingest_delta: cognitive map not found` is an access-scoped not-found, not an
  auth failure.** Auth succeeding while read reach is missing surfaces as "not found," not
  `401`. It means the credential authenticated but its profile has no reach to that map — go
  back and check the `--team` / `--cogmap` reach you registered it with. A genuine auth
  failure (unregistered or revoked client id) is a `401` with an explicit message naming the
  client id.

### Observing an auditor tick

The verdicts land in **`kb_citation_audits`**, one row per `(block, source)` weighed, each
carrying `audited_by_profile_id` (filled by the projector from the **owning event's**
emitter, never from an ambient principal — so a replay cannot re-attribute history).
`GET /api/resources/{id}/citation-audits` reads the attributed trail for any finding you can
already see. The dispatch tick itself logs:

```
[auditor-dispatch] tick <corr>: claimed N job(s): <job>→<cogmap>(M citation(s) across K finding(s))
```

Four readings that look like failures but are not:

- **`claimed 0 job(s) (no auditable citations)` is the steady state, not a fault.** Once
  every live citation this principal can reach has been weighed and nothing has changed
  since, there is genuinely nothing to do. It is the *expected* output of a healthy corpus
  between edits.
- **A job carrying fewer citations than the finding has is the filter working.** A finding
  with 8 citations and 1 already weighed contributes **7**. Compare against the auditable
  set, never against the live set.
- **An empty `citations` list on a claimed job is skipped, not failed.** The schedule skips
  jobs with no citations and the sweep re-offers their findings next tick — self-healing. If
  you see it on a *fresh* job, the expansion returned nothing and that is worth a look.
- **`404` on an audit write is the self-audit denial arm, and it is almost always a reach
  misconfiguration.** `AuditAuthority` is readability *minus* a self-audit denial, and both
  denial arms render `NotFound` (so the write can never become an existence oracle). If
  **every** audit 404s, the auditor almost certainly holds **write** on the cogmap — making
  it `Author` for every finding there — or is sharing the steward's client id. Check the
  `--cogmap` suffix before suspecting the data.

> **This is why the auditor must be its own principal, restated from the write side.** The
> filter is *per principal*, not per citation: a citation another principal weighed is still
> offered, because cross-principal audit is the entire premise. Point two agents at one
> client id and they become one principal — each silently suppressing the other's remaining
> work.

## Further reading

- **The instance this agent runs against:**
  [self-host Temper](./self-host-temper.md).
- **The enterprise (AS-mode) configuration:**
  [enterprise install](./enterprise-install.md).
- **The machine-token model (mint paths, reach, registration vs. admission):**
  [machine tokens](../concepts/machine-tokens.md).
- **The auditor's credential and reach constraints:** `machine-token-contract.md` §C —
  why a writable cogmap grant 404s every audit.
- **The auth strategy and `temperFetch`:** `temper-auth.ts`.
- **The model resolution and why it is build-time:** `model-config.ts`.
- **The fan-out dispatchers:** `steward.ts`, `materialize.ts`, `auditor.ts`.
- **The shared `ClientCredentials` mint, correct against both issuers:** `credentials.ts` —
  read it before hand-rolling one.
- **What the auditor weighs, and why its work list is handed to it:** the auditor subagent's
  `instructions.md`.
- **What the architecture fixes vs. what a deployment chooses:**
  [temperkb.io/operating/deployment](https://temperkb.io/operating/deployment).
