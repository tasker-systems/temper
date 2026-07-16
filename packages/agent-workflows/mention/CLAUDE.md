> This is the **@temper mention agent** — an Eve agent that answers Slack app mentions.
> It is a workspace-isolated Eve project; run tooling from THIS directory, not the repo root
> (`cd packages/agent-workflows/mention && npm install`). A root `npm install` inherits the root's
> bun overrides and fails. It is deliberately NOT a member of the root `workspaces` array.

**Scope (T1):** this proves the inbound pipe, not the identity. There is **no temper reach** — no
`temper-ts` dependency, no machine token, no account lookup. Every human who mentions the bot gets
the "connect your temper account" prompt with their resolved principal echoed back. That echo is
T1's acceptance evidence.

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

## Environment

| Var                    | Purpose                                                     |
| ---------------------- | ----------------------------------------------------------- |
| `SLACK_BOT_TOKEN`      | Outbound Slack Web API calls. eve's fallback when `credentials.botToken` is omitted. |
| `SLACK_SIGNING_SECRET` | HMAC-verifies inbound webhooks. eve's fallback when neither `signingSecret` nor `webhookVerifier` is supplied. |

**Model:** a plain string on `defineAgent` (`agent/agent.ts`). eve resolves the model at **build**
time, so a change takes a **redeploy**, not a restart. (The steward's env-driven `model-config.ts`
is deliberately not copied — this agent has no need for a fallback policy yet.)

**Tests:** `npm test` (vitest, `tests/`). They run in CI via `.github/workflows/test-agents-ts.yml`,
which has a hardcoded per-agent job — a new agent gets no CI until a job is added.

**Setup:** see [README.md](./README.md).

@AGENTS.md
