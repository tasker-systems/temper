# Temper Knowledge Base — MCP Access Patterns

This skill teaches you how to work with the Temper knowledge base through the
MCP server. Temper exposes both **resources** (browsable, context-injectable) and
**tools** (function calls for queries and mutations). Use resources for reads,
tools for writes and search.

## When to Use This Skill

Trigger when: the user mentions their knowledge base, vault, notes, contexts,
sessions, research, or wants to look up / store information across conversations.

## The Tool Surface (28 tools, read/write separable)

The MCP surface is consolidated: each tool serves an agent use case, not an
administrative one. Tools that shared a lifecycle are collapsed into one tool
with a discriminator (`action`, `view`, or `target`). Read and write are
separable — a consumer gating on "read-only" can grant the read tools without
drilling into action parameters.

**Reads (14):** `search`, `run_query`, `get_resource`, `list_resources`,
`resource_lineage`, `element_trail`, `get_block_provenance`, `cogmap_read`,
`cogmap_list`, `context_read`, `describe_schema`, `invocation_read`,
`facets_read`, `steward_ingest_delta`

**Writes (12):** `create_resource`, `update_resource`, `update_resource_meta`,
`delete_resource`, `annotate_resource`, `relationship`, `facet_set`,
`record_citation_audit`, `invocation_manage`, `segmented_ingest`,
`cogmap_create`, `cogmap_materialize`, `context_manage`, `steward_advance_watermark`

**Declared off-MCP (CLI door):** grants (`resource_grant`/`revoke`,
`cogmap_grant`/`revoke`), `admin_ledger`, cogmap bind/unbind, team invitations,
`get_profile`, `reassign`. Each capability stays at the CLI; its absence from
MCP is a declaration, not a gap.

## Resources vs Tools — Decision Table

| Intent | Use | Why |
|--------|-----|-----|
| See what a context is *about* before reading it | Tool: `context_read` (view: shape) | Region-level map, most salient first — the fastest orientation move |
| Per-region analytics for a context | Tool: `context_read` (view: metrics) | Centrality, cohesion, tension, telos alignment under an optional lens |
| Browse what's in a context | Resource: `temper://contexts/{ref}/resources` | No tool call overhead, client can cache |
| Read a specific document | Resource: `temper://resources/{id}` | Returns metadata + full markdown |
| Get raw markdown only | Resource: `temper://resources/{id}/content` | Lighter than full resource read |
| Find something by topic | Tool: `search` | Semantic vector search, can't do with resources |
| Create a new resource (with or without content) | Tool: `create_resource` | Mutation — tools only |
| Update title/metadata/content | Tool: `update_resource` | Mutation — tools only |
| Build a large / resumable body as ordered blocks | Tool: `segmented_ingest` (action: begin → append → finalize) | Segmented lifecycle; action: blocks reads landed segments to resume |
| Attach provenance sources without rewriting the body | Tool: `annotate_resource` | Provenance-only backfill — body_hash + embeddings unchanged |
| Read a resource's per-block provenance | Tool: `get_block_provenance` | Which sources each content block was distilled from |
| Read a resource or relationship's event history | Tool: `element_trail` | Append-only ledger — who created/updated/touched it and when |
| Read a resource with content via tool | Tool: `get_resource` with `include_content: true` | When resource browsing isn't available |
| Delete a resource | Tool: `delete_resource` | Soft-delete, tools only |
| Create a new context | Tool: `context_manage` (action: create) | Mutation — tools only |
| Discover valid document types | Tool: `describe_schema` (view: doc_types) | Returns id and name for each type |
| Get schema for a specific type | Tool: `describe_schema` (view: doc_type) | Returns JSON Schema and example_managed_meta |

## Session Start Pattern

When beginning work that involves the knowledge base:

1. **Discover contexts** — read `temper://contexts/{ref}/resources` for the
   relevant workspace, or use the `context_read` tool (view: list) if you don't
   know the context name
2. **Orient before you read** — call `context_read` (view: shape) on the context
   to see its materialized regions (what it is *about*) before pulling
   individual resources. This is the fastest way to understand a large context
   without reading every document in it. See **Context Orientation** below.
3. **Load relevant content** — read resources directly via
   `temper://resources/{id}` to build working context
4. **Search if needed** — use the `search` tool for semantic lookup when you
   don't know what exists or need fuzzy matching

Prefer resources for steps 1 and 3. They populate the context window without
consuming tool-call tokens.

## Context Orientation — Read a Context's Shape Before Its Resources

A context is not just a flat bag of documents. Temper continuously clusters a
context's resources into **regions** — groups of semantically related material —
and scores each region for salience. The orientation read lets you read that
region-level shape directly, so you can understand what a large context is
*about* without reading (or listing) every resource inside it. Reach for this
first when a context is unfamiliar or large.

All context reads are addressed by **context ref**, not resource ref:

- `@me/<slug>` — a context you own (e.g. `@me/temper`)
- `+<team>/<slug>` — a team context
- a bare UUID

Bare names are deliberately **not** accepted — a context is not a resource, so
the resource-ref parser is not used here.

### `context_read` (view: shape) — what the context is about

The primary orientation read. Returns the context's materialized regions, most
salient first, each with its salience, content cohesion, agent-authored label
(if any), and member count. This is the fastest way to see the structure of a
context before you commit tokens to reading its documents.

```
Tool: context_read
Input: { "view": "shape", "context": "@me/temper", "lens": "<optional lens ref>" }
```

### `context_read` (view: metrics) — the analytics tier

Deeper per-region metrics for the same regions: centrality, content cohesion,
internal tension, reference standing, and telos alignment. Use when
`context_read` (view: shape) has shown you the regions and you want to judge
which are load-bearing versus peripheral.

```
Tool: context_read
Input: { "view": "metrics", "context": "@me/temper", "lens": "<optional lens ref>" }
```

### The `lens` parameter

`context_read` (views: shape, metrics) takes an optional `lens` ref. A lens
is a perspective that produces its own regioning of the same context; omit `lens`
to read across all lenses. Leave it off unless you have a specific lens ref in
hand.

> **Cognitive-map peers**: these reads are the context-addressed peers of
> the cognitive-map orientation reads (`cogmap_read` with views: shape,
> metrics). The region reads beneath them are the same — only the anchor
> differs (a context ref instead of a cogmap ref). If you are orienting on
> a cognitive map rather than a context, use `cogmap_read` instead.

## Reading Content

### Via Resources (preferred for known documents)

Resources return structured data that clients can display and inject directly:

- `temper://resources/{id}` — returns two content blocks:
  1. JSON metadata (title, origin URI, timestamps, context ID)
  2. Full markdown content
- `temper://resources/{id}/content` — returns only the markdown
- `temper://contexts/{ref}/resources` — returns JSON array of all resources
  in that context (metadata only, no content)

### Via Tools (for discovery)

- `list_resources` — paginated list with optional `context_ref` and `doc_type_name`
  filters. Use when you need programmatic filtering (limit, offset) beyond what resource
  browsing provides. The response is a **capped page** — alongside `rows` it carries `total`
  (all matching rows), `returned` (this page's count), `limit`/`offset`, and **`truncated`**.
  Read `truncated`: it is the server's own answer to "are there matching rows beyond this
  page?", and **when it is `true` you must not conclude a resource is absent or that a set is
  complete.** Do not re-derive it from `rows.len() < total` — that is `true` on the last page
  of a walk (total 25, offset 20, returned 5), where nothing is in fact hidden. Raise `limit`
  (up to 200), page with `offset`, or narrow the filters (`doc_type_name`, `stage`) first.
- `get_resource` — single resource by ref (a UUID or the decorated `slug-<uuid>` form). Pass
  `include_content: true` to include the full markdown body.
- `search` — returns **two arms that are never merged**: `exact` (full-text) and `wide`
  (vector similarity over a 768-dimensional embedding), plus a shared `scope`. Each arm
  carries `hits` and a `reason`; each hit is `{ resource, <quantity> }` — the same
  `ResourceView` every other tool answers in, beside `fts_norm` on an exact hit and
  `vec_norm` on a wide one. **The two quantities are not comparable and there is no combined
  score**; read each arm on its own terms. There are no snippets — a hit names a resource,
  it does not excerpt one. Use for "find me notes about X" queries.

## Writing Content

All mutations go through tools. There are no writable resources.

### Creating Resources

Use `create_resource` to write content to the knowledge base. The server validates
`managed_meta` against the doc type schema, runs chunking and embedding inline, and returns
the fully processed resource in a single call. No polling, no intermediate state.

```
Tool: create_resource
Input: {
  "context_ref": "@me/myproject",
  "doc_type_name": "session",
  "title": "Human-readable title",
  "content": "Full markdown content goes here...",
  "managed_meta": { ... },  // doc-type-specific frontmatter fields (optional)
  "open_meta": { ... }      // free-form user fields (optional)
}
```

The resource is created immediately and returned with an `id`. If `managed_meta` validation
fails, the tool returns a structured error listing each issue with its field name — fix the
fields and retry.

**There is no `slug` field, on this call or any other.** The slug is derived from the title on
every surface, and addressing is trailing-UUID-only — a slug is display decoration on a ref, never
an input. See `references/frontmatter.md` for the doc types and both metadata tiers.

### Discovering Document Types

Before creating content, use `describe_schema` (view: doc_types) to see available types and
which have schemas. Common types include: `session`, `research`, `concept`, `task`, `goal`.

```
Tool: describe_schema
Input: { "view": "doc_types" }
```

For a specific type's JSON Schema and a usable `example_managed_meta` template:

```
Tool: describe_schema
Input: { "view": "doc_type", "name": "task" }
```

Pass the `name` field as `doc_type_name` in `create_resource`.

### Context handling

If the `context_ref` does not match an existing context, **do not silently create a new one**.
The context must already exist — `create_resource` will fail if the context is not found.
Instead, ask the user: "I don't see a context named `{name}`. Would you like me to create
it, or did you mean one of these: {list existing contexts}?" Use `context_read` (view: list)
to fetch the current list before asking.

### Updating Resources

Use `update_resource` to change an existing resource's title or content. All fields
except `id` are optional — only the fields you provide are changed.

```
Tool: update_resource
Input: {
  "id": "<resource UUID>",
  "title": "New title",       // optional
  "content": "New markdown content..."  // optional
}
```

### Deleting Resources

```
Tool: delete_resource
Input: { "id": "<resource UUID>" }
```

This is a soft-delete — the resource is deactivated, not permanently removed.

## Block-Grain Ingest & Attribution

A resource body is not always one opaque blob. The ingest surface lets you build it as an
**enumerated sequence of distinctly addressable, individually attributable content blocks**,
attach **per-block provenance** (which sources each block was distilled from), and record
**per-act authorship** (confidence + the model/persona that wrote it). Reach for this when
constructing a large or citation-graded document rather than defaulting to a single
whole-body `create_resource`.

> **Boundary — what this is NOT.** These tools *build* and *attribute* blocks; they do **not**
> surgically rewrite one existing block's text in a finalized resource. An in-place text edit
> is still a whole-body `update_resource`. `annotate_resource` is **provenance-only** — it
> never touches block text, `body_hash`, or embeddings.

### Per-block provenance & authorship (on any write)

`create_resource` and `update_resource` both accept:

- `sources` — an array of **resource refs** (UUID or decorated `slug-<uuid>`) and/or **http(s)
  URLs**. Each becomes a block-provenance record on the body block. A URL may carry a
  `#L<start>-L<end>` span locator (e.g. `https://ex.com/doc.md#L120-L180`) — preserved verbatim
  and surfaced by `get_block_provenance`. On `update_resource`, `sources` requires `content`
  (no body update → nothing to attribute).
- `content_block` — which block (a block UUID) the body revise + `sources` target. Omit to
  address the resource's sole body block; **required** once a resource has more than one block.
- Authorship/correlation (flattened top-level keys): `confidence`
  (`tentative|probable|confident`), `reasoning`, `rationale`, `persona`, `model`,
  `invocation_id`, `correlation_id`. `confidence` is **required** whenever any other authorship
  field is supplied.

### `annotate_resource` — provenance backfill, no body revise

Attach provenance `sources` to a block **without** re-chunking or re-embedding — `body_hash`
and embeddings are unchanged. This is the cheap way to make a corpus imported without sources
citation-grade after the fact.

```
Tool: annotate_resource
Input: {
  "id": "<resource UUID>",
  "sources": ["<resource-ref-or-url>", "https://ex.com/doc.md#L120-L180"],
  "content_block": "<block UUID>",   // omit for a single-block resource; required for multi-block
  "confidence": "probable"           // optional; required if you also pass reasoning/model/etc.
}
```

`sources` is required and non-empty (an annotate with nothing to attribute is an error). Verify
the result with `get_block_provenance`.

### `get_block_provenance` — read per-block provenance

```
Tool: get_block_provenance
Input: { "resource": "<resource UUID>" }
```

Returns each content block's provenance in (block, accretion) order — the sources each block
was distilled from, including any preserved span-locator fragments.

### `element_trail` — read an element's event history

The append-only ledger for one node (resource) or edge (relationship): a time-ordered list of
the events that produced and mutated it — created, updated, relationship asserted/folded, facets
set, etc. Each event carries its actor, time, and replay-sufficient payload. Use for orientation,
audit, and debugging ("what happened to this resource?"). Visibility is gated inside the read:
an unreadable or nonexistent element returns an empty trail, never an error.

```
Tool: element_trail
Input: { "kind": "node", "element": "<resource UUID or slug-<uuid>>" }
```

`kind` selects the trail function: `node` (a resource) or `edge` (a relationship). `element` is a
ref resolved trailing-UUID-only — a resource ref for a node, an edge UUID for an edge. The
decorated `slug-<uuid>` form is accepted and the slug half ignored.

### Segmented ingest lifecycle (large / resumable builds)

An MCP caller has no chunker or embedder, so it omits `chunks_packed` and the server chunks the
segment text itself (carrying the heading breadcrumb across block boundaries). Use the lifecycle
when a body is too large for one `create_resource` call, or when a build must be resumable:

| Action | Role |
|------|------|
| `segmented_ingest` (action: begin) | Lands segment 0 and creates the resource. Takes every `create_resource` field plus a bare-hex `content_hash` of segment 0, and optional `block_budget` / `total_blocks_hint` / `source_hash`. Returns `resource_id`, the landed block set, and an opaque `body_hash`. |
| `segmented_ingest` (action: append) | Lands segment N. `seq` starts at **1** (segment 0 landed at begin) and must go in order. **Idempotent** — re-appending an already-landed `seq` is a safe no-op, so retry/resume is safe. Pass `content` + its `content_hash`; optional per-segment `sources`. |
| `segmented_ingest` (action: blocks) | Reads the landed segment set back for a resource — how a stateless caller resumes after an interruption before continuing to append. |
| `segmented_ingest` (action: finalize) | Declares the session complete. Pass `expected_blocks` (counting segment 0) and **echo the `body_hash` back verbatim** from your most recent `append`/`blocks` response — it is opaque; never parse or recompute it. Fails loudly on a gap. |

Each segment's `content_hash` is a per-segment transit-integrity check (bare-hex sha256 of that
segment's text), verified server-side; a mismatch is rejected before anything lands.

## Relationships, Facets & Invocations

### `relationship` — manage graph edges (consolidated)

One tool with an `action` discriminator, collapsing assert/retype/reweight/fold:

| Action | What it does | Required fields |
|--------|-------------|-----------------|
| `assert` | Create a directed relationship from source to target | `source`, `target`, `edge_kind`, `polarity`, `label`, `weight` |
| `retype` | Change edge_kind and polarity of an existing edge | `edge_handle`, `edge_kind`, `polarity` |
| `reweight` | Change the weight of an existing edge | `edge_handle`, `weight` |
| `fold` | Retract (fold) an edge, marking it inactive | `edge_handle` (`reason` optional) |

The `edge_handle` comes from the `assert` response. Per-act authorship fields (`confidence`,
`reasoning`, `invocation_id`, etc.) are accepted on all actions.

### `facet_set` / `facets_read` — typed properties on resources and edges

`facet_set` sets a facet on a resource or a relationship (edge) via a `target` discriminator:
`resource` (requires `resource` ref) or `edge` (requires `edge_handle`). `facets_read` reads
the live facets with the same `target` discriminator — use it to confirm a `facet_set` landed,
since `get_resource` collapses facets into a single newest-wins value in `open_meta` and drops
the weight.

### `invocation_manage` / `invocation_read` — agent-run envelopes

`invocation_manage` (action: open) opens an accountability envelope for one agent run against a
cognitive map — returns the server-minted `invocation_id`. `invocation_manage` (action: close)
terminates it with a disposition (`completed`/`failed`/`abandoned`) and optional outcome.
`invocation_read` (view: show) reads one envelope plus its acts; `invocation_read` (view: list)
lists envelopes, optionally narrowed by cogmap and/or status.

## Context Navigation

Contexts are workspaces that group resources. The typical flow:

1. `context_read` (view: list) → see all available workspaces
2. `temper://contexts/{ref}/resources` resource → browse a workspace
3. `temper://resources/{id}` resource → read a specific document

## Tips

- **Resources are read-only and stateless** — they always reflect current state,
  no caching surprises.
- **Search supports text queries** — the `search` tool accepts a plain text
  `query` parameter. No embedding vector needed; the server embeds it for the
  `wide` arm, and the `exact` arm needs no embedding at all.
- **Pagination** — `list_resources` supports `limit` and `offset`. Resources
  listing is capped at 200 items, and its response carries a `truncated` flag —
  read it before asserting a set is complete or a resource is absent; `true`
  means there is more to fetch.
- **Access control is automatic** — you only see resources and contexts your
  authenticated profile has access to. No need to handle permissions.