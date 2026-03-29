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
