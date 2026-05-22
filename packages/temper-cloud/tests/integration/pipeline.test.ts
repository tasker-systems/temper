import { join } from "node:path";
import type postgres from "postgres";
import { afterAll, beforeAll, describe, expect, it } from "vitest";
import { chunkText } from "../../src/workflow/chunk.js";
import { EMBEDDING_DIM, embedTexts } from "../../src/workflow/embed.js";
import { extractText } from "../../src/workflow/extract.js";
import { buildStatusUpdateQuery } from "../../src/workflow/store.js";
import {
  cleanupTestResource,
  createTestBlobFile,
  createTestResource,
  getTestDb,
  type TestResource,
} from "./helpers/db.js";

const FIXTURES = join(import.meta.dirname, "../fixtures");

let sql: postgres.Sql;

beforeAll(() => {
  sql = getTestDb();
});

afterAll(async () => {
  await sql.end();
});

// ---------------------------------------------------------------------------
// 1. Extraction tests — verify kreuzberg handles each fixture format
// ---------------------------------------------------------------------------

describe("fixture extraction", () => {
  it("extracts text from markdown", async () => {
    const result = await extractText(join(FIXTURES, "simple.md"));
    expect(result.content).toContain("Getting Started");
    expect(result.content).toContain("cargo install temper");
    expect(result.content).toContain("Search Settings");
  });

  it("extracts text from plain text", async () => {
    const result = await extractText(join(FIXTURES, "simple.txt"));
    expect(result.content).toContain("knowledge management tool");
    expect(result.content).toContain("semantic search");
  });

  it("extracts text from PDF", async () => {
    const result = await extractText(join(FIXTURES, "simple.pdf"));
    expect(result.content).toContain("Temper Cloud Architecture");
    expect(result.content).toContain("vector embeddings");
  });

  it("extracts text from DOCX", async () => {
    const result = await extractText(join(FIXTURES, "simple.docx"));
    expect(result.content).toContain("Temper Design Document");
    expect(result.content).toContain("Upload Pipeline");
  });

  it("extracts text from SVG", async () => {
    const result = await extractText(join(FIXTURES, "simple.svg"));
    expect(result.content).toContain("Temper Knowledge Base");
  });

  it("extracts text from PNG via OCR", async (ctx) => {
    let result: Awaited<ReturnType<typeof extractText>>;
    try {
      result = await extractText(join(FIXTURES, "ocr-test.png"));
    } catch (e) {
      const msg = String(e);
      // kreuzberg throws when Tesseract isn't installed or tessdata is missing.
      // On CI (ubuntu-latest) tesseract is installed via apt in test-typescript.yml.
      // For local devs without it, emit a helpful skip notice instead of failing.
      if (msg.includes("Tesseract") || msg.includes("tessdata")) {
        console.warn(
          "\n  ⚠️  Skipping OCR test — Tesseract is not installed locally.\n" +
            "     Install with:\n" +
            "       macOS:  brew install tesseract\n" +
            "       Linux:  sudo apt-get install tesseract-ocr tesseract-ocr-eng\n",
        );
        ctx.skip();
        return;
      }
      throw e;
    }
    expect(result.content.length).toBeGreaterThan(0);
    expect(result.content.toLowerCase()).toContain("temper");
  });
});

// ---------------------------------------------------------------------------
// 2. Chunking tests — verify header_path, content_hash, chunk_index
// ---------------------------------------------------------------------------

describe("fixture chunking", () => {
  it("chunks markdown into indexed chunks with deterministic hashes", async () => {
    const { content } = await extractText(join(FIXTURES, "simple.md"));
    const chunks = chunkText(content);

    // The chunker merges small adjacent sections, so simple.md (a short
    // fixture with brief subsections) currently produces a single chunk with
    // an empty header_path. Header-path correctness on multi-chunk output
    // should be tested with a larger fixture — tracked as follow-up. For now
    // this test just pins the structural shape of the chunk output.
    expect(chunks.length).toBeGreaterThanOrEqual(1);
    expect(chunks[0].chunk_index).toBe(0);
    expect(typeof chunks[0].header_path).toBe("string");

    // Each chunk has a deterministic content hash.
    for (const chunk of chunks) {
      expect(chunk.content_hash).toMatch(/^[a-f0-9]{64}$/);
    }
  });

  it("produces sequential chunk indices", async () => {
    const { content } = await extractText(join(FIXTURES, "simple.md"));
    const chunks = chunkText(content);

    for (let i = 0; i < chunks.length; i++) {
      expect(chunks[i].chunk_index).toBe(i);
    }
  });

  it("plain text produces a single chunk", async () => {
    const { content } = await extractText(join(FIXTURES, "simple.txt"));
    const chunks = chunkText(content);

    expect(chunks.length).toBe(1);
    expect(chunks[0].header_path).toBe("");
  });
});

// ---------------------------------------------------------------------------
// 3. Embedding tests — verify 768-dim vectors from extracted+chunked content
// ---------------------------------------------------------------------------

/**
 * True when an error is a network-connectivity failure — typically reaching
 * HuggingFace Hub to pull the embedding model. This is CI-environment flake,
 * not a code defect, so the embedding test skips rather than fails on it.
 */
function isNetworkConnectivityError(err: unknown): boolean {
  const NETWORK_CODES = new Set([
    "UND_ERR_CONNECT_TIMEOUT",
    "UND_ERR_HEADERS_TIMEOUT",
    "UND_ERR_SOCKET",
    "ECONNRESET",
    "ECONNREFUSED",
    "ENOTFOUND",
    "EAI_AGAIN",
    "ETIMEDOUT",
  ]);
  // `fetch failed` wraps the underlying undici error as `.cause`; walk the chain.
  let current: unknown = err;
  for (let depth = 0; depth < 5 && current instanceof Error; depth++) {
    const code = (current as { code?: unknown }).code;
    if (typeof code === "string" && NETWORK_CODES.has(code)) return true;
    if (/fetch failed|connect timeout|getaddrinfo|network/i.test(current.message)) return true;
    current = (current as { cause?: unknown }).cause;
  }
  return false;
}

describe("fixture embedding", () => {
  it("embeds chunked markdown to 768-dim vectors", async (ctx) => {
    const { content } = await extractText(join(FIXTURES, "simple.md"));
    const chunks = chunkText(content);
    const texts = chunks.map((c) => c.content);

    let embeddings: number[][];
    try {
      embeddings = await embedTexts(texts);
    } catch (err) {
      // Pulling the embedding model reaches HuggingFace Hub over the network.
      // A connectivity failure there is environment flake, not a defect —
      // skip so it reads as a skipped test, not a misleading red signal.
      if (isNetworkConnectivityError(err)) {
        ctx.skip("HuggingFace Hub unreachable — embedding model could not be pulled");
      }
      throw err;
    }

    expect(embeddings.length).toBe(chunks.length);
    for (const vec of embeddings) {
      expect(vec.length).toBe(EMBEDDING_DIM);
      // L2-normalized: magnitude should be ~1.0
      const magnitude = Math.sqrt(vec.reduce((sum, v) => sum + v * v, 0));
      expect(magnitude).toBeCloseTo(1.0, 3);
    }
  });
});

// ---------------------------------------------------------------------------
// 4. Database storage — full pipeline into local Docker Postgres
// ---------------------------------------------------------------------------

describe("database storage", () => {
  const resourceIds: string[] = [];

  afterAll(async () => {
    for (const id of resourceIds) {
      await cleanupTestResource(sql, id);
    }
  });

  // Chunk write / version-bump / dedup semantics are exercised end-to-end
  // by the Rust `chunk_dedup_test.rs` against the canonical SQL path
  // (`persist_resource_chunks` / `replace_resource_chunks`). The TS
  // integration test used to re-implement the INSERT via a legacy builder
  // that pre-dated the revision model; that builder has been removed.

  it("content hash is deterministic across pipeline runs", async () => {
    const res = await createTestResource(sql, "integration-test-deterministic");
    resourceIds.push(res.id);

    const { content } = await extractText(join(FIXTURES, "simple.md"));
    const chunks1 = chunkText(content);
    const chunks2 = chunkText(content);

    for (let i = 0; i < chunks1.length; i++) {
      expect(chunks1[i].content_hash).toBe(chunks2[i].content_hash);
    }
  });
});

// ---------------------------------------------------------------------------
// 5. Blob files status transitions
// ---------------------------------------------------------------------------

describe("blob_files status lifecycle", () => {
  let resource: TestResource;
  let blobFileId: string;

  beforeAll(async () => {
    resource = await createTestResource(sql, "integration-test-blob-status");
  });

  afterAll(async () => {
    await cleanupTestResource(sql, resource.id);
  });

  it("creates blob_file in pending state", async () => {
    blobFileId = await createTestBlobFile(sql, resource.id);

    const rows = await sql`SELECT status FROM kb_blob_files WHERE id = ${blobFileId}`;
    expect(rows[0].status).toBe("pending");
  });

  it("transitions pending → processing", async () => {
    const q = buildStatusUpdateQuery(blobFileId, "processing", null);
    await sql.unsafe(q.sql, q.params);

    const rows = await sql`SELECT status FROM kb_blob_files WHERE id = ${blobFileId}`;
    expect(rows[0].status).toBe("processing");
  });

  it("transitions processing → processed", async () => {
    const q = buildStatusUpdateQuery(blobFileId, "processed", null);
    await sql.unsafe(q.sql, q.params);

    const rows = await sql`SELECT status, updated_at FROM kb_blob_files WHERE id = ${blobFileId}`;
    expect(rows[0].status).toBe("processed");
  });

  it("transitions to failed with error message", async () => {
    // Reset to processing first
    const q1 = buildStatusUpdateQuery(blobFileId, "processing", null);
    await sql.unsafe(q1.sql, q1.params);

    const q2 = buildStatusUpdateQuery(blobFileId, "failed", "ONNX model load timeout");
    await sql.unsafe(q2.sql, q2.params);

    const rows =
      await sql`SELECT status, error_message FROM kb_blob_files WHERE id = ${blobFileId}`;
    expect(rows[0].status).toBe("failed");
    expect(rows[0].error_message).toBe("ONNX model load timeout");
  });
});
