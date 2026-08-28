# Offboard a Departure

**For operators** — someone with system-admin standing on a Temper deployment, handling a
person who has left.

By the end you will have ended a departing person's admission and the credentials behind it,
handed off the work they owned, and know the figure to quote when nobody acts at all.

## Prerequisites

- **System-admin standing** on the deployment — `temper admin access` and `temper admin ledger`
  are admin-gated. See [The Trust Boundary](../concepts/trust-boundary.md) for what that gate
  is and where it is enforced.
- **The departing person's profile UUID.** `temper team show <team>` prints the roster — each
  member's profile UUID, handle, role, and `source` — and is the only place to read a
  teammate's id. If you share no team with them, a team owner who does can read it for you.
- **The teams they belonged to.** `temper team show` on each, or ask their manager. Step 2 is
  per-team and there is no all-teams sweep.
- **Owner or maintainer on each of those teams** — step 2 is authorized by team role, not by
  system-admin standing. If you do not manage a team, its owner or a maintainer runs that step;
  see [Run a Team](./run-a-team.md).
- **For the staleness readout only:** `psql` and the database connection string. Steps 1 and 2
  need neither.

## Why the revoke is the acting control

If your deployment derives team membership from IdP groups, it is natural to reach for the IdP
first: remove the person from the mapped group there and expect their Temper reach to follow.

It does not follow on its own. Temper reconciles IdP-derived memberships **when the user logs
in** — that is the only trigger, and there is no background poll and no SCIM. So an IdP-side
group removal takes effect on the departing person's *next successful login*, which is a login
they have no reason to perform.

This is also why the staleness readout below is a **diagnostic and not a control**. A fresh
`last_reconciled_at` says a user's reach agreed with what the IdP asserted at their last
sign-in. It does not say their reach agrees with the IdP now, and it is not the thing that
ends a departure.

**`temper admin access revoke` is.** It takes effect in the transaction you run it in and
waits for no login, which is why it is step 1.

## The sequence

### 1. End their admission

```bash
temper admin access revoke <departing-uuid> --reason "left the org 2026-03-14; offboarded by IT"
```

In one transaction this sets the principal's standing to revoked, demotes them from
system-admin if they held it, and ends their live refresh chains — so no held credential is
carried forward and no later sign-in mints a replacement.

`--reason` is required, and it is not bookkeeping: it is the only free-text record of why,
and it is what a reviewer reads if the person later contests the revocation.

**`revoke` is legal only from `approved` standing.** If it refuses because the principal was
previously denied, or has a request still open, use `deactivate` instead — it is legal from
every live state and reaches the same place:

```bash
temper admin access deactivate <departing-uuid>
```

**This is a standing act, not the `is_active` flag.** Two different things share the word
"deactivate": this one moves the principal's *admission* to a deactivated state that
`temper admin access reactivate` restores from, while
[deactivating an account](./self-host-with-saml.md#deactivating-an-account-authn-control) is a
`kb_profiles.is_active` change that stops the account *authenticating* at all. Neither implies
the other; use this one for a departure.

A revoked principal can still complete a sign-in, by design — contesting a revocation requires
a token. Exactly what that token can and cannot do is stated in the SAML playbook's
[Limitations](./self-host-with-saml.md#limitations); read it there rather than inferring it. If
you need the sign-in itself to stop, that is your IdP's control, not Temper's.

> [!IMPORTANT]
> **`INTERNAL_RESOLVE_URL` must be set for this step to end refresh chains.**
> `temper admin saml provision` emits it only into a freshly generated bundle, so a SAML
> deployment upgraded from before it existed will not have it, and nothing reports its absence —
> logins and refreshes keep returning `200`. After running the revoke, read the API's
> `standing terminal ended no refresh chains` warning: a **non-zero** `ownerless_live_chains`
> means the variable is missing. Point it at your API origin's `/internal/principal/resolve`
> and run the revoke again. See
> [the upgrade note in the SAML playbook](./self-host-with-saml.md#limitations).

**Confirm it landed** — the ledger is the record, and reads back by subject:

```bash
temper admin ledger --subject kb_profiles:<departing-uuid>
```

The revoke appears as a `principal_standing_changed` entry carrying the prior state, the
resulting state, who acted, and the reason you gave.

### 2. Hand off what they owned

Ownership does not move on its own — a revoked principal still owns every resource they owned a
minute earlier. Run this once **per team** they belonged to:

```bash
temper team reassign acme-eng --from <departing-uuid> --to <successor-uuid>
```

`--to` must be a current member of that team; `--from` need not be, so this works after step 1
and after they have been removed from the team. One transaction, and provenance is untouched.

**What it reaches, exactly.** Every resource the departing person owns that is homed in a
*live* context **shared to that team**. Three things are therefore out of scope and stay with
them:

- resources in their own contexts that were never shared to a team,
- resources in a context that has since been retired, and
- resources homed in a cognitive map rather than a context — map interiors are not personally
  owned, and both reassign paths refuse them.

There is no command that sweeps the first of those. A personal context is theirs, and moving it
is a decision rather than an offboarding step. That is the general rule and not a gap in this
sequence: ownership is held directly, so withdrawing a share never moves it. See
[Ownership is not a grant from a team](../concepts/authoring-authorization.md#ownership-is-not-a-grant-from-a-team).

Finally, the membership row itself. Which command applies — or whether one does — depends on
how the membership was created, and `temper team show` prints a `source` for each member.

**`native`** — added directly, by `temper team add-member` or an invitation:

```bash
temper team remove-member acme-eng <departing-uuid>
```

Run it **after** the reassign. On success it reports what the departing member still owns in
this team's contexts, computed by the same query the reassign moves, so the two cannot
disagree — which turns the removal into a check: nothing reported means the reassign reached
everything it could. If the departing member is the team's only `owner`, the removal is refused
until another owner exists; promote a successor first.

**`idp`** — created by SAML reconcile from a mapped group. `remove-member` refuses it, by
design: those rows belong to reconcile. Remove the person from the mapped group at your IdP,
which reconcile applies if they ever sign in again. The row grants nothing meanwhile — step 1
ended their admission, and admission is read on every request regardless of what team rows
exist. Note the consequence for the paragraph above: an IdP-mapped team yields no residual
report, so the reassign is the last step you can run there.

## Afterwards, on a SAML deployment: the staleness readout

The staleness readout answers a question the revoke does not — how stale the login-triggered
path has become for **everyone else**. It cannot confirm this departure, and does not need to;
step 1 did that.

The query, its NULL-versus-sentinel semantics, and what `last_signal_was_missing` means live in
one place:
[Seeing when each user's reach was last reconciled](./self-host-with-saml.md#seeing-when-each-users-reach-was-last-reconciled).

The departing person's own row will show a `last_reconciled_at` from before they left, and will
keep showing it. That is the expected reading and not a residue to clean up: the column records
when a reconcile last ran, and no reconcile has run for someone who is not logging in.

## The bound when nobody acts

Steps 1 and 2 are the answer for a departure someone knows about. If an IdP-side removal happens
and no administrator runs them, the figure to give a review board is
**`AS_REFRESH_CHAIN_MAX_SECONDS` + `AS_ACCESS_TTL_SECONDS`** — 90 days plus 15 minutes on the
defaults. Why it is the refresh chain and not the access TTL, and the cost of lowering it, are
in the SAML playbook's [Limitations](./self-host-with-saml.md#limitations).

## What this does not cover

- **Automated de-provisioning.** SCIM is not available. There is no background poll of the IdP,
  and no catch-up pass over users who are not signing in.
- **Stopping the sign-in itself.** That is your IdP's control. Temper's
  [account deactivation](./self-host-with-saml.md#deactivating-an-account-authn-control)
  (`is_active`) stops an account authenticating against the API, which is a different question
  from what its admission permits; note that reconcile never deactivates a profile.
- **Deleting anything.** Every act here is reversible and preserves history:
  `temper admin access reactivate` restores a deactivated principal's prior standing, and
  `temper admin access approve` readmits a revoked one.

## Further reading

- **Teams, roles, and the ownership handoff from the team owner's side:**
  [Run a Team](./run-a-team.md).
- **The inverse of this playbook — turning a fresh deployment into one with people in it:**
  [Bootstrap an Org](./bootstrap-an-org.md).
- **Where admission and `is_active` are enforced:**
  [The Trust Boundary](../concepts/trust-boundary.md).
- **Every flag of every command used here:** [`temper admin`](../reference/cli/admin.md).
