# @temper mention agent

An Eve agent that answers `@temper` app mentions in Slack. See [CLAUDE.md](./CLAUDE.md) for the
eve inbound identity contract and the design constraints.

**T1 scope:** the pipe, not the identity. Mention the bot and it replies with a "connect your
temper account" prompt echoing your resolved Slack principal. There is no temper reach yet.

## Local development

Run everything from **inside this directory** — a root `npm install` inherits the repo root's bun
overrides and fails.

```bash
cd packages/agent-workflows/mention
npm install
npm run typecheck
npm test
```

## Where this deploys

**Its own Vercel project** — not temperkb.io (that is `temper-ui`) and not `temper-cloud` (that is
the API). This is an eve app with its own build, and it serves `POST /eve/v1/slack` itself. The
`steward-agent` project is the established precedent for the shape.

## Setup

**Read this order and follow it exactly — the obvious order does not work.**

Slack refuses to save a request URL it cannot verify. eve verifies the request signature **first**
and fails **closed**: with no `SLACK_SIGNING_SECRET` the `/eve/v1/slack` route returns `401`, and
the `url_verification` branch is never reached (`verifyInbound` is the route's first statement;
the handshake lives inside `handleEventPost`, past the gate). But the signing secret only exists
once the Slack app exists.

That is a deadlock if you declare the request URL up front. So: **create the app with no event
subscriptions, collect the secret, deploy with it, and declare the URL last.**

### 1. Create the Vercel project

Match `steward-agent`'s shape:

- **Root Directory**: `packages/agent-workflows/mention`
- **Git-connected** to `tasker-systems/temper`, production branch `main` — merging `main` deploys
  it; any branch push gives a preview.
- `sourceFilesOutsideRootDirectory` is **not** needed. (The steward requires it because it takes an
  npm `file:` dependency on `clients/temper-ts`; this agent deliberately has no temper reach and no
  such dep, so its root directory is self-contained.)

Note the deployment host — step 5 needs it. At this point every request 401s, which is expected and
correct: there is no signing secret yet.

> **Do not** run `eve deploy` / `vercel deploy` from inside this directory as a habit. It is not
> fatal here (no sibling `file:` dep to miss, unlike the steward), but this repo deploys agents by
> git push, and the CLI path is the one that silently breaks the moment a sibling dep is added.

### 2. Create the Slack app from the manifest — as-is

Go to <https://api.slack.com/apps> → **Create New App** → **From a manifest**, pick the workspace,
and paste [`slack-app-manifest.yml`](./slack-app-manifest.yml) **unmodified**.

The event/interactivity block at the bottom is commented out on purpose. Leave it. With no event
subscriptions declared, Slack has nothing to verify and the app is created cleanly.

### 3. Install to the workspace and copy the credentials

1. **Install App** → *Install to Workspace* → authorize. Copy the **Bot User OAuth Token**
   (`xoxb-…`).
2. **Basic Information** → *App Credentials* → copy the **Signing Secret**.

### 4. Set the environment variables and redeploy

```bash
vercel env add SLACK_BOT_TOKEN production
vercel env add SLACK_SIGNING_SECRET production
```

The account-link flow needs two more. `TEMPER_API_URL` is the temper API this agent asks for a
link URL (e.g. `https://temperkb.io`), and `SLACK_LINK_SECRET` is the shared HMAC secret gating
`POST /internal/slack/link-intents`:

```bash
vercel env add TEMPER_API_URL production
vercel env add SLACK_LINK_SECRET production
```

`SLACK_LINK_SECRET` must be **the same value** on this agent and on the temper-api deployment
(where it is set alongside `SLACK_LINK_CLIENT_ID` and `PUBLIC_BASE_URL` — see
[enterprise-install.md](../../../docs/guides/enterprise-install.md)). It is a secret with no
default and no discovery: if the two sides disagree, every mention gets a 401 and the user sees
the generic "couldn't start the account-connect flow" reply. Generate one with
`openssl rand -hex 32`.

Then redeploy so the functions pick them up (push to `main`, or redeploy from the dashboard).

eve reads the Slack pair from the environment because `agent/channels/slack.ts` omits
`credentials`. See
[CLAUDE.md](./CLAUDE.md) for why T1 uses the env fallback, why one deployment serves exactly one
workspace, and what moving to Vercel Connect would and would not buy.

### 5. Now declare the request URL

In the app's **App Manifest** editor, uncomment the phase-2 block, replace `<YOUR-DEPLOYMENT-HOST>`
with the host from step 1, and save. (Equivalently: enable **Event Subscriptions** and
**Interactivity & Shortcuts** in the UI and paste the same URL into both.)

Slack POSTs a `url_verification` challenge. The secret is in place now, so eve verifies the
signature, answers with the raw challenge string, and Slack accepts the URL on the first try.

Adding the `message.im` subscription may prompt a **reinstall** to grant `im:history` — if Slack
asks, reinstall and re-copy the bot token if it changed.

### 6. Verify

Invite the bot to a channel (`/invite @temper`) and mention it:

> **@temper** hello

It should reply in-thread with the unlinked prompt and your resolved principal
(`slack:<team>:<user>`). **That reply is T1's acceptance.**

## Troubleshooting

| Symptom | Cause |
| --- | --- |
| Slack says the request URL didn't verify | `SLACK_SIGNING_SECRET` is unset or wrong on the deployment, or you declared the URL before step 4. eve returns `401` and Slack reports it as a failed handshake — the error names the URL, not the missing secret. |
| The bot never responds, no error | The mention was dropped. `onAppMention` returns `null` for a bot-authored message (`principalType: "service"`) and for an authorless event — both silent by design. Check the function logs. |
| `SLACK_BOT_TOKEN is required.` | `credentials` is omitted (intended), so eve fell through to the env var and found nothing. |
