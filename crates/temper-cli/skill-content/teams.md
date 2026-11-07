# Working with Teams

Read this when work must be shared across people, or when asked to create a team, invite, onboard
or offboard someone, change a role, or manage who can see a context.

## The model

- **A team is a named group of profiles that share access.** Teams own contexts, receive read and
  write grants on resources and cognitive maps, and gate visibility.
- **Every profile has a personal team** (`personal-<handle>`, or `personal-<handle>-2` and so on if
  the slug was taken), so member counts are never zero. `team create` refuses the reserved
  `personal-` prefix.
- **Roles are a rank:** `owner` > `maintainer` > `member` > `watcher`. Owner does everything,
  including delete and reassign; maintainer manages membership and metadata; member contributes;
  watcher reads. You can add, invite or grant a role up to your own, so an owner can make another
  owner and a maintainer at most a maintainer.
- **Handing a team over is a role change.** An owner grants `owner` to someone with `team set-role`,
  then lowers their own role. Only a system admin changes or removes an owner other than yourself.
- **The gating team's roster is a system admin's to change.** Every membership change on any team
  is recorded on the admin ledger.
- **Membership alone shares nothing.** Grant explicitly: `temper context share <context> <team-uuid>`
  for a whole context, `temper resource grant <ref> --to-team <team-ref> --read` (or `--write`) for
  one resource, `temper resource revoke <ref> --from-team <team-ref>` to take it back.
- **Reads flow down the team tree.** Every member of a child team, at any role, reads everything
  attached to each ancestor; a parent reads nothing of its children. A parent link confers read on
  the child's whole roster, so both sides consent: `team attach` needs a manager of both,
  `team reparent` moves a team in one act (it never holds both reaches), `team detach` needs a
  manager of either side. `team show` lists `parents`, and `children` only if you manage the team.
- **An auto-join team enrolls every approved person (never a machine).** Setting auto-join is a
  system admin's act, as is a temper-system or gating-team parent or an auto-join team as a child.
  A watcher pool for a team is an auto-join watcher team attached under it.

## Invitations

- **Someone with a profile is added by UUID; anyone else is invited by email.** `team show` lists
  each member's profile UUID, handle and role; `team add-member` takes the UUID, `team invite` the
  email.
- **The invited email decides who may redeem, and no email is sent.** Temper has no mailer. Only an
  account whose verified email is that address can accept or decline; a token handed to anyone
  else does nothing for them.
- **The invitee redeems their own invitation.** Sign-in is self-serve (OAuth or SAML provisions a
  profile). Then `temper invitations` lists pending invites addressed to their email, across teams,
  with tokens; `team join <token>` accepts and `team decline <token>` declines. A manager sees a
  team's outstanding invites with `team invitations <team>` and withdraws one with `team uninvite`.
- **An invite lasts only as long as its sender could still send it.** If the inviter steps down, is
  removed or leaves, their pending invites above what they can now offer are withdrawn at once; if
  they lose access to the instance, those invites are refused and withdrawn on accept. An invite
  that reads as revoked needs a new one from a current manager.
- **`temper warmup` counts what is waiting on you** in its `pending` block, so an invitation
  surfaces at session start. `pending: null` means the block could not be read;
  `pending.join_requests: null` means you are not an instance admin, as distinct from `0`, an
  admin's empty queue.

## Over MCP

`invitation_read` (views `mine`, `count`, `team`), `invitation_manage` (`accept`, `decline`,
`create`, `revoke`), `team_manage` (`leave` and every other team act: `create`, `update`, `delete`,
`restore`, `add_member`, `change_role`, `remove_member`, `attach`, `detach`, `reparent`,
`rename_slug`, `set_auto_join`) and `team_read` (`list`, `detail`) carry the same acts through the
same API routes and checks. They are a person's acts: the API refuses a machine principal.
Instance administration (grants, the admin ledger, access review) stays off MCP. Accept and leave
carry no extra confirmation on either surface, so act on them only when the user asks.

## Commands

`temper team` covers `create` (`--parent` repeatable, up to 16), `list`, `show`, `add-member`,
`invite`, `invitations`, `uninvite`, `set-role`, `remove-member`, `leave`, `update`, `attach`,
`reparent`, `detach`, `restore`, `rename-slug`, `reassign` and `delete`; `temper invitations`,
`team join` and `team decline` are the invitee's side. `temper team <cmd> --help` has the flags.

- **`team reassign <team> --from <uuid> --to <uuid>`** is offboarding: it moves every resource
  `--from` owns in this team's shared contexts to `--to` (a member), in one transaction. The
  original author's provenance is untouched.
- **`team delete`** is a soft delete (owner or system admin); the slug stays reserved. **`team restore`**
  (owner or admin) revives the roster, links and reach, and lists the parents, children and
  roster it revived; review them.

## Traps

- **`temper invitations` resolves an invite only when your email maps to exactly one profile.** An
  email spread across several profiles (possible only via unverified sign-ins) is discounted: not
  shown, not redeemable, and the printed token does not help. Resolve the duplicate address first.
- **A slug rename changes every `+team/...` address.** UUID refs are unaffected; refs written in
  content are not rewritten. New slugs are `a-z`, `0-9` and `-`, starting with a letter or digit,
  at most 63 characters.
