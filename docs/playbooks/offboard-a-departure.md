# Offboard a Departure

**For operators** — someone with system-admin standing on a Temper deployment, handling a
person who has left.

By the end you will have ended a departing person's admission and the credentials behind it,
handed off the work they owned, and know the figure to quote when nobody acts at all.

Every control this playbook uses already exists and is documented elsewhere, because each is a
different kind of act: ending admission is a system-admin act, handing off ownership is a
team-owner act, and the IdP staleness readout is a diagnostic for SAML deployments. What has
been missing is the sentence that puts them in order. This page is that sentence.

## Prerequisites

- **System-admin standing** on the deployment — `temper admin access` and `temper admin ledger`
  are admin-gated. See [The Trust Boundary](../concepts/trust-boundary.md) for what that gate
  is and where it is enforced.
- **The departing person's profile UUID.** `temper team show <team>` lists a team's members
  with their ids.
- **The teams they belonged to.** `temper team show` on each, or ask their manager. Step 2 is
  per-team and there is no all-teams sweep.
- **Owner or maintainer on each of those teams** — step 2 is authorized by team role, not by
  system-admin standing. If you do not manage a team, its owner or a maintainer runs that step;
  see [Run a Team](./run-a-team.md).

## Why the revoke is the acting control

If your deployment derives team membership from IdP groups, it is natural to reach for the IdP
first: remove the person from the mapped group there and expect their Temper reach to follow.

It does not follow on its own. Temper reconciles IdP-derived memberships **when the user logs
in** — that is the only trigger, and there is no background poll and no SCIM. So an IdP-side
group removal takes effect on the departing person's *next successful login*, which is a login
they have no reason to perform.

This is why the readout in step 3 is a **diagnostic and not a control**. A fresh
`last_reconciled_at` says a user's reach agreed with what the IdP asserted at their last
sign-in. It does not say their reach agrees with the IdP now, and it is not the thing that
ends a departure.

**`temper admin access revoke` is.** It takes effect in the transaction you run it in and
waits for no login, which is why it is step 1 rather than a footnote to step 3.

## The sequence

### 1. End their admission

```bash
temper admin access revoke <profile-uuid> --reason "left the org 2026-03-14; offboarded by IT"
```

In one transaction this sets the principal's standing to revoked, demotes them from
system-admin if they held it, and ends their live refresh chains — so no held credential is
carried forward and no later sign-in mints a replacement.

`--reason` is required, and it is not bookkeeping: it is the only free-text record of why,
and it is what a reviewer reads if the person later contests the revocation.

**`revoke` is legal only from `approved` standing.** For a principal in any other live state —
never admitted, or previously denied — use the act that is legal from all of them:

```bash
temper admin access deactivate <profile-uuid>
```

**This is a standing act, not the `is_active` flag.** Two different things share the word
"deactivate": this one moves the principal's *admission* to a deactivated state that
`temper admin access reactivate` restores from, while
[deactivating an account](./self-host-with-saml.md#deactivating-an-account-authn-control) is a
`kb_profiles.is_active` change that stops the account *authenticating* at all. Neither implies
the other; use this one for a departure.

Exactly what a revoked principal can and cannot still do is stated in the SAML playbook's
[Limitations](./self-host-with-saml.md#limitations), and is worth reading once rather than
inferring: they can still complete a sign-in and receive a short-lived, **non-renewable**
access token — deliberately, because contesting a revocation requires one — and that token is
refused on every data route. If you need the sign-in itself to stop, disable the account at
your IdP. That is the control Temper does not own.

> [!IMPORTANT]
> **On a SAML deployment upgraded from before `INTERNAL_RESOLVE_URL` existed, a revoke ends no
> refresh chains** — and it fails silently, because logins and refreshes keep returning `200`.
> The standing change still lands; the credentials it was supposed to end do not. Read
> [the upgrade note in the SAML playbook](./self-host-with-saml.md#limitations) and check the
> `ownerless_live_chains` field it describes **before** you rely on step 1.

**Confirm it landed** — the ledger is the record, and reads back by subject:

```bash
temper admin ledger --subject kb_profiles:<profile-uuid>
```

The revoke appears as a `principal_standing_changed` entry carrying the prior state, the
resulting state, who acted, and the reason you gave.

### 2. Hand off what they owned

Ownership does not move on its own either — a revoked principal still owns every resource they
owned a minute earlier. Run this once **per team** they belonged to:

```bash
temper team reassign acme-eng --from <departing-uuid> --to <successor-uuid>
```

One transaction, and provenance is untouched: the original author stays recorded, only
ownership moves. `--to` must be a current member of that team; `--from` need not be, so this
works after step 1 and after they have been removed from the team.

**What it reaches, exactly.** Every resource the departing person owns that is homed in a
*live* context **shared to that team**. Two things are therefore out of its scope and stay
with them:

- resources in their own contexts that were never shared to a team, and
- resources in a context that has since been retired.

There is no command that sweeps the first of those — a personal context is theirs, and moving
it is a decision rather than an offboarding step.

Finally, remove the membership:

```bash
temper team remove-member acme-eng <departing-uuid>
```

Do this **after** the reassign, not before. `remove-member` reports what the departing member
still owns in that team's contexts, so running it last turns it into a check: a clean removal
with nothing reported means step 2 reached everything it could.

### 3. On a SAML deployment, read the staleness readout

Not to confirm the departure — step 1 already did that, and this readout cannot confirm it.
Read it to see how stale the login-triggered path has become for **everyone else**, which is
the question the revoke does not answer.

The query, its NULL-versus-sentinel semantics, and what `last_signal_was_missing` means live in
one place:
[Seeing when each user's reach was last reconciled](./self-host-with-saml.md#seeing-when-each-users-reach-was-last-reconciled).

The departing person's own row will show a `last_reconciled_at` from before they left, and
will keep showing it. That is the expected reading and not a residue to clean up: the column
records when a reconcile last ran, and no reconcile has run for someone who is not logging in.

## The bound, for when nobody acts

Steps 1 and 2 are the answer for a departure someone knows about. The figure to give a review
board for the case where an IdP-side removal happens and **no** administrator acts is the
lifetime of a session that keeps refreshing without a fresh SAML login:

**`AS_REFRESH_CHAIN_MAX_SECONDS` + `AS_ACCESS_TTL_SECONDS`** — 90 days plus 15 minutes on the
defaults.

Not `AS_ACCESS_TTL_SECONDS` alone: an access token is reminted by refreshing, so its TTL says
how often a credential is renewed, not how long a session lives. What bounds the session is the
refresh **chain**, whose deadline is stamped at the last full SAML login and inherited unchanged
by every rotation. The reasoning, and the cost of lowering it, are in the SAML playbook's
[Limitations](./self-host-with-saml.md#limitations).

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
