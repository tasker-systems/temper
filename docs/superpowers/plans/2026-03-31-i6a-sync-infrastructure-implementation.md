# I6a: Sync Infrastructure & Core Protocol — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Implement the core bidirectional sync protocol: server-side API endpoints, Rust client extensions, sync actions layer, and `temper sync` CLI command.

**Architecture:** Server computes diff via `sync_diff_for_device()` SQL function (already deployed). Business logic lives in `packages/temper-cloud/src/` as individually testable functions. API handlers are thin orchestration layers. Rust client wraps API. CLI orchestrates the 10-step sync flow.

**Tech Stack:** TypeScript (Vercel serverless functions, neon serverless, zod), Rust (reqwest, serde, sha2, clap, tokio)

---

## File Map

### TypeScript — packages/temper-cloud
| Action | File | Responsibility |
|--------|------|----------------|
| Create | `packages/temper-cloud/src/middleware.ts` | Shared auth: extract bearer, verify JWT, resolve profileId |
| Create | `packages/temper-cloud/src/sync.ts` | Sync business logic + zod schemas |
| Create | `packages/temper-cloud/tests/middleware.test.ts` | Unit tests for auth middleware |
| Create | `packages/temper-cloud/tests/sync.test.ts` | Unit tests for sync logic + schema validation |

### TypeScript — API handlers (thin)
| Action | File | Responsibility |
|--------|------|----------------|
| Fix | `api/upload.ts:59-62` | Auth bug — use getProfileId from ingest.ts |
| Create | `api/sync/status.ts` | Thin handler: authenticate → validate → computeSyncDiff → response |
| Create | `api/sync/complete.ts` | Thin handler: authenticate → validate → completeSyncRound → response |

### Rust — temper-core
| Action | File | Responsibility |
|--------|------|----------------|
| Modify | `crates/temper-core/src/types/sync.rs` | Update types to match SQL function interface |
| Modify | `crates/temper-core/src/types/mod.rs` | Update re-exports |

### Rust — temper-client
| Action | File | Responsibility |
|--------|------|----------------|
| Create | `crates/temper-client/src/sync.rs` | SyncClient sub-client (status, complete) |
| Modify | `crates/temper-client/src/lib.rs` | Add `pub mod sync` and accessor |

### Rust — temper-cli
| Action | File | Responsibility |
|--------|------|----------------|
| Create | `crates/temper-cli/src/actions/runtime.rs` | Shared `with_client()` abstraction |
| Create | `crates/temper-cli/src/actions/sync.rs` | Sync business logic (rehash, push, pull, remove) |
| Modify | `crates/temper-cli/src/actions/mod.rs` | Register new modules |
| Modify | `crates/temper-cli/src/commands/import_cmd.rs` | Refactor to use `with_client()` |
| Modify | `crates/temper-cli/src/commands/pull.rs` | Refactor to use `with_client()` |
| Modify | `crates/temper-cli/src/commands/remove.rs` | Refactor to use `with_client()` |
| Create | `crates/temper-cli/src/commands/sync_cmd.rs` | CLI sync + sync status subcommands |
| Modify | `crates/temper-cli/src/cli.rs` | Add Sync variant to Commands enum |
| Modify | `crates/temper-cli/src/main.rs` | Wire sync command dispatch |

---

## Task 1: Fix api/upload.ts Auth Bug

**Files:**
- Fix: `api/upload.ts:59-69`

- [ ] **Step 1: Fix the auth query**

Replace lines 59-69 in `api/upload.ts`. The broken query hits a non-existent `auth_provider_sub` column. Replace with `getProfileId()` from ingest.ts:

Add to the dynamic imports block (near line 14-18):
```typescript
  const { getProfileId } = await import("../packages/temper-cloud/src/ingest.js");
```

Replace lines 59-69 with:
```typescript
  const profileId = await getProfileId(db, claims);
  if (!profileId) {
    return new Response(
      JSON.stringify({ error: "Profile not found" }),
      { status: 404, headers: { "Content-Type": "application/json" } }
    );
  }
```

- [ ] **Step 2: Verify TypeScript compiles**

Run: `npx tsc --noEmit`

- [ ] **Step 3: Commit**

```bash
git add api/upload.ts
git commit -m "fix: use kb_profile_auth_links join in upload.ts auth lookup"
```

---

## Task 2: Update Sync Types to Match SQL Function

**Files:**
- Modify: `crates/temper-core/src/types/sync.rs`
- Modify: `crates/temper-core/src/types/mod.rs`

The existing sync types were written before `sync_diff_for_device()` was finalized. They use `resource_id`-based manifest entries but the SQL function expects URI-based entries (`{uri, local_hash, remote_hash}`). Since no production code uses these types yet, replace in place.

- [ ] **Step 1: Replace sync.rs**

Replace `crates/temper-core/src/types/sync.rs` with types aligned to the SQL function:

```rust
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

// ---------------------------------------------------------------------------
// Status endpoint (POST /api/sync/status)
// ---------------------------------------------------------------------------

/// A single manifest entry sent to the server for diff computation.
/// Maps to the JSONB entries consumed by `sync_diff_for_device()`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncManifestEntry {
    pub uri: String,
    pub local_hash: String,
    pub remote_hash: String,
}

/// Per-context grouping of manifest entries.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncContextEntries {
    pub name: String,
    pub entries: Vec<SyncManifestEntry>,
}

/// Request body for `POST /api/sync/status`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncStatusRequest {
    pub contexts: Vec<SyncContextEntries>,
}

/// A resource the client should push (local-only or locally modified).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncPushItem {
    pub uri: String,
    pub resource_id: Option<Uuid>,
}

/// A resource the client should pull (server has newer or new resource).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncPullItem {
    pub uri: String,
    pub resource_id: Uuid,
    pub content_hash: String,
}

/// A resource with conflicting changes on both sides.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncConflictItem {
    pub uri: String,
    pub resource_id: Uuid,
    pub server_hash: String,
}

/// A resource that was removed from visibility.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncRemovedItem {
    pub uri: String,
    pub resource_id: Uuid,
}

/// Response body for `POST /api/sync/status`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncStatusResponse {
    pub to_push: Vec<SyncPushItem>,
    pub to_pull: Vec<SyncPullItem>,
    pub conflicts: Vec<SyncConflictItem>,
    pub removed: Vec<SyncRemovedItem>,
}

// ---------------------------------------------------------------------------
// Complete endpoint (POST /api/sync/complete)
// ---------------------------------------------------------------------------

/// A resource whose content_hash should be updated after sync.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MergedResource {
    pub resource_id: Uuid,
    pub content_hash: String,
}

/// Request body for `POST /api/sync/complete`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncCompleteRequest {
    pub client_id: String,
    pub merged_resources: Vec<MergedResource>,
}

/// Response body for `POST /api/sync/complete`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncCompleteResponse {
    pub last_sync_at: DateTime<Utc>,
    pub updated_count: u32,
}

// ---------------------------------------------------------------------------
// Resolve endpoint (I6c — placeholder types)
// ---------------------------------------------------------------------------

/// Conflict resolution type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResolutionType {
    Local,
    Remote,
    Merged,
}

/// Request body for `POST /api/sync/resolve` (I6c).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncResolveRequest {
    pub resource_id: Uuid,
    pub resolution: ResolutionType,
    pub content_hash: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sync_status_request_serde_roundtrip() {
        let req = SyncStatusRequest {
            contexts: vec![SyncContextEntries {
                name: "temper".to_string(),
                entries: vec![SyncManifestEntry {
                    uri: "kb://temper/task/00000000-0000-0000-0000-000000000000".to_string(),
                    local_hash: "sha256:abc".to_string(),
                    remote_hash: "sha256:abc".to_string(),
                }],
            }],
        };
        let json = serde_json::to_string(&req).unwrap();
        let parsed: SyncStatusRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.contexts.len(), 1);
        assert_eq!(parsed.contexts[0].entries.len(), 1);
        assert_eq!(parsed.contexts[0].entries[0].uri, "kb://temper/task/00000000-0000-0000-0000-000000000000");
    }

    #[test]
    fn sync_status_response_empty_roundtrip() {
        let resp = SyncStatusResponse {
            to_push: vec![],
            to_pull: vec![],
            conflicts: vec![],
            removed: vec![],
        };
        let json = serde_json::to_string(&resp).unwrap();
        let parsed: SyncStatusResponse = serde_json::from_str(&json).unwrap();
        assert!(parsed.to_pull.is_empty());
        assert!(parsed.conflicts.is_empty());
    }

    #[test]
    fn sync_complete_request_serde_roundtrip() {
        let req = SyncCompleteRequest {
            client_id: "device-abc".to_string(),
            merged_resources: vec![MergedResource {
                resource_id: Uuid::nil(),
                content_hash: "sha256:def".to_string(),
            }],
        };
        let json = serde_json::to_string(&req).unwrap();
        let parsed: SyncCompleteRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.client_id, "device-abc");
        assert_eq!(parsed.merged_resources.len(), 1);
    }

    #[test]
    fn sync_complete_response_serde_roundtrip() {
        let resp = SyncCompleteResponse {
            last_sync_at: Utc::now(),
            updated_count: 3,
        };
        let json = serde_json::to_string(&resp).unwrap();
        let parsed: SyncCompleteResponse = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.updated_count, 3);
    }

    #[test]
    fn resolution_type_serde() {
        assert_eq!(serde_json::to_string(&ResolutionType::Local).unwrap(), "\"local\"");
        assert_eq!(serde_json::to_string(&ResolutionType::Merged).unwrap(), "\"merged\"");
    }

    #[test]
    fn push_item_with_null_resource_id() {
        let item = SyncPushItem {
            uri: "kb://temper/note/new-uuid".to_string(),
            resource_id: None,
        };
        let json = serde_json::to_string(&item).unwrap();
        assert!(json.contains("null"));
        let parsed: SyncPushItem = serde_json::from_str(&json).unwrap();
        assert!(parsed.resource_id.is_none());
    }
}
```

- [ ] **Step 2: Update mod.rs re-exports**

In `crates/temper-core/src/types/mod.rs`, replace the `pub use sync::` block:

```rust
pub use sync::{
    MergedResource, ResolutionType, SyncCompleteRequest, SyncCompleteResponse,
    SyncConflictItem, SyncContextEntries, SyncManifestEntry, SyncPullItem,
    SyncPushItem, SyncRemovedItem, SyncResolveRequest, SyncStatusRequest,
    SyncStatusResponse,
};
```

- [ ] **Step 3: Verify**

Run: `cargo check -p temper-core --all-features && cargo test -p temper-core --all-features`

- [ ] **Step 4: Commit**

```bash
git add crates/temper-core/src/types/sync.rs crates/temper-core/src/types/mod.rs
git commit -m "refactor: update sync types to match sync_diff_for_device SQL interface"
```

---

## Task 3: Create Shared Auth Middleware

**Files:**
- Create: `packages/temper-cloud/src/middleware.ts`
- Create: `packages/temper-cloud/tests/middleware.test.ts`

The auth pattern (extract bearer → verify JWT → resolve profile) is duplicated in every handler. Extract it once.

- [ ] **Step 1: Write failing test for authenticateRequest**

Create `packages/temper-cloud/tests/middleware.test.ts`:

```typescript
import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import * as jose from "jose";
import { beforeAll, describe, expect, it } from "vitest";
import { authenticateRequest } from "../src/middleware.js";

const privateKeyPem = readFileSync(
  resolve(__dirname, "../../../crates/temper-api/tests/common/test_ed25519.key"),
  "utf-8",
);

let privateKey: jose.KeyLike;

beforeAll(async () => {
  privateKey = await jose.importPKCS8(privateKeyPem, "EdDSA");
});

async function signTestJwt(claims: Record<string, unknown>): Promise<string> {
  return new jose.SignJWT(claims as jose.JWTPayload)
    .setProtectedHeader({ alg: "EdDSA" })
    .setIssuedAt()
    .setExpirationTime("1h")
    .setIssuer("test-issuer")
    .sign(privateKey);
}

describe("authenticateRequest", () => {
  it("rejects request without Authorization header", async () => {
    const req = new Request("https://example.com/api/test", { method: "POST" });
    const result = await authenticateRequest(req);
    expect(result.ok).toBe(false);
    if (!result.ok) {
      expect(result.response.status).toBe(401);
    }
  });

  it("rejects request with malformed Authorization header", async () => {
    const req = new Request("https://example.com/api/test", {
      method: "POST",
      headers: { Authorization: "Basic abc" },
    });
    const result = await authenticateRequest(req);
    expect(result.ok).toBe(false);
    if (!result.ok) {
      expect(result.response.status).toBe(401);
    }
  });

  it("rejects request with invalid JWT", async () => {
    const req = new Request("https://example.com/api/test", {
      method: "POST",
      headers: { Authorization: "Bearer not-a-jwt" },
    });
    const result = await authenticateRequest(req);
    expect(result.ok).toBe(false);
    if (!result.ok) {
      expect(result.response.status).toBe(401);
    }
  });
});
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd packages/temper-cloud && npx vitest run tests/middleware.test.ts`
Expected: FAIL — `authenticateRequest` not found

- [ ] **Step 3: Implement middleware.ts**

Create `packages/temper-cloud/src/middleware.ts`:

```typescript
import type { AuthClaims } from "./auth.js";
import { verifyToken, getJwksVerifier, getIssuer } from "./auth.js";
import { getDb, type NeonClient } from "./db.js";
import { getProfileId } from "./ingest.js";

// ---------------------------------------------------------------------------
// Result types
// ---------------------------------------------------------------------------

export interface AuthSuccess {
  ok: true;
  db: NeonClient;
  profileId: string;
  claims: AuthClaims;
}

export interface AuthFailure {
  ok: false;
  response: Response;
}

export type AuthResult = AuthSuccess | AuthFailure;

// ---------------------------------------------------------------------------
// Shared auth middleware
// ---------------------------------------------------------------------------

function jsonResponse(status: number, body: unknown): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { "Content-Type": "application/json" },
  });
}

/**
 * Authenticate an incoming request: extract bearer token, verify JWT,
 * resolve profile ID from auth claims.
 *
 * Returns `{ ok: true, db, profileId, claims }` on success, or
 * `{ ok: false, response }` with a ready-to-return error Response on failure.
 */
export async function authenticateRequest(req: Request): Promise<AuthResult> {
  const authHeader = req.headers.get("authorization");
  if (!authHeader?.startsWith("Bearer ")) {
    return {
      ok: false,
      response: jsonResponse(401, {
        error: { code: "UNAUTHORIZED", message: "Missing Authorization header" },
      }),
    };
  }

  let claims: AuthClaims;
  try {
    claims = await verifyToken(authHeader.slice(7), getJwksVerifier(), getIssuer());
  } catch {
    return {
      ok: false,
      response: jsonResponse(401, {
        error: { code: "UNAUTHORIZED", message: "Invalid token" },
      }),
    };
  }

  const db = getDb();
  const profileId = await getProfileId(db, claims);
  if (!profileId) {
    return {
      ok: false,
      response: jsonResponse(404, { error: "Profile not found" }),
    };
  }

  return { ok: true, db, profileId, claims };
}

/**
 * Validate that the request method matches. Returns an error Response or null.
 */
export function requireMethod(req: Request, method: string): Response | null {
  if (req.method !== method) {
    return jsonResponse(405, { error: "Method not allowed" });
  }
  return null;
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cd packages/temper-cloud && npx vitest run tests/middleware.test.ts`

- [ ] **Step 5: Commit**

```bash
git add packages/temper-cloud/src/middleware.ts packages/temper-cloud/tests/middleware.test.ts
git commit -m "feat: add shared authenticateRequest middleware to temper-cloud"
```

---

## Task 4: Create Sync Business Logic Module

**Files:**
- Modify: `packages/temper-cloud/package.json` (add zod)
- Create: `packages/temper-cloud/src/sync.ts`
- Create: `packages/temper-cloud/tests/sync.test.ts`

- [ ] **Step 1: Add zod dependency**

Run: `cd packages/temper-cloud && npm install zod`

- [ ] **Step 2: Write failing tests for sync module**

Create `packages/temper-cloud/tests/sync.test.ts`:

```typescript
import { describe, expect, it } from "vitest";
import {
  SyncStatusBodySchema,
  SyncCompleteBodySchema,
  categorizeDiffRows,
} from "../src/sync.js";

describe("SyncStatusBodySchema", () => {
  it("accepts valid request body", () => {
    const body = {
      contexts: [
        {
          name: "temper",
          entries: [
            { uri: "kb://temper/task/abc", local_hash: "sha256:aaa", remote_hash: "sha256:bbb" },
          ],
        },
      ],
    };
    const result = SyncStatusBodySchema.safeParse(body);
    expect(result.success).toBe(true);
  });

  it("rejects missing contexts", () => {
    const result = SyncStatusBodySchema.safeParse({});
    expect(result.success).toBe(false);
  });

  it("rejects empty context name", () => {
    const body = {
      contexts: [{ name: "", entries: [] }],
    };
    const result = SyncStatusBodySchema.safeParse(body);
    expect(result.success).toBe(false);
  });

  it("rejects entries with missing fields", () => {
    const body = {
      contexts: [
        { name: "temper", entries: [{ uri: "kb://a/b/c" }] },
      ],
    };
    const result = SyncStatusBodySchema.safeParse(body);
    expect(result.success).toBe(false);
  });
});

describe("SyncCompleteBodySchema", () => {
  it("accepts valid request body", () => {
    const body = {
      client_id: "device-abc",
      merged_resources: [
        { resource_id: "00000000-0000-0000-0000-000000000001", content_hash: "sha256:abc" },
      ],
    };
    const result = SyncCompleteBodySchema.safeParse(body);
    expect(result.success).toBe(true);
  });

  it("rejects missing client_id", () => {
    const result = SyncCompleteBodySchema.safeParse({ merged_resources: [] });
    expect(result.success).toBe(false);
  });

  it("rejects invalid UUID in resource_id", () => {
    const body = {
      client_id: "device-abc",
      merged_resources: [{ resource_id: "not-a-uuid", content_hash: "abc" }],
    };
    const result = SyncCompleteBodySchema.safeParse(body);
    expect(result.success).toBe(false);
  });
});

describe("categorizeDiffRows", () => {
  it("categorizes rows by diff_type", () => {
    const rows = [
      { resource_id: "id-1", kb_uri: "kb://a/b/1", content_hash: "h1", updated: null, diff_type: "to_push" },
      { resource_id: "id-2", kb_uri: "kb://a/b/2", content_hash: "h2", updated: null, diff_type: "to_pull" },
      { resource_id: "id-3", kb_uri: "kb://a/b/3", content_hash: "h3", updated: null, diff_type: "conflict" },
      { resource_id: "id-4", kb_uri: "kb://a/b/4", content_hash: "h4", updated: null, diff_type: "removed" },
    ];
    const result = categorizeDiffRows(rows);
    expect(result.to_push).toHaveLength(1);
    expect(result.to_pull).toHaveLength(1);
    expect(result.conflicts).toHaveLength(1);
    expect(result.removed).toHaveLength(1);
    expect(result.to_push[0].uri).toBe("kb://a/b/1");
    expect(result.to_pull[0].content_hash).toBe("h2");
    expect(result.conflicts[0].server_hash).toBe("h3");
  });

  it("returns empty arrays for no rows", () => {
    const result = categorizeDiffRows([]);
    expect(result.to_push).toHaveLength(0);
    expect(result.to_pull).toHaveLength(0);
    expect(result.conflicts).toHaveLength(0);
    expect(result.removed).toHaveLength(0);
  });

  it("handles null resource_id for to_push (new local resource)", () => {
    const rows = [
      { resource_id: null, kb_uri: "kb://a/b/new", content_hash: "h1", updated: null, diff_type: "to_push" },
    ];
    const result = categorizeDiffRows(rows);
    expect(result.to_push[0].resource_id).toBeNull();
  });
});
```

- [ ] **Step 3: Run tests to verify they fail**

Run: `cd packages/temper-cloud && npx vitest run tests/sync.test.ts`
Expected: FAIL — modules not found

- [ ] **Step 4: Implement sync.ts**

Create `packages/temper-cloud/src/sync.ts`:

```typescript
import { z } from "zod";
import type { NeonClient } from "./db.js";

// ---------------------------------------------------------------------------
// Zod schemas — request body validation
// ---------------------------------------------------------------------------

const SyncManifestEntrySchema = z.object({
  uri: z.string().startsWith("kb://"),
  local_hash: z.string().min(1),
  remote_hash: z.string().min(1),
});

const SyncContextEntriesSchema = z.object({
  name: z.string().min(1),
  entries: z.array(SyncManifestEntrySchema),
});

export const SyncStatusBodySchema = z.object({
  contexts: z.array(SyncContextEntriesSchema).min(1),
});

export const SyncCompleteBodySchema = z.object({
  client_id: z.string().min(1),
  merged_resources: z.array(
    z.object({
      resource_id: z.string().uuid(),
      content_hash: z.string().min(1),
    }),
  ).default([]),
});

export type SyncStatusBody = z.infer<typeof SyncStatusBodySchema>;
export type SyncCompleteBody = z.infer<typeof SyncCompleteBodySchema>;

// ---------------------------------------------------------------------------
// Response types
// ---------------------------------------------------------------------------

export interface SyncPushItem {
  uri: string;
  resource_id: string | null;
}

export interface SyncPullItem {
  uri: string;
  resource_id: string;
  content_hash: string;
}

export interface SyncConflictItem {
  uri: string;
  resource_id: string;
  server_hash: string;
}

export interface SyncRemovedItem {
  uri: string;
  resource_id: string;
}

export interface SyncDiffResult {
  to_push: SyncPushItem[];
  to_pull: SyncPullItem[];
  conflicts: SyncConflictItem[];
  removed: SyncRemovedItem[];
}

export interface SyncCompleteResult {
  last_sync_at: string;
  updated_count: number;
}

// ---------------------------------------------------------------------------
// Row categorization (pure function, no DB)
// ---------------------------------------------------------------------------

interface DiffRow {
  resource_id: string | null;
  kb_uri: string;
  content_hash: string;
  updated: string | null;
  diff_type: string;
}

/**
 * Categorize raw rows from sync_diff_for_device() into typed buckets.
 * Pure function — no DB access, fully unit-testable.
 */
export function categorizeDiffRows(rows: DiffRow[]): SyncDiffResult {
  const to_push: SyncPushItem[] = [];
  const to_pull: SyncPullItem[] = [];
  const conflicts: SyncConflictItem[] = [];
  const removed: SyncRemovedItem[] = [];

  for (const row of rows) {
    switch (row.diff_type) {
      case "to_push":
        to_push.push({ uri: row.kb_uri, resource_id: row.resource_id });
        break;
      case "to_pull":
        to_pull.push({
          uri: row.kb_uri,
          resource_id: row.resource_id as string,
          content_hash: row.content_hash,
        });
        break;
      case "conflict":
        conflicts.push({
          uri: row.kb_uri,
          resource_id: row.resource_id as string,
          server_hash: row.content_hash,
        });
        break;
      case "removed":
        removed.push({
          uri: row.kb_uri,
          resource_id: row.resource_id as string,
        });
        break;
    }
  }

  return { to_push, to_pull, conflicts, removed };
}

// ---------------------------------------------------------------------------
// Business logic (DB functions)
// ---------------------------------------------------------------------------

/**
 * Compute the sync diff by calling sync_diff_for_device() and categorizing results.
 */
export async function computeSyncDiff(
  db: NeonClient,
  profileId: string,
  body: SyncStatusBody,
): Promise<SyncDiffResult> {
  const contextNames: string[] = [];
  const manifestEntries: Array<{ uri: string; local_hash: string; remote_hash: string }> = [];

  for (const ctx of body.contexts) {
    contextNames.push(ctx.name);
    for (const entry of ctx.entries) {
      manifestEntries.push(entry);
    }
  }

  const rows = await db`
    SELECT resource_id, kb_uri, content_hash, updated, diff_type
    FROM sync_diff_for_device(
      ${profileId}::uuid,
      ${contextNames}::text[],
      ${JSON.stringify(manifestEntries)}::jsonb
    )
  `;

  return categorizeDiffRows(rows as unknown as DiffRow[]);
}

/**
 * Finalize a sync round: update content hashes and upsert device sync state.
 */
export async function completeSyncRound(
  db: NeonClient,
  profileId: string,
  body: SyncCompleteBody,
): Promise<SyncCompleteResult> {
  let updatedCount = 0;

  for (const mr of body.merged_resources) {
    await db`
      UPDATE kb_resources
      SET content_hash = ${mr.content_hash}, updated = now()
      WHERE id = ${mr.resource_id}::uuid
    `;
    updatedCount++;
  }

  await db`
    INSERT INTO kb_device_sync_state (id, profile_id, client_id, last_sync_at)
    VALUES (gen_random_uuid(), ${profileId}::uuid, ${body.client_id}, now())
    ON CONFLICT (profile_id, client_id)
    DO UPDATE SET last_sync_at = now()
  `;

  return {
    last_sync_at: new Date().toISOString(),
    updated_count: updatedCount,
  };
}
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `cd packages/temper-cloud && npx vitest run tests/sync.test.ts`

- [ ] **Step 6: Commit**

```bash
git add packages/temper-cloud/package.json packages/temper-cloud/src/sync.ts \
       packages/temper-cloud/tests/sync.test.ts
git commit -m "feat: add sync business logic module with zod validation and unit tests"
```

---

## Task 5: Create Thin API Sync Handlers

**Files:**
- Create: `api/sync/status.ts`
- Create: `api/sync/complete.ts`

These handlers are pure orchestration — no auth logic, no SQL, no validation logic.

- [ ] **Step 1: Create api/sync/status.ts**

```typescript
export const config = { runtime: "nodejs" };

export default async function handler(req: Request): Promise<Response> {
  const { requireMethod, authenticateRequest } = await import(
    "../../packages/temper-cloud/src/middleware.js"
  );
  const { SyncStatusBodySchema, computeSyncDiff } = await import(
    "../../packages/temper-cloud/src/sync.js"
  );

  const methodError = requireMethod(req, "POST");
  if (methodError) return methodError;

  const auth = await authenticateRequest(req);
  if (!auth.ok) return auth.response;

  const rawBody = await req.json();
  const parsed = SyncStatusBodySchema.safeParse(rawBody);
  if (!parsed.success) {
    return new Response(
      JSON.stringify({ error: { code: "VALIDATION", issues: parsed.error.issues } }),
      { status: 400, headers: { "Content-Type": "application/json" } },
    );
  }

  const result = await computeSyncDiff(auth.db, auth.profileId, parsed.data);

  return new Response(JSON.stringify(result), {
    status: 200,
    headers: { "Content-Type": "application/json" },
  });
}
```

- [ ] **Step 2: Create api/sync/complete.ts**

```typescript
export const config = { runtime: "nodejs" };

export default async function handler(req: Request): Promise<Response> {
  const { requireMethod, authenticateRequest } = await import(
    "../../packages/temper-cloud/src/middleware.js"
  );
  const { SyncCompleteBodySchema, completeSyncRound } = await import(
    "../../packages/temper-cloud/src/sync.js"
  );

  const methodError = requireMethod(req, "POST");
  if (methodError) return methodError;

  const auth = await authenticateRequest(req);
  if (!auth.ok) return auth.response;

  const rawBody = await req.json();
  const parsed = SyncCompleteBodySchema.safeParse(rawBody);
  if (!parsed.success) {
    return new Response(
      JSON.stringify({ error: { code: "VALIDATION", issues: parsed.error.issues } }),
      { status: 400, headers: { "Content-Type": "application/json" } },
    );
  }

  const result = await completeSyncRound(auth.db, auth.profileId, parsed.data);

  return new Response(JSON.stringify(result), {
    status: 200,
    headers: { "Content-Type": "application/json" },
  });
}
```

- [ ] **Step 3: Verify TypeScript compiles**

Run: `npx tsc --noEmit`

- [ ] **Step 4: Commit**

```bash
git add api/sync/status.ts api/sync/complete.ts
git commit -m "feat: add thin sync API handlers using shared middleware and sync module"
```

---

## Task 6: Review Rust Patterns Before Implementation

**No code changes — evaluation checkpoint.**

Before writing Rust sync code, review the existing patterns in temper-cli for the same SRP/testability concerns raised in the TypeScript review:

- [ ] **Step 1: Evaluate runtime duplication**

Read `import_cmd.rs`, `pull.rs`, `remove.rs` — all create `tokio::runtime::Runtime` and `temper_client::config::build_client()` inline. Confirm `with_client()` abstraction is the right fix and won't cause borrow-checker issues with `&mut manifest`.

- [ ] **Step 2: Evaluate actions layer testability**

Check whether `actions/sync.rs` functions can be unit-tested without a running server. Functions like `rehash_manifest`, `build_status_request`, `strip_frontmatter`, `parse_kb_uri` are pure — good. Functions like `push_resource`, `pull_resource` need the client — these should be structured so the pure logic is testable separately from the HTTP calls.

- [ ] **Step 3: Evaluate sync_cmd.rs design**

The original plan had the full 10-step orchestration in `sync_cmd.rs`. Check whether this is the right split vs putting more orchestration in `actions/sync.rs` (which is testable) and keeping the command as a thin shell.

- [ ] **Step 4: Document findings and adjust plan**

If any Rust patterns need redesign, update tasks 7-9 before proceeding.

---

## Task 7: Implement Rust Sync Client + Runtime Abstraction

**Files:**
- Create: `crates/temper-client/src/sync.rs`
- Modify: `crates/temper-client/src/lib.rs`
- Create: `crates/temper-cli/src/actions/runtime.rs`
- Modify: `crates/temper-cli/src/actions/mod.rs`
- Modify: `crates/temper-cli/src/commands/import_cmd.rs`
- Modify: `crates/temper-cli/src/commands/pull.rs`
- Modify: `crates/temper-cli/src/commands/remove.rs`

Detailed code for these files is deferred to post-Task 6 review, but the shape is:

- [ ] **Step 1: Create temper-client/src/sync.rs**

SyncClient with `status()` and `complete()` methods following the IngestClient pattern. Unit test for Debug impl.

- [ ] **Step 2: Wire into TemperClient**

Add `pub mod sync` and `pub fn sync(&self)` accessor.

- [ ] **Step 3: Create actions/runtime.rs**

`build_client()` and `with_client()` — evaluate borrow-checker compatibility from Task 6 review.

- [ ] **Step 4: Refactor import_cmd.rs, pull.rs, remove.rs**

Replace `tokio::runtime::Runtime::new()` + `temper_client::config::build_client()` with `with_client()`.

- [ ] **Step 5: Verify and commit**

```bash
cargo check --workspace --all-features
cargo test -p temper-client --all-features
```

---

## Task 8: Create Sync Actions Layer

**Files:**
- Create: `crates/temper-cli/src/actions/sync.rs`

Pure functions for sync orchestration (testable without server):
- `rehash_manifest(manifest, vault_root) -> Result<usize>`
- `build_status_request(manifest, context_filter) -> SyncStatusRequest`
- `build_complete_request(device_id, merged) -> SyncCompleteRequest`
- `strip_frontmatter(content) -> String`
- `parse_kb_uri(uri) -> Result<(String, String)>`
- `find_entry_by_uri(manifest, uri) -> Result<(Uuid, ManifestEntry)>`

Async functions that need client (thin wrappers):
- `push_resource(client, manifest, vault_root, uri, resource_id)`
- `pull_resource(client, manifest, vault_root, uri, resource_id, content_hash)`
- `remove_resource(manifest, vault_root, uri, resource_id)`

- [ ] **Step 1: Write unit tests for pure functions**
- [ ] **Step 2: Implement pure functions**
- [ ] **Step 3: Run tests (red → green)**
- [ ] **Step 4: Implement async functions**
- [ ] **Step 5: Verify and commit**

---

## Task 9: Create CLI Sync Command

**Files:**
- Create: `crates/temper-cli/src/commands/sync_cmd.rs`
- Modify: `crates/temper-cli/src/cli.rs`
- Modify: `crates/temper-cli/src/main.rs`

- [ ] **Step 1: Add SyncAction enum to cli.rs**

```rust
#[derive(Subcommand)]
pub enum SyncAction {
    Run {
        #[arg(long)]
        context: Vec<String>,
        #[arg(long, default_value = "text")]
        format: String,
    },
    Status {
        #[arg(long)]
        context: Vec<String>,
        #[arg(long, default_value = "text")]
        format: String,
    },
}
```

- [ ] **Step 2: Create sync_cmd.rs**

Thin orchestration: calls into `actions::sync` and `actions::runtime` for all business logic.

- [ ] **Step 3: Wire dispatch in main.rs**
- [ ] **Step 4: Verify and commit**

---

## Task 10: Full Verification

- [ ] **Step 1:** `cargo check --workspace --all-features`
- [ ] **Step 2:** `cargo clippy --workspace --all-features -- -D warnings`
- [ ] **Step 3:** `cargo test --workspace --all-features`
- [ ] **Step 4:** `npx tsc --noEmit`
- [ ] **Step 5:** `npx @biomejs/biome check api/ packages/`
- [ ] **Step 6:** `cd packages/temper-cloud && npx vitest run`
- [ ] **Step 7:** Fix any issues, final commit
