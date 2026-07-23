> This is the **@temper mention agent** — an Eve agent that answers Slack app mentions.
> It is a workspace-isolated Eve project; run tooling from THIS directory, not the repo root
> (`cd packages/agent-workflows/mention && npm install`). A root `npm install` inherits the root's
> bun overrides and fails. It is deliberately NOT a member of the root `workspaces` array.

**Scope:** the inbound pipe, the account link, and a **read-only** turn under the mentioning
human's own identity. A linked user with a usable credential now gets a model turn whose tools
reach temper **as them** — never as a machine, and never as anyone else. Writes are out of scope
(see the allow-list below).

**Every mention costs two signed calls, and the second one decides.** First
`POST /internal/slack/link-state` asks *what to say to this person*; then, on the `linked` arm
only, `POST /internal/slack/mint` asks for that person's access token. The four outcomes:

| link-state | mint | Result |
| ---------- | ---- | ------ |
| `unlinked` | not called | ephemeral authorize-URL prompt, **drop** |
| `linked` | `token` | **dispatch** — `return { auth }` |
| `linked` | `refused` / `not_vaulted` | ephemeral "no stored credential, retrying won't fix it, re-link", **drop** |
| `linked` | `refused` / `standing` | ephemeral "an admin has to approve your access" — **never** re-link, **drop** |
| `linked` | `refused` / `not_linked` | ephemeral "mention me again for a fresh link" (the link vanished mid-mention), **drop** |

> **The mint's refusal is TYPED, and the remedy splits on it.** `MintOutcome` is two arms
> (`token` / `refused`), with the refusal carrying `reason` (`not_linked` / `not_vaulted` /
> `standing`) and, under `standing`, the `kind` of the `temper_principal::Refusal`. Three nested
> tags — `status`, `reason`, `kind` — distinct on purpose so they nest without collision.
>
> **Re-linking fixes `not_vaulted` and NOTHING else.** Every `standing` refusal is an admin
> decision; `temper slack disconnect` cannot move principal standing, so offering it there is a
> loop with no exit. The former flat `revoked` arm offered exactly that to everyone, which is the
> false remedy the linked-identity state machine (PR #522) exists to end. `standingReply`
> (`agent/lib/identity.ts`) is the one place the six admit-reachable kinds collapse onto three
> remedies: admin-decides (`denied` / `no_standing` / `revoked` / `deactivated`), wait
> (`requested`), and our-bug (`unrecognized_standing`, which also logs the raw value).
>
> The TS mirror of `Refusal` in `agent/lib/mint.ts` is **hand-written and ungated** — this package
> is workspace-isolated and cannot import the ts-rs export in temper-ui. Emitting it here and
> gating the drift is Task 8/9 of the linked-identity plan. Until then a `never` binding at each
> switch is the only thing that notices a new Rust variant, and it only notices at compile time.

**The mint is PRE-FLIGHTED in `onAppMention`, deliberately.** The connection's `getToken` mints
too, so a successful turn mints twice — that is cheap on purpose: the server hands back the
**cached** access token without touching the refresh token whenever it outlives a 5-minute skew
(`slack_grant_vault_service.rs`, `mint_access_token`), so the second mint is a row read, never a
second spend of the grant. What the pre-flight buys is the distinct replies. If the first
mint failure happened inside `getToken` it would be a **failed turn**, routed to the `turn.failed`
handler, which says one deliberately detail-free sentence — collapsing "you were never vaulted",
"an admin hasn't approved you" and "the network hiccuped" into a single generic error, when only
the last is worth retrying. Do not move the mint into `getToken` alone.

**The failure replies must stay distinct**, and `tests/identity.test.ts` +
`tests/slack-dispatch.test.ts` assert it (unlinked and every mint refusal must be different
strings, and only the unlinked one may carry a URL — every refusal is reached from link-state's
`linked` arm, which carries no `authorize_url`, so any URL in that copy is invented).

**Only `not_vaulted` may offer `temper slack disconnect`**, which routes the user back through the
unlinked arm and its fresh URL. A standing refusal offers the admin instead, and
`offers re-link ONLY where re-linking is the actual remedy` (`tests/identity.test.ts`) fails if
that leaks back. The one other reply that says "mention me again" is the `not_linked` race — and
there it is correct, because the user genuinely IS unlinked by then.

**The endpoint answers "what do I say?", never "mint me a URL".** Asking for a URL unconditionally —
which is what the first cut did — re-prompted an already-linked user to link again on **every**
mention, forever, and minted a junk `kb_slack_link_intents` row each time. `agent/lib/link.ts`
returns a `LinkState` discriminated union mirroring the Rust `SlackLinkStateResponse`
(`#[serde(tag = "status")]`); `agent/channels/slack.ts` branches on `status`. Only the `unlinked`
arm costs a write. If you find yourself adding a nullable field to that union, you are rebuilding
the bug: the two arms carry disjoint data on purpose.

**Every non-dispatching arm delivers a channel-root ephemeral, and then drops.** The unlinked
message carries a credential and must never reach a public channel; the two broken-credential
messages name the user's handle and their account state, which is nobody else's business. The
dispatching arm sends no ephemeral of its own — the model's answer *is* the reply, delivered by
the `message.completed` override, which is ephemeral for the stronger reason that it was produced
under that human's full temper reach. In all of it: no task numbers, no dates, no internal plans
in user-facing copy.

> **Deliver via `ctx.slack.request("chat.postEphemeral", { channel, user, text })`, NOT
> `ctx.thread.postEphemeral`.** The thread helper inherits the mention's `thread_ts`, so the
> ephemeral lands in a thread the user isn't viewing — invisible, no badge, indistinguishable from a
> dropped mention (this cost a live debugging session). The raw request also returns `{ ok, error }`
> instead of throwing on `ok:false`, so a delivery failure surfaces (a public, credential-free error
> line) instead of being swallowed by eve's dispatcher. Do not "simplify" it back.

**The invariant is exactly ONE `thread.post`, not zero.** It lives at the end of
`deliverEphemeral` (`agent/lib/ephemeral.ts`) and fires only when `chat.postEphemeral` itself
returned `ok:false`. It carries `ephemeralFailureNotice(error)` — **Slack's own error code and
nothing else**: never the undelivered reply, never a URL, never anything derived from the caller's
reach. It is public on purpose, because silence was the one outcome worth refusing. Any *other*
`thread.post` in this agent is a bug. (This page previously claimed "zero", which was false; a
wrong invariant is a maintenance hazard, because the next person greps for it, finds the one real
call, and cannot tell a deliberate exception from a regression.)

**Public sinks are not only `thread.post` — `thread.startTyping` is one too.** eve's
`reasoning.appended` default pushes `firstNonEmptyLine(event.reasoningSoFar)` into the typing
status, and its `actions.requested` default pushes `state.pendingToolCallMessage` (the model's own
mid-turn narration) or the tool names. Both are overridden in `agent/channels/events.ts` with the
constant `WORKING_STATUS`, and nothing writes `pendingToolCallMessage` any more — populating it
was feeding an un-overridden public sink. `turn.started` and `authorization.completed` are left as
eve's defaults, verified content-free. `tests/events.test.ts` derives all of this from eve's real
`defaultEvents` **at runtime**, so an eve upgrade that ADDS a default handler fails the test
instead of silently installing a new sink.

**DMs are explicitly refused.** `agent/channels/slack.ts` supplies `onDirectMessage: async () =>
null`. This is load-bearing: eve resolves `onDirectMessage ?? defaultOnDirectMessage`, and the
default **dispatches unconditionally** — no `decideIdentity`, no `principalType === "user"` gate,
no link-state, no mint pre-flight. Leaving the key absent inherits that. `message.im` is currently
commented out in `slack-app-manifest.yml`'s phase-2 block, but `im:history` is already a live
scope, so it is three uncommented lines away. **Enabling `message.im` requires wiring the identity
pipeline into that handler first.**

## The `temper` connection (`agent/connections/temper.ts`)

Registered by **filesystem convention** — there is no manifest to edit. The filename gives the
connection its name and its tools become `temper__*`.

> **`agent/channels/` and `agent/connections/` are DISCOVERY directories, not folders.** Every
> module in them is loaded by `eve build` and required to export the matching shape. A helper
> module parked next to `channels/slack.ts` because that is where its callers live fails the build
> with *"Expected the channel export `default` from `channels/…` to match the public eve shape"*.
> Helpers go in `agent/lib/`. This cost a red CI run: `events.ts` (the ephemeral event overrides)
> was authored into `channels/` and had to move.
>
> **`npm test` and `npm run typecheck` are both blind to this** — discovery happens only at build.
> Run **`npm run build`** before pushing, and note it needs `TEMPER_MCP_URL` set to *something*
> (`connections/temper.ts` resolves it at module load, so a missing value fails the build rather
> than the first user turn — deliberate, and the same as the steward). CI supplies a reserved
> `.invalid` host for exactly this.

**`principalType: "user"`, and that is the whole point.** The steward is app-scoped and speaks as a
machine; this agent speaks as whoever mentioned it, under exactly their reach. eve keys the token
cache on `user:${issuer}:${id}` so concurrent users never share tokens — which is why
`getTemperToken` memoizes **nothing**. Do not copy the steward's `mintM2mToken`: it memoizes a
process-wide singleton, which under per-user tokens is a cross-user credential leak.

A user-scoped connection also **fails fast** (`reason: "principal_required"`, non-retryable) when a
session has no authenticated user principal. So the connection and the dispatch are one change, not
two: a connection with nothing dispatching to it is broken, not dormant.

**The tool allow-list is READ-ONLY and it is the enforcement point.** Nine names, the read half of
the steward's twenty-four, in `TEMPER_READ_TOOLS` (`agent/lib/mcp-auth.ts`). Writes are not merely
unimplemented — a read-only context member can currently create a resource in that context, so a
write tool here would exercise that bug under a real human's whole reach. Tools left out for
*uncertainty* rather than for being known writes are named in that constant's doc comment; the rule
is that "probably a read" is not a reason to grant. Two tests guard it: an exact-list assertion, and
a mutating-name-family scan that keeps biting even if someone "fixes" the first by pasting the new
value.

**`getToken` fails closed.** Every `refused` arm throws `ConnectionAuthorizationFailedError`
with `retryable: false` — **not** `ConnectionAuthorizationRequiredError`, which would tell eve to
run an authorization flow and emit `authorization.required`, whose default handler posts a
framework-owned **public** status line an override cannot reach (known constraint 1 below). There is
no interactive flow to run: re-linking and admin approval both happen out of band. In practice the
pre-flight means this path only fires for a grant that stopped being mintable *between* the
pre-flight and the tool call — it must still fail closed rather than call the MCP server with no
credential. The thrown `reason` carries the refusal (`standing:denied`, not a flat `revoked`), so
the distinction survives into the log even though the user-facing remedy is grouped.

## eve inbound identity contract (verified against eve@0.18.1)

Verified by reading the installed package, not the docs:
`node_modules/eve/dist/src/public/channels/slack/auth.js` (`buildSlackAuthContext`),
`slackChannel.d.ts`, and `defaults.d.ts`. Re-verify against the package on any eve upgrade.

`defaultSlackAuth(message, ctx)` returns `SessionAuthContext | null` — **null when the message has
no author**. The context's real shape:

```ts
{
  attributes: Readonly<Record<string, string | readonly string[]>>;
  authenticator: string;   // "slack-webhook" for this channel
  issuer?: string;
  principalId: string;
  principalType: string;
  subject?: string;
}
```

### principalId has FOUR shapes — treat it as OPAQUE

`teamId` is nullable and bots carry an extra segment, so the segment count varies from **2 to 4**:

| teamId | author | `principalId`             | `principalType` |
| ------ | ------ | ------------------------- | --------------- |
| yes    | human  | `slack:<team>:<user>`     | `user`          |
| yes    | bot    | `slack:<team>:bot:<user>` | `service`       |
| no     | human  | `slack:<user>`            | `user`          |
| no     | bot    | `slack:bot:<user>`        | `service`       |

**Never parse it into segments.** A `split(":")` + index parse is wrong for at least one shape: it
throws on the short one, or silently mis-keys a user by reading `<user>` out of the `<team>` slot.
Store it whole, compare it whole, log it whole. `agent/lib/identity.ts` is the only module that
touches identity, and it is pure so the forks are testable without Slack (`tests/identity.test.ts`).
The decomposed parts are already on `attributes` (`user_id`, `team_id`) for anyone who needs them —
which is why parsing is never necessary.

### Other verified facts

- **`issuer`** = `slack:<team>` when teamId is present, else the bare string `slack`.
- **`subject` is NEVER set** by this channel. Do not read it.
- **THERE IS NO EMAIL.** Attributes the Slack channel sets are exactly: `author_type`,
  `channel_id`, `thread_ts`, `user_id`, plus optional `user_name`, `full_name`, `team_id`.
  Any temper account link must therefore key off the opaque `principalId` — an email-based
  auto-link is not expressible with what arrives on the wire.
- **Reject non-humans.** `principalType !== "user"` must not dispatch; bots surface as `"service"`.
  The gate is written `=== "user"` (not `!== "service"`) so a principalType eve adds later is
  refused by default rather than admitted by accident.
- **Route:** `POST /eve/v1/slack` (override via `slackChannel({ route })`). **HTTP only — Socket
  Mode is NOT supported.** The same route serves interactivity callbacks.
- **`onAppMention`** returns `{ auth }` to dispatch or `null` to drop. Thrown errors are caught and
  logged by eve and the mention is dropped — wrap best-effort side effects in try/catch.
- **`ctx`** is pre-dispatch: `{ thread, slack }`, and **`state` is ABSENT** (it only exists on the
  hydrated `SlackEventContext` handed to `events[...]` handlers). `ctx.thread` owns `post`,
  `postEphemeral`, `startTyping`, `refresh`, `recentMessages`, `mentionUser`; `ctx.slack` owns
  `channelId`, `threadTs`, `teamId`, `request`, `uploadFiles`.
- Supplying `onAppMention` **replaces** eve's default mention pipeline — both the auth derivation
  *and* the default `"Thinking..."` typing indicator.

## Known constraints for later tasks

1. **The `authorization.required` link challenge must be private.** An override of that event gets
   `SlackAuthorizationEventContext` — only `postEphemeral`, `postDirectMessage`, and `state`. There
   is deliberately **no public `post`** and no raw `slack.request` escape hatch, because the
   sign-in challenge is a credential: anyone who completes it binds their identity to the session's
   connection. An override can change the words, not the audience. So T2's account-link challenge
   has to be ephemeral or DM. (`postDirectMessage` needs the `im:write` scope — already in the
   manifest.) eve's *default* handler additionally posts a public link-free status and edits it on
   `authorization.completed`; that public post is framework-owned and not reachable from an override.
2. **ONE DEPLOYMENT SERVES ONE SLACK WORKSPACE.** This is a hard ceiling, not a convention, and it
   is the single most important fact on this page. Verified end-to-end in eve@0.18.1 +
   `@vercel/connect@0.2.2`:

   - `SlackBotToken = string | (() => Promise<string> | string)` — the function form takes **no
     arguments**. Terminal call is `await e()` (`eve/dist/src/compiled/@chat-adapter/slack/api.js`).
     There is no seam through which a team id could arrive.
   - Slack bot tokens are **app-scoped**: one token per workspace install, shared by every user in
     it. `connectSlackCredentials` pins `subject: { type: "app" }` and says so.
   - `connectSlackCredentials(connector, params)` closes over `params` at **module load**:
     `botToken: () => getToken(connector, { ...params, subject: { type: "app" } }, options)`.
     `getToken` POSTs `https://api.vercel.com/v1/connect/token/<connector>` with exactly those
     construction-time params; the ambient auth is the deployment's Vercel OIDC token, which
     identifies the **deployment**, never the Slack workspace.
   - Its cache key is `JSON.stringify({connector, ...params})` — **request-invariant**. Every request
     in a deployment shares one entry. That alone proves single-install intent.
   - `@vercel/connect` contains **zero** `AsyncLocalStorage` / ambient request context.
   - eve **does** parse `team_id` off the inbound webhook and hands it to `buildSlackBinding`, but it
     is used only for `slack.teamId` and session `state` — **never routed toward the token**.

   **So `installationId` is not a tenancy knob.** It means *"if your connector holds several
   installs, name which one THIS DEPLOYMENT uses."* The docstring's "invoked once per inbound
   webhook … multi-workspace tenancy handled server-side" conflates **rotation** (a fresh token for a
   *fixed* install — which is real) with **tenancy** (which is not reachable this way).

   **The capability exists one layer down and is unwired.** eve vendors a `SlackAdapter`
   (`dist/src/compiled/@chat-adapter/slack/index.js`) with genuine multi-workspace machinery —
   `installationProvider` (docstring: *"for multi-workspace apps using external token management
   (e.g. Vercel Connect)"*), `withBotToken`, `setInstallation`, `tokenClientCache`, and an
   `AsyncLocalStorage`. **`slackChannel` never imports it** — it takes only the stateless primitives
   from `.../slack/api.js`, whose options are `{ apiUrl?, fetch?, token }` with no context. Different
   module; no shared state.

   Consequence for the two deployment stories:

   | | Slack app | Credentials | Works today? |
   | --- | --- | --- | --- |
   | **Self-hosted** | one per customer workspace, request_url = their host | their own signing secret + bot token | **Yes.** One app : one workspace : one temper. Raw env is the *correct* choice here, not a compromise. |
   | **temperkb.io community** (one public @temper any workspace installs) | one app, request_url = temperkb.io | per-workspace token minted at OAuth install | **No.** Structurally impossible via `SLACK_BOT_TOKEN` *or* `connectSlackCredentials`. |

   Known options for the community case, none free: (a) one deployment per workspace — works
   unchanged, does not scale to a public app; (b) hand-roll the thunk — supply the ambient context
   eve doesn't, parsing `team_id` off the inbound body into your own ALS and calling `getToken` with
   a per-request `installationId`. `getToken` is exported and per-install cache keys fall out for
   free, **but ALS does not survive an await boundary crossed by the workflow runtime**, and
   `slackChannel.js` dispatches under `waitUntil(...)` — unverified and worth its own spike before
   committing; (c) upstream: have `slackChannel` route through `SlackAdapter`'s
   `installationProvider`, which is already built for exactly this.

3. **Credentials: Connect is the eventual path.** T1 uses the raw env fallback
   (`SLACK_BOT_TOKEN` + `SLACK_SIGNING_SECRET`) so the app is reproducible from the committed
   `slack-app-manifest.yml` and needs no feature-flagged Vercel CLI. eve's documented default is
   Vercel Connect — `connectSlackCredentials("slack/<uid>")` from `@vercel/connect/eve`, returning
   `{ botToken, webhookVerifier }`, which moves token rotation, multi-workspace tenancy, and
   webhook verification out of our code. Switching means adding the `@vercel/connect` dep, passing
   `credentials:`, and provisioning the trigger destination at `/eve/v1/slack`
   (`vercel connect attach <uid> --triggers --trigger-path /eve/v1/slack`, behind
   `FF_CONNECT_ENABLED=1`). See `node_modules/eve/docs/channels/slack.mdx`.

   > **This path is now BLOCKED, and the blocker is identity, not effort.** **Do not add
   > `@vercel/connect` as a dependency of this agent.** The `temper` connection passes
   > `principal.id` straight to the mint route with no translation, and that is only correct on one
   > of two branches in `resolveConnectionPrincipal`
   > (`eve/dist/src/runtime/connections/principal.js`):
   >
   > ```js
   > return i.vercelConnect!==void 0 && isVercelDevelopmentUser(o)
   >   ? {attributes:o.attributes, id:o.subject??o.principalId, type:`user`}
   >   : {attributes:o.attributes, id:o.principalId, issuer:o.issuer??o.authenticator, type:`user`}
   > ```
   >
   > On the **non-Connect** branch `principal.id` IS the `SessionAuthContext.principalId` — the
   > same `slack:<team>:<user>` string that link-state and the grant vault are keyed on. The
   > `vercelConnect` marker, which `connect()` stamps onto the auth definition, activates the
   > **first** branch and its `o.subject ?? o.principalId`. Adding the dep to get Connect-managed
   > *Slack* credentials would therefore change which string authenticates as the human — silently,
   > and in the direction of minting under an identity the vault does not hold. If Connect becomes
   > necessary, the mint seam has to be re-grounded against that branch **first**. The full
   > argument lives at the top of `agent/lib/mcp-auth.ts`.

## Environment

| Var                    | Purpose                                                     |
| ---------------------- | ----------------------------------------------------------- |
| `SLACK_BOT_TOKEN`      | Outbound Slack Web API calls. eve's fallback when `credentials.botToken` is omitted. |
| `SLACK_SIGNING_SECRET` | HMAC-verifies inbound webhooks. eve's fallback when neither `signingSecret` nor `webhookVerifier` is supplied. |
| `TEMPER_API_URL`       | Base URL of the temper API this agent asks for each mentioning user's link state and access token, e.g. `https://temperkb.io`. |
| `SLACK_LINK_SECRET`    | Shared HMAC secret gating `POST /internal/slack/link-state`. **Must equal temper-api's `SLACK_LINK_SECRET`** — a mismatch is a 401 on every mention, not a warning. |
| `SLACK_MINT_SECRET`    | Shared HMAC secret gating `POST /internal/slack/mint`. **Must equal temper-api's**, and **must DIFFER from `SLACK_LINK_SECRET`** — link-state answers a question, mint hands back a human's entire reach, so sharing one value makes the cheap capability yield the expensive one. `tests/mint.test.ts` asserts the agent never signs a mint with the link key. |
| `TEMPER_MCP_URL`       | temper-mcp endpoint the connection's tools call, e.g. `https://temperkb.io/api/mcp`. |

The first five are read at request time (`agent/lib/link.ts`'s `requireEnv`), so an unset one
throws on the first mention rather than at deploy. `TEMPER_MCP_URL` is the exception: it is read at
**module load** by `agent/connections/temper.ts` (matching the steward's `url: requireEnv(...)`),
so an unset value fails the whole function at import, not per-mention. That is also why
`getTemperToken` and the allow-list live in `agent/lib/mcp-auth.ts` — a plain `import` of the
connection file in a test process would otherwise throw.

**Model:** a plain string on `defineAgent` (`agent/agent.ts`). eve resolves the model at **build**
time, so a change takes a **redeploy**, not a restart. (The steward's env-driven `model-config.ts`
is deliberately not copied — this agent has no need for a fallback policy yet.)

**Tests:** `npm test` (vitest, `tests/`). They run in CI via `.github/workflows/test-agents-ts.yml`,
which has a hardcoded per-agent job — a new agent gets no CI until a job is added.

**Setup:** see [README.md](./README.md).

@AGENTS.md
