# MCP-CLI Resource Parity

**Date:** 2026-04-07
**Task:** 2026-04-07-rework-mcp-tools-to-follow-resource-crud-patterns
**Branch:** jct/mcp-cli-resource-parity
**Status:** approved

---

## 1. Problem

The MCP server has 14 tools with overlapping responsibilities, UUID-only interfaces, and
inconsistent patterns compared to the CLI's unified resource commands. Agents must juggle
`create_resource` (metadata-only) and `ingest_content` (with content) as separate tools,
call `get_resource_content` separately from `get_resource`, and work with raw UUIDs for
contexts and doc types — requiring discovery calls before any real operation.

The CLI was recently unified around `temper resource {create,list,show,update}` with
name-based resolution (`--type task`, `--context temper`). The MCP tools should follow
the same patterns: fewer tools, richer per-tool capability, name-based interfaces.

## 2. Design Principles

- **Name-based primary interface.** Agents interact with context names, doc type names,
  and slugs — not UUIDs. UUIDs appear in responses for cross-referencing but are not
  required as input except for `update` and `delete` (where ID is unambiguous).
- **MCP as adapter layer.** The MCP tool handlers resolve names to UUIDs and call the
  existing service layer. The service layer stays UUID-based and is shared with the REST
  API. Both adapters produce the same business logic outcomes.
- **Forward-compatible with R11.** All tools accept an optional `owner` parameter
  (defaults to `@me`) that is a no-op today but establishes the parameter shape for
  owner-scoped URIs and team support. No team-specific logic is implemented here.
- **Explicit context creation.** Contexts must exist before being referenced. No
  auto-creation on typos. Agents use `create_context` to create new contexts.
- **Content is optional on create, unified on update.** One `create_resource` tool
  handles both metadata-only and content-bearing creation. One `update_resource` tool
  handles both metadata and content updates.

## 3. Tool Inventory

### Before (14 tools)

| Tool | Issue |
|------|-------|
| `create_resource` | Metadata-only, UUID-based context/doc_type |
| `ingest_content` | Content creation, name-based — redundant with create |
| `get_resource` | UUID-only lookup |
| `get_resource_content` | Separate tool for content — should be unified with get |
| `update_resource` | Only title/slug |
| `update_resource_content` | Separate content update — should be unified with update |
| `delete_resource` | Fine |
| `search` | Fine |
| `list_contexts` / `get_context` / `create_context` | Fine |
| `list_doc_types` | Fine |
| `list_events` | Fine |
| `get_profile` | Fine |

### After (12 tools)

| Tool | Change |
|------|--------|
| `create_resource` | **Replaces** `create_resource` + `ingest_content` |
| `get_resource` | **Replaces** `get_resource` + `get_resource_content` |
| `list_resources` | **Enhanced** with name-based filters |
| `update_resource` | **Replaces** `update_resource` + `update_resource_content` |
| `delete_resource` | Unchanged |
| `search` | Unchanged |
| `list_contexts` / `get_context` / `create_context` | Unchanged |
| `list_doc_types` | Unchanged |
| `list_events` | Unchanged |
| `get_profile` | Unchanged |

## 4. Input Shapes

### 4.1 `create_resource`

```rust
#[derive(Debug, Deserialize, JsonSchema)]
pub struct CreateResourceInput {
    /// Human-readable context name (must already exist).
    pub context_name: String,
    /// Human-readable doc type name (e.g. "task", "session", "research").
    pub doc_type_name: String,
    /// Resource title.
    pub title: String,
    /// Optional markdown content body. If provided, triggers async
    /// chunk/embed processing via content-ingest POST.
    pub content: Option<String>,
    /// Optional URL-friendly slug.
    pub slug: Option<String>,
    /// Optional origin URI. Defaults to mcp://agent/<uuid>.
    pub origin_uri: Option<String>,
    /// Optional owner (defaults to @me). Reserved for future team scoping.
    pub owner: Option<String>,
}
```

**Behavior:**

1. Resolve context by name via `context_service::resolve_by_name` — error if not found.
2. Resolve doc type by name via `ingest_service::resolve_doc_type`.
3. If content provided, compute sha256 body hash and check for duplicates via
   `ingest_service::find_by_body_hash`. Return existing resource if duplicate found.
4. Create resource + manifest + event via `ingest_service::create_resource_with_manifest`.
5. If content provided, fire-and-forget POST to `/api/content-ingest` for async
   chunk/embed processing.
6. Return enriched response with context_name and doc_type_name.

**Tool description:** "Create a new resource in the knowledge base. Optionally include
markdown content for indexing and search. Context must already exist — use create_context
first if needed. Use list_doc_types to see available types."

### 4.2 `get_resource`

```rust
#[derive(Debug, Deserialize, JsonSchema)]
pub struct GetResourceInput {
    /// UUID of the resource. Provide either id or slug (not both).
    pub id: Option<Uuid>,
    /// Slug of the resource. Requires context_name for disambiguation.
    pub slug: Option<String>,
    /// Context name. Required when looking up by slug.
    pub context_name: Option<String>,
    /// If true, includes the full reconstituted markdown content.
    pub include_content: Option<bool>,
}
```

**Behavior:**

1. Validate: exactly one of `id` or `slug` must be provided. If `slug`, require
   `context_name`.
2. Look up resource — by ID via existing `resource_service::get_visible`, or by slug
   via new `resource_service::get_by_slug`.
3. If `include_content` is true, fetch reconstituted markdown via
   `resource_service::get_content`.
4. Return metadata + optional content in a single response.

**Tool description:** "Get a resource by ID or slug. When using slug, provide
context_name to disambiguate. Set include_content to true to get the full markdown."

### 4.3 `list_resources`

```rust
#[derive(Debug, Deserialize, JsonSchema)]
pub struct ListResourcesInput {
    /// Filter by context name.
    pub context_name: Option<String>,
    /// Filter by doc type name (e.g. "task", "research").
    pub doc_type_name: Option<String>,
    /// Max results (default 50, max 200).
    pub limit: Option<i64>,
    /// Pagination offset.
    pub offset: Option<i64>,
}
```

**Behavior:**

1. Resolve context_name to ID if provided.
2. Resolve doc_type_name to ID if provided.
3. Delegate to enhanced `resource_service::list_visible` with both optional filters.
4. Results sorted by `updated DESC` (most recent first — already the case).
5. Return enriched rows with context_name and doc_type_name.

**Tool description:** "List resources in the knowledge base. Filter by context and/or
document type. Returns most recent first."

### 4.4 `update_resource`

```rust
#[derive(Debug, Deserialize, JsonSchema)]
pub struct UpdateResourceInput {
    /// UUID of the resource to update.
    pub id: Uuid,
    /// New title.
    pub title: Option<String>,
    /// New slug.
    pub slug: Option<String>,
    /// New markdown content. Replaces existing content and triggers
    /// async re-processing (chunking, embedding, indexing).
    pub content: Option<String>,
}
```

**Behavior:**

1. Auth check via `can_modify_resource` before any mutation.
2. If title/slug provided, update via `resource_service::update`.
3. If content provided, compute body hash, update manifest via
   `ingest_service::update_resource_manifest`, fire content-ingest POST with
   `replace: true`.
4. Return enriched response.

**Tool description:** "Update a resource's title, slug, or content. Only provided
fields are changed. New content triggers re-indexing."

### 4.5 `delete_resource` — unchanged

```rust
pub struct DeleteResourceInput {
    pub id: Uuid,
}
```

## 5. Response Enrichment

Current responses return raw `kb_context_id` and `kb_doc_type_id` UUIDs, requiring
agents to cross-reference with `list_contexts`/`list_doc_types`. All resource-returning
tools will enrich responses with human-readable names.

**Approach:** The MCP tool handler resolves names after getting the resource row.
Context name comes from `context_service` (already has lookup-by-ID functions), doc type
name from a new `doc_type_service::get_name` function or a join in the query. This keeps
the service layer queries simple and avoids a new `ResourceRowEnriched` type that the
REST API doesn't need.

Response format for resource tools:

```json
{
  "id": "019d6885-...",
  "title": "My research note",
  "slug": "2026-04-07-my-research-note",
  "context_name": "temper",
  "doc_type_name": "research",
  "owner": "@me",
  "origin_uri": "mcp://agent/...",
  "is_active": true,
  "created": "2026-04-07T...",
  "updated": "2026-04-07T..."
}
```

When content is included (via `include_content: true` on get, or as confirmation on
create/update), it appears as a separate content block in the tool result:

```json
[
  { "type": "text", "text": "<resource metadata JSON>" },
  { "type": "text", "text": "<markdown content>" }
]
```

## 6. Service Layer Changes

All changes are additive — existing functions are not modified in breaking ways.

### 6.1 `resource_service`

**`list_visible` — add doc_type filter.** The current function branches on
`kb_context_id` presence. Add a `kb_doc_type_id: Option<Uuid>` parameter and extend the
query with an optional `AND r.kb_doc_type_id = $N` clause. This means the function
handles four combinations (no filter, context only, doc_type only, both). Use a runtime
query builder or four query branches — match the existing pattern of branching on
`Option` presence.

**`get_by_slug` — new function.** Look up an active resource by slug within a context,
scoped to profile visibility:

```sql
WITH visible AS (SELECT resource_id FROM resources_visible_to($1))
SELECT r.* FROM kb_resources r
  JOIN visible v ON v.resource_id = r.id
 WHERE r.slug = $2
   AND r.kb_context_id = $3
   AND r.is_active = true
```

Returns `ApiResult<ResourceRow>` — `NotFound` if no match.

### 6.2 `doc_type_service`

**`get_by_name` — new function** (or verify it exists). Look up a doc type row by name.
Used by MCP handler for response enrichment. May already be covered by
`ingest_service::resolve_doc_type` which returns just the UUID — if so, either reuse
that or add a variant that returns the full row or just the name.

### 6.3 No changes to `ingest_service`

All ingest functions (`create_resource_with_manifest`, `update_resource_manifest`,
`find_by_body_hash`, `resolve_doc_type`) are used as-is. The MCP handler orchestrates
calls to these functions the same way `ingest_content` does today.

## 7. Files Changed

| File | Change |
|------|--------|
| `crates/temper-mcp/src/tools/resources.rs` | Rewrite — new input structs, consolidated handlers with name resolution |
| `crates/temper-mcp/src/tools/ingest.rs` | **Delete** — content creation/update logic moves to resources.rs |
| `crates/temper-mcp/src/tools/mod.rs` | Remove `ingest` module |
| `crates/temper-mcp/src/service.rs` | Update tool registrations — remove `ingest_content`, `update_resource_content`, `get_resource_content`; update descriptions for consolidated tools |
| `crates/temper-api/src/services/resource_service.rs` | Add `get_by_slug`, add doc_type_id filter to `list_visible` |
| `crates/temper-api/src/services/doc_type_service.rs` | Add `get_name` or `get_by_id` if needed for enrichment |
| `crates/temper-core/src/types/resource.rs` | Update `ResourceListParams` if we want the doc_type filter at the type level (optional — MCP handler can resolve before calling) |

## 8. What's NOT Changing

- **REST API handlers** — the HTTP API keeps its current parameter shapes. It benefits
  from the new service-layer functions but its interface is untouched.
- **Context tools** — `list_contexts`, `get_context`, `create_context` stay as-is.
- **Search, events, profile, doc_types tools** — unchanged.
- **Metadata/frontmatter updates** (stage, mode, effort, tags) — deferred. The
  `update_resource` tool handles title, slug, and content only.
- **Team/owner resolution** — the `owner` parameter is accepted and validated (must
  start with `@` or `+`) but is not resolved or used. It establishes the parameter
  shape for R11.
- **`ResourceListParams` in temper-core** — the shared type can stay as-is if the MCP
  handler resolves names to UUIDs before calling the service. Whether to push the
  doc_type filter into the shared type or keep it as a service-layer parameter is an
  implementation detail.

## 9. Testing

- **Unit tests** for name resolution logic in MCP handlers (mock service responses).
- **Integration tests** (`test-db` feature) for new service functions:
  `get_by_slug`, `list_visible` with doc_type filter.
- **Existing tests** must continue to pass — the service layer changes are additive.
- **MCP tool registration** — verify the tool count is 12 and descriptions are accurate
  (can be checked by inspecting the tool router).

## 10. Forward Compatibility (R11)

The `owner` parameter on `create_resource` accepts `@profile-slug` or `+team-slug` and
defaults to `@me`. Today it is validated for format but not resolved. When R11 lands:

1. `create_resource` resolves owner to a profile or team, then uses owner-scoped
   context resolution (context names scoped to owner namespace).
2. `list_resources` and `get_resource` gain the same `owner` parameter for filtering.
3. Response enrichment adds the resolved owner to the output.
4. The `owner` parameter shape does not change — only the resolution logic behind it.
