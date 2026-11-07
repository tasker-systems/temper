# Data Artifacts — structured data with a home

Read this when you are about to store structured data — a measurement, a computation output, a
query plan — that a later session must get back whole, and are choosing between this and a body.

## The model

**A data artifact is structured data owned by a resource, reached through the graph, handed back
intact.** A resource body is prose: chunked, embedded and searched, so a JSON fence in it is split at
its own comment lines and reaches the next reader as a puzzle. Writer and reader are different
sessions with no shared context; the reader has only what was stored.

**Commit a data artifact** when the content is structured, a later session needs it whole, and
embedding it would be noise; **write the body** when it is prose read for understanding. The choice
is not about format: a data artifact is not "another way to store JSON".

**Selection** — a reader picks among a resource's artifacts from the stored record alone:

- **`kind`** — the family name (`measurement`, `query-plan`); free-form, consistent within a family.
- **`intent`** — closed (an unknown one is refused with the vocabulary): `current` is the family's
  default pick, `member` one of several ordered by `precedence` (default `0`), `pinned` held regardless.
- **`supersedes`** — ids this commit replaces; they fold out of the default list
  (`include_folded: true` returns them). **Supersession is never inferred from recency** — declare it.

**Shape** — a declaration of what a family's artifacts hold: a **field tree**, a subset of JSON
Schema 2020-12. **Declaring is never required to commit**; it is a separate act.

- **A shape governs its home** — a context or a cogmap, not the system — so one family can carry
  different shapes in different homes, and a shape you cannot read never verdicts your data.
- **Enforcement** is closed: `advisory` (default) records a non-conforming commit as
  `declared_not_satisfied`; `enforcing` refuses it and says what failed.
- **Keywords:** `type`, `properties`, `required`, `additionalProperties`, `items` (one schema),
  `enum` and `const` (scalars), `pattern`, `minLength`/`maxLength`, `minItems`/`maxItems` and `minimum`/`maximum`, annotations
  (`title`, `description`, `$comment`, `default`, `examples`, `format`), `$schema`/`$id` at the root.
  Every other keyword (`$ref`, `$defs`, `allOf`, `anyOf`, `oneOf`, `not`, `if`, …) is refused at
  declare, as is a schema past its caps; the refusal names the first keyword or cap that failed.

**Shape state** — every artifact carries one: `never_declared` (no shape for its family in its home —
**a first-class state, not a degraded one**; `""` or `NULL` is a decode error), `declared_satisfied`,
`declared_not_satisfied`, or `declared_not_yet_checked` (validation has not reached it). A stored
verdict holds **only** while its `(shape_id, shape_version, content_hash)` matches the governing
shape and current content; anything else reads `declared_not_yet_checked`, so staleness never reads
as conformance. Declaring later, or rehoming a resource, reconciles asynchronously.

## Asking what artifacts hold

A declared shape makes its family **askable**. An **ask** names a shape by id and predicates over the
paths it declares, with the closed operators `has_key`, `contains` and `compare`:
`{"shape": "<shape-id>", "predicates": [{"path": ["unit"], "op": {"op": "contains", "values": ["ms"]}}]}`.
One ask object, two arms:

- **Which resources hold matches** — `find-resources-with`'s `artifact_filter`, in a query
  composition; it answers resource ids that pipe on. A resource is selected when, for **every** ask,
  one of its artifacts satisfies **all** of that ask's predicates — so predicates in one ask hold of
  one artifact, and separate asks may be met by different artifacts.
- **The matching artifacts, whole** — the batch read, the ask sent alone. It answers artifacts, never
  resources, and is not a composition stage.

Both arms read the live artifacts of the family on resources homed in the shape's home that you can
see and that have finished ingesting. **Read the shape's `askable` first** (its by-id read): every
path and the operators each admits. Anything outside it is refused before anything runs, and the
refusal's `teaches` carries the vocabulary.
Send a composition with `run_query` and the batch read with `ask_data_artifacts`; a refusal returns
`invalid_params` whose `data` carries `{"refusals": [...]}`, each with its `reason` and `teaches`.

**The batch read** pages oldest first (`limit` 1 to 200, default 50) in enumeration order, never rank.

- **Only `next: null` ends it.** Send the same ask with `next` as `after`, unchanged. A page also
  stops at its read budget, so an empty page with a `next` means keep going, not "nothing matches".
- **`oversized` is fetched by id.** A match too large for a page, or whose content nests
  more than 96 arrays or objects deep, is named in `oversized` without its content; read it by id.
- **`population` is on the first page only** — exact counts by shape state, and how many matched.
- **`amended_since`** means the shape changed between your pages; `shape_state` and any
  `shape_states` narrowing now read against the new version, so if you narrowed, restart from a
  first page. A first page naming a folded version is refused `shape_superseded`.
- **An artifact with no content** returns only to an ask with no predicates.

## Traps

- **Not searchable, not embedded** — content never enters the search corpus; the graph is the index.
- **Not `open_meta`** — that holds small key-scoped values (`status`, `verified`); this holds content.
- **`format` is not enforced** — it is an annotation; use `pattern`. `x-` keys are carried, never read.
- **`value_outside_shape` means the shape does not declare the value, not that nothing matches** —
  advisory shapes, older artifacts and narrowed enums keep such content. Its `teaches` routes you to
  `has_key` with `shape_states: ["declared_not_satisfied", "declared_not_yet_checked"]` in the batch read.
- **`contains` below a path that declares values is refused** — scalars hold nothing at any key.
- **Asks of shapes in two homes cannot share a stage** (`ask_scope_conflict`) — one per home, `union`.
- **A shape whose home you cannot read is refused as if it did not exist** (`shape_not_askable`).
- **`include_folded: true` on an ask is refused** — asks read live artifacts only.
- **A shape declared before v0.6.3 may hold refused keywords** — commits land unchecked with
  `uncompilable_shape_notice`; asks answer `shape_schema_uncompilable` until it is amended.
- **An ask can answer fewer artifacts than listing each resource** — it is an enumeration and leaves
  out resources still ingesting; listing is an addressed read.
- **An empty stage says why in its trace** (`artifact_asks`: each ask's `population`, `matched`) —
  by-state counts ignore `shape_states`, so many `declared_not_yet_checked` beside a small answer is lag.
- **Pass a limit when you list** — without one the answer is an unbounded bare array, a retiring
  default. A page that carries a `next` is not the whole list; pass it back until `next` is `null`.

## The MCP tool surface

`commit_data_artifact` (`content` is a JSON value, not a string), `get_data_artifact`,
`list_data_artifacts` (pass `limit`; continue with `next` as `after`), `ask_data_artifacts`,
`declare_data_artifact_shape` (`home_type` `context` or `cogmap`, with `home_id`),
`list_data_artifact_shapes`, `get_data_artifact_shape`. A commit returns the artifact's id,
content hash and shape state.
