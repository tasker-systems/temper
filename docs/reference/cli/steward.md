<!-- GENERATED — do not edit. Emitted from the built binary's `--help` by scripts/emit-cli-reference.py; see .github/scripts/check-cli-reference-drift.sh. -->

# `temper steward`

Team-self-cognition steward ingest trigger (delta / advance-watermark)

```text
Team-self-cognition steward ingest trigger (delta / advance-watermark)

Usage: temper steward [OPTIONS] <COMMAND>

Commands:
  delta              Read a team-self-cognition cogmap's ingest delta since its watermark, and whether it clears the threshold (i.e. the steward should run)
  advance-watermark  Record a completed run's cursors: how far it read, and what shape its scope had
  help               Print this message or the help of the given subcommand(s)

Options:
      --vault <VAULT>      Path to vault (overrides TEMPER_VAULT and auto-detection)
      --format <FORMAT>    Output format: json | toon (default: toon on a TTY, json otherwise). Precedence: --format → TEMPER_FORMAT → cli.format config → TTY default
      --embed-threads <N>  ONNX intra-op threads for embedding. `0` = let ONNX Runtime decide. Default: this machine's performance-core count (NOT its total core count — efficiency cores measurably slow the batch down). Precedence: --embed-threads → TEMPER_ONNX_INTRA_THREADS → detected → 1
      --color <COLOR>      Color output: auto | always | never (default: auto). Precedence: --color → TEMPER_COLOR → cli.color config → NO_COLOR → auto
  -h, --help               Print help
```

### `temper steward delta`

```text
Read a team-self-cognition cogmap's ingest delta since its watermark, and whether it clears the threshold (i.e. the steward should run)

Usage: temper steward delta [OPTIONS] <COGMAP>

Arguments:
  <COGMAP>  The team-self-cognition cogmap, by ref (UUID or `slug-<uuid>`)

Options:
      --threshold <THRESHOLD>  Ingest threshold to gate on; omit for the server default
      --vault <VAULT>          Path to vault (overrides TEMPER_VAULT and auto-detection)
      --format <FORMAT>        Output format: json | toon (default: toon on a TTY, json otherwise). Precedence: --format → TEMPER_FORMAT → cli.format config → TTY default
      --embed-threads <N>      ONNX intra-op threads for embedding. `0` = let ONNX Runtime decide. Default: this machine's performance-core count (NOT its total core count — efficiency cores measurably slow the batch down). Precedence: --embed-threads → TEMPER_ONNX_INTRA_THREADS → detected → 1
      --color <COLOR>          Color output: auto | always | never (default: auto). Precedence: --color → TEMPER_COLOR → cli.color config → NO_COLOR → auto
  -h, --help                   Print help
```

### `temper steward advance-watermark`

```text
Record a completed run's cursors: how far it read, and what shape its scope had.

Both are optional. Omitting the event leaves the watermark where it is — the correct invocation for a run that fired on a moved boundary with an empty event window, which has no event id to advance to and must still be able to record its boundary.

Usage: temper steward advance-watermark [OPTIONS] <COGMAP> [EVENT]

Arguments:
  <COGMAP>
          The team-self-cognition cogmap, by ref (UUID or `slug-<uuid>`)

  [EVENT]
          The `kb_events.id` (UUID) to advance the watermark to — the delta's `max_event_id`. Omit for a boundary-only advance (the delta's `max_event_id` was null)

Options:
      --boundary-fingerprint <BOUNDARY_FINGERPRINT>
          The `boundary_fingerprint` from the delta this run processed. Omit only if you have none: the server then settles the boundary to its shape at write time, which is still a settle but silently absorbs any boundary change during the run

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
