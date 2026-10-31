# Teams and Roles

**For users, operators, and integrators.** Anyone using Temper with more than one person.

## What a team is

A **team** is a named group of profiles that share access. Teams own contexts, receive
capability grants on resources and cognitive maps, and gate who can read and modify what. A
team is the unit of shared reach: membership confers read access to everything the team owns
or is granted, automatically, through nested teams.

Every profile also gets a private **personal team** (`personal-<handle>`) automatically — so
you always belong to at least one team, and your personal contexts live under it.

A personal team is yours alone. It holds you and the machines you registered, and nobody else:
no other person can be added, invited or mapped from an identity provider into it, and no team
can be created beneath it. A system admin can list a personal team and its members for audit,
but cannot change its membership, attach anything to it or detach anything from it. Only erasure
acts on it without you.

The same holds for everything inside your personal space: your personal contexts, the resources
homed in them, and the connections and subscriptions your personal team owns. A system admin
cannot rename, retire, restore, share, unshare or reassign your personal contexts, grant or revoke
on what they hold, aim a re-embed at them, or manage your team's connections and subscriptions.
Nor does a system admin read your personal space through the side doors: your retired contexts,
the grant history on your personal resources, and your subscriptions stay out of their listings.
A system admin's own personal space is theirs, like anyone's.

A private cognitive map is personal space too. A map you created is yours until you bind it to a
shared team; a map with no recorded creator is the person's whose personal team it is bound to. A
system admin cannot grant or revoke on your map or on what is homed in it, bind or unbind it, or
register a machine with reach onto it. A map is personal to one person at most: a person's
personal team takes only a map that person created, so to share a map with someone else, bind it
to a shared team you both belong to.

When a system admin is refused on someone else's personal space by id (a share, reassign, bind,
grant or re-embed), the answer is the same `404` an id naming nothing gets, so the refusal does
not reveal that the id exists.

Machines are different: they are principals, and a system admin manages every principal, yours
included. What protects your personal team is the credential. If anyone but you gives a machine on
your personal team a new credential (a rotated secret, or a rebound client id), the machine leaves
your personal team in the same act, and loses its own grants on your personal contexts, maps and
resources, so the new credential cannot open them.

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
- **A system admin can make any change**, including repairing a team whose only owner has left —
  on every team except a personal one, which only its owner changes.
  On the instance's gating team and on `temper-system`, only a system admin adds, invites,
  changes or removes another member (any member may still leave it).

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
