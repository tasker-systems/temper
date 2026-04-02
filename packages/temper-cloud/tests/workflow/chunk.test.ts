import { createHash } from "node:crypto";
import { describe, expect, it } from "vitest";
import { chunkText } from "../../src/workflow/chunk.js";

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

    const expectedHash = createHash("sha256").update(chunks1[0].content).digest("hex");
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

  it("splits oversized sections at paragraph boundaries", () => {
    const para = "x".repeat(1000);
    const text = `# Big Section\n\n${para}\n\n${para}\n\n${para}`;
    const chunks = chunkText(text);

    expect(chunks.length).toBeGreaterThan(1);
    for (const chunk of chunks) {
      // ~1785 chars max + small tolerance
      expect(chunk.content.length).toBeLessThan(2000);
    }
  });

  it("maintains sequential chunk indices after splitting", () => {
    const para = "x".repeat(1000);
    const text = `# A\n\n${para}\n\n${para}\n\n## B\n\nSmall.`;
    const chunks = chunkText(text);

    for (let i = 0; i < chunks.length; i++) {
      expect(chunks[i].chunk_index).toBe(i);
    }
  });
});
