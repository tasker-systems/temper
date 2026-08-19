# Steward: shared M2M credentials (seeding temper-ts) + config-driven model selection

**Date:** 2026-07-13
**Context:** `@j-cole-taylor/temper` · **Mode/effort:** plan / medium
**Follows:** Phase A (PR #351), Phase B1 (PR #374), Phase B2 (PR #377, hardened by #384/#388), PR #391 (form-encoded mint)
**Status:** designed 2026-07-13

## Problem

Two unrelated defects in `packages/agent-workflows/steward`, bundled because they are the same
file's two reasons to change.

### 1. The steward cannot consume a temper-issued credential

`temper admin machine issue` mints a `tmpr_…` credential against temper's **own** Authorization
Server (Phase B1). Per `docs/guides/machine-credentials.md`, such a client must **omit** `audience`
— temper's AS ignores a request-supplied audience entirely and mints with its server-side
`AS_AUDIENCE`.

The steward cannot. `agent/lib/temper-auth.ts:47` reads:

```ts
audience: requireEnv("TEMPER_M2M_AUDIENCE"),
```

and `requireEnv` throws on absence. So the steward only fits the `provision` (Auth0-mints) shape.
It is temper's only first-party TypeScript M2M client, and it is pinned against **nothing** —
while `tests/contracts/m2m-token-request.json` names its own consumers and predicts this exact gap:

> *"Adding a client (temper-py, temper-ts) means pinning it against this file too."*

This matters beyond the steward. The reason to care about the AS path is **self-hosted / SAML
instances**, where there is no external IdP and `issue` is the *only* way to mint a machine
credential — and where the intended end state is that team owners mint scoped credentials for their
own agents, so that **not every agent runs at admin reach**. The steward is the proof that a
team-scoped, temper-issued credential can actually drive a deployed agent.

**We cannot prove that on prod.** temperkb.io is Auth0-fronted with no `AS_*` vars, and an instance
has exactly one issuer — a `tmpr_` token would not validate there at all. And we cannot test against
the enterprise self-hosted instance. So the proof has to come from **faithful mocks in-test**, not
from standing up an AS.

### 2. The steward's mint has a bug its own port already documented

`clients/temper-rb/lib/temper/credentials.rb` says it was **ported from** the steward, then names two
deliberate divergences. One is a bug report on the original:

> *"`#refresh!` exists. Refresh-ahead-of-expiry alone is insufficient: **the steward resolves a token
> once per tick, so a tick outliving its cached token takes a 401 nothing recovers.** A Sidekiq job
> holding a token across a long unit of work has precisely that bug. Re-mint ON 401."*

The gem fixed it. The steward still has it. It is latent today only because Auth0's TTL is long; the
AS's `AS_ACCESS_TTL_SECONDS` **defaults to 900 seconds**, which makes a tick outliving its token
ordinary rather than exotic. The other divergence — a mutex around the cache — is Puma-specific and
correctly absent from a single-threaded serverless function.

So the fix is not a design problem. It is **back-porting the gem's improvements into the original**,
and pinning both clients to one contract.

### 3. The model is a hardcoded string literal

`agent/agent.ts:15` pins `model: "minimax/minimax-m3"` with a comment declaring it a TRIAL and naming
`anthropic/claude-haiku-4.5` as the fallback "if it fumbles the tool sequence." Neither the primary
nor the fallback is configurable: changing either means editing source. An enterprise deployment that
wants sonnet/opus must fork the file.

## What already exists (verified 2026-07-13)

- **`tests/contracts/m2m-token-request.json`** — the cross-language wire contract. Consumers today:
  `clients/temper-rb/spec/temper/credentials_spec.rb` (client emits it) and
  `packages/temper-cloud/tests/integration/oauth/client-credentials.test.ts` (server accepts it).
- **The AS is well covered server-side.** That integration test asserts the machine claim shape, the
  contract request, `invalid_request` on a JSON body, `invalid_client` on a wrong secret, a revoked
  client, the rotation grace window, and HTTP Basic credentials.
- **The server-side machine story is covered end-to-end in Rust e2e**: `machine_gate_e2e`,
  `machine_registration_authz_e2e`, `auth_seam_m2m_e2e`, `auth_seam_parity_e2e`, `eddsa_auth_test`.
  **The uncovered surface is the client.**
- **`clients/` is the home of published first-party SDKs** — `temper-rb` ships a gemspec with a
  rubygems push host. `Temper::Credentials` is a module *inside* the gem, not a separate artifact.
- **eve resolves the model at BUILD time.** `compileAgentConfig`
  (`node_modules/eve/dist/src/compiler/normalize-agent-config.js`) executes `agent.ts`, resolves the
  model id against the AI Gateway catalog for its context window, and freezes the result into the
  compiled manifest (`compiledAgentConfigSchema`, version 32). There is no session, no request
  context, and no DB near that resolution — **env is the only lever eve offers.**
- **`providerOptions` rides through the compile untouched** — `compiledRuntimeModelReferenceSchema`
  carries `providerOptions`, and `defineAgent`'s `modelOptions.providerOptions` are forwarded to the
  model call.
- **The AI Gateway has a native ordered fallback list**: `providerOptions.gateway.models` tries the
  primary, then each entry in order, returning the first success.
- **The Vercel project `steward-agent`** has Root Directory `packages/agent-workflows/steward`,
  framework preset `eve`, install command `npm install`.
- **The steward's live principal is not an admin** — its agent profile
  (`agent-y23aqxuvzjysb5n8laueuigixoftcwyu`) holds only the auto-join `watcher` role on
  `temper-system`; its reach comes from cogmap grants. Its `kb_machine_clients` row is
  `issuer: auth0-m2m`, `team_id: null` (teamless), labelled `"backfilled: …"`. It predates the
  registration model rather than exercising it.

## Design

### 1. `clients/temper-ts` — seeded with its credentials module

The shared module is **the credentials layer of the TypeScript SDK**, mirroring the gem exactly. Not
a `packages/` micro-package: the gem does not ship a separate `temper-rb-auth` artifact, and neither
should we.

```
clients/temper-ts/
├── package.json          # name: "temper-ts", zero runtime deps, tsc → dist
├── src/
│   ├── credentials.ts    # BearerToken | ClientCredentials behind one interface
│   ├── index.ts
│   └── testing/
│       └── mock-issuer.ts   # the faithful mock, exported under ./testing
└── tests/
    ├── credentials.test.ts  # the client emits the contract; behaves against both issuers
    └── contract.test.ts     # the contract file is honored, field by field
```

`ClientCredentials` takes **explicit params** — `tokenUrl`, `clientId`, `clientSecret`, `audience?`
— and reads no environment. It caches against an **absolute** expiry with a 60s skew, mints
form-encoded, and exposes `refresh()` for the on-401 path. `audience` is optional because it is
Auth0's, not the protocol's. This is `Temper::Credentials` transliterated, and deliberately so: two
first-party clients that mint differently is the bug class this entire arc has been fighting.

The cache is a plain instance field. The gem's mutex exists because Puma is threaded; the steward's
runtime is not, and the gem's own comment says exactly that.

**Package name.** `temper-ts` (matching `temper-rb`). A `file:` dependency resolves by path but the
dependency *key* must match the package's `name`, so the name has to be chosen now. It is renameable
before first publish at the cost of one `package.json` key and one import specifier.

### 2. The faithful mock issuer

A real `node:http` server on an ephemeral port with **two personalities, built from the contract
file**:

| | Auth0-shaped | temper-AS-shaped |
|---|---|---|
| `audience` | required (rejects without) | ignored entirely |
| JSON body | tolerated (Auth0's extension) | `invalid_request`, never a 500 |
| `expires_in` | long | 900 |
| `client_secret_basic` | accepted | accepted, preferred over the body |
| rotation | — | previous secret valid inside its grace window |
| refresh token | never issued | never issued |

**Faithfulness is transitive, and that is the whole argument.** The mock is built from
`tests/contracts/m2m-token-request.json`; the **real** AS is asserted against that same file by
`packages/temper-cloud/tests/integration/oauth/client-credentials.test.ts`. A mock that drifts from
the AS breaks the AS's own test first. Nobody imports anybody:

```
   temper-ts tests ──┐                    ┌── temper-cloud AS integration test
                     ├──> contract file <─┤        (the real handleToken)
   temper-rb spec ───┘                    └── (future temper-py)
```

The mock is exported under a `./testing` subpath so the steward's own tests can drive it without
re-implementing it.

### 3. The steward composes the module

`agent/lib/temper-auth.ts` keeps what is genuinely steward-specific and delegates the rest:

- **Stays:** the `TEMPER_M2M_*` env resolution, `requireEnv`, and the machine-identity-first strategy
  ordering (M2M → Vercel Connect → static `TEMPER_TOKEN`). Connect and `TEMPER_TOKEN` are eve/Vercel
  concepts with no business in a general-purpose SDK.
- **Goes:** the hand-rolled mint. It becomes a `ClientCredentials` instance built from env, with
  `TEMPER_M2M_AUDIENCE` read **if present** rather than required.
- **New:** `temperFetch(url, init)` in the lib — mints, sends, and on a **401** calls `refresh()` and
  retries **exactly once**, folding in the existing 5xx `fetchWithRetry`. The three schedule call
  sites (`dispatch`, `candidates`, `materialize`) stop threading tokens by hand and call it. This is
  the fix for the bug the gem documented: today the schedules resolve one token and hold it across N
  parallel fetches.

The MCP connection (`agent/connections/temper.ts`) continues to hand eve a `getToken` returning
`{ token, expiresAt }`; eve refreshes ahead of expiry from that. Its auth ordering is unchanged, and
it shares the same `ClientCredentials` instance — the connection and the schedules still cannot drift
on how they authenticate, which is why the shared module existed in the first place.

### 4. Consumption: a `file:` dependency as a deliberate bridge

The steward is workspace-isolated by design (its own TS 7 toolchain and npm lockfile; deliberately
**not** a bun workspaces member), and its Vercel project builds from
`packages/agent-workflows/steward` as Root Directory. A sibling `clients/temper-ts` is therefore
**not in its build context**.

- `package.json`: `"temper-ts": "file:../../../clients/temper-ts"` (npm symlinks it).
- `prebuild` script builds the dependency before `eve build`, so the symlinked package's `dist` is
  present without relying on npm's `prepare` semantics for `file:` specs.
- The Vercel project setting **"Include source files outside of the Root Directory in the Build
  Step"** must be enabled on `steward-agent`.

**This is scaffolding, not structure.** Every future agent project would otherwise inherit the same
toggle. It disappears the moment temper-ts publishes to npm — at which point each agent takes a
normal dependency and the toggle goes back off. The `file:` dep is a bridge across days, not a
permanent coupling.

**One Vercel project per agent stays the rule** (one project = one machine principal = one reach),
because Vercel env vars are per-project and two agents sharing a project would share a credential —
collapsing the very boundary this work exists to establish. That is a *convention* rather than a
platform limit (a single eve app could namespace `STEWARD_M2M_*` / `ANALYST_M2M_*` and route each
schedule to its own principal), and it is worth revisiting when a second agent actually exists. It
does not affect this design either way: `clients/temper-ts` is outside `packages/agent-workflows`
regardless of where the root is drawn.

### 5. The contract file gains a response section

Extend `tests/contracts/m2m-token-request.json` with the two things the mock must be faithful to and
on which **both real issuers already agree**:

- **`response`** — `access_token`, `token_type: "Bearer"`, `expires_in`, and **no refresh token**
  (RFC 6749 §4.4.3: the credential *is* the refresh mechanism).
- **`credential_transport`** — `client_secret_post` (form body) and `client_secret_basic` (header,
  preferred by the AS when present, per RFC 6749 §2.3.1).

Client **policy** stays out: the 60s skew and the single on-401 re-mint are client behavior, not wire
shape. They are pinned in the temper-ts tests instead.

The existing AS test already asserts Basic, the grace window, and the absent refresh token, so the
extension should be additive on that side. **Verify rather than assume** — re-run the temper-cloud
integration suite and the Ruby gem spec after touching the file.

### 6. Model selection

```ts
// agent/lib/model-config.ts — pure, unit-tested
STEWARD_MODEL            → primary,   default "minimax/minimax-m3"        (today's behavior)
STEWARD_MODEL_FALLBACKS  → ordered,   default "anthropic/claude-haiku-4.5" (comma-separated)
```

```ts
export default defineAgent({
  model: modelConfig.primary,
  modelOptions: { providerOptions: { gateway: { models: modelConfig.fallbacks } } },
  description: "...",
});
```

Parsing trims, drops empties, dedupes the primary out of the fallback list, and omits
`gateway.models` entirely when the list is empty. Defaults reproduce today's behavior exactly, so a
deploy with no new env set is a no-op.

Three consequences, stated rather than discovered:

- **Config is build-time, not runtime.** eve freezes the model into the compiled manifest, so
  changing `STEWARD_MODEL` on Vercel requires a **redeploy**. There is no runtime model switch, and
  no DB-driven selection is possible — the resolution happens before any request context exists.
- **The primary is build-validated; the fallbacks are not.** eve resolves the primary against the
  Gateway catalog and fails the build on an unknown id. Entries in `gateway.models` ride through
  untouched — a typo there fails at runtime, *only when it is needed*. Tests can assert the
  `provider/model` shape; nothing can prove existence.
- **eve's `apply-model-name` tooling stops working.** It rewrites the `model` **string literal** in
  source and explicitly bails on "an env reference." The TUI's model-set command is the price of
  config-driven selection. Accepted.

**The honest limit:** Gateway fallback fires on *availability* failure — 5xx, rate limit, model
unavailable. It **cannot** detect "minimax fumbled the tool sequence," which is what `agent.ts`'s
current comment actually worries about. That is a quality judgment; its mechanism is changing
`STEWARD_MODEL` and redeploying — which is exactly what making it configurable buys.

### 7. CI — a test that runs nowhere is not a test

The steward is workspace-isolated and the repo pre-commit never touches it, so a suite added there
runs **nowhere** by default. That is the precise rot this repo has been burned by before
(`streaming_ingest_test` hid behind a green tick for months).

- A CI job runs `clients/temper-ts`'s tests (`npm ci && npm test`), alongside the existing
  `test-ruby` backstop for the gem.
- A CI job runs the steward's tests the same way, from **inside** its directory (a root install
  inherits the root's bun `overrides` and fails).
- Both are wired into `detect-ci-scope` so a docs-only change still skips them.

Runner: **vitest** as a devDependency in each isolated project. `node:test` would be zero-dep, but
the sources use `.js` import specifiers that Node's type-stripping will not resolve to `.ts`, and
vitest is already this repo's TypeScript runner.

## Verification

1. **Probe the deploy shape FIRST.** Land a trivial `file:` dep + the Vercel toggle and take a
   preview deployment before building anything else. This is the riskiest assumption in the plan and
   the cheapest to falsify. If eve's bundler cannot resolve the symlinked package, fall back to
   publishing temper-ts at `0.0.x` and taking a normal dependency.
2. Unit: model-config parsing (trim, dedupe, empty, absent).
3. Unit: `ClientCredentials` against **both** mock personalities — audience present and absent, JSON
   refusal, Basic, expiry skew, and a single re-mint on 401.
4. Unit: the steward's env composition and `temperFetch`'s 401 path.
5. Regression: the temper-cloud AS integration suite and the Ruby gem spec still pass after the
   contract file changes.
6. Prod is a no-op: `TEMPER_M2M_AUDIENCE` remains set, and the default model is today's model.

## Acceptance criteria

- The steward mints successfully against a temper-AS-shaped issuer with **no** `TEMPER_M2M_AUDIENCE`
  set, and against an Auth0-shaped issuer with it set — both proven in-test against the mock.
- A 401 mid-tick triggers exactly one re-mint and retry, and the tick completes.
- `temper-ts` is a real package with a credentials module, a mock issuer, and contract tests, and the
  steward composes it rather than duplicating it.
- `STEWARD_MODEL` / `STEWARD_MODEL_FALLBACKS` drive model selection; unset reproduces today's
  behavior byte for byte.
- Both new suites run in CI.
- Prod behavior is unchanged.

## Out of scope

### Rejected

- **Token-endpoint discovery** (`/.well-known/oauth-authorization-server`). It *would* work —
  verified: temperkb.io's metadata correctly returns Auth0's `token_endpoint`, and an AS-mode
  instance returns its own. But the credential and its endpoint are handed out together, and the
  Ruby gem takes `token_url:` explicitly. Two first-party clients inferring their issuer differently
  is the exact bug class this arc has been fighting. Explicit config, one hop fewer.
- **A custom `LanguageModel` wrapper doing its own model failover.** Reimplements what the Gateway
  provides natively, and would have to satisfy `WORKFLOW_SERIALIZE` to survive durable-workflow step
  boundaries. Strictly worse.
- **A `packages/temper-m2m` micro-package.** The gem does not ship credentials as a separate
  artifact; the SDK is the right home.
- **Proving the client in Rust e2e.** A Rust test cannot exercise the steward's TypeScript mint — it
  would test a different client and tell us nothing about this one.

### Deferred

- **Re-minting prod's steward principal** as team-owned with explicit `--team`/`--cogmap` reach. It
  is a registration act needing no code change, and it exercises B2's containment on a live agent —
  but it is a separate story from this one.
- **Publishing temper-ts to npm.** Its own goal, starting this week. It is the exit condition for the
  `file:` bridge.
- **A shared agent-glue module** (`packages/agent-workflows/shared/`) for the Connect strategy,
  `fetch-retry`, env resolution — and, with it, the question of whether several agents can share one
  Vercel project with namespaced credentials. Speculative until a second agent exists and actually
  shares something.
- **Quality-based model failover.** No gateway can detect a fumbled tool sequence. Config + redeploy
  is the mechanism.
- **Standing up an AS-mode instance.** The mocks are the proof; the contract makes them faithful.
