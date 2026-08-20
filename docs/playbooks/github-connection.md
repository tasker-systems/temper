# Provision a Read-Only GitHub Connection

**For operators.** This playbook provisions a verified read-only GitHub
connection against a temper deployment, end-toend in one sequence: a GitHub App
with read-only installation permissions, a non-managed Vercel Connect connector
backed by that App, the broker env vars that let temper mint, and the temper
connection row with an attached credential, registered webhook events, and a
declared read-only tool manifest.

By the end you will have a connection whose drift check passed — the mint that
reads the provider's `metadata.permissions` and confirms the App's read-only
ceiling is reflected in the actual token. After that, webhook events can land
and agents can read the remote back through the brokered token. Follow the
steps in order; every step has a verification — do not skip them.

## Prerequisites

- **A Vercel account and team**, with the Vercel CLI installed and authenticated. You will create a non-managed connector and set env vars on the temper-api Vercel project.
- **A GitHub organization** where you can create a GitHub App and install it on repositories.
- **A deployed temper instance** — the temper-api Vercel project, migrated and healthy. See [self-hosting Temper](./self-host-temper.md).
- **The `temper` CLI**, logged in as an admin (`temper auth login`, or `TEMPER_TOKEN` exported) for the `temper admin connection` commands from Step 5 on.
- **The trust boundary temper enforces** — read [the trust boundary](../concepts/trust-boundary.md) to understand the two-gate boundary every call crosses before you configure who it admits.

## Why a non-managed connector

Vercel Connect's *managed* GitHub connector returns a maximal write token (11
write scopes including `workflows:write`, all repositories) regardless of
`scopes:['read']` — silently, with HTTP 200. The only enforcement point that
exists is **the GitHub App itself**. A non-managed connector backed by your own
read-only App makes read-only real *by construction*.

## The four pieces

| Piece | What it is | Where it lives |
|---|---|---|
| **the GitHub App** | A GitHub App with read-only installation permissions. The enforcement point. | GitHub, under the org that owns the repos |
| **the Vercel Connect connector** | A non-managed (`creationMode: "manual"`) connector backed by the App's credentials. | Vercel Connect, in your team |
| **the broker env vars** | Four env vars on the temper-api Vercel project that let temper run the two-hop mint. | The temper-api Vercel project |
| **the temper connection** | The `kb_connections` row + credential + webhook events + tool manifest. | temper itself |

They are tied together by the **connector uid** (e.g. `github/<app-slug>`) and
the **installation ID** GitHub assigns when you install the App. A mismatch in
either is a `401` at mint time, not a warning.

---

## Step 1 — create the GitHub App

The App's installation permissions are the **ceiling** — everything above
depends on them being read-only. This is the most important step; do not rush
it.

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

**All other permissions — No access (the default).** No write scopes. No
`workflows` scope. If you leave a write permission on, the App's tokens will
have it, and the drift check at Step 6 will surface it — but the damage is done
if you didn't notice. **Read the permissions page twice before clicking
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

The connector is the bridge between your GitHub App and Vercel Connect's mint
endpoint. It carries the App's credentials (encrypted at rest by Vercel).

**Build the `--data` JSON file** (secrets stay out of shell history via
`@<path>`):

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
> Paste the output (including quotes) into the `privateKeyPem` field. The PEM
> becomes a single string with literal `\n` between lines.

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
# VERCEL_TOKEN — a Vercel access token (create one at https://vercel.com/<team>/settings/tokens)
curl -s "https://api.vercel.com/v1/connect/connectors/<connector-id>?teamId=<team-id>" \
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

The install flow is GitHub-native (not Vercel-brokered). The repo-selection
screen is present — unlike the managed flow's `autoinstall=true` which removes
it.

**Open the install URL in a browser:**

```
https://github.com/apps/<app-slug>/installations/new
```

Choose **"All repositories"** or **"Only select repositories"** depending on
your needs. A broad installation is fine — the read-only ceiling is what
matters, not the repo count. Per-mint `resources` narrowing can still scope
each token to one repo per subscription.

**Collect the installation ID** from the URL after installing:

```
https://github.com/organizations/<org>/settings/installations/<INSTALLATION_ID>
```

That integer is the `--installation` value for the `attach-credential` command
in Step 6.

---

## Step 4 — configure the broker env vars on the temper-api project

The broker needs four env vars, all-or-nothing. Without them, the credential is
recorded but the drift check (the mint that reads `metadata.permissions`) cannot
run.

| Env var | What it is | How to get it |
|---|---|---|
| `VERCEL_CONNECT_ACCESS_TOKEN` | A Vercel access token — the broker uses it to buy the project OIDC JWT (hop 1) | https://vercel.com/<team>/settings/tokens → Create Token |
| `VERCEL_CONNECT_PROJECT_ID` | The Vercel project ID the broker mints on behalf of | `prj_...` from `vercel project ls` or the project settings page |
| `VERCEL_CONNECT_TEAM_ID` | Your Vercel team ID | `team_...` from `vercel teams ls` |
| `VERCEL_CONNECT_TEAM_SLUG` | Your Vercel team slug | from `vercel teams ls` |

**Set them on the temper-api Vercel project:**

```
https://vercel.com/<team-slug>/<project-name>/settings/environment-variables
```

Apply to **Production** (and **Preview** if you want the drift check there too).
**Sensitive** for the access token; plain for the other three.

**Redeploy the project** for the broker to pick up the config — from the Vercel
dashboard, or with `vercel --prod` from the project directory.

**Verify the broker is configured** — check the temper-api logs on boot, or
proceed to Step 6 and confirm `verification.verified` is `true`.

---

## Step 5 — provision the temper connection row

From here, all commands use the `temper admin connection` CLI suite.

```bash
temper admin connection provision \
  --provider github \
  --name github-readonly \
  --owner-team <team-slug> \
  --reach org \
  --covers <org-slug>
```

**Arguments:**

| Arg | What it is | Example |
|---|---|---|
| `--provider` | The remote system | `github` |
| `--name` | Human-facing name; the addressable slug is derived from it | `github-readonly` |
| `--owner-team` | Team recorded as the connection's OWNER (not its reach). Omitting means teamless, which is admin-only and fails closed | `temper-system` |
| `--reach` | The grain the credential is scoped at, in the provider's terms: `org` \| `workspace` \| `installation` \| `repo-set` \| `project` | `org` |
| `--covers` | What the credential can ACTUALLY see, in provider terms | `tasker-systems` |

> **Declaring reach is not overhead — it IS the declaration.** A connector is a
> reach declaration. You cannot have 50 teams with 50 distinct reaches and
> fewer than 50 declarations. The `--reach`/`--covers` values are where the
> honesty lives; the drift check at Step 6 compares them against what the
> provider actually returns.

**Finding your team slug:**

```bash
temper team list | python3 -c "import sys,json; [print(t['slug'], t['name']) for t in json.load(sys.stdin)]"
```

**Verify — the response includes:**

```json
{
  "id": "<uuid>",
  "slug": "github-readonly",
  "credential": null,
  "webhook_events": [],
  "tool_manifest": {},
  "reach_granularity": "org",
  "reach_covers": "tasker-systems"
}
```

The trailing line says: `needs_credential — no credential is attached`. That is
correct — Step 6 fixes it.

**Copy the `id`** — it's the `<ID>` argument for the next commands.

---

## Step 6 — attach the credential (the drift check runs here)

```bash
temper admin connection attach-credential \
  --broker vercel-connect \
  --connector github/<connector-uid> \
  --installation <installation-id> \
  <connection-id>
```

**Arguments:**

| Arg | What it is | Example |
|---|---|---|
| `--broker` | The implementation behind the broker seam. Never a connector id | `vercel-connect` |
| `--connector` | The broker's identifier for this connector | `github/tasker-systems-temper-readonly` |
| `--installation` | The GitHub App installation ID (from the install URL in Step 3) | `154768494` |
| `<ID>` (positional) | The connection ID from Step 5 | `01a016db-...` |

> **No secret is stored.** `--broker` names the implementation; `--connector`
> identifies a connector that *the broker* holds the secret for. The connector
> id lives on the row, per instance — which is what lets a self-hosted operator
> use their own connectors.

**Verify — the response includes a `verification` object:**

```json
{
  "verification": {
    "verified": true,
    "observed_reach": {
      "permissions": {
        "contents": "read",
        "metadata": "read",
        "pull_requests": "read"
      },
      "repository_selection": "all"
    }
  }
}
```

**This is the drift check — read it carefully.** The mint returned
`metadata.permissions` and `metadata.repository_selection` from the provider,
and the service compared them against the declared
`reach_granularity`/`reach_covers`.

**What each outcome means:**

| `verified` | `observed_reach` | Meaning |
|---|---|---|
| `true` | read scopes only | The App is read-only, the token is read-only, the drift check passed. |
| `true` | write scopes | The App has write permissions — the token is NOT read-only. Go back to Step 1 and fix the App. |
| `false` | `"note": "not verified — no credential broker is configured"` | The broker env vars are missing on the temper-api project, or it wasn't redeployed. See Step 4. |
| `false` | `"note": "needs consent"` | The App isn't installed on any repos, or the installation ID is wrong. See Step 3. |

The trailing line names any gap between declared and observed reach:

> *Where the actual reach exceeds the declared, that gap is real and must be
> acknowledged before granting a team.*

That gap is the excess-reach affirmation — it's working as designed. When you
grant a team reach on this connection, the `grant-reach` command (Step 9) will
require an `--affirm-reach` rationale if the declared reach is broad.

> **The drift check is the end-to-end witness.** `verified: true` with only read
> scopes confirms the whole chain at once: the App is read-only (Step 1), the
> connector is non-managed (Step 2), the App is installed (Step 3), and the
> broker is configured and redeployed (Step 4). Each failure mode above names
> the step to return to.

---

## Step 7 — register webhook events (ledger-capable)

```bash
temper admin connection set-webhooks \
  --event pull_request \
  <connection-id>
```

`--event` is repeatable. The event names are the provider's own (e.g.
`pull_request`, `issue_comment`, `push`).

> **Replaces wholesale.** `set-webhooks` mirrors what the remote is actually
> configured to send — it does not merge. A merge would let a stale entry
> outlive the webhook it names.

**Verify:**

```json
{
  "webhook_events": ["pull_request"]
}
```

The trailing line no longer says "Not ledger-capable" — the connection now
accepts events.

---

## Step 8 — declare the tool manifest (reach-capable)

```bash
temper admin connection set-tools \
  --tool github_get_file_contents \
  --tool github_get_pull_request_files \
  --tool github_get_pull_request \
  --tool github_list_pull_requests \
  --tool github_get_issue \
  --tool github_list_issues \
  --tool github_search_code \
  --tool github_get_codeowners \
  <connection-id>
```

`--tool` is repeatable. The tool names are the read-only remote tools an agent
can call via the brokered token.

> **The manifest is not decorative.** It is the evidence the provider is
> admissible at all. An empty manifest means judgment is IMPOSSIBLE, not merely
> unconfigured. A subscription against a reach-incapable connection is legal and
> durable, but **inert for judgment**.

**Verify:**

```json
{
  "tool_manifest": [
    "github_get_file_contents",
    "github_get_pull_request_files",
    ...
  ]
}
```

The trailing line no longer says "Not reach-capable" — agents can now read the
remote back.

After Step 6 the connection is verified. After Step 7 events can land. After
Step 8 agents can reach the remote. Steps 7 and 8 can be done in either order,
but both must be done for the connection to be useful.

---

## Step 9 — grant a team reach (optional, separate from ownership)

Owning a connection is NOT reaching it. A team must be granted read-reach for
its members to inherit read on what the connection receives.

```bash
temper admin connection grant-reach \
  --team <team-slug> \
  [--affirm-reach "<rationale>"] \
  <connection-id>
```

**When `--affirm-reach` is required:** when the connection `declares_reach()`
(i.e. `reach_granularity` or `reach_covers` is set) AND the observed reach is
broad. The rationale is a named human's acknowledgment that the team's temper
scope is comparably broad — it is recorded, not just accepted. For the
governance policy behind the excess-reach affirmation, see
[governance and administration](https://temperkb.io/operating/governance-and-administration).

**What happens without it:** if the connection declares reach and no
affirmation is provided, the grant is rejected with `409 Conflict`. If the
connection declares no reach, the grant is a plain grant with no affirmation
needed.

**Verify:**

```bash
temper admin connection show <connection-id>
```

The `reach_affirmed_by` / `reach_affirmed_at` / `reach_affirmation` fields are
populated when an affirmation was recorded.

---

## Reference script

This is a reference, not a one-shot script — it assumes you've collected the
values and edited the `/tmp/github-app.json` file. Run it section by section,
verifying at each step.

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
VERCEL_TOKEN="<a-vercel-access-token>"
TEAM_SLUG="temper-system"
REACH="org"

# --- Step 1: verify the App exists ---
echo "=== Verifying GitHub App ==="
curl -sI "https://github.com/apps/$APP_SLUG" | head -1

# --- Step 2: get the org integer ID ---
ORG_ID=$(curl -s "https://api.github.com/orgs/$ORG_SLUG" | python3 -c "import sys,json; print(json.load(sys.stdin)['id'])")
echo "Org ID: $ORG_ID"

# --- Step 2: build the --data JSON (edit /tmp/github-app.json with secrets first) ---
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

# --- Step 2: create the connector ---
echo "=== Creating non-managed connector ==="
vercel connect create github --connector-type github --data @/tmp/github-app.json --name "$CONNECTOR_NAME" --json

# --- Step 2: verify the connector ---
CONNECTOR_ID=$(vercel connect list --json 2>&1 | sed '1,2d' | python3 -c "import sys,json; [print(c['id']) for c in json.load(sys.stdin)['connectors'] if c.get('uid')=='github/$APP_SLUG']")
echo "Connector ID: $CONNECTOR_ID"
curl -s "https://api.vercel.com/v1/connect/connectors/$CONNECTOR_ID?teamId=$VERCEL_TEAM_ID" \
  -H "Authorization: Bearer $VERCEL_TOKEN" | python3 -c "
import sys, json
d = json.load(sys.stdin)
print('creationMode:', d.get('creationMode'))
print('supportsRefinement:', d.get('appTokens', {}).get('supportsRefinement'))
print('supportsRevocation:', d.get('supportsRevocation'))
print('clientUrl:', d.get('clientUrl'))
"

# --- Step 2: clean up ---
rm /tmp/github-app.json
echo "=== Connector created. Now install the App and set broker env vars. ==="
echo "Install: https://github.com/apps/$APP_SLUG/installations/new"
echo "Env vars on the temper-api project:"
echo "  VERCEL_CONNECT_ACCESS_TOKEN=<create at vercel.com/settings/tokens>"
echo "  VERCEL_CONNECT_PROJECT_ID=$TEMPER_PROJECT_ID"
echo "  VERCEL_CONNECT_TEAM_ID=$VERCEL_TEAM_ID"
echo "  VERCEL_CONNECT_TEAM_SLUG=$VERCEL_TEAM_SLUG"
echo "Then redeploy the project."

# --- Step 5: provision ---
CONNECTION=$(temper admin connection provision \
  --provider github \
  --name github-readonly \
  --owner-team "$TEAM_SLUG" \
  --reach "$REACH" \
  --covers "$ORG_SLUG" \
  --format json)
CONNECTION_ID=$(echo "$CONNECTION" | python3 -c "import sys,json; print(json.load(sys.stdin)['id'])")
echo "Connection ID: $CONNECTION_ID"

# --- Step 6: attach credential (drift check runs here) ---
INSTALLATION_ID="<installation-id-from-the-install-URL>"
temper admin connection attach-credential \
  --broker vercel-connect \
  --connector "github/$APP_SLUG" \
  --installation "$INSTALLATION_ID" \
  "$CONNECTION_ID"

# --- Step 7: set webhooks (ledger-capable) ---
temper admin connection set-webhooks \
  --event pull_request \
  "$CONNECTION_ID"

# --- Step 8: set tools (reach-capable) ---
temper admin connection set-tools \
  --tool github_get_file_contents \
  --tool github_get_pull_request_files \
  --tool github_get_pull_request \
  --tool github_list_pull_requests \
  --tool github_get_issue \
  --tool github_list_issues \
  --tool github_search_code \
  --tool github_get_codeowners \
  "$CONNECTION_ID"

# --- Step 9 (optional): grant team reach ---
# temper admin connection grant-reach \
#   --team "$TEAM_SLUG" \
#   --affirm-reach "team scope is comparably broad (same org)" \
#   "$CONNECTION_ID"

# --- Verify ---
temper admin connection show "$CONNECTION_ID"
```

---

## Honest revocation semantics

`supportsRevocation: false` means revoking a connection in temper stops
*future* mints but does **not** invalidate already-minted provider tokens,
which stay live at GitHub until expiry (~1h for installation tokens). The
connection model must **say so** rather than imply revocation is immediate.
`kb_connections.revoked_at` records the temper-side action; the provider-side
gap is declared, not hidden.

---

## Cleanup

To remove a connector:

```bash
vercel connect remove github/<connector-name> --disconnect-all --yes
```

The GitHub App must be deleted separately at
`https://github.com/organizations/<org>/settings/apps/<slug>` → Danger Zone →
Delete GitHub App. **Removing the Vercel connector does not uninstall the
GitHub App.**

---

## Troubleshooting

**`invalid connection id 'github-readonly': invalid character`** — the `<ID>`
argument is a UUID, not the slug. Use the `id` from the `provision` response
(Step 5).

**`not verified — no credential broker is configured`** — the
`VERCEL_CONNECT_*` env vars are missing on the temper-api project, or it wasn't
redeployed after setting them. See Step 4.

**`needs consent`** — the App isn't installed on any repos, or the installation
ID is wrong. Re-install at `https://github.com/apps/<app-slug>/installations/new`
and use the installation ID from the URL (Step 3).

**Write scopes in `observed_reach.permissions`** — the GitHub App has write
permissions. Go back to Step 1, re-read the permissions, and fix the App.
Re-attach the credential after fixing; the drift check will re-run.

**`409 Conflict` on `grant-reach`** — the connection declares reach and you
didn't provide `--affirm-reach`. Add it with a rationale naming why the team's
scope is comparably broad (Step 9).
