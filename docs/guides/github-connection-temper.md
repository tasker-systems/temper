# Operator guide: provisioning a GitHub connection in temper

This is the companion to [`github-connection-infra.md`](./github-connection-infra.md). That guide
sets up the infra (the GitHub App, the Vercel Connect connector, the broker env vars). This guide
picks up where it leaves off: provisioning the temper connection row, attaching the credential,
declaring the webhook events and tool manifest, and reading the drift check output.

All commands use the `temper admin connection` CLI suite. Every step has a verification — do not
skip them.

> **Prerequisite.** The infra guide is complete: the GitHub App exists with read-only permissions,
> the non-managed connector exists on Vercel Connect, the App is installed on your repos, and the
> broker env vars are set on the temper-api project and redeployed. Without the broker, the drift
> check at `attach-credential` returns `verified: false`.

---

## The four commands

| Step | Command | What it does |
|---|---|---|
| 1 | `temper admin connection provision` | Creates the `kb_connections` row + profile + emitter entity + home context. Born `needs_credential`. |
| 2 | `temper admin connection attach-credential` | Attaches the credential reference. **Runs the drift check** — mints once, reads `metadata.permissions`, compares against declared reach. |
| 3 | `temper admin connection set-webhooks` | Registers the remote event types. Non-empty ⇒ **ledger-capable** — events land. |
| 4 | `temper admin connection set-tools` | Declares the read-only remote tools. Non-empty ⇒ **reach-capable** — agents can read the remote back. |

After step 2, the connection is verified. After step 3, events can land. After step 4, agents can
reach the remote. Steps 3 and 4 can be done in either order, but both must be done for the connection
to be useful.

---

## Step 1 — provision the connection row

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

> **Declaring reach is not overhead — it IS the declaration.** A connector is a reach declaration.
> You cannot have 50 teams with 50 distinct reaches and fewer than 50 declarations. The
> `--reach`/`--covers` values are where the honesty lives; the drift check at attach compares them
> against what the provider actually returns.

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

The trailing line says: `needs_credential — no credential is attached`. That is correct — step 2
fixes it.

**Copy the `id`** — it's the `<ID>` argument for the next commands.

---

## Step 2 — attach the credential (the drift check runs here)

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
| `--installation` | The GitHub App installation ID (from the install URL) | `154768494` |
| `<ID>` (positional) | The connection ID from step 1 | `01a016db-...` |

> **No secret is stored.** `--broker` names the implementation; `--connector` identifies a connector
> that *the broker* holds the secret for. The connector id lives on the row, per instance — which is
> what lets a self-hosted operator use their own connectors.

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

**This is the drift check — read it carefully.** The mint returned `metadata.permissions` and
`metadata.repository_selection` from the provider, and the service compared them against the declared
`reach_granularity`/`reach_covers`.

**What each outcome means:**

| `verified` | `observed_reach` | Meaning |
|---|---|---|
| `true` | read scopes only | The App is read-only, the token is read-only, the drift check passed. |
| `true` | write scopes | The App has write permissions — the token is NOT read-only. Go back to the infra guide and fix the App. |
| `false` | `"note": "not verified — no credential broker is configured"` | The broker env vars are missing on the temper-api project, or it wasn't redeployed. See infra guide Step 4. |
| `false` | `"note": "needs consent"` | The App isn't installed on any repos, or the installation ID is wrong. See infra guide Step 3. |

The trailing line names any gap between declared and observed reach:

> *Where the actual reach exceeds the declared, that gap is real and must be acknowledged before
> granting a team.*

That gap is the excess-reach affirmation (B3) — it's working as designed. When you grant a team reach
on this connection, the `grant-reach` command will require an `--affirm-reach` rationale if the
declared reach is broad.

---

## Step 3 — register webhook events (ledger-capable)

```bash
temper admin connection set-webhooks \
  --event pull_request \
  <connection-id>
```

`--event` is repeatable. The event names are the provider's own (e.g. `pull_request`,
`issue_comment`, `push`).

> **Replaces wholesale.** `set-webhooks` mirrors what the remote is actually configured to send —
> it does not merge. A merge would let a stale entry outlive the webhook it names.

**Verify:**

```json
{
  "webhook_events": ["pull_request"]
}
```

The trailing line no longer says "Not ledger-capable" — the connection now accepts events.

---

## Step 4 — declare the tool manifest (reach-capable)

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

`--tool` is repeatable. The tool names are the read-only remote tools an agent can call via the
brokered token.

> **The manifest is not decorative.** It is the evidence the provider is admissible at all. An empty
> manifest means judgment is IMPOSSIBLE, not merely unconfigured. A subscription against a
> reach-incapable connection is legal and durable, but **inert for judgment**.

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

The trailing line no longer says "Not reach-capable" — agents can now read the remote back.

---

## Step 5 — grant a team reach (optional, separate from ownership)

Owning a connection is NOT reaching it. A team must be granted read-reach for its members to inherit
read on what the connection receives.

```bash
temper admin connection grant-reach \
  --team <team-slug> \
  [--affirm-reach "<rationale>"] \
  <connection-id>
```

**When `--affirm-reach` is required:** when the connection `declares_reach()` (i.e.
`reach_granularity` or `reach_covers` is set) AND the observed reach is broad. The rationale is a
named human's acknowledgment that the team's temper scope is comparably broad — it is recorded, not
just accepted.

**What happens without it:** if the connection declares reach and no affirmation is provided, the
grant is rejected with `409 Conflict`. If the connection declares no reach, the grant is a plain
grant with no affirmation needed.

**Verify:**

```bash
temper admin connection show <connection-id>
```

The `reach_affirmed_by` / `reach_affirmed_at` / `reach_affirmation` fields are populated when an
affirmation was recorded.

---

## The full sequence, as one reference

```bash
#!/usr/bin/env bash
set -euo pipefail

# --- Config ---
TEAM_SLUG="temper-system"
CONNECTOR_UID="github/<your-org>-temper-readonly"
INSTALLATION_ID="<installation-id-from-the-install-URL>"
REACH="org"
COVERS="<your-github-org>"

# --- Step 1: provision ---
CONNECTION=$(temper admin connection provision \
  --provider github \
  --name github-readonly \
  --owner-team "$TEAM_SLUG" \
  --reach "$REACH" \
  --covers "$COVERS" \
  --format json)
CONNECTION_ID=$(echo "$CONNECTION" | python3 -c "import sys,json; print(json.load(sys.stdin)['id'])")
echo "Connection ID: $CONNECTION_ID"

# --- Step 2: attach credential (drift check runs here) ---
temper admin connection attach-credential \
  --broker vercel-connect \
  --connector "$CONNECTOR_UID" \
  --installation "$INSTALLATION_ID" \
  "$CONNECTION_ID"

# --- Step 3: set webhooks (ledger-capable) ---
temper admin connection set-webhooks \
  --event pull_request \
  "$CONNECTION_ID"

# --- Step 4: set tools (reach-capable) ---
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

# --- Step 5 (optional): grant team reach ---
# temper admin connection grant-reach \
#   --team "$TEAM_SLUG" \
#   --affirm-reach "team scope is comparably broad (same org)" \
#   "$CONNECTION_ID"

# --- Verify ---
temper admin connection show "$CONNECTION_ID"
```

---

## Troubleshooting

**`invalid connection id 'github-readonly': invalid character`** — the `<ID>` argument is a UUID,
not the slug. Use the `id` from the `provision` response.

**`not verified — no credential broker is configured`** — the `VERCEL_CONNECT_*` env vars are missing
on the temper-api project, or it wasn't redeployed after setting them. See the infra guide Step 4.

**`needs consent`** — the App isn't installed on any repos, or the installation ID is wrong. Re-install
at `https://github.com/apps/<app-slug>/installations/new` and use the installation ID from the URL.

**Write scopes in `observed_reach.permissions`** — the GitHub App has write permissions. Go back to
the infra guide Step 1, re-read the permissions, and fix the App. Re-attach the credential after
fixing; the drift check will re-run.

**`409 Conflict` on `grant-reach`** — the connection declares reach and you didn't provide
`--affirm-reach`. Add it with a rationale naming why the team's scope is comparably broad.