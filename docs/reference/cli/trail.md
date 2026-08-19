<!-- GENERATED — do not edit. Emitted from the built binary's `--help` by scripts/emit-cli-reference.py; see .github/scripts/check-cli-reference-drift.sh. -->

# `temper trail`

Read the event trail (append-only history) of a graph element — a resource node or a relationship edge.

```text
Read the event trail (append-only history) of a graph element — a resource node or a relationship edge.

Wraps the same access-gated ledger read the web UI's trail rail uses (`GET /api/graph/elements/{kind}/{id}/trail`): a time-ordered list of the events that produced and mutated the element, each with its actor, timestamp, confidence, and replay-sufficient payload. An element you cannot read (or that does not exist) yields an empty trail, never an error.

Usage: temper trail [OPTIONS] <KIND> <REF>

Arguments:
  <KIND>
          Which element to trail: `node` (a resource) or `edge` (a relationship)
          
          [possible values: node, edge]

  <REF>
          The element, by ref: a resource ref (UUID or decorated `slug-<uuid>`) for a node, or the edge's UUID for an edge. Only the trailing UUID is used

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
