<!-- GENERATED — do not edit. Emitted from the built binary's `--help` by scripts/emit-cli-reference.py; see .github/scripts/check-cli-reference-drift.sh. -->

# `temper context`

Manage contexts (projects)

```text
Manage contexts (projects)

Usage: temper context [OPTIONS] <COMMAND>

Commands:
  subscribe       Subscribe to a context locally so `temper pull` materializes it. Local config only — this does NOT create the context on the server (use `context create`) and has no server/RBAC effect
  unsubscribe     Unsubscribe from a context locally (drops it from the local pull set). Local config only — no server effect
  create          Create a new context on the server
  list            List the contexts you can see on the server (with owner ref + resource counts)
  share           Share a context into a team's read-reach. Requires that you administer the context (own it, or manage its owning team) AND manage the target team (owner/maintainer), OR that you are an instance administrator. The context ref is a UUID or the `@handle/slug` / `+team-slug/slug` form (from `context list`); `@me` shorthand is not accepted
  unshare         Unshare a context from a team (same authority as `share`)
  transfer        Transfer a context's ownership to a team — the single path to shared authorship (read-sharing stays `share`; writing into a context requires team ownership)
  rename          Rename a context. The slug is derived from the new name — there is no separate `--slug`
  shape           Orient in a context by its REGIONS: the distilled, region-level view of everything homed there, most salient first. The fastest way to see what a context is about without reading any single resource in it
  region-metrics  Per-region analytics for a context: centrality, content cohesion, internal tension, reference standing, telos alignment
  materialize     Re-form a context's regions when enough has changed since the last materialize. Below the threshold this is a safe no-op (`materialized: false`). Requires write access to the context
  help            Print this message or the help of the given subcommand(s)

Options:
      --vault <VAULT>      Path to vault (overrides TEMPER_VAULT and auto-detection)
      --format <FORMAT>    Output format: json | toon (default: toon on a TTY, json otherwise). Precedence: --format → TEMPER_FORMAT → cli.format config → TTY default
      --embed-threads <N>  ONNX intra-op threads for embedding. `0` = let ONNX Runtime decide. Default: this machine's performance-core count (NOT its total core count — efficiency cores measurably slow the batch down). Precedence: --embed-threads → TEMPER_ONNX_INTRA_THREADS → detected → 1
      --color <COLOR>      Color output: auto | always | never (default: auto). Precedence: --color → TEMPER_COLOR → cli.color config → NO_COLOR → auto
  -h, --help               Print help
```

### `temper context subscribe`

```text
Subscribe to a context locally so `temper pull` materializes it. Local config only — this does NOT create the context on the server (use `context create`) and has no server/RBAC effect

Usage: temper context subscribe [OPTIONS] <NAME>

Arguments:
  <NAME>  Context name to subscribe to

Options:
      --vault <VAULT>      Path to vault (overrides TEMPER_VAULT and auto-detection)
      --format <FORMAT>    Output format: json | toon (default: toon on a TTY, json otherwise). Precedence: --format → TEMPER_FORMAT → cli.format config → TTY default
      --embed-threads <N>  ONNX intra-op threads for embedding. `0` = let ONNX Runtime decide. Default: this machine's performance-core count (NOT its total core count — efficiency cores measurably slow the batch down). Precedence: --embed-threads → TEMPER_ONNX_INTRA_THREADS → detected → 1
      --color <COLOR>      Color output: auto | always | never (default: auto). Precedence: --color → TEMPER_COLOR → cli.color config → NO_COLOR → auto
  -h, --help               Print help
```

### `temper context unsubscribe`

```text
Unsubscribe from a context locally (drops it from the local pull set). Local config only — no server effect

Usage: temper context unsubscribe [OPTIONS] <NAME>

Arguments:
  <NAME>  

Options:
      --vault <VAULT>      Path to vault (overrides TEMPER_VAULT and auto-detection)
      --format <FORMAT>    Output format: json | toon (default: toon on a TTY, json otherwise). Precedence: --format → TEMPER_FORMAT → cli.format config → TTY default
      --embed-threads <N>  ONNX intra-op threads for embedding. `0` = let ONNX Runtime decide. Default: this machine's performance-core count (NOT its total core count — efficiency cores measurably slow the batch down). Precedence: --embed-threads → TEMPER_ONNX_INTRA_THREADS → detected → 1
      --color <COLOR>      Color output: auto | always | never (default: auto). Precedence: --color → TEMPER_COLOR → cli.color config → NO_COLOR → auto
  -h, --help               Print help
```

### `temper context create`

```text
Create a new context on the server

Usage: temper context create [OPTIONS] <NAME>

Arguments:
  <NAME>  Context name to create

Options:
      --owner <OWNER>      Owner of the context: `@me` (default) or `+<team-slug>` for a team-owned context (requires owner/maintainer on the team)
      --vault <VAULT>      Path to vault (overrides TEMPER_VAULT and auto-detection)
      --format <FORMAT>    Output format: json | toon (default: toon on a TTY, json otherwise). Precedence: --format → TEMPER_FORMAT → cli.format config → TTY default
      --embed-threads <N>  ONNX intra-op threads for embedding. `0` = let ONNX Runtime decide. Default: this machine's performance-core count (NOT its total core count — efficiency cores measurably slow the batch down). Precedence: --embed-threads → TEMPER_ONNX_INTRA_THREADS → detected → 1
      --color <COLOR>      Color output: auto | always | never (default: auto). Precedence: --color → TEMPER_COLOR → cli.color config → NO_COLOR → auto
  -h, --help               Print help
```

### `temper context list`

```text
List the contexts you can see on the server (with owner ref + resource counts)

Usage: temper context list [OPTIONS]

Options:
      --vault <VAULT>      Path to vault (overrides TEMPER_VAULT and auto-detection)
      --format <FORMAT>    Output format: json | toon (default: toon on a TTY, json otherwise). Precedence: --format → TEMPER_FORMAT → cli.format config → TTY default
      --embed-threads <N>  ONNX intra-op threads for embedding. `0` = let ONNX Runtime decide. Default: this machine's performance-core count (NOT its total core count — efficiency cores measurably slow the batch down). Precedence: --embed-threads → TEMPER_ONNX_INTRA_THREADS → detected → 1
      --color <COLOR>      Color output: auto | always | never (default: auto). Precedence: --color → TEMPER_COLOR → cli.color config → NO_COLOR → auto
  -h, --help               Print help
```

### `temper context share`

```text
Share a context into a team's read-reach. Requires that you administer the context (own it, or manage its owning team) AND manage the target team (owner/maintainer), OR that you are an instance administrator. The context ref is a UUID or the `@handle/slug` / `+team-slug/slug` form (from `context list`); `@me` shorthand is not accepted

Usage: temper context share [OPTIONS] <CONTEXT> <TEAM>

Arguments:
  <CONTEXT>  Context ref: a UUID or `@handle/slug` / `+team-slug/slug`
  <TEAM>     Team to share into: a team slug (optionally `+`-prefixed) or a team UUID

Options:
      --vault <VAULT>      Path to vault (overrides TEMPER_VAULT and auto-detection)
      --format <FORMAT>    Output format: json | toon (default: toon on a TTY, json otherwise). Precedence: --format → TEMPER_FORMAT → cli.format config → TTY default
      --embed-threads <N>  ONNX intra-op threads for embedding. `0` = let ONNX Runtime decide. Default: this machine's performance-core count (NOT its total core count — efficiency cores measurably slow the batch down). Precedence: --embed-threads → TEMPER_ONNX_INTRA_THREADS → detected → 1
      --color <COLOR>      Color output: auto | always | never (default: auto). Precedence: --color → TEMPER_COLOR → cli.color config → NO_COLOR → auto
  -h, --help               Print help
```

### `temper context unshare`

```text
Unshare a context from a team (same authority as `share`)

Usage: temper context unshare [OPTIONS] <CONTEXT> <TEAM>

Arguments:
  <CONTEXT>  Context ref: a UUID or `@handle/slug` / `+team-slug/slug`
  <TEAM>     Team to unshare: a team slug (optionally `+`-prefixed) or a team UUID

Options:
      --vault <VAULT>      Path to vault (overrides TEMPER_VAULT and auto-detection)
      --format <FORMAT>    Output format: json | toon (default: toon on a TTY, json otherwise). Precedence: --format → TEMPER_FORMAT → cli.format config → TTY default
      --embed-threads <N>  ONNX intra-op threads for embedding. `0` = let ONNX Runtime decide. Default: this machine's performance-core count (NOT its total core count — efficiency cores measurably slow the batch down). Precedence: --embed-threads → TEMPER_ONNX_INTRA_THREADS → detected → 1
      --color <COLOR>      Color output: auto | always | never (default: auto). Precedence: --color → TEMPER_COLOR → cli.color config → NO_COLOR → auto
  -h, --help               Print help
```

### `temper context transfer`

```text
Transfer a context's ownership to a team — the single path to shared authorship (read-sharing stays `share`; writing into a context requires team ownership)

Usage: temper context transfer [OPTIONS] <CONTEXT> <TEAM>

Arguments:
  <CONTEXT>  Context ref: a UUID or `@me/slug` / `@handle/slug` / `+team-slug/slug`
  <TEAM>     Target team: a team slug (optionally `+`-prefixed) or a team UUID

Options:
      --vault <VAULT>      Path to vault (overrides TEMPER_VAULT and auto-detection)
      --format <FORMAT>    Output format: json | toon (default: toon on a TTY, json otherwise). Precedence: --format → TEMPER_FORMAT → cli.format config → TTY default
      --embed-threads <N>  ONNX intra-op threads for embedding. `0` = let ONNX Runtime decide. Default: this machine's performance-core count (NOT its total core count — efficiency cores measurably slow the batch down). Precedence: --embed-threads → TEMPER_ONNX_INTRA_THREADS → detected → 1
      --color <COLOR>      Color output: auto | always | never (default: auto). Precedence: --color → TEMPER_COLOR → cli.color config → NO_COLOR → auto
  -h, --help               Print help
```

### `temper context rename`

```text
Rename a context. The slug is derived from the new name — there is no separate `--slug`.

THIS RE-ADDRESSES THE CONTEXT. After renaming `@me/temper` to "Temper KB", the ref `@me/temper` no longer resolves and `@me/temper-kb` does. Every stored `@owner/slug` string held by anyone, anywhere, is stale — scripts, agent instructions, bookmarks. Use the `context_ref` in the output as the address from now on (a context UUID never changes, and is the stable thing to store).

Local state is NOT updated: the vault's projected context directory keeps its old name (the next `temper pull` writes a second, new-named directory beside it, and the old one survives with stale files), and a `sync.subscriptions.contexts` entry naming the old slug silently stops matching. Refresh them yourself — re-subscribe with the new ref, and re-run `temper pull` / `temper skill`.

Usage: temper context rename [OPTIONS] --name <NAME> <CONTEXT>

Arguments:
  <CONTEXT>
          Context ref: a UUID or `@me/slug` / `@handle/slug` / `+team-slug/slug`

Options:
      --name <NAME>
          The new display name. The new slug is derived from it

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

### `temper context shape`

```text
Orient in a context by its REGIONS: the distilled, region-level view of everything homed there, most salient first. The fastest way to see what a context is about without reading any single resource in it.

Empty means the context has not materialized regions yet — run `context materialize`.

Usage: temper context shape [OPTIONS] <CONTEXT>

Arguments:
  <CONTEXT>
          Context ref: a UUID or `@me/slug` / `+team-slug/slug`

Options:
      --lens <LENS>
          Optional lens ref to narrow the read; omit for all lenses

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

### `temper context region-metrics`

```text
Per-region analytics for a context: centrality, content cohesion, internal tension, reference standing, telos alignment

Usage: temper context region-metrics [OPTIONS] <CONTEXT>

Arguments:
  <CONTEXT>  Context ref: a UUID or `@me/slug` / `+team-slug/slug`

Options:
      --lens <LENS>        Optional lens ref to narrow the read; omit for all lenses
      --vault <VAULT>      Path to vault (overrides TEMPER_VAULT and auto-detection)
      --format <FORMAT>    Output format: json | toon (default: toon on a TTY, json otherwise). Precedence: --format → TEMPER_FORMAT → cli.format config → TTY default
      --embed-threads <N>  ONNX intra-op threads for embedding. `0` = let ONNX Runtime decide. Default: this machine's performance-core count (NOT its total core count — efficiency cores measurably slow the batch down). Precedence: --embed-threads → TEMPER_ONNX_INTRA_THREADS → detected → 1
      --color <COLOR>      Color output: auto | always | never (default: auto). Precedence: --color → TEMPER_COLOR → cli.color config → NO_COLOR → auto
  -h, --help               Print help
```

### `temper context materialize`

```text
Re-form a context's regions when enough has changed since the last materialize. Below the threshold this is a safe no-op (`materialized: false`). Requires write access to the context

Usage: temper context materialize [OPTIONS] <CONTEXT>

Arguments:
  <CONTEXT>  Context ref: a UUID or `@me/slug` / `+team-slug/slug`

Options:
      --threshold <THRESHOLD>  Formation-event threshold to gate on; omit for the default
      --vault <VAULT>          Path to vault (overrides TEMPER_VAULT and auto-detection)
      --format <FORMAT>        Output format: json | toon (default: toon on a TTY, json otherwise). Precedence: --format → TEMPER_FORMAT → cli.format config → TTY default
      --embed-threads <N>      ONNX intra-op threads for embedding. `0` = let ONNX Runtime decide. Default: this machine's performance-core count (NOT its total core count — efficiency cores measurably slow the batch down). Precedence: --embed-threads → TEMPER_ONNX_INTRA_THREADS → detected → 1
      --color <COLOR>          Color output: auto | always | never (default: auto). Precedence: --color → TEMPER_COLOR → cli.color config → NO_COLOR → auto
  -h, --help                   Print help
```
