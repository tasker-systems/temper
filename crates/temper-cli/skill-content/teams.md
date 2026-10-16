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
temper team create <slug> [--name <n>] [--parent <ref>]
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

Once the invitee has signed in, THEY discover and redeem their own invitation:

```
temper invitations         # lists pending invites addressed to your email (all teams) + tokens
temper team join <token>   # accept
temper team decline <token>
```

Inviter-side view of a team's outstanding invites:

```
temper team invitations <team>    # owner/maintainer
```

`temper invitations` resolves an invite to you only when your email maps to exactly
one profile. An email spread across multiple profiles (possible only via unverified
sign-ins) is discounted — not shown, never mis-delivered, and not redeemable either: only the
one verified owner of the address can accept or decline, so handing someone the printed token
does not help. Resolve the duplicate address first.

**Over MCP, invitations are two tools and leaving is a third.** `invitation_read` (views
`mine`, `count`, `team`), `invitation_manage` (actions `accept`, `decline`, `create`, `revoke`)
and `team_manage` (action `leave`) carry the same loop as `temper invitations` (`mine`),
`temper warmup`'s pending count (`count`), `team invitations` (`team`), `team join`,
`team decline`, `team invite`, `team uninvite` and `team leave`, through the same API routes and
the same checks. Team *stewardship* (an owner or maintainer running their own team) belongs on
both surfaces; the instance-administration tools (grants, the admin ledger, access review) stay
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
