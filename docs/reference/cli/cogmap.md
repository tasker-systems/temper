<!-- GENERATED — do not edit. Emitted from the built binary's `--help` by scripts/emit-cli-reference.py; see .github/scripts/check-cli-reference-drift.sh. -->

# `temper cogmap`

Operate on cognitive maps (admin-gated content reconcile)

```text
Operate on cognitive maps (admin-gated content reconcile)

Usage: temper cogmap [OPTIONS] <COMMAND>

Commands:
  list            List the cognitive maps you can see — each with its ref, held-by scope, region/resource counts, and charter statement (what the map is for). Filter by name and/or team
  show            Orient on one cognitive map: its charter (what it's for) and the resources it's built on (its foundational homed set, with the telos flagged)
  reconcile       Reconcile a cognitive map's content to a committed manifest
  create          Genesis (create) a new cognitive map from a committed manifest
  shape           Read a cognitive map's materialized regions (surface tier)
  region-metrics  Read a cognitive map's per-region analytics metrics
  analytics       Read a cognitive map's map-level analytics (telos, staleness, regulation)
  materialize     Re-materialize a cognitive map's regions when its event delta clears the threshold
  bind            Bind a cognitive map to a team. Requires system-admin, OR that you manage the team (owner/maintainer) AND administer the map (hold a grant on it). Widens the map's reach to the team's shared resources
  unbind          Unbind a cognitive map from a team (same authority as bind)
  grant           Grant a capability on a cognitive map (admin or a can_grant holder). Post-Q-A, authoring a map requires an explicit write grant, not team membership
  revoke          Revoke a capability grant on a cognitive map (admin or a can_grant holder)
  help            Print this message or the help of the given subcommand(s)

Options:
      --vault <VAULT>      Path to vault (overrides TEMPER_VAULT and auto-detection)
      --format <FORMAT>    Output format: json | toon (default: toon on a TTY, json otherwise). Precedence: --format → TEMPER_FORMAT → cli.format config → TTY default
      --embed-threads <N>  ONNX intra-op threads for embedding. `0` = let ONNX Runtime decide. Default: this machine's performance-core count (NOT its total core count — efficiency cores measurably slow the batch down). Precedence: --embed-threads → TEMPER_ONNX_INTRA_THREADS → detected → 1
      --color <COLOR>      Color output: auto | always | never (default: auto). Precedence: --color → TEMPER_COLOR → cli.color config → NO_COLOR → auto
  -h, --help               Print help
```

### `temper cogmap list`

```text
List the cognitive maps you can see — each with its ref, held-by scope, region/resource counts, and charter statement (what the map is for). Filter by name and/or team

Usage: temper cogmap list [OPTIONS]

Options:
      --name-contains <NAME_CONTAINS>  Filter to maps whose name contains this substring (case-insensitive)
      --vault <VAULT>                  Path to vault (overrides TEMPER_VAULT and auto-detection)
      --format <FORMAT>                Output format: json | toon (default: toon on a TTY, json otherwise). Precedence: --format → TEMPER_FORMAT → cli.format config → TTY default
      --team <TEAM>                    Filter to maps held by this team: a slug (optionally `+`-prefixed), a decorated `slug-<uuid>` ref, or a team UUID
      --embed-threads <N>              ONNX intra-op threads for embedding. `0` = let ONNX Runtime decide. Default: this machine's performance-core count (NOT its total core count — efficiency cores measurably slow the batch down). Precedence: --embed-threads → TEMPER_ONNX_INTRA_THREADS → detected → 1
      --color <COLOR>                  Color output: auto | always | never (default: auto). Precedence: --color → TEMPER_COLOR → cli.color config → NO_COLOR → auto
  -h, --help                           Print help
```

### `temper cogmap show`

```text
Orient on one cognitive map: its charter (what it's for) and the resources it's built on (its foundational homed set, with the telos flagged)

Usage: temper cogmap show [OPTIONS] <COGMAP>

Arguments:
  <COGMAP>  The cognitive map, by ref (UUID or `slug-<uuid>`)

Options:
      --vault <VAULT>      Path to vault (overrides TEMPER_VAULT and auto-detection)
      --format <FORMAT>    Output format: json | toon (default: toon on a TTY, json otherwise). Precedence: --format → TEMPER_FORMAT → cli.format config → TTY default
      --embed-threads <N>  ONNX intra-op threads for embedding. `0` = let ONNX Runtime decide. Default: this machine's performance-core count (NOT its total core count — efficiency cores measurably slow the batch down). Precedence: --embed-threads → TEMPER_ONNX_INTRA_THREADS → detected → 1
      --color <COLOR>      Color output: auto | always | never (default: auto). Precedence: --color → TEMPER_COLOR → cli.color config → NO_COLOR → auto
  -h, --help               Print help
```

### `temper cogmap reconcile`

```text
Reconcile a cognitive map's content to a committed manifest.

Reads the authored manifest, embeds each entry client-side, and PUTs a pre-embedded desired-state request to `/api/cognitive-maps/{id}` (admin-gated, idempotent).

Usage: temper cogmap reconcile [OPTIONS] --manifest <MANIFEST> <REF>

Arguments:
  <REF>
          Cognitive-map ref: a UUID or the decorated `slug-<uuid>` form

Options:
      --manifest <MANIFEST>
          Path to the committed manifest (YAML)

      --vault <VAULT>
          Path to vault (overrides TEMPER_VAULT and auto-detection)

      --format <FORMAT>
          Output format: json | toon (default: toon on a TTY, json otherwise). Precedence: --format → TEMPER_FORMAT → cli.format config → TTY default

      --invocation <INVOCATION>
          Correlate this act with an open invocation envelope (its ref/UUID from `invocation open`)

      --correlation <CORRELATION>
          Stitch this write into an act-grain thread shared with other writes (a bare UUID you mint). Provenance only — it never authorizes. Omit and the event self-roots

      --embed-threads <N>
          ONNX intra-op threads for embedding. `0` = let ONNX Runtime decide. Default: this machine's performance-core count (NOT its total core count — efficiency cores measurably slow the batch down). Precedence: --embed-threads → TEMPER_ONNX_INTRA_THREADS → detected → 1

      --color <COLOR>
          Color output: auto | always | never (default: auto). Precedence: --color → TEMPER_COLOR → cli.color config → NO_COLOR → auto

      --confidence <CONFIDENCE>
          Graded authorship confidence: tentative, probable, or confident
          
          [possible values: tentative, probable, confident]

      --reasoning <REASONING>
          Free-text reasoning for the act (authorship; requires --confidence)

      --rationale <RATIONALE>
          Structured rationale for the act (authorship; requires --confidence)

      --persona <PERSONA>
          Persona/role the author acted as (authorship; requires --confidence)

      --model <MODEL>
          Model that authored the act (authorship; requires --confidence)

  -h, --help
          Print help (see a summary with '-h')
```

### `temper cogmap create`

```text
Genesis (create) a new cognitive map from a committed manifest.

Reads the authored genesis manifest (name, telos title, optional ids + telos charter), embeds the charter client-side, and POSTs to `/api/cognitive-maps` (open to any authenticated profile; idempotent). Manifest/`--id` ids are honored only for a system-admin — a non-admin always receives a server-minted id.

Usage: temper cogmap create [OPTIONS] --manifest <MANIFEST>

Options:
      --manifest <MANIFEST>
          Path to the genesis manifest (YAML)

      --vault <VAULT>
          Path to vault (overrides TEMPER_VAULT and auto-detection)

      --format <FORMAT>
          Output format: json | toon (default: toon on a TTY, json otherwise). Precedence: --format → TEMPER_FORMAT → cli.format config → TTY default

      --name <NAME>
          Override the manifest's cogmap name

      --embed-threads <N>
          ONNX intra-op threads for embedding. `0` = let ONNX Runtime decide. Default: this machine's performance-core count (NOT its total core count — efficiency cores measurably slow the batch down). Precedence: --embed-threads → TEMPER_ONNX_INTRA_THREADS → detected → 1

      --id <ID>
          Override the manifest's cogmap id (a UUID or the decorated `slug-<uuid>` form)

      --color <COLOR>
          Color output: auto | always | never (default: auto). Precedence: --color → TEMPER_COLOR → cli.color config → NO_COLOR → auto

  -h, --help
          Print help (see a summary with '-h')
```

### `temper cogmap shape`

```text
Read a cognitive map's materialized regions (surface tier)

Usage: temper cogmap shape [OPTIONS] <COGMAP>

Arguments:
  <COGMAP>  The cognitive map, by ref (UUID or `slug-<uuid>`)

Options:
      --lens <LENS>        Optional lens ref to filter regions
      --vault <VAULT>      Path to vault (overrides TEMPER_VAULT and auto-detection)
      --format <FORMAT>    Output format: json | toon (default: toon on a TTY, json otherwise). Precedence: --format → TEMPER_FORMAT → cli.format config → TTY default
      --embed-threads <N>  ONNX intra-op threads for embedding. `0` = let ONNX Runtime decide. Default: this machine's performance-core count (NOT its total core count — efficiency cores measurably slow the batch down). Precedence: --embed-threads → TEMPER_ONNX_INTRA_THREADS → detected → 1
      --color <COLOR>      Color output: auto | always | never (default: auto). Precedence: --color → TEMPER_COLOR → cli.color config → NO_COLOR → auto
  -h, --help               Print help
```

### `temper cogmap region-metrics`

```text
Read a cognitive map's per-region analytics metrics

Usage: temper cogmap region-metrics [OPTIONS] <COGMAP>

Arguments:
  <COGMAP>  The cognitive map, by ref (UUID or `slug-<uuid>`)

Options:
      --lens <LENS>        Optional lens ref to filter regions
      --vault <VAULT>      Path to vault (overrides TEMPER_VAULT and auto-detection)
      --format <FORMAT>    Output format: json | toon (default: toon on a TTY, json otherwise). Precedence: --format → TEMPER_FORMAT → cli.format config → TTY default
      --embed-threads <N>  ONNX intra-op threads for embedding. `0` = let ONNX Runtime decide. Default: this machine's performance-core count (NOT its total core count — efficiency cores measurably slow the batch down). Precedence: --embed-threads → TEMPER_ONNX_INTRA_THREADS → detected → 1
      --color <COLOR>      Color output: auto | always | never (default: auto). Precedence: --color → TEMPER_COLOR → cli.color config → NO_COLOR → auto
  -h, --help               Print help
```

### `temper cogmap analytics`

```text
Read a cognitive map's map-level analytics (telos, staleness, regulation)

Usage: temper cogmap analytics [OPTIONS] <COGMAP>

Arguments:
  <COGMAP>  The cognitive map, by ref (UUID or `slug-<uuid>`)

Options:
      --vault <VAULT>      Path to vault (overrides TEMPER_VAULT and auto-detection)
      --format <FORMAT>    Output format: json | toon (default: toon on a TTY, json otherwise). Precedence: --format → TEMPER_FORMAT → cli.format config → TTY default
      --embed-threads <N>  ONNX intra-op threads for embedding. `0` = let ONNX Runtime decide. Default: this machine's performance-core count (NOT its total core count — efficiency cores measurably slow the batch down). Precedence: --embed-threads → TEMPER_ONNX_INTRA_THREADS → detected → 1
      --color <COLOR>      Color output: auto | always | never (default: auto). Precedence: --color → TEMPER_COLOR → cli.color config → NO_COLOR → auto
  -h, --help               Print help
```

### `temper cogmap materialize`

```text
Re-materialize a cognitive map's regions when its event delta clears the threshold.

Regions only exist *after* a materialize. A map below the threshold is a no-op (`materialized: false`), not an error.

Usage: temper cogmap materialize [OPTIONS] <COGMAP>

Arguments:
  <COGMAP>
          The cognitive map, by ref (UUID or `slug-<uuid>`)

Options:
      --threshold <THRESHOLD>
          Minimum unmaterialized-event count required to trigger. Server default when omitted

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

### `temper cogmap bind`

```text
Bind a cognitive map to a team. Requires system-admin, OR that you manage the team (owner/maintainer) AND administer the map (hold a grant on it). Widens the map's reach to the team's shared resources

Usage: temper cogmap bind [OPTIONS] <REF> <TEAM>

Arguments:
  <REF>   Cognitive-map ref: a UUID or the decorated `slug-<uuid>` form
  <TEAM>  Team to bind to: a team slug (optionally `+`-prefixed) or a team UUID

Options:
      --vault <VAULT>      Path to vault (overrides TEMPER_VAULT and auto-detection)
      --format <FORMAT>    Output format: json | toon (default: toon on a TTY, json otherwise). Precedence: --format → TEMPER_FORMAT → cli.format config → TTY default
      --embed-threads <N>  ONNX intra-op threads for embedding. `0` = let ONNX Runtime decide. Default: this machine's performance-core count (NOT its total core count — efficiency cores measurably slow the batch down). Precedence: --embed-threads → TEMPER_ONNX_INTRA_THREADS → detected → 1
      --color <COLOR>      Color output: auto | always | never (default: auto). Precedence: --color → TEMPER_COLOR → cli.color config → NO_COLOR → auto
  -h, --help               Print help
```

### `temper cogmap unbind`

```text
Unbind a cognitive map from a team (same authority as bind)

Usage: temper cogmap unbind [OPTIONS] <REF> <TEAM>

Arguments:
  <REF>   Cognitive-map ref: a UUID or the decorated `slug-<uuid>` form
  <TEAM>  Team to unbind: a team slug (optionally `+`-prefixed) or a team UUID

Options:
      --vault <VAULT>      Path to vault (overrides TEMPER_VAULT and auto-detection)
      --format <FORMAT>    Output format: json | toon (default: toon on a TTY, json otherwise). Precedence: --format → TEMPER_FORMAT → cli.format config → TTY default
      --embed-threads <N>  ONNX intra-op threads for embedding. `0` = let ONNX Runtime decide. Default: this machine's performance-core count (NOT its total core count — efficiency cores measurably slow the batch down). Precedence: --embed-threads → TEMPER_ONNX_INTRA_THREADS → detected → 1
      --color <COLOR>      Color output: auto | always | never (default: auto). Precedence: --color → TEMPER_COLOR → cli.color config → NO_COLOR → auto
  -h, --help               Print help
```

### `temper cogmap grant`

```text
Grant a capability on a cognitive map (admin or a can_grant holder). Post-Q-A, authoring a map requires an explicit write grant, not team membership

Usage: temper cogmap grant [OPTIONS] <REF>

Arguments:
  <REF>  Cognitive-map ref: a UUID or the decorated `slug-<uuid>` form

Options:
      --to-profile <TO_PROFILE>  Grant to this profile (UUID). Mutually exclusive with `--to-team`
      --vault <VAULT>            Path to vault (overrides TEMPER_VAULT and auto-detection)
      --format <FORMAT>          Output format: json | toon (default: toon on a TTY, json otherwise). Precedence: --format → TEMPER_FORMAT → cli.format config → TTY default
      --to-team <TO_TEAM>        Grant to this team: a team slug (optionally `+`-prefixed), a decorated `slug-<uuid>` ref, or a team UUID. Mutually exclusive with `--to-profile`
      --embed-threads <N>        ONNX intra-op threads for embedding. `0` = let ONNX Runtime decide. Default: this machine's performance-core count (NOT its total core count — efficiency cores measurably slow the batch down). Precedence: --embed-threads → TEMPER_ONNX_INTRA_THREADS → detected → 1
      --read                     Grant read
      --color <COLOR>            Color output: auto | always | never (default: auto). Precedence: --color → TEMPER_COLOR → cli.color config → NO_COLOR → auto
      --write                    Grant write (implies read)
      --grant                    Grant delegated-grant authority (implies read)
  -h, --help                     Print help
```

### `temper cogmap revoke`

```text
Revoke a capability grant on a cognitive map (admin or a can_grant holder)

Usage: temper cogmap revoke [OPTIONS] <REF>

Arguments:
  <REF>  Cognitive-map ref: a UUID or the decorated `slug-<uuid>` form

Options:
      --from-profile <FROM_PROFILE>  Revoke this profile's grant (UUID). Mutually exclusive with `--from-team`
      --vault <VAULT>                Path to vault (overrides TEMPER_VAULT and auto-detection)
      --format <FORMAT>              Output format: json | toon (default: toon on a TTY, json otherwise). Precedence: --format → TEMPER_FORMAT → cli.format config → TTY default
      --from-team <FROM_TEAM>        Revoke this team's grant: a team slug (optionally `+`-prefixed), a decorated `slug-<uuid>` ref, or a team UUID. Mutually exclusive with `--from-profile`
      --embed-threads <N>            ONNX intra-op threads for embedding. `0` = let ONNX Runtime decide. Default: this machine's performance-core count (NOT its total core count — efficiency cores measurably slow the batch down). Precedence: --embed-threads → TEMPER_ONNX_INTRA_THREADS → detected → 1
      --color <COLOR>                Color output: auto | always | never (default: auto). Precedence: --color → TEMPER_COLOR → cli.color config → NO_COLOR → auto
  -h, --help                         Print help
```
