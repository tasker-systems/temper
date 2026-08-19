# MCP Content Creation Flow Design

**Date:** 2026-04-06
**Task:** `2026-04-06-temper-mcp-content-creation-agent-workflow`
**Branch:** `jct/mcp-content-creation-flow`
**Status:** Design approved, pending implementation

## Problem

The MCP server exposes 12 tools for reading and querying the knowledge base, but agents cannot write content back. `create_resource` creates a metadata shell with no content body. Two ingestion paths exist (Rust `POST /api/ingest` with pre-computed embeddings, TypeScript `POST /api/upload` with file upload), but neither is accessible through MCP tool calls.

This makes the knowledge base read-only for MCP-connected agents.

## Constraints

- **Vercel serverless cold starts** prevent running the ONNX embedding model synchronously in a Rust function. The model (`bge-base-en-v1.5`) requires downloading tokenizer + weights via hf-hub on cold start, which is non-viable as a synchronous wait. This is why the TypeScript workflow/queue pipeline exists.
- **Agents are the extraction layer.** When an agent reads a PDF, DOCX, or other document, it can produce curated markdown natively. The MCP content creation flow accepts markdown, not raw files. No kreuzberg/docling extraction is needed.
- **No blob storage needed.** The content is a markdown string, not a file. Vercel Blob is unnecessary overhead for this path.

## Architecture

Approach A: MCP tool (Rust) owns metadata and manifest creation, delegates content processing to a new TypeScript endpoint via internal HTTP call.

```
Agent
  │
  ├─ MCP tool: ingest_content
  │    ├─ resolve context_name → context_id
  │    ├─ resolve doc_type_name → doc_type_id
  │    ├─ compute body_hash (SHA256 of markdown)
  │    ├─ compute managed/open hashes (empty {})
  │    ├─ TX: INSERT kb_resources + kb_resource_manifests + event/audit
  │    ├─ POST /api/content-ingest { resource_id, content }
  │    └─ return { resource_id, title, context_name }
  │
  └─ /api/content-ingest (TypeScript)
       ├─ validate JWT + body
       ├─ trigger Vercel Workflow
       └─ return 202 Accepted
            │
            └─ Workflow: process-content-ingest
                 ├─ chunk (chunkText)
                 ├─ embed (embedTexts via ONNX)
                 └─ store (persist_resource_chunks SQL function)
```

### Responsibility Split

| Step | Owner | Details |
|------|-------|---------|
| Resolve context/doc_type | MCP tool (Rust) | Name → UUID, auto-create context if needed |
| Create resource + manifest | MCP tool (Rust) | INSERT with body_hash, managed/open hashes |
| `resource_created` event | MCP tool (Rust) | Event + audit row in same transaction |
| Chunk markdown | TS workflow | Split into sections via `chunkText()` |
| Embed chunks | TS workflow | ONNX `bge-base-en-v1.5` via `embedTexts()` |
| Store chunks | TS workflow | `persist_resource_chunks()` SQL function |
| `body_processed` event | TS workflow | Confirms content is searchable |

### Key Contract

The MCP tool computes `body_hash` (SHA256 of the markdown content) at resource creation time. The manifest row exists with the correct hash before the TS workflow runs. The TS workflow only adds chunks and embeddings — it does not touch the manifest or resource metadata.

## Components

### 1. `list_doc_types` MCP Tool

Simple read-only tool returning system-level document types.

**File:** `crates/temper-mcp/src/tools/doc_types.rs`

**Parameters:** None (doc types are system-level, not user-scoped).

**Service:** New `doc_type_service::list_all()` in temper-api — single query: `SELECT id, name, description FROM kb_doc_types ORDER BY name`.

**Returns:** JSON array of `{ id, name, description }`.

### 2. `ingest_content` MCP Tool

The core content creation tool.

**File:** `crates/temper-mcp/src/tools/ingest.rs`

**Parameters** (`schemars::JsonSchema` struct):

| Field | Type | Required | Notes |
|-------|------|----------|-------|
| `title` | String | yes | Resource title |
| `content` | String | yes | Markdown body |
| `context_name` | String | yes | Resolved to UUID server-side |
| `doc_type_name` | String | yes | Resolved to UUID server-side |
| `slug` | String | no | Auto-generated if omitted |
| `origin_uri` | String | no | Defaults to `mcp://agent/<resource_id>` |

**Flow:**

1. `ensure_profile_from_parts` (standard auth)
2. `context_service::resolve_by_name` → context_id (auto-creates if missing)
3. `ingest_service::resolve_doc_type` → doc_type_id (errors if not found)
4. Compute `body_hash` = `sha256:<hex>` of content bytes
5. `ingest_service::find_by_body_hash` — if match, return existing resource (dedup)
6. Compute `managed_hash` and `open_hash` from empty `{}`
7. **Transaction:** INSERT `kb_resources` + INSERT `kb_resource_manifests` + `insert_event_and_audit("resource_created", "create")`
8. POST `{ resource_id, content, replace: false }` to `/api/content-ingest` (fire and forget)
9. Return `{ resource_id, title, context_name }` to agent

**Visibility changes in `ingest_service.rs`:** `resolve_doc_type` and `find_by_body_hash` change from `async fn` to `pub async fn`. No logic changes.

**Refactor:** Extract `create_resource_with_manifest()` from `ingest_service::ingest()` so both the existing ingest handler and the new MCP tool share the INSERT logic (resource + manifest + event). Avoids SQL duplication.

### 3. `update_resource_content` MCP Tool

Re-ingest content for an existing resource.

**File:** Same as `ingest_content` — `crates/temper-mcp/src/tools/ingest.rs`

**Parameters:**

| Field | Type | Required | Notes |
|-------|------|----------|-------|
| `resource_id` | UUID | yes | Existing resource |
| `content` | String | yes | New markdown body |

**Flow:**

1. `ensure_profile_from_parts` (standard auth)
2. Verify `can_modify_resource(profile_id, resource_id)` — error if not authorized
3. Compute new `body_hash` from content
4. Upsert `kb_resource_manifests` with new body_hash (managed/open unchanged)
5. `insert_event_and_audit("body_updated", "update_body")`
6. POST `{ resource_id, content, replace: true }` to `/api/content-ingest`
7. Return `{ resource_id, status: "processing" }`

### 4. `POST /api/content-ingest` TypeScript Endpoint

**Entry point:** `api/content-ingest.ts` (thin Vercel Function)
**Business logic:** `packages/temper-cloud/src/content-ingest.ts`

**Request:**
- Method: POST
- Auth: Bearer JWT (validated via existing `authenticateRequest`)
- Body: `{ resource_id: string, content: string, replace: boolean }`

**Response:** `202 Accepted` with `{ resource_id, status: "processing" }`

**Logic:**
1. Validate JWT
2. Validate body (resource_id is UUID, content is non-empty, replace is boolean)
3. Trigger Vercel Workflow `process-content-ingest` with payload
4. Return 202

### 5. `process-content-ingest` Vercel Workflow

**File:** `api/workflows/process-content-ingest.ts`

**Steps** (reusing existing processing functions):

1. **Chunk** — `chunkText(content)` from `packages/temper-cloud/src/processing/chunk.ts`
2. **Embed** — `embedTexts(chunks.map(c => c.content))` from `packages/temper-cloud/src/processing/embed.ts`
3. **Store** — If `replace: false`, call `persist_resource_chunks(resource_id, chunks)`. If `replace: true`, call `replace_resource_chunks(resource_id, chunks)`.
4. **Event** — `insertEventAndAudit()` with event_type `body_processed`, action `process_content`

### 6. Routing

Add to `vercel.json` routes (before the Axum catch-all):

```json
{ "src": "/api/content-ingest", "dest": "/api/content-ingest.ts" }
```

### 7. Agent Skills Documentation

**`agent-skills/knowledge-base.md`** — Add "Writing Content" section:
- `ingest_content` tool: parameters, async behavior, dedup
- `list_doc_types` tool: discovery of valid doc types
- Recommended doc types for common tasks (session, research, concept, etc.)
- Resource_id returned immediately; content becomes searchable shortly after
- Context prompting: if context_name doesn't match an existing context, the agent should prompt the user before creating a new one

**`agent-skills/claude-desktop.md`** — Add content creation workflow section:
- `ingest_content` is the primary content creation path
- No manual HTTP upload needed
- The MCP connector handles auth transparently

**`agent-skills/SKILL.md`** — Update capability summary to include content creation.

## Testing

### Rust E2e Tests

**File:** `tests/e2e/tests/content_ingest_test.rs`

Using existing `E2eTestApp` infrastructure:

1. `list_doc_types` returns system doc types (research, session, etc.)
2. `ingest_content` creates resource with correct manifest hashes
3. Dedup: same content hash returns existing resource_id
4. Unknown doc_type_name returns error
5. Context auto-creation works for new context_name
6. `update_resource_content` verifies ownership before updating
7. `update_resource_content` updates manifest body_hash

### TypeScript Tests

**File:** `packages/temper-cloud/tests/content-ingest.test.ts`

1. Validation: rejects missing resource_id, empty content, invalid UUID
2. Workflow trigger: verify workflow is called with correct payload
3. Replace flag: verify `persist` vs `replace` SQL function selection

Existing `chunk.test.ts` and `embed.test.ts` already cover the processing functions.

### Unit Tests

- `ingest_service::create_resource_with_manifest` — extracted function, tested in isolation
- `doc_type_service::list_all` — returns expected rows from test database

## Not In Scope

- **MCP auth for Claude Code** — localhost redirect issue with the MCP connector is a separate concern from content creation
- **Crate dependency cleanup** — `temper-mcp` depending directly on `temper-api` is architecturally ugly but functional; extracting a shared service layer is a future task
- **Blob storage** — not needed for markdown content; the upload endpoint (`POST /api/upload`) remains for file-based ingestion
- **Raw file upload via MCP** — agents extract content themselves; if raw file upload is needed later, it's a separate tool

## Follow-Up Tasks

1. **Crate graph cleanup:** Extract shared service traits so `temper-mcp` doesn't depend on `temper-api` directly. Target: `temper-services` crate or trait-based abstraction in `temper-core`.
2. **MCP auth for Claude Code:** Investigate the localhost redirect issue that prevents the MCP connector from completing OAuth in Claude Code (works in Claude Desktop).
3. **Content status polling:** Consider a `get_ingest_status` tool that lets agents check whether content processing is complete (useful for large documents with slow embedding).
