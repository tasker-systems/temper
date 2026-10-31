# Data Artifacts — structured data with a home

Read this when **about to store structured data** — a computation output, a measurement, a query
plan, structured inputs or outputs — and deciding whether to write it into a resource body or
commit it as a data artifact.

## The problem this solves

Structured data produced by one session is read by a later, unrelated session. Today that data
lives in fenced code blocks inside resource bodies, and the system treats it as prose: the chunker
splits it at its own comment lines, embeds the fragments into a search corpus built for sentences,
and hands the next reader a reassembly puzzle with no shape declared.

Data artifacts give structured data a home of its own — owned by a resource, reached through the
graph, and handed to the next session intact.

## The why-anchor

This protects the **next session's** attention. The writer and the reader are different actors
separated in time — they share no context, cannot negotiate, and the reader has only what was
stored. Data artifacts exist so that what survives the gap is the data itself, whole, not a fence
inside prose that the system shredded.

If a change to how you use data artifacts stops serving the session that did **not** write the data,
it has drifted.

## When to commit a data artifact vs. writing into a resource body

**Commit a data artifact when:**
- The content is structured (JSON, YAML, a computation output, a measurement, a query plan).
- The reader is a later session that needs it whole, without reconstruction or inference.
- Embedding it into the search corpus would be semantic noise.

**Write into the resource body when:**
- The content is prose (session notes, problem statements, design rationale).
- The reader is a human or an agent reading for understanding.
- The content belongs in the searchable corpus.

**The trap: data artifacts are not "just another way to store JSON."** The distinction is not about
format — it is about whether the content is data that a later session must retrieve whole, or prose
that a reader must understand. A fenced JSON block in a session note is prose with a shape; a data
artifact is the shape without the prose. Teaching data artifacts as "another storage option"
collapses the why-anchor into a syntax choice and loses the thing the surface exists to protect.

## The selection vocabulary

When a resource owns several artifacts of one family, a reader determines which to take from the
stored record alone — with no out-of-band knowledge and no local convention.

- **`kind`** — the bare family name (e.g. `"measurement"`, `"query-plan"`). Free-form, but must be
  consistent within a family.
- **`intent`** — `current`, `member`, or `pinned`. A **closed vocabulary**: the system refuses an
  unrecognized intent, and the refusal carries the vocabulary so the caller can correct.
  - `current` — this artifact is the one to take by default for its family.
  - `member` — this artifact is one of several, ordered by `precedence`.
  - `pinned` — this artifact is explicitly held regardless of other artifacts.
- **`precedence`** — ordering among peers. Meaningful for `member`; carried for all. Default: `0`.
- **`supersedes`** — artifact IDs this commit replaces. The system folds superseded artifacts out
  of the default list; `--include-folded` / `include_folded: true` returns them. Supersession is
  never inferred from recency or ordering — a writer must declare it.

## Shape state

Every artifact carries a `shape_state`:

- `never_declared` — no shape has been declared for this family in the artifact's home.
- `declared_satisfied` — a shape is in force and the artifact conforms to it.
- `declared_not_satisfied` — a shape is in force and the artifact does not conform.
- `declared_not_yet_checked` — a shape was declared but the validation sweep has not reached this
  artifact yet.

The absence of a shape is a **first-class state**, not a degraded one. An actor can commit
structured data without first declaring its shape, and that is a first-class act — the system does
not require a prior declaration to persist data. `never_declared` is reported as a typed value, not
as a silent "looks fine" — a `""` or `NULL` is a decode error, not a default.

A stored verdict is trusted **only** while its `(shape_id, shape_version, content_hash)` triple
matches the currently governing shape and the artifact's current content. Anything else reads as
`declared_not_yet_checked`. This means staleness can never be misread as conformance — it reads as
unchecked, which is honest.

## Declaring a shape

A shape is a **field tree**: a subset of JSON Schema 2020-12 that uses only a small, fixed
vocabulary. Declaring is a separate act from committing data — you never need to declare a shape to
commit an artifact.

A field tree uses only `type`, `properties`, `required`, `additionalProperties` (`true`, `false` or a
schema), `items` (one schema), `enum` (at most 256 strings, or 32 values of any other kind), `const`,
`pattern` (at most 256 bytes of source, compiling within 10 KiB, at most 32 in a shape, no lookaround or backreferences; a Unicode class such as `\p{L}` is about 43 KB on its own and does not fit), `minLength`,
`maxLength`, `minItems`, `maxItems`, `minimum`, `maximum`, `title`, `description`, `$comment`,
`default` and `examples`. Property names (in `properties` and `required`) are at most 256 bytes.
`$schema` and `$id` may appear only at the root, and `$schema`, if present, must be
`https://json-schema.org/draft/2020-12/schema` (a trailing `#` is also accepted). A shape may nest at most 16 levels.
A `const` and every `enum` member must be a scalar: a string of at most 256 bytes (JSON-escaped), a
number, a boolean or `null`; an object or an array is refused. A shape's schema holds at most 12,288
JSON values in all, at most 3,072 of them objects, counted across the whole document with `default`
and `examples` included. That budget can refuse a schema every other cap allows: 48 enums of 256
strings each fit the per-keyword caps but not the budget. The HTTP declare routes also take a
request body of at most 512 KiB.
`$ref`, `$defs`, `allOf`, `anyOf`, `oneOf`, `not`, `if`, `format` and every other keyword are
refused at declare, with a sentence that lists this vocabulary. A refused commit, and a recorded
verdict, name at most one violation, its message cut to 1 KiB.

**Declaring is never required to commit.** The system persists your data whether or not a shape
exists. `never_declared` is a first-class state, not a gate. If you declare a shape later, the
system reconciles existing artifacts asynchronously — they read as `declared_not_yet_checked` until
the sweep reaches them, never as silently conforming.

**A shape governs its home.** The shape in force for a family is keyed per home (a context or a
cogmap), not globally. So the same family can carry different shapes in different contexts — a
`measurement` in one context may have a different schema than a `measurement` in another. This is
by construction: a shape declared in a context you cannot read must not verdict your data. When a
resource is rehomed, the governing shape changes with it, and the artifact reads as unchecked until
reconciled against the new home's shape.

### Enforcement modes

A shape carries an **enforcement mode** — a closed vocabulary, like `intent`:

- **`advisory`** (default) — a non-conforming commit **succeeds** and is recorded as
  `declared_not_satisfied`. The shape informs; it does not gate.
- **`enforcing`** — a non-conforming commit is **refused**, and the refusal carries what failed.

`advisory` is the default because the registry exists to make shapes **discoverable and
informative**, not to force a writer to be right up front. An `enforcing` shape is an opt-in
coercive contract. Shapes are not "just schema validation" — the why-anchor is still that the writer
and the reader are different sessions separated in time, and a shape tells the reader whether the
data they retrieved conforms to anything the writer declared.

## Asking — two arms, one ask

A declared shape makes its family **askable**: you can ask its artifacts what they hold, in the
paths the shape declares. One ask has two arms, and which you want depends on what you need back:

- **the resources that own matching artifacts** — `find-resources-with`'s `artifact_filter`, in a
  query composition. The answer is resource ids, which pipe into the next stage like any selection.
- **the matching artifacts themselves, whole** — the batch read, below. The answer is artifacts,
  never resources, and it is not a stage of a composition.

The ask is the same object on both arms: a shape by id, and predicates over that shape's paths.

## Asking which resources hold matching data

`find-resources-with`, the selection act of a query composition, takes `artifact_filter`: a list of
asks, each naming a shape by id and predicates over
the paths that shape declares, with the same closed operators a property predicate uses
(`has_key`, `contains`, `compare`). A resource is selected when, for **every** ask, one of its
artifacts satisfies **all** of that ask's predicates. So "an artifact with `unit = "ms"` and
`count > 10`" is one ask with two predicates, and "one artifact with `unit = "ms"` and another with
`count > 10`" is two asks.

- **Read the vocabulary first.** A shape's by-id read carries `askable`: every path it declares and
  the operators each admits. An ask naming a path, operator or value outside its shape is refused
  before anything runs, and the refusal's `teaches` carries the vocabulary to correct it from.
- **A value outside a declared enum is refused, and the refusal names the route to it.** Stored
  content is not closed by its shape — an advisory shape records non-conforming content, artifacts
  committed before the shape stay `declared_not_yet_checked`, and an amendment can narrow an enum
  over older data — so `value_outside_shape` says the shape does not declare the value, not that
  nothing matches. Its `teaches` carries the declared values and the route: ask `has_key` at the
  path with `shape_states: ["declared_not_satisfied", "declared_not_yet_checked"]` in the batch
  read, which returns those artifacts whole.
- **Below a path that declares values, a `contains` ask is refused.** A shape's `const` and `enum`
  members are scalars, which hold nothing at any key, so every path below a node that declares values
  holds the empty set: a `contains` ask there is refused `value_outside_shape`, while `compare` and
  `has_key` are admitted. A shape whose schema the compile refuses cannot be asked: the ask answers
  `shape_schema_uncompilable` until the shape is amended; re-declaring it returns the refusal
  sentence, which names every bound a shape is held to.
- **An ask ranges over its shape's home.** It reads the live artifacts of that shape's family on
  resources homed there that you can see and that have finished ingesting. Anchor the stage to that
  home or leave it unanchored; asks of shapes in two homes cannot share a stage (`ask_scope_conflict`)
  — ask each home in its own stage and `union` them.
- **You must be able to read the shape's home.** A shape you cannot read is refused exactly as one
  that does not exist (`shape_not_askable`), even when you can read some of its resources directly
  and so list their artifacts.
- **Live artifacts only.** `include_folded: true` on an ask is refused by name.
- **The answer is resource ids.** The stage pipes them into the next stage like any selection; read
  the artifacts themselves through the resources it found.
- **An empty stage says why in its trace entry.** The selection's entry in `trace.stages` carries
  `artifact_asks`: for each ask, in filter order, the shape version it was validated against, the
  version `in_force` when it ran, and the `population` it counted — how many governed artifacts are
  `declared_satisfied`, `declared_not_satisfied` and `declared_not_yet_checked`, and how many
  `matched`. All zeros: nothing governed by the shape is in what you can see (and with `in_force`
  null, you can no longer read the shape's home). A population with `matched: 0`: some is, and
  none matched. `matched` above zero beside an empty stage: matches exist, and another ask (asks
  AND on one resource) or the stage's `resource_filter` excluded their resources.
- **The by-state counts ignore your `shape_states`; `matched` applies the whole ask.** So an ask
  narrowed to `declared_satisfied` while the validation sweep is behind still names the excluded
  `declared_not_yet_checked` rows by count — a small answer reads as lag, not absence. Each ask
  counts its own population: two asks' counts do not add or intersect into the stage's answer.

{%- if surface == "cli" %}

Send the plan with `temper query`. `temper query --check` checks an ask's form offline but loads no
shape, so it cannot say whether a path lies inside one; its report lists those shapes under
`shapes_not_checked`. A refused plan, under the JSON output format, prints its refusals in the error
payload's `details.refusals`, each with its `reason` and `teaches`.
{%- else %}

Send the plan with `run_query`. A refused plan returns `invalid_params`; the error's `data` carries
`{"refusals": [...]}`, each refusal with its `reason` and `teaches`.
{%- endif %}

## Reading the matching artifacts themselves

The batch read answers one ask with the matching artifacts, whole: the same ask object, sent
alone. It reads what `find-resources-with` reads — the live artifacts of the shape's family on
resources homed in the shape's home that you can see — and the same refusals apply, every one at
once. A shape you cannot read is refused exactly as one that does not exist.

- **Pages, oldest first.** A page holds at most `limit` matching artifacts (1 to 200, default 50),
  in `(created, artifact_id)` order — enumeration order, never rank (`ordered_by: "created"`). The
  answer is `{shape, artifacts, oversized, population, ordered_by, next, amended_since}`.
- **Continue with `next`.** Send the same ask again with the page's `next` as `after`, unchanged.
  Only `next: null` ends the enumeration. A page also stops once it has read as much as one page
  may read to find its matches, and then `next` is the last artifact it read, matched or not — so
  a page can hold no artifacts and still carry a `next`. An empty page with a `next` means keep
  going, never "nothing matches". The position carries no authority: every page checks the ask
  again for you.
- **`oversized` is fetched by id.** A page is also bounded in bytes, and in how deeply its content
  nests, so that temper's own clients parse every page. A matching artifact too large for a page, or
  whose content nests more than 96 arrays or objects deep, is named in `oversized`, in order, by
  id, resource and size, without its content; read it whole by id. It does not end the page. A
  client's own nesting limit can refuse even that read of content nested very deeply, and on the
  hosted deployment the read is refused when the response carrying the artifact, with its content
  as storage serves it, is longer than the platform's response cap (storage prints some numbers
  longer than they were committed).
- **`population` is on the first page only**: the ask's governed artifacts by shape state, and how
  many matched — the same counts `find-resources-with`'s trace gives. Later pages carry `null`.
  The counts are exact, so to make them the first page matches every governed artifact in the home;
  only the page's own rows are bounded by what a page may read.
- **`amended_since`.** If the shape is amended between your pages, later pages are still validated
  against the version your first page named, and `amended_since` names the version now in force.
  But each row's `shape_state`, and any `shape_states` you narrowed by, are read against the
  version now in force, which reconciliation may not have checked yet: a narrowed ask can find
  nothing on the pages after an amendment. When `amended_since` appears and you narrowed by
  `shape_states`, start again from a first page, asking the version it names. A first page (no
  `after`) naming the folded version is refused `shape_superseded`, teaching its successor.
- **An artifact with no content** is returned only by an ask with no predicates, and is served
  without a `content` field.
- **The enumeration floor.** Listing a resource's artifacts is an *addressed* read: it names one
  resource and answers its artifacts whatever that resource's state. Asking is an *enumeration*
  across every resource you can see in the home, and like listing resources and search it leaves out
  a resource still ingesting, or one whose ingest ended without finishing. So an ask can answer
  fewer artifacts than listing each resource would, and that is the rule, not a gap.
{%- if surface == "cli" %}

```bash
# The ask whole: inline, @<path>, or - for stdin
temper data-artifact ask --ask '{"shape": "<shape-id>", "predicates": [{"path": ["unit"], "op": {"op": "contains", "values": ["ms"]}}]}'

# The same ask, built from flags: one --where per predicate, each one predicate as JSON
temper data-artifact ask --shape <shape-id> \
  --where '{"path": ["unit"], "op": {"op": "contains", "values": ["ms"]}}' \
  --limit 50

# The next page: the same ask, with the previous page's `next` passed back exactly
temper data-artifact ask --shape <shape-id> --where '...' \
  --after '{"created": "<next.created>", "artifact_id": "<next.artifact_id>"}'
```

`--ask` and `--shape`/`--where` are one form or the other, never both. A refused ask, under the JSON
output format, prints its refusals in the error payload's `details.refusals`, each with its `reason`
and `teaches`. Read an `oversized` artifact with `temper data-artifact get <artifact-id>`.
{%- else %}

```
Tool: ask_data_artifacts
Input: {
  "ask": {
    "shape": "<shape UUID>",
    "predicates": [ { "path": ["unit"], "op": { "op": "contains", "values": ["ms"] } } ]
  },
  "limit": 50,                                           // optional: 1 to 200, default 50
  "after": { "created": "...", "artifact_id": "..." }   // optional: the previous page's `next`
}
```

A refused ask returns `invalid_params`; the error's `data` carries `{"refusals": [...]}`, each
refusal with its `reason` and `teaches`. Read an `oversized` artifact with `get_data_artifact`.
{%- endif %}

## What data artifacts are NOT

- **Not searchable.** Data artifacts are never found by resemblance or text match. They are
  reached through the resource that owns them, and asked by what they hold only through a declared
  shape's paths (above) — the graph is the index; the artifact is the payload at the end of it.
  Asking adds nothing to the search corpus: an artifact's content is never found by search.
- **Not embedded.** Data artifact content never enters the search corpus. The corpus is built for
  prose, and structured data injected into it is semantic noise.
- **Not a replacement for `open_meta`.** `open_meta` is for small, key-scoped metadata values
  (a `status`, a `verified` date, a `descriptor`). Data artifacts are for structured content of any
  size — a measurement, a plan, a computation output — that does not fit in a property value and
  must survive a round-trip whole.

{%- if surface == "cli" %}

## The CLI surface
{%- else %}

## The MCP tool surface
{%- endif %}

{%- if surface == "cli" %}
```bash
# Commit a data artifact to a resource
temper data-artifact commit <resource-ref> \
  --kind "measurement" \
  --intent "current" \
  --content @path/to/data.json        # or --content - for stdin

# Retrieve a single artifact by ID
temper data-artifact show <resource-ref> <artifact-id>

# List artifacts owned by a resource, one bounded page at a time (with optional filters)
temper data-artifact list <resource-ref> \
  --kind "measurement" \
  --intent "current" \
  --include-folded \
  --limit 50                           # one page of at most 50 (1 to 200)

# The next page: pass the previous page's `next` back, with the same filters and --order
temper data-artifact list <resource-ref> --limit 50 \
  --after-created "<next.created>" --after-id "<next.artifact_id>"
```

The `--content` must be valid JSON. The commit returns the artifact with its server-assigned ID,
content hash, and shape state. See `reference.md` for the full command table.

**Pass `--limit` when you list.** With it, the answer is one page, oldest first (`--order desc` for
newest first), as `{"artifacts": [...], "next": {"created": ..., "artifact_id": ...} | null}`. To
continue, pass `next.created` and `next.artifact_id` back exactly as `--after-created` and
`--after-id`, with the same filters and `--order`. `next` is `null` on the last page, so a page with
a `next` is not the whole list. Without `--limit`, the answer is every artifact as a bare array; that
unbounded default is retiring. `--counts` is per-family counts and is not paged.

```bash
# Declare a shape for a family within a context (gated on authoring authority)
temper data-artifact schema declare <context-ref> \
  --kind "measurement" \
  --enforcement advisory          # default; use --enforcement enforcing to refuse non-conforming commits
  --content @schema.json          # a field tree (JSON Schema 2020-12 subset)

# List live shapes declared for a context
temper data-artifact schema list --context <context-ref>

# Show a single shape by ID
temper data-artifact schema show <shape-id>
```
{%- else %}
```
Tool: commit_data_artifact
Input: {
  "resource_id": "<resource UUID>",
  "kind": "measurement",
  "intent": "current",
  "content": { ... },                 // a JSON value, not a string
  "precedence": 0,                    // optional, default 0
  "supersedes": ["<artifact UUID>"]   // optional, repeatable
}

Tool: get_data_artifact
Input: { "artifact_id": "<artifact UUID>" }

Tool: list_data_artifacts
Input: {
  "resource_id": "<resource UUID>",
  "limit": 50,                        // one page of at most 50 (1 to 200); pass it
  "order": "asc",                     // optional: "asc" (default, oldest first) or "desc"
  "after": { "created": "...", "artifact_id": "..." },  // optional: the previous page's `next`
  "kind": "measurement",              // optional filter
  "intent": "current",                // optional filter
  "include_folded": false             // optional, default false
}
```

The `content` field takes a JSON value (object, array, string, number, boolean, or null), not a
string. The commit returns the artifact with its server-assigned ID, content hash, and shape state.

**Pass `limit` when you list.** With it, `list_data_artifacts` answers one page as
`{"artifacts": [...], "next": {"created": ..., "artifact_id": ...} | null}`. To continue, call it
again with `next` passed back unchanged as `after`, and the same filters and `order`. `next` is
`null` on the last page, so a page with a `next` is not the whole list. Without `limit`, the answer
is every artifact as a bare array; that unbounded default is retiring.

```
Tool: declare_data_artifact_shape
Input: {
  "home_type": "context",             // or "cogmap" — shapes are keyed per home
  "home_id": "<home UUID>",
  "kind": "measurement",
  "schema": { ... },                  // a field tree (JSON Schema 2020-12 subset)
  "enforcement": "advisory"           // "advisory" (default) or "enforcing"
}

Tool: list_data_artifact_shapes
Input: { "home_type": "context", "home_id": "<home UUID>" }

Tool: get_data_artifact_shape
Input: { "shape_id": "<shape UUID>" }
```
{%- endif %}