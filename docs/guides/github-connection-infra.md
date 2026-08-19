# Operator guide: read-only GitHub credential via a BYO App + Vercel Connect

This is the complete setup flow for **a read-only GitHub connection** against a temper deployment: a
GitHub App with read-only installation permissions, a non-managed Vercel Connect connector, the
temper-api environment variables that let the broker mint, and how to verify the whole chain end-to-end.

Follow the sections in order. Every step has a verification — do not skip them.

> **Why this exists.** Vercel Connect's *managed* GitHub connector returns a maximal write token (11
> write scopes incl. `workflows:write`, all repositories) regardless of `scopes:['read']` — silently,
> with HTTP 200. The only enforcement point that exists is **the GitHub App itself**. A non-managed
> connector backed by your own read-only App makes read-only real *by construction*. See the B5 probe
> research for the live evidence.

---

## The four pieces

| Piece | What it is | Where it lives |
|---|---|---|
| **the GitHub App** | A GitHub App with read-only installation permissions. The enforcement point. | GitHub, under the org that owns the repos |
| **the Vercel Connect connector** | A non-managed (`creationMode: "manual"`) connector backed by the App's credentials. | Vercel Connect, in your team |
| **the broker env vars** | Four env vars on the temper-api Vercel project that let `temper-services` run the two-hop mint. | The temper-cloud (or self-hosted) Vercel project |
| **the temper connection** | The `kb_connections` row + credential + webhook events + tool manifest. | temper itself |

They are tied together by the **connector uid** (e.g. `github/<app-slug>`) and the **installation ID**
GitHub assigns when you install the App. A mismatch in either is a `401` at mint time, not a warning.

---

## Step 1 — create the GitHub App

The App's installation permissions are the **ceiling** — everything above depends on them being
read-only. This is the most important step; do not rush it.

**Location:** `https://github.com/organizations/<org>/settings/apps/new`

**Fields:**

| Field | Value |
|---|---|
| GitHub App name | `<org>-temper-readonly` (or similar — the slug is derived from this) |
| Homepage URL | `https://vercel.com` (placeholder) |
| Webhook | **Unchecked** — "I don't want to set up webhooks for now" (Connect handles trigger forwarding) |

**Repository permissions — the enforcement point:**

| Permission | Level | Why |
|---|---|---|
| Contents | **Read-only** | Read changed files + CODEOWNERS (the enrichment need) |
| Pull requests | **Read-only** | Read PR metadata |
| Metadata | **Read-only** | Auto-required by GitHub |

**All other permissions — No access (the default).** No write scopes. No `workflows` scope. If you
leave a write permission on, the App's tokens will have it, and the drift check at attach will surface
it — but the damage is done if you didn't notice. **Read the permissions page twice before clicking
Create.**

**After creating, collect from the App's "General" settings page:**

- **App ID** — integer (e.g. `4640564`)
- **App slug** — URL slug (e.g. `tasker-systems-temper-readonly`)
- **App name** — display name
- **Client ID** — starts with `Iv` (e.g. `Iv23abcdefghijklmnop`)
- **Client secret** — generate one if not already generated
- **Private key** — generate and download the `.pem` file

**Verify the App exists:**

```bash
curl -sI "https://github.com/apps/<app-slug>"
# Expected: HTTP/2 200
```

---

## Step 2 — create the non-managed Vercel Connect connector

The connector is the bridge between your GitHub App and Vercel Connect's mint endpoint. It carries
the App's credentials (encrypted at rest by Vercel).

**Build the `--data` JSON file** (secrets stay out of shell history via `@<path>`):

```bash
cat > /tmp/github-app.json <<EOF
{
  "appId": <integer>,
  "appSlug": "<app-slug>",
  "appName": "<app-name>",
  "clientId": "<Iv...>",
  "owner": {
    "type": "organization",
    "id": <org-integer-id>,
    "slug": "<org-slug>",
    "name": "<org-display-name>"
  },
  "clientSecret": "<paste-client-secret-here>",
  "privateKeyPem": <paste-pem-as-json-string-here>
}
EOF
```

> **Converting the PEM to a JSON-safe string:**
> ```bash
> python3 -c "import json; print(json.dumps(open('/path/to/private-key.pem').read()))"
> ```
> Paste the output (including quotes) into the `privateKeyPem` field. The PEM becomes a single string
> with literal `\n` between lines.

**Find your org's integer ID:**

```bash
curl -s "https://api.github.com/orgs/<org-slug>" | python3 -c "import sys,json; print(json.load(sys.stdin)['id'])"
```

**Create the connector:**

```bash
vercel connect create github --connector-type github --data @/tmp/github-app.json --name <name> --json
```

**Verify the connector — read the full response via the REST API:**

```bash
VERCEL_TOKEN=$(python3 -c "import json; print(json.load(open('$HOME/Library/Application Support/com.vercel.cli/auth.json'))['token'])")
curl -s "https://api.vercel.com/v1/connect/connectors/scl_<id>?teamId=<teamId>" \
  -H "Authorization: Bearer $VERCEL_TOKEN" | python3 -m json.tool
```

Confirm:
- `creationMode: "manual"` — non-managed, first-class
- `type: "github"`
- `appTokens.supportsRefinement: true` — per-mint narrowing is supported
- `appTokens.crossInstallation: false`
- `supportsRevocation: false` — revocation does not reach the provider (known constraint)
- `clientUrl: "https://github.com/apps/<app-slug>"` — GitHub-native install URL

**Clean up the temp file:**

```bash
rm /tmp/github-app.json
```

---

## Step 3 — install the App on repositories

The install flow is GitHub-native (not Vercel-brokered). The repo-selection screen is present —
unlike the managed flow's `autoinstall=true` which removes it.

**Open the install URL in a browser:**

```
https://github.com/apps/<app-slug>/installations/new
```

Choose **"All repositories"** or **"Only select repositories"** depending on your needs. A broad
installation is fine — the read-only ceiling is what matters, not the repo count. Per-mint
`resources` narrowing can still scope each token to one repo per subscription.

**Collect the installation ID** from the URL after installing:

```
https://github.com/organizations/<org>/settings/installations/<INSTALLATION_ID>
```

That integer is the `--installation` value for the temper `attach-credential` command.

---

## Step 4 — configure the broker env vars on the temper-api project

The broker (`VercelConnectBroker` in `temper-services`) needs four env vars, all-or-nothing. Without
them, the credential is recorded but the drift check (the mint that reads `metadata.permissions`)
cannot run.

| Env var | What it is | How to get it |
|---|---|---|
| `VERCEL_CONNECT_ACCESS_TOKEN` | A Vercel access token — the broker uses it to buy the project OIDC JWT (hop 1) | https://vercel.com/<team>/settings/tokens → Create Token |
| `VERCEL_CONNECT_PROJECT_ID` | The Vercel project ID the broker mints on behalf of | `prj_...` from `vercel project ls` or the project settings page |
| `VERCEL_CONNECT_TEAM_ID` | Your Vercel team ID | `team_...` from `vercel teams ls` |
| `VERCEL_CONNECT_TEAM_SLUG` | Your Vercel team slug | from `vercel teams ls` |

**Set them on the temper-api Vercel project** (e.g. `temper-cloud`):

```
https://vercel.com/<team-slug>/<project-name>/settings/environment-variables
```

Apply to **Production** (and **Preview** if you want the drift check there too). **Sensitive** for the
access token; plain for the other three.

**Redeploy the project** for the broker to pick up the config:

```bash
vercel --prod --cwd <path-to-temper-cloud>  # or use the Vercel dashboard
```

**Verify the broker is configured** — check the temper-api logs on boot, or attempt an
`attach-credential` and confirm `verification.verified` is `true` (see the companion guide).

---

## Step 5 — verify the whole chain

After all four pieces are in place, verify end-to-end:

1. **The App is read-only** — re-read the App's permissions page on GitHub. Three read scopes,
   nothing else.
2. **The connector is non-managed** — `creationMode: "manual"` in the connector response.
3. **The broker is configured** — the temper-api project has all four env vars and was redeployed.
4. **The drift check passes** — `attach-credential` returns `verification.verified: true` with
   `observed_reach.permissions` showing only read scopes. This is the witness — the mint that confirms
   the App's read-only permissions are reflected in the actual token. See the companion guide for the
   exact commands.

If the drift check returns `verified: false` with `"note": "not verified — no credential broker is
configured"`, the env vars are missing or the project wasn't redeployed. If it returns write scopes
in `observed_reach.permissions`, the App's permissions are not read-only — go back to Step 1.

---

## Reference script

This is a reference, not a one-shot script — it assumes you've collected the values and edited the
`/tmp/github-app.json` file. Run it section by section, verifying at each step.

```bash
#!/usr/bin/env bash
set -euo pipefail

# --- Config ---
ORG_SLUG="<your-github-org>"
ORG_NAME="<Your Org>"
APP_SLUG="<your-org>-temper-readonly"
APP_NAME="<your-org>-temper-readonly"
APP_ID=<app-id-from-the-App-settings-page>
CLIENT_ID="<Iv23...-from-the-App-settings-page>"
CONNECTOR_NAME="<your-org>-temper-readonly"
VERCEL_TEAM_ID="<team_...>"
VERCEL_TEAM_SLUG="<your-vercel-team-slug>"
TEMPER_PROJECT_ID="<prj_...>"

# --- Step 1: verify the App exists ---
echo "=== Verifying GitHub App ==="
curl -sI "https://github.com/apps/$APP_SLUG" | head -1

# --- Step 2: get the org integer ID ---
ORG_ID=$(curl -s "https://api.github.com/orgs/$ORG_SLUG" | python3 -c "import sys,json; print(json.load(sys.stdin)['id'])")
echo "Org ID: $ORG_ID"

# --- Step 3: build the --data JSON (edit /tmp/github-app.json with secrets first) ---
# The file should look like:
# {
#   "appId": <APP_ID>,
#   "appSlug": "<APP_SLUG>",
#   "appName": "<APP_NAME>",
#   "clientId": "<CLIENT_ID>",
#   "owner": { "type": "organization", "id": <ORG_ID>, "slug": "<ORG_SLUG>", "name": "<ORG_NAME>" },
#   "clientSecret": "...",
#   "privateKeyPem": "..."
# }
echo "=== Create /tmp/github-app.json with secrets, then press Enter ==="
read

# --- Step 4: create the connector ---
echo "=== Creating non-managed connector ==="
vercel connect create github --connector-type github --data @/tmp/github-app.json --name "$CONNECTOR_NAME" --json

# --- Step 5: verify the connector ---
CONNECTOR_ID=$(vercel connect list --json 2>&1 | sed '1,2d' | python3 -c "import sys,json; [print(c['id']) for c in json.load(sys.stdin)['connectors'] if c.get('uid')=='github/$APP_SLUG']")
echo "Connector ID: $CONNECTOR_ID"

VERCEL_TOKEN=$(python3 -c "import json; print(json.load(open('$HOME/Library/Application Support/com.vercel.cli/auth.json'))['token'])")
curl -s "https://api.vercel.com/v1/connect/connectors/$CONNECTOR_ID?teamId=$VERCEL_TEAM_ID" \
  -H "Authorization: Bearer $VERCEL_TOKEN" | python3 -c "
import sys, json
d = json.load(sys.stdin)
print('creationMode:', d.get('creationMode'))
print('supportsRefinement:', d.get('appTokens', {}).get('supportsRefinement'))
print('supportsRevocation:', d.get('supportsRevocation'))
print('clientUrl:', d.get('clientUrl'))
"

# --- Step 6: clean up ---
rm /tmp/github-app.json
echo "=== Connector created. Now install the App and set broker env vars. ==="
echo "Install: https://github.com/apps/$APP_SLUG/installations/new"
echo "Env vars on temper-cloud:"
echo "  VERCEL_CONNECT_ACCESS_TOKEN=<create at vercel.com/settings/tokens>"
echo "  VERCEL_CONNECT_PROJECT_ID=$TEMPER_PROJECT_ID"
echo "  VERCEL_CONNECT_TEAM_ID=$VERCEL_TEAM_ID"
echo "  VERCEL_CONNECT_TEAM_SLUG=$VERCEL_TEAM_SLUG"
```

---

## Honest revocation semantics

`supportsRevocation: false` means revoking a connection in temper stops *future* mints but does
**not** invalidate already-minted provider tokens, which stay live at GitHub until expiry (~1h for
installation tokens). The connection model must **say so** rather than imply revocation is immediate.
`kb_connections.revoked_at` records the temper-side action; the provider-side gap is declared, not
hidden.

---

## Cleanup

To remove a connector (e.g. a probe):

```bash
vercel connect remove github/<connector-name> --disconnect-all --yes
```

The GitHub App must be deleted separately at
`https://github.com/organizations/<org>/settings/apps/<slug>` → Danger Zone → Delete GitHub App.
**Removing the Vercel connector does not uninstall the GitHub App.**

---

## What to do next

Once the infra is verified, head to the companion guide:
[`github-connection-temper.md`](./github-connection-temper.md) — provisioning the temper connection row,
attaching the credential, setting webhook events and tool manifest, and reading the drift check
output.