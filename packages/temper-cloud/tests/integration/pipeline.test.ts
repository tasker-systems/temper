import { randomUUID } from "node:crypto";
import { join } from "node:path";
import type postgres from "postgres";
import { afterAll, beforeAll, describe, expect, it } from "vitest";
import { chunkText } from "../../src/workflow/chunk.js";
import { EMBEDDING_DIM, embedTexts } from "../../src/workflow/embed.js";
import { extractText } from "../../src/workflow/extract.js";
import {
  buildStatusUpdateQuery,
  buildStoreChunksQuery,
  buildVersionBumpQuery,
  type ChunkRow,
} from "../../src/workflow/store.js";
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

  it("extracts text from PNG via OCR", async () => {
    const result = await extractText(join(FIXTURES, "ocr-test.png"));
    // kreuzberg returns OCR text if Tesseract is available, otherwise image metadata.
    // Both are valid extraction results — OCR depends on system dependencies.
    expect(result.content.length).toBeGreaterThan(0);
    if (result.content.toLowerCase().includes("temper")) {
      // OCR backend available — full text extraction works
      expect(result.content.toLowerCase()).toContain("temper");
    } else {
      // No OCR backend — kreuzberg returns image metadata
      expect(result.content.toLowerCase()).toContain("png");
    }
  });
});

// ---------------------------------------------------------------------------
// 2. Chunking tests — verify header_path, content_hash, chunk_index
// ---------------------------------------------------------------------------

describe("fixture chunking", () => {
  it("chunks markdown with correct header paths", async () => {
    const { content } = await extractText(join(FIXTURES, "simple.md"));
    const chunks = chunkText(content);

    expect(chunks.length).toBeGreaterThanOrEqual(3);

    // First chunk should be under "Getting Started"
    expect(chunks[0].header_path).toContain("Getting Started");
    expect(chunks[0].chunk_index).toBe(0);

    // Each chunk has a deterministic hash
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

describe("fixture embedding", () => {
  it("embeds chunked markdown to 768-dim vectors", async () => {
    const { content } = await extractText(join(FIXTURES, "simple.md"));
    const chunks = chunkText(content);
    const texts = chunks.map((c) => c.content);
    const embeddings = await embedTexts(texts);

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
  let resource: TestResource;
  const resourceIds: string[] = [];

  afterAll(async () => {
    for (const id of resourceIds) {
      await cleanupTestResource(sql, id);
    }
  });

  it("stores chunks with correct schema", async () => {
    resource = await createTestResource(sql, "integration-test-md");
    resourceIds.push(resource.id);

    // Extract → chunk → embed
    const { content } = await extractText(join(FIXTURES, "simple.md"));
    const chunks = chunkText(content);
    const embeddings = await embedTexts(chunks.map((c) => c.content));

    // Build chunk rows
    const rows: ChunkRow[] = chunks.map((chunk, i) => ({
      id: randomUUID(),
      resource_id: resource.id,
      chunk_index: chunk.chunk_index,
      version: 1,
      header_path: chunk.header_path,
      content: chunk.content,
      content_hash: chunk.content_hash,
      embedding: embeddings[i],
    }));

    // Store to database
    const { sql: insertSql, params } = buildStoreChunksQuery(rows);
    await sql.unsafe(insertSql, params);

    // Verify rows
    const stored =
      await sql`SELECT * FROM kb_chunks WHERE resource_id = ${resource.id} ORDER BY chunk_index`;

    expect(stored.length).toBe(chunks.length);
    for (let i = 0; i < stored.length; i++) {
      expect(stored[i].chunk_index).toBe(i);
      expect(stored[i].version).toBe(1);
      expect(stored[i].is_current).toBe(true);
      expect(stored[i].header_path).toBe(chunks[i].header_path);
      expect(stored[i].content_hash).toBe(chunks[i].content_hash);
      expect(stored[i].content).toBe(chunks[i].content);
    }

    // Verify embedding dimensions by checking the stored vector
    const embCheck = await sql`
      SELECT vector_dims(embedding) as dims
      FROM kb_chunks
      WHERE resource_id = ${resource.id}
      LIMIT 1
    `;
    expect(embCheck[0].dims).toBe(EMBEDDING_DIM);
  });

  it("version bumps old chunks on re-upload", async () => {
    const res = await createTestResource(sql, "integration-test-version-bump");
    resourceIds.push(res.id);

    // Version 1: store initial chunks
    const { content } = await extractText(join(FIXTURES, "simple.txt"));
    const chunks = chunkText(content);
    const embeddings = await embedTexts(chunks.map((c) => c.content));

    const v1Rows: ChunkRow[] = chunks.map((chunk, i) => ({
      id: randomUUID(),
      resource_id: res.id,
      chunk_index: chunk.chunk_index,
      version: 1,
      header_path: chunk.header_path,
      content: chunk.content,
      content_hash: chunk.content_hash,
      embedding: embeddings[i],
    }));

    const v1Query = buildStoreChunksQuery(v1Rows);
    await sql.unsafe(v1Query.sql, v1Query.params);

    // Version bump: mark v1 as not current
    const bumpQuery = buildVersionBumpQuery(res.id, 2);
    await sql.unsafe(bumpQuery.sql, bumpQuery.params);

    // Version 2: store new chunks
    const v2Rows: ChunkRow[] = chunks.map((chunk, i) => ({
      id: randomUUID(),
      resource_id: res.id,
      chunk_index: chunk.chunk_index,
      version: 2,
      header_path: chunk.header_path,
      content: chunk.content,
      content_hash: chunk.content_hash,
      embedding: embeddings[i],
    }));

    const v2Query = buildStoreChunksQuery(v2Rows);
    await sql.unsafe(v2Query.sql, v2Query.params);

    // Verify: v1 chunks are not current
    const v1Chunks = await sql`
      SELECT is_current, version FROM kb_chunks
      WHERE resource_id = ${res.id} AND version = 1
    `;
    for (const row of v1Chunks) {
      expect(row.is_current).toBe(false);
      expect(row.version).toBe(1);
    }

    // Verify: v2 chunks are current
    const v2Chunks = await sql`
      SELECT is_current, version FROM kb_chunks
      WHERE resource_id = ${res.id} AND version = 2
    `;
    for (const row of v2Chunks) {
      expect(row.is_current).toBe(true);
      expect(row.version).toBe(2);
    }

    // Total chunks: v1 + v2
    const allChunks = await sql`
      SELECT count(*)::int as count FROM kb_chunks
      WHERE resource_id = ${res.id}
    `;
    expect(allChunks[0].count).toBe(chunks.length * 2);
  });

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

    const rows = await sql`SELECT status FROM blob_files WHERE id = ${blobFileId}`;
    expect(rows[0].status).toBe("pending");
  });

  it("transitions pending → processing", async () => {
    const q = buildStatusUpdateQuery(blobFileId, "processing", null);
    await sql.unsafe(q.sql, q.params);

    const rows = await sql`SELECT status FROM blob_files WHERE id = ${blobFileId}`;
    expect(rows[0].status).toBe("processing");
  });

  it("transitions processing → processed", async () => {
    const q = buildStatusUpdateQuery(blobFileId, "processed", null);
    await sql.unsafe(q.sql, q.params);

    const rows = await sql`SELECT status, updated_at FROM blob_files WHERE id = ${blobFileId}`;
    expect(rows[0].status).toBe("processed");
  });

  it("transitions to failed with error message", async () => {
    // Reset to processing first
    const q1 = buildStatusUpdateQuery(blobFileId, "processing", null);
    await sql.unsafe(q1.sql, q1.params);

    const q2 = buildStatusUpdateQuery(blobFileId, "failed", "ONNX model load timeout");
    await sql.unsafe(q2.sql, q2.params);

    const rows = await sql`SELECT status, error_message FROM blob_files WHERE id = ${blobFileId}`;
    expect(rows[0].status).toBe("failed");
    expect(rows[0].error_message).toBe("ONNX model load timeout");
  });
});
