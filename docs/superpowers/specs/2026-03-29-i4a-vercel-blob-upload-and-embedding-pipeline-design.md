# I4a: Vercel Blob Upload & Embedding Pipeline

**Date:** 2026-03-29
**Ticket:** 2026-03-29-i4a-vercel-blob-file-storage-upload-endpoints-and-embedding-pipeline
**Branch:** jcoletaylor/temper-cloud
**Scope:** feature

## Summary

Add file upload and durable document processing to the temper-cloud deployment. Files are stored in Vercel Blob, then extracted, chunked, and embedded via a Vercel Workflow running kreuzberg-node and ONNX (bge-base-en-v1.5, 768-dim). Two runtimes coexist in one Vercel project: Rust (existing API) and TypeScript (upload + async processing).

## Architecture

### Two Runtimes, One Project

```
Vercel Project: temper-cloud
├── api/axum.rs          (Rust)  — auth, resources, profiles, search, events, health
├── api/upload.ts        (TS)    — file upload → Vercel Blob → trigger workflow
└── api/workflows/
    └── process-upload.ts (TS)   — durable: extract → chunk → embed → store
```

Vercel discovers each `api/` file as a separate function with its own runtime. Rust and TypeScript share the same environment variables (`DATABASE_URL`, `JWKS_URL`, `AUTH_ISSUER`, etc.).

### Request Flow

```
1. Client → POST /api/resources (Rust)
   ← 201 { resource_id }

2. Client → POST /api/upload (TypeScript) + resource_id + file
   → Verify JWT (same JWKS as Rust)
   → Validate resource_id belongs to profile (query Neon)
   → Store file in Vercel Blob (private)
   → Insert blob_files record (status: pending)
   → Trigger processing workflow
   ← 202 { blob_file_id, status: "pending" }

3. Vercel Workflow (async, durable)
   → Step 1: Extract text via @kreuzberg/node
   → Step 2: Chunk (header_path, content_hash, versioning)
   → Step 3: Embed via ONNX (bge-base-en-v1.5, 768-dim)
   → Step 4: Store chunks + embeddings in kb_chunks

4. Client → GET /api/resources/{id} (Rust)
   ← includes processing status from blob_files
```

### Resource-First, Upload-Second

The resource record is created via the Rust API first, establishing the profile/context/doctype associations. The file upload references this `resource_id`. This means:
- temper-client (I5) abstracts the two-step flow into a single `client.add_resource()` call
- CLI and MCP don't need to know about the endpoint split
- All resource metadata (context, doctype, tags) is set before upload begins

### Auth Parity

The TypeScript upload endpoint verifies JWTs using the `jose` library with `createRemoteJWKSet()`. Same JWKS URL, same issuer, same EdDSA algorithm, same tokens. A valid JWT for the Rust API works for the TypeScript endpoint and vice versa.

## TypeScript Project Structure

```
packages/temper-cloud/
  package.json            — bun + vitest
  tsconfig.json
  src/
    auth.ts               — JWT verification (jose + JWKS)
    upload.ts             — blob storage + db record logic
    workflow/
      extract.ts          — kreuzberg-node text extraction
      chunk.ts            — chunking (header_path, content_hash, versioning)
      embed.ts            — ONNX bge-base-en-v1.5 (768-dim)
      store.ts            — write chunks + embeddings to kb_chunks
  tests/
    auth.test.ts
    upload.test.ts
    workflow/
      extract.test.ts
      chunk.test.ts
      embed.test.ts
      store.test.ts

api/upload.ts             — thin entry: imports from packages/temper-cloud
api/workflows/
  process-upload.ts       — thin entry: workflow + step directives
```

`api/` files are thin entry points (~10-20 lines). All business logic lives in `packages/temper-cloud/src/` where vitest can test it. The `'use workflow'` and `'use step'` directives stay in the `api/` entry points since they're Vercel compile-time directives.

Root `package.json` uses bun workspaces to reference `packages/temper-cloud`.

### Key Dependencies

| Package | Purpose |
|---------|---------|
| `@vercel/blob` | Vercel Blob storage SDK |
| `@kreuzberg/node` | Document extraction (91+ formats, napi-rs) |
| `onnxruntime-node` | ONNX Runtime for bge-base-en-v1.5 embedding |
| `jose` | JWT verification with JWKS |
| `@neondatabase/serverless` | Postgres driver for Vercel functions |
| `workflow` | Vercel Workflow Development Kit |
| `vitest` | Test runner (dev dependency) |

### temper-embed Alignment

The TypeScript pipeline is the reference implementation. The Rust temper-embed crate will follow the same patterns — kreuzberg and ONNX are the same native code underneath regardless of whether they're called from TypeScript (napi-rs) or Rust directly. Same model, same dimensions, same chunking strategy, deterministic output.

## Schema — blob_files Table

Migration `20260329000001_blob_files.sql`:

```sql
CREATE TABLE blob_files (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    profile_id      UUID NOT NULL REFERENCES kb_profiles(id),
    resource_id     UUID REFERENCES resources(id),
    blob_url        TEXT NOT NULL,
    pathname        TEXT NOT NULL,
    content_type    TEXT,
    file_size_bytes BIGINT,
    status          TEXT NOT NULL DEFAULT 'pending'
                    CHECK (status IN ('pending', 'processing', 'processed', 'failed')),
    error_message   TEXT,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX idx_blob_files_profile ON blob_files(profile_id);
CREATE INDEX idx_blob_files_resource ON blob_files(resource_id);
CREATE INDEX idx_blob_files_status ON blob_files(status);
```

### Design Decisions

- **`profile_id` only, no `team_id`**: Upload actor is always one authenticated profile. Access control flows through the resource.
- **Optional `resource_id`**: FK for associating files with resources. Resource is created first via the Rust API.
- **`status` lifecycle**: `pending → processing → processed → failed`. Workflow updates status at each transition.
- **`error_message`**: Captures failure diagnostics when status is `failed`.
- **`pathname`**: Vercel Blob storage key (e.g., `{profile_id}/{resource_id}/{filename}`).
- **`blob_url`**: URL returned by Vercel Blob for file retrieval.

## Upload Endpoint

`api/upload.ts` receives a multipart form upload:
- `resource_id` (required) — UUID of the previously created resource
- `file` — the file content

Flow:
1. Verify JWT (jose + JWKS from env) — extract `profile_id` from claims
2. Validate `resource_id` belongs to the authenticated profile (query Neon via `resources_visible_to()`)
3. Store file in Vercel Blob (`put()`, private access, pathname: `{profile_id}/{resource_id}/{filename}`)
4. Insert `blob_files` record with status `pending`
5. Trigger the processing workflow (HTTP call to workflow route with `blob_file_id`)
6. Return 202 Accepted with `{ blob_file_id, status: "pending" }`

## Processing Workflow

`api/workflows/process-upload.ts` — four durable steps via Vercel Workflow:

### Step 1: Extract

Fetch file from `blob_url`, pass to `@kreuzberg/node` `extractFile()`. Update `blob_files.status` to `processing`. Returns extracted text + detected content type.

### Step 2: Chunk

Split extracted text into chunks following the `kb_chunks` schema:
- `header_path` — extracted from markdown headers (e.g., `## Section > ### Subsection`)
- `content_hash` — SHA-256 of chunk content (dedup and versioning)
- `chunk_index` — sequential position in document
- `version` — starts at 1, increments on re-upload of same resource
- On re-upload: existing chunks with matching `resource_id` get `is_current = false`, new chunks get `is_current = true`

### Step 3: Embed

Load bge-base-en-v1.5 via `onnxruntime-node`, batch-embed all chunks. Model downloaded to `/tmp` on first invocation (Vercel provides 500MB scratch space). Produces 768-dim vectors.

### Step 4: Store

Write chunks + embeddings to `kb_chunks` in a single transaction via `@neondatabase/serverless`. Update `blob_files.status` to `processed`. On failure at any step, status goes to `failed` with `error_message`.

Each step is independently retryable via `'use step'`. If step 3 fails (model load issue), it retries without re-running extraction or chunking.

## Vercel Configuration

Updated `vercel.json`:

```json
{
  "$schema": "https://openapi.vercel.sh/vercel.json",
  "fluid": true,
  "rewrites": [
    { "source": "/api/upload", "destination": "/api/upload" },
    { "source": "/api/workflows/(.*)", "destination": "/api/workflows/$1" },
    { "source": "/(.*)", "destination": "/api/axum" }
  ]
}
```

Upload and workflow routes match first (TypeScript), everything else falls through to axum (Rust). Order matters — specific routes before catch-all.

Root `package.json`:

```json
{
  "private": true,
  "workspaces": ["packages/temper-cloud"]
}
```

## Environment Variables (New)

| Variable | Purpose |
|----------|---------|
| `BLOB_READ_WRITE_TOKEN` | Vercel Blob storage authentication |

All existing env vars (`DATABASE_URL`, `JWKS_URL`, `AUTH_ISSUER`, etc.) are shared by both runtimes.

## Testing Strategy

### Unit Tests (vitest)

In `packages/temper-cloud/tests/`:
- `auth.test.ts` — JWT verification with valid/expired/missing tokens using test Ed25519 keypair (same keys as Rust tests)
- `chunk.test.ts` — header path extraction, content hashing, versioning, edge cases (empty doc, no headers, large doc)
- `embed.test.ts` — ONNX model loads, produces 768-dim vectors, deterministic output for same input
- `extract.test.ts` — kreuzberg extracts text from markdown, PDF
- `store.test.ts` — SQL generation for chunk upsert, version bumping logic

### Integration Tests (require database)

- Full pipeline: upload file → extract → chunk → embed → verify kb_chunks rows
- Re-upload: verify version increments, `is_current` flags updated
- Auth rejection: invalid JWT returns 401

Tests use the same local Docker Postgres as the Rust tests (`temper_test` database, port 5437).

## Out of Scope

- **temper-client / CLI integration** — I5 builds the client that abstracts both endpoints
- **temper-embed Rust crate implementation** — follows the same patterns later
- **Resource status enrichment** — enriching `GET /api/resources/{id}` with blob processing status is a small follow-up
- **Retry configuration / dead letter** — Vercel Workflow handles retries natively
- **Neon Previews integration for TypeScript** — manual for now, automated in I11
