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
| `owner` | Everything, plus delete the team and transfer/reassign resources. Only owners can invite at any role. |
| `maintainer` | Manage membership (invite, remove, change roles), update team metadata. |
| `member` | Read and contribute to the team's shared work. |
| `watcher` | Read-only visibility into the team's shared resources. |

**Ownership is never invited** — it is held at creation and moved deliberately (through
offboarding). Invitations top out at `maintainer`.

## Nested teams

A team can be nested under a parent (`--parent <ref>` at creation). Child teams inherit the
parent's read reach while the parent stays active. This is how a large organisation models
sub-teams without granting write across boundaries: the child reads up, the parent does not
write down.

## What membership grants — and what it does not

Membership confers **read reach**, not write. To write into a team-owned context, a member
needs to administer the context (or be an instance admin). To share one of your own contexts
with a team, you use `context share` — which grants the team **read** access, not write. The
only path to shared authorship is transferring ownership.

This is deliberate: read inherits up the team tree so that joining a team immediately gives
access to the shared body of work, but writing is a separate, explicit grant. A new member is
never silently able to modify something they just gained read access to.

## Further reading

- **The context model that teams own and share:**
  [Contexts and Refs](./contexts-and-refs.md).
- **Governance and administration:**
  [temperkb.io/operating/governance-and-administration](https://temperkb.io/operating/governance-and-administration).
