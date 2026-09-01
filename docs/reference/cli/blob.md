<!-- GENERATED — do not edit. Emitted from the built binary's `--help` by scripts/emit-cli-reference.py; see .github/scripts/check-cli-reference-drift.sh. -->

# `temper blob`

Commit, read, list, and relate binary blobs (writes go through the cloud API)

```text
Commit, read, list, and relate binary blobs (writes go through the cloud API)

Usage: temper blob [OPTIONS] <COMMAND>

Commands:
  put     Commit a file's bytes as a blob, homed in a context or cogmap you can author
  get     Read a blob's bytes back, whole, streamed (to --out, or stdout)
  list    List the blobs you can read (optionally scoped to one home anchor)
  relate  Relate a blob to another anchor (resource by ref, or cogmap/blob id)
  help    Print this message or the help of the given subcommand(s)

Options:
      --vault <VAULT>      Path to vault (overrides TEMPER_VAULT and auto-detection)
      --format <FORMAT>    Output format: json | toon (default: toon on a TTY, json otherwise). Precedence: --format → TEMPER_FORMAT → cli.format config → TTY default
      --embed-threads <N>  ONNX intra-op threads for embedding. `0` = let ONNX Runtime decide. Default: this machine's performance-core count (NOT its total core count — efficiency cores measurably slow the batch down). Precedence: --embed-threads → TEMPER_ONNX_INTRA_THREADS → detected → 1
      --color <COLOR>      Color output: auto | always | never (default: auto). Precedence: --color → TEMPER_COLOR → cli.color config → NO_COLOR → auto
  -h, --help               Print help
```

### `temper blob put`

```text
Commit a file's bytes as a blob, homed in a context or cogmap you can author.

At or under the single-request threshold (BLOB_SINGLE_REQUEST_MAX_BYTES, default 4 MB — deliberately under the platform's 4.5 MB request cap) this is ONE multipart call. Beyond it the segmented upload takes over automatically: begin/append/finalize over chunks of the same size, each segment's sha256 carried as its idempotent-append identity, the whole file's sha256 echoed into finalize as the integrity check.

Usage: temper blob put [OPTIONS] --home <HOME> <FILE>

Arguments:
  <FILE>
          Path of the file to commit, or `-` for stdin (stdin commits have no filename, so pass --filename or --content-type)

Options:
      --home <HOME>
          The home anchor: a context ref (`@me/notes`, `+team/shared`), or a bare / decorated UUID id. A name-based ref is a CONTEXT home by construction; with a UUID id, --home-table picks (default context)

      --vault <VAULT>
          Path to vault (overrides TEMPER_VAULT and auto-detection)

      --format <FORMAT>
          Output format: json | toon (default: toon on a TTY, json otherwise). Precedence: --format → TEMPER_FORMAT → cli.format config → TTY default

      --home-table <HOME_TABLE>
          Which kind of anchor a UUID --home names (ignored for name-based refs)
          
          [default: context]
          [possible values: context, cogmap]

      --content-type <CONTENT_TYPE>
          The media type to commit under. Guessed from the extension for the six allowlisted types (png, jpeg, webp, svg, gif, pdf); required when the extension is unknown and the commit is not the guessed type — the server's allowlist refusal names the vocabulary if the guessed type is refused

      --embed-threads <N>
          ONNX intra-op threads for embedding. `0` = let ONNX Runtime decide. Default: this machine's performance-core count (NOT its total core count — efficiency cores measurably slow the batch down). Precedence: --embed-threads → TEMPER_ONNX_INTRA_THREADS → detected → 1

      --color <COLOR>
          Color output: auto | always | never (default: auto). Precedence: --color → TEMPER_COLOR → cli.color config → NO_COLOR → auto

      --filename <FILENAME>
          The filename the multipart part carries (defaults to the file's basename; required for stdin unless --content-type is given)

  -h, --help
          Print help (see a summary with '-h')
```

### `temper blob get`

```text
Read a blob's bytes back, whole, streamed (to --out, or stdout)

Usage: temper blob get [OPTIONS] <BLOB>

Arguments:
  <BLOB>  The blob's id (from `blob list` / a prior put)

Options:
      --out <OUT>          Write the bytes to this path instead of stdout
      --vault <VAULT>      Path to vault (overrides TEMPER_VAULT and auto-detection)
      --format <FORMAT>    Output format: json | toon (default: toon on a TTY, json otherwise). Precedence: --format → TEMPER_FORMAT → cli.format config → TTY default
      --embed-threads <N>  ONNX intra-op threads for embedding. `0` = let ONNX Runtime decide. Default: this machine's performance-core count (NOT its total core count — efficiency cores measurably slow the batch down). Precedence: --embed-threads → TEMPER_ONNX_INTRA_THREADS → detected → 1
      --color <COLOR>      Color output: auto | always | never (default: auto). Precedence: --color → TEMPER_COLOR → cli.color config → NO_COLOR → auto
  -h, --help               Print help
```

### `temper blob list`

```text
List the blobs you can read (optionally scoped to one home anchor)

Usage: temper blob list [OPTIONS]

Options:
      --home <HOME>              Optional home scope: a context ref, or a UUID id (with --home-table)
      --vault <VAULT>            Path to vault (overrides TEMPER_VAULT and auto-detection)
      --format <FORMAT>          Output format: json | toon (default: toon on a TTY, json otherwise). Precedence: --format → TEMPER_FORMAT → cli.format config → TTY default
      --home-table <HOME_TABLE>  [default: context] [possible values: context, cogmap]
      --embed-threads <N>        ONNX intra-op threads for embedding. `0` = let ONNX Runtime decide. Default: this machine's performance-core count (NOT its total core count — efficiency cores measurably slow the batch down). Precedence: --embed-threads → TEMPER_ONNX_INTRA_THREADS → detected → 1
      --color <COLOR>            Color output: auto | always | never (default: auto). Precedence: --color → TEMPER_COLOR → cli.color config → NO_COLOR → auto
  -h, --help                     Print help
```

### `temper blob relate`

```text
Relate a blob to another anchor (resource by ref, or cogmap/blob id).

The edge homes on the blob's home anchor; retraction rides `temper edge fold <handle>`. Re-asserting the same relation updates its weight and returns the same handle — a relation neither creates nor removes any other.

Usage: temper blob relate [OPTIONS] --to <TO> --label <LABEL> <BLOB>

Arguments:
  <BLOB>
          The blob's id

Options:
      --to <TO>
          The peer: a resource ref (UUID or `slug-<uuid>` — the common case), or a cogmap/blob id with --peer-table

      --vault <VAULT>
          Path to vault (overrides TEMPER_VAULT and auto-detection)

      --format <FORMAT>
          Output format: json | toon (default: toon on a TTY, json otherwise). Precedence: --format → TEMPER_FORMAT → cli.format config → TTY default

      --peer-table <PEER_TABLE>
          The peer's table when --to is a bare cogmap or blob id
          
          [default: resource]
          [possible values: resource, cogmap, blob]

      --direction <DIRECTION>
          Which end the blob occupies. blob-as-source is the `figure_of`-shaped act (the figure points at what it figures); blob-as-target is the derivation-source act (resource → blob, the file it was created from)
          
          [default: blob-as-source]
          [possible values: blob-as-source, blob-as-target]

      --embed-threads <N>
          ONNX intra-op threads for embedding. `0` = let ONNX Runtime decide. Default: this machine's performance-core count (NOT its total core count — efficiency cores measurably slow the batch down). Precedence: --embed-threads → TEMPER_ONNX_INTRA_THREADS → detected → 1

      --color <COLOR>
          Color output: auto | always | never (default: auto). Precedence: --color → TEMPER_COLOR → cli.color config → NO_COLOR → auto

      --kind <KIND>
          Edge kind (express, contains, leads-to, near)
          
          [default: express]
          [possible values: express, contains, leads-to, near]

      --polarity <POLARITY>
          Edge polarity (forward, inverse)
          
          [default: forward]
          [possible values: forward, inverse]

      --label <LABEL>
          Human-readable label (e.g. "figure_of", "derivation_source")

      --weight <WEIGHT>
          Edge weight
          
          [default: 1.0]

      --invocation <INVOCATION>
          Correlate this act with an open invocation envelope (its ref/UUID from `invocation open`)

      --correlation <CORRELATION>
          Stitch this write into an act-grain thread shared with other writes (a bare UUID you mint). Provenance only — it never authorizes. Omit and the event self-roots

      --confidence <CONFIDENCE>
          Graded authorship confidence: tentative, probable, or confident
          
          [possible values: tentative, probable, confident]

      --reasoning <REASONING>
          Free-text reasoning for the act (authorship; requires --confidence)

      --rationale <RATIONALE>
          Structured rationale for the act (authorship; requires --confidence)

      --persona <PERSONA>
          Persona/role the author acted as (authorship; requires --confidence)

      --model <MODEL>
          Model that authored the act (authorship; requires --confidence)

  -h, --help
          Print help (see a summary with '-h')
```
