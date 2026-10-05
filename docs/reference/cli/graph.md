<!-- GENERATED — do not edit. Emitted from the built binary's `--help` by scripts/emit-cli-reference.py; see .github/scripts/check-cli-reference-drift.sh. -->

# `temper graph`

Walk the knowledge graph — orient with no question, or move from where you are.

```text
Walk the knowledge graph — orient with no question, or move from where you are.

The CLI peer of the web graph surface's two reads. Both are access-gated: you see your own reach and nothing beyond it.

Usage: temper graph [OPTIONS] <COMMAND>

Commands:
  entry                Read what your work is built around — for a reader who has asked nothing
  traverse             Move from where you are — walk outward from resources you already have
  home                 The atlas home: the contexts you build in and the cognitive maps you research in
  context-panorama     A context's panorama: its containers and their composition, with the residual grouped
  context-composition  Drill into a context: the subgraph composing one container, or one residual bucket
  region-composition   The subgraph composing one or more regions
  cogmap-panorama      A cognitive map's territory overview: territories, orphan nodes, and bridges
  cogmap-slice         The neighborhood of focus resources inside one cognitive map
  help                 Print this message or the help of the given subcommand(s)

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

### `temper graph home`

```text
The atlas home: the contexts you build in and the cognitive maps you research in.

Wraps `GET /api/graph/home`.

Usage: temper graph home [OPTIONS]

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

### `temper graph context-panorama`

```text
A context's panorama: its containers and their composition, with the residual grouped.

Wraps `GET /api/graph/contexts/panorama`.

Usage: temper graph context-panorama [OPTIONS] <CONTEXT>

Arguments:
  <CONTEXT>
          Context ref: `@me/<slug>`, `@<handle>/<slug>`, `+<team-slug>/<slug>`, or a UUID

Options:
      --group-by <GROUP_BY>
          Property key the residual tray groups by (the service defaults to `doc_type`)

      --vault <VAULT>
          Path to vault (overrides TEMPER_VAULT and auto-detection)

      --container-type <CONTAINER_TYPES>
          Doc-type treated as a container, repeatable (the service defaults to `goal`)

      --format <FORMAT>
          Output format: json | toon (default: toon on a TTY, json otherwise). Precedence: --format → TEMPER_FORMAT → cli.format config → TTY default

      --depth <DEPTH>
          Container-walk depth (the service defaults to 2 and clamps to 3)

      --embed-threads <N>
          ONNX intra-op threads for embedding. `0` = let ONNX Runtime decide. Default: this machine's performance-core count (NOT its total core count — efficiency cores measurably slow the batch down). Precedence: --embed-threads → TEMPER_ONNX_INTRA_THREADS → detected → 1

      --color <COLOR>
          Color output: auto | always | never (default: auto). Precedence: --color → TEMPER_COLOR → cli.color config → NO_COLOR → auto

  -h, --help
          Print help (see a summary with '-h')
```

### `temper graph context-composition`

```text
Drill into a context: the subgraph composing one container, or one residual bucket.

Wraps `GET /api/graph/contexts/composition`.

Usage: temper graph context-composition [OPTIONS] <CONTEXT>

Arguments:
  <CONTEXT>
          Context ref: `@me/<slug>`, `@<handle>/<slug>`, `+<team-slug>/<slug>`, or a UUID

Options:
      --container <CONTAINER>
          The container resource to drill (a resource ref)

      --vault <VAULT>
          Path to vault (overrides TEMPER_VAULT and auto-detection)

      --format <FORMAT>
          Output format: json | toon (default: toon on a TTY, json otherwise). Precedence: --format → TEMPER_FORMAT → cli.format config → TTY default

      --group <GROUP>
          The residual bucket to drill, as `<group_key>:<group_value>`

      --container-type <CONTAINER_TYPES>
          Doc-type treated as a container, repeatable (the service defaults to `goal`)

      --embed-threads <N>
          ONNX intra-op threads for embedding. `0` = let ONNX Runtime decide. Default: this machine's performance-core count (NOT its total core count — efficiency cores measurably slow the batch down). Precedence: --embed-threads → TEMPER_ONNX_INTRA_THREADS → detected → 1

      --color <COLOR>
          Color output: auto | always | never (default: auto). Precedence: --color → TEMPER_COLOR → cli.color config → NO_COLOR → auto

      --depth <DEPTH>
          Drill depth (the service defaults to 1 and clamps to 3)

      --container-depth <CONTAINER_DEPTH>
          Container-walk depth for a `--group` drill; must match the panorama's `--depth` (the service defaults to 2)

  -h, --help
          Print help (see a summary with '-h')
```

### `temper graph region-composition`

```text
The subgraph composing one or more regions.

Wraps `GET /api/graph/regions/composition`.

Usage: temper graph region-composition [OPTIONS] --region <REGIONS>

Options:
      --region <REGIONS>
          A region id, repeatable

      --vault <VAULT>
          Path to vault (overrides TEMPER_VAULT and auto-detection)

      --depth <DEPTH>
          Composition depth (the service defaults to 1 and clamps to 3)

      --format <FORMAT>
          Output format: json | toon (default: toon on a TTY, json otherwise). Precedence: --format → TEMPER_FORMAT → cli.format config → TTY default

      --embed-threads <N>
          ONNX intra-op threads for embedding. `0` = let ONNX Runtime decide. Default: this machine's performance-core count (NOT its total core count — efficiency cores measurably slow the batch down). Precedence: --embed-threads → TEMPER_ONNX_INTRA_THREADS → detected → 1

      --color <COLOR>
          Color output: auto | always | never (default: auto). Precedence: --color → TEMPER_COLOR → cli.color config → NO_COLOR → auto

  -h, --help
          Print help (see a summary with '-h')
```

### `temper graph cogmap-panorama`

```text
A cognitive map's territory overview: territories, orphan nodes, and bridges.

Wraps `GET /api/graph/cogmaps/{id}/panorama`.

Usage: temper graph cogmap-panorama [OPTIONS] <COGMAP>

Arguments:
  <COGMAP>
          Cognitive-map ref: a UUID or the decorated `slug-<uuid>` form

Options:
      --lens <LENS>
          Lens override (defaults to the map's primary lens)

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

### `temper graph cogmap-slice`

```text
The neighborhood of focus resources inside one cognitive map.

Wraps `POST /api/cogmaps/{id}/graph/slice`.

Usage: temper graph cogmap-slice [OPTIONS] --seed <SEEDS> <COGMAP>

Arguments:
  <COGMAP>
          Cognitive-map ref: a UUID or the decorated `slug-<uuid>` form

Options:
      --seed <SEEDS>
          Focus resource, repeatable; at least one

      --vault <VAULT>
          Path to vault (overrides TEMPER_VAULT and auto-detection)

      --depth <DEPTH>
          Walk depth from the seeds (the service clamps to 10)
          
          [default: 1]

      --format <FORMAT>
          Output format: json | toon (default: toon on a TTY, json otherwise). Precedence: --format → TEMPER_FORMAT → cli.format config → TTY default

      --edge-kind <EDGE_KINDS>
          Edge kind to walk, repeatable (snake_case, e.g. `leads_to`); none walks every kind

      --embed-threads <N>
          ONNX intra-op threads for embedding. `0` = let ONNX Runtime decide. Default: this machine's performance-core count (NOT its total core count — efficiency cores measurably slow the batch down). Precedence: --embed-threads → TEMPER_ONNX_INTRA_THREADS → detected → 1

      --color <COLOR>
          Color output: auto | always | never (default: auto). Precedence: --color → TEMPER_COLOR → cli.color config → NO_COLOR → auto

  -h, --help
          Print help (see a summary with '-h')
```
