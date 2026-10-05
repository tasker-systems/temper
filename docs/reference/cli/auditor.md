<!-- GENERATED — do not edit. Emitted from the built binary's `--help` by scripts/emit-cli-reference.py; see .github/scripts/check-cli-reference-drift.sh. -->

# `temper auditor`

Auditor worker doors: claim citation-audit jobs, complete them, survey coverage

```text
Auditor worker doors: claim citation-audit jobs, complete them, survey coverage

Usage: temper auditor [OPTIONS] <COMMAND>

Commands:
  dispatch  Claim a batch of auditor jobs for one tick
  complete  Complete your in-flight audit job on a cognitive map. Carries no outcome: the verdicts live in the audit trail the session wrote
  sweep     Survey audit coverage across the findings you can read
  help      Print this message or the help of the given subcommand(s)

Options:
      --vault <VAULT>      Path to vault (overrides TEMPER_VAULT and auto-detection)
      --format <FORMAT>    Output format: json | toon (default: toon on a TTY, json otherwise). Precedence: --format → TEMPER_FORMAT → cli.format config → TTY default
      --embed-threads <N>  ONNX intra-op threads for embedding. `0` = let ONNX Runtime decide. Default: this machine's performance-core count (NOT its total core count — efficiency cores measurably slow the batch down). Precedence: --embed-threads → TEMPER_ONNX_INTRA_THREADS → detected → 1
      --color <COLOR>      Color output: auto | always | never (default: auto). Precedence: --color → TEMPER_COLOR → cli.color config → NO_COLOR → auto
  -h, --help               Print help
```

### `temper auditor dispatch`

```text
Claim a batch of auditor jobs for one tick.

Wraps `POST /api/auditor/dispatch`. Claims are real: a claimed job is yours until you complete it or it times out.

Usage: temper auditor dispatch [OPTIONS]

Options:
      --cap <CAP>
          Max jobs to claim (the server clamps; omit for its default)

      --vault <VAULT>
          Path to vault (overrides TEMPER_VAULT and auto-detection)

      --correlation-id <CORRELATION_ID>
          A per-tick correlation id, stamped onto every claimed job and echoed back

      --format <FORMAT>
          Output format: json | toon (default: toon on a TTY, json otherwise). Precedence: --format → TEMPER_FORMAT → cli.format config → TTY default

      --embed-threads <N>
          ONNX intra-op threads for embedding. `0` = let ONNX Runtime decide. Default: this machine's performance-core count (NOT its total core count — efficiency cores measurably slow the batch down). Precedence: --embed-threads → TEMPER_ONNX_INTRA_THREADS → detected → 1

      --color <COLOR>
          Color output: auto | always | never (default: auto). Precedence: --color → TEMPER_COLOR → cli.color config → NO_COLOR → auto

  -h, --help
          Print help (see a summary with '-h')
```

### `temper auditor complete`

```text
Complete your in-flight audit job on a cognitive map. Carries no outcome: the verdicts live in the audit trail the session wrote.

Wraps `POST /api/auditor/{cogmap}/complete`.

Usage: temper auditor complete [OPTIONS] <COGMAP>

Arguments:
  <COGMAP>
          Cognitive-map ref: a UUID or the decorated `slug-<uuid>` form

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

### `temper auditor sweep`

```text
Survey audit coverage across the findings you can read.

Wraps `GET /api/auditor/sweep`.

Usage: temper auditor sweep [OPTIONS]

Options:
      --cap <CAP>
          Max findings to report (the server clamps; omit for its default)

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
