# Working with Teams

A **team** is a named group of profiles that share access — teams own contexts,
receive read/write grants on resources and cognitive maps, and gate visibility.
Every profile also has an automatic personal team (`personal-<handle>`, or
`personal-<handle>-2` and so on if that slug was already held), so member counts are never
zero. The `personal-` prefix is reserved: `temper team create` refuses it.

Reach for teams when work needs to be shared across people, or when you're asked
to invite/onboard/offboard someone or manage who can see a context.

## Roles

`owner` > `maintainer` > `member` > `watcher`. Owner does everything (incl. delete,
reassign); maintainer manages membership + metadata; member contributes; watcher is
read-only. Roles are a rank: you can add, invite or grant a role up to your own, so an
owner can make another owner and a maintainer at most a maintainer. Only a system admin
changes or removes an owner other than yourself; an owner hands a team over by granting
`owner` to someone, then lowering their own role. On the gating team, only a system admin
changes the roster. Every membership change is recorded on the admin ledger.

## Create / inspect

```
temper team create <slug> [--name <n>] [--parent <ref>]...   # --parent repeatable (≤16)
temper team list
temper team show <team>      # roster: each member's profile UUID, handle, role
```

`team show` is where you get a teammate's profile UUID (needed for member commands).

## Bringing people in

**If they already have a profile** — add by profile UUID:

```
temper team add-member <team> <profile-uuid> --role <role>
```

**If they're not in the system yet** — invite by email:

```
temper team invite <team> <email> --role <role>
```

Key facts about invites:
- **No email is sent.** Temper has no mailer. The `invited_email` is not a delivery
  channel; it decides **who may redeem**: only an account whose verified email is that
  address can accept or decline. A token handed to anyone else does nothing for them.
- **Sign-in is self-serve** (OAuth/SAML auto-provisions a profile). The invitee does
  not need the token handed to them.
- **An invite lasts only as long as its sender could still send it.** If the inviter
  steps down, is removed or leaves, their pending invites above what they can now offer
  are withdrawn at once. If they lose access to the instance, those invites are refused
  (and withdrawn) when someone tries to accept them. A maintainer keeps invites up to
  `maintainer`; someone who can no longer invite keeps none. If an invite reads as revoked,
  ask a current manager of the team to send a new one.

Once the invitee has signed in, THEY discover and redeem their own invitation:

```
temper invitations         # lists pending invites addressed to your email (all teams) + tokens
temper team join <token>   # accept
temper team decline <token>
```

Inviter-side view of a team's outstanding invites:

```
temper team invitations <team>    # a manager of the team (owner/maintainer)
```

`temper invitations` resolves an invite to you only when your email maps to exactly
one profile. An email spread across multiple profiles (possible only via unverified
sign-ins) is discounted — not shown, never mis-delivered, and not redeemable either: only the
one verified owner of the address can accept or decline, so handing someone the printed token
does not help. Resolve the duplicate address first.

**Over MCP, invitations are two tools and leaving is a third.** `invitation_read` (views
`mine`, `count`, `team`), `invitation_manage` (actions `accept`, `decline`, `create`, `revoke`)
and `team_manage` carry the same loop as `temper invitations` (`mine`),
`temper warmup`'s pending count (`count`), `team invitations` (`team`), `team join`,
`team decline`, `team invite`, `team uninvite` and `team leave`, through the same API routes and
the same checks. `team_manage` also carries every other team act (`create`, `update`, `delete`,
`restore`, `add_member`, `change_role`, `remove_member`, `attach`, `detach`, `reparent`,
`rename_slug`, `set_auto_join`) and `team_read` the reads (`list`, `detail`): they are a person's
acts, and the API refuses a machine principal's call. Team *stewardship* belongs on both surfaces; the instance-administration tools (grants, the admin ledger, access review) stay
off MCP. Accept and leave carry no extra confirmation on either surface: the user's client asks
before an agent's write, whichever door it uses. Act on them only when the user asks.

`temper warmup` counts what is waiting on you, so an invitation surfaces at session start
without anyone having to think to look:

```
"pending": { "invitations": 2, "join_requests": null, "review_requests": null }
```

Two nulls at different levels mean different things. `pending: null` means the block could
not be read at all; `pending.join_requests: null` means you are not an instance admin, so
nothing was read — as distinct from `0`, which means an admin read an empty queue.

## Membership + metadata

```
temper team set-role <team> <profile-uuid> --role <role>
temper team remove-member <team> <profile-uuid>
temper team leave <team>
temper team update <team> [--name <n>] [--description <d>]
```

## Placing teams in the tree

Reads flow **down** the team tree: every member of a child team, at any role, reads everything
attached to each of its ancestors; a parent reads nothing of its children. So writing a parent
link confers read on the child's whole roster, and both sides consent.

```
temper team attach <parent> <child>              # a manager of both; also adds a second parent
temper team reparent <team> --from <p> --to <q>  # one act: never holds both reaches
temper team detach <parent> <child>              # a manager of either side
temper team show <team>                          # lists `parents`; `children` if you manage it
```

A temper-system or gating-team parent, or an auto-join team as the child, is a system admin's
act. A watcher pool for a team is an auto-join watcher team attached **under** it (setting
auto-join is a system admin's act).

## Restore and rename

```
temper team restore <team-uuid>         # owner or admin; revives roster, links and reach
temper team rename-slug <team> <slug>   # owner/maintainer; +old/... refs stop resolving
```

A restore lists the parents, children and roster it revived — review them. A slug rename changes
every `+team/...` address; UUID refs are unaffected and refs written in content are not
rewritten. New slugs are `a-z`, `0-9` and `-`, starting with a letter or digit (≤63).

## Sharing work with a team

Membership alone shares nothing — grant explicitly:

```
temper context share <context> <team-uuid>     # whole context
temper resource grant <ref> --to-team <team-ref> --read   # or --write
temper resource revoke <ref> --from-team <team-ref>
```

## Offboarding + retire

```
temper team reassign <team> --from <departing-uuid> --to <successor-uuid>
temper team delete <team>     # soft-delete, owner only; slug stays reserved
```

`reassign` moves ownership of every resource `--from` owns in this team's shared
contexts to `--to` (a member), in one transaction. Provenance (original author) is
untouched.
