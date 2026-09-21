# The Admin Surface

Read this when the work is **administering a temper deployment** — principals, machine clients,
connections, subscriptions, or instance settings — or when a refusal makes you suspect you are
not the admin the surface thinks it needs. Flag syntax lives in `reference.md` (generated from
the binary itself); this file is the map, the gate model, and the traps.

## The gate model

Every `temper admin` verb is gated **server-side** on *system-admin standing*, identically in
cloud and self-host — there is no branch anywhere. Standing is the **principal-governance
grant** and nothing else:

> Owning the gating team does **not** make you a system admin. The gating team is instance
> state (see `admin settings`); the grant is what `admin promote` writes and `admin demote`
> revokes.

Consequences worth internalizing before you act:

- A refusal reads as a structured error, not as "wrong syntax". Check `temper auth status`
  first — it reports the caller's own standing.
- Gate checks run server-side, so a CLI flag cannot widen them, and an MCP/API caller is
  refused by exactly the same rule.
- The **ledger** is the one read gated per act family rather than wholesale, and it denies as
  **404, never 403** — on that surface "you may not read that" and "there is nothing there" are
  deliberately indistinguishable.

## The map

| Group | What it is for |
|-------|----------------|
| `admin settings` | Instance state: name, terms version/URI, the gating-team slug. Show with no flags; any flag *updates*. |
| `admin promote` / `admin demote` | Grant / revoke the governance grant (plus approved standing if needed). `promote` adds an `owner` row on a team as a **side effect** — that row is not what confers admin, and `--team` is not where admin comes from. |
| `admin access` | The standing lifecycle: `approve`, `revoke`, `deactivate`, `reactivate` — legal transitions are narrow (`revoke` only from approved; `deactivate` from any live state; `reactivate` restores prior standing). |
| `admin requests` | The join queue: `list` pending requests for the gating team, `review` with `--approve`/`--reject`. Approving also enrolls gating-team membership atomically. |
| `admin reviews` | Reconsideration requests from revoked principals: `list`, then `close` — which records the decision and **grants nothing**. |
| `admin profiles` | The operator directory: `list` (default filter `needs-access` — the work queue of everyone who lacks access, including principals with no standing row) and `show` (one principal's state card, by UUID **or exact verified email** — the bridge into the strict-UUID acts). |
| `admin ledger` | Who granted what, to whom, and when. Every standing and governance act is ledgered — read it after acting, and treat it as the audit trail you are writing into. |
| `admin saml` | SAML provisioning tooling: `provision` (keys + env bundle + SQL), `map-group` (IdP group → team), `verify`. |
| `admin machine` | Machine principals (client_credentials): `provision`, `rebind`, `issue`, `rotate-secret`, `list`, `show`, `revoke`. See below — each verb answers a different failure. |
| `admin slack` | Account links: `disconnect` (idempotent). |
| `admin connection` | Provision and configure the authed link to a remote system (GitHub, Linear). See below — capability is derived, not flagged. |
| `admin subscription` | Point a team/context/cogmap at a connection's events: `create`, `list`, `show`, `revoke`. The two-leg authz gate (authoring-team manage-capable **and** a reach grant on the connection) runs server-side. |
| `admin reembed` / `admin reblock` | Corpus maintenance: re-embed stale vectors; run one bounded re-blocking step. Both are survey-first (`--dry-run`) and idempotent by design. |

## Resolving a person to an actionable id

Every principal-facing *act* takes a strict **profile UUID**, and the substring filter never
resolves — but `admin profiles` is the bridge from a human to an id:

1. Know the email → `admin profiles show --email <exact>` resolves it to a single state card;
   zero matches or a verified-address collision answers `not found` (naming the collision) —
   the server refuses to guess.
2. Don't know who, only that someone lacks access → `admin profiles list` (default
   `needs-access`) is the work queue; rows carry `matched_email` — *why* that person was
   enumerated.
3. The card names the enablement commands; copy the `profile_id` from it. Never guess a UUID
   from a prefix — write all 36 characters (see *Referencing Other Resources* in SKILL.md).

`admin requests list` remains the queue-side email bridge for principals who signed in **and**
filed a join request; the directory's `needs-access` view is wider — it includes principals
who signed in and never requested.

## Machine principals — which verb answers which failure

- **First credential for a new machine** → `admin machine provision`. Run it *before* the
  machine's first call: it creates the agent profile, gating-team membership, and the reach you
  name (`--team`, `--cogmap`). Reach is plural and never inferred from `--owner-team`.
- **IdP application rotated, profile must survive** → `admin machine rebind`. It binds the new
  client id to the existing profile, preserving authorship history; the old client is revoked
  unless `--no-revoke-old`. (Rotating only the IdP *secret* needs no temper action at all.)
- **Temper is the issuer** → `admin machine issue`. The secret is printed **once**; a lost one
  is rotated, not recovered: `admin machine rotate-secret` keeps the previous secret valid for
  a grace window so already-issued tokens keep verifying.
- **Auth must stop** → `admin machine revoke`. It denies authentication **and nothing else** —
  grants and memberships hang off the profile and survive.

## Connections — capability is derived, not flagged

A connection is born `needs_credential`; the state is derived from the credential column being
NULL, and only `admin connection attach-credential` flips it — there is no status flag to set.
Capability comes in two independent halves:

- **`set-webhooks` non-empty ⇒ ledger-capable** — remote events land and facts accrue.
- **`set-tools` non-empty ⇒ reach-capable** — agents can read the remote back.

And the rule people get wrong: **owning a connection is not reaching it.** Team read-reach is a
separate grant (`grant-reach` / `revoke-reach`), and it is read-only. `revoke` keeps the
profile, emitter entity, and home context alive — events already attributed to the emitter must
keep resolving. Subscriptions are revoked, never deleted: a revoked one stops matching but stays
resolvable.

## Maintenance

- `admin reembed`: staleness is **derived, not marked**, so it is safe to re-run and nothing is
  destroyed — a stale vector stays searchable until a fresh one replaces it. Start with
  `--dry-run`.
- `admin reblock`: **exactly one scope** per run. `--all` requires system-administrator
  standing and must be asked for by name — never the default. Survey (`--dry-run`), run, then
  survey again to verify; the receipt carries the resume cursor.

## Bootstrap and operator references

First-admin and org provisioning runs through `scripts/bootstrap/system-bootstrap.sh` — an
idempotent applier over an install profile. Do **not** follow the stale SQL initialization
template: it grants team ownership but neither standing nor governance, which recreates the
bootstrap chicken-and-egg.

Per-command reference: `docs/reference/cli/admin.md` (also a committed projection, gated like
this skill). Deeper reading: `docs/concepts/auth-identity.md`, `docs/concepts/machine-tokens.md`,
`docs/playbooks/standing-up-a-machine-credential.md`, `docs/concepts/authoring-authorization.md`.

## Boundaries — stated, not inferred

- **The CLI is the complete operator surface.** The MCP tree deliberately carries no admin
  tools (the ledger/profile implementations exist but are declared off-MCP); an MCP client
  cannot do this work.
- **Volume is mostly full dumps.** The ledger and `admin profiles list` paginate (offset-based,
  default 50, clamp 200, no total/cursor); the other list reads return every row in one
  response. Treat "the list looked empty" the way SKILL.md's truncation rule teaches: absence
  needs a second look, though here the trap is volume, not paging.
- Refusals are the gate working, not a syntax problem — escalating past one is an operator
  decision (a `promote`), never something to route around.
