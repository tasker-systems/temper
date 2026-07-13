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
