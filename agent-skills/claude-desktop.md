# Temper Knowledge Base — Claude Desktop MCP Guide

Instructions for Claude Desktop sessions connected to the Temper MCP server.
This content can be used as a system prompt supplement or placed in the MCP
server's `instructions` field (returned during `initialize`).

## Setup

Add the Temper MCP server to your Claude Desktop configuration
(`claude_desktop_config.json`):

```json
{
  "mcpServers": {
    "temper": {
      "url": "https://temperkb.io/mcp",
      "note": "Temper Knowledge Base — cloud vault access"
    }
  }
}
```

Authentication happens automatically via OAuth when you first connect. Claude
Desktop will prompt you to log in through Auth0.

## What You Have Access To

### Resources Panel (Browse & Inject)

The Temper server exposes your knowledge base as browsable MCP resources.
In Claude Desktop, these appear in the resources panel (attachment icon).

**Available resource templates:**

| URI Pattern | Returns |
|-------------|---------|
| `temper://resources/{id}` | Full resource: metadata (JSON) + markdown content |
| `temper://resources/{id}/content` | Raw markdown content only |
| `temper://contexts/{name}/resources` | All resources in a named context (JSON list) |

**How to use:** Click the resources panel, browse your contexts, and attach
relevant documents to the conversation. The content is injected into context
before the model runs — no tool call needed.

### Tools (Query & Mutate)

These are available as function calls during conversation:

**Read operations:**
- `list_contexts` — show all workspaces
- `get_context` — get details for a specific context
- `list_resources` — list resources, optionally filtered by context or doc type
- `get_resource` — get one resource by ID or slug (pass `include_content: true` for full markdown)
- `search` — semantic search across all resources
- `list_doc_types` — discover available document types
- `describe_doc_type` — get the JSON Schema and example_managed_meta for a specific type

**Write operations:**
- `create_resource` — add a new document to the knowledge base (include `content` field to write markdown in one call)
- `update_resource` — change a resource's title, slug, or content
- `delete_resource` — soft-delete a resource
- `create_context` — create a new workspace

**Utility:**
- `get_profile` — see your authenticated identity and preferences
- `list_events` — view recent activity for debugging

## Recommended Workflows

### "What do I have on X?"

1. Ask Claude to search: "Search my knowledge base for notes about authentication"
2. Claude uses the `search` tool to find relevant documents
3. Browse the results and ask Claude to read specific ones in full

### "Load my project context"

1. Attach resources from the panel: browse `temper://contexts/myproject/resources`
2. Select and attach the documents you want as context
3. Start working — Claude has your project knowledge loaded

### "Save what we discussed"

1. Ask Claude to create a resource with your conversation summary
2. Claude uses `create_resource` with a `content` field to persist it in the right context
3. The note is available in future sessions via resources

### "Write content to the knowledge base"

1. Use `list_doc_types` to see available document types
2. Optionally use `describe_doc_type` to get the JSON Schema and an `example_managed_meta` template
3. Use `list_contexts` to check existing contexts
4. Ask Claude to use `create_resource` with your content, context, doc type, and any `managed_meta` fields
5. The resource is created, validated, and embedded in a single call — returned immediately with an `id`

### "Update my notes"

1. Read the current document via resources
2. Ask Claude to update the title or metadata
3. Claude uses `update_resource` to make the change

## Tips for Effective Use

- **Attach resources before asking questions** — content injected via the
  resources panel is cheaper (no tool call overhead) and gives Claude full
  context upfront.
- **Use search for fuzzy discovery** — when you don't know exactly what you're
  looking for, the `search` tool does semantic matching across all your content.
- **Contexts are workspaces** — if you work across multiple projects, each has
  its own context. Browse by context to stay focused.
- **Everything is access-controlled** — you only see your own resources and
  contexts you have access to. No configuration needed.
- **Sessions are stateless** — each conversation with the MCP server is
  independent. There's no "memory" between conversations beyond what's stored
  in the knowledge base itself. That's the point — the vault *is* the memory.
