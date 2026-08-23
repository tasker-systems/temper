<!-- GENERATED — do not edit. Emitted from the built binary's `--help` by scripts/emit-cli-reference.py; see .github/scripts/check-cli-reference-drift.sh. -->

# `temper graph`

Walk the knowledge graph — orient with no question, or move from where you are.

```text
Walk the knowledge graph — orient with no question, or move from where you are.

The CLI peer of the web graph surface's two reads. Both are access-gated: you see your own reach and nothing beyond it.

Usage: temper graph [OPTIONS] <COMMAND>

Commands:
  entry     Read what your work is built around — for a reader who has asked nothing
  traverse  Move from where you are — walk outward from resources you already have
  help      Print this message or the help of the given subcommand(s)

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

### `temper graph entry`

```text
Read what your work is built around — for a reader who has asked nothing.

Ranks the resources you can see by how connected they are and returns the most connected of them **plus every edge among them**, so nothing comes back pointing at something that was not drawn. The response declares its own bounds, including how many resources it did not draw for having no connections at all.

Wraps `GET /api/graph/entry`.

Usage: temper graph entry [OPTIONS]

Options:
      --in <IN>
          Confine the ranking to these places — a context or cogmap ref, repeatable.
          
          Omitted, the ranking runs across everything you can see. Named, it answers "a place, and no question at all" — ranking within the place rather than across the whole corpus.

      --vault <VAULT>
          Path to vault (overrides TEMPER_VAULT and auto-detection)

      --format <FORMAT>
          Output format: json | toon (default: toon on a TTY, json otherwise). Precedence: --format → TEMPER_FORMAT → cli.format config → TTY default

  -k <K>
          How many marks to draw. Unset lets the service's ruled default stand; the service caps it regardless and says so

      --embed-threads <N>
          ONNX intra-op threads for embedding. `0` = let ONNX Runtime decide. Default: this machine's performance-core count (NOT its total core count — efficiency cores measurably slow the batch down). Precedence: --embed-threads → TEMPER_ONNX_INTRA_THREADS → detected → 1

      --color <COLOR>
          Color output: auto | always | never (default: auto). Precedence: --color → TEMPER_COLOR → cli.color config → NO_COLOR → auto

  -h, --help
          Print help (see a summary with '-h')
```

### `temper graph traverse`

```text
Move from where you are — walk outward from resources you already have.

Walks your whole visible corpus from the given seeds; it is deliberately not confined to the result set of any earlier question. Each returned node carries its title, type, degree and an excerpt, so a walk answers without a `resource show` per node.

Wraps `GET /api/graph/traverse`.

Usage: temper graph traverse [OPTIONS] --from <FROM>

Options:
      --from <FROM>
          Seed to hop from — a resource ref, repeatable. At least one is required

      --vault <VAULT>
          Path to vault (overrides TEMPER_VAULT and auto-detection)

      --depth <DEPTH>
          Hops to walk (1..=3). Unset lets the service default it to 1.
          
          Out-of-range values are refused rather than clamped: the traversal response carries no bounds, so a clamped walk would be indistinguishable from the one you asked for.

      --format <FORMAT>
          Output format: json | toon (default: toon on a TTY, json otherwise). Precedence: --format → TEMPER_FORMAT → cli.format config → TTY default

      --embed-threads <N>
          ONNX intra-op threads for embedding. `0` = let ONNX Runtime decide. Default: this machine's performance-core count (NOT its total core count — efficiency cores measurably slow the batch down). Precedence: --embed-threads → TEMPER_ONNX_INTRA_THREADS → detected → 1

      --color <COLOR>
          Color output: auto | always | never (default: auto). Precedence: --color → TEMPER_COLOR → cli.color config → NO_COLOR → auto

  -h, --help
          Print help (see a summary with '-h')
```
