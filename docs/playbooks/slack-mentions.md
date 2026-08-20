# Slack Mentions

**For operators** — someone wiring `@temper` mentions in Slack against a Temper
deployment.

## Outcome

By the end of this playbook you will have a Slack bot that responds to `@temper`
mentions. A user who mentions the bot for the first time receives a private
connect link; completing it links their Slack identity to their Temper profile.
A linked user who mentions the bot gets a confirmation that the link resolved.
Standing up the link now provisions the durable identity binding so that future
answer-delivery lands cleanly when it ships.

## Prerequisites

- **A running Temper deployment** — API, MCP server, and CLI. See
  [self-hosting Temper](./self-host-temper.md) if you have not stood one up.
- **The trust boundary Temper enforces** — read
  [the trust boundary](../concepts/trust-boundary.md) to understand the
  two-gate boundary the Slack path crosses.
- **The auth-identity contract** — read
  [auth identity](../concepts/auth-identity.md) to understand whose tokens your
  instance trusts. The Slack link flow uses the same issuer.
- **An identity provider client** (Auth0 or the Temper AS) configured for
  Authorization Code + PKCE with no client secret — a public client, not a
  confidential one.
- **A Vercel account** for the mention agent (it is its own Vercel project,
  separate from the Temper deployment).

> **One deployment serves exactly one Slack workspace.** This is a hard ceiling
> in the eve runtime, not a setup choice. Self-hosted (one app : one workspace :
> one Temper) is the natural shape.

## Topology

Three pieces are tied together by one shared secret:

| Piece | What it is | Where it lives |
|---|---|---|
| **Temper API** | Serves the OAuth callback, resolves the user, holds the encrypted grant vault. | Your Temper deployment. |
| **The mention agent** | An eve app that watches Slack, asks Temper "what do I say to this user?", and posts the connect link. | Its own Vercel project. |
| **The Slack app** | The bot users mention. Created from the manifest below. | api.slack.com, one app per workspace. |

The shared secret is **`SLACK_LINK_SECRET`**, which must be byte-identical on
the Temper API and the agent. A mismatch produces a `401` on every mention, not
a warning.

## Configure the IdP client

The link flow is an OAuth **client** of whatever issuer fronts your instance,
using **Authorization Code + PKCE with no client secret** (it mirrors the Temper
CLI).

**Auth0 mode:**

- **Application type: a public client** — Token Endpoint Auth Method = None. A
  confidential client rejects the secret-less exchange.
- **Allowed Callback URLs** must include `<PUBLIC_BASE_URL>/api/auth/slack/callback`
  exactly — not a wildcard.
- **Refresh tokens enabled, rotation ON** — the flow requests `offline_access`.
- **Enable rotation leeway (grace window).** A real requirement: Temper refreshes
  a grant while holding a short database lock, but the IdP's rotation is an
  external step that cannot be made atomic with the local write. A leeway window
  (a few seconds to a minute) tolerates a brief reuse if the process is killed
  at the wrong instant. Recovery if it happens anyway: the user re-links.

**Temper AS mode (self-hosted):** register the same `redirect_uri` in the
client's configuration; endpoints are derived on the instance itself.

The requested scopes are fixed: `openid profile email offline_access`.
`email`/`profile` let the link resolve the user by the IdP's email;
`offline_access` returns the refresh token the vault stores.

## Set Temper API environment variables

Set these on your Temper deployment. All **four** are required as a unit — any
missing, or a malformed vault key, **disables the whole link flow** (fail-closed).
There is no half-on state.

| Variable | What it is |
|---|---|
| `SLACK_LINK_CLIENT_ID` | The public client from the previous step. |
| `SLACK_LINK_SECRET` | Shared HMAC secret gating the link-state endpoint. Generate with `openssl rand -hex 32`. **Must match the agent's copy.** |
| `PUBLIC_BASE_URL` | This instance's public origin, e.g. `https://temperkb.io`. The callback is `<PUBLIC_BASE_URL>/api/auth/slack/callback`. |
| `SLACK_VAULT_ENC_KEY` | AEAD key encrypting each stored refresh token. **32 bytes, base64** — `openssl rand -base64 32`. |

A fifth secret gates the **mint** (vending an access token acting as the
mentioning human) and is deliberately *not* part of the all-or-nothing four, so
an instance that has not set it keeps a fully working link flow and loses only
minting:

| Variable | What it is |
|---|---|
| `SLACK_MINT_SECRET` | Shared HMAC secret gating the mint endpoint. `openssl rand -hex 32`. **Must match the agent's copy.** Unset disables minting only. |

> **These secrets must all hold different values, and Temper refuses to boot if
> any two match.** The split is the security property: `SLACK_LINK_SECRET` gates
> an endpoint that answers "is this principal linked?", `SLACK_MINT_SECRET`
> gates one that hands back a token carrying that human's entire Temper reach,
> and `SLACK_VAULT_ENC_KEY` decrypts every stored refresh token. One value across
> two of them means whoever holds the cheap capability already holds the
> expensive one. Generate each one separately.

> **Set `SLACK_VAULT_ENC_KEY` as part of the deploy that ships the vault, not
> after.** Because the four link vars are all-or-nothing, deploying vault code
> to an instance already running the link flow turns the link flow off until the
> key is present.

## Create the mention agent's Vercel project

The agent is its own git-connected Vercel project:

- **Root Directory**: `packages/agent-workflows/mention`
- **Git-connected** to the Temper repository, production branch `main` — merging
  `main` deploys it; any branch push gives a preview. Deploy by git push, never
  `eve deploy` or `vercel deploy` from the agent directory.

Note the **deployment host** — you need it later. Every request will `401` until
the signing secret is set; that is expected.

## Create the Slack app (phase 1)

> **This is a two-phase manifest, and the order is not the obvious one.** The eve
> runtime verifies the request signature first and fails closed: with no
> `SLACK_SIGNING_SECRET` the route returns `401` and never reaches Slack's
> `url_verification` handshake. But the signing secret only exists once the app
> exists. Declaring a request URL up front is a deadlock — Slack will not save a
> URL it cannot verify, and the URL cannot verify without a secret that does not
> exist yet. The way out: create the app with no event subscriptions, collect the
> secret, deploy, then declare the URL.

Go to <https://api.slack.com/apps> → **Create New App** → **From a manifest**,
choose the workspace, and paste the following **unmodified**. The
event/interactivity block is omitted on purpose — leave it out.

```yaml
display_information:
  name: temper
  description: Ask temper about your team's knowledge base, from Slack.
  background_color: "#2c2d30"

features:
  bot_user:
    display_name: temper
    always_online: false

oauth_config:
  scopes:
    bot:
      - app_mentions:read
      - chat:write
      - im:history
      - im:write
      - channels:history

settings:
  socket_mode_enabled: false
  org_deploy_enabled: false
  token_rotation_enabled: false
```

With no event subscriptions declared, Slack has nothing to verify and the app is
created cleanly.

## Install to the workspace and copy credentials

1. **Install App** → *Install to Workspace* → authorize. Copy the **Bot User
   OAuth Token** (`xoxb-…`).
2. **Basic Information** → *App Credentials* → copy the **Signing Secret**.

## Set the agent environment and redeploy

On the mention agent's Vercel project, set:

| Variable | What it is |
|---|---|
| `SLACK_BOT_TOKEN` | The `xoxb-…` token from the previous step. |
| `SLACK_SIGNING_SECRET` | The signing secret from the previous step (HMAC-verifies inbound webhooks). |
| `TEMPER_API_URL` | The Temper **API origin**, e.g. `https://temper-cloud.vercel.app`. |
| `SLACK_LINK_SECRET` | **The same value** as the Temper API's `SLACK_LINK_SECRET`. |
| `SLACK_MINT_SECRET` | **The same value** as the Temper API's `SLACK_MINT_SECRET` — and a **different** value from `SLACK_LINK_SECRET`. |

```sh
vercel env add SLACK_BOT_TOKEN production
vercel env add SLACK_SIGNING_SECRET production
vercel env add TEMPER_API_URL production
vercel env add SLACK_LINK_SECRET production
vercel env add SLACK_MINT_SECRET production
```

Then redeploy (push to `main`, or redeploy from the dashboard) so the functions
pick them up.

> **`TEMPER_API_URL` must point at the API origin, not the UI origin.** The
> agent calls `/internal/slack/link-state`, and the UI proxy does not forward
> `/internal`. Point it at the UI origin and the internal call hits the UI shell
> and fails. (The Temper API's `PUBLIC_BASE_URL` does stay the public UI origin,
> because the callback lives under `/api`, which the UI proxy forwards. The two
> are genuinely different origins.)

## Declare the request URL (phase 2)

Now the signing secret is in place, so the handshake will succeed. In the Slack
app's **App Manifest** editor, add the event/interactivity block below, replacing
`<YOUR-DEPLOYMENT-HOST>` with the agent host from the Vercel project step, and
save:

```yaml
event_subscriptions:
  request_url: https://<YOUR-DEPLOYMENT-HOST>/eve/v1/slack
  bot_events:
    - app_mention
    - message.im
interactivity:
  is_enabled: true
  request_url: https://<YOUR-DEPLOYMENT-HOST>/eve/v1/slack
```

Slack POSTs a `url_verification` challenge; eve verifies the signature and
answers with the raw challenge on the first try. Adding `message.im` may prompt
a **reinstall** to grant `im:history` — if Slack asks, reinstall and re-copy the
bot token if it changed.

## Verify

1. Invite the bot to a channel: `/invite @temper`.
2. **Unlinked mention.** Mention `@temper` from an account that has not linked.
   You should get an **ephemeral** message (visible only to you) with a one-time
   connect link. Nothing at all → check the agent's `TEMPER_API_URL` and
   `SLACK_LINK_SECRET`.
3. **Complete the link.** Click the link, sign in at your IdP. You should land
   on a Temper-branded "Account connected — Linked as @your-handle" page.
4. **Linked mention.** Mention `@temper` again. Today you get "You're connected
   as @your-handle. I can't answer questions yet" — that reply is proof the link
   resolved from the database. A second connect link here means the link-state
   lookup is not finding your row.

## Troubleshooting

| Symptom | Cause |
|---|---|
| Slack says the request URL didn't verify | `SLACK_SIGNING_SECRET` unset or wrong, or you declared the URL before the signing secret was deployed. The eve runtime returns `401`; Slack reports it as a failed handshake. |
| Bot never responds, no error | The mention was dropped (bot-authored or authorless events are silent by design), or a `SLACK_LINK_SECRET` mismatch is `401`ing every mention. Check the agent function logs. |
| "Sign-in could not be completed" on the callback | The IdP client is confidential; make it a public/PKCE client. |
| "No temper account is linked to this login" | The user has no Temper account yet — the link is lookup-only; sign in at the Temper UI first. |
| "Account linking is not configured" | One of the four Temper API vars is missing or malformed. |
| Everything green but no logs on failure | The eve runtime swallows handler errors, and serverless logs surface HTTP events, not app stdout — "no error in the logs" proves nothing. Diagnose from observable Slack behavior. |

## Further reading

- **The trust boundary the Slack path crosses:**
  [The Trust Boundary](../concepts/trust-boundary.md).
- **The auth-identity contract the link flow uses:**
  [Auth Identity](../concepts/auth-identity.md).
- **Self-hosting Temper:** [Self-Host Temper](./self-host-temper.md).
- **Governance and administration:**
  [temperkb.io/operating/governance-and-administration](https://temperkb.io/operating/governance-and-administration).
