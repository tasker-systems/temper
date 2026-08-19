<!-- GENERATED — do not edit. Emitted from the built binary's `--help` by scripts/emit-cli-reference.py; see .github/scripts/check-cli-reference-drift.sh. -->

# `temper pull`

Materialize a context's resources into the local read-only projection

```text
Materialize a context's resources into the local read-only projection

Usage: temper pull [OPTIONS] <CONTEXT>

Arguments:
  <CONTEXT>  Context name to pull

Options:
      --vault <VAULT>      Path to vault (overrides TEMPER_VAULT and auto-detection)
      --format <FORMAT>    Output format: json | toon (default: toon on a TTY, json otherwise). Precedence: --format → TEMPER_FORMAT → cli.format config → TTY default
      --embed-threads <N>  ONNX intra-op threads for embedding. `0` = let ONNX Runtime decide. Default: this machine's performance-core count (NOT its total core count — efficiency cores measurably slow the batch down). Precedence: --embed-threads → TEMPER_ONNX_INTRA_THREADS → detected → 1
      --color <COLOR>      Color output: auto | always | never (default: auto). Precedence: --color → TEMPER_COLOR → cli.color config → NO_COLOR → auto
  -h, --help               Print help
```
