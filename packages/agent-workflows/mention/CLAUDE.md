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
2. **Credentials: Connect is the eventual path.** T1 uses the raw env fallback
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
