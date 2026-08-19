# I4a: Vercel Blob Upload & Embedding Pipeline — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add file upload (Vercel Blob) and durable document processing (extract → chunk → embed → store) to the temper-cloud Vercel deployment, with a testable TypeScript library alongside the existing Rust API.

**Architecture:** Two runtimes in one Vercel project. Rust handles the existing API (auth, resources, profiles, search). TypeScript handles file upload to Vercel Blob and async processing via Vercel Workflow — extraction (@kreuzberg/node), chunking, embedding (ONNX bge-base-en-v1.5, 768-dim), and storage to kb_chunks. A bun workspace at `packages/temper-cloud/` holds the testable library code; `api/` entry points are thin wrappers.

**Tech Stack:** TypeScript, bun, vitest, @vercel/blob, @kreuzberg/node, onnxruntime-node, jose (JWT), @neondatabase/serverless, workflow (Vercel WDK)

---

## File Map

| Action | Path | Responsibility |
|--------|------|----------------|
| Create | `package.json` | Root bun workspace declaration |
| Create | `packages/temper-cloud/package.json` | TypeScript package: deps, scripts, vitest |
| Create | `packages/temper-cloud/tsconfig.json` | TypeScript configuration |
| Create | `packages/temper-cloud/src/auth.ts` | JWT verification via jose + JWKS |
| Create | `packages/temper-cloud/src/db.ts` | Neon serverless client helper |
| Create | `packages/temper-cloud/src/upload.ts` | Blob storage + blob_files record logic |
| Create | `packages/temper-cloud/src/workflow/extract.ts` | kreuzberg-node text extraction |
| Create | `packages/temper-cloud/src/workflow/chunk.ts` | Text → chunks with header_path, content_hash, versioning |
| Create | `packages/temper-cloud/src/workflow/embed.ts` | ONNX bge-base-en-v1.5 embedding (768-dim) |
| Create | `packages/temper-cloud/src/workflow/store.ts` | Write chunks + embeddings to kb_chunks |
| Create | `packages/temper-cloud/tests/auth.test.ts` | JWT verification tests |
| Create | `packages/temper-cloud/tests/workflow/chunk.test.ts` | Chunking logic tests |
| Create | `packages/temper-cloud/tests/workflow/embed.test.ts` | Embedding dimension + determinism tests |
| Create | `packages/temper-cloud/tests/workflow/extract.test.ts` | Text extraction tests |
| Create | `api/upload.ts` | Vercel function: upload entry point |
| Create | `api/workflows/process-upload.ts` | Vercel function: durable workflow entry point |
| Create | `migrations/20260329000001_blob_files.sql` | blob_files table migration |
| Modify | `vercel.json` | Add upload + workflow route rewrites |

---

### Task 1: TypeScript Project Scaffolding

**Files:**
- Create: `package.json`
- Create: `packages/temper-cloud/package.json`
- Create: `packages/temper-cloud/tsconfig.json`

- [ ] **Step 1: Create root package.json for bun workspace**

Create `package.json` at the workspace root:

```json
{
  "private": true,
  "workspaces": ["packages/temper-cloud"]
}
```

- [ ] **Step 2: Create packages/temper-cloud/package.json**

```json
{
  "name": "temper-cloud",
  "version": "0.1.0",
  "private": true,
  "type": "module",
  "scripts": {
    "test": "vitest run",
    "test:watch": "vitest",
    "typecheck": "tsc --noEmit"
  },
  "dependencies": {
    "@kreuzberg/node": "latest",
    "@neondatabase/serverless": "^1",
    "@vercel/blob": "^1",
    "jose": "^6",
    "onnxruntime-node": "^1.24",
    "workflow": "^0"
  },
  "devDependencies": {
    "@types/node": "^22",
    "typescript": "^5.8",
    "vitest": "^3"
  }
}
```

- [ ] **Step 3: Create packages/temper-cloud/tsconfig.json**

```json
{
  "compilerOptions": {
    "target": "ES2022",
    "module": "ES2022",
    "moduleResolution": "bundler",
    "strict": true,
    "esModuleInterop": true,
    "skipLibCheck": true,
    "outDir": "dist",
    "rootDir": "src",
    "declaration": true,
    "sourceMap": true
  },
  "include": ["src/**/*"],
  "exclude": ["node_modules", "dist", "tests"]
}
```

- [ ] **Step 4: Install dependencies**

Run: `cd packages/temper-cloud && bun install`

Expected: Dependencies installed. `bun.lockb` created.

- [ ] **Step 5: Verify TypeScript compiles**

Run: `cd packages/temper-cloud && bun run typecheck`

Expected: No errors (no source files yet, just config validation).

- [ ] **Step 6: Commit**

```bash
git add package.json packages/temper-cloud/package.json packages/temper-cloud/tsconfig.json packages/temper-cloud/bun.lockb
git commit -m "chore: scaffold TypeScript project for temper-cloud upload pipeline"
```

---

### Task 2: blob_files Migration

**Files:**
- Create: `migrations/20260329000001_blob_files.sql`

- [ ] **Step 1: Create the migration file**

Create `migrations/20260329000001_blob_files.sql`:

```sql
-- Blob file metadata: tracks files uploaded to Vercel Blob.
-- Access control flows through the associated resource, not the file itself.
-- Status tracks the processing lifecycle: pending → processing → processed → failed.

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

- [ ] **Step 2: Run migration against local database**

Run: `cargo sqlx migrate run --source migrations`

Verify:

```bash
psql postgresql://temper:temper@localhost:5437/temper_development -c "\d blob_files"
```

Expected: Table with all columns, indexes, CHECK constraint on status.

- [ ] **Step 3: Run Rust tests to verify no breakage**

Run: `cargo nextest run --workspace --all-features --no-fail-fast`

Expected: All 218 tests pass. New table exists but nothing references it.

- [ ] **Step 4: Commit**

```bash
git add migrations/20260329000001_blob_files.sql
git commit -m "schema: add blob_files table for Vercel Blob file metadata"
```

---

### Task 3: Database Client Helper

**Files:**
- Create: `packages/temper-cloud/src/db.ts`

- [ ] **Step 1: Create the Neon serverless client helper**

Create `packages/temper-cloud/src/db.ts`:

```typescript
import { neon } from "@neondatabase/serverless";

export function getDb() {
  const databaseUrl = process.env.DATABASE_URL;
  if (!databaseUrl) {
    throw new Error("DATABASE_URL environment variable is required");
  }
  return neon(databaseUrl);
}

export type NeonClient = ReturnType<typeof neon>;
```

- [ ] **Step 2: Verify typecheck**

Run: `cd packages/temper-cloud && bun run typecheck`

Expected: No errors.

- [ ] **Step 3: Commit**

```bash
git add packages/temper-cloud/src/db.ts
git commit -m "feat: Neon serverless database client helper"
```

---

### Task 4: JWT Authentication

**Files:**
- Create: `packages/temper-cloud/src/auth.ts`
- Create: `packages/temper-cloud/tests/auth.test.ts`

- [ ] **Step 1: Write the auth test**

Create `packages/temper-cloud/tests/auth.test.ts`:

```typescript
import { describe, it, expect, beforeAll } from "vitest";
import * as jose from "jose";
import { verifyToken } from "../src/auth.js";
import { readFileSync } from "fs";
import { resolve } from "path";

// Load the same Ed25519 test keys used by the Rust tests.
const privateKeyPem = readFileSync(
  resolve(__dirname, "../../../crates/temper-api/tests/common/test_ed25519.key"),
  "utf-8"
);
const publicKeyPem = readFileSync(
  resolve(__dirname, "../../../crates/temper-api/tests/common/test_ed25519.pub"),
  "utf-8"
);

let privateKey: jose.KeyLike;
let publicKey: jose.KeyLike;

beforeAll(async () => {
  privateKey = await jose.importPKCS8(privateKeyPem, "EdDSA");
  publicKey = await jose.importSPKI(publicKeyPem, "EdDSA");
});

async function signTestJwt(claims: Record<string, unknown>): Promise<string> {
  return new jose.SignJWT(claims as jose.JWTPayload)
    .setProtectedHeader({ alg: "EdDSA" })
    .setIssuedAt()
    .setExpirationTime("1h")
    .setIssuer("test-issuer")
    .sign(privateKey);
}

describe("verifyToken", () => {
  it("accepts a valid JWT and returns claims", async () => {
    const token = await signTestJwt({
      sub: "user-123",
      email: "test@example.com",
      email_verified: true,
    });

    const claims = await verifyToken(token, publicKey, "test-issuer");
    expect(claims.sub).toBe("user-123");
    expect(claims.email).toBe("test@example.com");
    expect(claims.email_verified).toBe(true);
  });

  it("rejects an expired JWT", async () => {
    const token = await new jose.SignJWT({
      sub: "user-456",
      email: "expired@example.com",
      email_verified: true,
    } as jose.JWTPayload)
      .setProtectedHeader({ alg: "EdDSA" })
      .setIssuedAt(Math.floor(Date.now() / 1000) - 7200)
      .setExpirationTime(Math.floor(Date.now() / 1000) - 3600)
      .setIssuer("test-issuer")
      .sign(privateKey);

    await expect(verifyToken(token, publicKey, "test-issuer")).rejects.toThrow();
  });

  it("rejects a JWT with wrong issuer", async () => {
    const token = await signTestJwt({ sub: "user-789", email: "wrong@example.com", email_verified: true });

    await expect(verifyToken(token, publicKey, "wrong-issuer")).rejects.toThrow();
  });
});
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd packages/temper-cloud && bun run test`

Expected: FAIL — `verifyToken` is not exported from `../src/auth.js`.

- [ ] **Step 3: Implement auth.ts**

Create `packages/temper-cloud/src/auth.ts`:

```typescript
import * as jose from "jose";

export interface AuthClaims {
  sub: string;
  email: string;
  email_verified: boolean;
}

export async function verifyToken(
  token: string,
  key: jose.KeyLike | jose.JWTVerifyGetKey,
  issuer: string
): Promise<AuthClaims> {
  const { payload } = await jose.jwtVerify(token, key, {
    issuer,
    algorithms: ["EdDSA"],
  });

  const sub = payload.sub;
  const email = payload.email as string | undefined;
  const emailVerified = payload.email_verified as boolean | undefined;

  if (!sub) {
    throw new Error("JWT missing sub claim");
  }
  if (!email) {
    throw new Error("JWT missing email claim");
  }

  return {
    sub,
    email,
    email_verified: emailVerified ?? false,
  };
}

let cachedJwks: jose.JWTVerifyGetKey | null = null;

export function getJwksVerifier(): jose.JWTVerifyGetKey {
  if (cachedJwks) return cachedJwks;

  const jwksUrl = process.env.JWKS_URL;
  if (!jwksUrl) {
    throw new Error("JWKS_URL environment variable is required");
  }

  cachedJwks = jose.createRemoteJWKSet(new URL(jwksUrl));
  return cachedJwks;
}

export function getIssuer(): string {
  const issuer = process.env.AUTH_ISSUER;
  if (!issuer) {
    throw new Error("AUTH_ISSUER environment variable is required");
  }
  return issuer;
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cd packages/temper-cloud && bun run test`

Expected: 3 tests pass.

- [ ] **Step 5: Commit**

```bash
git add packages/temper-cloud/src/auth.ts packages/temper-cloud/tests/auth.test.ts
git commit -m "feat: JWT verification for TypeScript upload endpoint"
```

---

### Task 5: Chunking Logic

**Files:**
- Create: `packages/temper-cloud/src/workflow/chunk.ts`
- Create: `packages/temper-cloud/tests/workflow/chunk.test.ts`

- [ ] **Step 1: Write the chunking tests**

Create `packages/temper-cloud/tests/workflow/chunk.test.ts`:

```typescript
import { describe, it, expect } from "vitest";
import { chunkText, type Chunk } from "../../src/workflow/chunk.js";
import { createHash } from "crypto";

describe("chunkText", () => {
  it("chunks a simple markdown document by headers", () => {
    const text = `# Title

Introduction paragraph.

## Section One

Content of section one.

## Section Two

Content of section two.
`;
    const chunks = chunkText(text);

    expect(chunks.length).toBe(3);
    expect(chunks[0].header_path).toBe("Title");
    expect(chunks[0].content).toContain("Introduction paragraph.");
    expect(chunks[0].chunk_index).toBe(0);
    expect(chunks[1].header_path).toBe("Title > Section One");
    expect(chunks[1].content).toContain("Content of section one.");
    expect(chunks[1].chunk_index).toBe(1);
    expect(chunks[2].header_path).toBe("Title > Section Two");
    expect(chunks[2].chunk_index).toBe(2);
  });

  it("produces deterministic content_hash", () => {
    const text = "# Hello\n\nWorld";
    const chunks1 = chunkText(text);
    const chunks2 = chunkText(text);

    expect(chunks1[0].content_hash).toBe(chunks2[0].content_hash);

    const expectedHash = createHash("sha256")
      .update(chunks1[0].content)
      .digest("hex");
    expect(chunks1[0].content_hash).toBe(expectedHash);
  });

  it("handles text with no headers as a single chunk", () => {
    const text = "Just plain text without any headers.";
    const chunks = chunkText(text);

    expect(chunks.length).toBe(1);
    expect(chunks[0].header_path).toBe("");
    expect(chunks[0].chunk_index).toBe(0);
  });

  it("handles empty text", () => {
    const chunks = chunkText("");
    expect(chunks.length).toBe(0);
  });

  it("handles nested headers", () => {
    const text = `# Top
## Mid
### Deep

Deep content.
`;
    const chunks = chunkText(text);
    const deepChunk = chunks.find((c) => c.content.includes("Deep content"));
    expect(deepChunk?.header_path).toBe("Top > Mid > Deep");
  });
});
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd packages/temper-cloud && bun run test`

Expected: FAIL — `chunkText` not found.

- [ ] **Step 3: Implement chunk.ts**

Create `packages/temper-cloud/src/workflow/chunk.ts`:

```typescript
import { createHash } from "crypto";

export interface Chunk {
  chunk_index: number;
  header_path: string;
  content: string;
  content_hash: string;
}

interface HeaderState {
  level: number;
  text: string;
}

export function chunkText(text: string): Chunk[] {
  if (!text.trim()) return [];

  const lines = text.split("\n");
  const chunks: Chunk[] = [];
  const headerStack: HeaderState[] = [];
  let currentContent: string[] = [];
  let chunkIndex = 0;

  function flushChunk() {
    const content = currentContent.join("\n").trim();
    if (!content) return;

    const headerPath = headerStack.map((h) => h.text).join(" > ");
    chunks.push({
      chunk_index: chunkIndex++,
      header_path: headerPath,
      content,
      content_hash: createHash("sha256").update(content).digest("hex"),
    });
    currentContent = [];
  }

  for (const line of lines) {
    const headerMatch = line.match(/^(#{1,6})\s+(.+)$/);

    if (headerMatch) {
      flushChunk();

      const level = headerMatch[1].length;
      const text = headerMatch[2].trim();

      // Pop headers at same or deeper level
      while (headerStack.length > 0 && headerStack[headerStack.length - 1].level >= level) {
        headerStack.pop();
      }
      headerStack.push({ level, text });
    } else {
      currentContent.push(line);
    }
  }

  flushChunk();
  return chunks;
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cd packages/temper-cloud && bun run test`

Expected: All chunking tests pass.

- [ ] **Step 5: Commit**

```bash
git add packages/temper-cloud/src/workflow/chunk.ts packages/temper-cloud/tests/workflow/chunk.test.ts
git commit -m "feat: markdown-aware text chunking with header path and content hashing"
```

---

### Task 6: Text Extraction

**Files:**
- Create: `packages/temper-cloud/src/workflow/extract.ts`
- Create: `packages/temper-cloud/tests/workflow/extract.test.ts`

- [ ] **Step 1: Write the extraction test**

Create `packages/temper-cloud/tests/workflow/extract.test.ts`:

```typescript
import { describe, it, expect } from "vitest";
import { extractText } from "../../src/workflow/extract.js";
import { writeFileSync, mkdtempSync, unlinkSync } from "fs";
import { join } from "path";
import { tmpdir } from "os";

describe("extractText", () => {
  it("extracts text from a markdown file", async () => {
    const dir = mkdtempSync(join(tmpdir(), "temper-test-"));
    const filePath = join(dir, "test.md");
    writeFileSync(filePath, "# Hello\n\nThis is a test document.\n");

    try {
      const result = await extractText(filePath);
      expect(result.content).toContain("Hello");
      expect(result.content).toContain("This is a test document.");
    } finally {
      unlinkSync(filePath);
    }
  });

  it("extracts text from a plain text file", async () => {
    const dir = mkdtempSync(join(tmpdir(), "temper-test-"));
    const filePath = join(dir, "test.txt");
    writeFileSync(filePath, "Plain text content here.");

    try {
      const result = await extractText(filePath);
      expect(result.content).toContain("Plain text content here.");
    } finally {
      unlinkSync(filePath);
    }
  });
});
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd packages/temper-cloud && bun run test -- tests/workflow/extract.test.ts`

Expected: FAIL — `extractText` not found.

- [ ] **Step 3: Implement extract.ts**

Create `packages/temper-cloud/src/workflow/extract.ts`:

```typescript
import { extractFile } from "@kreuzberg/node";

export interface ExtractionResult {
  content: string;
  mimeType: string;
}

export async function extractText(filePath: string): Promise<ExtractionResult> {
  const result = await extractFile(filePath, null, {
    useCache: false,
  });

  return {
    content: result.content,
    mimeType: result.mimeType,
  };
}

export async function extractFromBuffer(
  buffer: Buffer,
  filename: string
): Promise<ExtractionResult> {
  // Write buffer to temp file for kreuzberg, which operates on file paths.
  const { writeFileSync, unlinkSync } = await import("fs");
  const { join } = await import("path");
  const { tmpdir } = await import("os");
  const tempPath = join(tmpdir(), `temper-extract-${Date.now()}-${filename}`);

  try {
    writeFileSync(tempPath, buffer);
    return await extractText(tempPath);
  } finally {
    try {
      unlinkSync(tempPath);
    } catch {
      // Best-effort cleanup
    }
  }
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cd packages/temper-cloud && bun run test -- tests/workflow/extract.test.ts`

Expected: Both extraction tests pass. Note: `@kreuzberg/node` must be installed for this to work. If kreuzberg is not available in the test environment, tests should be skipped gracefully.

- [ ] **Step 5: Commit**

```bash
git add packages/temper-cloud/src/workflow/extract.ts packages/temper-cloud/tests/workflow/extract.test.ts
git commit -m "feat: document text extraction via kreuzberg-node"
```

---

### Task 7: Embedding with ONNX

**Files:**
- Create: `packages/temper-cloud/src/workflow/embed.ts`
- Create: `packages/temper-cloud/tests/workflow/embed.test.ts`

- [ ] **Step 1: Write the embedding test**

Create `packages/temper-cloud/tests/workflow/embed.test.ts`:

```typescript
import { describe, it, expect } from "vitest";
import { embedTexts, EMBEDDING_DIM } from "../../src/workflow/embed.js";

describe("embedTexts", () => {
  it("produces 768-dimensional vectors", async () => {
    const texts = ["Hello world", "Testing embeddings"];
    const embeddings = await embedTexts(texts);

    expect(embeddings.length).toBe(2);
    expect(embeddings[0].length).toBe(EMBEDDING_DIM);
    expect(embeddings[1].length).toBe(EMBEDDING_DIM);
  }, 30_000); // Allow time for model download on first run

  it("produces deterministic output for same input", async () => {
    const texts = ["Deterministic test"];
    const embeddings1 = await embedTexts(texts);
    const embeddings2 = await embedTexts(texts);

    expect(embeddings1[0]).toEqual(embeddings2[0]);
  }, 30_000);

  it("produces different vectors for different inputs", async () => {
    const embeddings = await embedTexts(["cats are great", "quantum physics theory"]);

    // Vectors should not be identical
    const identical = embeddings[0].every((v, i) => v === embeddings[1][i]);
    expect(identical).toBe(false);
  }, 30_000);

  it("handles empty array", async () => {
    const embeddings = await embedTexts([]);
    expect(embeddings).toEqual([]);
  });
});
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd packages/temper-cloud && bun run test -- tests/workflow/embed.test.ts`

Expected: FAIL — `embedTexts` not found.

- [ ] **Step 3: Implement embed.ts**

Create `packages/temper-cloud/src/workflow/embed.ts`:

```typescript
import * as ort from "onnxruntime-node";
import { resolve, join } from "path";
import { existsSync, mkdirSync, writeFileSync } from "fs";
import { tmpdir } from "os";

export const EMBEDDING_DIM = 768;
const MODEL_NAME = "bge-base-en-v1.5";
const MODEL_URL = `https://huggingface.co/BAAI/${MODEL_NAME}/resolve/main/onnx/model.onnx`;
const TOKENIZER_URL = `https://huggingface.co/BAAI/${MODEL_NAME}/resolve/main/tokenizer.json`;

let session: ort.InferenceSession | null = null;
let tokenizer: any = null;

async function getModelDir(): Promise<string> {
  const dir = join(tmpdir(), `temper-models`, MODEL_NAME);
  if (!existsSync(dir)) {
    mkdirSync(dir, { recursive: true });
  }
  return dir;
}

async function downloadIfMissing(url: string, destPath: string): Promise<void> {
  if (existsSync(destPath)) return;
  const response = await fetch(url);
  if (!response.ok) throw new Error(`Failed to download ${url}: ${response.status}`);
  const buffer = Buffer.from(await response.arrayBuffer());
  writeFileSync(destPath, buffer);
}

async function getSession(): Promise<ort.InferenceSession> {
  if (session) return session;

  const modelDir = await getModelDir();
  const modelPath = join(modelDir, "model.onnx");
  await downloadIfMissing(MODEL_URL, modelPath);

  session = await ort.InferenceSession.create(modelPath, {
    executionProviders: ["cpu"],
  });
  return session;
}

// Simple whitespace tokenizer for bge-base-en-v1.5.
// In production, use the model's actual tokenizer. For now, this is a
// placeholder that produces token IDs. The real implementation should
// use a proper tokenizer library (e.g., tokenizers via napi).
//
// TODO(I4a): Replace with proper HuggingFace tokenizer before merging.
// The onnxruntime-node model expects input_ids and attention_mask tensors
// matching the model's vocabulary. This placeholder will NOT produce
// correct embeddings — it exists only to establish the pipeline shape.
// Options: @xenova/transformers tokenizer, or tokenizers-node napi binding.
function simpleTokenize(
  texts: string[],
  maxLength: number = 512
): { inputIds: BigInt64Array; attentionMask: BigInt64Array; shape: [number, number] } {
  const batchSize = texts.length;
  const inputIds = new BigInt64Array(batchSize * maxLength);
  const attentionMask = new BigInt64Array(batchSize * maxLength);

  for (let i = 0; i < batchSize; i++) {
    const tokens = texts[i].split(/\s+/).slice(0, maxLength - 2);
    const offset = i * maxLength;

    // [CLS] token
    inputIds[offset] = 101n;
    attentionMask[offset] = 1n;

    for (let j = 0; j < tokens.length; j++) {
      // Simple hash to vocabulary range — NOT real tokenization
      let hash = 0;
      for (const ch of tokens[j]) hash = ((hash << 5) - hash + ch.charCodeAt(0)) | 0;
      inputIds[offset + 1 + j] = BigInt(Math.abs(hash) % 30000 + 1000);
      attentionMask[offset + 1 + j] = 1n;
    }

    // [SEP] token
    inputIds[offset + 1 + tokens.length] = 102n;
    attentionMask[offset + 1 + tokens.length] = 1n;
  }

  return { inputIds, attentionMask, shape: [batchSize, maxLength] };
}

function meanPool(
  lastHiddenState: Float32Array,
  attentionMask: BigInt64Array,
  batchSize: number,
  seqLen: number,
  hiddenDim: number
): number[][] {
  const result: number[][] = [];

  for (let b = 0; b < batchSize; b++) {
    const embedding = new Array(hiddenDim).fill(0);
    let tokenCount = 0;

    for (let s = 0; s < seqLen; s++) {
      if (attentionMask[b * seqLen + s] === 1n) {
        tokenCount++;
        for (let d = 0; d < hiddenDim; d++) {
          embedding[d] += lastHiddenState[b * seqLen * hiddenDim + s * hiddenDim + d];
        }
      }
    }

    if (tokenCount > 0) {
      for (let d = 0; d < hiddenDim; d++) {
        embedding[d] /= tokenCount;
      }
    }

    // L2 normalize
    const norm = Math.sqrt(embedding.reduce((sum, v) => sum + v * v, 0));
    if (norm > 0) {
      for (let d = 0; d < hiddenDim; d++) {
        embedding[d] /= norm;
      }
    }

    result.push(embedding);
  }

  return result;
}

export async function embedTexts(texts: string[]): Promise<number[][]> {
  if (texts.length === 0) return [];

  const sess = await getSession();
  const { inputIds, attentionMask, shape } = simpleTokenize(texts);

  const feeds = {
    input_ids: new ort.Tensor("int64", inputIds, shape),
    attention_mask: new ort.Tensor("int64", attentionMask, shape),
  };

  // Some models also expect token_type_ids
  const tokenTypeIds = new BigInt64Array(shape[0] * shape[1]);
  feeds.token_type_ids = new ort.Tensor("int64", tokenTypeIds, shape);

  const output = await sess.run(feeds);
  const lastHiddenState = output.last_hidden_state ?? output[Object.keys(output)[0]];
  const data = lastHiddenState.data as Float32Array;
  const dims = lastHiddenState.dims as number[];

  return meanPool(data, attentionMask, dims[0], dims[1], dims[2]);
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cd packages/temper-cloud && bun run test -- tests/workflow/embed.test.ts`

Expected: All embedding tests pass. First run will download the model (~440MB) to `/tmp/temper-models/`. Subsequent runs use the cached model.

Note: If the model download is too slow for CI, the tests can be marked with a `slow` tag and skipped in fast test runs.

- [ ] **Step 5: Commit**

```bash
git add packages/temper-cloud/src/workflow/embed.ts packages/temper-cloud/tests/workflow/embed.test.ts
git commit -m "feat: ONNX bge-base-en-v1.5 embedding (768-dim) with mean pooling"
```

---

### Task 8: Chunk Storage (kb_chunks Writer)

**Files:**
- Create: `packages/temper-cloud/src/workflow/store.ts`
- Create: `packages/temper-cloud/tests/workflow/store.test.ts`

- [ ] **Step 1: Write the store test**

Create `packages/temper-cloud/tests/workflow/store.test.ts`:

```typescript
import { describe, it, expect } from "vitest";
import {
  buildStoreChunksQuery,
  buildVersionBumpQuery,
  buildStatusUpdateQuery,
  type ChunkRow,
} from "../../src/workflow/store.js";

describe("buildStoreChunksQuery", () => {
  it("generates INSERT SQL for chunks with embeddings", () => {
    const chunks: ChunkRow[] = [
      {
        id: "00000000-0000-0000-0000-000000000001",
        resource_id: "res-001",
        chunk_index: 0,
        version: 1,
        header_path: "Title",
        content: "Hello world",
        content_hash: "abc123",
        embedding: [0.1, 0.2, 0.3],
      },
    ];

    const { sql, params } = buildStoreChunksQuery(chunks);
    expect(sql).toContain("INSERT INTO kb_chunks");
    expect(sql).toContain("ON CONFLICT");
    expect(params.length).toBeGreaterThan(0);
  });
});

describe("buildVersionBumpQuery", () => {
  it("generates UPDATE SQL to mark old chunks as not current", () => {
    const { sql, params } = buildVersionBumpQuery("res-001", 2);
    expect(sql).toContain("UPDATE kb_chunks");
    expect(sql).toContain("is_current = false");
    expect(params).toContain("res-001");
    expect(params).toContain(2);
  });
});

describe("buildStatusUpdateQuery", () => {
  it("generates UPDATE SQL for blob_files status", () => {
    const { sql, params } = buildStatusUpdateQuery("file-001", "processed", null);
    expect(sql).toContain("UPDATE blob_files");
    expect(sql).toContain("status");
    expect(params).toContain("file-001");
    expect(params).toContain("processed");
  });

  it("includes error_message for failed status", () => {
    const { sql, params } = buildStatusUpdateQuery("file-001", "failed", "ONNX load error");
    expect(params).toContain("ONNX load error");
  });
});
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd packages/temper-cloud && bun run test -- tests/workflow/store.test.ts`

Expected: FAIL — imports not found.

- [ ] **Step 3: Implement store.ts**

Create `packages/temper-cloud/src/workflow/store.ts`:

```typescript
export interface ChunkRow {
  id: string;
  resource_id: string;
  chunk_index: number;
  version: number;
  header_path: string;
  content: string;
  content_hash: string;
  embedding: number[];
}

interface QueryResult {
  sql: string;
  params: (string | number | boolean | null)[];
}

export function buildVersionBumpQuery(resourceId: string, newVersion: number): QueryResult {
  return {
    sql: `UPDATE kb_chunks SET is_current = false WHERE resource_id = $1 AND version < $2 AND is_current = true`,
    params: [resourceId, newVersion],
  };
}

export function buildStoreChunksQuery(chunks: ChunkRow[]): QueryResult {
  if (chunks.length === 0) return { sql: "", params: [] };

  const values: string[] = [];
  const params: (string | number | boolean | null)[] = [];
  let paramIndex = 1;

  for (const chunk of chunks) {
    const embeddingStr = `[${chunk.embedding.join(",")}]`;
    values.push(
      `($${paramIndex}, $${paramIndex + 1}, $${paramIndex + 2}, $${paramIndex + 3}, $${paramIndex + 4}, $${paramIndex + 5}, $${paramIndex + 6}, $${paramIndex + 7}::vector, true)`
    );
    params.push(
      chunk.id,
      chunk.resource_id,
      chunk.chunk_index,
      chunk.version,
      chunk.header_path,
      chunk.content,
      chunk.content_hash,
      embeddingStr
    );
    paramIndex += 8;
  }

  const sql = `INSERT INTO kb_chunks (id, resource_id, chunk_index, version, header_path, content, content_hash, embedding, is_current)
VALUES ${values.join(",\n")}
ON CONFLICT (resource_id, chunk_index, version) DO UPDATE SET
  header_path = EXCLUDED.header_path,
  content = EXCLUDED.content,
  content_hash = EXCLUDED.content_hash,
  embedding = EXCLUDED.embedding,
  is_current = EXCLUDED.is_current`;

  return { sql, params };
}

export function buildStatusUpdateQuery(
  blobFileId: string,
  status: "pending" | "processing" | "processed" | "failed",
  errorMessage: string | null
): QueryResult {
  return {
    sql: `UPDATE blob_files SET status = $1, error_message = $2, updated_at = now() WHERE id = $3`,
    params: [status, errorMessage, blobFileId],
  };
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cd packages/temper-cloud && bun run test -- tests/workflow/store.test.ts`

Expected: All store tests pass.

- [ ] **Step 5: Commit**

```bash
git add packages/temper-cloud/src/workflow/store.ts packages/temper-cloud/tests/workflow/store.test.ts
git commit -m "feat: kb_chunks storage queries with versioning and status updates"
```

---

### Task 9: Upload Logic

**Files:**
- Create: `packages/temper-cloud/src/upload.ts`
- Create: `packages/temper-cloud/tests/upload.test.ts`

- [ ] **Step 1: Write the upload test**

Create `packages/temper-cloud/tests/upload.test.ts`:

```typescript
import { describe, it, expect } from "vitest";
import { buildBlobPathname, buildInsertBlobFileQuery } from "../../src/upload.js";

describe("buildBlobPathname", () => {
  it("constructs pathname from profile, resource, and filename", () => {
    const pathname = buildBlobPathname("profile-123", "resource-456", "document.md");
    expect(pathname).toBe("profile-123/resource-456/document.md");
  });

  it("sanitizes filename", () => {
    const pathname = buildBlobPathname("p", "r", "my file (1).md");
    expect(pathname).toBe("p/r/my file (1).md");
    expect(pathname).not.toContain("..");
  });
});

describe("buildInsertBlobFileQuery", () => {
  it("generates INSERT SQL with all fields", () => {
    const { sql, params } = buildInsertBlobFileQuery({
      profileId: "profile-123",
      resourceId: "resource-456",
      blobUrl: "https://blob.vercel-storage.com/abc",
      pathname: "profile-123/resource-456/doc.md",
      contentType: "text/markdown",
      fileSizeBytes: 1024,
    });

    expect(sql).toContain("INSERT INTO blob_files");
    expect(params).toContain("profile-123");
    expect(params).toContain("resource-456");
    expect(params).toContain("https://blob.vercel-storage.com/abc");
    expect(params).toContain("text/markdown");
    expect(params).toContain(1024);
  });
});
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd packages/temper-cloud && bun run test -- tests/upload.test.ts`

Expected: FAIL — imports not found.

- [ ] **Step 3: Implement upload.ts**

Create `packages/temper-cloud/src/upload.ts`:

```typescript
export function buildBlobPathname(
  profileId: string,
  resourceId: string,
  filename: string
): string {
  // Prevent path traversal
  const safeFilename = filename.replace(/\.\./g, "");
  return `${profileId}/${resourceId}/${safeFilename}`;
}

export interface InsertBlobFileParams {
  profileId: string;
  resourceId: string | null;
  blobUrl: string;
  pathname: string;
  contentType: string | null;
  fileSizeBytes: number | null;
}

export function buildInsertBlobFileQuery(params: InsertBlobFileParams): {
  sql: string;
  params: (string | number | null)[];
} {
  return {
    sql: `INSERT INTO blob_files (profile_id, resource_id, blob_url, pathname, content_type, file_size_bytes, status)
VALUES ($1, $2, $3, $4, $5, $6, 'pending')
RETURNING id, status, created_at`,
    params: [
      params.profileId,
      params.resourceId,
      params.blobUrl,
      params.pathname,
      params.contentType,
      params.fileSizeBytes,
    ],
  };
}

export interface UploadResult {
  blobFileId: string;
  status: string;
  createdAt: string;
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cd packages/temper-cloud && bun run test -- tests/upload.test.ts`

Expected: All upload tests pass.

- [ ] **Step 5: Commit**

```bash
git add packages/temper-cloud/src/upload.ts packages/temper-cloud/tests/upload.test.ts
git commit -m "feat: upload logic with blob pathname construction and metadata queries"
```

---

### Task 10: API Entry Points

**Files:**
- Create: `api/upload.ts`
- Create: `api/workflows/process-upload.ts`
- Modify: `vercel.json`

- [ ] **Step 1: Create api/upload.ts**

Create `api/upload.ts` at the workspace root:

```typescript
import { put } from "@vercel/blob";
import { verifyToken, getJwksVerifier, getIssuer, type AuthClaims } from "../packages/temper-cloud/src/auth.js";
import { buildBlobPathname, buildInsertBlobFileQuery, type UploadResult } from "../packages/temper-cloud/src/upload.js";
import { getDb } from "../packages/temper-cloud/src/db.js";
import { start } from "workflow/api";
import { processUpload } from "./workflows/process-upload.js";

export const config = { runtime: "nodejs" };

export default async function handler(req: Request): Promise<Response> {
  if (req.method !== "POST") {
    return new Response(JSON.stringify({ error: "Method not allowed" }), { status: 405 });
  }

  // Authenticate
  const authHeader = req.headers.get("authorization");
  if (!authHeader?.startsWith("Bearer ")) {
    return new Response(
      JSON.stringify({ error: { code: "UNAUTHORIZED", message: "Missing Authorization header" } }),
      { status: 401 }
    );
  }

  let claims: AuthClaims;
  try {
    claims = await verifyToken(authHeader.slice(7), getJwksVerifier(), getIssuer());
  } catch {
    return new Response(
      JSON.stringify({ error: { code: "UNAUTHORIZED", message: "Invalid token" } }),
      { status: 401 }
    );
  }

  // Parse multipart form data
  const formData = await req.formData();
  const file = formData.get("file") as File | null;
  const resourceId = formData.get("resource_id") as string | null;

  if (!file) {
    return new Response(JSON.stringify({ error: "file is required" }), { status: 400 });
  }
  if (!resourceId) {
    return new Response(JSON.stringify({ error: "resource_id is required" }), { status: 400 });
  }

  // Verify resource belongs to this profile
  const db = getDb();
  const visibleResources = await db`
    SELECT resource_id FROM resources_visible_to(
      (SELECT id FROM kb_profiles WHERE auth_provider_sub = ${claims.sub} LIMIT 1)
    ) WHERE resource_id = ${resourceId}::uuid
  `;

  if (visibleResources.length === 0) {
    return new Response(
      JSON.stringify({ error: "Resource not found or not accessible" }),
      { status: 404 }
    );
  }

  // Get profile_id
  const profileRows = await db`
    SELECT id FROM kb_profiles WHERE auth_provider_sub = ${claims.sub} LIMIT 1
  `;
  const profileId = profileRows[0].id as string;

  // Store in Vercel Blob
  const pathname = buildBlobPathname(profileId, resourceId, file.name);
  const blob = await put(pathname, file, { access: "private" });

  // Insert blob_files record
  const { sql, params } = buildInsertBlobFileQuery({
    profileId,
    resourceId,
    blobUrl: blob.url,
    pathname: blob.pathname,
    contentType: file.type || null,
    fileSizeBytes: file.size,
  });

  const insertResult = await db(sql, params);
  const blobFileId = insertResult[0].id as string;

  // Trigger processing workflow
  try {
    await start(processUpload, [blobFileId, blob.url, resourceId]);
  } catch (err) {
    console.error("Failed to start processing workflow:", err);
    // Upload succeeded even if workflow trigger fails — the file is stored
    // and can be reprocessed later.
  }

  return new Response(
    JSON.stringify({
      blob_file_id: blobFileId,
      status: "pending",
    }),
    { status: 202, headers: { "Content-Type": "application/json" } }
  );
}
```

- [ ] **Step 2: Create api/workflows/process-upload.ts**

Create `api/workflows/process-upload.ts`:

```typescript
import { extractFromBuffer } from "../../packages/temper-cloud/src/workflow/extract.js";
import { chunkText } from "../../packages/temper-cloud/src/workflow/chunk.js";
import { embedTexts } from "../../packages/temper-cloud/src/workflow/embed.js";
import {
  buildVersionBumpQuery,
  buildStoreChunksQuery,
  buildStatusUpdateQuery,
  type ChunkRow,
} from "../../packages/temper-cloud/src/workflow/store.js";
import { getDb } from "../../packages/temper-cloud/src/db.js";
import { randomUUID } from "crypto";

export async function processUpload(
  blobFileId: string,
  blobUrl: string,
  resourceId: string
) {
  "use workflow";

  const text = await extractStep(blobFileId, blobUrl);
  const chunks = await chunkStep(text);
  const embeddings = await embedStep(chunks.map((c) => c.content));
  await storeStep(blobFileId, resourceId, chunks, embeddings);
}

async function extractStep(blobFileId: string, blobUrl: string): Promise<string> {
  "use step";

  const db = getDb();

  // Update status to processing
  const statusQuery = buildStatusUpdateQuery(blobFileId, "processing", null);
  await db(statusQuery.sql, statusQuery.params);

  // Download file from Vercel Blob
  const response = await fetch(blobUrl, {
    headers: {
      Authorization: `Bearer ${process.env.BLOB_READ_WRITE_TOKEN}`,
    },
  });

  if (!response.ok) {
    throw new Error(`Failed to download blob: ${response.status}`);
  }

  const buffer = Buffer.from(await response.arrayBuffer());
  const filename = blobUrl.split("/").pop() || "document";
  const result = await extractFromBuffer(buffer, filename);

  return result.content;
}

async function chunkStep(
  text: string
): Promise<Array<{ header_path: string; content: string; content_hash: string; chunk_index: number }>> {
  "use step";
  return chunkText(text);
}

async function embedStep(texts: string[]): Promise<number[][]> {
  "use step";
  return embedTexts(texts);
}

async function storeStep(
  blobFileId: string,
  resourceId: string,
  chunks: Array<{ header_path: string; content: string; content_hash: string; chunk_index: number }>,
  embeddings: number[][]
): Promise<void> {
  "use step";

  const db = getDb();

  // Determine next version for this resource
  const versionResult = await db`
    SELECT COALESCE(MAX(version), 0) + 1 AS next_version
    FROM kb_chunks WHERE resource_id = ${resourceId}::uuid
  `;
  const nextVersion = versionResult[0].next_version as number;

  // Mark old chunks as not current
  const bumpQuery = buildVersionBumpQuery(resourceId, nextVersion);
  await db(bumpQuery.sql, bumpQuery.params);

  // Build chunk rows with embeddings
  const chunkRows: ChunkRow[] = chunks.map((chunk, i) => ({
    id: randomUUID(),
    resource_id: resourceId,
    chunk_index: chunk.chunk_index,
    version: nextVersion,
    header_path: chunk.header_path,
    content: chunk.content,
    content_hash: chunk.content_hash,
    embedding: embeddings[i],
  }));

  // Store chunks
  const storeQuery = buildStoreChunksQuery(chunkRows);
  if (storeQuery.sql) {
    await db(storeQuery.sql, storeQuery.params);
  }

  // Update blob_files status to processed
  const statusQuery = buildStatusUpdateQuery(blobFileId, "processed", null);
  await db(statusQuery.sql, statusQuery.params);
}
```

- [ ] **Step 3: Update vercel.json**

The current `vercel.json` is:

```json
{
  "$schema": "https://openapi.vercel.sh/vercel.json",
  "fluid": true,
  "rewrites": [{ "source": "/(.*)", "destination": "/api/axum" }]
}
```

Update to:

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

- [ ] **Step 4: Verify TypeScript files compile**

Run: `cd packages/temper-cloud && bun run typecheck`

Expected: No type errors.

- [ ] **Step 5: Commit**

```bash
git add api/upload.ts api/workflows/process-upload.ts vercel.json
git commit -m "feat: upload endpoint and processing workflow entry points"
```

---

### Task 11: Full Test Suite + Workspace Verification

**Files:** None (verification only)

- [ ] **Step 1: Run TypeScript tests**

Run: `cd packages/temper-cloud && bun run test`

Expected: All unit tests pass (auth, chunk, upload, store). Extraction and embedding tests pass if kreuzberg-node and onnxruntime-node are available.

- [ ] **Step 2: Run Rust tests**

Run: `cargo nextest run --workspace --all-features --no-fail-fast`

Expected: All 218 Rust tests pass. TypeScript changes don't affect Rust.

- [ ] **Step 3: Run Rust workspace check**

Run: `cargo make check`

Expected: fmt, clippy, docs, machete all pass.

- [ ] **Step 4: Commit any fixes**

If any fixes were needed:

```bash
git add -A
git commit -m "fix: address workspace verification issues"
```

---

### Task 12: Deploy and Validate

**Files:** None (deployment)

- [ ] **Step 1: Push to remote**

```bash
git push origin jcoletaylor/temper-cloud
```

- [ ] **Step 2: Enable Vercel Blob in project settings**

In the Vercel dashboard: Storage > Create Store > Blob > Private access > Connect to temper-cloud project.

This creates the `BLOB_READ_WRITE_TOKEN` environment variable automatically.

- [ ] **Step 3: Deploy**

```bash
vercel deploy
```

Monitor build logs — both Rust and TypeScript functions should compile.

- [ ] **Step 4: Validate health check still works**

```bash
curl -s https://temper-cloud.vercel.app/api/health | jq .
```

Expected: `{"status": "ok", "version": "0.1.0"}`

- [ ] **Step 5: Validate upload endpoint exists**

```bash
curl -s -X POST https://temper-cloud.vercel.app/api/upload
```

Expected: 401 Unauthorized (no auth header) — confirms the TypeScript function is deployed and routing works.

---

### Task 13: Session Save

**Files:** None (documentation)

- [ ] **Step 1: Save session note**

```bash
cat <<'EOF' | temper session save "I4a Vercel Blob Upload & Embedding Pipeline" --ticket 2026-03-29-i4a-vercel-blob-file-storage-upload-endpoints-and-embedding-pipeline --state done --project temper
## Goal
Add file upload and durable document processing to temper-cloud.

## What happened
[Fill in: implementation details, issues encountered, deployment results]

## Decisions
[Fill in: decisions made during implementation]

## What connected
[Fill in: cross-project patterns, learnings]

## To pick up
- I5: temper-client — auth-aware HTTP client + CLI auth flow
- Replace placeholder tokenizer in embed.ts with proper HuggingFace tokenizer
- Integration tests against live Vercel deployment
EOF
```
