# Operator guide: `@temper` on Slack — end-to-end setup

This is the complete setup flow for **`@temper` mentions in Slack** against a temper deployment:
the Slack app, the mention agent, the temper-api environment, the IdP client, and how to verify it.
Follow the sections in order.

> **Scope.** This wires the *account link* (a Slack user connects their temper account, once, in a
> browser) and the *grant vault* (temper stores an encrypted per-user grant so a future mention can
> act as that human). **Answering mentions with real work — search, writes — is not wired yet.**
> Today a linked mention gets a "you're connected, but I can't answer questions yet" reply. Standing
> this up now provisions the durable link so those answers land cleanly when they ship.

---

## The three pieces

| Piece | What it is | Where it lives |
|---|---|---|
| **temper-api** | Serves the OAuth callback, resolves the user, holds the encrypted vault. | Your temper deployment (e.g. `temper-cloud.vercel.app` behind `temperkb.io`). |
| **the mention agent** | An [eve](https://eve.dev) app that watches Slack, asks temper "what do I say to this user?", and posts the connect link. | **Its own Vercel project**, root `packages/agent-workflows/mention` — *not* temperkb.io, *not* temper-cloud. |
| **the Slack app** | The bot users mention. Created from the committed manifest. | api.slack.com, one app per workspace. |

They are tied together by one shared secret, **`SLACK_LINK_SECRET`**, which must be byte-identical on
temper-api and the agent. A mismatch is a `401` on every mention, not a warning.

> **One deployment serves exactly one Slack workspace.** This is a hard ceiling in eve +
> `@vercel/connect`, not a setup choice — see the mention agent's
> [`CLAUDE.md`](../../packages/agent-workflows/mention/CLAUDE.md) before planning a multi-workspace or
> community rollout. Self-hosted (one app : one workspace : one temper) is the natural shape.

---

## Step 1 — the IdP client (Auth0, or the temper AS)

The link flow is an OAuth **client** of whatever issuer fronts your instance, using **Authorization
Code + PKCE with no client secret** (it mirrors the temper CLI). Configure the client
`SLACK_LINK_CLIENT_ID` will point at:

**Auth0 mode (e.g. temperkb.io):**

1. **Application type: a _public_ client** — Token Endpoint Auth Method = **None**. A confidential
   "Regular Web App" **rejects** the secret-less exchange with "Sign-in could not be completed." Use
   a Native/SPA-style public app. (The temper CLI's public native client is a proven working shape.)
2. **Allowed Callback URLs** must include `<PUBLIC_BASE_URL>/api/auth/slack/callback` exactly — not a
   wildcard. An unregistered `redirect_uri` is the classic authorization-code interception vector.
3. **Refresh tokens enabled, rotation ON** — the flow requests `offline_access`.
4. **Enable rotation _leeway_ (grace window).** A real requirement, not a nicety — see
   [§ Security properties](#security-properties).

**AS mode (self-hosted temper AS):** register the same `redirect_uri` in the client's `AS_CLIENTS`
entry; endpoints are derived on the instance itself (`<issuer>/oauth/authorize`, `/oauth/token`).

The requested scopes are fixed: `openid profile email offline_access`. `email`/`profile` let the link
resolve the user by the IdP's email; `offline_access` is what returns the refresh token the vault
stores.

---

## Step 2 — temper-api environment

Set these on your temper deployment (they sit alongside the auth vars in
[enterprise-install.md](enterprise-install.md)). All **four** are required as a unit — any missing, or
a malformed vault key, **disables the whole link flow** (fail-closed; the callback answers "Account
linking is not configured"). There is no half-on state.

| Variable | What it is |
|---|---|
| `SLACK_LINK_CLIENT_ID` | The **public** client from Step 1. |
| `SLACK_LINK_SECRET` | Shared HMAC secret gating `POST /internal/slack/link-state`. `openssl rand -hex 32`. **Must match the agent's copy** (Step 6). |
| `PUBLIC_BASE_URL` | This instance's public origin, e.g. `https://temperkb.io`. The callback is `<PUBLIC_BASE_URL>/api/auth/slack/callback`. |
| `SLACK_VAULT_ENC_KEY` | AEAD key encrypting each stored refresh token. **32 bytes, base64** — `openssl rand -base64 32`. See [§ The vault key](#the-vault-key). |

> **Set `SLACK_VAULT_ENC_KEY` as part of the deploy that ships the vault, not after.** Because the
> four are all-or-nothing, deploying vault code to an instance already running the link flow **turns
> the link flow off** until the key is present.

---

## Step 3 — create the mention agent's Vercel project

The agent is its own git-connected Vercel project (the `steward-agent` project is the precedent):

- **Root Directory**: `packages/agent-workflows/mention`
- **Git-connected** to `tasker-systems/temper`, production branch `main` — merging `main` deploys it;
  any branch push gives a preview. Deploy by **git push**, never `eve deploy`/`vercel deploy` from the
  agent dir.
- `sourceFilesOutsideRootDirectory` is **not** needed (this agent has no sibling `file:` dep).

Note the **deployment host** — Step 7 needs it. Every request will `401` until Step 6; that's expected
(no signing secret yet).

---

## Step 4 — create the Slack app from the manifest (phase 1, as-is)

> **This is a two-phase manifest, and the order is not the obvious one.** eve verifies the request
> signature **first** and fails **closed**: with no `SLACK_SIGNING_SECRET` the `/eve/v1/slack` route
> returns `401` and never reaches Slack's `url_verification` handshake. But the signing secret only
> exists once the app exists. Declaring a request URL up front is therefore a deadlock — Slack won't
> save a URL it can't verify, and the URL can't verify without a secret that doesn't exist yet. The
> way out: **create the app with no event subscriptions, collect the secret, deploy, then declare the
> URL.**

Go to <https://api.slack.com/apps> → **Create New App** → **From a manifest**, choose the workspace,
and paste [`packages/agent-workflows/mention/slack-app-manifest.yml`](../../packages/agent-workflows/mention/slack-app-manifest.yml)
**unmodified**. The event/interactivity block at the bottom is commented out on purpose — leave it.
With no event subscriptions declared, Slack has nothing to verify and the app is created cleanly.

The manifest already declares the bot scopes the flow needs (`app_mentions:read`, `chat:write`,
`im:history`, `im:write`, `channels:history`) and disables Socket Mode (eve is HTTP-only).

---

## Step 5 — install to the workspace and copy credentials

1. **Install App** → *Install to Workspace* → authorize. Copy the **Bot User OAuth Token** (`xoxb-…`).
2. **Basic Information** → *App Credentials* → copy the **Signing Secret**.

---

## Step 6 — set the agent environment and redeploy

On the mention agent's Vercel project:

| Variable | What it is |
|---|---|
| `SLACK_BOT_TOKEN` | The `xoxb-…` token from Step 5. |
| `SLACK_SIGNING_SECRET` | The signing secret from Step 5 (HMAC-verifies inbound webhooks). |
| `TEMPER_API_URL` | The temper **API origin**, e.g. `https://temper-cloud.vercel.app`. |
| `SLACK_LINK_SECRET` | **The same value** as temper-api's `SLACK_LINK_SECRET` (Step 2). |

```sh
vercel env add SLACK_BOT_TOKEN production
vercel env add SLACK_SIGNING_SECRET production
vercel env add TEMPER_API_URL production
vercel env add SLACK_LINK_SECRET production
```

Then redeploy (push to `main`, or redeploy from the dashboard) so the functions pick them up.

> **`TEMPER_API_URL` must point at the API origin, NOT the UI origin.** The agent calls
> `/internal/slack/link-state`, and the temper-UI (SvelteKit) proxy only forwards `/api`, `/mcp`,
> `/oauth`, `/.well-known` — **not `/internal`**. Point it at `temperkb.io` and the internal call hits
> the UI shell and fails. (temper-api's `PUBLIC_BASE_URL` *does* stay the public UI origin, because
> the callback lives under `/api`, which the UI proxy forwards. The two are genuinely different
> origins.)

---

## Step 7 — declare the request URL (phase 2)

Now the secret is in place, so the handshake will succeed. In the Slack app's **App Manifest** editor,
uncomment the phase-2 block, replace `<YOUR-DEPLOYMENT-HOST>` with the agent host from Step 3, and
save (equivalently: enable **Event Subscriptions** and **Interactivity & Shortcuts** and paste the
same `https://<host>/eve/v1/slack` URL into both):

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

Slack POSTs a `url_verification` challenge; eve verifies the signature and answers with the raw
challenge on the first try. Adding `message.im` may prompt a **reinstall** to grant `im:history` — if
Slack asks, reinstall and re-copy the bot token if it changed.

---

## The vault key

```sh
openssl rand -base64 32   # 44 base64 chars → 32 bytes. This is SLACK_VAULT_ENC_KEY.
```

- Seals every stored refresh/access token with XChaCha20-Poly1305; the database never sees the key or
  any plaintext token.
- Keep it in your platform's secret store, never in the repo.
- **Rotation is flag-day today.** There is one key. Changing `SLACK_VAULT_ENC_KEY` makes every stored
  grant unreadable — affected users simply mention `@temper` and re-link. (The schema reserves a
  `key_version` column for a future zero-downtime keyring; it is not implemented, so do not treat
  rotation as seamless.) If the key is compromised, rotating it *is* the mitigation — it renders every
  stored ciphertext useless — at the cost of a re-link per user.

---

## Security properties

- **What's stored:** each linked user's own refresh token (their independent grant from *their*
  consent — never a copy of anyone's CLI token), encrypted at rest, plus a short-lived cached access
  token. Which Slack principal maps to which profile lives in a separate table with no secret column.
  Each ciphertext is additionally bound to its principal, so a stolen DB row can't be replayed under
  another user.
- **Revocation is not an instant cutoff.** Stopping a grant prevents *future* token mints; an access
  token already issued stays valid until its own expiry (≤ the IdP's access-token TTL), because
  validation consults no revocation list. Deactivating a temper profile *does* immediately stop new
  mints for that user.
- **Enable Auth0 refresh-token rotation _leeway_.** temper refreshes a grant while holding a short DB
  lock, but the IdP's rotation is an external step that can't be made atomic with the local write: if
  the process is killed at exactly the wrong instant (after the IdP rotates the token but before
  temper records the new one), the stored token is stale, and the next refresh would otherwise trip
  Auth0's reuse-detection and kill the grant. A **leeway** window (a few seconds to a minute)
  tolerates that brief reuse and closes the gap. Recovery if it happens anyway: the user re-links.

---

## Verify it end to end

1. Invite the bot to a channel: `/invite @temper`.
2. **Unlinked mention.** Mention `@temper` from an account that has **not** linked. You should get an
   **ephemeral** message (visible only to you) with a one-time "connect your account" link. Nothing at
   all → check the agent's `TEMPER_API_URL` and `SLACK_LINK_SECRET`.
3. **Complete the link.** Click it, sign in at your IdP. You should land on a temper-branded **"Account
   connected — Linked as @your-handle"** page.
4. **Linked mention.** Mention `@temper` again. Today you get **"You're connected as @your-handle. I
   can't answer questions yet"** — that reply *is* proof the link resolved from the database. A second
   *connect link* here means the link-state lookup isn't finding your row.

Optional DB confirmation: a row in `kb_profile_auth_links` (`auth_provider = 'slack'`) and an
encrypted row in `kb_slack_grant_vault`.

---

## Disconnecting an account

A user unbinds their own link:

```bash
temper slack disconnect
```

An operator unbinds any principal — offboarding, or a user who linked the wrong profile:

```bash
temper admin slack disconnect 'slack:T0BHAHEN79C:U0BH6A3L6JF'
```

The principal is opaque and has two to four segments. Pass it whole, quoted — never split it.

Both are **idempotent**: disconnecting an already-disconnected principal succeeds quietly, and says
so. Until this existed there was no self-service recovery at all: a user who linked the wrong profile
needed an operator with direct SQL access. The "already connected to a different temper account"
refusal page has always told people to "disconnect it there first" — this is the affordance that
sentence was promising.

### What it does

- Deletes the identity row (`kb_profile_auth_links`).
- **Destroys** the encrypted grant (`kb_slack_grant_vault`) — the row is deleted, not flagged, so the
  sealed refresh token stops existing rather than merely being ignored.
- Sweeps that principal's pending link intents. This is a security step, not hygiene: the link flow
  is safe partly because an already-linked user is never issued an authorize URL and partly because
  rebinding is refused. A disconnect removes **both** guarantees at once, so an intent minted just
  before it would otherwise survive as a live first-link URL for a now-unlinked principal.
- Attempts to revoke the grant at the identity provider.

### What it does NOT do

- **It is not deactivation.** The profile, its team memberships, and its resources are untouched.
- **It is not an instant cutoff.** Revocation stops *future* mints; an access token already issued
  stays valid until its own expiry (up to an hour), because validation consults no revocation list.
  This is the same honesty the rest of this guide keeps about revocation — see *Security properties*.
- **It does not uninstall the Slack app.** That is workspace-level and admin-only, which is precisely
  why a per-user disconnect has to exist.

### If the IdP revocation fails

The response reports `idp_revoked: false` and the CLI warns on stderr. **The disconnect still
succeeded** — the local grant is destroyed either way, so temper can no longer use it. The grant may
remain live at the IdP until it expires; revoke it from the Auth0 dashboard if that matters to you.

temper deliberately does not keep the token around to retry the revocation later: doing so would
preserve the exact secret the user just asked it to destroy. The failure is logged with the principal
and the status, never the token.

On self-hosted installs (temper-AS mode) revocation is local and happens in the same transaction as
the deletes, so it cannot fail this way at all.

### Reconnecting

Just mention `@temper` again. The principal is unlinked, so the normal flow offers a fresh authorize
URL — there is no special reconnect path.

### Intent retention

Expired and consumed link intents are swept hourly by the `/api/slack/intents/reap` cron, gated on
the same bearer secret as the embed crons (`EMBED_DISPATCH_SECRET`). Consumed rows are removed
because their nonce is single-use and already spent; live unconsumed ones are spared.

---

## Troubleshooting

| Symptom | Cause |
|---|---|
| Slack says the request URL didn't verify | `SLACK_SIGNING_SECRET` unset/wrong, or you declared the URL before Step 6. eve returns `401`; Slack reports it as a failed handshake and names the URL, not the missing secret. |
| Bot never responds, no error | The mention was dropped (bot-authored or authorless events are silent by design), or a `SLACK_LINK_SECRET` mismatch is `401`ing every mention. Check the agent function logs. |
| "Sign-in could not be completed" on the callback | The IdP client is **confidential**; make it a public/PKCE client (Step 1.1). |
| "No temper account is linked to this login" | The user has no temper account yet — the link is lookup-only; sign in at the temper UI first. |
| "Account linking is not configured" | One of the four temper-api vars is missing/malformed (Step 2). |
| Everything green but no logs on failure | The eve runtime swallows handler errors, and serverless runtime logs surface HTTP events, not app stdout — "no error in the logs" proves nothing. Diagnose from observable Slack behavior (ephemeral? thread badge?) and the DB rows. |

For the agent internals (the eve inbound identity contract, why the connect message is a channel-root
ephemeral, the one-workspace ceiling), see
[`packages/agent-workflows/mention/CLAUDE.md`](../../packages/agent-workflows/mention/CLAUDE.md) and
[`README.md`](../../packages/agent-workflows/mention/README.md).
