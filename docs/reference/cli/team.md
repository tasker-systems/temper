<!-- GENERATED — do not edit. Emitted from the built binary's `--help` by scripts/emit-cli-reference.py; see .github/scripts/check-cli-reference-drift.sh. -->

# `temper team`

Manage team membership and access

```text
Manage team membership and access

Usage: temper team [OPTIONS] <COMMAND>

Commands:
  join           Accept a team invitation by its token. Only the account whose verified email the invitation names can accept it
  invite         Invite an email to a team (a team manager or a system admin; on a personal team, its owner only)
  decline        Decline a team invitation by its token
  invitations    List pending invitations for a team (a team manager or a system admin; on a personal team, its owner only)
  uninvite       Revoke (withdraw) a pending invitation (a team manager or a system admin; on a personal team, its owner only)
  show           Show a team's detail and member roster
  leave          Leave a team you are a member of (removes your membership)
  attach         Attach a child team under a parent team (a manager of both; a temper-system or gating-team parent, or a child with an auto-join or the gating team in or below it, a system admin)
  reparent       Move a team from one parent to another, in one act (a manager of the team and of `--to`)
  restore        Restore a soft-deleted team (its owner, or a system admin)
  rename-slug    Change a team's slug (an owner or maintainer; temper-system and the gating team, a system admin)
  detach         Detach a child team from a parent team (a manager of the parent or of the child, or a system admin)
  remove-member  Remove a member from a team (owner/maintainer)
  set-role       Change a member's role (a team owner/maintainer or a system admin; personal team: its owner)
  update         Update a team's metadata (owner/maintainer)
  delete         Soft-delete a team (its owner, or a system admin)
  reassign       Bulk-reassign a departing member's team-scoped resources (offboarding)
  create         Create a team (you become its owner)
  add-member     Add a member to a team (owner/maintainer only)
  list           List the teams you are a member of
  public-within  Open a team's owned contexts to the people under one of its ancestors, close that opening, forbid it over a subtree, or list the openings you read through
  help           Print this message or the help of the given subcommand(s)

Options:
      --vault <VAULT>      Path to vault (overrides TEMPER_VAULT and auto-detection)
      --format <FORMAT>    Output format: json | toon (default: toon on a TTY, json otherwise). Precedence: --format → TEMPER_FORMAT → cli.format config → TTY default
      --embed-threads <N>  ONNX intra-op threads for embedding. `0` = let ONNX Runtime decide. Default: this machine's performance-core count (NOT its total core count — efficiency cores measurably slow the batch down). Precedence: --embed-threads → TEMPER_ONNX_INTRA_THREADS → detected → 1
      --color <COLOR>      Color output: auto | always | never (default: auto). Precedence: --color → TEMPER_COLOR → cli.color config → NO_COLOR → auto
  -h, --help               Print help
```

### `temper team join`

```text
Accept a team invitation by its token. Only the account whose verified email the invitation names can accept it

Usage: temper team join [OPTIONS] <TOKEN>

Arguments:
  <TOKEN>  Invitation token (from `temper invitations`)

Options:
      --vault <VAULT>      Path to vault (overrides TEMPER_VAULT and auto-detection)
      --format <FORMAT>    Output format: json | toon (default: toon on a TTY, json otherwise). Precedence: --format → TEMPER_FORMAT → cli.format config → TTY default
      --embed-threads <N>  ONNX intra-op threads for embedding. `0` = let ONNX Runtime decide. Default: this machine's performance-core count (NOT its total core count — efficiency cores measurably slow the batch down). Precedence: --embed-threads → TEMPER_ONNX_INTRA_THREADS → detected → 1
      --color <COLOR>      Color output: auto | always | never (default: auto). Precedence: --color → TEMPER_COLOR → cli.color config → NO_COLOR → auto
  -h, --help               Print help
```

### `temper team invite`

```text
Invite an email to a team (a team manager or a system admin; on a personal team, its owner only)

Usage: temper team invite [OPTIONS] --role <ROLE> <TEAM> <EMAIL>

Arguments:
  <TEAM>   Team slug (optionally `+`-prefixed) or UUID
  <EMAIL>  Email address to invite

Options:
      --role <ROLE>        Role to grant on acceptance: owner | maintainer | member | watcher, up to the inviter's own rank on the team
      --vault <VAULT>      Path to vault (overrides TEMPER_VAULT and auto-detection)
      --format <FORMAT>    Output format: json | toon (default: toon on a TTY, json otherwise). Precedence: --format → TEMPER_FORMAT → cli.format config → TTY default
      --embed-threads <N>  ONNX intra-op threads for embedding. `0` = let ONNX Runtime decide. Default: this machine's performance-core count (NOT its total core count — efficiency cores measurably slow the batch down). Precedence: --embed-threads → TEMPER_ONNX_INTRA_THREADS → detected → 1
      --color <COLOR>      Color output: auto | always | never (default: auto). Precedence: --color → TEMPER_COLOR → cli.color config → NO_COLOR → auto
  -h, --help               Print help
```

### `temper team decline`

```text
Decline a team invitation by its token

Usage: temper team decline [OPTIONS] <TOKEN>

Arguments:
  <TOKEN>  Invitation token

Options:
      --vault <VAULT>      Path to vault (overrides TEMPER_VAULT and auto-detection)
      --format <FORMAT>    Output format: json | toon (default: toon on a TTY, json otherwise). Precedence: --format → TEMPER_FORMAT → cli.format config → TTY default
      --embed-threads <N>  ONNX intra-op threads for embedding. `0` = let ONNX Runtime decide. Default: this machine's performance-core count (NOT its total core count — efficiency cores measurably slow the batch down). Precedence: --embed-threads → TEMPER_ONNX_INTRA_THREADS → detected → 1
      --color <COLOR>      Color output: auto | always | never (default: auto). Precedence: --color → TEMPER_COLOR → cli.color config → NO_COLOR → auto
  -h, --help               Print help
```

### `temper team invitations`

```text
List pending invitations for a team (a team manager or a system admin; on a personal team, its owner only)

Usage: temper team invitations [OPTIONS] <TEAM>

Arguments:
  <TEAM>  Team slug (optionally `+`-prefixed) or UUID

Options:
      --vault <VAULT>      Path to vault (overrides TEMPER_VAULT and auto-detection)
      --format <FORMAT>    Output format: json | toon (default: toon on a TTY, json otherwise). Precedence: --format → TEMPER_FORMAT → cli.format config → TTY default
      --embed-threads <N>  ONNX intra-op threads for embedding. `0` = let ONNX Runtime decide. Default: this machine's performance-core count (NOT its total core count — efficiency cores measurably slow the batch down). Precedence: --embed-threads → TEMPER_ONNX_INTRA_THREADS → detected → 1
      --color <COLOR>      Color output: auto | always | never (default: auto). Precedence: --color → TEMPER_COLOR → cli.color config → NO_COLOR → auto
  -h, --help               Print help
```

### `temper team uninvite`

```text
Revoke (withdraw) a pending invitation (a team manager or a system admin; on a personal team, its owner only)

Usage: temper team uninvite [OPTIONS] <TEAM> <INVITATION_ID>

Arguments:
  <TEAM>           Team slug (optionally `+`-prefixed) or UUID
  <INVITATION_ID>  Invitation UUID (from `temper team invitations`)

Options:
      --vault <VAULT>      Path to vault (overrides TEMPER_VAULT and auto-detection)
      --format <FORMAT>    Output format: json | toon (default: toon on a TTY, json otherwise). Precedence: --format → TEMPER_FORMAT → cli.format config → TTY default
      --embed-threads <N>  ONNX intra-op threads for embedding. `0` = let ONNX Runtime decide. Default: this machine's performance-core count (NOT its total core count — efficiency cores measurably slow the batch down). Precedence: --embed-threads → TEMPER_ONNX_INTRA_THREADS → detected → 1
      --color <COLOR>      Color output: auto | always | never (default: auto). Precedence: --color → TEMPER_COLOR → cli.color config → NO_COLOR → auto
  -h, --help               Print help
```

### `temper team show`

```text
Show a team's detail and member roster

Usage: temper team show [OPTIONS] <TEAM>

Arguments:
  <TEAM>  Team slug (optionally `+`-prefixed) or UUID

Options:
      --vault <VAULT>      Path to vault (overrides TEMPER_VAULT and auto-detection)
      --format <FORMAT>    Output format: json | toon (default: toon on a TTY, json otherwise). Precedence: --format → TEMPER_FORMAT → cli.format config → TTY default
      --embed-threads <N>  ONNX intra-op threads for embedding. `0` = let ONNX Runtime decide. Default: this machine's performance-core count (NOT its total core count — efficiency cores measurably slow the batch down). Precedence: --embed-threads → TEMPER_ONNX_INTRA_THREADS → detected → 1
      --color <COLOR>      Color output: auto | always | never (default: auto). Precedence: --color → TEMPER_COLOR → cli.color config → NO_COLOR → auto
  -h, --help               Print help
```

### `temper team leave`

```text
Leave a team you are a member of (removes your membership)

Usage: temper team leave [OPTIONS] <TEAM>

Arguments:
  <TEAM>  Team slug (optionally `+`-prefixed) or UUID

Options:
      --vault <VAULT>      Path to vault (overrides TEMPER_VAULT and auto-detection)
      --format <FORMAT>    Output format: json | toon (default: toon on a TTY, json otherwise). Precedence: --format → TEMPER_FORMAT → cli.format config → TTY default
      --embed-threads <N>  ONNX intra-op threads for embedding. `0` = let ONNX Runtime decide. Default: this machine's performance-core count (NOT its total core count — efficiency cores measurably slow the batch down). Precedence: --embed-threads → TEMPER_ONNX_INTRA_THREADS → detected → 1
      --color <COLOR>      Color output: auto | always | never (default: auto). Precedence: --color → TEMPER_COLOR → cli.color config → NO_COLOR → auto
  -h, --help               Print help
```

### `temper team attach`

```text
Attach a child team under a parent team (a manager of both; a temper-system or gating-team parent, or a child with an auto-join or the gating team in or below it, a system admin)

Every member of the child, at any role, then reads everything attached to the parent and its ancestors; the output lists them as `ancestors` (the child's whole live chain after the link). Also the way to give a team a second parent.

Usage: temper team attach [OPTIONS] <PARENT> <CHILD>

Arguments:
  <PARENT>
          Parent team slug (optionally `+`-prefixed) or UUID

  <CHILD>
          Child team slug (optionally `+`-prefixed), decorated ref, or UUID

Options:
      --vault <VAULT>
          Path to vault (overrides TEMPER_VAULT and auto-detection)

      --format <FORMAT>
          Output format: json | toon (default: toon on a TTY, json otherwise). Precedence: --format → TEMPER_FORMAT → cli.format config → TTY default

      --embed-threads <N>
          ONNX intra-op threads for embedding. `0` = let ONNX Runtime decide. Default: this machine's performance-core count (NOT its total core count — efficiency cores measurably slow the batch down). Precedence: --embed-threads → TEMPER_ONNX_INTRA_THREADS → detected → 1

      --color <COLOR>
          Color output: auto | always | never (default: auto). Precedence: --color → TEMPER_COLOR → cli.color config → NO_COLOR → auto

  -h, --help
          Print help (see a summary with '-h')
```

### `temper team reparent`

```text
Move a team from one parent to another, in one act (a manager of the team and of `--to`)

The new link is written before the old one is cut, so a refusal leaves the team where it was. The output lists `ancestors`, the team's live chain after the move.

Usage: temper team reparent [OPTIONS] --from <FROM> --to <TO> <TEAM>

Arguments:
  <TEAM>
          The team being moved: slug (optionally `+`-prefixed) or UUID

Options:
      --from <FROM>
          The current parent to leave (a team ref)

      --vault <VAULT>
          Path to vault (overrides TEMPER_VAULT and auto-detection)

      --format <FORMAT>
          Output format: json | toon (default: toon on a TTY, json otherwise). Precedence: --format → TEMPER_FORMAT → cli.format config → TTY default

      --to <TO>
          The new parent to join (a team ref)

      --embed-threads <N>
          ONNX intra-op threads for embedding. `0` = let ONNX Runtime decide. Default: this machine's performance-core count (NOT its total core count — efficiency cores measurably slow the batch down). Precedence: --embed-threads → TEMPER_ONNX_INTRA_THREADS → detected → 1

      --color <COLOR>
          Color output: auto | always | never (default: auto). Precedence: --color → TEMPER_COLOR → cli.color config → NO_COLOR → auto

  -h, --help
          Print help (see a summary with '-h')
```

### `temper team restore`

```text
Restore a soft-deleted team (its owner, or a system admin)

Every preserved membership, parent link and child's reach revives; the output lists them, and `ancestors`, every live team above the restored one. A system admin's act alone when the team keeps a link to temper-system or the gating team, or when it or any team below it, live or deleted, is an auto-join or the gating team. A deleted team is no longer in `team list`, so name it by UUID.

Usage: temper team restore [OPTIONS] <TEAM>

Arguments:
  <TEAM>
          Team UUID (or decorated ref)

Options:
      --vault <VAULT>
          Path to vault (overrides TEMPER_VAULT and auto-detection)

      --format <FORMAT>
          Output format: json | toon (default: toon on a TTY, json otherwise). Precedence: --format → TEMPER_FORMAT → cli.format config → TTY default

      --embed-threads <N>
          ONNX intra-op threads for embedding. `0` = let ONNX Runtime decide. Default: this machine's performance-core count (NOT its total core count — efficiency cores measurably slow the batch down). Precedence: --embed-threads → TEMPER_ONNX_INTRA_THREADS → detected → 1

      --color <COLOR>
          Color output: auto | always | never (default: auto). Precedence: --color → TEMPER_COLOR → cli.color config → NO_COLOR → auto

  -h, --help
          Print help (see a summary with '-h')
```

### `temper team rename-slug`

```text
Change a team's slug (an owner or maintainer; temper-system and the gating team, a system admin)

Addresses change: `+old-slug/...` refs stop resolving, and refs written in content are not rewritten. UUID refs are unaffected.

Usage: temper team rename-slug [OPTIONS] <TEAM> <SLUG>

Arguments:
  <TEAM>
          Team slug (optionally `+`-prefixed) or UUID

  <SLUG>
          The new slug: a-z, 0-9 and `-`, starting with a letter or digit, at most 63 characters

Options:
      --vault <VAULT>
          Path to vault (overrides TEMPER_VAULT and auto-detection)

      --format <FORMAT>
          Output format: json | toon (default: toon on a TTY, json otherwise). Precedence: --format → TEMPER_FORMAT → cli.format config → TTY default

      --embed-threads <N>
          ONNX intra-op threads for embedding. `0` = let ONNX Runtime decide. Default: this machine's performance-core count (NOT its total core count — efficiency cores measurably slow the batch down). Precedence: --embed-threads → TEMPER_ONNX_INTRA_THREADS → detected → 1

      --color <COLOR>
          Color output: auto | always | never (default: auto). Precedence: --color → TEMPER_COLOR → cli.color config → NO_COLOR → auto

  -h, --help
          Print help (see a summary with '-h')
```

### `temper team detach`

```text
Detach a child team from a parent team (a manager of the parent or of the child, or a system admin)

The child keeps its roster and only this link is cut; its members stop reaching what is attached to the parent. `temper team show <parent>` lists the parent's children.

Usage: temper team detach [OPTIONS] <PARENT> <CHILD>

Arguments:
  <PARENT>
          Parent team slug (optionally `+`-prefixed) or UUID

  <CHILD>
          Child team slug, decorated ref, or UUID

Options:
      --vault <VAULT>
          Path to vault (overrides TEMPER_VAULT and auto-detection)

      --format <FORMAT>
          Output format: json | toon (default: toon on a TTY, json otherwise). Precedence: --format → TEMPER_FORMAT → cli.format config → TTY default

      --embed-threads <N>
          ONNX intra-op threads for embedding. `0` = let ONNX Runtime decide. Default: this machine's performance-core count (NOT its total core count — efficiency cores measurably slow the batch down). Precedence: --embed-threads → TEMPER_ONNX_INTRA_THREADS → detected → 1

      --color <COLOR>
          Color output: auto | always | never (default: auto). Precedence: --color → TEMPER_COLOR → cli.color config → NO_COLOR → auto

  -h, --help
          Print help (see a summary with '-h')
```

### `temper team remove-member`

```text
Remove a member from a team (owner/maintainer)

Usage: temper team remove-member [OPTIONS] <TEAM> <PROFILE>

Arguments:
  <TEAM>     Team slug or UUID
  <PROFILE>  Member profile UUID

Options:
      --vault <VAULT>      Path to vault (overrides TEMPER_VAULT and auto-detection)
      --format <FORMAT>    Output format: json | toon (default: toon on a TTY, json otherwise). Precedence: --format → TEMPER_FORMAT → cli.format config → TTY default
      --embed-threads <N>  ONNX intra-op threads for embedding. `0` = let ONNX Runtime decide. Default: this machine's performance-core count (NOT its total core count — efficiency cores measurably slow the batch down). Precedence: --embed-threads → TEMPER_ONNX_INTRA_THREADS → detected → 1
      --color <COLOR>      Color output: auto | always | never (default: auto). Precedence: --color → TEMPER_COLOR → cli.color config → NO_COLOR → auto
  -h, --help               Print help
```

### `temper team set-role`

```text
Change a member's role (a team owner/maintainer or a system admin; personal team: its owner)

You can set roles up to your own and act on members up to your own rank; only a system admin changes an owner other than yourself. To hand a team over, grant `owner` to another member, then lower your own role.

Usage: temper team set-role [OPTIONS] --role <ROLE> <TEAM> <PROFILE>

Arguments:
  <TEAM>
          Team slug or UUID

  <PROFILE>
          Member profile UUID

Options:
      --role <ROLE>
          New role: owner | maintainer | member | watcher (up to your own role)

      --vault <VAULT>
          Path to vault (overrides TEMPER_VAULT and auto-detection)

      --format <FORMAT>
          Output format: json | toon (default: toon on a TTY, json otherwise). Precedence: --format → TEMPER_FORMAT → cli.format config → TTY default

      --embed-threads <N>
          ONNX intra-op threads for embedding. `0` = let ONNX Runtime decide. Default: this machine's performance-core count (NOT its total core count — efficiency cores measurably slow the batch down). Precedence: --embed-threads → TEMPER_ONNX_INTRA_THREADS → detected → 1

      --color <COLOR>
          Color output: auto | always | never (default: auto). Precedence: --color → TEMPER_COLOR → cli.color config → NO_COLOR → auto

  -h, --help
          Print help (see a summary with '-h')
```

### `temper team update`

```text
Update a team's metadata (owner/maintainer)

Usage: temper team update [OPTIONS] <TEAM>

Arguments:
  <TEAM>  Team slug (optionally `+`-prefixed) or UUID

Options:
      --name <NAME>                New display name
      --vault <VAULT>              Path to vault (overrides TEMPER_VAULT and auto-detection)
      --description <DESCRIPTION>  New description
      --format <FORMAT>            Output format: json | toon (default: toon on a TTY, json otherwise). Precedence: --format → TEMPER_FORMAT → cli.format config → TTY default
      --embed-threads <N>          ONNX intra-op threads for embedding. `0` = let ONNX Runtime decide. Default: this machine's performance-core count (NOT its total core count — efficiency cores measurably slow the batch down). Precedence: --embed-threads → TEMPER_ONNX_INTRA_THREADS → detected → 1
      --color <COLOR>              Color output: auto | always | never (default: auto). Precedence: --color → TEMPER_COLOR → cli.color config → NO_COLOR → auto
  -h, --help                       Print help
```

### `temper team delete`

```text
Soft-delete a team (its owner, or a system admin)

Usage: temper team delete [OPTIONS] <TEAM>

Arguments:
  <TEAM>  Team slug (optionally `+`-prefixed) or UUID

Options:
      --vault <VAULT>      Path to vault (overrides TEMPER_VAULT and auto-detection)
      --format <FORMAT>    Output format: json | toon (default: toon on a TTY, json otherwise). Precedence: --format → TEMPER_FORMAT → cli.format config → TTY default
      --embed-threads <N>  ONNX intra-op threads for embedding. `0` = let ONNX Runtime decide. Default: this machine's performance-core count (NOT its total core count — efficiency cores measurably slow the batch down). Precedence: --embed-threads → TEMPER_ONNX_INTRA_THREADS → detected → 1
      --color <COLOR>      Color output: auto | always | never (default: auto). Precedence: --color → TEMPER_COLOR → cli.color config → NO_COLOR → auto
  -h, --help               Print help
```

### `temper team reassign`

```text
Bulk-reassign a departing member's team-scoped resources (offboarding).

Reassigns every resource owned by `--from` (a current or former member) and homed in a context this team owns, over to `--to` (who must be a team member). Contexts merely shared to the team are out of scope. Owner/maintainer only. Sends a `POST /api/teams/{id}/reassign` request via `temper-client`.

Usage: temper team reassign [OPTIONS] --from <FROM> --to <TO> <TEAM>

Arguments:
  <TEAM>
          Team slug (optionally `+`-prefixed) or UUID

Options:
      --from <FROM>
          Current owner (departing) profile UUID

      --vault <VAULT>
          Path to vault (overrides TEMPER_VAULT and auto-detection)

      --format <FORMAT>
          Output format: json | toon (default: toon on a TTY, json otherwise). Precedence: --format → TEMPER_FORMAT → cli.format config → TTY default

      --to <TO>
          New owner profile UUID (must be a team member)

      --embed-threads <N>
          ONNX intra-op threads for embedding. `0` = let ONNX Runtime decide. Default: this machine's performance-core count (NOT its total core count — efficiency cores measurably slow the batch down). Precedence: --embed-threads → TEMPER_ONNX_INTRA_THREADS → detected → 1

      --color <COLOR>
          Color output: auto | always | never (default: auto). Precedence: --color → TEMPER_COLOR → cli.color config → NO_COLOR → auto

  -h, --help
          Print help (see a summary with '-h')
```

### `temper team create`

```text
Create a team (you become its owner)

Usage: temper team create [OPTIONS] <SLUG>

Arguments:
  <SLUG>  Globally-unique team slug

Options:
      --name <NAME>
          Display name (defaults to the slug)
      --vault <VAULT>
          Path to vault (overrides TEMPER_VAULT and auto-detection)
      --format <FORMAT>
          Output format: json | toon (default: toon on a TTY, json otherwise). Precedence: --format → TEMPER_FORMAT → cli.format config → TTY default
      --parent <PARENT>
          Parent team ref (`+slug`, bare slug, or UUID); creates a child team. Repeat for several parents (at most 16); you must manage each
      --auto-join-role <AUTO_JOIN_ROLE>
          Auto-join role for an "everyone" pool (admin-only): member/watcher
      --embed-threads <N>
          ONNX intra-op threads for embedding. `0` = let ONNX Runtime decide. Default: this machine's performance-core count (NOT its total core count — efficiency cores measurably slow the batch down). Precedence: --embed-threads → TEMPER_ONNX_INTRA_THREADS → detected → 1
      --color <COLOR>
          Color output: auto | always | never (default: auto). Precedence: --color → TEMPER_COLOR → cli.color config → NO_COLOR → auto
  -h, --help
          Print help
```

### `temper team add-member`

```text
Add a member to a team (owner/maintainer only)

Usage: temper team add-member [OPTIONS] --role <ROLE> <TEAM> <PROFILE>

Arguments:
  <TEAM>     Team ID (UUID)
  <PROFILE>  Profile ID (UUID)

Options:
      --role <ROLE>        Role to grant: owner/maintainer/member/watcher (up to your own role)
      --vault <VAULT>      Path to vault (overrides TEMPER_VAULT and auto-detection)
      --format <FORMAT>    Output format: json | toon (default: toon on a TTY, json otherwise). Precedence: --format → TEMPER_FORMAT → cli.format config → TTY default
      --embed-threads <N>  ONNX intra-op threads for embedding. `0` = let ONNX Runtime decide. Default: this machine's performance-core count (NOT its total core count — efficiency cores measurably slow the batch down). Precedence: --embed-threads → TEMPER_ONNX_INTRA_THREADS → detected → 1
      --color <COLOR>      Color output: auto | always | never (default: auto). Precedence: --color → TEMPER_COLOR → cli.color config → NO_COLOR → auto
  -h, --help               Print help
```

### `temper team list`

```text
List the teams you are a member of

Usage: temper team list [OPTIONS]

Options:
      --vault <VAULT>      Path to vault (overrides TEMPER_VAULT and auto-detection)
      --format <FORMAT>    Output format: json | toon (default: toon on a TTY, json otherwise). Precedence: --format → TEMPER_FORMAT → cli.format config → TTY default
      --embed-threads <N>  ONNX intra-op threads for embedding. `0` = let ONNX Runtime decide. Default: this machine's performance-core count (NOT its total core count — efficiency cores measurably slow the batch down). Precedence: --embed-threads → TEMPER_ONNX_INTRA_THREADS → detected → 1
      --color <COLOR>      Color output: auto | always | never (default: auto). Precedence: --color → TEMPER_COLOR → cli.color config → NO_COLOR → auto
  -h, --help               Print help
```

### `temper team public-within`

```text
Open a team's owned contexts to the people under one of its ancestors, close that opening, forbid it over a subtree, or list the openings you read through

For people reading, never for automation: a machine principal is refused every act here and never reads through an opening. Readers may reference the opened material from homes they author; acting on it (writing, granting, sharing, subscribing, auditing) still needs membership.

Usage: temper team public-within [OPTIONS] <COMMAND>

Commands:
  open    Open a team's owned contexts to an ancestor's subtree (a direct owner or maintainer of the team, and of the anchor or a system admin)
  close   Close an opening (a manager of either side, or a system admin)
  policy  Forbid or allow openings over a team and everything beneath it (its direct owner or a system admin; on temper-system, the instance-wide off, a system admin only)
  list    List the openings that confer on you right now: each names the team you read and the anchor you read it through
  help    Print this message or the help of the given subcommand(s)

Options:
      --vault <VAULT>
          Path to vault (overrides TEMPER_VAULT and auto-detection)

      --format <FORMAT>
          Output format: json | toon (default: toon on a TTY, json otherwise). Precedence: --format → TEMPER_FORMAT → cli.format config → TTY default

      --embed-threads <N>
          ONNX intra-op threads for embedding. `0` = let ONNX Runtime decide. Default: this machine's performance-core count (NOT its total core count — efficiency cores measurably slow the batch down). Precedence: --embed-threads → TEMPER_ONNX_INTRA_THREADS → detected → 1

      --color <COLOR>
          Color output: auto | always | never (default: auto). Precedence: --color → TEMPER_COLOR → cli.color config → NO_COLOR → auto

  -h, --help
          Print help (see a summary with '-h')
```

#### `temper team public-within open`

```text
Open a team's owned contexts to an ancestor's subtree (a direct owner or maintainer of the team, and of the anchor or a system admin)

The people on the anchor and on every live team beneath it then read every context the team owns. That audience grows as the anchor's subtree grows: every manager of a team under the anchor, identity-provider group sync and a system admin can add to it. Either side can close it at any time. An anchor whose subtree reaches everyone (temper-system, the gating team, or one holding an auto-join team) is a system admin's open.

Usage: temper team public-within open [OPTIONS] <TEAM> <ANCHOR>

Arguments:
  <TEAM>
          The team whose owned contexts are opened: slug (optionally `+`-prefixed), decorated ref, or UUID

  <ANCHOR>
          The ancestor whose subtree reads them: slug, decorated ref, or UUID

Options:
      --vault <VAULT>
          Path to vault (overrides TEMPER_VAULT and auto-detection)

      --format <FORMAT>
          Output format: json | toon (default: toon on a TTY, json otherwise). Precedence: --format → TEMPER_FORMAT → cli.format config → TTY default

      --embed-threads <N>
          ONNX intra-op threads for embedding. `0` = let ONNX Runtime decide. Default: this machine's performance-core count (NOT its total core count — efficiency cores measurably slow the batch down). Precedence: --embed-threads → TEMPER_ONNX_INTRA_THREADS → detected → 1

      --color <COLOR>
          Color output: auto | always | never (default: auto). Precedence: --color → TEMPER_COLOR → cli.color config → NO_COLOR → auto

  -h, --help
          Print help (see a summary with '-h')
```

#### `temper team public-within close`

```text
Close an opening (a manager of either side, or a system admin)

What readers already read is not recalled.

Usage: temper team public-within close [OPTIONS] <TEAM> <ANCHOR>

Arguments:
  <TEAM>
          The opened team: slug, decorated ref, or UUID

  <ANCHOR>
          The anchor: slug, decorated ref, or UUID

Options:
      --vault <VAULT>
          Path to vault (overrides TEMPER_VAULT and auto-detection)

      --format <FORMAT>
          Output format: json | toon (default: toon on a TTY, json otherwise). Precedence: --format → TEMPER_FORMAT → cli.format config → TTY default

      --embed-threads <N>
          ONNX intra-op threads for embedding. `0` = let ONNX Runtime decide. Default: this machine's performance-core count (NOT its total core count — efficiency cores measurably slow the batch down). Precedence: --embed-threads → TEMPER_ONNX_INTRA_THREADS → detected → 1

      --color <COLOR>
          Color output: auto | always | never (default: auto). Precedence: --color → TEMPER_COLOR → cli.color config → NO_COLOR → auto

  -h, --help
          Print help (see a summary with '-h')
```

#### `temper team public-within policy`

```text
Forbid or allow openings over a team and everything beneath it (its direct owner or a system admin; on temper-system, the instance-wide off, a system admin only)

A forbid covers every team beneath by every path, and nothing beneath it lifts it. Rows are kept and confer again when the forbid is cleared.

Usage: temper team public-within policy [OPTIONS] <TEAM> <SETTING>

Arguments:
  <TEAM>
          Team slug (optionally `+`-prefixed), decorated ref, or UUID

  <SETTING>
          forbid | allow
          
          [possible values: forbid, allow]

Options:
      --vault <VAULT>
          Path to vault (overrides TEMPER_VAULT and auto-detection)

      --format <FORMAT>
          Output format: json | toon (default: toon on a TTY, json otherwise). Precedence: --format → TEMPER_FORMAT → cli.format config → TTY default

      --embed-threads <N>
          ONNX intra-op threads for embedding. `0` = let ONNX Runtime decide. Default: this machine's performance-core count (NOT its total core count — efficiency cores measurably slow the batch down). Precedence: --embed-threads → TEMPER_ONNX_INTRA_THREADS → detected → 1

      --color <COLOR>
          Color output: auto | always | never (default: auto). Precedence: --color → TEMPER_COLOR → cli.color config → NO_COLOR → auto

  -h, --help
          Print help (see a summary with '-h')
```

#### `temper team public-within list`

```text
List the openings that confer on you right now: each names the team you read and the anchor you read it through

Usage: temper team public-within list [OPTIONS]

Options:
      --vault <VAULT>      Path to vault (overrides TEMPER_VAULT and auto-detection)
      --format <FORMAT>    Output format: json | toon (default: toon on a TTY, json otherwise). Precedence: --format → TEMPER_FORMAT → cli.format config → TTY default
      --embed-threads <N>  ONNX intra-op threads for embedding. `0` = let ONNX Runtime decide. Default: this machine's performance-core count (NOT its total core count — efficiency cores measurably slow the batch down). Precedence: --embed-threads → TEMPER_ONNX_INTRA_THREADS → detected → 1
      --color <COLOR>      Color output: auto | always | never (default: auto). Precedence: --color → TEMPER_COLOR → cli.color config → NO_COLOR → auto
  -h, --help               Print help
```
