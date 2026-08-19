<!-- GENERATED — do not edit. Emitted from the built binary's `--help` by scripts/emit-cli-reference.py; see .github/scripts/check-cli-reference-drift.sh. -->

# `temper invocation`

Operate on agent-invocation envelopes (open / close / show / list)

```text
Operate on agent-invocation envelopes (open / close / show / list)

Usage: temper invocation [OPTIONS] <COMMAND>

Commands:
  open   Open an agent-invocation envelope. The server mints the id and returns it
  close  Close an open envelope with a terminal disposition and optional outcome
  show   Read one envelope plus its acts by ref
  list   List envelopes, optionally narrowed by originating cogmap and/or status
  help   Print this message or the help of the given subcommand(s)

Options:
      --vault <VAULT>      Path to vault (overrides TEMPER_VAULT and auto-detection)
      --format <FORMAT>    Output format: json | toon (default: toon on a TTY, json otherwise). Precedence: --format → TEMPER_FORMAT → cli.format config → TTY default
      --embed-threads <N>  ONNX intra-op threads for embedding. `0` = let ONNX Runtime decide. Default: this machine's performance-core count (NOT its total core count — efficiency cores measurably slow the batch down). Precedence: --embed-threads → TEMPER_ONNX_INTRA_THREADS → detected → 1
      --color <COLOR>      Color output: auto | always | never (default: auto). Precedence: --color → TEMPER_COLOR → cli.color config → NO_COLOR → auto
  -h, --help               Print help
```

### `temper invocation open`

```text
Open an agent-invocation envelope. The server mints the id and returns it

Usage: temper invocation open [OPTIONS] --cogmap <COGMAP> --trigger-kind <TRIGGER_KIND>

Options:
      --cogmap <COGMAP>              The originating cognitive map, by ref (UUID or `slug-<uuid>`)
      --vault <VAULT>                Path to vault (overrides TEMPER_VAULT and auto-detection)
      --format <FORMAT>              Output format: json | toon (default: toon on a TTY, json otherwise). Precedence: --format → TEMPER_FORMAT → cli.format config → TTY default
      --parent <PARENT>              Optional delegating-parent cogmap ref; omit when not spawned beneath another
      --embed-threads <N>            ONNX intra-op threads for embedding. `0` = let ONNX Runtime decide. Default: this machine's performance-core count (NOT its total core count — efficiency cores measurably slow the batch down). Precedence: --embed-threads → TEMPER_ONNX_INTRA_THREADS → detected → 1
      --trigger-kind <TRIGGER_KIND>  Free-form trigger label (e.g. `manual`, `delegated`, `scheduled`)
      --color <COLOR>                Color output: auto | always | never (default: auto). Precedence: --color → TEMPER_COLOR → cli.color config → NO_COLOR → auto
  -h, --help                         Print help
```

### `temper invocation close`

```text
Close an open envelope with a terminal disposition and optional outcome

Usage: temper invocation close [OPTIONS] --disposition <DISPOSITION> <INVOCATION>

Arguments:
  <INVOCATION>
          The invocation to close, by ref (the UUID returned by `open`)

Options:
      --disposition <DISPOSITION>
          Terminal disposition for the invocation.
          
          completed  — the run achieved its purpose
          failed     — the run errored or produced an unusable result
          abandoned  — the run was cancelled, aborted, or superseded
          
          There is no `cancelled` value: use `abandoned`.
          
          [possible values: completed, failed, abandoned]

      --vault <VAULT>
          Path to vault (overrides TEMPER_VAULT and auto-detection)

      --format <FORMAT>
          Output format: json | toon (default: toon on a TTY, json otherwise). Precedence: --format → TEMPER_FORMAT → cli.format config → TTY default

      --outcome <OUTCOME>
          Opaque, agent-defined terminal outcome as a JSON value; omit for none

      --embed-threads <N>
          ONNX intra-op threads for embedding. `0` = let ONNX Runtime decide. Default: this machine's performance-core count (NOT its total core count — efficiency cores measurably slow the batch down). Precedence: --embed-threads → TEMPER_ONNX_INTRA_THREADS → detected → 1

      --color <COLOR>
          Color output: auto | always | never (default: auto). Precedence: --color → TEMPER_COLOR → cli.color config → NO_COLOR → auto

  -h, --help
          Print help (see a summary with '-h')
```

### `temper invocation show`

```text
Read one envelope plus its acts by ref

Usage: temper invocation show [OPTIONS] <INVOCATION>

Arguments:
  <INVOCATION>  The invocation to read, by ref (UUID or `slug-<uuid>`)

Options:
      --vault <VAULT>      Path to vault (overrides TEMPER_VAULT and auto-detection)
      --format <FORMAT>    Output format: json | toon (default: toon on a TTY, json otherwise). Precedence: --format → TEMPER_FORMAT → cli.format config → TTY default
      --embed-threads <N>  ONNX intra-op threads for embedding. `0` = let ONNX Runtime decide. Default: this machine's performance-core count (NOT its total core count — efficiency cores measurably slow the batch down). Precedence: --embed-threads → TEMPER_ONNX_INTRA_THREADS → detected → 1
      --color <COLOR>      Color output: auto | always | never (default: auto). Precedence: --color → TEMPER_COLOR → cli.color config → NO_COLOR → auto
  -h, --help               Print help
```

### `temper invocation list`

```text
List envelopes, optionally narrowed by originating cogmap and/or status

Usage: temper invocation list [OPTIONS]

Options:
      --cogmap <COGMAP>    Optional originating cogmap ref to filter by; omit for all maps
      --vault <VAULT>      Path to vault (overrides TEMPER_VAULT and auto-detection)
      --format <FORMAT>    Output format: json | toon (default: toon on a TTY, json otherwise). Precedence: --format → TEMPER_FORMAT → cli.format config → TTY default
      --status <STATUS>    Optional lifecycle status filter: open | completed | failed | abandoned
      --embed-threads <N>  ONNX intra-op threads for embedding. `0` = let ONNX Runtime decide. Default: this machine's performance-core count (NOT its total core count — efficiency cores measurably slow the batch down). Precedence: --embed-threads → TEMPER_ONNX_INTRA_THREADS → detected → 1
      --color <COLOR>      Color output: auto | always | never (default: auto). Precedence: --color → TEMPER_COLOR → cli.color config → NO_COLOR → auto
  -h, --help               Print help
```
