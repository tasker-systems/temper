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

## Session Start Pattern

When beginning work that involves the knowledge base:

1. **Discover contexts** — read `temper://contexts/{name}/resources` for the
   relevant workspace, or use the `list_contexts` tool if you don't know the
   context name
2. **Load relevant content** — read resources directly via
   `temper://resources/{id}` to build working context
3. **Search if needed** — use the `search` tool for semantic lookup when you
   don't know what exists or need fuzzy matching

Prefer resources for steps 1-2. They populate the context window without
consuming tool-call tokens.

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
  browsing provides.
- `get_resource` — single resource by ID or slug (with `context_name`). Pass
  `include_content: true` to include the full markdown body.
- `search` — semantic search using a 768-dimensional embedding vector. Returns
  scored results with snippets. Use for "find me notes about X" queries.

## Writing Content

All mutations go through tools. There are no writable resources.

### Creating Resources

Use `create_resource` to write content to the knowledge base. The `content` field is
optional — omit it to create a metadata-only shell, or include it to create a resource
with full content in one call. Content processing (embedding for search) happens
asynchronously in the background.

```
Tool: create_resource
Input: {
  "context_name": "myproject",
  "doc_type_name": "session",
  "title": "Human-readable title",
  "content": "Full markdown content goes here...",  // optional
  "slug": "optional-kebab-case-slug",              // optional
  "origin_uri": "optional source URL or reference" // optional
}
```

The resource is created immediately and returned with an `id`.

### Context handling

If `context_name` does not match an existing context, **do not silently create a new one**.
The context must already exist — `create_resource` will fail if the context is not found.
Instead, ask the user: "I don't see a context named `{name}`. Would you like me to create
it, or did you mean one of these: {list existing contexts}?" Use `list_contexts` to fetch
the current list before asking.

### Discovering Document Types

Use `list_doc_types` to see what document types are available before creating content.
Common types include: `session`, `research`, `concept`, `task`, `goal`. The tool returns
an id and name for each type.

```
Tool: list_doc_types
Input: {}
```

Pass the `name` field as `doc_type_name` in `create_resource`.

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
  `offset`. Resources listing is capped at 200 items.
- **Access control is automatic** — you only see resources and contexts your
  authenticated profile has access to. No need to handle permissions.
