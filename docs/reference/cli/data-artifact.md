<!-- GENERATED — do not edit. Emitted from the built binary's `--help` by scripts/emit-cli-reference.py; see .github/scripts/check-cli-reference-drift.sh. -->

# `temper data-artifact`

List and show data artifacts owned by a resource

```text
List and show data artifacts owned by a resource

Usage: temper data-artifact [OPTIONS] <COMMAND>

Commands:
  list    List data artifacts owned by a resource
  show    Show a single data artifact by ID
  commit  Commit one data artifact to a resource
  help    Print this message or the help of the given subcommand(s)

Options:
      --vault <VAULT>      Path to vault (overrides TEMPER_VAULT and auto-detection)
      --format <FORMAT>    Output format: json | toon (default: toon on a TTY, json otherwise). Precedence: --format → TEMPER_FORMAT → cli.format config → TTY default
      --embed-threads <N>  ONNX intra-op threads for embedding. `0` = let ONNX Runtime decide. Default: this machine's performance-core count (NOT its total core count — efficiency cores measurably slow the batch down). Precedence: --embed-threads → TEMPER_ONNX_INTRA_THREADS → detected → 1
      --color <COLOR>      Color output: auto | always | never (default: auto). Precedence: --color → TEMPER_COLOR → cli.color config → NO_COLOR → auto
  -h, --help               Print help
```

### `temper data-artifact list`

```text
List data artifacts owned by a resource

Usage: temper data-artifact list [OPTIONS] <REF>

Arguments:
  <REF>  Resource ref: a UUID or the decorated `slug-<uuid>` form

Options:
      --kind <KIND>        Filter by the bare family name (e.g. `"measurement"`)
      --vault <VAULT>      Path to vault (overrides TEMPER_VAULT and auto-detection)
      --format <FORMAT>    Output format: json | toon (default: toon on a TTY, json otherwise). Precedence: --format → TEMPER_FORMAT → cli.format config → TTY default
      --intent <INTENT>    Filter by selection intent: `"current"`, `"member"`, or `"pinned"`
      --embed-threads <N>  ONNX intra-op threads for embedding. `0` = let ONNX Runtime decide. Default: this machine's performance-core count (NOT its total core count — efficiency cores measurably slow the batch down). Precedence: --embed-threads → TEMPER_ONNX_INTRA_THREADS → detected → 1
      --include-folded     Include folded (superseded) artifacts in the result. Default: false
      --color <COLOR>      Color output: auto | always | never (default: auto). Precedence: --color → TEMPER_COLOR → cli.color config → NO_COLOR → auto
      --counts             Return per-family counts instead of full artifacts. No content hydration
  -h, --help               Print help
```

### `temper data-artifact show`

```text
Show a single data artifact by ID

Usage: temper data-artifact show [OPTIONS] <REF> <ARTIFACT_ID>

Arguments:
  <REF>          Resource ref: a UUID or the decorated `slug-<uuid>` form
  <ARTIFACT_ID>  Artifact ID (UUID)

Options:
      --vault <VAULT>      Path to vault (overrides TEMPER_VAULT and auto-detection)
      --format <FORMAT>    Output format: json | toon (default: toon on a TTY, json otherwise). Precedence: --format → TEMPER_FORMAT → cli.format config → TTY default
      --embed-threads <N>  ONNX intra-op threads for embedding. `0` = let ONNX Runtime decide. Default: this machine's performance-core count (NOT its total core count — efficiency cores measurably slow the batch down). Precedence: --embed-threads → TEMPER_ONNX_INTRA_THREADS → detected → 1
      --color <COLOR>      Color output: auto | always | never (default: auto). Precedence: --color → TEMPER_COLOR → cli.color config → NO_COLOR → auto
  -h, --help               Print help
```

### `temper data-artifact commit`

```text
Commit one data artifact to a resource

Usage: temper data-artifact commit [OPTIONS] --kind <KIND> --intent <INTENT> <REF>

Arguments:
  <REF>  Resource ref: a UUID or the decorated `slug-<uuid>` form

Options:
      --kind <KIND>                The bare family name (e.g. `"measurement"`)
      --vault <VAULT>              Path to vault (overrides TEMPER_VAULT and auto-detection)
      --format <FORMAT>            Output format: json | toon (default: toon on a TTY, json otherwise). Precedence: --format → TEMPER_FORMAT → cli.format config → TTY default
      --intent <INTENT>            Selection intent: `"current"`, `"member"`, or `"pinned"`
      --embed-threads <N>          ONNX intra-op threads for embedding. `0` = let ONNX Runtime decide. Default: this machine's performance-core count (NOT its total core count — efficiency cores measurably slow the batch down). Precedence: --embed-threads → TEMPER_ONNX_INTRA_THREADS → detected → 1
      --precedence <PRECEDENCE>    Ordering among peers. Meaningful for `member`; carried for all. Default: 0.0 [default: 0]
      --color <COLOR>              Color output: auto | always | never (default: auto). Precedence: --color → TEMPER_COLOR → cli.color config → NO_COLOR → auto
      --content <CONTENT>          Content source: `@<path>` (file), `-` (stdin), or omitted for implicit stdin. The content must be valid JSON
      --supersedes <SUPERSEDES>    Artifact IDs this commit supersedes (UUIDs). Repeatable: `--supersedes <id> --supersedes <id>`
      --invocation <INVOCATION>    Correlate this act with an open invocation envelope (its ref/UUID from `invocation open`)
      --correlation <CORRELATION>  Stitch this write into an act-grain thread shared with other writes (a bare UUID you mint). Provenance only — it never authorizes. Omit and the event self-roots
      --confidence <CONFIDENCE>    Graded authorship confidence: tentative, probable, or confident [possible values: tentative, probable, confident]
      --reasoning <REASONING>      Free-text reasoning for the act (authorship; requires --confidence)
      --rationale <RATIONALE>      Structured rationale for the act (authorship; requires --confidence)
      --persona <PERSONA>          Persona/role the author acted as (authorship; requires --confidence)
      --model <MODEL>              Model that authored the act (authorship; requires --confidence)
  -h, --help                       Print help
```
