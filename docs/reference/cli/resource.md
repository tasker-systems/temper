<!-- GENERATED — do not edit. Emitted from the built binary's `--help` by scripts/emit-cli-reference.py; see .github/scripts/check-cli-reference-drift.sh. -->

# `temper resource`

Manage resources (tasks, goals, sessions, research, concepts, decisions)

```text
Manage resources (tasks, goals, sessions, research, concepts, decisions)

Usage: temper resource [OPTIONS] <COMMAND>

Commands:
  create              Create a new resource
  list                List resources, optionally of a given type
  describe-open-meta  Describe the recognized open_meta conventions (the self-describing schema)
  show                Show a resource's content
  evidence            Show a resource's evidential-standing shape — the maturity vector (independence-discounted breadth, adversarial survival, contradiction balance, freshness) plus a lossy read-time `band` chip carried WITH the shape, never in place of it. Calls GET /evidence
  update              Update a resource's frontmatter and/or body
  annotate            Attach provenance sources to a resource's block — WITHOUT a body revise (issue #355)
  delete              Delete a resource (soft-delete via the API)
  reassign            Reassign a resource's owner (mis-attribution self-fix, or a team admin acting over a resource scoped to their team)
  grant               Grant a capability on a resource to a profile or team (system-admin, a can_grant holder, or the resource owner)
  revoke              Revoke a capability grant on a resource (system-admin, a can_grant holder, or the owner)
  facet               Set a facet property on a resource (cloud-mode-only API write)
  facets              List the live facets of a resource — one row per assert, with weights
  help                Print this message or the help of the given subcommand(s)

Options:
      --vault <VAULT>      Path to vault (overrides TEMPER_VAULT and auto-detection)
      --format <FORMAT>    Output format: json | toon (default: toon on a TTY, json otherwise). Precedence: --format → TEMPER_FORMAT → cli.format config → TTY default
      --embed-threads <N>  ONNX intra-op threads for embedding. `0` = let ONNX Runtime decide. Default: this machine's performance-core count (NOT its total core count — efficiency cores measurably slow the batch down). Precedence: --embed-threads → TEMPER_ONNX_INTRA_THREADS → detected → 1
      --color <COLOR>      Color output: auto | always | never (default: auto). Precedence: --color → TEMPER_COLOR → cli.color config → NO_COLOR → auto
  -h, --help               Print help
```

### `temper resource create`

```text
Create a new resource

Usage: temper resource create [OPTIONS] --type <TYPE>

Options:
      --type <TYPE>
          Resource type (task, goal, session, research, concept, decision)

      --vault <VAULT>
          Path to vault (overrides TEMPER_VAULT and auto-detection)

      --format <FORMAT>
          Output format: json | toon (default: toon on a TTY, json otherwise). Precedence: --format → TEMPER_FORMAT → cli.format config → TTY default

      --title <TITLE>
          Resource title

      --context <CONTEXT>
          Context ref (UUID or @owner/slug, e.g. @me/temper or +team/general). Mutually exclusive with --cogmap; specify exactly one home

      --embed-threads <N>
          ONNX intra-op threads for embedding. `0` = let ONNX Runtime decide. Default: this machine's performance-core count (NOT its total core count — efficiency cores measurably slow the batch down). Precedence: --embed-threads → TEMPER_ONNX_INTRA_THREADS → detected → 1

      --cogmap <COGMAP>
          Cognitive-map ref (UUID or decorated `slug-<uuid>`) to home the resource in. Mutually exclusive with --context; specify exactly one

      --color <COLOR>
          Color output: auto | always | never (default: auto). Precedence: --color → TEMPER_COLOR → cli.color config → NO_COLOR → auto

      --mode <MODE>
          Work mode: plan or build (task only)

      --effort <EFFORT>
          Work effort: small, medium, large (task only)

      --open-meta <OPEN_META>
          Open (caller-defined) frontmatter as a JSON object string, e.g. --open-meta '{"marker":"x","reviewed":true}'. These are the free-form "bring-your-own" fields; the closed temper-* vocabulary uses the typed flags (--mode/--effort/…). Must be a JSON object

      --goal <GOAL>
          Link this resource to a goal by ref (UUID or decorated `slug-<uuid>`). Projects a live `advances`→goal edge from the new resource on create

      --task <TASK>
          Link this session to a task by slug (session only). Asserts a session→task `advances` relationship after creation

      --show-template
          Print the raw template and exit

      --body <BODY>
          Body content: '@PATH' reads a file, '-' reads stdin, or omit to use piped stdin implicitly

      --from <FROM>
          Source path or http(s) URL — extract markdown via temper-ingest and use as body. Supported formats: md/markdown, txt/text, html/htm, pdf (text-layer PDFs only — a scanned or image-only PDF extracts nothing, as there is no OCR). Mutually exclusive with --body. http(s) URLs are fetched; a local file may be given as a plain path or a file:// URI

      --sources <SOURCES>
          Provenance sources this body was distilled from — comma-separated resource refs (UUID or decorated) and/or external http/https URLs. Each becomes a block-provenance record on the resource's body block (URLs via the 'remote' kind)

      --sources-as-edges
          Also assert a `derived_from` edge from the new resource to each resource-valued `--sources` entry. Remote URLs are skipped (no edge target).
          
          Not atomic: the edges are asserted after the create commits. A failed edge warns rather than failing the command — `edge assert` is idempotent, so re-asserting is safe, while re-running a create is not.

      --no-source
          Suppress the `--from <url>` provenance default. By default a URL `--from` sets the resource's origin and seeds a Remote block-provenance record from it (so `create --from <url>` is citation-grade with no extra flags); `--no-source` opts out, leaving the origin empty and recording no provenance. Mutually exclusive with `--sources`

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

### `temper resource list`

```text
List resources, optionally of a given type

Usage: temper resource list [OPTIONS]

Options:
      --type <TYPE>
          Resource type (task, goal, session, research, concept, decision). Optional — omit to list across every doc type, which is what makes a cross-type axis like `--tag` answerable in one call
      --vault <VAULT>
          Path to vault (overrides TEMPER_VAULT and auto-detection)
      --format <FORMAT>
          Output format: json | toon (default: toon on a TTY, json otherwise). Precedence: --format → TEMPER_FORMAT → cli.format config → TTY default
      --tag <TAG>
          Filter by tag (repeatable). `--tag a --tag b` returns resources carrying BOTH — each added tag narrows. Matching is exact and case-insensitive. Not doc-type-scoped: tags span every doc type, so this composes with `--type` or stands alone
      --context <CONTEXT>
          Filter by context ref (UUID or @owner/slug, e.g. @me/temper or +team/general)
      --embed-threads <N>
          ONNX intra-op threads for embedding. `0` = let ONNX Runtime decide. Default: this machine's performance-core count (NOT its total core count — efficiency cores measurably slow the batch down). Precedence: --embed-threads → TEMPER_ONNX_INTRA_THREADS → detected → 1
      --cogmap <COGMAP>
          Scope to resources homed in one or more cognitive maps (UUID or decorated ref). Repeatable — `--cogmap A --cogmap B` lists resources homed in either. Mutually exclusive with --context
      --color <COLOR>
          Color output: auto | always | never (default: auto). Precedence: --color → TEMPER_COLOR → cli.color config → NO_COLOR → auto
      --limit <LIMIT>
          Page size (default 20). A DEFAULT, not a cap: whatever you pass is honoured unchanged, and there is no server-side clamp. The response always carries `total` (the full match count), `returned`, and `truncated`, so a capped page is self-evident. Conflicts with --all
      --all
          Return ALL matching results (no page cap). Reach for this before asserting a set is complete or a resource is absent. Conflicts with --limit
      --offset <OFFSET>
          Skip the first N matching results (pagination). Conflicts with --page, which is the same axis counted in pages instead of rows
      --page <PAGE>
          Page number, 1-indexed — `--page 1` is the first page. Resolves to `(page - 1) * <effective limit>`, so it counts in whatever `--limit` is in force (`--page 3 --limit 5` starts at row 10, not 40). Conflicts with --offset (the same axis in rows) and with --all (an uncapped page has no page number)
      --sort <SORT>
          Sort as `<field>[:asc|desc]`. Fields: updated, created, title, stage, seq, context, doctype. Direction defaults per field (time/seq → desc, text → asc). Omit for the default `updated:desc`
      --title-contains <TITLE_CONTAINS>
          Filter to titles containing this substring (case-insensitive). A cheap way to narrow a large set instead of paging blind
      --stage <STAGE>
          Filter by stage (task only)
      --goal <GOAL>
          Filter by goal (task only)
      --status <STATUS>
          Filter by status (goal only)
      --with <WITH>
          Add a section to every row (comma-separated or repeated). `--with open-meta` fills the open metadata tier — the same envelope and the same row type as the default list, since asking for a section adds a part to the one shape rather than selecting a second one. The managed tier is always present either way. `body` is deliberately not offered here: a page of reconstructed bodies is an unbounded payload behind a flag that reads as cheap — use `show` per row [possible values: open-meta]
      --without <WITHOUT>
          Drop a section from every row (comma-separated or repeated). `list` asks for none by default, so this is only meaningful against a `--with` on the same invocation — and naming one section in both is a hard error, not a precedence rule [possible values: open-meta]
      --fields <FIELDS>
          Subselect top-level response keys on each row (anchor key always preserved). Use jq for nested projection
  -h, --help
          Print help
```

### `temper resource describe-open-meta`

```text
Describe the recognized open_meta conventions (the self-describing schema)

Prints the recognized open (caller-defined) frontmatter keys, their shapes, and — via each key's description — whether it is FTS-indexed (and at what weight) or shape-only, plus the discouraged bare keys. The open tier stays free-form; this is guidance, not a closed vocabulary. Mirrors the MCP `describe_open_meta` tool.

Usage: temper resource describe-open-meta [OPTIONS]

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

### `temper resource show`

```text
Show a resource's content

Usage: temper resource show [OPTIONS] <REF>

Arguments:
  <REF>  Resource ref: a UUID or the decorated `slug-<uuid>` form

Options:
      --edges              Show graph edges connected to this resource
      --vault <VAULT>      Path to vault (overrides TEMPER_VAULT and auto-detection)
      --format <FORMAT>    Output format: json | toon (default: toon on a TTY, json otherwise). Precedence: --format → TEMPER_FORMAT → cli.format config → TTY default
      --lineage            Show the resource's derived_from lineage — what it derives from (ancestors) and what derives from it (descendants), access-gated. Calls GET /lineage
      --embed-threads <N>  ONNX intra-op threads for embedding. `0` = let ONNX Runtime decide. Default: this machine's performance-core count (NOT its total core count — efficiency cores measurably slow the batch down). Precedence: --embed-threads → TEMPER_ONNX_INTRA_THREADS → detected → 1
      --provenance         Show itemized per-block provenance — the sources each of the resource's content blocks was distilled from. Calls GET /provenance
      --color <COLOR>      Color output: auto | always | never (default: auto). Precedence: --color → TEMPER_COLOR → cli.color config → NO_COLOR → auto
      --with <WITH>        Add a section (comma-separated or repeated). `show` already carries `body` and `open-meta`; `--with edges` folds this resource's graph edges into the same document (the long form of `--edges`) [possible values: body, open-meta, edges]
      --without <WITHOUT>  Drop a section (comma-separated or repeated). `--without body` is the cheap orientation read: everything `show` returns except the reconstructed markdown, and it composes freely with `--with edges`. Naming one section in both `--with` and `--without` is a hard error, not a precedence rule [possible values: body, open-meta, edges]
      --fields <FIELDS>    Subselect top-level response keys (the anchor key `id` is always preserved). Use jq for nested projection
  -h, --help               Print help
```

### `temper resource evidence`

```text
Show a resource's evidential-standing shape — the maturity vector (independence-discounted breadth, adversarial survival, contradiction balance, freshness) plus a lossy read-time `band` chip carried WITH the shape, never in place of it. Calls GET /evidence

Usage: temper resource evidence [OPTIONS] <REF>

Arguments:
  <REF>  Resource ref: a UUID or the decorated `slug-<uuid>` form

Options:
      --vault <VAULT>      Path to vault (overrides TEMPER_VAULT and auto-detection)
      --format <FORMAT>    Output format: json | toon (default: toon on a TTY, json otherwise). Precedence: --format → TEMPER_FORMAT → cli.format config → TTY default
      --embed-threads <N>  ONNX intra-op threads for embedding. `0` = let ONNX Runtime decide. Default: this machine's performance-core count (NOT its total core count — efficiency cores measurably slow the batch down). Precedence: --embed-threads → TEMPER_ONNX_INTRA_THREADS → detected → 1
      --color <COLOR>      Color output: auto | always | never (default: auto). Precedence: --color → TEMPER_COLOR → cli.color config → NO_COLOR → auto
  -h, --help               Print help
```

### `temper resource update`

```text
Update a resource's frontmatter and/or body

Mutates frontmatter from flag args. Optionally rewrites the body — the body trio (content + content_hash + chunks_packed) is PATCHed alongside any frontmatter changes in a single API call.

Body-source precedence, first match wins:

1. `--body @<path>` — read the file; stdin is ignored entirely. 2. `--body -` — read stdin explicitly (blocks; errors on a TTY or empty input). 3. implicit stdin — read stdin only when it is a non-TTY with input *ready* and non-empty (the `cat new.md | temper resource update <ref>` case); an idle or empty pipe is "no body". 4. none of the above — the body is left unchanged; only frontmatter is PATCHed.

FOOTGUN: implicit non-TTY stdin is a body rewrite. Do NOT run `update` inside a redirected loop (`while read n ref; do temper resource update "$ref" --title …; done < refs.txt`): every `update` inherits the loop's stdin (`refs.txt`) and rewrites the body with the leftover lines. For frontmatter-only edits (e.g. `--title`), invoke once per resource with stdin untouched; rewrite a body only via an explicit `cat file | temper resource update <ref>`.

Usage: temper resource update [OPTIONS] <REF>

Arguments:
  <REF>
          Resource ref: a UUID or the decorated `slug-<uuid>` form

Options:
      --type-to <TYPE_TO>
          New resource type (converts the resource)

      --vault <VAULT>
          Path to vault (overrides TEMPER_VAULT and auto-detection)

      --context-to <CONTEXT_TO>
          Move resource to a new context

      --format <FORMAT>
          Output format: json | toon (default: toon on a TTY, json otherwise). Precedence: --format → TEMPER_FORMAT → cli.format config → TTY default

      --embed-threads <N>
          ONNX intra-op threads for embedding. `0` = let ONNX Runtime decide. Default: this machine's performance-core count (NOT its total core count — efficiency cores measurably slow the batch down). Precedence: --embed-threads → TEMPER_ONNX_INTRA_THREADS → detected → 1

      --title <TITLE>
          Update title

      --color <COLOR>
          Color output: auto | always | never (default: auto). Precedence: --color → TEMPER_COLOR → cli.color config → NO_COLOR → auto

      --tags <TAGS>
          Add tag, keeping existing ones (repeatable). To replace or clear the list instead, use --open-meta '{"tags":[...]}' (or [] to clear)

      --aliases <ALIASES>
          Add alias, keeping existing ones (repeatable)

      --relates-to <RELATES_TO>
          Add relates-to reference, keeping existing ones (repeatable)

      --references <REFERENCES>
          Add reference, keeping existing ones (repeatable)

      --depends-on <DEPENDS_ON>
          Add depends-on reference, keeping existing ones (repeatable)

      --extends <EXTENDS>
          Add extends reference, keeping existing ones (repeatable)

      --preceded-by <PRECEDED_BY>
          Add preceded-by reference, keeping existing ones (repeatable)

      --derived-from <DERIVED_FROM>
          Add derived-from reference, keeping existing ones (repeatable)

      --open-meta <OPEN_META>
          Open (caller-defined) frontmatter as a JSON object string, e.g. --open-meta '{"marker":"x","reviewed":true}'. REPLACES each key it names — including lists, so '{"tags":[]}' clears tags. The repeatable flags above ADD instead; when both name a key, this replace lands first and the additions union on top. Free-form "bring-your-own" fields; temper-* keys use the typed flags. Must be a JSON object

      --open-meta-add <OPEN_META_ADD>
          Open (caller-defined) frontmatter to ADD, as a JSON object string of list-valued keys, e.g. --open-meta-add '{"reinforced":["2026-08-02"]}'. Mirrors --open-meta exactly but UNIONS each key over the stored list instead of replacing it, so accumulated history survives. This is the only way to add to a key the repeatable flags above do not name; every value must be a list (a scalar is refused rather than replacing the stored list). Where both this and a repeatable flag name one key, the two sets union

      --stage <STAGE>
          Task stage (backlog, in-progress, done, cancelled)

      --mode <MODE>
          Task mode (plan, build)

      --effort <EFFORT>
          Task effort (small, medium, large)

      --seq <SEQ>
          Task sequence number

      --branch <BRANCH>
          Git branch

      --pr <PR>
          Pull request URL

      --goal <GOAL>
          Set (or replace) the resource's goal by ref (UUID or decorated `slug-<uuid>`). Folds any existing `advances`→goal edge and asserts the new one. Conflicts with --clear-goal

      --clear-goal
          Clear the resource's goal — retract its `advances`→goal edge, leaving it goal-less. Conflicts with --goal

      --status <STATUS>
          Goal status (active, completed, paused, cancelled)

      --body <BODY>
          Body source, first match wins: `@<path>` reads a file (stdin ignored); `-` reads stdin explicitly (blocks; errors on a TTY or empty input); omit to auto-detect a *ready* non-TTY stdin pipe (an idle/empty pipe = no body). WARNING: implicit stdin is a body rewrite — never run this inside a `while read … done < file` loop (each call inherits the redirected file as its body); for frontmatter-only edits leave stdin untouched

      --sources <SOURCES>
          Provenance sources this body was distilled from — comma-separated resource refs (UUID or decorated) or http(s) URLs. Each becomes a block-provenance record on the addressed block. Requires a body update

      --content-block <CONTENT_BLOCK>
          Which content block the body revise + `--sources` target (a block UUID). Omit to address the resource's sole body block (the default); required to revise a resource that has more than one block. The block must belong to the resource and be non-folded

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

### `temper resource annotate`

```text
Attach provenance sources to a resource's block — WITHOUT a body revise (issue #355).

The annotate-only backfill: records block-provenance rows on the addressed block without re-chunking or re-embedding (body_hash and embeddings are unchanged), so a corpus imported without sources can be made citation-grade cheaply. Verify with `resource show --provenance`.

Span locators ride the source URL verbatim via a URL-fragment convention — e.g. `--sources 'https://example.com/doc.md#L120-L180'` records the line range and surfaces it in `--provenance` output (no schema change; the fragment is preserved end-to-end).

Usage: temper resource annotate [OPTIONS] --sources <SOURCES> <REF>

Arguments:
  <REF>
          Resource ref: a UUID or the decorated `slug-<uuid>` form

Options:
      --sources <SOURCES>
          Provenance sources to attach — comma-separated resource refs (UUID or decorated) or http(s) URLs (optionally with a `#L<start>-L<end>` locator fragment). At least one required. Each becomes a block-provenance record on the addressed block

      --vault <VAULT>
          Path to vault (overrides TEMPER_VAULT and auto-detection)

      --content-block <CONTENT_BLOCK>
          Which content block to annotate (a block UUID). Omit to address the resource's sole body block (the default); required for a resource that has more than one block. The block must belong to the resource and be non-folded

      --format <FORMAT>
          Output format: json | toon (default: toon on a TTY, json otherwise). Precedence: --format → TEMPER_FORMAT → cli.format config → TTY default

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

### `temper resource delete`

```text
Delete a resource (soft-delete via the API).

Sets `is_active = false` server-side; the row is preserved. Removing a projected file from disk with `rm` is just a local cache miss and has no server effect — run `temper resource delete` to actually delete, then `temper pull <context>` to re-materialize state on a fresh device. Delete is non-interactive on all surfaces — there is no confirmation prompt. `--force` is vestigial (a no-op holdover from the pre-cloud local-mode TTY gate); it is accepted for clarity but changes nothing.

Usage: temper resource delete [OPTIONS] <REF>

Arguments:
  <REF>
          Resource ref: a UUID or the decorated `slug-<uuid>` form

Options:
      --force
          Skip the local-file confirmation prompt

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

### `temper resource reassign`

```text
Reassign a resource's owner (mis-attribution self-fix, or a team admin acting over a resource scoped to their team).

Sends a `POST /api/resources/{id}/reassign` request via `temper-client`.

Usage: temper resource reassign [OPTIONS] --to <TO> <REF>

Arguments:
  <REF>
          Resource ref: a UUID or the decorated `slug-<uuid>` form

Options:
      --to <TO>
          Recipient profile UUID

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

### `temper resource grant`

```text
Grant a capability on a resource to a profile or team (system-admin, a can_grant holder, or the resource owner)

Usage: temper resource grant [OPTIONS] <REF>

Arguments:
  <REF>  Resource ref: a UUID or the decorated `slug-<uuid>` form

Options:
      --to-profile <TO_PROFILE>  Grant to this profile (UUID). Mutually exclusive with `--to-team`
      --vault <VAULT>            Path to vault (overrides TEMPER_VAULT and auto-detection)
      --format <FORMAT>          Output format: json | toon (default: toon on a TTY, json otherwise). Precedence: --format → TEMPER_FORMAT → cli.format config → TTY default
      --to-team <TO_TEAM>        Grant to this team: a team slug (optionally `+`-prefixed), a decorated `slug-<uuid>` ref, or a team UUID. Mutually exclusive with `--to-profile`
      --embed-threads <N>        ONNX intra-op threads for embedding. `0` = let ONNX Runtime decide. Default: this machine's performance-core count (NOT its total core count — efficiency cores measurably slow the batch down). Precedence: --embed-threads → TEMPER_ONNX_INTRA_THREADS → detected → 1
      --read                     Grant read
      --color <COLOR>            Color output: auto | always | never (default: auto). Precedence: --color → TEMPER_COLOR → cli.color config → NO_COLOR → auto
      --write                    Grant write (implies read)
      --grant                    Grant delegated-grant authority (implies read)
  -h, --help                     Print help
```

### `temper resource revoke`

```text
Revoke a capability grant on a resource (system-admin, a can_grant holder, or the owner)

Usage: temper resource revoke [OPTIONS] <REF>

Arguments:
  <REF>  Resource ref: a UUID or the decorated `slug-<uuid>` form

Options:
      --from-profile <FROM_PROFILE>  Revoke this profile's grant (UUID). Mutually exclusive with `--from-team`
      --vault <VAULT>                Path to vault (overrides TEMPER_VAULT and auto-detection)
      --format <FORMAT>              Output format: json | toon (default: toon on a TTY, json otherwise). Precedence: --format → TEMPER_FORMAT → cli.format config → TTY default
      --from-team <FROM_TEAM>        Revoke this team's grant: a team slug (optionally `+`-prefixed), a decorated `slug-<uuid>` ref, or a team UUID. Mutually exclusive with `--from-profile`
      --embed-threads <N>            ONNX intra-op threads for embedding. `0` = let ONNX Runtime decide. Default: this machine's performance-core count (NOT its total core count — efficiency cores measurably slow the batch down). Precedence: --embed-threads → TEMPER_ONNX_INTRA_THREADS → detected → 1
      --color <COLOR>                Color output: auto | always | never (default: auto). Precedence: --color → TEMPER_COLOR → cli.color config → NO_COLOR → auto
  -h, --help                         Print help
```

### `temper resource facet`

```text
Set a facet property on a resource (cloud-mode-only API write).

Sends a `POST /api/facets` request via `temper-client`.

Usage: temper resource facet [OPTIONS] --values <VALUES> <REF>

Arguments:
  <REF>
          Resource ref: a UUID or the decorated `slug-<uuid>` form

Options:
      --values <VALUES>
          The facet's typed value payload, as a JSON string

      --vault <VAULT>
          Path to vault (overrides TEMPER_VAULT and auto-detection)

      --format <FORMAT>
          Output format: json | toon (default: toon on a TTY, json otherwise). Precedence: --format → TEMPER_FORMAT → cli.format config → TTY default

      --weight <WEIGHT>
          Facet weight (default: 1.0)

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

### `temper resource facets`

```text
List the live facets of a resource — one row per assert, with weights.

Sends `GET /api/resources/{id}/facets`. This is the faithful view: `resource show` carries a facet inside `open_meta` collapsed to a single value, newest-wins, with the weight dropped and any sibling row hidden. `facet_set` appends rather than upserts, so a resource can legitimately carry several live facet rows — this is where you see all of them.

Not to be confused with the `facets` key in `resource list`'s response envelope, which is search-style aggregate counts over the listed set, not properties of a resource.

Usage: temper resource facets [OPTIONS] <REF>

Arguments:
  <REF>
          Resource ref: a UUID or the decorated `slug-<uuid>` form

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
