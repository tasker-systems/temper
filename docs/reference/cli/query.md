<!-- GENERATED — do not edit. Emitted from the built binary's `--help` by scripts/emit-cli-reference.py; see .github/scripts/check-cli-reference-drift.sh. -->

# `temper query`

Run a composed query — declared acts, piped, answered in one round trip

```text
Run a composed query — declared acts, piped, answered in one round trip

The plan is JSON. Source it like a resource body: `--plan @<path>` wins, `--plan -` always reads stdin, and a piped stdin is auto-detected. Unlike `resource update` there is no frontmatter-only case, so a missing plan is an error.

Usage: temper query [OPTIONS]

Options:
      --plan <PLAN>
          Plan source: `@<path>` to read a file, `-` to read stdin. Omit to auto-detect a pipe

      --vault <VAULT>
          Path to vault (overrides TEMPER_VAULT and auto-detection)

      --check
          Check the plan's shape locally and exit — no network, no token, no server consulted.
          
          Reports EXPRESSIBILITY: whether the plan is well-formed against the published contract. A clean result does not promise the server will run it; only the server knows what it has built. Refusals print to stdout as data and the exit code is non-zero.

      --format <FORMAT>
          Output format: json | toon (default: toon on a TTY, json otherwise). Precedence: --format → TEMPER_FORMAT → cli.format config → TTY default

      --embed-threads <N>
          ONNX intra-op threads for embedding. `0` = let ONNX Runtime decide. Default: this machine's performance-core count (NOT its total core count — efficiency cores measurably slow the batch down). Precedence: --embed-threads → TEMPER_ONNX_INTRA_THREADS → detected → 1

      --color <COLOR>
          Color output: auto | always | never (default: auto). Precedence: --color → TEMPER_COLOR → cli.color config → NO_COLOR → auto

  -h, --help
          Print help (see a summary with '-h')
```
