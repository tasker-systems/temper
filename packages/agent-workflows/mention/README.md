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

## Setup

The order matters: Slack validates the request URL when the app is created, so the deployment has
to exist first.

### 1. Deploy the agent

```bash
VERCEL_USE_EXPERIMENTAL_FRAMEWORKS=1 vercel deploy --prod
```

The flag lets the Vercel CLI recognize eve as a framework during the build. Note the deployment
host — the next step needs it.

### 2. Create the Slack app from the manifest

1. Edit [`slack-app-manifest.yml`](./slack-app-manifest.yml) and replace `<YOUR-DEPLOYMENT-HOST>`
   in **both** `request_url` fields with the host from step 1. Keep the `/eve/v1/slack` path — it
   is eve's default Slack route, and it serves both events and interactivity.
2. Go to <https://api.slack.com/apps> → **Create New App** → **From a manifest**, pick the
   workspace, and paste the edited YAML.

Slack verifies the request URL at creation time by POSTing a `url_verification` challenge. If it
fails, the deployment isn't reachable at that host — fix that before retrying.

### 3. Install to the workspace and copy the credentials

1. **Install App** → *Install to Workspace* → authorize. Copy the **Bot User OAuth Token**
   (`xoxb-…`).
2. **Basic Information** → *App Credentials* → copy the **Signing Secret**.

### 4. Set the environment variables

```bash
vercel env add SLACK_BOT_TOKEN production
vercel env add SLACK_SIGNING_SECRET production
```

eve reads both from the environment because `agent/channels/slack.ts` omits `credentials`. See
CLAUDE.md for why T1 uses the env fallback and how to move to Vercel Connect later.

### 5. Redeploy and point Slack at it

Redeploy so the functions pick up the new env vars:

```bash
VERCEL_USE_EXPERIMENTAL_FRAMEWORKS=1 vercel deploy --prod
```

If the deployment host changed, update the request URL under **Event Subscriptions** and
**Interactivity & Shortcuts** in the Slack app config to match.

### 6. Verify

Invite the bot to a channel (`/invite @temper`) and mention it:

> **@temper** hello

It should reply in-thread with the unlinked prompt and your resolved principal
(`slack:<team>:<user>`). That reply is T1's acceptance.
