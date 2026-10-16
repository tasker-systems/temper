# Teams and Roles

**For users, operators, and integrators.** Anyone using Temper with more than one person.

## What a team is

A **team** is a named group of profiles that share access. Teams own contexts, receive
capability grants on resources and cognitive maps, and gate who can read and modify what. A
team is the unit of shared reach: membership confers read access to everything the team owns
or is granted, automatically, through nested teams.

Every profile also gets a private **personal team** (`personal-<handle>`) automatically — so
you always belong to at least one team, and your personal contexts live under it.

## The role ladder

A member holds one role per team, from most to least capable:

| Role | Can do |
|---|---|
| `owner` | Everything a maintainer can, plus delete the team and grant `owner`. |
| `maintainer` | Manage membership (invite, add, remove, change roles) up to their own role, update team metadata. |
| `member` | Read and contribute to the team's shared work. |
| `watcher` | Read-only visibility into the team's shared resources. |

### Who can change whose role

Roles are a rank, and every membership change follows one rule:

- **You can grant a role up to your own.** An owner can make another member an owner; a
  maintainer can grant up to `maintainer`.
- **You can act on members up to your own rank, except owners.** A maintainer can change or
  remove another maintainer. No one but a system admin changes or removes an owner other than
  themselves, so a team may have several owners and none of them can remove another.
- **You can lower your own role or leave, but never raise it.** Members and watchers can leave a
  team but change no one's role.
- **A team keeps at least one owner.** The last owner cannot leave or step down until someone
  else holds `owner`. To hand a team over, grant `owner` to another member, then lower your own
  role.
- **A system admin can make any change**, including repairing a team whose only owner has left.
  On the instance's gating team, only a system admin adds, invites, changes or removes another
  member (any member may still leave it).

Invitations follow the same rank: you can invite at any role up to your own, so an owner can
invite a new owner. An invitation lasts only as long as whoever sent it could still send it. If
they step down, are removed or leave, their pending invitations above what they can now offer are
withdrawn at once. If they lose access to the instance, those invitations are refused, and
withdrawn, when someone tries to accept them. Either way, a departing owner cannot leave
co-owners behind.

Every membership change is recorded on the admin ledger — a member added, a role changed, a
member removed, an invitation accepted, an access request approved, a machine registered onto a
team, and a change made by your identity provider's group sync — readable by the team's owners
and maintainers and by system admins. Identity-provider sync is the one writer that can leave a
team without an owner (the identity provider is authoritative over the roles it assigns); the
ledger records when that happens, for a system admin to repair.

## Nested teams

A team can be nested under a parent (`--parent <ref>` at creation). Child teams inherit the
parent's read reach while the parent stays active. This is how a large organisation models
sub-teams without granting write across boundaries: the child reads up, the parent does not
write down.

## What membership grants — and what it does not

Membership confers **read reach** automatically; write comes with direct membership in the
owning team. A member of the team that owns a context can write into it — no administer step.
To share one of your own contexts with a team, you use `context share` — which grants the team
**read** access, not write. The only path to shared authorship is transferring ownership.

This is deliberate: read inherits up the team tree so that joining a team immediately gives
access to the shared body of work, but reach granted to a team is read-only — a team that
does not own a context never writes into it.

## Further reading

- **The context model that teams own and share:**
  [Contexts and Refs](./contexts-and-refs.md).
- **Governance and administration:**
  [temperkb.io/operating/governance-and-administration](https://temperkb.io/operating/governance-and-administration).
