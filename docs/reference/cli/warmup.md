<!-- GENERATED — do not edit. Emitted from the built binary's `--help` by scripts/emit-cli-reference.py; see .github/scripts/check-cli-reference-drift.sh. -->

# `temper warmup`

Context primer for new sessions — active goals, in-progress tasks, recent session pointers

```text
Context primer for new sessions — active goals, in-progress tasks, recent session pointers

Usage: temper warmup [OPTIONS] --context <CONTEXT>

Options:
      --context <CONTEXT>    Context ref (`@owner/slug` or UUID). Required — no context name is guaranteed to exist for a given principal, so there is no safe default
      --vault <VAULT>        Path to vault (overrides TEMPER_VAULT and auto-detection)
      --format <FORMAT>      Output format: json | toon (default: toon on a TTY, json otherwise). Precedence: --format → TEMPER_FORMAT → cli.format config → TTY default
      --sessions <SESSIONS>  How many recent sessions to surface. Precedence: --sessions → cli.warmup_sessions config → 5
      --embed-threads <N>    ONNX intra-op threads for embedding. `0` = let ONNX Runtime decide. Default: this machine's performance-core count (NOT its total core count — efficiency cores measurably slow the batch down). Precedence: --embed-threads → TEMPER_ONNX_INTRA_THREADS → detected → 1
      --goals <GOALS>        How many active goals to list. Precedence: --goals → cli.warmup_goals config → 20
      --color <COLOR>        Color output: auto | always | never (default: auto). Precedence: --color → TEMPER_COLOR → cli.color config → NO_COLOR → auto
  -h, --help                 Print help
```
