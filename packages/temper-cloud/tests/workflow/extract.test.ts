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
