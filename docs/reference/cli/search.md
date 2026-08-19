<!-- GENERATED — do not edit. Emitted from the built binary's `--help` by scripts/emit-cli-reference.py; see .github/scripts/check-cli-reference-drift.sh. -->

# `temper search`

Search the knowledge base

```text
Search the knowledge base

Usage: temper search [OPTIONS] <QUERY>

Arguments:
  <QUERY>  Search query text

Options:
      --context <CONTEXT>    Filter by context ref (UUID or @owner/slug, e.g. @me/temper or +team/general)
      --vault <VAULT>        Path to vault (overrides TEMPER_VAULT and auto-detection)
      --cogmap <COGMAP>      Scope search to a cognitive map (UUID or decorated ref). Mutually exclusive with --context. Search scopes to ONE anchor; asking several maps at once is a composition
      --format <FORMAT>      Output format: json | toon (default: toon on a TTY, json otherwise). Precedence: --format → TEMPER_FORMAT → cli.format config → TTY default
      --doc-type <DOC_TYPE>  Filter by document type
      --embed-threads <N>    ONNX intra-op threads for embedding. `0` = let ONNX Runtime decide. Default: this machine's performance-core count (NOT its total core count — efficiency cores measurably slow the batch down). Precedence: --embed-threads → TEMPER_ONNX_INTRA_THREADS → detected → 1
      --color <COLOR>        Color output: auto | always | never (default: auto). Precedence: --color → TEMPER_COLOR → cli.color config → NO_COLOR → auto
      --limit <LIMIT>        Maximum results (default 10)
      --offset <OFFSET>      Skip this many results. Applied per arm — the exact and wide arms page independently, because their quantities are incommensurable
      --within <WITHIN>      Narrow to specific resources, by ref (UUID or decorated `slug-<uuid>`). Repeatable. Composes with --context / --cogmap rather than replacing them
      --text-only            Use text-only search (no local embedding needed). The wide arm has no signal to run on without an embedding and will say so rather than returning an empty list
  -h, --help                 Print help
```
