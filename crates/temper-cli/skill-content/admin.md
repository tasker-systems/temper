# The Admin Surface

Read this when administering a temper deployment (principals, machine clients, connections,
subscriptions, settings) or when a refusal suggests you lack standing. Flags: `reference.md`.

## The gate model

- **Every `temper admin` verb is gated server-side on system-admin standing,** identically in cloud
  and self-host. A CLI flag cannot widen it, and an MCP or API caller is refused by the same rule.
- **Standing is the principal-governance grant and nothing else.** `admin promote` writes it and
  `admin demote` revokes it. Owning the gating team does not make you a system admin; `promote`'s
  `owner` row on a team is a side effect, not the source of admin.
- **A refusal is the gate working, not a syntax problem.** Check `temper auth status` for your own
  standing. Escalating past a refusal is an operator decision (a `promote`), never a workaround.
- **The ledger and erasure deny as 404, never 403.** The ledger is gated per act family; there,
  "you may not read that" and "there is nothing there" are deliberately indistinguishable.
- **The CLI is the whole operator surface.** The MCP tree carries no admin tools.

## Acts take full profile UUIDs

Every principal-facing act takes a strict profile UUID. The substring filter never resolves, and a
prefix is not an id: v7 UUIDs lead with a timestamp, so neighbours share a prefix. Write all 36
characters, copied from a state card's `profile_id` (the card also names the enablement commands).

- **Know the email:** `admin profiles show --email <exact>` resolves one state card; zero matches
  or a verified-address collision answers `not found`, naming the collision; it never guesses.
- **Know only that someone lacks access:** `admin profiles list` (default filter `needs-access`) is
  the work queue, including principals with no standing row. Each row's `matched_email` says why it
  was listed. `admin requests list` is narrower: only principals who signed in and filed a request.

## The map

| Group | For |
|---|---|
| `admin settings` | Instance name, terms version/URI, gating-team slug. No flags shows; any flag updates. |
| `admin promote` / `admin demote` | Grant or revoke governance (plus approved standing if needed). |
| `admin access` | Standing lifecycle: `approve`, `revoke` (only from approved), `deactivate` (any live state), `reactivate` (restores prior standing). |
| `admin requests` | The join queue: `list`, then `review --approve`/`--reject`. Approval enrolls a person (never a machine) in the gating team. |
| `admin reviews` | Reconsideration requests: `list`, then `close`, which records the decision and grants nothing. |
| `admin profiles` | The operator directory: `list` and `show` (by UUID or exact verified email). |
| `admin ledger` | Who granted what, to whom, when. Every standing and governance act lands here; read it after acting. |
| `admin saml` | `provision` (keys, env bundle, SQL), `map-group` (IdP group to team), `verify`. |
| `admin machine` | Machine principals (client_credentials); see below. |
| `admin slack` | Account links: `disconnect` (idempotent). |
| `admin team` | `auto-join <team> <member\|watcher\|none>`. Every approved person (never a machine) is enrolled and reads what the team and its ancestors hold; existing memberships never change. Refused on temper-system, the gating team and personal teams. |
| `admin connection` | The authed link to GitHub or Linear; see below. |
| `admin subscription` | Point a team, context or cogmap at a connection's events: `create`, `list`, `show`, `revoke`. Needs a manage-capable authoring team, reach on the connection, and authorship of a context or map subscriber. |
| `admin erasure` | Permanent erasure; see below. |
| `admin reembed` / `admin reblock` | Re-embed stale vectors (staleness is derived, so a stale vector stays searchable until replaced); one bounded re-blocking step. Both survey with `--dry-run` and are idempotent. |

## Machine principals: which verb answers which failure

- **A new machine's first credential:** `admin machine provision` for a client id your external IdP
  issued (system admin only; refused where temper runs its own authorization server), or
  `admin machine issue` for a temper-minted one (team owners too; `dormant` until the instance runs
  its own AS). Run it before the machine's first call. It creates the agent profile and only the
  reach you name (`--team`, `--cogmap`); reach is never inferred from `--owner-team`.
- **The IdP application rotated:** `admin machine rebind` binds the new client id to the profile,
  keeping authorship, and revokes the old unless `--no-revoke-old`. A new IdP secret needs nothing.
- **A temper-issued secret is lost:** the secret prints once and is rotated, never recovered.
  `admin machine rotate-secret` keeps the previous secret valid for a grace window.
- **Auth must stop:** `admin machine revoke` denies authentication; grants and memberships survive.
  A system admin revoking the last live credential also revokes standing; a team owner's never does.

## Connections: capability is derived, not flagged

- Born `needs_credential`; only `admin connection attach-credential` clears it. Non-empty
  `set-webhooks` makes it ledger-capable (events land), non-empty `set-tools` reach-capable.
- **Owning a connection is not reaching it.** Team read-reach is a separate, read-only grant
  (`grant-reach` / `revoke-reach`).
- `revoke` keeps the profile, emitter and home context so attributed events still resolve.
  Subscriptions are revoked, never deleted.

## Erasure: the survey is the default

Erasure is the one admin act that cannot be undone. Without `--execute`, each of `resource`,
`principal`, `block-history` and `field` surveys what it would reach and leave, and records nothing.
Survey, read it, then re-run with `--execute`.

- `principal --execute` needs `--request-reference <uuid>` (the operator's DSAR record). A refused
  act exits non-zero after printing its answer, which carries the `request_reference`.
- **An act is sent once and never retried.** If `--execute` ends in an error rather than an answer,
  whether it ran is unknown: survey again and re-issue only if it did not land. A landed resource
  surveys `already_erased`; with non-empty `completion_fields` (ledger text from before redaction),
  `resource --execute` again completes it.
- `field <resource>` with no `--field` lists what can be scrubbed by handle, never key or value.
  **Keep is the default:** earlier values are redacted and today's stays; `--clear` says today's
  value is itself the leak. `properties` cannot be cleared.

## Traps

- **`admin reblock` takes exactly one scope;** `--all` needs system-admin standing and must be asked
  for by name. Survey, run, survey again; the receipt carries the resume cursor.
- **Most admin list reads are full dumps.** Only the ledger and `admin profiles list` paginate
  (offset, default 50, clamp 200, no total). An empty-looking list still needs a second look.
- **Bootstrap through `scripts/bootstrap/system-bootstrap.sh`.** The old SQL initialization template
  grants team ownership but neither standing nor governance.
- Deeper reading: `docs/reference/cli/admin.md`, `docs/concepts/auth-identity.md`,
  `docs/concepts/machine-tokens.md`, `docs/playbooks/standing-up-a-machine-credential.md`,
  `docs/concepts/authoring-authorization.md`.
