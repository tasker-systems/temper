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

A shape is a JSON Schema (draft 2020-12) declared for a family within a home. Declaring is a
separate act from committing data — you never need to declare a shape to commit an artifact.

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

## What data artifacts are NOT

- **Not searchable.** Data artifacts are never found by resemblance or text match. They are
  reached only through the resource that owns them — the graph is the index; the artifact is the
  payload at the end of it.
- **Not embedded.** Data artifact content never enters the search corpus. The corpus is built for
  prose, and structured data injected into it is semantic noise.
- **Not a replacement for `open_meta`.** `open_meta` is for small, key-scoped metadata values
  (a `status`, a `verified` date, a `descriptor`). Data artifacts are for structured content of any
  size — a measurement, a plan, a computation output — that does not fit in a property value and
  must survive a round-trip whole.

## The CLI surface

{%- if surface == "cli" %}
```bash
# Commit a data artifact to a resource
temper data-artifact commit <resource-ref> \
  --kind "measurement" \
  --intent "current" \
  --content @path/to/data.json        # or --content - for stdin

# Retrieve a single artifact by ID
temper data-artifact show <resource-ref> <artifact-id>

# List artifacts owned by a resource (with optional filters)
temper data-artifact list <resource-ref> \
  --kind "measurement" \
  --intent "current" \
  --include-folded                     # include superseded artifacts
```

The `--content` must be valid JSON. The commit returns the artifact with its server-assigned ID,
content hash, and shape state. See `reference.md` for the full command table.

```bash
# Declare a shape for a family within a context (gated on authoring authority)
temper data-artifact schema declare <context-ref> \
  --kind "measurement" \
  --enforcement advisory          # default; use --enforcement enforcing to refuse non-conforming commits
  --content @schema.json          # a JSON Schema draft 2020-12 document

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
  "kind": "measurement",              // optional filter
  "intent": "current",                // optional filter
  "include_folded": false             // optional, default false
}
```

The `content` field takes a JSON value (object, array, string, number, boolean, or null), not a
string. The commit returns the artifact with its server-assigned ID, content hash, and shape state.

```
Tool: declare_data_artifact_shape
Input: {
  "home_type": "context",             // or "cogmap" — shapes are keyed per home
  "home_id": "<home UUID>",
  "kind": "measurement",
  "schema": { ... },                  // a JSON Schema draft 2020-12 document
  "enforcement": "advisory"           // "advisory" (default) or "enforcing"
}

Tool: list_data_artifact_shapes
Input: { "home_type": "context", "home_id": "<home UUID>" }

Tool: get_data_artifact_shape
Input: { "shape_id": "<shape UUID>" }
```
{%- endif %}