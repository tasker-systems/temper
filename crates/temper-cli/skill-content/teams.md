# Working with Teams

A **team** is a named group of profiles that share access — teams own contexts,
receive read/write grants on resources and cognitive maps, and gate visibility.
Every profile also has an automatic personal team (`personal-<handle>`), so member
counts are never zero.

Reach for teams when work needs to be shared across people, or when you're asked
to invite/onboard/offboard someone or manage who can see a context.

## Roles

`owner` > `maintainer` > `member` > `watcher`. Owner does everything (incl. delete,
reassign); maintainer manages membership + metadata; member contributes; watcher is
read-only. Ownership is held at creation, never invited. Invitations cap at
`maintainer`.

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
- **No email is sent.** Temper has no mailer. The `invited_email` is a *correlator*,
  not a delivery channel.
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
sign-ins) is discounted — not shown, never mis-delivered; the fallback there is the
inviter sharing the printed token directly.

Over MCP the same three verbs exist: `list_my_invitations`, `accept_invitation`,
`decline_invitation`.

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
