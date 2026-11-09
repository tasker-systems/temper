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

A cognitive map can be personal space too. A map you created is yours while it is bound to no
shared team: unbound, or bound only to your own personal team. Once you bind it to a shared team it
is shared, and the team's admins and a system admin reach it as they reach any shared map. A map is
one person's at most: it can be bound to a personal team only when that team's person created it,
and a map with no person as its creator (one a machine made, or one from before creators were
recorded) only to one person's personal team. To share a map between
people, bind it to a shared team. On your personal map a system admin cannot grant or revoke, or
bind or unbind it. The grants you made on it keep working, and you keep every act.

When a system admin is refused because the target is someone else's personal space, the refusal
says so: a `403` naming the kind of object and its person's handle, such as "this cognitive map is
@ana's personal space; the admin arm does not reach it". It never names the content. This is there
to catch a mistaken act (a wrong id, a stale ref), not to hide that the space exists from someone
who can already list personal teams. Anyone else who cannot see the target gets the same answers as
before.

Machines are different: they are principals, and a system admin manages every principal, yours
included. What protects your personal team is the credential. If anyone but you gives a machine on
your personal team a new credential (a rotated secret, or a rebound client id), the machine leaves
your personal team in the same act, so the new credential cannot open it. It also loses any grant
it held on someone's personal space. When you rotate your own machine's credential, nothing changes.

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

Teams form a tree, and a team may have more than one parent. **Reads flow down:** every member
of a child team, at any role, reads everything attached to each of its live ancestors. A parent
reads nothing of its children. This is how a large organisation models sub-teams without granting
write across boundaries. It is also why the leaf-most, most private team stays closed by default.
Opening it is a deliberate act: placing a team under a parent hands the parent's material to the
child's whole roster.

| Act | Command | Who |
|---|---|---|
| Create under parents | `temper team create <slug> --parent <ref> [--parent <ref>…]` | a manager (owner or maintainer) of each parent |
| Attach (or add a second parent) | `temper team attach <parent> <child>` | a manager of **both** teams |
| Move to another parent | `temper team reparent <team> --from <p> --to <q>` | a manager of the team and of the new parent |
| Detach | `temper team detach <parent> <child>` | a manager of either side |
| See the links | `temper team show <team>` | `parents`: every member; `children`: managers only |

A move is one act. The new link is written before the old one is cut, so a refusal (a cycle, for
instance) leaves the team where it was. Placing a team under temper-system or the gating team, or
attaching an auto-join team as a child, is a system admin's act. An auto-join team's roster is
every approved person, so attaching one hands the parent's material to the whole instance. A
watcher pool for a team is an auto-join watcher team attached **under** it
(`temper admin team auto-join <team> watcher`).

Every link and unlink is recorded on the admin ledger, readable by the parent's managers.

## Lifecycle: delete, restore, rename

- `temper team delete <team>` soft-deletes a team (its owner, or a system admin). Every row is
  kept, and a deleted team confers nothing: its children stop reaching anything above it.
- `temper team restore <team-uuid>` is the inverse, with the same authority. Every preserved
  membership, parent link and child's reach confers again. The output lists the parents, children
  and roster it revived.
- `temper team rename-slug <team> <slug>` changes a team's slug (an owner or maintainer). Every
  `+old-slug/...` address stops resolving. UUID refs are unaffected, and refs written in content
  are not rewritten. New slugs are `a-z`, `0-9` and `-`, start with a letter or digit, and are at
  most 63 characters.
- `temper admin team auto-join <team> <member|watcher|none>` (a system admin) sets or clears an
  auto-join pool. Existing memberships are never changed.

Each of these acts is recorded on the admin ledger as `team_lifecycle_changed`, as is a team's
creation.

## Public within

Reads flow down the tree and never up or across: by default a team's contexts are readable only by
its own roster and the teams beneath it. **Public within** is the one way to open a team's material
to a wider part of the tree, for reading.

**Every team is born closed.** Nothing is opened until someone opens it, and nothing opens by
itself: no team, setting or default does it on a team's behalf.

**Both sides consent, or an admin stands in for the anchor.** An opening names a team (T) and one of
its live ancestors (the anchor, A). While it confers, the people on A and on every live team beneath
A, at any role, read every context T owns. It needs a direct owner or maintainer of T, together with
a direct owner or maintainer of A or a system admin. A system admin stands in for the anchor's side
only, never for the team's. When the anchor's subtree reaches the whole instance (temper-system, the
gating team, or a subtree holding an auto-join team), only a system admin opens it. Either side
closes it alone, at any time (when the anchor is temper-system or the gating team, a system admin
closes from the anchor's side). What readers already read is not recalled. The opening is not
inherited by T's child teams, and every open, close and change of effect is recorded on the admin
ledger, readable by the managers of both teams.

| Act | Command | Who |
|---|---|---|
| Open | `temper team public-within open <team> <anchor>` | a manager of the team, and a manager of the anchor or a system admin |
| Close | `temper team public-within close <team> <anchor>` | a manager of either side (a system admin for a temper-system or gating-team anchor), or a system admin |
| Forbid or allow | `temper team public-within policy <team> forbid\|allow` | the team's direct owner, or a system admin |
| See what you read through | `temper team public-within list` | anyone, for themselves |
| See a team's openings | `temper team show <team>` | `public_within`: the team's members; `public_within_in`: the anchor's managers |

**It is for people reading, not for automation.** A machine client, a connection agent, a steward
and an auditor never read through an opening, and a machine principal is refused every act above. A
person reads through it with any token issued to them, such as an MCP session acting as them, and
may reference the opened material from places they author: an edge, a goal link or a blob relation
homed in their own context. Acting on the material still needs membership. That covers writing into
the team's contexts, granting, attenuating a grant, re-sharing, subscribing, and auditing the team's
findings.

**Who grows the audience.** Consent is to the anchor's subtree *as it changes*, not to the roster on
the day it was opened. Among the ways the audience grows:

- any manager of any team under the anchor creates a child team, invites or adds someone,
  attaches or moves in a team they manage, or restores a deleted team beneath it;
- an identity provider's group sync adds people to one of those teams;
- a system admin attaches a team or sets auto-join;
- for a system admin's instance-wide opening, every newly approved person;
- an opening that had stopped conferring confers again when a forbid above it is cleared, or a pool
  team is moved out from under the anchor.

That is a wider set of people than the anchor's own managers. Three things keep it honest:

- `temper team show <team>` reports each opening's current audience as counts of teams and people;
- bringing an auto-join or the gating team under the anchor makes every opening that is not a
  system admin's instance-wide one stop conferring;
- either side can close it at any time.

**What it never exposes.** An opening confers reading of the contexts T itself owns, and nothing
else. It never exposes a context shared *to* T, a grant held by T, a cognitive map bound to T, or
an edge homed in any of those. It never authorizes a write into T's contexts, a grant, a re-share or
a subscription on T's material, or an audit of T's own findings. It never adds anyone to T's roster.
A team's detail says whether each opening confers right now, as yes or no only. It never says which
forbid, deleted team or pool team stops one.

**A forbid covers every path.** A forbid on a team stops every opening over that team and every team
beneath it, by any path, live or deleted. Nothing beneath it can lift the forbid, except by leaving
that subtree. The openings are kept, and they confer again when the forbid is cleared. An opening
also stops conferring while T or the anchor is deleted. If the anchor stops being above T by any
path, the opening is deleted for good, and reopening it needs both sides again. A system admin's
forbid on temper-system turns the feature off for the whole instance.

**`+team/slug` resolves for anyone who reads the context.** A team-owned context is addressable as
`+<team-slug>/<slug>` by every reader of it: its team's members, members of the teams beneath it,
people reading it through an opening, and holders of a share or read grant. Resolving an address
confers nothing: a write aimed at it is checked as a write, and refused to a reader who is not a
member.

## What membership grants — and what it does not

Membership confers **read reach** automatically; write comes with direct membership in the
owning team. A member of the team that owns a context can write into it — no administer step.
To share one of your own contexts with a team, you use `context share` — which grants the team
**read** access, not write. The only path to shared authorship is transferring ownership.

This is deliberate: read inherits down the team tree, from each ancestor to the teams beneath it, so that joining a team immediately gives
access to the shared body of work, but reach granted to a team is read-only — a team that
does not own a context never writes into it.

## Further reading

- **The context model that teams own and share:**
  [Contexts and Refs](./contexts-and-refs.md).
- **Governance and administration:**
  [temperkb.io/operating/governance-and-administration](https://temperkb.io/operating/governance-and-administration).
