<!-- GENERATED — do not edit. Emitted from the built binary's `--help` by scripts/emit-cli-reference.py; see .github/scripts/check-cli-reference-drift.sh. -->

# `temper edge`

Assert or mutate a relationship between resources (writes go through the cloud API)

```text
Assert or mutate a relationship between resources (writes go through the cloud API)

Usage: temper edge [OPTIONS] <COMMAND>

Commands:
  assert    Assert a new relationship between two resources
  retype    Change the kind and polarity of an existing relationship
  reweight  Adjust the weight of an existing relationship
  fold      Retract (soft-delete) an existing relationship
  facet     Set a facet (typed property) on a relationship
  facets    List the live facets of a relationship
  help      Print this message or the help of the given subcommand(s)

Options:
      --vault <VAULT>      Path to vault (overrides TEMPER_VAULT and auto-detection)
      --format <FORMAT>    Output format: json | toon (default: toon on a TTY, json otherwise). Precedence: --format → TEMPER_FORMAT → cli.format config → TTY default
      --embed-threads <N>  ONNX intra-op threads for embedding. `0` = let ONNX Runtime decide. Default: this machine's performance-core count (NOT its total core count — efficiency cores measurably slow the batch down). Precedence: --embed-threads → TEMPER_ONNX_INTRA_THREADS → detected → 1
      --color <COLOR>      Color output: auto | always | never (default: auto). Precedence: --color → TEMPER_COLOR → cli.color config → NO_COLOR → auto
  -h, --help               Print help
```

### `temper edge assert`

```text
Assert a new relationship between two resources.

Sends a `POST /api/relationships` request. Returns a `edge_handle` that identifies the relationship chain for subsequent retype/reweight/fold.

Usage: temper edge assert [OPTIONS] --kind <KIND> --polarity <POLARITY> --label <LABEL> <SOURCE> <TARGET>

Arguments:
  <SOURCE>
          Source resource ref: a UUID or the decorated `slug-<uuid>` form

  <TARGET>
          Target resource ref: a UUID or the decorated `slug-<uuid>` form

Options:
      --kind <KIND>
          Edge kind (express, contains, leads-to, near)
          
          [possible values: express, contains, leads-to, near]

      --vault <VAULT>
          Path to vault (overrides TEMPER_VAULT and auto-detection)

      --format <FORMAT>
          Output format: json | toon (default: toon on a TTY, json otherwise). Precedence: --format → TEMPER_FORMAT → cli.format config → TTY default

      --polarity <POLARITY>
          Edge polarity (forward, inverse)
          
          [possible values: forward, inverse]

      --embed-threads <N>
          ONNX intra-op threads for embedding. `0` = let ONNX Runtime decide. Default: this machine's performance-core count (NOT its total core count — efficiency cores measurably slow the batch down). Precedence: --embed-threads → TEMPER_ONNX_INTRA_THREADS → detected → 1

      --label <LABEL>
          Human-readable label for the relationship (e.g. "depends_on")

      --color <COLOR>
          Color output: auto | always | never (default: auto). Precedence: --color → TEMPER_COLOR → cli.color config → NO_COLOR → auto

      --weight <WEIGHT>
          Edge weight (default: 1.0)
          
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

### `temper edge retype`

```text
Change the kind and polarity of an existing relationship.

Sends `POST /api/relationships/{edge_handle}/retype`.

Usage: temper edge retype [OPTIONS] --kind <KIND> --polarity <POLARITY> <EDGE_HANDLE>

Arguments:
  <EDGE_HANDLE>
          Correlation ID of the relationship to retype

Options:
      --kind <KIND>
          New edge kind
          
          [possible values: express, contains, leads-to, near]

      --vault <VAULT>
          Path to vault (overrides TEMPER_VAULT and auto-detection)

      --format <FORMAT>
          Output format: json | toon (default: toon on a TTY, json otherwise). Precedence: --format → TEMPER_FORMAT → cli.format config → TTY default

      --polarity <POLARITY>
          New edge polarity
          
          [possible values: forward, inverse]

      --embed-threads <N>
          ONNX intra-op threads for embedding. `0` = let ONNX Runtime decide. Default: this machine's performance-core count (NOT its total core count — efficiency cores measurably slow the batch down). Precedence: --embed-threads → TEMPER_ONNX_INTRA_THREADS → detected → 1

      --invocation <INVOCATION>
          Correlate this act with an open invocation envelope (its ref/UUID from `invocation open`)

      --color <COLOR>
          Color output: auto | always | never (default: auto). Precedence: --color → TEMPER_COLOR → cli.color config → NO_COLOR → auto

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

### `temper edge reweight`

```text
Adjust the weight of an existing relationship.

Sends `POST /api/relationships/{edge_handle}/reweight`.

Usage: temper edge reweight [OPTIONS] --weight <WEIGHT> <EDGE_HANDLE>

Arguments:
  <EDGE_HANDLE>
          Correlation ID of the relationship to reweight

Options:
      --vault <VAULT>
          Path to vault (overrides TEMPER_VAULT and auto-detection)

      --weight <WEIGHT>
          New weight value

      --format <FORMAT>
          Output format: json | toon (default: toon on a TTY, json otherwise). Precedence: --format → TEMPER_FORMAT → cli.format config → TTY default

      --invocation <INVOCATION>
          Correlate this act with an open invocation envelope (its ref/UUID from `invocation open`)

      --correlation <CORRELATION>
          Stitch this write into an act-grain thread shared with other writes (a bare UUID you mint). Provenance only — it never authorizes. Omit and the event self-roots

      --embed-threads <N>
          ONNX intra-op threads for embedding. `0` = let ONNX Runtime decide. Default: this machine's performance-core count (NOT its total core count — efficiency cores measurably slow the batch down). Precedence: --embed-threads → TEMPER_ONNX_INTRA_THREADS → detected → 1

      --color <COLOR>
          Color output: auto | always | never (default: auto). Precedence: --color → TEMPER_COLOR → cli.color config → NO_COLOR → auto

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

### `temper edge fold`

```text
Retract (soft-delete) an existing relationship.

Sends `POST /api/relationships/{edge_handle}/fold`.

Usage: temper edge fold [OPTIONS] <EDGE_HANDLE>

Arguments:
  <EDGE_HANDLE>
          Correlation ID of the relationship to fold

Options:
      --reason <REASON>
          Optional human-readable reason for folding

      --vault <VAULT>
          Path to vault (overrides TEMPER_VAULT and auto-detection)

      --format <FORMAT>
          Output format: json | toon (default: toon on a TTY, json otherwise). Precedence: --format → TEMPER_FORMAT → cli.format config → TTY default

      --invocation <INVOCATION>
          Correlate this act with an open invocation envelope (its ref/UUID from `invocation open`)

      --correlation <CORRELATION>
          Stitch this write into an act-grain thread shared with other writes (a bare UUID you mint). Provenance only — it never authorizes. Omit and the event self-roots

      --embed-threads <N>
          ONNX intra-op threads for embedding. `0` = let ONNX Runtime decide. Default: this machine's performance-core count (NOT its total core count — efficiency cores measurably slow the batch down). Precedence: --embed-threads → TEMPER_ONNX_INTRA_THREADS → detected → 1

      --color <COLOR>
          Color output: auto | always | never (default: auto). Precedence: --color → TEMPER_COLOR → cli.color config → NO_COLOR → auto

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

### `temper edge facet`

```text
Set a facet (typed property) on a relationship.

Sends `POST /api/relationships/{edge_handle}/facets`. A facet here qualifies the *link* — e.g. which clause of a goal a task's `advances` edge is evidence for — rather than either endpoint. Authorizes through the same clauses as retype/reweight/fold.

Usage: temper edge facet [OPTIONS] --values <VALUES> <EDGE_HANDLE>

Arguments:
  <EDGE_HANDLE>
          Correlation ID of the relationship to set the facet on

Options:
      --values <VALUES>
          The facet's typed value payload, as a JSON string

      --vault <VAULT>
          Path to vault (overrides TEMPER_VAULT and auto-detection)

      --format <FORMAT>
          Output format: json | toon (default: toon on a TTY, json otherwise). Precedence: --format → TEMPER_FORMAT → cli.format config → TTY default

      --weight <WEIGHT>
          Facet weight (default: 1.0)
          
          [default: 1.0]

      --embed-threads <N>
          ONNX intra-op threads for embedding. `0` = let ONNX Runtime decide. Default: this machine's performance-core count (NOT its total core count — efficiency cores measurably slow the batch down). Precedence: --embed-threads → TEMPER_ONNX_INTRA_THREADS → detected → 1

      --invocation <INVOCATION>
          Correlate this act with an open invocation envelope (its ref/UUID from `invocation open`)

      --color <COLOR>
          Color output: auto | always | never (default: auto). Precedence: --color → TEMPER_COLOR → cli.color config → NO_COLOR → auto

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

### `temper edge facets`

```text
List the live facets of a relationship.

Sends `GET /api/relationships/{edge_handle}/facets`. Folding the edge folds its facets, so every row returned belongs to a live relationship.

Usage: temper edge facets [OPTIONS] <EDGE_HANDLE>

Arguments:
  <EDGE_HANDLE>
          Correlation ID of the relationship to read

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
