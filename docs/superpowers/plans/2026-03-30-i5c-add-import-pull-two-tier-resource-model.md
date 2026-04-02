# I5c: Add, Import, Pull — Two-Tier Resource Model Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Implement the two-tier resource model — `temper add`, `temper import`, `temper pull`, `temper remove` — with CLI-side kreuzberg extraction and server-side inline chunk+embed processing via a new `/api/ingest` endpoint.

**Architecture:** CLI extracts files to markdown locally via kreuzberg (Rust, feature-flagged). Extracted markdown is sent to a new TypeScript `/api/ingest` endpoint that creates the resource record and runs chunk → embed → store inline. For large documents (>2MB markdown), the CLI falls back to the existing blob upload path. A shared processing library is extracted from the existing Vercel Workflow so both paths use the same chunk/embed/store logic.

**Tech Stack:** Rust (clap, reqwest, kreuzberg, indicatif, sha2), TypeScript (Neon serverless, @kreuzberg/node, onnxruntime-node, jose), Vercel serverless functions

**Spec:** `docs/superpowers/specs/2026-03-30-i5c-add-import-pull-two-tier-resource-model-design.md`

---

## File Structure

### TypeScript — New Files

| File | Responsibility |
|------|---------------|
| `packages/temper-cloud/src/processing/chunk.ts` | Shared `chunkText()` — moved from workflow |
| `packages/temper-cloud/src/processing/embed.ts` | Shared `embedTexts()` — moved from workflow |
| `packages/temper-cloud/src/processing/store.ts` | Shared store logic — moved from workflow |
| `packages/temper-cloud/src/processing/index.ts` | Barrel export |
| `packages/temper-cloud/src/ingest.ts` | Ingest helpers: auth, profile lookup, resource insert, hash check |
| `api/ingest.ts` | `POST /api/ingest` — create resource + inline process |
| `api/ingest/[id].ts` | `PUT /api/ingest/:id` — update content for existing resource |

### TypeScript — Modified Files

| File | Change |
|------|--------|
| `packages/temper-cloud/src/workflow/chunk.ts` | Re-export from `../processing/chunk` |
| `packages/temper-cloud/src/workflow/embed.ts` | Re-export from `../processing/embed` |
| `packages/temper-cloud/src/workflow/store.ts` | Re-export from `../processing/store` |

### Rust — New Files

| File | Responsibility |
|------|---------------|
| `crates/temper-core/src/types/ingest.rs` | `IngestRequest`, `IngestResponse` types |
| `crates/temper-client/src/ingest.rs` | `IngestClient` — POST/PUT to `/api/ingest` |
| `crates/temper-cli/src/extract.rs` | kreuzberg wrapper behind `extract` feature flag |
| `crates/temper-cli/src/manifest_io.rs` | Manifest read/write to `<vault>/.temper/manifest.json` |
| `crates/temper-cli/src/output.rs` | Progress bar and JSON output helpers (if not already present) |
| `crates/temper-cli/src/commands/add.rs` | `temper add` handler |
| `crates/temper-cli/src/commands/import_cmd.rs` | `temper import` handler (`import` is a Rust keyword) |
| `crates/temper-cli/src/commands/pull.rs` | `temper pull` handler |
| `crates/temper-cli/src/commands/remove.rs` | `temper remove` handler |

### Rust — Modified Files

| File | Change |
|------|--------|
| `crates/temper-core/src/types/mod.rs` | Export ingest types |
| `crates/temper-client/src/lib.rs` | Add `ingest()` sub-client accessor |
| `crates/temper-client/Cargo.toml` | No new deps needed (reqwest multipart already available) |
| `crates/temper-cli/Cargo.toml` | Add kreuzberg (feature-flagged), indicatif, ignore (gitignore) |
| `crates/temper-cli/src/cli.rs` | Add `Add`, `Import`, `Pull`, `Remove` to `Commands` enum |
| `crates/temper-cli/src/main.rs` | Add dispatch arms |
| `crates/temper-cli/src/commands/mod.rs` | Register new command modules |

---

## Task 1: Shared Processing Library (TypeScript)

Refactor existing workflow functions into a shared `processing/` module. Both the Vercel Workflow and the new `/api/ingest` endpoint will import from here.

**Files:**
- Create: `packages/temper-cloud/src/processing/chunk.ts`
- Create: `packages/temper-cloud/src/processing/embed.ts`
- Create: `packages/temper-cloud/src/processing/store.ts`
- Create: `packages/temper-cloud/src/processing/index.ts`
- Modify: `packages/temper-cloud/src/workflow/chunk.ts`
- Modify: `packages/temper-cloud/src/workflow/embed.ts`
- Modify: `packages/temper-cloud/src/workflow/store.ts`

- [ ] **Step 1: Move `chunkText` to processing module**

Copy the full implementation from `packages/temper-cloud/src/workflow/chunk.ts` to `packages/temper-cloud/src/processing/chunk.ts`. Keep the same `Chunk` interface and `chunkText` function signature:

```typescript
// packages/temper-cloud/src/processing/chunk.ts
import { createHash } from "node:crypto";

export interface Chunk {
  chunk_index: number;
  header_path: string;
  content: string;
  content_hash: string;
}

export function chunkText(text: string): Chunk[] {
  // ... existing implementation from workflow/chunk.ts
}
```

Then replace `workflow/chunk.ts` with a re-export:

```typescript
// packages/temper-cloud/src/workflow/chunk.ts
export { chunkText, type Chunk } from "../processing/chunk.js";
```

- [ ] **Step 2: Move `embedTexts` to processing module**

Copy the full implementation from `packages/temper-cloud/src/workflow/embed.ts` to `packages/temper-cloud/src/processing/embed.ts`. Keep the same `EMBEDDING_DIM` constant and `embedTexts` function:

```typescript
// packages/temper-cloud/src/processing/embed.ts
export const EMBEDDING_DIM = 768;
export async function embedTexts(texts: string[]): Promise<number[][]> {
  // ... existing implementation from workflow/embed.ts
}
```

Replace `workflow/embed.ts` with re-export:

```typescript
// packages/temper-cloud/src/workflow/embed.ts
export { embedTexts, EMBEDDING_DIM } from "../processing/embed.js";
```

- [ ] **Step 3: Move store logic to processing module**

Copy the full implementation from `packages/temper-cloud/src/workflow/store.ts` to `packages/temper-cloud/src/processing/store.ts`. Keep all interfaces and query builder functions:

```typescript
// packages/temper-cloud/src/processing/store.ts
export interface ChunkRow { /* ... existing */ }
export function buildVersionBumpQuery(resourceId: string, newVersion: number) { /* ... */ }
export function buildStoreChunksQuery(chunks: ChunkRow[]) { /* ... */ }
export function buildStatusUpdateQuery(blobFileId: string, status: string, errorMessage?: string) { /* ... */ }
```

Replace `workflow/store.ts` with re-export:

```typescript
// packages/temper-cloud/src/workflow/store.ts
export {
  type ChunkRow,
  buildVersionBumpQuery,
  buildStoreChunksQuery,
  buildStatusUpdateQuery,
} from "../processing/store.js";
```

- [ ] **Step 4: Create barrel export**

```typescript
// packages/temper-cloud/src/processing/index.ts
export { chunkText, type Chunk } from "./chunk.js";
export { embedTexts, EMBEDDING_DIM } from "./embed.js";
export {
  type ChunkRow,
  buildVersionBumpQuery,
  buildStoreChunksQuery,
  buildStatusUpdateQuery,
} from "./store.js";
```

- [ ] **Step 5: Verify TypeScript compiles and existing workflow still works**

Run:
```bash
cd packages/temper-cloud && bun run tsc --noEmit && tsc --noEmit --project tsconfig.api.json
```

Expected: No type errors. The workflow imports resolve through the re-export wrappers.

- [ ] **Step 6: Commit**

```bash
git add packages/temper-cloud/src/processing/ packages/temper-cloud/src/workflow/chunk.ts packages/temper-cloud/src/workflow/embed.ts packages/temper-cloud/src/workflow/store.ts
git commit -m "refactor: extract shared processing library from workflow modules"
```

---

## Task 2: Ingest Helpers (TypeScript)

Create shared helper functions for the ingest endpoints — auth, profile lookup, resource insert, content hash check, and the full inline processing pipeline.

**Files:**
- Create: `packages/temper-cloud/src/ingest.ts`

- [ ] **Step 1: Create ingest helper module**

This module provides the core logic that both `POST /api/ingest` and `PUT /api/ingest/[id]` will use. Read `api/upload.ts` for the auth and profile lookup patterns to follow.

```typescript
// packages/temper-cloud/src/ingest.ts
import type { NeonQueryFunction } from "@neondatabase/serverless";
import type { AuthClaims } from "./auth.js";
import type { Chunk } from "./processing/chunk.js";

export interface IngestMetadata {
  title: string;
  kb_context_id: string;
  kb_doc_type_id: string;
  uri: string;
  slug?: string;
  mimetype?: string;
  tags?: string[];
  metadata?: Record<string, unknown>;
}

export interface ResourceRecord {
  id: string;
  kb_context_id: string;
  kb_doc_type_id: string;
  uri: string;
  title: string;
  slug: string | null;
  content_hash: string | null;
  mimetype: string | null;
  originator_profile_id: string;
  owner_profile_id: string;
  is_active: boolean;
  created: string;
  updated: string;
}

/** Look up the profile_id for an auth claim. Returns null if no profile found. */
export async function getProfileId(
  db: NeonQueryFunction<false, false>,
  claims: AuthClaims,
): Promise<string | null> {
  const rows = await db`
    SELECT id FROM kb_profiles
    WHERE auth_provider_sub = ${claims.sub}
    LIMIT 1
  `;
  return rows.length > 0 ? rows[0].id : null;
}

/** Check if a resource with matching content_hash exists for this profile. */
export async function findByContentHash(
  db: NeonQueryFunction<false, false>,
  contentHash: string,
  profileId: string,
): Promise<ResourceRecord | null> {
  const rows = await db`
    SELECT * FROM resources
    WHERE content_hash = ${contentHash}
      AND owner_profile_id = ${profileId}::uuid
      AND is_active = true
    LIMIT 1
  `;
  return rows.length > 0 ? (rows[0] as ResourceRecord) : null;
}

/** Insert a new resource record. Returns the created resource. */
export async function insertResource(
  db: NeonQueryFunction<false, false>,
  meta: IngestMetadata,
  contentHash: string,
  profileId: string,
): Promise<ResourceRecord> {
  const rows = await db`
    INSERT INTO resources (
      kb_context_id, kb_doc_type_id, uri, title, slug, mimetype,
      content_hash, originator_profile_id, owner_profile_id
    ) VALUES (
      ${meta.kb_context_id}::uuid,
      ${meta.kb_doc_type_id}::uuid,
      ${meta.uri},
      ${meta.title},
      ${meta.slug ?? null},
      ${meta.mimetype ?? null},
      ${contentHash},
      ${profileId}::uuid,
      ${profileId}::uuid
    )
    RETURNING *
  `;
  return rows[0] as ResourceRecord;
}

/** Update the content_hash on an existing resource. */
export async function updateResourceHash(
  db: NeonQueryFunction<false, false>,
  resourceId: string,
  contentHash: string,
): Promise<ResourceRecord> {
  const rows = await db`
    UPDATE resources
    SET content_hash = ${contentHash}, updated = now()
    WHERE id = ${resourceId}::uuid
    RETURNING *
  `;
  return rows[0] as ResourceRecord;
}

/**
 * Run the full inline processing pipeline: chunk → embed → store.
 * Returns the number of chunks stored.
 */
export async function processContentInline(
  db: NeonQueryFunction<false, false>,
  resourceId: string,
  markdown: string,
): Promise<number> {
  const { chunkText } = await import("./processing/chunk.js");
  const { embedTexts } = await import("./processing/embed.js");
  const { buildVersionBumpQuery, buildStoreChunksQuery } = await import(
    "./processing/store.js"
  );

  // 1. Chunk
  const chunks = chunkText(markdown);
  if (chunks.length === 0) return 0;

  // 2. Embed
  const texts = chunks.map((c) => c.content);
  const embeddings = await embedTexts(texts);

  // 3. Determine next version
  const versionRows = await db`
    SELECT COALESCE(MAX(version), 0) + 1 AS next_version
    FROM kb_chunks
    WHERE resource_id = ${resourceId}::uuid
  `;
  const nextVersion = versionRows[0].next_version;

  // 4. Mark old chunks as not current
  if (nextVersion > 1) {
    const bump = buildVersionBumpQuery(resourceId, nextVersion);
    await db.query(bump.sql, bump.params);
  }

  // 5. Build chunk rows with embeddings
  const chunkRows = chunks.map((chunk, i) => ({
    id: crypto.randomUUID(),
    resource_id: resourceId,
    chunk_index: chunk.chunk_index,
    version: nextVersion,
    header_path: chunk.header_path,
    content: chunk.content,
    content_hash: chunk.content_hash,
    embedding: `[${embeddings[i].join(",")}]`,
    is_current: true,
  }));

  // 6. Store
  const storeQuery = buildStoreChunksQuery(chunkRows);
  await db.query(storeQuery.sql, storeQuery.params);

  return chunks.length;
}
```

- [ ] **Step 2: Verify TypeScript compiles**

Run:
```bash
cd packages/temper-cloud && bun run tsc --noEmit
```

Expected: No type errors.

- [ ] **Step 3: Commit**

```bash
git add packages/temper-cloud/src/ingest.ts
git commit -m "feat: add ingest helper module for inline content processing"
```

---

## Task 3: POST /api/ingest Endpoint (TypeScript)

Create the new endpoint that accepts metadata + markdown content, creates a resource record, and processes content inline.

**Files:**
- Create: `api/ingest.ts`

- [ ] **Step 1: Create the POST /api/ingest handler**

Follow the pattern from `api/upload.ts` for auth, profile lookup, and error handling. Use dynamic imports for ESM compatibility.

```typescript
// api/ingest.ts
export const config = { runtime: "nodejs" };

export default async function handler(req: Request): Promise<Response> {
  if (req.method !== "POST") {
    return new Response(JSON.stringify({ error: "Method not allowed" }), {
      status: 405,
      headers: { "Content-Type": "application/json" },
    });
  }

  try {
    const { verifyToken, getJwksVerifier, getIssuer } = await import(
      "../packages/temper-cloud/src/auth.js"
    );
    const { getDb } = await import("../packages/temper-cloud/src/db.js");
    const {
      getProfileId,
      findByContentHash,
      insertResource,
      processContentInline,
    } = await import("../packages/temper-cloud/src/ingest.js");

    // Auth
    const authHeader = req.headers.get("authorization");
    if (!authHeader?.startsWith("Bearer ")) {
      return new Response(JSON.stringify({ error: "Missing authorization" }), {
        status: 401,
        headers: { "Content-Type": "application/json" },
      });
    }
    const claims = await verifyToken(
      authHeader.slice(7),
      getJwksVerifier(),
      getIssuer(),
    );

    const db = getDb();

    // Profile lookup
    const profileId = await getProfileId(db, claims);
    if (!profileId) {
      return new Response(JSON.stringify({ error: "Profile not found" }), {
        status: 404,
        headers: { "Content-Type": "application/json" },
      });
    }

    // Parse multipart form
    const formData = await req.formData();
    const metadataStr = formData.get("metadata");
    const content = formData.get("content");

    if (typeof metadataStr !== "string" || typeof content !== "string") {
      return new Response(
        JSON.stringify({ error: "Missing metadata or content fields" }),
        { status: 400, headers: { "Content-Type": "application/json" } },
      );
    }

    const metadata = JSON.parse(metadataStr);

    // Content hash
    const { createHash } = await import("node:crypto");
    const contentHash = createHash("sha256").update(content).digest("hex");

    // Idempotency: check for existing resource with same hash
    const existing = await findByContentHash(db, contentHash, profileId);
    if (existing) {
      return new Response(JSON.stringify(existing), {
        status: 200,
        headers: { "Content-Type": "application/json" },
      });
    }

    // Create resource record
    const resource = await insertResource(db, metadata, contentHash, profileId);

    // Inline processing: chunk → embed → store
    await processContentInline(db, resource.id, content);

    return new Response(JSON.stringify(resource), {
      status: 201,
      headers: { "Content-Type": "application/json" },
    });
  } catch (err) {
    console.error("Ingest error:", err);
    const message = err instanceof Error ? err.message : "Internal error";
    return new Response(JSON.stringify({ error: message }), {
      status: 500,
      headers: { "Content-Type": "application/json" },
    });
  }
}
```

- [ ] **Step 2: Verify TypeScript compiles**

Run:
```bash
tsc --noEmit --project tsconfig.api.json
```

Expected: No type errors.

- [ ] **Step 3: Commit**

```bash
git add api/ingest.ts
git commit -m "feat: add POST /api/ingest endpoint for inline content processing"
```

---

## Task 4: PUT /api/ingest/[id] Endpoint (TypeScript)

Create the update endpoint for the light sync path — update content for an existing resource.

**Files:**
- Create: `api/ingest/[id].ts`

- [ ] **Step 1: Create the PUT handler**

```typescript
// api/ingest/[id].ts
export const config = { runtime: "nodejs" };

export default async function handler(req: Request): Promise<Response> {
  if (req.method !== "PUT") {
    return new Response(JSON.stringify({ error: "Method not allowed" }), {
      status: 405,
      headers: { "Content-Type": "application/json" },
    });
  }

  try {
    const { verifyToken, getJwksVerifier, getIssuer } = await import(
      "../../packages/temper-cloud/src/auth.js"
    );
    const { getDb } = await import("../../packages/temper-cloud/src/db.js");
    const {
      getProfileId,
      updateResourceHash,
      processContentInline,
    } = await import("../../packages/temper-cloud/src/ingest.js");

    // Extract resource ID from URL path: /api/ingest/{id}
    const url = new URL(req.url);
    const segments = url.pathname.split("/");
    const resourceId = segments[segments.length - 1];
    if (!resourceId) {
      return new Response(JSON.stringify({ error: "Missing resource ID" }), {
        status: 400,
        headers: { "Content-Type": "application/json" },
      });
    }

    // Auth
    const authHeader = req.headers.get("authorization");
    if (!authHeader?.startsWith("Bearer ")) {
      return new Response(JSON.stringify({ error: "Missing authorization" }), {
        status: 401,
        headers: { "Content-Type": "application/json" },
      });
    }
    const claims = await verifyToken(
      authHeader.slice(7),
      getJwksVerifier(),
      getIssuer(),
    );

    const db = getDb();

    // Profile lookup
    const profileId = await getProfileId(db, claims);
    if (!profileId) {
      return new Response(JSON.stringify({ error: "Profile not found" }), {
        status: 404,
        headers: { "Content-Type": "application/json" },
      });
    }

    // Verify resource exists and caller can modify
    const accessRows = await db`
      SELECT 1 FROM resources
      WHERE id = ${resourceId}::uuid
        AND id IN (SELECT resource_id FROM can_modify_resource(${profileId}::uuid, ${resourceId}::uuid) WHERE can_modify_resource)
    `;

    // Fallback: simpler ownership check if can_modify_resource is a boolean function
    const ownerRows = await db`
      SELECT id FROM resources
      WHERE id = ${resourceId}::uuid
        AND (owner_profile_id = ${profileId}::uuid OR originator_profile_id = ${profileId}::uuid)
        AND is_active = true
    `;
    if (ownerRows.length === 0) {
      return new Response(JSON.stringify({ error: "Resource not found or not modifiable" }), {
        status: 404,
        headers: { "Content-Type": "application/json" },
      });
    }

    // Parse form data
    const formData = await req.formData();
    const content = formData.get("content");
    if (typeof content !== "string") {
      return new Response(
        JSON.stringify({ error: "Missing content field" }),
        { status: 400, headers: { "Content-Type": "application/json" } },
      );
    }

    // Content hash — skip if unchanged
    const { createHash } = await import("node:crypto");
    const contentHash = createHash("sha256").update(content).digest("hex");

    const currentRows = await db`
      SELECT content_hash FROM resources WHERE id = ${resourceId}::uuid
    `;
    if (currentRows.length > 0 && currentRows[0].content_hash === contentHash) {
      // No change — return current resource
      const unchanged = await db`SELECT * FROM resources WHERE id = ${resourceId}::uuid`;
      return new Response(JSON.stringify(unchanged[0]), {
        status: 200,
        headers: { "Content-Type": "application/json" },
      });
    }

    // Process: chunk → embed → store (version bump happens inside)
    await processContentInline(db, resourceId, content);

    // Update resource hash
    const resource = await updateResourceHash(db, resourceId, contentHash);

    return new Response(JSON.stringify(resource), {
      status: 200,
      headers: { "Content-Type": "application/json" },
    });
  } catch (err) {
    console.error("Ingest update error:", err);
    const message = err instanceof Error ? err.message : "Internal error";
    return new Response(JSON.stringify({ error: message }), {
      status: 500,
      headers: { "Content-Type": "application/json" },
    });
  }
}
```

- [ ] **Step 2: Check the `can_modify_resource` SQL function signature**

Read `migrations/` to verify how `can_modify_resource(profile_id, resource_id)` works — it returns a boolean, not a table. Adjust the access check accordingly:

```typescript
// If can_modify_resource returns boolean:
const accessRows = await db`
  SELECT can_modify_resource(${profileId}::uuid, ${resourceId}::uuid) AS allowed
`;
if (!accessRows[0]?.allowed) {
  return new Response(JSON.stringify({ error: "Not authorized to modify" }), {
    status: 403,
    headers: { "Content-Type": "application/json" },
  });
}
```

Update the handler to use whichever pattern the migration defines.

- [ ] **Step 3: Verify TypeScript compiles**

Run:
```bash
tsc --noEmit --project tsconfig.api.json
```

- [ ] **Step 4: Commit**

```bash
git add api/ingest/
git commit -m "feat: add PUT /api/ingest/:id endpoint for content updates"
```

---

## Task 5: Ingest Types (Rust — temper-core)

Add the `IngestRequest` and related types that the client and CLI will use.

**Files:**
- Create: `crates/temper-core/src/types/ingest.rs`
- Modify: `crates/temper-core/src/types/mod.rs`

- [ ] **Step 1: Write the test for IngestRequest serialization**

```rust
// crates/temper-core/src/types/ingest.rs
#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    #[test]
    fn ingest_request_serializes_with_required_fields() {
        let req = IngestRequest {
            content: "# Hello\nWorld".to_string(),
            title: "Test Doc".to_string(),
            kb_context_id: Uuid::nil(),
            kb_doc_type_id: Uuid::nil(),
            uri: "kb://temper/resource/test".to_string(),
            slug: None,
            mimetype: Some("text/markdown".to_string()),
            tags: None,
            metadata: None,
            context_name: Some("temper".to_string()),
            doc_type_name: Some("resource".to_string()),
        };
        let json = serde_json::to_string(&req).unwrap();
        assert!(json.contains("Test Doc"));
        assert!(json.contains("# Hello"));
    }

    #[test]
    fn ingest_request_serializes_with_optional_fields() {
        let req = IngestRequest {
            content: "test".to_string(),
            title: "Test".to_string(),
            kb_context_id: Uuid::nil(),
            kb_doc_type_id: Uuid::nil(),
            uri: "kb://test".to_string(),
            slug: Some("test-slug".to_string()),
            mimetype: None,
            tags: Some(vec!["tag1".to_string()]),
            metadata: Some(serde_json::json!({"device_id": "abc", "original_path": "/tmp/foo.pdf"})),
            context_name: None,
            doc_type_name: None,
        };
        let json = serde_json::to_string(&req).unwrap();
        assert!(json.contains("test-slug"));
        assert!(json.contains("tag1"));
        assert!(json.contains("device_id"));
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p temper-core ingest`
Expected: FAIL — module doesn't exist yet.

- [ ] **Step 3: Implement IngestRequest and IngestResponse**

```rust
// crates/temper-core/src/types/ingest.rs
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Request body for `POST /api/ingest` — create resource + upload content.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IngestRequest {
    /// Extracted markdown content
    pub content: String,
    pub title: String,
    pub kb_context_id: Uuid,
    pub kb_doc_type_id: Uuid,
    /// Resource URI (e.g., "kb://temper/resource/my-doc")
    pub uri: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub slug: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mimetype: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tags: Option<Vec<String>>,
    /// Provenance metadata: device_id, original_path, etc.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<serde_json::Value>,
    /// Context name — resolved to UUID server-side (alternative to kb_context_id)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context_name: Option<String>,
    /// Doc type name — resolved to UUID server-side (alternative to kb_doc_type_id)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub doc_type_name: Option<String>,
}
```

- [ ] **Step 4: Export from types/mod.rs**

Add to `crates/temper-core/src/types/mod.rs`:

```rust
pub mod ingest;
pub use ingest::IngestRequest;
```

- [ ] **Step 5: Run tests**

Run: `cargo test -p temper-core ingest`
Expected: PASS

- [ ] **Step 6: Commit**

```bash
git add crates/temper-core/src/types/ingest.rs crates/temper-core/src/types/mod.rs
git commit -m "feat: add IngestRequest type to temper-core"
```

---

## Task 6: IngestClient (Rust — temper-client)

Add the client for calling `/api/ingest` endpoints.

**Files:**
- Create: `crates/temper-client/src/ingest.rs`
- Modify: `crates/temper-client/src/lib.rs`

- [ ] **Step 1: Write failing tests for IngestClient**

```rust
// crates/temper-client/src/ingest.rs
#[cfg(test)]
mod tests {
    use super::*;
    use crate::http::HttpClient;
    use temper_core::types::IngestRequest;
    use uuid::Uuid;

    fn test_client() -> HttpClient {
        HttpClient::new("http://localhost:3000", None)
    }

    #[test]
    fn ingest_client_create_builds_multipart_form() {
        let client = IngestClient::new(&test_client());
        let req = IngestRequest {
            content: "# Test".to_string(),
            title: "Test Doc".to_string(),
            kb_context_id: Uuid::nil(),
            kb_doc_type_id: Uuid::nil(),
            uri: "kb://test".to_string(),
            slug: None,
            mimetype: None,
            tags: None,
            metadata: None,
        };
        // Verify the struct can be constructed and serialized
        let metadata_json = serde_json::to_string(&req).unwrap();
        assert!(metadata_json.contains("Test Doc"));
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p temper-client ingest`
Expected: FAIL — module doesn't exist.

- [ ] **Step 3: Implement IngestClient**

Follow the pattern from `crates/temper-client/src/upload.rs` for multipart requests:

```rust
// crates/temper-client/src/ingest.rs
use crate::auth;
use crate::http::HttpClient;
use temper_core::types::{IngestRequest, ResourceRow};

pub struct IngestClient<'a> {
    http: &'a HttpClient,
}

impl<'a> IngestClient<'a> {
    pub fn new(http: &'a HttpClient) -> Self {
        Self { http }
    }

    /// POST /api/ingest — create resource + process content inline.
    pub async fn create(&self, request: &IngestRequest) -> crate::Result<ResourceRow> {
        let token = auth::current_token()?;

        // Separate content from metadata for multipart form
        let metadata = serde_json::json!({
            "title": request.title,
            "kb_context_id": request.kb_context_id,
            "kb_doc_type_id": request.kb_doc_type_id,
            "uri": request.uri,
            "slug": request.slug,
            "mimetype": request.mimetype,
            "tags": request.tags,
            "metadata": request.metadata,
        });

        let form = reqwest::multipart::Form::new()
            .text("metadata", serde_json::to_string(&metadata)?)
            .text("content", request.content.clone());

        let req = self.http.post("/api/ingest").multipart(form);
        self.http.send_json(req, Some(&token)).await
    }

    /// PUT /api/ingest/:id — update content for existing resource.
    pub async fn update(&self, id: uuid::Uuid, content: &str) -> crate::Result<ResourceRow> {
        let token = auth::current_token()?;

        let form = reqwest::multipart::Form::new()
            .text("content", content.to_string());

        let path = format!("/api/ingest/{id}");
        let req = self.http.put(&path).multipart(form);
        self.http.send_json(req, Some(&token)).await
    }
}
```

- [ ] **Step 4: Register IngestClient on TemperClient**

Add to `crates/temper-client/src/lib.rs`:

```rust
pub mod ingest;

// In impl TemperClient:
pub fn ingest(&self) -> ingest::IngestClient<'_> {
    ingest::IngestClient::new(&self.http)
}
```

- [ ] **Step 5: Run tests**

Run: `cargo test -p temper-client ingest`
Expected: PASS

- [ ] **Step 6: Commit**

```bash
git add crates/temper-client/src/ingest.rs crates/temper-client/src/lib.rs
git commit -m "feat: add IngestClient to temper-client for /api/ingest endpoints"
```

---

## Task 7: kreuzberg Extraction Module (Rust — temper-cli)

Add the kreuzberg wrapper behind a feature flag for extracting files to markdown.

**Files:**
- Modify: `crates/temper-cli/Cargo.toml`
- Create: `crates/temper-cli/src/extract.rs`
- Modify: `crates/temper-cli/src/main.rs` (or `lib.rs`)

- [ ] **Step 1: Research kreuzberg Rust API**

Before writing code, check the kreuzberg crate's actual Rust API. Run:
```bash
cargo search kreuzberg
```

Then check docs at https://docs.rs/kreuzberg or read the crate source if needed. The API may differ from what's documented — verify the exact function signatures for:
- Extracting a file path to text
- Supported file formats
- Configuration options (presets, etc.)

Adjust the wrapper implementation below based on what you find.

- [ ] **Step 2: Add kreuzberg dependency behind feature flag**

Add to `crates/temper-cli/Cargo.toml`:

```toml
[features]
default = ["extract"]
extract = ["dep:kreuzberg"]

[dependencies]
kreuzberg = { version = "0.5", optional = true }
# Also add indicatif for progress bars (used in later tasks)
indicatif = "0.17"
ignore = "0.4"  # For .gitignore/.temperignore support
```

Note: Check the current version of kreuzberg on crates.io. The version above is a placeholder — use whatever is latest.

- [ ] **Step 3: Write the failing test**

```rust
// crates/temper-cli/src/extract.rs

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn extract_markdown_file_reads_directly() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.md");
        std::fs::write(&path, "# Hello\n\nWorld").unwrap();

        let result = extract_to_markdown(&path).unwrap();
        assert_eq!(result.content, "# Hello\n\nWorld");
        assert_eq!(result.mime_type, "text/markdown");
    }

    #[test]
    fn extract_plain_text_file_reads_directly() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.txt");
        std::fs::write(&path, "Plain text content").unwrap();

        let result = extract_to_markdown(&path).unwrap();
        assert_eq!(result.content, "Plain text content");
    }

    #[test]
    fn extract_unsupported_without_feature() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.xyz");
        std::fs::write(&path, "binary data").unwrap();

        // Without the extract feature, non-text files should error
        // With the extract feature, kreuzberg handles them
        let result = extract_to_markdown(&path);
        // Behavior depends on feature flag — test accordingly
        #[cfg(not(feature = "extract"))]
        assert!(result.is_err());
    }
}
```

- [ ] **Step 4: Run tests to verify failure**

Run: `cargo test -p temper-cli extract`
Expected: FAIL — module doesn't exist.

- [ ] **Step 5: Implement extraction module**

```rust
// crates/temper-cli/src/extract.rs
use std::path::Path;

/// Result of extracting a file to markdown.
pub struct ExtractionResult {
    pub content: String,
    pub mime_type: String,
}

/// Extract a file to markdown text.
///
/// Markdown and plain text files are read directly.
/// Other formats require the `extract` feature (kreuzberg).
pub fn extract_to_markdown(path: &Path) -> crate::error::Result<ExtractionResult> {
    let extension = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();

    match extension.as_str() {
        "md" | "markdown" => {
            let content = std::fs::read_to_string(path)
                .map_err(|e| crate::error::TemperError::Io(e.to_string()))?;
            Ok(ExtractionResult {
                content,
                mime_type: "text/markdown".to_string(),
            })
        }
        "txt" | "text" => {
            let content = std::fs::read_to_string(path)
                .map_err(|e| crate::error::TemperError::Io(e.to_string()))?;
            Ok(ExtractionResult {
                content,
                mime_type: "text/plain".to_string(),
            })
        }
        _ => extract_with_kreuzberg(path),
    }
}

#[cfg(feature = "extract")]
fn extract_with_kreuzberg(path: &Path) -> crate::error::Result<ExtractionResult> {
    // Use kreuzberg's Rust API — adjust based on actual API discovered in Step 1
    let result = kreuzberg::extract_file(path)
        .map_err(|e| crate::error::TemperError::Extraction(format!("{e}")))?;
    Ok(ExtractionResult {
        content: result.content,
        mime_type: result.mime_type,
    })
}

#[cfg(not(feature = "extract"))]
fn extract_with_kreuzberg(path: &Path) -> crate::error::Result<ExtractionResult> {
    Err(crate::error::TemperError::Extraction(format!(
        "Cannot extract '{}': non-text format requires the 'extract' feature. \
         Install with: cargo install temper-cli --features extract",
        path.display()
    )))
}
```

- [ ] **Step 6: Add Extraction error variant if not present**

Check `crates/temper-cli/src/error.rs` and add an `Extraction(String)` variant to `TemperError` if it doesn't exist. Also add an `Io(String)` variant if needed.

- [ ] **Step 7: Register module**

Add `pub mod extract;` to the appropriate place in `crates/temper-cli/src/main.rs` or `lib.rs`.

- [ ] **Step 8: Run tests**

Run: `cargo test -p temper-cli extract`
Expected: PASS

- [ ] **Step 9: Commit**

```bash
git add crates/temper-cli/Cargo.toml crates/temper-cli/src/extract.rs crates/temper-cli/src/error.rs crates/temper-cli/src/main.rs
git commit -m "feat: add kreuzberg extraction module behind feature flag"
```

---

## Task 8: Manifest I/O (Rust — temper-cli)

Implement manifest read/write logic. The types exist in temper-core but no file I/O has been implemented.

**Files:**
- Create: `crates/temper-cli/src/manifest_io.rs`

- [ ] **Step 1: Write failing tests**

```rust
// crates/temper-cli/src/manifest_io.rs

#[cfg(test)]
mod tests {
    use super::*;
    use temper_core::types::{Manifest, ManifestEntry, ManifestEntryState};
    use uuid::Uuid;

    #[test]
    fn load_manifest_returns_new_if_not_exists() {
        let dir = tempfile::tempdir().unwrap();
        let temper_dir = dir.path().join(".temper");
        let manifest = load_manifest(&temper_dir, "test-device").unwrap();
        assert_eq!(manifest.device_id, "test-device");
        assert!(manifest.entries.is_empty());
    }

    #[test]
    fn save_and_load_manifest_roundtrips() {
        let dir = tempfile::tempdir().unwrap();
        let temper_dir = dir.path().join(".temper");
        std::fs::create_dir_all(&temper_dir).unwrap();

        let mut manifest = Manifest::new("test-device".to_string());
        manifest.entries.insert(
            Uuid::nil(),
            ManifestEntry {
                path: "temper/resource/test.md".to_string(),
                content_hash: "abc123".to_string(),
                remote_hash: "abc123".to_string(),
                synced_at: chrono::Utc::now(),
                state: ManifestEntryState::Clean,
            },
        );

        save_manifest(&temper_dir, &manifest).unwrap();
        let loaded = load_manifest(&temper_dir, "test-device").unwrap();
        assert_eq!(loaded.entries.len(), 1);
        assert_eq!(loaded.entries[&Uuid::nil()].path, "temper/resource/test.md");
        assert_eq!(loaded.entries[&Uuid::nil()].state, ManifestEntryState::Clean);
    }
}
```

- [ ] **Step 2: Run tests to verify failure**

Run: `cargo test -p temper-cli manifest_io`
Expected: FAIL

- [ ] **Step 3: Implement manifest I/O**

```rust
// crates/temper-cli/src/manifest_io.rs
use std::path::Path;
use temper_core::types::Manifest;

/// Load the manifest from `<temper_dir>/manifest.json`.
/// If the file doesn't exist, returns a new empty manifest for the given device.
pub fn load_manifest(temper_dir: &Path, device_id: &str) -> crate::error::Result<Manifest> {
    let manifest_path = temper_dir.join("manifest.json");
    if !manifest_path.exists() {
        return Ok(Manifest::new(device_id.to_string()));
    }
    let content = std::fs::read_to_string(&manifest_path)
        .map_err(|e| crate::error::TemperError::Io(e.to_string()))?;
    let manifest: Manifest = serde_json::from_str(&content)
        .map_err(|e| crate::error::TemperError::Json(e))?;
    Ok(manifest)
}

/// Save the manifest to `<temper_dir>/manifest.json`.
/// Creates the directory if it doesn't exist.
pub fn save_manifest(temper_dir: &Path, manifest: &Manifest) -> crate::error::Result<()> {
    std::fs::create_dir_all(temper_dir)
        .map_err(|e| crate::error::TemperError::Io(e.to_string()))?;
    let content = serde_json::to_string_pretty(manifest)
        .map_err(|e| crate::error::TemperError::Json(e))?;
    let manifest_path = temper_dir.join("manifest.json");
    std::fs::write(&manifest_path, content)
        .map_err(|e| crate::error::TemperError::Io(e.to_string()))?;
    Ok(())
}
```

- [ ] **Step 4: Register module**

Add `pub mod manifest_io;` to the appropriate place.

- [ ] **Step 5: Run tests**

Run: `cargo test -p temper-cli manifest_io`
Expected: PASS

- [ ] **Step 6: Commit**

```bash
git add crates/temper-cli/src/manifest_io.rs
git commit -m "feat: add manifest read/write for vault sync tracking"
```

---

## Task 9: CLI Command Structure

Add the four new commands to the clap CLI and dispatch skeleton.

**Files:**
- Modify: `crates/temper-cli/src/cli.rs`
- Create: `crates/temper-cli/src/commands/add.rs`
- Create: `crates/temper-cli/src/commands/import_cmd.rs`
- Create: `crates/temper-cli/src/commands/pull.rs`
- Create: `crates/temper-cli/src/commands/remove.rs`
- Modify: `crates/temper-cli/src/commands/mod.rs`
- Modify: `crates/temper-cli/src/main.rs`

- [ ] **Step 1: Add command variants to cli.rs**

Add to the `Commands` enum in `crates/temper-cli/src/cli.rs`:

```rust
/// Add a file to the cloud (fire-and-forget, searchable, pullable)
Add {
    /// File path or directory path to add
    path: String,
    /// Add all files in a directory
    #[arg(long)]
    dir: bool,
    /// Context name (required)
    #[arg(long)]
    context: String,
    /// Doc type (default: "resource")
    #[arg(long, default_value = "resource")]
    doc_type: String,
    /// Output format
    #[arg(long, default_value = "text")]
    format: String,
    /// Override size guardrails for directory mode
    #[arg(long)]
    force: bool,
},

/// Import a file into the vault (managed, frontmatter, sync-ready)
Import {
    /// File path, directory path, or resource UUID (for promotion)
    path: String,
    /// Import all files in a directory
    #[arg(long)]
    dir: bool,
    /// Context name (required for file imports)
    #[arg(long)]
    context: Option<String>,
    /// Doc type (default: "resource")
    #[arg(long, default_value = "resource")]
    doc_type: String,
    /// Output format
    #[arg(long, default_value = "text")]
    format: String,
    /// Override size guardrails for directory mode
    #[arg(long)]
    force: bool,
},

/// Pull a resource from the cloud
Pull {
    /// Resource UUID
    resource_id: String,
},

/// Remove a resource from the cloud
Remove {
    /// Resource UUID
    resource_id: String,
    /// Skip confirmation for vault file removal
    #[arg(long)]
    force: bool,
},
```

- [ ] **Step 2: Create stub command handlers**

Create each file with a stub that prints "not yet implemented":

```rust
// crates/temper-cli/src/commands/add.rs
pub fn run(
    path: &str,
    dir: bool,
    context: &str,
    doc_type: &str,
    format: &str,
    force: bool,
) -> crate::error::Result<()> {
    // Check for URL
    if path.starts_with("http://") || path.starts_with("https://") {
        return Err(crate::error::TemperError::Config(
            "URL support not yet implemented. Please provide a file path.".to_string(),
        ));
    }
    eprintln!("temper add: not yet implemented");
    Ok(())
}
```

```rust
// crates/temper-cli/src/commands/import_cmd.rs
pub fn run(
    path: &str,
    dir: bool,
    context: Option<&str>,
    doc_type: &str,
    format: &str,
    force: bool,
) -> crate::error::Result<()> {
    eprintln!("temper import: not yet implemented");
    Ok(())
}
```

```rust
// crates/temper-cli/src/commands/pull.rs
pub fn run(resource_id: &str) -> crate::error::Result<()> {
    eprintln!("temper pull: not yet implemented");
    Ok(())
}
```

```rust
// crates/temper-cli/src/commands/remove.rs
pub fn run(resource_id: &str, force: bool) -> crate::error::Result<()> {
    eprintln!("temper remove: not yet implemented");
    Ok(())
}
```

- [ ] **Step 3: Register modules in mod.rs**

Add to `crates/temper-cli/src/commands/mod.rs`:

```rust
pub mod add;
pub mod import_cmd;
pub mod pull;
pub mod remove;
```

- [ ] **Step 4: Wire dispatch in main.rs**

Add match arms in the `run()` function:

```rust
Commands::Add { path, dir, context, doc_type, format, force } => {
    temper_cli::commands::add::run(&path, dir, &context, &doc_type, &format, force)
}
Commands::Import { path, dir, context, doc_type, format, force } => {
    temper_cli::commands::import_cmd::run(&path, dir, context.as_deref(), &doc_type, &format, force)
}
Commands::Pull { resource_id } => {
    temper_cli::commands::pull::run(&resource_id)
}
Commands::Remove { resource_id, force } => {
    temper_cli::commands::remove::run(&resource_id, force)
}
```

- [ ] **Step 5: Verify it compiles and help text works**

Run:
```bash
cargo build -p temper-cli
cargo run -p temper-cli -- add --help
cargo run -p temper-cli -- import --help
cargo run -p temper-cli -- pull --help
cargo run -p temper-cli -- remove --help
```

Expected: Help text displays correctly for all four commands.

- [ ] **Step 6: Commit**

```bash
git add crates/temper-cli/src/cli.rs crates/temper-cli/src/commands/ crates/temper-cli/src/main.rs
git commit -m "feat: add CLI command structure for add, import, pull, remove"
```

---

## Task 10: temper add — Single File

Implement the full `temper add` flow for a single file.

**Files:**
- Modify: `crates/temper-cli/src/commands/add.rs`

- [ ] **Step 1: Write the test for single file add flow**

```rust
// crates/temper-cli/src/commands/add.rs

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn url_detection_returns_error() {
        let result = run("https://example.com/doc", false, "temper", "resource", "text", false);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("URL support not yet implemented"));
    }

    #[test]
    fn nonexistent_file_returns_error() {
        let result = run("/nonexistent/file.md", false, "temper", "resource", "text", false);
        assert!(result.is_err());
    }

    #[test]
    fn content_hash_is_deterministic() {
        let hash1 = compute_content_hash("hello world");
        let hash2 = compute_content_hash("hello world");
        assert_eq!(hash1, hash2);
        assert_ne!(hash1, compute_content_hash("different content"));
    }
}
```

- [ ] **Step 2: Run tests to verify failure**

Run: `cargo test -p temper-cli add::tests`
Expected: FAIL

- [ ] **Step 3: Implement single file add**

```rust
// crates/temper-cli/src/commands/add.rs
use sha2::{Digest, Sha256};
use std::path::Path;
use uuid::Uuid;

pub fn compute_content_hash(content: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(content.as_bytes());
    hex::encode(hasher.finalize())
}

pub fn run(
    path: &str,
    dir: bool,
    context: &str,
    doc_type: &str,
    format: &str,
    force: bool,
) -> crate::error::Result<()> {
    // URL detection
    if path.starts_with("http://") || path.starts_with("https://") {
        return Err(crate::error::TemperError::Config(
            "URL support not yet implemented. Please provide a file path.".to_string(),
        ));
    }

    if dir {
        return run_directory(path, context, doc_type, format, force);
    }

    run_single_file(path, context, doc_type, format)
}

fn run_single_file(
    path: &str,
    context: &str,
    doc_type: &str,
    format: &str,
) -> crate::error::Result<()> {
    let file_path = Path::new(path);
    if !file_path.exists() {
        return Err(crate::error::TemperError::Config(
            format!("File not found: {path}"),
        ));
    }

    // 1. Extract to markdown
    let extraction = crate::extract::extract_to_markdown(file_path)?;
    let content_hash = compute_content_hash(&extraction.content);

    // 2. Resolve context and doc_type IDs
    //    For now, pass names — the ingest endpoint will need to resolve them.
    //    TODO: The API needs to accept names or the CLI needs a lookup endpoint.
    //    For initial implementation, use the names directly in URI construction.
    let title = file_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("Untitled")
        .to_string();

    let uri = format!("kb://{context}/{doc_type}/{}", title.to_lowercase().replace(' ', "-"));

    // 3. Build ingest request
    let rt = tokio::runtime::Runtime::new()
        .map_err(|e| crate::error::TemperError::Config(format!("tokio runtime: {e}")))?;

    rt.block_on(async {
        let client = temper_client::config::build_client()
            .map_err(|e| crate::error::TemperError::Config(e.to_string()))?;

        // Resolve context and doc_type IDs
        // This requires looking up kb_contexts and kb_doc_types.
        // For now, we'll need context_id and doc_type_id to be passed or resolved.
        // The practical approach: add a lookup to temper-client or accept IDs.
        //
        // Simplification for initial implementation:
        // Use the resource client to look up context/doc_type, or
        // extend the ingest endpoint to accept names instead of UUIDs.
        //
        // For the plan, we'll extend the ingest endpoint to accept names
        // and resolve server-side. This is noted in the ingest helpers task.

        let device_id = temper_client::config::load_device_id();
        let metadata = serde_json::json!({
            "device_id": device_id,
            "original_path": std::fs::canonicalize(file_path)
                .unwrap_or_else(|_| file_path.to_path_buf())
                .to_string_lossy()
                .to_string(),
        });

        let request = temper_core::types::IngestRequest {
            content: extraction.content,
            title: title.clone(),
            kb_context_id: uuid::Uuid::nil(), // Resolved server-side from context name
            kb_doc_type_id: uuid::Uuid::nil(), // Resolved server-side from doc_type name
            uri,
            slug: None,
            mimetype: Some(extraction.mime_type),
            tags: None,
            metadata: Some(metadata),
        };

        let resource = client.ingest().create(&request).await
            .map_err(|e| crate::error::TemperError::Config(e.to_string()))?;

        // 4. Output
        match format {
            "json" => {
                let json = serde_json::to_string(&resource)
                    .map_err(crate::error::TemperError::Json)?;
                println!("{json}");
            }
            _ => {
                eprintln!("  Extracting... done ({} bytes markdown)", request.content.len());
                eprintln!("  Uploading... done");
                println!("✓ Added: \"{}\" ({})", resource.title, resource.id);
            }
        }

        Ok(())
    })
}

fn run_directory(
    _path: &str,
    _context: &str,
    _doc_type: &str,
    _format: &str,
    _force: bool,
) -> crate::error::Result<()> {
    // Implemented in Task 11
    Err(crate::error::TemperError::Config(
        "Directory mode not yet implemented".to_string(),
    ))
}
```

- [ ] **Step 4: Handle context/doc_type name resolution**

The ingest endpoint currently expects UUIDs for `kb_context_id` and `kb_doc_type_id`. We need the server to accept context and doc_type by name. Update `packages/temper-cloud/src/ingest.ts` to add name resolution:

```typescript
// Add to packages/temper-cloud/src/ingest.ts

/** Resolve a context name to its UUID. Creates if not found. */
export async function resolveContextId(
  db: NeonQueryFunction<false, false>,
  name: string,
): Promise<string> {
  const rows = await db`
    SELECT id FROM kb_contexts WHERE name = ${name} LIMIT 1
  `;
  if (rows.length > 0) return rows[0].id;
  // Auto-create context
  const created = await db`
    INSERT INTO kb_contexts (name) VALUES (${name}) RETURNING id
  `;
  return created[0].id;
}

/** Resolve a doc_type name to its UUID. Returns null if not found. */
export async function resolveDocTypeId(
  db: NeonQueryFunction<false, false>,
  name: string,
): Promise<string | null> {
  const rows = await db`
    SELECT id FROM kb_doc_types WHERE name = ${name} LIMIT 1
  `;
  return rows.length > 0 ? rows[0].id : null;
}
```

Update `api/ingest.ts` to accept either UUIDs or names in the metadata:

```typescript
// In api/ingest.ts, before calling insertResource:
let contextId = metadata.kb_context_id;
let docTypeId = metadata.kb_doc_type_id;

// If context/doc_type are names (not UUIDs), resolve them
if (metadata.context_name) {
  contextId = await resolveContextId(db, metadata.context_name);
}
if (metadata.doc_type_name) {
  const resolved = await resolveDocTypeId(db, metadata.doc_type_name);
  if (!resolved) {
    return new Response(
      JSON.stringify({ error: `Unknown doc type: ${metadata.doc_type_name}` }),
      { status: 400, headers: { "Content-Type": "application/json" } },
    );
  }
  docTypeId = resolved;
}
```

Update the Rust `IngestRequest` to include name fields:

```rust
// Add to temper-core/src/types/ingest.rs IngestRequest:
/// Context name — resolved to UUID server-side
#[serde(skip_serializing_if = "Option::is_none")]
pub context_name: Option<String>,
/// Doc type name — resolved to UUID server-side
#[serde(skip_serializing_if = "Option::is_none")]
pub doc_type_name: Option<String>,
```

Update the CLI to use names instead of UUIDs:

```rust
// In add.rs, replace the UUID::nil() fields:
let request = temper_core::types::IngestRequest {
    content: extraction.content,
    title: title.clone(),
    kb_context_id: Uuid::nil(),  // Ignored when context_name is set
    kb_doc_type_id: Uuid::nil(), // Ignored when doc_type_name is set
    uri,
    slug: None,
    mimetype: Some(extraction.mime_type),
    tags: None,
    metadata: Some(metadata),
    context_name: Some(context.to_string()),
    doc_type_name: Some(doc_type.to_string()),
};
```

- [ ] **Step 5: Run tests**

Run: `cargo test -p temper-cli add`
Expected: PASS for unit tests. Integration test requires deployed API.

- [ ] **Step 6: Commit**

```bash
git add crates/temper-cli/src/commands/add.rs crates/temper-core/src/types/ingest.rs packages/temper-cloud/src/ingest.ts api/ingest.ts
git commit -m "feat: implement temper add for single file with context name resolution"
```

---

## Task 11: temper add — Directory Mode

Implement directory walking with guardrails and concurrent uploads.

**Files:**
- Modify: `crates/temper-cli/src/commands/add.rs`

- [ ] **Step 1: Write tests for directory walking**

```rust
#[cfg(test)]
mod directory_tests {
    use super::*;

    #[test]
    fn walk_directory_respects_max_depth() {
        let dir = tempfile::tempdir().unwrap();
        let sub = dir.path().join("sub");
        let deep = sub.join("deep").join("too_deep");
        std::fs::create_dir_all(&deep).unwrap();
        std::fs::write(dir.path().join("a.md"), "top level").unwrap();
        std::fs::write(sub.join("b.md"), "sub level").unwrap();
        std::fs::write(deep.join("c.md"), "too deep").unwrap();

        let config = DirectoryConfig::default();
        let files = collect_files(dir.path(), &config).unwrap();
        // max_depth 2 means top-level + 1 subdirectory
        assert!(files.iter().any(|f| f.file_name().unwrap() == "a.md"));
        assert!(files.iter().any(|f| f.file_name().unwrap() == "b.md"));
        assert!(!files.iter().any(|f| f.file_name().unwrap() == "c.md"));
    }

    #[test]
    fn walk_directory_respects_size_limit() {
        let dir = tempfile::tempdir().unwrap();
        // Create a file larger than default limit (for testing, use a small limit)
        let config = DirectoryConfig {
            max_total_bytes: 100,
            ..Default::default()
        };
        std::fs::write(dir.path().join("big.md"), "x".repeat(200)).unwrap();

        let result = preflight_check(dir.path(), &config);
        assert!(result.is_err());
    }
}
```

- [ ] **Step 2: Run tests**

Run: `cargo test -p temper-cli directory_tests`
Expected: FAIL

- [ ] **Step 3: Implement directory config and collection**

```rust
// In crates/temper-cli/src/commands/add.rs

pub struct DirectoryConfig {
    pub max_depth: usize,
    pub max_total_bytes: u64,
    pub max_concurrent: usize,
    pub allowed_extensions: Vec<String>,
}

impl Default for DirectoryConfig {
    fn default() -> Self {
        Self {
            max_depth: 2,
            max_total_bytes: 50 * 1024 * 1024, // 50MB
            max_concurrent: 4,
            allowed_extensions: vec![
                "md", "markdown", "txt", "pdf", "docx", "doc",
                "html", "htm", "rst", "org", "tex", "rtf",
            ].into_iter().map(String::from).collect(),
        }
    }
}

/// Walk a directory and collect files matching guardrails.
pub fn collect_files(
    dir: &Path,
    config: &DirectoryConfig,
) -> crate::error::Result<Vec<std::path::PathBuf>> {
    use ignore::WalkBuilder;

    let mut files = Vec::new();
    let walker = WalkBuilder::new(dir)
        .max_depth(Some(config.max_depth))
        .hidden(true)       // skip hidden files
        .git_ignore(true)   // respect .gitignore
        .add_custom_ignore_filename(".temperignore")
        .build();

    for entry in walker {
        let entry = entry.map_err(|e| crate::error::TemperError::Io(e.to_string()))?;
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let ext = path.extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_lowercase();
        if config.allowed_extensions.contains(&ext) {
            files.push(path.to_path_buf());
        }
    }

    Ok(files)
}

/// Pre-flight check: total size must be within limits.
pub fn preflight_check(
    dir: &Path,
    config: &DirectoryConfig,
) -> crate::error::Result<Vec<std::path::PathBuf>> {
    let files = collect_files(dir, config)?;
    let total_size: u64 = files.iter()
        .filter_map(|f| std::fs::metadata(f).ok())
        .map(|m| m.len())
        .sum();

    if total_size > config.max_total_bytes {
        return Err(crate::error::TemperError::Config(format!(
            "Directory total size ({:.1} MB) exceeds limit ({:.1} MB). Use --force to override.",
            total_size as f64 / 1_048_576.0,
            config.max_total_bytes as f64 / 1_048_576.0,
        )));
    }

    Ok(files)
}
```

- [ ] **Step 4: Implement directory mode with concurrent uploads**

```rust
fn run_directory(
    path: &str,
    context: &str,
    doc_type: &str,
    format: &str,
    force: bool,
) -> crate::error::Result<()> {
    let dir = Path::new(path);
    if !dir.is_dir() {
        return Err(crate::error::TemperError::Config(
            format!("Not a directory: {path}"),
        ));
    }

    let config = DirectoryConfig::default();
    let files = if force {
        collect_files(dir, &config)?
    } else {
        preflight_check(dir, &config)?
    };

    if files.is_empty() {
        println!("No matching files found in {path}");
        return Ok(());
    }

    let rt = tokio::runtime::Runtime::new()
        .map_err(|e| crate::error::TemperError::Config(format!("tokio runtime: {e}")))?;

    rt.block_on(async {
        let client = temper_client::config::build_client()
            .map_err(|e| crate::error::TemperError::Config(e.to_string()))?;

        let semaphore = std::sync::Arc::new(tokio::sync::Semaphore::new(config.max_concurrent));
        let client = std::sync::Arc::new(client);

        let mut added = 0u32;
        let mut skipped = 0u32;
        let mut failed = 0u32;
        let total = files.len();

        // Process files with concurrency limit
        let mut handles = Vec::new();
        for file_path in &files {
            let sem = semaphore.clone();
            let client = client.clone();
            let file_path = file_path.clone();
            let context = context.to_string();
            let doc_type = doc_type.to_string();

            handles.push(tokio::spawn(async move {
                let _permit = sem.acquire().await.unwrap();
                add_single_file(&client, &file_path, &context, &doc_type).await
            }));
        }

        for (i, handle) in handles.into_iter().enumerate() {
            match handle.await {
                Ok(Ok(AddResult::Created(resource))) => {
                    added += 1;
                    if format == "json" {
                        println!("{}", serde_json::json!({
                            "event": "upload",
                            "file": files[i].display().to_string(),
                            "status": "done",
                            "resource_id": resource.id.to_string(),
                        }));
                    }
                }
                Ok(Ok(AddResult::Duplicate)) => {
                    skipped += 1;
                }
                Ok(Err(e)) | Err(e) => {
                    failed += 1;
                    eprintln!("  ✗ {}: {e}", files[i].display());
                }
            }
        }

        if format == "json" {
            println!("{}", serde_json::json!({
                "event": "complete",
                "added": added,
                "skipped": skipped,
                "failed": failed,
            }));
        } else {
            println!("✓ {} added, {} skipped (duplicate), {} failed (of {} total)",
                added, skipped, failed, total);
        }

        Ok(())
    })
}
```

Note: `add_single_file` is a helper that extracts + uploads one file and returns `AddResult` (Created/Duplicate). Extract this from the `run_single_file` logic. `AddResult` and the join handle error type will need adjustment — implement based on what the compiler tells you.

- [ ] **Step 5: Run tests**

Run: `cargo test -p temper-cli directory_tests`
Expected: PASS

- [ ] **Step 6: Commit**

```bash
git add crates/temper-cli/src/commands/add.rs
git commit -m "feat: implement temper add --dir with directory walking and concurrent uploads"
```

---

## Task 12: temper import

Implement `temper import` for single files and resource ID promotion.

**Files:**
- Modify: `crates/temper-cli/src/commands/import_cmd.rs`

- [ ] **Step 1: Write tests for vault file creation**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_vault_path_uses_context_doctype_uuid() {
        let vault_root = Path::new("/vault");
        let id = uuid::Uuid::nil();
        let path = build_vault_path(vault_root, "temper", "resource", id);
        assert_eq!(path, PathBuf::from("/vault/temper/resource/00000000-0000-0000-0000-000000000000.md"));
    }

    #[test]
    fn write_frontmatter_includes_required_fields() {
        let fm = build_frontmatter(
            uuid::Uuid::nil(),
            "Test Title",
            "temper",
            "resource",
            Some("/original/path.pdf"),
        );
        assert!(fm.contains("temper-id:"));
        assert!(fm.contains("Test Title"));
        assert!(fm.contains("context: temper"));
        assert!(fm.contains("doc_type: resource"));
        assert!(fm.contains("ingestion_source:"));
    }
}
```

- [ ] **Step 2: Run tests**

Run: `cargo test -p temper-cli import_cmd`
Expected: FAIL

- [ ] **Step 3: Implement import command**

```rust
// crates/temper-cli/src/commands/import_cmd.rs
use std::path::{Path, PathBuf};
use uuid::Uuid;

pub fn build_vault_path(vault_root: &Path, context: &str, doc_type: &str, id: Uuid) -> PathBuf {
    vault_root.join(context).join(doc_type).join(format!("{id}.md"))
}

pub fn build_frontmatter(
    id: Uuid,
    title: &str,
    context: &str,
    doc_type: &str,
    ingestion_source: Option<&str>,
) -> String {
    let now = chrono::Utc::now().to_rfc3339();
    let mut fm = format!(
        "---\ntemper-id: {id}\ntitle: \"{title}\"\ncontext: {context}\ndoc_type: {doc_type}\n"
    );
    if let Some(source) = ingestion_source {
        fm.push_str(&format!("ingestion_source: \"{source}\"\n"));
    }
    fm.push_str(&format!("created: {now}\n---\n\n"));
    fm
}

pub fn run(
    path: &str,
    dir: bool,
    context: Option<&str>,
    doc_type: &str,
    format: &str,
    force: bool,
) -> crate::error::Result<()> {
    // Check if path is a UUID (promotion from added → imported)
    if let Ok(resource_id) = Uuid::parse_str(path) {
        return promote_resource(resource_id, context, doc_type, format);
    }

    // File/directory import requires context
    let context = context.ok_or_else(|| {
        crate::error::TemperError::Config("--context is required for file imports".to_string())
    })?;

    if dir {
        // Reuse directory walking from add.rs
        return run_directory_import(path, context, doc_type, format, force);
    }

    run_single_import(path, context, doc_type, format)
}

fn run_single_import(
    path: &str,
    context: &str,
    doc_type: &str,
    format: &str,
) -> crate::error::Result<()> {
    let file_path = Path::new(path);
    if !file_path.exists() {
        return Err(crate::error::TemperError::Config(format!("File not found: {path}")));
    }

    // 1. Extract
    let extraction = crate::extract::extract_to_markdown(file_path)?;

    let title = file_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("Untitled")
        .to_string();

    let original_path = std::fs::canonicalize(file_path)
        .unwrap_or_else(|_| file_path.to_path_buf())
        .to_string_lossy()
        .to_string();

    let rt = tokio::runtime::Runtime::new()
        .map_err(|e| crate::error::TemperError::Config(format!("tokio runtime: {e}")))?;

    rt.block_on(async {
        let client = temper_client::config::build_client()
            .map_err(|e| crate::error::TemperError::Config(e.to_string()))?;

        // 2. Upload via ingest
        let device_id = temper_client::config::load_device_id();
        let request = temper_core::types::IngestRequest {
            content: extraction.content.clone(),
            title: title.clone(),
            kb_context_id: Uuid::nil(),
            kb_doc_type_id: Uuid::nil(),
            uri: format!("kb://{context}/{doc_type}/{}", title.to_lowercase().replace(' ', "-")),
            slug: None,
            mimetype: Some(extraction.mime_type),
            tags: None,
            metadata: Some(serde_json::json!({
                "device_id": device_id,
                "original_path": original_path,
            })),
            context_name: Some(context.to_string()),
            doc_type_name: Some(doc_type.to_string()),
        };

        let resource = client.ingest().create(&request).await
            .map_err(|e| crate::error::TemperError::Config(e.to_string()))?;

        // 3. Write vault file with frontmatter
        let config = crate::config::Config::load(None)?;
        let vault_path = build_vault_path(&config.vault_root, context, doc_type, resource.id);
        std::fs::create_dir_all(vault_path.parent().unwrap())
            .map_err(|e| crate::error::TemperError::Io(e.to_string()))?;

        let frontmatter = build_frontmatter(
            resource.id,
            &resource.title,
            context,
            doc_type,
            Some(&original_path),
        );
        let vault_content = format!("{frontmatter}{}", extraction.content);
        std::fs::write(&vault_path, &vault_content)
            .map_err(|e| crate::error::TemperError::Io(e.to_string()))?;

        // 4. Register in manifest
        let temper_dir = config.vault_root.join(".temper");
        let device_id_str = device_id.unwrap_or_else(|| "unknown".to_string());
        let mut manifest = crate::manifest_io::load_manifest(&temper_dir, &device_id_str)?;
        let content_hash = crate::commands::add::compute_content_hash(&vault_content);
        manifest.entries.insert(
            resource.id,
            temper_core::types::ManifestEntry {
                path: vault_path.strip_prefix(&config.vault_root)
                    .unwrap_or(&vault_path)
                    .to_string_lossy()
                    .to_string(),
                content_hash: content_hash.clone(),
                remote_hash: resource.content_hash.unwrap_or_default(),
                synced_at: chrono::Utc::now(),
                state: temper_core::types::ManifestEntryState::Clean,
            },
        );
        crate::manifest_io::save_manifest(&temper_dir, &manifest)?;

        // 5. Output
        match format {
            "json" => {
                println!("{}", serde_json::to_string(&resource)
                    .map_err(crate::error::TemperError::Json)?);
            }
            _ => {
                println!("✓ Imported: \"{}\" ({})", resource.title, resource.id);
                println!("  Vault: {}", vault_path.display());
            }
        }

        Ok(())
    })
}

fn promote_resource(
    resource_id: Uuid,
    context: Option<&str>,
    doc_type: &str,
    format: &str,
) -> crate::error::Result<()> {
    let rt = tokio::runtime::Runtime::new()
        .map_err(|e| crate::error::TemperError::Config(format!("tokio runtime: {e}")))?;

    rt.block_on(async {
        let client = temper_client::config::build_client()
            .map_err(|e| crate::error::TemperError::Config(e.to_string()))?;

        // 1. Fetch resource metadata
        let resource = client.resources().get(resource_id).await
            .map_err(|e| crate::error::TemperError::Config(e.to_string()))?;

        // 2. Fetch content
        let content_resp = client.resources().content(resource_id).await
            .map_err(|e| crate::error::TemperError::Config(e.to_string()))?;

        // 3. Determine context — use provided or derive from resource
        let ctx = context.unwrap_or("default");

        // 4. Write vault file
        let config = crate::config::Config::load(None)?;
        let vault_path = build_vault_path(&config.vault_root, ctx, doc_type, resource.id);
        std::fs::create_dir_all(vault_path.parent().unwrap())
            .map_err(|e| crate::error::TemperError::Io(e.to_string()))?;

        let frontmatter = build_frontmatter(
            resource.id,
            &resource.title,
            ctx,
            doc_type,
            None,
        );
        let vault_content = format!("{frontmatter}{}", content_resp.markdown);
        std::fs::write(&vault_path, &vault_content)
            .map_err(|e| crate::error::TemperError::Io(e.to_string()))?;

        // 5. Register in manifest
        let temper_dir = config.vault_root.join(".temper");
        let device_id = temper_client::config::load_device_id()
            .unwrap_or_else(|| "unknown".to_string());
        let mut manifest = crate::manifest_io::load_manifest(&temper_dir, &device_id)?;
        let content_hash = crate::commands::add::compute_content_hash(&vault_content);
        manifest.entries.insert(
            resource.id,
            temper_core::types::ManifestEntry {
                path: vault_path.strip_prefix(&config.vault_root)
                    .unwrap_or(&vault_path)
                    .to_string_lossy()
                    .to_string(),
                content_hash,
                remote_hash: resource.content_hash.unwrap_or_default(),
                synced_at: chrono::Utc::now(),
                state: temper_core::types::ManifestEntryState::Clean,
            },
        );
        crate::manifest_io::save_manifest(&temper_dir, &manifest)?;

        match format {
            "json" => {
                println!("{}", serde_json::to_string(&resource)
                    .map_err(crate::error::TemperError::Json)?);
            }
            _ => {
                println!("✓ Imported (promoted): \"{}\" ({})", resource.title, resource.id);
                println!("  Vault: {}", vault_path.display());
            }
        }

        Ok(())
    })
}

fn run_directory_import(
    path: &str,
    context: &str,
    doc_type: &str,
    format: &str,
    force: bool,
) -> crate::error::Result<()> {
    // Reuse directory walking from add command, but call import flow per file
    // Implementation follows same pattern as add::run_directory but calls
    // run_single_import for each file instead of run_single_file
    let dir = Path::new(path);
    if !dir.is_dir() {
        return Err(crate::error::TemperError::Config(format!("Not a directory: {path}")));
    }

    let config = crate::commands::add::DirectoryConfig::default();
    let files = if force {
        crate::commands::add::collect_files(dir, &config)?
    } else {
        crate::commands::add::preflight_check(dir, &config)?
    };

    for file in &files {
        let file_str = file.to_string_lossy().to_string();
        if let Err(e) = run_single_import(&file_str, context, doc_type, format) {
            eprintln!("  ✗ {}: {e}", file.display());
        }
    }

    Ok(())
}
```

- [ ] **Step 4: Run tests**

Run: `cargo test -p temper-cli import_cmd`
Expected: PASS for unit tests

- [ ] **Step 5: Commit**

```bash
git add crates/temper-cli/src/commands/import_cmd.rs
git commit -m "feat: implement temper import with vault file creation and promotion"
```

---

## Task 13: temper pull

Implement `temper pull` for downloading resources.

**Files:**
- Modify: `crates/temper-cli/src/commands/pull.rs`

- [ ] **Step 1: Implement pull command**

```rust
// crates/temper-cli/src/commands/pull.rs
use std::path::Path;
use uuid::Uuid;

pub fn run(resource_id: &str) -> crate::error::Result<()> {
    let id = Uuid::parse_str(resource_id)
        .map_err(|e| crate::error::TemperError::Config(format!("Invalid UUID: {e}")))?;

    let rt = tokio::runtime::Runtime::new()
        .map_err(|e| crate::error::TemperError::Config(format!("tokio runtime: {e}")))?;

    rt.block_on(async {
        let client = temper_client::config::build_client()
            .map_err(|e| crate::error::TemperError::Config(e.to_string()))?;

        // Fetch resource metadata
        let resource = client.resources().get(id).await
            .map_err(|e| crate::error::TemperError::Config(e.to_string()))?;

        // Fetch content
        let content = client.resources().content(id).await
            .map_err(|e| crate::error::TemperError::Config(e.to_string()))?;

        // Check if resource is in manifest (imported)
        let config = crate::config::Config::load(None)?;
        let temper_dir = config.vault_root.join(".temper");
        let device_id = temper_client::config::load_device_id()
            .unwrap_or_else(|| "unknown".to_string());
        let mut manifest = crate::manifest_io::load_manifest(&temper_dir, &device_id)?;

        if let Some(entry) = manifest.entries.get_mut(&id) {
            // Imported resource — write to vault path
            let vault_path = config.vault_root.join(&entry.path);
            std::fs::create_dir_all(vault_path.parent().unwrap())
                .map_err(|e| crate::error::TemperError::Io(e.to_string()))?;

            // Rebuild with frontmatter
            // Context and doc_type names are stored in the manifest entry path:
            // "{context}/{doc_type}/{uuid}.md"
            let parts: Vec<&str> = entry.path.split('/').collect();
            let ctx = parts.first().unwrap_or(&"default");
            let dtype = if parts.len() > 1 { parts[1] } else { "resource" };

            let frontmatter = crate::commands::import_cmd::build_frontmatter(
                id,
                &resource.title,
                ctx,
                dtype,
                None,
            );
            let full_content = format!("{frontmatter}{}", content.markdown);
            std::fs::write(&vault_path, &full_content)
                .map_err(|e| crate::error::TemperError::Io(e.to_string()))?;

            // Update manifest
            let content_hash = crate::commands::add::compute_content_hash(&full_content);
            entry.content_hash = content_hash;
            entry.remote_hash = resource.content_hash.unwrap_or_default();
            entry.synced_at = chrono::Utc::now();
            entry.state = temper_core::types::ManifestEntryState::Clean;
            crate::manifest_io::save_manifest(&temper_dir, &manifest)?;

            println!("✓ Pulled: \"{}\" → {}", resource.title, vault_path.display());
        } else {
            // Added resource — write as snapshot to CWD
            let filename = format!("{id}.md");
            std::fs::write(&filename, &content.markdown)
                .map_err(|e| crate::error::TemperError::Io(e.to_string()))?;

            println!("✓ Pulled: \"{}\" → {filename}", resource.title);
        }

        Ok(())
    })
}
```

- [ ] **Step 2: Run compilation check**

Run: `cargo build -p temper-cli`
Expected: Compiles successfully.

- [ ] **Step 3: Commit**

```bash
git add crates/temper-cli/src/commands/pull.rs
git commit -m "feat: implement temper pull for downloading resources"
```

---

## Task 14: temper remove

Implement `temper remove` with confirmation for vault cleanup.

**Files:**
- Modify: `crates/temper-cli/src/commands/remove.rs`

- [ ] **Step 1: Implement remove command**

```rust
// crates/temper-cli/src/commands/remove.rs
use std::io::{self, Write};
use uuid::Uuid;

pub fn run(resource_id: &str, force: bool) -> crate::error::Result<()> {
    let id = Uuid::parse_str(resource_id)
        .map_err(|e| crate::error::TemperError::Config(format!("Invalid UUID: {e}")))?;

    let rt = tokio::runtime::Runtime::new()
        .map_err(|e| crate::error::TemperError::Config(format!("tokio runtime: {e}")))?;

    rt.block_on(async {
        let client = temper_client::config::build_client()
            .map_err(|e| crate::error::TemperError::Config(e.to_string()))?;

        // Delete from cloud
        client.resources().delete(id).await
            .map_err(|e| crate::error::TemperError::Config(e.to_string()))?;

        println!("✓ Deleted from cloud: {id}");

        // Check if in manifest
        let config = crate::config::Config::load(None)?;
        let temper_dir = config.vault_root.join(".temper");
        let device_id = temper_client::config::load_device_id()
            .unwrap_or_else(|| "unknown".to_string());
        let mut manifest = crate::manifest_io::load_manifest(&temper_dir, &device_id)?;

        if let Some(entry) = manifest.entries.get(&id) {
            let vault_path = config.vault_root.join(&entry.path);

            let should_remove = if force {
                true
            } else {
                eprint!("Also remove vault file at {}? [y/N] ", vault_path.display());
                io::stderr().flush().ok();
                let mut input = String::new();
                io::stdin().read_line(&mut input).ok();
                input.trim().eq_ignore_ascii_case("y")
            };

            if should_remove {
                if vault_path.exists() {
                    std::fs::remove_file(&vault_path)
                        .map_err(|e| crate::error::TemperError::Io(e.to_string()))?;
                    println!("  Removed vault file: {}", vault_path.display());
                }
                manifest.entries.remove(&id);
                crate::manifest_io::save_manifest(&temper_dir, &manifest)?;
            }
        }

        Ok(())
    })
}
```

- [ ] **Step 2: Run compilation check**

Run: `cargo build -p temper-cli`
Expected: Compiles successfully.

- [ ] **Step 3: Commit**

```bash
git add crates/temper-cli/src/commands/remove.rs
git commit -m "feat: implement temper remove with vault cleanup confirmation"
```

---

## Task 15: CLI Progress Output

Add `indicatif` progress bars for TTY mode and structured JSONL for non-TTY.

**Files:**
- Modify: `crates/temper-cli/src/commands/add.rs` (directory mode)

- [ ] **Step 1: Add progress bar to directory add**

Update `run_directory` in `add.rs` to use `indicatif`:

```rust
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};

// In run_directory, before processing files:
let is_tty = atty::is(atty::Stream::Stderr);
let use_progress = is_tty && format != "json";

let multi = MultiProgress::new();
let overall = if use_progress {
    let pb = multi.add(ProgressBar::new(files.len() as u64));
    pb.set_style(
        ProgressStyle::default_bar()
            .template("  [{bar:40.cyan/blue}] {pos}/{len}  {msg}")
            .unwrap()
            .progress_chars("█░░"),
    );
    Some(pb)
} else {
    None
};

// After each file completes:
if let Some(pb) = &overall {
    pb.set_message(format!("{}", file_path.display()));
    pb.inc(1);
}

// At the end:
if let Some(pb) = &overall {
    pb.finish_with_message("done");
}
```

Note: Add `atty = "0.2"` to `crates/temper-cli/Cargo.toml` dependencies for TTY detection. Alternatively, check `std::io::IsTerminal` if on Rust 1.70+.

- [ ] **Step 2: Verify progress bar renders**

Create a test directory with a few markdown files and run:
```bash
cargo run -p temper-cli -- add --dir --context test --path ./test-dir/
```

Expected: Progress bar renders in terminal. With `--format json`, JSONL output instead.

- [ ] **Step 3: Commit**

```bash
git add crates/temper-cli/src/commands/add.rs crates/temper-cli/Cargo.toml
git commit -m "feat: add indicatif progress bars for directory operations"
```

---

## Task 16: End-to-End Verification

Run the full flow against the deployed API.

- [ ] **Step 1: Build and test locally**

```bash
cargo make check
cargo test --all
```

Expected: All tests pass, no clippy warnings.

- [ ] **Step 2: TypeScript verification**

```bash
tsc --noEmit && tsc --noEmit --project tsconfig.api.json
biome check
```

Expected: No errors.

- [ ] **Step 3: Deploy to preview**

Push branch and verify Vercel preview deployment succeeds.

- [ ] **Step 4: Test temper add end-to-end**

```bash
temper auth login   # If needed
temper add test-file.md --context temper
```

Expected: Resource created, ID printed.

- [ ] **Step 5: Test temper import end-to-end**

```bash
temper import test-file.pdf --context temper
```

Expected: Resource created, vault file written with frontmatter, manifest updated.

- [ ] **Step 6: Test temper pull end-to-end**

```bash
temper pull <resource-id-from-above>
```

Expected: Content downloaded.

- [ ] **Step 7: Test temper remove end-to-end**

```bash
temper remove <resource-id> --force
```

Expected: Resource deleted from cloud and vault.

- [ ] **Step 8: Test directory add**

```bash
temper add --dir --context temper --path ./test-dir/
```

Expected: Progress bar, all files uploaded, summary printed.

- [ ] **Step 9: Final commit and cleanup**

Fix any issues discovered during E2E testing. Commit fixes.

```bash
git add -A
git commit -m "fix: address issues from end-to-end testing"
```

---

## Deferred (Document for Future)

- **URL support** (`temper add <url>`): Stubbed — stretch goal for end of I5c
- **Zip-batch upload**: Sequential + concurrency sufficient for I5c scope
- **temper-embed**: No local embeddings — server handles all embedding
- **Full sync protocol** (I6): I5c writes manifest entries only
- **Wiring workflow docs** (tasks/goals/sessions) to `PUT /api/ingest/:id`: Follow-up after I5c
