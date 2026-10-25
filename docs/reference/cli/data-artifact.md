<!-- GENERATED — do not edit. Emitted from the built binary's `--help` by scripts/emit-cli-reference.py; see .github/scripts/check-cli-reference-drift.sh. -->

# `temper data-artifact`

List and show data artifacts owned by a resource

```text
List and show data artifacts owned by a resource

Usage: temper data-artifact [OPTIONS] <COMMAND>

Commands:
  list    List data artifacts owned by a resource
  show    Show a single data artifact by ID
  get     Read a single data artifact by its own id — the flat read, no owning-resource address needed (`GET /api/data-artifacts/{id}`)
  ask     Ask one shape's artifacts what they hold, and print the matching artifacts, whole, one bounded page at a time (`POST /api/data-artifacts/ask`)
  commit  Commit one data artifact to a resource
  schema  Declare and inspect data-artifact shapes (the schema registry)
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
List data artifacts owned by a resource.

With `--limit`, answers one bounded page as `{"artifacts": [...], "next": {...} | null}`; continue by passing `next.created` and `next.artifact_id` back as `--after-created` and `--after-id`, with the same filters and `--order`. `next` is `null` on the last page. Prefer `--limit`. Without `--limit`, answers every artifact as a bare array; that unbounded default is retiring.

Usage: temper data-artifact list [OPTIONS] <REF>

Arguments:
  <REF>
          Resource ref: a UUID or the decorated `slug-<uuid>` form

Options:
      --kind <KIND>
          Filter by the bare family name (e.g. `"measurement"`)

      --vault <VAULT>
          Path to vault (overrides TEMPER_VAULT and auto-detection)

      --format <FORMAT>
          Output format: json | toon (default: toon on a TTY, json otherwise). Precedence: --format → TEMPER_FORMAT → cli.format config → TTY default

      --intent <INTENT>
          Filter by selection intent: `"current"`, `"member"`, or `"pinned"`

      --embed-threads <N>
          ONNX intra-op threads for embedding. `0` = let ONNX Runtime decide. Default: this machine's performance-core count (NOT its total core count — efficiency cores measurably slow the batch down). Precedence: --embed-threads → TEMPER_ONNX_INTRA_THREADS → detected → 1

      --include-folded
          Include folded (superseded) artifacts in the result. Default: false

      --color <COLOR>
          Color output: auto | always | never (default: auto). Precedence: --color → TEMPER_COLOR → cli.color config → NO_COLOR → auto

      --counts
          Return per-family counts instead of full artifacts. No content hydration. Not paged: conflicts with --limit, --order, --after-created and --after-id

      --limit <LIMIT>
          Answer one page of at most this many artifacts, 1 to 200, in the envelope `{"artifacts": [...], "next": ... | null}`. Out of range is refused, never clamped

      --order <ORDER>
          Direction of a bounded page over (created, id): asc, oldest first (the default), or desc, newest first. Requires --limit; keep it the same on every page of one walk
          
          [possible values: asc, desc]

      --after-created <AFTER_CREATED>
          Continue after this position: the previous page's `next.created` (RFC 3339), passed back exactly. Requires --limit and --after-id

      --after-id <AFTER_ID>
          Continue after this position: the previous page's `next.artifact_id`. Requires --limit and --after-created

  -h, --help
          Print help (see a summary with '-h')
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

### `temper data-artifact get`

```text
Read a single data artifact by its own id — the flat read, no owning-resource address needed (`GET /api/data-artifacts/{id}`).

The peer of `resource show`: works when the owning resource's address is not at hand, and answers folded (superseded) artifacts by their own id.

Usage: temper data-artifact get [OPTIONS] <ARTIFACT>

Arguments:
  <ARTIFACT>
          Artifact ref: a UUID or the decorated `slug-<uuid>` form

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

### `temper data-artifact ask`

```text
Ask one shape's artifacts what they hold, and print the matching artifacts, whole, one bounded page at a time (`POST /api/data-artifacts/ask`)

The ask names a shape by id and predicates over the paths that shape declares (read them with `data-artifact schema show <shape-id>`). Send it whole with `--ask`, or build it with `--shape` and one `--where` per predicate; the two forms send the same document. The answer is `{shape, artifacts, oversized, population, ordered_by, next, amended_since}`, oldest first. To count the first page's `population`, the page reads the whole population, outside its read budget. To continue, run the same ask with `--after` set to the previous page's `next`, passed back exactly; only `next: null` ends the answer, and a page may hold no artifacts yet carry a `next` when it stopped on its read budget. An artifact in `oversized` is too large or too deeply nested for a page (content more than 96 arrays or objects deep), and the page goes on: read it whole with `data-artifact get <artifact-id>`, though a client's own nesting limit can refuse that read too when the content nests very deeply, and on the hosted deployment that read is refused when the content, as storage serves it, is longer than the platform's response cap (storage prints some numbers longer than they were committed).

This is an enumeration, and `data-artifact list` is addressed. An ask reads across every resource you can see in the shape's home, and like listing and search it leaves out a resource still ingesting, or one whose ingest ended without finishing. `list <resource>` names one resource and answers its artifacts even then. To find the resources that own matching artifacts rather than the artifacts, use `temper query` with `find-resources-with`.

Usage: temper data-artifact ask [OPTIONS]

Options:
      --ask <ASK>
          The ask as JSON — `{"shape": "<id>", "predicates": [...], ...}` — inline, `@<path>` to read a file, or `-` to read stdin. Not with --shape or --where

      --vault <VAULT>
          Path to vault (overrides TEMPER_VAULT and auto-detection)

      --format <FORMAT>
          Output format: json | toon (default: toon on a TTY, json otherwise). Precedence: --format → TEMPER_FORMAT → cli.format config → TTY default

      --shape <SHAPE>
          The shape to ask, by id: the ask's `shape`, with no other field but the predicates --where gives

      --embed-threads <N>
          ONNX intra-op threads for embedding. `0` = let ONNX Runtime decide. Default: this machine's performance-core count (NOT its total core count — efficiency cores measurably slow the batch down). Precedence: --embed-threads → TEMPER_ONNX_INTRA_THREADS → detected → 1

      --where <PREDICATE>
          One predicate as JSON, `{"path": ["unit"], "op": {"op": "contains", "values": ["ms"]}}` — a path of object keys and an operator (`has_key`, `contains`, `compare`). Repeatable; the predicates AND. Requires --shape

      --color <COLOR>
          Color output: auto | always | never (default: auto). Precedence: --color → TEMPER_COLOR → cli.color config → NO_COLOR → auto

      --limit <LIMIT>
          How many matching artifacts the page examines, 1 to 200 (default 50). Out of range is refused, never clamped

      --after <AFTER>
          Continue after this position: the previous page's `next`, as JSON (`{"created": "...", "artifact_id": "..."}`), passed back exactly with the same ask

  -h, --help
          Print help (see a summary with '-h')
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
      --kind-owner <KIND_OWNER>    Namespace override for the family name: `kb_profiles:<uuid>` or `kb_teams:<uuid>`. Omit to let the server default it from the resource's home
      --embed-threads <N>          ONNX intra-op threads for embedding. `0` = let ONNX Runtime decide. Default: this machine's performance-core count (NOT its total core count — efficiency cores measurably slow the batch down). Precedence: --embed-threads → TEMPER_ONNX_INTRA_THREADS → detected → 1
      --intent <INTENT>            Selection intent: `"current"`, `"member"`, or `"pinned"`
      --color <COLOR>              Color output: auto | always | never (default: auto). Precedence: --color → TEMPER_COLOR → cli.color config → NO_COLOR → auto
      --precedence <PRECEDENCE>    Ordering among peers. Meaningful for `member`; carried for all. Default: 0.0 [default: 0]
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

### `temper data-artifact schema`

```text
Declare and inspect data-artifact shapes (the schema registry)

Usage: temper data-artifact schema [OPTIONS] <COMMAND>

Commands:
  list     List live shapes declared for a home anchor — what families are governed and how. Exactly one of `--context` / `--cogmap`: a shape's family is homed in one anchor kind
  show     Show a single shape by its ID — the schema, version, enforcement mode, and `askable`, every path an artifact ask may name
  declare  Declare a shape for a data-artifact family within a context or cognitive-map home
  help     Print this message or the help of the given subcommand(s)

Options:
      --vault <VAULT>      Path to vault (overrides TEMPER_VAULT and auto-detection)
      --format <FORMAT>    Output format: json | toon (default: toon on a TTY, json otherwise). Precedence: --format → TEMPER_FORMAT → cli.format config → TTY default
      --embed-threads <N>  ONNX intra-op threads for embedding. `0` = let ONNX Runtime decide. Default: this machine's performance-core count (NOT its total core count — efficiency cores measurably slow the batch down). Precedence: --embed-threads → TEMPER_ONNX_INTRA_THREADS → detected → 1
      --color <COLOR>      Color output: auto | always | never (default: auto). Precedence: --color → TEMPER_COLOR → cli.color config → NO_COLOR → auto
  -h, --help               Print help
```

#### `temper data-artifact schema list`

```text
List live shapes declared for a home anchor — what families are governed and how. Exactly one of `--context` / `--cogmap`: a shape's family is homed in one anchor kind

Usage: temper data-artifact schema list [OPTIONS]

Options:
      --context <CONTEXT>  Context ref: a UUID or the `@owner/slug` / `+team-slug/slug` form
      --vault <VAULT>      Path to vault (overrides TEMPER_VAULT and auto-detection)
      --cogmap <COGMAP>    Cognitive-map ref: a UUID or the decorated `slug-<uuid>` form. Mutually exclusive with --context
      --format <FORMAT>    Output format: json | toon (default: toon on a TTY, json otherwise). Precedence: --format → TEMPER_FORMAT → cli.format config → TTY default
      --embed-threads <N>  ONNX intra-op threads for embedding. `0` = let ONNX Runtime decide. Default: this machine's performance-core count (NOT its total core count — efficiency cores measurably slow the batch down). Precedence: --embed-threads → TEMPER_ONNX_INTRA_THREADS → detected → 1
      --color <COLOR>      Color output: auto | always | never (default: auto). Precedence: --color → TEMPER_COLOR → cli.color config → NO_COLOR → auto
  -h, --help               Print help
```

#### `temper data-artifact schema show`

```text
Show a single shape by its ID — the schema, version, enforcement mode, and `askable`, every path an artifact ask may name

Usage: temper data-artifact schema show [OPTIONS] <REF>

Arguments:
  <REF>  Shape ref: a UUID or the decorated `slug-<uuid>` form

Options:
      --vault <VAULT>      Path to vault (overrides TEMPER_VAULT and auto-detection)
      --format <FORMAT>    Output format: json | toon (default: toon on a TTY, json otherwise). Precedence: --format → TEMPER_FORMAT → cli.format config → TTY default
      --embed-threads <N>  ONNX intra-op threads for embedding. `0` = let ONNX Runtime decide. Default: this machine's performance-core count (NOT its total core count — efficiency cores measurably slow the batch down). Precedence: --embed-threads → TEMPER_ONNX_INTRA_THREADS → detected → 1
      --color <COLOR>      Color output: auto | always | never (default: auto). Precedence: --color → TEMPER_COLOR → cli.color config → NO_COLOR → auto
  -h, --help               Print help
```

#### `temper data-artifact schema declare`

```text
Declare a shape for a data-artifact family within a context or cognitive-map home.

Gated on authoring authority over the home. Exactly one of the positional context ref / `--cogmap`. The schema content is read from `--content @<path>`, `--content -` (stdin), or piped stdin — the same convention as `data-artifact commit`. The content must be a valid JSON Schema (draft 2020-12).

Usage: temper data-artifact schema declare [OPTIONS] --kind <KIND> [REF]

Arguments:
  [REF]
          Context ref: a UUID or the `@owner/slug` / `+team-slug/slug` form. Mutually exclusive with --cogmap — exactly one home

Options:
      --cogmap <COGMAP>
          Cognitive-map ref: a UUID or the decorated `slug-<uuid>` form. Mutually exclusive with the positional context ref

      --vault <VAULT>
          Path to vault (overrides TEMPER_VAULT and auto-detection)

      --format <FORMAT>
          Output format: json | toon (default: toon on a TTY, json otherwise). Precedence: --format → TEMPER_FORMAT → cli.format config → TTY default

      --kind <KIND>
          The bare family name (e.g. `"measurement"`)

      --embed-threads <N>
          ONNX intra-op threads for embedding. `0` = let ONNX Runtime decide. Default: this machine's performance-core count (NOT its total core count — efficiency cores measurably slow the batch down). Precedence: --embed-threads → TEMPER_ONNX_INTRA_THREADS → detected → 1

      --kind-owner <KIND_OWNER>
          Namespace override for the family name: `kb_profiles:<uuid>` or `kb_teams:<uuid>`. Omit to let the server default it from the home — which refuses on an empty context (no homed resource to default from)

      --color <COLOR>
          Color output: auto | always | never (default: auto). Precedence: --color → TEMPER_COLOR → cli.color config → NO_COLOR → auto

      --enforcement <ENFORCEMENT>
          Enforcement mode: `advisory` (default — non-conforming commits succeed and are recorded) or `enforcing` (non-conforming commits are refused)

          Possible values:
          - advisory:  Non-conforming commits succeed and are recorded as non-conforming
          - enforcing: Non-conforming commits are refused, and the refusal carries what failed
          
          [default: advisory]

      --content <CONTENT>
          Content source: `@<path>` (file), `-` (stdin), or omitted for implicit stdin. The content must be valid JSON (a JSON Schema draft 2020-12 document)

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
