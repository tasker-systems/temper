<!-- GENERATED — do not edit. Emitted from the built binary's `--help` by scripts/emit-cli-reference.py; see .github/scripts/check-cli-reference-drift.sh. -->

# `temper admin`

Administer the instance (system settings, promote admins, review requests)

```text
Administer the instance (system settings, promote admins, review requests)

Usage: temper admin [OPTIONS] <COMMAND>

Commands:
  settings      Show system settings, or update them when any flag is provided
  promote       Promote a profile to system admin: grants principal-governance and, if needed, approved standing. Also adds an `owner` row on a team (defaults to the gating team) as a side effect
  demote        Demote a system admin — revoke its governance grant (the manual twin of `promote`)
  access        Admit, revoke, deactivate, or reactivate a principal's system access
  requests      Review pending join requests
  ledger        Read the admin ledger: who granted what, to whom, and when
  saml          SAML provisioning: generate keys + emit the consistent env bundle and SQL (operator tooling)
  machine       Register and rotate machine (client_credentials) principals
  slack         Administer Slack account links
  connection    Provision connections — temper's authed link to a remote system (GitHub, Linear)
  subscription  Manage subscriptions — a team/context/cogmap subscribes to a connection's events
  reembed       Re-embed chunks whose vectors were produced by an older model (the drain does the work)
  help          Print this message or the help of the given subcommand(s)

Options:
      --vault <VAULT>      Path to vault (overrides TEMPER_VAULT and auto-detection)
      --format <FORMAT>    Output format: json | toon (default: toon on a TTY, json otherwise). Precedence: --format → TEMPER_FORMAT → cli.format config → TTY default
      --embed-threads <N>  ONNX intra-op threads for embedding. `0` = let ONNX Runtime decide. Default: this machine's performance-core count (NOT its total core count — efficiency cores measurably slow the batch down). Precedence: --embed-threads → TEMPER_ONNX_INTRA_THREADS → detected → 1
      --color <COLOR>      Color output: auto | always | never (default: auto). Precedence: --color → TEMPER_COLOR → cli.color config → NO_COLOR → auto
  -h, --help               Print help
```

### `temper admin settings`

```text
Show system settings, or update them when any flag is provided

Usage: temper admin settings [OPTIONS]

Options:
      --gating-team <GATING_TEAM_SLUG>  Gating team slug recorded in instance settings. Ownership of this team does NOT by itself confer system-admin: `is_system_admin` reads the principal-governance grant and nothing else
      --vault <VAULT>                   Path to vault (overrides TEMPER_VAULT and auto-detection)
      --format <FORMAT>                 Output format: json | toon (default: toon on a TTY, json otherwise). Precedence: --format → TEMPER_FORMAT → cli.format config → TTY default
      --instance-name <INSTANCE_NAME>   Human-facing instance name
      --embed-threads <N>               ONNX intra-op threads for embedding. `0` = let ONNX Runtime decide. Default: this machine's performance-core count (NOT its total core count — efficiency cores measurably slow the batch down). Precedence: --embed-threads → TEMPER_ONNX_INTRA_THREADS → detected → 1
      --terms-version <TERMS_VERSION>   Terms-of-service version label
      --color <COLOR>                   Color output: auto | always | never (default: auto). Precedence: --color → TEMPER_COLOR → cli.color config → NO_COLOR → auto
      --terms-uri <TERMS_RESOURCE_URI>  Terms-of-service resource URI
  -h, --help                            Print help
```

### `temper admin promote`

```text
Promote a profile to system admin: grants principal-governance and, if needed, approved standing. Also adds an `owner` row on a team (defaults to the gating team) as a side effect

Usage: temper admin promote [OPTIONS] <PROFILE>

Arguments:
  <PROFILE>  Profile ID (UUID) to promote

Options:
      --team <TEAM>        Team ref for the side-effect `owner` row (`+slug`, bare slug, or UUID); defaults to the gating team. Not what confers admin
      --vault <VAULT>      Path to vault (overrides TEMPER_VAULT and auto-detection)
      --format <FORMAT>    Output format: json | toon (default: toon on a TTY, json otherwise). Precedence: --format → TEMPER_FORMAT → cli.format config → TTY default
      --embed-threads <N>  ONNX intra-op threads for embedding. `0` = let ONNX Runtime decide. Default: this machine's performance-core count (NOT its total core count — efficiency cores measurably slow the batch down). Precedence: --embed-threads → TEMPER_ONNX_INTRA_THREADS → detected → 1
      --color <COLOR>      Color output: auto | always | never (default: auto). Precedence: --color → TEMPER_COLOR → cli.color config → NO_COLOR → auto
  -h, --help               Print help
```

### `temper admin demote`

```text
Demote a system admin — revoke its governance grant (the manual twin of `promote`)

Usage: temper admin demote [OPTIONS] <PROFILE>

Arguments:
  <PROFILE>  Profile ID (UUID) to demote

Options:
      --vault <VAULT>      Path to vault (overrides TEMPER_VAULT and auto-detection)
      --format <FORMAT>    Output format: json | toon (default: toon on a TTY, json otherwise). Precedence: --format → TEMPER_FORMAT → cli.format config → TTY default
      --embed-threads <N>  ONNX intra-op threads for embedding. `0` = let ONNX Runtime decide. Default: this machine's performance-core count (NOT its total core count — efficiency cores measurably slow the batch down). Precedence: --embed-threads → TEMPER_ONNX_INTRA_THREADS → detected → 1
      --color <COLOR>      Color output: auto | always | never (default: auto). Precedence: --color → TEMPER_COLOR → cli.color config → NO_COLOR → auto
  -h, --help               Print help
```

### `temper admin access`

```text
Admit, revoke, deactivate, or reactivate a principal's system access

Usage: temper admin access [OPTIONS] <COMMAND>

Commands:
  approve     Admit a principal directly (legal from denied, revoked, or requested)
  revoke      Revoke a principal's admission (legal only from approved)
  deactivate  Deactivate a principal (legal from any live state)
  reactivate  Reactivate a deactivated principal, restoring its prior standing
  help        Print this message or the help of the given subcommand(s)

Options:
      --vault <VAULT>      Path to vault (overrides TEMPER_VAULT and auto-detection)
      --format <FORMAT>    Output format: json | toon (default: toon on a TTY, json otherwise). Precedence: --format → TEMPER_FORMAT → cli.format config → TTY default
      --embed-threads <N>  ONNX intra-op threads for embedding. `0` = let ONNX Runtime decide. Default: this machine's performance-core count (NOT its total core count — efficiency cores measurably slow the batch down). Precedence: --embed-threads → TEMPER_ONNX_INTRA_THREADS → detected → 1
      --color <COLOR>      Color output: auto | always | never (default: auto). Precedence: --color → TEMPER_COLOR → cli.color config → NO_COLOR → auto
  -h, --help               Print help
```

#### `temper admin access approve`

```text
Admit a principal directly (legal from denied, revoked, or requested)

Usage: temper admin access approve [OPTIONS] <PROFILE>

Arguments:
  <PROFILE>  Profile ID (UUID) to approve

Options:
      --vault <VAULT>      Path to vault (overrides TEMPER_VAULT and auto-detection)
      --format <FORMAT>    Output format: json | toon (default: toon on a TTY, json otherwise). Precedence: --format → TEMPER_FORMAT → cli.format config → TTY default
      --embed-threads <N>  ONNX intra-op threads for embedding. `0` = let ONNX Runtime decide. Default: this machine's performance-core count (NOT its total core count — efficiency cores measurably slow the batch down). Precedence: --embed-threads → TEMPER_ONNX_INTRA_THREADS → detected → 1
      --color <COLOR>      Color output: auto | always | never (default: auto). Precedence: --color → TEMPER_COLOR → cli.color config → NO_COLOR → auto
  -h, --help               Print help
```

#### `temper admin access revoke`

```text
Revoke a principal's admission (legal only from approved)

Usage: temper admin access revoke [OPTIONS] --reason <REASON> <PROFILE>

Arguments:
  <PROFILE>  Profile ID (UUID) to revoke

Options:
      --reason <REASON>    Why — recorded on the ledger; a later review's reviewer needs it
      --vault <VAULT>      Path to vault (overrides TEMPER_VAULT and auto-detection)
      --format <FORMAT>    Output format: json | toon (default: toon on a TTY, json otherwise). Precedence: --format → TEMPER_FORMAT → cli.format config → TTY default
      --embed-threads <N>  ONNX intra-op threads for embedding. `0` = let ONNX Runtime decide. Default: this machine's performance-core count (NOT its total core count — efficiency cores measurably slow the batch down). Precedence: --embed-threads → TEMPER_ONNX_INTRA_THREADS → detected → 1
      --color <COLOR>      Color output: auto | always | never (default: auto). Precedence: --color → TEMPER_COLOR → cli.color config → NO_COLOR → auto
  -h, --help               Print help
```

#### `temper admin access deactivate`

```text
Deactivate a principal (legal from any live state)

Usage: temper admin access deactivate [OPTIONS] <PROFILE>

Arguments:
  <PROFILE>  Profile ID (UUID) to deactivate

Options:
      --vault <VAULT>      Path to vault (overrides TEMPER_VAULT and auto-detection)
      --format <FORMAT>    Output format: json | toon (default: toon on a TTY, json otherwise). Precedence: --format → TEMPER_FORMAT → cli.format config → TTY default
      --embed-threads <N>  ONNX intra-op threads for embedding. `0` = let ONNX Runtime decide. Default: this machine's performance-core count (NOT its total core count — efficiency cores measurably slow the batch down). Precedence: --embed-threads → TEMPER_ONNX_INTRA_THREADS → detected → 1
      --color <COLOR>      Color output: auto | always | never (default: auto). Precedence: --color → TEMPER_COLOR → cli.color config → NO_COLOR → auto
  -h, --help               Print help
```

#### `temper admin access reactivate`

```text
Reactivate a deactivated principal, restoring its prior standing

Usage: temper admin access reactivate [OPTIONS] <PROFILE>

Arguments:
  <PROFILE>  Profile ID (UUID) to reactivate

Options:
      --vault <VAULT>      Path to vault (overrides TEMPER_VAULT and auto-detection)
      --format <FORMAT>    Output format: json | toon (default: toon on a TTY, json otherwise). Precedence: --format → TEMPER_FORMAT → cli.format config → TTY default
      --embed-threads <N>  ONNX intra-op threads for embedding. `0` = let ONNX Runtime decide. Default: this machine's performance-core count (NOT its total core count — efficiency cores measurably slow the batch down). Precedence: --embed-threads → TEMPER_ONNX_INTRA_THREADS → detected → 1
      --color <COLOR>      Color output: auto | always | never (default: auto). Precedence: --color → TEMPER_COLOR → cli.color config → NO_COLOR → auto
  -h, --help               Print help
```

### `temper admin requests`

```text
Review pending join requests

Usage: temper admin requests [OPTIONS] <COMMAND>

Commands:
  list    List pending join requests for the gating team
  review  Approve or reject a join request
  help    Print this message or the help of the given subcommand(s)

Options:
      --vault <VAULT>      Path to vault (overrides TEMPER_VAULT and auto-detection)
      --format <FORMAT>    Output format: json | toon (default: toon on a TTY, json otherwise). Precedence: --format → TEMPER_FORMAT → cli.format config → TTY default
      --embed-threads <N>  ONNX intra-op threads for embedding. `0` = let ONNX Runtime decide. Default: this machine's performance-core count (NOT its total core count — efficiency cores measurably slow the batch down). Precedence: --embed-threads → TEMPER_ONNX_INTRA_THREADS → detected → 1
      --color <COLOR>      Color output: auto | always | never (default: auto). Precedence: --color → TEMPER_COLOR → cli.color config → NO_COLOR → auto
  -h, --help               Print help
```

#### `temper admin requests list`

```text
List pending join requests for the gating team

Usage: temper admin requests list [OPTIONS]

Options:
      --vault <VAULT>      Path to vault (overrides TEMPER_VAULT and auto-detection)
      --format <FORMAT>    Output format: json | toon (default: toon on a TTY, json otherwise). Precedence: --format → TEMPER_FORMAT → cli.format config → TTY default
      --embed-threads <N>  ONNX intra-op threads for embedding. `0` = let ONNX Runtime decide. Default: this machine's performance-core count (NOT its total core count — efficiency cores measurably slow the batch down). Precedence: --embed-threads → TEMPER_ONNX_INTRA_THREADS → detected → 1
      --color <COLOR>      Color output: auto | always | never (default: auto). Precedence: --color → TEMPER_COLOR → cli.color config → NO_COLOR → auto
  -h, --help               Print help
```

#### `temper admin requests review`

```text
Approve or reject a join request

Usage: temper admin requests review [OPTIONS] <ID>

Arguments:
  <ID>  Join request ID (UUID)

Options:
      --approve            Approve the request
      --vault <VAULT>      Path to vault (overrides TEMPER_VAULT and auto-detection)
      --format <FORMAT>    Output format: json | toon (default: toon on a TTY, json otherwise). Precedence: --format → TEMPER_FORMAT → cli.format config → TTY default
      --reject             Reject the request
      --embed-threads <N>  ONNX intra-op threads for embedding. `0` = let ONNX Runtime decide. Default: this machine's performance-core count (NOT its total core count — efficiency cores measurably slow the batch down). Precedence: --embed-threads → TEMPER_ONNX_INTRA_THREADS → detected → 1
      --note <NOTE>        Optional decision note
      --color <COLOR>      Color output: auto | always | never (default: auto). Precedence: --color → TEMPER_COLOR → cli.color config → NO_COLOR → auto
  -h, --help               Print help
```

### `temper admin ledger`

```text
Read the admin ledger: who granted what, to whom, and when

Exactly one axis. `--subject` asks what was done TO a thing; `--actor` asks what a principal DID. A refusal is a 404 by design — on this surface "you may not read that" and "there is nothing there" are deliberately indistinguishable.

Usage: temper admin ledger [OPTIONS]

Options:
      --subject <SUBJECT>
          Subject axis: `<kind>:<uuid>`, e.g. `kb_resources:0199c3f1-...`

      --vault <VAULT>
          Path to vault (overrides TEMPER_VAULT and auto-detection)

      --actor <ACTOR>
          Actor axis: a profile UUID. Your own acts are always readable; another's is an audit and requires admin

      --format <FORMAT>
          Output format: json | toon (default: toon on a TTY, json otherwise). Precedence: --format → TEMPER_FORMAT → cli.format config → TTY default

      --embed-threads <N>
          ONNX intra-op threads for embedding. `0` = let ONNX Runtime decide. Default: this machine's performance-core count (NOT its total core count — efficiency cores measurably slow the batch down). Precedence: --embed-threads → TEMPER_ONNX_INTRA_THREADS → detected → 1

      --limit <LIMIT>
          Page size (server clamps to 200)

      --color <COLOR>
          Color output: auto | always | never (default: auto). Precedence: --color → TEMPER_COLOR → cli.color config → NO_COLOR → auto

      --offset <OFFSET>
          

  -h, --help
          Print help (see a summary with '-h')
```

### `temper admin saml`

```text
SAML provisioning: generate keys + emit the consistent env bundle and SQL (operator tooling)

Usage: temper admin saml [OPTIONS] <COMMAND>

Commands:
  provision  Generate the AS signing key + reconcile secret and emit the env bundle + kb_saml_idp SQL
  map-group  Emit a kb_saml_group_mappings INSERT for `group → (+team, role)` (run AFTER teams exist)
  verify     Verify a provisioned instance: AS metadata reachable, caller is a system admin (governance grant + approved standing), and — with --db — one active kb_saml_idp row
  help       Print this message or the help of the given subcommand(s)

Options:
      --vault <VAULT>      Path to vault (overrides TEMPER_VAULT and auto-detection)
      --format <FORMAT>    Output format: json | toon (default: toon on a TTY, json otherwise). Precedence: --format → TEMPER_FORMAT → cli.format config → TTY default
      --embed-threads <N>  ONNX intra-op threads for embedding. `0` = let ONNX Runtime decide. Default: this machine's performance-core count (NOT its total core count — efficiency cores measurably slow the batch down). Precedence: --embed-threads → TEMPER_ONNX_INTRA_THREADS → detected → 1
      --color <COLOR>      Color output: auto | always | never (default: auto). Precedence: --color → TEMPER_COLOR → cli.color config → NO_COLOR → auto
  -h, --help               Print help
```

#### `temper admin saml provision`

```text
Generate the AS signing key + reconcile secret and emit the env bundle + kb_saml_idp SQL.

Interactive by default; pass --no-interactive with the flags below for scripted runs. Emits to stdout unless --env-out / --sql-out are given; --apply runs the SQL via psql.

Usage: temper admin saml provision [OPTIONS]

Options:
      --no-interactive
          

      --vault <VAULT>
          Path to vault (overrides TEMPER_VAULT and auto-detection)

      --format <FORMAT>
          Output format: json | toon (default: toon on a TTY, json otherwise). Precedence: --format → TEMPER_FORMAT → cli.format config → TTY default

      --instance-url <INSTANCE_URL>
          

      --api-origin <API_ORIGIN>
          API origin the AS calls for reconcile (defaults to --instance-url)

      --embed-threads <N>
          ONNX intra-op threads for embedding. `0` = let ONNX Runtime decide. Default: this machine's performance-core count (NOT its total core count — efficiency cores measurably slow the batch down). Precedence: --embed-threads → TEMPER_ONNX_INTRA_THREADS → detected → 1

      --color <COLOR>
          Color output: auto | always | never (default: auto). Precedence: --color → TEMPER_COLOR → cli.color config → NO_COLOR → auto

      --idp-key <IDP_KEY>
          

      --idp-cert-file <IDP_CERT_FILE>
          Path to the IdP signing certificate (PEM)

      --idp-sso-url <IDP_SSO_URL>
          

      --idp-entity-id <IDP_ENTITY_ID>
          

      --nameid-format <NAMEID_FORMAT>
          [default: urn:oasis:names:tc:SAML:2.0:nameid-format:persistent]

      --email-attr <EMAIL_ATTR>
          [default: email]

      --stable-id-attr <STABLE_ID_ATTR>
          [default: uid]

      --groups-attr <GROUPS_ATTR>
          Assertion attribute carrying the group list (omit for authn-only)

      --kid <KID>
          Override the signing key id (default `as-<YYYY-MM>`)

      --client <CLIENTS>
          Repeatable `client_id=redirect_uri` for AS_CLIENTS (e.g. `temper-cli=https://host/api/auth/cli-callback`)

      --env-out <ENV_OUT>
          Write the env bundle here instead of stdout (chmod 0600 — contains the private key)

      --sql-out <SQL_OUT>
          Write the SQL here instead of stdout

      --apply
          Run the kb_saml_idp SQL against $DATABASE_URL via psql (default: emit only)

  -h, --help
          Print help (see a summary with '-h')
```

#### `temper admin saml map-group`

```text
Emit a kb_saml_group_mappings INSERT for `group → (+team, role)` (run AFTER teams exist)

Usage: temper admin saml map-group [OPTIONS] --idp-key <IDP_KEY> [GROUP] [TEAM]

Arguments:
  [GROUP]  The IdP-asserted group value. Required unless `--from-seen`
  [TEAM]   Team to map into: a slug (optionally `+`-prefixed) or a UUID. Required unless `--from-seen`

Options:
      --idp-key <IDP_KEY>  
      --vault <VAULT>      Path to vault (overrides TEMPER_VAULT and auto-detection)
      --format <FORMAT>    Output format: json | toon (default: toon on a TTY, json otherwise). Precedence: --format → TEMPER_FORMAT → cli.format config → TTY default
      --role <ROLE>        [default: member]
      --embed-threads <N>  ONNX intra-op threads for embedding. `0` = let ONNX Runtime decide. Default: this machine's performance-core count (NOT its total core count — efficiency cores measurably slow the batch down). Precedence: --embed-threads → TEMPER_ONNX_INTRA_THREADS → detected → 1
      --from-seen          Instead of emitting a mapping, list groups the IdP has actually asserted (reads kb_saml_seen_groups via psql; needs DATABASE_URL)
      --apply              Run the INSERT against $DATABASE_URL via psql (default: emit only)
      --color <COLOR>      Color output: auto | always | never (default: auto). Precedence: --color → TEMPER_COLOR → cli.color config → NO_COLOR → auto
  -h, --help               Print help
```

#### `temper admin saml verify`

```text
Verify a provisioned instance: AS metadata reachable, caller is a system admin (governance grant + approved standing), and — with --db — one active kb_saml_idp row

Usage: temper admin saml verify [OPTIONS] --instance-url <INSTANCE_URL>

Options:
      --instance-url <INSTANCE_URL>  Instance base URL to probe (e.g. <https://temper.acme.com>)
      --vault <VAULT>                Path to vault (overrides TEMPER_VAULT and auto-detection)
      --db                           Also check kb_saml_idp via psql (needs DATABASE_URL)
      --format <FORMAT>              Output format: json | toon (default: toon on a TTY, json otherwise). Precedence: --format → TEMPER_FORMAT → cli.format config → TTY default
      --embed-threads <N>            ONNX intra-op threads for embedding. `0` = let ONNX Runtime decide. Default: this machine's performance-core count (NOT its total core count — efficiency cores measurably slow the batch down). Precedence: --embed-threads → TEMPER_ONNX_INTRA_THREADS → detected → 1
      --color <COLOR>                Color output: auto | always | never (default: auto). Precedence: --color → TEMPER_COLOR → cli.color config → NO_COLOR → auto
  -h, --help                         Print help
```

### `temper admin machine`

```text
Register and rotate machine (client_credentials) principals

Usage: temper admin machine [OPTIONS] <COMMAND>

Commands:
  provision      Register a machine principal: creates its agent profile, emitters, gating-team membership, and the reach you specify. Run this BEFORE the machine's first call
  rebind         Point a fresh client id at an existing agent profile, preserving its authorship history. Revokes the old client unless --no-revoke-old
  issue          Issue a temper-minted machine credential (client_credentials on temper's own AS). temper mints the client id and a secret; the secret is printed once
  rotate-secret  Rotate a temper-issued secret. The previous secret stays valid for a grace window
  list           List registered machine clients
  show           Show one machine client
  revoke         Revoke a machine client. Denies authentication; grants and memberships survive
  help           Print this message or the help of the given subcommand(s)

Options:
      --vault <VAULT>      Path to vault (overrides TEMPER_VAULT and auto-detection)
      --format <FORMAT>    Output format: json | toon (default: toon on a TTY, json otherwise). Precedence: --format → TEMPER_FORMAT → cli.format config → TTY default
      --embed-threads <N>  ONNX intra-op threads for embedding. `0` = let ONNX Runtime decide. Default: this machine's performance-core count (NOT its total core count — efficiency cores measurably slow the batch down). Precedence: --embed-threads → TEMPER_ONNX_INTRA_THREADS → detected → 1
      --color <COLOR>      Color output: auto | always | never (default: auto). Precedence: --color → TEMPER_COLOR → cli.color config → NO_COLOR → auto
  -h, --help               Print help
```

#### `temper admin machine provision`

```text
Register a machine principal: creates its agent profile, emitters, gating-team membership, and the reach you specify. Run this BEFORE the machine's first call

Usage: temper admin machine provision [OPTIONS] --client-id <CLIENT_ID> --label <LABEL>

Options:
      --client-id <CLIENT_ID>    The IdP client id (Auth0 M2M application client id)
      --vault <VAULT>            Path to vault (overrides TEMPER_VAULT and auto-detection)
      --format <FORMAT>          Output format: json | toon (default: toon on a TTY, json otherwise). Precedence: --format → TEMPER_FORMAT → cli.format config → TTY default
      --label <LABEL>            Human-facing label
      --embed-threads <N>        ONNX intra-op threads for embedding. `0` = let ONNX Runtime decide. Default: this machine's performance-core count (NOT its total core count — efficiency cores measurably slow the batch down). Precedence: --embed-threads → TEMPER_ONNX_INTRA_THREADS → detected → 1
      --owner-team <OWNER_TEAM>  Team recorded as this machine's OWNER. Not its reach
      --color <COLOR>            Color output: auto | always | never (default: auto). Precedence: --color → TEMPER_COLOR → cli.color config → NO_COLOR → auto
      --team <TEAMS>             Team to enroll in, as `<ref>` or `<ref>:<role>` (`member` by default, and the highest a machine may hold — `maintainer`/`owner` are refused). Repeatable. Reach is plural and never inferred from --owner-team
      --cogmap <COGMAPS>         Cogmap to grant, as `<ref>` or `<ref>:ro` (defaults to read+write). Repeatable
  -h, --help                     Print help
```

#### `temper admin machine rebind`

```text
Point a fresh client id at an existing agent profile, preserving its authorship history. Revokes the old client unless --no-revoke-old

Usage: temper admin machine rebind [OPTIONS] --client-id <CLIENT_ID> --label <LABEL> <FROM>

Arguments:
  <FROM>  The machine client being rotated away from (its `id`, from `list`)

Options:
      --client-id <CLIENT_ID>  The new IdP client id
      --vault <VAULT>          Path to vault (overrides TEMPER_VAULT and auto-detection)
      --format <FORMAT>        Output format: json | toon (default: toon on a TTY, json otherwise). Precedence: --format → TEMPER_FORMAT → cli.format config → TTY default
      --label <LABEL>          Label for the new registration
      --embed-threads <N>      ONNX intra-op threads for embedding. `0` = let ONNX Runtime decide. Default: this machine's performance-core count (NOT its total core count — efficiency cores measurably slow the batch down). Precedence: --embed-threads → TEMPER_ONNX_INTRA_THREADS → detected → 1
      --no-revoke-old          Leave both credentials live for an overlap window
      --color <COLOR>          Color output: auto | always | never (default: auto). Precedence: --color → TEMPER_COLOR → cli.color config → NO_COLOR → auto
  -h, --help                   Print help
```

#### `temper admin machine issue`

```text
Issue a temper-minted machine credential (client_credentials on temper's own AS). temper mints the client id and a secret; the secret is printed once

Usage: temper admin machine issue [OPTIONS] --label <LABEL>

Options:
      --label <LABEL>            Human-facing label
      --vault <VAULT>            Path to vault (overrides TEMPER_VAULT and auto-detection)
      --format <FORMAT>          Output format: json | toon (default: toon on a TTY, json otherwise). Precedence: --format → TEMPER_FORMAT → cli.format config → TTY default
      --owner-team <OWNER_TEAM>  Team recorded as this machine's OWNER. Not its reach
      --embed-threads <N>        ONNX intra-op threads for embedding. `0` = let ONNX Runtime decide. Default: this machine's performance-core count (NOT its total core count — efficiency cores measurably slow the batch down). Precedence: --embed-threads → TEMPER_ONNX_INTRA_THREADS → detected → 1
      --team <TEAMS>             Team to enroll in, as `<ref>` or `<ref>:<role>` (`member` by default, and the highest a machine may hold — `maintainer`/`owner` are refused). Repeatable
      --cogmap <COGMAPS>         Cogmap to grant, as `<ref>` or `<ref>:ro` (defaults to read+write). Repeatable
      --color <COLOR>            Color output: auto | always | never (default: auto). Precedence: --color → TEMPER_COLOR → cli.color config → NO_COLOR → auto
  -h, --help                     Print help
```

#### `temper admin machine rotate-secret`

```text
Rotate a temper-issued secret. The previous secret stays valid for a grace window

Usage: temper admin machine rotate-secret [OPTIONS] <ID>

Arguments:
  <ID>  The machine client to rotate (its `id`, from `list`)

Options:
      --grace <GRACE_SECONDS>  Seconds the previous secret stays valid after rotation (default 86400 = 24h) [default: 86400]
      --vault <VAULT>          Path to vault (overrides TEMPER_VAULT and auto-detection)
      --format <FORMAT>        Output format: json | toon (default: toon on a TTY, json otherwise). Precedence: --format → TEMPER_FORMAT → cli.format config → TTY default
      --embed-threads <N>      ONNX intra-op threads for embedding. `0` = let ONNX Runtime decide. Default: this machine's performance-core count (NOT its total core count — efficiency cores measurably slow the batch down). Precedence: --embed-threads → TEMPER_ONNX_INTRA_THREADS → detected → 1
      --color <COLOR>          Color output: auto | always | never (default: auto). Precedence: --color → TEMPER_COLOR → cli.color config → NO_COLOR → auto
  -h, --help                   Print help
```

#### `temper admin machine list`

```text
List registered machine clients

Usage: temper admin machine list [OPTIONS]

Options:
      --include-revoked    Include revoked clients
      --vault <VAULT>      Path to vault (overrides TEMPER_VAULT and auto-detection)
      --format <FORMAT>    Output format: json | toon (default: toon on a TTY, json otherwise). Precedence: --format → TEMPER_FORMAT → cli.format config → TTY default
      --embed-threads <N>  ONNX intra-op threads for embedding. `0` = let ONNX Runtime decide. Default: this machine's performance-core count (NOT its total core count — efficiency cores measurably slow the batch down). Precedence: --embed-threads → TEMPER_ONNX_INTRA_THREADS → detected → 1
      --color <COLOR>      Color output: auto | always | never (default: auto). Precedence: --color → TEMPER_COLOR → cli.color config → NO_COLOR → auto
  -h, --help               Print help
```

#### `temper admin machine show`

```text
Show one machine client

Usage: temper admin machine show [OPTIONS] <ID>

Arguments:
  <ID>  

Options:
      --vault <VAULT>      Path to vault (overrides TEMPER_VAULT and auto-detection)
      --format <FORMAT>    Output format: json | toon (default: toon on a TTY, json otherwise). Precedence: --format → TEMPER_FORMAT → cli.format config → TTY default
      --embed-threads <N>  ONNX intra-op threads for embedding. `0` = let ONNX Runtime decide. Default: this machine's performance-core count (NOT its total core count — efficiency cores measurably slow the batch down). Precedence: --embed-threads → TEMPER_ONNX_INTRA_THREADS → detected → 1
      --color <COLOR>      Color output: auto | always | never (default: auto). Precedence: --color → TEMPER_COLOR → cli.color config → NO_COLOR → auto
  -h, --help               Print help
```

#### `temper admin machine revoke`

```text
Revoke a machine client. Denies authentication; grants and memberships survive

Usage: temper admin machine revoke [OPTIONS] <ID>

Arguments:
  <ID>  

Options:
      --vault <VAULT>      Path to vault (overrides TEMPER_VAULT and auto-detection)
      --format <FORMAT>    Output format: json | toon (default: toon on a TTY, json otherwise). Precedence: --format → TEMPER_FORMAT → cli.format config → TTY default
      --embed-threads <N>  ONNX intra-op threads for embedding. `0` = let ONNX Runtime decide. Default: this machine's performance-core count (NOT its total core count — efficiency cores measurably slow the batch down). Precedence: --embed-threads → TEMPER_ONNX_INTRA_THREADS → detected → 1
      --color <COLOR>      Color output: auto | always | never (default: auto). Precedence: --color → TEMPER_COLOR → cli.color config → NO_COLOR → auto
  -h, --help               Print help
```

### `temper admin slack`

```text
Administer Slack account links

Usage: temper admin slack [OPTIONS] <COMMAND>

Commands:
  disconnect  Disconnect a Slack principal from its temper profile. Idempotent
  help        Print this message or the help of the given subcommand(s)

Options:
      --vault <VAULT>      Path to vault (overrides TEMPER_VAULT and auto-detection)
      --format <FORMAT>    Output format: json | toon (default: toon on a TTY, json otherwise). Precedence: --format → TEMPER_FORMAT → cli.format config → TTY default
      --embed-threads <N>  ONNX intra-op threads for embedding. `0` = let ONNX Runtime decide. Default: this machine's performance-core count (NOT its total core count — efficiency cores measurably slow the batch down). Precedence: --embed-threads → TEMPER_ONNX_INTRA_THREADS → detected → 1
      --color <COLOR>      Color output: auto | always | never (default: auto). Precedence: --color → TEMPER_COLOR → cli.color config → NO_COLOR → auto
  -h, --help               Print help
```

#### `temper admin slack disconnect`

```text
Disconnect a Slack principal from its temper profile. Idempotent

Usage: temper admin slack disconnect [OPTIONS] <PRINCIPAL>

Arguments:
  <PRINCIPAL>  

Options:
      --vault <VAULT>      Path to vault (overrides TEMPER_VAULT and auto-detection)
      --format <FORMAT>    Output format: json | toon (default: toon on a TTY, json otherwise). Precedence: --format → TEMPER_FORMAT → cli.format config → TTY default
      --embed-threads <N>  ONNX intra-op threads for embedding. `0` = let ONNX Runtime decide. Default: this machine's performance-core count (NOT its total core count — efficiency cores measurably slow the batch down). Precedence: --embed-threads → TEMPER_ONNX_INTRA_THREADS → detected → 1
      --color <COLOR>      Color output: auto | always | never (default: auto). Precedence: --color → TEMPER_COLOR → cli.color config → NO_COLOR → auto
  -h, --help               Print help
```

### `temper admin connection`

```text
Provision connections — temper's authed link to a remote system (GitHub, Linear)

Usage: temper admin connection [OPTIONS] <COMMAND>

Commands:
  provision          Provision a connection: creates its profile, its `<handle>@webhook` emitter entity, and the context that homes it. Born `needs_credential` — attach the credential separately
  list               List connections
  show               Show one connection
  attach-credential  Attach the credential. This is what flips `needs_credential` off — the state is derived from the column being non-NULL, never from a status flag
  set-webhooks       Register the remote event types this connection receives. Non-empty ⇒ LEDGER-CAPABLE: events land, facts accrue
  set-tools          Declare the read-only remote tools. Non-empty ⇒ REACH-CAPABLE: agents can read the remote back, so judgment becomes possible
  revoke             Revoke a connection. Its profile, emitter entity, and home context survive — events already attributed to the emitter must keep resolving
  grant-reach        Grant a TEAM read-reach on this connection. Owning a connection is NOT reaching it — this writes an access grant so the team's members inherit read on what the connection receives. Reach is read-only; it confers no write
  revoke-reach       Revoke a team's read-reach on this connection. Idempotent — an absent grant is a no-op
  help               Print this message or the help of the given subcommand(s)

Options:
      --vault <VAULT>      Path to vault (overrides TEMPER_VAULT and auto-detection)
      --format <FORMAT>    Output format: json | toon (default: toon on a TTY, json otherwise). Precedence: --format → TEMPER_FORMAT → cli.format config → TTY default
      --embed-threads <N>  ONNX intra-op threads for embedding. `0` = let ONNX Runtime decide. Default: this machine's performance-core count (NOT its total core count — efficiency cores measurably slow the batch down). Precedence: --embed-threads → TEMPER_ONNX_INTRA_THREADS → detected → 1
      --color <COLOR>      Color output: auto | always | never (default: auto). Precedence: --color → TEMPER_COLOR → cli.color config → NO_COLOR → auto
  -h, --help               Print help
```

#### `temper admin connection provision`

```text
Provision a connection: creates its profile, its `<handle>@webhook` emitter entity, and the context that homes it. Born `needs_credential` — attach the credential separately.

Declaring the reach is not overhead, it IS the declaration. A connector is a reach declaration: you cannot have 50 teams with 50 distinct reaches and fewer than 50 declarations. Silence must never encode absence of capability — and never excess of it.

Usage: temper admin connection provision [OPTIONS] --provider <PROVIDER> --name <NAME>

Options:
      --provider <PROVIDER>
          The remote system: `github` | `linear`

      --vault <VAULT>
          Path to vault (overrides TEMPER_VAULT and auto-detection)

      --format <FORMAT>
          Output format: json | toon (default: toon on a TTY, json otherwise). Precedence: --format → TEMPER_FORMAT → cli.format config → TTY default

      --name <NAME>
          Human-facing name. The addressable slug is derived from it

      --embed-threads <N>
          ONNX intra-op threads for embedding. `0` = let ONNX Runtime decide. Default: this machine's performance-core count (NOT its total core count — efficiency cores measurably slow the batch down). Precedence: --embed-threads → TEMPER_ONNX_INTRA_THREADS → detected → 1

      --owner-team <OWNER_TEAM>
          Team recorded as this connection's OWNER. Not its reach. Omitting it means teamless, which is admin-only and fails closed

      --color <COLOR>
          Color output: auto | always | never (default: auto). Precedence: --color → TEMPER_COLOR → cli.color config → NO_COLOR → auto

      --reach <REACH>
          The grain the credential is scoped at, in the PROVIDER's terms: `org` | `workspace` | `installation` | `repo-set` | `project`

      --covers <COVERS>
          What the credential can ACTUALLY see, in provider terms (e.g. `acme/temper`)

  -h, --help
          Print help (see a summary with '-h')
```

#### `temper admin connection list`

```text
List connections

Usage: temper admin connection list [OPTIONS]

Options:
      --include-revoked    Include revoked connections
      --vault <VAULT>      Path to vault (overrides TEMPER_VAULT and auto-detection)
      --format <FORMAT>    Output format: json | toon (default: toon on a TTY, json otherwise). Precedence: --format → TEMPER_FORMAT → cli.format config → TTY default
      --embed-threads <N>  ONNX intra-op threads for embedding. `0` = let ONNX Runtime decide. Default: this machine's performance-core count (NOT its total core count — efficiency cores measurably slow the batch down). Precedence: --embed-threads → TEMPER_ONNX_INTRA_THREADS → detected → 1
      --color <COLOR>      Color output: auto | always | never (default: auto). Precedence: --color → TEMPER_COLOR → cli.color config → NO_COLOR → auto
  -h, --help               Print help
```

#### `temper admin connection show`

```text
Show one connection

Usage: temper admin connection show [OPTIONS] <ID>

Arguments:
  <ID>  

Options:
      --vault <VAULT>      Path to vault (overrides TEMPER_VAULT and auto-detection)
      --format <FORMAT>    Output format: json | toon (default: toon on a TTY, json otherwise). Precedence: --format → TEMPER_FORMAT → cli.format config → TTY default
      --embed-threads <N>  ONNX intra-op threads for embedding. `0` = let ONNX Runtime decide. Default: this machine's performance-core count (NOT its total core count — efficiency cores measurably slow the batch down). Precedence: --embed-threads → TEMPER_ONNX_INTRA_THREADS → detected → 1
      --color <COLOR>      Color output: auto | always | never (default: auto). Precedence: --color → TEMPER_COLOR → cli.color config → NO_COLOR → auto
  -h, --help               Print help
```

#### `temper admin connection attach-credential`

```text
Attach the credential. This is what flips `needs_credential` off — the state is derived from the column being non-NULL, never from a status flag.

No secret is stored: `--broker` names the implementation behind the seam and `--connector` identifies a connector THAT BROKER holds the secret for. The connector id lives on the row, per instance — which is what lets a self-hosted operator use their own connectors.

Usage: temper admin connection attach-credential [OPTIONS] --broker <BROKER> --connector <CONNECTOR> <ID>

Arguments:
  <ID>
          

Options:
      --broker <BROKER>
          The implementation behind the broker seam, e.g. `vercel-connect`. Never a connector id

      --vault <VAULT>
          Path to vault (overrides TEMPER_VAULT and auto-detection)

      --connector <CONNECTOR>
          The broker's identifier for this connector

      --format <FORMAT>
          Output format: json | toon (default: toon on a TTY, json otherwise). Precedence: --format → TEMPER_FORMAT → cli.format config → TTY default

      --embed-threads <N>
          ONNX intra-op threads for embedding. `0` = let ONNX Runtime decide. Default: this machine's performance-core count (NOT its total core count — efficiency cores measurably slow the batch down). Precedence: --embed-threads → TEMPER_ONNX_INTRA_THREADS → detected → 1

      --installation <INSTALLATION>
          The specific installation, where the provider has that concept (a GitHub App installation)

      --color <COLOR>
          Color output: auto | always | never (default: auto). Precedence: --color → TEMPER_COLOR → cli.color config → NO_COLOR → auto

  -h, --help
          Print help (see a summary with '-h')
```

#### `temper admin connection set-webhooks`

```text
Register the remote event types this connection receives. Non-empty ⇒ LEDGER-CAPABLE: events land, facts accrue.

Replaces the registered set wholesale — it mirrors what the remote is actually configured to send, and a merge would let a stale entry outlive the webhook it names.

Usage: temper admin connection set-webhooks [OPTIONS] --event <EVENTS> <ID>

Arguments:
  <ID>
          

Options:
      --event <EVENTS>
          A remote event type, e.g. `pull_request`. Repeatable

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

#### `temper admin connection set-tools`

```text
Declare the read-only remote tools. Non-empty ⇒ REACH-CAPABLE: agents can read the remote back, so judgment becomes possible.

Not decorative — the manifest is the evidence the provider is admissible at all. An empty manifest means judgment is IMPOSSIBLE, not merely unconfigured.

Usage: temper admin connection set-tools [OPTIONS] --tool <TOOLS> <ID>

Arguments:
  <ID>
          

Options:
      --tool <TOOLS>
          A read-only remote tool name. Repeatable

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

#### `temper admin connection revoke`

```text
Revoke a connection. Its profile, emitter entity, and home context survive — events already attributed to the emitter must keep resolving

Usage: temper admin connection revoke [OPTIONS] <ID>

Arguments:
  <ID>  

Options:
      --vault <VAULT>      Path to vault (overrides TEMPER_VAULT and auto-detection)
      --format <FORMAT>    Output format: json | toon (default: toon on a TTY, json otherwise). Precedence: --format → TEMPER_FORMAT → cli.format config → TTY default
      --embed-threads <N>  ONNX intra-op threads for embedding. `0` = let ONNX Runtime decide. Default: this machine's performance-core count (NOT its total core count — efficiency cores measurably slow the batch down). Precedence: --embed-threads → TEMPER_ONNX_INTRA_THREADS → detected → 1
      --color <COLOR>      Color output: auto | always | never (default: auto). Precedence: --color → TEMPER_COLOR → cli.color config → NO_COLOR → auto
  -h, --help               Print help
```

#### `temper admin connection grant-reach`

```text
Grant a TEAM read-reach on this connection. Owning a connection is NOT reaching it — this writes an access grant so the team's members inherit read on what the connection receives. Reach is read-only; it confers no write

Usage: temper admin connection grant-reach [OPTIONS] --team <TEAM> <ID>

Arguments:
  <ID>  The connection id (a bare UUID, as printed by `connection list`/`show`)

Options:
      --team <TEAM>                  The team receiving read-reach: a slug, a decorated `slug-<uuid>` ref, or a team UUID
      --vault <VAULT>                Path to vault (overrides TEMPER_VAULT and auto-detection)
      --affirm-reach <AFFIRM_REACH>  Affirm that binding this connection's coarse remote reach to the team is intentional — REQUIRED when the connection declares a reach; the value is the stated reason. Granting without it FAILS rather than proceeding silently. It records the intent for review; it does NOT make the connection's coarse remote reach any narrower
      --format <FORMAT>              Output format: json | toon (default: toon on a TTY, json otherwise). Precedence: --format → TEMPER_FORMAT → cli.format config → TTY default
      --embed-threads <N>            ONNX intra-op threads for embedding. `0` = let ONNX Runtime decide. Default: this machine's performance-core count (NOT its total core count — efficiency cores measurably slow the batch down). Precedence: --embed-threads → TEMPER_ONNX_INTRA_THREADS → detected → 1
      --color <COLOR>                Color output: auto | always | never (default: auto). Precedence: --color → TEMPER_COLOR → cli.color config → NO_COLOR → auto
  -h, --help                         Print help
```

#### `temper admin connection revoke-reach`

```text
Revoke a team's read-reach on this connection. Idempotent — an absent grant is a no-op

Usage: temper admin connection revoke-reach [OPTIONS] --team <TEAM> <ID>

Arguments:
  <ID>  The connection id (a bare UUID, as printed by `connection list`/`show`)

Options:
      --team <TEAM>        The team whose read-reach is revoked: a slug, a decorated `slug-<uuid>` ref, or a UUID
      --vault <VAULT>      Path to vault (overrides TEMPER_VAULT and auto-detection)
      --format <FORMAT>    Output format: json | toon (default: toon on a TTY, json otherwise). Precedence: --format → TEMPER_FORMAT → cli.format config → TTY default
      --embed-threads <N>  ONNX intra-op threads for embedding. `0` = let ONNX Runtime decide. Default: this machine's performance-core count (NOT its total core count — efficiency cores measurably slow the batch down). Precedence: --embed-threads → TEMPER_ONNX_INTRA_THREADS → detected → 1
      --color <COLOR>      Color output: auto | always | never (default: auto). Precedence: --color → TEMPER_COLOR → cli.color config → NO_COLOR → auto
  -h, --help               Print help
```

### `temper admin subscription`

```text
Manage subscriptions — a team/context/cogmap subscribes to a connection's events

Usage: temper admin subscription [OPTIONS] <COMMAND>

Commands:
  create  Create a subscription. The two-leg authz gate (authoring-team manage-capable + reach grant held on the connection) runs server-side
  list    List subscriptions visible to the caller
  show    Show one subscription
  revoke  Revoke a subscription. Rows are never deleted — a revoked subscription stops matching but stays resolvable for the delivery row's research-corpus property
  help    Print this message or the help of the given subcommand(s)

Options:
      --vault <VAULT>      Path to vault (overrides TEMPER_VAULT and auto-detection)
      --format <FORMAT>    Output format: json | toon (default: toon on a TTY, json otherwise). Precedence: --format → TEMPER_FORMAT → cli.format config → TTY default
      --embed-threads <N>  ONNX intra-op threads for embedding. `0` = let ONNX Runtime decide. Default: this machine's performance-core count (NOT its total core count — efficiency cores measurably slow the batch down). Precedence: --embed-threads → TEMPER_ONNX_INTRA_THREADS → detected → 1
      --color <COLOR>      Color output: auto | always | never (default: auto). Precedence: --color → TEMPER_COLOR → cli.color config → NO_COLOR → auto
  -h, --help               Print help
```

#### `temper admin subscription create`

```text
Create a subscription. The two-leg authz gate (authoring-team manage-capable + reach grant held on the connection) runs server-side

Usage: temper admin subscription create [OPTIONS] --subscriber-table <SUBSCRIBER_TABLE> --subscriber-id <SUBSCRIBER_ID> --authoring-team-id <AUTHORING_TEAM_ID> --connection-id <CONNECTION_ID> --selector <SELECTOR>

Options:
      --subscriber-table <SUBSCRIBER_TABLE>
          The subscriber kind: `kb_contexts` | `kb_cogmaps` | `kb_teams`
      --vault <VAULT>
          Path to vault (overrides TEMPER_VAULT and auto-detection)
      --format <FORMAT>
          Output format: json | toon (default: toon on a TTY, json otherwise). Precedence: --format → TEMPER_FORMAT → cli.format config → TTY default
      --subscriber-id <SUBSCRIBER_ID>
          The subscriber row id (UUID)
      --authoring-team-id <AUTHORING_TEAM_ID>
          The team whose manage-capable role authorizes this subscription (UUID). For `kb_teams` subscribers, this must equal `--subscriber-id`
      --embed-threads <N>
          ONNX intra-op threads for embedding. `0` = let ONNX Runtime decide. Default: this machine's performance-core count (NOT its total core count — efficiency cores measurably slow the batch down). Precedence: --embed-threads → TEMPER_ONNX_INTRA_THREADS → detected → 1
      --color <COLOR>
          Color output: auto | always | never (default: auto). Precedence: --color → TEMPER_COLOR → cli.color config → NO_COLOR → auto
      --connection-id <CONNECTION_ID>
          The connection id (UUID)
      --selector <SELECTOR>
          The selector: a JSON string, or `@file.json` to read from a file. The shape is the `SubscriptionSelector` enum (`{"kind": "git_hub_repository", "repo": "acme/temper", ...}`)
  -h, --help
          Print help
```

#### `temper admin subscription list`

```text
List subscriptions visible to the caller

Usage: temper admin subscription list [OPTIONS]

Options:
      --include-revoked                Include revoked subscriptions
      --vault <VAULT>                  Path to vault (overrides TEMPER_VAULT and auto-detection)
      --connection-id <CONNECTION_ID>  Optional: filter to subscriptions against this connection (UUID)
      --format <FORMAT>                Output format: json | toon (default: toon on a TTY, json otherwise). Precedence: --format → TEMPER_FORMAT → cli.format config → TTY default
      --embed-threads <N>              ONNX intra-op threads for embedding. `0` = let ONNX Runtime decide. Default: this machine's performance-core count (NOT its total core count — efficiency cores measurably slow the batch down). Precedence: --embed-threads → TEMPER_ONNX_INTRA_THREADS → detected → 1
      --color <COLOR>                  Color output: auto | always | never (default: auto). Precedence: --color → TEMPER_COLOR → cli.color config → NO_COLOR → auto
  -h, --help                           Print help
```

#### `temper admin subscription show`

```text
Show one subscription

Usage: temper admin subscription show [OPTIONS] <ID>

Arguments:
  <ID>  

Options:
      --vault <VAULT>      Path to vault (overrides TEMPER_VAULT and auto-detection)
      --format <FORMAT>    Output format: json | toon (default: toon on a TTY, json otherwise). Precedence: --format → TEMPER_FORMAT → cli.format config → TTY default
      --embed-threads <N>  ONNX intra-op threads for embedding. `0` = let ONNX Runtime decide. Default: this machine's performance-core count (NOT its total core count — efficiency cores measurably slow the batch down). Precedence: --embed-threads → TEMPER_ONNX_INTRA_THREADS → detected → 1
      --color <COLOR>      Color output: auto | always | never (default: auto). Precedence: --color → TEMPER_COLOR → cli.color config → NO_COLOR → auto
  -h, --help               Print help
```

#### `temper admin subscription revoke`

```text
Revoke a subscription. Rows are never deleted — a revoked subscription stops matching but stays resolvable for the delivery row's research-corpus property

Usage: temper admin subscription revoke [OPTIONS] <ID>

Arguments:
  <ID>  

Options:
      --vault <VAULT>      Path to vault (overrides TEMPER_VAULT and auto-detection)
      --format <FORMAT>    Output format: json | toon (default: toon on a TTY, json otherwise). Precedence: --format → TEMPER_FORMAT → cli.format config → TTY default
      --embed-threads <N>  ONNX intra-op threads for embedding. `0` = let ONNX Runtime decide. Default: this machine's performance-core count (NOT its total core count — efficiency cores measurably slow the batch down). Precedence: --embed-threads → TEMPER_ONNX_INTRA_THREADS → detected → 1
      --color <COLOR>      Color output: auto | always | never (default: auto). Precedence: --color → TEMPER_COLOR → cli.color config → NO_COLOR → auto
  -h, --help               Print help
```

### `temper admin reembed`

```text
Re-embed chunks whose vectors were produced by an older model (the drain does the work)

Nothing is destroyed: a stale vector stays searchable until a fresh one replaces it. Staleness is derived, not marked, so this is idempotent and safe to re-run. Start with --dry-run.

Usage: temper admin reembed [OPTIONS]

Options:
      --resource <RESOURCE>
          Re-embed just this resource (UUID or decorated ref)

      --vault <VAULT>
          Path to vault (overrides TEMPER_VAULT and auto-detection)

      --context <CONTEXT>
          Re-embed every stale resource in this context (`@me/slug`, `+team/slug`, or UUID)

      --format <FORMAT>
          Output format: json | toon (default: toon on a TTY, json otherwise). Precedence: --format → TEMPER_FORMAT → cli.format config → TTY default

      --all
          Re-embed everything stale. Must be asked for by name — never the default

      --embed-threads <N>
          ONNX intra-op threads for embedding. `0` = let ONNX Runtime decide. Default: this machine's performance-core count (NOT its total core count — efficiency cores measurably slow the batch down). Precedence: --embed-threads → TEMPER_ONNX_INTRA_THREADS → detected → 1

      --color <COLOR>
          Color output: auto | always | never (default: auto). Precedence: --color → TEMPER_COLOR → cli.color config → NO_COLOR → auto

      --limit <LIMIT>
          Max resources to enqueue in this call (walk the index in bounded steps)

      --dry-run
          Report what is stale without enqueuing anything

  -h, --help
          Print help (see a summary with '-h')
```
