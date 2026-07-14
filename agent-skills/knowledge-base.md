# Temper Knowledge Base — MCP Access Patterns

This skill teaches you how to work with the Temper knowledge base through the
MCP server. Temper exposes both **resources** (browsable, context-injectable) and
**tools** (function calls for queries and mutations). Use resources for reads,
tools for writes and search.

## When to Use This Skill

Trigger when: the user mentions their knowledge base, vault, notes, contexts,
sessions, research, or wants to look up / store information across conversations.

## Resources vs Tools — Decision Table

| Intent | Use | Why |
|--------|-----|-----|
| See what a context is *about* before reading it | Tool: `context_shape` | Region-level map, most salient first — the fastest orientation move |
| Per-region analytics for a context | Tool: `context_region_metrics` | Centrality, cohesion, tension, telos alignment under an optional lens |
| Refresh a context's regions after big changes | Tool: `context_materialize` | Re-forms stale region shape; safe no-op below threshold; needs write access |
| Browse what's in a context | Resource: `temper://contexts/{name}/resources` | No tool call overhead, client can cache |
| Read a specific document | Resource: `temper://resources/{id}` | Returns metadata + full markdown |
| Get raw markdown only | Resource: `temper://resources/{id}/content` | Lighter than full resource read |
| Find something by topic | Tool: `search` | Semantic vector search, can't do with resources |
| Create a new resource (with or without content) | Tool: `create_resource` | Mutation — tools only |
| Update title/metadata/content | Tool: `update_resource` | Mutation — tools only |
| Build a large / resumable body as ordered blocks | Tools: `ingest_begin` → `ingest_append` → `ingest_finalize` | Segmented lifecycle; `ingest_blocks` reads landed segments to resume |
| Attach provenance sources without rewriting the body | Tool: `annotate_resource` | Provenance-only backfill — body_hash + embeddings unchanged |
| Read a resource's per-block provenance | Tool: `get_block_provenance` | Which sources each content block was distilled from |
| Read a resource with content via tool | Tool: `get_resource` with `include_content: true` | When resource browsing isn't available |
| Delete a resource | Tool: `delete_resource` | Soft-delete, tools only |
| Create a new context | Tool: `create_context` | Mutation — tools only |
| Check who you are | Tool: `get_profile` | Identity/settings |
| Audit recent activity | Tool: `list_events` | Debugging, event history |
| Discover valid document types | Tool: `list_doc_types` | Returns id and name for each type |
| Get schema for a specific type | Tool: `describe_doc_type` | Returns JSON Schema and example_managed_meta |

## Session Start Pattern

When beginning work that involves the knowledge base:

1. **Discover contexts** — read `temper://contexts/{name}/resources` for the
   relevant workspace, or use the `list_contexts` tool if you don't know the
   context name
2. **Orient before you read** — call `context_shape` on the context to see its
   materialized regions (what it is *about*) before pulling individual resources.
   This is the fastest way to understand a large context without reading every
   document in it. See **Context Orientation** below.
3. **Load relevant content** — read resources directly via
   `temper://resources/{id}` to build working context
4. **Search if needed** — use the `search` tool for semantic lookup when you
   don't know what exists or need fuzzy matching

Prefer resources for steps 1 and 3. They populate the context window without
consuming tool-call tokens.

## Context Orientation — Read a Context's Shape Before Its Resources

A context is not just a flat bag of documents. Temper continuously clusters a
context's resources into **regions** — groups of semantically related material —
and scores each region for salience. The orientation trio lets you read that
region-level shape directly, so you can understand what a large context is
*about* without reading (or listing) every resource inside it. Reach for these
first when a context is unfamiliar or large.

All three are addressed by **context ref**, not resource ref:

- `@me/<slug>` — a context you own (e.g. `@me/temper`)
- `+<team>/<slug>` — a team context
- a bare UUID

Bare names are deliberately **not** accepted — a context is not a resource, so
the resource-ref parser is not used here.

### `context_shape` — what the context is about

The primary orientation read. Returns the context's materialized regions, most
salient first, each with its salience, content cohesion, agent-authored label
(if any), and member count. This is the fastest way to see the structure of a
context before you commit tokens to reading its documents.

```
Tool: context_shape
Input: { "context": "@me/temper", "lens": "<optional lens ref>" }
```

### `context_region_metrics` — the analytics tier

Deeper per-region metrics for the same regions: centrality, content cohesion,
internal tension, reference standing, and telos alignment. Use when `context_shape`
has shown you the regions and you want to judge which are load-bearing versus
peripheral.

```
Tool: context_region_metrics
Input: { "context": "@me/temper", "lens": "<optional lens ref>" }
```

### `context_materialize` — refresh the shape

Re-forms a context's regions when enough has changed since the last materialize.
Below that change threshold it is a safe idempotent no-op, so it is cheap to call
defensively before orienting a context you have just written to heavily. **Requires
write access** to the context (direct membership with an authoring role).

```
Tool: context_materialize
Input: { "context": "@me/temper" }
```

### The `lens` parameter

`context_shape` and `context_region_metrics` take an optional `lens` ref. A lens
is a perspective that produces its own regioning of the same context; omit `lens`
to read across all lenses. Leave it off unless you have a specific lens ref in
hand.

> **Cognitive-map peers**: these three tools are the context-addressed peers of
> the cognitive-map orientation tools (`cogmap_shape`, `cogmap_region_metrics`,
> `cogmap_materialize`). The region reads beneath them are the same — only the
> anchor differs (a context ref instead of a cogmap ref). If you are orienting on
> a cognitive map rather than a context, use the `cogmap_*` trio instead.

## Reading Content

### Via Resources (preferred for known documents)

Resources return structured data that clients can display and inject directly:

- `temper://resources/{id}` — returns two content blocks:
  1. JSON metadata (title, origin URI, timestamps, context ID)
  2. Full markdown content
- `temper://resources/{id}/content` — returns only the markdown
- `temper://contexts/{name}/resources` — returns JSON array of all resources
  in that context (metadata only, no content)

### Via Tools (for discovery)

- `list_resources` — paginated list with optional `context_name` and `doc_type_name`
  filters. Use when you need programmatic filtering (limit, offset) beyond what resource
  browsing provides. The response is a **capped page** — it carries `total` (all matching
  rows) alongside `rows`. If `rows.len() < total`, the list is truncated: **do not conclude
  a resource is absent, or that a set is complete, from a truncated page.** Raise `limit`
  (up to 200), page with `offset`, or narrow the filters (`doc_type_name`, `stage`) first.
- `get_resource` — single resource by ID or slug (with `context_name`). Pass
  `include_content: true` to include the full markdown body.
- `search` — semantic search using a 768-dimensional embedding vector. Returns
  scored results with snippets. Use for "find me notes about X" queries.

## Writing Content

All mutations go through tools. There are no writable resources.

### Creating Resources

Use `create_resource` to write content to the knowledge base. The server validates
`managed_meta` against the doc type schema, runs chunking and embedding inline, and returns
the fully processed resource in a single call. No polling, no intermediate state.

```
Tool: create_resource
Input: {
  "context_name": "myproject",
  "doc_type_name": "session",
  "title": "Human-readable title",
  "slug": "url-safe-identifier",
  "content": "Full markdown content goes here...",
  "managed_meta": { ... },  // doc-type-specific frontmatter fields (optional)
  "open_meta": { ... }      // free-form user fields (optional)
}
```

The resource is created immediately and returned with an `id`. If `managed_meta` validation
fails, the tool returns a structured error listing each issue with its field name — fix the
fields and retry.

### Discovering Document Types

Before creating content, use `list_doc_types` to see available types and which have schemas.
Common types include: `session`, `research`, `concept`, `task`, `goal`.

```
Tool: list_doc_types
Input: {}
```

For a specific type's JSON Schema and a usable `example_managed_meta` template:

```
Tool: describe_doc_type
Input: { "name": "task" }
```

Pass the `name` field as `doc_type_name` in `create_resource`.

### Context handling

If `context_name` does not match an existing context, **do not silently create a new one**.
The context must already exist — `create_resource` will fail if the context is not found.
Instead, ask the user: "I don't see a context named `{name}`. Would you like me to create
it, or did you mean one of these: {list existing contexts}?" Use `list_contexts` to fetch
the current list before asking.

### Updating Resources

Use `update_resource` to change an existing resource's title, slug, or content. All fields
except `id` are optional — only the fields you provide are changed.

```
Tool: update_resource
Input: {
  "id": "<resource UUID>",
  "title": "New title",       // optional
  "slug": "new-slug",         // optional
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

### Segmented ingest lifecycle (large / resumable builds)

An MCP caller has no chunker or embedder, so it omits `chunks_packed` and the server chunks the
segment text itself (carrying the heading breadcrumb across block boundaries). Use the lifecycle
when a body is too large for one `create_resource` call, or when a build must be resumable:

| Tool | Role |
|------|------|
| `ingest_begin` | Lands segment 0 and creates the resource. Takes every `create_resource` field plus a bare-hex `content_hash` of segment 0, and optional `block_budget` / `total_blocks_hint` / `source_hash`. Returns `resource_id`, the landed block set, and an opaque `body_hash`. |
| `ingest_append` | Lands segment N. `seq` starts at **1** (segment 0 landed at begin) and must go in order. **Idempotent** — re-appending an already-landed `seq` is a safe no-op, so retry/resume is safe. Pass `content` + its `content_hash`; optional per-segment `sources`. |
| `ingest_blocks` | Reads the landed segment set back for a resource — how a stateless caller resumes after an interruption before continuing to append. |
| `ingest_finalize` | Declares the session complete. Pass `expected_blocks` (counting segment 0) and **echo the `body_hash` back verbatim** from your most recent `ingest_append`/`ingest_blocks` response — it is opaque; never parse or recompute it. Fails loudly on a gap. |

Each segment's `content_hash` is a per-segment transit-integrity check (bare-hex sha256 of that
segment's text), verified server-side; a mismatch is rejected before anything lands.

## Context Navigation

Contexts are workspaces that group resources. The typical flow:

1. `list_contexts` tool → see all available workspaces
2. `temper://contexts/{name}/resources` resource → browse a workspace
3. `temper://resources/{id}` resource → read a specific document

## Tips

- **Resources are read-only and stateless** — they always reflect current state,
  no caching surprises.
- **Search supports text queries** — the `search` tool accepts a plain text
  `query` parameter for full-text search. No embedding vector needed.
- **Pagination** — `list_resources` and `list_events` support `limit` and
  `offset`. Resources listing is capped at 200 items. Compare `rows.len()`
  against the response's `total` before asserting a set is complete or a
  resource is absent — a short page means there is more to fetch.
- **Access control is automatic** — you only see resources and contexts your
  authenticated profile has access to. No need to handle permissions.
