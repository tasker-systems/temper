# Run a Team

**For individual users.** This playbook covers the operational lifecycle of a
Temper team: creating one, bringing people in, managing their roles, sharing
your work, and offboarding a departing member.

By the end you will have created a team, added or invited its members at the
right roles, shared a context or granted a resource so the team can read your
work, and know how to reassign ownership when someone leaves.

## Prerequisites

- **Temper installed** — see [Install Temper](../playbooks/install-temper.md).
- **Authenticated** — you have signed in and have a profile. Run `temper whoami`
  to confirm.
- **Familiar with contexts and refs** — team membership gates access to
  [contexts](../concepts/contexts-and-refs.md), and the sharing commands use
  the ref grammar described there.

For the durable explanation of what a team is, the role ladder, nested teams,
and what membership grants (and does not), see
[Teams and Roles](../concepts/teams-and-roles.md). This playbook is the
how-to; that page is the why.

## Create a team

```bash
temper team create acme-eng --name "Acme Engineering"
```

You become its owner. Slugs are globally unique. Pass `--parent <ref>` to nest
the team under another — child teams inherit the parent's read reach while the
parent stays active.

List the teams you belong to, or inspect one:

```bash
temper team list
temper team show acme-eng
```

`show` prints the roster — each member's profile UUID, handle, and role. This
is also the one place you can read a teammate's profile UUID, which you need
for the member commands below.

## Bring people in

There are two ways someone joins, depending on whether they already have a
Temper profile.

### Already have a profile: add them directly

If you know a person's profile UUID (from `temper team show` on a shared
team), add them straight away:

```bash
temper team add-member acme-eng 019f41f3-74ab-7ec0-8b0d-cb21662c51cb --role member
```

### Not in the system yet: invite by email

Invite someone by the email they will sign in with:

```bash
temper team invite acme-eng newcomer@acme.com --role member
```

This creates a pending invitation and prints it back — **including a `token`**.
Two things are worth understanding:

- **No email is sent.** Temper doesn't run a mailer. The `invited_email` is a
  *correlator*, not a delivery address — it's how the invitation finds its way
  to the right profile once that profile exists.
- **Signing in is self-serve.** With OAuth/SAML, the newcomer just signs in and
  a profile is provisioned automatically. They don't need the token handed to
  them.

Once the newcomer has signed in, they discover the invitation themselves:

```bash
temper invitations
```

This lists every pending invitation addressed to *their* verified email —
team, role, and the redemption token — across all teams. They then accept:

```bash
temper team join <token>
```

…or decline: `temper team decline <token>`.

As the inviter, you can review a team's outstanding invitations any time:

```bash
temper team invitations acme-eng
```

> **One edge to know about.** `temper invitations` resolves an invitation to
> you only when your email maps unambiguously to a single profile. In the rare
> case where the same email is spread across more than one profile (which can
> only happen with unverified sign-ins), the invitation is *not* auto-surfaced
> — it stays safely invisible rather than risk showing up for the wrong
> account. The fallback there is the old path: the inviter shares the printed
> token directly.

## Manage membership

```bash
temper team set-role acme-eng <profile-uuid> --role maintainer
temper team remove-member acme-eng <profile-uuid>
temper team leave acme-eng          # remove your own membership
```

Update the team's metadata (owner or maintainer):

```bash
temper team update acme-eng --name "Acme Engineering" --description "Platform team"
```

## Share work with a team

Membership alone doesn't share your resources — you grant access explicitly.
For what membership does and does not confer, see
[Teams and Roles](../concepts/teams-and-roles.md).

### Share a context

Sharing a whole context lets the team see everything homed in it:

```bash
temper context share @<handle>/<slug> +<team-slug>
temper context unshare @<handle>/<slug> +<team-slug>
```

The context argument takes a handle-qualified ref (`@<handle>/<slug>`) or a
context UUID. **`@me` is not accepted here** — use your own handle or the
context UUID. To find your handle, run `temper context list` and read the
`owner_ref` column.

The team argument takes a bare slug (`acme-eng`) or a `+`-prefixed slug
(`+acme-eng`).

### Grant a single resource

Grant a single resource to a team at a specific capability:

```bash
temper resource grant <ref> --to-team <team-ref> --read     # visibility
temper resource grant <ref> --to-team <team-ref> --write    # modify
temper resource revoke <ref> --from-team <team-ref>
```

Sharing grants the team **read** access. The only path to shared authorship is
transferring context ownership — see [Contexts and Refs](../concepts/contexts-and-refs.md)
for the model.

## Offboarding

When someone leaves, reassign the work they owned in the team's shared contexts
to whoever picks it up:

```bash
temper team reassign acme-eng --from <departing-uuid> --to <successor-uuid>
```

This is an in-place ownership change over every resource `--from` owns that is
homed in a context shared to this team; `--to` must be a current member. It's a
single transaction — good for bulk offboarding. Provenance is untouched: the
original author stays recorded.

## Retire a team

```bash
temper team delete acme-eng
```

Soft-delete (owner only). The team stops conferring any access immediately, its
children lose the inherited umbrella, but the slug stays reserved and the row is
preserved server-side. The root `temper-system` team cannot be deleted.

## Further reading

- **What a team is, the role ladder, nested teams, and what membership grants:**
  [Teams and Roles](../concepts/teams-and-roles.md).
- **The context model that teams own and share:**
  [Contexts and Refs](../concepts/contexts-and-refs.md).
- **Governance and administration:**
  [temperkb.io/operating/governance-and-administration](https://temperkb.io/operating/governance-and-administration).
