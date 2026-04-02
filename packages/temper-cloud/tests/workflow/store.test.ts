import { describe, expect, it } from "vitest";
import {
  buildStatusUpdateQuery,
  buildStoreChunksQueries,
  buildVersionBumpQuery,
  type ChunkRow,
} from "../../src/workflow/store.js";

describe("buildStoreChunksQueries", () => {
  it("generates INSERT SQL for chunks and content in separate tables", () => {
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

    const queries = buildStoreChunksQueries(chunks);
    expect(queries.length).toBe(2);

    // First query: kb_chunks (no content column)
    expect(queries[0].sql).toContain("INSERT INTO kb_chunks");
    expect(queries[0].sql).not.toContain("content,");
    expect(queries[0].sql).toContain("ON CONFLICT");

    // Second query: kb_chunk_content
    expect(queries[1].sql).toContain("INSERT INTO kb_chunk_content");
    expect(queries[1].params).toContain("Hello world");
  });

  it("returns empty array for no chunks", () => {
    expect(buildStoreChunksQueries([])).toEqual([]);
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
  it("generates UPDATE SQL for kb_blob_files status", () => {
    const { sql, params } = buildStatusUpdateQuery("file-001", "processed", null);
    expect(sql).toContain("UPDATE kb_blob_files");
    expect(sql).toContain("status");
    expect(params).toContain("file-001");
    expect(params).toContain("processed");
  });

  it("includes error_message for failed status", () => {
    const { params } = buildStatusUpdateQuery("file-001", "failed", "ONNX load error");
    expect(params).toContain("ONNX load error");
  });
});
