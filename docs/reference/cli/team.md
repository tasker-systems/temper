<!-- GENERATED — do not edit. Emitted from the built binary's `--help` by scripts/emit-cli-reference.py; see .github/scripts/check-cli-reference-drift.sh. -->

# `temper team`

Manage team membership and access

```text
Manage team membership and access

Usage: temper team [OPTIONS] <COMMAND>

Commands:
  join           Accept a team invitation by its token
  invite         Invite an email to a team (owner/maintainer)
  decline        Decline a team invitation by its token
  invitations    List pending invitations for a team (owner/maintainer)
  uninvite       Revoke (withdraw) a pending invitation (owner/maintainer)
  show           Show a team's detail and member roster
  leave          Leave a team you are a member of (removes your membership)
  remove-member  Remove a member from a team (owner/maintainer)
  set-role       Change a member's role (owner/maintainer)
  update         Update a team's metadata (owner/maintainer)
  delete         Soft-delete a team (owner only)
  reassign       Bulk-reassign a departing member's team-scoped resources (offboarding)
  create         Create a team (you become its owner)
  add-member     Add a member to a team (owner/maintainer only)
  list           List the teams you are a member of
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
Accept a team invitation by its token

Usage: temper team join [OPTIONS] <TOKEN>

Arguments:
  <TOKEN>  Invitation token (from `temper team invite`)

Options:
      --vault <VAULT>      Path to vault (overrides TEMPER_VAULT and auto-detection)
      --format <FORMAT>    Output format: json | toon (default: toon on a TTY, json otherwise). Precedence: --format → TEMPER_FORMAT → cli.format config → TTY default
      --embed-threads <N>  ONNX intra-op threads for embedding. `0` = let ONNX Runtime decide. Default: this machine's performance-core count (NOT its total core count — efficiency cores measurably slow the batch down). Precedence: --embed-threads → TEMPER_ONNX_INTRA_THREADS → detected → 1
      --color <COLOR>      Color output: auto | always | never (default: auto). Precedence: --color → TEMPER_COLOR → cli.color config → NO_COLOR → auto
  -h, --help               Print help
```

### `temper team invite`

```text
Invite an email to a team (owner/maintainer)

Usage: temper team invite [OPTIONS] --role <ROLE> <TEAM> <EMAIL>

Arguments:
  <TEAM>   Team slug (optionally `+`-prefixed) or UUID
  <EMAIL>  Email address to invite

Options:
      --role <ROLE>        Role to grant on acceptance: maintainer | member | watcher
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
List pending invitations for a team (owner/maintainer)

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
Revoke (withdraw) a pending invitation (owner/maintainer)

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
Change a member's role (owner/maintainer)

Usage: temper team set-role [OPTIONS] --role <ROLE> <TEAM> <PROFILE>

Arguments:
  <TEAM>     Team slug or UUID
  <PROFILE>  Member profile UUID

Options:
      --role <ROLE>        New role: maintainer | member | watcher
      --vault <VAULT>      Path to vault (overrides TEMPER_VAULT and auto-detection)
      --format <FORMAT>    Output format: json | toon (default: toon on a TTY, json otherwise). Precedence: --format → TEMPER_FORMAT → cli.format config → TTY default
      --embed-threads <N>  ONNX intra-op threads for embedding. `0` = let ONNX Runtime decide. Default: this machine's performance-core count (NOT its total core count — efficiency cores measurably slow the batch down). Precedence: --embed-threads → TEMPER_ONNX_INTRA_THREADS → detected → 1
      --color <COLOR>      Color output: auto | always | never (default: auto). Precedence: --color → TEMPER_COLOR → cli.color config → NO_COLOR → auto
  -h, --help               Print help
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
Soft-delete a team (owner only)

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

Reassigns every resource owned by `--from` and homed in a context shared to this team, over to `--to` (who must be a team member). Owner/maintainer only. Sends a `POST /api/teams/{id}/reassign` request via `temper-client`.

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
          Parent team ref (`+slug` or bare slug); creates a child team
      --auto-join-role <AUTO_JOIN_ROLE>
          Auto-join role for an "everyone" pool (admin-only): owner/maintainer/member/watcher
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
      --role <ROLE>        Role to grant: owner/maintainer/member/watcher
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
