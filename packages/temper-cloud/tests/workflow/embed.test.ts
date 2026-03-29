import { describe, it, expect } from "vitest";
import { embedTexts, EMBEDDING_DIM } from "../../src/workflow/embed.js";

describe("embedTexts", () => {
  it("produces 768-dimensional vectors", async () => {
    const texts = ["Hello world", "Testing embeddings"];
    const embeddings = await embedTexts(texts);

    expect(embeddings.length).toBe(2);
    expect(embeddings[0].length).toBe(EMBEDDING_DIM);
    expect(embeddings[1].length).toBe(EMBEDDING_DIM);
  }, 60_000); // Allow time for model download on first run

  it("produces deterministic output for same input", async () => {
    const texts = ["Deterministic test"];
    const embeddings1 = await embedTexts(texts);
    const embeddings2 = await embedTexts(texts);

    // Check first few values are close (floating point)
    for (let i = 0; i < 10; i++) {
      expect(embeddings1[0][i]).toBeCloseTo(embeddings2[0][i], 5);
    }
  }, 60_000);

  it("produces different vectors for different inputs", async () => {
    const embeddings = await embedTexts(["cats are great", "quantum physics theory"]);

    // Vectors should not be identical
    const identical = embeddings[0].every((v, i) => Math.abs(v - embeddings[1][i]) < 1e-6);
    expect(identical).toBe(false);
  }, 60_000);

  it("handles empty array", async () => {
    const embeddings = await embedTexts([]);
    expect(embeddings).toEqual([]);
  });
});
